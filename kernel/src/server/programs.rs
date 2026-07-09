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
        let (task_context, task_id_str, layer_head, session_id) = match &self.task_store {
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
                (Some(tc), task_id.to_string(), Some(layer_head), session_id)
            }
            None => (None, String::new(), None, uuid::Uuid::nil()),
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
                    // Record the failure if we have a task store.
                    if let (Some(store), Some(head)) = (&self.task_store, layer_head.as_ref()) {
                        if let Some(tid) = task_context.as_ref().map(|tc| tc.task_id) {
                            let mut rec = crate::task::TaskRecord::new_running(
                                session_id,
                                tid,
                                String::new(),
                                String::new(),
                                head.clone(),
                                now_millis(),
                            );
                            rec.status = crate::task::TaskStatus::Failed;
                            rec.updated_at = now_millis();
                            let _ = store.put_task(&rec);
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
            let mut types = match output.get(&is_a_iri) {
                Some(crate::ontology::resource::Value::Array(arr)) => arr.clone(),
                _ => Vec::new(),
            };
            types.push(crate::ontology::resource::Value::String(
                wk::DERIVED_RESOURCE.to_string(),
            ));
            output.set(is_a_iri, crate::ontology::resource::Value::Array(types));
            output.set(
                Iri::parse(wk::DERIVATION).unwrap(),
                crate::ontology::resource::Value::String(trace_iri_str.clone()),
            );
            output.set(
                Iri::parse(wk::EPISTEMIC_STATUS).unwrap(),
                crate::ontology::resource::Value::String(wk::EPISTEMIC_DERIVED.to_string()),
            );
        }

        let mut trace_resource = Resource::new(Iri::parse(&trace_iri_str).unwrap());
        trace_resource.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    "urn:eigenius:reflection:ProgramTrace".to_string(),
                ),
            ]),
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
                Iri::parse("urn:eigenius:reflection:resource").unwrap(),
                crate::ontology::resource::Value::String(out_id.as_str().to_string()),
            );
        }
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:source").unwrap(),
            crate::ontology::resource::Value::String("kernel:run_program".to_string()),
        );
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:timestamp").unwrap(),
            crate::ontology::resource::Value::String(millis_to_iso8601(completed_at_ms)),
        );
        if let Some(prog_id) = program.id() {
            trace_resource.set(
                Iri::parse("urn:eigenius:reflection:program").unwrap(),
                crate::ontology::resource::Value::String(prog_id.as_str().to_string()),
            );
        }
        // Required: trace_tree — serialized tree-structured trace
        if let Some(ref trace) = root_trace {
            let trace_tree = crate::program::trace::trace_to_resource(trace);
            trace_resource.set(
                Iri::parse("urn:eigenius:reflection:trace_tree").unwrap(),
                crate::ontology::resource::Value::Embedded(Box::new(trace_tree)),
            );
        }
        // Required: started_at, completed_at (ISO 8601)
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:started_at").unwrap(),
            crate::ontology::resource::Value::String(millis_to_iso8601(started_at_ms)),
        );
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:completed_at").unwrap(),
            crate::ontology::resource::Value::String(millis_to_iso8601(completed_at_ms)),
        );
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:total_tokens").unwrap(),
            crate::ontology::resource::Value::Integer(total_tokens),
        );
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:executed_steps").unwrap(),
            crate::ontology::resource::Value::Integer(executed_steps),
        );
        // Recommended: universe_level = 0 (traces about domain resources)
        trace_resource.set(
            Iri::parse(crate::ontology::well_known::UNIVERSE_LEVEL).unwrap(),
            crate::ontology::resource::Value::Integer(0),
        );

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
            // Without this the committed `ProgramTrace` points at a target
            // that isn't chain-resident, so the D49 witness emitter can't
            // read its `canonical_proposition` and no `IsDerivedAs` is
            // minted — breaking the D56 wrapped-component derivation path.
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
            // committed `ProgramTrace`'s `reflection:program` reference resolves
            // (reference integrity, Rule 22). Inline `RunProgram` supplies the program
            // as bytes that never otherwise reach the chain; `RunProgramByIri`'s program
            // is already committed (`resolve` finds it), so this is a no-op there. Same
            // provenance fix as the output-resource commit above (`reflection:resource`).
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
                // Validate template references against input type
                let mut template_errors = Vec::new();
                let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
                let input_type_prop = Iri::parse("urn:eigenius:program:input_type").unwrap();
                // `program:input_type` is `data_type: resource`; after
                // canonicalisation the value is `ResourceRef`. Match
                // both shapes via `as_iri_str` so template validation
                // actually runs on production-shaped programs.
                if let (
                    Some(input_type_str),
                    Some(crate::ontology::resource::Value::Embedded(body)),
                ) = (
                    program.get(&input_type_prop).and_then(|v| v.as_iri_str()),
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

                if template_errors.is_empty() {
                    tracing::debug!(
                        { field::OPERATION } = operation::PROGRAM_TYPE_CHECK,
                        program_iri = program.id().map(|i| i.as_str()).unwrap_or(""),
                        program_type = ?typ,
                        "program type-check succeeded"
                    );
                    Ok(Response::new(ValidateProgramResponse {
                        valid: true,
                        errors: Vec::new(),
                        program_type: format!("{typ:?}"),
                    }))
                } else {
                    Ok(Response::new(ValidateProgramResponse {
                        valid: false,
                        errors: template_errors,
                        program_type: format!("{typ:?}"),
                    }))
                }
            }
            Err(e) => Ok(Response::new(ValidateProgramResponse {
                valid: false,
                errors: vec![ValidationError {
                    resource_iri: String::new(),
                    property_iri: String::new(),
                    rule: "type_check".to_string(),
                    message: e,
                    severity: "error".to_string(),
                }],
                program_type: String::new(),
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
