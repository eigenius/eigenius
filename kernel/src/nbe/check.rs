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

        // Eigenius: property access type inference
        // Walk the Sigma chain to find the field matching the property's local name.
        Exp::PropAccess(e, prop) => {
            let t = check_infer(rho, gamma, e)?;
            let prop_name = prop.local_name();
            find_sigma_field(&t, prop_name).ok_or_else(|| {
                format!(
                    "property '{}' not found in type {:?}",
                    prop,
                    readback_val(rho.len(), &t)
                )
            })
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
}
