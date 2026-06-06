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

//! EigenTT semantic values.
//!
//! Ported from `Main.hs` lines 80-163 in the EigenTT reference.
//! Values are the result of evaluation. Neutral terms represent
//! computations blocked on an unknown variable.

use crate::nbe::env::Rho;
use crate::nbe::eval::EvalError;
use crate::nbe::term::{CodataDecl, Exp, InductiveDecl, Name, Patt, PrimitiveType};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use std::sync::Arc;

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
    /// Universe at a specific level: Sort(n).
    /// `Sort(0) = Prop`, `Sort(1) = Set`, `Sort(n+1)` was `Type(n)` for `n >= 1`.
    /// See D46 §3.
    Sort(usize),
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
    /// Template value with resolved property type requirements.
    /// Template("literal", [(iri, resolved_type)])
    TemplateVal(String, Vec<(Iri, Val)>),

    // --- Codata (D11, Phase 9b-i) ---
    /// Codata type value: captures the observation-type pairs plus the
    /// environment needed to evaluate them. The anonymous form —
    /// unparameterised and self-reference-incapable. Used for legacy
    /// codata declarations and the projected view of an applied
    /// `CodataType` at use sites that don't need the decl reference.
    Codata(Vec<(Name, Exp)>, Rho),

    /// Parameterised codata type former applied to evaluated
    /// parameters: `C(p₁, …, pₙ)`. The `Arc<CodataDecl>` carries the
    /// observation list; self-references inside observation types
    /// resolve via name-based `PartialEq`. Parallels
    /// `Val::InductiveType`.
    ///
    /// `params` is empty for the *unapplied* type former; use
    /// `Exp::CodataType(decl, args)` to apply arguments.
    CodataType {
        decl: Arc<CodataDecl>,
        params: Vec<Val>,
    },
    /// Codata value (corecord): lazy copattern definitions. Each entry
    /// is `(obs_name, body_exp)`; the body is evaluated only when the
    /// matching `Observe` is applied, in the captured environment.
    CoRecord(Vec<(Name, Exp)>, Rho),

    // --- Map/Reduce (Phase 11a) ---
    /// Finite list (array). Primary representation for resource arrays
    /// and the result of Map. Phase 11b's inductive List evaluates to
    /// this at runtime.
    List(Vec<Val>),

    // --- Inductive types (Phase 11b, D19) ---
    /// Inductive type former applied to evaluated parameters: `I(p₁, …, pₙ)`.
    InductiveType {
        decl: Arc<InductiveDecl>,
        params: Vec<Val>,
    },
    /// Constructor value: `c(args)` on the named inductive.
    InductiveVal {
        decl: Arc<InductiveDecl>,
        ctor_name: Name,
        args: Vec<Val>,
    },

    // --- Sized types (Phase 11b step 14, D19 §8) ---
    /// The sort of size expressions, `SizeSort`. Itself a type
    /// (lives at universe `Type(1)` for our hierarchy).
    SizeSort,
    /// `SizeSucc(s)` — the successor of a size value. The smallest
    /// size strictly larger than `s`.
    SizeSucc(Box<Val>),
    /// The unbounded "infinity" size — the top element under the
    /// size partial order.
    SizeInf,

    /// Bounded size Π-type value: `Π {i < upper}. body(i)`.
    /// `upper` is the evaluated size upper bound; the closure binds
    /// the fresh size variable in the body.
    SizedPi(Box<Val>, Clos),
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

    // --- Codata (D11, Phase 9b-i) ---
    /// Observation on a neutral codata value: (neut).obs
    Observe(Box<Neut>, Name),

    // --- Map/Reduce (Phase 11a) ---
    /// Map blocked on a neutral collection.
    NtMap(Box<Val>, Box<Neut>),
    /// Reduce blocked on a neutral collection.
    NtReduce(Box<Val>, Box<Val>, Box<Neut>),

    // --- Inductive types (Phase 11b, D19) ---
    /// Recursor application blocked on a neutral major premise.
    /// `I.rec params motive minors major` where `major` has not yet
    /// reduced to a constructor.
    NtRec {
        decl: Arc<InductiveDecl>,
        motive: Box<Val>,
        minors: Vec<Val>,
        major: Box<Neut>,
    },

    /// `match` blocked on a neutral scrutinee (Phase 11b step 12).
    ///
    /// Carries the original arms verbatim — they are not yet
    /// pre-evaluated because we don't know which one will run, and
    /// some arms may safely diverge until their constructor is
    /// matched. The captured `Rho` is the environment in effect at
    /// the match site, used when the neutral eventually unblocks.
    ///
    /// Type-level: a neutral `Match` does not yet know its motive
    /// (motive inference happens in checking mode). Readback emits
    /// `Exp::Match` rather than `Exp::InductiveRec`, preserving the
    /// motive-free shape.
    NtMatch {
        scrutinee: Box<Neut>,
        arms: Vec<crate::nbe::term::MatchArm>,
        env: Rho,
    },
}

// --- Sized types (Phase 11b step 14, D19 §8) ---
//
// Size values live as their own subset of `Val` rather than going
// through Sigma/Pi. They form a partial order under `SizeSucc`
// applications, with `SizeInf` as the top element. Constraint-
// generation logic (Phase 11b step 15+) inspects these forms when
// type-checking sized inductive/coinductive applications.

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
    pub fn apply(&self, v: Val) -> Result<Val, EvalError> {
        crate::nbe::eval::eval(&self.body, &self.env.clone().extend(self.patt.clone(), v))
    }

    /// Instantiate the closure with a value and capability context.
    pub fn apply_ctx(&self, v: Val, ctx: &crate::nbe::eval::EvalCtx) -> Result<Val, EvalError> {
        crate::nbe::eval::eval_ctx(
            &self.body,
            &self.env.clone().extend(self.patt.clone(), v),
            ctx,
        )
    }

    /// Instantiate the closure with tracing.
    pub fn apply_ctx_traced(
        &self,
        v: Val,
        ctx: &crate::nbe::eval::EvalCtx,
    ) -> Result<(Val, Option<crate::program::trace::Trace>), EvalError> {
        crate::nbe::eval::eval_traced(
            &self.body,
            &self.env.clone().extend(self.patt.clone(), v),
            ctx,
        )
    }
}

// --- Operations on values (reference lines 147-163) ---

impl Val {
    /// Function application: (λ f) v = f * v; (fun ...) ($c v) = ...; neutral app
    pub fn app(self, v: Val) -> Result<Val, EvalError> {
        match self {
            Val::Lam(f) => f.apply(v),
            Val::Fun(cases, rho) => {
                if let Val::Con(c, cv) = v {
                    for (name, exp) in &cases {
                        if *name == c {
                            return crate::nbe::eval::eval(exp, &rho)?.app(*cv);
                        }
                    }
                    Err(EvalError::ConstructorNotFound(c))
                } else if let Val::Nt(k) = v {
                    Ok(Val::Nt(Neut::NtFun(cases, rho, Box::new(k))))
                } else {
                    Err(EvalError::InvalidCaseTarget(format!("{v:?}")))
                }
            }
            Val::Nt(k) => Ok(Val::Nt(Neut::App(Box::new(k), Box::new(v)))),
            other => Err(EvalError::NotAFunction(format!("{other:?}"))),
        }
    }

    /// Function application with capability context.
    pub fn app_ctx(self, v: Val, ctx: &crate::nbe::eval::EvalCtx) -> Result<Val, EvalError> {
        match self {
            Val::Lam(f) => f.apply_ctx(v, ctx),
            Val::Fun(cases, rho) => {
                if let Val::Con(c, cv) = v {
                    for (name, exp) in &cases {
                        if *name == c {
                            return crate::nbe::eval::eval_ctx(exp, &rho, ctx)?.app_ctx(*cv, ctx);
                        }
                    }
                    Err(EvalError::ConstructorNotFound(c))
                } else if let Val::Nt(k) = v {
                    Ok(Val::Nt(Neut::NtFun(cases, rho, Box::new(k))))
                } else {
                    Err(EvalError::InvalidCaseTarget(format!("{v:?}")))
                }
            }
            Val::Nt(k) => Ok(Val::Nt(Neut::App(Box::new(k), Box::new(v)))),
            other => Err(EvalError::NotAFunction(format!("{other:?}"))),
        }
    }

    /// First projection: fst (a, b) = a; fst (neutral) = neutral
    pub fn vfst(self) -> Result<Val, EvalError> {
        match self {
            Val::Pair(u1, _) => Ok(*u1),
            Val::Nt(k) => Ok(Val::Nt(Neut::Fst(Box::new(k)))),
            other => Err(EvalError::NotAPair(format!("vfst: {other:?}"))),
        }
    }

    /// Second projection: snd (a, b) = b; snd (neutral) = neutral
    pub fn vsnd(self) -> Result<Val, EvalError> {
        match self {
            Val::Pair(_, u2) => Ok(*u2),
            Val::Nt(k) => Ok(Val::Nt(Neut::Snd(Box::new(k)))),
            other => Err(EvalError::NotAPair(format!("vsnd: {other:?}"))),
        }
    }

    /// Observation on a codata value: v.obs looks up the named field in
    /// a `CoRecord` and evaluates its body in the captured environment.
    /// For a neutral value, produces a blocked `Neut::Observe`. Pure mode.
    pub fn vobserve(self, obs: &str) -> Result<Val, EvalError> {
        match self {
            Val::CoRecord(fields, rho) => {
                for (name, body) in &fields {
                    if name == obs {
                        return crate::nbe::eval::eval(body, &rho);
                    }
                }
                Err(EvalError::ObservationNotFound(obs.to_string()))
            }
            Val::Nt(k) => Ok(Val::Nt(Neut::Observe(Box::new(k), obs.to_string()))),
            other => Err(EvalError::NotACorecord(format!("{other:?}"))),
        }
    }

    /// Function application with tracing.
    ///
    /// For `Fun` applied to `Con`, produces a `Trace::Case`.
    /// For `Lam`, delegates to `apply_ctx_traced`.
    pub fn app_ctx_traced(
        self,
        v: Val,
        ctx: &crate::nbe::eval::EvalCtx,
    ) -> Result<(Val, Option<crate::program::trace::Trace>), EvalError> {
        use crate::program::trace::Trace;
        match self {
            Val::Lam(f) => f.apply_ctx_traced(v, ctx),
            Val::Fun(cases, rho) => {
                if let Val::Con(c, cv) = v {
                    for (name, exp) in &cases {
                        if *name == c {
                            let (branch_fn, branch_trace) =
                                crate::nbe::eval::eval_traced(exp, &rho, ctx)?;
                            let (result, app_trace) = branch_fn.app_ctx_traced(*cv, ctx)?;
                            let trace = Some(Trace::Case {
                                scrutinee_trace: None,
                                branch_taken: c,
                                branch_trace: app_trace.or(branch_trace).map(Box::new),
                            });
                            return Ok((result, trace));
                        }
                    }
                    Err(EvalError::ConstructorNotFound(c))
                } else if let Val::Nt(k) = v {
                    Ok((Val::Nt(Neut::NtFun(cases, rho, Box::new(k))), None))
                } else {
                    Err(EvalError::InvalidCaseTarget(format!("{v:?}")))
                }
            }
            Val::Nt(k) => Ok((Val::Nt(Neut::App(Box::new(k), Box::new(v))), None)),
            other => Err(EvalError::NotAFunction(format!("{other:?}"))),
        }
    }

    /// Observation on a codata value with tracing.
    pub fn vobserve_ctx_traced(
        self,
        obs: &str,
        ctx: &crate::nbe::eval::EvalCtx,
    ) -> Result<(Val, Option<crate::program::trace::Trace>), EvalError> {
        match self {
            Val::CoRecord(fields, rho) => {
                for (name, body) in &fields {
                    if name == obs {
                        return crate::nbe::eval::eval_traced(body, &rho, ctx);
                    }
                }
                Err(EvalError::ObservationNotFound(obs.to_string()))
            }
            Val::Nt(k) => Ok((Val::Nt(Neut::Observe(Box::new(k), obs.to_string())), None)),
            other => Err(EvalError::NotACorecord(format!("{other:?}"))),
        }
    }

    /// Observation on a codata value, with capability context.
    pub fn vobserve_ctx(
        self,
        obs: &str,
        ctx: &crate::nbe::eval::EvalCtx,
    ) -> Result<Val, EvalError> {
        match self {
            Val::CoRecord(fields, rho) => {
                for (name, body) in &fields {
                    if name == obs {
                        return crate::nbe::eval::eval_ctx(body, &rho, ctx);
                    }
                }
                Err(EvalError::ObservationNotFound(obs.to_string()))
            }
            Val::Nt(k) => Ok(Val::Nt(Neut::Observe(Box::new(k), obs.to_string()))),
            other => Err(EvalError::NotACorecord(format!("{other:?}"))),
        }
    }
}

/// Convert a cons-pair list value to a `Vec`.
///
/// Recognises the `Con("nil", _)` / `Con("cons", Pair(head, tail))` encoding
/// used by `Exp::list()`. Returns `None` if the value is not a well-formed
/// cons list.
pub fn cons_to_vec(val: &Val) -> Option<Vec<Val>> {
    let mut items = Vec::new();
    let mut current = val;
    loop {
        match current {
            Val::Con(name, _) if name == "nil" => return Some(items),
            Val::Con(name, inner) if name == "cons" => {
                if let Val::Pair(head, tail) = inner.as_ref() {
                    items.push(head.as_ref().clone());
                    current = tail.as_ref();
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
}

/// Convert a canonical-`List` inductive value to a `Vec`.
///
/// Recognises `Val::InductiveVal { decl, ctor_name, args }` where
/// `decl.name == "List"` and `ctor_name` is `nil` or `cons`. The `cons`
/// case expects exactly two args: head and tail (where tail is itself
/// a list value to recurse into).
///
/// Closes the runtime gap left open by Phase 11b step 6: list types
/// migrated to the canonical inductive `List(A)`, but list values
/// remained in the legacy `Val::List(Vec)` and `Val::Con` forms.
/// Step 7+ producers (ESL `data` syntax compilation, direct
/// `Exp::InductiveCtor(list_decl(), …)` use) will produce
/// `InductiveVal`-backed lists; Map/Reduce dispatch on this helper
/// to keep them working uniformly.
pub fn inductive_list_to_vec(val: &Val) -> Option<Vec<Val>> {
    let mut items = Vec::new();
    let mut current = val;
    loop {
        match current {
            Val::InductiveVal {
                decl,
                ctor_name,
                args,
            } if decl.name == "List" && ctor_name == "nil" => {
                if !args.is_empty() {
                    return None;
                }
                return Some(items);
            }
            Val::InductiveVal {
                decl,
                ctor_name,
                args,
            } if decl.name == "List" && ctor_name == "cons" => {
                if args.len() != 2 {
                    return None;
                }
                items.push(args[0].clone());
                current = &args[1];
            }
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::eval::EvalError;

    #[test]
    fn vfst_pair() -> Result<(), EvalError> {
        let p = Val::Pair(Box::new(Val::Unit), Box::new(Val::Sort(1)));
        assert!(matches!(p.vfst()?, Val::Unit));
        Ok(())
    }

    #[test]
    fn vsnd_pair() -> Result<(), EvalError> {
        let p = Val::Pair(Box::new(Val::Unit), Box::new(Val::Sort(1)));
        assert!(matches!(p.vsnd()?, Val::Sort(1)));
        Ok(())
    }

    #[test]
    fn vfst_neutral() -> Result<(), EvalError> {
        let n = Val::Nt(Neut::Gen(0, "x".to_string()));
        assert!(matches!(n.vfst()?, Val::Nt(Neut::Fst(_))));
        Ok(())
    }

    #[test]
    fn vsnd_neutral() -> Result<(), EvalError> {
        let n = Val::Nt(Neut::Gen(0, "x".to_string()));
        assert!(matches!(n.vsnd()?, Val::Nt(Neut::Snd(_))));
        Ok(())
    }

    #[test]
    fn app_neutral() -> Result<(), EvalError> {
        let n = Val::Nt(Neut::Gen(0, "f".to_string()));
        let result = n.app(Val::Unit)?;
        assert!(matches!(result, Val::Nt(Neut::App(_, _))));
        Ok(())
    }
}
