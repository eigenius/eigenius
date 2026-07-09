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
    // are definitionally equal. (D46 proof irrelevance further
    // collapses *any* two witnesses of the same Prop-typed predicate
    // type to equal at that type via `def_eq_at_type`; this branch is
    // the conservative fast path used when the proof-irrelevance
    // shortcut wasn't reachable — e.g., direct `eq_nf` calls without
    // a type in hand.)
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
        Exp::CodataType(_, args) => args.iter().any(|a| exp_mentions_var(a, name)),
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
        Exp::InductiveType(decl, _) => matches!(decl.sort, Exp::Sort(0)),
        Exp::CodataType(decl, _) => matches!(decl.sort, Exp::Sort(0)),
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
    if impredicative && n == 0 {
        Ok(Val::Sort(0))
    } else {
        Ok(Val::Sort(m.max(n)))
    }
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
    Ok(matches!(typ_sort, Val::Sort(0)))
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
        Val::InductiveType { decl, .. } => Some(matches!(decl.sort, Exp::Sort(0))),
        Val::CodataType { decl, .. } => Some(matches!(decl.sort, Exp::Sort(0))),
        Val::One
        | Val::Sort(_)
        | Val::EigonClass(_)
        | Val::EigonPrimitive(_)
        | Val::SizeSort
        | Val::Codata(_, _) => Some(false),
        _ => None,
    }
}

/// Subtyping check: admits `sub <: super` (Phase 11b step 15d, D19 §8.3).
///
/// Calls [`subtype_of_with_hyps`] with an empty TSO — use this variant
/// when you don't have bounded size hypotheses to bring to bear.
pub fn subtype_of(level: usize, sub: &Val, super_: &Val) -> Result<(), CheckError> {
    subtype_of_with_hyps(level, sub, super_, &crate::nbe::sized_rigid::Tso::new())
}

/// Subtyping check consulting a TSO of rigid size hypotheses.
///
/// Current scope is exactly the sized-types relaxation — everywhere
/// else subtyping degenerates to equality (`eq_nf`). The relaxation:
///
/// For a pair of applied inductive types `I(p₁ … pₙ)` with identical
/// declarations, each parameter is compared position-wise:
/// - positions whose declared type is `SizeSort` are compared with
///   [`crate::nbe::sized::size_le_with_hyps`] — `sub_pᵢ ≤ sup_pᵢ`
///   suffices, with the TSO consulted for neutral entailment;
/// - all other positions must be definitionally equal (`eq_nf`).
///
/// This is what makes `T(s) <: T(ŝ s) <: T(∞)` admissible — the
/// driving motivation for sized types. With `tso` populated from
/// bounded binders in scope, `T(i) <: T(j)` also becomes admissible
/// whenever `i ≤ j` is entailed by the hypothesis chain.
///
/// Codata (`Val::Codata`) is structurally identical and will benefit
/// once sized codata arrives; it falls through to `eq_nf` today
/// because the checker doesn't yet thread size parameters onto
/// `Codata` value shapes.
pub fn subtype_of_with_hyps(
    level: usize,
    sub: &Val,
    super_: &Val,
    tso: &crate::nbe::sized_rigid::Tso,
) -> Result<(), CheckError> {
    // Universe cumulativity: Sort(m) <: Sort(n) iff m <= n.
    // D46 §3.2 — Prop ⊆ Set ⊆ Type(1) ⊆ Type(2) ⊆ …
    if let (Val::Sort(m), Val::Sort(n)) = (sub, super_) {
        if m <= n {
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
            indices: _,
        },
        Val::InductiveType {
            decl: d2,
            params: p2,
            indices: _,
        },
    ) = (sub, super_)
    {
        if d1 == d2 && p1.len() == p2.len() && p1.len() == d1.params.len() {
            for (i, (sub_p, sup_p)) in p1.iter().zip(p2.iter()).enumerate() {
                let decl_param_ty = &d1.params[i].1;
                if matches!(decl_param_ty, Exp::SizeSort) {
                    if !crate::nbe::sized::size_le_with_hyps(sub_p, sup_p, tso) {
                        return Err(CheckError::TypeMismatch(format!(
                            "size subtyping failed at param {i}: \
                             {:?} ≰ {:?}",
                            readback_val(level, sub_p),
                            readback_val(level, sup_p),
                        )));
                    }
                } else if matches!(decl_param_ty, Exp::Sort(0)) {
                    // Proof irrelevance (D46 §5): if the parameter's declared
                    // type is Prop, any two parameter values are equal as
                    // inhabitants of a propositional sort.
                    continue;
                } else {
                    eq_nf(level, sub_p, sup_p)?;
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
        let pi = Exp::Pi(Patt::Unit, Box::new(Exp::One), Box::new(Exp::Sort(0)));
        check(&mut ctx(), &pi, &Val::Sort(1)).unwrap();
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
            Box::new(Exp::Sort(0)),
            Box::new(inner),
        );
        // The whole thing lives in Prop — that's the impredicative rule.
        check(&mut ctx(), &outer, &Val::Sort(0)).unwrap();
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
            Box::new(Exp::Sort(0)),
            Box::new(Exp::Var("P".to_string())),
        );
        // First check inner is itself in Prop.
        check(&mut ctx(), &false_prop, &Val::Sort(0)).unwrap();
        // Then wrap with `∀ (X : Set). False` — also in Prop.
        let outer = Exp::Pi(
            Patt::Var("X".to_string()),
            Box::new(Exp::Sort(1)),
            Box::new(false_prop),
        );
        check(&mut ctx(), &outer, &Val::Sort(0)).unwrap();
    }

    #[test]
    fn predicative_sigma_in_prop_requires_both_components_in_prop() {
        // Σ (P : Prop) (Q : Prop). 1  — first component is in Prop, second is `1 : Set`.
        // Per D46 §3.4, Sigma in Prop requires BOTH components in Prop.
        // Mixed → should be rejected when checked against Sort(0).
        let mixed = Exp::Sig(
            Patt::Var("P".to_string()),
            Box::new(Exp::Sort(0)),
            Box::new(Exp::One),
        );
        assert!(
            check(&mut ctx(), &mixed, &Val::Sort(0)).is_err(),
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
            Box::new(Exp::Sort(0)),
            Box::new(Exp::Var("P".to_string())),
        );
        let false_q = Exp::Pi(
            Patt::Var("Q".to_string()),
            Box::new(Exp::Sort(0)),
            Box::new(Exp::Var("Q".to_string())),
        );
        let sig = Exp::Sig(Patt::Unit, Box::new(false_p), Box::new(false_q));
        check(&mut ctx(), &sig, &Val::Sort(0)).unwrap();
    }

    #[test]
    fn sort_cumulativity_prop_subtypes_set() {
        // Prop : Set — both as a check rule (Sort(0) inhabits Sort(1) by
        // the Sort(n) : Sort(n+1) rule) and as a subtype rule (Sort(0) <:
        // Sort(1) by D46 §3.2 cumulativity).
        check(&mut ctx(), &Exp::Sort(0), &Val::Sort(1)).unwrap();
        subtype_of(0, &Val::Sort(0), &Val::Sort(1)).unwrap();
    }

    #[test]
    fn sort_strict_cumulativity_set_not_subtype_of_prop() {
        // Sort(1) is NOT a subtype of Sort(0). Catches the wrong direction.
        assert!(subtype_of(0, &Val::Sort(1), &Val::Sort(0)).is_err());
    }

    // ---------- D46 §5 — proof irrelevance tests ----------

    #[test]
    fn proof_irrelevance_fires_for_id_type() {
        // Two structurally distinct values used as inhabitants of an Id type
        // should be accepted as equal via proof irrelevance — the structural
        // fast-path recognises Val::Id as a propositional type.
        let id_typ = Val::Id(Box::new(Val::One), Box::new(Val::Unit), Box::new(Val::Unit));
        let v1 = Val::Sort(1);
        let v2 = Val::Sort(2);
        def_eq_at_type(&mut ctx(), &v1, &v2, &id_typ).unwrap();
    }

    #[test]
    fn proof_irrelevance_does_not_fire_for_non_prop_type() {
        // Two distinct values at type `1` (Unit type) should NOT be accepted
        // as equal — `1` is not propositional (inhabits Sort(1)), so neither
        // the structural fast-path nor the inference path admits irrelevance.
        let one_typ = Val::One;
        let v1 = Val::Sort(1);
        let v2 = Val::Sort(2);
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
            sort: Exp::Sort(0),
            ctors: Vec::new(),
        });
        let typ = Val::InductiveType {
            decl: prop_decl,
            params: Vec::new(),
            indices: Vec::new(),
        };
        def_eq_at_type(&mut ctx(), &Val::Sort(1), &Val::Sort(2), &typ).unwrap();
    }

    #[test]
    fn proof_irrelevance_does_not_fire_for_set_typed_inductive() {
        // An inductive declared with sort = Sort(1) is NOT propositional.
        let set_decl = std::sync::Arc::new(crate::nbe::term::InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:MyData").unwrap(),
            name: "MyData".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        });
        let typ = Val::InductiveType {
            decl: set_decl,
            params: Vec::new(),
            indices: Vec::new(),
        };
        assert!(def_eq_at_type(&mut ctx(), &Val::Sort(1), &Val::Sort(2), &typ).is_err());
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
            Box::new(Exp::Sort(0)),
            Box::new(Exp::Var("P".to_string())),
        );
        let typ = ctx().eval(&false_prop_exp, &Rho::Nil).expect("eval Pi");
        // Sanity: this is a Val::Pi, not a fast-path shape.
        assert!(matches!(typ, Val::Pi(_, _)));
        // Inference path must classify it as propositional.
        def_eq_at_type(&mut ctx(), &Val::Sort(1), &Val::Sort(2), &typ).unwrap();
    }

    #[test]
    fn proof_irrelevance_via_inference_negative_for_pi_into_set() {
        // Counter-test: `∀ (X : Set). X` lives in Set, not Prop.
        // The inference path must REJECT proof irrelevance here.
        let pi_exp = Exp::Pi(
            Patt::Var("X".to_string()),
            Box::new(Exp::Sort(1)),
            Box::new(Exp::Var("X".to_string())),
        );
        let typ = ctx().eval(&pi_exp, &Rho::Nil).expect("eval Pi");
        assert!(matches!(typ, Val::Pi(_, _)));
        assert!(def_eq_at_type(&mut ctx(), &Val::Sort(1), &Val::Sort(2), &typ).is_err());
    }

    // --- Size-aware subtyping (Phase 11b step 15d, D19 §8.3) ---

    #[test]
    fn subtype_sized_finite_to_inf_admitted() {
        // SizedStream(ŝ ∞, A) is blocked by ∞-absorption (∞ stays ∞).
        // Use a neutral size to get a real "finite-side-of-∞" value.
        let decl = sized_stream_decl();
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "i".into()));
        let sub = mk_sized_type(decl.clone(), neut.clone(), Val::One);
        let sup = mk_sized_type(decl, Val::SizeInf, Val::One);
        subtype_of(0, &sub, &sup).expect("T(i) <: T(∞)");
    }

    #[test]
    fn subtype_sized_inf_to_finite_rejected() {
        let decl = sized_stream_decl();
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "i".into()));
        let sub = mk_sized_type(decl.clone(), Val::SizeInf, Val::One);
        let sup = mk_sized_type(decl, neut, Val::One);
        assert!(
            subtype_of(0, &sub, &sup).is_err(),
            "T(∞) <: T(i) must be rejected"
        );
    }

    #[test]
    fn subtype_sized_step_rule_admitted() {
        // T(i) <: T(ŝ i) admitted by the right-step rule on sizes.
        let decl = sized_stream_decl();
        let neut = Val::Nt(crate::nbe::val::Neut::Gen(0, "i".into()));
        let sub = mk_sized_type(decl.clone(), neut.clone(), Val::One);
        let sup = mk_sized_type(decl, Val::SizeSucc(Box::new(neut)), Val::One);
        subtype_of(0, &sub, &sup).expect("T(i) <: T(ŝ i)");
    }

    #[test]
    fn subtype_sized_same_inf_reflexive() {
        let decl = sized_stream_decl();
        let sub = mk_sized_type(decl.clone(), Val::SizeInf, Val::One);
        let sup = mk_sized_type(decl, Val::SizeInf, Val::One);
        subtype_of(0, &sub, &sup).expect("T(∞) <: T(∞) reflexive");
    }

    #[test]
    fn subtype_non_size_parameter_still_requires_equality() {
        // Sized stream parameters disagree on the element type —
        // size_le only relaxes size positions, so the other position
        // must still be equal.
        let decl = sized_stream_decl();
        let sub = mk_sized_type(decl.clone(), Val::SizeInf, Val::One);
        let sup = mk_sized_type(decl, Val::SizeInf, Val::Sort(1));
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
        assert!(subtype_of(0, &Val::One, &Val::Sort(1)).is_err());
    }

    #[test]
    fn subtype_distinct_inductive_decls_fall_back_to_eq_nf() {
        // Two inductive types with different names: the sized-subtyping
        // branch is skipped (decls differ), and `eq_nf` correctly
        // rejects them.
        let decl_a = sized_stream_decl();
        let decl_b = Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:OtherStream").unwrap(),
            name: "OtherStream".to_string(),
            params: decl_a.params.clone(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![],
        });
        let sub = mk_sized_type(decl_a, Val::SizeInf, Val::One);
        let sup = mk_sized_type(decl_b, Val::SizeInf, Val::One);
        assert!(subtype_of(0, &sub, &sup).is_err());
    }

    #[test]
    fn check_var_with_finite_size_against_inf_expected_succeeds() {
        // End-to-end: a variable `x : SizedStream(i, One)` checks
        // against the expected type `SizedStream(∞, One)`.
        //
        // This exercises the `check()` fallthrough at line ~388 —
        // it infers `x`'s type from gamma, then calls subtype_of
        // against the expected type. Without sized subtyping this
        // would fail (neutral `i` ≠ SizeInf syntactically).
        let decl = sized_stream_decl();

        // Bind `i : SizeSort`, then `x : SizedStream(i, One)`.
        let i_val = gen_val(&Rho::Nil); // Val::Nt(Gen(0, _))
        let rho1 = Rho::Nil.extend(Patt::Var("i".to_string()), i_val.clone());
        let gamma1 = up_gamma(
            &Vec::new(),
            &Patt::Var("i".to_string()),
            &Val::SizeSort,
            &i_val,
        )
        .unwrap();

        let sub_stream = mk_sized_type(decl.clone(), i_val, Val::One);
        let x_val = gen_val(&rho1); // Val::Nt(Gen(1, _))
        let rho2 = rho1.extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&gamma1, &Patt::Var("x".to_string()), &sub_stream, &x_val).unwrap();

        let mut c = CheckCtx::new(rho2, gamma2);
        let expected = mk_sized_type(decl, Val::SizeInf, Val::One);
        check(&mut c, &Exp::Var("x".to_string()), &expected)
            .expect("x : SizedStream(i, 1) should check against SizedStream(∞, 1)");
    }
}
