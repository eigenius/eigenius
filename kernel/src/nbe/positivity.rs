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
//! positive in the inductive being defined: `I` may appear as the head of a
//! constructor argument's type, possibly behind a Π telescope whose domains
//! are all `I`-free, and in the constructor's result — never to the left of
//! an arrow.
//!
//! The algorithm follows nanoda_lib's `check_positivity1`
//! (`references/nanoda_lib/src/inductive.rs:758` @ pinned commit `6ae1f0c`,
//! with `which_valid_ind_app` `:867` and `is_rec_argument` `:1082`).
//!
//! [`recursive_arg_shape`] is the single definition of *"this constructor
//! argument is a recursive occurrence"*, and it is deliberately shared:
//! `recursor::derive_minor_type` and `eval::iota_reduce_impl` consume it too.
//! Widening the criterion here without widening those is how the halves of an
//! eliminator come to disagree — which is exactly eigenius#138's defect in a
//! different pair of functions.
//!
//! ### Accepted
//! - `Nat { zero : Nat, succ : Nat → Nat }`
//! - `List(A : Set) { nil : List(A), cons : A → List(A) → List(A) }`
//! - Constructors that do not mention the inductive at all (zero-arity
//!   or fully parametric constructors).
//! - **Higher-order positive occurrence (eigenius#92)** —
//!   `Foo { mk : (Nat → Foo) → Foo }`, and the shape the bootstrap needs,
//!   `lexicon:Cat { cat_forall : Num → (Set → Cat) → Cat }`. `Cat` occurs
//!   only in the codomain of the argument's Π telescope, never in a domain.
//!
//!   This module rejected that shape until eigenius#92, on the grounds that
//!   Phase 11b's iota reduction cannot construct the corresponding induction
//!   hypothesis and that accepting it "would create a soundness gap". **The
//!   reading was wrong**: the two sites that build the hypothesis both consume
//!   [`recursive_arg_shape`], so they skipped such an argument identically —
//!   a *missing* hypothesis, not a wrong one, which is incompleteness rather
//!   than unsoundness. That is what let eigenius#92's routing land ahead of
//!   eigenius#138.
//!
//!   Since step 2 the hypothesis exists: `derive_minor_type` emits
//!   `Π b₁:B₁ … B_k. C(idx…) (arg b₁ … b_k)` and `iota_reduce_impl` supplies
//!   `λ b₁ … b_k. rec … (arg b₁ … b_k)`, so induction THROUGH a reflexive
//!   argument computes. Pinned by
//!   `higher_order_positive_arg_gets_a_function_typed_ih_in_both_sites` and
//!   `iota_recurses_through_a_higher_order_argument` (`nbe/eval/iota.rs`).
//!
//! ### Rejected
//! - **Negative occurrence** — `Bad { mk : (Bad → Nat) → Bad }`. The
//!   inductive appears in the DOMAIN of an argument's Π telescope. This is
//!   the case the whole module exists to stop, and it is what separates it
//!   from the higher-order positive case above.
//! - **Nested occurrence** — `Tree { node : List(Tree) → Tree }`. The
//!   inductive appears inside another inductive's parameters (eigenius#21).
//! - **Occurrence in its own indices** — a recursive occurrence whose index
//!   arguments mention the inductive.
//! - **Wrong result type** — constructor whose Π-telescope ends in
//!   anything other than an application of the parent inductive.
//! - **Non-uniform parameters** — a recursive occurrence or conclusion
//!   that instantiates a declaration parameter to anything other than
//!   the parameter variable itself (`P(A) { mk : P(1) → P(A) }`,
//!   `Q(A) { mk : Q(1) }`). Port of nanoda_lib's `ctor_app_params_ok`.
//!
//! ### Not handled
//! Mutual (eigenius#20) and nested (eigenius#21) declarations. The walk takes
//! a single `decl`; nanoda threads `all_inductives_incl_specialized` through
//! it instead. Making [`recursive_arg_shape`] take a block is then a
//! signature change rather than a redesign.

use crate::nbe::term::{Decl, Exp, InductiveDecl, Patt};

/// How a constructor argument mentions the inductive being declared.
///
/// The single definition of *"this argument is a recursive occurrence"*, shared by the three sites
/// that need it — this module (which admits the argument), `recursor::derive_minor_type` (which
/// emits an induction-hypothesis binder for it) and `eval::iota_reduce_impl` (which applies one).
/// Before eigenius#92 each site asked `InductiveDecl::is_direct_recursive_ref` separately, so
/// widening the criterion in one place and not the others would have made the eliminator's halves
/// disagree — eigenius#138's defect, in a different pair of functions.
///
/// Borrows throughout: `iota_reduce_impl` calls this per constructor argument on every reduction.
#[derive(Debug)]
pub struct RecArgShape<'a> {
    /// Π binders standing in front of the occurrence. **Empty for a direct recursive argument**
    /// (`D(params)(indices)`); non-empty for a higher-order positive one
    /// (`(a : A) → D(params)(indices)`, with `D` absent from every `A`).
    pub binders: Vec<(&'a Patt, &'a Exp)>,
    /// The occurrence's own arguments: the parameter prefix followed by the indices.
    pub args: &'a [Exp],
}

impl RecArgShape<'_> {
    /// A direct recursive argument — no binders in front of the occurrence.
    ///
    /// Both halves of the eliminator handle either case since eigenius#92 step 2, but they build
    /// DIFFERENT shapes: a direct argument gets `C(idx…) arg`, a higher-order one gets
    /// `Π b₁:B₁ … B_k. C(idx…) (arg b₁ … b_k)`. `derive_minor_type` and `iota_reduce_impl` branch
    /// on this together — changing one without the other is what makes an eliminator's halves
    /// disagree (eigenius#138 was that defect in a different pair of functions).
    pub fn is_direct(&self) -> bool {
        self.binders.is_empty()
    }
}

/// Classify a constructor argument type as an occurrence of `decl`, or not.
///
/// Returns `None` when the argument does not mention `decl` at all, and when it mentions it in a
/// position this fragment does not admit — a negative occurrence, a nested one, or an occurrence in
/// the recursive application's own indices. `None` therefore means *"not a recursive argument"*,
/// which is what the eliminator sites want; [`check_arg_positivity`] separately distinguishes
/// *"I-free, fine"* from *"mentions I illegally, reject"* and produces the diagnostic.
///
/// Follows nanoda's `is_rec_argument` (`references/nanoda_lib/src/inductive.rs:1082` @ `6ae1f0c`).
pub fn recursive_arg_shape<'a>(decl: &InductiveDecl, typ: &'a Exp) -> Option<RecArgShape<'a>> {
    let mut binders: Vec<(&'a Patt, &'a Exp)> = Vec::new();
    let mut cursor = typ;
    loop {
        match cursor {
            // A Π in front of the occurrence is admissible only when the inductive does not appear
            // in its DOMAIN. `I` in a domain is the negative occurrence — the thing positivity
            // exists to stop — and it is the only difference between `(Nat → I) → I` (fine) and
            // `(I → Nat) → I` (unsound).
            Exp::Pi(patt, dom, body) => {
                if has_ind_occurrence(decl, dom) {
                    return None;
                }
                binders.push((patt, dom));
                cursor = body;
            }
            Exp::InductiveType(d, args) if d.iri == decl.iri => {
                // An occurrence inside its own index arguments is not something the eliminator can
                // build a hypothesis for; rejected here and diagnosed by `check_arg_positivity`.
                let n_params = decl.params.len();
                if args.len() < n_params
                    || args[n_params..].iter().any(|a| has_ind_occurrence(decl, a))
                {
                    return None;
                }
                return Some(RecArgShape { binders, args });
            }
            _ => return None,
        }
    }
}

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
/// `ctor_app_params_ok` (inductive.rs @ `6ae1f0c`) — without this, a
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
/// 2. [`recursive_arg_shape`] classifies it as an occurrence — either direct,
///    `D(params)(indices)`, or higher-order positive, `(a : A) → D(params)(indices)` with `D`
///    absent from every domain (eigenius#92) — and the parameter prefix passes through
///    unchanged → accept.
/// 3. Otherwise the inductive appears in a domain, nested inside another inductive, or in the
///    occurrence's own indices — reject, with a diagnostic naming which.
fn check_arg_positivity(
    decl: &InductiveDecl,
    ctor_name: &str,
    arg_typ: &Exp,
    param_refs: &[Option<String>],
) -> Result<(), String> {
    if !has_ind_occurrence(decl, arg_typ) {
        return Ok(());
    }
    if let Some(shape) = recursive_arg_shape(decl, arg_typ) {
        let n_params = decl.params.len();
        let n_indices = decl.indices.len();
        if shape.args.len() != n_params + n_indices {
            return Err(format!(
                "constructor `{}.{ctor_name}`: recursive occurrence of `{}` \
                 must apply {} parameter(s) + {} index/indices, got {} argument(s)",
                decl.name,
                decl.name,
                n_params,
                n_indices,
                shape.args.len()
            ));
        }
        // Binders standing in front of the occurrence shadow parameter names for the
        // uniformity check: inside `(A : Set) → D(A)` the `A` in `D(A)` is the BINDER's
        // `A`, not the declaration parameter of that name, so uniformity is unsatisfiable
        // through it. Applying the same shadowing rule the ctor telescope uses.
        let mut local_refs = param_refs.to_vec();
        for (patt, _) in &shape.binders {
            shadow_patt(&mut local_refs, patt);
        }
        return check_params_uniform(
            decl,
            ctor_name,
            &shape.args[..n_params],
            &local_refs,
            "a recursive occurrence",
        );
    }
    // `recursive_arg_shape` returned `None` on a type that DOES mention the inductive, so the
    // occurrence is in a position this fragment does not admit. Name which one — the three
    // are different mistakes and a reader who gets "not a direct recursive position" for a
    // negative occurrence learns nothing about why it is unsound.
    Err(match classify_bad_occurrence(decl, arg_typ) {
        BadOccurrence::Negative => format!(
            "non-positive occurrence: constructor `{}.{ctor_name}` has `{}` in the DOMAIN \
             of an argument's function type. A negative occurrence admits a fixpoint that \
             inhabits every proposition; `(A -> {}) -> {}` is fine, `({} -> A) -> {}` is not",
            decl.name, decl.name, decl.name, decl.name, decl.name, decl.name
        ),
        BadOccurrence::InOwnIndices => format!(
            "non-positive occurrence: constructor `{}.{ctor_name}` has a nested inductive \
             use of `{}` inside its own indices",
            decl.name, decl.name
        ),
        BadOccurrence::Nested => format!(
            "non-positive occurrence: constructor `{}.{ctor_name}` mentions inductive `{}` \
             nested inside another type's arguments (eigenius#21 — nested inductives need \
             the specialize/unspecialize pass and are not supported)",
            decl.name, decl.name
        ),
    })
}

/// Which inadmissible position an occurrence sits in — for diagnostics only.
enum BadOccurrence {
    /// `I` left of an arrow: the unsound case.
    Negative,
    /// `I` inside the index arguments of a recursive occurrence of `I`.
    InOwnIndices,
    /// `I` inside some other type's arguments (`List(I)`), or any other shape.
    Nested,
}

/// Distinguish the three rejection cases so the error can say which one applies.
fn classify_bad_occurrence(decl: &InductiveDecl, typ: &Exp) -> BadOccurrence {
    let mut cursor = typ;
    loop {
        match cursor {
            Exp::Pi(_, dom, body) => {
                if has_ind_occurrence(decl, dom) {
                    return BadOccurrence::Negative;
                }
                cursor = body;
            }
            Exp::InductiveType(d, args) if d.iri == decl.iri => {
                let n_params = decl.params.len();
                let tail = args.get(n_params..).unwrap_or(&[]);
                return if tail.iter().any(|a| has_ind_occurrence(decl, a)) {
                    BadOccurrence::InOwnIndices
                } else {
                    BadOccurrence::Nested
                };
            }
            _ => return BadOccurrence::Nested,
        }
    }
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
        // Cross-institution translation — scan the source
        // expression; the comorphism IRI is opaque.
        Exp::InstitutionInvoke { source, .. } => has_ind_occurrence(decl, source),
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
        | Exp::LitFloat(_)
        | Exp::LitBool(_) => false,
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
            sort: Exp::sort(1),
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
            sort: Exp::sort(1),
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
            sort: Exp::sort(1),
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

    /// **eigenius#92 — a higher-order POSITIVE occurrence is admitted.**
    ///
    /// This test read `rejects_higher_order_positive` until eigenius#92. The inversion is the
    /// substance of that issue, so the reason is recorded rather than the assertion silently
    /// flipping: `(Nat → Foo) → Foo` is strictly positive in the classical sense — `Foo` occurs
    /// only in the CODOMAIN — and the criterion that rejected it was narrower than the type
    /// theory requires. It is the shape the bootstrap needs (`lexicon:Cat`'s `cat_forall`,
    /// `cat_fin_forall`, `cat_num_forall`), so wiring the pass into the declaration path with the
    /// old criterion would have rejected `ontologies/lexicon/lexicon-ontology.esl`.
    ///
    /// Step 1 admitted the declaration while the eliminator still skipped such an argument — a
    /// completeness limit, not a soundness one, since both halves skipped it identically. Step 2
    /// gave it a function-typed induction hypothesis, so induction through `mk` now computes; see
    /// `iota_recurses_through_a_higher_order_argument` in `nbe/eval/iota.rs`.
    ///
    /// `rejects_negative_occurrence` and `rejects_disguised_inductive_negative_occurrence` are the
    /// other side of this line and still pass unchanged: `(Foo → Nat) → Foo` stays rejected.
    #[test]
    fn accepts_higher_order_positive() {
        // Foo : (Nat → Foo) → Foo  — Foo only in the codomain
        let s = self_ref("Foo");
        let foo_ty = Exp::InductiveType(s, Vec::new());
        let nat_ty = Exp::Var("Nat".to_string());
        let decl = InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Foo").unwrap(),
            name: "Foo".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
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
        check_positivity(&decl).expect("`(Nat -> Foo) -> Foo` is strictly positive");

        // And the shape is classified as recursive-but-not-direct, which is what keeps the two
        // eliminator halves in step: both consult `is_direct` and both skip it.
        let arg_typ = match &decl.ctors[0].typ {
            Exp::Pi(_, dom, _) => (**dom).clone(),
            other => panic!("expected a Pi ctor type, got {other:?}"),
        };
        let shape = recursive_arg_shape(&decl, &arg_typ).expect("recursive occurrence");
        assert_eq!(
            shape.binders.len(),
            1,
            "one binder in front of the occurrence"
        );
        assert!(
            !shape.is_direct(),
            "higher-order, so not a direct recursive arg"
        );
    }

    /// The negative counterpart, spelled next to its positive twin because the ONLY difference is
    /// which side of the arrow `Foo` sits on, and that difference is the whole of positivity.
    #[test]
    fn rejects_negative_occurrence_in_the_same_shape() {
        // Foo : (Foo → Nat) → Foo  — Foo in the DOMAIN
        let s = self_ref("Foo");
        let foo_ty = Exp::InductiveType(s, Vec::new());
        let nat_ty = Exp::Var("Nat".to_string());
        let decl = InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Foo").unwrap(),
            name: "Foo".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: Exp::Pi(
                    Patt::Unit,
                    Box::new(Exp::Pi(
                        Patt::Unit,
                        Box::new(foo_ty.clone()),
                        Box::new(nat_ty),
                    )),
                    Box::new(foo_ty),
                ),
            }],
        };
        let err = check_positivity(&decl).expect_err("`(Foo -> Nat) -> Foo` must be rejected");
        assert!(
            err.contains("DOMAIN"),
            "the diagnostic should name the domain, not just say non-positive: {err}"
        );
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
            sort: Exp::sort(1),
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
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: Exp::Pi(
                    Patt::Unit,
                    Box::new(Exp::Var("Nat".to_string())),
                    Box::new(Exp::sort(1)),
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
    /// `6ae1f0c`). Without the check, the derived IH for such an arg is
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
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: Exp::Pi(
                    Patt::Var("A".to_string()),
                    Box::new(Exp::sort(1)),
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
            sort: Exp::sort(1),
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
