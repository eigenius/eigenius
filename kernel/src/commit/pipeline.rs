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

//! `CommitPipeline` — one-layer pipeline of phases + `didPersist`
//! hooks plus the four canned shapes:
//!
//! - [`CommitPipeline::structural_only`] — build, structural_validate, persist.
//! - [`CommitPipeline::with_retroactive`] — + retroactive_with_cascade.
//! - [`CommitPipeline::with_institutions`] — + autoonload_dispatch;
//!   `didPersist`: `register_wasm_components`.
//! - [`CommitPipeline::structural_followup`] — same phases as
//!   `structural_only`; kept distinct as a name so call sites
//!   document intent and so the orchestrator can evolve followup
//!   pipelines differently in the future (D41 §5).
//!
//! Phases are stored as `&'static [Phase]` slices — zero allocation,
//! data-driven. The function items are defined in `phases.rs` and
//! referenced from the static slices at the bottom of this module.
//! Phase A: the `run` body is `unimplemented!("phase A scaffolding;
//! see d41 §5/§6")`.
//!
//! See D41 §2, §5, §6.

use std::collections::BTreeSet;

use crate::lattice::{CommitError, CommitPolicy};
use crate::layer::LayerBuilder;
use crate::observability::{field, operation};
use crate::validation::CommitWorkingSet;

use super::hooks::{register_wasm_components, DidPersistHook};
use super::outcome::LayerCommitOutcome;
use super::persister::LayerPersister;
use super::phases::{
    autoonload_dispatch, build, persist, retroactive_with_cascade, structural_validate,
};
use super::state::{CommitState, InstitutionContext};

/// Phase function signature. See `phases.rs` for the five concrete
/// phases and D41 §3 for the contract.
pub type Phase = fn(&mut super::state::CommitState<'_>) -> Result<PhaseControl, CommitError>;

/// Per-phase control flow.
///
/// `Continue` is the happy path: the next phase runs.
/// `SkipEmptyCommit` short-circuits the pipeline — the builder was
/// empty (no resources, no tombstones), so the run returns a no-op
/// outcome without invoking later phases. Distinguished from
/// `Continue` so callers can tell "we ran but the layer was a no-op"
/// apart from "we ran and landed a layer."
#[derive(Debug, Clone, Copy)]
pub enum PhaseControl {
    /// Run the next phase.
    Continue,
    /// Builder was empty; skip the rest of the pipeline and return a
    /// `Skipped` outcome.
    SkipEmptyCommit,
}

/// Which canned [`CommitPipeline`] an emission should run through.
///
/// Used both on [`super::outcome::LayerEmission::pipeline`] (the
/// orchestrator looks up the canned pipeline from the kind) and on
/// the per-RPC root-emission mapping in D41 §10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineKind {
    /// `build`, `structural_validate`, `persist`.
    StructuralOnly,
    /// `build`, `structural_validate`, `retroactive_with_cascade`,
    /// `persist`.
    WithRetroactive,
    /// `build`, `structural_validate`, `retroactive_with_cascade`,
    /// `autoonload_dispatch`, `persist`; `didPersist`:
    /// `register_wasm_components`.
    WithInstitutions,
    /// Same phase list as `StructuralOnly`; kept distinct so
    /// followup-layer call sites document intent and so the
    /// orchestrator can evolve followups differently in the future.
    StructuralFollowup,
}

/// Inputs to a pipeline run that vary per orchestrator invocation
/// but stay constant across pipeline runs in one orchestrator call.
///
/// The pipeline's `run` constructs a fresh `CommitState` from a
/// `PipelineConfig`, plus the per-emission `LayerBuilder` and a
/// mutable borrow on the pooled `CommitWorkingSet`.
///
/// D41 §5.
pub struct PipelineConfig<'a> {
    /// Persist seam used by the `persist` phase.
    pub persister: &'a dyn LayerPersister,
    /// Branch name for this commit.
    pub branch: &'a str,
    /// Global commit policy for the run.
    pub policy: CommitPolicy,
    /// `Some` for `with_institutions` pipelines; `None` otherwise.
    pub institutions: Option<InstitutionContext<'a>>,
    /// Shared layer storage view, threaded into `CommitState.storage`.
    pub storage: crate::layer::LayerStorage,
}

/// One-layer commit pipeline.
///
/// Holds two static slices — phases and `didPersist` hooks — plus the
/// [`PipelineKind`] this pipeline corresponds to. The slices are
/// `&'static` so a [`CommitPipeline`] is zero-allocation and the
/// canned constructors are `const fn`.
///
/// D41 §2.1, §5.
#[derive(Debug, Clone, Copy)]
pub struct CommitPipeline {
    /// Which canned shape this is.
    pub kind: PipelineKind,
    /// Phase slice. Run in order; abort on the first `Err`.
    pub phases: &'static [Phase],
    /// `didPersist` hooks. Run after a successful persist iff
    /// `persist` set `branch_advanced = true`.
    pub did_persist: &'static [DidPersistHook],
}

impl CommitPipeline {
    /// `build`, `structural_validate`, `persist`.
    pub const fn structural_only() -> Self {
        Self {
            kind: PipelineKind::StructuralOnly,
            phases: STRUCTURAL_ONLY_PHASES,
            did_persist: NO_DID_PERSIST,
        }
    }

    /// `build`, `structural_validate`, `retroactive_with_cascade`,
    /// `persist`.
    pub const fn with_retroactive() -> Self {
        Self {
            kind: PipelineKind::WithRetroactive,
            phases: WITH_RETROACTIVE_PHASES,
            did_persist: NO_DID_PERSIST,
        }
    }

    /// `build`, `structural_validate`, `retroactive_with_cascade`,
    /// `autoonload_dispatch`, `persist`; `didPersist`:
    /// `register_wasm_components`.
    pub const fn with_institutions() -> Self {
        Self {
            kind: PipelineKind::WithInstitutions,
            phases: WITH_INSTITUTIONS_PHASES,
            did_persist: WITH_INSTITUTIONS_DID_PERSIST,
        }
    }

    /// Same phase list as [`Self::structural_only`]; distinct name
    /// for follow-up layers (`verdict_provenance`, `institution_classes`).
    pub const fn structural_followup() -> Self {
        Self {
            kind: PipelineKind::StructuralFollowup,
            phases: STRUCTURAL_FOLLOWUP_PHASES,
            did_persist: NO_DID_PERSIST,
        }
    }

    /// Look up the canned pipeline for a [`PipelineKind`].
    ///
    /// The orchestrator calls this once per drained emission.
    pub const fn for_kind(kind: PipelineKind) -> Self {
        match kind {
            PipelineKind::StructuralOnly => Self::structural_only(),
            PipelineKind::WithRetroactive => Self::with_retroactive(),
            PipelineKind::WithInstitutions => Self::with_institutions(),
            PipelineKind::StructuralFollowup => Self::structural_followup(),
        }
    }

    /// Execute the pipeline against `builder`.
    ///
    /// Constructs a fresh [`super::state::CommitState`], opens a
    /// `COMMIT_PIPELINE_RUN` span, walks `phases`, runs `did_persist`
    /// under a `COMMIT_DID_PERSIST` span iff the `persist` phase set
    /// `branch_advanced = true`, and constructs the
    /// [`LayerCommitOutcome`] from the accumulators.
    ///
    /// **Phase B status (D41 §5).** This implementation services the
    /// lattice wrapper path only — single-pipeline, no orchestrator.
    /// The full `PipelineRunErr` / `sibling_emissions` plumbing
    /// described in the doc is Phase D work; the body returns
    /// `Result<LayerCommitOutcome, CommitError>` directly so the
    /// `with_retroactive` callers (the lattice wrappers in this
    /// commit) can use it unchanged. Phase D will widen the return
    /// type once the orchestrator needs to partition emissions on Err.
    ///
    /// `did_persist` hooks dispatch as a list iff the persist phase
    /// reported `branch_advanced = true`; for `with_retroactive` the
    /// slice is empty so the loop body never executes. Phase B leaves
    /// hook *bodies* `unimplemented!()`; the dispatch site is wired so
    /// Phase D only has to fill the bodies.
    pub fn run(
        &self,
        builder: LayerBuilder,
        cfg: PipelineConfig<'_>,
        ws: &mut CommitWorkingSet,
    ) -> Result<LayerCommitOutcome, CommitError> {
        let span = tracing::info_span!(operation::COMMIT_PIPELINE_RUN, kind = ?self.kind);
        let _enter = span.enter();

        let mut state = CommitState {
            // Inputs
            storage: cfg.storage,
            persist: cfg.persister,
            policy: cfg.policy,
            branch: cfg.branch,
            institutions: cfg.institutions,

            // Transient
            builder,
            layer: None,

            // Accumulators
            cascade_tombstones: BTreeSet::new(),
            cascade_iterations: 0,
            dispatched_verdicts: Vec::new(),
            provenance_resources: Vec::new(),
            emissions: Vec::new(),
            hook_errors: Vec::new(),

            // Working buffers
            working_set: ws,

            // Persist result
            persisted: None,
        };

        // Walk phases. The first `Err` aborts the rest of the walk;
        // didPersist hooks are not run.
        for phase in self.phases {
            match phase(&mut state)? {
                PhaseControl::Continue => {}
                PhaseControl::SkipEmptyCommit => {
                    // Phase B: only `build` returns this. The lattice
                    // wrappers never produce an empty builder today,
                    // so this branch is exercised by future RPC paths
                    // only. The brief calls for returning a `Skipped`
                    // outcome; today's `LayerCommitOutcome` is not
                    // shaped for that case (it requires `layer` +
                    // `persist`). Treat empty-commit as a programming
                    // error from the Phase B caller and surface it
                    // via an explicit panic — Phase D will widen the
                    // outcome shape to carry `Skipped`.
                    unreachable!(
                        "SkipEmptyCommit returned from `build`, but the Phase B \
                         caller never queues empty builders; LayerCommitOutcome \
                         has no `Skipped` shape yet (Phase D)."
                    );
                }
            }
        }

        // didPersist hooks. Skip when persist didn't advance the
        // branch — there's no successfully-persisted layer to hook
        // off (D41 §3.6 / §6.1). For the lattice wrappers'
        // `BackendStorePersister`, `branch_advanced` is always
        // `false` *and* the `with_retroactive` slice is empty, so
        // this loop is unreachable on that path; the structure is in
        // place for Phase D's `with_institutions` flow.
        let branch_advanced = state
            .persisted
            .as_ref()
            .map(|i| i.branch_advanced)
            .unwrap_or(false);
        if branch_advanced && !self.did_persist.is_empty() {
            let hook_span = tracing::info_span!(operation::COMMIT_DID_PERSIST);
            let _hook_enter = hook_span.enter();
            for hook in self.did_persist {
                let outcome = hook(&mut state);
                state.hook_errors.extend(outcome.errors);
            }
        }

        // Construct the LayerCommitOutcome. `persist` is required at
        // this point: every canned pipeline ends with the `persist`
        // phase, so `state.persisted` must be `Some`.
        let layer = state
            .layer
            .expect("build phase populated layer; pipeline ran to persist");
        let persist_info = state
            .persisted
            .expect("persist phase populated state.persisted on Ok");
        tracing::info!(
            { field::OPERATION } = operation::COMMIT_PIPELINE_RUN,
            { field::LAYER_ID } = %layer.id(),
            "commit.pipeline_run.ok"
        );
        Ok(LayerCommitOutcome {
            layer,
            persist: persist_info,
            cascade_tombstones: state.cascade_tombstones,
            cascade_iterations: state.cascade_iterations,
            dispatched_verdicts: state.dispatched_verdicts,
            emissions: state.emissions,
            hook_errors: state.hook_errors,
        })
    }
}

// -------------------------------------------------------------------
// Static phase / hook slices for the four canned pipelines.
//
// These are at file scope so the canned `const fn` constructors can
// reference them. Function items (`build`, `structural_validate`, ...)
// have a stable address that can populate a `&'static [Phase]` slice
// even though the bodies are `unimplemented!()` — calling them will
// trap, but defining the slices is sound and lets the rest of the
// pipeline machinery compile cleanly during Phase A.
// -------------------------------------------------------------------

/// `structural_only` phase slice — D41 §5.
static STRUCTURAL_ONLY_PHASES: &[Phase] = &[build, structural_validate, persist];

/// `with_retroactive` phase slice — D41 §5.
static WITH_RETROACTIVE_PHASES: &[Phase] = &[
    build,
    structural_validate,
    retroactive_with_cascade,
    persist,
];

/// `with_institutions` phase slice — D41 §5.
static WITH_INSTITUTIONS_PHASES: &[Phase] = &[
    build,
    structural_validate,
    retroactive_with_cascade,
    autoonload_dispatch,
    persist,
];

/// `structural_followup` phase slice — D41 §5.
static STRUCTURAL_FOLLOWUP_PHASES: &[Phase] = &[build, structural_validate, persist];

/// `with_institutions` `didPersist` slice — D41 §3.6 / §5.
static WITH_INSTITUTIONS_DID_PERSIST: &[DidPersistHook] = &[register_wasm_components];

/// Empty `didPersist` slice shared by pipelines without post-persist
/// hooks.
static NO_DID_PERSIST: &[DidPersistHook] = &[];
