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

//! D14 institution / IO component dispatch engine: comorphism
//! invocation, component dispatch with tracing, and constraint
//! deciding (D14 §9.2). Split from `eval.rs`; extraction out of the
//! NbE core behind effect hooks is §3.3 of
//! `docs/notes/nbe-reorganization-analysis.md`.

use super::marshal::val_to_resource_value;
use super::{eval_ctx, EvalCtx, EvalError};
use crate::nbe::env::Rho;
use crate::nbe::term::Exp;
use crate::nbe::val::Val;
use crate::observability::{field, operation};
use crate::ontology::iri::Iri;
use crate::program::trace::ComponentTrace;
use std::sync::Arc;

/// Compute a deterministic content-hash IRI for a resource produced
/// during program execution (D14 §9.3 step 4 — comorphism reify
/// outputs; the program-run final output).
///
/// Shape: `urn:eigenius:<namespace>:<origin-tail>:<hex>` where
/// `<hex>` is the first 16 hex chars of SHA-256 over the canonical
/// Eigon-CBOR of the resource with `@id` cleared. Two calls that
/// produce identical resource content collide on the same IRI —
/// that is the dedup we want.
pub(crate) fn deterministic_run_output_iri(
    namespace: &str,
    origin_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
) -> Iri {
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

/// D14 four-step InstitutionInvoke pipeline.
///
/// Returns:
/// - `Ok(Some(translated))` if the comorphism IRI resolved through the
///   D14 index and the pipeline ran end-to-end.
/// - `Ok(None)` if the D14 index / runtime aren't attached to the
///   evaluation context, or the comorphism IRI isn't found in the
///   index — the caller falls back to legacy.
/// - `Err(_)` if the index *did* find the comorphism but a downstream
///   step failed (missing format, missing institution, transformation
///   error, marshalling error). Failure of a configured pipeline is
///   not a reason to fall back — the comorphism is structurally
///   broken and the caller should surface the error.
pub(super) fn try_d14_institution_invoke(
    comorphism_iri: &Iri,
    source_val: &Val,
    target_iri: Option<&Iri>,
    ctx: &EvalCtx,
) -> Result<Option<Val>, EvalError> {
    let (Some(index), Some(runtime)) = (ctx.institution_index(), ctx.institution_runtime()) else {
        return Ok(None);
    };
    let Some(comorphism) = index.comorphism(comorphism_iri) else {
        return Ok(None);
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
    // M5 supports ResourceVal directly; primitives are wrapped in a
    // single-property resource (matching the legacy fallback).
    let source_resource = match val_to_resource_value(source_val) {
        crate::ontology::resource::Value::Embedded(r) => *r,
        other => {
            let mut r = crate::ontology::resource::Resource::new_embedded();
            r.set(
                Iri::parse("urn:eigenius:core:value").expect("well-known IRI"),
                other,
            );
            r
        }
    };

    let storage = crate::layer::LayerStorage::in_memory();
    let head = ctx.layer().cloned().unwrap_or_else(|| {
        Arc::new(
            crate::layer::LayerBuilder::new("__invoke_empty_layer__", None).build(storage.clone()),
        )
    });
    let exec_ctx = crate::context::ExecutionContext::new(
        Arc::clone(&head),
        "__invoke__",
        crate::context::ExecutionMode::ReadOnly,
        storage,
    );

    // The chain stores resource-typed properties as IRI references
    // (post `canonicalise_resource_refs`). Substrate-runtime decoders
    // expect each nested chain resource embedded — a bare IRI string
    // hits no decoder and surfaces as `MethodError(decode_X, ("urn:...",))`
    // at the boundary. Walk the source resource and dereference every
    // resource-typed IRI to its embedded form before handing off.
    // Same fix the FIBER path applies (`embed_typed_resource_param`)
    // and the AutoOnLoad path applies (D14 §9.1 dispatch).
    let source_resource =
        crate::institution::marshal::embed_typed_resource_refs_recursively(source_resource, &head)
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

    // Step 3: apply the transformation Component to the typed payload.
    // The Component must be in the kernel's ComponentRegistry, which
    // means the eval context must be IO mode. If it isn't, the four-
    // step pipeline can't complete here — surface the error rather
    // than silently falling back.
    if !matches!(ctx, EvalCtx::IO { .. }) {
        return Err(EvalError::ModeError(format!(
            "comorphism `{comorphism_iri}`: D14 InstitutionInvoke requires IO mode \
             (transformation Component application); found {ctx_kind}",
            ctx_kind = match ctx {
                EvalCtx::Pure => "Pure",
                EvalCtx::Read { .. } => "Read",
                EvalCtx::Check { .. } => "Check",
                EvalCtx::IO { .. } => unreachable!(),
            }
        )));
    }
    let transformed =
        dispatch_component(comorphism.transformation.as_str(), &typed_source, None, ctx)?;

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

    // D14 §9.3 step 4: assign a chain-resident IRI to the produced
    // target-class resource. Caller-supplied `target_iri` (e.g. from
    // EigenQL `INTO`) overrides; otherwise the kernel mints a
    // deterministic content-hash IRI so identical reify outputs
    // dedupe naturally on commit.
    let assigned_iri = match target_iri {
        Some(iri) => iri.clone(),
        None => deterministic_run_output_iri("comorphism-output", comorphism_iri, &target_resource),
    };
    target_resource.set_id(Some(assigned_iri));

    // Step 5 (D14 §9.3): post-translation validation invariant. Run
    // any AutoOnLoad QueryClasses bound to the produced target
    // resource's class. A `Fails` here indicates the comorphism
    // produced a target-invalid result — a comorphism-implementation
    // bug, not a user error. Surface as a typed error rather than
    // committing the bad resource.
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
    // server's RunProgram path commits it to the chain alongside
    // ProgramTrace + ComponentTrace observability resources.
    if let EvalCtx::IO {
        produced_resources, ..
    } = ctx
    {
        produced_resources
            .lock()
            .expect("produced_resources mutex poisoned")
            .push(target_resource.clone());
    }

    Ok(Some(Val::ResourceVal(Box::new(target_resource))))
}

/// Dispatch an IO component call.
///
/// Converts the Val argument to a Resource, calls the component via the
/// registry, and converts the result back to a Val.
pub(super) fn dispatch_component(
    component_iri: &str,
    input_val: &Val,
    component_arg: Option<&Val>,
    ctx: &EvalCtx,
) -> Result<Val, EvalError> {
    let (registry, layer, trace_store, dispatched_traces, task_context) = match ctx {
        EvalCtx::IO {
            registry,
            layer,
            trace_store,
            dispatched_traces,
            task_context,
            ..
        } => (
            registry,
            layer,
            trace_store,
            dispatched_traces,
            task_context,
        ),
        _ => {
            return Err(EvalError::ModeError(
                "dispatch_component called outside IO mode".into(),
            ))
        }
    };

    let component = match registry.get(component_iri) {
        Some(c) => c,
        None => {
            // Unknown component — return input unchanged (identity fallback)
            return Ok(input_val.clone());
        }
    };

    // Convert Val to Resource for the component interface
    let input_resource = val_to_resource(input_val);
    let mut arg_resource = component_arg.map(val_to_resource);

    // Ontology-driven schema generation:
    // Look up the component definition, find its argument_type class,
    // scan argument properties for Class-valued references that need
    // JSON Schema generation. Phase 18e.2: the function also embeds
    // the short-name table on the argument for the orchestrator's
    // CompleteJson handler. Return value is ignored — the kernel no
    // longer uses the table post-hoc.
    let _ = resolve_component_schemas(component_iri, &mut arg_resource, layer);

    // Cache routing is determinism-gated (D21 §3.3):
    //   - Deterministic components (!is_io): content-address memo —
    //     identical input yields identical output, so cross-task
    //     reuse is sound.
    //   - IO components: positional per-task keys only, via
    //     TaskContext. The content-address cache would silently
    //     collapse distinct observations into one (the Phase-9a bug
    //     D21 §1 motivates); without a TaskContext, IO is simply
    //     re-dispatched every time rather than mis-cached.
    if component.is_io() {
        // D21 §3.2 replay path: when a TaskContext is attached, look
        // up this step's trace by (task_id, step_seq) first. A hit
        // means we're re-running after a crash and this IO call has
        // already completed — return the cached output without
        // re-dispatching.
        //
        // `step_seq` is consumed (fetch_add) whether the lookup hits
        // or misses — it's the monotonic position in the task's IO
        // log.
        let replay_slot = task_context.as_ref().map(|tc| (tc.clone(), tc.next_step()));
        if let Some((tc, step)) = replay_slot.as_ref() {
            if let Ok(Some(bytes)) =
                tc.task_store
                    .get_trace_bytes(&tc.session_id, &tc.task_id, *step)
            {
                // `parse_resource_lenient` — IO component outputs are
                // often anonymous embedded Resources with no `@id`,
                // which the strict parser rejects.
                if let Ok(output) = crate::ontology::eigon_cbor::parse_resource_lenient(&bytes) {
                    return Ok(Val::ResourceVal(Box::new(output)));
                }
                // Corrupt trace bytes — fall through to re-dispatch.
            }
        }

        match component.execute(&input_resource, arg_resource.as_ref(), layer) {
            Ok(result) => {
                // Phase 18e.2: orchestrator-side `CompleteJson` handler now
                // applies the `ShortNameTable` translation before returning,
                // so the kernel sees a properly-IRI-keyed Eigon resource
                // here and no post-hoc translation is needed.
                let output = result.output.clone();

                let ct = ComponentTrace {
                    component: component_iri.to_string(),
                    // input_hash is retained for reflection / audit
                    // even though IO dispatch no longer routes
                    // through the content-address cache.
                    input_hash: crate::program::trace::compute_trace_key(
                        component_iri,
                        &input_resource,
                    ),
                    argument_hash: None,
                    output: output.clone(),
                    cached: false,
                    metrics: result.metrics,
                };

                // Persist the per-task trace via commit_step so the
                // output bytes and updated TaskRecord land atomically
                // (D21 §8 step atomicity). Without a TaskContext
                // there is no safe place to cache an IO output, so
                // we just record the trace for the layer commit.
                //
                // If this dispatch was `components:Checkpoint`, also
                // build a Checkpoint alongside the trace — the
                // commit_step method writes all three (trace, record,
                // checkpoint) atomically (D21 §4).
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

                // Record for trace layer commit
                if let Ok(mut traces) = dispatched_traces.lock() {
                    traces.push(ct);
                }
                Ok(Val::ResourceVal(Box::new(output)))
            }
            Err(e) => {
                tracing::warn!(
                    { field::OPERATION } = operation::CAPABILITY_DISPATCH,
                    { field::ERROR_KIND } = "dispatch_failed",
                    { field::COMPONENT_IRI } = %component_iri,
                    { field::ERROR_MESSAGE } = %e,
                    "IO component dispatch failed"
                );
                // Propagate the failure rather than masking it with an
                // empty resource. The pre-fix behaviour fed a
                // properties-less, is_a-less embedded resource into
                // downstream `Construct` fields and let the chain
                // validator catch it later — but the validator can
                // only say "is_a [] doesn't match class_types"; the
                // user has no way to trace that back to the dispatch
                // failure that produced the empty resource. With the
                // error propagated, `execute_program` surfaces the
                // original component error directly.
                Err(EvalError::ComponentDispatchFailed {
                    component_iri: component_iri.to_string(),
                    message: e,
                })
            }
        }
    } else {
        // Deterministic component — content-address memo is sound
        // and reused cross-task (D21 §3.3). Identical input across
        // two tasks hits the same entry, amortizing the dispatch.
        let cache_key = crate::program::trace::compute_trace_key(component_iri, &input_resource);
        if let Some(store) = trace_store {
            if let Some(cached) = store.get_component_trace(&cache_key) {
                return Ok(Val::ResourceVal(Box::new(cached.output)));
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
                    traces.push(ct);
                }
                Ok(Val::ResourceVal(Box::new(output)))
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

/// Current time in milliseconds since the Unix epoch. Falls back to 0
/// if the system clock is before the epoch (shouldn't happen in
/// practice).
fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Convert a Val to a Resource for component dispatch.
fn val_to_resource(val: &Val) -> crate::ontology::resource::Resource {
    match val {
        Val::ResourceVal(r) => r.as_ref().clone(),
        Val::Unit => crate::ontology::resource::Resource::new_embedded(),
        _ => {
            // Lossy conversion — not all Vals map to Resources.
            // Fire in debug builds so tests surface unexpected Val types
            // reaching component dispatch (Phase 10c, defence-in-depth layer 3).
            debug_assert!(
                false,
                "val_to_resource: lossy conversion of {:?} to empty resource",
                val
            );
            crate::ontology::resource::Resource::new_embedded()
        }
    }
}

/// Ontology-driven schema resolution for component arguments.
///
/// Looks up the component's `argument_type` class in the layer chain.
/// For each property on that class whose value in the actual argument
/// resolves to a Class IRI, generates a JSON Schema and packs it into
/// the argument. Returns the ShortNameTable and class IRI if schema was generated.
fn resolve_component_schemas(
    component_iri: &str,
    arg_resource: &mut Option<crate::ontology::resource::Resource>,
    layer: &crate::layer::Layer,
) -> Option<(crate::program::schema::ShortNameTable, Iri)> {
    let arg = arg_resource.as_mut()?;

    // Look up the component definition
    let comp_iri = Iri::parse(component_iri).ok()?;
    let comp_def = layer.resolve(&comp_iri)?;

    // Get the argument_type class
    let arg_type_prop = Iri::parse("urn:eigenius:program:component:argument_type").ok()?;
    // `component:argument_type` is a `data_type: resource` property —
    // canonicalised to `ResourceRef` at LayerBuilder::build time, so
    // read via `as_iri_str` (which handles both String and ResourceRef).
    // `as_str` here used to silently return None for canonicalised
    // component definitions, which short-circuited the entire schema
    // resolution and surfaced as "CompleteJson requires output_schema
    // in component argument" at the orchestrator handler.
    let arg_type_str = comp_def.get(&arg_type_prop)?.as_iri_str()?;
    let arg_type_iri = Iri::parse(arg_type_str).ok()?;
    let arg_type_def = layer.resolve(&arg_type_iri)?;

    // Collect all property IRIs from requires + recommends on the argument class
    let requires_iri = Iri::parse("urn:eigenius:core:requires").ok()?;
    let recommends_iri = Iri::parse("urn:eigenius:core:recommends").ok()?;
    let mut prop_iris = Vec::new();
    if let Some(req) = arg_type_def.get(&requires_iri) {
        prop_iris.extend(req.as_iri_array());
    }
    if let Some(rec) = arg_type_def.get(&recommends_iri) {
        prop_iris.extend(rec.as_iri_array());
    }

    // For each property, check if its value in the actual argument references a Class
    let class_types_iri = Iri::parse("urn:eigenius:core:class_types").ok()?;
    let class_iri = Iri::parse("urn:eigenius:core:Class").ok()?;
    let data_type_iri = Iri::parse("urn:eigenius:core:data_type").ok()?;

    for prop_iri in &prop_iris {
        // Look up the property definition
        let prop_def = match layer.resolve(prop_iri) {
            Some(d) => d,
            None => continue,
        };

        // Check if this property has class_types: [Class] (meaning it references a class)
        let is_class_ref = if let Some(ct) = prop_def.get(&class_types_iri) {
            ct.as_iri_array().contains(&class_iri)
        } else {
            false
        };

        // Check if the data_type is 'resource' (not 'template' or 'string').
        // `data_type` is itself `data_type: resource` (references a
        // DataType class), so post-canonicalisation it's `ResourceRef`,
        // not `String`. Read via `as_iri_str`.
        let is_resource = prop_def
            .get(&data_type_iri)
            .and_then(|v| v.as_iri_str())
            .is_some_and(|s| s == "urn:eigenius:core:resource");

        if is_class_ref && is_resource {
            // This property references a Class — check if the actual
            // argument has a value. Read via `as_iri_str` so we
            // accept both `Value::String` (pre-canonicalisation
            // shape, e.g. FIBER-synthesised programs or RPC payloads
            // that bypass `LayerBuilder::build`) and `Value::ResourceRef`
            // (post-canonicalisation, the common production shape).
            // Matching only `Value::String` here caused CompleteJson
            // to receive a raw class IRI instead of the generated
            // JSON Schema, which surfaced as the orchestrator's
            // "CompleteJson requires output_schema in component argument"
            // error.
            if let Some(class_iri_str) = arg.get(prop_iri).and_then(|v| v.as_iri_str()) {
                if let Ok(schema_class_iri) = Iri::parse(class_iri_str) {
                    // Generate JSON Schema from this class
                    match crate::program::schema::schema_for_class(&schema_class_iri, layer) {
                        Ok((json_schema, table)) => {
                            // Replace the class IRI with the actual JSON Schema
                            arg.set(
                                prop_iri.clone(),
                                crate::ontology::resource::Value::Json(json_schema),
                            );
                            // Phase 18e.2: embed the short-name table on the
                            // argument so the orchestrator-side handler can
                            // translate LLM short-name output back to
                            // IRI-keyed shape before returning. Replaces the
                            // kernel-side post-hoc translation that lived
                            // in dispatch_component until 18e.1.
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

/// Extract the payload value from a single-property wrapper resource.
///
/// Convention: `resource_value_to_val` wraps primitive values in a
/// Resource with one property keyed on the type IRI (e.g.
/// `urn:eigenius:core:string`). This function extracts that value.
fn resource_payload(
    r: &crate::ontology::resource::Resource,
) -> Option<&crate::ontology::resource::Value> {
    let props = r.properties();
    if props.len() == 1 {
        props.values().next()
    } else {
        // Multi-property resources don't follow the wrapper convention;
        // fall back to first value for backwards compatibility.
        props.values().next()
    }
}

/// Decide a constraint against a value, three-valued.
///
/// Kernel-hardcoded scalar constraints (MinValue/MaxValue/…) fold
/// to `Holds` or `Fails` based on the structural check. Institution-
/// dispatched constraints (`Constraint::Institution { iri, args }`)
/// resolve the IRI as a Decidable QueryClass via the D14 institution
/// index; arguments are marshalled via [`val_to_resource_value`] onto
/// the synthetic input resource and the call dispatches through
/// `Institution::query` (D14 §9.2). Without an attached index/runtime
/// or a matching Decidable QueryClass, the constraint reduces to
/// `Undecidable` so downstream reducers leave it as a passthrough
/// neutral.
pub(super) fn decide_constraint(
    constraint: &crate::nbe::term::Constraint,
    val: &Val,
    rho: &Rho,
    ctx: &EvalCtx,
) -> Result<crate::institution::DecResult, EvalError> {
    use crate::institution::DecResult;
    use crate::nbe::term::Constraint;
    let bool_to_dec = |b: bool| {
        if b {
            DecResult::Holds
        } else {
            DecResult::Fails
        }
    };
    match constraint {
        Constraint::MinValue(min) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_integer())
                .is_some_and(|n| n >= *min),
            _ => false,
        })),
        Constraint::MaxValue(max) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_integer())
                .is_some_and(|n| n <= *max),
            _ => false,
        })),
        Constraint::MinLength(min) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.len() as i64 >= *min),
            _ => false,
        })),
        Constraint::MaxLength(max) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.len() as i64 <= *max),
            _ => false,
        })),
        Constraint::Pattern(pattern) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| {
                    let full = format!("^(?:{pattern})$");
                    regex::Regex::new(&full).is_ok_and(|re| re.is_match(s))
                }),
            _ => false,
        })),
        Constraint::Format(fmt) => Ok(bool_to_dec(match val {
            Val::ResourceVal(r) => resource_payload(r)
                .and_then(|v| v.as_str())
                .is_some_and(|s| match fmt.as_str() {
                    "date" => s.len() == 10 && s.chars().nth(4) == Some('-'),
                    "uuid" => s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4,
                    _ => true,
                }),
            _ => false,
        })),
        Constraint::Institution { iri, args } => {
            // D14 §9.2 dispatch: a Decidable QueryClass declares the
            // constraint IRI; args are marshalled onto a synthetic
            // input resource and the call goes through the institution
            // runtime. When no index/runtime is attached or no
            // Decidable QueryClass matches the IRI, the constraint
            // reduces to Undecidable so downstream reducers leave it
            // as a passthrough neutral.
            try_d14_decide(iri, args, rho, ctx).map(|opt| opt.unwrap_or(DecResult::Undecidable))
        }
    }
}

/// D14 §9.2 dispatch for an institution-bound Decidable constraint.
///
/// Returns:
/// - `Ok(Some(_))` if the D14 index has a Decidable QueryClass
///   declaring the constraint IRI and the dispatch ran end-to-end.
/// - `Ok(None)` if either the index or runtime is unattached, or the
///   constraint IRI doesn't resolve to a Decidable QueryClass. The
///   caller folds this to `DecResult::Undecidable` so reducers leave
///   the constraint as a passthrough neutral.
/// - `Err(_)` if the index *did* find the QueryClass but a downstream
///   step failed (missing institution, bad Verdict shape, etc.). A
///   configured-but-broken QueryClass is a structural error and not a
///   reason to silently fold to Undecidable.
fn try_d14_decide(
    iri: &Iri,
    args: &[Exp],
    rho: &Rho,
    ctx: &EvalCtx,
) -> Result<Option<crate::institution::DecResult>, EvalError> {
    use crate::institution::registry::DispatchRole;

    let (Some(index), Some(runtime)) = (ctx.institution_index(), ctx.institution_runtime()) else {
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

    // Marshal args into a synthetic input resource via the shared
    // `institution::marshal::marshal_decidable_input` helper. Same
    // logic as the EigenQL-side `query::evaluate::try_dispatch_decidable`
    // — input class typed required properties get populated in
    // `requires` declaration order, IRI-shaped values targeting
    // `core:resource` properties dereference to embedded resources.
    let arg_values: Result<Vec<_>, EvalError> = args
        .iter()
        .map(|a| eval_ctx(a, rho, ctx).map(|v| val_to_resource_value(&v)))
        .collect();
    let arg_values = arg_values?;
    let layer = ctx.layer().ok_or_else(|| {
        EvalError::InvalidCaseTarget(format!(
            "QueryClass `{iri}` Decidable call: no layer attached to EvalCtx — cannot \
             resolve input class `{}` for typed-property marshaling",
            query_class.query_class
        ))
    })?;
    let input = crate::institution::marshal::marshal_decidable_input(
        &query_class.query_class,
        &arg_values,
        layer,
    )
    .map_err(|e| EvalError::InvalidCaseTarget(format!("QueryClass `{iri}` Decidable call: {e}")))?;

    let head = ctx.layer().cloned().unwrap_or_else(|| {
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

    // Component-implemented QueryClasses go through extract → component
    // → reify; institution-runtime ones land in `Institution::query`.
    // M6 wires the institution-runtime path; the Component path lands
    // alongside Component-driven AutoOnLoad in M7.
    let outcome = institution
        .query(&query_class.query_handler, &input, &exec_ctx)
        .map_err(|e| {
            EvalError::InvalidCaseTarget(format!(
                "QueryClass `{iri}` Decidable handler `{}` failed: {e}",
                query_class.query_handler
            ))
        })?;

    // Read off the Verdict from the result resource. The result must
    // be (or wrap) a `Verdict` inductive value with one of the three
    // constructor names — `Holds`, `Fails`, `Undecidable`. Decidable
    // dispatch is type-check-time and produces no chain-side
    // RuntimeInvocation commit, so the partial provenance (if any) is
    // intentionally dropped here.
    Ok(Some(parse_verdict(&outcome.output).map_err(|e| {
        EvalError::InvalidCaseTarget(format!(
            "QueryClass `{iri}` Decidable handler returned a non-Verdict result: {e}"
        ))
    })?))
}

/// Read a `Verdict` inductive value off a result resource.
///
/// The institution handler is expected to set `is_a` to one of the
/// three Verdict constructor IRIs:
///   urn:eigenius:institution:verdicts:holds
///   urn:eigenius:institution:verdicts:fails
///   urn:eigenius:institution:verdicts:undecidable
///
/// (or, equivalently, set a `ctor_name` property to one of "Holds" /
/// "Fails" / "Undecidable" against an is_a of `Verdict`). Both shapes
/// are accepted.
fn parse_verdict(
    result: &crate::ontology::resource::Resource,
) -> Result<crate::institution::DecResult, String> {
    use crate::institution::DecResult;
    use crate::ontology::well_known as wk;

    // First look for an explicit `ctor_name` property — produced when
    // a Component returns a EigenTT Verdict value via the inductive
    // serialisation.
    if let Some(ctor) = result
        .get(&Iri::parse(wk::CTOR_NAME).expect("well-known IRI"))
        .and_then(|v| v.as_str().map(str::to_owned))
    {
        return match ctor.as_str() {
            "Holds" => Ok(DecResult::Holds),
            "Fails" => Ok(DecResult::Fails),
            "Undecidable" => Ok(DecResult::Undecidable),
            other => Err(format!("unknown Verdict ctor_name `{other}`")),
        };
    }

    // Otherwise check `is_a` against the three Verdict constructor
    // IRIs the institution might tag the result with.
    for class_iri in result.is_a() {
        match class_iri.as_str() {
            "urn:eigenius:institution:verdicts:holds" => return Ok(DecResult::Holds),
            "urn:eigenius:institution:verdicts:fails" => return Ok(DecResult::Fails),
            "urn:eigenius:institution:verdicts:undecidable" => return Ok(DecResult::Undecidable),
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

    // ─── D14 four-step InstitutionInvoke pipeline ──────────────────

    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::ontology::well_known as wk;
    use std::sync::Mutex;

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
                Iri::parse("urn:eigenius:test:d14_pipeline:extracted_via").expect("well-known IRI"),
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
                Iri::parse("urn:eigenius:test:d14_pipeline:reified_via").expect("well-known IRI"),
                crate::ontology::resource::Value::String(procedure_iri.as_str().into()),
            );
            Ok(tagged)
        }
    }

    fn build_d14_pipeline_chain() -> Arc<crate::layer::Layer> {
        // Layer holds: Institution + ExportFormat + ImportFormat +
        // Comorphism declarations. Same institution_ref for source
        // and target — deliberately, to keep the runtime registry
        // setup minimal.
        let mut b = crate::layer::LayerBuilder::new("test", None);

        let mut institution = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:inst").unwrap(),
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
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:inst".into()),
        );
        institution.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            crate::ontology::resource::Value::String("Pipeline test institution".into()),
        );
        b.add_resource(institution).unwrap();

        let mut export = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:export").unwrap(),
        );
        export.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::EXPORT_FORMAT_CLASS.into()),
            ]),
        );
        export.set(
            Iri::parse(wk::FROM_CLASS).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:SourceClass".into(),
            ),
        );
        export.set(
            Iri::parse(wk::PAYLOAD_TYPE).unwrap(),
            crate::ontology::resource::Value::String(wk::FLOAT.into()),
        );
        export.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:inst".into()),
        );
        export.set(
            Iri::parse(wk::PROCEDURE).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:proc:extract".into(),
            ),
        );
        b.add_resource(export).unwrap();

        let mut import = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:import").unwrap(),
        );
        import.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::IMPORT_FORMAT_CLASS.into()),
            ]),
        );
        import.set(
            Iri::parse(wk::TO_CLASS).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:TargetClass".into(),
            ),
        );
        import.set(
            Iri::parse(wk::PAYLOAD_TYPE).unwrap(),
            crate::ontology::resource::Value::String(wk::FLOAT.into()),
        );
        import.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:inst".into()),
        );
        import.set(
            Iri::parse(wk::PROCEDURE).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:proc:reify".into(),
            ),
        );
        b.add_resource(import).unwrap();

        let mut comorphism = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:cm").unwrap(),
        );
        comorphism.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::COMORPHISM.into()),
            ]),
        );
        comorphism.set(
            Iri::parse(wk::EXPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:export".into()),
        );
        comorphism.set(
            Iri::parse(wk::TRANSFORMATION).unwrap(),
            // No real Component — dispatch_component falls back to
            // identity for unknown component IRIs, which is what we
            // want for this structural test.
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:identity_transform".into(),
            ),
        );
        comorphism.set(
            Iri::parse(wk::IMPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:import".into()),
        );
        comorphism.set(
            Iri::parse(wk::EXACT).unwrap(),
            crate::ontology::resource::Value::Boolean(false),
        );
        b.add_resource(comorphism).unwrap();

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    fn build_d14_pipeline_ctx(log: Arc<Mutex<Vec<String>>>) -> (EvalCtx, Arc<InstitutionIndex>) {
        let layer = build_d14_pipeline_chain();
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "index errors: {errors:?}");
        let idx = Arc::new(idx);

        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(PipelineLogger {
                iri: Iri::parse("urn:eigenius:test:d14_pipe:inst").unwrap(),
                log,
            }))
            .unwrap();

        let ctx = EvalCtx::IO {
            layer,
            registry: Arc::new(ComponentRegistry::default()),
            trace_store: None,
            dispatched_traces: Arc::new(Mutex::new(Vec::new())),
            produced_resources: Arc::new(Mutex::new(Vec::new())),
            task_context: None,
            institution_index: Some(Arc::clone(&idx)),
            institution_runtime: Some(Arc::new(runtime)),
        };
        (ctx, idx)
    }

    #[test]
    fn institution_invoke_runs_d14_four_step_pipeline_end_to_end() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (ctx, _idx) = build_d14_pipeline_ctx(Arc::clone(&log));

        let source = Exp::EigonResource(Box::new(crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:source_instance").unwrap(),
        )));
        let exp = Exp::InstitutionInvoke {
            comorphism_iri: Iri::parse("urn:eigenius:test:d14_pipe:cm").unwrap(),
            source: Box::new(source),
            target_iri: None,
        };
        let v = eval_ctx(&exp, &Rho::Nil, &ctx).expect("D14 pipeline eval");
        let result = match v {
            Val::ResourceVal(r) => *r,
            other => panic!("expected ResourceVal from pipeline, got {other:?}"),
        };

        // Extract → identity-transform → reify all ran:
        let extracted_via = result
            .get(&Iri::parse("urn:eigenius:test:d14_pipeline:extracted_via").unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(
            extracted_via.as_deref(),
            Some("urn:eigenius:test:d14_pipe:proc:extract"),
            "extract_typed should have tagged the resource with the export procedure IRI"
        );
        let reified_via = result
            .get(&Iri::parse("urn:eigenius:test:d14_pipeline:reified_via").unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(
            reified_via.as_deref(),
            Some("urn:eigenius:test:d14_pipe:proc:reify"),
            "reify should have tagged the resource with the import procedure IRI"
        );

        // Order: extract first, reify last — confirms the four-step
        // pipeline shape (transformation in between is the identity
        // fallback for the unregistered Component IRI).
        let trail = log.lock().unwrap().clone();
        assert_eq!(
            trail,
            vec![
                "extract@urn:eigenius:test:d14_pipe:proc:extract(urn:eigenius:test:d14_pipe:source_instance)".to_string(),
                "reify@urn:eigenius:test:d14_pipe:proc:reify".to_string(),
            ]
        );
    }

    #[test]
    fn institution_invoke_d14_missing_format_surfaces_typed_error() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (ctx, idx) = build_d14_pipeline_ctx(Arc::clone(&log));

        // Sanity: the index has the comorphism we'll reference.
        assert!(idx
            .comorphism(&Iri::parse("urn:eigenius:test:d14_pipe:cm").unwrap())
            .is_some());

        // Build a *separate* comorphism that points at an
        // ExportFormat IRI not in the index. Must drop it into a new
        // layer above the existing chain so the InstitutionIndex can
        // still see the original declarations.
        let mut top =
            crate::layer::LayerBuilder::new("orphan_cm", Some(Arc::clone(ctx.layer().unwrap())));
        let mut orphan = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:test:d14_pipe:orphan_cm").unwrap(),
        );
        orphan.set(
            Iri::parse(wk::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(wk::COMORPHISM.into()),
            ]),
        );
        orphan.set(
            Iri::parse(wk::EXPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:not_in_index".into(),
            ),
        );
        orphan.set(
            Iri::parse(wk::TRANSFORMATION).unwrap(),
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_pipe:identity_transform".into(),
            ),
        );
        orphan.set(
            Iri::parse(wk::IMPORT_FORMAT).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:d14_pipe:import".into()),
        );
        top.add_resource(orphan).unwrap();
        let new_layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        // Re-derive the index over the new chain so it picks up the
        // orphan comorphism.
        let (new_idx, _errs) = InstitutionIndex::from_layer(&new_layer);
        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(PipelineLogger {
                iri: Iri::parse("urn:eigenius:test:d14_pipe:inst").unwrap(),
                log,
            }))
            .unwrap();
        let ctx = EvalCtx::IO {
            layer: new_layer,
            registry: Arc::new(ComponentRegistry::default()),
            trace_store: None,
            dispatched_traces: Arc::new(Mutex::new(Vec::new())),
            produced_resources: Arc::new(Mutex::new(Vec::new())),
            task_context: None,
            institution_index: Some(Arc::new(new_idx)),
            institution_runtime: Some(Arc::new(runtime)),
        };

        let exp = Exp::InstitutionInvoke {
            comorphism_iri: Iri::parse("urn:eigenius:test:d14_pipe:orphan_cm").unwrap(),
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

    // ─── D14 NativeDecide dispatch ─────────────────────────────────

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

    fn build_d14_decide_ctx(verdict_class: &'static str, arg_count: usize) -> EvalCtx {
        use crate::ontology::well_known as wk;
        let mut b = crate::layer::LayerBuilder::new("test", None);

        let inst_iri = "urn:eigenius:test:d14_decide:inst";
        let constraint_iri = "urn:eigenius:test:d14_decide:has_property";
        let input_class = "urn:eigenius:test:d14_decide:Subject";

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
            crate::ontology::resource::Value::String(
                "urn:eigenius:test:d14_decide:proc:check".into(),
            ),
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

        EvalCtx::IO {
            layer,
            registry: Arc::new(ComponentRegistry::default()),
            trace_store: None,
            dispatched_traces: Arc::new(Mutex::new(Vec::new())),
            produced_resources: Arc::new(Mutex::new(Vec::new())),
            task_context: None,
            institution_index: Some(Arc::new(idx)),
            institution_runtime: Some(Arc::new(runtime)),
        }
    }

    #[test]
    fn native_decide_d14_holds_reduces_to_refl() {
        let ctx = build_d14_decide_ctx("urn:eigenius:institution:verdicts:holds", 1);
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:d14_decide:has_property").unwrap(),
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
    fn native_decide_d14_fails_produces_failing_neutral() {
        let ctx = build_d14_decide_ctx("urn:eigenius:institution:verdicts:fails", 0);
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:d14_decide:has_property").unwrap(),
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
    fn native_decide_d14_undecidable_produces_passthrough_neutral() {
        let ctx = build_d14_decide_ctx("urn:eigenius:institution:verdicts:undecidable", 0);
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:d14_decide:has_property").unwrap(),
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
    fn native_decide_d14_falls_back_to_legacy_when_no_decidable_query_class() {
        // Constraint IRI not in the D14 index → fallback to legacy
        // institutions registry. With neither configured the legacy
        // path returns Undecidable (passthrough).
        let layer = Arc::new(
            crate::layer::LayerBuilder::new("test", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let (idx, _) = InstitutionIndex::from_layer(&layer);
        let ctx = EvalCtx::IO {
            layer,
            registry: Arc::new(ComponentRegistry::default()),
            trace_store: None,
            dispatched_traces: Arc::new(Mutex::new(Vec::new())),
            produced_resources: Arc::new(Mutex::new(Vec::new())),
            task_context: None,
            institution_index: Some(Arc::new(idx)),
            institution_runtime: Some(Arc::new(InstitutionRuntime::new())),
        };
        let constraint = crate::nbe::term::Constraint::Institution {
            iri: Iri::parse("urn:eigenius:test:d14_decide:not_declared").unwrap(),
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
