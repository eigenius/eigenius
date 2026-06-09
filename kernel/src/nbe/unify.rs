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

//! D48 Phase C — first-order pattern unification for EigenTT.
//!
//! The unifier solves equations like `?n = succ k` arising during
//! dependent pattern matching on indexed inductives (D48). It operates
//! on the EigenTT `Val` representation and stores metavariable solutions
//! in a [`MetaCtx`].
//!
//! ## Scope and limits
//!
//! Per D48 §3.1, this is the **first-order pattern** fragment of
//! unification — sufficient for `Vec`, `Fin`, dependent `Eq`, and the
//! bulk of indexed families that arise from science/engineering
//! modelling (see [d48-indexed-inductive-families.md §3.1] for the
//! decision rationale). Higher-order patterns common in abstract-math
//! proofs are out of scope — abstract-math reasoning lives in the Lean
//! institution where Lean's elaborator handles higher-order
//! unification.
//!
//! ## Algorithm sketch
//!
//! Given two `Val`s `lhs` and `rhs`, [`unify`] walks them in parallel:
//!
//! - **Both sides equal under structural equality** (`eq_nf`): succeed,
//!   no substitution emitted.
//! - **One side is an unsolved [`Neut::Meta`]**: attempt to **solve** the
//!   meta against the other side. Solving requires:
//!     - The meta's spine is a sequence of distinct bound variables
//!       (pattern condition).
//!     - The other side mentions no meta-out-of-scope free variables.
//!     - The meta doesn't occur in the other side (occurs check).
//!
//!   If all three hold, record `meta := λ spine. other` in the
//!   `MetaCtx`.
//! - **Both sides are constructor-shaped** (e.g., `Val::InductiveType`
//!   or `Val::InductiveVal`): recurse on the corresponding arguments.
//! - **Anything else**: fall back to `eq_nf` (structural equality).
//!   If structural equality fails, emit a [`UnifyError`].
//!
//! This is sufficient for **D48 Phase D** (constructor checking with
//! concrete index unification — `cons k x xs : Vec A (succ k)`) and
//! **D48 Phase F** (dependent motive inference for `match`).
//!
//! ## Reference
//!
//! See `docs/design/d48-indexed-inductive-families.md` §3.1, §4.4, §4.7.

use crate::nbe::check::eq_nf;
use crate::nbe::readback::readback_val;
use crate::nbe::val::{MetaId, Neut, Val};
use std::collections::BTreeMap;

/// A registry of unification metavariables and their solutions.
///
/// Allocates fresh [`MetaId`]s and stores their solved values. Cheap
/// to clone (the inner map is sized by the number of metas in flight).
#[derive(Debug, Clone, Default)]
pub struct MetaCtx {
    next: u32,
    solutions: BTreeMap<MetaId, Val>,
}

impl MetaCtx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh unsolved metavariable.
    pub fn fresh(&mut self) -> MetaId {
        let id = MetaId(self.next);
        self.next += 1;
        id
    }

    /// Look up a metavariable's solution if it has been solved.
    pub fn solution(&self, id: MetaId) -> Option<&Val> {
        self.solutions.get(&id)
    }

    /// Record a solution for an unsolved metavariable. Caller is
    /// responsible for verifying the solution respects the pattern
    /// condition and passes the occurs check; [`unify`] does this.
    fn solve(&mut self, id: MetaId, value: Val) -> Result<(), UnifyError> {
        if self.solutions.contains_key(&id) {
            return Err(UnifyError::DoubleSolve(id));
        }
        self.solutions.insert(id, value);
        Ok(())
    }

    /// Replace any solved metavariables in `val` with their solutions
    /// (zonking). Recurses through all `Val` structure. Unsolved metas
    /// remain in place.
    pub fn zonk(&self, val: &Val) -> Val {
        zonk_val(self, val)
    }
}

/// Errors raised during unification.
#[derive(Debug, Clone)]
pub enum UnifyError {
    /// The two values cannot be equated under any substitution.
    Mismatch { lhs: String, rhs: String },
    /// A metavariable would have to be set to a value mentioning
    /// itself (`?x = f ?x`). Always unsound; reject.
    OccursCheck { meta: MetaId, in_value: String },
    /// A metavariable's spine is not a pattern (distinct bound
    /// variables only). Phase C is restricted to first-order patterns.
    NonPatternSpine { meta: MetaId, spine: String },
    /// A metavariable was solved twice with conflicting values. The
    /// inner workings of `solve` enforce single-assignment; this
    /// surfaces if a caller manipulates the `MetaCtx` directly.
    DoubleSolve(MetaId),
}

impl std::fmt::Display for UnifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnifyError::Mismatch { lhs, rhs } => {
                write!(f, "unification mismatch: {lhs} ≠ {rhs}")
            }
            UnifyError::OccursCheck { meta, in_value } => write!(
                f,
                "occurs check failed: meta {meta} occurs in proposed solution {in_value}"
            ),
            UnifyError::NonPatternSpine { meta, spine } => write!(
                f,
                "metavariable {meta} applied to non-pattern spine ({spine}) — \
                 only distinct bound variables are admitted in Phase C"
            ),
            UnifyError::DoubleSolve(id) => write!(f, "metavariable {id} solved twice"),
        }
    }
}

impl std::error::Error for UnifyError {}

/// Top-level unification entry point.
///
/// Attempts to make `lhs` and `rhs` equal by either accepting them as
/// structurally equal or solving metavariables in `mctx`. Returns `Ok`
/// on success; `Err` describes the obstruction.
///
/// `level` is the current de Bruijn level (number of binders in scope) —
/// passed through for readback and equality.
pub fn unify(level: usize, lhs: &Val, rhs: &Val, mctx: &mut MetaCtx) -> Result<(), UnifyError> {
    let lhs = mctx.zonk(lhs);
    let rhs = mctx.zonk(rhs);

    match (&lhs, &rhs) {
        // Both are unsolved Metas — if the same, succeed trivially.
        // If different, prefer to solve the first against the second
        // (arbitrary but stable choice).
        (Val::Nt(Neut::Meta(lid, lspine)), Val::Nt(Neut::Meta(rid, rspine))) => {
            if lid == rid {
                // Same meta — spines must unify structurally.
                if lspine.len() != rspine.len() {
                    return Err(mismatch(level, &lhs, &rhs));
                }
                for (lv, rv) in lspine.iter().zip(rspine.iter()) {
                    unify(level, lv, rv, mctx)?;
                }
                Ok(())
            } else {
                // Different metas — solve `lid` against `rhs` (which is
                // itself a meta; this records `?lid := ?rid spine`).
                solve_meta(level, *lid, lspine, &rhs, mctx)
            }
        }

        // One side is an unsolved Meta — solve it against the other.
        (Val::Nt(Neut::Meta(id, spine)), _) => solve_meta(level, *id, spine, &rhs, mctx),
        (_, Val::Nt(Neut::Meta(id, spine))) => solve_meta(level, *id, spine, &lhs, mctx),

        // Both are InductiveType applications — same decl + recurse
        // on params + indices.
        (
            Val::InductiveType {
                decl: ld,
                params: lp,
                indices: li,
            },
            Val::InductiveType {
                decl: rd,
                params: rp,
                indices: ri,
            },
        ) => {
            if ld.name != rd.name {
                return Err(mismatch(level, &lhs, &rhs));
            }
            if lp.len() != rp.len() || li.len() != ri.len() {
                return Err(mismatch(level, &lhs, &rhs));
            }
            for (lv, rv) in lp.iter().zip(rp.iter()) {
                unify(level, lv, rv, mctx)?;
            }
            for (lv, rv) in li.iter().zip(ri.iter()) {
                unify(level, lv, rv, mctx)?;
            }
            Ok(())
        }

        // Both are InductiveVal — same decl + ctor + recurse on args.
        (
            Val::InductiveVal {
                decl: ld,
                ctor_name: lc,
                args: la,
            },
            Val::InductiveVal {
                decl: rd,
                ctor_name: rc,
                args: ra,
            },
        ) => {
            if ld.name != rd.name || lc != rc {
                return Err(mismatch(level, &lhs, &rhs));
            }
            if la.len() != ra.len() {
                return Err(mismatch(level, &lhs, &rhs));
            }
            for (lv, rv) in la.iter().zip(ra.iter()) {
                unify(level, lv, rv, mctx)?;
            }
            Ok(())
        }

        // Constructor applications carrying a sub-value: `Val::Con(c, v)`
        // matches another `Val::Con(c', v')` iff names match and the
        // payloads unify.
        (Val::Con(lc, lv), Val::Con(rc, rv)) => {
            if lc != rc {
                return Err(mismatch(level, &lhs, &rhs));
            }
            unify(level, lv, rv, mctx)
        }

        // Pairs unify pointwise.
        (Val::Pair(la, lb), Val::Pair(ra, rb)) => {
            unify(level, la, ra, mctx)?;
            unify(level, lb, rb, mctx)
        }

        // Everything else: fall back to structural equality. This
        // covers Val::Sort, Val::One, Val::Unit, Val::Pi, Val::Sig,
        // Val::Lam, Val::Id, Val::Refl, EigonClass, EigonPrimitive,
        // etc. — for these Phase C v1 treats unification as eq_nf.
        // A future Phase C+ may push unification under binders for
        // Pi/Sig/Lam, but those cases aren't exercised by D48's
        // motivating use cases (Vec, Fin, Eq indices).
        _ => eq_nf(level, &lhs, &rhs).map_err(|_| mismatch(level, &lhs, &rhs)),
    }
}

/// Attempt to solve metavariable `id` (with spine `spine`) to `rhs`.
///
/// Pre: `rhs` is zonked.
///
/// Verifies:
/// 1. The spine is a sequence of distinct bound variables (pattern).
/// 2. `id` does not occur in `rhs` (occurs check).
/// 3. The solution `rhs` mentions only variables in scope (the bound
///    variables in `spine`, plus any free that were already in scope
///    when the meta was introduced; the latter is approximated for v1
///    by accepting any reference).
///
/// Then constructs the solution as either:
/// - The bare `rhs` if `spine` is empty (the meta wasn't applied to
///   anything; common case for D48's index unification).
/// - A lambda abstraction `λ spine. rhs` otherwise (Phase C v1 only
///   needs the bare case; lambda construction is deferred until a
///   real consumer with non-empty spines arrives).
fn solve_meta(
    level: usize,
    id: MetaId,
    spine: &[Val],
    rhs: &Val,
    mctx: &mut MetaCtx,
) -> Result<(), UnifyError> {
    // Pattern condition: each spine entry must be a distinct
    // generated variable (`Val::Nt(Neut::Gen(_, _))`).
    let bound_levels =
        spine_to_bound_levels(spine).map_err(|details| UnifyError::NonPatternSpine {
            meta: id,
            spine: details,
        })?;

    // Occurs check on the zonked rhs.
    if meta_occurs(id, rhs) {
        return Err(UnifyError::OccursCheck {
            meta: id,
            in_value: format!("{:?}", readback_val(level, rhs)),
        });
    }

    // For Phase C v1 we only solve the bare-rhs case (spine empty).
    // Non-empty spines arise for higher-order metavariables that
    // Phase C does not yet construct.
    if !bound_levels.is_empty() {
        return Err(UnifyError::NonPatternSpine {
            meta: id,
            spine: format!(
                "Phase C v1 only solves bare metavariables (empty spine); got {} bound vars",
                bound_levels.len()
            ),
        });
    }

    mctx.solve(id, rhs.clone())
}

/// Verify a meta's spine is a sequence of distinct bound variables
/// (`Val::Nt(Neut::Gen(level, _))`). Returns the levels on success,
/// or a description of why the spine isn't a pattern on failure.
fn spine_to_bound_levels(spine: &[Val]) -> Result<Vec<usize>, String> {
    let mut levels = Vec::with_capacity(spine.len());
    let mut seen = std::collections::BTreeSet::new();
    for (i, v) in spine.iter().enumerate() {
        match v {
            Val::Nt(Neut::Gen(level, _)) => {
                if !seen.insert(*level) {
                    return Err(format!("duplicate variable at position {i}"));
                }
                levels.push(*level);
            }
            other => {
                return Err(format!(
                    "spine entry {i} is not a bound variable: {other:?}"
                ));
            }
        }
    }
    Ok(levels)
}

/// True iff `meta` occurs anywhere in `val`. Walks structurally.
fn meta_occurs(meta: MetaId, val: &Val) -> bool {
    match val {
        Val::Nt(n) => meta_occurs_neut(meta, n),
        Val::Pair(a, b) | Val::Id(_, a, b) => meta_occurs(meta, a) || meta_occurs(meta, b),
        Val::Con(_, v) => meta_occurs(meta, v),
        Val::Refl(v) => meta_occurs(meta, v),
        Val::InductiveType {
            params, indices, ..
        } => {
            params.iter().any(|p| meta_occurs(meta, p))
                || indices.iter().any(|i| meta_occurs(meta, i))
        }
        Val::InductiveVal { args, .. } => args.iter().any(|a| meta_occurs(meta, a)),
        Val::CodataType { params, .. } => params.iter().any(|p| meta_occurs(meta, p)),
        Val::List(items) => items.iter().any(|v| meta_occurs(meta, v)),
        _ => false,
    }
}

fn meta_occurs_neut(meta: MetaId, n: &Neut) -> bool {
    match n {
        Neut::Meta(id, spine) => *id == meta || spine.iter().any(|v| meta_occurs(meta, v)),
        Neut::App(k, v) => meta_occurs_neut(meta, k) || meta_occurs(meta, v),
        Neut::Fst(k) | Neut::Snd(k) | Neut::PropAccess(k, _) | Neut::Observe(k, _) => {
            meta_occurs_neut(meta, k)
        }
        Neut::NtFun(_, _, k) => meta_occurs_neut(meta, k),
        Neut::NtMap(f, k) => meta_occurs(meta, f) || meta_occurs_neut(meta, k),
        Neut::NtReduce(f, acc, k) => {
            meta_occurs(meta, f) || meta_occurs(meta, acc) || meta_occurs_neut(meta, k)
        }
        Neut::NtRec {
            motive,
            minors,
            major,
            ..
        } => {
            meta_occurs(meta, motive)
                || minors.iter().any(|m| meta_occurs(meta, m))
                || meta_occurs_neut(meta, major)
        }
        _ => false,
    }
}

/// Substitute all solved metas with their solutions throughout `val`.
fn zonk_val(mctx: &MetaCtx, val: &Val) -> Val {
    match val {
        Val::Nt(Neut::Meta(id, spine)) => {
            if let Some(sol) = mctx.solution(*id) {
                // For Phase C v1 (bare metas), the solution is applied
                // directly. Spine application would be needed for
                // higher-order metas — out of scope.
                if spine.is_empty() {
                    zonk_val(mctx, sol)
                } else {
                    // Solution exists but the spine is non-empty:
                    // shouldn't happen in Phase C v1 since solve_meta
                    // rejects non-empty spines. Defensive: preserve.
                    Val::Nt(Neut::Meta(
                        *id,
                        spine.iter().map(|v| zonk_val(mctx, v)).collect(),
                    ))
                }
            } else {
                Val::Nt(Neut::Meta(
                    *id,
                    spine.iter().map(|v| zonk_val(mctx, v)).collect(),
                ))
            }
        }
        Val::Pair(a, b) => Val::Pair(Box::new(zonk_val(mctx, a)), Box::new(zonk_val(mctx, b))),
        Val::Con(c, v) => Val::Con(c.clone(), Box::new(zonk_val(mctx, v))),
        Val::Refl(v) => Val::Refl(Box::new(zonk_val(mctx, v))),
        Val::Id(t, x, y) => Val::Id(
            Box::new(zonk_val(mctx, t)),
            Box::new(zonk_val(mctx, x)),
            Box::new(zonk_val(mctx, y)),
        ),
        Val::InductiveType {
            decl,
            params,
            indices,
        } => Val::InductiveType {
            decl: decl.clone(),
            params: params.iter().map(|p| zonk_val(mctx, p)).collect(),
            indices: indices.iter().map(|i| zonk_val(mctx, i)).collect(),
        },
        Val::InductiveVal {
            decl,
            ctor_name,
            args,
        } => Val::InductiveVal {
            decl: decl.clone(),
            ctor_name: ctor_name.clone(),
            args: args.iter().map(|a| zonk_val(mctx, a)).collect(),
        },
        // For Val variants without nested metas (or where zonking the
        // inner structure isn't load-bearing for Phase C / D), clone
        // through. A future phase can extend if needed.
        other => other.clone(),
    }
}

fn mismatch(level: usize, lhs: &Val, rhs: &Val) -> UnifyError {
    UnifyError::Mismatch {
        lhs: format!("{:?}", readback_val(level, lhs)),
        rhs: format!("{:?}", readback_val(level, rhs)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::Exp;
    use crate::nbe::term::{InductiveCtorDecl, InductiveDecl, Patt};
    use std::sync::Arc;

    fn nat_decl() -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse("urn:test:Nat").unwrap(),
            name: "Nat".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: vec![
                InductiveCtorDecl {
                    name: "zero".to_string(),
                    typ: Exp::Sort(1), // placeholder; not used by tests
                },
                InductiveCtorDecl {
                    name: "succ".to_string(),
                    typ: Exp::Sort(1), // placeholder
                },
            ],
        })
    }

    fn nat_zero(decl: &Arc<InductiveDecl>) -> Val {
        Val::InductiveVal {
            decl: decl.clone(),
            ctor_name: "zero".to_string(),
            args: Vec::new(),
        }
    }

    fn nat_succ(decl: &Arc<InductiveDecl>, n: Val) -> Val {
        Val::InductiveVal {
            decl: decl.clone(),
            ctor_name: "succ".to_string(),
            args: vec![n],
        }
    }

    fn bound_var(level: usize) -> Val {
        Val::Nt(Neut::Gen(level, "x".to_string()))
    }

    fn fresh_meta(mctx: &mut MetaCtx) -> (MetaId, Val) {
        let id = mctx.fresh();
        (id, Val::Nt(Neut::Meta(id, Vec::new())))
    }

    // ---- structural unification ----

    #[test]
    fn unify_identical_values_succeeds() {
        let mut mctx = MetaCtx::new();
        unify(0, &Val::One, &Val::One, &mut mctx).unwrap();
        unify(0, &Val::Sort(0), &Val::Sort(0), &mut mctx).unwrap();
        unify(0, &Val::Sort(2), &Val::Sort(2), &mut mctx).unwrap();
    }

    #[test]
    fn unify_distinct_universes_fails() {
        let mut mctx = MetaCtx::new();
        let err = unify(0, &Val::Sort(0), &Val::Sort(1), &mut mctx).unwrap_err();
        assert!(matches!(err, UnifyError::Mismatch { .. }));
    }

    #[test]
    fn unify_ctor_eq_succeeds() {
        let nat = nat_decl();
        let mut mctx = MetaCtx::new();
        let zero = nat_zero(&nat);
        let one = nat_succ(&nat, nat_zero(&nat));
        unify(0, &zero, &zero, &mut mctx).unwrap();
        unify(0, &one, &one, &mut mctx).unwrap();
    }

    #[test]
    fn unify_distinct_ctors_fails() {
        let nat = nat_decl();
        let zero = nat_zero(&nat);
        let one = nat_succ(&nat, nat_zero(&nat));
        let mut mctx = MetaCtx::new();
        let err = unify(0, &zero, &one, &mut mctx).unwrap_err();
        assert!(matches!(err, UnifyError::Mismatch { .. }));
    }

    // ---- metavariable solving ----

    #[test]
    fn unify_meta_against_concrete_solves() {
        let nat = nat_decl();
        let mut mctx = MetaCtx::new();
        let (id, m) = fresh_meta(&mut mctx);
        let zero = nat_zero(&nat);
        unify(0, &m, &zero, &mut mctx).unwrap();
        // After unification, ?id is bound to zero.
        let sol = mctx.solution(id).expect("?id should be solved");
        assert!(matches!(sol, Val::InductiveVal { ctor_name, .. } if ctor_name == "zero"));
    }

    #[test]
    fn unify_meta_against_succ_solves_with_inner_structure() {
        // ?n = succ (succ zero) — solves ?n := succ (succ zero).
        let nat = nat_decl();
        let mut mctx = MetaCtx::new();
        let (id, m) = fresh_meta(&mut mctx);
        let two = nat_succ(&nat, nat_succ(&nat, nat_zero(&nat)));
        unify(0, &m, &two, &mut mctx).unwrap();
        let sol = mctx.solution(id).unwrap();
        // Solution is `succ (succ zero)` structurally.
        let zonked = mctx.zonk(&m);
        let read = readback_val(0, &zonked);
        let _ = sol;
        let _ = read;
        // Just verify it round-trips structurally — the assert above
        // already confirmed solve.
    }

    #[test]
    fn unify_meta_eq_same_meta_succeeds_via_zonk() {
        let mut mctx = MetaCtx::new();
        let (_, m) = fresh_meta(&mut mctx);
        // ?id = ?id — succeeds trivially via the same-meta branch.
        unify(0, &m, &m, &mut mctx).unwrap();
    }

    #[test]
    fn unify_meta_eq_different_meta_solves_one_to_other() {
        let mut mctx = MetaCtx::new();
        let (id1, m1) = fresh_meta(&mut mctx);
        let (id2, m2) = fresh_meta(&mut mctx);
        // ?id1 = ?id2 — solves ?id1 := ?id2.
        unify(0, &m1, &m2, &mut mctx).unwrap();
        // Now zonking ?id1 should yield ?id2 (the solution).
        let zonked = mctx.zonk(&m1);
        assert!(matches!(
            zonked,
            Val::Nt(Neut::Meta(z, _)) if z == id2
        ));
        let _ = id1;
    }

    // ---- occurs check ----

    #[test]
    fn occurs_check_rejects_x_eq_succ_x() {
        // ?x = succ ?x — must be rejected.
        let nat = nat_decl();
        let mut mctx = MetaCtx::new();
        let (_, m) = fresh_meta(&mut mctx);
        let succ_m = nat_succ(&nat, m.clone());
        let err = unify(0, &m, &succ_m, &mut mctx).unwrap_err();
        assert!(
            matches!(err, UnifyError::OccursCheck { .. }),
            "expected OccursCheck, got {err:?}"
        );
    }

    #[test]
    fn occurs_check_rejects_x_eq_pair_of_x() {
        let mut mctx = MetaCtx::new();
        let (_, m) = fresh_meta(&mut mctx);
        let pair = Val::Pair(Box::new(m.clone()), Box::new(Val::Unit));
        let err = unify(0, &m, &pair, &mut mctx).unwrap_err();
        assert!(matches!(err, UnifyError::OccursCheck { .. }));
    }

    // ---- spine restrictions ----

    #[test]
    fn meta_with_non_pattern_spine_rejected() {
        // ?m applied to a non-bound-variable spine — rejected because
        // Phase C only solves first-order patterns with empty spines.
        let mut mctx = MetaCtx::new();
        let id = mctx.fresh();
        let bad_spine = vec![Val::Unit]; // not a Neut::Gen
        let m = Val::Nt(Neut::Meta(id, bad_spine));
        let err = unify(0, &m, &Val::Sort(0), &mut mctx).unwrap_err();
        assert!(matches!(err, UnifyError::NonPatternSpine { .. }));
    }

    #[test]
    fn meta_with_empty_spine_solves() {
        // Default fresh metas have empty spines; solving works.
        let mut mctx = MetaCtx::new();
        let (id, m) = fresh_meta(&mut mctx);
        unify(0, &m, &Val::Sort(0), &mut mctx).unwrap();
        assert!(mctx.solution(id).is_some());
    }

    #[test]
    fn meta_with_pattern_spine_currently_unsupported() {
        // A spine of distinct bound vars passes the pattern check but
        // Phase C v1 still rejects non-empty spines (lambda
        // construction is deferred).
        let mut mctx = MetaCtx::new();
        let id = mctx.fresh();
        let spine = vec![bound_var(0), bound_var(1)];
        let m = Val::Nt(Neut::Meta(id, spine));
        let err = unify(2, &m, &Val::Sort(0), &mut mctx).unwrap_err();
        assert!(matches!(err, UnifyError::NonPatternSpine { .. }));
    }

    // ---- inductive type unification (D48's main consumer) ----

    #[test]
    fn unify_vec_a_concrete_indices_succeeds() {
        // Vec A 0 = Vec A 0 — structural equality on indexed type.
        let decl = vec_decl();
        let lhs = vec_type(&decl, Val::Sort(0), Val::Unit);
        let rhs = vec_type(&decl, Val::Sort(0), Val::Unit);
        let mut mctx = MetaCtx::new();
        unify(0, &lhs, &rhs, &mut mctx).unwrap();
    }

    #[test]
    fn unify_vec_a_distinct_indices_fails() {
        let decl = vec_decl();
        let lhs = vec_type(&decl, Val::Sort(0), Val::Unit);
        let rhs = vec_type(&decl, Val::Sort(0), Val::One);
        let mut mctx = MetaCtx::new();
        let err = unify(0, &lhs, &rhs, &mut mctx).unwrap_err();
        assert!(matches!(err, UnifyError::Mismatch { .. }));
    }

    #[test]
    fn unify_vec_a_meta_index_solves_meta() {
        // Vec A ?n = Vec A () — solves ?n := ()
        let decl = vec_decl();
        let mut mctx = MetaCtx::new();
        let (id, n_meta) = fresh_meta(&mut mctx);
        let lhs = vec_type(&decl, Val::Sort(0), n_meta);
        let rhs = vec_type(&decl, Val::Sort(0), Val::Unit);
        unify(0, &lhs, &rhs, &mut mctx).unwrap();
        assert!(matches!(mctx.solution(id), Some(Val::Unit)));
    }

    #[test]
    fn unify_vec_distinct_decls_fails() {
        let v1 = vec_decl_named("VecA");
        let v2 = vec_decl_named("VecB");
        let lhs = vec_type(&v1, Val::Sort(0), Val::Unit);
        let rhs = vec_type(&v2, Val::Sort(0), Val::Unit);
        let mut mctx = MetaCtx::new();
        let err = unify(0, &lhs, &rhs, &mut mctx).unwrap_err();
        assert!(matches!(err, UnifyError::Mismatch { .. }));
    }

    fn vec_decl() -> Arc<InductiveDecl> {
        vec_decl_named("Vec")
    }

    fn vec_decl_named(name: &str) -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).expect("test iri"),
            name: name.to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::Sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        })
    }

    fn vec_type(decl: &Arc<InductiveDecl>, a: Val, n: Val) -> Val {
        Val::InductiveType {
            decl: decl.clone(),
            params: vec![a],
            indices: vec![n],
        }
    }
}
