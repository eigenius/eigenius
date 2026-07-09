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

//! Codata checking support: observation resolution (D11) and the
//! guardedness check (D11 §3). Split from `check.rs`.

use super::{CheckCtx, CheckError};
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::term::{Exp, Patt};
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::well_known as wk;
use std::sync::Arc;

/// Rehydrate a possibly-stub `Arc<CodataDecl>` to the full
/// declaration with populated observations.
///
/// The ground resolver emits self-references inside observation types
/// as `Exp::CodataType(self_ref_stub, args)` where `self_ref_stub` is
/// an `Arc<CodataDecl>` with empty observations — it's the
/// initial-Arc trick that mirrors `resolve_inductive_type`'s pattern.
/// That works for inductive types because constructor applications
/// always carry the full decl at the use site, but corecord values
/// and observations don't carry a decl reference in their Exp — the
/// decl comes from the inferred/expected type, which may be the
/// stub.
///
/// This helper walks the current layer looking for a `CodataType`
/// resource whose short name matches `stub.name` and re-resolves it
/// to a full decl. Costly per call; a future optimisation could
/// memoise this in `CheckCtx` next to `type_cache`.
pub(super) fn resolve_full_codata_decl(
    ctx: &CheckCtx,
    stub: &Arc<crate::nbe::term::CodataDecl>,
) -> Result<Arc<crate::nbe::term::CodataDecl>, CheckError> {
    if !stub.observations.is_empty() {
        return Ok(stub.clone());
    }
    let layer = ctx.layer.as_ref().ok_or_else(|| {
        format!(
            "cannot rehydrate stub codata decl `{}` — no layer in check context",
            stub.name
        )
    })?;
    let short_name_iri =
        Iri::parse(crate::ontology::well_known::SHORT_NAME).expect("well-known IRI");
    for (iri, resource) in layer.iter_all_resources() {
        if !resource
            .is_a()
            .iter()
            .any(|c| c.as_str() == wk::CODATA_TYPE)
        {
            continue;
        }
        if let Some(crate::ontology::resource::Value::String(sn)) = resource.get(&short_name_iri) {
            if sn == &stub.name {
                let v = ctx.hooks.resolve_class(&iri, layer)?;
                match v {
                    Val::CodataType { decl, .. } => return Ok(decl),
                    Val::Codata(_, _) => {
                        return Err(CheckError::IllFormed(format!(
                            "codata `{}` resolved to the non-parameterised `Val::Codata` \
                             form — cannot be used as a stub target",
                            stub.name
                        )));
                    }
                    _ => {
                        return Err(CheckError::IllFormed(format!(
                            "codata `{}` resolved to an unexpected Val form",
                            stub.name
                        )));
                    }
                }
            }
        }
    }
    Err(CheckError::IllFormed(format!(
        "cannot find codata decl with short_name `{}` in the layer chain",
        stub.name
    )))
}

/// Look up an observation by name on an applied codata type, returning
/// the observation's type evaluated in an environment that binds the
/// codata's parameters to the applied argument values.
///
/// This is the parameterised-codata analogue of the projection that
/// `Val::Codata(observations, rho)` does inline — the decl carries
/// the observation list, the `params` vector supplies the concrete
/// argument values, and self-references inside observation types
/// unify by name via `CodataDecl::PartialEq`.
pub fn lookup_codata_observation(
    decl: &Arc<crate::nbe::term::CodataDecl>,
    params: &[Val],
    obs_name: &str,
    level: usize,
) -> Result<Val, CheckError> {
    let obs = decl
        .observations
        .iter()
        .find(|o| o.name == obs_name)
        .ok_or_else(|| {
            format!(
                "observation '{}' not found in codata type '{}'",
                obs_name, decl.name
            )
        })?;
    let mut env = Rho::Nil;
    for ((patt, _), val) in decl.params.iter().zip(params.iter()) {
        env = env.extend(patt.clone(), val.clone());
    }
    let _ = level; // reserved for richer diagnostics in future
    eval(&obs.typ, &env).map_err(CheckError::from)
}

/// Collect the variable names bound by a pattern.
pub(super) fn collect_pattern_names<'a>(p: &'a Patt, out: &mut std::collections::HashSet<&'a str>) {
    match p {
        Patt::Var(n) => {
            out.insert(n.as_str());
        }
        Patt::Pair(p1, p2) => {
            collect_pattern_names(p1, out);
            collect_pattern_names(p2, out);
        }
        Patt::Unit => {}
    }
}

/// If `exp` reduces syntactically to a forbidden variable through a
/// chain of observations and projections, return that variable's name.
/// Used by the guardedness check to detect unguarded corecursive
/// references at the head of an `Observe`.
///
/// This intentionally stops at `App` / `Lam` / `CoRecord` / constructor
/// boundaries — crossing any of those makes the reference guarded.
fn has_forbidden_head<'a>(
    exp: &'a Exp,
    forbidden: &std::collections::HashSet<&str>,
) -> Option<&'a str> {
    match exp {
        Exp::Var(x) if forbidden.contains(x.as_str()) => Some(x.as_str()),
        Exp::Observe(inner, _) => has_forbidden_head(inner, forbidden),
        Exp::Fst(inner) | Exp::Snd(inner) => has_forbidden_head(inner, forbidden),
        _ => None,
    }
}

/// Syntactic guardedness check for corecursive definitions (D11 §3).
///
/// A corecord definition `letrec x = ...` is guarded iff `x` (or any
/// mutually-bound name) never appears at the *head* of an
/// `Observe` expression within a field body — because doing so would
/// trigger immediate unfolding of the same corecord at the same layer,
/// producing no progress.
///
/// The check is syntactic and Agda-style. Productive patterns covered:
/// - `letrec nats(n) = corecord { head = n; tail = nats(n+1) }` — the
///   corecursive call is under `App`, which breaks the observation
///   chain; each observation produces a fresh corecord.
/// - `letrec ones = corecord { head = 1; tail = ones }` — a naked
///   reference at a field body is fine; observing `ones.tail.tail...`
///   re-returns the corecord value each time, with finite cost per
///   step.
///
/// Rejected:
/// - `letrec bad = corecord { head = bad.head; tail = ... }` — observing
///   `bad.head` requires evaluating `bad.head`, infinite loop.
///
/// Conservative approximation: syntactic guardedness cannot catch
/// cases where the loop goes through a function call (e.g. `broken(n).head`
/// where `broken` returns a corecord whose head body is
/// `broken(n).head`). Sized types would close that gap — out of scope
/// for v1. See D11 §3.4 and [eigenius#16][1].
///
/// [1]: https://github.com/eigenius/eigenius/issues/16
pub fn check_guarded(
    exp: &Exp,
    forbidden: &std::collections::HashSet<&str>,
) -> Result<(), CheckError> {
    match exp {
        Exp::Observe(inner, obs) => {
            if let Some(name) = has_forbidden_head(inner, forbidden) {
                return Err(CheckError::IllFormed(format!(
                    "unguarded corecursive reference: '{name}' is observed at field '{obs}' \
                     inside its own definition — this would loop at evaluation time. \
                     Put the recursive call under a function application or inside \
                     another constructor so that each observation makes progress."
                )));
            }
            check_guarded(inner, forbidden)
        }

        // Sub-expressions that need recursive checking.
        Exp::Lam(_, e) => check_guarded(e, forbidden),
        Exp::Ann(e, t) => {
            check_guarded(e, forbidden)?;
            check_guarded(t, forbidden)
        }
        Exp::App(e1, e2) => {
            check_guarded(e1, forbidden)?;
            check_guarded(e2, forbidden)
        }
        Exp::Pair(e1, e2) => {
            check_guarded(e1, forbidden)?;
            check_guarded(e2, forbidden)
        }
        Exp::Con(_, e) => check_guarded(e, forbidden),
        Exp::Fst(e) | Exp::Snd(e) => check_guarded(e, forbidden),
        Exp::Pi(_, a, b) | Exp::Sig(_, a, b) => {
            check_guarded(a, forbidden)?;
            check_guarded(b, forbidden)
        }
        Exp::Arrow(a, b) | Exp::Times(a, b) => {
            check_guarded(a, forbidden)?;
            check_guarded(b, forbidden)
        }
        Exp::Data(summands) => {
            for s in summands {
                check_guarded(&s.typ, forbidden)?;
            }
            Ok(())
        }
        Exp::Case(branches) => {
            for b in branches {
                check_guarded(&b.body, forbidden)?;
            }
            Ok(())
        }
        Exp::Dec(_, e) => check_guarded(e, forbidden),
        Exp::Id(a, x, y) => {
            check_guarded(a, forbidden)?;
            check_guarded(x, forbidden)?;
            check_guarded(y, forbidden)
        }
        Exp::Refl(a) => check_guarded(a, forbidden),
        Exp::IdJ(args) => {
            for a in args.iter() {
                check_guarded(a, forbidden)?;
            }
            Ok(())
        }
        Exp::NativeDecide(c, v) => {
            if let crate::nbe::term::Constraint::Institution { args, .. } = c {
                for a in args {
                    check_guarded(a, forbidden)?;
                }
            }
            check_guarded(v, forbidden)
        }
        Exp::InstitutionInvoke { source, .. } => check_guarded(source, forbidden),
        Exp::DecEq(a, x, y) => {
            check_guarded(a, forbidden)?;
            check_guarded(x, forbidden)?;
            check_guarded(y, forbidden)
        }
        Exp::PropAccess(e, _) => check_guarded(e, forbidden),
        Exp::Template(_, refs) => {
            for (_, t) in refs {
                check_guarded(t, forbidden)?;
            }
            Ok(())
        }
        Exp::Construct(_, fields) => {
            for (_, e) in fields {
                check_guarded(e, forbidden)?;
            }
            Ok(())
        }

        // Codata forms
        Exp::Codata(observations) => {
            for o in observations {
                check_guarded(&o.typ, forbidden)?;
            }
            Ok(())
        }
        // Parameterised codata application — recurse into its
        // argument expressions only; the codata decl's observations
        // are type-level and already validated at decl-site.
        Exp::CodataType(_, args) => {
            for a in args {
                check_guarded(a, forbidden)?;
            }
            Ok(())
        }
        Exp::CoRecord(fields) => {
            for f in fields {
                check_guarded(&f.body, forbidden)?;
            }
            Ok(())
        }

        // Map/Reduce (Phase 11a)
        Exp::Map(f, coll) => {
            check_guarded(f, forbidden)?;
            check_guarded(coll, forbidden)
        }
        Exp::Reduce(f, init, coll) => {
            check_guarded(f, forbidden)?;
            check_guarded(init, forbidden)?;
            check_guarded(coll, forbidden)
        }

        // Inductive types (Phase 11b, D19): walk parameter / argument /
        // motive / minor / major sub-expressions structurally. The
        // `InductiveDecl` itself is treated as a closed declaration —
        // its constructor types are not visited here.
        Exp::Inductive(_) => Ok(()),
        Exp::InductiveType(_, params) => {
            for p in params {
                check_guarded(p, forbidden)?;
            }
            Ok(())
        }
        Exp::InductiveCtor(_, _, args) => {
            for a in args {
                check_guarded(a, forbidden)?;
            }
            Ok(())
        }
        Exp::InductiveRec {
            motive,
            minors,
            major,
            ..
        } => {
            check_guarded(motive, forbidden)?;
            for m in minors {
                check_guarded(m, forbidden)?;
            }
            check_guarded(major, forbidden)
        }
        Exp::Match { scrutinee, arms } => {
            check_guarded(scrutinee, forbidden)?;
            for arm in arms {
                check_guarded(&arm.body, forbidden)?;
            }
            Ok(())
        }

        // Sized types (Phase 11b step 14): size primitives are
        // structurally simple — `SizeSucc` has one sub-expression,
        // `SizeSort` and `SizeInf` are leaves.
        Exp::SizeSucc(s) => check_guarded(s, forbidden),
        Exp::SizeSort | Exp::SizeInf => Ok(()),
        // SizedPi binder — recurse into upper and body. (The binder
        // doesn't shadow corecursive names from `forbidden` because
        // it binds a size, not a value of a codata type.)
        Exp::SizedPi { upper, body, .. } => {
            check_guarded(upper, forbidden)?;
            check_guarded(body, forbidden)
        }

        // Leaves — no sub-expressions to check.
        Exp::Var(_)
        | Exp::Sort(1)
        | Exp::Sort(_)
        | Exp::One
        | Exp::Unit
        | Exp::EigonClass(_)
        | Exp::EigonAxiom(_)
        | Exp::EigonPrimitive(_)
        | Exp::EigonResource(_)
        | Exp::LitString(_)
        | Exp::LitInt(_)
        | Exp::LitFloat(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::nbe::check::testutil::*;
    use crate::nbe::check::*;

    // --- Codata tests (D11, Phase 9b-i) ---

    fn pair_codata_type() -> Exp {
        // codata { fst : 1; snd : 1 }
        Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "fst".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "snd".to_string(),
                typ: Exp::One,
            },
        ])
    }

    fn unit_pair_corecord() -> Exp {
        // corecord { fst = (); snd = () }
        Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "fst".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "snd".to_string(),
                body: Exp::Unit,
            },
        ])
    }

    #[test]
    fn codata_type_is_a_type() {
        check_type(&mut ctx(), &pair_codata_type()).unwrap();
        check(&mut ctx(), &pair_codata_type(), &Val::Sort(1)).unwrap();
    }

    #[test]
    fn codata_duplicate_observation_rejected() {
        let bad = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "x".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "x".to_string(),
                typ: Exp::One,
            },
        ]);
        assert!(check_type(&mut ctx(), &bad).is_err());
    }

    #[test]
    fn corecord_checks_against_codata_type() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        check(&mut ctx(), &unit_pair_corecord(), &codata_typ)?;
        Ok(())
    }

    #[test]
    fn corecord_mismatched_fields_rejected() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        // Missing 'snd'
        let bad = Exp::CoRecord(vec![crate::nbe::term::CoField {
            name: "fst".to_string(),
            body: Exp::Unit,
        }]);
        assert!(check(&mut ctx(), &bad, &codata_typ).is_err());
        Ok(())
    }

    #[test]
    fn corecord_wrong_field_order_rejected() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        // Fields in wrong order
        let bad = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "snd".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "fst".to_string(),
                body: Exp::Unit,
            },
        ]);
        assert!(check(&mut ctx(), &bad, &codata_typ).is_err());
        Ok(())
    }

    #[test]
    fn observation_evaluates_to_field_body() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // corecord { fst = (); snd = () }.fst → ()
        let observe = Exp::Observe(Box::new(unit_pair_corecord()), "fst".to_string());
        let result = eval(&observe, &Rho::Nil)?;
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    #[test]
    fn observation_unknown_field_returns_err() {
        // vobserve now returns Err for unknown fields (issue #19)
        use crate::nbe::eval::eval;
        let observe = Exp::Observe(Box::new(unit_pair_corecord()), "missing".to_string());
        let result = eval(&observe, &Rho::Nil);
        assert!(result.is_err());
    }

    #[test]
    fn observation_type_inference() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::env::up_gamma;
        use crate::nbe::eval::eval;
        // Given x : codata { fst : 1; snd : 1 }, infer x.fst : 1
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil)?;
        let gen = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let gamma = up_gamma(&vec![], &Patt::Var("x".to_string()), &codata_typ, &gen)?;
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), gen);
        let mut c = CheckCtx::new(rho, gamma);
        let observe = Exp::Observe(Box::new(Exp::Var("x".to_string())), "fst".to_string());
        let t = check_infer(&mut c, &observe)?;
        assert!(matches!(t, Val::One));
        Ok(())
    }

    #[test]
    fn observation_on_neutral_blocks() -> Result<(), Box<dyn std::error::Error>> {
        use crate::nbe::eval::eval;
        // let x = <neutral>; x.fst should produce a Neut::Observe
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), neut);
        let observe = Exp::Observe(Box::new(Exp::Var("x".to_string())), "fst".to_string());
        let result = eval(&observe, &rho)?;
        assert!(matches!(
            result,
            Val::Nt(crate::nbe::val::Neut::Observe(_, _))
        ));
        Ok(())
    }

    #[test]
    fn stream_two_observations_advance() -> Result<(), Box<dyn std::error::Error>> {
        // letrec nats : Nat → codata { head : Nat; tail : codata { head : Nat; tail : ... } } = λn. corecord { head = n; tail = nats(n+1) }
        //
        // Simplified for testing: use Unit as the element type and
        // represent Nat as a chain of Con values. Observing head twice
        // should advance the stream.
        //
        // Stream type (same at every step, so we use a self-referential
        // type by using EigonPrimitive::Integer as a stand-in — type
        // checking is not the focus here; we just want to verify
        // evaluation and observation plumbing).
        use crate::nbe::eval::eval;
        use crate::nbe::term::PrimitiveType;

        // Build: λn. corecord { head = n; tail = f(n) }
        // where f is a free variable we'll instantiate via Rho.
        //
        // Instead of full recursion, verify two cases:
        //   corecord { head = (), tail = corecord { head = (), tail = <bottom> } }
        // and confirm that .tail.head returns ().
        let inner = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::EigonPrimitive(PrimitiveType::Integer), // placeholder "bottom"
            },
        ]);
        let outer = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: inner,
            },
        ]);
        // outer.tail.head → ()
        let expr = Exp::Observe(
            Box::new(Exp::Observe(Box::new(outer), "tail".to_string())),
            "head".to_string(),
        );
        let result = eval(&expr, &Rho::Nil)?;
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    #[test]
    fn recursive_stream_via_letrec() -> Result<(), Box<dyn std::error::Error>> {
        // letrec nats : codata { head : 1; tail : codata {...} } = corecord { head = (); tail = nats }
        // Observing nats.tail.tail.head should give ().
        use crate::nbe::eval::eval;

        // Self-referential codata type is tricky without proper type
        // theory; sidestep by using a simpler fixpoint test: the
        // evaluator should handle the corecursive reference via
        // Rho::UpDec.
        let corecord = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Var("nats".to_string()),
            },
        ]);
        // We don't need the type to check — just evaluate.
        let letrec = Exp::Dec(
            Decl::Drec(
                Patt::Var("nats".to_string()),
                Box::new(Exp::One), // placeholder type (not checked here)
                Box::new(corecord),
            ),
            // nats.tail.tail.head
            Box::new(Exp::Observe(
                Box::new(Exp::Observe(
                    Box::new(Exp::Observe(
                        Box::new(Exp::Var("nats".to_string())),
                        "tail".to_string(),
                    )),
                    "tail".to_string(),
                )),
                "head".to_string(),
            )),
        );
        let result = eval(&letrec, &Rho::Nil)?;
        assert!(matches!(result, Val::Unit));
        Ok(())
    }

    // --- Guardedness tests (D11 §3, Phase 9b-i) ---

    fn forbidden(names: &[&'static str]) -> std::collections::HashSet<&'static str> {
        names.iter().copied().collect()
    }

    #[test]
    fn guardedness_accepts_naked_corecursive_field_body() {
        // letrec ones = corecord { head = (); tail = ones }
        // The `tail` body is a naked reference to the corecursive name.
        // This is productive: observing tail returns the corecord,
        // subsequent observations are fresh steps.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Var("ones".to_string()),
            },
        ]);
        check_guarded(&body, &forbidden(&["ones"])).unwrap();
    }

    #[test]
    fn guardedness_accepts_corecursive_call_under_app() {
        // corecord { head = n; tail = nats(n+1) }
        // tail body is App(Var(nats), ...) — call under function
        // application is productive.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Var("n".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::App(Box::new(Exp::Var("nats".to_string())), Box::new(Exp::Unit)),
            },
        ]);
        check_guarded(&body, &forbidden(&["nats"])).unwrap();
    }

    #[test]
    fn guardedness_rejects_bare_corecursive_observation() {
        // corecord { head = bad.head; tail = ... }
        // Observing a corecord's own field inside its own body loops.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(Box::new(Exp::Var("bad".to_string())), "head".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        let err = check_guarded(&body, &forbidden(&["bad"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unguarded"));
        assert!(err.contains("bad"));
    }

    #[test]
    fn guardedness_rejects_chained_corecursive_observation() {
        // bad.tail.head — chain of observations on corecursive name
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(
                    Box::new(Exp::Observe(
                        Box::new(Exp::Var("bad".to_string())),
                        "tail".to_string(),
                    )),
                    "head".to_string(),
                ),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        assert!(check_guarded(&body, &forbidden(&["bad"])).is_err());
    }

    #[test]
    fn guardedness_accepts_non_corecursive_letrec() {
        // letrec f = λx. f(x) — data recursion (not codata), no corecord.
        // Guardedness is a no-op here (data termination is a separate
        // concern; EigenTT doesn't check it either).
        let body = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::App(
                Box::new(Exp::Var("f".to_string())),
                Box::new(Exp::Var("x".to_string())),
            )),
        );
        check_guarded(&body, &forbidden(&["f"])).unwrap();
    }

    #[test]
    fn guardedness_accepts_observation_of_non_corecursive_ref() {
        // corecord { head = other.head; tail = () }
        // `other` is not a corecursive name here — observing it is fine.
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(Box::new(Exp::Var("other".to_string())), "head".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        // Only `self` is forbidden; `other` is free.
        check_guarded(&body, &forbidden(&["self"])).unwrap();
    }

    #[test]
    fn guardedness_in_check_decl_rejects_bad_corecord() {
        // letrec bad : codata { head : 1; tail : 1 } = corecord { head = bad.head; tail = () }
        // The Drec pathway in check_decl now invokes check_guarded; this
        // should surface the unguarded reference.
        let codata_typ = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "head".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "tail".to_string(),
                typ: Exp::One,
            },
        ]);
        let body = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Observe(Box::new(Exp::Var("bad".to_string())), "head".to_string()),
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Unit,
            },
        ]);
        let d = Decl::Drec(
            Patt::Var("bad".to_string()),
            Box::new(codata_typ),
            Box::new(body),
        );
        let err = check_decl(&mut ctx(), &d).unwrap_err().to_string();
        assert!(
            err.contains("unguarded"),
            "expected unguarded error, got: {err}"
        );
    }

    // --- Productivity via sized codata (Phase 11b step 15f) ---
    //
    // A sized codata type's observations use `SizedPi` for recursive
    // positions. A field body that inhabits such an observation type
    // is typically `λ j. body` — the new `Lam`-vs-`SizedPi` check arm
    // opens the size binder, registers `j < upper` in the TSO, and
    // checks the body against the codomain. Recursive references to
    // the corecord are forced (by type) to produce results at sizes
    // strictly below the outer size, yielding productivity.

    #[test]
    fn lam_against_sized_pi_at_inf() {
        // `λ j. Unit` : `{j < ∞}. One`. Trivial sanity.
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::SizeInf),
            body: Box::new(Exp::One),
        };
        let ty = eval(&ty_exp, &c.rho).expect("eval ty");
        let lam = Exp::Lam(Patt::Var("j".to_string()), Box::new(Exp::Unit));
        check(&mut c, &lam, &ty).expect("λ j. Unit : {j < ∞}. 1");
    }

    #[test]
    fn lam_against_sized_pi_at_rigid() {
        // With `i : SizeSort`, `λ j. Unit` : `{j < i}. One`.
        let (mut c, _) = ctx_with_size_var("i");
        let ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        let ty = eval(&ty_exp, &c.rho).expect("eval ty");
        let lam = Exp::Lam(Patt::Var("j".to_string()), Box::new(Exp::Unit));
        check(&mut c, &lam, &ty).expect("λ j. Unit : {j < i}. 1");
    }

    #[test]
    fn lam_body_uses_bounded_size_in_application() {
        // With `i : SizeSort` and `f : Π k:SizeSort. One`,
        // `λ j. f(j)` checks against `{j < i}. One`.
        // Exercises: binder opens j, app of f to j gets the size
        // hypothesis from TSO (though trivially — we're going through
        // Pi, not SizedPi, so no strict bound needed).
        let (c, _i_val) = ctx_with_size_var("i");

        let f_ty_exp = Exp::Pi(
            Patt::Var("k".to_string()),
            Box::new(Exp::SizeSort),
            Box::new(Exp::One),
        );
        let f_ty = eval(&f_ty_exp, &c.rho).expect("eval f_ty");
        let f_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("f".to_string()), f_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("f".to_string()), &f_ty, &f_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        let target_ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        let target_ty = eval(&target_ty_exp, &c2.rho).expect("eval target");
        let lam = Exp::Lam(
            Patt::Var("j".to_string()),
            Box::new(Exp::App(
                Box::new(Exp::Var("f".to_string())),
                Box::new(Exp::Var("j".to_string())),
            )),
        );
        check(&mut c2, &lam, &target_ty).expect("λ j. f(j) : {j < i}. 1");
    }

    #[test]
    fn lam_body_invokes_sized_function_productively() {
        // The core productivity-by-typing scenario.
        //
        // Given `i : SizeSort` and a size-polymorphic producer
        // `g : Π k:SizeSort. SizedStream(k, 1)`, the expression
        // `λ j. g(j)` checks against `{j < i}. SizedStream(j, 1)`.
        //
        // This is exactly the shape of a sized corecord's `tail`
        // field when the corecord is defined by a size-polymorphic
        // function of itself: `tail = λ j. self(j)`. Type-checking
        // this field IS the productivity argument — the body must
        // produce a value at size `j`, which (since `j < i`) is
        // strictly smaller than the outer size.
        let decl = sized_stream_decl();
        let (c, _) = ctx_with_size_var("i");

        let stream_k = Exp::InductiveType(decl.clone(), vec![Exp::Var("k".to_string()), Exp::One]);
        let g_ty_exp = Exp::Pi(
            Patt::Var("k".to_string()),
            Box::new(Exp::SizeSort),
            Box::new(stream_k),
        );
        let g_ty = eval(&g_ty_exp, &c.rho).expect("eval g_ty");
        let g_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("g".to_string()), g_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("g".to_string()), &g_ty, &g_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        let stream_j = Exp::InductiveType(decl.clone(), vec![Exp::Var("j".to_string()), Exp::One]);
        let target_ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(stream_j),
        };
        let target_ty = eval(&target_ty_exp, &c2.rho).expect("eval target");

        let lam = Exp::Lam(
            Patt::Var("j".to_string()),
            Box::new(Exp::App(
                Box::new(Exp::Var("g".to_string())),
                Box::new(Exp::Var("j".to_string())),
            )),
        );
        check(&mut c2, &lam, &target_ty)
            .expect("λ j. g(j) : {j < i}. SizedStream(j, 1) — productive by typing");
    }

    #[test]
    fn non_productive_body_rejected_by_sized_type() {
        // Given `h : SizedStream(i, 1)` at the OUTER size i, the body
        // `λ j. h` checks at the expected type `{j < i}. SizedStream(j, 1)`
        // iff `SizedStream(i, 1) <: SizedStream(j, 1)`, i.e. `i ≤ j`.
        // But TSO has `j < i`, not `i ≤ j`, so this must be rejected —
        // capturing the non-productive "reuse outer value at smaller
        // size" bug.
        let decl = sized_stream_decl();
        let (c, _) = ctx_with_size_var("i");

        let stream_i = Exp::InductiveType(decl.clone(), vec![Exp::Var("i".to_string()), Exp::One]);
        let h_ty = eval(&stream_i, &c.rho).expect("eval h_ty");
        let h_val = gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("h".to_string()), h_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("h".to_string()), &h_ty, &h_val).unwrap();
        let mut c2 = CheckCtx::new(rho2, gamma2);

        let stream_j = Exp::InductiveType(decl.clone(), vec![Exp::Var("j".to_string()), Exp::One]);
        let target_ty_exp = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(stream_j),
        };
        let target_ty = eval(&target_ty_exp, &c2.rho).expect("eval target");

        // `λ j. h` — h has type SizedStream(i,1). j is bounded below i.
        // The body would need to be SizedStream(j,1), but h is at i.
        let lam = Exp::Lam(
            Patt::Var("j".to_string()),
            Box::new(Exp::Var("h".to_string())),
        );
        assert!(
            check(&mut c2, &lam, &target_ty).is_err(),
            "λ j. h must not check against {{j < i}}. SizedStream(j, 1) — h is at outer size i"
        );
    }

    #[test]
    fn sized_codata_type_formation() {
        // With `i : SizeSort`, check that the codata type
        //   codata { head : One, tail : {j < i}. One }
        // is a valid type. This is the minimal sized codata shape.
        let (mut c, _) = ctx_with_size_var("i");
        let tail_ty = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        let codata = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "head".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "tail".to_string(),
                typ: tail_ty,
            },
        ]);
        check_type(&mut c, &codata).expect("sized codata is a valid type");
    }

    #[test]
    fn sized_corecord_type_checks_against_sized_codata() {
        // End-to-end: construct a corecord that inhabits a sized
        // codata type. Uses the Lam-vs-SizedPi arm for the tail
        // field.
        //
        // Type:  codata { head : One, tail : {j < i}. One }
        // Value: corecord { head = Unit; tail = λ j. Unit }
        let (mut c, _) = ctx_with_size_var("i");
        let tail_obs_ty = Exp::SizedPi {
            patt: Patt::Var("j".to_string()),
            upper: Box::new(Exp::Var("i".to_string())),
            body: Box::new(Exp::One),
        };
        let codata = Exp::Codata(vec![
            crate::nbe::term::Observation {
                name: "head".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Observation {
                name: "tail".to_string(),
                typ: tail_obs_ty,
            },
        ]);
        let ty = eval(&codata, &c.rho).expect("eval codata");
        let corecord = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "head".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "tail".to_string(),
                body: Exp::Lam(Patt::Var("j".to_string()), Box::new(Exp::Unit)),
            },
        ]);
        check(&mut c, &corecord, &ty).expect("sized corecord inhabits sized codata");
    }
}
