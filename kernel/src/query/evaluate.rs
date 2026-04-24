//! EigenQL evaluator: pattern matching, fixpoint, aggregation, result shaping.

use crate::context::ExecutionContext;
use crate::institution::InstitutionRegistry;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::document::QueryFingerprint;
use crate::query::error::QueryError;
use crate::query::functions::{self, like_match, to_f64, values_compare, values_equal};
use std::collections::BTreeMap;

/// Runtime resources available to FIBER clause evaluation. `None` means
/// FIBER clauses will be rejected (e.g. CLI local-only queries that have
/// no registered institutions).
#[derive(Default, Clone, Copy)]
pub struct FiberRuntime<'a> {
    pub institutions: Option<&'a InstitutionRegistry>,
    pub ctx: Option<&'a ExecutionContext>,
}

/// Resources produced at runtime by FIBER clauses. They live for the
/// duration of a single query and are discarded when evaluation ends.
/// Pattern matching scans these in addition to the layer chain — see
/// D2 Appendix B §B.3 (the "transient overlay").
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

    // 3. GROUP BY + aggregation
    if !program.query.group_by.is_empty() || has_aggregates(&program.query.result) {
        bindings = apply_group_by(
            &program.query.group_by,
            &program.query.result,
            &bindings,
            layer,
            runtime.institutions,
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
                runtime.institutions,
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
                eval_expression(cond, b, layer, None)
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
        bindings.retain(|b| {
            part.conditions.iter().all(|cond| {
                eval_expression(cond, b, layer, runtime.institutions)
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
    let institutions = runtime.institutions.ok_or_else(|| {
        QueryError::evaluation(
            "FIBER requires an institution registry — not available in this execution context",
        )
    })?;
    let ctx = runtime.ctx.ok_or_else(|| {
        QueryError::evaluation(
            "FIBER requires an execution context — not available in this execution context",
        )
    })?;

    let inst_iri = resolve_fiber_institution(&fc.institution, aliases)?;
    let reasoner = institutions.get(&inst_iri).ok_or_else(|| {
        QueryError::evaluation(format!("no institution registered for IRI '{inst_iri}'"))
    })?;

    let query_class_iri = match &fc.query_class {
        Name::FullIri(iri) => iri.clone(),
        Name::ShortName(name) => resolve_name_to_class_iri(layer, name).ok_or_else(|| {
            QueryError::evaluation(format!(
                "FIBER query class '{name}' not resolvable in layer"
            ))
        })?,
    };

    // Build per-class param IRI resolution table (short_name → Iri)
    // from the class's requires ∪ recommends.
    let short_to_iri = build_param_iri_table(layer, &query_class_iri);

    let is_a_iri = Iri::parse(wk::IS_A).unwrap();

    let mut extended = Vec::with_capacity(existing.len());
    for (binding_idx, binding) in existing.iter().enumerate() {
        // Construct the query resource.
        let mut query_res = Resource::new_embedded();
        query_res.set(
            is_a_iri.clone(),
            Value::Array(vec![Value::ResourceRef(query_class_iri.clone())]),
        );

        for param in &fc.params {
            let param_iri = match &param.name {
                Name::FullIri(iri) => iri.clone(),
                Name::ShortName(short) => short_to_iri.get(short).cloned().ok_or_else(|| {
                    QueryError::evaluation(format!(
                        "FIBER param '{short}' unresolvable against query class '{query_class_iri}'"
                    ))
                })?,
            };
            let value = eval_expression(&param.expression, binding, layer, runtime.institutions)?;
            query_res.set(param_iri, value);
        }

        // Dispatch.
        let response = reasoner.query(&query_res, ctx).map_err(|e| {
            QueryError::evaluation(format!("fiber dispatch failed (clause {clause_idx}): {e}"))
        })?;

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

fn resolve_name_to_class_iri(layer: &Layer, short: &str) -> Option<Iri> {
    let class_iri = Iri::parse(wk::CLASS).unwrap();
    let short_prop = Iri::parse(wk::SHORT_NAME).unwrap();
    for (iri, res) in layer.all_resources() {
        if !res.is_instance_of(&class_iri) {
            continue;
        }
        if let Some(Value::String(s)) = res.get(&short_prop) {
            if s == short {
                return Some(iri.clone());
            }
        }
    }
    None
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

    // Collect from layer chain + FIBER overlay.
    let all = layer.all_resources();
    let class_iri = pattern.class.as_ref().and_then(|n| resolve_name(n, layer));

    let mut candidates: Vec<(Option<Iri>, BTreeMap<Iri, Value>)> = all
        .into_iter()
        .filter(|(_, resource)| {
            if let Some(ref class) = class_iri {
                resource.is_instance_of(class) || is_subclass_instance(resource, class, layer)
            } else {
                true // Untyped pattern matches all
            }
        })
        .map(|(iri, resource)| (Some(iri.clone()), resource.properties().clone()))
        .collect();

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
            for (iri, resource) in layer.all_resources() {
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
    institutions: Option<&InstitutionRegistry>,
) -> Result<Value, QueryError> {
    match expr {
        Expression::Literal(lit) => Ok(literal_to_value(lit)),
        Expression::Variable(var) => binding
            .get(&var.name)
            .cloned()
            .ok_or_else(|| QueryError::evaluation(format!("unbound variable: ?{}", var.name))),
        Expression::Binary { op, left, right } => {
            let l = eval_expression(left, binding, layer, institutions)?;
            let r = eval_expression(right, binding, layer, institutions)?;
            eval_binary(*op, &l, &r)
        }
        Expression::Unary { op, operand } => {
            let v = eval_expression(operand, binding, layer, institutions)?;
            eval_unary(*op, &v)
        }
        Expression::NotExists(var) => Ok(Value::Boolean(!binding.contains_key(&var.name))),
        Expression::FunctionCall { name, args } => {
            let arg_vals: Result<Vec<Value>, QueryError> = args
                .iter()
                .map(|a| eval_expression(a, binding, layer, institutions))
                .collect();
            let arg_vals = arg_vals?;
            // Phase 11e.2: qualified-name function calls (containing
            // a `:`) may classify as institution-dispatched
            // capabilities. Attempt institution dispatch first; fall
            // back to builtin dispatch if the IRI isn't registered
            // or the registry isn't available.
            if name.contains(':') {
                if let Some(registry) = institutions {
                    if let Ok(iri_parsed) = Iri::parse(name) {
                        if let Some(cap) = registry.classify(&iri_parsed) {
                            return dispatch_institution_call(
                                cap,
                                &iri_parsed,
                                &arg_vals,
                                layer,
                                registry,
                            );
                        }
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

            // Walk each segment except the last — resolve intermediate resources
            for (i, segment) in segments.iter().enumerate() {
                let resource = layer.resolve(&current_iri).ok_or_else(|| {
                    QueryError::evaluation(format!(
                        "resource '{}' not found in layer chain",
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
                .map(|e| eval_expression(e, binding, layer, institutions))
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
        Expression::FunctionCall { args, .. } => args.iter().any(expr_has_aggregate),
        _ => false,
    }
}

/// Dispatch a function-call IRI classified as an institution
/// capability to `FiberReasoner::decide` / `::translate` (Phase
/// 11e.2). Decide predicates produce a boolean; comorphisms produce
/// a resource value.
fn dispatch_institution_call(
    capability: crate::institution::InstitutionCapability,
    iri: &Iri,
    args: &[Value],
    layer: &Layer,
    registry: &InstitutionRegistry,
) -> Result<Value, QueryError> {
    use crate::institution::{DecResult, InstitutionCapability};
    // Build a fresh ExecutionContext for the call — mirrors the
    // kernel's NativeDecide / InstitutionInvoke behaviour.
    let head = std::sync::Arc::new(layer.clone());
    let exec_ctx = crate::context::ExecutionContext::new(
        head,
        "__eigenql_dispatch__",
        crate::context::ExecutionMode::ReadOnly,
    );
    match capability {
        InstitutionCapability::DecidePredicate => {
            let reasoner = registry.institution_for_decide(iri).ok_or_else(|| {
                QueryError::evaluation(format!(
                    "decide procedure `{iri}` classified but no institution registered"
                ))
            })?;
            let result = reasoner
                .decide(iri, args, &exec_ctx)
                .map_err(|e| QueryError::evaluation(format!("decide `{iri}` failed: {e}")))?;
            Ok(Value::Boolean(matches!(result, DecResult::Holds)))
        }
        InstitutionCapability::Comorphism => {
            if args.len() != 1 {
                return Err(QueryError::evaluation(format!(
                    "comorphism `{iri}` expects exactly 1 source argument, got {}",
                    args.len()
                )));
            }
            let reasoner = registry.institution_for_comorphism(iri).ok_or_else(|| {
                QueryError::evaluation(format!(
                    "comorphism `{iri}` classified but no institution registered"
                ))
            })?;
            let source = match &args[0] {
                Value::Embedded(r) => (**r).clone(),
                other => {
                    let mut r = Resource::new_embedded();
                    r.set(
                        Iri::parse("urn:eigenius:core:value").expect("well-known IRI"),
                        other.clone(),
                    );
                    r
                }
            };
            let translated = reasoner.translate(iri, &source, &exec_ctx).map_err(|e| {
                QueryError::evaluation(format!("comorphism `{iri}` translate failed: {e}"))
            })?;
            Ok(Value::Embedded(Box::new(translated)))
        }
    }
}

/// Apply GROUP BY and aggregation.
fn apply_group_by(
    group_by: &[Expression],
    result: &[ReturnItem],
    bindings: &[Binding],
    layer: &Layer,
    institutions: Option<&InstitutionRegistry>,
) -> Result<Vec<Binding>, QueryError> {
    // Group bindings by their group key values
    let mut groups: BTreeMap<Vec<String>, Vec<&Binding>> = BTreeMap::new();

    for binding in bindings {
        let key: Vec<String> = group_by
            .iter()
            .map(|expr| {
                eval_expression(expr, binding, layer, institutions)
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
                eval_aggregate(&item.expression, group, layer, institutions)?
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
    institutions: Option<&InstitutionRegistry>,
) -> Result<Option<(String, Value)>, QueryError> {
    if let Expression::Aggregate { op, arg } = expr {
        let values: Vec<Value> = group
            .iter()
            .filter_map(|b| eval_expression(arg, b, layer, institutions).ok())
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
    institutions: Option<&InstitutionRegistry>,
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
            _ => eval_expression(&item.expression, binding, layer, institutions)
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
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }

        // Add example animals
        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animal_resources = eigon_json::parse_document(animals_json).unwrap();
        // Need a new layer on top of core
        let core = Arc::new(builder.build());
        let mut domain_builder = LayerBuilder::new("animals", Some(core));
        for r in animal_resources {
            domain_builder.add_resource(r).unwrap();
        }
        Arc::new(domain_builder.build())
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
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build());

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

        Arc::new(builder.build())
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

    // --- Phase 11e.2: institution-capability surface for EigenQL ---

    use crate::institution::error::{InstitutionError, MorphismValidation};
    use crate::institution::{DecResult, FiberDeclaration, FiberReasoner, InstitutionRegistry};

    /// Test institution declaring one decide predicate and one
    /// comorphism. Decide returns Holds for Integer args > 0.
    /// Comorphism translates any source to a fixed marker resource.
    struct QueryCapInst;

    impl FiberReasoner for QueryCapInst {
        fn fiber_declaration(&self) -> FiberDeclaration {
            let decide_iri = Iri::parse("urn:eigenius:test:q_positive").unwrap();
            let comorphism_iri = Iri::parse("urn:eigenius:test:q_translate").unwrap();

            let mut cm = Resource::new(comorphism_iri.clone());
            cm.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String(wk::COMORPHISM.to_string())]),
            );
            cm.set(
                Iri::parse(wk::SOURCE_INSTITUTION).unwrap(),
                Value::String("urn:eigenius:test:q_inst".to_string()),
            );
            cm.set(
                Iri::parse(wk::TARGET_INSTITUTION).unwrap(),
                Value::String("urn:eigenius:test:target".to_string()),
            );
            cm.set(
                Iri::parse(wk::TRANSLATION_PROCEDURE).unwrap(),
                Value::String(comorphism_iri.as_str().to_string()),
            );
            FiberDeclaration {
                institution_iri: Iri::parse("urn:eigenius:test:q_inst").unwrap(),
                name: "QueryCapInst".to_string(),
                morphism_types: vec![],
                query_types: vec![],
                structural_properties: vec![],
                comorphism_types: vec![cm],
                decide_procedures: vec![decide_iri],
            }
        }
        fn query(
            &self,
            _q: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            unreachable!()
        }
        fn validate_morphism(
            &self,
            _m: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<MorphismValidation, InstitutionError> {
            unreachable!()
        }
        fn discover_morphisms(
            &self,
            _rs: &[Resource],
            _ctx: &ExecutionContext,
        ) -> Result<Vec<Resource>, InstitutionError> {
            unreachable!()
        }
        fn decide(
            &self,
            _iri: &Iri,
            args: &[Value],
            _ctx: &ExecutionContext,
        ) -> Result<DecResult, InstitutionError> {
            let ok = args
                .first()
                .and_then(|v| v.as_integer())
                .is_some_and(|n| n > 0);
            Ok(if ok {
                DecResult::Holds
            } else {
                DecResult::Fails
            })
        }
        fn translate(
            &self,
            _iri: &Iri,
            _source: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            Ok(Resource::new(
                Iri::parse("urn:eigenius:test:q_translated").unwrap(),
            ))
        }
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
    fn where_clause_decide_dispatch_filters_by_boolean() {
        // WHERE cap:q_positive(n) returns true for positive ints,
        // false otherwise. Use the registry-enabled query path.
        let mut reg = InstitutionRegistry::new();
        reg.register_rehydrated(Box::new(QueryCapInst)).unwrap();

        // Minimal layer — just the core ontology. WHERE runs on an
        // empty initial binding plus a literal-only expression.
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(builder.build());

        // Use FunctionCall directly at eval_expression level for a
        // focused test — the full-query integration would need more
        // pattern-matching infrastructure. This verifies the core
        // dispatch path.
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "urn:eigenius:test:q_positive".to_string(),
            args: vec![Expression::Literal(Literal::Integer(42))],
        };
        let v = eval_expression(&expr, &binding, &layer, Some(&reg)).expect("eval");
        assert_eq!(v, Value::Boolean(true));

        // Negative arg — decide returns Fails → Bool false.
        let expr_neg = Expression::FunctionCall {
            name: "urn:eigenius:test:q_positive".to_string(),
            args: vec![Expression::Literal(Literal::Integer(-5))],
        };
        let v = eval_expression(&expr_neg, &binding, &layer, Some(&reg)).expect("eval");
        assert_eq!(v, Value::Boolean(false));
    }

    #[test]
    fn comorphism_dispatch_produces_translated_resource() {
        let mut reg = InstitutionRegistry::new();
        reg.register_rehydrated(Box::new(QueryCapInst)).unwrap();

        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(builder.build());

        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "urn:eigenius:test:q_translate".to_string(),
            args: vec![Expression::Literal(Literal::String(
                "urn:eigenius:test:some_src".to_string(),
            ))],
        };
        let v = eval_expression(&expr, &binding, &layer, Some(&reg)).expect("eval");
        match v {
            Value::Embedded(r) => {
                assert_eq!(
                    r.id().map(|i| i.as_str()),
                    Some("urn:eigenius:test:q_translated")
                );
            }
            other => panic!("expected embedded translated resource, got {other:?}"),
        }
    }

    #[test]
    fn unknown_iri_falls_through_to_builtin_error() {
        // An IRI that isn't registered as either decide or
        // comorphism falls through to `functions::call_function`,
        // which errors with "no such function."
        let reg = InstitutionRegistry::new();
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(builder.build());

        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "urn:eigenius:test:unknown_fn".to_string(),
            args: vec![],
        };
        let err = eval_expression(&expr, &binding, &layer, Some(&reg)).unwrap_err();
        let msg = format!("{err}");
        // Builtin dispatch rejects unknown function names.
        assert!(msg.contains("unknown") || msg.contains("function"));
    }
}
