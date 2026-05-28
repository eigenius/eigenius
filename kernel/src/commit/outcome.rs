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

//! Pipeline / orchestrator outcome shapes.
//!
//! - [`LayerCommitOutcome`] — one per pipeline run, returned by
//!   `CommitPipeline::run`.
//! - [`MultiLayerOutcome`] — one per orchestrator run, returned by
//!   `CommitOrchestrator::run`. Carries the per-layer outcomes plus any
//!   `didDrain` hook errors.
//! - [`LayerEmission`] — the unit of work the orchestrator drains. The
//!   root emission represents the RPC's primary layer; phases / hooks
//!   may queue further emissions as follow-up layers (verdict
//!   provenance, institution-classes, etc).
//! - [`DispatchEntry`] — one institution dispatch reading. Internal
//!   shape; the design doc deliberately leaves this open. Phase A
//!   provides a minimal record carrying the subject IRI, the queried
//!   QueryClass IRI, and the [`crate::institution::dispatch::VerdictReading`].
//!
//! See D41 §3 (`HookOutcome` is in `hooks.rs`), §4 (`CommitState`),
//! §6 (`LayerEmission`), §11 (`LayerCommitOutcome`).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::institution::dispatch::VerdictReading;
use crate::layer::Layer;
use crate::ontology::{Iri, Resource};
use crate::validation::ValidationError;

use super::persister::PersistedLayerInfo;
use super::pipeline::PipelineKind;

/// Outcome of a single `CommitPipeline::run`.
///
/// One per layer landed (or attempted) in an orchestrator run. The
/// orchestrator's [`MultiLayerOutcome`] is a `Vec<LayerCommitOutcome>`
/// plus drain-hook accumulators.
///
/// D41 §4 / §11.
#[derive(Debug)]
pub struct LayerCommitOutcome {
    /// The layer the `build` phase materialised. Identical across the
    /// outcome for cache-hit and CAS-loss paths — those are reflected
    /// in `persist.branch_advanced`, not by a different `layer`.
    pub layer: Arc<Layer>,
    /// Result of the `persist` phase. Drives the orchestrator's
    /// drain / revert decision and the `didPersist` hook gate.
    pub persist: PersistedLayerInfo,
    /// IRIs the cascade tombstoned beyond the caller's builder-level
    /// tombstones. Always empty for pipelines without
    /// `retroactive_with_cascade`.
    pub cascade_tombstones: BTreeSet<Iri>,
    /// Number of cascade fixpoint iterations. `0` if the phase didn't
    /// run or found no retroactive violations.
    pub cascade_iterations: u32,
    /// Per-subject institution dispatch readings collected by
    /// `autoonload_dispatch`. Empty for pipelines without that phase.
    pub dispatched_verdicts: Vec<DispatchEntry>,
    /// Follow-up emissions queued by phases / `didPersist` hooks. The
    /// orchestrator drains these in FIFO order; see D41 §6.2.
    pub emissions: Vec<LayerEmission>,
    /// Non-unwinding errors raised by `didPersist` hooks. The commit
    /// stands — the layer is on disk — but callers can surface these.
    /// See D41 §3.6.
    pub hook_errors: Vec<ValidationError>,
}

/// Outcome of an orchestrator drain.
///
/// `layers` holds one [`LayerCommitOutcome`] per pipeline run, in the
/// order they drained (root → user-emitted children → hook-emitted
/// children, FIFO). `drain_hook_errors` collects non-unwinding errors
/// raised by `didDrain` hooks; see D41 §6.5.
#[derive(Debug)]
pub struct MultiLayerOutcome {
    /// Per-layer outcomes, in drain order.
    pub layers: Vec<LayerCommitOutcome>,
    /// Non-unwinding errors from `didDrain` hooks. Surfaced to the
    /// caller; all layers in `layers` are durably on disk regardless.
    pub drain_hook_errors: Vec<ValidationError>,
}

/// The unit of work the orchestrator drains.
///
/// The root emission represents the RPC's primary layer; phases /
/// hooks may push additional emissions onto `state.emissions` for
/// follow-up layers (verdict provenance after AutoOnLoad dispatch,
/// institution-classes after WASM-component registration, etc.).
///
/// `name` is a stable, static string used both for diagnostics and as
/// the `LayerBuilder` name when the orchestrator constructs a builder
/// for this emission. See D41 §6.
#[derive(Debug)]
pub struct LayerEmission {
    /// Stable, diagnostic name (`"user"`, `"verdict_provenance"`,
    /// `"institution_classes"`, ...). Also used as the builder name.
    pub name: &'static str,
    /// Which canned pipeline to run on this emission.
    pub pipeline: PipelineKind,
    /// Resources to add to the emission's `LayerBuilder`. Followup
    /// emissions populate this from phase / hook output; the root
    /// emission populates it from the RPC request.
    pub resources: Vec<Resource>,
    /// Tombstones to apply to the emission's `LayerBuilder`. Followup
    /// emissions populate this from phase / hook output; the root
    /// emission populates it from the RPC's explicit tombstones
    /// (D41 §10.1).
    pub tombstones: BTreeSet<Iri>,
}

/// One institution dispatch reading collected by the
/// `autoonload_dispatch` phase.
///
/// The design doc deliberately leaves the interior shape open
/// (D41 §10 lists this as something the handler translates into
/// the response). Phase A picks a minimal record matching how the
/// handler already surfaces dispatch outcomes. Phase B / D may widen
/// it (e.g. to carry runtime invocation provenance) without breaking
/// the pipeline contract.
#[derive(Debug, Clone)]
pub struct DispatchEntry {
    /// IRI of the resource the gate was evaluated against. `None` only
    /// for gates that target whole-layer predicates (none exist today).
    pub subject_iri: Option<Iri>,
    /// IRI of the QueryClass that produced this reading. Stored as a
    /// `String` because the dispatch surface today (`InstitutionContext`
    /// snapshots) keeps it as a string; Phase B will move to `Iri` if
    /// the dispatch surface migrates.
    pub query_class_iri: String,
    /// Reading off the dispatch result resource.
    pub verdict: VerdictReading,
}
