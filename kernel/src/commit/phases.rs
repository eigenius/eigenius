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

//! The five phase functions of the commit pipeline.
//!
//! Each phase is a free function with signature
//! `fn(&mut CommitState<'_>) -> Result<PhaseControl, CommitError>`.
//! Phases read and write named fields of [`CommitState`]; the arena
//! shape is in `state.rs` and the slice plumbing is in `pipeline.rs`.
//!
//! Phase B status (D41 §3):
//!
//! - [`build`], [`structural_validate`], [`retroactive_with_cascade`],
//!   [`persist`] — implemented. The cascade port lifts today's
//!   `commit_reject_path` + `commit_cascade_path` bodies from
//!   `lattice.rs`.
//! - [`autoonload_dispatch`] — still `unimplemented!()`; Phase D
//!   ports it.
//!
//! See D41 §3 for the phase contract.

use std::sync::Arc;

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::validation::{retroactive_validate, ValidationError, Validator};

use super::pipeline::PhaseControl;
use super::state::CommitState;

// `CommitError` / `CommitPolicy` are re-exported from the lattice while
// Phase B keeps the existing enums; see `commit::mod`.
use crate::lattice::{CommitError, CommitPolicy};
use crate::observability::{field, operation};

/// Phase 3.1 — materialise the [`crate::layer::LayerBuilder`] into an
/// `Arc<Layer>` and stash it on `state.layer`.
///
/// Builds from a *clone* of `state.builder` so the original survives
/// for [`retroactive_with_cascade`]'s per-iteration rebuilds (D41 §3.3).
/// The cost is one `BTreeMap` clone + a few `Arc` bumps — negligible
/// against the validation work that dwarfs it.
///
/// Returns [`PhaseControl::SkipEmptyCommit`] if the builder is empty
/// (no resources and no tombstones) so the pipeline can short-circuit
/// to a no-op outcome without running later phases.
///
/// D41 §3.1.
pub fn build(state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    let builder = &state.builder;
    let is_empty = builder.resources().is_empty() && builder.tombstoned_iris().is_empty();
    if is_empty {
        // Phase B: callers (the lattice wrappers) never construct an
        // empty builder today — they always carry user content. We
        // still honour the contract so future RPC paths that might
        // queue an empty builder don't write a no-op layer.
        return Ok(PhaseControl::SkipEmptyCommit);
    }

    let layer = Arc::new(builder.clone().build(state.storage.clone()));
    tracing::info!(
        { field::OPERATION } = operation::COMMIT_BUILD,
        { field::LAYER_ID } = %layer.id(),
        "commit.build"
    );
    state.layer = Some(layer);
    Ok(PhaseControl::Continue)
}

/// Phase 3.2 — run `Validator::validate` against the just-built layer.
///
/// Structural check: referential integrity, type shape, constraint
/// satisfaction at the level of Decidable-QC.
///
/// Applies the policy's `max_violations` cap to the surfaced error
/// list; `total_violations` carries the full count so callers can
/// surface "showing X of Y." Under [`CommitPolicy::CascadeTombstone`]
/// the cap is bypassed (the cascade can't tombstone new-layer
/// resources, so any per-new-layer error rejects regardless of count
/// — the user wants to see all of them).
///
/// D41 §3.2.
pub fn structural_validate(state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    let layer = state
        .layer
        .as_ref()
        .expect("structural_validate runs after build; layer must be Some");

    let validator = Validator::new(Arc::clone(layer));
    let errors = validator.validate();
    if errors.is_empty() {
        tracing::info!(
            { field::OPERATION } = operation::COMMIT_STRUCTURAL_VALIDATE,
            { field::LAYER_ID } = %layer.id(),
            { field::COUNT } = 0_u64,
            "commit.structural_validate"
        );
        return Ok(PhaseControl::Continue);
    }

    let total = errors.len();
    let max = match &state.policy {
        CommitPolicy::Reject { max_violations } => *max_violations,
        // Cascade can't tombstone new-layer resources, so the
        // commit must reject. Use a generous cap so the user sees
        // every error.
        CommitPolicy::CascadeTombstone => usize::MAX,
    };
    let mut truncated = errors;
    truncated.truncate(max);
    tracing::info!(
        { field::OPERATION } = operation::COMMIT_STRUCTURAL_VALIDATE,
        { field::LAYER_ID } = %layer.id(),
        { field::COUNT } = total as u64,
        { field::ERROR_KIND } = "validation_failed",
        "commit.structural_validate.failed"
    );
    Err(CommitError::Validation {
        errors: truncated,
        total_violations: total,
    })
}

/// Phase 3.3 — fixpoint cascade against retroactive violations.
///
/// Under [`CommitPolicy::CascadeTombstone`], iterates: probe lower
/// layers for retroactive violations against the new layer's
/// declarations; tombstone the offenders; rebuild the layer; repeat
/// until stable. Under [`CommitPolicy::Reject`], fails the first time
/// a retroactive violation is found.
///
/// Emits a `COMMIT_CASCADE` event per iteration so cascade depth is
/// visible in trace output.
///
/// Ported from `commit_reject_path` + `commit_cascade_path` in
/// `lattice.rs`. The cascade phase reads / clones `state.builder` for
/// per-iteration rebuilds; the original builder is preserved across
/// iterations (it's the user's content, not the cascade's).
///
/// D41 §3.3 / Phase B.
pub fn retroactive_with_cascade(state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    let layer = state
        .layer
        .as_ref()
        .expect("retroactive_with_cascade runs after build; layer must be Some")
        .clone();

    tracing::info!(
        { field::OPERATION } = operation::COMMIT_RETROACTIVE,
        { field::LAYER_ID } = %layer.id(),
        "commit.retroactive.start"
    );

    match state.policy.clone() {
        CommitPolicy::Reject { max_violations } => reject_path(state, layer, max_violations),
        CommitPolicy::CascadeTombstone => cascade_path(state, layer),
    }
}

/// Reject path: single retroactive pass, surface violations if any.
fn reject_path(
    state: &mut CommitState<'_>,
    layer: Arc<Layer>,
    max_violations: usize,
) -> Result<PhaseControl, CommitError> {
    retroactive_validate(&layer, state.working_set).map_err(CommitError::WorkingSetExhausted)?;
    if state.working_set.violations.is_empty() {
        return Ok(PhaseControl::Continue);
    }
    let drained = state.working_set.violations.drain(max_violations);
    Err(CommitError::Validation {
        errors: drained.errors,
        total_violations: drained.total,
    })
}

/// CascadeTombstone path: fixpoint loop adding tombstones for every
/// violating lower-layer IRI until no more violations arise. Aborts
/// if any iteration would invalidate a new-layer resource.
fn cascade_path(
    state: &mut CommitState<'_>,
    initial_layer: Arc<Layer>,
) -> Result<PhaseControl, CommitError> {
    let mut current_layer = initial_layer;
    let mut iterations: u32 = 0;

    loop {
        iterations += 1;

        // Reset per-iteration state but preserve the cumulative
        // cascade_tombstones set.
        state.working_set.pending.clear();
        state.working_set.revalidated.clear();
        state.working_set.violations.clear();

        retroactive_validate(&current_layer, state.working_set)
            .map_err(CommitError::WorkingSetExhausted)?;

        tracing::info!(
            { field::OPERATION } = operation::COMMIT_CASCADE,
            { field::LAYER_ID } = %current_layer.id(),
            { field::COUNT } = iterations as u64,
            "commit.cascade.iteration"
        );

        if state.working_set.violations.is_empty() {
            break; // Fixpoint reached.
        }

        // Partition violations: those on new-layer IRIs (cascade
        // breakage — abort) vs lower-layer IRIs (tombstone candidates).
        let drained = state.working_set.violations.drain(usize::MAX);
        let new_layer_defined: std::collections::BTreeSet<Iri> =
            current_layer.defined_iris().clone();
        let mut breakage: Vec<ValidationError> = Vec::new();
        let mut new_tombs: Vec<Iri> = Vec::new();
        for err in drained.errors {
            match &err.resource_id {
                Some(iri) if new_layer_defined.contains(iri) => {
                    breakage.push(err);
                }
                Some(iri) if !state.working_set.cascade_tombstones.contains(iri) => {
                    new_tombs.push(iri.clone());
                }
                // Already cascade-tombstoned (shouldn't happen because
                // the resource would resolve to None after tombstone
                // and not surface violations), or violation without
                // resource_id (defensive — skip).
                _ => {}
            }
        }

        if !breakage.is_empty() {
            let cascade_set: std::collections::BTreeSet<Iri> =
                state.working_set.cascade_tombstones.iter().collect();
            let total = breakage.len();
            return Err(CommitError::CascadeAbort {
                iterations,
                cascade_tombstones: cascade_set,
                errors: breakage,
                total_violations: total,
            });
        }

        if new_tombs.is_empty() {
            // No progress — every violation was on an already-tombstoned
            // or unidentified resource. Treat as fixpoint to avoid
            // infinite looping; the next per-new-layer revalidation
            // below catches anything genuinely broken.
            break;
        }

        // Accumulate cascade tombstones.
        for iri in new_tombs {
            state
                .working_set
                .cascade_tombstones
                .insert(iri)
                .map_err(CommitError::WorkingSetExhausted)?;
        }

        // Rebuild the layer with the accumulated cascade tombstones
        // applied on top of the user's original builder state.
        let mut iter_builder = state.builder.clone();
        for tomb_iri in state.working_set.cascade_tombstones.iter() {
            // `tombstone` is idempotent on the underlying BTreeSet, so
            // re-adding the same IRI across iterations is a no-op. The
            // guard against tombstoning a new-layer-defined IRI is
            // handled by the breakage check above.
            iter_builder
                .tombstone(tomb_iri)
                .map_err(CommitError::Layer)?;
        }
        current_layer = Arc::new(iter_builder.build(state.storage.clone()));

        // Re-validate the new layer's own resources after the rebuild.
        // The cascade tombstones may have invalidated new-layer
        // resources that reference now-suppressed IRIs (e.g., a
        // new-layer resource's `is_a` pointed at a class the cascade
        // just tombstoned). That's new-layer breakage by another path
        // — surface as CascadeAbort.
        let validator = Validator::new(Arc::clone(&current_layer));
        let new_errs = validator.validate();
        if !new_errs.is_empty() {
            let cascade_set: std::collections::BTreeSet<Iri> =
                state.working_set.cascade_tombstones.iter().collect();
            let total = new_errs.len();
            return Err(CommitError::CascadeAbort {
                iterations,
                cascade_tombstones: cascade_set,
                errors: new_errs,
                total_violations: total,
            });
        }
    }

    // Fixpoint reached. Stash the cascade results on state and let
    // `persist` write the final layer; the orchestrator constructs the
    // outcome from `state.cascade_tombstones` / `state.cascade_iterations`.
    state.cascade_tombstones = state.working_set.cascade_tombstones.iter().collect();
    state.cascade_iterations = iterations;
    state.layer = Some(current_layer);
    Ok(PhaseControl::Continue)
}

/// Phase 3.4 — AutoOnLoad institution dispatch (D14 / D31).
///
/// For each AutoOnLoad QueryClass covering an IRI in the new layer,
/// dispatches the gate, collects `Verdict` / `RuntimeInvocation`
/// pairs into `state.provenance_resources`, and queues a
/// `verdict_provenance` emission (pipeline kind
/// [`super::pipeline::PipelineKind::StructuralFollowup`],
/// `EmissionKind::Sibling`) whenever any verdict was produced.
///
/// A `Fails` verdict returns `Err(CommitError::Validation { ... })`.
/// The emission is queued *before* the phase decides Ok vs Err — the
/// orchestrator drains it either way (see D41 §3.4 / §6.1).
///
/// Phase is absent unless `state.institutions` is `Some` — the
/// pipeline kind controls whether it runs.
///
/// D41 §3.4. **Phase D will implement; Phase B leaves
/// `unimplemented!()`.**
pub fn autoonload_dispatch(_state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    unimplemented!("phase autoonload_dispatch — Phase D / D41 §3.4")
}

/// Phase 3.5 — call [`crate::commit::persister::LayerPersister::persist`]
/// once, store the result on `state.persisted`.
///
/// The persister's body is today's `persist_layer_if_backend`:
/// anchored-commit cache probe (D33 §6) → `backend.store_layer` →
/// branch CAS. The phase does not interpret the result; the
/// orchestrator does.
///
/// Persister errors are mapped to [`CommitError::Persist`]
/// (D41 Phase B Option A — see the commit message). The lattice's
/// pre-D41 [`CommitError::Storage`] variant is reserved for direct
/// storage I/O outside the persister boundary and is unused by the
/// pipeline path.
///
/// D41 §3.5.
pub fn persist(state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    let layer = state
        .layer
        .as_ref()
        .expect("persist runs after build; layer must be Some");
    let info = state
        .persist
        .persist(state.branch, layer)
        .map_err(CommitError::Persist)?;
    tracing::info!(
        { field::OPERATION } = operation::COMMIT_PERSIST,
        { field::LAYER_ID } = %info.layer_id,
        "commit.persist"
    );
    state.persisted = Some(info);
    Ok(PhaseControl::Continue)
}
