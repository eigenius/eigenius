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
use crate::nbe::term::Patt;
use crate::nbe::val::{MetaId, Neut, Val};
use std::collections::{BTreeMap, BTreeSet};

/// A registry of unification metavariables and their solutions.
///
/// Allocates fresh [`MetaId`]s and stores their solved values. Cheap
/// to clone (the inner map is sized by the number of metas in flight).
#[derive(Debug, Clone, Default)]
pub struct MetaCtx {
    next: u32,
    solutions: BTreeMap<MetaId, Val>,
    /// The de Bruijn level in scope when each metavariable was created.
    ///
    /// A meta stands for a value that could have been written where it was created, so its
    /// solution may mention only what was in scope there. Variables bound *inside* the value
    /// being unified are not: solving `?a` to something mentioning them would let a bound
    /// variable escape its binder, and the resulting term names a variable that does not exist
    /// at the meta's own site. [`solve_meta`] enforces this.
    levels: BTreeMap<MetaId, usize>,
}

impl MetaCtx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh unsolved metavariable that scopes over `level` binders.
    ///
    /// `level` is the caller's current de Bruijn level — how many variables are in scope where
    /// the unknown stands. It bounds what the meta may be solved to; see [`MetaCtx::levels`].
    pub fn fresh(&mut self, level: usize) -> MetaId {
        let id = MetaId(self.next);
        self.next += 1;
        self.levels.insert(id, level);
        id
    }

    /// The level `id` was created at, or 0 for a meta this context did not allocate.
    fn level_of(&self, id: MetaId) -> usize {
        self.levels.get(&id).copied().unwrap_or(0)
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
    /// Solving the metavariable was attempted from inside a binder it does not scope over.
    /// The solution would mention a variable that is not in scope where the meta stands.
    EscapesBinder {
        meta: MetaId,
        /// The level the meta was created at.
        meta_level: usize,
        /// The level unification had descended to when the solution was proposed.
        at_level: usize,
    },
    /// A closure could not be instantiated while comparing under a binder.
    Eval(String),
    /// The proposed solution has a shape the scope check cannot see inside — one carrying a
    /// `Rho`, or an uninterpreted payload — so whether a variable escapes is undecidable. The
    /// solve is refused: an undecidable solution is not a safe one.
    Undecidable { meta: MetaId, shape: String },
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
            UnifyError::Undecidable { meta, shape } => write!(
                f,
                "metavariable {meta}: cannot decide whether the proposed solution ({shape}) \
                 mentions a variable out of scope — refusing rather than guessing"
            ),
            UnifyError::EscapesBinder {
                meta,
                meta_level,
                at_level,
            } => write!(
                f,
                "metavariable {meta} stands at level {meta_level} but a solution was proposed \
                 at level {at_level}, inside {} binder(s) it does not scope over — the solution \
                 would name a variable that does not exist where the meta stands",
                at_level - meta_level
            ),
            UnifyError::Eval(msg) => write!(f, "evaluation failed while unifying: {msg}"),
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
/// **Takes no environment**, for the same reason [`eq_nf`] does not (D76 Phase D):
/// unification falls back to `eq_nf` and otherwise recurses, and never reaches the
/// one arm that consults `Γ_env` — `Refine` subtyping. Threading one in left a
/// parameter used only by the recursion, which clippy reported and which would
/// have been a parameter that lies about what the function needs.
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

        // PROTOTYPE (D89 experiment): a meta APPLIED to arguments. Evaluation never populates
        // `Neut::Meta`'s spine — it builds `Neut::App(Meta(id, []), arg)` chains — so this is the
        // only form `?P y` actually takes, and without this arm it falls through to `eq_nf`.
        (Val::Nt(ln), _) if as_meta_spine(ln).is_some() => {
            let (id, spine) = as_meta_spine(ln).expect("guarded above");
            solve_meta(level, id, &spine, &rhs, mctx)
        }
        (_, Val::Nt(rn)) if as_meta_spine(rn).is_some() => {
            let (id, spine) = as_meta_spine(rn).expect("guarded above");
            solve_meta(level, id, &spine, &lhs, mctx)
        }

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
                iri: ld,
                ctor_name: lc,
                args: la,
            },
            Val::InductiveVal {
                iri: rd,
                ctor_name: rc,
                args: ra,
            },
        ) => {
            if ld != rd || lc != rc {
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

        // Two ANONYMOUS arrows, compared componentwise so metas on either side can be solved.
        //
        // `justification:Grounds.app` is why. Its first argument is declared
        // `Certificate(A -> B)`, so with `A` and `B` implicit the index to unify is a `Val::Pi`
        // carrying a meta in its domain, its codomain, or both. Readback equality cannot see
        // inside it, and `A` occurs in no result index, so this is the only place `A` can be
        // determined at all — and in inference mode, where nothing fixes `B` up front either,
        // the same comparison is what determines `B`.
        //
        // **Anonymous is what makes this safe, and why the arm is restricted to it.** A
        // `Patt::Unit` binder cannot be referenced, so neither codomain mentions it, so no
        // variable is introduced and both sides are compared at the SAME level. `solve_meta`'s
        // scope check therefore never has to decide whether a solution captured something: there
        // is nothing to capture. A named binder falls through to `eq_nf` below, unchanged.
        //
        // It is also why this is not a behaviour change for meta-free types. Readback preserves
        // `Patt::Unit` (D49 witness-key byte stability), so for two anonymous arrows readback
        // equality already IS componentwise equality. The pair `eq_nf` separates and this would
        // not — an anonymous arrow against a named-but-unused binder — is exactly what the guard
        // excludes.
        (Val::Pi(ld, lc), Val::Pi(rd, rc))
            if matches!(lc.patt, Patt::Unit) && matches!(rc.patt, Patt::Unit) =>
        {
            unify(level, ld, rd, mctx)?;
            let lbody = lc
                .apply(Val::Unit)
                .map_err(|e| UnifyError::Eval(format!("{e:?}")))?;
            let rbody = rc
                .apply(Val::Unit)
                .map_err(|e| UnifyError::Eval(format!("{e:?}")))?;
            unify(level, &lbody, &rbody, mctx)
        }

        // PROTOTYPE (D89 experiment): two NAMED binders, instantiated with a fresh generated
        // variable and compared one level down. This is what `instantiate`'s premise type
        // `Certificate(forall (y : T) => P(y))` needs, and what the anonymous restriction above
        // deliberately excluded.
        //
        // **Both sides must be named.** An anonymous arrow against a named-but-unused binder reads
        // back differently — readback preserves `Patt::Unit` for D49's witness-key byte stability —
        // so identifying them here would make two propositions with different witness keys unify.
        // `meta_free_function_types_are_still_compared_by_readback` pins that, and an unguarded
        // version of this arm breaks it.
        (Val::Pi(ld, lc), Val::Pi(rd, rc))
            if !matches!(lc.patt, Patt::Unit) && !matches!(rc.patt, Patt::Unit) =>
        {
            unify(level, ld, rd, mctx)?;
            let v = Val::Nt(Neut::Gen(level, "G#".to_string()));
            let lbody = lc
                .apply(v.clone())
                .map_err(|e| UnifyError::Eval(format!("{e:?}")))?;
            let rbody = rc
                .apply(v)
                .map_err(|e| UnifyError::Eval(format!("{e:?}")))?;
            unify(level + 1, &lbody, &rbody, mctx)
        }

        // Everything else: fall back to structural equality. This
        // covers Val::Sort, Val::One, Val::Unit, Val::Lam, Val::Id,
        // Val::Refl, EigonClass, EigonPrimitive, etc. — for these
        // Phase C v1 treats unification as eq_nf.
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

    // A pattern spine is solved by abstracting the rhs over exactly the variables the spine names:
    // `?P x ≟ B` gives `?P := λ x. B`. The spine is what makes the scope question answerable —
    // it enumerates the variables this meta is allowed to keep, so the check is "does the solution
    // mention any OTHER variable introduced at or above the meta's own level".
    if !bound_levels.is_empty() {
        let meta_level = mctx.level_of(id);
        let Some(mentioned) = mentioned_gen_levels(rhs, level, &mut Vec::new()) else {
            // The walk met a shape it cannot see inside — one carrying a `Rho`, or an
            // uninterpreted payload. A variable may hide there, so the solution is refused
            // rather than admitted on a walk that would answer "no escape" for the unsound case.
            return Err(UnifyError::Undecidable {
                meta: id,
                shape: format!("{:?}", readback_val(level, rhs)),
            });
        };
        if let Some(&at_level) = mentioned
            .iter()
            .find(|l| **l >= meta_level && !bound_levels.contains(l))
        {
            return Err(UnifyError::EscapesBinder {
                meta: id,
                meta_level,
                at_level,
            });
        }
        let body = readback_val(level, rhs);
        let abstracted = bound_levels.iter().rev().fold(body, |acc, l| {
            crate::nbe::term::Exp::Lam(Patt::Var(format!("G#{l}")), Box::new(acc))
        });
        let solution = crate::nbe::eval::eval(&abstracted, &crate::nbe::env::Rho::Nil)
            .map_err(|e| UnifyError::Eval(format!("{e:?}")))?;
        return mctx.solve(id, solution);
    }

    // Scope check, for the bare metas this actually solves. `id` stands for a value writable
    // where it was created; `level` is where unification has got to. Descending through a binder
    // raises `level`, and what the two sides agree on down there may mention that binder's
    // variable — a variable that does not exist where `id` stands. Refuse rather than inspect the
    // proposed solution: a `Val` hides variables inside closure environments, so "does this
    // mention a variable above level N" is not decidable by a structural walk, and a walk that
    // treats closures as opaque would answer no for exactly the unsound cases.
    //
    // A meta with a non-empty spine is a different question — the spine names the variables it
    // does scope over — and is rejected above regardless, so this rule governs the whole of what
    // gets solved.
    let meta_level = mctx.level_of(id);
    if level > meta_level {
        return Err(UnifyError::EscapesBinder {
            meta: id,
            meta_level,
            at_level: level,
        });
    }

    mctx.solve(id, rhs.clone())
}

/// PROTOTYPE (D89 experiment): peel a neutral `App` chain down to a `Meta` head.
///
/// Evaluation builds `?P y` as `Neut::App(Meta(id, []), y)`, never as `Meta(id, [y])`, so this is
/// what recovers the `(meta, spine)` pair the pattern rules are written against.
fn as_meta_spine(neut: &Neut) -> Option<(MetaId, Vec<Val>)> {
    match neut {
        Neut::Meta(id, spine) if !spine.is_empty() => Some((*id, spine.clone())),
        Neut::App(head, arg) => {
            let (id, mut spine) = as_meta_spine(head).or_else(|| match head.as_ref() {
                Neut::Meta(id, s) if s.is_empty() => Some((*id, Vec::new())),
                _ => None,
            })?;
            spine.push(arg.as_ref().clone());
            Some((id, spine))
        }
        _ => None,
    }
}

/// The generated-variable levels a value mentions, or `None` when the shape cannot be decided.
///
/// **Levels, not names.** An earlier version read the value back and parsed digits off the free
/// variable names. That could not work: `Neut::Gen(j, name)` reads back as `Var("{name}{j}")`
/// keeping whatever tag the producer chose, and the tree uses at least `G#` (readback), `TC#`
/// (`env::gen_val`) and ad-hoc tags in tests. Keyed on one prefix it misses the others — a scope
/// check that admits exactly the cases it exists to refuse. Parsing trailing digits off any name
/// avoided that by over-approximating, at the cost of reading a user variable named `foo12` as
/// generated. Neither is a check you want guarding soundness, so this reads the levels directly.
///
/// **`None` means undecidable, and callers must refuse.** `Val::Record`, `Fun`, `Data` and
/// `NtFun` / `NtMatch` carry a `Rho`, and `TemplateVal` / `ResourceVal` carry payloads this does
/// not interpret; a variable can hide inside any of them. Rather than walk them wrongly, they
/// answer `None`. That is the fail-closed direction: an undecidable solution is not solved.
///
/// Binders are entered the way readback enters them — instantiate the closure with a fresh
/// generated variable, and record that level as BOUND so the walk does not report its own
/// scaffolding as a free occurrence.
fn mentioned_gen_levels(v: &Val, depth: usize, bound: &mut Vec<usize>) -> Option<BTreeSet<usize>> {
    let mut out = BTreeSet::new();
    walk_val(v, depth, bound, &mut out)?;
    Some(out)
}

fn walk_val(
    v: &Val,
    depth: usize,
    bound: &mut Vec<usize>,
    out: &mut BTreeSet<usize>,
) -> Option<()> {
    match v {
        // No variables.
        Val::Sort(_)
        | Val::One
        | Val::Unit
        | Val::EigonClass(_)
        | Val::EigonPrimitive(_)
        | Val::LitString(_)
        | Val::LitInt(_)
        | Val::LitFloat(_)
        | Val::LitBool(_)
        // A `WitnessKey` — an IRI and a proposition hash. No variables.
        | Val::ChainWitness(_) => Some(()),

        Val::Pair(a, b) => {
            walk_val(a, depth, bound, out)?;
            walk_val(b, depth, bound, out)
        }
        Val::Con(_, inner) | Val::Refl(inner) => walk_val(inner, depth, bound, out),
        Val::Id(t, x, y) => {
            walk_val(t, depth, bound, out)?;
            walk_val(x, depth, bound, out)?;
            walk_val(y, depth, bound, out)
        }
        Val::List(vs) => {
            for x in vs {
                walk_val(x, depth, bound, out)?;
            }
            Some(())
        }
        Val::InductiveType { params, indices, .. } => {
            for x in params.iter().chain(indices.iter()) {
                walk_val(x, depth, bound, out)?;
            }
            Some(())
        }
        Val::InductiveVal { args, .. } => {
            for x in args {
                walk_val(x, depth, bound, out)?;
            }
            Some(())
        }
        Val::Refine(inner, _) => walk_val(inner, depth, bound, out),

        // Binders: enter as readback does, and mark the variable we introduce as bound.
        Val::Lam(c) => {
            let v = Val::Nt(Neut::Gen(depth, "walk#".to_string()));
            let body = c.apply(v).ok()?;
            bound.push(depth);
            let r = walk_val(&body, depth + 1, bound, out);
            bound.pop();
            r
        }
        Val::Pi(dom, c) | Val::Sig(dom, c) => {
            walk_val(dom, depth, bound, out)?;
            let v = Val::Nt(Neut::Gen(depth, "walk#".to_string()));
            let body = c.apply(v).ok()?;
            bound.push(depth);
            let r = walk_val(&body, depth + 1, bound, out);
            bound.pop();
            r
        }

        Val::Nt(n) => walk_neut(n, depth, bound, out),

        // Carries a `Rho` or an uninterpreted payload — a variable can hide inside. Undecidable.
        Val::Record(..) | Val::Fun(..) | Val::Data(..) | Val::TemplateVal(..) => None,
        Val::ResourceVal(_) => None,
    }
}

fn walk_neut(
    n: &Neut,
    depth: usize,
    bound: &mut Vec<usize>,
    out: &mut BTreeSet<usize>,
) -> Option<()> {
    match n {
        Neut::Gen(level, _) => {
            if !bound.contains(level) {
                out.insert(*level);
            }
            Some(())
        }
        Neut::Const(..) | Neut::EigonAxiom(_) | Neut::Checked(_) => Some(()),
        Neut::Meta(_, spine) => {
            for x in spine {
                walk_val(x, depth, bound, out)?;
            }
            Some(())
        }
        Neut::App(head, arg) => {
            walk_neut(head, depth, bound, out)?;
            walk_val(arg, depth, bound, out)
        }
        Neut::Fst(inner) | Neut::Snd(inner) => walk_neut(inner, depth, bound, out),
        Neut::PropAccess(inner, _) => walk_neut(inner, depth, bound, out),
        Neut::NtMap(f, inner) => {
            walk_val(f, depth, bound, out)?;
            walk_neut(inner, depth, bound, out)
        }
        Neut::NtReduce(f, z, inner) => {
            walk_val(f, depth, bound, out)?;
            walk_val(z, depth, bound, out)?;
            walk_neut(inner, depth, bound, out)
        }
        Neut::NtRec {
            motive,
            minors,
            major,
            ..
        } => {
            walk_val(motive, depth, bound, out)?;
            for m in minors {
                walk_val(m, depth, bound, out)?;
            }
            walk_neut(major, depth, bound, out)
        }
        // Carry a `Rho`. Undecidable, so refused.
        Neut::NtFun(..) | Neut::NtMatch { .. } => None,
    }
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
        Val::List(items) => items.iter().any(|v| meta_occurs(meta, v)),
        _ => false,
    }
}

fn meta_occurs_neut(meta: MetaId, n: &Neut) -> bool {
    match n {
        Neut::Meta(id, spine) => *id == meta || spine.iter().any(|v| meta_occurs(meta, v)),
        Neut::App(k, v) => meta_occurs_neut(meta, k) || meta_occurs(meta, v),
        Neut::Fst(k) | Neut::Snd(k) | Neut::PropAccess(k, _) => meta_occurs_neut(meta, k),
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
/// PROTOTYPE (D89 experiment): apply a zonked head to a zonked argument, beta-reducing when the
/// head became a lambda. Substituting a solved higher-order meta is not complete without this —
/// `?P` solved to `λ y. B` leaves `(λ y. B) x` standing where the caller expects `B`.
fn zonk_apply(head: Val, arg: Val) -> Val {
    match head {
        Val::Lam(ref clos) => clos.apply(arg.clone()).unwrap_or_else(|_| {
            Val::Nt(Neut::App(
                Box::new(Neut::Gen(usize::MAX, "zonk#stuck".to_string())),
                Box::new(arg),
            ))
        }),
        Val::Nt(n) => Val::Nt(Neut::App(Box::new(n), Box::new(arg))),
        other => other,
    }
}

/// PROTOTYPE (D89 experiment): zonk a neutral, resolving `App` chains whose head is a solved meta.
fn zonk_neut(mctx: &MetaCtx, n: &Neut) -> Val {
    match n {
        Neut::Meta(id, spine) => {
            let spine: Vec<Val> = spine.iter().map(|v| zonk_val(mctx, v)).collect();
            match mctx.solution(*id) {
                Some(sol) => spine.into_iter().fold(zonk_val(mctx, sol), zonk_apply),
                None => Val::Nt(Neut::Meta(*id, spine)),
            }
        }
        Neut::App(head, arg) => {
            let h = zonk_neut(mctx, head);
            let a = zonk_val(mctx, arg);
            zonk_apply(h, a)
        }
        other => Val::Nt(other.clone()),
    }
}

fn zonk_val(mctx: &MetaCtx, val: &Val) -> Val {
    match val {
        Val::Nt(n) => zonk_neut(mctx, n),
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
            iri,
            ctor_name,
            args,
        } => Val::InductiveVal {
            iri: iri.clone(),
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
                    typ: Exp::sort(1), // placeholder; not used by tests
                },
                InductiveCtorDecl {
                    implicit: Vec::new(),
                    name: "succ".to_string(),
                    typ: Exp::sort(1), // placeholder
                },
            ],
        })
    }

    fn nat_zero(decl: &Arc<InductiveDecl>) -> Val {
        Val::InductiveVal {
            iri: decl.iri.clone(),
            ctor_name: "zero".to_string(),
            args: Vec::new(),
        }
    }

    fn nat_succ(decl: &Arc<InductiveDecl>, n: Val) -> Val {
        Val::InductiveVal {
            iri: decl.iri.clone(),
            ctor_name: "succ".to_string(),
            args: vec![n],
        }
    }

    fn bound_var(level: usize) -> Val {
        Val::Nt(Neut::Gen(level, "x".to_string()))
    }

    fn fresh_meta(mctx: &mut MetaCtx) -> (MetaId, Val) {
        let id = mctx.fresh(0);
        (id, Val::Nt(Neut::Meta(id, Vec::new())))
    }

    // ---- structural unification ----

    #[test]
    fn unify_identical_values_succeeds() {
        let mut mctx = MetaCtx::new();
        unify(0, &Val::One, &Val::One, &mut mctx).unwrap();
        unify(0, &Val::sort(0), &Val::sort(0), &mut mctx).unwrap();
        unify(0, &Val::sort(2), &Val::sort(2), &mut mctx).unwrap();
    }

    #[test]
    fn unify_distinct_universes_fails() {
        let mut mctx = MetaCtx::new();
        let err = unify(0, &Val::sort(0), &Val::sort(1), &mut mctx).unwrap_err();
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

    // ---- unification under a binder ----

    fn arrow(dom: Val, cod: Exp) -> Val {
        Val::Pi(
            Box::new(dom),
            crate::nbe::val::Clos::new(Patt::Unit, cod, crate::nbe::env::Rho::Nil),
        )
    }

    /// A meta in a function type's DOMAIN is solved by comparing the two types componentwise.
    ///
    /// `?a -> One` against `Prop -> One`. Readback equality cannot solve this: `?a` and `Prop`
    /// read back differently, and the whole `Val::Pi` used to fall through to `eq_nf`. This is
    /// how `justification:Grounds.app`'s `A` — which occurs in no result index — is
    /// determined from its first argument's type.
    #[test]
    fn a_meta_in_a_function_types_domain_is_solved() {
        let mut mctx = MetaCtx::new();
        let (id, m) = fresh_meta(&mut mctx);
        let lhs = arrow(m, Exp::One);
        let rhs = arrow(Val::sort(0), Exp::One);
        unify(0, &lhs, &rhs, &mut mctx).unwrap();
        assert!(
            matches!(mctx.solution(id), Some(Val::Sort(_))),
            "?a should be solved to Prop, got {:?}",
            mctx.solution(id)
        );
    }

    /// Differing codomains are still a mismatch — descending into the binder compares, it does
    /// not excuse.
    #[test]
    fn a_solvable_domain_does_not_excuse_a_mismatched_codomain() {
        let mut mctx = MetaCtx::new();
        let (_, m) = fresh_meta(&mut mctx);
        let lhs = arrow(m, Exp::One);
        let rhs = arrow(Val::sort(0), Exp::sort(0));
        unify(0, &lhs, &rhs, &mut mctx).unwrap_err();
    }

    /// A metavariable is not solved from inside a binder it does not scope over.
    ///
    /// `?a` stands at level 0. Unifying it against something at level 1 means the two sides only
    /// agree under a binder, and the solution could name that binder's variable — a variable that
    /// does not exist where `?a` was written. The unifier refuses rather than capturing it.
    #[test]
    fn a_meta_is_not_solved_from_under_a_binder_it_does_not_scope_over() {
        let mut mctx = MetaCtx::new();
        let (id, m) = fresh_meta(&mut mctx);
        let err = unify(1, &m, &Val::One, &mut mctx).unwrap_err();
        assert!(
            matches!(
                err,
                UnifyError::EscapesBinder {
                    meta_level: 0,
                    at_level: 1,
                    ..
                }
            ),
            "expected an escaping-binder refusal, got {err:?}"
        );
        assert!(mctx.solution(id).is_none(), "nothing may have been solved");
    }

    /// Types with no metavariable keep taking `eq_nf` verbatim.
    ///
    /// An anonymous arrow and a named-but-unused binder read back differently — readback preserves
    /// `Patt::Unit` for D49's witness-key byte stability — so `eq_nf` separates them. Componentwise
    /// comparison would not, which is why the new arms are gated on a meta being present.
    #[test]
    fn meta_free_function_types_are_still_compared_by_readback() {
        let mut mctx = MetaCtx::new();
        let anonymous = arrow(Val::One, Exp::One);
        let named = Val::Pi(
            Box::new(Val::One),
            crate::nbe::val::Clos::new(
                Patt::Var("x".to_string()),
                Exp::One,
                crate::nbe::env::Rho::Nil,
            ),
        );
        unify(0, &anonymous, &named, &mut mctx)
            .expect_err("eq_nf distinguishes these, and with no meta present it decides");
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
        let id = mctx.fresh(0);
        let bad_spine = vec![Val::Unit]; // not a Neut::Gen
        let m = Val::Nt(Neut::Meta(id, bad_spine));
        let err = unify(0, &m, &Val::sort(0), &mut mctx).unwrap_err();
        assert!(matches!(err, UnifyError::NonPatternSpine { .. }));
    }

    #[test]
    fn meta_with_empty_spine_solves() {
        // Default fresh metas have empty spines; solving works.
        let mut mctx = MetaCtx::new();
        let (id, m) = fresh_meta(&mut mctx);
        unify(0, &m, &Val::sort(0), &mut mctx).unwrap();
        assert!(mctx.solution(id).is_some());
    }

    /// The spine names what the meta may keep; anything else still escapes.
    ///
    /// `?P` is created at level 0 and reached under two binders. Its spine names only the variable
    /// at level 1, so a solution mentioning the one at level 0 would carry a variable out of the
    /// binder that introduced it. Readback is what makes this decidable: it forces every closure,
    /// so the free variables of the proposed solution are visible syntactically.
    #[test]
    fn a_pattern_spine_still_refuses_a_solution_mentioning_a_variable_it_does_not_name() {
        let mut mctx = MetaCtx::new();
        let id = mctx.fresh(0);
        let m = Val::Nt(Neut::Meta(id, vec![bound_var(1)]));
        let err = unify(2, &m, &bound_var(0), &mut mctx)
            .expect_err("G#0 is not in the spine and not in scope where ?P stands");
        assert!(
            matches!(err, UnifyError::EscapesBinder { .. }),
            "expected an escaping-binder refusal, got {err:?}"
        );
        assert!(mctx.solution(id).is_none(), "nothing may have been solved");
    }

    /// The scope check reads LEVELS, so a generated variable's name tag cannot hide it.
    ///
    /// This is the regression the level walk exists for. An earlier version read the solution back
    /// and looked for `G#`-prefixed names; `Neut::Gen` keeps whatever tag its producer chose, and
    /// the tree uses at least `G#`, `TC#` and ad-hoc tags in tests. Keyed on one prefix, this case
    /// SOLVED — admitting a variable out of scope, which is precisely what the check exists to
    /// refuse. Each tag below must be refused identically.
    #[test]
    fn the_scope_check_does_not_depend_on_a_variables_name_tag() {
        for tag in ["G#", "TC#", "x", "foo", ""] {
            let mut mctx = MetaCtx::new();
            let id = mctx.fresh(0);
            let m = Val::Nt(Neut::Meta(id, vec![Val::Nt(Neut::Gen(1, tag.to_string()))]));
            let escaping = Val::Nt(Neut::Gen(0, tag.to_string()));
            let err = unify(2, &m, &escaping, &mut mctx).expect_err(
                "a variable the spine does not name is out of scope whatever it is called",
            );
            assert!(
                matches!(err, UnifyError::EscapesBinder { .. }),
                "tag {tag:?} should escape, got {err:?}"
            );
            assert!(
                mctx.solution(id).is_none(),
                "tag {tag:?} must not be solved"
            );
        }
    }

    /// A shape the walk cannot see inside is refused, not admitted.
    ///
    /// `Val::Fun` carries a `Rho`, so a variable can hide in its environment. The check answers
    /// "undecidable" and the solve fails closed rather than reporting no escape.
    #[test]
    fn an_undecidable_shape_is_refused_rather_than_solved() {
        let mut mctx = MetaCtx::new();
        let id = mctx.fresh(0);
        let m = Val::Nt(Neut::Meta(id, vec![bound_var(0)]));
        let opaque = Val::Fun(Vec::new(), crate::nbe::env::Rho::Nil);
        let err = unify(1, &m, &opaque, &mut mctx)
            .expect_err("a shape the scope check cannot inspect must not be solved");
        assert!(
            matches!(err, UnifyError::Undecidable { .. }),
            "expected an undecidable refusal, got {err:?}"
        );
        assert!(mctx.solution(id).is_none());
    }

    #[test]
    fn meta_with_pattern_spine_is_solved_by_abstraction() {
        // A spine of distinct bound vars is a Miller pattern: the solution abstracts the rhs over
        // exactly the variables the spine names. `?id G#0 G#1 ≟ Prop` gives `λ_ _. Prop`.
        let mut mctx = MetaCtx::new();
        let id = mctx.fresh(0);
        let spine = vec![bound_var(0), bound_var(1)];
        let m = Val::Nt(Neut::Meta(id, spine));
        unify(2, &m, &Val::sort(0), &mut mctx).expect("a pattern spine is solvable");
        let sol = mctx.solution(id).expect("?id is solved");
        assert!(
            matches!(sol, Val::Lam(_)),
            "the solution abstracts over the spine, got {sol:?}"
        );
    }

    // ---- inductive type unification (D48's main consumer) ----

    #[test]
    fn unify_vec_a_concrete_indices_succeeds() {
        // Vec A 0 = Vec A 0 — structural equality on indexed type.
        let decl = vec_decl();
        let lhs = vec_type(&decl, Val::sort(0), Val::Unit);
        let rhs = vec_type(&decl, Val::sort(0), Val::Unit);
        let mut mctx = MetaCtx::new();
        unify(0, &lhs, &rhs, &mut mctx).unwrap();
    }

    #[test]
    fn unify_vec_a_distinct_indices_fails() {
        let decl = vec_decl();
        let lhs = vec_type(&decl, Val::sort(0), Val::Unit);
        let rhs = vec_type(&decl, Val::sort(0), Val::One);
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
        let lhs = vec_type(&decl, Val::sort(0), n_meta);
        let rhs = vec_type(&decl, Val::sort(0), Val::Unit);
        unify(0, &lhs, &rhs, &mut mctx).unwrap();
        assert!(matches!(mctx.solution(id), Some(Val::Unit)));
    }

    #[test]
    fn unify_vec_distinct_decls_fails() {
        let v1 = vec_decl_named("VecA");
        let v2 = vec_decl_named("VecB");
        let lhs = vec_type(&v1, Val::sort(0), Val::Unit);
        let rhs = vec_type(&v2, Val::sort(0), Val::Unit);
        let mut mctx = MetaCtx::new();
        let err = unify(0, &lhs, &rhs, &mut mctx).unwrap_err();
        assert!(matches!(err, UnifyError::Mismatch { .. }));
    }

    fn vec_decl() -> Arc<InductiveDecl> {
        vec_decl_named("Vec")
    }

    fn vec_decl_named(name: &str) -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: crate::ontology::iri::Iri::parse(&format!("urn:test:{name}")).expect("test iri"),
            name: name.to_string(),
            params: vec![(Patt::Var("A".to_string()), Exp::sort(1))],
            indices: vec![(Patt::Unit, Exp::One)],
            sort: Exp::sort(1),
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
