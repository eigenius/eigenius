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
//! A recursive argument may be DIRECT (`D(params)(idx…)`) or HIGHER-ORDER POSITIVE
//! (`(b₁:B₁) → … → D(params)(idx…)`, eigenius#92). Both contribute an IH; the higher-order one's
//! is itself a Π — `Π b₁:B₁ … B_k. C(idx…) (arg b₁ … b_k)`. `positivity::recursive_arg_shape` is
//! the single classifier, shared with `eval::iota_reduce_impl`, so the binders emitted here and
//! the applications made there cannot drift apart.
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
    while let Exp::Pi(patt, dom, body) = current {
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

    // Pick a stable, fresh variable name for each non-param arg. We
    // need names so the IH bindings and the constructor application in
    // the result type can refer back to them. Original `Patt::Var`
    // names are reused; anonymous binders get `__a_<idx>`.
    let arg_names: Vec<String> = arg_specs
        .iter()
        .enumerate()
        .map(|(i, a)| match a.patt() {
            Patt::Var(n) => n.clone(),
            _ => format!("A#{i}"),
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
    let conclusion_indices: Vec<Exp> = match current.as_const_spine() {
        Some((_, _, all_args)) if all_args.len() >= n_params => {
            all_args[n_params..].iter().map(|e| (*e).clone()).collect()
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
    let ctor_app = Exp::InductiveCtor(decl.iri.clone(), ctor.name.clone(), arg_var_exps.clone());
    let mut body_exp = Exp::App(Box::new(motive_at_concl_indices), Box::new(ctor_app));

    // Wrap one IH binder per recursive argument, in original order
    // (rev iteration so the first recursive arg ends up outermost
    // among the IHs, matching iota_reduce's application order).
    // Only `MinorArg::Value` entries can be recursive occurrences.
    let recursive_indices: Vec<usize> = arg_specs
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            // eigenius#92: `positivity::recursive_arg_shape` is the ONE definition of "this
            // argument is a recursive occurrence"; iota consults the same function, so the
            // minor's binders and the reduction's applications cannot drift apart.
            //
            // Step 2 removed the `is_direct()` guard that stood here: a higher-order positive
            // argument now contributes an IH too, of FUNCTION type. See the loop below.
            matches!(a, MinorArg::Value { typ, .. }
                if crate::nbe::positivity::recursive_arg_shape(decl, typ).is_some())
        })
        .map(|(i, _)| i)
        .collect();
    for (rec_pos, &arg_idx) in recursive_indices.iter().enumerate().rev() {
        let arg_var = arg_var_exps[arg_idx].clone();
        let arg_typ = match &arg_specs[arg_idx] {
            MinorArg::Value { typ, .. } => typ.clone(),
        };
        let shape = crate::nbe::positivity::recursive_arg_shape(decl, &arg_typ)
            .expect("`recursive_indices` filtered on exactly this");

        // The occurrence's binders (eigenius#92 step 2). Empty for a DIRECT recursive argument
        // `D(params)(idx…)`, non-empty for a higher-order positive one
        // `(b₁ : B₁) → … → (b_k : B_k) → D(params)(idx…)`.
        //
        // A named binder keeps its declared name, because the occurrence's index expressions are
        // written against it — `(b : Nat) → D(params)(f b)` refers to `b`. An anonymous binder
        // (`Nat → D`, the common shape) gets `HB#{rec_pos}_{j}`: the IH has to APPLY `arg` to it,
        // which needs a name, and `#` cannot occur in an ESL identifier, so nothing the
        // declaration could name collides with it.
        let binder_names: Vec<String> = shape
            .binders
            .iter()
            .enumerate()
            .map(|(j, (patt, _))| match patt {
                Patt::Var(n) => n.clone(),
                _ => format!("HB#{rec_pos}_{j}"),
            })
            .collect();

        // D48: `motive idx₁ … idx_m` at the occurrence's own indices. These sit INSIDE the
        // binders above, since an index expression may mention them.
        let arg_idx_exps: Vec<Exp> = shape.args[n_params.min(shape.args.len())..]
            .iter()
            .map(|e| (*e).clone())
            .collect();
        let motive_at_arg_indices = arg_idx_exps.iter().fold(motive_exp.clone(), |acc, i| {
            Exp::App(Box::new(acc), Box::new(i.clone()))
        });
        // `arg b₁ … b_k` — for a direct argument this is just `arg`.
        let applied_arg = binder_names.iter().fold(arg_var, |acc, n| {
            Exp::App(Box::new(acc), Box::new(Exp::Var(n.clone())))
        });
        let mut ih_typ = Exp::App(Box::new(motive_at_arg_indices), Box::new(applied_arg));
        // Wrap `Π b₁:B₁ … Π b_k:B_k.` around it, outermost first.
        for ((_, b_typ), name) in shape.binders.iter().zip(binder_names.iter()).rev() {
            ih_typ = Exp::Pi(
                Patt::Var(name.clone()),
                Box::new((*b_typ).clone()),
                Box::new(ih_typ),
            );
        }
        body_exp = Exp::Pi(
            // `IH#{rec_pos}`, not the former `__ih_{rec_pos}`: the IH type mentions the
            // constructor's argument names, so a constructor argument named `__ih_0` captured
            // them. Same discipline as `gen_val`'s `TC#` and readback's `G#`.
            Patt::Var(format!("IH#{rec_pos}")),
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
}

impl MinorArg {
    fn patt(&self) -> &Patt {
        match self {
            MinorArg::Value { patt, .. } => patt,
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
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).expect("test iri"),
            name: name.to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: Vec::new(),
        })
    }

    fn nat_decl() -> Arc<InductiveDecl> {
        let s = self_ref("Nat");
        let nat_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
        Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:Nat").unwrap(),
            name: "Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "zero".to_string(),
                    typ: nat_ty.clone(),
                },
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "succ".to_string(),
                    typ: Exp::Pi(Patt::Unit, Box::new(nat_ty.clone()), Box::new(nat_ty)),
                },
            ],
        })
    }

    /// Constant motive `λ_. Set`. Applied to anything, returns `Val::sort(1)`.
    fn const_set_motive() -> Val {
        Val::Lam(Clos::new(Patt::Unit, Exp::sort(1), Rho::Nil))
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

    /// **eigenius#92 step 2 — the IH binder cannot capture a constructor argument.**
    ///
    /// The minor's shape is `Π args… Π ihs… motive (c args…)`, so the IH binders wrap a conclusion
    /// that refers to the constructor's arguments BY NAME. The IH binder used to be
    /// `__ih_{rec_pos}` — a legal ESL identifier — so a constructor argument of that name was
    /// shadowed, and the conclusion `motive (c __ih_0 …)` picked up the induction hypothesis
    /// instead of the argument. A wrong term in the conclusion, silently, with no diagnostic.
    ///
    /// `step : (__ih_0 : One) → D → D` is that constructor. The recursive second argument produces
    /// one IH, which under the old name was also `__ih_0` and sat innermost.
    ///
    /// A constant motive cannot see the difference, so the motive here is `λx. Id(D, x, x)` — it
    /// puts its argument in the result, letting the test read which variable reached the
    /// constructor application. The binder is now `IH#{rec_pos}`, and `#` cannot occur in an ESL
    /// identifier (`esl/lexer.rs:485`).
    #[test]
    fn ih_binder_does_not_capture_a_constructor_argument() {
        let s = self_ref("D");
        let d_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
        let decl = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:D").unwrap(),
            name: "D".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![
                crate::nbe::term::InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "base".to_string(),
                    typ: d_ty.clone(),
                },
                crate::nbe::term::InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "step".to_string(),
                    // (__ih_0 : One) -> D -> D
                    typ: Exp::Pi(
                        Patt::Var("__ih_0".to_string()),
                        Box::new(Exp::One),
                        Box::new(Exp::Pi(
                            Patt::Unit,
                            Box::new(d_ty.clone()),
                            Box::new(d_ty.clone()),
                        )),
                    ),
                },
            ],
        });

        // motive = λx. Id(D, x, x) — surfaces its argument in the result type.
        let motive = Val::Lam(Clos::new(
            Patt::Var("x".to_string()),
            Exp::Id(
                Box::new(d_ty),
                Box::new(Exp::Var("x".to_string())),
                Box::new(Exp::Var("x".to_string())),
            ),
            Rho::Nil,
        ));

        let minor = derive_minor_type(&decl, 1, &[], &motive, &EvalCtx::pure())
            .expect("derive_minor_type for `step`");

        // Walk the three binders — the `One` argument, the recursive `D` argument, the IH —
        // applying a distinguishable value to each, and read back the conclusion.
        let mut cursor = minor;
        let mut applied = Vec::new();
        while let Val::Pi(_, clos) = cursor {
            let n = applied.len() + 1;
            let gen = Val::Nt(crate::nbe::val::Neut::Gen(n, format!("arg{n}_")));
            applied.push(gen.clone());
            cursor = clos.apply(gen).expect("apply pi clos");
        }
        assert_eq!(applied.len(), 3, "two ctor args plus one IH");

        // Conclusion is `Id(D, step a b, step a b)`; pull out the constructor application.
        let body = readback_val(0, &cursor);
        let Exp::Id(_, lhs, _) = body else {
            panic!("motive is `λx. Id(D, x, x)`, so the conclusion is an Id; got {body:?}")
        };
        let Exp::InductiveCtor(_, ctor, ctor_args) = *lhs else {
            panic!("the Id's endpoint is the constructor application")
        };
        assert_eq!(ctor, "step");
        assert_eq!(ctor_args.len(), 2);
        // The FIRST binder — the `One` argument named `__ih_0` — must be what reaches the
        // constructor's first slot. Under the old binder name the THIRD (the IH) did.
        assert_eq!(
            ctor_args[0],
            readback_val(0, &applied[0]),
            "the constructor's `__ih_0` argument must be the one declared, not the induction \
             hypothesis that shadowed it; got {:?}",
            ctor_args[0]
        );
        assert_ne!(
            ctor_args[0],
            readback_val(0, &applied[2]),
            "the IH must not appear in the constructor application"
        );
    }

    #[test]
    fn nat_zero_minor_type_is_motive_at_zero() {
        // motive = const Set ⇒ motive(zero) = Set; zero has no args.
        let nat = nat_decl();
        let motive = const_set_motive();
        let typ =
            derive_minor_type(&nat, 0, &[], &motive, &EvalCtx::pure()).expect("derive_minor_type");
        assert!(
            matches!(&typ, Val::Sort(l) if l.is_nat(1)),
            "expected Set, got {typ:?}"
        );
    }

    #[test]
    fn nat_succ_minor_type_has_two_pis() {
        // succ has one direct recursive arg ⇒ minor type is Π n:Nat. Π ih:motive(n). motive(succ n)
        let nat = nat_decl();
        let motive = const_set_motive();
        let typ =
            derive_minor_type(&nat, 1, &[], &motive, &EvalCtx::pure()).expect("derive_minor_type");
        let (count, body) = count_pi_chain(typ);
        assert_eq!(count, 2, "expected 2 Π binders, got {count}");
        assert!(
            matches!(&body, Val::Sort(l) if l.is_nat(1)),
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
        let tree_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
        let tree = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:Tree").unwrap(),
            name: "Tree".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "leaf".to_string(),
                    typ: tree_ty.clone(),
                },
                InductiveCtorDecl {
                    implicit: Vec::new(),
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
        let typ = derive_minor_type(&tree, 1, &[], &motive, &EvalCtx::pure())
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
        assert!(domains[0].as_const_spine().is_some());
        assert!(domains[1].as_const_spine().is_some());
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
        let list_ty =
            Exp::const_applied(s.iri.clone(), Vec::new(), vec![Exp::Var("A".to_string())]);
        let list = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:List").unwrap(),
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "nil".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::sort(1)),
                        Box::new(list_ty.clone()),
                    ),
                },
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "cons".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::sort(1)),
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
        // Use Val::sort(1) as the concrete param value (i.e. List(Set)). This
        // suffices for the shape check; element types do not matter for
        // counting Π binders.
        let motive = const_set_motive();
        let typ = derive_minor_type(&list, 1, &[Val::sort(1)], &motive, &EvalCtx::pure())
            .expect("derive_minor_type");
        let (count, body) = count_pi_chain(typ);
        assert_eq!(count, 3, "expected 3 Π binders, got {count}");
        assert!(
            matches!(&body, Val::Sort(l) if l.is_nat(1)),
            "expected final body Set, got {body:?}"
        );
    }

    #[test]
    fn list_nil_minor_type_no_pis() {
        // nil has no non-param args ⇒ minor type = motive(nil)
        let s = self_ref("List");
        let list_ty =
            Exp::const_applied(s.iri.clone(), Vec::new(), vec![Exp::Var("A".to_string())]);
        let list = Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:List").unwrap(),
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                implicit: Vec::new(),
                name: "nil".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::sort(1)),
                    Box::new(list_ty),
                ),
            }],
        });
        let motive = const_set_motive();
        let typ = derive_minor_type(&list, 0, &[Val::sort(1)], &motive, &EvalCtx::pure())
            .expect("derive_minor_type");
        assert!(
            matches!(&typ, Val::Sort(l) if l.is_nat(1)),
            "expected Set, got {typ:?}"
        );
    }

    #[test]
    fn derive_minor_types_returns_one_per_constructor() {
        let nat = nat_decl();
        let motive = const_set_motive();
        let typs =
            derive_minor_types(&nat, &[], &motive, &EvalCtx::pure()).expect("derive_minor_types");
        assert_eq!(typs.len(), 2);
        // zero minor: Set
        assert!(matches!(&&typs[0], Val::Sort(l) if l.is_nat(1)));
        // succ minor: Pi(_, Pi(_, Set))
        let (count, _) = count_pi_chain(typs[1].clone());
        assert_eq!(count, 2);
    }

    #[test]
    fn param_count_mismatch_errors() {
        let nat = nat_decl();
        let motive = const_set_motive();
        // Nat takes no params; passing one should error.
        let err =
            derive_minor_type(&nat, 0, &[Val::sort(1)], &motive, &EvalCtx::pure()).unwrap_err();
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
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:SimpleVec").unwrap(),
            name: "SimpleVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        let vec_a_unit = Exp::const_applied(
            self_ref.iri.clone(),
            Vec::new(),
            vec![Exp::Var("A".to_string()), Exp::Unit],
        );
        Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse("urn:test:SimpleVec").unwrap(),
            name: "SimpleVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "nil".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::sort(1)),
                        Box::new(vec_a_unit.clone()),
                    ),
                },
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "cons".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::sort(1)),
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
            Exp::Lam(Patt::Unit, Box::new(Exp::sort(1))),
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
        let typ = derive_minor_type(&decl, 0, &[Val::sort(0)], &motive, &EvalCtx::pure())
            .expect("derive nil minor");
        // The minor type is `Π A:Set. motive () (nil A)` — a Pi over
        // the ctor's value-arg telescope (here just the A binder).
        // After the const motive reduces, the inner result is Sort(1).
        match typ {
            Val::Pi(_dom, body_clos) => {
                let body = body_clos
                    .apply(Val::sort(0))
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
                    matches!(&other, Val::Sort(l) if l.is_nat(1)),
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
        let typ = derive_minor_type(&decl, 1, &[Val::sort(0)], &motive, &EvalCtx::pure())
            .expect("derive cons minor");
        // Verify the minor type starts with a Pi — `cons` has non-param
        // value args (h : 1, x : A, xs : SimpleVec A ()) plus an IH for
        // the recursive xs, so the outer shape must be a binder.
        assert!(
            matches!(typ, Val::Pi(_, _)),
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
            derive_minor_type(&nat, 0, &[], &motive, &EvalCtx::pure()).expect("derive zero minor");
        assert!(
            matches!(&zero_typ, Val::Sort(l) if l.is_nat(1)),
            "Nat.zero minor should reduce to Sort(1) under const-Set motive; got {zero_typ:?}"
        );
    }
}
