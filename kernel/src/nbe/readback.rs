//! Mini-TT readback: values → normal-form expressions.
//!
//! Ported from `Main.hs` lines 226-255 in the Mini-TT reference.
//! Readback converts semantic values back to syntax, producing
//! normal forms. Two values are definitionally equal iff their
//! readbacks at the same level are syntactically equal.

use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, Name, Patt};
use crate::nbe::val::{Neut, Val};

/// Readback a value to a normal-form expression.
///
/// `level` is the current de Bruijn level (number of binders above).
/// Port of `rbV` from the reference.
pub fn readback_val(level: usize, val: &Val) -> Exp {
    match val {
        Val::Lam(f) => {
            let gen = gen_val(level);
            Exp::Lam(
                gen_patt(level),
                Box::new(readback_val(level + 1, &f.apply(gen))),
            )
        }
        Val::Pair(u, v) => Exp::Pair(
            Box::new(readback_val(level, u)),
            Box::new(readback_val(level, v)),
        ),
        Val::Con(c, v) => Exp::Con(c.clone(), Box::new(readback_val(level, v))),
        Val::Unit => Exp::Unit,
        Val::Set => Exp::Set,
        Val::Pi(t, g) => {
            let gen = gen_val(level);
            Exp::Pi(
                gen_patt(level),
                Box::new(readback_val(level, t)),
                Box::new(readback_val(level + 1, &g.apply(gen))),
            )
        }
        Val::Sig(t, g) => {
            let gen = gen_val(level);
            Exp::Sig(
                gen_patt(level),
                Box::new(readback_val(level, t)),
                Box::new(readback_val(level + 1, &g.apply(gen))),
            )
        }
        Val::One => Exp::One,
        Val::Fun(cases, rho) => readback_closure_like(level, cases, rho, true),
        Val::Data(summands, rho) => readback_closure_like(level, summands, rho, false),
        Val::Nt(k) => readback_neut(level, k),

        // Eigenius extensions
        Val::EigonClass(iri) => Exp::EigonClass(iri.clone()),
        Val::EigonPrimitive(p) => Exp::EigonPrimitive(*p),
        Val::ResourceVal(r) => Exp::EigonResource(r.clone()),
    }
}

/// Readback a neutral term to an expression.
///
/// Port of `rbN` from the reference.
pub fn readback_neut(level: usize, neut: &Neut) -> Exp {
    match neut {
        Neut::Gen(j, name) => Exp::Var(format!("{name}{j}")),
        Neut::App(k, m) => Exp::App(
            Box::new(readback_neut(level, k)),
            Box::new(readback_val(level, m)),
        ),
        Neut::Fst(k) => Exp::Fst(Box::new(readback_neut(level, k))),
        Neut::Snd(k) => Exp::Snd(Box::new(readback_neut(level, k))),
        Neut::NtFun(cases, rho, k) => {
            let fun_exp = readback_closure_like(level, cases, rho, true);
            Exp::App(Box::new(fun_exp), Box::new(readback_neut(level, k)))
        }
        // Eigenius extension
        Neut::PropAccess(k, prop) => {
            Exp::PropAccess(Box::new(readback_neut(level, k)), prop.clone())
        }
    }
}

/// Readback a Fun or Data closure.
fn readback_closure_like(level: usize, _cases: &[(Name, Exp)], rho: &Rho, is_fun: bool) -> Exp {
    // Read back the environment
    let rho_exps = readback_rho(level, rho);
    let tag = format!("__{}_{}", if is_fun { "fun" } else { "data" }, level);
    let mut result = Exp::Var(tag);
    for exp in rho_exps {
        result = Exp::App(Box::new(result), Box::new(exp));
    }
    result
}

/// Read back the values in an environment.
fn readback_rho(level: usize, rho: &Rho) -> Vec<Exp> {
    match rho {
        Rho::Nil => vec![],
        Rho::UpVar(rho, _, v) => {
            let mut exps = vec![readback_val(level, v)];
            exps.extend(readback_rho(level, rho));
            exps
        }
        Rho::UpDec(rho, _) => readback_rho(level, rho),
    }
}

/// Generate a fresh variable value at a given level.
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
        assert_eq!(readback_val(0, &Val::Set), Exp::Set);
    }

    #[test]
    fn readback_one() {
        assert_eq!(readback_val(0, &Val::One), Exp::One);
    }

    #[test]
    fn readback_pair() {
        let v = Val::Pair(Box::new(Val::Unit), Box::new(Val::Set));
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

        let v3 = Val::Set;
        assert_ne!(readback_val(0, &v1), readback_val(0, &v3));
    }
}
