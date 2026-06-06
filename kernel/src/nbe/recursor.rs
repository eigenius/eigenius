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

//! Recursor type derivation for inductive types (Phase 11b step 4, D19 §6).
//!
//! Given an inductive declaration `I`, concrete parameter values, and a
//! motive `C : I(params) → Sort u`, derives the expected type of each
//! minor in a recursor application of `I`.
//!
//! For a constructor `cⱼ(a₁, …, aₘ)` whose argument types are
//! `T₁, …, Tₘ` (some of which are direct recursive references to `I`),
//! the minor's expected type is:
//!
//! ```text
//! Π a₁:T₁ … Π aₘ:Tₘ. Π ih₁:C(rec_arg₁) … Π ihₖ:C(rec_argₖ). C(cⱼ a₁ … aₘ)
//! ```
//!
//! where `rec_arg₁, …, rec_argₖ` are the recursive arguments in their
//! original order. The IHs are appended *after* all constructor
//! arguments — matching the iota-reduction order in
//! [`eval::iota_reduce`](super::eval).
//!
//! Restricted to the same fragment as the positivity checker and iota
//! reduction: direct recursive arguments only (no higher-order, no
//! nested). Higher-order recursion would need IHs of function type
//! (`Π x:T. C(arg(x))`); deferred until those features land together.
//!
//! Used by Phase 11b step 5 (type checking for `Exp::InductiveRec`) to
//! verify that user-supplied minors have the right type.

use crate::nbe::env::Rho;
use crate::nbe::eval::{eval_ctx, EvalCtx, EvalError};
use crate::nbe::readback::readback_val;
use crate::nbe::term::{Exp, InductiveDecl, Patt};
use crate::nbe::val::Val;
use std::sync::Arc;

/// Derive the expected types of every minor for a recursor application
/// of `decl` with the given concrete `params` and `motive`.
///
/// The returned `Vec` is one-to-one with `decl.ctors`.
pub fn derive_minor_types(
    decl: &Arc<InductiveDecl>,
    params: &[Val],
    motive: &Val,
    ctx: &EvalCtx,
) -> Result<Vec<Val>, EvalError> {
    (0..decl.ctors.len())
        .map(|i| derive_minor_type(decl, i, params, motive, ctx))
        .collect()
}

/// Derive the expected type of a single minor for constructor index
/// `ctor_idx` of `decl`.
pub fn derive_minor_type(
    decl: &Arc<InductiveDecl>,
    ctor_idx: usize,
    params: &[Val],
    motive: &Val,
    ctx: &EvalCtx,
) -> Result<Val, EvalError> {
    if params.len() != decl.params.len() {
        return Err(EvalError::InvalidCaseTarget(format!(
            "derive_minor_type for `{}.{}`: expected {} params, got {}",
            decl.name,
            decl.ctors[ctor_idx].name,
            decl.params.len(),
            params.len()
        )));
    }
    if ctor_idx >= decl.ctors.len() {
        return Err(EvalError::InvalidCaseTarget(format!(
            "derive_minor_type: ctor_idx {} out of range for `{}` (has {} ctors)",
            ctor_idx,
            decl.name,
            decl.ctors.len()
        )));
    }

    let ctor = &decl.ctors[ctor_idx];

    // Collect non-parameter binders from the constructor's Π-telescope.
    // Handles both ordinary Pi and bounded-size SizedPi binders; the
    // latter are preserved in the generated minor so that the user's
    // minor body gets the `bound < upper` hypothesis available.
    let mut current = &ctor.typ;
    let mut params_to_skip = decl.params.len();
    let mut arg_specs: Vec<MinorArg> = Vec::new();
    loop {
        match current {
            Exp::Pi(patt, dom, body) => {
                if params_to_skip > 0 {
                    params_to_skip -= 1;
                } else {
                    arg_specs.push(MinorArg::Value {
                        patt: patt.clone(),
                        typ: (**dom).clone(),
                    });
                }
                current = body;
            }
            Exp::SizedPi { patt, upper, body } => {
                // Size binders never appear in the param prefix.
                arg_specs.push(MinorArg::Size {
                    patt: patt.clone(),
                    upper: (**upper).clone(),
                });
                current = body;
            }
            _ => break,
        }
    }

    // Pick a stable, fresh variable name for each non-param arg. We
    // need names so the IH bindings and the constructor application in
    // the result type can refer back to them. Original `Patt::Var`
    // names are reused; anonymous binders get `__a_<idx>`.
    let arg_names: Vec<String> = arg_specs
        .iter()
        .enumerate()
        .map(|(i, a)| match a.patt() {
            Patt::Var(n) => n.clone(),
            _ => format!("__a_{i}"),
        })
        .collect();
    let arg_var_exps: Vec<Exp> = arg_names.iter().map(|n| Exp::Var(n.clone())).collect();

    // Read back the motive into an Exp so we can splice it into the
    // generated Π-chain and re-evaluate. Closed motives round-trip
    // exactly; neutral motives also round-trip via their generated
    // variable names.
    let motive_exp = readback_val(0, motive);

    // Result type: motive(cⱼ args)
    let ctor_app = Exp::InductiveCtor(decl.clone(), ctor.name.clone(), arg_var_exps.clone());
    let mut body_exp = Exp::App(Box::new(motive_exp.clone()), Box::new(ctor_app));

    // Wrap one IH binder per recursive argument, in original order
    // (rev iteration so the first recursive arg ends up outermost
    // among the IHs, matching iota_reduce's application order).
    // Only `MinorArg::Value` entries can be recursive occurrences —
    // size binders always have domain `SizeSort`.
    let recursive_indices: Vec<usize> = arg_specs
        .iter()
        .enumerate()
        .filter(
            |(_, a)| matches!(a, MinorArg::Value { typ, .. } if is_direct_recursive_ref(decl, typ)),
        )
        .map(|(i, _)| i)
        .collect();
    for (rec_pos, &arg_idx) in recursive_indices.iter().enumerate().rev() {
        let arg_var = arg_var_exps[arg_idx].clone();
        let ih_typ = Exp::App(Box::new(motive_exp.clone()), Box::new(arg_var));
        body_exp = Exp::Pi(
            Patt::Var(format!("__ih_{rec_pos}")),
            Box::new(ih_typ),
            Box::new(body_exp),
        );
    }

    // Wrap the constructor argument binders, in reverse so the first
    // arg ends up outermost. Preserves SizedPi for Size args so the
    // minor's body gets the bound hypothesis available via the same
    // check-mode plumbing used on any `SizedPi`-typed value.
    for (i, spec) in arg_specs.iter().enumerate().rev() {
        let binder_patt = Patt::Var(arg_names[i].clone());
        body_exp = match spec {
            MinorArg::Value { typ, .. } => {
                Exp::Pi(binder_patt, Box::new(typ.clone()), Box::new(body_exp))
            }
            MinorArg::Size { upper, .. } => Exp::SizedPi {
                patt: binder_patt,
                upper: Box::new(upper.clone()),
                body: Box::new(body_exp),
            },
        };
    }

    // Evaluate in an environment that binds parameter names to their
    // concrete values. Constructor argument binder types may reference
    // parameter names (e.g. `cons` has binder `_:A` referring to the
    // bound parameter `A`); the param substitution happens through
    // normal `eval` lookup.
    let mut env = Rho::Nil;
    for ((patt, _), val) in decl.params.iter().zip(params.iter()) {
        env = env.extend(patt.clone(), val.clone());
    }
    eval_ctx(&body_exp, &env, ctx)
}

/// Whether `typ` is a direct application of `decl` — the only shape
/// of recursive constructor argument that Phase 11b's iota reduction
/// can eliminate.
///
/// Duplicated from `eval::is_recursive_arg_type` until Phase 11b's
/// helpers are deduplicated in a follow-up pass.
fn is_direct_recursive_ref(decl: &InductiveDecl, typ: &Exp) -> bool {
    matches!(typ, Exp::InductiveType(d, _) if d.name == decl.name)
}

/// One constructor arg in the minor-derivation telescope.
///
/// Mirror of `check::CtorArg` — kept separate so recursor.rs stays
/// independent of check.rs. Consolidate if a third site emerges.
#[derive(Debug, Clone)]
enum MinorArg {
    Value { patt: Patt, typ: Exp },
    Size { patt: Patt, upper: Exp },
}

impl MinorArg {
    fn patt(&self) -> &Patt {
        match self {
            MinorArg::Value { patt, .. } | MinorArg::Size { patt, .. } => patt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::InductiveCtorDecl;
    use crate::nbe::val::Clos;

    fn self_ref(name: &str) -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            name: name.to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        })
    }

    fn nat_decl() -> Arc<InductiveDecl> {
        let s = self_ref("Nat");
        let nat_ty = Exp::InductiveType(s, Vec::new());
        Arc::new(InductiveDecl {
            name: "Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "zero".to_string(),
                    typ: nat_ty.clone(),
                },
                InductiveCtorDecl {
                    name: "succ".to_string(),
                    typ: Exp::Pi(Patt::Unit, Box::new(nat_ty.clone()), Box::new(nat_ty)),
                },
            ],
        })
    }

    /// Constant motive `λ_. Set`. Applied to anything, returns `Val::Sort(1)`.
    fn const_set_motive() -> Val {
        Val::Lam(Clos::new(Patt::Unit, Exp::Sort(1), Rho::Nil))
    }

    /// Walk a `Val::Pi` chain, applying generated variables, and return
    /// `(domain_count, final_body)`.
    fn count_pi_chain(typ: Val) -> (usize, Val) {
        let mut count = 0usize;
        let mut current = typ;
        loop {
            match current {
                Val::Pi(_, clos) => {
                    count += 1;
                    let gen = Val::Nt(crate::nbe::val::Neut::Gen(count, format!("v{count}")));
                    current = clos.apply(gen).expect("apply pi clos");
                }
                other => return (count, other),
            }
        }
    }

    #[test]
    fn nat_zero_minor_type_is_motive_at_zero() {
        // motive = const Set ⇒ motive(zero) = Set; zero has no args.
        let nat = nat_decl();
        let motive = const_set_motive();
        let typ =
            derive_minor_type(&nat, 0, &[], &motive, &EvalCtx::Pure).expect("derive_minor_type");
        assert!(matches!(typ, Val::Sort(1)), "expected Set, got {typ:?}");
    }

    #[test]
    fn nat_succ_minor_type_has_two_pis() {
        // succ has one direct recursive arg ⇒ minor type is Π n:Nat. Π ih:motive(n). motive(succ n)
        let nat = nat_decl();
        let motive = const_set_motive();
        let typ =
            derive_minor_type(&nat, 1, &[], &motive, &EvalCtx::Pure).expect("derive_minor_type");
        let (count, body) = count_pi_chain(typ);
        assert_eq!(count, 2, "expected 2 Π binders, got {count}");
        assert!(
            matches!(body, Val::Sort(1)),
            "expected final body Set, got {body:?}"
        );
    }

    #[test]
    fn list_cons_minor_type_has_three_pis() {
        // List(A) cons has args [elem:A, rest:List A], one recursive ⇒
        // minor type is Π elem:A. Π rest:List(A). Π ih:motive(rest). motive(cons elem rest)
        let s = self_ref("List");
        let list_ty = Exp::InductiveType(s, vec![Exp::Var("A".to_string())]);
        let list = Arc::new(InductiveDecl {
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "nil".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::Sort(1)),
                        Box::new(list_ty.clone()),
                    ),
                },
                InductiveCtorDecl {
                    name: "cons".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::Sort(1)),
                        Box::new(Exp::Pi(
                            Patt::Unit,
                            Box::new(Exp::Var("A".to_string())),
                            Box::new(Exp::Pi(
                                Patt::Unit,
                                Box::new(list_ty.clone()),
                                Box::new(list_ty),
                            )),
                        )),
                    ),
                },
            ],
        });
        // Use Val::Sort(1) as the concrete param value (i.e. List(Set)). This
        // suffices for the shape check; element types do not matter for
        // counting Π binders.
        let motive = const_set_motive();
        let typ = derive_minor_type(&list, 1, &[Val::Sort(1)], &motive, &EvalCtx::Pure)
            .expect("derive_minor_type");
        let (count, body) = count_pi_chain(typ);
        assert_eq!(count, 3, "expected 3 Π binders, got {count}");
        assert!(
            matches!(body, Val::Sort(1)),
            "expected final body Set, got {body:?}"
        );
    }

    #[test]
    fn list_nil_minor_type_no_pis() {
        // nil has no non-param args ⇒ minor type = motive(nil)
        let s = self_ref("List");
        let list_ty = Exp::InductiveType(s, vec![Exp::Var("A".to_string())]);
        let list = Arc::new(InductiveDecl {
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "nil".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Sort(1)),
                    Box::new(list_ty),
                ),
            }],
        });
        let motive = const_set_motive();
        let typ = derive_minor_type(&list, 0, &[Val::Sort(1)], &motive, &EvalCtx::Pure)
            .expect("derive_minor_type");
        assert!(matches!(typ, Val::Sort(1)), "expected Set, got {typ:?}");
    }

    #[test]
    fn derive_minor_types_returns_one_per_constructor() {
        let nat = nat_decl();
        let motive = const_set_motive();
        let typs =
            derive_minor_types(&nat, &[], &motive, &EvalCtx::Pure).expect("derive_minor_types");
        assert_eq!(typs.len(), 2);
        // zero minor: Set
        assert!(matches!(&typs[0], Val::Sort(1)));
        // succ minor: Pi(_, Pi(_, Set))
        let (count, _) = count_pi_chain(typs[1].clone());
        assert_eq!(count, 2);
    }

    #[test]
    fn param_count_mismatch_errors() {
        let nat = nat_decl();
        let motive = const_set_motive();
        // Nat takes no params; passing one should error.
        let err = derive_minor_type(&nat, 0, &[Val::Sort(1)], &motive, &EvalCtx::Pure).unwrap_err();
        match err {
            EvalError::InvalidCaseTarget(msg) => assert!(msg.contains("params")),
            other => panic!("expected InvalidCaseTarget, got {other:?}"),
        }
    }
}
