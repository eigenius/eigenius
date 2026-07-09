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

//! FIBER clause dispatch + the `FiberRuntime` / overlay machinery
//! that flows transient response resources into pattern matching,
//! WHERE filtering, and RETURN shaping.

use crate::context::ExecutionContext;
use crate::institution::registry::{DispatchRole, InstitutionIndex};
use crate::institution::runtime::InstitutionRuntime;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::document::QueryFingerprint;
use crate::query::error::QueryError;
use std::collections::BTreeMap;

use super::expression::eval_expression;
use super::pattern::{apply_negated_pattern, apply_pattern, Binding};

/// Runtime resources available to FIBER clause evaluation.
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
    /// D43 §6 — similarity-operator pre-pass results, built once
    /// per query and threaded through every per-row evaluation so
    /// `Expression::Similarity` is an O(1) score-map lookup. `None`
    /// for callers outside [`evaluate`] (e.g. AST-only test
    /// harnesses); the operator then fails at evaluation with a
    /// clear "similarity context unavailable" diagnostic.
    pub similarity: Option<&'a super::similarity::SimilarityContext>,
    /// D43 §3.5 / §5.2 — registry of Embedder Components dispatched
    /// by the `~` similarity operator when the active index is a
    /// VectorIndex. `None` when no vector path is in use; the operator
    /// then fails at evaluation with a clear "no embedders registered"
    /// diagnostic.
    pub embedders: Option<&'a crate::program::embedder::EmbedderRegistry>,
    /// D43 §5.3 — content-addressed embedding cache shared between
    /// query-side `EMBED` calls (M4) and the indexing-side sweep
    /// (M5, deferred). `None` means every `EMBED` dispatch goes
    /// through to the Embedder; for tests that's the right default,
    /// for production a single long-lived cache pins repeat-embed
    /// cost to a hash lookup.
    pub embedding_cache: Option<&'a crate::program::embedding_cache::EmbeddingCache>,
    /// D43 §5.9 — vector SegmentCache shared across `VECTOR_NEAR` /
    /// `VECTOR_SIM` probes (M5.6). `None` means each probe goes
    /// through to the `VectorIndex` backend; production callers
    /// pass a kernel-shared cache so repeat probes against the same
    /// `(index_iri, layer_id)` are an in-memory `BTreeMap` lookup.
    pub vector_segment_cache: Option<&'a crate::query::vector::cache::SegmentCache>,
}

/// Resources produced at runtime by FIBER clauses. They live for the
/// duration of a single query and are discarded when evaluation ends.
/// Pattern matching scans these in addition to the layer chain — see
/// D2 §6.12 (the "transient overlay").
#[derive(Default)]
pub(super) struct FiberOverlay {
    pub(super) entries: Vec<(Iri, Resource)>,
}

impl FiberOverlay {
    fn push(&mut self, iri: Iri, resource: Resource) {
        self.entries.push((iri, resource));
    }
}

/// Evaluate a MatchPart's pattern-only bodies (DEFINE rules).
///
/// Errors if any FIBER clause is present — DEFINE bodies can't dispatch
/// to institutions (no overlay, no runtime context at rule-fixpoint time).
/// The type checker rejects FIBER in DEFINE bodies so this is a defensive
/// check.
pub(super) fn evaluate_match_part(
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
            bindings = apply_negated_pattern(
                pattern,
                layer,
                derived,
                &[],
                bindings,
                &part.using_namespaces,
            )?;
        } else {
            bindings = apply_pattern(
                pattern,
                layer,
                derived,
                &[],
                bindings,
                &part.conditions,
                &part.using_namespaces,
            )?;
        }
    }

    if !part.conditions.is_empty() {
        // DEFINE bodies have no FIBER access; the institution
        // surface is unavailable here.
        bindings.retain(|b| {
            part.conditions.iter().all(|cond| {
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
#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate_match_part_with_fiber(
    part: &MatchPart,
    layer: &Layer,
    derived: &BTreeMap<String, Vec<Binding>>,
    runtime: FiberRuntime<'_>,
    fp: &QueryFingerprint,
    overlay: &mut FiberOverlay,
    into_collector: &mut Vec<Resource>,
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
                    apply_negated_pattern(
                        pattern,
                        layer,
                        derived,
                        &overlay.entries,
                        bindings,
                        &part.using_namespaces,
                    )?
                } else {
                    apply_pattern(
                        pattern,
                        layer,
                        derived,
                        &overlay.entries,
                        bindings,
                        &part.conditions,
                        &part.using_namespaces,
                    )?
                };
            }
            Clause::Fiber(fc) => {
                bindings = apply_fiber_clause(
                    fc,
                    clause_idx,
                    layer,
                    runtime,
                    fp,
                    &aliases,
                    overlay,
                    into_collector,
                    bindings,
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
    into_collector: &mut Vec<Resource>,
    existing: Vec<Binding>,
) -> Result<Vec<Binding>, QueryError> {
    // institution dispatch (D2 §6.12): FIBER requires both halves of the
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

        // Dispatch via Institution::query.
        let outcome = institution
            .query(&qc_entry.query_handler, &query_res, ctx)
            .map_err(|e| {
                QueryError::evaluation(format!("fiber dispatch failed (clause {clause_idx}): {e}"))
            })?;
        // FIBER queries don't commit RuntimeInvocation provenance —
        // they're explicit-invocation queries (D14 §6.2 OnDemand)
        // whose audit trail rides on the EigenQL trace, not the chain.
        let response = outcome.output;

        // Stamp response with an `@id` and attach to the transient
        // overlay so subsequent patterns and the WHERE/RETURN
        // expression evaluator can resolve `?var` back to the
        // response resource. The IRI choice depends on FIBER `INTO`:
        //
        // - With `INTO "<iri>"` (D14 §9.3 chain-reinsertion via
        //   EigenQL): the user-named IRI stamps both the overlay
        //   entry and the chain-commit collector. After the query
        //   commits, the response is a first-class chain resident
        //   addressable at that IRI. Each input binding produces its
        //   own response — a multi-row FIBER with INTO would attempt
        //   to commit multiple resources at the same IRI. For v1 we
        //   reject the second arrival with a clear error so the
        //   semantics stay obvious.
        // - Without `INTO`: synthesize a query-scope transient IRI
        //   (the prior behaviour); the response disappears at query
        //   end.
        let (response_iri, persist_to_chain) = match &fc.into {
            Some(target) => {
                if into_collector.iter().any(|r| r.id() == Some(target)) {
                    return Err(QueryError::evaluation(format!(
                        "FIBER `INTO \"{target}\"` matched more than one input binding; \
                         a single INTO IRI cannot name two distinct chain resources. \
                         Constrain the FIBER inputs so it fires once, or drop INTO and \
                         let the response stay query-scoped."
                    )));
                }
                (target.clone(), true)
            }
            None => (fp.fiber_response_iri(clause_idx, binding_idx), false),
        };
        let mut stamped = Resource::new(response_iri.clone());
        for (k, v) in response.properties() {
            stamped.set(k.clone(), v.clone());
        }
        if persist_to_chain {
            into_collector.push(stamped.clone());
        }
        overlay.push(response_iri.clone(), stamped);

        // Extend the binding with ?var → response_iri (the chain-
        // resident IRI when INTO is set; the transient overlay IRI
        // otherwise).
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
/// [`crate::nbe::eval::try_institution_invoke`] but operates on
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::query::lexer::tokenize;
    use crate::query::parser;
    use std::sync::Arc;

    #[test]
    fn parser_recognises_comorphism_coercion_in_fiber_param() {
        // A single-arg qualified-name function call in FIBER param value
        // position is a comorphism coercion: parser produces
        // ParamValue::Comorphism { name, source }, not
        // ParamValue::Expression(FunctionCall).
        use crate::query::ast::{Clause, ParamValue};
        let source = r#"
            USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
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
            USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
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
}
