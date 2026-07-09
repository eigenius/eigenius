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

//! Institution / IO effect engine — the [`EffectHooks`] implementation
//! the NbE evaluator delegates its three effectful expression forms to.
//!
//! `InstitutionEngine` carries the runtime state that used to live
//! inline on `EvalCtx::IO` / `EvalCtx::Check`: the layer chain, the
//! D14 institution index + runtime, and (IO only) the component
//! registry, trace store, and the run-boundary collectors. Component
//! dispatch, the D14 §9.3 comorphism pipeline, and D14 §9.2 constraint
//! deciding all live here rather than in the pure kernel. §3.3 of
//! `docs/notes/nbe-reorganization-analysis.md`.

use crate::institution::registry::InstitutionIndex;
use crate::institution::runtime::InstitutionRuntime;
use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::eval::{
    eval_ctx, val_to_resource_value, Decision, EffectHooks, EvalCtx, EvalError,
};
use crate::nbe::term::{Constraint, Exp};
use crate::nbe::val::Val;
use crate::observability::{field, operation};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::program::component::ComponentRegistry;
use crate::program::trace::{ComponentTrace, TraceStore};
use crate::task::TaskContext;
use std::sync::{Arc, Mutex};

/// The runtime effect engine: institution dispatch + IO component
/// invocation. A single struct serves both capability tiers — the IO
/// tier populates every field, the check tier leaves the component
/// registry / trace store / run-boundary collectors empty (so
/// component dispatch is unavailable there, matching the pre-hooks
/// `EvalCtx::Check`).
pub struct InstitutionEngine {
    pub layer: Option<Arc<Layer>>,
    pub institution_index: Option<Arc<InstitutionIndex>>,
    pub institution_runtime: Option<Arc<InstitutionRuntime>>,
    /// IO-only: `None` in check mode (no component dispatch).
    pub registry: Option<Arc<ComponentRegistry>>,
    /// IO-only: deterministic-component memoization cache.
    pub trace_store: Option<Arc<dyn TraceStore>>,
    /// ComponentTraces produced this run, drained at the run-boundary
    /// for the trace-layer commit.
    pub dispatched_traces: Arc<Mutex<Vec<ComponentTrace>>>,
    /// Top-level resources produced this run (comorphism reify outputs)
    /// that the run-boundary commits to the chain.
    pub produced_resources: Arc<Mutex<Vec<Resource>>>,
    /// Optional task context — routes IO through per-task positional
    /// trace keys (D21 §3.2) instead of the content-address cache.
    pub task_context: Option<Arc<TaskContext>>,
}

impl InstitutionEngine {
    /// Full IO engine: component dispatch + comorphism pipeline +
    /// trace/produced-resource collection.
    #[allow(clippy::too_many_arguments)]
    pub fn for_io(
        layer: Arc<Layer>,
        registry: Arc<ComponentRegistry>,
        trace_store: Option<Arc<dyn TraceStore>>,
        dispatched_traces: Arc<Mutex<Vec<ComponentTrace>>>,
        produced_resources: Arc<Mutex<Vec<Resource>>>,
        task_context: Option<Arc<TaskContext>>,
        institution_index: Option<Arc<InstitutionIndex>>,
        institution_runtime: Option<Arc<InstitutionRuntime>>,
    ) -> Self {
        Self {
            layer: Some(layer),
            institution_index,
            institution_runtime,
            registry: Some(registry),
            trace_store,
            dispatched_traces,
            produced_resources,
            task_context,
        }
    }

    /// Check-time engine: institution deciding only (no component
    /// registry / trace collection). Used by the type checker to fire
    /// `Constraint::Institution` predicates at check time.
    pub fn for_check(
        layer: Option<Arc<Layer>>,
        institution_index: Option<Arc<InstitutionIndex>>,
        institution_runtime: Option<Arc<InstitutionRuntime>>,
    ) -> Self {
        Self {
            layer,
            institution_index,
            institution_runtime,
            registry: None,
            trace_store: None,
            dispatched_traces: Arc::new(Mutex::new(Vec::new())),
            produced_resources: Arc::new(Mutex::new(Vec::new())),
            task_context: None,
        }
    }

    /// The D14 §9.3 four-step comorphism pipeline. Mirrors the pre-hooks
    /// `try_institution_invoke`; `ctx` is the effectful context (this
    /// engine) — unused here because every re-entry point (transformation
    /// Component application) goes through `self.run_component`.
    fn run_institution_invoke(
        &self,
        comorphism_iri: &Iri,
        source_val: &Val,
        target_iri: Option<&Iri>,
    ) -> Result<Option<Val>, EvalError> {
        // No institution backing attached → `Ok(None)`, the evaluator
        // yields a passthrough neutral (a bare Pure/no-institution
        // context reaching an InstitutionInvoke during type-check /
        // conversion). Backing present but the comorphism isn't in the
        // index is a structural error, not a passthrough.
        let (Some(index), Some(runtime)) = (
            self.institution_index.as_ref(),
            self.institution_runtime.as_ref(),
        ) else {
            return Ok(None);
        };
        let Some(comorphism) = index.comorphism(comorphism_iri) else {
            return Err(EvalError::InvalidCaseTarget(format!(
                "no Comorphism declaration found in the InstitutionIndex for `{comorphism_iri}`"
            )));
        };

        // Step 1: source-side ExportFormat.
        let export = index
            .export_format(&comorphism.export_format)
            .ok_or_else(|| {
                EvalError::InvalidCaseTarget(format!(
                    "comorphism `{comorphism_iri}`: export_format `{}` not in InstitutionIndex",
                    comorphism.export_format
                ))
            })?;
        let source_inst = runtime.get(&export.institution_ref).ok_or_else(|| {
            EvalError::InvalidCaseTarget(format!(
                "comorphism `{comorphism_iri}`: source institution `{}` not registered in runtime",
                export.institution_ref
            ))
        })?;

        // Marshal the source Val into a Resource for the boundary call —
        // ResourceVal directly; primitives wrapped in a single-property
        // resource (matching the legacy fallback).
        let source_resource = match val_to_resource_value(source_val) {
            crate::ontology::resource::Value::Embedded(r) => *r,
            other => {
                let mut r = Resource::new_embedded();
                r.set(
                    Iri::parse("urn:eigenius:core:value").expect("well-known IRI"),
                    other,
                );
                r
            }
        };

        let storage = crate::layer::LayerStorage::in_memory();
        let head = self.layer.clone().unwrap_or_else(|| {
            Arc::new(
                crate::layer::LayerBuilder::new("__invoke_empty_layer__", None)
                    .build(storage.clone()),
            )
        });
        let exec_ctx = crate::context::ExecutionContext::new(
            Arc::clone(&head),
            "__invoke__",
            crate::context::ExecutionMode::ReadOnly,
            storage,
        );

        // Dereference resource-typed IRI references to embedded form
        // before the boundary call (substrate decoders expect embedded
        // resources, not bare IRI strings). Same fix the FIBER and
        // AutoOnLoad paths apply (D14 §9.1 dispatch).
        let source_resource = crate::institution::marshal::embed_typed_resource_refs_recursively(
            source_resource,
            &head,
        )
        .map_err(|e| {
            EvalError::InvalidCaseTarget(format!(
                "comorphism `{comorphism_iri}`: source-resource embedding failed before \
                     extract_typed via `{}`: {e}",
                export.procedure
            ))
        })?;

        // Step 2: extract typed payload from source-side resource.
        let typed_source = source_inst
            .extract_typed(&export.procedure, &source_resource, &exec_ctx)
            .map_err(|e| {
                EvalError::InvalidCaseTarget(format!(
                    "comorphism `{comorphism_iri}`: extract_typed via `{}` failed: {e}",
                    export.procedure
                ))
            })?;

        // Step 3: apply the transformation Component to the typed
        // payload. The Component must be in the ComponentRegistry, which
        // means this must be the IO engine. If it isn't, the pipeline
        // can't complete — surface the error rather than falling back.
        if self.registry.is_none() {
            return Err(EvalError::ModeError(format!(
                "comorphism `{comorphism_iri}`: InstitutionInvoke requires IO mode \
                 (transformation Component application); found a check-mode engine \
                 with no component registry"
            )));
        }
        let (transformed, _trace) =
            self.run_component(comorphism.transformation.as_str(), &typed_source, None)?;

        // Step 4: target-side ImportFormat reifies the typed result.
        let import = index
            .import_format(&comorphism.import_format)
            .ok_or_else(|| {
                EvalError::InvalidCaseTarget(format!(
                    "comorphism `{comorphism_iri}`: import_format `{}` not in InstitutionIndex",
                    comorphism.import_format
                ))
            })?;
        let target_inst = runtime.get(&import.institution_ref).ok_or_else(|| {
            EvalError::InvalidCaseTarget(format!(
                "comorphism `{comorphism_iri}`: target institution `{}` not registered in runtime",
                import.institution_ref
            ))
        })?;
        let mut target_resource = target_inst
            .reify(&import.procedure, &transformed, &exec_ctx)
            .map_err(|e| {
                EvalError::InvalidCaseTarget(format!(
                    "comorphism `{comorphism_iri}`: reify via `{}` failed: {e}",
                    import.procedure
                ))
            })?;

        // D14 §9.3 step 4: assign a chain-resident IRI. Caller-supplied
        // `target_iri` overrides; otherwise mint a deterministic
        // content-hash IRI so identical reify outputs dedupe on commit.
        let assigned_iri = match target_iri {
            Some(iri) => iri.clone(),
            None => {
                deterministic_run_output_iri("comorphism-output", comorphism_iri, &target_resource)
            }
        };
        target_resource.set_id(Some(assigned_iri));

        // Step 5 (D14 §9.3): post-translation validation invariant —
        // run any AutoOnLoad QueryClasses bound to the produced target
        // class. A `Fails` is a comorphism-implementation bug; surface
        // it rather than committing the bad resource.
        let post_errors = crate::institution::dispatch::dispatch_auto_on_load_for_resource(
            &target_resource,
            index,
            runtime,
            &exec_ctx,
        )
        .flatten_to_errors();
        if !post_errors.is_empty() {
            let reasons = post_errors
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(EvalError::InvalidCaseTarget(format!(
                "comorphism `{comorphism_iri}`: post-translation validation rejected the \
                 reified resource: {reasons}"
            )));
        }

        // Push the IRI'd resource into the run-boundary collector so the
        // server's RunProgram path commits it to the chain.
        self.produced_resources
            .lock()
            .expect("produced_resources mutex poisoned")
            .push(target_resource.clone());

        Ok(Some(Val::ResourceVal(Box::new(target_resource))))
    }

    /// Dispatch a registered IO component call. Converts the Val input
    /// to a Resource, calls the component via the registry, and converts
    /// the result back to a Val. Returns the result plus the produced
    /// `ComponentTrace` (if the dispatch built one — cache/replay hits
    /// return `None`). Mirrors the pre-hooks `dispatch_component`.
    fn run_component(
        &self,
        component_iri: &str,
        input_val: &Val,
        component_arg: Option<&Val>,
    ) -> Result<(Val, Option<ComponentTrace>), EvalError> {
        let Some(registry) = self.registry.as_ref() else {
            return Err(EvalError::ModeError(
                "dispatch_component called outside IO mode".into(),
            ));
        };
        let layer = self
            .layer
            .as_ref()
            .ok_or_else(|| EvalError::ModeError("dispatch_component requires a layer".into()))?;
        let trace_store = &self.trace_store;
        let dispatched_traces = &self.dispatched_traces;
        let task_context = &self.task_context;

        let component = match registry.get(component_iri) {
            Some(c) => c,
            // Unknown component — return input unchanged (identity fallback).
            None => return Ok((input_val.clone(), None)),
        };

        let input_resource = val_to_resource(input_val);
        let mut arg_resource = component_arg.map(val_to_resource);

        // Ontology-driven schema generation on the argument (return
        // value ignored — the kernel no longer uses the table post-hoc).
        let _ = resolve_component_schemas(component_iri, &mut arg_resource, layer);

        // Cache routing is determinism-gated (D21 §3.3): deterministic
        // components use the content-address memo; IO components use
        // positional per-task keys via TaskContext.
        if component.is_io() {
            // D21 §3.2 replay: with a TaskContext, look up this step's
            // trace by (task_id, step_seq); a hit means we're re-running
            // after a crash. `step_seq` is consumed whether hit or miss.
            let replay_slot = task_context.as_ref().map(|tc| (tc.clone(), tc.next_step()));
            if let Some((tc, step)) = replay_slot.as_ref() {
                if let Ok(Some(bytes)) =
                    tc.task_store
                        .get_trace_bytes(&tc.session_id, &tc.task_id, *step)
                {
                    if let Ok(output) = crate::ontology::eigon_cbor::parse_resource_lenient(&bytes)
                    {
                        return Ok((Val::ResourceVal(Box::new(output)), None));
                    }
                    // Corrupt trace bytes — fall through to re-dispatch.
                }
            }

            match component.execute(&input_resource, arg_resource.as_ref(), layer) {
                Ok(result) => {
                    let output = result.output.clone();
                    let ct = ComponentTrace {
                        component: component_iri.to_string(),
                        input_hash: crate::program::trace::compute_trace_key(
                            component_iri,
                            &input_resource,
                        ),
                        argument_hash: None,
                        output: output.clone(),
                        cached: false,
                        metrics: result.metrics,
                    };

                    // Persist the per-task trace via commit_step so bytes
                    // and TaskRecord land atomically (D21 §8). Also build
                    // a Checkpoint for `components:Checkpoint` (D21 §4).
                    if let Some((tc, step)) = replay_slot.as_ref() {
                        let output_bytes = crate::ontology::eigon_cbor::serialize_resource(&output);
                        let is_checkpoint =
                            component_iri == crate::program::component::CHECKPOINT_COMPONENT_IRI;
                        let checkpoint = if is_checkpoint {
                            let state_bytes =
                                crate::ontology::eigon_cbor::serialize_resource(&input_resource);
                            Some(crate::task::Checkpoint {
                                session_id: tc.session_id,
                                task_id: tc.task_id,
                                step_seq: *step,
                                state: state_bytes,
                                created_at: now_millis(),
                            })
                        } else {
                            None
                        };
                        if let Ok(Some(mut record)) =
                            tc.task_store.get_task(&tc.session_id, &tc.task_id)
                        {
                            record.step_seq = step + 1;
                            record.latest_trace_seq = *step;
                            if is_checkpoint {
                                record.last_checkpoint = Some(*step);
                            }
                            record.updated_at = now_millis();
                            if let Err(e) = tc.task_store.commit_step(
                                &record,
                                Some((*step, output_bytes)),
                                checkpoint.as_ref(),
                            ) {
                                tracing::warn!(
                                    { field::OPERATION } = operation::TASK_CHECKPOINT,
                                    { field::ERROR_KIND } = "commit_step_failed",
                                    { field::TASK_ID } = ?tc.task_id,
                                    { field::ERROR_MESSAGE } = %e,
                                    "task commit_step failed"
                                );
                            }
                        }
                    }

                    if let Ok(mut traces) = dispatched_traces.lock() {
                        traces.push(ct.clone());
                    }
                    Ok((Val::ResourceVal(Box::new(output)), Some(ct)))
                }
                Err(e) => {
                    tracing::warn!(
                        { field::OPERATION } = operation::CAPABILITY_DISPATCH,
                        { field::ERROR_KIND } = "dispatch_failed",
                        { field::COMPONENT_IRI } = %component_iri,
                        { field::ERROR_MESSAGE } = %e,
                        "IO component dispatch failed"
                    );
                    Err(EvalError::ComponentDispatchFailed {
                        component_iri: component_iri.to_string(),
                        message: e,
                    })
                }
            }
        } else {
            // Deterministic component — content-address memo is sound
            // and reused cross-task (D21 §3.3).
            let cache_key =
                crate::program::trace::compute_trace_key(component_iri, &input_resource);
            if let Some(store) = trace_store {
                if let Some(cached) = store.get_component_trace(&cache_key) {
                    return Ok((Val::ResourceVal(Box::new(cached.output)), None));
                }
            }

            match component.execute(&input_resource, arg_resource.as_ref(), layer) {
                Ok(result) => {
                    let output = result.output.clone();
                    let ct = ComponentTrace {
                        component: component_iri.to_string(),
                        input_hash: cache_key,
                        argument_hash: None,
                        output: output.clone(),
                        cached: false,
                        metrics: result.metrics,
                    };
                    if let Some(store) = trace_store {
                        store.put_component_trace(cache_key, ct.clone());
                    }
                    if let Ok(mut traces) = dispatched_traces.lock() {
                        traces.push(ct.clone());
                    }
                    Ok((Val::ResourceVal(Box::new(output)), Some(ct)))
                }
                Err(e) => {
                    tracing::warn!(
                        { field::OPERATION } = operation::CAPABILITY_DISPATCH,
                        { field::ERROR_KIND } = "pure_dispatch_failed",
                        { field::COMPONENT_IRI } = %component_iri,
                        { field::ERROR_MESSAGE } = %e,
                        "pure component dispatch failed"
                    );
                    Err(EvalError::ComponentDispatchFailed {
                        component_iri: component_iri.to_string(),
                        message: e,
                    })
                }
            }
        }
    }

    /// D14 §9.2 dispatch for an institution-bound Decidable constraint.
    /// `ctx` is threaded so the argument expressions can be reduced by
    /// re-entering the evaluator. Mirrors the pre-hooks
    /// `try_institution_decide`.
    fn run_institution_decide(
        &self,
        iri: &Iri,
        args: &[Exp],
        rho: &Rho,
        ctx: &EvalCtx,
    ) -> Result<Option<Decision>, EvalError> {
        use crate::institution::registry::DispatchRole;

        let (Some(index), Some(runtime)) = (
            self.institution_index.as_ref(),
            self.institution_runtime.as_ref(),
        ) else {
            return Ok(None);
        };
        let Some(query_class) = index.query_class(iri) else {
            return Ok(None);
        };
        if !query_class
            .dispatch_roles
            .contains(&DispatchRole::Decidable)
        {
            return Ok(None);
        }
        let Some(institution) = runtime.get(&query_class.institution_ref) else {
            return Err(EvalError::InvalidCaseTarget(format!(
                "QueryClass `{iri}` declares institution `{}` not registered in runtime",
                query_class.institution_ref
            )));
        };

        let arg_values: Result<Vec<_>, EvalError> = args
            .iter()
            .map(|a| eval_ctx(a, rho, ctx).map(|v| val_to_resource_value(&v)))
            .collect();
        let arg_values = arg_values?;
        let layer = self.layer.as_ref().ok_or_else(|| {
            EvalError::InvalidCaseTarget(format!(
                "QueryClass `{iri}` Decidable call: no layer attached — cannot resolve input \
                 class `{}` for typed-property marshaling",
                query_class.query_class
            ))
        })?;
        let input = crate::institution::marshal::marshal_decidable_input(
            &query_class.query_class,
            &arg_values,
            layer,
        )
        .map_err(|e| {
            EvalError::InvalidCaseTarget(format!("QueryClass `{iri}` Decidable call: {e}"))
        })?;

        let head = self.layer.clone().unwrap_or_else(|| {
            Arc::new(
                crate::layer::LayerBuilder::new("__decide_empty_layer__", None)
                    .build(crate::layer::LayerStorage::in_memory()),
            )
        });
        let storage = head.storage().clone();
        let exec_ctx = crate::context::ExecutionContext::new(
            head,
            "__decide__",
            crate::context::ExecutionMode::ReadOnly,
            storage,
        );

        let outcome = institution
            .query(&query_class.query_handler, &input, &exec_ctx)
            .map_err(|e| {
                EvalError::InvalidCaseTarget(format!(
                    "QueryClass `{iri}` Decidable handler `{}` failed: {e}",
                    query_class.query_handler
                ))
            })?;

        Ok(Some(parse_verdict(&outcome.output).map_err(|e| {
            EvalError::InvalidCaseTarget(format!(
                "QueryClass `{iri}` Decidable handler returned a non-Verdict result: {e}"
            ))
        })?))
    }
}

impl EffectHooks for InstitutionEngine {
    fn is_component(&self, name: &str) -> bool {
        self.registry
            .as_ref()
            .is_some_and(|r| r.get(name).is_some())
    }

    fn dispatch_component(
        &self,
        name: &str,
        arg_val: &Val,
    ) -> Result<(Val, Option<ComponentTrace>), EvalError> {
        let (input_val, comp_arg) = match arg_val {
            Val::Pair(input, comp_arg) => (input.as_ref().clone(), Some(comp_arg.as_ref())),
            other => (other.clone(), None),
        };
        self.run_component(name, &input_val, comp_arg)
    }

    fn institution_invoke(
        &self,
        comorphism_iri: &Iri,
        source: &Val,
        target_iri: Option<&Iri>,
    ) -> Result<Option<Val>, EvalError> {
        self.run_institution_invoke(comorphism_iri, source, target_iri)
    }

    fn decide_institution(
        &self,
        constraint: &Constraint,
        _value: &Val,
        rho: &Rho,
        ctx: &EvalCtx,
    ) -> Result<Decision, EvalError> {
        match constraint {
            Constraint::Institution { iri, args } => Ok(self
                .run_institution_decide(iri, args, rho, ctx)?
                .unwrap_or(Decision::Undecidable)),
            // Structural constraints are handled in the pure core and
            // never reach the hook.
            _ => Ok(Decision::Undecidable),
        }
    }
}

/// Compute a deterministic content-hash IRI for a resource produced
/// during program execution (D14 §9.3 step 4). Shape:
/// `urn:eigenius:<namespace>:<origin-tail>:<hex>` where `<hex>` is the
/// first 16 hex chars of SHA-256 over the canonical Eigon-CBOR of the
/// resource with `@id` cleared — identical content collides on the
/// same IRI (the dedup we want).
pub fn deterministic_run_output_iri(namespace: &str, origin_iri: &Iri, resource: &Resource) -> Iri {
    use sha2::{Digest, Sha256};
    let mut for_hashing = resource.clone();
    for_hashing.set_id(None);
    let cbor = crate::ontology::eigon_cbor::canonicalize(&for_hashing);
    let digest = Sha256::digest(&cbor);
    let hex = digest
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let tail = origin_iri.as_str().rsplit(':').next().unwrap_or("anon");
    Iri::parse(format!("urn:eigenius:{namespace}:{tail}:{hex}").as_str())
        .expect("deterministic run-output IRI is well-formed")
}

/// Current time in milliseconds since the Unix epoch (0 if before).
fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Convert a Val to a Resource for component dispatch.
fn val_to_resource(val: &Val) -> Resource {
    match val {
        Val::ResourceVal(r) => r.as_ref().clone(),
        Val::Unit => Resource::new_embedded(),
        _ => {
            debug_assert!(
                false,
                "val_to_resource: lossy conversion of {:?} to empty resource",
                val
            );
            Resource::new_embedded()
        }
    }
}

/// Ontology-driven schema resolution for component arguments. Looks up
/// the component's `argument_type` class; for each Class-valued
/// property, generates a JSON Schema and packs it into the argument.
fn resolve_component_schemas(
    component_iri: &str,
    arg_resource: &mut Option<Resource>,
    layer: &Layer,
) -> Option<(crate::program::schema::ShortNameTable, Iri)> {
    let arg = arg_resource.as_mut()?;

    let comp_iri = Iri::parse(component_iri).ok()?;
    let comp_def = layer.resolve(&comp_iri)?;

    let arg_type_prop = Iri::parse("urn:eigenius:program:component:argument_type").ok()?;
    let arg_type_str = comp_def.get(&arg_type_prop)?.as_iri_str()?;
    let arg_type_iri = Iri::parse(arg_type_str).ok()?;
    let arg_type_def = layer.resolve(&arg_type_iri)?;

    let requires_iri = Iri::parse("urn:eigenius:core:requires").ok()?;
    let recommends_iri = Iri::parse("urn:eigenius:core:recommends").ok()?;
    let mut prop_iris = Vec::new();
    if let Some(req) = arg_type_def.get(&requires_iri) {
        prop_iris.extend(req.as_iri_array());
    }
    if let Some(rec) = arg_type_def.get(&recommends_iri) {
        prop_iris.extend(rec.as_iri_array());
    }

    let class_types_iri = Iri::parse("urn:eigenius:core:class_types").ok()?;
    let class_iri = Iri::parse("urn:eigenius:core:Class").ok()?;
    let data_type_iri = Iri::parse("urn:eigenius:core:data_type").ok()?;

    for prop_iri in &prop_iris {
        let prop_def = match layer.resolve(prop_iri) {
            Some(d) => d,
            None => continue,
        };

        let is_class_ref = if let Some(ct) = prop_def.get(&class_types_iri) {
            ct.as_iri_array().contains(&class_iri)
        } else {
            false
        };

        let is_resource = prop_def
            .get(&data_type_iri)
            .and_then(|v| v.as_iri_str())
            .is_some_and(|s| s == "urn:eigenius:core:resource");

        if is_class_ref && is_resource {
            if let Some(class_iri_str) = arg.get(prop_iri).and_then(|v| v.as_iri_str()) {
                if let Ok(schema_class_iri) = Iri::parse(class_iri_str) {
                    match crate::program::schema::schema_for_class(&schema_class_iri, layer) {
                        Ok((json_schema, table)) => {
                            arg.set(
                                prop_iri.clone(),
                                crate::ontology::resource::Value::Json(json_schema),
                            );
                            let table_iri =
                                Iri::parse("urn:eigenius:program:components:short_name_table")
                                    .expect("static IRI is well-formed");
                            arg.set(
                                table_iri,
                                crate::ontology::resource::Value::Json(
                                    table.to_json(&schema_class_iri),
                                ),
                            );
                            return Some((table, schema_class_iri));
                        }
                        Err(e) => {
                            tracing::warn!(
                                { field::OPERATION } = operation::CAPABILITY_DISPATCH,
                                { field::ERROR_KIND } = "schema_generation_failed",
                                { field::CLASS_IRI } = %class_iri_str,
                                { field::ERROR_MESSAGE } = %e,
                                "schema generation failed for class"
                            );
                        }
                    }
                }
            }
        }
    }

    None
}

/// Read a `Verdict` inductive value off a result resource — either an
/// explicit `ctor_name` ("Holds"/"Fails"/"Undecidable") or an `is_a`
/// tagged with one of the three Verdict constructor IRIs.
fn parse_verdict(result: &Resource) -> Result<Decision, String> {
    use crate::ontology::well_known as wk;

    if let Some(ctor) = result
        .get(&Iri::parse(wk::CTOR_NAME).expect("well-known IRI"))
        .and_then(|v| v.as_str().map(str::to_owned))
    {
        return match ctor.as_str() {
            "Holds" => Ok(Decision::Holds),
            "Fails" => Ok(Decision::Fails),
            "Undecidable" => Ok(Decision::Undecidable),
            other => Err(format!("unknown Verdict ctor_name `{other}`")),
        };
    }

    for class_iri in result.is_a() {
        match class_iri.as_str() {
            "urn:eigenius:institution:verdicts:holds" => return Ok(Decision::Holds),
            "urn:eigenius:institution:verdicts:fails" => return Ok(Decision::Fails),
            "urn:eigenius:institution:verdicts:undecidable" => return Ok(Decision::Undecidable),
            _ => {}
        }
    }

    Err(format!(
        "result resource is_a={:?} carries no Verdict marker",
        result.is_a()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::nbe::env::Rho;
    use crate::nbe::eval::{eval, eval_ctx, EvalCtx};
    use crate::nbe::term::Exp;
    use crate::nbe::val::{Neut, Val};
    use crate::program::component::ComponentRegistry;
    // --- Exp::InstitutionInvoke eval dispatch ---

    /// Pure-mode `Exp::InstitutionInvoke` produces a passthrough
    /// neutral when no institution context is attached. Verified
    /// in pure mode (no `EvalCtx::IO` / `Check`); the legacy
    /// `__institution_invoke_no_registry:<cm>` neutral name keeps
    /// the surface stable.
    #[test]
    fn institution_invoke_without_context_produces_passthrough_neutral() {
        let src_iri = Iri::parse("urn:eigenius:test:src").unwrap();
        let src_resource = crate::ontology::resource::Resource::new(src_iri);
        let source = Exp::EigonResource(Box::new(src_resource));

        let exp = Exp::InstitutionInvoke {
            comorphism_iri: Iri::parse("urn:eigenius:test:marker_cm").unwrap(),
            source: Box::new(source),
            target_iri: None,
        };
        let v = eval(&exp, &Rho::Nil).expect("eval");
        match v {
            Val::Nt(Neut::Gen(_, name)) => {
                assert!(name.starts_with("__institution_invoke_no_registry"));
            }
            other => panic!("expected passthrough neutral, got {other:?}"),
        }
    }

    // ─── four-step InstitutionInvoke pipeline ──────────────────

    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::ontology::well_known as wk;
    use std::sync::{Arc, Mutex};

    /// In-process Institution that records every dispatched call so a
    /// test can assert on the four-step pipeline routing — extract on
    /// the source side, reify on the target side. Both sides are the
    /// same institution here for setup brevity; production
    /// deployments cross institution boundaries.
    struct PipelineLogger {
        iri: Iri,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Institution for PipelineLogger {
        fn institution_iri(&self) -> &Iri {
            &self.iri
        }

        fn extract_typed(
            &self,
            procedure_iri: &Iri,
            resource: &crate::ontology::resource::Resource,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<Val, crate::institution::error::InstitutionError> {
            let id = resource
                .id()
                .map(|i| i.as_str().to_string())
                .unwrap_or_else(|| "<embedded>".to_string());
            self.log
                .lock()
                .unwrap()
                .push(format!("extract@{procedure_iri}({id})"));
            // Tag the resource with a provenance marker so reify can
            // confirm it received the extracted payload.
            let mut tagged = resource.clone();
            tagged.set(
                Iri::parse("urn:eigenius:test:pipeline:extracted_via").expect("well-known IRI"),
                crate::ontology::resource::Value::String(procedure_iri.as_str().into()),
            );
            Ok(Val::ResourceVal(Box::new(tagged)))
        }

        fn reify(
            &self,
            procedure_iri: &Iri,
            value: &Val,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<crate::ontology::resource::Resource, crate::institution::error::InstitutionError>
        {
            self.log
                .lock()
                .unwrap()
                .push(format!("reify@{procedure_iri}"));
            let payload = match value {
                Val::ResourceVal(r) => r.as_ref().clone(),
                other => panic!("PipelineLogger.reify: expected ResourceVal, got {other:?}"),
            };
            // Tag the produced resource so the test can assert reify ran.
            let mut tagged = payload;
            tagged.set(
                Iri::parse("urn:eigenius:test:pipeline:reified_via").expect("well-known IRI"),
                crate::ontology::resource::Value::String(procedure_iri.as_str().into()),
            );
            Ok(tagged)
        }
    }

    fn build_pipeline_chain() -> Arc<crate::layer::Layer> {
        // Layer holds: Institution + ExportFormat + ImportFormat +
        // Comorphism declarations. Same institution_ref for source
        // and target — deliberately, to keep the runtime registry
        // setup minimal.
        let mut b = crate::layer::LayerBuilder::new("test", None);

        let mut institution = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:pipe:inst").unwrap(),
        );
        institution.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    "urn:eigenius:institution:Institution".into(),
                ),
            ]),
        );
        institution.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:inst".into()),
        );
        institution.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            crate::ontology::resource::Value::String("Pipeline test institution".into()),
        );
        b.add_resource(institution).unwrap();

        let mut export = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:pipe:export").unwrap(),
        );
        export.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::EXPORT_FORMAT_CLASS.into()),
            ]),
        );
        export.set(
            Iri::parse(wk::FROM_CLASS).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:SourceClass".into()),
        );
        export.set(
            Iri::parse(wk::PAYLOAD_TYPE).unwrap(),
            crate::ontology::resource::Value::String(wk::FLOAT.into()),
        );
        export.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:inst".into()),
        );
        export.set(
            Iri::parse(wk::PROCEDURE).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:proc:extract".into()),
        );
        b.add_resource(export).unwrap();

        let mut import = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:pipe:import").unwrap(),
        );
        import.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::IMPORT_FORMAT_CLASS.into()),
            ]),
        );
        import.set(
            Iri::parse(wk::TO_CLASS).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:TargetClass".into()),
        );
        import.set(
            Iri::parse(wk::PAYLOAD_TYPE).unwrap(),
            crate::ontology::resource::Value::String(wk::FLOAT.into()),
        );
        import.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:inst".into()),
        );
        import.set(
            Iri::parse(wk::PROCEDURE).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:proc:reify".into()),
        );
        b.add_resource(import).unwrap();

        let mut comorphism = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:pipe:cm").unwrap(),
        );
        comorphism.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::COMORPHISM.into()),
            ]),
        );
        comorphism.set(
            Iri::parse(wk::EXPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:export".into()),
        );
        comorphism.set(
            Iri::parse(wk::TRANSFORMATION).unwrap(),
            // No real Component — dispatch_component falls back to
            // identity for unknown component IRIs, which is what we
            // want for this structural test.
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:pipe:identity_transform".into(),
            ),
        );
        comorphism.set(
            Iri::parse(wk::IMPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:import".into()),
        );
        comorphism.set(
            Iri::parse(wk::EXACT).unwrap(),
            crate::ontology::resource::Value::Boolean(false),
        );
        b.add_resource(comorphism).unwrap();

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    fn build_pipeline_ctx(log: Arc<Mutex<Vec<String>>>) -> (EvalCtx, Arc<InstitutionIndex>) {
        let layer = build_pipeline_chain();
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "index errors: {errors:?}");
        let idx = Arc::new(idx);

        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(PipelineLogger {
                iri: Iri::parse("urn:eigenius:test:pipe:inst").unwrap(),
                log,
            }))
            .unwrap();

        let ctx = {
            let __engine_layer = layer;
            EvalCtx::effectful(
                Some(Arc::clone(&__engine_layer)),
                Arc::new(crate::institution::eval_hooks::InstitutionEngine::for_io(
                    Arc::clone(&__engine_layer),
                    Arc::new(ComponentRegistry::default()),
                    None,
                    Arc::new(Mutex::new(Vec::new())),
                    Arc::new(Mutex::new(Vec::new())),
                    None,
                    Some(Arc::clone(&idx)),
                    Some(Arc::new(runtime)),
                )),
            )
        };
        (ctx, idx)
    }

    #[test]
    fn institution_invoke_runs_four_step_pipeline_end_to_end() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (ctx, _idx) = build_pipeline_ctx(Arc::clone(&log));

        let source = Exp::EigonResource(Box::new(crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:pipe:source_instance").unwrap(),
        )));
        let exp = Exp::InstitutionInvoke {
            comorphism_iri: Iri::parse("urn:eigenius:test:pipe:cm").unwrap(),
            source: Box::new(source),
            target_iri: None,
        };
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("institution pipeline eval");
        let result = match v {
            Val::ResourceVal(r) => *r,
            other => panic!("expected ResourceVal from pipeline, got {other:?}"),
        };

        // Extract → identity-transform → reify all ran:
        let extracted_via = result
            .get(&Iri::parse("urn:eigenius:test:pipeline:extracted_via").unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(
            extracted_via.as_deref(),
            Some("urn:eigenius:test:pipe:proc:extract"),
            "extract_typed should have tagged the resource with the export procedure IRI"
        );
        let reified_via = result
            .get(&Iri::parse("urn:eigenius:test:pipeline:reified_via").unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(
            reified_via.as_deref(),
            Some("urn:eigenius:test:pipe:proc:reify"),
            "reify should have tagged the resource with the import procedure IRI"
        );

        // Order: extract first, reify last — confirms the four-step
        // pipeline shape (transformation in between is the identity
        // fallback for the unregistered Component IRI).
        let trail = log.lock().unwrap().clone();
        assert_eq!(
            trail,
            vec![
                "extract@urn:eigenius:test:pipe:proc:extract(urn:eigenius:test:pipe:source_instance)".to_string(),
                "reify@urn:eigenius:test:pipe:proc:reify".to_string(),
            ]
        );
    }

    #[test]
    fn institution_invoke_missing_format_surfaces_typed_error() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (ctx, idx) = build_pipeline_ctx(Arc::clone(&log));

        // Sanity: the index has the comorphism we'll reference.
        assert!(idx
            .comorphism(&Iri::parse("urn:eigenius:test:pipe:cm").unwrap())
            .is_some());

        // Build a *separate* comorphism that points at an
        // ExportFormat IRI not in the index. Must drop it into a new
        // layer above the existing chain so the InstitutionIndex can
        // still see the original declarations.
        let mut top =
            crate::layer::LayerBuilder::new("orphan_cm", Some(Arc::clone(ctx.layer().unwrap())));
        let mut orphan = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:pipe:orphan_cm").unwrap(),
        );
        orphan.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::COMORPHISM.into()),
            ]),
        );
        orphan.set(
            Iri::parse(wk::EXPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:not_in_index".into()),
        );
        orphan.set(
            Iri::parse(wk::TRANSFORMATION).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:pipe:identity_transform".into(),
            ),
        );
        orphan.set(
            Iri::parse(wk::IMPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:pipe:import".into()),
        );
        top.add_resource(orphan).unwrap();
        let new_layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        // Re-derive the index over the new chain so it picks up the
        // orphan comorphism.
        let (new_idx, _errs) = InstitutionIndex::from_layer(&new_layer);
        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(PipelineLogger {
                iri: Iri::parse("urn:eigenius:test:pipe:inst").unwrap(),
                log,
            }))
            .unwrap();
        let ctx = {
            let __engine_layer = new_layer;
            EvalCtx::effectful(
                Some(Arc::clone(&__engine_layer)),
                Arc::new(crate::institution::eval_hooks::InstitutionEngine::for_io(
                    Arc::clone(&__engine_layer),
                    Arc::new(ComponentRegistry::default()),
                    None,
                    Arc::new(Mutex::new(Vec::new())),
                    Arc::new(Mutex::new(Vec::new())),
                    None,
                    Some(Arc::new(new_idx)),
                    Some(Arc::new(runtime)),
                )),
            )
        };

        let exp = Exp::InstitutionInvoke {
            comorphism_iri: Iri::parse("urn:eigenius:test:pipe:orphan_cm").unwrap(),
            source: Box::new(Exp::EigonResource(Box::new(
                crate::ontology::resource::Resource::new(
                    Iri::parse("urn:eigenius:test:src").unwrap(),
                ),
            ))),
            target_iri: None,
        };
        let err = eval_ctx(&exp, &Rho::Nil, &ctx).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("export_format")
                && msg.contains("not_in_index")
                && msg.contains("not in InstitutionIndex"),
            "expected typed error about the missing ExportFormat; got: {msg}"
        );
    }

    // ─── NativeDecide institution dispatch ─────────────────────────────────

    /// In-process Institution that answers Decidable QueryClasses by
    /// inspecting the `decide_args` array on the input resource and
    /// returning a Verdict resource. The verdict is configured at
    /// construction time so the test can assert on each branch.
    struct VerdictInstitution {
        iri: Iri,
        verdict_class: &'static str,
    }

    impl Institution for VerdictInstitution {
        fn institution_iri(&self) -> &Iri {
            &self.iri
        }
        fn extract_typed(
            &self,
            _procedure_iri: &Iri,
            _resource: &crate::ontology::resource::Resource,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<Val, crate::institution::error::InstitutionError> {
            unreachable!("VerdictInstitution exposes no ExportFormats")
        }
        fn reify(
            &self,
            _procedure_iri: &Iri,
            _value: &Val,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<crate::ontology::resource::Resource, crate::institution::error::InstitutionError>
        {
            unreachable!("VerdictInstitution exposes no ImportFormats")
        }
        fn query(
            &self,
            _procedure_iri: &Iri,
            input: &crate::ontology::resource::Resource,
            _ctx: &crate::context::ExecutionContext,
        ) -> Result<
            crate::institution::runtime::QueryOutcome,
            crate::institution::error::InstitutionError,
        > {
            // Confirm the kernel stamped `is_a` to the input class IRI
            // (Phase 19d.7: positional args ride on typed required
            // properties, not on a `decide_args` array; the
            // structural marker we can rely on regardless of arity is
            // the auto-stamped is_a).
            let _ = input
                .get(&Iri::parse(crate::ontology::well_known::IS_A).unwrap())
                .expect("kernel must stamp is_a onto the synthetic input resource");
            let mut verdict = crate::ontology::resource::Resource::new_embedded();
            verdict.set(
                Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
                crate::ontology::resource::Value::Array(vec![
                    crate::ontology::resource::Value::String(self.verdict_class.into()),
                ]),
            );
            Ok(crate::institution::runtime::QueryOutcome::from_output(
                verdict,
            ))
        }
    }

    fn build_decide_ctx(verdict_class: &'static str, arg_count: usize) -> EvalCtx {
        use crate::ontology::well_known as wk;
        let mut b = crate::layer::LayerBuilder::new("test", None);

        let inst_iri = "urn:eigenius:test:decide:inst";
        let constraint_iri = "urn:eigenius:test:decide:has_property";
        let input_class = "urn:eigenius:test:decide:Subject";

        // Phase 19d.7: the input class must declare typed required
        // properties for the kernel's typed-property marshaling to
        // populate. Each arg slot is its own Property resource named
        // `arg_N`, listed in `requires` in declaration order.
        let mut requires = Vec::with_capacity(arg_count);
        for n in 0..arg_count {
            let prop_iri = format!("{input_class}:arg_{n}");
            let mut p = crate::ontology::resource::Resource::new(Iri::parse(&prop_iri).unwrap());
            p.set(
                Iri::parse(wk::IS_A).unwrap(),
                crate::ontology::resource::Value::Array(vec![
                    crate::ontology::resource::Value::String(wk::PROPERTY.into()),
                ]),
            );
            b.add_resource(p).unwrap();
            requires.push(crate::ontology::resource::Value::String(prop_iri));
        }
        let mut input_class_res =
            crate::ontology::resource::Resource::new(Iri::parse(input_class).unwrap());
        input_class_res.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::CLASS.into()),
            ]),
        );
        input_class_res.set(
            Iri::parse(wk::REQUIRES).unwrap(),
            crate::ontology::resource::Value::Array(requires),
        );
        b.add_resource(input_class_res).unwrap();

        // QueryClass declaring Decidable role for `constraint_iri`.
        let mut qc = crate::ontology::resource::Resource::new(Iri::parse(constraint_iri).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::QUERY_CLASS_CLASS.into()),
            ]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            crate::ontology::resource::Value::String(input_class.into()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            crate::ontology::resource::Value::String(wk::VERDICT.into()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::DISPATCH_DECIDABLE.into()),
            ]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:decide:proc:check".into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            crate::ontology::resource::Value::String(inst_iri.into()),
        );
        b.add_resource(qc).unwrap();

        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");

        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(VerdictInstitution {
                iri: Iri::parse(inst_iri).unwrap(),
                verdict_class,
            }))
            .unwrap();

        {
            let __engine_layer = layer;
            EvalCtx::effectful(
                Some(Arc::clone(&__engine_layer)),
                Arc::new(crate::institution::eval_hooks::InstitutionEngine::for_io(
                    Arc::clone(&__engine_layer),
                    Arc::new(ComponentRegistry::default()),
                    None,
                    Arc::new(Mutex::new(Vec::new())),
                    Arc::new(Mutex::new(Vec::new())),
                    None,
                    Some(Arc::new(idx)),
                    Some(Arc::new(runtime)),
                )),
            )
        }
    }

    #[test]
    fn native_decide_holds_reduces_to_refl() {
        let ctx = build_decide_ctx("urn:eigenius:institution:verdicts:holds", 1);
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:decide:has_property").unwrap(),
            args: vec![Exp::Unit],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Refl(_) => {}
            other => panic!("expected Refl from Holds verdict, got {other:?}"),
        }
    }

    #[test]
    fn native_decide_fails_produces_failing_neutral() {
        let ctx = build_decide_ctx("urn:eigenius:institution:verdicts:fails", 0);
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:decide:has_property").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(Neut::Gen(_, name)) if name == "__constraint_failed" => {}
            other => panic!("expected __constraint_failed neutral, got {other:?}"),
        }
    }

    #[test]
    fn native_decide_undecidable_produces_passthrough_neutral() {
        let ctx = build_decide_ctx("urn:eigenius:institution:verdicts:undecidable", 0);
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:decide:has_property").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(Neut::Gen(_, name)) if name == "__constraint_undecidable" => {}
            other => panic!("expected __constraint_undecidable neutral, got {other:?}"),
        }
    }

    #[test]
    fn native_decide_falls_back_to_legacy_when_no_decidable_query_class() {
        // Constraint IRI not in the institution index → fallback to legacy
        // institutions registry. With neither configured the legacy
        // path returns Undecidable (passthrough).
        let layer = Arc::new(
            crate::layer::LayerBuilder::new("test", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let (idx, _) = InstitutionIndex::from_layer(&layer);
        let ctx = {
            let __engine_layer = layer;
            EvalCtx::effectful(
                Some(Arc::clone(&__engine_layer)),
                Arc::new(crate::institution::eval_hooks::InstitutionEngine::for_io(
                    Arc::clone(&__engine_layer),
                    Arc::new(ComponentRegistry::default()),
                    None,
                    Arc::new(Mutex::new(Vec::new())),
                    Arc::new(Mutex::new(Vec::new())),
                    None,
                    Some(Arc::new(idx)),
                    Some(Arc::new(InstitutionRuntime::new())),
                )),
            )
        };
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:decide:not_declared").unwrap(),
            args: vec![],
        };
        let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("eval");
        match v {
            Val::Nt(Neut::Gen(_, name)) if name == "__constraint_undecidable" => {}
            other => panic!("expected fallback Undecidable, got {other:?}"),
        }
    }

    // ──────────────────────────────────────────────────────────────────
    // D48 Phase G — iota reduction on indexed inductives (end-to-end)
    // ──────────────────────────────────────────────────────────────────
}
