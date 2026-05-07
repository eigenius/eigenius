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

//! EigenQL evaluator: pattern matching, fixpoint, aggregation, result shaping.

use crate::context::ExecutionContext;
use crate::institution::registry::{DispatchRole, InstitutionIndex};
use crate::institution::runtime::InstitutionRuntime;
use crate::layer::{is_indexable_predicate, scan_chain, Layer};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::document::QueryFingerprint;
use crate::query::error::QueryError;
use crate::query::functions::{self, like_match, to_f64, values_compare, values_equal};
use std::collections::{BTreeMap, BTreeSet};

/// Runtime resources available to FIBER clause evaluation under D14.
/// Both `index` and `runtime` must be `Some` for FIBER dispatch to
/// succeed; `None` for either means FIBER clauses error at dispatch
/// time (typical of CLI local-only queries with no kernel runtime).
#[derive(Default, Clone, Copy)]
pub struct FiberRuntime<'a> {
    /// Derived index over institution / QueryClass / Comorphism /
    /// ExportFormat / ImportFormat declarations in the layer chain.
    pub index: Option<&'a InstitutionIndex>,
    /// `Institution` trait implementations keyed by institution IRI.
    pub runtime: Option<&'a InstitutionRuntime>,
    /// Kernel ComponentRegistry, used only by FIBER comorphism
    /// coercion (D2 v2 §3.5 / §6.12) to dispatch the transformation
    /// Component step of the four-step pipeline. Coercion errors at
    /// evaluation time when this is `None`. v1 restricts the cited
    /// transformation Component to Pure or Read capability levels.
    pub components: Option<&'a crate::program::component::ComponentRegistry>,
    /// Query-scoped transient overlay populated by FIBER clauses with
    /// their response resources (D2 v2 §6.12). Threaded into the
    /// expression evaluator so postfix Verdict predicates and
    /// resource-typed projections can resolve a FIBER-bound `?var`
    /// (held as the synthesized response IRI) back to the actual
    /// response resource. `None` outside of FIBER-bearing match
    /// parts.
    pub overlay: Option<&'a [(Iri, Resource)]>,
    pub ctx: Option<&'a ExecutionContext>,
}

/// Resources produced at runtime by FIBER clauses. They live for the
/// duration of a single query and are discarded when evaluation ends.
/// Pattern matching scans these in addition to the layer chain — see
/// D2 §6.12 (the "transient overlay").
#[derive(Default)]
struct FiberOverlay {
    entries: Vec<(Iri, Resource)>,
}

impl FiberOverlay {
    fn push(&mut self, iri: Iri, resource: Resource) {
        self.entries.push((iri, resource));
    }
}

/// A binding maps variable names to values.
type Binding = BTreeMap<String, Value>;

/// Evaluate a parsed and validated EigenQL program against a layer.
///
/// Row Property IRIs for RETURN items are synthesized using `fp`, so that
/// the downstream `document::wrap` step produces Property/Class metadata
/// resources that line up with the row keys.
///
/// FIBER clauses require `runtime` to carry both an institution registry
/// and an execution context; otherwise they error at dispatch time.
pub fn evaluate(
    program: &Program,
    layer: &Layer,
    fp: &QueryFingerprint,
    runtime: FiberRuntime<'_>,
) -> Result<Vec<Resource>, QueryError> {
    let mut derived: BTreeMap<String, Vec<Binding>> = BTreeMap::new();

    // 1. Evaluate DEFINE rules with seminaive fixpoint
    if !program.definitions.is_empty() {
        // Initial pass: evaluate all rules from base facts
        for def in &program.definitions {
            let bindings = evaluate_match_part(&def.body, layer, &derived)?;
            let entry = derived.entry(def.name.clone()).or_default();
            entry.extend(bindings);
        }

        // Fixpoint loop: keep evaluating until no new facts are derived
        let max_iterations = 1000; // Safety bound
        for _ in 0..max_iterations {
            let mut new_facts = false;
            for def in &program.definitions {
                let bindings = evaluate_match_part(&def.body, layer, &derived)?;
                let entry = derived.entry(def.name.clone()).or_default();
                let prev_len = entry.len();
                // Add only truly new bindings (not already present)
                for binding in bindings {
                    if !entry.contains(&binding) {
                        entry.push(binding);
                        new_facts = true;
                    }
                }
                if entry.len() > prev_len {
                    new_facts = true;
                }
            }
            if !new_facts {
                break; // Fixpoint reached
            }
        }
    }

    // 2. Evaluate the query
    let mut overlay = FiberOverlay::default();
    let mut bindings = evaluate_match_part_with_fiber(
        &program.query.body,
        layer,
        &derived,
        runtime,
        fp,
        &mut overlay,
    )?;

    // The overlay must remain visible to GROUP BY and RETURN shaping
    // — both can read FIBER-bound `?var.prop` projections via DotPath
    // and Verdict postfix predicates via `resolve_iri_string`. Layer
    // the populated overlay onto the user-supplied runtime once and
    // thread the result through both phases.
    let runtime_with_overlay = FiberRuntime {
        overlay: Some(&overlay.entries),
        ..runtime
    };

    // 3. GROUP BY + aggregation
    if !program.query.group_by.is_empty() || has_aggregates(&program.query.result) {
        bindings = apply_group_by(
            &program.query.group_by,
            &program.query.result,
            &bindings,
            layer,
            runtime_with_overlay,
        )?;
    }

    // 4. RETURN shaping
    let mut results = if program.query.result.is_empty() {
        bindings
            .iter()
            .map(|b| binding_to_resource(b, &program.query.result_classes))
            .collect()
    } else {
        let mut resources = Vec::new();
        for binding in &bindings {
            let resource = shape_result(
                binding,
                &program.query.result_classes,
                &program.query.result,
                layer,
                fp,
                runtime_with_overlay,
            )?;
            resources.push(resource);
        }
        resources
    };

    // 5. DISTINCT
    if program.query.distinct {
        results = deduplicate(results);
    }

    // 6. ORDER BY
    if !program.query.order_by.is_empty() {
        sort_results(&mut results, &program.query.order_by, fp);
    }

    // 7. OFFSET
    if let Some(offset) = program.query.offset {
        if offset < results.len() {
            results = results.into_iter().skip(offset).collect();
        } else {
            results.clear();
        }
    }

    // 8. LIMIT
    if let Some(limit) = program.query.limit {
        results.truncate(limit);
    }

    Ok(results)
}

/// Evaluate a MatchPart's pattern-only bodies (DEFINE rules).
///
/// Errors if any FIBER clause is present — DEFINE bodies can't dispatch
/// to institutions (no overlay, no runtime context at rule-fixpoint time).
/// The type checker rejects FIBER in DEFINE bodies so this is a defensive
/// check.
fn evaluate_match_part(
    part: &MatchPart,
    layer: &Layer,
    derived: &BTreeMap<String, Vec<Binding>>,
) -> Result<Vec<Binding>, QueryError> {
    if part.has_fiber() {
        return Err(QueryError::evaluation(
            "FIBER clauses are not allowed in DEFINE bodies",
        ));
    }

    let mut bindings: Vec<Binding> = vec![BTreeMap::new()];
    for pattern in part.patterns() {
        if pattern.negated {
            bindings = apply_negated_pattern(pattern, layer, derived, &[], bindings)?;
        } else {
            bindings = apply_pattern(pattern, layer, derived, &[], bindings)?;
        }
    }

    if !part.conditions.is_empty() {
        bindings.retain(|b| {
            part.conditions.iter().all(|cond| {
                // DEFINE bodies have no FIBER access; the institution
                // surface is unavailable here.
                eval_expression(cond, b, layer, FiberRuntime::default())
                    .and_then(|v| {
                        v.as_boolean().ok_or_else(|| {
                            QueryError::evaluation("WHERE condition must be boolean")
                        })
                    })
                    .unwrap_or(false)
            })
        });
    }

    Ok(bindings)
}

/// Evaluate a MatchPart with FIBER-clause support (top-level queries).
///
/// Walks `clauses` in order: Pattern clauses extend bindings via the
/// normal equi-join mechanism, Fiber clauses dispatch once per binding,
/// inject the response into the overlay, and extend the binding with
/// the bound variable. WHERE is applied once after all clauses.
fn evaluate_match_part_with_fiber(
    part: &MatchPart,
    layer: &Layer,
    derived: &BTreeMap<String, Vec<Binding>>,
    runtime: FiberRuntime<'_>,
    fp: &QueryFingerprint,
    overlay: &mut FiberOverlay,
) -> Result<Vec<Binding>, QueryError> {
    let mut bindings: Vec<Binding> = vec![BTreeMap::new()];

    // Resolve USING INSTITUTION aliases once; used to dereference FIBER
    // `institution` short names at dispatch time.
    let aliases: BTreeMap<&str, &Iri> = part
        .using_institutions
        .iter()
        .map(|a| (a.alias.as_str(), &a.iri))
        .collect();

    for (clause_idx, clause) in part.clauses.iter().enumerate() {
        match clause {
            Clause::Pattern(pattern) => {
                bindings = if pattern.negated {
                    apply_negated_pattern(pattern, layer, derived, &overlay.entries, bindings)?
                } else {
                    apply_pattern(pattern, layer, derived, &overlay.entries, bindings)?
                };
            }
            Clause::Fiber(fc) => {
                bindings = apply_fiber_clause(
                    fc, clause_idx, layer, runtime, fp, &aliases, overlay, bindings,
                )?;
            }
        }
    }

    if !part.conditions.is_empty() {
        // Thread the FIBER overlay into expression eval so postfix
        // Verdict predicates and resource-typed projections can
        // resolve a `?var` bound to a FIBER-synthesized response IRI
        // back to the response resource.
        let where_runtime = FiberRuntime {
            overlay: Some(&overlay.entries),
            ..runtime
        };
        bindings.retain(|b| {
            part.conditions.iter().all(|cond| {
                eval_expression(cond, b, layer, where_runtime)
                    .and_then(|v| {
                        v.as_boolean().ok_or_else(|| {
                            QueryError::evaluation("WHERE condition must be boolean")
                        })
                    })
                    .unwrap_or(false)
            })
        });
    }

    Ok(bindings)
}

/// Dispatch a FIBER clause once per binding in the current candidate set.
/// Each response is:
///   - stamped with a synthesized IRI (deterministic per query/clause/binding)
///   - attached to the transient overlay so later patterns see it
///   - bound to `fc.binding` in the extended binding
#[allow(clippy::too_many_arguments)]
fn apply_fiber_clause(
    fc: &FiberClause,
    clause_idx: usize,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
    fp: &QueryFingerprint,
    aliases: &BTreeMap<&str, &Iri>,
    overlay: &mut FiberOverlay,
    existing: Vec<Binding>,
) -> Result<Vec<Binding>, QueryError> {
    // D14 dispatch (D2 §6.12): FIBER requires both halves of the
    // institution machinery — the InstitutionIndex (resolves the
    // QueryClass) and the InstitutionRuntime (supplies the
    // Institution trait impl).
    let index = runtime.index.ok_or_else(|| {
        QueryError::evaluation(
            "FIBER requires an institution index — not available in this execution context",
        )
    })?;
    let inst_runtime = runtime.runtime.ok_or_else(|| {
        QueryError::evaluation(
            "FIBER requires an institution runtime — not available in this execution context",
        )
    })?;
    let ctx = runtime.ctx.ok_or_else(|| {
        QueryError::evaluation(
            "FIBER requires an execution context — not available in this execution context",
        )
    })?;

    let aliased_inst_iri = resolve_fiber_institution(&fc.institution, aliases)?;

    // Resolve the QueryClass IRI from the AST. Short names look up the
    // resource in the layer by short_name and use its @id; full IRIs
    // are used directly. Either way, the resolved IRI must be an
    // indexed QueryClass entry.
    let query_class_iri = resolve_query_class_iri(&fc.query_class, layer)?;
    let qc_entry = index.query_class(&query_class_iri).ok_or_else(|| {
        QueryError::evaluation(format!(
            "FIBER query class '{query_class_iri}' is not a registered QueryClass"
        ))
    })?;

    // D2 v2 §5.8 step 3 — runtime-checked echo of the type rule:
    // FIBER dispatches only OnDemand QueryClasses.
    if !qc_entry.dispatch_roles.contains(&DispatchRole::OnDemand) {
        return Err(QueryError::evaluation(format!(
            "FIBER query class '{query_class_iri}' has no OnDemand dispatch role"
        )));
    }

    // D2 v2 §5.8 step 4 — institution agreement.
    if qc_entry.institution_ref != aliased_inst_iri {
        return Err(QueryError::evaluation(format!(
            "FIBER cites institution '{aliased_inst_iri}' but QueryClass '{query_class_iri}' \
             declares institution_ref '{}'",
            qc_entry.institution_ref
        )));
    }

    let institution = inst_runtime.get(&qc_entry.institution_ref).ok_or_else(|| {
        QueryError::evaluation(format!(
            "institution '{}' not registered in runtime",
            qc_entry.institution_ref
        ))
    })?;

    // Build per-class param IRI resolution table (short_name → Iri)
    // from the QueryClass input class's requires ∪ recommends.
    let short_to_iri = build_param_iri_table(layer, &qc_entry.query_class);

    let is_a_iri = Iri::parse(wk::IS_A).unwrap();

    let mut extended = Vec::with_capacity(existing.len());
    for (binding_idx, binding) in existing.iter().enumerate() {
        // Construct the input resource. is_a is the QueryClass's
        // declared input class (D2 §6.12 step 3).
        let mut query_res = Resource::new_embedded();
        query_res.set(
            is_a_iri.clone(),
            Value::Array(vec![Value::ResourceRef(qc_entry.query_class.clone())]),
        );

        for param in &fc.params {
            let param_iri = match &param.name {
                Name::FullIri(iri) => iri.clone(),
                Name::ShortName(short) => short_to_iri.get(short).cloned().ok_or_else(|| {
                    QueryError::evaluation(format!(
                        "FIBER param '{short}' unresolvable against query class '{}'",
                        qc_entry.query_class
                    ))
                })?,
            };
            let value = match &param.value {
                ParamValue::Expression(expr) => eval_expression(expr, binding, layer, runtime)?,
                ParamValue::Comorphism { name, source } => {
                    let components = runtime.components.ok_or_else(|| {
                        QueryError::evaluation(
                            "FIBER comorphism coercion requires a ComponentRegistry — not \
                             available in this execution context",
                        )
                    })?;
                    eval_comorphism_coercion(
                        name,
                        source,
                        binding,
                        layer,
                        index,
                        inst_runtime,
                        components,
                        ctx,
                    )?
                }
            };
            // For params whose target property declares
            // `data_type: core:resource` (or `core:resource_array`),
            // dereference IRI-shaped values into embedded resources
            // before they flow to the institution. MATCH bindings
            // carry resource subjects as IRI strings; the
            // institution-runtime boundary serialises one typed
            // resource where class-typed fields must be fully
            // embedded for the worker's mirror decoders to match.
            // Inductive-typed fields (`core:inductive`) and
            // primitives pass through unchanged — IRIs there are
            // legitimate string/typed values, not resource references.
            let value = embed_typed_resource_param(&param_iri, value, layer)?;
            query_res.set(param_iri, value);
        }

        // Dispatch via D14 Institution::query.
        let outcome = institution
            .query(&qc_entry.query_handler, &query_res, ctx)
            .map_err(|e| {
                QueryError::evaluation(format!("fiber dispatch failed (clause {clause_idx}): {e}"))
            })?;
        // FIBER queries don't commit RuntimeInvocation provenance —
        // they're explicit-invocation queries (D14 §6.2 OnDemand)
        // whose audit trail rides on the EigenQL trace, not the chain.
        let response = outcome.output;

        // Stamp response with a synthesized @id + attach to overlay.
        let response_iri = fp.fiber_response_iri(clause_idx, binding_idx);
        let mut stamped = Resource::new(response_iri.clone());
        for (k, v) in response.properties() {
            stamped.set(k.clone(), v.clone());
        }
        overlay.push(response_iri.clone(), stamped);

        // Extend the binding with ?var → response_iri.
        let mut new_binding = binding.clone();
        new_binding.insert(
            fc.binding.name.clone(),
            Value::String(response_iri.as_str().to_string()),
        );
        extended.push(new_binding);
    }

    Ok(extended)
}

fn resolve_fiber_institution(
    name: &Name,
    aliases: &BTreeMap<&str, &Iri>,
) -> Result<Iri, QueryError> {
    match name {
        Name::FullIri(iri) => Ok(iri.clone()),
        Name::ShortName(alias) => aliases
            .get(alias.as_str())
            .map(|i| (*i).clone())
            .ok_or_else(|| {
                QueryError::evaluation(format!(
                    "FIBER references undeclared institution alias '{alias}'"
                ))
            }),
    }
}

/// Run the four-step comorphism pipeline for a FIBER param coercion
/// (D2 v2 §3.5 / §6.12). Mirrors the kernel-side
/// [`crate::nbe::eval::try_d14_institution_invoke`] but operates on
/// EigenQL `Value`s and dispatches the transformation Component
/// directly via `BuiltinComponent::execute` — v1 restricts coercion
/// transformations to Pure/Read so we don't need IO mode plumbing.
#[allow(clippy::too_many_arguments)]
pub fn eval_comorphism_coercion(
    name: &Name,
    source: &Expression,
    binding: &Binding,
    layer: &Layer,
    index: &InstitutionIndex,
    inst_runtime: &InstitutionRuntime,
    components: &crate::program::component::ComponentRegistry,
    ctx: &ExecutionContext,
) -> Result<Value, QueryError> {
    // Resolve the comorphism by name / IRI to its index entry.
    let comorphism_iri = match name {
        Name::FullIri(i) => i.clone(),
        Name::ShortName(short) => Iri::parse(short).map_err(|_| {
            QueryError::evaluation(format!(
                "comorphism_coercion: '{short}' is not a parseable IRI"
            ))
        })?,
    };
    let comorphism = index.comorphism(&comorphism_iri).ok_or_else(|| {
        QueryError::evaluation(format!(
            "comorphism `{comorphism_iri}` not registered in InstitutionIndex"
        ))
    })?;

    // Source-side institution lookup.
    let export = index
        .export_format(&comorphism.export_format)
        .ok_or_else(|| {
            QueryError::evaluation(format!(
                "comorphism `{comorphism_iri}`: export_format `{}` not in InstitutionIndex",
                comorphism.export_format
            ))
        })?;
    let source_inst = inst_runtime.get(&export.institution_ref).ok_or_else(|| {
        QueryError::evaluation(format!(
            "comorphism `{comorphism_iri}`: source institution `{}` not registered in runtime",
            export.institution_ref
        ))
    })?;

    // Evaluate the source expression against the current binding;
    // unwrap an Embedded resource or dereference a String → IRI →
    // resource lookup. Other primitive values are wrapped on a
    // single core:value property.
    let source_value = eval_expression(source, binding, layer, FiberRuntime::default())?;
    let source_resource = value_to_source_resource(&source_value, layer);

    // Step 2 — extract typed payload via the source institution.
    let typed_source = source_inst
        .extract_typed(&export.procedure, &source_resource, ctx)
        .map_err(|e| {
            QueryError::evaluation(format!(
                "comorphism `{comorphism_iri}`: extract_typed via `{}` failed: {e}",
                export.procedure
            ))
        })?;
    let typed_resource = match typed_source {
        crate::nbe::val::Val::ResourceVal(r) => *r,
        other => {
            return Err(QueryError::evaluation(format!(
                "comorphism `{comorphism_iri}`: extract_typed returned {other:?}, but the \
                 EigenQL four-step pipeline only marshals ResourceVal payloads in v1"
            )));
        }
    };

    // Step 3 — apply the transformation Component.
    let component = components
        .get(comorphism.transformation.as_str())
        .ok_or_else(|| {
            QueryError::evaluation(format!(
                "comorphism `{comorphism_iri}`: transformation Component `{}` not registered",
                comorphism.transformation
            ))
        })?;
    let transformed_resource = component
        .execute(&typed_resource, None, layer)
        .map_err(|e| {
            QueryError::evaluation(format!(
                "comorphism `{comorphism_iri}`: transformation `{}` failed: {e}",
                comorphism.transformation
            ))
        })?
        .output;

    // Step 4 — target-side institution reify.
    let import = index
        .import_format(&comorphism.import_format)
        .ok_or_else(|| {
            QueryError::evaluation(format!(
                "comorphism `{comorphism_iri}`: import_format `{}` not in InstitutionIndex",
                comorphism.import_format
            ))
        })?;
    let target_inst = inst_runtime.get(&import.institution_ref).ok_or_else(|| {
        QueryError::evaluation(format!(
            "comorphism `{comorphism_iri}`: target institution `{}` not registered in runtime",
            import.institution_ref
        ))
    })?;
    let transformed_val = crate::nbe::val::Val::ResourceVal(Box::new(transformed_resource));
    let target_resource = target_inst
        .reify(&import.procedure, &transformed_val, ctx)
        .map_err(|e| {
            QueryError::evaluation(format!(
                "comorphism `{comorphism_iri}`: reify via `{}` failed: {e}",
                import.procedure
            ))
        })?;

    // Post-translation validation invariant (D14 §9.3 step 5).
    let post_errors = crate::institution::dispatch::dispatch_auto_on_load_for_resource(
        &target_resource,
        index,
        inst_runtime,
        ctx,
    )
    .flatten_to_errors();
    if !post_errors.is_empty() {
        let reasons = post_errors
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(QueryError::evaluation(format!(
            "comorphism `{comorphism_iri}`: post-translation validation rejected the reified \
             resource: {reasons}"
        )));
    }

    Ok(Value::Embedded(Box::new(target_resource)))
}

/// Convert a FIBER param-coercion source `Value` to a Resource.
/// Embedded values pass through; IRI-shaped Strings dereference
/// against the layer; all other shapes are wrapped on a single
/// `core:value` property.
fn value_to_source_resource(value: &Value, layer: &Layer) -> Resource {
    match value {
        Value::Embedded(r) => r.as_ref().clone(),
        Value::String(s) => {
            if let Ok(iri) = Iri::parse(s) {
                if let Some(r) = layer.resolve(&iri) {
                    return (*r).clone();
                }
            }
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse("urn:eigenius:core:value").expect("well-known IRI"),
                Value::String(s.clone()),
            );
            r
        }
        other => {
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse("urn:eigenius:core:value").expect("well-known IRI"),
                other.clone(),
            );
            r
        }
    }
}

/// Resolve a `FIBER fc.query_class` reference (short name or full IRI)
/// to a QueryClass declaration's IRI. Short-name lookup walks the
/// layer for a resource with matching `short_name` whose `is_a`
/// includes `urn:eigenius:institution:QueryClass`.
fn resolve_query_class_iri(name: &Name, layer: &Layer) -> Result<Iri, QueryError> {
    match name {
        Name::FullIri(iri) => Ok(iri.clone()),
        Name::ShortName(short) => {
            let qc_class_iri = Iri::parse(wk::QUERY_CLASS_CLASS).unwrap();
            let short_prop = Iri::parse(wk::SHORT_NAME).unwrap();
            for (iri, res) in layer.iter_all_resources() {
                if !res.is_instance_of(&qc_class_iri) {
                    continue;
                }
                if let Some(Value::String(s)) = res.get(&short_prop) {
                    if s == short {
                        return Ok(iri.clone());
                    }
                }
            }
            Err(QueryError::evaluation(format!(
                "FIBER query class '{short}' not resolvable in layer (no QueryClass resource with that short_name)"
            )))
        }
    }
}

/// For FIBER param values whose target property is typed
/// `core:resource` (or `core:resource_array`), dereference IRI-shaped
/// values against the layer and substitute the embedded resource so
/// the institution-runtime serialisation carries a fully-embedded
/// typed map. Other property shapes — primitives, `core:inductive`,
/// `core:json`, `core:template` — pass through unchanged.
///
/// Closes the gap between FIBER's textual surface (where MATCH
/// bindings hold resource subjects as IRI strings) and the
/// institution-runtime boundary (where the mirror's typed decoders
/// for class-typed fields require the embedded shape).
fn embed_typed_resource_param(
    param_iri: &Iri,
    value: Value,
    layer: &Layer,
) -> Result<Value, QueryError> {
    let Some(prop_def) = layer.resolve(param_iri) else {
        // Unknown property — leave the value as-is. The dispatch path
        // surfaces a clearer error downstream than a kernel-side
        // "unknown property" raised here would.
        return Ok(value);
    };
    let dt_iri = Iri::parse(wk::DATA_TYPE_PROP).unwrap();
    let dt = match prop_def.get(&dt_iri) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::ResourceRef(i)) => i.as_str().to_string(),
        _ => return Ok(value),
    };
    match dt.as_str() {
        wk::RESOURCE => deref_resource_value(value, param_iri, layer),
        wk::RESOURCE_ARRAY => match value {
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(deref_resource_value(item, param_iri, layer)?);
                }
                Ok(Value::Array(out))
            }
            // Single value where array expected — leave it; type
            // mismatch surfaces downstream with a more precise error
            // than this kernel-side rewrite could produce.
            other => Ok(other),
        },
        _ => Ok(value),
    }
}

/// Dereference a single IRI-shaped value (`Value::ResourceRef` or
/// IRI-parseable `Value::String`) against the layer. Embedded values
/// pass through; non-IRI strings (and other primitives) pass through
/// — the worker's mirror decoder will surface a `MethodError` if the
/// shape is wrong, with the same diagnostic clarity as it does today.
fn deref_resource_value(value: Value, param_iri: &Iri, layer: &Layer) -> Result<Value, QueryError> {
    match value {
        Value::Embedded(r) => Ok(Value::Embedded(r)),
        Value::ResourceRef(iri) => deref_iri_to_embedded(&iri, param_iri, layer),
        Value::String(s) => match Iri::parse(&s) {
            Ok(iri) => deref_iri_to_embedded(&iri, param_iri, layer),
            Err(_) => Ok(Value::String(s)),
        },
        other => Ok(other),
    }
}

/// Resolve `iri` against the layer chain and wrap the result in
/// `Value::Embedded`. An unresolved IRI on a typed-resource property
/// is a clear authoring bug, not a compatibility concern, so we
/// error rather than passing through.
fn deref_iri_to_embedded(iri: &Iri, param_iri: &Iri, layer: &Layer) -> Result<Value, QueryError> {
    match layer.resolve(iri) {
        Some(r) => Ok(Value::Embedded(Box::new((*r).clone()))),
        None => Err(QueryError::evaluation(format!(
            "FIBER param `{param_iri}`: resource `{iri}` does not resolve in the layer chain"
        ))),
    }
}

fn build_param_iri_table(layer: &Layer, class_iri: &Iri) -> BTreeMap<String, Iri> {
    let requires_prop = Iri::parse(wk::REQUIRES).unwrap();
    let recommends_prop = Iri::parse(wk::RECOMMENDS).unwrap();
    let short_prop = Iri::parse(wk::SHORT_NAME).unwrap();

    let class_resource = match layer.resolve(class_iri) {
        Some(r) => r,
        None => return BTreeMap::new(),
    };

    let mut out = BTreeMap::new();
    let mut collect = |prop: &Iri| {
        if let Some(Value::Array(arr)) = class_resource.get(prop) {
            for v in arr {
                let prop_iri = match v {
                    Value::String(s) => Iri::parse(s).ok(),
                    Value::ResourceRef(i) => Some(i.clone()),
                    _ => None,
                };
                if let Some(iri) = prop_iri {
                    if let Some(prop_res) = layer.resolve(&iri) {
                        if let Some(Value::String(name)) = prop_res.get(&short_prop) {
                            out.insert(name.clone(), iri);
                        }
                    }
                }
            }
        }
    };
    collect(&requires_prop);
    collect(&recommends_prop);
    out
}

/// Apply a positive pattern: join with existing bindings.
///
/// `overlay` is the slice of transient fiber-response resources (possibly
/// empty) produced by earlier FIBER clauses in the same query. They are
/// merged into the candidate set alongside layer resources so pattern
/// matching on FIBER-bound variables works uniformly.
fn apply_pattern(
    pattern: &Pattern,
    layer: &Layer,
    derived: &BTreeMap<String, Vec<Binding>>,
    overlay: &[(Iri, Resource)],
    existing: Vec<Binding>,
) -> Result<Vec<Binding>, QueryError> {
    let candidates = collect_candidates(pattern, layer, derived, overlay);
    let mut result = Vec::new();

    for binding in &existing {
        for (resource_iri, resource) in &candidates {
            if let Some(new_binding) = try_match_resource(pattern, resource, resource_iri, binding)
            {
                result.push(new_binding);
            }
        }
    }

    Ok(result)
}

/// Apply a negated pattern: keep bindings where no match exists.
fn apply_negated_pattern(
    pattern: &Pattern,
    layer: &Layer,
    derived: &BTreeMap<String, Vec<Binding>>,
    overlay: &[(Iri, Resource)],
    existing: Vec<Binding>,
) -> Result<Vec<Binding>, QueryError> {
    let candidates = collect_candidates(pattern, layer, derived, overlay);
    let mut result = Vec::new();

    for binding in &existing {
        let has_match = candidates
            .iter()
            .any(|(iri, resource)| try_match_resource(pattern, resource, iri, binding).is_some());
        if !has_match {
            result.push(binding.clone());
        }
    }

    Ok(result)
}

/// Collect candidate resources for a pattern.
///
/// Phase 14h: when the pattern's class is bound and the `is_a` predicate
/// is indexable (its `Property.data_type` is `resource` or
/// `resource_array`), this uses [`scan_chain`] to enumerate matching
/// subjects via the per-layer triple index instead of the full chain
/// scan that pre-14h code used. The scan path remains as a fallback for
/// untyped patterns and for setups where `is_a` somehow lost its
/// indexable data_type.
fn collect_candidates<'a>(
    pattern: &Pattern,
    layer: &'a Layer,
    derived: &'a BTreeMap<String, Vec<Binding>>,
    overlay: &'a [(Iri, Resource)],
) -> Vec<(Option<Iri>, BTreeMap<Iri, Value>)> {
    // Check if this references a derived relation
    if let Some(Name::ShortName(ref name)) = pattern.class {
        if let Some(derived_bindings) = derived.get(name) {
            // Convert derived bindings to pseudo-resources
            return derived_bindings
                .iter()
                .map(|b| {
                    let props: BTreeMap<Iri, Value> = b
                        .iter()
                        .filter_map(|(k, v)| {
                            Iri::parse(&format!("urn:derived:{k}"))
                                .ok()
                                .map(|iri| (iri, v.clone()))
                        })
                        .collect();
                    (None, props)
                })
                .collect();
        }
    }

    let class_iri = pattern.class.as_ref().and_then(|n| resolve_name(n, layer));
    let is_a_iri = Iri::parse(wk::IS_A).expect("well-known is_a IRI");

    // Indexed path: bound class + indexable is_a predicate.
    let mut candidates: Vec<(Option<Iri>, BTreeMap<Iri, Value>)> =
        if let Some(ref class) = class_iri {
            if is_indexable_predicate(layer, &is_a_iri) {
                let class_closure = class_with_subclass_closure(class, layer);
                let mut subjects: BTreeSet<Iri> = BTreeSet::new();
                for concrete in &class_closure {
                    for s in scan_chain(layer, &is_a_iri, concrete) {
                        subjects.insert(s);
                    }
                }
                subjects
                    .into_iter()
                    .filter_map(|iri| {
                        layer
                            .resolve(&iri)
                            .map(|r| (Some(iri), r.properties().clone()))
                    })
                    .collect()
            } else {
                collect_candidates_via_scan(layer, Some(class))
            }
        } else {
            // Untyped pattern: no predicate to index by, fall back to scan.
            collect_candidates_via_scan(layer, None)
        };

    for (iri, resource) in overlay {
        let matches = if let Some(ref class) = class_iri {
            resource.is_instance_of(class) || is_subclass_instance(resource, class, layer)
        } else {
            true
        };
        if matches {
            candidates.push((Some(iri.clone()), resource.properties().clone()));
        }
    }

    candidates
}

/// Pre-14h scan path retained for the untyped-pattern case and as
/// fallback when `is_a`'s data_type isn't indexable. Walks the entire
/// chain via `iter_all_resources`.
fn collect_candidates_via_scan(
    layer: &Layer,
    class_iri: Option<&Iri>,
) -> Vec<(Option<Iri>, BTreeMap<Iri, Value>)> {
    layer
        .iter_all_resources()
        .filter(|(_, resource)| {
            if let Some(class) = class_iri {
                resource.is_instance_of(class) || is_subclass_instance(resource, class, layer)
            } else {
                true
            }
        })
        .map(|(iri, resource)| (Some(iri.clone()), resource.properties().clone()))
        .collect()
}

/// `{class} ∪ all transitive subclasses(class)` — the set of concrete
/// classes whose instances satisfy `MATCH ?x : class { ... }`. Walks the
/// `subclass_of` index recursively. When `subclass_of` isn't indexable,
/// returns just `{class}` and accepts the (small) loss of subclass
/// matches — pre-14h behavior would also have missed them via the
/// scan-only `is_subclass_instance` walk in degenerate setups.
fn class_with_subclass_closure(class_iri: &Iri, layer: &Layer) -> BTreeSet<Iri> {
    let subclass_of = Iri::parse(wk::PARENT_CLASSES).expect("well-known subclass_of IRI");
    let mut closure: BTreeSet<Iri> = BTreeSet::new();
    closure.insert(class_iri.clone());
    if !is_indexable_predicate(layer, &subclass_of) {
        return closure;
    }
    let mut frontier: Vec<Iri> = vec![class_iri.clone()];
    while let Some(parent) = frontier.pop() {
        for sub in scan_chain(layer, &subclass_of, &parent) {
            if closure.insert(sub.clone()) {
                frontier.push(sub);
            }
        }
    }
    closure
}

/// Try to match a resource against a pattern, extending an existing binding.
fn try_match_resource(
    pattern: &Pattern,
    resource_props: &BTreeMap<Iri, Value>,
    resource_iri: &Option<Iri>,
    existing: &Binding,
) -> Option<Binding> {
    let mut binding = existing.clone();

    // Bind the subject variable
    let subject_name = &pattern.subject.name;
    if let Some(iri) = resource_iri {
        let iri_val = Value::String(iri.as_str().to_string());
        if let Some(existing_val) = binding.get(subject_name) {
            if !values_equal(existing_val, &iri_val) {
                return None; // Conflict with existing binding
            }
        }
        binding.insert(subject_name.clone(), iri_val);
    }

    // Match property patterns
    for prop_pat in &pattern.properties {
        let prop_iri = match &prop_pat.property {
            Name::ShortName(s) => {
                // Find by shortname in resource properties
                find_property_by_shortname(s, resource_props)?
            }
            Name::FullIri(iri) => iri.clone(),
        };

        let value = resource_props.get(&prop_iri);

        match &prop_pat.object {
            ValueOrVariable::Variable(var) => {
                match value {
                    Some(val) => {
                        if let Some(existing_val) = binding.get(&var.name) {
                            if !values_equal(existing_val, val) {
                                return None; // Conflict
                            }
                        } else {
                            binding.insert(var.name.clone(), val.clone());
                        }
                    }
                    None => {
                        // Property not present — mark as unbound for NOT EXISTS
                        // Don't insert anything; NOT EXISTS checks for absence
                        // But this means the pattern doesn't match this resource
                        // unless we're specifically allowing optional matching
                        return None;
                    }
                }
            }
            ValueOrVariable::Literal(lit) => {
                let expected = literal_to_value(lit);
                match value {
                    Some(val) if values_equal(val, &expected) => {}
                    _ => return None,
                }
            }
        }
    }

    Some(binding)
}

/// Find a property IRI by shortname by looking it up in the resource's keys.
fn find_property_by_shortname(shortname: &str, props: &BTreeMap<Iri, Value>) -> Option<Iri> {
    props
        .keys()
        .find(|iri| iri.local_name() == shortname)
        .cloned()
}

/// Check if a resource is an instance of a class via subclass_of chain.
fn is_subclass_instance(resource: &Resource, class_iri: &Iri, layer: &Layer) -> bool {
    let resource_classes = resource.is_a();
    let subclass_iri = Iri::parse(wk::PARENT_CLASSES).unwrap();

    for res_class in &resource_classes {
        if is_subclass_of(res_class, class_iri, layer, &subclass_iri) {
            return true;
        }
    }
    false
}

fn is_subclass_of(sub: &Iri, target: &Iri, layer: &Layer, subclass_prop: &Iri) -> bool {
    if let Some(class_def) = layer.resolve(sub) {
        if let Some(parents) = class_def.get(subclass_prop) {
            for parent in parents.as_iri_array() {
                if parent == *target {
                    return true;
                }
                if is_subclass_of(&parent, target, layer, subclass_prop) {
                    return true;
                }
            }
        }
    }
    false
}

/// Resolve a Name to an IRI.
fn resolve_name(name: &Name, layer: &Layer) -> Option<Iri> {
    match name {
        Name::FullIri(iri) => Some(iri.clone()),
        Name::ShortName(s) => {
            // Search layer for a resource with this shortname
            let short_name_iri = Iri::parse(wk::SHORT_NAME).ok()?;
            for (iri, resource) in layer.iter_all_resources() {
                if let Some(Value::String(sn)) = resource.get(&short_name_iri) {
                    if sn == s {
                        return Some(iri.clone());
                    }
                }
            }
            None
        }
    }
}

/// Evaluate an expression against a binding.
fn eval_expression(
    expr: &Expression,
    binding: &Binding,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    match expr {
        Expression::Literal(lit) => Ok(literal_to_value(lit)),
        Expression::Variable(var) => binding
            .get(&var.name)
            .cloned()
            .ok_or_else(|| QueryError::evaluation(format!("unbound variable: ?{}", var.name))),
        Expression::Binary { op, left, right } => {
            let l = eval_expression(left, binding, layer, runtime)?;
            let r = eval_expression(right, binding, layer, runtime)?;
            eval_binary(*op, &l, &r)
        }
        Expression::Unary { op, operand } => {
            let v = eval_expression(operand, binding, layer, runtime)?;
            eval_unary(*op, &v)
        }
        Expression::VerdictPredicate { kind, operand } => {
            let v = eval_expression(operand, binding, layer, runtime)?;
            eval_verdict_predicate(*kind, &v, layer, runtime)
        }
        Expression::NotExists(var) => Ok(Value::Boolean(!binding.contains_key(&var.name))),
        Expression::FunctionCall { name, args } => {
            let arg_vals: Result<Vec<Value>, QueryError> = args
                .iter()
                .map(|a| eval_expression(a, binding, layer, runtime))
                .collect();
            let arg_vals = arg_vals?;
            // D2 §3.8: qualified-name function calls dispatch as a
            // Decidable QueryClass invocation. The result is a
            // Verdict-typed resource (Value::Embedded). Comorphism
            // dispatch in expression position is not supported under
            // D14 — comorphisms surface only as FIBER param coercion
            // (D2 §3.5).
            if name.contains(':') {
                if let Ok(iri_parsed) = Iri::parse(name) {
                    if let Some(verdict) = try_dispatch_decidable(&iri_parsed, &arg_vals, runtime)?
                    {
                        return Ok(verdict);
                    }
                }
            }
            functions::call_function(name, &arg_vals)
        }
        Expression::Aggregate { .. } => {
            // Aggregates are handled during GROUP BY, not per-binding
            Err(QueryError::evaluation(
                "aggregate function outside GROUP BY context",
            ))
        }
        Expression::DotPath { root, segments } => {
            // Resolve the root variable to a resource IRI
            let root_val = binding.get(&root.name).ok_or_else(|| {
                QueryError::evaluation(format!("unbound variable: ?{}", root.name))
            })?;
            let mut current_iri = match root_val {
                Value::String(s) => Iri::parse(s).map_err(|_| {
                    QueryError::evaluation(format!("variable ?{} is not a resource IRI", root.name))
                })?,
                _ => {
                    return Err(QueryError::evaluation(format!(
                        "variable ?{} is not a resource IRI",
                        root.name
                    )))
                }
            };

            // Walk each segment except the last — resolve intermediate
            // resources via the overlay (FIBER responses) ∪ layer
            // chain, in that order. Without the overlay check, a
            // dot-path on a `FIBER … AS ?bound` variable would fail
            // to find ?bound because the synthesized response IRI
            // lives in the transient overlay, not the chain.
            for (i, segment) in segments.iter().enumerate() {
                let resource = resolve_iri_string(current_iri.as_str(), layer, runtime)
                    .ok_or_else(|| {
                        QueryError::evaluation(format!(
                            "resource '{}' not found in layer chain or FIBER overlay",
                            current_iri
                        ))
                    })?;
                let prop_iri = find_property_by_shortname(segment, resource.properties())
                    .ok_or_else(|| {
                        QueryError::evaluation(format!(
                            "property '{}' not found on resource '{}'",
                            segment, current_iri
                        ))
                    })?;
                let value = resource.get(&prop_iri).ok_or_else(|| {
                    QueryError::evaluation(format!(
                        "property '{}' has no value on resource '{}'",
                        segment, current_iri
                    ))
                })?;

                if i < segments.len() - 1 {
                    // Intermediate segment: must be a resource reference
                    current_iri = match value {
                        Value::String(s) => Iri::parse(s).map_err(|_| {
                            QueryError::evaluation(format!(
                                "property '{}' on '{}' is not a resource reference",
                                segment, current_iri
                            ))
                        })?,
                        _ => {
                            return Err(QueryError::evaluation(format!(
                                "property '{}' on '{}' is not a resource reference",
                                segment, current_iri
                            )))
                        }
                    };
                } else {
                    // Final segment: return the value
                    return Ok(value.clone());
                }
            }
            Err(QueryError::evaluation("empty dot-path segments"))
        }
        Expression::Array(elements) => {
            let vals: Result<Vec<Value>, QueryError> = elements
                .iter()
                .map(|e| eval_expression(e, binding, layer, runtime))
                .collect();
            Ok(Value::Array(vals?))
        }
        Expression::Object(_) => Err(QueryError::evaluation(
            "object literals in expressions not yet implemented",
        )),
    }
}

fn eval_binary(op: BinaryOp, left: &Value, right: &Value) -> Result<Value, QueryError> {
    match op {
        BinaryOp::Eq => Ok(Value::Boolean(values_equal(left, right))),
        BinaryOp::Neq => Ok(Value::Boolean(!values_equal(left, right))),
        BinaryOp::Lt => Ok(Value::Boolean(
            values_compare(left, right) == Some(std::cmp::Ordering::Less),
        )),
        BinaryOp::Lte => Ok(Value::Boolean(matches!(
            values_compare(left, right),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ))),
        BinaryOp::Gt => Ok(Value::Boolean(
            values_compare(left, right) == Some(std::cmp::Ordering::Greater),
        )),
        BinaryOp::Gte => Ok(Value::Boolean(matches!(
            values_compare(left, right),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ))),
        BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::Mod
        | BinaryOp::Pow => {
            let a = to_f64(left)
                .ok_or_else(|| QueryError::evaluation("arithmetic requires numeric operands"))?;
            let b = to_f64(right)
                .ok_or_else(|| QueryError::evaluation("arithmetic requires numeric operands"))?;
            let result = match op {
                BinaryOp::Add => a + b,
                BinaryOp::Sub => a - b,
                BinaryOp::Mul => a * b,
                BinaryOp::Div => {
                    if b == 0.0 {
                        return Err(QueryError::evaluation("division by zero"));
                    }
                    a / b
                }
                BinaryOp::Mod => {
                    if b == 0.0 {
                        return Err(QueryError::evaluation("modulo by zero"));
                    }
                    a % b
                }
                BinaryOp::Pow => a.powf(b),
                _ => unreachable!(),
            };
            // Preserve integer type if both operands are integers and result is integral
            if matches!(left, Value::Integer(_))
                && matches!(right, Value::Integer(_))
                && result.fract() == 0.0
                && !matches!(op, BinaryOp::Pow)
            {
                Ok(Value::Integer(result as i64))
            } else {
                Ok(Value::Float(result))
            }
        }
        BinaryOp::StringConcat => {
            let a = left
                .as_str()
                .ok_or_else(|| QueryError::evaluation("|| requires string operands"))?;
            let b = right
                .as_str()
                .ok_or_else(|| QueryError::evaluation("|| requires string operands"))?;
            Ok(Value::String(format!("{a}{b}")))
        }
        BinaryOp::And => {
            let a = left
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("AND requires boolean operands"))?;
            let b = right
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("AND requires boolean operands"))?;
            Ok(Value::Boolean(a && b))
        }
        BinaryOp::Or => {
            let a = left
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("OR requires boolean operands"))?;
            let b = right
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("OR requires boolean operands"))?;
            Ok(Value::Boolean(a || b))
        }
        BinaryOp::In => {
            if let Value::Array(arr) = right {
                Ok(Value::Boolean(arr.iter().any(|v| values_equal(left, v))))
            } else {
                Err(QueryError::evaluation("IN requires array on right side"))
            }
        }
        BinaryOp::NotIn => {
            if let Value::Array(arr) = right {
                Ok(Value::Boolean(!arr.iter().any(|v| values_equal(left, v))))
            } else {
                Err(QueryError::evaluation(
                    "NOT IN requires array on right side",
                ))
            }
        }
        BinaryOp::Like => {
            let val = left
                .as_str()
                .ok_or_else(|| QueryError::evaluation("LIKE requires string operands"))?;
            let pat = right
                .as_str()
                .ok_or_else(|| QueryError::evaluation("LIKE requires string operands"))?;
            Ok(Value::Boolean(like_match(val, pat)))
        }
        BinaryOp::NotLike => {
            let val = left
                .as_str()
                .ok_or_else(|| QueryError::evaluation("NOT LIKE requires string operands"))?;
            let pat = right
                .as_str()
                .ok_or_else(|| QueryError::evaluation("NOT LIKE requires string operands"))?;
            Ok(Value::Boolean(!like_match(val, pat)))
        }
    }
}

fn eval_unary(op: UnaryOp, val: &Value) -> Result<Value, QueryError> {
    match op {
        UnaryOp::Not => {
            let b = val
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("NOT requires boolean"))?;
            Ok(Value::Boolean(!b))
        }
        UnaryOp::Pos => {
            let n = to_f64(val).ok_or_else(|| QueryError::evaluation("+ requires numeric"))?;
            Ok(Value::Float(n))
        }
        UnaryOp::Neg => match val {
            Value::Integer(n) => Ok(Value::Integer(-n)),
            Value::Float(f) => Ok(Value::Float(-f)),
            _ => Err(QueryError::evaluation("- requires numeric")),
        },
    }
}

/// Project a `Verdict`-typed value to `Boolean` per a postfix predicate
/// (D2 v2 §3.7 / §3.8). The operand is one of:
///
/// - `Value::Embedded(verdict)` — the Verdict resource directly.
/// - `Value::String(iri)` / `Value::ResourceRef(iri)` — a synthesized
///   IRI (typically from a FIBER `AS ?var` binding) that resolves to
///   the response resource through the runtime's transient overlay or
///   the layer chain.
fn eval_verdict_predicate(
    kind: crate::query::ast::VerdictPredicate,
    val: &Value,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    let resolved: Resource;
    let resource: &Resource = match val {
        Value::Embedded(r) => r.as_ref(),
        Value::String(s) => {
            resolved = resolve_iri_string(s, layer, runtime).ok_or_else(|| {
                QueryError::evaluation(format!(
                    "{kw} operand IRI `{s}` does not resolve to a resource (FIBER overlay or layer chain)",
                    kw = kind.ctor_name(),
                ))
            })?;
            &resolved
        }
        Value::ResourceRef(iri) => {
            resolved = resolve_iri_string(iri.as_str(), layer, runtime).ok_or_else(|| {
                QueryError::evaluation(format!(
                    "{kw} operand IRI `{iri}` does not resolve to a resource",
                    kw = kind.ctor_name(),
                ))
            })?;
            &resolved
        }
        other => {
            return Err(QueryError::evaluation(format!(
                "{kw} expects a Verdict-typed operand; got {other:?}",
                kw = kind.ctor_name(),
            )));
        }
    };
    let ctor_iri = Iri::parse(wk::CTOR_NAME).expect("well-known IRI");
    let ctor = resource
        .get(&ctor_iri)
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| {
            QueryError::evaluation(
                "Verdict postfix predicate operand carries no `ctor_name` property",
            )
        })?;
    Ok(Value::Boolean(ctor == kind.ctor_name()))
}

/// Resolve a String IRI to a Resource — checks the FiberOverlay first
/// (so FIBER-bound responses are visible) then walks the layer chain.
fn resolve_iri_string(s: &str, layer: &Layer, runtime: FiberRuntime<'_>) -> Option<Resource> {
    let iri = Iri::parse(s).ok()?;
    if let Some(overlay) = runtime.overlay {
        for (entry_iri, entry_resource) in overlay {
            if entry_iri == &iri {
                return Some(entry_resource.clone());
            }
        }
    }
    layer.resolve(&iri).map(|r| (*r).clone())
}

fn literal_to_value(lit: &Literal) -> Value {
    match lit {
        Literal::String(s) => Value::String(s.clone()),
        Literal::Integer(n) => Value::Integer(*n),
        Literal::Float(f) => Value::Float(*f),
        Literal::Boolean(b) => Value::Boolean(*b),
    }
}

/// Check if any return item uses an aggregate function.
fn has_aggregates(result: &[ReturnItem]) -> bool {
    result
        .iter()
        .any(|item| expr_has_aggregate(&item.expression))
}

fn expr_has_aggregate(expr: &Expression) -> bool {
    match expr {
        Expression::Aggregate { .. } => true,
        Expression::Binary { left, right, .. } => {
            expr_has_aggregate(left) || expr_has_aggregate(right)
        }
        Expression::Unary { operand, .. } => expr_has_aggregate(operand),
        Expression::VerdictPredicate { operand, .. } => expr_has_aggregate(operand),
        Expression::FunctionCall { args, .. } => args.iter().any(expr_has_aggregate),
        _ => false,
    }
}

/// Try to dispatch a qualified-name call as a Decidable QueryClass
/// invocation (D2 §3.8 / §6.13). Returns:
///
/// - `Ok(Some(verdict))` if the IRI resolved to a Decidable QueryClass
///   and dispatch ran end-to-end. The Verdict is returned as a
///   `Value::Embedded` resource carrying `is_a = [Verdict]` and
///   `ctor_name`.
/// - `Ok(None)` if the index/runtime aren't attached, the IRI doesn't
///   resolve to a QueryClass, or the resolved QueryClass has no
///   Decidable role. The caller falls through to builtin function
///   dispatch (which raises `unknown function`).
/// - `Err(_)` if the index *did* find a Decidable QueryClass but a
///   downstream step failed (missing institution registration,
///   handler failure, etc.). A configured-but-broken QueryClass is a
///   structural error, not a reason to silently fall through.
///
/// Comorphism dispatch is not available in expression position under
/// D14; comorphisms surface only as FIBER param coercion (D2 §3.5).
fn try_dispatch_decidable(
    iri: &Iri,
    args: &[Value],
    runtime: FiberRuntime<'_>,
) -> Result<Option<Value>, QueryError> {
    let (Some(index), Some(inst_runtime), Some(ctx)) =
        (runtime.index, runtime.runtime, runtime.ctx)
    else {
        return Ok(None);
    };
    let Some(qc_entry) = index.query_class(iri) else {
        return Ok(None);
    };
    if !qc_entry.dispatch_roles.contains(&DispatchRole::Decidable) {
        return Ok(None);
    }
    let institution = inst_runtime.get(&qc_entry.institution_ref).ok_or_else(|| {
        QueryError::evaluation(format!(
            "Decidable QueryClass `{iri}` declares institution `{}` not registered in runtime",
            qc_entry.institution_ref
        ))
    })?;

    // Marshal positional args onto a synthetic input resource of the
    // QueryClass's input class (D14 §9.2, mirrored by the kernel-side
    // `try_d14_decide`).
    let mut input = Resource::new_embedded();
    input.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::String(qc_entry.query_class.as_str().into())]),
    );
    input.set(
        Iri::parse("urn:eigenius:institution:decide_args").expect("well-known IRI"),
        Value::Array(args.to_vec()),
    );

    let outcome = institution
        .query(&qc_entry.query_handler, &input, ctx)
        .map_err(|e| {
            QueryError::evaluation(format!(
                "Decidable QueryClass `{iri}` handler `{}` failed: {e}",
                qc_entry.query_handler
            ))
        })?;

    // Decidable evaluation produces no chain-side RuntimeInvocation
    // commit — it's type-check-time reduction, not a Load. The
    // partial provenance (if any) is dropped here on purpose.
    Ok(Some(Value::Embedded(Box::new(outcome.output))))
}

/// Apply GROUP BY and aggregation.
fn apply_group_by(
    group_by: &[Expression],
    result: &[ReturnItem],
    bindings: &[Binding],
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Vec<Binding>, QueryError> {
    // Group bindings by their group key values
    let mut groups: BTreeMap<Vec<String>, Vec<&Binding>> = BTreeMap::new();

    for binding in bindings {
        let key: Vec<String> = group_by
            .iter()
            .map(|expr| {
                eval_expression(expr, binding, layer, runtime)
                    .map(|v| format!("{v:?}"))
                    .unwrap_or_default()
            })
            .collect();
        groups.entry(key).or_default().push(binding);
    }

    let mut result_bindings = Vec::new();
    for group in groups.values() {
        let mut binding = group[0].clone(); // Start with first binding for non-aggregate values

        // Compute aggregates
        for item in result {
            if let Some((agg_name, agg_val)) =
                eval_aggregate(&item.expression, group, layer, runtime)?
            {
                binding.insert(agg_name, agg_val);
            }
        }

        result_bindings.push(binding);
    }

    Ok(result_bindings)
}

/// Evaluate an aggregate expression over a group of bindings.
fn eval_aggregate(
    expr: &Expression,
    group: &[&Binding],
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Option<(String, Value)>, QueryError> {
    if let Expression::Aggregate { op, arg } = expr {
        let values: Vec<Value> = group
            .iter()
            .filter_map(|b| eval_expression(arg, b, layer, runtime).ok())
            .collect();

        let result = match op {
            AggregateOp::Count => Value::Integer(values.len() as i64),
            AggregateOp::Sum => {
                let sum: f64 = values.iter().filter_map(to_f64).sum();
                if values.iter().all(|v| matches!(v, Value::Integer(_))) {
                    Value::Integer(sum as i64)
                } else {
                    Value::Float(sum)
                }
            }
            AggregateOp::Avg => {
                let vals: Vec<f64> = values.iter().filter_map(to_f64).collect();
                if vals.is_empty() {
                    Value::Float(0.0)
                } else {
                    Value::Float(vals.iter().sum::<f64>() / vals.len() as f64)
                }
            }
            AggregateOp::Min => values
                .iter()
                .min_by(|a, b| values_compare(a, b).unwrap_or(std::cmp::Ordering::Equal))
                .cloned()
                .unwrap_or(Value::Integer(0)),
            AggregateOp::Max => values
                .iter()
                .max_by(|a, b| values_compare(a, b).unwrap_or(std::cmp::Ordering::Equal))
                .cloned()
                .unwrap_or(Value::Integer(0)),
        };

        // Use a synthetic name for the aggregate in the binding
        let name = format!("__agg_{op:?}");
        Ok(Some((name, result)))
    } else {
        Ok(None)
    }
}

/// Shape a binding into a result resource.
///
/// Property IRIs for short-name RETURN items are synthesized from `fp`,
/// so the downstream document wrapper produces matching Property metadata
/// resources. Full-IRI RETURN items use the user-supplied IRI unchanged.
fn shape_result(
    binding: &Binding,
    classes: &[Name],
    items: &[ReturnItem],
    layer: &Layer,
    fp: &QueryFingerprint,
    runtime: FiberRuntime<'_>,
) -> Result<Resource, QueryError> {
    let mut resource = Resource::new_embedded(); // Result resources don't get @id

    // Set is_a from result classes
    if !classes.is_empty() {
        let is_a_iri = Iri::parse(wk::IS_A).unwrap();
        let class_values: Vec<Value> = classes
            .iter()
            .map(|n| match n {
                Name::FullIri(iri) => Value::String(iri.as_str().to_string()),
                Name::ShortName(s) => Value::String(s.clone()),
            })
            .collect();
        if !class_values.is_empty() {
            resource.set(is_a_iri, Value::Array(class_values));
        }
    }

    for item in items {
        let prop_iri = match &item.name {
            Name::FullIri(iri) => iri.clone(),
            Name::ShortName(s) => fp.row_property_iri(s),
        };

        // Handle aggregate expressions specially
        let value = match &item.expression {
            Expression::Aggregate { op, .. } => {
                let agg_key = format!("__agg_{op:?}");
                binding.get(&agg_key).cloned().unwrap_or(Value::Integer(0))
            }
            _ => eval_expression(&item.expression, binding, layer, runtime)
                .map_err(|e| QueryError::evaluation(format!("in RETURN: {e}")))?,
        };

        resource.set(prop_iri, value);
    }

    Ok(resource)
}

/// Convert a binding to a simple resource (for match queries without RETURN).
fn binding_to_resource(binding: &Binding, _classes: &[Name]) -> Resource {
    let mut resource = Resource::new_embedded();
    for (key, value) in binding {
        if let Ok(iri) = Iri::parse(&format!("urn:query:var:{key}")) {
            resource.set(iri, value.clone());
        }
    }
    resource
}

/// Deduplicate resources (DISTINCT).
fn deduplicate(resources: Vec<Resource>) -> Vec<Resource> {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let mut result = Vec::new();
    for resource in resources {
        let canonical = crate::ontology::eigon_json::canonicalize(&resource);
        if !seen.contains(&canonical) {
            seen.push(canonical);
            result.push(resource);
        }
    }
    result
}

/// Sort results by ORDER BY expressions.
fn sort_results(resources: &mut [Resource], order_by: &[OrderItem], fp: &QueryFingerprint) {
    resources.sort_by(|a, b| {
        for item in order_by {
            // Try to evaluate the expression for each resource
            // For now, handle variable references by looking at resource properties
            let val_a = extract_sort_value(a, &item.expression, fp);
            let val_b = extract_sort_value(b, &item.expression, fp);

            if let (Some(va), Some(vb)) = (&val_a, &val_b) {
                if let Some(ord) = values_compare(va, vb) {
                    let ord = match item.direction {
                        SortDirection::Asc => ord,
                        SortDirection::Desc => ord.reverse(),
                    };
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                }
            }
        }
        std::cmp::Ordering::Equal
    });
}

fn extract_sort_value(
    resource: &Resource,
    expr: &Expression,
    fp: &QueryFingerprint,
) -> Option<Value> {
    match expr {
        Expression::Variable(var) => {
            let iri = fp.row_property_iri(&var.name);
            resource.get(&iri).cloned()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::eigon_json;
    use crate::query::lexer::tokenize;
    use crate::query::parser;
    use std::sync::Arc;

    fn build_test_layer() -> Arc<Layer> {
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }

        // Add example animals
        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animal_resources = eigon_json::parse_document(animals_json).unwrap();
        // Need a new layer on top of core. Share the same `LayerStorage`
        // so the bloom cache, resource cache, and triple index are all
        // populated from one set of writes — production bootstrap does
        // the same (see `bootstrap_with_storage`).
        let core = Arc::new(builder.build(storage.clone()));
        let mut domain_builder = LayerBuilder::new("animals", Some(core));
        for r in animal_resources {
            domain_builder.add_resource(r).unwrap();
        }
        Arc::new(domain_builder.build(storage))
    }

    fn run_query(layer: &Layer, query_str: &str) -> Vec<Resource> {
        let tokens = tokenize(query_str).unwrap();
        let program = parser::parse(tokens).unwrap();
        let fp = QueryFingerprint::of(query_str);
        evaluate(&program, layer, &fp, FiberRuntime::default()).unwrap()
    }

    #[test]
    fn find_all_classes() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            USING "urn:eigenius:core:Class"
            MATCH Class(?c) {
                short_name: ?name
            }
            RETURN [] {
                short_name: ?name
            }
            "#,
        );
        // Should find core classes + example classes (Animal, Dog)
        assert!(
            results.len() >= 6,
            "expected at least 6 classes, got {}",
            results.len()
        );
    }

    #[test]
    fn find_dog_instance() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH "urn:eigenius:example:Dog"(?d) {
                "urn:eigenius:example:name": ?name,
                "urn:eigenius:example:breed": ?breed
            }
            RETURN [] {
                "urn:eigenius:example:name": ?name,
                "urn:eigenius:example:breed": ?breed
            }
            "#,
        );
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn where_filtering() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH "urn:eigenius:example:Dog"(?d) {
                "urn:eigenius:example:breed": ?breed
            }
            WHERE ?breed = "German Shepherd"
            RETURN [] {
                "urn:eigenius:example:breed": ?breed
            }
            "#,
        );
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn where_no_match() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH "urn:eigenius:example:Dog"(?d) {
                "urn:eigenius:example:breed": ?breed
            }
            WHERE ?breed = "Poodle"
            RETURN [] {
                "urn:eigenius:example:breed": ?breed
            }
            "#,
        );
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn match_only_guard() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH "urn:eigenius:example:Dog"(?d) {
                "urn:eigenius:example:breed": ?breed
            }
            WHERE ?breed = "German Shepherd"
            "#,
        );
        // Guard query returns bindings (non-empty = true)
        assert!(!results.is_empty());
    }

    #[test]
    fn limit_results() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            USING "urn:eigenius:core:Property"
            MATCH Property(?p) {
                short_name: ?name
            }
            RETURN [] {
                short_name: ?name
            }
            LIMIT 3
            "#,
        );
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn like_operator() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            USING "urn:eigenius:core:Property"
            MATCH Property(?p) {
                short_name: ?name
            }
            WHERE ?name LIKE "data_%"
            RETURN [] {
                short_name: ?name
            }
            "#,
        );
        // Should find data_type
        assert!(!results.is_empty());
    }

    #[test]
    fn arithmetic_in_where() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH ?x {}
            WHERE 1 + 2 = 3
            RETURN [] {}
            LIMIT 1
            "#,
        );
        assert!(!results.is_empty());
    }

    fn build_hierarchy_layer() -> Arc<Layer> {
        // Build a simple hierarchy: Alice -> Bob -> Charlie
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(storage.clone()));

        let mut builder = LayerBuilder::new("hierarchy", Some(core));

        let mut alice = Resource::new(Iri::parse("urn:eigenius:test:alice").unwrap());
        alice.set(
            Iri::parse("urn:eigenius:test:name").unwrap(),
            Value::String("Alice".into()),
        );
        alice.set(
            Iri::parse("urn:eigenius:test:reports_to").unwrap(),
            Value::String("urn:eigenius:test:bob".into()),
        );
        builder.add_resource(alice).unwrap();

        let mut bob = Resource::new(Iri::parse("urn:eigenius:test:bob").unwrap());
        bob.set(
            Iri::parse("urn:eigenius:test:name").unwrap(),
            Value::String("Bob".into()),
        );
        bob.set(
            Iri::parse("urn:eigenius:test:reports_to").unwrap(),
            Value::String("urn:eigenius:test:charlie".into()),
        );
        builder.add_resource(bob).unwrap();

        let mut charlie = Resource::new(Iri::parse("urn:eigenius:test:charlie").unwrap());
        charlie.set(
            Iri::parse("urn:eigenius:test:name").unwrap(),
            Value::String("Charlie".into()),
        );
        builder.add_resource(charlie).unwrap();

        Arc::new(builder.build(storage))
    }

    #[test]
    fn recursive_define_ancestor() {
        let layer = build_hierarchy_layer();
        let results = run_query(
            &layer,
            r#"
            DEFINE Ancestor(?x, ?z) FROM
                MATCH ?x { "urn:eigenius:test:reports_to": ?z }
            DEFINE Ancestor(?x, ?z) FROM
                MATCH ?x { "urn:eigenius:test:reports_to": ?y },
                Ancestor(?y) { "urn:eigenius:test:reports_to": ?z }
            MATCH ?person {}
            WHERE ?person = "urn:eigenius:test:alice"
            RETURN [] {}
            "#,
        );
        // Alice should match
        assert!(!results.is_empty());
    }

    #[test]
    fn non_recursive_define() {
        let layer = build_hierarchy_layer();
        let results = run_query(
            &layer,
            r#"
            DEFINE Manager(?x, ?mgr) FROM
                MATCH ?x { "urn:eigenius:test:reports_to": ?mgr }
            MATCH ?x {}
            RETURN [] { "urn:eigenius:test:name": ?x }
            LIMIT 5
            "#,
        );
        assert!(!results.is_empty());
    }

    // --- D14: institution-dispatch surface for EigenQL ---

    use crate::context::ExecutionMode;
    use crate::institution::error::InstitutionError;
    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::nbe::val::Val;

    const Q_INST_IRI: &str = "urn:eigenius:test:q_inst";
    const Q_POSITIVE_IRI: &str = "urn:eigenius:test:q_positive";
    const Q_INPUT_CLASS_IRI: &str = "urn:eigenius:test:QPositiveInput";
    const Q_HANDLER_IRI: &str = "urn:eigenius:test:proc:q_positive";

    /// Test institution implementing one Decidable QueryClass.
    /// `q_positive` returns Holds for the first positive Integer in
    /// `decide_args`, Fails otherwise.
    struct QueryCapInst;

    impl Institution for QueryCapInst {
        fn institution_iri(&self) -> &Iri {
            static INST: std::sync::OnceLock<Iri> = std::sync::OnceLock::new();
            INST.get_or_init(|| Iri::parse(Q_INST_IRI).unwrap())
        }
        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<Val, InstitutionError> {
            unreachable!("test fixture only implements query")
        }
        fn reify(
            &self,
            _: &Iri,
            _: &Val,
            _: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            unreachable!("test fixture only implements query")
        }
        fn query(
            &self,
            _procedure_iri: &Iri,
            input: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<crate::institution::runtime::QueryOutcome, InstitutionError> {
            // Read decide_args off the synthesized input resource.
            let args_iri = Iri::parse("urn:eigenius:institution:decide_args").unwrap();
            let ok = match input.get(&args_iri) {
                Some(Value::Array(items)) => items
                    .first()
                    .and_then(|v| v.as_integer())
                    .is_some_and(|n| n > 0),
                _ => false,
            };
            // Build a Verdict response carrying ctor_name.
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
            );
            r.set(
                Iri::parse(wk::CTOR_NAME).unwrap(),
                Value::String(if ok { "Holds" } else { "Fails" }.into()),
            );
            Ok(crate::institution::runtime::QueryOutcome::from_output(r))
        }
    }

    /// Build an InstitutionIndex carrying a single Decidable QueryClass
    /// (q_positive) declared by the test institution.
    fn q_index() -> Arc<InstitutionIndex> {
        let mut b = LayerBuilder::new("q_test", None);

        let mut inst = Resource::new(Iri::parse(Q_INST_IRI).unwrap());
        inst.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:institution:Institution".to_string(),
            )]),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            Value::String(Q_INST_IRI.to_string()),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            Value::String("QueryCapInst".to_string()),
        );
        b.add_resource(inst).unwrap();

        let mut qc = Resource::new(Iri::parse(Q_POSITIVE_IRI).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.to_string())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            Value::String(Q_INPUT_CLASS_IRI.to_string()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            Value::String(wk::VERDICT.to_string()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            Value::Array(vec![Value::String(wk::DISPATCH_DECIDABLE.to_string())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            Value::String(Q_HANDLER_IRI.to_string()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            Value::String(Q_INST_IRI.to_string()),
        );
        b.add_resource(qc).unwrap();

        let layer = b.build(crate::layer::LayerStorage::in_memory());
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "fixture index errors: {errors:?}");
        Arc::new(idx)
    }

    fn q_runtime() -> Arc<InstitutionRuntime> {
        let mut runtime = InstitutionRuntime::new();
        runtime.register(Box::new(QueryCapInst)).unwrap();
        Arc::new(runtime)
    }

    fn q_exec_ctx(
        layer: Arc<crate::layer::Layer>,
        storage: crate::layer::LayerStorage,
    ) -> ExecutionContext {
        ExecutionContext::new(layer, "q_test", ExecutionMode::ReadOnly, storage)
    }

    #[test]
    fn parser_accepts_qualified_function_calls() {
        // Parse-only: the parser must accept `ns:local(args)`
        // without requiring institution registration.
        let source = r#"
            MATCH ?x {}
            WHERE cap:q_positive(42)
            RETURN [] { ok: ?x }
        "#;
        let tokens = tokenize(source).unwrap();
        let _program = parser::parse(tokens).expect("parse qualified call");
    }

    #[test]
    fn where_clause_decide_dispatch_returns_verdict() {
        // Under D14, a Decidable QueryClass call returns a Verdict
        // resource (not a Boolean). The postfix predicate (D2 §3.8)
        // is what projects to Boolean — a separate parser concern.
        let index = q_index();
        let inst_runtime = q_runtime();

        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(builder.build(storage.clone()));
        let exec_ctx = q_exec_ctx(Arc::clone(&layer), storage);

        let runtime = FiberRuntime {
            index: Some(&index),
            runtime: Some(&inst_runtime),
            components: None,
            overlay: None,
            ctx: Some(&exec_ctx),
        };

        // Use FunctionCall directly at eval_expression level for a
        // focused test — the full-query integration would need more
        // pattern-matching infrastructure. This verifies the core
        // dispatch path.
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: Q_POSITIVE_IRI.to_string(),
            args: vec![Expression::Literal(Literal::Integer(42))],
        };
        let v = eval_expression(&expr, &binding, &layer, runtime).expect("eval");
        let verdict = match v {
            Value::Embedded(r) => r,
            other => panic!("expected embedded Verdict, got {other:?}"),
        };
        let ctor = verdict
            .get(&Iri::parse(wk::CTOR_NAME).unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(ctor.as_deref(), Some("Holds"));

        // Negative arg → Fails.
        let expr_neg = Expression::FunctionCall {
            name: Q_POSITIVE_IRI.to_string(),
            args: vec![Expression::Literal(Literal::Integer(-5))],
        };
        let v = eval_expression(&expr_neg, &binding, &layer, runtime).expect("eval");
        let verdict = match v {
            Value::Embedded(r) => r,
            other => panic!("expected embedded Verdict, got {other:?}"),
        };
        let ctor = verdict
            .get(&Iri::parse(wk::CTOR_NAME).unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(ctor.as_deref(), Some("Fails"));
    }

    #[test]
    fn unknown_iri_falls_through_to_builtin_error() {
        // An IRI that doesn't resolve to a Decidable QueryClass falls
        // through to `functions::call_function`, which errors with
        // "no such function."
        let index = q_index();
        let inst_runtime = q_runtime();

        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(builder.build(storage.clone()));
        let exec_ctx = q_exec_ctx(Arc::clone(&layer), storage);

        let runtime = FiberRuntime {
            index: Some(&index),
            runtime: Some(&inst_runtime),
            components: None,
            overlay: None,
            ctx: Some(&exec_ctx),
        };

        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "urn:eigenius:test:unknown_fn".to_string(),
            args: vec![],
        };
        let err = eval_expression(&expr, &binding, &layer, runtime).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown") || msg.contains("function"));
    }

    // ─── D2 v2 §3.7 / §3.8 — postfix Verdict predicate ─────────────────

    #[test]
    fn parser_accepts_postfix_verdict_predicates() {
        // The grammar verdict_term ::= primary_expr (verdict_predicate)?
        // sits between unary and primary. All three postfix tokens must
        // parse, AND combinations across postfix-projected operands must
        // still parse.
        let source = r#"
            MATCH ?x {}
            WHERE cap:q_positive(42) HOLDS
              AND cap:other(?x) FAILS
              AND cap:third(?x) UNDECIDABLE
            RETURN [] { ok: ?x }
        "#;
        let tokens = tokenize(source).unwrap();
        let _program = parser::parse(tokens).expect("parse postfix predicates");
    }

    #[test]
    fn parser_postfix_binds_tighter_than_not() {
        // `NOT qc:check(?x) HOLDS` should parse as `NOT (qc:check(?x) HOLDS)`,
        // not `(NOT qc:check(?x)) HOLDS`. Verify by inspecting the AST shape.
        use crate::query::ast::{Expression, UnaryOp, VerdictPredicate};
        let source = r#"
            MATCH ?x {}
            WHERE NOT cap:q_positive(?x) HOLDS
            RETURN [] { ok: ?x }
        "#;
        let tokens = tokenize(source).unwrap();
        let program = parser::parse(tokens).expect("parse NOT-postfix");
        let cond = program
            .query
            .body
            .conditions
            .first()
            .expect("WHERE condition");
        match cond {
            Expression::Unary { op, operand } => {
                assert_eq!(*op, UnaryOp::Not);
                match operand.as_ref() {
                    Expression::VerdictPredicate { kind, .. } => {
                        assert_eq!(*kind, VerdictPredicate::Holds);
                    }
                    other => panic!("expected `NOT (qc HOLDS)`, got NOT followed by {other:?}"),
                }
            }
            other => panic!("expected `NOT …`, got {other:?}"),
        }
    }

    #[test]
    fn postfix_holds_projects_verdict_to_boolean() {
        // Build a Verdict resource with ctor_name = "Holds" and project
        // it through each of the three postfix predicates.
        use crate::query::ast::{Expression, VerdictPredicate};
        let mut verdict = Resource::new_embedded();
        verdict.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
        );
        verdict.set(
            Iri::parse(wk::CTOR_NAME).unwrap(),
            Value::String("Holds".into()),
        );
        let layer = Arc::new(
            LayerBuilder::new("postfix-test", None).build(crate::layer::LayerStorage::in_memory()),
        );
        let runtime = FiberRuntime::default();
        let mut binding: BTreeMap<String, Value> = BTreeMap::new();
        binding.insert("v".into(), Value::Embedded(Box::new(verdict)));

        let var_v = Expression::Variable(crate::query::ast::Variable { name: "v".into() });
        let project = |kind: VerdictPredicate| -> Value {
            eval_expression(
                &Expression::VerdictPredicate {
                    kind,
                    operand: Box::new(var_v.clone()),
                },
                &binding,
                &layer,
                runtime,
            )
            .expect("eval verdict predicate")
        };
        assert_eq!(project(VerdictPredicate::Holds), Value::Boolean(true));
        assert_eq!(project(VerdictPredicate::Fails), Value::Boolean(false));
        assert_eq!(
            project(VerdictPredicate::Undecidable),
            Value::Boolean(false)
        );
    }

    // ─── D2 v2 §3.5 — comorphism coercion in FIBER param values ────────

    #[test]
    fn parser_recognises_comorphism_coercion_in_fiber_param() {
        // A single-arg qualified-name function call in FIBER param value
        // position is a comorphism coercion: parser produces
        // ParamValue::Comorphism { name, source }, not
        // ParamValue::Expression(FunctionCall).
        use crate::query::ast::{Clause, ParamValue};
        let source = r#"
            USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay
            MATCH ?d {}
            FIBER assay:within_tolerance {
                predicted_ic50: dock:dock_to_assay(?d)
            } AS ?v
            RETURN [] { d: ?d }
        "#;
        let tokens = tokenize(source).unwrap();
        let program = parser::parse(tokens).expect("parse FIBER + coercion");
        let fiber = program
            .query
            .body
            .clauses
            .iter()
            .find_map(|c| match c {
                Clause::Fiber(fc) => Some(fc),
                _ => None,
            })
            .expect("FIBER clause");
        let predicted = fiber
            .params
            .iter()
            .find(|p| matches!(&p.name, Name::ShortName(s) if s == "predicted_ic50"))
            .expect("predicted_ic50 param");
        match &predicted.value {
            ParamValue::Comorphism { name, .. } => match name {
                Name::ShortName(s) => assert_eq!(s, "dock:dock_to_assay"),
                Name::FullIri(i) => assert_eq!(i.as_str(), "dock:dock_to_assay"),
            },
            other => panic!("expected ParamValue::Comorphism, got {other:?}"),
        }
    }

    #[test]
    fn parser_treats_multi_arg_qualified_call_as_expression() {
        // Multi-arg qualified-name function calls stay as Expression
        // in FIBER param value position (comorphisms are unary by
        // construction).
        use crate::query::ast::{Clause, Expression, ParamValue};
        let source = r#"
            USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay
            MATCH ?d {}
            FIBER assay:within_tolerance {
                predicted_ic50: cap:multi(?d, 1.0)
            } AS ?v
            RETURN [] { d: ?d }
        "#;
        let tokens = tokenize(source).unwrap();
        let program = parser::parse(tokens).expect("parse FIBER + multi-arg");
        let fiber = program
            .query
            .body
            .clauses
            .iter()
            .find_map(|c| match c {
                Clause::Fiber(fc) => Some(fc),
                _ => None,
            })
            .expect("FIBER clause");
        match &fiber.params[0].value {
            ParamValue::Expression(Expression::FunctionCall { args, .. }) => {
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected ParamValue::Expression(FunctionCall), got {other:?}"),
        }
    }

    // --- FIBER param IRI-dereference (D2 v2 §6.12 / Phase 19d.2 follow-on) ---
    //
    // `embed_typed_resource_param` rewrites IRI-shaped FIBER param
    // values into embedded resources when the target property is
    // typed `core:resource` / `core:resource_array`, so the
    // institution-runtime boundary's typed decoders see a
    // fully-embedded map rather than a bare IRI string. These tests
    // pin each branch of that rewrite without requiring a live
    // institution dispatch.

    fn deref_layer_with_props() -> Arc<crate::layer::Layer> {
        // A minimal layer carrying:
        //   - a `core:resource` property `prop_obj`,
        //   - a `core:resource_array` property `prop_arr`,
        //   - a `core:string` property `prop_str`,
        //   - a target Class `Target` with a chain-committed
        //     instance `target_instance`.
        let mut b = LayerBuilder::new("deref-test", None);

        // The Class of the target.
        let mut target_class = Resource::new(Iri::parse("urn:test:deref:Target").unwrap());
        target_class.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(Iri::parse(wk::CLASS).unwrap())]),
        );
        target_class.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Target".into()),
        );
        b.add_resource(target_class).unwrap();

        // A target instance the deref will resolve to.
        let mut inst = Resource::new(Iri::parse("urn:test:deref:target_instance").unwrap());
        inst.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse("urn:test:deref:Target").unwrap(),
            )]),
        );
        inst.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("target_instance".into()),
        );
        b.add_resource(inst).unwrap();

        // `prop_obj : core:resource → Target`.
        let mut prop_obj = Resource::new(Iri::parse("urn:test:deref:prop_obj").unwrap());
        prop_obj.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(Iri::parse(wk::PROPERTY).unwrap())]),
        );
        prop_obj.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::ResourceRef(Iri::parse(wk::RESOURCE).unwrap()),
        );
        b.add_resource(prop_obj).unwrap();

        // `prop_arr : core:resource_array → [Target]`.
        let mut prop_arr = Resource::new(Iri::parse("urn:test:deref:prop_arr").unwrap());
        prop_arr.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(Iri::parse(wk::PROPERTY).unwrap())]),
        );
        prop_arr.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::ResourceRef(Iri::parse(wk::RESOURCE_ARRAY).unwrap()),
        );
        b.add_resource(prop_arr).unwrap();

        // `prop_str : core:string`.
        let mut prop_str = Resource::new(Iri::parse("urn:test:deref:prop_str").unwrap());
        prop_str.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(Iri::parse(wk::PROPERTY).unwrap())]),
        );
        prop_str.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::ResourceRef(Iri::parse(wk::STRING).unwrap()),
        );
        b.add_resource(prop_str).unwrap();

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn embed_typed_resource_param_dereferences_iri_string() {
        let layer = deref_layer_with_props();
        let prop = Iri::parse("urn:test:deref:prop_obj").unwrap();
        let value = Value::String("urn:test:deref:target_instance".into());
        let out = embed_typed_resource_param(&prop, value, &layer).expect("deref ok");
        match out {
            Value::Embedded(r) => {
                assert_eq!(
                    r.id().map(|i| i.as_str()),
                    Some("urn:test:deref:target_instance")
                );
            }
            other => panic!("expected Embedded after deref, got {other:?}"),
        }
    }

    #[test]
    fn embed_typed_resource_param_dereferences_resource_ref() {
        // Same as above but the input is the canonical `ResourceRef`
        // shape MATCH bindings produce post-canonicalisation.
        let layer = deref_layer_with_props();
        let prop = Iri::parse("urn:test:deref:prop_obj").unwrap();
        let value = Value::ResourceRef(Iri::parse("urn:test:deref:target_instance").unwrap());
        let out = embed_typed_resource_param(&prop, value, &layer).expect("deref ok");
        assert!(matches!(out, Value::Embedded(_)));
    }

    #[test]
    fn embed_typed_resource_param_dereferences_array_elements() {
        let layer = deref_layer_with_props();
        let prop = Iri::parse("urn:test:deref:prop_arr").unwrap();
        let value = Value::Array(vec![
            Value::ResourceRef(Iri::parse("urn:test:deref:target_instance").unwrap()),
            Value::String("urn:test:deref:target_instance".into()),
        ]);
        let out = embed_typed_resource_param(&prop, value, &layer).expect("deref ok");
        match out {
            Value::Array(items) => {
                assert_eq!(items.len(), 2);
                for it in items {
                    assert!(
                        matches!(it, Value::Embedded(_)),
                        "array element must be embedded after deref"
                    );
                }
            }
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn embed_typed_resource_param_passes_through_string_property() {
        // A property typed `core:string` carries IRI-shaped values as
        // legitimate strings (e.g. correlation IDs, user-supplied
        // tokens). The rewrite must leave them alone.
        let layer = deref_layer_with_props();
        let prop = Iri::parse("urn:test:deref:prop_str").unwrap();
        let value = Value::String("urn:test:deref:target_instance".into());
        let out = embed_typed_resource_param(&prop, value, &layer).expect("passthrough ok");
        match out {
            Value::String(s) => assert_eq!(s, "urn:test:deref:target_instance"),
            other => panic!("expected String to pass through, got {other:?}"),
        }
    }

    #[test]
    fn embed_typed_resource_param_passes_through_embedded_value() {
        // An already-embedded resource passes through unchanged —
        // the rewrite is idempotent.
        let layer = deref_layer_with_props();
        let prop = Iri::parse("urn:test:deref:prop_obj").unwrap();
        let mut emb = Resource::new_embedded();
        emb.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("inline".into()),
        );
        let value = Value::Embedded(Box::new(emb));
        let out = embed_typed_resource_param(&prop, value, &layer).expect("passthrough ok");
        match out {
            Value::Embedded(r) => {
                assert_eq!(
                    r.get(&Iri::parse(wk::SHORT_NAME).unwrap()),
                    Some(&Value::String("inline".into()))
                );
            }
            other => panic!("expected Embedded passthrough, got {other:?}"),
        }
    }

    #[test]
    fn embed_typed_resource_param_errors_on_unresolvable_iri() {
        // An IRI on a `core:resource` property that doesn't resolve
        // is a clear authoring bug — surface it at the kernel rather
        // than letting the worker fail on a missing field.
        let layer = deref_layer_with_props();
        let prop = Iri::parse("urn:test:deref:prop_obj").unwrap();
        let value = Value::String("urn:test:deref:does_not_exist".into());
        let err = embed_typed_resource_param(&prop, value, &layer).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("does not resolve"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn embed_typed_resource_param_passes_through_unknown_property() {
        // No prop definition in the layer → leave the value alone;
        // dispatch surfaces a clearer error downstream.
        let layer = deref_layer_with_props();
        let prop = Iri::parse("urn:test:deref:no_such_prop").unwrap();
        let value = Value::String("urn:test:something".into());
        let out = embed_typed_resource_param(&prop, value, &layer).expect("passthrough ok");
        assert!(matches!(out, Value::String(_)));
    }

    #[test]
    fn postfix_predicate_rejects_non_verdict_operand() {
        // A non-Verdict operand (e.g. an Integer) should error with a
        // type-mismatch evaluation error rather than silently returning
        // false. Type-checker enforcement of this rule lands as part of
        // §5.9 rule coverage; the runtime guard is the floor.
        use crate::query::ast::{Expression, Literal, VerdictPredicate};
        let layer = Arc::new(
            LayerBuilder::new("postfix-test", None).build(crate::layer::LayerStorage::in_memory()),
        );
        let runtime = FiberRuntime::default();
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::VerdictPredicate {
            kind: VerdictPredicate::Holds,
            operand: Box::new(Expression::Literal(Literal::Integer(42))),
        };
        let err = eval_expression(&expr, &binding, &layer, runtime).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Verdict-typed operand"),
            "unexpected message: {msg}"
        );
    }
}
