//! Mini-TT semantic values.
//!
//! Ported from `Main.hs` lines 80-163 in the Mini-TT reference.
//! Values are the result of evaluation. Neutral terms represent
//! computations blocked on an unknown variable.

use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, Name, Patt, PrimitiveType};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;

/// Semantic values — the result of evaluation.
#[derive(Debug, Clone)]
pub enum Val {
    /// Lambda abstraction (closure)
    Lam(Clos),
    /// Pair value
    Pair(Box<Val>, Box<Val>),
    /// Constructor value: $c v
    Con(Name, Box<Val>),
    /// Unit value
    Unit,
    /// Universe of types (level 0)
    Set,
    /// Universe at a specific level
    Type(usize),
    /// Dependent function type: Π(A, x.B)
    Pi(Box<Val>, Clos),
    /// Dependent pair type: Σ(A, x.B)
    Sig(Box<Val>, Clos),
    /// Unit type
    One,
    /// Case function (from Sum): maps constructor names to branches
    Fun(Vec<(Name, Exp)>, Rho),
    /// Sum type: maps constructor names to their types
    Data(Vec<(Name, Exp)>, Rho),
    /// Neutral term — blocked on an unknown
    Nt(Neut),

    // --- Eigenius extensions ---
    /// Identity type: Id(A, x, y)
    Id(Box<Val>, Box<Val>, Box<Val>),
    /// Reflexivity proof: refl(a) inhabits Id(A, a, a)
    Refl(Box<Val>),

    /// Eigon class ground type (resolved from layer chain)
    EigonClass(Iri),
    /// Eigon primitive type
    EigonPrimitive(PrimitiveType),
    /// Concrete Eigon resource value
    ResourceVal(Box<Resource>),
}

/// Neutral terms — computations that cannot reduce further.
#[derive(Debug, Clone)]
pub enum Neut {
    /// Generated variable (de Bruijn level + name for readback)
    Gen(usize, Name),
    /// Application of a neutral to a value
    App(Box<Neut>, Box<Val>),
    /// First projection of a neutral pair
    Fst(Box<Neut>),
    /// Second projection of a neutral pair
    Snd(Box<Neut>),
    /// Case split on a neutral value
    NtFun(Vec<(Name, Exp)>, Rho, Box<Neut>),

    // --- Eigenius extension ---
    /// Property access on a neutral resource
    PropAccess(Box<Neut>, Iri),
}

/// A closure: a pattern, body expression, and captured environment.
#[derive(Debug, Clone)]
pub struct Clos {
    pub patt: Patt,
    pub body: Exp,
    pub env: Rho,
}

impl Clos {
    pub fn new(patt: Patt, body: Exp, env: Rho) -> Self {
        Self { patt, body, env }
    }

    /// Instantiate the closure with a value (Pure mode).
    pub fn apply(&self, v: Val) -> Val {
        crate::nbe::eval::eval(&self.body, &self.env.clone().extend(self.patt.clone(), v))
    }

    /// Instantiate the closure with a value and capability context.
    pub fn apply_ctx(&self, v: Val, ctx: &crate::nbe::eval::EvalCtx) -> Val {
        crate::nbe::eval::eval_ctx(
            &self.body,
            &self.env.clone().extend(self.patt.clone(), v),
            ctx,
        )
    }
}

// --- Operations on values (reference lines 147-163) ---

impl Val {
    /// Function application: (λ f) v = f * v; (fun ...) ($c v) = ...; neutral app
    pub fn app(self, v: Val) -> Val {
        match self {
            Val::Lam(f) => f.apply(v),
            Val::Fun(cases, rho) => {
                if let Val::Con(c, cv) = v {
                    for (name, exp) in &cases {
                        if *name == c {
                            return crate::nbe::eval::eval(exp, &rho).app(*cv);
                        }
                    }
                    panic!("app: constructor {} not found in case", c);
                } else if let Val::Nt(k) = v {
                    Val::Nt(Neut::NtFun(cases, rho, Box::new(k)))
                } else {
                    panic!("app Fun to non-constructor non-neutral");
                }
            }
            Val::Nt(k) => Val::Nt(Neut::App(Box::new(k), Box::new(v))),
            other => panic!("app: not a function: {:?}", other),
        }
    }

    /// Function application with capability context.
    pub fn app_ctx(self, v: Val, ctx: &crate::nbe::eval::EvalCtx) -> Val {
        match self {
            Val::Lam(f) => f.apply_ctx(v, ctx),
            Val::Fun(cases, rho) => {
                if let Val::Con(c, cv) = v {
                    for (name, exp) in &cases {
                        if *name == c {
                            return crate::nbe::eval::eval_ctx(exp, &rho, ctx).app_ctx(*cv, ctx);
                        }
                    }
                    panic!("app_ctx: constructor {} not found in case", c);
                } else if let Val::Nt(k) = v {
                    Val::Nt(Neut::NtFun(cases, rho, Box::new(k)))
                } else {
                    panic!("app_ctx Fun to non-constructor non-neutral");
                }
            }
            Val::Nt(k) => Val::Nt(Neut::App(Box::new(k), Box::new(v))),
            other => panic!("app_ctx: not a function: {:?}", other),
        }
    }

    /// First projection: fst (a, b) = a; fst (neutral) = neutral
    pub fn vfst(self) -> Val {
        match self {
            Val::Pair(u1, _) => *u1,
            Val::Nt(k) => Val::Nt(Neut::Fst(Box::new(k))),
            other => panic!("vfst: not a pair: {:?}", other),
        }
    }

    /// Second projection: snd (a, b) = b; snd (neutral) = neutral
    pub fn vsnd(self) -> Val {
        match self {
            Val::Pair(_, u2) => *u2,
            Val::Nt(k) => Val::Nt(Neut::Snd(Box::new(k))),
            other => panic!("vsnd: not a pair: {:?}", other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vfst_pair() {
        let p = Val::Pair(Box::new(Val::Unit), Box::new(Val::Set));
        assert!(matches!(p.vfst(), Val::Unit));
    }

    #[test]
    fn vsnd_pair() {
        let p = Val::Pair(Box::new(Val::Unit), Box::new(Val::Set));
        assert!(matches!(p.vsnd(), Val::Set));
    }

    #[test]
    fn vfst_neutral() {
        let n = Val::Nt(Neut::Gen(0, "x".to_string()));
        assert!(matches!(n.vfst(), Val::Nt(Neut::Fst(_))));
    }

    #[test]
    fn vsnd_neutral() {
        let n = Val::Nt(Neut::Gen(0, "x".to_string()));
        assert!(matches!(n.vsnd(), Val::Nt(Neut::Snd(_))));
    }

    #[test]
    fn app_neutral() {
        let n = Val::Nt(Neut::Gen(0, "f".to_string()));
        let result = n.app(Val::Unit);
        assert!(matches!(result, Val::Nt(Neut::App(_, _))));
    }
}
