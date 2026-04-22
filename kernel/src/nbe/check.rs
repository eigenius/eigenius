//! Mini-TT bidirectional type checker.
//!
//! Ported from `Main.hs` lines 289-378 in the Mini-TT reference.
//! Uses NbE (eval + readback) for type equality checking.

use crate::nbe::env::{gen_val, lookup_gamma, up_gamma, Gamma, Rho};
use crate::nbe::eval::eval;
use crate::nbe::readback::readback_val;
use crate::nbe::term::{Decl, Exp, Patt};
use crate::nbe::val::{Clos, Val};

/// Check that a declaration is well-typed, returning the extended type context.
///
/// Port of `checkD` from the reference.
pub fn check_decl(rho: &Rho, gamma: &Gamma, decl: &Decl) -> Result<Gamma, String> {
    match decl {
        Decl::Def(patt, typ, body) => {
            // Check that the type is well-formed
            check_type(rho, gamma, typ)?;
            let t = eval(typ, rho);
            // Check that the body has the declared type
            check(rho, gamma, body, &t)?;
            // Extend the type context
            up_gamma(gamma, patt, &t, &eval(body, rho))
        }
        Decl::Drec(patt, typ, body) => {
            // Check that the type is well-formed
            check_type(rho, gamma, typ)?;
            let t = eval(typ, rho);
            let gen = gen_val(rho);
            // Extend context with the recursive variable
            let gamma1 = up_gamma(gamma, patt, &t, &gen)?;
            // Check body under extended context
            let rho1 = rho.clone().extend(patt.clone(), gen);
            check(&rho1, &gamma1, body, &t)?;
            // Guardedness: if the recursive body constructs a corecord,
            // verify every corecursive reference appears under a
            // constructor/lambda/app — not at the bare head of an
            // observation. D11 §3 "productivity."
            let mut forbidden: std::collections::HashSet<&str> = std::collections::HashSet::new();
            collect_pattern_names(patt, &mut forbidden);
            check_guarded(body, &forbidden)?;
            // Re-evaluate with the recursive binding
            let v = eval(body, &Rho::UpDec(Box::new(rho.clone()), decl.clone()));
            up_gamma(gamma, patt, &t, &v)
        }
    }
}

/// Check that an expression is a well-formed type.
///
/// Port of `checkT` from the reference.
pub fn check_type(rho: &Rho, gamma: &Gamma, exp: &Exp) -> Result<(), String> {
    match exp {
        Exp::Pi(p, a, b) | Exp::Sig(p, a, b) => {
            check_type(rho, gamma, a)?;
            let gen = gen_val(rho);
            let gamma1 = up_gamma(gamma, p, &eval(a, rho), &gen)?;
            let rho1 = rho.clone().extend(p.clone(), gen);
            check_type(&rho1, &gamma1, b)
        }
        Exp::Set | Exp::One | Exp::Type(_) => Ok(()),
        // Id(A, x, y) is a type if A is a type and x, y : A
        Exp::Id(a, x, y) => {
            check_type(rho, gamma, a)?;
            let a_val = eval(a, rho);
            check(rho, gamma, x, &a_val)?;
            check(rho, gamma, y, &a_val)
        }
        // Eigenius ground types are always valid types
        Exp::EigonClass(_) | Exp::EigonPrimitive(_) => Ok(()),

        // Codata type declaration: each observation's type must be a type.
        // Observation names must be distinct.
        Exp::Codata(observations) => {
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for obs in observations {
                if !seen.insert(obs.name.as_str()) {
                    return Err(format!(
                        "duplicate observation name in codata type: '{}'",
                        obs.name
                    ));
                }
                check_type(rho, gamma, &obs.typ)?;
            }
            Ok(())
        }

        a => check(rho, gamma, a, &Val::Set),
    }
}

/// Check that an expression has a given type (checking mode).
///
/// Port of `check` from the reference.
pub fn check(rho: &Rho, gamma: &Gamma, exp: &Exp, typ: &Val) -> Result<(), String> {
    match (exp, typ) {
        // Lambda against Pi type
        (Exp::Lam(p, e), Val::Pi(t, g)) => {
            let gen = gen_val(rho);
            let gamma1 = up_gamma(gamma, p, t, &gen)?;
            let rho1 = rho.clone().extend(p.clone(), gen.clone());
            check(&rho1, &gamma1, e, &g.apply(gen))
        }

        // Pair against Sigma type
        (Exp::Pair(e1, e2), Val::Sig(t, g)) => {
            check(rho, gamma, e1, t)?;
            check(rho, gamma, e2, &g.apply(eval(e1, rho)))
        }

        // Constructor against Sum type
        (Exp::Con(c, e), Val::Data(cases, rho1)) => {
            let a = cases
                .iter()
                .find(|(name, _)| name == c)
                .map(|(_, typ)| typ)
                .ok_or_else(|| format!("constructor {c} not in sum type"))?;
            check(rho, gamma, e, &eval(a, rho1))
        }

        // Case function against Pi from Sum to result
        (Exp::Case(branches), Val::Pi(domain, g)) if matches!(**domain, Val::Data(_, _)) => {
            let (cases, rho1) = match &**domain {
                Val::Data(cases, rho1) => (cases, rho1),
                _ => unreachable!(),
            };
            let branch_names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
            let case_names: Vec<&str> = cases.iter().map(|(n, _)| n.as_str()).collect();
            if branch_names != case_names {
                return Err(format!(
                    "case branches {:?} do not match sum type {:?}",
                    branch_names, case_names
                ));
            }
            for (branch, (c, a)) in branches.iter().zip(cases.iter()) {
                let a_val = eval(a, rho1);
                let g_c = Clos {
                    patt: Patt::Var("__case_arg".to_string()),
                    body: Exp::App(
                        Box::new(readback_val(rho.len(), &Val::Lam(g.clone()))),
                        Box::new(Exp::Con(
                            c.clone(),
                            Box::new(Exp::Var("__case_arg".to_string())),
                        )),
                    ),
                    env: rho.clone(),
                };
                check(rho, gamma, &branch.body, &Val::Pi(Box::new(a_val), g_c))?;
            }
            Ok(())
        }

        // Unit value against One type
        (Exp::Unit, Val::One) => Ok(()),

        // One against Set (One is a type)
        (Exp::One, Val::Set) => Ok(()),

        // Pi type against Set
        (Exp::Pi(p, a, b), Val::Set) | (Exp::Sig(p, a, b), Val::Set) => {
            check(rho, gamma, a, &Val::Set)?;
            let gen = gen_val(rho);
            let gamma1 = up_gamma(gamma, p, &eval(a, rho), &gen)?;
            let rho1 = rho.clone().extend(p.clone(), gen);
            check(&rho1, &gamma1, b, &Val::Set)
        }

        // Sum type against Set
        (Exp::Data(summands), Val::Set) => {
            for s in summands {
                check(rho, gamma, &s.typ, &Val::Set)?;
            }
            Ok(())
        }

        // Declaration
        (Exp::Dec(d, e), t) => {
            let gamma1 = check_decl(rho, gamma, d)?;
            check(&Rho::UpDec(Box::new(rho.clone()), d.clone()), &gamma1, e, t)
        }

        // refl(a) : Id(A, a, a) — check that x and y are both a
        (Exp::Refl(a), Val::Id(typ, x, y)) => {
            check(rho, gamma, a, typ)?;
            let a_val = eval(a, rho);
            eq_nf(rho.len(), x, &a_val)?;
            eq_nf(rho.len(), y, &a_val)
        }

        // Id(A, x, y) : Set
        (Exp::Id(a, x, y), Val::Set) => {
            check(rho, gamma, a, &Val::Set)?;
            let a_val = eval(a, rho);
            check(rho, gamma, x, &a_val)?;
            check(rho, gamma, y, &a_val)
        }

        // Type(n) : Type(n+1)
        (Exp::Type(n), Val::Type(m)) if *n + 1 == *m => Ok(()),
        // Type(n) : Set (Set is the top universe for backward compatibility)
        (Exp::Type(_), Val::Set) => Ok(()),
        // Set : Type(1)
        (Exp::Set, Val::Type(1)) => Ok(()),

        // Eigenius ground types against Set
        (Exp::EigonClass(_), Val::Set) | (Exp::EigonPrimitive(_), Val::Set) => Ok(()),
        // Eigenius ground types against any Type level
        (Exp::EigonClass(_), Val::Type(_)) | (Exp::EigonPrimitive(_), Val::Type(_)) => Ok(()),

        // Codata type formation: codata { ... } : Set
        (Exp::Codata(_), Val::Set) => check_type(rho, gamma, exp),
        (Exp::Codata(_), Val::Type(_)) => check_type(rho, gamma, exp),

        // Corecord against a codata type: each field's body must have
        // the corresponding observation's type, and every declared
        // observation must be covered.
        (Exp::CoRecord(fields), Val::Codata(observations, rho1)) => {
            let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            let obs_names: Vec<&str> = observations.iter().map(|(n, _)| n.as_str()).collect();
            if field_names != obs_names {
                return Err(format!(
                    "corecord fields {:?} do not match codata observations {:?}",
                    field_names, obs_names
                ));
            }
            for (field, (_, obs_typ)) in fields.iter().zip(observations.iter()) {
                let t = eval(obs_typ, rho1);
                check(rho, gamma, &field.body, &t)?;
            }
            Ok(())
        }

        // Fallthrough: infer type and compare
        (e, t) => {
            let t1 = check_infer(rho, gamma, e)?;
            eq_nf(rho.len(), t, &t1)
        }
    }
}

/// Infer the type of an expression (inference mode).
///
/// Port of `checkI` from the reference.
pub fn check_infer(rho: &Rho, gamma: &Gamma, exp: &Exp) -> Result<Val, String> {
    match exp {
        Exp::Var(x) => lookup_gamma(gamma, x),

        Exp::App(e1, e2) => {
            let t1 = check_infer(rho, gamma, e1)?;
            let (t, g) = ext_pi(&t1)?;
            check(rho, gamma, e2, &t)?;
            Ok(g.apply(eval(e2, rho)))
        }

        Exp::Fst(e) => {
            let t = check_infer(rho, gamma, e)?;
            let (t1, _) = ext_sig(&t)?;
            Ok(t1)
        }

        Exp::Snd(e) => {
            let t = check_infer(rho, gamma, e)?;
            let (_, g) = ext_sig(&t)?;
            Ok(g.apply(eval(e, rho).vfst()))
        }

        // Eigenius: property/observation access type inference.
        //
        // ESL's `.name` syntax unifies two operations:
        // - property access on resources / Sigma-typed values
        // - observation on codata-typed values
        // We dispatch on the inferred type of the target.
        Exp::PropAccess(e, prop) => {
            let t = check_infer(rho, gamma, e)?;
            let prop_name = prop.local_name();

            // Codata observation — same lookup that Exp::Observe does.
            if let Val::Codata(observations, rho1) = &t {
                for (name, typ) in observations {
                    if name == prop_name {
                        return Ok(eval(typ, rho1));
                    }
                }
                return Err(format!(
                    "observation '{}' not found in codata type {:?}",
                    prop_name,
                    readback_val(rho.len(), &t)
                ));
            }

            // Fall back to the existing Sigma / resource behaviour.
            find_sigma_field(&t, prop_name).ok_or_else(|| {
                format!(
                    "property '{}' not found in type {:?}",
                    prop,
                    readback_val(rho.len(), &t)
                )
            })
        }

        // Codata observation type inference: e.obs has type T where
        // `obs : T` appears in the inferred codata type of e.
        Exp::Observe(e, obs) => {
            let t = check_infer(rho, gamma, e)?;
            match &t {
                Val::Codata(observations, rho1) => {
                    for (name, typ) in observations {
                        if name == obs {
                            return Ok(eval(typ, rho1));
                        }
                    }
                    Err(format!(
                        "observation '{}' not found in codata type {:?}",
                        obs,
                        readback_val(rho.len(), &t)
                    ))
                }
                other => Err(format!(
                    "observation target is not a codata value: {:?}",
                    readback_val(rho.len(), other)
                )),
            }
        }

        e => Err(format!("cannot infer type of: {e:?}")),
    }
}

/// Check type equality by normalization.
///
/// Port of `eqNf` from the reference: normalize both sides
/// and compare syntactically.
pub fn eq_nf(level: usize, v1: &Val, v2: &Val) -> Result<(), String> {
    let e1 = readback_val(level, v1);
    let e2 = readback_val(level, v2);
    if e1 == e2 {
        Ok(())
    } else {
        Err(format!("type mismatch: {e1:?} ≠ {e2:?}"))
    }
}

/// Collect the variable names bound by a pattern.
fn collect_pattern_names<'a>(p: &'a Patt, out: &mut std::collections::HashSet<&'a str>) {
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
pub fn check_guarded(exp: &Exp, forbidden: &std::collections::HashSet<&str>) -> Result<(), String> {
    match exp {
        Exp::Observe(inner, obs) => {
            if let Some(name) = has_forbidden_head(inner, forbidden) {
                return Err(format!(
                    "unguarded corecursive reference: '{name}' is observed at field '{obs}' \
                     inside its own definition — this would loop at evaluation time. \
                     Put the recursive call under a function application or inside \
                     another constructor so that each observation makes progress."
                ));
            }
            check_guarded(inner, forbidden)
        }

        // Sub-expressions that need recursive checking.
        Exp::Lam(_, e) => check_guarded(e, forbidden),
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
        Exp::NativeDecide(_, v) => check_guarded(v, forbidden),
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
        Exp::CoRecord(fields) => {
            for f in fields {
                check_guarded(&f.body, forbidden)?;
            }
            Ok(())
        }

        // Leaves — no sub-expressions to check.
        Exp::Var(_)
        | Exp::Set
        | Exp::Type(_)
        | Exp::One
        | Exp::Unit
        | Exp::EigonClass(_)
        | Exp::EigonPrimitive(_)
        | Exp::EigonResource(_) => Ok(()),
    }
}

/// Find a field by name in a Sigma chain.
/// Walks Σ name₁ : T₁. Σ name₂ : T₂. ... looking for a matching name.
fn find_sigma_field(typ: &Val, field_name: &str) -> Option<Val> {
    match typ {
        Val::Sig(t, g) => {
            if g.patt == Patt::Var(field_name.to_string()) {
                // Found — return the field's type
                Some(*t.clone())
            } else {
                // Not this field — apply the closure with a dummy value
                // and search the rest of the chain
                let gen = gen_val(&g.env);
                let rest = g.apply(gen);
                find_sigma_field(&rest, field_name)
            }
        }
        // Also check EigonClass — could resolve to a Sigma if we had layer access.
        // For now, return Set as a fallback for unresolved class types.
        Val::EigonClass(_) => Some(Val::Set),
        _ => None,
    }
}

/// Extract a Pi type: Pi(A, x.B) → (A, x.B)
fn ext_pi(val: &Val) -> Result<(Val, Clos), String> {
    match val {
        Val::Pi(t, g) => Ok((*t.clone(), g.clone())),
        u => Err(format!("expected Pi type, got: {u:?}")),
    }
}

/// Extract a Sigma type: Sig(A, x.B) → (A, x.B)
fn ext_sig(val: &Val) -> Result<(Val, Clos), String> {
    match val {
        Val::Sig(t, g) => Ok((*t.clone(), g.clone())),
        u => Err(format!("expected Sigma type, got: {u:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::PrimitiveType;
    use crate::ontology::iri::Iri;

    #[test]
    fn check_unit_has_type_one() {
        check(&Rho::Nil, &vec![], &Exp::Unit, &Val::One).unwrap();
    }

    #[test]
    fn check_one_has_type_set() {
        check(&Rho::Nil, &vec![], &Exp::One, &Val::Set).unwrap();
    }

    #[test]
    fn check_set_is_type() {
        check_type(&Rho::Nil, &vec![], &Exp::Set).unwrap();
    }

    #[test]
    fn check_one_is_type() {
        check_type(&Rho::Nil, &vec![], &Exp::One).unwrap();
    }

    #[test]
    fn check_pi_is_type() {
        // Π _ : 1. 1 is a valid type
        let pi = Exp::Pi(Patt::Unit, Box::new(Exp::One), Box::new(Exp::One));
        check_type(&Rho::Nil, &vec![], &pi).unwrap();
    }

    #[test]
    fn check_identity_function() {
        // λx.x : Π x : 1. 1
        let lam = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::Var("x".to_string())),
        );
        let pi = Val::Pi(
            Box::new(Val::One),
            Clos::new(Patt::Unit, Exp::One, Rho::Nil),
        );
        check(&Rho::Nil, &vec![], &lam, &pi).unwrap();
    }

    #[test]
    fn check_pair() {
        // ((), ()) : Σ _ : 1. 1
        let pair = Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Unit));
        let sig = Val::Sig(
            Box::new(Val::One),
            Clos::new(Patt::Unit, Exp::One, Rho::Nil),
        );
        check(&Rho::Nil, &vec![], &pair, &sig).unwrap();
    }

    #[test]
    fn check_type_mismatch_fails() {
        // () : U should fail (unit is not a type)
        let result = check(&Rho::Nil, &vec![], &Exp::Unit, &Val::Set);
        assert!(result.is_err());
    }

    #[test]
    fn check_let_declaration() {
        // let x : 1 = (); x : 1
        let d = Decl::Def(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(Exp::Unit),
        );
        let e = Exp::Dec(d, Box::new(Exp::Var("x".to_string())));
        check(&Rho::Nil, &vec![], &e, &Val::One).unwrap();
    }

    #[test]
    fn infer_variable_type() {
        let gamma: Gamma = vec![("x".to_string(), Val::One)];
        let t = check_infer(&Rho::Nil, &gamma, &Exp::Var("x".to_string())).unwrap();
        assert!(matches!(t, Val::One));
    }

    #[test]
    fn infer_application_type() {
        // f : 1 → 1, f () : 1
        let pi_type = Val::Pi(
            Box::new(Val::One),
            Clos::new(Patt::Unit, Exp::One, Rho::Nil),
        );
        let gamma: Gamma = vec![("f".to_string(), pi_type)];
        let rho = Rho::Nil.extend(
            Patt::Var("f".to_string()),
            Val::Lam(Clos::new(
                Patt::Var("x".to_string()),
                Exp::Var("x".to_string()),
                Rho::Nil,
            )),
        );
        let t = check_infer(
            &rho,
            &gamma,
            &Exp::App(Box::new(Exp::Var("f".to_string())), Box::new(Exp::Unit)),
        )
        .unwrap();
        assert!(matches!(t, Val::One));
    }

    #[test]
    fn eq_nf_equal() {
        eq_nf(0, &Val::One, &Val::One).unwrap();
        eq_nf(0, &Val::Unit, &Val::Unit).unwrap();
        eq_nf(0, &Val::Set, &Val::Set).unwrap();
    }

    #[test]
    fn eq_nf_not_equal() {
        assert!(eq_nf(0, &Val::One, &Val::Set).is_err());
        assert!(eq_nf(0, &Val::Unit, &Val::One).is_err());
    }

    #[test]
    fn check_sum_type() {
        // Sum(a 1 | b 1) : U
        let data = Exp::Data(vec![
            crate::nbe::term::Summand {
                name: "a".to_string(),
                typ: Exp::One,
            },
            crate::nbe::term::Summand {
                name: "b".to_string(),
                typ: Exp::One,
            },
        ]);
        check(&Rho::Nil, &vec![], &data, &Val::Set).unwrap();
    }

    #[test]
    fn check_constructor_against_sum() {
        // $a () : Sum(a 1 | b 1)
        let data_val = Val::Data(
            vec![("a".to_string(), Exp::One), ("b".to_string(), Exp::One)],
            Rho::Nil,
        );
        let con = Exp::Con("a".to_string(), Box::new(Exp::Unit));
        check(&Rho::Nil, &vec![], &con, &data_val).unwrap();
    }

    #[test]
    fn check_constructor_wrong_name_fails() {
        let data_val = Val::Data(vec![("a".to_string(), Exp::One)], Rho::Nil);
        let con = Exp::Con("b".to_string(), Box::new(Exp::Unit));
        assert!(check(&Rho::Nil, &vec![], &con, &data_val).is_err());
    }

    #[test]
    fn check_id_is_type() {
        // Id(1, (), ()) : Set
        let id = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        check(&Rho::Nil, &vec![], &id, &Val::Set).unwrap();
    }

    #[test]
    fn check_id_type_well_formed() {
        let id = Exp::Id(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        check_type(&Rho::Nil, &vec![], &id).unwrap();
    }

    #[test]
    fn check_refl_against_id() {
        // refl(()) : Id(1, (), ())
        let refl = Exp::Refl(Box::new(Exp::Unit));
        let id_type = Val::Id(Box::new(Val::One), Box::new(Val::Unit), Box::new(Val::Unit));
        check(&Rho::Nil, &vec![], &refl, &id_type).unwrap();
    }

    #[test]
    fn check_refl_wrong_endpoints_fails() {
        // refl(()) : Id(1, (), x) should fail when x ≠ ()
        let refl = Exp::Refl(Box::new(Exp::Unit));
        let gen = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let id_type = Val::Id(Box::new(Val::One), Box::new(Val::Unit), Box::new(gen));
        assert!(check(&Rho::Nil, &vec![], &refl, &id_type).is_err());
    }

    #[test]
    fn eval_j_with_refl_reduces() {
        // J(1, C, d, (), (), refl(())) should reduce to d(())
        use crate::nbe::eval::eval;
        let j = Exp::IdJ(Box::new([
            Exp::One,                                                        // A
            Exp::Set,                                                        // C (placeholder)
            Exp::Lam(Patt::Var("a".into()), Box::new(Exp::Var("a".into()))), // d = λa. a
            Exp::Unit,                                                       // x
            Exp::Unit,                                                       // y
            Exp::Refl(Box::new(Exp::Unit)),                                  // p = refl(())
        ]));
        let result = eval(&j, &Rho::Nil);
        // d(()) = (λa.a)(()) = ()
        assert!(matches!(result, Val::Unit));
    }

    #[test]
    fn deceq_equal_reduces_to_refl() {
        use crate::nbe::eval::eval;
        // DecEq(1, (), ()) → refl(())
        let deceq = Exp::DecEq(Box::new(Exp::One), Box::new(Exp::Unit), Box::new(Exp::Unit));
        let result = eval(&deceq, &Rho::Nil);
        assert!(matches!(result, Val::Refl(_)));
    }

    #[test]
    fn deceq_unequal_produces_neutral() {
        use crate::nbe::eval::eval;
        // DecEq(Set, 1, Set) — One ≠ Set, produces neutral
        let deceq = Exp::DecEq(Box::new(Exp::Set), Box::new(Exp::One), Box::new(Exp::Set));
        let result = eval(&deceq, &Rho::Nil);
        assert!(matches!(result, Val::Nt(_)));
    }

    #[test]
    fn deceq_iri_equal() {
        use crate::nbe::eval::eval;
        let iri = Iri::parse("urn:eigenius:core:string").unwrap();
        let deceq = Exp::DecEq(
            Box::new(Exp::Set),
            Box::new(Exp::EigonClass(iri.clone())),
            Box::new(Exp::EigonClass(iri)),
        );
        let result = eval(&deceq, &Rho::Nil);
        assert!(matches!(result, Val::Refl(_)));
    }

    #[test]
    fn deceq_iri_unequal() {
        use crate::nbe::eval::eval;
        let iri1 = Iri::parse("urn:eigenius:core:string").unwrap();
        let iri2 = Iri::parse("urn:eigenius:core:integer").unwrap();
        let deceq = Exp::DecEq(
            Box::new(Exp::Set),
            Box::new(Exp::EigonClass(iri1)),
            Box::new(Exp::EigonClass(iri2)),
        );
        let result = eval(&deceq, &Rho::Nil);
        assert!(matches!(result, Val::Nt(_)));
    }

    #[test]
    fn check_eigon_primitive_is_type() {
        check_type(
            &Rho::Nil,
            &vec![],
            &Exp::EigonPrimitive(PrimitiveType::String),
        )
        .unwrap();
        check(
            &Rho::Nil,
            &vec![],
            &Exp::EigonPrimitive(PrimitiveType::Integer),
            &Val::Set,
        )
        .unwrap();
    }

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
        check_type(&Rho::Nil, &vec![], &pair_codata_type()).unwrap();
        check(&Rho::Nil, &vec![], &pair_codata_type(), &Val::Set).unwrap();
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
        assert!(check_type(&Rho::Nil, &vec![], &bad).is_err());
    }

    #[test]
    fn corecord_checks_against_codata_type() {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil);
        check(&Rho::Nil, &vec![], &unit_pair_corecord(), &codata_typ).unwrap();
    }

    #[test]
    fn corecord_mismatched_fields_rejected() {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil);
        // Missing 'snd'
        let bad = Exp::CoRecord(vec![crate::nbe::term::CoField {
            name: "fst".to_string(),
            body: Exp::Unit,
        }]);
        assert!(check(&Rho::Nil, &vec![], &bad, &codata_typ).is_err());
    }

    #[test]
    fn corecord_wrong_field_order_rejected() {
        use crate::nbe::eval::eval;
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil);
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
        assert!(check(&Rho::Nil, &vec![], &bad, &codata_typ).is_err());
    }

    #[test]
    fn observation_evaluates_to_field_body() {
        use crate::nbe::eval::eval;
        // corecord { fst = (); snd = () }.fst → ()
        let observe = Exp::Observe(Box::new(unit_pair_corecord()), "fst".to_string());
        let result = eval(&observe, &Rho::Nil);
        assert!(matches!(result, Val::Unit));
    }

    #[test]
    fn observation_unknown_field_panics() {
        // Using catch_unwind since vobserve panics on unknown field
        use crate::nbe::eval::eval;
        let observe = Exp::Observe(Box::new(unit_pair_corecord()), "missing".to_string());
        let result = std::panic::catch_unwind(|| eval(&observe, &Rho::Nil));
        assert!(result.is_err());
    }

    #[test]
    fn observation_type_inference() {
        use crate::nbe::env::up_gamma;
        use crate::nbe::eval::eval;
        // Given x : codata { fst : 1; snd : 1 }, infer x.fst : 1
        let codata_typ = eval(&pair_codata_type(), &Rho::Nil);
        let gen = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let gamma = up_gamma(&vec![], &Patt::Var("x".to_string()), &codata_typ, &gen).unwrap();
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), gen);
        let observe = Exp::Observe(Box::new(Exp::Var("x".to_string())), "fst".to_string());
        let t = check_infer(&rho, &gamma, &observe).unwrap();
        assert!(matches!(t, Val::One));
    }

    #[test]
    fn observation_on_neutral_blocks() {
        use crate::nbe::eval::eval;
        // let x = <neutral>; x.fst should produce a Neut::Observe
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "x".to_string()));
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), neut);
        let observe = Exp::Observe(Box::new(Exp::Var("x".to_string())), "fst".to_string());
        let result = eval(&observe, &rho);
        assert!(matches!(
            result,
            Val::Nt(crate::nbe::val::Neut::Observe(_, _))
        ));
    }

    #[test]
    fn stream_two_observations_advance() {
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
        let result = eval(&expr, &Rho::Nil);
        assert!(matches!(result, Val::Unit));
    }

    #[test]
    fn recursive_stream_via_letrec() {
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
        let result = eval(&letrec, &Rho::Nil);
        assert!(matches!(result, Val::Unit));
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
        let err = check_guarded(&body, &forbidden(&["bad"])).unwrap_err();
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
        // concern; Mini-TT doesn't check it either).
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
        let err = check_decl(&Rho::Nil, &vec![], &d).unwrap_err();
        assert!(
            err.contains("unguarded"),
            "expected unguarded error, got: {err}"
        );
    }
}
