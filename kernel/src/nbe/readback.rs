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
use crate::nbe::term::{CoField, Exp, Name, Observation, Patt, Summand};
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
        Val::Sort(n) => Exp::Sort(*n),
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

        // Codata (D11, Phase 9b-i)
        // Types can be read back safely — observation type expressions
        // terminate under evaluation like any other type expression.
        Val::Codata(observations, rho) => Exp::Codata(
            observations
                .iter()
                .map(|(name, typ)| {
                    let v = crate::nbe::eval::eval(typ, rho)?;
                    Ok(Observation {
                        name: name.clone(),
                        typ: try_readback_val(level, &v)?,
                    })
                })
                .collect::<Result<Vec<_>, EvalError>>()?,
        ),
        // Corecord values use a *conservative* readback: emit the
        // original syntactic field bodies without evaluating them.
        // Evaluating could diverge for streams (tail -> next corecord
        // -> tail -> ...). Under this scheme, two corecords are
        // definitionally equal only if their field bodies are
        // syntactically identical — sound but incomplete. See D11 §3.
        Val::CoRecord(fields, _rho) => Exp::CoRecord(
            fields
                .iter()
                .map(|(name, body)| CoField {
                    name: name.clone(),
                    body: body.clone(),
                })
                .collect(),
        ),

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
        // The `Exp::InductiveType` args slot carries `params ++ indices`,
        // split on the decoder side by `decl.params.len()` (D48 Phase B).
        // For non-indexed declarations (`decl.indices` empty), this is
        // equivalent to the pre-D48 behaviour.
        Val::InductiveType {
            decl,
            params,
            indices,
        } => Exp::InductiveType(
            decl.clone(),
            params
                .iter()
                .chain(indices.iter())
                .map(|p| try_readback_val(level, p))
                .collect::<Result<Vec<_>, EvalError>>()?,
        ),
        // Parameterised codata types (D19 §8, self-referential codata).
        Val::CodataType { decl, params } => Exp::CodataType(
            decl.clone(),
            params
                .iter()
                .map(|p| try_readback_val(level, p))
                .collect::<Result<Vec<_>, EvalError>>()?,
        ),
        Val::InductiveVal {
            decl,
            ctor_name,
            args,
        } => Exp::InductiveCtor(
            decl.clone(),
            ctor_name.clone(),
            args.iter()
                .map(|a| try_readback_val(level, a))
                .collect::<Result<Vec<_>, EvalError>>()?,
        ),

        // eigenius#71 / D49 — literals round-trip as themselves.
        Val::LitString(s) => Exp::LitString(s.clone()),
        Val::LitInt(n) => Exp::LitInt(*n),
        Val::LitFloat(f) => Exp::LitFloat(*f),

        // Sized types (Phase 11b step 14, D19 §8).
        Val::SizeSort => Exp::SizeSort,
        Val::SizeSucc(s) => Exp::SizeSucc(Box::new(try_readback_val(level, s)?)),
        Val::SizeInf => Exp::SizeInf,
        Val::SizedPi(upper, g) => {
            let gen = gen_val(level);
            Exp::SizedPi {
                patt: gen_patt(level),
                upper: Box::new(try_readback_val(level, upper)?),
                body: Box::new(try_readback_val(level + 1, &g.apply(gen)?)?),
            }
        }
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

        // Codata (D11, Phase 9b-i)
        Neut::Observe(k, obs) => Exp::Observe(Box::new(try_readback_neut(level, k)?), obs.clone()),

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
            decl: decl.clone(),
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
        assert_eq!(readback_val(0, &Val::Sort(1)), Exp::Sort(1));
    }

    #[test]
    fn readback_one() {
        assert_eq!(readback_val(0, &Val::One), Exp::One);
    }

    #[test]
    fn readback_pair() {
        let v = Val::Pair(Box::new(Val::Unit), Box::new(Val::Sort(1)));
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

        let v3 = Val::Sort(1);
        assert_ne!(readback_val(0, &v1), readback_val(0, &v3));
    }

    // --- Codata readback tests (D11, Phase 9b-i) ---

    #[test]
    fn readback_codata_type() {
        let v = Val::Codata(
            vec![
                ("head".to_string(), Exp::One),
                ("tail".to_string(), Exp::One),
            ],
            Rho::Nil,
        );
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::Codata(_)));
        if let Exp::Codata(obs) = e {
            assert_eq!(obs.len(), 2);
            assert_eq!(obs[0].name, "head");
            assert_eq!(obs[1].name, "tail");
        }
    }

    #[test]
    fn readback_corecord_conservative() {
        // Conservative readback: body exprs are emitted as-is,
        // without evaluating. This avoids divergence on stream
        // corecords.
        let v = Val::CoRecord(
            vec![
                ("head".to_string(), Exp::Unit),
                ("tail".to_string(), Exp::Var("self".to_string())),
            ],
            Rho::Nil,
        );
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::CoRecord(_)));
        if let Exp::CoRecord(fields) = e {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].body, Exp::Unit);
            assert_eq!(fields[1].body, Exp::Var("self".to_string()));
        }
    }

    // --- Map/Reduce readback tests (Phase 11a) ---

    #[test]
    fn readback_empty_list() {
        let v = Val::List(vec![]);
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::Con(ref c, _) if c == "nil"));
    }

    #[test]
    fn readback_two_element_list() {
        let v = Val::List(vec![Val::Unit, Val::Sort(1)]);
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
            Box::new(Val::Sort(1)), // placeholder accumulator
            Box::new(Neut::Gen(0, "xs".to_string())),
        ));
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::Reduce(_, _, _)));
    }

    #[test]
    fn readback_observe_neutral() {
        // (neut).obs → Observe(neut_readback, obs)
        let v = Val::Nt(Neut::Observe(
            Box::new(Neut::Gen(0, "x".to_string())),
            "head".to_string(),
        ));
        let e = readback_val(0, &v);
        assert!(matches!(e, Exp::Observe(_, ref s) if s == "head"));
    }
}
