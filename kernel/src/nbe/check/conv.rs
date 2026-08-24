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

//! Conversion and subtyping: definitional equality via readback
//! (`eq_nf`), type-directed equality with D46 proof irrelevance
//! (`def_eq_at_type`), cumulativity/size-aware subtyping, and
//! propositionality classification. Split from `check.rs`.

use super::{check_infer, CheckCtx, CheckError};
use crate::nbe::env::gen_val;
use crate::nbe::readback::readback_val;
use crate::nbe::term::{Exp, Patt};
use crate::nbe::val::Val;

/// Check type equality by normalization.
///
/// Port of `eqNf` from the reference: normalize both sides
/// and compare syntactically.
pub fn eq_nf(level: usize, v1: &Val, v2: &Val) -> Result<(), CheckError> {
    // D49 §8 — ChainWitness values are opaque kernel-internal markers
    // that intentionally do not read back into surface syntax. Equality
    // on them is key-based: two witnesses with the same `WitnessKey`
    // are definitionally equal, and two with different keys are a type
    // mismatch.
    //
    // Key comparison is not a fast path: it is the only path. D46 proof
    // irrelevance would collapse *any* two witnesses of the same
    // Prop-typed predicate to equal at that type, but that route runs
    // through `def_eq_at_type`, whose only two production call sites are
    // both inside the `(Exp::Refl(a), Val::Id(..))` arm of `check`
    // (`check/mod.rs`). `eq_nf` takes no type and has no propositional
    // short-circuit, so every witness comparison that is not a `refl`
    // check lands here and is decided by key equality alone. Two
    // witnesses of one Prop-typed predicate with different keys are
    // therefore rejected, where D46 says they are equal. Wiring
    // irrelevance into the conversion algorithm is D46 §5.1 and is not
    match (v1, v2) {
        (Val::ChainWitness(k1), Val::ChainWitness(k2)) => {
            return if k1 == k2 {
                Ok(())
            } else {
                Err(CheckError::TypeMismatch(format!(
                    "ChainWitness keys differ: {} vs {}",
                    k1.category.label(),
                    k2.category.label(),
                )))
            };
        }
        (Val::ChainWitness(k), _) | (_, Val::ChainWitness(k)) => {
            return Err(CheckError::TypeMismatch(format!(
                "ChainWitness vs non-witness value (witness category {})",
                k.category.label(),
            )));
        }
        _ => {}
    }
    let e1 = readback_val(level, v1);
    let e2 = readback_val(level, v2);
    if e1 == e2 {
        Ok(())
    } else {
        Err(CheckError::TypeMismatch(format!(
            "type mismatch: {e1:?} ≠ {e2:?}"
        )))
    }
}

/// Whether `exp` contains a free reference to `Exp::Var(name)`.
/// Structural walk; binders that shadow `name` cut off the search
/// in their bodies. Used by the D48 Phase H singleton-elim extension
/// to decide whether a ctor arg "appears in the conclusion", and by the
/// DCG open-parse carrier (D64) to detect referent-hole free variables.
pub fn exp_mentions_var(exp: &Exp, name: &str) -> bool {
    match exp {
        Exp::Var(n) => n == name,
        Exp::Lam(patt, body) | Exp::Pi(patt, _, body) | Exp::Sig(patt, _, body) => {
            // Domain types are checked too (for Pi/Sig); the body is
            // only checked if the binder doesn't shadow.
            let dom_or_typ = if let Exp::Lam(_, _) = exp {
                None
            } else {
                Some(match exp {
                    Exp::Pi(_, dom, _) => dom.as_ref(),
                    Exp::Sig(_, dom, _) => dom.as_ref(),
                    _ => unreachable!(),
                })
            };
            let dom_hit = dom_or_typ
                .map(|d| exp_mentions_var(d, name))
                .unwrap_or(false);
            let shadowed = patt_binds(patt, name);
            let body_hit = !shadowed && exp_mentions_var(body, name);
            dom_hit || body_hit
        }
        Exp::App(h, a) => exp_mentions_var(h, name) || exp_mentions_var(a, name),
        Exp::Ann(e, t) => exp_mentions_var(e, name) || exp_mentions_var(t, name),
        Exp::Arrow(a, b) | Exp::Times(a, b) => {
            exp_mentions_var(a, name) || exp_mentions_var(b, name)
        }
        Exp::Pair(a, b) => exp_mentions_var(a, name) || exp_mentions_var(b, name),
        Exp::Fst(e) | Exp::Snd(e) => exp_mentions_var(e, name),
        Exp::Con(_, e) | Exp::Refl(e) => exp_mentions_var(e, name),
        Exp::Id(a, x, y) => {
            exp_mentions_var(a, name) || exp_mentions_var(x, name) || exp_mentions_var(y, name)
        }
        Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => {
            args.iter().any(|a| exp_mentions_var(a, name))
        }
        // For other Exp variants (Sort, One, Unit, Set, primitives,
        // EigonClass, etc.) there's no Var inside to find.
        _ => false,
    }
}

/// Whether `patt` binds `name`, shadowing any outer occurrence.
fn patt_binds(patt: &Patt, name: &str) -> bool {
    match patt {
        Patt::Var(n) => n == name,
        Patt::Pair(p1, p2) => patt_binds(p1, name) || patt_binds(p2, name),
        Patt::Unit => false,
    }
}

/// Conservative syntactic test for "this type expression inhabits Prop".
///
/// Returns true for known-propositional shapes: `Id(_, _, _)`,
/// Pi-into-Prop (impredicative), Sigma-of-two-Props, and applied
/// inductive/codata declarations whose `sort` is `Sort(0)`. Returns
/// false (conservatively — may reject a valid Prop arg that requires
/// evaluation to resolve) for variables, applications, neutrals, and
/// the universe `Sort(0)` itself (which inhabits `Sort(1)`).
pub(super) fn is_syntactically_propositional_type(typ: &Exp) -> bool {
    match typ {
        Exp::Id(_, _, _) => true,
        Exp::Pi(_, _, body) => is_syntactically_propositional_type(body),
        Exp::Arrow(_, body) => is_syntactically_propositional_type(body),
        Exp::Sig(_, dom, body) | Exp::Times(dom, body) => {
            is_syntactically_propositional_type(dom) && is_syntactically_propositional_type(body)
        }
        Exp::InductiveType(decl, _) => matches!(&decl.sort, Exp::Sort(l) if l.is_nat(0)),
        _ => false,
    }
}

/// Type-directed definitional equality with proof irrelevance (D46 §5).
///
/// If `typ` is propositional (inhabits `Sort(0)`), any two inhabitants are
/// definitionally equal regardless of structure — proof irrelevance fires
/// as a short-circuit before structural comparison. Otherwise falls back
/// to [`eq_nf`].
///
/// Propositionality is detected by [`is_propositional_in_ctx`]: a structural
/// fast-path for the common shapes (`Val::Id`, sort-Sort(0) inductives /
/// codata), then a full inference-based check that readbacks `typ` and
/// asks the kernel for its universe. The inference path covers the cases
/// the fast-path misses (Pi-into-Prop, Sigma-of-Props, neutrals whose
/// type reduces to Prop, etc.).
pub fn def_eq_at_type(ctx: &mut CheckCtx, v1: &Val, v2: &Val, typ: &Val) -> Result<(), CheckError> {
    if is_propositional_in_ctx(ctx, typ)? {
        return Ok(());
    }
    eq_nf(ctx.rho.len(), v1, v2)
}

/// Infer the universe of a dependent binder (Pi or Sigma). Used by
/// [`check_infer`] to compute the sort of a type-former for proof-
/// irrelevance classification (D46 §5.1) and other downstream needs.
///
/// `impredicative=true` applies the Pi impredicative rule (D46 §4.1):
/// when the codomain inhabits `Sort(0)`, the whole binder is in `Sort(0)`
/// regardless of the domain's level. `impredicative=false` (Sigma) always
/// takes `Sort(max(m, n))`.
pub(super) fn infer_dependent_sort(
    ctx: &mut CheckCtx,
    patt: &Patt,
    a: &Exp,
    b: &Exp,
    impredicative: bool,
) -> Result<Val, CheckError> {
    let a_sort = check_infer(ctx, a)?;
    let m = match a_sort {
        Val::Sort(m) => m,
        other => {
            return Err(CheckError::ExpectedSort(format!(
                "binder domain is not a sort: {:?}",
                readback_val(ctx.rho.len(), &other)
            )));
        }
    };
    let a_val = ctx.eval(a, &ctx.rho)?;
    let gen = gen_val(&ctx.rho);
    let mut inner = ctx.extend(patt, &a_val, &gen)?;
    let b_sort = check_infer(&mut inner, b)?;
    let n = match b_sort {
        Val::Sort(n) => n,
        other => {
            return Err(CheckError::ExpectedSort(format!(
                "binder codomain is not a sort: {:?}",
                readback_val(inner.rho.len(), &other)
            )));
        }
    };
    // eigenius#188: `Pi (a : A) (b : B)` lives at `imax (level A) (level B)` and `Sigma` at
    // `max`. With `usize` levels this was a branch on `n == 0`; with a level that may be a
    // `Param`, whether the codomain is `Prop` is not known until the parameter is instantiated,
    // and `IMax` is the term that defers it. `simplify` collapses it back to the old answer
    // whenever both levels are concrete — `imax m 0 == 0`, `imax m (k+1) == max m (k+1)`.
    let out = if impredicative {
        crate::nbe::level::Level::IMax(Box::new(m), Box::new(n))
    } else {
        crate::nbe::level::Level::Max(Box::new(m), Box::new(n))
    };
    Ok(Val::Sort(out.simplify()))
}

/// Decide whether `typ` is a propositional type (inhabits `Sort(0)`).
///
/// Three-stage decision: (1) structural fast-path for shapes whose
/// propositionality is decidable without inference; (2) if the fast-path
/// returns `None`, readback `typ` and call [`check_infer`] to classify
/// its universe; (3) classify `Sort(0)` as propositional, anything else
/// not. Per D46 §5.3, this is the type-inference path the spec calls
/// for; cost is one inference per call, memoised by `CheckCtx::type_cache`.
pub(super) fn is_propositional_in_ctx(ctx: &mut CheckCtx, typ: &Val) -> Result<bool, CheckError> {
    if let Some(decided) = is_propositional_type_structural(typ) {
        return Ok(decided);
    }
    let typ_exp = readback_val(ctx.rho.len(), typ);
    let typ_sort = check_infer(ctx, &typ_exp)?;
    Ok(matches!(&typ_sort, Val::Sort(l) if l.is_nat(0)))
}

/// Three-valued structural fast-path for propositional-type recognition.
///
/// - `Some(true)` — definitely propositional (`Val::Id`, sort-Sort(0)
///   inductive/codata).
/// - `Some(false)` — definitely not propositional (universes, primitives,
///   `One`, `SizeSort`, anonymous codata, EigonClass / EigonPrimitive,
///   inductive/codata at higher sorts).
/// - `None` — undecidable from shape alone; caller falls back to
///   inference. Reaches Pi, Sig, neutrals, lambdas/values reachable
///   through the catch-all.
fn is_propositional_type_structural(typ: &Val) -> Option<bool> {
    match typ {
        Val::Id(_, _, _) => Some(true),
        Val::InductiveType { decl, .. } => Some(matches!(&decl.sort, Exp::Sort(l) if l.is_nat(0))),
        Val::One | Val::Sort(_) | Val::EigonClass(_) | Val::EigonPrimitive(_) => Some(false),
        _ => None,
    }
}

/// Whether [`subtype_of_inner`] compares the index telescope of two
/// applications of the same inductive declaration.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Indices {
    /// Indices are compared with `eq_nf`, position-wise. The rule for
    /// every caller except constructor checking.
    Compare,
    /// Indices are left to the caller. The single caller is
    /// [`super::inductive::check_inductive_ctor_args`], which unifies the
    /// conclusion indices itself (D48 Phase D) immediately afterwards;
    /// unification can instantiate metavariables that `eq_nf` would reject.
    DeferToCaller,
}

/// Subtyping check: admits `sub <: super`.
///
/// Two rules, and after eigenius#218 that is all of them:
///
/// - **Universe cumulativity** — `Sort(m) <: Sort(n)` iff `m ≤ n` in the LEVEL order
///   (D46 §3.2, Prop ⊆ Set ⊆ Type(1) ⊆ …).
/// - **Everything else is definitional equality** (`eq_nf`), position-wise for an applied
///   inductive's parameters and indices alike.
///
/// It used to take a `Tso` of rigid size hypotheses, and the parameter telescope was
/// COVARIANT at `SizeSort` positions — `T(s) <: T(ŝ s) <: T(∞)`, the driving motivation for
/// sized types (D19 §8.3). Sized types are gone (#218), so no parameter position is covariant
/// any more and parameters are invariant exactly as indices always were (eigenius#137).
pub fn subtype_of(level: usize, sub: &Val, super_: &Val) -> Result<(), CheckError> {
    subtype_of_inner(level, sub, super_, Indices::Compare)
}

/// Constructor-site subtyping: [`subtype_of_with_hyps`] with the index
/// telescope left to the caller.
///
/// Only sound when the caller compares the indices itself. The one caller
/// is `check_inductive_ctor_args`, which runs D48 Phase D index unification
/// on the same pair of values on the next statement.
pub(super) fn subtype_of_deferring_indices(
    level: usize,
    sub: &Val,
    super_: &Val,
) -> Result<(), CheckError> {
    subtype_of_inner(level, sub, super_, Indices::DeferToCaller)
}

fn subtype_of_inner(
    level: usize,
    sub: &Val,
    super_: &Val,
    index_policy: Indices,
) -> Result<(), CheckError> {
    // Universe cumulativity: `Sort(m) <: Sort(n)` iff `m <= n` in the LEVEL order.
    // D46 §3.2 — Prop ⊆ Set ⊆ Type(1) ⊆ Type(2) ⊆ …
    //
    // eigenius#188: this must be `Level::leq`, not `<=`. `Level` deliberately does not derive
    // `Ord` — the derived order is structural (discriminant, then fields) and is not the universe
    // order at all: it would rank `Param("u")` against `Max(..)` by variant position. It happens
    // to agree on `Succ`-chains, which is exactly why the bug would not have shown up in a test.
    // ── D78 §3 — refinement subtyping ─────────────────────────────────────
    //
    // `Refine(R, S) <: R`: forgetting constraints is always safe, which is how a
    // refined record flows into a context expecting a plain record.
    if let Val::Refine(carrier, _) = sub {
        if !matches!(super_, Val::Refine(..)) {
            return subtype_of_inner(level, carrier, super_, index_policy);
        }
    }
    // `Refine(R, S) <: Refine(R′, S′)`.
    //
    // **Sound but incomplete, and deliberately so.** D78 §3 states the rule as
    // `R <: R′` and `⋀S ⊨ D` for every `D ∈ S′`. Entailment resolves class IRIs
    // against the layer chain, and conversion has **no layer** — `subtype_of` and
    // `eq_nf` take no context at all. Supplying one is D76's subject (D75 §8 Q1),
    // so the complete rule is blocked on Seam A.
    //
    // The rule used here is set inclusion, `S ⊇ S′`, which is sound because a
    // constraint present in `S` is trivially entailed by `⋀S`. It rejects the
    // case where `S` entails `D` without containing it — an incompleteness, so
    // some legal programs are refused, never an unsound admission. Strengthening
    // it to the full rule is a one-arm change once conversion carries `Γ_env`.
    //
    // The alternative — precomputing each constraint's field set into the value
    // so conversion needs no layer — is exactly the inline-the-environment
    // antipattern D75 §3.1 diagnoses as the root defect, and is not taken.
    if let (Val::Refine(r_sub, s_sub), Val::Refine(r_super, s_super)) = (sub, super_) {
        if !s_super.is_subset(s_sub) {
            let missing: Vec<&str> = s_super.difference(s_sub).map(|i| i.as_str()).collect();
            return Err(CheckError::TypeMismatch(format!(
                "refinement mismatch: the subtype does not declare {}. \
                 (Conversion cannot yet decide entailment — it has no layer; see D78 §3.)",
                missing.join(", ")
            )));
        }
        return subtype_of_inner(level, r_sub, r_super, index_policy);
    }

    if let (Val::Sort(m), Val::Sort(n)) = (sub, super_) {
        if m.leq(n) {
            return Ok(());
        } else {
            return Err(CheckError::TypeMismatch(format!(
                "universe mismatch: Sort({m}) is not a subtype of Sort({n})"
            )));
        }
    }
    if let (
        Val::InductiveType {
            decl: d1,
            params: p1,
            indices: i1,
        },
        Val::InductiveType {
            decl: d2,
            params: p2,
            indices: i2,
        },
    ) = (sub, super_)
    {
        if d1 == d2 && p1.len() == p2.len() && p1.len() == d1.params.len() && i1.len() == i2.len() {
            for (sub_p, sup_p) in p1.iter().zip(p2.iter()) {
                eq_nf(level, sub_p, sup_p)?;
            }
            // Indices are invariant (eigenius#137). Before this loop the
            // function returned right after the parameter telescope, so
            // `Vec A 0` and `Vec A 1` were interconvertible — and for a
            // family declared with zero parameters and only indices, which
            // is the shape of every `data P : core:string -> Prop` predicate
            // in the ontologies, ANY two applications were interconvertible.
            // No relaxation applies here: the sized-types rule above is a
            // parameter-telescope discipline, and an index is precisely what
            // distinguishes two types of one family.
            if index_policy == Indices::Compare {
                for (i, (sub_i, sup_i)) in i1.iter().zip(i2.iter()).enumerate() {
                    eq_nf(level, sub_i, sup_i).map_err(|err| {
                        CheckError::TypeMismatch(format!(
                            "`{}`: index #{i} mismatch: {err}",
                            d1.name
                        ))
                    })?;
                }
            }
            return Ok(());
        }
    }
    eq_nf(level, sub, super_)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::nbe::check::testutil::*;
    use crate::nbe::check::*;
    use crate::nbe::term::InductiveDecl;
    // ---------- D46 §4 — impredicative Pi formation tests ----------

    #[test]
    fn impredicative_pi_codomain_in_prop_lives_in_prop() {
        // ∀ (_ : 1). Prop : Prop
        // The codomain `Prop` is in `Sort(1)` (the universe-of-types), not
        // in `Sort(0)` itself, so this Pi lands in `Sort(1)`, not in Prop —
        // confirming the impredicative rule fires only on Prop-codomain.
        let pi = Exp::Pi(Patt::Unit, Box::new(Exp::One), Box::new(Exp::sort(0)));
        check(&mut ctx(), &pi, &Val::sort(1)).unwrap();
    }

    #[test]
    fn impredicative_pi_with_prop_codomain_in_prop() {
        // ∀ (_ : 1). 1 → 1 — not in Prop (codomain is `1 : Set`, not Prop)
        // ∀ (P : Prop). P → P : Prop — IS in Prop (codomain `P` is Prop)
        // We model the second: outer Pi binds `P : Prop`, inner Pi `_ : P. P`.
        // Inner Pi's codomain is `Var("P")` which has inferred type `Sort(0)`.
        let inner = Exp::Pi(
            Patt::Unit,
            Box::new(Exp::Var("P".to_string())),
            Box::new(Exp::Var("P".to_string())),
        );
        let outer = Exp::Pi(
            Patt::Var("P".to_string()),
            Box::new(Exp::sort(0)),
            Box::new(inner),
        );
        // The whole thing lives in Prop — that's the impredicative rule.
        check(&mut ctx(), &outer, &Val::sort(0)).unwrap();
    }

    #[test]
    fn impredicative_pi_quantifying_over_set_still_in_prop() {
        // ∀ (X : Set). (Π _ : X. 1 → 1) — outer Pi binds X at Set (Sort(1));
        // inner Pi's codomain is `1 → 1`, in Set (Sort(1)).
        // The outer Pi is NOT in Prop (codomain not in Prop).
        // But if we want `∀ (X : Set). False : Prop` then it IS in Prop.
        // We model the latter using Prop as the codomain (Sort(0) is a Prop
        // — every closed inhabitant of Sort(0) is propositional).
        // For a clean test, use ∀ (X : Set). Prop's-codomain — encoded as a Pi
        // whose body is a Pi `_ : X. X` (which won't typecheck against Prop —
        // X is in Set). So instead: ∀ (X : Set). (∀ _ : 1. 1 = 1). The inner
        // `1 = 1 : Prop` then makes the whole thing impredicative.
        //
        // Simpler test: ∀ (X : Set). False, where False = ∀ (P : Prop). P.
        // `∀ (P : Prop). P` lives in Prop (impredicative). Wrapping it in
        // ∀ X : Set. … keeps it in Prop (impredicative on the outer too).
        let false_prop = Exp::Pi(
            Patt::Var("P".to_string()),
            Box::new(Exp::sort(0)),
            Box::new(Exp::Var("P".to_string())),
        );
        // First check inner is itself in Prop.
        check(&mut ctx(), &false_prop, &Val::sort(0)).unwrap();
        // Then wrap with `∀ (X : Set). False` — also in Prop.
        let outer = Exp::Pi(
            Patt::Var("X".to_string()),
            Box::new(Exp::sort(1)),
            Box::new(false_prop),
        );
        check(&mut ctx(), &outer, &Val::sort(0)).unwrap();
    }

    #[test]
    fn predicative_sigma_in_prop_requires_both_components_in_prop() {
        // Σ (P : Prop) (Q : Prop). 1  — first component is in Prop, second is `1 : Set`.
        // Per D46 §3.4, Sigma in Prop requires BOTH components in Prop.
        // Mixed → should be rejected when checked against Sort(0).
        let mixed = Exp::Sig(
            Patt::Var("P".to_string()),
            Box::new(Exp::sort(0)),
            Box::new(Exp::One),
        );
        assert!(
            check(&mut ctx(), &mixed, &Val::sort(0)).is_err(),
            "Sigma with a non-Prop component should not check against Prop"
        );
    }

    #[test]
    fn predicative_sigma_both_in_prop_lives_in_prop() {
        // Σ (_ : ∀ P : Prop. P) (_ : ∀ Q : Prop. Q) — both components are
        // closed propositions (each is `False`-shaped, in Prop via the
        // impredicative rule). The Sigma of two Props lives in Prop.
        // The universe `Prop` itself lives in Sort(1), not in Prop, so we
        // cannot use it directly as a Sigma component.
        let false_p = Exp::Pi(
            Patt::Var("P".to_string()),
            Box::new(Exp::sort(0)),
            Box::new(Exp::Var("P".to_string())),
        );
        let false_q = Exp::Pi(
            Patt::Var("Q".to_string()),
            Box::new(Exp::sort(0)),
            Box::new(Exp::Var("Q".to_string())),
        );
        let sig = Exp::Sig(Patt::Unit, Box::new(false_p), Box::new(false_q));
        check(&mut ctx(), &sig, &Val::sort(0)).unwrap();
    }

    #[test]
    fn sort_cumulativity_prop_subtypes_set() {
        // Prop : Set — both as a check rule (Sort(0) inhabits Sort(1) by
        // the Sort(n) : Sort(n+1) rule) and as a subtype rule (Sort(0) <:
        // Sort(1) by D46 §3.2 cumulativity).
        check(&mut ctx(), &Exp::sort(0), &Val::sort(1)).unwrap();
        subtype_of(0, &Val::sort(0), &Val::sort(1)).unwrap();
    }

    #[test]
    fn sort_strict_cumulativity_set_not_subtype_of_prop() {
        // Sort(1) is NOT a subtype of Sort(0). Catches the wrong direction.
        assert!(subtype_of(0, &Val::sort(1), &Val::sort(0)).is_err());
    }

    // ---------- D46 §5 — proof irrelevance tests ----------

    #[test]
    fn proof_irrelevance_fires_for_id_type() {
        // Two structurally distinct values used as inhabitants of an Id type
        // should be accepted as equal via proof irrelevance — the structural
        // fast-path recognises Val::Id as a propositional type.
        let id_typ = Val::Id(Box::new(Val::One), Box::new(Val::Unit), Box::new(Val::Unit));
        let v1 = Val::sort(1);
        let v2 = Val::sort(2);
        def_eq_at_type(&mut ctx(), &v1, &v2, &id_typ).unwrap();
    }

    #[test]
    fn proof_irrelevance_does_not_fire_for_non_prop_type() {
        // Two distinct values at type `1` (Unit type) should NOT be accepted
        // as equal — `1` is not propositional (inhabits Sort(1)), so neither
        // the structural fast-path nor the inference path admits irrelevance.
        let one_typ = Val::One;
        let v1 = Val::sort(1);
        let v2 = Val::sort(2);
        assert!(
            def_eq_at_type(&mut ctx(), &v1, &v2, &one_typ).is_err(),
            "non-Prop type should fall through to structural equality"
        );
    }

    #[test]
    fn proof_irrelevance_fires_for_prop_typed_inductive() {
        // An inductive declared with sort = Sort(0) is propositional — caught
        // by the structural fast-path on Val::InductiveType.
        let prop_decl = std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:MyProp").unwrap(),
            name: "MyProp".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(0),
            ctors: Vec::new(),
        });
        let typ = Val::InductiveType {
            decl: prop_decl,
            params: Vec::new(),
            indices: Vec::new(),
        };
        def_eq_at_type(&mut ctx(), &Val::sort(1), &Val::sort(2), &typ).unwrap();
    }

    #[test]
    fn proof_irrelevance_does_not_fire_for_set_typed_inductive() {
        // An inductive declared with sort = Sort(1) is NOT propositional.
        let set_decl = std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:MyData").unwrap(),
            name: "MyData".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        let typ = Val::InductiveType {
            decl: set_decl,
            params: Vec::new(),
            indices: Vec::new(),
        };
        assert!(def_eq_at_type(&mut ctx(), &Val::sort(1), &Val::sort(2), &typ).is_err());
    }

    #[test]
    fn proof_irrelevance_via_inference_for_pi_into_prop() {
        // Test that the inference path catches a Prop-shaped type that the
        // structural fast-path misses.
        // typ = `∀ (P : Prop). P` — a Pi-into-Prop, propositional by the
        // impredicative rule (D46 §4.1). Structural fast-path doesn't match
        // Val::Pi, so the inference path must fire: readback to
        // `Exp::Pi(P, Sort(0), Var(P))`, infer sort, get Sort(0).
        let false_prop_exp = Exp::Pi(
            Patt::Var("P".to_string()),
            Box::new(Exp::sort(0)),
            Box::new(Exp::Var("P".to_string())),
        );
        let typ = ctx().eval(&false_prop_exp, &Rho::Nil).expect("eval Pi");
        // Sanity: this is a Val::Pi, not a fast-path shape.
        assert!(matches!(typ, Val::Pi(_, _)));
        // Inference path must classify it as propositional.
        def_eq_at_type(&mut ctx(), &Val::sort(1), &Val::sort(2), &typ).unwrap();
    }

    #[test]
    fn proof_irrelevance_via_inference_negative_for_pi_into_set() {
        // Counter-test: `∀ (X : Set). X` lives in Set, not Prop.
        // The inference path must REJECT proof irrelevance here.
        let pi_exp = Exp::Pi(
            Patt::Var("X".to_string()),
            Box::new(Exp::sort(1)),
            Box::new(Exp::Var("X".to_string())),
        );
        let typ = ctx().eval(&pi_exp, &Rho::Nil).expect("eval Pi");
        assert!(matches!(typ, Val::Pi(_, _)));
        assert!(def_eq_at_type(&mut ctx(), &Val::sort(1), &Val::sort(2), &typ).is_err());
    }

    // --- Size-aware subtyping (Phase 11b step 15d, D19 §8.3) ---

    #[test]
    fn subtype_parameters_require_equality() {
        // Sized stream parameters disagree on the element type —
        // size_le only relaxes size positions, so the other position
        // must still be equal.
        let decl = two_param_decl();
        let sub = mk_two_param(decl.clone(), Val::One, Val::One);
        let sup = mk_two_param(decl, Val::One, Val::sort(1));
        assert!(
            subtype_of(0, &sub, &sup).is_err(),
            "element type mismatch must be rejected"
        );
    }

    #[test]
    fn subtype_non_inductive_falls_back_to_eq_nf() {
        // Simple non-inductive types fall through to `eq_nf` —
        // equal types accept, mismatched types reject.
        subtype_of(0, &Val::One, &Val::One).expect("1 <: 1");
        assert!(subtype_of(0, &Val::One, &Val::sort(1)).is_err());
    }

    #[test]
    fn subtype_distinct_inductive_decls_fall_back_to_eq_nf() {
        // Two inductive types with different names: the sized-subtyping
        // branch is skipped (decls differ), and `eq_nf` correctly
        // rejects them.
        let decl_a = two_param_decl();
        let decl_b = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:OtherStream").unwrap(),
            name: "OtherStream".to_string(),
            params: decl_a.params.clone(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: vec![],
        });
        let sub = mk_two_param(decl_a, Val::One, Val::One);
        let sup = mk_two_param(decl_b, Val::One, Val::One);
        assert!(subtype_of(0, &sub, &sup).is_err());
    }
}

/// eigenius#137 — indices are part of a type's identity.
#[cfg(test)]
mod index_conversion_tests {
    use super::*;

    use crate::nbe::check::testutil::*;
    use crate::nbe::check::{check, CheckCtx};
    use crate::nbe::env::{gen_val, up_gamma, Rho};
    use crate::nbe::term::{InductiveCtorDecl, InductiveDecl, Patt, PrimitiveType};
    use std::sync::Arc;

    /// `data Vec (A : Set) : Nat -> Set` — one parameter, one index.
    fn vec_decl() -> Arc<InductiveDecl> {
        let nat = nat_decl();
        Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Vec").unwrap(),
            name: "Vec".to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::InductiveType(nat, Vec::new()))],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        })
    }

    /// `Vec 1 n` — the parameter is instantiated at `One`, which is the
    /// `Set` the tests do not vary.
    fn vec_at(decl: &Arc<InductiveDecl>, n: Val) -> Val {
        Val::InductiveType {
            decl: decl.clone(),
            params: vec![Val::One],
            indices: vec![n],
        }
    }

    /// Nat `zero` and `succ zero`, evaluated.
    fn zero_and_one() -> (Val, Val) {
        let nat = nat_decl();
        let c = CheckCtx::new(Rho::Nil, vec![]);
        let zero_exp = nat_zero_exp(&nat);
        let one_exp = Exp::InductiveCtor(nat, "succ".to_string(), vec![zero_exp.clone()]);
        let zero = c.eval(&zero_exp, &Rho::Nil).unwrap();
        let one = c.eval(&one_exp, &Rho::Nil).unwrap();
        (zero, one)
    }

    /// `Vec A 0` and `Vec A 1` are different types.
    ///
    /// The inductive case of [`subtype_of_with_hyps`] returned `Ok(())` right
    /// after the parameter telescope, never reaching the `eq_nf` fallback that
    /// compares indices, so this pair was definitionally equal on every path
    /// that goes through conversion — every expression form without a
    /// dedicated `check` arm.
    #[test]
    fn vec_at_distinct_indices_is_not_convertible() {
        let decl = vec_decl();
        let (zero, one) = zero_and_one();
        let err = subtype_of(0, &vec_at(&decl, zero), &vec_at(&decl, one))
            .expect_err("`Vec A 0 <: Vec A 1` must be rejected");
        assert!(
            format!("{err:?}").contains("index #0 mismatch"),
            "expected an index-mismatch diagnostic, got: {err:?}"
        );
    }

    /// Conversion at equal indices is unaffected.
    #[test]
    fn vec_at_equal_indices_is_convertible() {
        let decl = vec_decl();
        let (zero, _) = zero_and_one();
        subtype_of(0, &vec_at(&decl, zero.clone()), &vec_at(&decl, zero))
            .expect("`Vec A 0 <: Vec A 0` must hold");
    }

    /// `data P : core:string -> Prop` — zero parameters, one index. The shape
    /// of every domain predicate in the ontologies (`onco:MSI`,
    /// `screen:HasLowIC50`, `bench:concerns`, …).
    fn string_predicate_decl() -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:HasLowIC50").unwrap(),
            name: "HasLowIC50".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::EigonPrimitive(PrimitiveType::String))],
            sort: Exp::sort(0),
            ctors: Vec::new(),
        })
    }

    /// The old guard read `p1.len() == p2.len() && p1.len() == d1.params.len()`
    /// over the *parameter* vectors, which holds trivially at zero parameters;
    /// the parameter loop then ran zero times and any two applications of the
    /// family were interconvertible.
    #[test]
    fn zero_parameter_family_at_distinct_indices_is_not_convertible() {
        let decl = string_predicate_decl();
        let at = |s: &str| Val::InductiveType {
            decl: decl.clone(),
            params: Vec::new(),
            indices: vec![Val::LitString(s.to_string())],
        };
        let err = subtype_of(0, &at("compound-A"), &at("compound-B"))
            .expect_err("HasLowIC50(\"compound-A\") <: HasLowIC50(\"compound-B\") is rejected");
        assert!(
            format!("{err:?}").contains("index #0 mismatch"),
            "expected an index-mismatch diagnostic, got: {err:?}"
        );
        subtype_of(0, &at("compound-A"), &at("compound-A"))
            .expect("the same application converts with itself");
    }

    /// A two-index family rejects a mismatch in the *second* index — the shape
    /// of `reasoning:JustifiedBy : JustificationTerm -> Prop -> Type 0`, whose
    /// index #1 is the proposition the certificate is about.
    #[test]
    fn a_mismatch_in_a_later_index_is_rejected() {
        let decl = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Concerns").unwrap(),
            name: "Concerns".to_string(),
            params: Vec::new(),
            indices: vec![
                (Patt::Unit, Exp::EigonPrimitive(PrimitiveType::String)),
                (Patt::Unit, Exp::EigonPrimitive(PrimitiveType::String)),
            ],
            sort: Exp::sort(0),
            ctors: Vec::new(),
        });
        let at = |a: &str, b: &str| Val::InductiveType {
            decl: decl.clone(),
            params: Vec::new(),
            indices: vec![Val::LitString(a.to_string()), Val::LitString(b.to_string())],
        };
        let err = subtype_of(0, &at("WRN", "MSI"), &at("WRN", "MSS"))
            .expect_err("a second-index mismatch must be rejected");
        assert!(
            format!("{err:?}").contains("index #1 mismatch"),
            "expected an index-#1 diagnostic, got: {err:?}"
        );
    }

    /// Conversion is what `check` falls back to for every expression form
    /// without a dedicated arm, so the rejection has to be visible from
    /// `check`, not only from the `subtype_of` API.
    #[test]
    fn a_variable_at_one_index_does_not_check_against_another() {
        let decl = vec_decl();
        let (zero, one) = zero_and_one();
        let x_val = gen_val(&Rho::Nil);
        let rho = Rho::Nil.extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma = up_gamma(
            &Vec::new(),
            &Patt::Var("x".to_string()),
            &vec_at(&decl, zero),
            &x_val,
        )
        .unwrap();
        let mut c = CheckCtx::new(rho, gamma);
        let err = check(&mut c, &Exp::Var("x".to_string()), &vec_at(&decl, one))
            .expect_err("`x : Vec A 0` must not check against `Vec A 1`");
        assert!(
            format!("{err:?}").contains("index #0 mismatch"),
            "expected an index-mismatch diagnostic, got: {err:?}"
        );
    }

    /// The constructor path keeps its own D48 Phase D index unification —
    /// [`subtype_of_deferring_indices`] leaves indices to it — so a
    /// constructor whose conclusion index differs from the expected one is
    /// still rejected, with the unification diagnostic.
    #[test]
    fn a_constructor_at_the_wrong_index_is_still_rejected() {
        // `data Box : 1 -> Set { mk : Box () }`: the one ctor concludes at
        // index `()`, so checking it against `Box x` for a rigid `x` fails.
        let self_ref = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Box").unwrap(),
            name: "Box".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: Vec::new(),
        });
        let box_unit = Exp::InductiveType(self_ref, vec![Exp::Unit]);
        let decl = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Box").unwrap(),
            name: "Box".to_string(),
            params: Vec::new(),
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
            ctors: vec![InductiveCtorDecl {
                name: "mk".to_string(),
                typ: box_unit,
            }],
        });
        let mut c = CheckCtx::new(Rho::Nil, vec![]);
        let expected = Val::InductiveType {
            decl: decl.clone(),
            params: Vec::new(),
            indices: vec![gen_val(&Rho::Nil)],
        };
        let ctor = Exp::InductiveCtor(decl, "mk".to_string(), Vec::new());
        let err = check(&mut c, &ctor, &expected)
            .expect_err("`Box.mk : Box ()` must not check against `Box x`");
        assert!(
            format!("{err:?}").contains("index #0 mismatch"),
            "expected the D48 Phase D unification diagnostic, got: {err:?}"
        );
    }

    // ---------- eigenius#209 — parameters that RANGE OVER propositions ----------

    /// `logic:And (P : Prop, Q : Prop) : Prop` — the real declaration
    /// (`ontologies/logic/logic.esl:38`).
    fn and_decl() -> std::sync::Arc<InductiveDecl> {
        std::sync::Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:eigenius:logic:And").unwrap(),
            name: "And".to_string(),
            params: vec![
                (Patt::Var("P".into()), Exp::sort(0)),
                (Patt::Var("Q".into()), Exp::sort(0)),
            ],
            indices: Vec::new(),
            sort: Exp::sort(0),
            ctors: Vec::new(),
        })
    }

    fn and_of(a: &str, b: &str) -> Val {
        let prop = |n: &str| Val::EigonClass(crate::ontology::iri::Iri::parse(n).unwrap());
        Val::InductiveType {
            decl: and_decl(),
            params: vec![prop(a), prop(b)],
            indices: Vec::new(),
        }
    }

    #[test]
    fn distinct_conjunctions_are_not_convertible() {
        // The parameter loop used to skip any parameter whose DECLARED TYPE was `Sort(0)`, calling
        // it proof irrelevance. It is not: proof irrelevance collapses two INHABITANTS OF a
        // proposition, and a parameter declared `P : Prop` ranges over PROPOSITIONS. The arm
        // asserted that all propositions are equal, which made `And(A, B)` and `And(C, D)`
        // convertible for any operands at all.
        //
        // #137's shape, one telescope over: there the INDEX loop returned early, here the PARAMETER
        // loop relaxed. That fix left this one standing because it never touched this loop.
        //
        // The correct rule already lives at `def_eq_at_type`, keyed on `is_propositional_in_ctx` —
        // does the TYPE inhabit `Sort(0)` — which is what nanoda's kernel does
        // (`is_proof(e) = is_prop(infer(e))`). Nothing is lost by deleting the duplicate: a
        // parameter that genuinely is a proof reaches irrelevance through that rule instead.
        assert!(
            subtype_of(
                0,
                &and_of("urn:test:A", "urn:test:B"),
                &and_of("urn:test:C", "urn:test:D")
            )
            .is_err(),
            "conjunctions sharing no operand must not be convertible"
        );
    }

    #[test]
    fn the_same_conjunction_is_still_convertible_with_itself() {
        // The guard against over-correcting: removing the arm must not make a family
        // non-reflexive.
        subtype_of(
            0,
            &and_of("urn:test:A", "urn:test:B"),
            &and_of("urn:test:A", "urn:test:B"),
        )
        .expect("a conjunction is convertible with itself");
    }
}
