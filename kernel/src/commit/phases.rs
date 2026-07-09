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

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::context::{ExecutionContext, ExecutionMode};
use crate::institution::dispatch::{
    allocate_invocation_iri, build_runtime_invocation_resource, build_verdict_resource,
    dispatch_auto_on_load_for_layer, finalize_emitted_derivation, VerdictReading,
};
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::validation::{retroactive_validate, ValidationError, ValidationRule, Validator};

use super::outcome::{DispatchEntry, EmissionKind, LayerEmission, LayerRole};
use super::pipeline::{PhaseControl, PipelineKind};
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

    // Anchored-commit revalidation skip (D33 §6): this exact (content, supporting
    // content) already committed-and-validated, so structural validation is a proven
    // no-op. `persist` will hit the same cache and skip `store_layer` too.
    if state.persist.already_validated(layer) {
        tracing::debug!(
            { field::OPERATION } = operation::COMMIT_STRUCTURAL_VALIDATE,
            { field::LAYER_ID } = %layer.id(),
            "commit.structural_validate.skipped_anchored_revalidation"
        );
        return Ok(PhaseControl::Continue);
    }

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

    // Anchored-commit revalidation skip (D33 §6): a proven-valid (content, supporting
    // content) has no new retroactive dependents to find — it passed in this exact
    // context before. Skip the enumeration (the dominant commit cost at scale).
    if state.persist.already_validated(&layer) {
        tracing::debug!(
            { field::OPERATION } = operation::COMMIT_RETROACTIVE,
            { field::LAYER_ID } = %layer.id(),
            "commit.retroactive.skipped_anchored_revalidation"
        );
        return Ok(PhaseControl::Continue);
    }

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
/// pairs into `state.provenance_resources`, and queues exactly one
/// `verdict_provenance` emission (pipeline kind
/// [`PipelineKind::StructuralFollowup`], [`EmissionKind::Sibling`])
/// whenever any verdict was produced — Holds, Undecidable, or Fails.
/// Ported from `commit_with_validation` in `kernel/src/context/mod.rs`.
///
/// A `Fails` verdict returns `Err(CommitError::Validation { ... })`.
/// The emission is queued *before* the phase decides Ok vs Err — the
/// orchestrator rescues it on the Err path as the audit anchor for
/// the rejected commit (D41 §6.1).
///
/// Phase requires `state.institutions` to be `Some` — the pipeline
/// kind controls whether it runs. Hitting the phase with
/// `state.institutions == None` is a wiring bug and aborts the commit
/// with `CommitError::Validation` carrying a single
/// `InstitutionValidation` error describing the misconfiguration.
///
/// D41 §3.4.
pub fn autoonload_dispatch(state: &mut CommitState<'_>) -> Result<PhaseControl, CommitError> {
    let Some(institutions) = state.institutions.as_ref() else {
        // Defensive: `with_institutions` pipelines always carry an
        // `InstitutionContext`. Hitting this branch indicates a
        // pipeline-kind / config mismatch (a callable bug). Surface
        // through the normal Validation channel so the orchestrator's
        // Err arm partitions emissions cleanly.
        return Err(CommitError::Validation {
            errors: vec![ValidationError {
                resource_id: None,
                property: None,
                rule: ValidationRule::InstitutionValidation,
                message: "autoonload_dispatch invoked without an InstitutionContext".to_string(),
            }],
            total_violations: 1,
        });
    };

    let layer = state
        .layer
        .as_ref()
        .expect("autoonload_dispatch runs after build; layer must be Some")
        .clone();

    tracing::info!(
        { field::OPERATION } = operation::COMMIT_AUTOONLOAD,
        { field::LAYER_ID } = %layer.id(),
        "commit.autoonload.start"
    );

    // Read-only ExecutionContext over the freshly-built (not-yet-persisted)
    // user layer so AutoOnLoad QueryClasses can resolve cross-references
    // against the candidate chain. Mirrors the snapshot in
    // `commit_with_validation`.
    let snapshot = ExecutionContext::new(
        Arc::clone(&layer),
        "__validate__",
        ExecutionMode::ReadOnly,
        state.storage.clone(),
    );

    let auto_outcome = dispatch_auto_on_load_for_layer(
        layer.as_ref(),
        institutions.index.as_ref(),
        institutions.runtime.as_ref(),
        &snapshot,
    );

    // Handler-side failures (missing institution, malformed Verdict)
    // carry no provenance — surface as plain Validation. The
    // partition step in `pipeline.run` drops any `Child` emissions
    // queued earlier in the phase walk (none today) and rescues any
    // `Sibling` (none queued yet, because the verdict-pair build
    // below hasn't run). The institution-runtime error simply unwinds.
    if !auto_outcome.errors.is_empty() {
        let total = auto_outcome.errors.len();
        return Err(CommitError::Validation {
            errors: auto_outcome.errors,
            total_violations: total,
        });
    }

    // Build `RuntimeInvocation` + `Verdict` provenance resources for
    // every dispatch (Holds, Undecidable, and Fails alike). Per D31
    // §6.3, every dispatch that produced a Verdict gets a chain-side
    // provenance pair; the Fails arm additionally surfaces an error.
    let mut provenance: Vec<crate::ontology::Resource> = Vec::new();
    let mut fail_errors: Vec<ValidationError> = Vec::new();
    for dispatch in &auto_outcome.dispatches {
        // Record the per-subject dispatch reading for surfacing back
        // to the handler / response, irrespective of Holds / Fails /
        // Undecidable.
        state.dispatched_verdicts.push(DispatchEntry {
            subject_iri: dispatch.subject_iri.clone(),
            query_class_iri: dispatch.query_class_iri.as_str().to_string(),
            verdict: dispatch.verdict.clone(),
        });

        let invocation_iri = allocate_invocation_iri();
        let invocation = build_runtime_invocation_resource(
            dispatch,
            &invocation_iri,
            &derive_verdict_iri(&invocation_iri),
        );
        let verdict = build_verdict_resource(
            dispatch,
            invocation.as_ref().map(|_| &invocation_iri),
            None,
            None,
        );
        if matches!(dispatch.verdict, VerdictReading::Fails) {
            let verdict_ref = verdict
                .as_ref()
                .and_then(|v| v.id().map(|i| i.as_str().to_string()))
                .unwrap_or_else(|| "<embedded>".to_string());
            fail_errors.push(ValidationError {
                resource_id: dispatch.subject_iri.clone(),
                property: None,
                rule: ValidationRule::InstitutionValidation,
                message: format!(
                    "AutoOnLoad QueryClass `{}` returned Fails (Verdict `{}`)",
                    dispatch.query_class_iri, verdict_ref
                ),
            });
        }
        if let Some(inv) = invocation.as_ref() {
            provenance.push(inv.clone());
        }
        if let Some(v) = verdict {
            provenance.push(v);
        }

        // Emit institution-side derivations alongside the verdict when
        // the gate Holds (or Undecidable — undecidable verdicts still
        // commit chain artefacts; the orchestrator just doesn't admit
        // the gated subject's commit). On Fails, the per-effect
        // derivations are dropped — a failed analysis attests nothing
        // statistically, so its would-be StatisticalAnalysisResults must not
        // pollute the witness index.
        if !matches!(dispatch.verdict, VerdictReading::Fails) {
            for raw_derivation in &dispatch.derivations {
                if let Some(stamped) = finalize_emitted_derivation(
                    dispatch,
                    invocation.as_ref().map(|_| &invocation_iri),
                    raw_derivation.clone(),
                ) {
                    provenance.push(stamped);
                }
            }
        }
    }

    // Queue the audit-anchor Sibling emission whenever ANY dispatch
    // ran — Holds, Undecidable, or Fails. The `Sibling` kind tells
    // the orchestrator's drain that this emission lands regardless of
    // whether the user-layer pipeline returned Ok or Err (D41 §3.4 /
    // §6.1). Crucially, queue BEFORE the Err return below so the
    // partition step in `pipeline.run` can rescue it.
    if !provenance.is_empty() {
        // Stash for diagnostics; the emission consumes a clone of the
        // resources (well, the canonical copy) so the orchestrator
        // can drive the follow-up persist without re-running dispatch.
        state.provenance_resources = provenance.clone();
        state.emissions.push(LayerEmission {
            role: LayerRole::AuditProvenance,
            name: "verdict_provenance",
            pipeline: PipelineKind::StructuralFollowup,
            kind: EmissionKind::Sibling,
            resources: provenance,
            tombstones: BTreeSet::new(),
        });
    }

    tracing::info!(
        { field::OPERATION } = operation::COMMIT_AUTOONLOAD,
        { field::LAYER_ID } = %layer.id(),
        { field::COUNT } = auto_outcome.dispatches.len() as u64,
        fail_count = fail_errors.len() as u64,
        "commit.autoonload.done"
    );

    if !fail_errors.is_empty() {
        let total = fail_errors.len();
        return Err(CommitError::Validation {
            errors: fail_errors,
            total_violations: total,
        });
    }

    Ok(PhaseControl::Continue)
}

/// Derive the deterministic Verdict IRI for a given RuntimeInvocation
/// per D31 §6.3 — `urn:eigenius:invocation:<inv-id>:verdict`.
fn derive_verdict_iri(invocation_iri: &Iri) -> Iri {
    Iri::parse(&format!("{}:verdict", invocation_iri.as_str())).expect("derived Verdict IRI parses")
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

#[cfg(test)]
mod tests {
    //! D41 Phase F.5 — `autoonload_dispatch` phase coverage for the
    //! Holds and no-match paths (the Fails path is covered indirectly
    //! by `orchestrator::tests::sibling_rescue_on_fails_verdict`).

    use super::*;
    use crate::commit::hooks::CommitHookHost;
    use crate::commit::outcome::{DispatchEntry, EmissionKind};
    use crate::commit::persister::{LayerPersister, PersistedLayerInfo};
    use crate::commit::state::{CommitState, InstitutionContext};
    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime, QueryOutcome};
    use crate::layer::{Layer, LayerBuilder, LayerStorage};
    use crate::ontology::eigon_json;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known;
    use crate::validation::CommitWorkingSet;

    /// Institution stub that returns a Holds Verdict for every query.
    /// Mirror of `orchestrator::tests::AlwaysFails` but for Holds.
    struct AlwaysHolds;
    impl Institution for AlwaysHolds {
        fn institution_iri(&self) -> &Iri {
            static INST_IRI: std::sync::OnceLock<Iri> = std::sync::OnceLock::new();
            INST_IRI.get_or_init(|| Iri::parse("urn:eigenius:test:phase:inst").unwrap())
        }
        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<crate::nbe::val::Val, crate::institution::error::InstitutionError> {
            unreachable!()
        }
        fn reify(
            &self,
            _: &Iri,
            _: &crate::nbe::val::Val,
            _: &ExecutionContext,
        ) -> Result<Resource, crate::institution::error::InstitutionError> {
            unreachable!()
        }
        fn query(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<QueryOutcome, crate::institution::error::InstitutionError> {
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse(well_known::IS_A).unwrap(),
                Value::Array(vec![Value::String(
                    "urn:eigenius:institution:verdicts:holds".into(),
                )]),
            );
            Ok(QueryOutcome::from_output(r))
        }
    }

    /// Persister stub — never invoked by `autoonload_dispatch`.
    struct UnusedPersister;
    impl LayerPersister for UnusedPersister {
        fn persist(
            &self,
            _branch: &str,
            _layer: &Arc<Layer>,
        ) -> Result<PersistedLayerInfo, ValidationError> {
            unreachable!("autoonload_dispatch does not call persist")
        }
    }

    /// Host stub — never invoked by `autoonload_dispatch`.
    struct UnusedHost;
    impl CommitHookHost for UnusedHost {
        fn rebuild_institution_index(
            &self,
            _top_layer: &Arc<Layer>,
        ) -> Result<(), Vec<ValidationError>> {
            unreachable!()
        }
    }

    /// Build the bootstrap chain extended with a QueryClass targeting
    /// `urn:eigenius:test:phase:Subject` through `AlwaysHolds`. Mirrors
    /// the orchestrator's `build_rescue_setup` shape but for the Holds
    /// path the phase test exercises.
    fn build_holds_chain() -> (
        Arc<Layer>,
        Arc<InstitutionIndex>,
        Arc<InstitutionRuntime>,
        LayerStorage,
    ) {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let storage = ctx.storage().clone();
        let bootstrap_head = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("phase_test", Some(bootstrap_head));

        let inst_iri = "urn:eigenius:test:phase:inst";
        let qc_iri = "urn:eigenius:test:phase:check";
        let subject = "urn:eigenius:test:phase:Subject";

        let mut qc = Resource::new(Iri::parse(qc_iri).unwrap());
        qc.set(
            Iri::parse(well_known::IS_A).unwrap(),
            Value::Array(vec![Value::String(well_known::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:query_class").unwrap(),
            Value::String(subject.into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:result_class").unwrap(),
            Value::String("urn:eigenius:institution:Verdict".into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:dispatch_role").unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:institution:dispatch_roles:auto_on_load".into(),
            )]),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:query_handler").unwrap(),
            Value::String("urn:eigenius:test:phase:proc:check".into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            Value::String(inst_iri.into()),
        );
        b.add_resource(qc).unwrap();

        // Declare the Subject class so any new-layer resource typed as
        // it is well-formed enough to reach autoonload_dispatch.
        let mut subject_class = Resource::new(Iri::parse(subject).unwrap());
        subject_class.set(
            Iri::parse(well_known::IS_A).unwrap(),
            Value::Array(vec![Value::String(well_known::CLASS.to_string())]),
        );
        subject_class.set(
            Iri::parse("urn:eigenius:core:description").unwrap(),
            Value::String("phase test subject".into()),
        );
        subject_class.set(
            Iri::parse("urn:eigenius:core:short_name").unwrap(),
            Value::String("PhaseSubject".into()),
        );
        b.add_resource(subject_class).unwrap();

        let chain = Arc::new(b.build(storage.clone()));
        let (idx, errors) = InstitutionIndex::from_layer(&chain);
        assert!(errors.is_empty(), "{errors:?}");
        let mut runtime = InstitutionRuntime::new();
        runtime.register(Box::new(AlwaysHolds)).unwrap();
        (chain, Arc::new(idx), Arc::new(runtime), storage)
    }

    /// Build a single-resource user layer typed as the Subject class.
    fn build_user_layer(parent: &Arc<Layer>, storage: LayerStorage) -> Arc<Layer> {
        let mut b = LayerBuilder::new("user", Some(Arc::clone(parent)));
        let mut subject = Resource::new(Iri::parse("urn:eigenius:test:phase:s1").unwrap());
        subject.set(
            Iri::parse(well_known::IS_A).unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:test:phase:Subject".into(),
            )]),
        );
        b.add_resource(subject).unwrap();
        Arc::new(b.build(storage))
    }

    /// Construct a `CommitState` for direct phase invocation. Returns
    /// `(state, ws)` — the working set must outlive `state` so we
    /// surface it back to the caller.
    fn make_state<'a>(
        layer: Arc<Layer>,
        storage: LayerStorage,
        institutions: Option<InstitutionContext<'a>>,
        host: &'a dyn CommitHookHost,
        persister: &'a dyn LayerPersister,
        ws: &'a mut CommitWorkingSet,
    ) -> CommitState<'a> {
        CommitState {
            storage: storage.clone(),
            persist: persister,
            host,
            policy: CommitPolicy::default(),
            branch: "main",
            institutions,
            builder: LayerBuilder::new("ignored", None),
            layer: Some(layer),
            cascade_tombstones: BTreeSet::new(),
            cascade_iterations: 0,
            dispatched_verdicts: Vec::<DispatchEntry>::new(),
            provenance_resources: Vec::new(),
            emissions: Vec::new(),
            hook_errors: Vec::new(),
            working_set: ws,
            persisted: None,
        }
    }

    /// Hole 1 — Holds verdict queues a `verdict_provenance` Sibling
    /// emission and returns `Ok(Continue)`.
    #[test]
    fn autoonload_queues_provenance_sibling_for_holds_verdict() {
        let (chain, idx, runtime, storage) = build_holds_chain();
        let user_layer = build_user_layer(&chain, storage.clone());
        let host = UnusedHost;
        let persister = UnusedPersister;
        let mut ws = CommitWorkingSet::in_memory();
        let institutions = Some(InstitutionContext {
            index: idx,
            runtime,
            _marker: std::marker::PhantomData,
        });
        let mut state = make_state(
            user_layer,
            storage,
            institutions,
            &host,
            &persister,
            &mut ws,
        );

        let result = autoonload_dispatch(&mut state);

        // Phase returns Ok(Continue) — Holds is not a rejection.
        assert!(
            matches!(result, Ok(PhaseControl::Continue)),
            "Holds verdict must not error; got {result:?}"
        );
        // Provenance accumulated.
        assert!(
            !state.provenance_resources.is_empty(),
            "Holds dispatch must populate provenance_resources"
        );
        // Exactly one emission queued.
        assert_eq!(state.emissions.len(), 1);
        let em = &state.emissions[0];
        assert_eq!(em.role, LayerRole::AuditProvenance);
        assert_eq!(em.name, "verdict_provenance");
        assert_eq!(em.pipeline, PipelineKind::StructuralFollowup);
        assert_eq!(em.kind, EmissionKind::Sibling);
        // Emission carries the same resources as provenance_resources.
        assert_eq!(em.resources.len(), state.provenance_resources.len());
        // dispatched_verdicts records the Holds reading for institutions.
        assert_eq!(state.dispatched_verdicts.len(), 1);
        assert!(matches!(
            state.dispatched_verdicts[0].verdict,
            crate::institution::dispatch::VerdictReading::Holds
        ));
    }

    /// Hole 2 — chain with no AutoOnLoad QueryClass declared leaves
    /// the phase a no-op: no emissions, no provenance, Ok(Continue).
    #[test]
    fn autoonload_emits_no_sibling_when_no_classes_match() {
        // Plain core layer, no QueryClass declarations.
        let storage = LayerStorage::in_memory();
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(storage.clone()));

        // Empty institution index — nothing to dispatch.
        let (idx, errors) = InstitutionIndex::from_layer(&core);
        assert!(errors.is_empty(), "{errors:?}");
        let runtime = InstitutionRuntime::new();

        // Build a trivial user layer with a single non-classy resource
        // — its `is_a` doesn't match any QueryClass so dispatch is empty.
        let mut b = LayerBuilder::new("user", Some(Arc::clone(&core)));
        let r = Resource::new(Iri::parse("urn:eigenius:user:plain").unwrap());
        b.add_resource(r).unwrap();
        let user_layer = Arc::new(b.build(storage.clone()));

        let host = UnusedHost;
        let persister = UnusedPersister;
        let mut ws = CommitWorkingSet::in_memory();
        let institutions = Some(InstitutionContext {
            index: Arc::new(idx),
            runtime: Arc::new(runtime),
            _marker: std::marker::PhantomData,
        });
        let mut state = make_state(
            user_layer,
            storage,
            institutions,
            &host,
            &persister,
            &mut ws,
        );

        let result = autoonload_dispatch(&mut state);

        assert!(
            matches!(result, Ok(PhaseControl::Continue)),
            "no matching classes must not error; got {result:?}"
        );
        assert!(
            state.provenance_resources.is_empty(),
            "no QueryClass match means no provenance"
        );
        assert!(
            state.emissions.is_empty(),
            "no provenance means no emission queued"
        );
        assert!(state.dispatched_verdicts.is_empty());
    }
}
