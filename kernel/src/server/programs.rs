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

//! `RunProgram`, `RunProgramByIri`, and `ValidateProgram` RPC handlers,
//! plus the shared `execute_program` path that both run handlers
//! collapse into once they have a resolved program + input.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::commit::persister::PersistedLayerInfo;
use crate::observability::{field, operation, RpcGuard};
use crate::ontology::{Iri, Resource};
use crate::program::expr;
use std::sync::Arc;
use tonic::{Response, Status};

/// The kernel's program evaluator, as a `prov:Activity`. Declared in
/// `ontologies/prov/prov.esl` so a run's trace has something on-chain to name.
const KERNEL_RUN_PROGRAM_ACTIVITY: &str = "urn:eigenius:prov:activity:kernel_run_program";

/// Names for `ValidateProgramResponse::checks_performed`. A name is
/// emitted only by the code path that runs the check it names, so the
/// list is a record of what happened rather than a description of what
/// the handler intends to do.
///
/// There is deliberately no `type_check` constant: nothing in this
/// module runs `nbe::check::check` — see [`EigeniusService::handle_validate_program`].
pub const CHECK_PARSE: &str = "parse";
pub const CHECK_COMPONENT_TEMPLATE: &str = "component_template";
pub const CHECK_OUTPUT_SCHEMA: &str = "output_schema";

impl EigeniusService {
    /// Shared execution path for `RunProgram` and `RunProgramByIri`.
    ///
    /// Both RPCs end up here once they have a resolved program +
    /// input Resource. This method handles task allocation (D21 §3.1),
    /// NbE evaluation in IO mode, ProgramTrace assembly, derived-output
    /// stamping (D6b §6), and trace-layer commit.
    pub(super) async fn execute_program(
        &self,
        branch: &str,
        program: Resource,
        input: Resource,
    ) -> Result<Response<RunProgramResponse>, Status> {
        // Resolve the per-branch ExecutionContext up front. Same Arc is
        // used for the layer-head snapshot below (task pin), the eval
        // step (read), and the trace-layer commit (write).
        let ctx_arc = self.get_branch_context(branch).await?;

        // D21 §3.1: allocate a task for this invocation. When a task
        // store is attached (persistent backend), the record is
        // persisted on entry and again on completion so a mid-flight
        // crash leaves a recoverable `Running` record for the resume
        // sweep. The evaluator routes IO dispatches through a
        // TaskContext so repeated calls with the same input each
        // occupy their own step_seq slot (D21 §3.2).
        // `layer_head` and `session_id` were destructured here only to build the blank
        // failure record eigenius#135 removed; the `TaskContext` carries the session id and
        // the existing record carries its own layer head, so neither is needed now.
        let (task_context, task_id_str) = match &self.task_store {
            Some(store) => {
                let session_id = self.session.read().await.session_id;
                let task_id = uuid::Uuid::new_v4();
                let layer_head = {
                    let ctx = ctx_arc.read().await;
                    ctx.head().id().clone()
                };
                let program_iri = program
                    .id()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default();
                let input_iri = input
                    .id()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default();
                let record = crate::task::TaskRecord::new_running(
                    session_id,
                    task_id,
                    program_iri,
                    input_iri,
                    layer_head.clone(),
                    now_millis(),
                );
                if let Err(e) = store.put_task(&record) {
                    return Err(Status::internal(format!("failed to persist task: {e}")));
                }
                let tc = Arc::new(crate::task::TaskContext::new(
                    session_id,
                    task_id,
                    Arc::clone(store),
                ));
                (Some(tc), task_id.to_string())
            }
            None => (None, String::new()),
        };

        // Execute via NbE in IO mode
        let started_at_ms = now_millis();
        let exec_result = {
            let ctx = ctx_arc.read().await;
            let components = Arc::clone(&*self.components.read().await);
            let index = Arc::clone(&*self.institution_index.read().await);
            let runtime = Arc::clone(&*self.institution_runtime.read().await);
            match crate::program::eval_io::execute_program_nbe_with_institutions(
                &program,
                &input,
                Arc::clone(ctx.head()),
                components,
                Some(index),
                Some(runtime),
                Some(Arc::clone(&self.trace_store)),
                task_context.clone(),
            ) {
                Ok(result) => result,
                Err(e) => {
                    // Record the failure by UPDATING the task's record, the way the
                    // completion path below does — `get_task`, mutate, `put_task`.
                    //
                    // This used to construct a fresh `TaskRecord::new_running` with
                    // `String::new()` for `program_iri` and `input_iri` and overwrite the
                    // record with it (eigenius#135). Those two fields are the
                    // `TaskKind::ProgramRun` payload — precisely what says *which* run
                    // failed — so a failed run's record named neither its program nor its
                    // input, and everything else `new_of_kind` defaults (creation time,
                    // retention, checkpoint state) was reset with them. A client polling
                    // `GetTaskStatus` saw a failure it could not attribute.
                    if let (Some(store), Some(tc)) = (&self.task_store, task_context.as_ref()) {
                        match store.get_task(&tc.session_id, &tc.task_id) {
                            Ok(Some(mut rec)) => {
                                rec.status = crate::task::TaskStatus::Failed;
                                rec.updated_at = now_millis();
                                if let Err(e) = store.put_task(&rec) {
                                    tracing::warn!(
                                        { field::OPERATION } = operation::TASK_CHECKPOINT,
                                        { field::ERROR_KIND } = "task_record_update_failed",
                                        { field::TASK_ID } = ?tc.task_id,
                                        { field::ERROR_MESSAGE } = %e,
                                        "failed to record run failure on the task record"
                                    );
                                }
                            }
                            // No record to update. Writing a fresh one here is what the
                            // defect did; a blank record is worse than none, because it
                            // reports a failure against a run it cannot name.
                            Ok(None) => tracing::warn!(
                                { field::OPERATION } = operation::TASK_CHECKPOINT,
                                { field::ERROR_KIND } = "task_record_absent",
                                { field::TASK_ID } = ?tc.task_id,
                                "run failed but no task record exists to mark Failed"
                            ),
                            Err(e) => tracing::warn!(
                                { field::OPERATION } = operation::TASK_CHECKPOINT,
                                { field::ERROR_KIND } = "task_record_read_failed",
                                { field::TASK_ID } = ?tc.task_id,
                                { field::ERROR_MESSAGE } = %e,
                                "run failed and its task record could not be read"
                            ),
                        }
                    }
                    // Eval errored before the commit attempt — no CAS
                    // happened, so `merge` stays None. (Sending an
                    // `UNSPECIFIED` MergeInfo here would render as a
                    // misleading `cached` badge in notebook UIs.)
                    return Ok(Response::new(RunProgramResponse {
                        success: false,
                        output: Vec::new(),
                        errors: vec![ValidationError {
                            resource_iri: String::new(),
                            property_iri: String::new(),
                            rule: "execution".to_string(),
                            message: format!("{e}"),
                            severity: "error".to_string(),
                        }],
                        trace_iri: String::new(),
                        task_id: task_id_str.clone(),
                        output_resource_iris: Vec::new(),
                        branch_advanced: false,
                        merge: None,
                    }));
                }
            }
        };

        let completed_at_ms = now_millis();
        let mut output = exec_result.output;
        let dispatched_traces = exec_result.dispatched_traces;
        let produced_resources = exec_result.produced_resources;
        let root_trace = exec_result.root_trace;

        // Compute metrics from the tree-structured trace (preferred) or
        // flat dispatched_traces list (fallback).
        let metrics = crate::program::trace::ProgramMetrics::from_trace(&root_trace);
        let total_tokens = metrics.total_tokens;
        let executed_steps = metrics.executed_steps;

        // Build ProgramTrace with all required fields (D6b §2)
        let trace_iri_str = format!("urn:eigenius:trace:exec-{}", uuid::Uuid::new_v4());

        // Attach DerivedResource epistemic stamp to the output (D6b §6, Phase 10b Step 4)
        {
            use crate::ontology::well_known as wk;
            let is_a_iri = Iri::parse("urn:eigenius:core:is_a").unwrap();
            let types = match output.get(&is_a_iri) {
                Some(crate::ontology::resource::Value::Array(arr)) => arr.clone(),
                _ => Vec::new(),
            };
            // An output that states NO class takes the one the program DECLARES it
            // produces. A program is typed `I -> O`, so its output inhabits `O`; reading
            // `program:output_type` here is that type discipline applied to the resource
            // rather than a second place to decide what the thing is.
            //
            // The gap this fills: a wrapped-R script builds its result through
            // `r_eigon_begin` / `r_eigon_set_*` and need not name a class, and the runtime
            // substrate used to supply one by stamping `reflection:DerivedResource` on every
            // output. P4 (6/n) deleted that whole axis — a computed claim rests on
            // `App(Declared(plan), Observed(inputs))`, not on the fact that a run happened —
            // and P5 (2/n) removed the class, leaving those outputs with no `is_a` at all and
            // every wrapped-R warrant failing Rule 1 on commit.
            let types = if types.is_empty() {
                let out_ty = Iri::parse(wk::PROGRAM_OUTPUT_TYPE).unwrap();
                program
                    .get(&out_ty)
                    .and_then(|v| v.as_str())
                    .map(|t| vec![crate::ontology::resource::Value::String(t.to_string())])
                    .unwrap_or_default()
            } else {
                types
            };
            output.set(is_a_iri, crate::ontology::resource::Value::Array(types));
            // The run is recorded by the ProductionTrace this points at. It used to
            // also stamp `DerivedResource` and nominate `epistemic_status =
            // epistemic:derived` — a resource declaring its own grade, which is the
            // self-nomination the design forbids, on an axis that no longer exists.
            output.set(
                Iri::parse(wk::DERIVATION).unwrap(),
                crate::ontology::resource::Value::String(trace_iri_str.clone()),
            );
        }

        // Build the two records this run leaves behind. Extracted so the resume sweep
        // emits the same pair rather than a second copy that drifts (eigenius#148) — the
        // task-record arms in this same function had already drifted that way
        // (eigenius#135).
        let RunRecords {
            program_trace: trace_resource,
            observation_trace,
            observation_iri: observation_iri_str,
        } = build_run_records(RunRecordInputs {
            trace_iri: &trace_iri_str,
            output: &output,
            program: &program,
            input: &input,
            root_trace: root_trace.as_ref(),
            started_at_ms,
            completed_at_ms,
            total_tokens,
            executed_steps,
        });

        // Auto-commit program-run layer: produced domain resources
        // (comorphism reify outputs, program-final output) +
        // ProgramTrace + all IO ComponentTraces.
        //
        // Per D41 §10, RunProgram / RunProgramByIri commit through
        // `WithRetroactive` — not `WithInstitutions` — because only
        // Load runs AutoOnLoad today and RunProgram output is
        // kernel-generated (comorphism reify outputs + ProgramTrace),
        // not user-authored content the AutoOnLoad gate is designed
        // to police. Cascade tombstoning under `WithRetroactive`
        // still applies.
        let output_resource_iris: Vec<String> = produced_resources
            .iter()
            .filter_map(|r| r.id().map(|i| i.as_str().to_string()))
            .collect();
        // `branch_advanced` reports whether the durable branch ref
        // moved as a result of this run's commit. A fresh commit or
        // same-position cache hit advances the branch; a
        // different-position cache hit (D33 §6) does not.
        //
        // `errors` accumulates every failure that should turn this
        // response into a `success=false` (D34 §6 trace-not-found bug
        // — previously these were `warn!`'d and silently discarded,
        // leaving the caller holding a `trace_iri` that pointed at a
        // layer the chain never accepted).
        let mut branch_advanced = false;
        // The user-layer's persist info. We stash the full struct so
        // the response can disambiguate `CACHED_DIFFERENT_POSITION`
        // from `UNSPECIFIED` via `info.cache_hit_different_position`;
        // surfacing only `merge_outcome` would conflate them.
        let mut user_persist_info: Option<PersistedLayerInfo> = None;
        // True iff the commit pipeline ran (orchestrator was invoked).
        // Distinguishes "the run committed (or tried to) — report the
        // outcome" from "we never got to the commit step — say nothing
        // about merge state." The notebook UI keys its cell-footer
        // badges on this distinction (D34 §6.1).
        let mut commit_attempted = false;
        let mut errors: Vec<ValidationError> = Vec::new();
        let result_layer_head = {
            let mut ctx = ctx_arc.write().await;

            // Add domain resources produced by the run (chain-resident
            // outputs of comorphism reify and the program's final
            // Resource value). Every resource added here is
            // kernel-generated — a failure to add one is an internal
            // bug (malformed IRI, conflicting type, etc.) and must
            // surface as a kernel-internal error, not be swallowed.
            for r in &produced_resources {
                if let Err(e) = ctx.add_resource(r.clone()) {
                    errors.push(ValidationError {
                        resource_iri: r.id().map(|i| i.as_str().to_string()).unwrap_or_default(),
                        property_iri: String::new(),
                        rule: "internal".to_string(),
                        message: format!("failed to add produced resource: {e}"),
                        severity: "error".to_string(),
                    });
                }
            }
            // Commit the program's final output Resource itself when it
            // carries an `@id` and isn't already among `produced_resources`
            // (the comorphism-reify path pushes its output there; a plain
            // component application — e.g. `RunRuntimeScript` — does not).
            // Without this the committed `ProgramTrace` points at a target that
            // isn't chain-resident, so the run record names something the chain
            // cannot resolve — breaking the D56 wrapped-component derivation path.
            if let Some(out_id) = output.id().cloned() {
                let already = produced_resources.iter().any(|r| r.id() == Some(&out_id));
                if !already {
                    if let Err(e) = ctx.add_resource(output.clone()) {
                        errors.push(ValidationError {
                            resource_iri: out_id.as_str().to_string(),
                            property_iri: String::new(),
                            rule: "internal".to_string(),
                            message: format!("failed to add program output: {e}"),
                            severity: "error".to_string(),
                        });
                    }
                }
            }
            // Commit the program resource itself when it carries an `@id` and isn't
            // already chain-resident or among the produced/output resources — so the
            // committed `ProgramTrace`'s `prov:program` reference resolves
            // (reference integrity, Rule 22). Inline `RunProgram` supplies the program
            // as bytes that never otherwise reach the chain; `RunProgramByIri`'s program
            // is already committed (`resolve` finds it), so this is a no-op there. Same
            // provenance fix as the output-resource commit above (`prov:resource`).
            if let Some(prog_id) = program.id().cloned() {
                let already = produced_resources.iter().any(|r| r.id() == Some(&prog_id))
                    || output.id() == Some(&prog_id);
                if !already && ctx.head().resolve(&prog_id).is_none() {
                    if let Err(e) = ctx.add_resource(program.clone()) {
                        errors.push(ValidationError {
                            resource_iri: prog_id.as_str().to_string(),
                            property_iri: String::new(),
                            rule: "internal".to_string(),
                            message: format!("failed to add program resource: {e}"),
                            severity: "error".to_string(),
                        });
                    }
                }
            }
            // Capture the trace IRI before moving the resource — needed
            // for the failure path's error message (trace_iri_str is
            // semantically the same value, but reading it off the
            // resource ties the error to the actual object that
            // failed).
            let trace_iri_for_err = trace_resource
                .id()
                .map(|i| i.as_str().to_string())
                .unwrap_or_default();
            if let Err(e) = ctx.add_resource(trace_resource) {
                errors.push(ValidationError {
                    resource_iri: trace_iri_for_err,
                    property_iri: String::new(),
                    rule: "internal".to_string(),
                    message: format!("failed to add ProgramTrace: {e}"),
                    severity: "error".to_string(),
                });
            }
            if let Err(e) = ctx.add_resource(observation_trace) {
                errors.push(ValidationError {
                    resource_iri: observation_iri_str.clone(),
                    property_iri: String::new(),
                    rule: "internal".to_string(),
                    message: format!("failed to add ObservationTrace: {e}"),
                    severity: "error".to_string(),
                });
            }
            // ComponentTraces are designed to be embedded inside the
            // ProgramTrace's `trace_tree` (see `Resource::new_embedded`
            // in `trace_to_resource`), not added as standalone chain
            // resources — they have no `@id`. The flat `dispatched_traces`
            // list is purely for metrics aggregation (see
            // `ProgramMetrics::from_trace` above); the audit-anchor copy
            // lives in `trace_tree` via `root_trace`. Suppress the
            // `dispatched_traces` variable to make the intent explicit.
            let _ = &dispatched_traces;

            if !errors.is_empty() {
                // Don't attempt the commit if any kernel-generated
                // resource failed to add — the layer would be missing
                // the trace or an output and the response would be
                // structurally inconsistent.
                None
            } else {
                let working = match ctx.take_working("program-run") {
                    Ok(b) => b,
                    Err(e) => {
                        errors.push(ValidationError {
                            resource_iri: String::new(),
                            property_iri: String::new(),
                            rule: "commit".to_string(),
                            message: format!("program-run take_working failed: {e}"),
                            severity: "error".to_string(),
                        });
                        return Ok(Response::new(RunProgramResponse {
                            success: false,
                            output: Vec::new(),
                            errors,
                            trace_iri: String::new(),
                            task_id: task_id_str,
                            output_resource_iris: Vec::new(),
                            branch_advanced: false,
                            merge: None,
                        }));
                    }
                };
                let root = crate::commit::LayerEmission::from_builder(
                    crate::commit::LayerRole::User,
                    "program-run",
                    crate::commit::PipelineKind::WithRetroactive,
                    crate::commit::EmissionKind::Child,
                    working,
                );

                let commit_outcome = {
                    let orchestrator = crate::commit::CommitOrchestrator {
                        ctx: &mut ctx,
                        pool: &self.commit_ws_pool,
                        persister: &*self.persister,
                        host: self as &dyn crate::commit::CommitHookHost,
                        branch,
                        policy: crate::lattice::CommitPolicy::default(),
                        institutions: None,
                        did_drain: crate::commit::CommitOrchestrator::default_did_drain(),
                    };
                    orchestrator.run(root)
                };
                commit_attempted = true;

                // Surface didPersist + drain hook errors as
                // ValidationErrors (commits stand either way per
                // D41 §3.6, but the caller should still see them).
                for layer_outcome in &commit_outcome.layers {
                    for ve in &layer_outcome.hook_errors {
                        errors.push(kernel_validation_error_to_proto(ve));
                    }
                }
                for ve in &commit_outcome.drain_hook_errors {
                    errors.push(kernel_validation_error_to_proto(ve));
                }

                // Surface the pipeline error (if any). Pre-D41 logged
                // one event per rule violation so dashboards can group
                // on `error_kind` — keep that.
                if let Some(commit_err) = commit_outcome.error.as_ref() {
                    match commit_err {
                        crate::commit::CommitError::Validation { errors: verrs, .. }
                        | crate::commit::CommitError::CascadeAbort { errors: verrs, .. } => {
                            for ve in verrs {
                                tracing::warn!(
                                    { field::OPERATION } = operation::VALIDATE_RESOURCE,
                                    { field::ERROR_KIND } = ?ve.rule,
                                    { field::RESOURCE_IRI } = ve.resource_id.as_ref().map(|i| i.as_str()).unwrap_or(""),
                                    { field::PROPERTY_IRI } = ve.property.as_ref().map(|i| i.as_str()).unwrap_or(""),
                                    { field::ERROR_MESSAGE } = %ve.message,
                                    "program-run validation error"
                                );
                            }
                        }
                        other => {
                            tracing::warn!(
                                { field::OPERATION } = operation::LAYER_COMMIT,
                                { field::ERROR_KIND } = "program_run_commit_failed",
                                { field::ERROR_MESSAGE } = %other,
                                "program-run layer commit failed"
                            );
                        }
                    }
                    for proto_err in commit_error_to_proto(commit_err) {
                        errors.push(proto_err);
                    }
                }

                // Inspect outcome.layers[0] for the user-layer persist
                // info (RunProgram emits no follow-up layers under
                // `WithRetroactive`).
                if let Some(user) = commit_outcome.layers.first() {
                    branch_advanced |= user.persist.branch_advanced;
                    user_persist_info = Some(user.persist.clone());
                    // Return the user layer's id so the task record
                    // can point at it for completion / failure audit.
                    Some(user.persist.layer_id.clone())
                } else {
                    None
                }
            }
        };

        let success = errors.is_empty();

        // Record the task's final state. A successful run records the
        // result layer id so clients that polled via GetTaskStatus can
        // resolve it (D21 §3.7); a failed run records `Failed` and the
        // provenance layer id (if any) so the failure audit is also
        // discoverable through the same path.
        if let (Some(store), Some(tc)) = (&self.task_store, task_context.as_ref()) {
            if let Ok(Some(mut rec)) = store.get_task(&tc.session_id, &tc.task_id) {
                rec.status = if success {
                    crate::task::TaskStatus::Completed
                } else {
                    crate::task::TaskStatus::Failed
                };
                rec.result_layer_head = result_layer_head;
                rec.updated_at = now_millis();
                if let Err(e) = store.put_task(&rec) {
                    tracing::warn!(
                        { field::OPERATION } = operation::TASK_CHECKPOINT,
                        { field::ERROR_KIND } = "task_record_update_failed",
                        { field::TASK_ID } = ?tc.task_id,
                        { field::ERROR_MESSAGE } = %e,
                        "failed to update task record after run completion"
                    );
                }
            }
        }

        // On failure, blank the response's `output` / `trace_iri` /
        // `output_resource_iris` — those IRIs reference resources the
        // chain didn't accept, so returning them gives clients a
        // dangling pointer (the exact bug this fix closes).
        Ok(Response::new(RunProgramResponse {
            success,
            output: if success {
                Self::serialize_resource(&output)
            } else {
                Vec::new()
            },
            errors,
            trace_iri: if success {
                trace_iri_str
            } else {
                String::new()
            },
            task_id: task_id_str,
            output_resource_iris: if success {
                output_resource_iris
            } else {
                Vec::new()
            },
            branch_advanced,
            // Only populate `merge` when persist actually ran — see
            // `commit_attempted`'s declaration. A failure that aborts
            // before persist (add_resource on a kernel-generated
            // resource, eval error) sends `merge=None` so the notebook
            // doesn't render a misleading badge.
            merge: if commit_attempted {
                Some(merge_info_from_persist_info(user_persist_info.as_ref()))
            } else {
                None
            },
        }))
    }

    /// Run the static checks the kernel has for a `program:Program`
    /// and report exactly which ones ran.
    ///
    /// Three checks: the body decodes to a EigenTT term and every
    /// referenced class resolves (`parse`), D8 component templates
    /// resolve against the input type (`component_template`), and the
    /// D8 §4 output schemas are bijective (`output_schema`). Each name
    /// goes into `checks_performed` as it runs, and `valid` is the
    /// conjunction over that list and nothing else.
    ///
    /// **No EigenTT type-check runs here** (issue #143). `parse_program`
    /// returns a term and a Pi type, and running
    /// `nbe::check::check(term, typ)` is two lines — but the checker
    /// cannot type a `program:Component` reference. `parse_apply`
    /// encodes a component as `Exp::Var(<component IRI>)` and
    /// `check_infer`'s `Var` arm resolves names in `Gamma` only, so
    /// checking the repo's own `ontologies/examples/simple-program.json`
    /// returns `IllFormed("unbound variable in type context:
    /// urn:eigenius:program:components:Identity")`. Reporting that as
    /// `valid: false` would be as untrue as the success claim it
    /// replaced: the program runs correctly. Wiring the check honestly
    /// needs a typing rule for component references first — see the
    /// issue for what that entails. Until then `checks_performed` omits
    /// `"type_check"`, and no log or field here claims one ran.
    pub(super) async fn handle_validate_program(
        &self,
        req: ValidateProgramRequest,
    ) -> Result<Response<ValidateProgramResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_VALIDATE_PROGRAM);
        let resources = self
            .parse_resources(&req.program, &req.content_type, Some(DEFAULT_BRANCH))
            .await?;
        let program = resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no program resource"))?;

        let ctx_arc = self.get_branch_context(DEFAULT_BRANCH).await?;
        let ctx = ctx_arc.read().await;

        match expr::parse_program(&program, ctx.head()) {
            Ok((_term, typ)) => {
                // `parse` ran and passed. Every further name is pushed
                // by the code that runs the check it names, so a check
                // skipped for shape reasons stays out of the list.
                let mut checks_performed = vec![CHECK_PARSE.to_string()];
                // Validate template references against input type
                let mut template_errors = Vec::new();
                let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
                let input_type_prop = Iri::parse("urn:eigenius:program:input_type").unwrap();
                // `program:input_type` is `data_type: resource`, so an IRI string. Read it
                // through an accessor rather than by matching a variant.
                if let (
                    Some(input_type_str),
                    Some(crate::ontology::resource::Value::Embedded(body)),
                ) = (
                    program.get(&input_type_prop).and_then(|v| v.as_str()),
                    program.get(&body_prop),
                ) {
                    if let Ok(input_type_iri) = Iri::parse(input_type_str) {
                        let comp_arg_prop =
                            Iri::parse("urn:eigenius:program:component_argument").unwrap();
                        // Walk expression tree looking for component arguments
                        fn find_comp_args(resource: &Resource, prop: &Iri) -> Vec<Resource> {
                            let mut args = Vec::new();
                            if let Some(crate::ontology::resource::Value::Embedded(arg)) =
                                resource.get(prop)
                            {
                                args.push(arg.as_ref().clone());
                            }
                            // Recurse into embedded resources
                            for val in resource.properties().values() {
                                if let crate::ontology::resource::Value::Embedded(child) = val {
                                    args.extend(find_comp_args(child, prop));
                                }
                            }
                            args
                        }
                        for comp_arg in find_comp_args(body, &comp_arg_prop) {
                            let errs = crate::program::schema::validate_component_templates(
                                &comp_arg,
                                &input_type_iri,
                                ctx.head(),
                            );
                            for e in errs {
                                template_errors.push(ValidationError {
                                    resource_iri: String::new(),
                                    property_iri: String::new(),
                                    rule: "template".to_string(),
                                    message: format!("{e}"),
                                    severity: "error".to_string(),
                                });
                            }
                        }
                        checks_performed.push(CHECK_COMPONENT_TEMPLATE.to_string());
                    }
                }

                // Validate output schemas (bijectivity check, D8 §4)
                for e in crate::program::schema::validate_output_schemas(&program, ctx.head()) {
                    template_errors.push(ValidationError {
                        resource_iri: String::new(),
                        property_iri: String::new(),
                        rule: "schema_bijectivity".to_string(),
                        message: format!("{e}"),
                        severity: "error".to_string(),
                    });
                }
                checks_performed.push(CHECK_OUTPUT_SCHEMA.to_string());

                if template_errors.is_empty() {
                    tracing::debug!(
                        { field::OPERATION } = operation::PROGRAM_STATIC_CHECKS,
                        program_iri = program.id().map(|i| i.as_str()).unwrap_or(""),
                        declared_type = ?typ,
                        checks_performed = ?checks_performed,
                        "program static checks passed; no EigenTT type-check ran (#143)"
                    );
                    Ok(Response::new(ValidateProgramResponse {
                        valid: true,
                        errors: Vec::new(),
                        program_type: format!("{typ:?}"),
                        checks_performed,
                    }))
                } else {
                    Ok(Response::new(ValidateProgramResponse {
                        valid: false,
                        errors: template_errors,
                        program_type: format!("{typ:?}"),
                        checks_performed,
                    }))
                }
            }
            // `parse_program` failed, so nothing downstream of it ran.
            // The rule is `parse`, not `type_check`: the body did not
            // decode to a EigenTT term, which is a decoding failure —
            // the term was never handed to the checker.
            Err(e) => Ok(Response::new(ValidateProgramResponse {
                valid: false,
                errors: vec![ValidationError {
                    resource_iri: String::new(),
                    property_iri: String::new(),
                    rule: CHECK_PARSE.to_string(),
                    message: e,
                    severity: "error".to_string(),
                }],
                program_type: String::new(),
                checks_performed: vec![CHECK_PARSE.to_string()],
            })),
        }
    }

    pub(super) async fn handle_run_program(
        &self,
        req: RunProgramRequest,
    ) -> Result<Response<RunProgramResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_RUN_PROGRAM);
        tracing::debug!(
            { field::OPERATION } = operation::RPC_RUN_PROGRAM,
            { field::CONTENT_TYPE } = %req.content_type,
            "run_program payload"
        );
        let branch = resolve_branch_name(&req.branch).to_string();
        let program_resources = self
            .parse_resources(&req.program, &req.content_type, Some(&branch))
            .await?;
        let program = program_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no program resource"))?;

        let input_resources = self
            .parse_resources(&req.input, &req.content_type, Some(&branch))
            .await?;
        let input = input_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no input resource"))?;

        self.execute_program(&branch, program, input).await
    }

    pub(super) async fn handle_run_program_by_iri(
        &self,
        req: RunProgramByIriRequest,
    ) -> Result<Response<RunProgramResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_RUN_PROGRAM_BY_IRI);
        tracing::debug!(
            { field::OPERATION } = operation::RPC_RUN_PROGRAM_BY_IRI,
            { field::PROGRAM_IRI } = %req.program_iri,
            { field::RESOURCE_IRI } = %req.input_iri,
            "run_program_by_iri target"
        );
        if req.program_iri.is_empty() {
            return Err(Status::invalid_argument("program_iri is required"));
        }
        if req.input_iri.is_empty() {
            return Err(Status::invalid_argument("input_iri is required"));
        }

        let program_iri = Iri::parse(&req.program_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid program_iri: {e}")))?;
        let input_iri = Iri::parse(&req.input_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid input_iri: {e}")))?;

        let layer = self.resolve_read_layer(&req.at_layer, &req.branch).await?;
        let program = layer
            .resolve(&program_iri)
            .map(|arc| (*arc).clone())
            .ok_or_else(|| {
                Status::not_found(format!("program resource not found: {}", req.program_iri))
            })?;
        let input = layer
            .resolve(&input_iri)
            .map(|arc| (*arc).clone())
            .ok_or_else(|| {
                Status::not_found(format!("input resource not found: {}", req.input_iri))
            })?;

        let branch = resolve_branch_name(&req.branch).to_string();
        self.execute_program(&branch, program, input).await
    }
}

/// The two records a completed program run leaves behind.
///
/// Extracted from `execute_program` so the resume sweep emits the same pair rather than a
/// second copy that drifts (eigenius#148). The two task-record arms inside
/// `execute_program` had already drifted exactly that way — the failure arm overwrote what
/// the completion arm updated (eigenius#135) — which is the argument for one builder.
///
/// Pure: no `self`, no I/O, no clock. Timestamps arrive as arguments so a caller that wants
/// a reproducible artifact fixes them, the same discipline `dcg::formalizer` uses.
pub(super) struct RunRecords {
    /// Provenance. Grounds nothing: a computed claim rests on
    /// `App(Declared(plan), Observed(inputs))`, and that a run happened is not a third
    /// ground.
    pub program_trace: Resource,
    /// The `Observed` leaf a sampled outcome is owed, on the run's output.
    pub observation_trace: Resource,
    /// The observation trace's IRI, for the caller's error reporting.
    pub observation_iri: String,
}

/// Inputs to [`build_run_records`].
pub(super) struct RunRecordInputs<'a> {
    pub trace_iri: &'a str,
    pub output: &'a Resource,
    pub program: &'a Resource,
    /// What the run was applied to — `prov:input` (eigenius#147).
    pub input: &'a Resource,
    pub root_trace: Option<&'a crate::program::trace::Trace>,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub total_tokens: i64,
    pub executed_steps: i64,
}

pub(super) fn build_run_records(i: RunRecordInputs<'_>) -> RunRecords {
    use crate::ontology::resource::Value;

    let trace_iri_str = i.trace_iri;
    let output = i.output;
    let program = i.program;
    let input = i.input;
    let root_trace = i.root_trace;
    let started_at_ms = i.started_at_ms;
    let completed_at_ms = i.completed_at_ms;
    let total_tokens = i.total_tokens;
    let executed_steps = i.executed_steps;

    let mut trace_resource = Resource::new(Iri::parse(trace_iri_str).unwrap());
    trace_resource.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(
            "urn:eigenius:prov:ProgramTrace".to_string(),
        )]),
    );
    // ProgramTrace's three required fields, unified with
    // DeclarationTrace and ObservationTrace around the D49 witness-
    // emitter contract: `resource` is the target IRI the trace
    // points at (the program's output here); `source` is a string
    // naming the producer; `timestamp` is the wall-clock the trace
    // was emitted (the completion timestamp). The rich execution-
    // trace metadata (program / started_at / completed_at /
    // trace_tree / metrics) lives in recommends; this handler
    // fills every one.
    if let Some(out_id) = output.id() {
        trace_resource.set(
            Iri::parse("urn:eigenius:prov:resource").unwrap(),
            Value::iri(out_id),
        );
    }
    // `prov:was_generated_by` is resource-typed, so the run needs an Activity
    // to point at rather than the free-text `"kernel:run_program"` this used to
    // write. The activity is the kernel's own program-running facility; it is
    // committed alongside the trace so the reference resolves.
    trace_resource.set(
        Iri::parse("urn:eigenius:prov:was_generated_by").unwrap(),
        Value::String(KERNEL_RUN_PROGRAM_ACTIVITY.to_string()),
    );
    trace_resource.set(
        Iri::parse("urn:eigenius:prov:timestamp").unwrap(),
        Value::String(millis_to_iso8601(completed_at_ms)),
    );
    if let Some(prog_id) = program.id() {
        trace_resource.set(
            Iri::parse("urn:eigenius:prov:program").unwrap(),
            Value::iri(prog_id),
        );
    }
    // What the run was applied to (eigenius#147). Never populated before: the only thing
    // written was `reflection:input_hash` (`program/trace.rs:313`), a different property,
    // so a trace recorded that a run happened without naming its subject.
    //
    // Embedded, always — **not** referenced by IRI even when the input carries one. A run
    // does not commit its input: `execute_program` adds the produced resources, the output,
    // the program and these traces, and never the input. So an IRI reference dangles and
    // Rule 22 rejects the trace with `UnresolvedClassReference`, which is what a first
    // attempt at this did.
    //
    // Committing the input instead would make a reference resolvable and avoid duplicating
    // a large input across traces, but it changes what a run puts on the chain — a separate
    // decision, and one an author may not want for an input they passed by value.
    trace_resource.set(
        Iri::parse("urn:eigenius:prov:input").unwrap(),
        Value::Embedded(Box::new(input.clone())),
    );

    // `prov:trace_tree` is `recommends`, not `requires` — `prov:ProgramTrace` requires only
    // `prov:resource`, `prov:was_generated_by` and `prov:timestamp`. The comment here said
    // "Required" and was wrong (eigenius#147).
    //
    // It IS read, contrary to that issue's first half: `notebooks/src/runtime/traceResource.ts`
    // resolves it and flattens the right-leaning `Trace::Let` chain into siblings for the
    // notebook's trace panel. The issue's scope enumerated "the kernel, the crates, the CLI
    // or the orchestrator" — all four true, and `notebooks/` is none of them.
    if let Some(trace) = root_trace {
        let trace_tree = crate::program::trace::trace_to_resource(trace);
        trace_resource.set(
            Iri::parse("urn:eigenius:prov:trace_tree").unwrap(),
            Value::Embedded(Box::new(trace_tree)),
        );
    }
    // Required: started_at, completed_at (ISO 8601)
    trace_resource.set(
        Iri::parse("urn:eigenius:prov:started_at").unwrap(),
        Value::String(millis_to_iso8601(started_at_ms)),
    );
    trace_resource.set(
        Iri::parse("urn:eigenius:prov:completed_at").unwrap(),
        Value::String(millis_to_iso8601(completed_at_ms)),
    );
    trace_resource.set(
        Iri::parse("urn:eigenius:prov:total_tokens").unwrap(),
        Value::Integer(total_tokens),
    );
    trace_resource.set(
        Iri::parse("urn:eigenius:prov:executed_steps").unwrap(),
        Value::Integer(executed_steps),
    );
    // Recommended: universe_level = 0 (traces about domain resources)
    trace_resource.set(
        Iri::parse(crate::ontology::well_known::UNIVERSE_LEVEL).unwrap(),
        Value::Integer(0),
    );

    // The run's outcome is SAMPLED, so the output carries an `ObservationTrace`
    // beside the `ProgramTrace` (kernel-run-records §2).
    //
    // Two resources, two roles. The `ProgramTrace` above is provenance and grounds
    // nothing — a computed claim rests on `App(Declared(plan), Observed(inputs))`,
    // and the fact that a run happened is not a third ground. The paper puts the
    // execution trace in the provenance graph explicitly: *"a sampled outcome
    // reduces to a single Observed leaf. Details such as the specific instrument,
    // the configuration parameters, and the execution trace belong in the provenance
    // graph, not within the justification term."*
    //
    // **Why sampled rather than computed.** The paper's criterion is *"whether the
    // plan formalizes a deterministic function, not the medium of execution"*.
    // Nothing on any chain asserts that: 0 of 21 `stats:StatisticalAnalysisPlan`
    // resources carry a `prov:DeclarationTrace`, and eigenius#43 records why the
    // assertion is shaky even where it is made. So the outcome is a recording under a
    // declared protocol, which is an `Observed` leaf.
    //
    // `program:component:deterministic` is deliberately NOT the predicate here. A
    // component flag cannot see whether a deterministic acceptor stands between a
    // stochastic step and the committed output — the shape the parser (LLMs rank, the
    // kernel type-checks) and the Lean institution both have, where the acceptance
    // grounds the result and the search record does not. It becomes the opt-out once
    // a plan can formalize its function; it is not the test.
    let observation_iri_str = format!("{trace_iri_str}:observed");
    let mut observation_trace = Resource::new(Iri::parse(&observation_iri_str).unwrap());
    observation_trace.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(
            "urn:eigenius:prov:ObservationTrace".to_string(),
        )]),
    );
    if let Some(out_id) = output.id() {
        observation_trace.set(
            Iri::parse("urn:eigenius:prov:resource").unwrap(),
            Value::iri(out_id),
        );
    }
    // `ObservationTrace` requires the Activity that produced the recording — the same
    // one the ProgramTrace names, committed alongside so the reference resolves.
    observation_trace.set(
        Iri::parse("urn:eigenius:prov:was_generated_by").unwrap(),
        Value::String(KERNEL_RUN_PROGRAM_ACTIVITY.to_string()),
    );
    observation_trace.set(
        Iri::parse("urn:eigenius:prov:timestamp").unwrap(),
        Value::String(millis_to_iso8601(completed_at_ms)),
    );
    // No link from here to the `ProgramTrace`. `prov:derivation` describes how a
    // RESOURCE was produced and the output already carries it (set above), so an
    // auditor reaching the output finds both traces. Putting it on this trace would
    // read as "this observation was produced by that program run", which is not what
    // the property means.

    RunRecords {
        program_trace: trace_resource,
        observation_trace,
        observation_iri: observation_iri_str,
    }
}

/// Tests for the `RunProgram` handler.
///
/// **Why these exist here rather than under `kernel/tests/`.** `execute_program` is
/// `pub(super)`, so only a module inside `crate::server` reaches it — and the paths worth
/// pinning are gated on `self.task_store`, which `EigeniusService::new()` leaves `None`.
/// `with_persistent_backend` over a `MemoryPersistentBackend` supplies both a task store
/// and a bootstrapped context in three lines, with no server, port or proto marshalling.
///
/// Before this module the handler had no test of any kind, and three items of the
/// kernel-run-records batch depended on it (§2, §3.2, §3.3).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::eigon_json;
    use crate::program::component::ComponentRegistry;
    use crate::storage::memory::MemoryPersistentBackend;

    /// A service with a task store, over an in-memory backend.
    fn service() -> EigeniusService {
        let backend = Arc::new(MemoryPersistentBackend::new());
        EigeniusService::with_persistent_backend(ComponentRegistry::default(), backend)
            .expect("service over memory backend")
    }

    /// A program that evaluates *and whose output commits*.
    ///
    /// The output class must resolve in the bootstrap chain and its properties must be
    /// declared, or the run succeeds and the commit fails `UnresolvedClassReference`.
    /// `prov:Agent` requires nothing and recommends `core:short_name`, so a `Construct`
    /// over it validates without seeding a layer first.
    fn working_program() -> Resource {
        eigon_json::parse_document(
            r#"{
                "@id": "urn:eigenius:test:runprog:ok",
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
                "urn:eigenius:program:input_type": "urn:eigenius:prov:Agent",
                "urn:eigenius:program:output_type": "urn:eigenius:prov:Agent",
                "urn:eigenius:program:body": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Construct"],
                    "urn:eigenius:program:class": "urn:eigenius:prov:Agent",
                    "urn:eigenius:program:fields": {
                        "urn:eigenius:core:short_name": {
                            "urn:eigenius:core:is_a": ["urn:eigenius:program:Literal"],
                            "urn:eigenius:program:value": "run-record-fixture"
                        }
                    }
                }
            }"#,
        )
        .expect("program parses")
        .remove(0)
    }

    /// A program that does not evaluate: the body applies a component IRI the registry
    /// does not hold, which is `ComponentDispatchFailed` since eigenius#144.
    fn failing_program() -> Resource {
        eigon_json::parse_document(
            r#"{
                "@id": "urn:eigenius:test:runprog:bad",
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
                "urn:eigenius:program:input_type": "urn:eigenius:prov:Agent",
                "urn:eigenius:program:output_type": "urn:eigenius:prov:Agent",
                "urn:eigenius:program:body": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
                    "urn:eigenius:program:component": "urn:eigenius:program:components:Transform",
                    "urn:eigenius:program:argument": {
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Literal"],
                        "urn:eigenius:program:value": 1
                    }
                }
            }"#,
        )
        .expect("program parses")
        .remove(0)
    }

    /// §3.3 / eigenius#135 — a failed run's task record keeps what it was.
    ///
    /// The failure arm used to overwrite the record with a fresh `new_running` carrying
    /// `String::new()` for `program_iri` and `input_iri`. Those two are the
    /// `TaskKind::ProgramRun` payload, so a failed run named neither its program nor its
    /// input and a client polling `GetTaskStatus` saw a failure it could not attribute.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_run_keeps_its_task_records_identity() {
        let svc = service();
        let resp = svc
            .execute_program("main", failing_program(), Resource::new_embedded())
            .await
            .expect("the handler returns a response even when the run fails");
        assert!(
            !resp.get_ref().success,
            "the program applies an unregistered component, so the run must fail"
        );

        let store = svc
            .task_store
            .as_ref()
            .expect("memory backend supplies one");
        let session_id = svc.session.read().await.session_id;
        let tasks = store.list_tasks(&session_id).expect("list tasks");
        let rec = tasks.last().expect("the run allocated a task record");

        assert_eq!(
            rec.status,
            crate::task::TaskStatus::Failed,
            "the failure is recorded"
        );
        match &rec.kind {
            crate::task::TaskKind::ProgramRun {
                program_iri,
                input_iri,
            } => assert!(
                !program_iri.is_empty() || !input_iri.is_empty(),
                "the record must still name what ran; both blank is eigenius#135, where the \
                 failure arm overwrote the record instead of updating it"
            ),
            other => panic!("expected a ProgramRun task record, got {other:?}"),
        }
    }

    /// §3.4 / eigenius#147 — the trace names what the run was applied to.
    ///
    /// `prov:input` was never populated. The only thing written was
    /// `reflection:input_hash` (`program/trace.rs:313`), a different property, so a
    /// `ProgramTrace` recorded that a run happened without naming its subject — while
    /// `prov:input`'s domain is `ProgramTrace` and the class recommends it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_runs_trace_names_its_input() {
        let svc = service();
        let mut input = Resource::new(Iri::parse("urn:eigenius:test:runprog:in").unwrap());
        input.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String("urn:eigenius:prov:Agent".to_string()),
            ]),
        );
        let resp = svc
            .execute_program("main", working_program(), input)
            .await
            .expect("handler responds");
        assert!(
            resp.get_ref().success,
            "errors: {:?}",
            resp.get_ref().errors
        );

        let ctx = svc
            .get_branch_context("main")
            .await
            .expect("branch context");
        let head = Arc::clone(ctx.read().await.head());
        let trace = head
            .iter_resources()
            .map(|(_, r)| r)
            .find(|r| {
                r.is_a()
                    .iter()
                    .any(|c| c.as_str() == "urn:eigenius:prov:ProgramTrace")
            })
            .expect("the run commits a ProgramTrace");

        match trace.get(&Iri::parse("urn:eigenius:prov:input").unwrap()) {
            Some(crate::ontology::resource::Value::Embedded(r)) => assert_eq!(
                r.id().map(|i| i.as_str()),
                Some("urn:eigenius:test:runprog:in"),
                "the embedded input is the one the run was applied to"
            ),
            other => panic!(
                "prov:input must be the embedded input resource — a run does not commit its \
                 input, so an IRI reference dangles; got {other:?}"
            ),
        }
    }

    /// §2 — a successful run's output carries both records.
    ///
    /// The `ProgramTrace` is provenance and grounds nothing; the `ObservationTrace` is the
    /// `Observed` leaf a sampled outcome is owed. Two resources, two roles.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_successful_run_commits_a_program_trace_and_an_observation_trace() {
        let svc = service();
        let resp = svc
            .execute_program("main", working_program(), Resource::new_embedded())
            .await
            .expect("handler responds");
        assert!(
            resp.get_ref().success,
            "the literal program evaluates; errors: {:?}",
            resp.get_ref().errors
        );

        let ctx = svc
            .get_branch_context("main")
            .await
            .expect("branch context");
        let head = Arc::clone(ctx.read().await.head());
        let mut program_traces = 0;
        let mut observation_traces = 0;
        for (_iri, r) in head.iter_resources() {
            for c in r.is_a() {
                match c.as_str() {
                    "urn:eigenius:prov:ProgramTrace" => program_traces += 1,
                    "urn:eigenius:prov:ObservationTrace" => observation_traces += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(
            (program_traces, observation_traces),
            (1, 1),
            "a run commits exactly one ProgramTrace (provenance) and one ObservationTrace \
             (the Observed leaf on its output)"
        );
    }
}
