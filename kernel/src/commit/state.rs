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

//! `CommitState` arena (one per pipeline run) and `DrainState`
//! (one per orchestrator `didDrain` stage).
//!
//! The arena is intentionally a single struct rather than per-phase
//! typed handoffs: the phase ordering is small, fixed, and known at
//! compile time, so the cost of a per-phase type chain is not worth
//! the gain. Phases that don't touch a field simply don't look at it.
//!
//! See D41 §4 (`CommitState`) and §6.5 (`DrainState`).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::institution::registry::InstitutionIndex;
use crate::institution::runtime::InstitutionRuntime;
use crate::layer::{Layer, LayerBuilder, LayerStorage};
use crate::ontology::{Iri, Resource};
use crate::validation::{CommitWorkingSet, ValidationError};

use super::hooks::CommitHookHost;
use super::outcome::{DispatchEntry, LayerEmission};
use super::persister::{LayerPersister, PersistedLayerInfo};

// `CommitPolicy` is re-exported from `lattice` via `super` so that
// phases / state name it without referencing a different module.
use crate::lattice::CommitPolicy;

/// Borrowed handle on the institution runtime + dispatch index used by
/// the `autoonload_dispatch` phase and the `rebuild_institution_index`
/// `didDrain` hook.
///
/// `InstitutionContext` exists so the pipeline can be parameterised on
/// "institutional" pipelines (`with_institutions`) without each pipeline
/// kind defining a different `CommitState` shape. For non-institutional
/// pipelines the field is `None` and the `autoonload_dispatch` phase is
/// simply absent from the phase slice.
///
/// Phase A defines the minimum: shared handles on the dispatch index
/// and the runtime. Phase D may widen this to include the institution
/// classes accumulator currently threaded through the Load handler.
pub struct InstitutionContext<'a> {
    /// Shared dispatch index snapshot. `Arc` so phase / hook code can
    /// clone cheaply when they need to outlive the borrow.
    pub index: Arc<InstitutionIndex>,
    /// Shared institution runtime (backend dispatch, gate registrations).
    pub runtime: Arc<InstitutionRuntime>,
    /// Lifetime parameter pin: ensures the context can't outlive the
    /// orchestrator borrow that produced it.
    pub _marker: std::marker::PhantomData<&'a ()>,
}

/// Per-pipeline-run mutable arena.
///
/// Fields split into four groups (D41 §4):
///
/// - **Inputs.** Set at orchestrator entry; phases read only.
/// - **Transient.** Heavily rewritten by `build` /
///   `retroactive_with_cascade`.
/// - **Accumulators.** Append-only across the pipeline run; read once
///   at outcome construction.
/// - **Persist result.** Set exactly once by `persist`; inspected by
///   the orchestrator for the drain / revert decision.
///
/// The arena is per-pipeline-run; `working_set` is the only borrowed
/// mutable that survives across pipeline runs in one orchestrator call
/// (re-used to amortise allocation across user / provenance /
/// institution-classes layers).
pub struct CommitState<'a> {
    // --- Inputs (read by phases; set once at orchestrator entry) ---
    /// Shared layer storage view.
    pub storage: LayerStorage,
    /// Persist seam. Phase `persist` is its only caller.
    pub persist: &'a dyn LayerPersister,
    /// Host seam — used by `didPersist` hooks (today the vector sweep)
    /// to delegate kernel-side registrations / index rebuilds back into
    /// `EigeniusService`.
    /// Threaded into the state by `PipelineConfig`; identical for
    /// every pipeline run within one orchestrator invocation. Phase D.
    pub host: &'a dyn CommitHookHost,
    /// Global commit policy for this orchestrator run. Today the same
    /// policy threads through every pipeline run; per-phase policies
    /// are future work (D41 §13.2).
    pub policy: CommitPolicy,
    /// Branch name for this commit.
    pub branch: &'a str,
    /// `Some` for `with_institutions` pipelines; `None` otherwise.
    pub institutions: Option<InstitutionContext<'a>>,

    // --- Transient (rewritten across cascade iterations / phases) ---
    /// The current builder. `build` consumes it into `layer`;
    /// `retroactive_with_cascade` may rebuild from a clone if it has
    /// to add cascade tombstones.
    pub builder: LayerBuilder,
    /// The materialised layer once `build` has run. `None` before
    /// `build`; `Some` thereafter for all remaining phases.
    pub layer: Option<Arc<Layer>>,

    // --- Accumulators (read once at outcome construction) ---
    /// IRIs the cascade tombstoned beyond the caller's builder-level
    /// tombstones. Always empty for pipelines without
    /// `retroactive_with_cascade`.
    pub cascade_tombstones: BTreeSet<Iri>,
    /// Cascade fixpoint iteration count.
    pub cascade_iterations: u32,
    /// Per-subject institution dispatch readings.
    pub dispatched_verdicts: Vec<DispatchEntry>,
    /// Resources produced by AutoOnLoad dispatch (the verdict /
    /// runtime-invocation pairs from D14 / D31). Drained by the
    /// `autoonload_dispatch` phase into a follow-up
    /// `verdict_provenance` emission.
    pub provenance_resources: Vec<Resource>,
    /// Follow-up emissions queued by phases / `didPersist` hooks.
    pub emissions: Vec<LayerEmission>,
    /// Non-unwinding errors raised by `didPersist` hooks. Flows into
    /// `LayerCommitOutcome.hook_errors`.
    pub hook_errors: Vec<ValidationError>,

    // --- Working buffers (borrowed; not owned) ---
    /// Pooled commit working set, re-used across pipeline runs in the
    /// same orchestrator call.
    pub working_set: &'a mut CommitWorkingSet,

    // --- Persist result (set exactly once by `persist`) ---
    /// Populated by the `persist` phase. The orchestrator reads
    /// `persisted.as_ref().map(|i| i.branch_advanced)` to decide whether
    /// to drain emissions, run `didPersist` hooks, or revert head.
    pub persisted: Option<PersistedLayerInfo>,
}

/// State the `didDrain` hook stage operates against.
///
/// Constructed by the orchestrator after the drain loop exits, once
/// the final top layer is known. `top_layer` is `Some(_)` for the
/// most recent successfully-advanced layer (the "current head" after
/// the drain), or `None` if no layer landed during the drain (e.g.,
/// the root emission's persist failed with no Sibling rescue).
///
/// `DrainState` deliberately does **not** expose `state.emissions` —
/// by the time `didDrain` runs, no further pipelines will execute and
/// queuing more work would be silently dropped. See D41 §6.5.
pub struct DrainState<'a> {
    /// Final top of branch after the drain. `None` iff no layer landed.
    pub top_layer: Option<Arc<Layer>>,
    /// Host seam — used by `didDrain` hooks (today
    /// `rebuild_institution_index`) to delegate kernel-side state
    /// updates back into `EigeniusService`. Phase D.
    pub host: &'a dyn CommitHookHost,
    /// Errors collected from `didDrain` hooks across this orchestrator
    /// run. The orchestrator copies this into
    /// `MultiLayerOutcome.drain_hook_errors` when constructing the
    /// final outcome.
    pub hook_errors: Vec<ValidationError>,
    /// Lifetime parameter pin.
    pub _marker: std::marker::PhantomData<&'a ()>,
}
