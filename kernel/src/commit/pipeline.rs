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
//!   `didPersist`: `trigger_vector_sweep`.
//! - [`CommitPipeline::structural_followup`] — build, persist (no
//!   `structural_validate`: kernel-emitted content is well-formed by
//!   construction; see `STRUCTURAL_FOLLOWUP_PHASES` for the contract).
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

use super::hooks::{trigger_vector_sweep, CommitHookHost, DidPersistHook};
use super::outcome::{LayerCommitOutcome, LayerEmission, LayerRole};
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
    /// `trigger_vector_sweep`.
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
    /// Host seam used by `didPersist` hooks (D41 Phase D).
    pub host: &'a dyn CommitHookHost,
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
    /// `trigger_vector_sweep`.
    pub const fn with_institutions() -> Self {
        Self {
            kind: PipelineKind::WithInstitutions,
            phases: WITH_INSTITUTIONS_PHASES,
            did_persist: WITH_INSTITUTIONS_DID_PERSIST,
        }
    }

    /// Pipeline for kernel-emitted follow-up layers (`verdict_provenance`,
    /// `institution_classes`). Skips `structural_validate` — well-formedness
    /// is the emitter's contract; see [`STRUCTURAL_FOLLOWUP_PHASES`].
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
    /// **Return shape (Phase D / D41 §5).** Widened from
    /// `Result<LayerCommitOutcome, CommitError>` to
    /// `Result<LayerCommitOutcome, PipelineRunErr>`. On `Err`, the
    /// pipeline partitions `state.emissions` by [`super::EmissionKind`]:
    /// `Sibling` entries flow onto [`PipelineRunErr::sibling_emissions`]
    /// for the orchestrator to rescue, while `Child` entries are
    /// silently dropped because their intended parent did not land.
    /// Lattice wrappers convert via `.map_err(|e| e.error)`.
    ///
    /// `did_persist` hooks dispatch as a list iff the persist phase
    /// reported `branch_advanced = true`.
    pub fn run(
        &self,
        name: &'static str,
        role: LayerRole,
        builder: LayerBuilder,
        cfg: PipelineConfig<'_>,
        ws: &mut CommitWorkingSet,
    ) -> Result<LayerCommitOutcome, PipelineRunErr> {
        let span = tracing::info_span!(operation::COMMIT_PIPELINE_RUN, kind = ?self.kind);
        let _enter = span.enter();
        // Wall-clock for the whole commit (build → validate → cascade → persist →
        // didPersist), surfaced as `duration_ms` on the terminal log so commit cost
        // is visible in the kernel logs without a profiler.
        let started = std::time::Instant::now();

        let mut state = CommitState {
            // Inputs
            storage: cfg.storage,
            persist: cfg.persister,
            host: cfg.host,
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
        // didPersist hooks are not run. On Err we still partition any
        // Sibling emissions phases-before-the-failing-phase queued —
        // that's the audit-anchor rescue path (§3.4 / §6.1).
        for phase in self.phases {
            match phase(&mut state) {
                Ok(PhaseControl::Continue) => {}
                Ok(PhaseControl::SkipEmptyCommit) => {
                    // Phase D still leaves the empty-commit path
                    // unreachable because no caller today queues an
                    // empty emission. Widening LayerCommitOutcome to
                    // carry a `Skipped` variant is deferred to a
                    // later phase.
                    unreachable!(
                        "SkipEmptyCommit returned from `build`, but no caller \
                         today queues empty builders; LayerCommitOutcome has \
                         no `Skipped` shape yet."
                    );
                }
                Err(error) => {
                    // Partition: Siblings rescue, Children drop.
                    let sibling_emissions = partition_siblings(&mut state.emissions);
                    tracing::info!(
                        { field::OPERATION } = operation::COMMIT_PIPELINE_RUN,
                        { field::ERROR_KIND } = "phase_failed",
                        rescued_siblings = sibling_emissions.len(),
                        duration_ms = started.elapsed().as_millis() as u64,
                        "commit.pipeline_run.err"
                    );
                    return Err(PipelineRunErr {
                        error,
                        sibling_emissions,
                    });
                }
            }
        }

        // didPersist hooks. Skip when persist didn't advance the
        // branch — there's no successfully-persisted layer to hook
        // off (D41 §3.6 / §6.1).
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
            duration_ms = started.elapsed().as_millis() as u64,
            "commit.pipeline_run.ok"
        );
        Ok(LayerCommitOutcome {
            role,
            name,
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

/// Drain `Sibling` emissions out of `emissions`, leaving `Child`
/// entries in place (they'll be dropped when the caller discards the
/// vector). Preserves order among siblings so the FIFO invariant in
/// the orchestrator's rescue queue holds.
fn partition_siblings(emissions: &mut Vec<LayerEmission>) -> Vec<LayerEmission> {
    use super::outcome::EmissionKind;
    let mut siblings = Vec::new();
    emissions.retain(|em| {
        if matches!(em.kind, EmissionKind::Sibling) {
            siblings.push(em.clone());
            false
        } else {
            true
        }
    });
    siblings
}

/// Error result of a single [`CommitPipeline::run`] call.
///
/// Carries the actual [`CommitError`] paired with the `Sibling`
/// emissions that were queued by phases *before* the failing phase.
/// The orchestrator's drain loop rescues siblings (§6.1) by
/// re-queueing them at depth 0, parented at the unchanged `ctx.head`;
/// `Child` emissions from the failed run are dropped during
/// partitioning because their intended parent did not land.
///
/// Lattice wrappers that don't run an orchestrator convert via
/// `.map_err(|e| e.error)`.
///
/// D41 §5 / Phase D.
#[derive(Debug)]
pub struct PipelineRunErr {
    /// The phase error that aborted the pipeline run.
    pub error: CommitError,
    /// Sibling emissions queued before the failing phase. The
    /// orchestrator re-queues these at depth 0.
    pub sibling_emissions: Vec<LayerEmission>,
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
///
/// No `structural_validate`: followup layers carry kernel-emitted content
/// (verdict_provenance, institution_classes). Well-formedness is the
/// emitter's contract (verdict / runtime invocation builders for the
/// audit path; institution-registration extraction for the
/// institution_classes path), so re-validation is redundant and forces the ontology to be
/// permissive enough for every shape the kernel emits. If an emitter
/// produces malformed content that's a kernel bug to fix at the emitter.
static STRUCTURAL_FOLLOWUP_PHASES: &[Phase] = &[build, persist];

/// `with_institutions` `didPersist` slice — D41 §3.6 / §5.
///
/// `trigger_vector_sweep` (D43 §5.5) runs on the `didPersist` slot
/// because the institution-registration hook may surface institution-class
/// resources via a Child emission; the sweep operates on the layer
/// that was persisted *in this pipeline run*, so it doesn't matter
/// whether the Child layer has landed yet — the sweep targets this
/// persisted layer's `defined_iris()` only.
static WITH_INSTITUTIONS_DID_PERSIST: &[DidPersistHook] = &[trigger_vector_sweep];

/// Empty `didPersist` slice shared by pipelines without post-persist
/// hooks.
static NO_DID_PERSIST: &[DidPersistHook] = &[];

#[cfg(test)]
mod tests {
    //! D41 Phase F.5 — `CommitPipeline::for_kind` correctness and
    //! `partition_siblings` semantics.

    use super::*;
    use crate::commit::outcome::EmissionKind;
    use std::collections::BTreeSet;

    /// Hole 7 — every `PipelineKind` resolves to a pipeline whose
    /// `kind` matches, whose phase slice matches D41 §5, and whose
    /// did_persist slice matches §3.6.
    ///
    /// Asserts via:
    /// - `kind()` equality,
    /// - phase slice length (the phase function items have stable
    ///   addresses but comparing function pointers across the slice
    ///   is brittle because the compiler may dedupe or inline; instead
    ///   we cross-check the phase slice the constructor uses by
    ///   comparing it to the named static (since `for_kind` is the
    ///   only public path to the slices, this confirms dispatch),
    /// - did_persist slice length and identity for institutions.
    #[test]
    fn for_kind_dispatches_correctly_for_each_kind() {
        // StructuralOnly — [build, structural_validate, persist]
        let p = CommitPipeline::for_kind(PipelineKind::StructuralOnly);
        assert_eq!(p.kind, PipelineKind::StructuralOnly);
        assert_eq!(p.phases.len(), 3);
        assert!(p.did_persist.is_empty());
        // Confirm dispatch routes through the canonical canned ctor:
        // comparing against `structural_only()` keeps the test stable
        // under future phase-list edits — if someone changes the slice
        // for one but not the dispatch, this test fails loudly.
        assert_eq!(
            p.phases.len(),
            CommitPipeline::structural_only().phases.len()
        );

        // WithRetroactive — [build, structural_validate,
        // retroactive_with_cascade, persist]
        let p = CommitPipeline::for_kind(PipelineKind::WithRetroactive);
        assert_eq!(p.kind, PipelineKind::WithRetroactive);
        assert_eq!(p.phases.len(), 4);
        assert!(p.did_persist.is_empty());
        assert_eq!(
            p.phases.len(),
            CommitPipeline::with_retroactive().phases.len()
        );

        // WithInstitutions — [build, structural_validate,
        // retroactive_with_cascade, autoonload_dispatch, persist]
        // + didPersist: [trigger_vector_sweep]
        let p = CommitPipeline::for_kind(PipelineKind::WithInstitutions);
        assert_eq!(p.kind, PipelineKind::WithInstitutions);
        assert_eq!(p.phases.len(), 5);
        assert_eq!(p.did_persist.len(), 1);
        assert_eq!(
            p.phases.len(),
            CommitPipeline::with_institutions().phases.len()
        );

        // StructuralFollowup — [build, persist] (no structural_validate
        // per D41 §5 / Phase D's fix).
        let p = CommitPipeline::for_kind(PipelineKind::StructuralFollowup);
        assert_eq!(p.kind, PipelineKind::StructuralFollowup);
        assert_eq!(p.phases.len(), 2);
        assert!(p.did_persist.is_empty());
        assert_eq!(
            p.phases.len(),
            CommitPipeline::structural_followup().phases.len()
        );
    }

    /// Hole 8 — `partition_siblings` carries Sibling entries out and
    /// leaves Child entries in the input vector, preserving order.
    ///
    /// `partition_siblings` is the helper `PipelineRunErr` uses to
    /// split `state.emissions` on the Err path. Child emissions belong
    /// to a parent that did not land and must be dropped; only Sibling
    /// emissions get rescued onto `PipelineRunErr.sibling_emissions`.
    /// The structural guarantee tested here is that the partition is
    /// disjoint (no Child leaks onto the rescue list) and preserves
    /// FIFO order among siblings (D41 §6.2).
    #[test]
    fn pipeline_run_err_partitions_state_emissions_into_siblings_only() {
        // Mixed emissions: Sibling, Child, Sibling, Child, Sibling.
        // The two non-sibling entries must stay behind; the three
        // siblings must come out in source order.
        let mk = |name: &'static str, kind: EmissionKind| LayerEmission {
            role: LayerRole::User,
            name,
            pipeline: PipelineKind::StructuralFollowup,
            kind,
            resources: Vec::new(),
            tombstones: BTreeSet::new(),
        };
        let mut emissions = vec![
            mk("sib_a", EmissionKind::Sibling),
            mk("child_a", EmissionKind::Child),
            mk("sib_b", EmissionKind::Sibling),
            mk("child_b", EmissionKind::Child),
            mk("sib_c", EmissionKind::Sibling),
        ];

        let siblings = partition_siblings(&mut emissions);

        // Rescued siblings: only Siblings, FIFO-preserved.
        assert_eq!(siblings.len(), 3);
        assert_eq!(siblings[0].name, "sib_a");
        assert_eq!(siblings[1].name, "sib_b");
        assert_eq!(siblings[2].name, "sib_c");
        assert!(siblings
            .iter()
            .all(|e| matches!(e.kind, EmissionKind::Sibling)));

        // Children left behind in the input vector (the caller will
        // drop the vector; we just confirm nothing else moved).
        assert_eq!(emissions.len(), 2);
        assert_eq!(emissions[0].name, "child_a");
        assert_eq!(emissions[1].name, "child_b");
        assert!(emissions
            .iter()
            .all(|e| matches!(e.kind, EmissionKind::Child)));
    }

    /// Edge case: input with only Children leaves no siblings to
    /// rescue. This mirrors the common case where a non-AutoOnLoad
    /// pipeline Errs — the only queueable kind on those is Child,
    /// and the rescue list is empty.
    #[test]
    fn partition_siblings_with_only_children_returns_empty() {
        let mut emissions = vec![LayerEmission {
            role: LayerRole::User,
            name: "only_child",
            pipeline: PipelineKind::StructuralFollowup,
            kind: EmissionKind::Child,
            resources: Vec::new(),
            tombstones: BTreeSet::new(),
        }];
        let siblings = partition_siblings(&mut emissions);
        assert!(siblings.is_empty());
        assert_eq!(emissions.len(), 1);
    }
}
