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

//! Recursor runtime: iota-reduction for inductive types (Phase 11b,
//! D19 §6; indexed families per D48). Split from `eval.rs`.

use super::{EvalCtx, EvalError, Tracer};
use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, Patt};
use crate::nbe::val::{Neut, Val};
use std::sync::Arc;

/// Iota reduction for an inductive recursor (Phase 11b step 2, D19 §3.4).
///
/// Reduces `I.rec params motive m₁..mₖ (cⱼ args)` to
/// `mⱼ(args, ih₁, …, ihₘ)` where each `ihᵢ` is the recursor applied to a
/// recursive sub-argument of `cⱼ`. Recursive sub-arguments are identified
/// by walking the constructor's type telescope: any binder (after the
/// parameter prefix) whose type is a self-reference `Exp::InductiveType(I, _)`
/// contributes one IH, computed by recursing into `iota_reduce` for
/// constructor sub-values or producing a blocked `Neut::NtRec` for
/// neutrals.
///
/// Higher-order recursive arguments (e.g. `(Nat → I) → I`) are admitted by the positivity checker
/// since eigenius#92, and since its step 2 they get a FUNCTION-typed hypothesis here —
/// `λ b₁ … b_k. rec … (arg b₁ … b_k)`, matching the `Π b₁:B₁ … B_k.` binder
/// `recursor::derive_minor_type` emits. Pinned by
/// `higher_order_positive_arg_gets_a_function_typed_ih_in_both_sites` and
/// `iota_recurses_through_a_higher_order_argument` below.
pub(super) fn iota_reduce_impl<T: Tracer>(
    decl: &Arc<crate::nbe::term::InductiveDecl>,
    motive: &Val,
    minors: &[Val],
    ctor_name: &str,
    args: &[Val],
    ctx: &EvalCtx,
) -> Result<(Val, T::Node), EvalError> {
    let ctor_idx = decl
        .ctors
        .iter()
        .position(|c| c.name == ctor_name)
        .ok_or_else(|| {
            EvalError::ConstructorNotFound(format!(
                "{}.{ctor_name} (no such constructor in inductive `{}`)",
                decl.name, decl.name
            ))
        })?;

    if minors.len() != decl.ctors.len() {
        return Err(EvalError::InvalidCaseTarget(format!(
            "InductiveRec on `{}`: expected {} minors, got {}",
            decl.name,
            decl.ctors.len(),
            minors.len()
        )));
    }

    let arg_types = extract_ctor_arg_types(decl, &decl.ctors[ctor_idx].typ);
    if arg_types.len() != args.len() {
        return Err(EvalError::InvalidCaseTarget(format!(
            "InductiveRec: constructor `{}.{ctor_name}` expects {} args, got {}",
            decl.name,
            arg_types.len(),
            args.len()
        )));
    }

    // Apply minor to each constructor argument (in original order).
    let mut result = minors[ctor_idx].clone();
    let mut nodes = Vec::new();
    for arg in args {
        let (next, node) = result.app_impl::<T>(arg.clone(), T::leaf(), ctx)?;
        result = next;
        nodes.push(node);
    }

    // Then apply an induction hypothesis for each recursive argument,
    // in the order the recursive arguments appear.
    for (arg, arg_typ) in args.iter().zip(arg_types.iter()) {
        // eigenius#92: one definition of "recursive occurrence", shared with
        // `recursor::derive_minor_type`, so the minor's IH binders and the applications made
        // here cannot drift apart.
        let Some(shape) = crate::nbe::positivity::recursive_arg_shape(decl, arg_typ) else {
            continue;
        };
        let (ih, ih_node) = if shape.is_direct() {
            build_recursor_ih::<T>(decl, motive, minors, arg, ctx)?
        } else {
            // Higher-order positive argument (eigenius#92 step 2): `arg` is a FUNCTION into the
            // inductive, so its induction hypothesis is a function too —
            // `λ b₁ … b_k. rec … (arg b₁ … b_k)` — matching the `Π b₁:B₁ … B_k. motive idx… (arg
            // b₁ … b_k)` binder `derive_minor_type` emits. The recursive call cannot be made here
            // because there is no value to recurse ON until the hypothesis is applied.
            (
                higher_order_ih(decl, motive, minors, arg, shape.binders.len())?,
                T::leaf(),
            )
        };
        nodes.push(ih_node);
        let (next, node) = result.app_impl::<T>(ih, T::leaf(), ctx)?;
        result = next;
        nodes.push(node);
    }

    Ok((result, T::combine(nodes)))
}

/// Untraced iota reduction (test convenience; the evaluator calls the
/// generic form directly).
#[cfg(test)]
pub(super) fn iota_reduce(
    decl: &Arc<crate::nbe::term::InductiveDecl>,
    motive: &Val,
    minors: &[Val],
    ctor_name: &str,
    args: &[Val],
    ctx: &EvalCtx,
) -> Result<Val, EvalError> {
    iota_reduce_impl::<super::NoTrace>(decl, motive, minors, ctor_name, args, ctx).map(|(v, ())| v)
}

/// Walk the constructor's full type expression, skip the parameter
/// prefix, and return the remaining argument types in order.
///
/// The returned slice references the original `Exp` nodes — no
/// substitution is performed. Callers only inspect the syntactic head
/// (specifically, looking for `Exp::InductiveType` to detect recursive
/// arguments), so leaving free variable references intact is fine.
fn extract_ctor_arg_types<'a>(
    decl: &crate::nbe::term::InductiveDecl,
    ctor_typ: &'a Exp,
) -> Vec<&'a Exp> {
    // Sentinel for size-binder positions — these carry a size value
    // at runtime but aren't recursive occurrences, so iota-reduction
    // treats them the same as non-inductive value args (skip IH).
    // Use `SizeSort` itself as the stand-in domain type; only
    // `InductiveDecl::is_direct_recursive_ref` inspects these entries
    // and `SizeSort` is never a recursive reference.
    static SIZE_SORT: Exp = Exp::SizeSort;
    let mut types = Vec::new();
    let mut current = ctor_typ;
    let mut params_to_skip = decl.params.len();
    loop {
        match current {
            Exp::Pi(_, dom, body) => {
                if params_to_skip > 0 {
                    params_to_skip -= 1;
                } else {
                    types.push(dom.as_ref());
                }
                current = body;
            }
            Exp::SizedPi { body, .. } => {
                // Size binders never appear in the param prefix.
                types.push(&SIZE_SORT);
                current = body;
            }
            _ => break,
        }
    }
    types
}

/// Build the induction hypothesis for a recursive constructor argument.
///
/// Either recurses into `iota_reduce` (if the argument is itself a
/// constructor) or produces a blocked `Neut::NtRec` (if the argument
/// is neutral).
/// The induction hypothesis for a HIGHER-ORDER positive recursive argument (eigenius#92 step 2).
///
/// `arg : (b₁ : B₁) → … → (b_k : B_k) → D(params)(idx…)` is a function into the inductive, so
/// there is nothing to recurse on until it is applied. The hypothesis is therefore
/// `λ b₁ … b_k. D.rec motive minors… (arg b₁ … b_k)`, built as a closure whose environment binds
/// the recursor's parts and whose body is an `Exp::InductiveRec`. Reducing that application is
/// then the ordinary iota path, entered when the hypothesis is used.
///
/// Every bound name carries `#`, which cannot occur in an ESL identifier, so nothing reachable
/// from a declaration can capture them. The names must also stay distinct from readback's `G#`
/// and the checker's `TC#`.
fn higher_order_ih(
    decl: &Arc<crate::nbe::term::InductiveDecl>,
    motive: &Val,
    minors: &[Val],
    arg: &Val,
    arity: usize,
) -> Result<Val, EvalError> {
    let binder_names: Vec<String> = (0..arity).map(|j| format!("HB#{j}")).collect();
    let minor_names: Vec<String> = (0..minors.len()).map(|j| format!("HM#{j}")).collect();

    let mut env = Rho::Nil
        .extend(Patt::Var("HA#".to_string()), arg.clone())
        .extend(Patt::Var("HV#".to_string()), motive.clone());
    for (name, m) in minor_names.iter().zip(minors.iter()) {
        env = env.extend(Patt::Var(name.clone()), m.clone());
    }

    // `HA# b₁ … b_k` — the argument applied to the hypothesis's own binders.
    let major = binder_names
        .iter()
        .fold(Exp::Var("HA#".to_string()), |acc, n| {
            Exp::App(Box::new(acc), Box::new(Exp::Var(n.clone())))
        });
    let mut body = Exp::InductiveRec {
        decl: decl.clone(),
        motive: Box::new(Exp::Var("HV#".to_string())),
        minors: minor_names.iter().map(|n| Exp::Var(n.clone())).collect(),
        major: Box::new(major),
    };
    // Wrap `λ b₁ … λ b_k` — innermost binders first, so `b₁` ends up outermost.
    for name in binder_names.iter().skip(1).rev() {
        body = Exp::Lam(Patt::Var(name.clone()), Box::new(body));
    }
    match binder_names.first() {
        Some(first) => Ok(Val::Lam(crate::nbe::val::Clos::new(
            Patt::Var(first.clone()),
            body,
            env,
        ))),
        // `arity == 0` is the direct case, which never reaches here.
        None => Err(EvalError::InvalidCaseTarget(
            "higher_order_ih called with arity 0 — that is the direct recursive case".to_string(),
        )),
    }
}

fn build_recursor_ih<T: Tracer>(
    decl: &Arc<crate::nbe::term::InductiveDecl>,
    motive: &Val,
    minors: &[Val],
    arg: &Val,
    ctx: &EvalCtx,
) -> Result<(Val, T::Node), EvalError> {
    match arg {
        Val::InductiveVal {
            ctor_name, args, ..
        } => iota_reduce_impl::<T>(decl, motive, minors, ctor_name, args, ctx),
        Val::Nt(n) => Ok((
            Val::Nt(Neut::NtRec {
                decl: decl.clone(),
                motive: Box::new(motive.clone()),
                minors: minors.to_vec(),
                major: Box::new(n.clone()),
            }),
            T::leaf(),
        )),
        other => Err(EvalError::InvalidCaseTarget(format!(
            "InductiveRec: recursive argument is not an inductive value: {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::nbe::env::Rho;
    use crate::nbe::eval::testutil::*;
    use crate::nbe::eval::{eval_ctx, EvalCtx};
    use crate::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, Patt};
    use crate::nbe::val::{Neut, Val};
    use std::sync::Arc;
    /// **eigenius#92 step 2 — the minor derivation and iota agree on a HIGHER-ORDER positive
    /// argument: both give it a FUNCTION-typed induction hypothesis.**
    ///
    /// This test used to assert the opposite — that both sites SKIPPED such an argument. That was
    /// step 1's staging invariant: `positivity` had just been widened to admit
    /// `(Set → Foo) → Foo`, while `derive_minor_type` and `iota_reduce_impl` still filtered on
    /// `is_direct()`, so neither derived an IH nor applied one. They agreed, which made the
    /// restriction a completeness limit rather than a soundness one and let eigenius#92 land ahead
    /// of eigenius#138. Step 2 lifts the guard in both places at once, and the invariant it pins is
    /// unchanged in kind: **the arity the minor's type declares is the arity iota applies.**
    ///
    /// For `rall : (Set → Foo) → Foo` the minor is now
    /// `Π f:(Set → Foo). Π ih:(Π b:Set. motive (f b)). motive (rall f)` — two binders, where step 1
    /// derived one.
    ///
    /// The shape is `lexicon:Cat`'s, minus the parameter:
    /// `cat_fin_forall : (lexicon:Fin -> lexicon:Cat) -> lexicon:Cat`.
    #[test]
    fn higher_order_positive_arg_gets_a_function_typed_ih_in_both_sites() {
        // inductive Foo { base : Foo, rall : (Set -> Foo) -> Foo }
        let s = ind_self_ref("Foo");
        let foo_ty = Exp::InductiveType(s, Vec::new());
        let foo = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Foo").unwrap(),
            name: "Foo".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "base".to_string(),
                    typ: foo_ty.clone(),
                },
                InductiveCtorDecl {
                    name: "rall".to_string(),
                    // (Set -> Foo) -> Foo
                    typ: Exp::Pi(
                        Patt::Var("f".to_string()),
                        Box::new(Exp::Pi(
                            Patt::Unit,
                            Box::new(Exp::sort(1)),
                            Box::new(foo_ty.clone()),
                        )),
                        Box::new(foo_ty.clone()),
                    ),
                },
            ],
        });

        // Constant motive `λ_. Set`, so a fully applied minor reduces to `Val::sort(1)`.
        let motive = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::sort(1),
            Rho::Nil,
        ));

        // 1. How many binders does the minor derivation ask for?
        let minor_ty =
            crate::nbe::recursor::derive_minor_type(&foo, 1, &[], &motive, &EvalCtx::Pure)
                .expect("derive_minor_type for `rall`");
        let mut arity = 0usize;
        let mut cursor = minor_ty;
        while let Val::Pi(_, clos) = cursor {
            arity += 1;
            cursor = clos
                .apply(Val::Nt(Neut::Gen(arity, format!("v{arity}"))))
                .expect("apply pi clos");
        }
        assert_eq!(
            arity, 2,
            "`rall` has one constructor argument AND a function-typed IH for it. One binder would \
             mean the derivation still skips higher-order arguments; three would mean it and iota \
             have drifted."
        );

        // 2. Does iota apply exactly that many? A minor of arity 2 returning `Sort(1)`: too few
        //    applications leaves a `Val::Lam`, too many applies `Sort(1)` to something and errors.
        let base_minor = Val::sort(1);
        let rall_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(Patt::Unit, Box::new(Exp::sort(1))),
            Rho::Nil,
        ));
        // The constructor argument: `λ_. Foo.base`, a value of type `Set -> Foo`.
        let arg = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::InductiveCtor(foo.clone(), "base".to_string(), Vec::new()),
            Rho::Nil,
        ));

        let result = iota_reduce(
            &foo,
            &motive,
            &[base_minor, rall_minor],
            "rall",
            &[arg],
            &EvalCtx::Pure,
        )
        .expect("iota_reduce over `rall`");
        assert!(
            matches!(&result, Val::Sort(l) if l.is_nat(1)),
            "iota applied exactly the {arity} argument(s) the minor's type declares, leaving the \
             minor's body; got {result:?}"
        );
    }

    /// **The payoff: induction THROUGH a higher-order argument actually computes.**
    ///
    /// The test above pins that the minor's declared arity and iota's applied arity agree. Agreeing
    /// is what step 1 already had — both sites skipped the argument, consistently — so agreement
    /// alone does not show the eliminator got any stronger. This one uses the hypothesis.
    ///
    /// `Foo { base : Foo, rall : (One → Foo) → Foo }`, eliminated with the minor
    /// `λf. λih. ih unit`. Applying `ih` at `unit` is the recursive call on `f unit`, so reducing
    /// `rec (rall (λ_. base))` must run one step of recursion and land on the `base` minor. Under
    /// step 1 there was no `ih` to apply and this program could not be written.
    #[test]
    fn iota_recurses_through_a_higher_order_argument() {
        let s = ind_self_ref("Foo");
        let foo_ty = Exp::InductiveType(s, Vec::new());
        let foo = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Foo").unwrap(),
            name: "Foo".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "base".to_string(),
                    typ: foo_ty.clone(),
                },
                InductiveCtorDecl {
                    name: "rall".to_string(),
                    // (One -> Foo) -> Foo
                    typ: Exp::Pi(
                        Patt::Var("f".to_string()),
                        Box::new(Exp::Pi(
                            Patt::Unit,
                            Box::new(Exp::One),
                            Box::new(foo_ty.clone()),
                        )),
                        Box::new(foo_ty.clone()),
                    ),
                },
            ],
        });

        // base_minor is a marker: reaching it proves the recursive call was made.
        let base_minor = Val::Con("reached-base".to_string(), Box::new(Val::Unit));
        // rall_minor = λf. λih. ih unit   — `ih unit` IS the recursive call on `f unit`.
        let rall_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(
                Patt::Var("ih".to_string()),
                Box::new(Exp::App(
                    Box::new(Exp::Var("ih".to_string())),
                    Box::new(Exp::Unit),
                )),
            ),
            Rho::Nil,
        ));
        // The constructor argument `λ_. base`, so the recursive call lands on `base`.
        let f = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::InductiveCtor(foo.clone(), "base".to_string(), Vec::new()),
            Rho::Nil,
        ));

        let result = iota_reduce(
            &foo,
            &Val::Lam(crate::nbe::val::Clos::new(Patt::Unit, Exp::One, Rho::Nil)),
            &[base_minor, rall_minor],
            "rall",
            &[f],
            &EvalCtx::Pure,
        )
        .expect("iota over `rall` with a hypothesis that recurses");
        match result {
            Val::Con(c, _) if c == "reached-base" => {}
            other => panic!(
                "applying the IH at `unit` must recurse into `f unit` = `base` and yield the base \
                 minor; got {other:?}"
            ),
        }
    }

    #[test]
    fn iota_zero_arity_constructor() {
        // inductive Bool { True, False }
        // Bool.rec C true_minor false_minor True ↝ true_minor
        let s = ind_self_ref("Bool");
        let bool_ty = Exp::InductiveType(s, Vec::new());
        let bool_decl = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Bool").unwrap(),
            name: "Bool".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "True".to_string(),
                    typ: bool_ty.clone(),
                },
                InductiveCtorDecl {
                    name: "False".to_string(),
                    typ: bool_ty,
                },
            ],
        });
        let true_minor = Val::Con("yes".to_string(), Box::new(Val::Unit));
        let false_minor = Val::Con("no".to_string(), Box::new(Val::Unit));
        let result = iota_reduce(
            &bool_decl,
            &Val::sort(1),
            &[true_minor, false_minor],
            "True",
            &[],
            &EvalCtx::Pure,
        )
        .expect("iota_reduce");
        match result {
            Val::Con(c, _) if c == "yes" => {}
            other => panic!("expected Con(\"yes\", _), got {other:?}"),
        }
    }

    #[test]
    fn iota_recursive_constructor_double() {
        // Nat.rec zero (λ_n. λih. succ (succ ih)) (succ (succ zero))
        // ↝ succ (succ (succ (succ zero)))
        let nat = nat_decl();
        let zero_minor = nat_n(&nat, 0);

        // succ_minor body: succ (succ ih)
        let succ_body = Exp::InductiveCtor(
            nat.clone(),
            "succ".to_string(),
            vec![Exp::InductiveCtor(
                nat.clone(),
                "succ".to_string(),
                vec![Exp::Var("ih".to_string())],
            )],
        );
        // λ_n. λih. body
        let succ_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(Patt::Var("ih".to_string()), Box::new(succ_body)),
            Rho::Nil,
        ));

        let result = iota_reduce(
            &nat,
            &Val::sort(1),
            &[zero_minor, succ_minor],
            "succ",
            &[nat_n(&nat, 1)],
            &EvalCtx::Pure,
        )
        .expect("iota_reduce");

        let expected = nat_n(&nat, 4);
        let result_exp = crate::nbe::readback::readback_val(0, &result);
        let expected_exp = crate::nbe::readback::readback_val(0, &expected);
        assert_eq!(result_exp, expected_exp);
    }

    /// Port-fidelity witness (docs/notes/nbe-reorganization-analysis.md
    /// §4), paired with recursor.rs's
    /// `node_minor_binder_order_is_args_then_ihs_in_arg_order`: iota
    /// application order is minor → ctor args (original order) → one IH
    /// per recursive arg (original order). An asymmetric tree makes any
    /// deviation (reversed or interleaved IHs) produce a different value.
    #[test]
    fn iota_two_recursive_args_ih_order_matches_minor_binders() {
        // Tree { leaf : Tree, node : Tree → Tree → Tree }
        let s = ind_self_ref("Tree");
        let tree_ty = Exp::InductiveType(s, Vec::new());
        let tree = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Tree").unwrap(),
            name: "Tree".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
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
        let leaf = Val::InductiveVal {
            decl: tree.clone(),
            ctor_name: "leaf".to_string(),
            args: Vec::new(),
        };
        let node = |l: Val, r: Val| Val::InductiveVal {
            decl: tree.clone(),
            ctor_name: "node".to_string(),
            args: vec![l, r],
        };
        // leaf ↦ 7; node ↦ λl. λr. λihl. λihr. (ihl, ihr).
        let leaf_minor = Val::LitInt(7);
        let node_body = Exp::Lam(
            Patt::Var("r".to_string()),
            Box::new(Exp::Lam(
                Patt::Var("ihl".to_string()),
                Box::new(Exp::Lam(
                    Patt::Var("ihr".to_string()),
                    Box::new(Exp::Pair(
                        Box::new(Exp::Var("ihl".to_string())),
                        Box::new(Exp::Var("ihr".to_string())),
                    )),
                )),
            )),
        );
        let node_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Var("l".to_string()),
            node_body,
            Rho::Nil,
        ));

        // Scrutinee: node(node(leaf, leaf), leaf) — asymmetric, so the
        // two IHs are distinguishable. iota takes the outer ctor's args.
        let result = iota_reduce(
            &tree,
            &Val::sort(1),
            &[leaf_minor, node_minor],
            "node",
            &[node(leaf.clone(), leaf.clone()), leaf],
            &EvalCtx::Pure,
        )
        .expect("iota_reduce");
        // rec(node(node(leaf,leaf), leaf)) = (rec(node(leaf,leaf)), rec(leaf))
        //                                  = ((7, 7), 7)
        // Reversed IH order would yield (7, (7, 7)).
        let expected = Val::Pair(
            Box::new(Val::Pair(
                Box::new(Val::LitInt(7)),
                Box::new(Val::LitInt(7)),
            )),
            Box::new(Val::LitInt(7)),
        );
        let result_exp = crate::nbe::readback::readback_val(0, &result);
        let expected_exp = crate::nbe::readback::readback_val(0, &expected);
        assert_eq!(result_exp, expected_exp);
    }

    #[test]
    fn iota_list_length() {
        // List.rec zero (λa rest ih. succ ih) [_, _, _] = succ (succ (succ zero))
        let nat = nat_decl();
        let s = ind_self_ref("List");
        let list_ty = Exp::InductiveType(s, vec![Exp::Var("A".to_string())]);
        let list_decl = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:List").unwrap(),
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "nil".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::sort(1)),
                        Box::new(list_ty.clone()),
                    ),
                },
                InductiveCtorDecl {
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

        let elem = Val::Unit;
        let nil_val = Val::InductiveVal {
            decl: list_decl.clone(),
            ctor_name: "nil".to_string(),
            args: Vec::new(),
        };
        let cons = |a: Val, l: Val| Val::InductiveVal {
            decl: list_decl.clone(),
            ctor_name: "cons".to_string(),
            args: vec![a, l],
        };
        let three = cons(elem.clone(), cons(elem.clone(), cons(elem, nil_val)));

        let nil_minor = nat_n(&nat, 0);
        // λ_a. λ_rest. λih. succ ih
        let cons_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(
                Patt::Unit,
                Box::new(Exp::Lam(
                    Patt::Var("ih".to_string()),
                    Box::new(Exp::InductiveCtor(
                        nat.clone(),
                        "succ".to_string(),
                        vec![Exp::Var("ih".to_string())],
                    )),
                )),
            ),
            Rho::Nil,
        ));

        let three_args = match &three {
            Val::InductiveVal { args, .. } => args.clone(),
            _ => unreachable!(),
        };
        let result = iota_reduce(
            &list_decl,
            &Val::sort(1),
            &[nil_minor, cons_minor],
            "cons",
            &three_args,
            &EvalCtx::Pure,
        )
        .expect("iota_reduce");

        let expected = nat_n(&nat, 3);
        let result_exp = crate::nbe::readback::readback_val(0, &result);
        let expected_exp = crate::nbe::readback::readback_val(0, &expected);
        assert_eq!(result_exp, expected_exp);
    }

    #[test]
    fn iota_neutral_major_blocks() {
        // Eval Exp::InductiveRec on a neutral major must produce Neut::NtRec.
        let nat = nat_decl();
        let neutral = Val::Nt(Neut::Gen(0, "n".to_string()));
        let zero_minor = ind_zero(&nat);
        // Dummy succ minor body: Unit
        let succ_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(Patt::Unit, Box::new(Exp::Unit)),
            Rho::Nil,
        ));
        let rho = Rho::Nil
            .extend(Patt::Var("n".to_string()), neutral)
            .extend(Patt::Var("zero_min".to_string()), zero_minor)
            .extend(Patt::Var("succ_min".to_string()), succ_minor);
        let exp = Exp::InductiveRec {
            decl: nat.clone(),
            motive: Box::new(Exp::sort(1)),
            minors: vec![
                Exp::Var("zero_min".to_string()),
                Exp::Var("succ_min".to_string()),
            ],
            major: Box::new(Exp::Var("n".to_string())),
        };
        let result = eval_ctx(&exp, &rho, &EvalCtx::Pure).expect("eval");
        match result {
            Val::Nt(Neut::NtRec { decl: d, .. }) => assert_eq!(d.name, "Nat"),
            other => panic!("expected NtRec, got {other:?}"),
        }
    }

    /// SimpleVec (A : Set) : 1 → Set with nil : SimpleVec A () and
    /// cons : (h:1) → A → SimpleVec A () → SimpleVec A (). Mirrors the
    /// check.rs / recursor.rs Phase B/E test fixtures so iota behavior
    /// can be verified against the same shape.
    fn simple_vec_decl_for_eval() -> Arc<InductiveDecl> {
        let self_ref = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:SimpleVec").unwrap(),
            name: "SimpleVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        let vec_a_unit =
            Exp::InductiveType(self_ref.clone(), vec![Exp::Var("A".to_string()), Exp::Unit]);
        Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:SimpleVec").unwrap(),
            name: "SimpleVec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: vec![
                crate::nbe::term::InductiveCtorDecl {
                    name: "nil".to_string(),
                    typ: Exp::Pi(
                        Patt::Var("A".to_string()),
                        Box::new(Exp::sort(1)),
                        Box::new(vec_a_unit.clone()),
                    ),
                },
                crate::nbe::term::InductiveCtorDecl {
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

    #[test]
    fn d48_iota_indexed_vec_nil_reduces() {
        // rec(motive, [nil_minor, cons_minor], nil ()) → nil_minor
        // (after the minor receives any value-args; `nil` has none
        // beyond the param A, which the recursor consumes during
        // ctor reconstruction, not at the minor level).
        //
        // Build SimpleVec, construct an InductiveVal for nil, and
        // run InductiveRec on it. With const-unit minors, the
        // result must reduce to Unit.
        let decl = simple_vec_decl_for_eval();
        let nil_val = Val::InductiveVal {
            decl: decl.clone(),
            ctor_name: "nil".to_string(),
            args: Vec::new(),
        };
        // Motive: λ_idx. λ_v. 1  (a constant-One motive — the
        // recursor's result type is Unit regardless of indices).
        let motive = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(Patt::Unit, Box::new(Exp::One)),
            Rho::Nil,
        ));
        // Minors: nil_minor = Unit, cons_minor = λ_h. λ_x. λ_xs. λ_ih. Unit
        let nil_minor = Val::Unit;
        let cons_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(
                Patt::Unit,
                Box::new(Exp::Lam(
                    Patt::Unit,
                    Box::new(Exp::Lam(Patt::Unit, Box::new(Exp::Unit))),
                )),
            ),
            Rho::Nil,
        ));
        let rho = Rho::Nil
            .extend(Patt::Var("v".to_string()), nil_val.clone())
            .extend(Patt::Var("m".to_string()), motive.clone())
            .extend(Patt::Var("nil_min".to_string()), nil_minor)
            .extend(Patt::Var("cons_min".to_string()), cons_minor);
        let rec_exp = Exp::InductiveRec {
            decl,
            motive: Box::new(Exp::Var("m".to_string())),
            minors: vec![
                Exp::Var("nil_min".to_string()),
                Exp::Var("cons_min".to_string()),
            ],
            major: Box::new(Exp::Var("v".to_string())),
        };
        let result = eval_ctx(&rec_exp, &rho, &EvalCtx::Pure).expect("iota nil");
        // For nil with no value-args, the minor is applied to nothing —
        // the result is nil_minor itself, which is Unit.
        assert!(
            matches!(result, Val::Unit),
            "expected iota(rec on nil) to reduce to Unit (the nil_minor); got {result:?}"
        );
    }

    #[test]
    fn d48_iota_indexed_vec_cons_reduces_with_ih() {
        // rec on a 1-element cons should:
        //   - apply cons_minor to (h, x, xs)
        //   - then apply an IH for xs (which itself is nil)
        // With the cons_minor `λ_h. λ_x. λ_xs. λ_ih. Unit`, the
        // result is Unit regardless of the IH value.
        let decl = simple_vec_decl_for_eval();
        let nil_val = Val::InductiveVal {
            decl: decl.clone(),
            ctor_name: "nil".to_string(),
            args: Vec::new(),
        };
        let cons_val = Val::InductiveVal {
            decl: decl.clone(),
            ctor_name: "cons".to_string(),
            args: vec![Val::Unit, Val::Unit, nil_val],
        };
        let motive = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(Patt::Unit, Box::new(Exp::One)),
            Rho::Nil,
        ));
        let nil_minor = Val::Unit;
        let cons_minor = Val::Lam(crate::nbe::val::Clos::new(
            Patt::Unit,
            Exp::Lam(
                Patt::Unit,
                Box::new(Exp::Lam(
                    Patt::Unit,
                    Box::new(Exp::Lam(Patt::Unit, Box::new(Exp::Unit))),
                )),
            ),
            Rho::Nil,
        ));
        let rho = Rho::Nil
            .extend(Patt::Var("v".to_string()), cons_val)
            .extend(Patt::Var("m".to_string()), motive)
            .extend(Patt::Var("nil_min".to_string()), nil_minor)
            .extend(Patt::Var("cons_min".to_string()), cons_minor);
        let rec_exp = Exp::InductiveRec {
            decl,
            motive: Box::new(Exp::Var("m".to_string())),
            minors: vec![
                Exp::Var("nil_min".to_string()),
                Exp::Var("cons_min".to_string()),
            ],
            major: Box::new(Exp::Var("v".to_string())),
        };
        let result = eval_ctx(&rec_exp, &rho, &EvalCtx::Pure).expect("iota cons");
        assert!(
            matches!(result, Val::Unit),
            "expected iota(rec on cons) to reduce to Unit (const cons_minor); got {result:?}"
        );
    }
}
