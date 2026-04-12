//! Mini-TT evaluator: terms → values.
//!
//! Ported from `Main.hs` lines 198-217 in the Mini-TT reference.

use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, Patt};
use crate::nbe::val::{Clos, Neut, Val};
use crate::ontology::iri::Iri;

/// Evaluate an expression in an environment to produce a semantic value.
///
/// Port of `eval` from the reference.
pub fn eval(exp: &Exp, rho: &Rho) -> Val {
    match exp {
        Exp::Set => Val::Set,
        Exp::One => Val::One,
        Exp::Unit => Val::Unit,

        Exp::Dec(d, e) => eval(e, &Rho::UpDec(Box::new(rho.clone()), d.clone())),

        Exp::Lam(p, e) => Val::Lam(Clos::new(p.clone(), *e.clone(), rho.clone())),

        Exp::Pi(p, a, b) => Val::Pi(
            Box::new(eval(a, rho)),
            Clos::new(p.clone(), *b.clone(), rho.clone()),
        ),

        Exp::Sig(p, a, b) => Val::Sig(
            Box::new(eval(a, rho)),
            Clos::new(p.clone(), *b.clone(), rho.clone()),
        ),

        Exp::Fst(e) => eval(e, rho).vfst(),
        Exp::Snd(e) => eval(e, rho).vsnd(),

        Exp::App(e1, e2) => eval(e1, rho).app(eval(e2, rho)),

        Exp::Var(x) => rho.get(x).unwrap_or_else(|e| panic!("eval: {e}")),

        Exp::Pair(e1, e2) => Val::Pair(Box::new(eval(e1, rho)), Box::new(eval(e2, rho))),

        Exp::Con(c, e) => Val::Con(c.clone(), Box::new(eval(e, rho))),

        Exp::Data(summands) => Val::Data(
            summands
                .iter()
                .map(|s| (s.name.clone(), s.typ.clone()))
                .collect(),
            rho.clone(),
        ),

        Exp::Case(branches) => Val::Fun(
            branches
                .iter()
                .map(|b| (b.name.clone(), b.body.clone()))
                .collect(),
            rho.clone(),
        ),

        // Sugar: A → B = Π _ : A. B
        Exp::Arrow(a, b) => eval(&Exp::Pi(Patt::Unit, a.clone(), b.clone()), rho),
        // Sugar: A × B = Σ _ : A. B
        Exp::Times(a, b) => eval(&Exp::Sig(Patt::Unit, a.clone(), b.clone()), rho),

        // Eigenius extensions
        Exp::EigonClass(iri) => Val::EigonClass(iri.clone()),
        Exp::EigonPrimitive(p) => Val::EigonPrimitive(*p),
        Exp::EigonResource(r) => Val::ResourceVal(r.clone()),

        Exp::PropAccess(e, prop) => {
            let v = eval(e, rho);
            match v {
                Val::ResourceVal(r) => {
                    // Direct property access on a known resource
                    match r.get(prop) {
                        Some(val) => resource_value_to_val(val),
                        None => panic!("property {} not found on resource", prop),
                    }
                }
                Val::Nt(n) => Val::Nt(Neut::PropAccess(Box::new(n), prop.clone())),
                other => panic!("property access on non-resource: {:?}", other),
            }
        }
    }
}

/// Convert an Eigon resource Value to a Mini-TT Val.
fn resource_value_to_val(v: &crate::ontology::resource::Value) -> Val {
    use crate::ontology::resource::Value as RVal;
    match v {
        RVal::String(s) => {
            // Check if it looks like an IRI reference
            if let Ok(iri) = Iri::parse(s) {
                if s.starts_with("urn:") || s.starts_with("http") {
                    return Val::EigonClass(iri);
                }
            }
            Val::ResourceVal(Box::new({
                let mut r = crate::ontology::resource::Resource::new_embedded();
                let str_iri = Iri::parse("urn:eigenius:core:string").unwrap();
                r.set(str_iri, RVal::String(s.clone()));
                r
            }))
        }
        RVal::Integer(_) | RVal::Float(_) | RVal::Boolean(_) => {
            Val::ResourceVal(Box::new(crate::ontology::resource::Resource::new_embedded()))
        }
        RVal::Embedded(r) => Val::ResourceVal(r.clone()),
        _ => Val::Unit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::PrimitiveType;

    #[test]
    fn eval_set() {
        let v = eval(&Exp::Set, &Rho::Nil);
        assert!(matches!(v, Val::Set));
    }

    #[test]
    fn eval_unit() {
        let v = eval(&Exp::Unit, &Rho::Nil);
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_one() {
        let v = eval(&Exp::One, &Rho::Nil);
        assert!(matches!(v, Val::One));
    }

    #[test]
    fn eval_var() {
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), Val::Unit);
        let v = eval(&Exp::Var("x".to_string()), &rho);
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_pair() {
        let v = eval(
            &Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Set)),
            &Rho::Nil,
        );
        assert!(matches!(v, Val::Pair(_, _)));
    }

    #[test]
    fn eval_fst() {
        let v = eval(
            &Exp::Fst(Box::new(Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Set)))),
            &Rho::Nil,
        );
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_snd() {
        let v = eval(
            &Exp::Snd(Box::new(Exp::Pair(Box::new(Exp::Unit), Box::new(Exp::Set)))),
            &Rho::Nil,
        );
        assert!(matches!(v, Val::Set));
    }

    #[test]
    fn eval_lambda_app() {
        // (λx. x) () = ()
        let lam = Exp::Lam(
            Patt::Var("x".to_string()),
            Box::new(Exp::Var("x".to_string())),
        );
        let v = eval(&Exp::App(Box::new(lam), Box::new(Exp::Unit)), &Rho::Nil);
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_constructor() {
        let v = eval(&Exp::Con("ok".to_string(), Box::new(Exp::Unit)), &Rho::Nil);
        assert!(matches!(v, Val::Con(ref c, _) if c == "ok"));
    }

    #[test]
    fn eval_let() {
        // let x : 1 = (); x
        let d = crate::nbe::term::Decl::Def(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(Exp::Unit),
        );
        let v = eval(&Exp::Dec(d, Box::new(Exp::Var("x".to_string()))), &Rho::Nil);
        assert!(matches!(v, Val::Unit));
    }

    #[test]
    fn eval_neutral_var() {
        // An unbound variable in the environment produces a neutral
        let rho = Rho::Nil.extend(
            Patt::Var("x".to_string()),
            Val::Nt(Neut::Gen(0, "x".to_string())),
        );
        let v = eval(&Exp::Var("x".to_string()), &rho);
        assert!(matches!(v, Val::Nt(Neut::Gen(0, _))));
    }

    #[test]
    fn eval_neutral_app() {
        // f x where f is neutral — produces neutral application
        let rho = Rho::Nil
            .extend(
                Patt::Var("f".to_string()),
                Val::Nt(Neut::Gen(0, "f".to_string())),
            )
            .extend(Patt::Var("x".to_string()), Val::Unit);
        let v = eval(
            &Exp::App(
                Box::new(Exp::Var("f".to_string())),
                Box::new(Exp::Var("x".to_string())),
            ),
            &rho,
        );
        assert!(matches!(v, Val::Nt(Neut::App(_, _))));
    }

    #[test]
    fn eval_eigon_primitive() {
        let v = eval(&Exp::EigonPrimitive(PrimitiveType::String), &Rho::Nil);
        assert!(matches!(v, Val::EigonPrimitive(PrimitiveType::String)));
    }
}
