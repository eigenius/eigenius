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

//! Strict positivity checker for inductive types (Phase 11b step 3, D19 §5).
//!
//! Verifies that every constructor of an inductive declaration is strictly
//! positive in the inductive being defined: the inductive may appear only
//! as the head of a constructor argument's type (a direct recursive
//! reference such as `List A`) or in the constructor's result, never
//! under a nested Π.
//!
//! The algorithm follows nanoda_lib's `check_positivity1`
//! (`references/nanoda_lib/src/inductive.rs:666-787` @ pinned commit
//! `f58f2f6`) restricted to the fragment that the Phase 11b iota
//! reduction can actually eliminate — direct recursive arguments only.
//!
//! ### Accepted
//! - `Nat { zero : Nat, succ : Nat → Nat }`
//! - `List(A : Set) { nil : List(A), cons : A → List(A) → List(A) }`
//! - Constructors that do not mention the inductive at all (zero-arity
//!   or fully parametric constructors).
//!
//! ### Rejected
//! - **Negative occurrence** — `Bad { mk : (Bad → Nat) → Bad }`. The
//!   inductive appears as the domain of a function inside a binder.
//! - **Higher-order positive occurrence** — `Foo { mk : (Nat → Foo) → Foo }`.
//!   Strictly positive in the classical sense, but Phase 11b's iota
//!   reduction cannot construct the corresponding induction hypothesis;
//!   accepting it here would create a soundness gap.
//! - **Nested occurrence** — `Tree { node : List(Tree) → Tree }`. The
//!   inductive appears inside another inductive's parameters.
//! - **Wrong result type** — constructor whose Π-telescope ends in
//!   anything other than an application of the parent inductive.
//! - **Non-uniform parameters** — a recursive occurrence or conclusion
//!   that instantiates a declaration parameter to anything other than
//!   the parameter variable itself (`P(A) { mk : P(1) → P(A) }`,
//!   `Q(A) { mk : Q(1) }`). Port of nanoda_lib's `ctor_app_params_ok`.

use crate::nbe::term::{Decl, Exp, InductiveDecl, Patt};

/// Validate every constructor of `decl` for strict positivity.
///
/// Returns `Ok(())` if every constructor's type is a Π-telescope whose
/// non-parameter binders are either I-free or direct applications of
/// `decl`, and whose final result is an application of `decl`.
pub fn check_positivity(decl: &InductiveDecl) -> Result<(), String> {
    for ctor in &decl.ctors {
        check_constructor(decl, ctor.name.as_str(), &ctor.typ)?;
    }
    Ok(())
}

/// Check one constructor's full type expression.
///
/// Walks the Π-telescope, skipping the first `decl.params.len()` binders
/// (the parameter prefix), and validates each remaining binder type plus
/// the final result. Tracks the prefix binder names so occurrences of
/// `decl` can be checked for parameter uniformity: a recursive
/// occurrence (or the conclusion) must pass the parameters through
/// unchanged, as the parameter variables themselves. A later binder
/// that rebinds a parameter's name shadows it — `Var(name)` no longer
/// refers to the parameter, so uniformity becomes unsatisfiable through
/// that name.
fn check_constructor(decl: &InductiveDecl, ctor_name: &str, ctor_typ: &Exp) -> Result<(), String> {
    let mut current = ctor_typ;
    let mut params_to_skip = decl.params.len();
    // Ctor-prefix binder names, in parameter order; `None` = anonymous
    // or shadowed (unreferencable).
    let mut param_refs: Vec<Option<String>> = Vec::with_capacity(decl.params.len());
    while let Exp::Pi(patt, dom, body) = current {
        if params_to_skip > 0 {
            params_to_skip -= 1;
            let name = match patt {
                Patt::Var(n) => Some(n.clone()),
                _ => None,
            };
            // A duplicate parameter name shadows the earlier one.
            if let Some(n) = &name {
                shadow_param_refs(&mut param_refs, n);
            }
            param_refs.push(name);
        } else {
            // The binder's own domain is checked before its pattern
            // enters scope; shadowing applies to later args and the
            // conclusion only.
            check_arg_positivity(decl, ctor_name, dom, &param_refs)?;
            shadow_patt(&mut param_refs, patt);
        }
        current = body;
    }
    check_result_type(decl, ctor_name, current, &param_refs)
}

/// Clear every `param_refs` entry equal to `name` (it is shadowed).
fn shadow_param_refs(param_refs: &mut [Option<String>], name: &str) {
    for entry in param_refs.iter_mut() {
        if entry.as_deref() == Some(name) {
            *entry = None;
        }
    }
}

/// Apply the shadowing effect of a binder pattern to `param_refs`.
fn shadow_patt(param_refs: &mut [Option<String>], patt: &Patt) {
    match patt {
        Patt::Var(n) => shadow_param_refs(param_refs, n),
        Patt::Pair(a, b) => {
            shadow_patt(param_refs, a);
            shadow_patt(param_refs, b);
        }
        Patt::Unit => {}
    }
}

/// Check that the parameter prefix of an application of `decl` passes
/// the declaration parameters through unchanged: argument #i must be
/// the (unshadowed) parameter variable itself. Port of nanoda_lib's
/// `ctor_app_params_ok` (inductive.rs @ f58f2f6) — without this, a
/// recursive occurrence like `P(1)` inside `P(A)` derives an induction
/// hypothesis `C(arg)` with `arg : P(1)` against a motive
/// `C : P(A) → Sort`, and a conclusion like `Q(1)` gives the ctor a
/// type unrelated to the declared family.
fn check_params_uniform(
    decl: &InductiveDecl,
    ctor_name: &str,
    param_args: &[Exp],
    param_refs: &[Option<String>],
    context: &str,
) -> Result<(), String> {
    for (i, arg) in param_args.iter().enumerate() {
        let ok = matches!(
            (arg, param_refs.get(i)),
            (Exp::Var(n), Some(Some(p))) if n == p
        );
        if !ok {
            return Err(format!(
                "constructor `{}.{ctor_name}`: {context} of `{}` must pass the \
                 declaration parameters through unchanged — argument #{i} is not \
                 the parameter variable",
                decl.name, decl.name
            ));
        }
    }
    Ok(())
}

/// Validate one constructor argument's type for strict positivity.
///
/// Three cases, in order:
/// 1. The type does not mention the inductive at all → accept (non-recursive arg).
/// 2. The type is a direct application `Exp::InductiveType(decl, args)`
///    with full arity, the parameter prefix passed through unchanged,
///    and no inductive occurrence in the index arguments → accept
///    (direct recursive arg; Phase 11b iota produces one IH per such arg).
/// 3. Otherwise the inductive appears either under a Π or nested inside
///    another inductive — reject.
fn check_arg_positivity(
    decl: &InductiveDecl,
    ctor_name: &str,
    arg_typ: &Exp,
    param_refs: &[Option<String>],
) -> Result<(), String> {
    if !has_ind_occurrence(decl, arg_typ) {
        return Ok(());
    }
    if let Exp::InductiveType(d, args) = arg_typ {
        if d.iri == decl.iri {
            let n_params = decl.params.len();
            let n_indices = decl.indices.len();
            if args.len() != n_params + n_indices {
                return Err(format!(
                    "constructor `{}.{ctor_name}`: recursive occurrence of `{}` \
                     must apply {} parameter(s) + {} index/indices, got {} argument(s)",
                    decl.name,
                    decl.name,
                    n_params,
                    n_indices,
                    args.len()
                ));
            }
            check_params_uniform(
                decl,
                ctor_name,
                &args[..n_params],
                param_refs,
                "a recursive occurrence",
            )?;
            for arg in &args[n_params..] {
                if has_ind_occurrence(decl, arg) {
                    return Err(format!(
                        "non-positive occurrence: constructor `{}.{ctor_name}` has a \
                         nested inductive use of `{}` inside its own indices",
                        decl.name, decl.name
                    ));
                }
            }
            return Ok(());
        }
    }
    Err(format!(
        "non-positive occurrence: constructor `{}.{ctor_name}` mentions inductive `{}` \
         outside of a direct recursive position",
        decl.name, decl.name
    ))
}

/// The constructor's result type must be a direct application of the
/// parent inductive, with the parameter prefix passed through unchanged.
/// (Conclusion arity — params + indices — is validated with friendlier
/// diagnostics by `check::validate_indexed_ctor_conclusions`, which
/// runs alongside this checker in `check_type`.)
fn check_result_type(
    decl: &InductiveDecl,
    ctor_name: &str,
    typ: &Exp,
    param_refs: &[Option<String>],
) -> Result<(), String> {
    match typ {
        Exp::InductiveType(d, args) if d.iri == decl.iri => {
            let upto = decl.params.len().min(args.len());
            check_params_uniform(decl, ctor_name, &args[..upto], param_refs, "the conclusion")
        }
        _ => Err(format!(
            "constructor `{}.{ctor_name}` must end in an application of `{}`",
            decl.name, decl.name
        )),
    }
}

/// Whether `decl.name` occurs anywhere in `exp`.
///
/// Walks every `Exp` constructor structurally. Conservative: any
/// occurrence — in a parameter position, under a Π, inside a sum or
/// case branch — counts.
pub fn has_ind_occurrence(decl: &InductiveDecl, exp: &Exp) -> bool {
    match exp {
        Exp::InductiveType(d, args) => {
            d.iri == decl.iri || args.iter().any(|a| has_ind_occurrence(decl, a))
        }
        Exp::InductiveCtor(d, _, args) => {
            d.iri == decl.iri || args.iter().any(|a| has_ind_occurrence(decl, a))
        }
        Exp::InductiveRec {
            decl: d,
            motive,
            minors,
            major,
        } => {
            d.iri == decl.iri
                || has_ind_occurrence(decl, motive)
                || minors.iter().any(|m| has_ind_occurrence(decl, m))
                || has_ind_occurrence(decl, major)
        }
        // A declaration expression evaluates to the same type former as
        // `Exp::InductiveType(d, [])` (see `eval`'s `Exp::Inductive`
        // arm), so a reference to `decl` in this form IS an occurrence.
        // Also scan the embedded declaration's constructor types: a
        // different declaration nested in argument position may itself
        // reference `decl`. (Self-reference stubs carry empty `ctors`,
        // and the iri short-circuit fires first for the decl itself, so
        // this cannot recurse unboundedly.)
        Exp::Inductive(d) => {
            d.iri == decl.iri || d.ctors.iter().any(|c| has_ind_occurrence(decl, &c.typ))
        }

        Exp::Pi(_, a, b) | Exp::Sig(_, a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) => {
            has_ind_occurrence(decl, a) || has_ind_occurrence(decl, b)
        }
        Exp::Lam(_, body) => has_ind_occurrence(decl, body),
        Exp::Ann(e, t) => has_ind_occurrence(decl, e) || has_ind_occurrence(decl, t),
        Exp::App(f, x) => has_ind_occurrence(decl, f) || has_ind_occurrence(decl, x),
        Exp::Pair(a, b) => has_ind_occurrence(decl, a) || has_ind_occurrence(decl, b),
        Exp::Con(_, e) => has_ind_occurrence(decl, e),
        Exp::Fst(e) | Exp::Snd(e) => has_ind_occurrence(decl, e),
        Exp::Data(summands) => summands.iter().any(|s| has_ind_occurrence(decl, &s.typ)),
        Exp::Case(branches) => branches.iter().any(|b| has_ind_occurrence(decl, &b.body)),
        Exp::Dec(d, e) => {
            let from_decl = match d {
                Decl::Def(_, t, body) | Decl::Drec(_, t, body) => {
                    has_ind_occurrence(decl, t) || has_ind_occurrence(decl, body)
                }
            };
            from_decl || has_ind_occurrence(decl, e)
        }
        Exp::Id(a, x, y) => {
            has_ind_occurrence(decl, a)
                || has_ind_occurrence(decl, x)
                || has_ind_occurrence(decl, y)
        }
        Exp::Refl(a) => has_ind_occurrence(decl, a),
        Exp::IdJ(args) => args.iter().any(|a| has_ind_occurrence(decl, a)),
        Exp::NativeDecide(c, e) => {
            let args_contain = match c {
                crate::nbe::term::Constraint::Institution { args, .. } => {
                    args.iter().any(|a| has_ind_occurrence(decl, a))
                }
                _ => false,
            };
            args_contain || has_ind_occurrence(decl, e)
        }
        Exp::DecEq(a, x, y) => {
            has_ind_occurrence(decl, a)
                || has_ind_occurrence(decl, x)
                || has_ind_occurrence(decl, y)
        }
        Exp::PropAccess(e, _) => has_ind_occurrence(decl, e),
        Exp::Template(_, refs) => refs.iter().any(|(_, t)| has_ind_occurrence(decl, t)),
        Exp::Construct(_, fields) => fields.iter().any(|(_, e)| has_ind_occurrence(decl, e)),
        Exp::Codata(observations) => observations
            .iter()
            .any(|o| has_ind_occurrence(decl, &o.typ)),
        // A parameterised codata application at an inductive arg
        // position must recurse into any param that carries a
        // recursive occurrence, exactly like `Exp::InductiveType`.
        // The codata decl itself is never recursive into the
        // enclosing inductive (different sort), so we skip the
        // decl.name check and scan args only.
        Exp::CodataType(_, args) => args.iter().any(|a| has_ind_occurrence(decl, a)),
        // Cross-institution translation — scan the source
        // expression; the comorphism IRI is opaque.
        Exp::InstitutionInvoke { source, .. } => has_ind_occurrence(decl, source),
        Exp::CoRecord(fields) => fields.iter().any(|f| has_ind_occurrence(decl, &f.body)),
        Exp::Observe(e, _) => has_ind_occurrence(decl, e),
        Exp::Map(f, c) => has_ind_occurrence(decl, f) || has_ind_occurrence(decl, c),
        Exp::Reduce(f, i, c) => {
            has_ind_occurrence(decl, f)
                || has_ind_occurrence(decl, i)
                || has_ind_occurrence(decl, c)
        }
        Exp::Match { scrutinee, arms } => {
            has_ind_occurrence(decl, scrutinee)
                || arms.iter().any(|a| has_ind_occurrence(decl, &a.body))
        }

        // Size primitives are over a disjoint sort and can never
        // contain inductive occurrences.
        Exp::SizeSucc(s) => has_ind_occurrence(decl, s),
        Exp::SizeSort | Exp::SizeInf => false,
        // SizedPi is a binder; recurse into both the upper bound and
        // body. The upper bound is a size and can't carry an inductive
        // occurrence, but `body` may.
        Exp::SizedPi { upper, body, .. } => {
            has_ind_occurrence(decl, upper) || has_ind_occurrence(decl, body)
        }

        Exp::Var(_)
        | Exp::Sort(_)
        | Exp::One
        | Exp::Unit
        | Exp::EigonClass(_)
        | Exp::EigonAxiom(_)
        | Exp::EigonPrimitive(_)
        | Exp::EigonResource(_)
        | Exp::LitString(_)
        | Exp::LitInt(_)
        | Exp::LitFloat(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::{InductiveCtorDecl, Patt};
    use std::sync::Arc;

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

    #[test]
    fn accepts_nat() {
        let s = self_ref("Nat");
        let nat_ty = Exp::InductiveType(s, Vec::new());
        let decl = InductiveDecl {
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
        };
        check_positivity(&decl).expect("Nat should be strictly positive");
    }

    #[test]
    fn accepts_list() {
        let s = self_ref("List");
        let list_ty = Exp::InductiveType(s, vec![Exp::Var("A".to_string())]);
        let decl = InductiveDecl {
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
        };
        check_positivity(&decl).expect("List should be strictly positive");
    }

    #[test]
    fn accepts_bool_zero_arity() {
        let s = self_ref("Bool");
        let bool_ty = Exp::InductiveType(s, Vec::new());
        let decl = InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Bool").unwrap(),
            name: "Bool".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
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
        };
        check_positivity(&decl).expect("Bool should be strictly positive");
    }

    #[test]
    fn rejects_negative_occurrence() {
        // Bad : (Bad → Nat) → Bad
        let s = self_ref("Bad");
        let bad_ty = Exp::InductiveType(s, Vec::new());
        let nat_ty = Exp::Var("Nat".to_string());
        let decl = InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Bad").unwrap(),
            name: "Bad".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: Exp::Pi(
                    Patt::Unit,
                    Box::new(Exp::Pi(
                        Patt::Unit,
                        Box::new(bad_ty.clone()),
                        Box::new(nat_ty),
                    )),
                    Box::new(bad_ty),
                ),
            }],
        };
        let err = check_positivity(&decl).expect_err("Bad should be rejected");
        assert!(err.contains("non-positive"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_higher_order_positive() {
        // Foo : (Nat → Foo) → Foo  — strictly positive but beyond Phase 11b iota
        let s = self_ref("Foo");
        let foo_ty = Exp::InductiveType(s, Vec::new());
        let nat_ty = Exp::Var("Nat".to_string());
        let decl = InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Foo").unwrap(),
            name: "Foo".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: Exp::Pi(
                    Patt::Unit,
                    Box::new(Exp::Pi(
                        Patt::Unit,
                        Box::new(nat_ty),
                        Box::new(foo_ty.clone()),
                    )),
                    Box::new(foo_ty),
                ),
            }],
        };
        let err = check_positivity(&decl).expect_err("Foo should be rejected");
        assert!(err.contains("non-positive"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_nested_occurrence() {
        // Tree : List(Tree) → Tree
        let tree_self = self_ref("Tree");
        let list_self = self_ref("List");
        let tree_ty = Exp::InductiveType(tree_self, Vec::new());
        let nested = Exp::InductiveType(list_self, vec![tree_ty.clone()]);
        let decl = InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Tree").unwrap(),
            name: "Tree".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "node".to_string(),
                typ: Exp::Pi(Patt::Unit, Box::new(nested), Box::new(tree_ty)),
            }],
        };
        let err = check_positivity(&decl).expect_err("Tree should be rejected");
        assert!(err.contains("non-positive"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_wrong_result_type() {
        // mk : Nat → Set  — does not return the inductive
        let decl = InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Bogus").unwrap(),
            name: "Bogus".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: Exp::Pi(
                    Patt::Unit,
                    Box::new(Exp::Var("Nat".to_string())),
                    Box::new(Exp::Sort(1)),
                ),
            }],
        };
        let err = check_positivity(&decl).expect_err("Bogus should be rejected");
        assert!(err.contains("must end in"), "unexpected error: {err}");
    }

    /// Closes finding F-2 (port-fidelity analysis,
    /// docs/notes/nbe-reorganization-analysis.md §4): a recursive
    /// occurrence that instantiates the block parameter to something
    /// other than the parameter itself is rejected, matching nanoda's
    /// `is_valid_ind_app`/`ctor_app_params_ok` (inductive.rs:691 @
    /// f58f2f6). Without the check, the derived IH for such an arg is
    /// `C(arg)` with `arg : P(1)` against a motive `C : P(A) → Sort`.
    #[test]
    fn rejects_param_mismatch_in_recursive_arg() {
        // P(A : Set) { mk : P(1) → P(A) }
        let s = self_ref("P");
        let rec_occ_wrong_param = Exp::InductiveType(s.clone(), vec![Exp::One]);
        let conclusion = Exp::InductiveType(s, vec![Exp::Var("A".to_string())]);
        let decl = InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:P").unwrap(),
            name: "P".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::Sort(1)),
                    Box::new(Exp::Pi(
                        Patt::Unit,
                        Box::new(rec_occ_wrong_param),
                        Box::new(conclusion),
                    )),
                ),
            }],
        };
        let err = check_positivity(&decl).expect_err("param-mismatched recursive arg");
        assert!(err.contains("parameters through unchanged"), "got: {err}");
    }

    /// Closes finding F-1 (port-fidelity analysis,
    /// docs/notes/nbe-reorganization-analysis.md §4): `Exp::Inductive(d)`
    /// evaluates to the same `Val::InductiveType` as
    /// `Exp::InductiveType(d, [])` (eval.rs `Exp::Inductive` arm), so
    /// `has_ind_occurrence` treats it as an occurrence — a negative
    /// occurrence written in the `Exp::Inductive` form no longer evades
    /// the checker.
    #[test]
    fn rejects_disguised_inductive_negative_occurrence() {
        // Neg { mk : (Neg → 1) → Neg } with the negative `Neg` written
        // as `Exp::Inductive(stub)` instead of `Exp::InductiveType`.
        let s = self_ref("Neg");
        let neg_ty = Exp::InductiveType(s.clone(), Vec::new());
        let disguised_negative = Exp::Pi(
            Patt::Unit,
            Box::new(Exp::Inductive(s)), // ← same type former, non-canonical spelling
            Box::new(Exp::One),
        );
        let decl = InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Neg").unwrap(),
            name: "Neg".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: Exp::Pi(Patt::Unit, Box::new(disguised_negative), Box::new(neg_ty)),
            }],
        };
        // The canonical-form spelling of the same declaration is
        // rejected by `rejects_negative_occurrence` above; the
        // disguised spelling must be too.
        let err = check_positivity(&decl).expect_err("disguised negative occurrence");
        assert!(err.contains("non-positive"), "got: {err}");
    }
}
