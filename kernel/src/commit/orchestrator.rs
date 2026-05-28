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

//! `CommitOrchestrator` — multi-layer FIFO drain loop and
//! post-drain `didDrain` hook stage.
//!
//! Every commit-shaped RPC goes `handler → orchestrator → pipeline`,
//! including the single-layer ones. A `Query INTO` with no
//! emissions runs through the orchestrator as the degenerate case
//! (one pipeline run, empty emission queue, returns immediately).
//! This keeps the handler shape uniform: build a root
//! [`super::outcome::LayerEmission`] from the RPC inputs, call
//! `orchestrator.run(root)`, translate the
//! [`super::outcome::MultiLayerOutcome`] back into RPC response
//! fields.
//!
//! The orchestrator owns:
//! - the FIFO `(depth, LayerEmission)` queue,
//! - the `MAX_EMISSION_DEPTH = 4` cap (D41 §6.3),
//! - the revert-to-`last_advanced` head bookkeeping (D41 §6.4),
//! - the `didDrain` hook stage (D41 §6.5).
//!
//! Phase A: `run` is `unimplemented!("phase A scaffolding; see d41
//! §6")`.

use crate::context::ExecutionContext;
use crate::lattice::{CommitError, CommitPolicy};
use crate::validation::CommitWorkingSetPool;

use super::hooks::{rebuild_institution_index, DidDrainHook};
use super::outcome::{LayerEmission, MultiLayerOutcome};
use super::persister::LayerPersister;
use super::state::InstitutionContext;

/// Static safety net: a phase or hook that produced emissions
/// transitively past this depth aborts the orchestrator with
/// [`CommitError`]. Today the maximum depth is 1 (Load emits
/// `verdict_provenance` and `institution_classes`); 4 leaves room
/// for two follow-up generations beyond that.
///
/// D41 §6.3.
pub const MAX_EMISSION_DEPTH: u32 = 4;

/// Multi-layer commit orchestrator.
///
/// Borrows the execution context, working-set pool, persister, and
/// (optionally) institution context for the duration of one
/// `run(root)` invocation. The orchestrator constructs one
/// `CommitPipeline` per drained emission (looked up via
/// `CommitPipeline::for_kind`) and one `CommitState` per pipeline
/// run; the working set is re-used across pipeline runs to amortise
/// allocation.
///
/// D41 §6.
pub struct CommitOrchestrator<'a> {
    /// Execution context being driven. The orchestrator advances and
    /// reverts `ctx.head` as pipeline runs land or fail to land.
    pub ctx: &'a mut ExecutionContext,
    /// Per-server pool. The orchestrator acquires one
    /// [`crate::validation::CommitWorkingSet`] for the entire drain.
    pub pool: &'a CommitWorkingSetPool,
    /// Persist seam threaded into every `CommitState`.
    pub persister: &'a dyn LayerPersister,
    /// Branch name for this orchestrator run.
    pub branch: &'a str,
    /// Global commit policy.
    pub policy: CommitPolicy,
    /// Borrowed institution context for `with_institutions` pipelines.
    pub institutions: Option<InstitutionContext<'a>>,
    /// `didDrain` hooks. The canonical orchestrator includes
    /// [`rebuild_institution_index`] — see [`Self::default_did_drain`].
    pub did_drain: &'static [DidDrainHook],
}

impl<'a> CommitOrchestrator<'a> {
    /// Default `didDrain` hook list: a single
    /// [`rebuild_institution_index`] hook. Phase C will wire this
    /// from `EigeniusService::commit_orchestrator`; for Phase A the
    /// slice is exposed as an associated function so callers don't
    /// hand-roll their own.
    ///
    /// D41 §6.5.
    pub const fn default_did_drain() -> &'static [DidDrainHook] {
        DEFAULT_DID_DRAIN
    }

    /// Run the FIFO drain starting from `root`.
    ///
    /// See the pseudocode in D41 §6.1. The body opens a
    /// `COMMIT_ORCHESTRATOR_RUN` span, drains emissions in order,
    /// runs `did_drain` hooks under a `COMMIT_DID_DRAIN` span after
    /// the queue is empty, and returns the accumulated
    /// [`MultiLayerOutcome`].
    pub fn run(self, _root: LayerEmission) -> Result<MultiLayerOutcome, CommitError> {
        unimplemented!("phase A scaffolding; see d41 §6")
    }
}

/// Default `didDrain` slice: `rebuild_institution_index` only.
static DEFAULT_DID_DRAIN: &[DidDrainHook] = &[rebuild_institution_index];
