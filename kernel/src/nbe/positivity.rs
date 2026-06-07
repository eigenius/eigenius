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
//! ([inductive.rs:666-787](https://github.com/ammkrn/nanoda_lib/blob/main/src/inductive.rs#L666-L787))
//! restricted to the fragment that the Phase 11b iota reduction can
//! actually eliminate — direct recursive arguments only.
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

use crate::nbe::term::{Decl, Exp, InductiveDecl};

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
/// the final result.
fn check_constructor(decl: &InductiveDecl, ctor_name: &str, ctor_typ: &Exp) -> Result<(), String> {
    let mut current = ctor_typ;
    let mut params_to_skip = decl.params.len();
    while let Exp::Pi(_, dom, body) = current {
        if params_to_skip > 0 {
            params_to_skip -= 1;
        } else {
            check_arg_positivity(decl, ctor_name, dom)?;
        }
        current = body;
    }
    check_result_type(decl, ctor_name, current)
}

/// Validate one constructor argument's type for strict positivity.
///
/// Three cases, in order:
/// 1. The type does not mention the inductive at all → accept (non-recursive arg).
/// 2. The type is a direct application `Exp::InductiveType(decl, args)`
///    and none of `args` mentions the inductive → accept (direct
///    recursive arg; Phase 11b iota produces one IH per such arg).
/// 3. Otherwise the inductive appears either under a Π or nested inside
///    another inductive — reject.
fn check_arg_positivity(
    decl: &InductiveDecl,
    ctor_name: &str,
    arg_typ: &Exp,
) -> Result<(), String> {
    if !has_ind_occurrence(decl, arg_typ) {
        return Ok(());
    }
    if let Exp::InductiveType(d, args) = arg_typ {
        if d.name == decl.name {
            for arg in args {
                if has_ind_occurrence(decl, arg) {
                    return Err(format!(
                        "non-positive occurrence: constructor `{}.{ctor_name}` has a \
                         nested inductive use of `{}` inside its own parameters",
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
/// parent inductive.
fn check_result_type(decl: &InductiveDecl, ctor_name: &str, typ: &Exp) -> Result<(), String> {
    match typ {
        Exp::InductiveType(d, _) if d.name == decl.name => Ok(()),
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
            d.name == decl.name || args.iter().any(|a| has_ind_occurrence(decl, a))
        }
        Exp::InductiveCtor(d, _, args) => {
            d.name == decl.name || args.iter().any(|a| has_ind_occurrence(decl, a))
        }
        Exp::InductiveRec {
            decl: d,
            motive,
            minors,
            major,
        } => {
            d.name == decl.name
                || has_ind_occurrence(decl, motive)
                || minors.iter().any(|m| has_ind_occurrence(decl, m))
                || has_ind_occurrence(decl, major)
        }
        Exp::Inductive(_) => false,

        Exp::Pi(_, a, b) | Exp::Sig(_, a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) => {
            has_ind_occurrence(decl, a) || has_ind_occurrence(decl, b)
        }
        Exp::Lam(_, body) => has_ind_occurrence(decl, body),
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
}
