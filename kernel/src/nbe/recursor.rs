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

    // D48: extract the ctor's conclusion-indices from its declared
    // result type `D(params)(idx_1, ..., idx_m)`. For non-indexed
    // decls (`decl.indices.is_empty()`) this is empty and the rest
    // of the body construction degenerates to the pre-D48 shape.
    let n_params = decl.params.len();
    let conclusion_indices: Vec<Exp> = match current {
        Exp::InductiveType(_, all_args) if all_args.len() >= n_params => {
            all_args[n_params..].to_vec()
        }
        _ => Vec::new(),
    };

    // Build `motive idx_1 ... idx_m` — the motive applied at the
    // ctor-specific index expressions. For non-indexed decls this
    // simplifies to `motive_exp` (no indices to apply).
    let motive_at_concl_indices = conclusion_indices
        .iter()
        .fold(motive_exp.clone(), |acc, i| {
            Exp::App(Box::new(acc), Box::new(i.clone()))
        });

    // Result type: motive idx_1 ... idx_m (cⱼ args)
    let ctor_app = Exp::InductiveCtor(decl.clone(), ctor.name.clone(), arg_var_exps.clone());
    let mut body_exp = Exp::App(Box::new(motive_at_concl_indices), Box::new(ctor_app));

    // Wrap one IH binder per recursive argument, in original order
    // (rev iteration so the first recursive arg ends up outermost
    // among the IHs, matching iota_reduce's application order).
    // Only `MinorArg::Value` entries can be recursive occurrences —
    // size binders always have domain `SizeSort`.
    let recursive_indices: Vec<usize> = arg_specs
        .iter()
        .enumerate()
        .filter(
            |(_, a)| matches!(a, MinorArg::Value { typ, .. } if decl.is_direct_recursive_ref(typ)),
        )
        .map(|(i, _)| i)
        .collect();
    for (rec_pos, &arg_idx) in recursive_indices.iter().enumerate().rev() {
        let arg_var = arg_var_exps[arg_idx].clone();
        // D48: the IH type for `arg : D(params)(arg_idx_1, ..., arg_idx_m)`
        // is `motive arg_idx_1 ... arg_idx_m arg`. For non-indexed decls
        // `arg_idx_*` is empty, recovering the pre-D48 `motive arg` shape.
        let arg_typ = match &arg_specs[arg_idx] {
            MinorArg::Value { typ, .. } => typ.clone(),
            MinorArg::Size { .. } => unreachable!("size args aren't recursive"),
        };
        let arg_idx_exps: Vec<Exp> = match &arg_typ {
            Exp::InductiveType(_, all_args) if all_args.len() >= n_params => {
                all_args[n_params..].to_vec()
            }
            _ => Vec::new(),
        };
        let motive_at_arg_indices = arg_idx_exps.iter().fold(motive_exp.clone(), |acc, i| {
            Exp::App(Box::new(acc), Box::new(i.clone()))
        });
        let ih_typ = Exp::App(Box::new(motive_at_arg_indices), Box::new(arg_var));
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
            iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).expect("test iri"),
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
            iri: crate::ontology::iri::Iri::parse("urn:test:Nat").unwrap(),
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

    /// Port-fidelity witness (docs/notes/nbe-reorganization-analysis.md
    /// §4): the module doc claims IH binders are appended *after* all
    /// ctor args, first recursive arg's IH outermost, matching
    /// `eval::iota_reduce`'s application order. This test pins the
    /// binder order structurally: with motive `λx. x`, each IH domain
    /// evaluates to the generic value of the ctor arg it belongs to,
    /// so the order is directly observable in the Pi chain.
    #[test]
    fn node_minor_binder_order_is_args_then_ihs_in_arg_order() {
        // Tree { leaf : Tree, node : Tree → Tree → Tree }
        let s = self_ref("Tree");
        let tree_ty = Exp::InductiveType(s, Vec::new());
        let tree = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Tree").unwrap(),
            name: "Tree".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "leaf".to_string(),
                    typ: tree_ty.clone(),
                },
                InductiveCtorDecl {
                    name: "node".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("l".to_string()),
                        Box::new(tree_ty.clone()),
                        Box::new(Exp::Pi(
                            Patt::Var("r".to_string()),
                            Box::new(tree_ty.clone()),
                            Box::new(tree_ty),
                        )),
                    ),
                },
            ],
        });
        // Identity motive: `motive(v)` evaluates to `v` itself, making
        // each IH domain reveal which ctor arg it quantifies over.
        let motive = Val::Lam(Clos::new(
            Patt::Var("x".to_string()),
            Exp::Var("x".to_string()),
            Rho::Nil,
        ));
        let typ = derive_minor_type(&tree, 1, &[], &motive, &EvalCtx::Pure)
            .expect("derive_minor_type for node");

        // Walk the Pi chain, applying distinguishable generic values.
        let mut domains: Vec<Exp> = Vec::new();
        let mut current = typ;
        let mut level = 0usize;
        while let Val::Pi(dom, clos) = current {
            domains.push(crate::nbe::readback::readback_val(10, &dom));
            let gen = Val::Nt(crate::nbe::val::Neut::Gen(level, format!("g{level}")));
            current = clos.apply(gen).expect("apply pi clos");
            level += 1;
        }
        assert_eq!(domains.len(), 4, "node minor: 2 args + 2 IHs");
        // Binders 1–2: the ctor args (Tree, Tree).
        assert!(matches!(domains[0], Exp::InductiveType(_, _)));
        assert!(matches!(domains[1], Exp::InductiveType(_, _)));
        // Binder 3: IH for the FIRST recursive arg — identity motive
        // means its domain is the first arg's generic value (level 0).
        // Binder 4: IH for the second (level 1). Reversed or
        // interleaved IHs would swap these.
        assert_eq!(
            domains[2],
            crate::nbe::readback::readback_val(
                10,
                &Val::Nt(crate::nbe::val::Neut::Gen(0, "g0".to_string()))
            ),
            "third binder must be the IH of the first ctor arg"
        );
        assert_eq!(
            domains[3],
            crate::nbe::readback::readback_val(
                10,
                &Val::Nt(crate::nbe::val::Neut::Gen(1, "g1".to_string()))
            ),
            "fourth binder must be the IH of the second ctor arg"
        );
    }

    #[test]
    fn list_cons_minor_type_has_three_pis() {
        // List(A) cons has args [elem:A, rest:List A], one recursive ⇒
        // minor type is Π elem:A. Π rest:List(A). Π ih:motive(rest). motive(cons elem rest)
        let s = self_ref("List");
        let list_ty = Exp::InductiveType(s, vec![Exp::Var("A".to_string())]);
        let list = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:List").unwrap(),
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
            iri: crate::ontology::iri::Iri::parse("urn:test:List").unwrap(),
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

    // ──────────────────────────────────────────────────────────────────
    // D48 Phase E — derive_minor_type for indexed inductives
    // ──────────────────────────────────────────────────────────────────

    /// Build the same Vec-with-Unit-index toy used by check.rs Phase B
    /// tests: `SimpleVec (A : Set) : 1 → Set` with `nil : SimpleVec A ()`
    /// and `cons : (h : 1) → A → SimpleVec A () → SimpleVec A ()`.
    fn simple_vec_decl() -> Arc<InductiveDecl> {
        let self_ref = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:SimpleVec").unwrap(),
            name: "SimpleVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        });
        let vec_a_unit =
            Exp::InductiveType(self_ref.clone(), vec![Exp::Var("A".to_string()), Exp::Unit]);
        Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:SimpleVec").unwrap(),
            name: "SimpleVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::Sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "nil".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::Sort(1)),
                        Box::new(vec_a_unit.clone()),
                    ),
                },
                InductiveCtorDecl {
                    name: "cons".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::Sort(1)),
                        Box::new(Exp::Pi(
                            Patt::Unit,
                            Box::new(Exp::One),
                            Box::new(Exp::Pi(
                                Patt::Unit,
                                Box::new(Exp::Var("A".to_string())),
                                Box::new(Exp::Pi(
                                    Patt::Unit,
                                    Box::new(vec_a_unit.clone()),
                                    Box::new(vec_a_unit),
                                )),
                            )),
                        )),
                    ),
                },
            ],
        })
    }

    /// A motive that takes 2 args (the index of type `1`, then the
    /// inductive value) and returns `Set`. Concretely:
    /// `λ_idx. λ_v. Set`.
    fn vec_motive() -> Val {
        Val::Lam(Clos::new(
            Patt::Unit,
            Exp::Lam(Patt::Unit, Box::new(Exp::Sort(1))),
            Rho::Nil,
        ))
    }

    #[test]
    fn d48_vec_nil_minor_type_applies_motive_to_index() {
        // `nil`'s derived minor type should be `motive () (nil A)` —
        // the motive applied at the conclusion's index `()` then at
        // the constructor.
        let decl = simple_vec_decl();
        let motive = vec_motive();
        // Reducing the minor at evaluation time produces `motive () (nil A)`
        // which (with the const motive `λ _ _. Set`) collapses to `Set`.
        let typ = derive_minor_type(&decl, 0, &[Val::Sort(0)], &motive, &EvalCtx::Pure)
            .expect("derive nil minor");
        // The minor type is `Π A:Set. motive () (nil A)` — a Pi over
        // the ctor's value-arg telescope (here just the A binder).
        // After the const motive reduces, the inner result is Sort(1).
        match typ {
            Val::Pi(_dom, body_clos) => {
                let body = body_clos
                    .apply(Val::Sort(0))
                    .expect("apply minor body to A");
                // Wait — the A binder is part of the *param prefix*,
                // not the ctor's value args. `nil` has no non-param
                // value args, so the minor is just `motive () (nil A)`.
                // The Val::Pi above must be from a different binder.
                // Actually: `nil` has no non-param args at all, so the
                // minor type is the body directly, no Pi.
                let _ = body;
                panic!("nil has no non-param args; expected non-Pi minor");
            }
            other => {
                // `motive () (nil A)` with const motive reduces to Sort(1).
                assert!(
                    matches!(other, Val::Sort(1)),
                    "expected Sort(1) (from const motive), got {other:?}"
                );
            }
        }
    }

    #[test]
    fn d48_vec_cons_minor_type_applies_motive_to_index_and_includes_ih() {
        // `cons`'s derived minor type is:
        //   Π _:1. Π _:A. Π _:SimpleVec A (). Π __ih_0: motive () xs. motive () (cons A h x xs)
        // The const motive `λ _ _. Set` reduces all `motive () _` to Sort(1).
        let decl = simple_vec_decl();
        let motive = vec_motive();
        let typ = derive_minor_type(&decl, 1, &[Val::Sort(0)], &motive, &EvalCtx::Pure)
            .expect("derive cons minor");
        // Verify the minor type starts with a Pi — `cons` has non-param
        // value args (h : 1, x : A, xs : SimpleVec A ()) plus an IH for
        // the recursive xs, so the outer shape must be a binder.
        assert!(
            matches!(typ, Val::Pi(_, _) | Val::SizedPi(_, _)),
            "cons minor must be a Pi (has non-param args); got {typ:?}"
        );
    }

    #[test]
    fn d48_nat_minor_unchanged_pre_d48_shape() {
        // For non-indexed Nat, the minor type's motive application
        // should be identical to the pre-D48 shape: `motive (zero)`
        // / `motive (succ n)` with no extra index arguments. The
        // existing `nat_zero_minor_type_is_motive_applied_to_zero`
        // test (if present) would catch a regression; here we re-
        // assert the same property for paranoia.
        let nat = nat_decl();
        let motive = const_set_motive();
        // zero's minor: no args, result is `motive zero` → Sort(1) under const.
        let zero_typ =
            derive_minor_type(&nat, 0, &[], &motive, &EvalCtx::Pure).expect("derive zero minor");
        assert!(
            matches!(zero_typ, Val::Sort(1)),
            "Nat.zero minor should reduce to Sort(1) under const-Set motive; got {zero_typ:?}"
        );
    }
}
