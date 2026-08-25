// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! EigenTT readback: values → normal-form expressions.
//!
//! Ported from `Main.hs` lines 226-255 in the EigenTT reference.
//! Readback converts semantic values back to syntax, producing
//! normal forms. Two values are definitionally equal iff their
//! readbacks at the same level are syntactically equal.

use crate::nbe::env::Rho;
use crate::nbe::eval::EvalError;
use crate::nbe::term::{Exp, Name, Patt, Summand};
use crate::nbe::val::{Neut, Val};

/// Readback a value to a normal-form expression — **asserting the value is well-typed**.
///
/// `level` is the current de Bruijn level (number of binders above). Port of `rbV` from the
/// reference.
///
/// Readback is *total on well-typed values*: a value in function position under a binder is, by
/// well-typedness, a function, so `apply` never fails there. In the Haskell reference this was
/// simply structural — there was no failure case. This entry point preserves that invariant: a
/// failure means the caller handed readback a value the type checker never sanctioned, so it is a
/// kernel-invariant violation and panics. It is the right call for the ~all callers that read back
/// a value the checker already produced.
///
/// A caller that hands readback an **un-vetted** term — the felicity gate normalises candidate
/// parser sems precisely to test whether they are well-typed (GH#104) — must instead use
/// [`try_readback_val`], which returns the `apply`/`eval` failure as an `Err`. `eval` is already
/// fallible this way; `try_readback_val` restores the parity the port dropped, and is why the
/// felicity gate no longer needs a `catch_unwind` around the panic.
pub fn readback_val(level: usize, val: &Val) -> Exp {
    try_readback_val(level, val).expect(
        "readback_val: apply/eval failed on a value assumed well-typed — the caller handed \
         readback an un-vetted term; use try_readback_val at that boundary",
    )
}

/// Readback a neutral term, asserting well-typedness — see [`readback_val`].
pub fn readback_neut(level: usize, neut: &Neut) -> Exp {
    try_readback_neut(level, neut).expect(
        "readback_neut: apply/eval failed on a value assumed well-typed; use try_readback_val at \
         an un-vetted boundary",
    )
}

/// Fallible readback (see [`readback_val`] for the invariant it upholds). Returns `Err` — rather
/// than panicking — when a value in function position is **not a function**, or an embedded `eval`
/// fails: the signature of ill-typed input. On well-typed input it is identical to
/// [`readback_val`]. This is the entry point for the felicity gate, which reads back un-vetted
/// candidate sems.
pub fn try_readback_val(level: usize, val: &Val) -> Result<Exp, EvalError> {
    Ok(match val {
        Val::Lam(f) => {
            let gen = gen_val(level);
            Exp::Lam(
                gen_patt(level),
                Box::new(try_readback_val(level + 1, &f.apply(gen)?)?),
            )
        }
        Val::Pair(u, v) => Exp::Pair(
            Box::new(try_readback_val(level, u)?),
            Box::new(try_readback_val(level, v)?),
        ),
        Val::Con(c, v) => Exp::Con(c.clone(), Box::new(try_readback_val(level, v)?)),
        Val::Unit => Exp::Unit,
        Val::Sort(n) => Exp::Sort(n.clone()),
        Val::Pi(t, g) => {
            // Preserve Patt::Unit (anonymous binders) from the original
            // closure so round-tripping `A -> B` through eval+readback
            // doesn't introduce a `G#N` binder name that would diverge
            // from the author's encoding. Critical for D49 witness-key
            // hashes — chain-stored canonical_proposition encodes
            // anonymous arrow binders as `Patt::Unit`; the synthesis
            // hook's readback+encode must produce identical bytes.
            let gen = gen_val(level);
            let patt = if matches!(g.patt, Patt::Unit) {
                Patt::Unit
            } else {
                gen_patt(level)
            };
            Exp::Pi(
                patt,
                Box::new(try_readback_val(level, t)?),
                Box::new(try_readback_val(level + 1, &g.apply(gen)?)?),
            )
        }
        Val::Sig(t, g) => {
            let gen = gen_val(level);
            let patt = if matches!(g.patt, Patt::Unit) {
                Patt::Unit
            } else {
                gen_patt(level)
            };
            Exp::Sig(
                patt,
                Box::new(try_readback_val(level, t)?),
                Box::new(try_readback_val(level + 1, &g.apply(gen)?)?),
            )
        }
        Val::One => Exp::One,
        Val::Fun(cases, rho) => try_readback_fun(level, cases, rho)?,
        Val::Data(summands, rho) => try_readback_data(level, summands, rho)?,
        // D78 §1 — walk the telescope, instantiating each binder at a fresh
        // generic so a later field's type reads back against the same binders it
        // was written against. Field order is already canonical (`Exp::record`
        // establishes it), so readback preserves it and `eq_nf`'s syntactic
        // comparison decides record equality with no bespoke conversion arm.
        Val::Refine(carrier, classes) => {
            Exp::Refine(Box::new(try_readback_val(level, carrier)?), classes.clone())
        }
        Val::Record(fields, rho) => {
            let mut out: Vec<(crate::ontology::iri::Iri, Patt, Exp)> = Vec::new();
            let mut env = rho.clone();
            for (i, (iri, patt, ty)) in fields.iter().enumerate() {
                let lvl = level + i;
                let ty_val = crate::nbe::eval::eval(ty, &env)?;
                out.push((iri.clone(), gen_patt(lvl), try_readback_val(lvl, &ty_val)?));
                env = env.extend(patt.clone(), gen_val(lvl));
            }
            Exp::Record(out)
        }
        Val::Nt(k) => try_readback_neut(level, k)?,

        // Identity type
        Val::Id(a, x, y) => Exp::Id(
            Box::new(try_readback_val(level, a)?),
            Box::new(try_readback_val(level, x)?),
            Box::new(try_readback_val(level, y)?),
        ),
        Val::Refl(a) => Exp::Refl(Box::new(try_readback_val(level, a)?)),

        // Template
        Val::TemplateVal(s, refs) => Exp::Template(
            s.clone(),
            refs.iter()
                .map(|(iri, val)| Ok((iri.clone(), Box::new(try_readback_val(level, val)?))))
                .collect::<Result<Vec<_>, EvalError>>()?,
        ),

        // Eigenius extensions
        Val::EigonClass(iri) => Exp::EigonClass(iri.clone()),
        Val::EigonPrimitive(p) => Exp::EigonPrimitive(*p),
        Val::ResourceVal(r) => Exp::EigonResource(r.clone()),

        // Map/Reduce (Phase 11a)
        Val::List(items) => {
            // Read back as nested Con("cons", Pair(head, ...)) terminated by Con("nil", Unit)
            let mut result = Exp::Con("nil".into(), Box::new(Exp::Unit));
            for item in items.iter().rev() {
                result = Exp::Con(
                    "cons".into(),
                    Box::new(Exp::Pair(
                        Box::new(try_readback_val(level, item)?),
                        Box::new(result),
                    )),
                );
            }
            result
        }

        // Inductive types (Phase 11b, D19; D48 indices).
        // The reference's `App` spine carries `params ++ indices`, split on the
        // decoder side by `decl.params.len()` (D48 Phase B).
        // For non-indexed declarations (`decl.indices` empty), this is
        // equivalent to the pre-D48 behaviour.
        Val::InductiveType {
            decl,
            params,
            indices,
        } => Exp::const_applied(
            decl.iri.clone(),
            // **Levels are not carried yet.** `Val::InductiveType` has no level
            // slot, so a polymorphic instantiation would be lost here — the same
            // gap `Neut::Const` had before this phase, and E2's to close when
            // declarations gain `uparams`. Empty is exact today: nothing produces
            // a non-empty level list.
            Vec::new(),
            params
                .iter()
                .chain(indices.iter())
                .map(|p| try_readback_val(level, p))
                .collect::<Result<Vec<_>, EvalError>>()?,
        ),
        Val::InductiveVal {
            iri,
            ctor_name,
            args,
        } => Exp::InductiveCtor(
            iri.clone(),
            ctor_name.clone(),
            args.iter()
                .map(|a| try_readback_val(level, a))
                .collect::<Result<Vec<_>, EvalError>>()?,
        ),

        // eigenius#71 / D49 — literals round-trip as themselves.
        Val::LitString(s) => Exp::LitString(s.clone()),
        Val::LitInt(n) => Exp::LitInt(*n),
        Val::LitFloat(f) => Exp::LitFloat(*f),
        Val::LitBool(b) => Exp::LitBool(*b),

        // D49 §8 — `ChainWitness` values are opaque, kernel-internal
        // proof-of-existence markers admitted by the per-Layer witness
        // index. They never appear in surface syntax, so readback into
        // an `Exp` is a programming error: they should only be produced
        // by the type checker's synthesis hook at `JustifiedBy.*`
        // type-check time and consumed within the same type-check; they
        // do not survive normalisation into a readback-able form. This is
        // a genuine kernel-internal invariant (never input-dependent), so
        // it stays a hard panic even on the fallible path.
        Val::ChainWitness(key) => panic!(
            "readback_val: ChainWitness {:?} reached readback — witness values are \
             kernel-internal and should be consumed at JustifiedBy.* type-check time, \
             never readback into surface syntax",
            key
        ),
    })
}

/// Fallible readback of a neutral term (see [`readback_val`]/[`try_readback_val`]).
///
/// Port of `rbN` from the reference.
pub fn try_readback_neut(level: usize, neut: &Neut) -> Result<Exp, EvalError> {
    Ok(match neut {
        // D76 Phase B1 — an unresolved named reference reads back as itself.
        Neut::Const(iri, levels) => Exp::Const(iri.clone(), levels.clone()),
        Neut::Gen(j, name) => Exp::Var(format!("{name}{j}")),
        // D48 Phase C: an unsolved metavariable reads back as a fresh
        // variable name (`?<id>`) plus the spine applied. Solved metas
        // are resolved before readback by the unifier (`zonk` step);
        // a Meta surviving to readback is by definition unsolved.
        Neut::Meta(id, spine) => {
            let mut acc = Exp::Var(format!("?{}", id.0));
            for v in spine.iter() {
                acc = Exp::App(Box::new(acc), Box::new(try_readback_val(level, v)?));
            }
            acc
        }
        Neut::App(k, m) => Exp::App(
            Box::new(try_readback_neut(level, k)?),
            Box::new(try_readback_val(level, m)?),
        ),
        Neut::Fst(k) => Exp::Fst(Box::new(try_readback_neut(level, k)?)),
        Neut::Snd(k) => Exp::Snd(Box::new(try_readback_neut(level, k)?)),
        Neut::NtFun(cases, rho, k) => {
            let fun_exp = try_readback_fun(level, cases, rho)?;
            Exp::App(Box::new(fun_exp), Box::new(try_readback_neut(level, k)?))
        }
        // Eigenius extension
        Neut::EigonAxiom(iri) => Exp::EigonAxiom(iri.clone()),
        Neut::PropAccess(k, prop) => {
            Exp::PropAccess(Box::new(try_readback_neut(level, k)?), prop.clone())
        }

        // Map/Reduce (Phase 11a)
        Neut::NtMap(f, k) => Exp::Map(
            Box::new(try_readback_val(level, f)?),
            Box::new(try_readback_neut(level, k)?),
        ),
        Neut::NtReduce(f, acc, k) => Exp::Reduce(
            Box::new(try_readback_val(level, f)?),
            Box::new(try_readback_val(level, acc)?),
            Box::new(try_readback_neut(level, k)?),
        ),

        // Inductive types (Phase 11b, D19)
        Neut::NtRec {
            decl,
            motive,
            minors,
            major,
        } => Exp::InductiveRec {
            iri: decl.iri.clone(),
            motive: Box::new(try_readback_val(level, motive)?),
            minors: minors
                .iter()
                .map(|m| try_readback_val(level, m))
                .collect::<Result<Vec<_>, EvalError>>()?,
            major: Box::new(try_readback_neut(level, major)?),
        },

        // Pattern-match blocked on a neutral scrutinee (Phase 11b
        // step 12). Read back as `Exp::Match`, preserving the motive-
        // free shape — the type checker re-synthesises the motive
        // from context the next time this term is checked.
        //
        // The captured `env` is intentionally not consulted during
        // readback. Arm bodies may reference variables from that env;
        // for the readback to be self-contained we'd have to inline
        // those references. This is the conservative readback shape
        // (parallel to how `Val::CoRecord` is read back).
        Neut::NtMatch {
            scrutinee,
            arms,
            env: _,
        } => Exp::Match {
            scrutinee: Box::new(try_readback_neut(level, scrutinee)?),
            arms: arms.clone(),
        },
    })
}

/// Readback a Data (Sum type) value.
///
/// Evaluates each summand's type expression in the captured environment,
/// then reads back the resulting value. This avoids the old placeholder
/// approach that produced `__data_N` variable references.
fn try_readback_data(level: usize, summands: &[(Name, Exp)], rho: &Rho) -> Result<Exp, EvalError> {
    let read_summands: Vec<Summand> = summands
        .iter()
        .map(|(name, exp)| {
            let val = crate::nbe::eval::eval(exp, rho)?;
            Ok(Summand {
                name: name.clone(),
                typ: try_readback_val(level, &val)?,
            })
        })
        .collect::<Result<Vec<_>, EvalError>>()?;
    Ok(Exp::Data(read_summands))
}

/// Readback a Fun (case function) value.
///
/// Evaluates each branch body in the captured environment to produce
/// a proper case expression.
fn try_readback_fun(level: usize, cases: &[(Name, Exp)], rho: &Rho) -> Result<Exp, EvalError> {
    // A Fun is a case function: fun(c₁ → e₁ | c₂ → e₂ | ...)
    // Each branch is a closure over the constructor's payload.
    // We evaluate each branch with a fresh variable and read back.
    let gen = gen_val(level);
    let branches: Vec<(Name, Exp)> = cases
        .iter()
        .map(|(name, body)| {
            let branch_val = crate::nbe::eval::eval(body, rho)?.app(gen.clone())?;
            Ok((name.clone(), try_readback_val(level + 1, &branch_val)?))
        })
        .collect::<Result<Vec<_>, EvalError>>()?;
    Ok(Exp::Case(
        branches
            .into_iter()
            .map(|(name, body)| crate::nbe::term::Branch {
                name,
                body: Exp::Lam(gen_patt(level), Box::new(body)),
            })
            .collect(),
    ))
}

/// Generate a fresh variable value at a given level. The `G#` name tag
/// pairs with [`gen_patt`]'s `G#{level}` and is load-bearing — a
/// `Neut::Gen(j, name)` reads back as `Exp::Var("{name}{j}")` — so this
/// is intentionally distinct from `env::gen_val`'s `TC#` convention,
/// not a duplication to merge.
fn gen_val(level: usize) -> Val {
    Val::Nt(Neut::Gen(level, "G#".to_string()))
}

/// Generate a pattern for a fresh variable at a given level.
fn gen_patt(level: usize) -> Patt {
    Patt::Var(format!("G#{level}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::val::Clos;

    #[test]
    fn readback_unit() {
        assert_eq!(readback_val(0, &Val::Unit), Exp::Unit);
    }

    #[test]
    fn readback_set() {
        assert_eq!(readback_val(0, &Val::sort(1)), Exp::sort(1));
    }

    #[test]
    fn readback_one() {
        assert_eq!(readback_val(0, &Val::One), Exp::One);
    }

    #[test]
    fn readback_pair() {
        let v = Val::Pair(Box::new(Val::Unit), Box::new(Val::sort(1)));
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::Pair(_, _)));
    }

    #[test]
    fn readback_constructor() {
        let v = Val::Con("ok".to_string(), Box::new(Val::Unit));
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::Con(ref c, _) if c == "ok"));
    }

    #[test]
    fn readback_neutral_gen() {
        let v = Val::Nt(Neut::Gen(0, "x".to_string()));
        let e = readback_val(0, &v);
        assert_eq!(e, Exp::Var("x0".to_string()));
    }

    #[test]
    fn readback_neutral_app() {
        let v = Val::Nt(Neut::App(
            Box::new(Neut::Gen(0, "f".to_string())),
            Box::new(Val::Unit),
        ));
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::App(_, _)));
    }

    #[test]
    fn readback_lambda() {
        // λx.x — identity function
        let f = Clos::new(
            Patt::Var("x".to_string()),
            Exp::Var("x".to_string()),
            Rho::Nil,
        );
        let v = Val::Lam(f);
        let e = readback_val(0, &v);
        // Should readback as λG#0. G#0
        assert!(matches!(e, Exp::Lam(_, _)));
    }

    #[test]
    fn eq_nf_by_readback() {
        // Two values are equal iff their readbacks are equal
        let v1 = Val::Unit;
        let v2 = Val::Unit;
        assert_eq!(readback_val(0, &v1), readback_val(0, &v2));

        let v3 = Val::sort(1);
        assert_ne!(readback_val(0, &v1), readback_val(0, &v3));
    }

    /// eigenius#142 — `LitBool` needs no conversion rule of its own:
    /// `eq_nf` is readback plus `Exp`'s derived `PartialEq`, so
    /// `true` and `false` are distinguished by the readback arm alone.
    #[test]
    fn lit_bool_readback_distinguishes_true_from_false() {
        use crate::nbe::check::eq_nf;
        assert_eq!(readback_val(0, &Val::LitBool(true)), Exp::LitBool(true));
        assert!(eq_nf(0, &Val::LitBool(true), &Val::LitBool(true)).is_ok());
        assert!(eq_nf(0, &Val::LitBool(true), &Val::LitBool(false)).is_err());
    }

    // --- Codata readback tests (D11, Phase 9b-i) ---

    // --- Map/Reduce readback tests (Phase 11a) ---

    #[test]
    fn readback_empty_list() {
        let v = Val::List(vec![]);
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::Con(ref c, _) if c == "nil"));
    }

    #[test]
    fn readback_two_element_list() {
        let v = Val::List(vec![Val::Unit, Val::sort(1)]);
        let e = readback_val(0, &v);
        // Should be Con("cons", Pair(Unit, Con("cons", Pair(Set, Con("nil", Unit)))))
        assert!(matches!(e, Exp::Con(ref c, _) if c == "cons"));
    }

    #[test]
    fn readback_neutral_map() {
        let v = Val::Nt(Neut::NtMap(
            Box::new(Val::Unit), // placeholder function
            Box::new(Neut::Gen(0, "xs".to_string())),
        ));
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::Map(_, _)));
    }

    #[test]
    fn readback_neutral_reduce() {
        let v = Val::Nt(Neut::NtReduce(
            Box::new(Val::Unit),    // placeholder function
            Box::new(Val::sort(1)), // placeholder accumulator
            Box::new(Neut::Gen(0, "xs".to_string())),
        ));
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::Reduce(_, _, _)));
    }
}

#[cfg(test)]
mod record_round_trip {
    use crate::nbe::check::eq_nf;
    use crate::nbe::env::Rho;
    use crate::nbe::eval::eval;
    use crate::nbe::term::{Exp, Patt};
    use crate::ontology::iri::Iri;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }
    fn plain(name: &str, binder: &str) -> (Iri, Patt, Exp) {
        (iri(name), Patt::Var(binder.into()), Exp::sort(1))
    }
    /// A field whose type mentions an earlier binder.
    fn dependent(name: &str, binder: &str, on: &str) -> (Iri, Patt, Exp) {
        (
            iri(name),
            Patt::Var(binder.into()),
            Exp::Times(Box::new(Exp::Var(on.into())), Box::new(Exp::sort(1))),
        )
    }

    #[test]
    fn a_record_survives_eval_and_readback() {
        let e = Exp::record(vec![plain("urn:t:a", "a"), plain("urn:t:b", "b")]).unwrap();
        let v = eval(&e, &Rho::Nil).unwrap();
        let back = super::readback_val(0, &v);
        match back {
            Exp::Record(fs) => {
                let names: Vec<&str> = fs.iter().map(|(i, _, _)| i.as_str()).collect();
                assert_eq!(
                    names,
                    ["urn:t:a", "urn:t:b"],
                    "field keys and order survive"
                );
            }
            other => panic!("expected a record, got {other:?}"),
        }
    }

    #[test]
    fn a_dependent_field_reads_back_against_its_binder() {
        // `b`'s type mentions `a`'s binder. Readback instantiates each binder at
        // a fresh generic, so the mention must survive as a bound occurrence
        // rather than dangling.
        let e = Exp::record(vec![plain("urn:t:a", "a"), dependent("urn:t:b", "b", "a")]).unwrap();
        let v = eval(&e, &Rho::Nil).unwrap();
        let back = super::readback_val(0, &v);
        match &back {
            Exp::Record(fs) => {
                assert_eq!(fs.len(), 2);
                let mentions = crate::nbe::subst::free_vars(&fs[1].2);
                assert!(
                    !mentions.is_empty(),
                    "the dependency must survive readback: {:?}",
                    fs[1].2
                );
            }
            other => panic!("expected a record, got {other:?}"),
        }
    }

    #[test]
    fn canonical_order_makes_two_spellings_convertible() {
        // The payoff of D78 §1's invariant: `eq_nf` compares by readback and
        // syntactic equality, so the same field set written two ways must
        // converge with no bespoke conversion arm for records.
        let a = Exp::record(vec![plain("urn:t:a", "a"), plain("urn:t:b", "b")]).unwrap();
        let b = Exp::record(vec![plain("urn:t:b", "b"), plain("urn:t:a", "a")]).unwrap();
        let va = eval(&a, &Rho::Nil).unwrap();
        let vb = eval(&b, &Rho::Nil).unwrap();
        assert!(
            eq_nf(0, &va, &vb).is_ok(),
            "two spellings of one field set must be convertible"
        );
    }

    #[test]
    fn records_over_different_field_sets_are_not_convertible() {
        let a = Exp::record(vec![plain("urn:t:a", "a")]).unwrap();
        let b = Exp::record(vec![plain("urn:t:b", "b")]).unwrap();
        let va = eval(&a, &Rho::Nil).unwrap();
        let vb = eval(&b, &Rho::Nil).unwrap();
        assert!(
            eq_nf(0, &va, &vb).is_err(),
            "different field keys must not be convertible"
        );
    }

    #[test]
    fn field_identity_is_the_full_iri_not_the_local_name() {
        // The collision `find_sigma_field` has today: it projects by
        // `local_name()`, so these two would be the same field. Keyed by IRI
        // they are not (D78 §9).
        let a = Exp::record(vec![plain("urn:eigenius:a:name", "n")]).unwrap();
        let b = Exp::record(vec![plain("urn:eigenius:b:name", "n")]).unwrap();
        let va = eval(&a, &Rho::Nil).unwrap();
        let vb = eval(&b, &Rho::Nil).unwrap();
        assert!(
            eq_nf(0, &va, &vb).is_err(),
            "same local name, different namespace — must not be convertible"
        );
    }
}

#[cfg(test)]
mod refine_semantics {
    //! D78 §3 — `Val::Refine` carries a *set* of class constraints.

    use crate::nbe::check::{eq_nf, subtype_of};
    use crate::nbe::env::Rho;
    use crate::nbe::eval::eval;
    use crate::nbe::term::{Exp, Patt};
    use crate::ontology::iri::Iri;
    use std::collections::BTreeSet;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }
    fn set(names: &[&str]) -> BTreeSet<Iri> {
        names.iter().map(|n| iri(n)).collect()
    }
    fn rec(fields: &[&str]) -> Exp {
        Exp::record(
            fields
                .iter()
                .map(|f| (iri(f), Patt::Var(f.to_string()), Exp::sort(1)))
                .collect(),
        )
        .unwrap()
    }
    fn v(e: &Exp) -> crate::nbe::val::Val {
        eval(e, &Rho::Nil).unwrap()
    }

    #[test]
    fn the_empty_constraint_set_degenerates_to_the_carrier() {
        // D78 §3 reason 2 for the flat form: "0 or more constraints" needs no
        // special case, and there is only one representation of zero.
        let carrier = rec(&["urn:t:a"]);
        let refined = Exp::Refine(Box::new(carrier.clone()), BTreeSet::new());
        assert!(
            eq_nf(0, &v(&refined), &v(&carrier)).is_ok(),
            "Refine(R, {{}}) must be R"
        );
    }

    #[test]
    fn constraint_identity_is_nominal_not_structural() {
        // The measured case: 749 of 894 shipped classes have identical (empty)
        // field sets, so only the names distinguish them (D78 §1.2).
        let carrier = rec(&["urn:t:a"]);
        let alpha = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:Alpha"]));
        let beta = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:Beta"]));
        assert!(
            eq_nf(0, &v(&alpha), &v(&beta)).is_err(),
            "same carrier, different class — must not be convertible"
        );
    }

    #[test]
    fn constraint_order_does_not_matter() {
        // A `BTreeSet` has one representation, which is why the flat form beats
        // nesting: `Refine(Refine(R,C),D)` and `Refine(Refine(R,D),C)` would be
        // two spellings of one type (D78 §3 reason 1).
        let carrier = rec(&["urn:t:a"]);
        let cd = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:C", "urn:t:D"]));
        let dc = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:D", "urn:t:C"]));
        assert!(eq_nf(0, &v(&cd), &v(&dc)).is_ok());
    }

    #[test]
    fn forgetting_constraints_is_safe_but_inventing_them_is_not() {
        let carrier = rec(&["urn:t:a"]);
        let refined = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:C"]));
        assert!(
            subtype_of(
                &crate::nbe::env_global::Env::empty(),
                0,
                &v(&refined),
                &v(&carrier)
            )
            .is_ok(),
            "Refine(R, S) <: R — a refined record flows into a plain-record context"
        );
        assert!(
            subtype_of(
                &crate::nbe::env_global::Env::empty(),
                0,
                &v(&carrier),
                &v(&refined)
            )
            .is_err(),
            "R <: Refine(R, S) must NOT hold — that would invent a claim"
        );
    }

    #[test]
    fn a_larger_constraint_set_is_a_subtype() {
        let carrier = rec(&["urn:t:a"]);
        let more = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:C", "urn:t:D"]));
        let fewer = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:C"]));
        assert!(
            subtype_of(
                &crate::nbe::env_global::Env::empty(),
                0,
                &v(&more),
                &v(&fewer)
            )
            .is_ok(),
            "satisfying more constraints is satisfying fewer"
        );
        assert!(
            subtype_of(
                &crate::nbe::env_global::Env::empty(),
                0,
                &v(&fewer),
                &v(&more)
            )
            .is_err(),
            "the converse must not hold"
        );
    }

    #[test]
    fn a_refinement_flows_into_a_nominal_class_context_by_its_constraint_set() {
        // D78 Phase E. `Construct C {}` yields `Refine(record, {C})`, and an
        // inductive constructor's parameter is `EigonClass(C)`. What makes the
        // one an inhabitant of the other is the **constraint set**, not the
        // carrier.
        //
        // Forgetting the set and comparing carriers is wrong against a *nominal*
        // supertype, and since Phase C a no-`requires` class carries an empty
        // record — so the comparison became `Record([]) ≠ EigonClass(C)` and a
        // well-typed composition was rejected
        // (`felicity_filter_accepts_well_typed_composition`).
        let carrier = rec(&["urn:t:a"]);
        let refined = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:C"]));
        let as_class = crate::nbe::val::Val::EigonClass(iri("urn:t:C"));

        assert!(
            subtype_of(
                &crate::nbe::env_global::Env::empty(),
                0,
                &v(&refined),
                &as_class
            )
            .is_ok(),
            "a record declaring C must flow into a context expecting class C"
        );
        assert!(
            subtype_of(
                &crate::nbe::env_global::Env::empty(),
                0,
                &v(&refined),
                &crate::nbe::val::Val::EigonClass(iri("urn:t:Other"))
            )
            .is_err(),
            "but not into a class it does not declare"
        );

        // The empty carrier is the case that actually broke: 749 of 894 shipped
        // classes have no `requires`.
        let empty = Exp::Refine(Box::new(Exp::record(vec![]).unwrap()), set(&["urn:t:C"]));
        assert!(
            subtype_of(
                &crate::nbe::env_global::Env::empty(),
                0,
                &v(&empty),
                &as_class
            )
            .is_ok(),
            "an empty record declaring C is still an instance of C"
        );

        // Forgetting still applies against a structural supertype.
        assert!(
            subtype_of(
                &crate::nbe::env_global::Env::empty(),
                0,
                &v(&refined),
                &v(&carrier)
            )
            .is_ok(),
            "Refine(R, S) <: R must keep working"
        );
    }

    /// D76 Phase D — the parked obligation, discharged.
    ///
    /// This asserted the *incompleteness*: `Refine(R, {Pup}) <: Refine(R, {Dog})`
    /// was rejected because deciding `Pup ⊨ Dog` resolves class IRIs and
    /// conversion had no environment. It now decides it, and the assertion is
    /// inverted.
    ///
    /// **Set inclusion stays the fast path.** The environment is consulted only
    /// where inclusion fails — that is, only where the conservative rule was about
    /// to reject — which is how conversion still resolves nothing on the equal
    /// path (D76 §5).
    #[test]
    fn entailment_beyond_set_inclusion_is_now_decided() {
        // `Pup` requires everything `Dog` does and more, so `⋀{Pup} ⊨ Dog` — but
        // `{Pup} ⊉ {Dog}`, so set inclusion alone rejects it.
        use crate::layer::{LayerBuilder, LayerStorage};
        use crate::ontology::resource::{Resource, Value};
        use crate::ontology::well_known as wk;

        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let mut b = LayerBuilder::new("core", None);
        for r in crate::ontology::eigon_json::parse_document(core_json).unwrap() {
            b.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(b.build(LayerStorage::in_memory()));

        // Two properties. `constraint_fields` reads `requires` and collects IRIs,
        // so no range is needed to decide entailment (D78 §4.1: the per-field
        // variance clause is vacuous because a field's type is a function of the
        // property, not of the class).
        let property = |id: &str| {
            let mut r = Resource::new(iri(id));
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
            );
            r
        };

        let class_with = |id: &str, reqs: Vec<&str>| {
            let mut r = Resource::new(iri(id));
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
            );
            r.set(
                iri(wk::REQUIRES),
                Value::Array(
                    reqs.into_iter()
                        .map(|p| Value::ResourceRef(iri(p)))
                        .collect(),
                ),
            );
            r
        };

        let mut d = LayerBuilder::new("t", Some(core));
        for r in [
            property("urn:t:name"),
            property("urn:t:tag"),
            class_with("urn:t:Dog", vec!["urn:t:name"]),
            class_with("urn:t:Pup", vec!["urn:t:name", "urn:t:tag"]),
            class_with("urn:t:Cat", vec!["urn:t:tag"]),
        ] {
            d.add_resource(r).unwrap();
        }
        let layer = std::sync::Arc::new(d.build(LayerStorage::in_memory()));
        let env = crate::nbe::env_global::Env::of(layer);

        let carrier = rec(&["urn:t:a"]);
        let pup = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:Pup"]));
        let dog = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:Dog"]));
        let cat = Exp::Refine(Box::new(carrier.clone()), set(&["urn:t:Cat"]));

        assert!(
            subtype_of(&env, 0, &v(&pup), &v(&dog)).is_ok(),
            "Pup's fields cover Dog's, so ⋀{{Pup}} ⊨ Dog — legal since D76 Phase D"
        );
        assert!(
            subtype_of(&env, 0, &v(&dog), &v(&pup)).is_err(),
            "and not the converse: Dog does not require `tag`"
        );
        assert!(
            subtype_of(&env, 0, &v(&cat), &v(&dog)).is_err(),
            "nor between classes whose fields merely overlap"
        );

        // The empty environment decides nothing, so the conservative rule stands —
        // an environment omitted by accident cannot silently widen what is legal.
        assert!(
            subtype_of(&crate::nbe::env_global::Env::empty(), 0, &v(&pup), &v(&dog)).is_err(),
            "with no environment, entailment is undecidable and inclusion governs"
        );
    }
}
