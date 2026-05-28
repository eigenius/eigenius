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
//! Phase A: signatures only. All bodies are
//! `unimplemented!("phase X")`. The canned pipeline slices in
//! `pipeline.rs` reference these function items directly; they
//! compile even with `unimplemented!()` bodies.
//!
//! See D41 §3 for the phase contract.

use super::pipeline::PhaseControl;
use super::state::CommitState;

// `CommitError` is re-exported from the lattice while Phase A keeps
// the existing enum shape; see `commit::mod`.
use crate::lattice::CommitError;

/// Phase 3.1 — materialise the [`crate::layer::LayerBuilder`] into an
/// `Arc<Layer>` and stash it on `state.layer`.
///
/// Returns [`PhaseControl::SkipEmptyCommit`] if the builder is empty
/// so the pipeline can short-circuit to a no-op outcome without
/// running later phases.
///
/// D41 §3.1.
pub fn build(_state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    unimplemented!("phase build")
}

/// Phase 3.2 — run `Validator::validate` against the just-built layer.
///
/// Structural check: referential integrity, type shape, constraint
/// satisfaction at the level of Decidable-QC.
///
/// D41 §3.2.
pub fn structural_validate(_state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    unimplemented!("phase structural_validate")
}

/// Phase 3.3 — fixpoint cascade against retroactive violations.
///
/// Under `CommitPolicy::CascadeTombstone`, iterates: probe lower
/// layers for retroactive violations against the new layer's
/// declarations; tombstone the offenders; rebuild the layer; repeat
/// until stable. Under `CommitPolicy::Reject`, fails the first time
/// a retroactive violation is found.
///
/// Emits a `COMMIT_CASCADE` event per iteration so cascade depth is
/// visible in trace output.
///
/// D41 §3.3.
pub fn retroactive_with_cascade(_state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    unimplemented!("phase retroactive_with_cascade")
}

/// Phase 3.4 — AutoOnLoad institution dispatch (D14 / D31).
///
/// For each AutoOnLoad QueryClass covering an IRI in the new layer,
/// dispatches the gate, collects `Verdict` / `RuntimeInvocation`
/// pairs into `state.provenance_resources`, and queues a
/// `verdict_provenance` emission (pipeline kind
/// [`super::pipeline::PipelineKind::StructuralFollowup`]) if any
/// readings land on a Holds / Undecidable path.
///
/// A `Fails` verdict short-circuits with
/// `CommitError::ValidationFailed { provenance_layer: ... }`,
/// mirroring today's shape from `commit_with_validation`.
///
/// Phase is absent unless `state.institutions` is `Some` — the
/// pipeline kind controls whether it runs.
///
/// D41 §3.4.
pub fn autoonload_dispatch(_state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    unimplemented!("phase autoonload_dispatch")
}

/// Phase 3.5 — call [`crate::commit::persister::LayerPersister::persist`]
/// once, store the result on `state.persisted`.
///
/// The persister's body is today's `persist_layer_if_backend`:
/// anchored-commit cache probe (D33 §6) → `backend.store_layer` →
/// branch CAS. The phase does not interpret the result; the
/// orchestrator does.
///
/// D41 §3.5.
pub fn persist(_state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    unimplemented!("phase persist")
}
