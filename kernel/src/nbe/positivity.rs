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
    ///
    /// `Vec<&Exp>` rather than `&[Exp]` since D76 Phase B. A fused
    /// `InductiveType(d, args)` has its arguments contiguous, but a de-fused
    /// `App(App(Const(I), a₁), a₂)` does not — they are nested one per `App`.
    /// Borrowed references collect a spine's arguments without cloning.
    pub args: Vec<&'a Exp>,
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
            // D76 Phase B — a bare `Const` naming this inductive is a
            // zero-argument recursive occurrence, the form the stub used to take.
            //
            // `has_ind_occurrence` alone is not enough: this classifier decides
            // the *shape*, and `check_arg_positivity` treats an unclassifiable
            // occurrence as a bad one — so without this arm a strictly positive
            // constructor written with a `Const` was **rejected**. The failure is
            // over-strictness, not unsoundness: `None` here reaches
            // `Err(classify_bad_occurrence(..))`, never a silent accept.
            Exp::Const(iri, _) if *iri == decl.iri => {
                return Some(RecArgShape {
                    binders,
                    args: Vec::new(),
                });
            }
            // D76 Phase B — a **de-fused** occurrence: `App(App(Const(I), a₁), a₂)`.
            //
            // Peel the spine to its head. This is the same occurrence the fused
            // `InductiveType(d, args)` arm below classifies, written the way the
            // wire has always written it (`encode_type_json` emits
            // `ConstRef(iri)` + an `App` spine), so both forms must be
            // recognised while the migration is in flight.
            //
            // Arguments come off the spine outermost-first, so they are
            // collected and reversed — the fused arm's `args` are already in
            // application order.
            Exp::App(..) => {
                // A de-fused occurrence. `as_const_spine` is the shared walker —
                // it returns `None` for a head that is not a name (some other
                // application that happens to mention the inductive;
                // unclassifiable here, and `check_arg_positivity` diagnoses it)
                // and recovers the arguments in application order.
                let (iri, _levels, spine) = cursor.as_const_spine()?;
                if *iri != decl.iri {
                    return None;
                }
                // An occurrence inside its own index arguments is not something
                // the eliminator can build a hypothesis for — the same rule the
                // fused arm applies.
                let n_params = decl.params.len();
                if spine.len() < n_params
                    || spine[n_params..]
                        .iter()
                        .any(|a| has_ind_occurrence(decl, a))
                {
                    return None;
                }
                return Some(RecArgShape {
                    binders,
                    args: spine,
                });
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
    param_args: &[&Exp],
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
            e if e
                .as_const_spine()
                .is_some_and(|(iri, _, _)| *iri == decl.iri) =>
            {
                let (_, _, args) = e.as_const_spine().expect("just matched");
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
        // D76 Phase B — a constructor may conclude in a bare `Const` naming its
        // inductive, the zero-argument case of the application below.
        //
        // The third site in this module that must know the occurrence form:
        // `has_ind_occurrence` says *whether*, `recursive_arg_shape` says *what
        // shape*, this says *is it a valid conclusion*. All three fail by
        // over-rejecting a new form, none by admitting a bad one — the module is
        // fail-closed on shapes it does not recognise.
        e if e
            .as_const_spine()
            .is_some_and(|(iri, _, _)| *iri == decl.iri) =>
        {
            let (_, _, args) = e.as_const_spine().expect("just matched");
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
        Exp::InductiveCtor(iri, _, args) => {
            *iri == decl.iri || args.iter().any(|a| has_ind_occurrence(decl, a))
        }
        Exp::InductiveRec {
            iri,
            motive,
            minors,
            major,
        } => {
            *iri == decl.iri
                || has_ind_occurrence(decl, motive)
                || minors.iter().any(|m| has_ind_occurrence(decl, m))
                || has_ind_occurrence(decl, major)
        }
        // D78 §1 — a record's field types are subterms; any occurrence in one
        // counts, the same as under a Π or Σ.
        // D76 Phase B1 — a `Const` naming this inductive is a recursive
        // occurrence, exactly as the stub was. nanoda scans for occurrences of
        // `st.ind_consts` (`inductive.rs:762`), a Vec covering the whole mutual
        // block; this scans for one, which is §6.5's gap.
        Exp::Const(iri, _) => *iri == decl.iri,

        Exp::Record(fields) => fields.iter().any(|(_, _, ty)| has_ind_occurrence(decl, ty)),

        // The constraint set is names; only the carrier can hold an occurrence.
        Exp::Refine(carrier, _) => has_ind_occurrence(decl, carrier),

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
            uparams: Vec::new(),
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
        let nat_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
        let decl = InductiveDecl {
            uparams: Vec::new(),
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
        let list_ty =
            Exp::const_applied(s.iri.clone(), Vec::new(), vec![Exp::Var("A".to_string())]);
        let decl = InductiveDecl {
            uparams: Vec::new(),
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
        let bool_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
        let decl = InductiveDecl {
            uparams: Vec::new(),
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
        let bad_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
        let nat_ty = Exp::Var("Nat".to_string());
        let decl = InductiveDecl {
            uparams: Vec::new(),
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
        let foo_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
        let nat_ty = Exp::Var("Nat".to_string());
        let decl = InductiveDecl {
            uparams: Vec::new(),
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
        let foo_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
        let nat_ty = Exp::Var("Nat".to_string());
        let decl = InductiveDecl {
            uparams: Vec::new(),
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
        let tree_ty = Exp::const_applied(tree_self.iri.clone(), Vec::new(), Vec::new());
        let nested = Exp::const_applied(list_self.iri.clone(), Vec::new(), vec![tree_ty.clone()]);
        let decl = InductiveDecl {
            uparams: Vec::new(),
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
            uparams: Vec::new(),
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
        let rec_occ_wrong_param = Exp::const_applied(s.iri.clone(), Vec::new(), vec![Exp::One]);
        let conclusion =
            Exp::const_applied(s.iri.clone(), Vec::new(), vec![Exp::Var("A".to_string())]);
        let decl = InductiveDecl {
            uparams: Vec::new(),
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

    /// **Retired premise, D76 Phase B.** This closed finding F-1 (port-fidelity
    /// analysis, `docs/notes/nbe-reorganization-analysis.md` §4): `Exp::Inductive(d)`
    /// evaluated to the same `Val::InductiveType` as `Exp::InductiveType(d, [])`, so
    /// a negative occurrence written in the first form evaded the checker until
    /// `has_ind_occurrence` learned to treat it as an occurrence.
    ///
    /// **The second spelling no longer exists** — one `Const` names a type former,
    /// applied or not — so the disguise is not expressible and the finding is closed
    /// by construction rather than by a scan. The test stays as the direct
    /// assertion: a negative occurrence is rejected.
    #[test]
    fn rejects_disguised_inductive_negative_occurrence() {
        // Neg { mk : (Neg → 1) → Neg }, the negative `Neg` in a Π domain.
        let s = self_ref("Neg");
        let neg_ty = Exp::const_applied(s.iri.clone(), Vec::new(), Vec::new());
        let disguised_negative = Exp::Pi(Patt::Unit, Box::new(neg_ty.clone()), Box::new(Exp::One));
        let decl = InductiveDecl {
            uparams: Vec::new(),
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
        let err = check_positivity(&decl).expect_err("disguised negative occurrence");
        assert!(err.contains("non-positive"), "got: {err}");
    }
}

#[cfg(test)]
mod const_self_reference {
    //! D76 Phase B1 — a self-reference is a `Const`, as in nanoda.

    use super::*;
    use crate::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, Patt};

    fn iri(s: &str) -> crate::ontology::iri::Iri {
        crate::ontology::iri::Iri::parse(s).unwrap()
    }

    fn decl_with(ctor_typ: Exp) -> InductiveDecl {
        InductiveDecl {
            uparams: Vec::new(),
            iri: iri("urn:test:T"),
            name: "T".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: ctor_typ,
            }],
        }
    }

    #[test]
    fn a_const_naming_the_inductive_is_a_recursive_occurrence() {
        // What the stub did, without the stub. `has_ind_occurrence` must see a
        // `Const` bearing the declaration's own IRI exactly as it saw an
        // `InductiveType` carrying a stub of it.
        let d = decl_with(Exp::sort(1));
        assert!(
            has_ind_occurrence(&d, &Exp::Const(iri("urn:test:T"), Vec::new())),
            "a Const naming this inductive is a recursive occurrence"
        );
        assert!(
            !has_ind_occurrence(&d, &Exp::Const(iri("urn:test:Other"), Vec::new())),
            "a Const naming a different declaration is not"
        );
    }

    #[test]
    fn a_de_fused_parametric_occurrence_is_classified() {
        // `List(A)` written as `App(Const(List), A)` — the form replacing a stub
        // produces, and the form the wire has always used.
        let list = InductiveDecl {
            uparams: Vec::new(),
            iri: iri("urn:test:List"),
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: Vec::new(),
        };
        let occurrence = Exp::App(
            Box::new(Exp::Const(iri("urn:test:List"), Vec::new())),
            Box::new(Exp::Var("A".to_string())),
        );

        let shape = recursive_arg_shape(&list, &occurrence)
            .expect("a de-fused parametric occurrence must classify");
        assert_eq!(shape.args.len(), 1, "the spine's argument is recovered");
        assert!(
            shape.binders.is_empty(),
            "a direct occurrence has no binders"
        );
        assert!(
            matches!(shape.args[0], Exp::Var(n) if n == "A"),
            "and in application order: {:?}",
            shape.args[0]
        );
    }

    #[test]
    fn a_de_fused_occurrence_under_a_binder_keeps_its_binders() {
        // `(n : Nat) → List(A)` — higher-order positive.
        let list = InductiveDecl {
            uparams: Vec::new(),
            iri: iri("urn:test:List"),
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: Vec::new(),
        };
        let occurrence = Exp::Pi(
            Patt::Var("n".to_string()),
            Box::new(Exp::sort(1)),
            Box::new(Exp::App(
                Box::new(Exp::Const(iri("urn:test:List"), Vec::new())),
                Box::new(Exp::Var("A".to_string())),
            )),
        );
        let shape = recursive_arg_shape(&list, &occurrence).expect("higher-order positive");
        assert_eq!(shape.binders.len(), 1, "the Π binder is kept");
        assert_eq!(shape.args.len(), 1);
    }

    #[test]
    fn a_de_fused_negative_occurrence_is_still_refused() {
        // `(List(A) → Nat) → …` — the inductive in a domain.
        let list = InductiveDecl {
            uparams: Vec::new(),
            iri: iri("urn:test:List"),
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: Vec::new(),
        };
        let neg = Exp::Pi(
            Patt::Unit,
            Box::new(Exp::App(
                Box::new(Exp::Const(iri("urn:test:List"), Vec::new())),
                Box::new(Exp::Var("A".to_string())),
            )),
            Box::new(Exp::sort(1)),
        );
        assert!(
            recursive_arg_shape(&list, &neg).is_none(),
            "an occurrence in a Π domain is not a positive recursive argument"
        );
    }

    #[test]
    fn positivity_still_rejects_a_negative_self_occurrence_expressed_as_a_const() {
        // The control that matters: swapping the representation must not lose
        // the check. `mk : (T → T) → T` with the self-reference as a `Const`.
        let self_ref = || Exp::Const(iri("urn:test:T"), Vec::new());
        let bad = decl_with(Exp::Pi(
            Patt::Unit,
            Box::new(Exp::Pi(
                Patt::Unit,
                Box::new(self_ref()),
                Box::new(self_ref()),
            )),
            Box::new(self_ref()),
        ));
        assert!(
            check_positivity(&bad).is_err(),
            "a negative self-occurrence must still be rejected when written as a Const"
        );

        // And a positive one is still accepted: `mk : T → T`.
        let good = decl_with(Exp::Pi(
            Patt::Unit,
            Box::new(self_ref()),
            Box::new(self_ref()),
        ));
        check_positivity(&good).expect("a strictly positive self-occurrence must be accepted");
    }
}

#[cfg(test)]
mod mutual_positivity_gap {
    //! D76 §6.5 — does `check_positivity` catch a negative occurrence that
    //! crosses between two mutually-recursive inductives?

    use super::*;
    use crate::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, Patt};
    use std::sync::Arc;

    fn stub(name: &str) -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).unwrap(),
            name: name.to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: Vec::new(),
        })
    }
    fn ty(name: &str) -> Exp {
        Exp::const_applied(stub(name).iri.clone(), Vec::new(), Vec::new())
    }
    fn decl(name: &str, ctors: Vec<InductiveCtorDecl>) -> InductiveDecl {
        InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).unwrap(),
            name: name.to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors,
        }
    }

    /// Control: a type negative in **its own** constructor is rejected. Without
    /// this the test below proves nothing — it would pass on a checker that
    /// never rejects anything.
    #[test]
    fn a_self_negative_occurrence_is_rejected() {
        let bad = decl(
            "SelfBad",
            vec![InductiveCtorDecl {
                name: "mk".to_string(),
                // mk : (SelfBad → SelfBad) → SelfBad
                typ: Exp::Pi(
                    Patt::Unit,
                    Box::new(Exp::Pi(
                        Patt::Unit,
                        Box::new(ty("SelfBad")),
                        Box::new(ty("SelfBad")),
                    )),
                    Box::new(ty("SelfBad")),
                ),
            }],
        );
        assert!(
            check_positivity(&bad).is_err(),
            "a type occurring to the left of an arrow in its own ctor must be rejected"
        );
    }

    /// **The gap.** `A` and `B` are mutually recursive, and `A` occurs
    /// **negatively** in `B`'s constructor:
    ///
    /// ```text
    /// A ::= mkA (B → A)        -- B positive in A
    /// B ::= mkB ((A → A) → B)  -- A NEGATIVE, and it is B being checked
    /// ```
    ///
    /// If `{A, B}` were a mutual block this violates strict positivity: every
    /// member must occur only strictly positively in every constructor of the
    /// block. `check_positivity(&b)` scans `B`'s constructors for occurrences of
    /// **`B`**, and there are none in the offending position — the negative
    /// occurrence is `A`.
    #[test]
    fn a_cross_type_negative_occurrence_is_not_caught() {
        let a = decl(
            "MutA",
            vec![InductiveCtorDecl {
                name: "mkA".to_string(),
                // mkA : B → A   (B strictly positive; A fine)
                typ: Exp::Pi(Patt::Unit, Box::new(ty("MutB")), Box::new(ty("MutA"))),
            }],
        );
        let b = decl(
            "MutB",
            vec![InductiveCtorDecl {
                name: "mkB".to_string(),
                // mkB : (A → A) → B   ← A to the LEFT of an arrow
                typ: Exp::Pi(
                    Patt::Unit,
                    Box::new(Exp::Pi(
                        Patt::Unit,
                        Box::new(ty("MutA")),
                        Box::new(ty("MutA")),
                    )),
                    Box::new(ty("MutB")),
                ),
            }],
        );

        let a_ok = check_positivity(&a).is_ok();
        let b_ok = check_positivity(&b).is_ok();

        assert!(a_ok, "A alone is positive — it is B's ctor that offends");
        assert!(
            b_ok,
            "**D76 §6.5, the finding**: B's ctor puts A to the left of an arrow, which violates \
             strict positivity for the block {{A, B}}. `check_positivity(&b)` scans for occurrences \
             of B and finds none there, so it passes. Checked apart, the pair is admitted.\n\n\
             This is INCOMPLETENESS in a per-declaration checker, not a hole in the rule it \
             implements — there is no mutual-block checker to be incomplete *for* (#20). It becomes \
             unsoundness the moment mutual blocks are admitted without simultaneous positivity.\n\n\
             If this assertion ever fails, cross-type positivity has been implemented and D76 §6.5 \
             should be closed."
        );
    }

    /// What stops the gap being exploitable today: an eliminator over the pair
    /// does not exist. `derive_recursor_type` is per-declaration, so there is no
    /// cross-type recursion to smuggle a non-terminating term through.
    /// **Nothing stops a mutual pair being committed today.** It compiles from
    /// ESL and validates clean, then sits in the chain uneliminable — no shared
    /// recursor exists (#20). With `a_cross_type_negative_occurrence_is_not_caught`
    /// above, a *non-positive* pair commits clean too.
    ///
    /// This is the "nothing failed; the wrong thing succeeded" shape. The fix is
    /// **fail-closed detection**, and the detector is already designed: D76
    /// Phase A's SCC pass computes exactly "does this layer contain an inductive
    /// SCC larger than one". Rejecting that with a message naming #20 costs
    /// nothing once Phase A exists.
    ///
    /// Rejecting it is not the Band-Aid CLAUDE.md warns about. That guidance is
    /// about refusing input *that should be expressible* — papering over a wrong
    /// AST or grammar. A mutual block should be expressible once #20 is built;
    /// until then, accepting it and producing something uneliminable is the
    /// defect. Fail-closed converts silent acceptance into a tracked limitation.
    #[test]
    fn a_mutual_pair_commits_clean_today() {
        let esl = r#"
namespace core = "urn:eigenius:core";
namespace t    = "urn:test";
data t:A { description = "half of a mutually-recursive pair"; mkA(t:B) }
data t:B { description = "the other half"; mkB(t:A) }
"#;
        let toks = crate::esl::lexer::tokenize(esl).expect("lexes");
        let file = crate::esl::parser::parse(&toks).expect("parses");
        let rs = crate::esl::compile::compile_file(&file).expect("compiles");
        assert_eq!(rs.len(), 2, "two inductive declarations");

        let core = crate::bootstrap::bootstrap().expect("bootstrap");
        let mut b = crate::layer::LayerBuilder::new(
            "mutual-pair",
            Some(std::sync::Arc::clone(core.head())),
        );
        for r in rs {
            b.add_resource(r).unwrap();
        }
        let layer = std::sync::Arc::new(b.build(crate::layer::LayerStorage::in_memory()));
        let errs = crate::validation::Validator::new(layer).validate();

        assert!(
            errs.is_empty(),
            "current behaviour: a mutually-recursive pair validates clean. If this starts \
             failing, fail-closed detection has landed (D76 §6.5) — check that the diagnostic \
             names #20, then close this test out. Got: {errs:?}"
        );
    }

    #[test]
    fn no_eliminator_spans_the_pair() {
        // The kernel has no mutual-block construct at all — the check is that
        // `InductiveDecl` cannot even name a sibling as part of its own block.
        let b = decl("MutB", vec![]);
        assert!(
            b.ctors.is_empty(),
            "sanity: an InductiveDecl carries only its own constructors, so a mutual block \
             has no representation and no shared recursor"
        );
    }
}

/// D76 Phase B — where a level argument can live, and what equality then sees.
///
/// The phase was specified as "`PartialEq for InductiveDecl` becomes structural,
/// and that is what unblocks #188". Both halves are wrong, and these tests are
/// the check.
#[cfg(test)]
mod level_slot {
    use crate::nbe::level::Level;
    use crate::nbe::term::{Exp, InductiveDecl, Patt};
    use crate::ontology::iri::Iri;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn decl_at(sort: Level) -> InductiveDecl {
        InductiveDecl {
            uparams: Vec::new(),
            iri: iri("urn:test:List"),
            name: "List".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: Vec::new(),
            sort: Exp::Sort(sort),
            ctors: Vec::new(),
        }
    }

    #[test]
    fn the_fused_form_has_nowhere_to_put_a_level_but_inside_the_declaration() {
        // `Exp::const_applied(decl.iri.clone(), Vec::new(), args)` has two slots: a declaration and
        // value arguments. A *level* argument is neither, so instantiating
        // `List.{0}` and `List.{1}` can only differ inside the declaration —
        // and declaration identity is the IRI, which does not see it.
        let at_zero = decl_at(Level::of_nat(0));
        let at_one = decl_at(Level::of_nat(1));
        assert_ne!(at_zero.sort, at_one.sort, "the declarations do differ");
        assert_eq!(
            at_zero, at_one,
            "yet they compare equal — identity is the IRI, so a level folded \
             into the declaration is invisible to equality"
        );
    }

    #[test]
    fn the_de_fused_form_carries_the_level_on_the_reference_and_equality_sees_it() {
        // `Const(iri, levels)` has the slot the fused form lacks. This is
        // nanoda's shape — `def_eq_const` is `name == name && levels equal`
        // (`references/nanoda_lib/src/tc.rs:886`) — and it is what actually
        // unblocks #188's residual.
        let at_zero = Exp::const_applied(
            iri("urn:test:List"),
            vec![Level::of_nat(0)],
            vec![Exp::Var("A".to_string())],
        );
        let at_one = Exp::const_applied(
            iri("urn:test:List"),
            vec![Level::of_nat(1)],
            vec![Exp::Var("A".to_string())],
        );
        assert_ne!(
            at_zero, at_one,
            "two instantiations of one declaration must not be interconvertible"
        );

        let (head, levels, args) = at_zero.as_const_spine().expect("a const spine");
        assert_eq!(head, &iri("urn:test:List"), "one declaration, named once");
        assert_eq!(levels, &[Level::of_nat(0)]);
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn an_unresolved_reference_keeps_its_levels_through_readback() {
        // `Neut::Const` used to carry only the IRI, so readback rebuilt
        // `Exp::Const(iri, vec![])` and the level arguments were silently gone.
        // Nothing produces non-empty levels yet, which is exactly why this had to
        // be fixed before E2 rather than by E2.
        use crate::nbe::env::Rho;
        use crate::nbe::eval::{eval_ctx, EvalCtx};
        use crate::nbe::readback::readback_val;

        let reference = Exp::Const(iri("urn:test:Unresolvable"), vec![Level::of_nat(2)]);
        let value = eval_ctx(&reference, &Rho::Nil, &EvalCtx::pure()).expect("eval");
        assert_eq!(
            readback_val(0, &value),
            reference,
            "the level argument must survive eval → readback"
        );
    }

    #[test]
    fn a_de_fused_application_reaches_the_same_value_as_the_fused_one() {
        // The equivalence Phase B's sweep rests on: `App(Const(I), a)` evaluated
        // in an environment must produce the value `InductiveType(I, [a])`
        // produced. If it did not, de-inlining would be a semantic change rather
        // than a representation change.
        use crate::nbe::env::Rho;
        use crate::nbe::env_global::Env;
        use crate::nbe::eval::{eval_ctx, EvalCtx};
        use crate::nbe::val::Val;

        let level_iri = Iri::parse("urn:eigenius:core:Level").unwrap();
        let layer = {
            let json = include_str!("../../../ontologies/core/core-ontology.json");
            let mut b = crate::layer::LayerBuilder::new("app-arm", None);
            for r in crate::ontology::eigon_json::parse_document(json).unwrap() {
                b.add_resource(r).unwrap();
            }
            std::sync::Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
        };
        let ctx = EvalCtx::in_env(Env::of(layer));

        let bare =
            eval_ctx(&Exp::Const(level_iri.clone(), Vec::new()), &Rho::Nil, &ctx).expect("eval");
        match &bare {
            Val::InductiveType {
                params, indices, ..
            } => assert!(
                params.is_empty() && indices.is_empty(),
                "an unapplied former carries no arguments"
            ),
            other => panic!("expected the type former, got {other:?}"),
        }

        // Applying it one argument at a time is what an `App` spine does. `List` is
        // the parametric former to use: `core:Level` takes NO arguments, and applying
        // it to one was only ever accepted because the arity check was suppressed
        // (D76 Phase B2). The check now refuses it, correctly.
        let list_iri = crate::nbe::term::list_decl().iri.clone();
        let applied = eval_ctx(
            &Exp::const_applied(list_iri.clone(), Vec::new(), vec![Exp::sort(1)]),
            &Rho::Nil,
            &ctx,
        )
        .expect("eval");
        match applied {
            Val::InductiveType {
                decl: d, params, ..
            } => {
                assert_eq!(d.iri, list_iri);
                assert_eq!(params.len(), 1, "the argument folded onto the former");
            }
            other => panic!("a de-fused application must stay a type: {other:?}"),
        }

        // And the nullary former refuses an argument it has no slot for — the
        // leniency B2 removed.
        assert!(
            eval_ctx(
                &Exp::const_applied(level_iri, Vec::new(), vec![Exp::sort(1)]),
                &Rho::Nil,
                &ctx,
            )
            .is_err(),
            "`core:Level` takes no arguments; applying one must be refused"
        );
    }

    #[test]
    fn identity_by_iri_is_the_reference_behaviour_not_a_workaround() {
        // The corollary: with levels on the reference, comparing declarations by
        // name is what nanoda does, and structural comparison would be *wrong* —
        // two lookups of one declaration must be equal however they decoded.
        let once = decl_at(Level::of_nat(1));
        let twice = decl_at(Level::of_nat(1));
        assert_eq!(once, twice);
    }
}

/// D76 Phase E2 / eigenius#188's residual — universe polymorphism, end to end.
///
/// Phase B built the *slot* (`Const(iri, levels)`); these pin that it now carries
/// something. Before E2 a polymorphic declaration compiled, persisted and
/// validated, and then every reference to it saw `Sort(Param("u"))` with nothing
/// bound — "implemented and unreachable", which is what N3 §3 warned the feature
/// would be if the surface landed without instantiation.
#[cfg(test)]
mod universe_polymorphism {
    use crate::nbe::level::Level;
    use crate::nbe::term::Exp;
    use crate::ontology::iri::Iri;

    fn polymorphic_layer() -> std::sync::Arc<crate::layer::Layer> {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let mut b = crate::layer::LayerBuilder::new("core", None);
        for r in crate::ontology::eigon_json::parse_document(core_json).unwrap() {
            b.add_resource(r).unwrap();
        }
        let core = std::sync::Arc::new(b.build(crate::layer::LayerStorage::in_memory()));
        let src = r#"
            namespace core = "urn:eigenius:core";
            namespace p    = "urn:eigenius:p";
            universe u;
            data p:Box(A : Sort u) : Sort u { mk(A), }
        "#;
        let mut d = crate::layer::LayerBuilder::new("p", Some(core));
        for r in crate::esl::compile(src).expect("polymorphic ESL compiles") {
            d.add_resource(r).unwrap();
        }
        std::sync::Arc::new(d.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn a_declaration_binds_the_level_variables_it_mentions() {
        // Generalisation: `universe u;` is FILE-scoped, so what binds `u` on this
        // declaration is that the declaration uses it.
        let layer = polymorphic_layer();
        let iri = Iri::parse("urn:eigenius:p:Box").unwrap();
        match crate::nbe::env_global::Env::of(layer).lookup(&iri) {
            crate::nbe::env_global::Global::Inductive(d) => {
                assert_eq!(d.uparams, vec!["u".to_string()], "Box binds exactly `u`");
                assert!(
                    matches!(&d.sort, Exp::Sort(Level::Param(n)) if n == "u"),
                    "and its sort is that parameter, not a numeral: {:?}",
                    d.sort
                );
            }
            other => panic!("expected an inductive, got {other:?}"),
        }
    }

    #[test]
    fn a_reference_instantiates_the_declaration_at_its_level_argument() {
        // The payoff. `Box.{0}` and `Box.{1}` are different types, which is what
        // `level_slot` said the fused representation could not express.
        use crate::nbe::env::Rho;
        use crate::nbe::eval::{eval_env, EvalCtx};
        let _ = EvalCtx::pure;
        let env = crate::nbe::env_global::Env::of(polymorphic_layer());
        let iri = Iri::parse("urn:eigenius:p:Box").unwrap();

        let at = |k: usize| {
            let e = Exp::Const(iri.clone(), vec![Level::of_nat(k)]);
            match eval_env(&e, &Rho::Nil, &env).expect("eval") {
                crate::nbe::val::Val::InductiveType { decl, .. } => decl,
                other => panic!("expected the type former, got {other:?}"),
            }
        };

        let zero = at(0);
        let one = at(1);
        assert!(
            zero.uparams.is_empty(),
            "instantiation CONSUMES the parameter — the result is monomorphic"
        );
        assert!(
            matches!(&zero.sort, Exp::Sort(l) if l.is_nat(0)),
            "Box.{{0}} is at Sort(0): {:?}",
            zero.sort
        );
        assert!(
            matches!(&one.sort, Exp::Sort(l) if l.is_nat(1)),
            "Box.{{1}} is at Sort(1): {:?}",
            one.sort
        );
        assert_ne!(
            zero.sort, one.sort,
            "and the two instantiations differ — #188's residual, closed"
        );
    }

    #[test]
    fn level_arguments_round_trip_through_the_wire() {
        // The chain-format half. A monomorphic reference is byte-identical to what
        // shipped before; a polymorphic one carries its arguments.
        use crate::program::eigentt_type_mirror::{decode_type, encode_type};
        let layer = polymorphic_layer();
        let iri = Iri::parse("urn:eigenius:p:Box").unwrap();

        let poly = Exp::Const(iri.clone(), vec![Level::of_nat(1)]);
        let encoded = encode_type(&poly).expect("encodes");
        let decoded = decode_type(&encoded, &layer).expect("decodes");
        assert_eq!(decoded, poly, "a level-carrying reference round-trips");

        // Monomorphic: one argument, exactly as before E2.
        let mono = Exp::Const(iri, Vec::new());
        let enc_mono = encode_type(&mono).expect("encodes");
        let as_json = match &enc_mono {
            crate::ontology::resource::Value::Json(j) => j.clone(),
            other => panic!("expected Json, got {other:?}"),
        };
        assert_eq!(
            as_json["args"].as_array().map(|a| a.len()),
            Some(1),
            "a monomorphic ConstRef keeps its single argument: {as_json}"
        );
    }
}
