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

//! Type checker for EigenQL programs.
//!
//! Validates a parsed AST against the ontology before evaluation.
//! Checks variable binding, USING resolution, and aggregate/GROUP BY consistency.

use crate::institution::registry::{DispatchRole, InstitutionIndex};
use crate::layer::{
    resolve_active_text_indexes, resolve_active_vector_indexes, ActiveTextIndex, ActiveVectorIndex,
    Layer,
};
use crate::ontology::iri::Iri;
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::error::QueryError;
use std::collections::{BTreeMap, BTreeSet};

/// Type-check a parsed EigenQL program against a layer.
///
/// Returns a list of errors (empty if valid).
pub fn type_check(program: &Program, layer: &Layer) -> Vec<QueryError> {
    let mut errors = Vec::new();

    // Build the D14 institution index once for the whole pass — every
    // FIBER / qualified-call check resolves through it.
    let (index, _index_errors) = InstitutionIndex::from_layer(layer);

    // Check DEFINE rules
    for def in &program.definitions {
        check_match_part(&def.body, layer, &mut errors);
    }

    // Check the query
    check_match_part(&program.query.body, layer, &mut errors);

    // FIBER-clause specifics: USING INSTITUTION alias + IRI resolution,
    // FIBER QueryClass / institution-agreement / OnDemand-role checks,
    // param scope + required coverage, comorphism coercion rules.
    check_fiber_clauses(&program.query.body, layer, &index, &mut errors);

    // Qualified-name function calls in expression position must
    // resolve to a Decidable QueryClass under D14 (D2 v2 §5.9).
    for cond in &program.query.body.conditions {
        check_qualified_calls(cond, &index, &mut errors);
    }
    for item in &program.query.result {
        check_qualified_calls(&item.expression, &index, &mut errors);
    }
    for expr in &program.query.group_by {
        check_qualified_calls(expr, &index, &mut errors);
    }
    for item in &program.query.order_by {
        check_qualified_calls(&item.expression, &index, &mut errors);
    }
    if let Some(top_k) = &program.query.top_k_by {
        check_qualified_calls(&top_k.expression, &index, &mut errors);
    }

    // D2 v2 §5.9 — Verdict-typed expression rules. Verdicts only
    // arise from two doorways (Decidable QueryClass call; FIBER ?v
    // bound to a Verdict-result_class QueryClass), so the check
    // reduces to a static is-Verdict-source predicate. No general
    // expression-type inference required.
    let verdict_vars = collect_verdict_bound_vars(program, layer, &index);
    check_verdict_typing(&program.query.body, &verdict_vars, &index, &mut errors);
    for item in &program.query.result {
        check_verdict_in_expression(&item.expression, &verdict_vars, &index, &mut errors);
    }
    for expr in &program.query.group_by {
        check_verdict_in_expression(expr, &verdict_vars, &index, &mut errors);
    }
    for item in &program.query.order_by {
        check_verdict_in_expression(&item.expression, &verdict_vars, &index, &mut errors);
    }
    if let Some(top_k) = &program.query.top_k_by {
        check_verdict_in_expression(&top_k.expression, &verdict_vars, &index, &mut errors);
    }

    // Collect all bound variables from MATCH / FIBER / BIND across
    // the whole program. This is the "everything bound" set used by
    // the RETURN / ORDER BY / TOP K BY / GROUP BY check, where
    // textual order within the WHERE list doesn't matter.
    let bound_vars = collect_bound_variables(program);

    // Check variables used in WHERE are bound.
    for condition in &program.query.body.conditions {
        check_expression_variables(condition, &bound_vars, &mut errors);
    }
    for def in &program.definitions {
        for condition in &def.body.conditions {
            check_expression_variables(condition, &bound_vars, &mut errors);
        }
    }

    // Check variables used in RETURN are bound
    for item in &program.query.result {
        check_expression_variables(&item.expression, &bound_vars, &mut errors);
    }

    // Check variables used in GROUP BY are bound
    for expr in &program.query.group_by {
        check_expression_variables(expr, &bound_vars, &mut errors);
    }

    // Check variables used in ORDER BY are bound
    for item in &program.query.order_by {
        check_expression_variables(&item.expression, &bound_vars, &mut errors);
    }

    // D43 §3.7 — TOP K BY's score expression participates in the
    // same variable-binding contract as ORDER BY. The mutual
    // exclusion with ORDER BY is enforced at parse time, so only
    // one of these two branches has any items in practice.
    if let Some(top_k) = &program.query.top_k_by {
        check_expression_variables(&top_k.expression, &bound_vars, &mut errors);
    }

    // Check aggregate/GROUP BY consistency
    check_aggregate_consistency(program, &mut errors);

    // D43 §4.6 — TEXT_MATCH / TEXT_SCORE typing rules.
    //
    // Builds a property-variable index from MATCH patterns (which
    // `?var` was bound by which property of which class), then walks
    // every expression that can contain a retrieval call (WHERE,
    // RETURN, GROUP BY, ORDER BY, and rule bodies' WHERE) and rejects:
    //   1. arg-count mismatches,
    //   2. arg[0] not a Variable bound through a PropertyPattern,
    //   3. property without an active TextIndex at this head,
    //   4. property whose declared data_type is not `core:string`,
    //   5. arg[1] not a literal string.
    let prop_var_index = build_property_variable_index(program, layer);
    let text_indexes = resolve_active_text_indexes(layer);
    for cond in &program.query.body.conditions {
        check_text_retrieval(cond, &prop_var_index, &text_indexes, layer, &mut errors);
    }
    for item in &program.query.result {
        check_text_retrieval(
            &item.expression,
            &prop_var_index,
            &text_indexes,
            layer,
            &mut errors,
        );
    }
    for expr in &program.query.group_by {
        check_text_retrieval(expr, &prop_var_index, &text_indexes, layer, &mut errors);
    }
    for item in &program.query.order_by {
        check_text_retrieval(
            &item.expression,
            &prop_var_index,
            &text_indexes,
            layer,
            &mut errors,
        );
    }
    if let Some(top_k) = &program.query.top_k_by {
        check_text_retrieval(
            &top_k.expression,
            &prop_var_index,
            &text_indexes,
            layer,
            &mut errors,
        );
    }
    for def in &program.definitions {
        for cond in &def.body.conditions {
            check_text_retrieval(cond, &prop_var_index, &text_indexes, layer, &mut errors);
        }
    }

    // D43 §4.5 — VECTOR_NEAR / VECTOR_SIM typing rules. Same shape
    // as the text-retrieval pass above, applied to all expression
    // positions that can contain a retrieval call. v1 enforces the
    // structural rules (arity, property-bound `?var`, active Index,
    // K literal-integer); the cross-call model agreement and EMBED
    // model inference (§4.4) are the M5 follow-up.
    let vector_indexes = resolve_active_vector_indexes(layer);
    for cond in &program.query.body.conditions {
        check_vector_retrieval(cond, &prop_var_index, &vector_indexes, &mut errors);
    }
    for item in &program.query.result {
        check_vector_retrieval(
            &item.expression,
            &prop_var_index,
            &vector_indexes,
            &mut errors,
        );
    }
    for expr in &program.query.group_by {
        check_vector_retrieval(expr, &prop_var_index, &vector_indexes, &mut errors);
    }
    for item in &program.query.order_by {
        check_vector_retrieval(
            &item.expression,
            &prop_var_index,
            &vector_indexes,
            &mut errors,
        );
    }
    if let Some(top_k) = &program.query.top_k_by {
        check_vector_retrieval(
            &top_k.expression,
            &prop_var_index,
            &vector_indexes,
            &mut errors,
        );
    }
    for def in &program.definitions {
        for cond in &def.body.conditions {
            check_vector_retrieval(cond, &prop_var_index, &vector_indexes, &mut errors);
        }
    }

    errors
}

/// Check a MatchPart: validate USING IRIs resolve to classes.
fn check_match_part(part: &MatchPart, layer: &Layer, errors: &mut Vec<QueryError>) {
    // Check USING IRIs
    let class_iri = Iri::parse(wk::CLASS).unwrap();
    for iri in &part.using {
        match layer.resolve(iri) {
            Some(resource) => {
                if !resource.is_instance_of(&class_iri) {
                    errors.push(QueryError::type_check(
                        "using_not_class",
                        format!("USING '{}' does not resolve to a Class", iri),
                    ));
                }
            }
            None => {
                errors.push(QueryError::type_check(
                    "using_unresolved",
                    format!("USING '{}' does not resolve to any resource", iri),
                ));
            }
        }
    }
}

/// Collect every variable name bound by MATCH patterns, FIBER
/// clauses, and `BIND` items across the program. The result is the
/// universe of bindings visible to RETURN / ORDER BY / TOP K BY /
/// GROUP BY positions.
///
fn collect_bound_variables(program: &Program) -> BTreeSet<String> {
    let mut vars = BTreeSet::new();

    for def in &program.definitions {
        collect_pattern_vars(def.body.patterns(), &mut vars);
        for v in &def.variables {
            vars.insert(v.name.clone());
        }
    }

    collect_pattern_vars(program.query.body.patterns(), &mut vars);
    // FIBER clauses bind a result variable — make it visible to WHERE /
    // RETURN / subsequent MATCH patterns.
    for c in &program.query.body.clauses {
        if let Clause::Fiber(fc) = c {
            vars.insert(fc.binding.name.clone());
        }
    }
    for def in &program.definitions {
        for c in &def.body.clauses {
            if let Clause::Fiber(fc) = c {
                vars.insert(fc.binding.name.clone());
            }
        }
    }
    vars
}

fn collect_pattern_vars<'a>(
    patterns: impl Iterator<Item = &'a Pattern>,
    vars: &mut BTreeSet<String>,
) {
    for pattern in patterns {
        vars.insert(pattern.subject.name.clone());
        for prop in &pattern.properties {
            if let ValueOrVariable::Variable(v) = &prop.object {
                vars.insert(v.name.clone());
            }
        }
    }
}

/// Check that all variables referenced in an expression are bound.
fn check_expression_variables(
    expr: &Expression,
    bound: &BTreeSet<String>,
    errors: &mut Vec<QueryError>,
) {
    match expr {
        Expression::Variable(var) => {
            if !bound.contains(&var.name) {
                errors.push(QueryError::type_check(
                    "unbound_variable",
                    format!("variable '?{}' is not bound in any MATCH pattern", var.name),
                ));
            }
        }
        Expression::Binary { left, right, .. } => {
            check_expression_variables(left, bound, errors);
            check_expression_variables(right, bound, errors);
        }
        Expression::Unary { operand, .. } => {
            check_expression_variables(operand, bound, errors);
        }
        Expression::VerdictPredicate { operand, .. } => {
            check_expression_variables(operand, bound, errors);
        }
        Expression::NotExists(var) => {
            if !bound.contains(&var.name) {
                errors.push(QueryError::type_check(
                    "not_exists_unbound",
                    format!(
                        "NOT EXISTS variable '?{}' is not bound in any MATCH pattern",
                        var.name
                    ),
                ));
            }
        }
        Expression::FunctionCall { args, .. } => {
            for arg in args {
                check_expression_variables(arg, bound, errors);
            }
        }
        Expression::Aggregate { arg, .. } => {
            check_expression_variables(arg, bound, errors);
        }
        Expression::DotPath { root, .. } => {
            if !bound.contains(&root.name) {
                errors.push(QueryError::type_check(
                    "unbound_variable",
                    format!(
                        "dot-path root '?{}' is not bound in any MATCH pattern",
                        root.name
                    ),
                ));
            }
        }
        Expression::Array(elements) => {
            for elem in elements {
                check_expression_variables(elem, bound, errors);
            }
        }
        Expression::Object(pairs) => {
            for (_, value) in pairs {
                check_expression_variables(value, bound, errors);
            }
        }
        Expression::Literal(_) => {}
    }
}

/// Check aggregate/GROUP BY consistency:
/// - Aggregates only in RETURN
/// - Non-aggregated RETURN expressions must appear in GROUP BY
fn check_aggregate_consistency(program: &Program, errors: &mut Vec<QueryError>) {
    // Check aggregates don't appear in WHERE (always invalid, regardless of RETURN)
    for cond in &program.query.body.conditions {
        if expr_has_aggregate(cond) {
            errors.push(QueryError::type_check(
                "aggregate_in_where",
                "aggregate functions are not allowed in WHERE clauses".to_string(),
            ));
        }
    }

    let has_agg = program
        .query
        .result
        .iter()
        .any(|item| expr_has_aggregate(&item.expression));

    if !has_agg {
        return;
    }

    // If we have aggregates in RETURN, check that non-aggregated return expressions
    // appear in GROUP BY
    if program.query.group_by.is_empty() {
        for item in &program.query.result {
            if !expr_has_aggregate(&item.expression) {
                errors.push(QueryError::type_check(
                    "aggregate_without_group_by",
                    format!(
                        "return item '{:?}' is not an aggregate but no GROUP BY is specified",
                        item.name
                    ),
                ));
            }
        }
    }
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
        Expression::Array(elements) => elements.iter().any(expr_has_aggregate),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// FIBER-clause checks (D2 §5.8)
// ---------------------------------------------------------------------------

fn check_fiber_clauses(
    part: &MatchPart,
    layer: &Layer,
    index: &InstitutionIndex,
    errors: &mut Vec<QueryError>,
) {
    // D2 v2 §5.7 — USING INSTITUTION uniqueness and IRI resolution.
    let mut alias_set: BTreeSet<&str> = BTreeSet::new();
    let mut alias_iri: std::collections::BTreeMap<&str, Iri> = std::collections::BTreeMap::new();
    for alias in &part.using_institutions {
        if !alias_set.insert(alias.alias.as_str()) {
            errors.push(QueryError::type_check(
                "duplicate_using_institution_alias",
                format!("duplicate USING INSTITUTION alias: '{}'", alias.alias),
            ));
        }
        alias_iri.insert(alias.alias.as_str(), alias.iri.clone());
        if index.institution(&alias.iri).is_none() {
            errors.push(QueryError::type_check(
                "using_institution_unresolved",
                format!(
                    "USING INSTITUTION '{}' does not resolve to an indexed Institution",
                    alias.iri
                ),
            ));
        }
    }

    // D2 v2 §5.8 — each FIBER clause.
    let requires_prop = Iri::parse(wk::REQUIRES).unwrap();
    let recommends_prop = Iri::parse(wk::RECOMMENDS).unwrap();
    let short_name_prop = Iri::parse(wk::SHORT_NAME).unwrap();

    for c in &part.clauses {
        let fc = match c {
            Clause::Fiber(fc) => fc,
            _ => continue,
        };

        // 1. Institution ref — alias must be declared, or inline IRI must
        //    resolve to an indexed Institution. Capture the resolved IRI
        //    for the institution-agreement check below.
        let aliased_inst_iri: Option<Iri> = match &fc.institution {
            Name::ShortName(alias) => {
                if !alias_set.contains(alias.as_str()) {
                    errors.push(QueryError::type_check(
                        "undeclared_institution_alias",
                        format!(
                            "FIBER refers to undeclared institution alias '{alias}' — \
                             add `USING INSTITUTION \"...\" AS {alias}` or use an inline IRI"
                        ),
                    ));
                    None
                } else {
                    alias_iri.get(alias.as_str()).cloned()
                }
            }
            Name::FullIri(iri) => {
                if index.institution(iri).is_none() {
                    errors.push(QueryError::type_check(
                        "using_institution_unresolved",
                        format!(
                            "FIBER inline institution '{iri}' does not resolve to an indexed Institution"
                        ),
                    ));
                    None
                } else {
                    Some(iri.clone())
                }
            }
        };

        // 2. Resolve the QueryClass against the index. Short-name lookup
        //    walks indexed QueryClass declarations by their resource
        //    short_name.
        let qc_iri = match &fc.query_class {
            Name::FullIri(iri) => Some(iri.clone()),
            Name::ShortName(short) => resolve_short_name_to_query_class(layer, short),
        };
        let qc_entry = qc_iri.as_ref().and_then(|i| index.query_class(i));
        let qc_entry = match qc_entry {
            Some(e) => e,
            None => {
                errors.push(QueryError::type_check(
                    "fiber_query_class_not_query_class",
                    format!(
                        "FIBER query class '{}' does not resolve to an indexed QueryClass declaration",
                        query_class_name_display(&fc.query_class)
                    ),
                ));
                continue;
            }
        };

        // 3. QueryClass must include OnDemand in its dispatch_role set.
        if !qc_entry.dispatch_roles.contains(&DispatchRole::OnDemand) {
            errors.push(QueryError::type_check(
                "fiber_query_class_not_on_demand",
                format!(
                    "FIBER query class '{}' has no OnDemand dispatch role — \
                     declare on_demand on the QueryClass to allow FIBER dispatch",
                    qc_entry.iri
                ),
            ));
        }

        // 4. The QueryClass's institution_ref must equal the aliased
        //    institution.
        if let Some(ref aliased) = aliased_inst_iri {
            if qc_entry.institution_ref != *aliased {
                errors.push(QueryError::type_check(
                    "fiber_institution_mismatch",
                    format!(
                        "FIBER cites institution '{}' but QueryClass '{}' declares institution_ref '{}'",
                        aliased, qc_entry.iri, qc_entry.institution_ref
                    ),
                ));
            }
        }

        // 5. Param scope: short-name params must resolve in the
        //    QueryClass's input class (requires ∪ recommends). Required
        //    params must all be supplied.
        let input_class_resource = match layer.resolve(&qc_entry.query_class) {
            Some(r) => r.clone(),
            None => {
                // The QueryClass declares an input class IRI that
                // doesn't resolve in the chain. The runtime would
                // surface this; flag it.
                errors.push(QueryError::type_check(
                    "fiber_query_class_not_query_class",
                    format!(
                        "QueryClass '{}' declares input class '{}' which does not resolve in the layer chain",
                        qc_entry.iri, qc_entry.query_class
                    ),
                ));
                continue;
            }
        };

        let mut allowed_prop_iris: BTreeSet<String> = BTreeSet::new();
        let mut required_prop_iris: BTreeSet<String> = BTreeSet::new();
        let mut short_to_iri: BTreeMap<String, String> = BTreeMap::new();

        for iri in collect_property_iris(&input_class_resource, &requires_prop) {
            allowed_prop_iris.insert(iri.as_str().to_string());
            // `is_a` is auto-stamped by `apply_fiber_clause` from the
            // QueryClass's declared input class — the user can't be
            // required to supply it. `short_name` is chain-commit
            // bookkeeping (used for short-name resolution on persisted
            // resources) and irrelevant to a FIBER-synthesized
            // transient input. Both legitimately appear in the input
            // class's `requires` (a FIBER QueryClass may still admit
            // direct chain commits, where these matter), but for the
            // FIBER dispatch the kernel handles them — the type-check
            // must skip them or every FIBER call ends up boilerplated
            // with `is_a: …, short_name: …` lines.
            if iri.as_str() != wk::IS_A && iri.as_str() != wk::SHORT_NAME {
                required_prop_iris.insert(iri.as_str().to_string());
            }
        }
        for iri in collect_property_iris(&input_class_resource, &recommends_prop) {
            allowed_prop_iris.insert(iri.as_str().to_string());
        }
        for iri in &allowed_prop_iris {
            if let Ok(iri_parsed) = Iri::parse(iri) {
                if let Some(prop_res) = layer.resolve(&iri_parsed) {
                    if let Some(crate::ontology::resource::Value::String(s)) =
                        prop_res.get(&short_name_prop)
                    {
                        short_to_iri.insert(s.clone(), iri.clone());
                    }
                }
            }
        }

        let mut supplied_iris: BTreeSet<String> = BTreeSet::new();
        for param in &fc.params {
            let resolved_iri = match &param.name {
                Name::FullIri(iri) => Some(iri.as_str().to_string()),
                Name::ShortName(short) => {
                    if let Some(iri) = short_to_iri.get(short) {
                        Some(iri.clone())
                    } else {
                        errors.push(QueryError::type_check(
                            "fiber_param_short_name_unresolved",
                            format!(
                                "FIBER param '{short}' is not a declared property of \
                                 QueryClass input class '{}' (requires ∪ recommends)",
                                qc_entry.query_class
                            ),
                        ));
                        None
                    }
                }
            };
            if let Some(ref iri) = resolved_iri {
                supplied_iris.insert(iri.clone());
            }

            // 6. Comorphism coercion sub-checks (D2 v2 §5.8 step 9).
            if let ParamValue::Comorphism { name, source } = &param.value {
                check_comorphism_coercion(
                    name,
                    source,
                    qc_entry,
                    aliased_inst_iri.as_ref(),
                    resolved_iri.as_deref(),
                    layer,
                    index,
                    errors,
                );
            }
        }

        for req in &required_prop_iris {
            if !supplied_iris.contains(req) {
                errors.push(QueryError::type_check(
                    "fiber_missing_required_param",
                    format!(
                        "FIBER for QueryClass '{}' is missing required param '{}'",
                        qc_entry.iri, req
                    ),
                ));
            }
        }
    }
}

/// D2 v2 §5.8 step 9 — comorphism-coercion sub-checks.
#[allow(clippy::too_many_arguments)]
fn check_comorphism_coercion(
    name: &Name,
    source: &Expression,
    qc_entry: &crate::institution::registry::QueryClassEntry,
    aliased_inst_iri: Option<&Iri>,
    target_param_iri: Option<&str>,
    layer: &Layer,
    index: &InstitutionIndex,
    errors: &mut Vec<QueryError>,
) {
    let _ = (qc_entry, source); // qc_entry used only for context; source typed-checked via expression-variable walk
                                // Resolve the comorphism IRI.
    let comorphism_iri = match name {
        Name::FullIri(i) => i.clone(),
        Name::ShortName(s) => match Iri::parse(s) {
            Ok(i) => i,
            Err(_) => {
                errors.push(QueryError::type_check(
                    "comorphism_unresolved",
                    format!("comorphism coercion `{s}` is not a parseable IRI"),
                ));
                return;
            }
        },
    };
    let comorphism = match index.comorphism(&comorphism_iri) {
        Some(c) => c,
        None => {
            errors.push(QueryError::type_check(
                "comorphism_unresolved",
                format!(
                    "comorphism coercion '{comorphism_iri}' does not resolve to an indexed Comorphism"
                ),
            ));
            return;
        }
    };

    // Target-side institution must equal the FIBER's aliased institution.
    let import = match index.import_format(&comorphism.import_format) {
        Some(i) => i,
        None => {
            errors.push(QueryError::type_check(
                "comorphism_unresolved",
                format!(
                    "comorphism '{comorphism_iri}' references import_format '{}' which is not indexed",
                    comorphism.import_format
                ),
            ));
            return;
        }
    };
    if let Some(aliased) = aliased_inst_iri {
        if import.institution_ref != *aliased {
            errors.push(QueryError::type_check(
                "comorphism_target_mismatch",
                format!(
                    "comorphism '{comorphism_iri}' reifies into institution '{}' but FIBER cites '{aliased}'",
                    import.institution_ref
                ),
            ));
        }
    }

    // The reified target class must satisfy the FIBER param's declared
    // class_types (D2 v2 §5.8 step 9d).
    if let Some(param_iri_str) = target_param_iri {
        if let Ok(param_iri) = Iri::parse(param_iri_str) {
            if let Some(prop_res) = layer.resolve(&param_iri) {
                let class_types_iri = Iri::parse("urn:eigenius:core:class_types").unwrap();
                if let Some(crate::ontology::resource::Value::Array(items)) =
                    prop_res.get(&class_types_iri)
                {
                    let accepted: Vec<Iri> = items
                        .iter()
                        .filter_map(|v| match v {
                            crate::ontology::resource::Value::String(s) => Iri::parse(s).ok(),
                            crate::ontology::resource::Value::ResourceRef(i) => Some(i.clone()),
                            _ => None,
                        })
                        .collect();
                    if !accepted.is_empty() && !accepted.contains(&import.to_class) {
                        errors.push(QueryError::type_check(
                            "comorphism_target_class_mismatch",
                            format!(
                                "comorphism '{comorphism_iri}' produces an instance of '{}' but \
                                 FIBER param '{param_iri_str}' declares class_types {accepted:?}",
                                import.to_class
                            ),
                        ));
                    }
                }
            }
        }
    }

    // v1 restriction: transformation Component must be Pure / Read.
    let cap_level_iri = Iri::parse("urn:eigenius:program:component:capability_level").unwrap();
    if let Some(comp_res) = layer.resolve(&comorphism.transformation) {
        if let Some(crate::ontology::resource::Value::String(level)) = comp_res.get(&cap_level_iri)
        {
            if level == "urn:eigenius:program:capability_levels:io" {
                errors.push(QueryError::type_check(
                    "comorphism_io_not_supported_in_v1",
                    format!(
                        "comorphism '{comorphism_iri}' transformation '{}' has IO capability — \
                         v1 restricts FIBER coercion transformations to Pure or Read",
                        comorphism.transformation
                    ),
                ));
            }
        }
    }
}

/// D2 v2 §5.9 — qualified-name function calls in expression position
/// must resolve to an indexed Decidable QueryClass. Untyped/unknown
/// IRIs fall through to evaluation-time `unknown function` (no
/// type-check error so late institution registration stays valid).
fn check_qualified_calls(
    expr: &Expression,
    index: &InstitutionIndex,
    errors: &mut Vec<QueryError>,
) {
    match expr {
        Expression::FunctionCall { name, args } => {
            for a in args {
                check_qualified_calls(a, index, errors);
            }
            if name.contains(':') {
                if let Ok(iri) = Iri::parse(name) {
                    if let Some(qc) = index.query_class(&iri) {
                        if !qc.dispatch_roles.contains(&DispatchRole::Decidable) {
                            errors.push(QueryError::type_check(
                                "qualified_call_not_decidable",
                                format!(
                                    "qualified function call '{name}' resolves to QueryClass '{}' \
                                     but its dispatch_role does not include Decidable — \
                                     use FIBER for OnDemand QueryClasses",
                                    qc.iri
                                ),
                            ));
                        }
                    }
                    // No QueryClass entry → fall-through to builtin /
                    // unknown-function at evaluation. Comorphism IRIs in
                    // expression position are not classified here; they
                    // also fall through and surface at evaluation as
                    // unknown function.
                }
            }
        }
        Expression::Binary { left, right, .. } => {
            check_qualified_calls(left, index, errors);
            check_qualified_calls(right, index, errors);
        }
        Expression::Unary { operand, .. } => {
            check_qualified_calls(operand, index, errors);
        }
        Expression::VerdictPredicate { operand, .. } => {
            check_qualified_calls(operand, index, errors);
        }
        Expression::Aggregate { arg, .. } => {
            check_qualified_calls(arg, index, errors);
        }
        Expression::Array(items) => {
            for it in items {
                check_qualified_calls(it, index, errors);
            }
        }
        Expression::Object(pairs) => {
            for (_, v) in pairs {
                check_qualified_calls(v, index, errors);
            }
        }
        _ => {}
    }
}

// ─── D2 v2 §5.9 — Verdict-typed expression rules ──────────────────────

/// Collect every variable name that's FIBER-bound to a Verdict
/// resource. Under D14 these are the only Verdict-typed `?var`
/// references in EigenQL — Verdicts have no algebra, so a static
/// "is this a Verdict source?" predicate is sufficient (no general
/// type inference required).
fn collect_verdict_bound_vars(
    program: &Program,
    layer: &Layer,
    index: &InstitutionIndex,
) -> BTreeSet<String> {
    let verdict_iri = Iri::parse(wk::VERDICT).expect("well-known IRI");
    let mut verdict_vars = BTreeSet::new();
    let visit = |part: &MatchPart, set: &mut BTreeSet<String>| {
        for clause in &part.clauses {
            if let Clause::Fiber(fc) = clause {
                let qc_iri = match &fc.query_class {
                    Name::FullIri(iri) => Some(iri.clone()),
                    Name::ShortName(short) => resolve_short_name_to_query_class(layer, short),
                };
                if let Some(iri) = qc_iri {
                    if let Some(qc) = index.query_class(&iri) {
                        if qc.result_class == verdict_iri {
                            set.insert(fc.binding.name.clone());
                        }
                    }
                }
            }
        }
    };
    visit(&program.query.body, &mut verdict_vars);
    for def in &program.definitions {
        visit(&def.body, &mut verdict_vars);
    }
    verdict_vars
}

/// Decide whether `expr` is statically a Verdict source (D2 v2 §3.8 /
/// §6.13). Only two productions count:
///
/// 1. A qualified-name function call `qc:check(args)` where the IRI
///    resolves to a `Decidable` QueryClass.
/// 2. A `?v` reference where `?v` is bound by a FIBER clause whose
///    QueryClass declares `result_class = Verdict`.
///
/// All other expression shapes return `false` — Verdicts have no
/// algebra (no operator that consumes a Verdict and yields a Verdict),
/// so propagation through binary / unary / aggregate / dot-path / etc.
/// is structurally impossible.
fn is_verdict_source(
    expr: &Expression,
    verdict_vars: &BTreeSet<String>,
    index: &InstitutionIndex,
) -> bool {
    match expr {
        Expression::FunctionCall { name, .. } if name.contains(':') => Iri::parse(name)
            .ok()
            .and_then(|iri| index.query_class(&iri))
            .is_some_and(|qc| qc.dispatch_roles.contains(&DispatchRole::Decidable)),
        Expression::Variable(v) => verdict_vars.contains(&v.name),
        _ => false,
    }
}

/// Check the WHERE conditions of a MatchPart for D2 v2 §3.8 / §5.9
/// rules:
///
/// - `verdict_predicate_non_verdict_operand` — postfix `HOLDS` /
///   `FAILS` / `UNDECIDABLE` over a non-Verdict-source operand.
/// - `bare_verdict_in_boolean_position` — a Verdict source appearing
///   directly in WHERE (or as an AND/OR/NOT operand) without a
///   wrapping postfix predicate.
fn check_verdict_typing(
    part: &MatchPart,
    verdict_vars: &BTreeSet<String>,
    index: &InstitutionIndex,
    errors: &mut Vec<QueryError>,
) {
    for item in &part.conditions {
        let cond = item;
        // Top-level WHERE expression: must NOT itself be a Verdict
        // source (forces explicit projection).
        check_boolean_position(cond, verdict_vars, index, errors);
        // Recurse into sub-expressions for postfix-operand checks
        // and AND/OR/NOT-operand bare-Verdict checks.
        check_verdict_in_expression(cond, verdict_vars, index, errors);
    }
}

/// Recursively walk `expr` checking every `VerdictPredicate { operand }`
/// node and every Boolean-required sub-position.
fn check_verdict_in_expression(
    expr: &Expression,
    verdict_vars: &BTreeSet<String>,
    index: &InstitutionIndex,
    errors: &mut Vec<QueryError>,
) {
    match expr {
        Expression::VerdictPredicate { kind, operand } => {
            if !is_verdict_source(operand, verdict_vars, index) {
                errors.push(QueryError::type_check(
                    "verdict_predicate_non_verdict_operand",
                    format!(
                        "postfix `{kw}` requires a Verdict-typed operand (a Decidable \
                         QueryClass call, or a FIBER-bound variable whose result_class \
                         is Verdict); given operand is not a Verdict source",
                        kw = kind.ctor_name(),
                    ),
                ));
            }
            check_verdict_in_expression(operand, verdict_vars, index, errors);
        }
        Expression::Binary { op, left, right } => {
            // AND / OR are Boolean-position contexts; their operands
            // must not be bare Verdict sources.
            if matches!(op, BinaryOp::And | BinaryOp::Or) {
                check_boolean_position(left, verdict_vars, index, errors);
                check_boolean_position(right, verdict_vars, index, errors);
            }
            check_verdict_in_expression(left, verdict_vars, index, errors);
            check_verdict_in_expression(right, verdict_vars, index, errors);
        }
        Expression::Unary { op, operand } => {
            // `NOT operand` requires Boolean.
            if matches!(op, UnaryOp::Not) {
                check_boolean_position(operand, verdict_vars, index, errors);
            }
            check_verdict_in_expression(operand, verdict_vars, index, errors);
        }
        Expression::FunctionCall { args, .. } => {
            for a in args {
                check_verdict_in_expression(a, verdict_vars, index, errors);
            }
        }
        Expression::Aggregate { arg, .. } => {
            check_verdict_in_expression(arg, verdict_vars, index, errors);
        }
        Expression::Array(items) => {
            for it in items {
                check_verdict_in_expression(it, verdict_vars, index, errors);
            }
        }
        Expression::Object(pairs) => {
            for (_, v) in pairs {
                check_verdict_in_expression(v, verdict_vars, index, errors);
            }
        }
        _ => {}
    }
}

/// A Boolean-required position (top-level WHERE, AND/OR/NOT operand)
/// rejects a bare Verdict source — the user must apply a postfix
/// predicate (`?v HOLDS`, etc.) to project to Boolean.
fn check_boolean_position(
    expr: &Expression,
    verdict_vars: &BTreeSet<String>,
    index: &InstitutionIndex,
    errors: &mut Vec<QueryError>,
) {
    if is_verdict_source(expr, verdict_vars, index) {
        let display = match expr {
            Expression::FunctionCall { name, .. } => format!("`{name}(...)`"),
            Expression::Variable(v) => format!("`?{}`", v.name),
            _ => "this expression".to_string(),
        };
        errors.push(QueryError::type_check(
            "bare_verdict_in_boolean_position",
            format!(
                "{display} evaluates to a Verdict but appears in a Boolean position — \
                 apply a postfix predicate (`HOLDS`, `FAILS`, or `UNDECIDABLE`) to \
                 project to Boolean"
            ),
        ));
    }
}

fn collect_property_iris(class_resource: &Resource, prop_iri: &Iri) -> Vec<Iri> {
    use crate::ontology::resource::Value;
    match class_resource.get(prop_iri) {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Iri::parse(s).ok(),
                Value::ResourceRef(i) => Some(i.clone()),
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => Iri::parse(s).ok().into_iter().collect(),
        Some(Value::ResourceRef(i)) => vec![i.clone()],
        _ => Vec::new(),
    }
}

/// Resolve a `FIBER fc.query_class` short name against indexed
/// QueryClass declarations. The QueryClass class itself is not a
/// `urn:eigenius:core:Class` instance — it is its own ontology class
/// — so the lookup filters on `is_a == QueryClass` directly.
fn resolve_short_name_to_query_class(layer: &Layer, short: &str) -> Option<Iri> {
    use crate::ontology::resource::Value;
    let qc_class_iri = Iri::parse(wk::QUERY_CLASS_CLASS).unwrap();
    let short_prop = Iri::parse(wk::SHORT_NAME).unwrap();
    for (iri, res) in layer.iter_all_resources() {
        if !res.is_instance_of(&qc_class_iri) {
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

fn query_class_name_display(name: &Name) -> String {
    match name {
        Name::ShortName(s) => s.clone(),
        Name::FullIri(i) => i.as_str().to_string(),
    }
}

// ─── D43 §4.6 — TEXT_MATCH / TEXT_SCORE typing rules ─────────────

/// Schema view a retrieval call needs: which property a `?var` was
/// bound to, plus an optional class hint from the MATCH pattern.
///
/// Recorded per property-bound variable. The pattern
/// `MATCH Document(?d) { description: ?desc }` produces an entry
/// `?desc → PropertyBinding { property_iri: …:description,
/// class_iri: Some(…:Document) }`. Variables bound directly as
/// subjects (`?d` above) are not recorded — retrieval calls take a
/// property-value variable, not a subject variable.
struct PropertyBinding {
    /// IRI of the Property whose value bound the variable.
    property_iri: Iri,
}

/// Walk every MATCH pattern in the program and build the
/// variable → PropertyBinding map. Used by TEXT_MATCH / TEXT_SCORE
/// typing to recover the source property of a retrieval call's
/// first argument.
fn build_property_variable_index(
    program: &Program,
    layer: &Layer,
) -> BTreeMap<String, PropertyBinding> {
    let mut out: BTreeMap<String, PropertyBinding> = BTreeMap::new();
    let mut visit = |patterns: &[Pattern]| {
        for pat in patterns {
            for pp in &pat.properties {
                if let ValueOrVariable::Variable(var) = &pp.object {
                    if let Some(property_iri) = resolve_property_name(&pp.property, layer) {
                        out.entry(var.name.clone())
                            .or_insert(PropertyBinding { property_iri });
                    }
                }
            }
        }
    };
    let collect = |part: &MatchPart| -> Vec<Pattern> { part.patterns().cloned().collect() };
    visit(&collect(&program.query.body));
    for def in &program.definitions {
        visit(&collect(&def.body));
    }
    out
}

/// Resolve a property `Name` to its IRI. `FullIri` returns the IRI
/// directly; `ShortName` scans the layer chain for a Property
/// Resource whose `short_name` matches.
fn resolve_property_name(name: &Name, layer: &Layer) -> Option<Iri> {
    match name {
        Name::FullIri(iri) => Some(iri.clone()),
        Name::ShortName(s) => {
            use crate::ontology::resource::Value;
            let prop_class = Iri::parse(wk::PROPERTY).ok()?;
            let short_prop = Iri::parse(wk::SHORT_NAME).ok()?;
            for (iri, res) in layer.iter_all_resources() {
                if !res.is_instance_of(&prop_class) {
                    continue;
                }
                if let Some(Value::String(sn)) = res.get(&short_prop) {
                    if sn == s {
                        return Some(iri.clone());
                    }
                }
            }
            None
        }
    }
}

/// Recursively walk `expr`, checking every TEXT_MATCH / TEXT_SCORE
/// call against the schema view per D43 §4.6.
fn check_text_retrieval(
    expr: &Expression,
    prop_var_index: &BTreeMap<String, PropertyBinding>,
    text_indexes: &[ActiveTextIndex],
    layer: &Layer,
    errors: &mut Vec<QueryError>,
) {
    match expr {
        Expression::FunctionCall { name, args } => {
            if name == "TEXT_MATCH" || name == "TEXT_SCORE" {
                check_text_retrieval_call(name, args, prop_var_index, text_indexes, layer, errors);
            }
            for a in args {
                check_text_retrieval(a, prop_var_index, text_indexes, layer, errors);
            }
        }
        Expression::Binary { left, right, .. } => {
            check_text_retrieval(left, prop_var_index, text_indexes, layer, errors);
            check_text_retrieval(right, prop_var_index, text_indexes, layer, errors);
        }
        Expression::Unary { operand, .. } | Expression::VerdictPredicate { operand, .. } => {
            check_text_retrieval(operand, prop_var_index, text_indexes, layer, errors);
        }
        Expression::Aggregate { arg, .. } => {
            check_text_retrieval(arg, prop_var_index, text_indexes, layer, errors);
        }
        Expression::Array(items) => {
            for it in items {
                check_text_retrieval(it, prop_var_index, text_indexes, layer, errors);
            }
        }
        Expression::Object(pairs) => {
            for (_, v) in pairs {
                check_text_retrieval(v, prop_var_index, text_indexes, layer, errors);
            }
        }
        _ => {}
    }
}

/// Enforce the per-call structural and schema-view requirements.
fn check_text_retrieval_call(
    fn_name: &str,
    args: &[Expression],
    prop_var_index: &BTreeMap<String, PropertyBinding>,
    text_indexes: &[ActiveTextIndex],
    layer: &Layer,
    errors: &mut Vec<QueryError>,
) {
    if args.len() != 2 {
        errors.push(QueryError::type_check(
            "text_retrieval_arity",
            format!(
                "{fn_name} requires exactly 2 arguments: a property-bound variable and a query string \
                 (got {} arguments)",
                args.len()
            ),
        ));
        return;
    }

    // Argument 0 — must be a `?var` bound by a property pattern.
    let var = match &args[0] {
        Expression::Variable(v) => v,
        _ => {
            errors.push(QueryError::type_check(
                "text_retrieval_arg0_not_variable",
                format!(
                    "{fn_name} first argument must be a property-bound variable (e.g. `?desc` \
                     from `MATCH Class(?c) {{ description: ?desc }}`)"
                ),
            ));
            return;
        }
    };
    let binding = match prop_var_index.get(&var.name) {
        Some(b) => b,
        None => {
            errors.push(QueryError::type_check(
                "text_retrieval_arg0_not_property_bound",
                format!(
                    "{fn_name} first argument `?{}` is not bound by a property pattern — \
                     it must appear as the object of a `{{ prop: ?var }}` binding in MATCH",
                    var.name
                ),
            ));
            return;
        }
    };

    // Argument 1 — must be a literal string.
    if !matches!(&args[1], Expression::Literal(Literal::String(_))) {
        errors.push(QueryError::type_check(
            "text_retrieval_arg1_not_string_literal",
            format!("{fn_name} second argument must be a literal query string"),
        ));
        // No early return — still validate the property side.
    }

    // Schema view: property must be string-typed and must have an
    // active TextIndex at this head.
    let property_iri = &binding.property_iri;
    if !property_is_string_typed(property_iri, layer) {
        errors.push(QueryError::type_check(
            "text_retrieval_property_not_string",
            format!(
                "{fn_name} requires a String-typed property; `{}` has a non-string data_type",
                property_iri.as_str()
            ),
        ));
    }
    if !text_indexes
        .iter()
        .any(|ti| ti.target_property == *property_iri)
    {
        errors.push(QueryError::type_check(
            "text_retrieval_no_active_index",
            format!(
                "property `{}` has no active TextIndex at this head — declare a \
                 `core:TextIndex` Resource targeting it before using {fn_name}",
                property_iri.as_str()
            ),
        ));
    }
}

/// Does the Property Resource at `property_iri` declare
/// `data_type: core:string`?
///
/// Properties without a `data_type` slot are treated as
/// non-string-typed — defensive: the typechecker should never
/// silently pass a retrieval call against a property whose data
/// shape is unspecified.
fn property_is_string_typed(property_iri: &Iri, layer: &Layer) -> bool {
    let resource = match layer.resolve(property_iri) {
        Some(r) => r,
        None => return false,
    };
    let data_type_prop = match Iri::parse(wk::DATA_TYPE_PROP) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let string_iri = match Iri::parse(wk::STRING) {
        Ok(i) => i,
        Err(_) => return false,
    };
    match resource.get(&data_type_prop) {
        Some(v) => v
            .as_iri_str()
            .and_then(|s| Iri::parse(s).ok())
            .map(|iri| iri == string_iri)
            .unwrap_or(false),
        None => false,
    }
}

// ─── D43 §4.5 — VECTOR_NEAR / VECTOR_SIM typing rules ────────────

/// Recursively walk `expr`, checking every `VECTOR_NEAR` /
/// `VECTOR_SIM` call against the schema view per D43 §4.5.
fn check_vector_retrieval(
    expr: &Expression,
    prop_var_index: &BTreeMap<String, PropertyBinding>,
    vector_indexes: &[ActiveVectorIndex],
    errors: &mut Vec<QueryError>,
) {
    match expr {
        Expression::FunctionCall { name, args } => {
            if name == "VECTOR_NEAR" || name == "VECTOR_SIM" {
                check_vector_retrieval_call(name, args, prop_var_index, vector_indexes, errors);
            }
            for a in args {
                check_vector_retrieval(a, prop_var_index, vector_indexes, errors);
            }
        }
        Expression::Binary { left, right, .. } => {
            check_vector_retrieval(left, prop_var_index, vector_indexes, errors);
            check_vector_retrieval(right, prop_var_index, vector_indexes, errors);
        }
        Expression::Unary { operand, .. } | Expression::VerdictPredicate { operand, .. } => {
            check_vector_retrieval(operand, prop_var_index, vector_indexes, errors);
        }
        Expression::Aggregate { arg, .. } => {
            check_vector_retrieval(arg, prop_var_index, vector_indexes, errors);
        }
        Expression::Array(items) => {
            for it in items {
                check_vector_retrieval(it, prop_var_index, vector_indexes, errors);
            }
        }
        Expression::Object(pairs) => {
            for (_, v) in pairs {
                check_vector_retrieval(v, prop_var_index, vector_indexes, errors);
            }
        }
        _ => {}
    }
}

/// v1 positional signatures:
///   VECTOR_NEAR(?vec, query_vec, K)    — Boolean
///   VECTOR_SIM(?vec, query_vec)        — Float
///
/// `?vec` must be a property-bound variable whose source property
/// has an active VectorIndex at this head. `query_vec` is allowed
/// to be any expression — full Vector-type tracking (so model and
/// dim of `query_vec` match the active VectorIndex's declared
/// values) is the M5 follow-up. `K` must be a positive integer
/// literal — the planner needs it statically (D43 §4.5).
fn check_vector_retrieval_call(
    fn_name: &str,
    args: &[Expression],
    prop_var_index: &BTreeMap<String, PropertyBinding>,
    vector_indexes: &[ActiveVectorIndex],
    errors: &mut Vec<QueryError>,
) {
    // VECTOR_NEAR accepts 3 args (?vec, query_vec, K) or 4
    // (?vec, query_vec, K, ef). VECTOR_SIM always takes 2.
    let arity_ok = match fn_name {
        "VECTOR_NEAR" => args.len() == 3 || args.len() == 4,
        _ => args.len() == 2,
    };
    if !arity_ok {
        errors.push(QueryError::type_check(
            "vector_retrieval_arity",
            format!(
                "{fn_name} {arity_desc} (got {} arguments)",
                args.len(),
                arity_desc = if fn_name == "VECTOR_NEAR" {
                    "requires 3 or 4 arguments (?vec, query_vec, K, ef?)"
                } else {
                    "requires exactly 2 arguments"
                }
            ),
        ));
        return;
    }

    // Argument 0 — must be a `?var` bound by a property pattern.
    let var = match &args[0] {
        Expression::Variable(v) => v,
        _ => {
            errors.push(QueryError::type_check(
                "vector_retrieval_arg0_not_variable",
                format!(
                    "{fn_name} first argument must be a property-bound variable (e.g. `?vec` \
                     from `MATCH Class(?c) {{ embedding: ?vec }}`)"
                ),
            ));
            return;
        }
    };
    let binding = match prop_var_index.get(&var.name) {
        Some(b) => b,
        None => {
            errors.push(QueryError::type_check(
                "vector_retrieval_arg0_not_property_bound",
                format!(
                    "{fn_name} first argument `?{}` is not bound by a property pattern — \
                     it must appear as the object of a `{{ prop: ?var }}` binding in MATCH",
                    var.name
                ),
            ));
            return;
        }
    };

    let property_iri = &binding.property_iri;
    if !vector_indexes
        .iter()
        .any(|vi| vi.target_property == *property_iri)
    {
        errors.push(QueryError::type_check(
            "vector_retrieval_no_active_index",
            format!(
                "property `{}` has no active VectorIndex at this head — declare a \
                 `core:VectorIndex` Resource targeting it before using {fn_name}",
                property_iri.as_str()
            ),
        ));
    }

    // VECTOR_NEAR's K must be a positive integer literal.
    if fn_name == "VECTOR_NEAR" {
        match &args[2] {
            Expression::Literal(Literal::Integer(k)) if *k > 0 => {}
            Expression::Literal(Literal::Integer(_)) => {
                errors.push(QueryError::type_check(
                    "vector_retrieval_k_not_positive",
                    "VECTOR_NEAR `k` argument must be a positive integer literal".to_string(),
                ));
            }
            _ => {
                errors.push(QueryError::type_check(
                    "vector_retrieval_k_not_literal",
                    "VECTOR_NEAR `k` argument must be a positive integer literal".to_string(),
                ));
            }
        }
        // Optional `ef` — same positive-integer-literal constraint
        // (D43 §4.5: "K and (if present) E are positive integer
        // literals — the planner requires statically-known values
        // to push them into index probes").
        if args.len() == 4 {
            match &args[3] {
                Expression::Literal(Literal::Integer(ef)) if *ef > 0 => {}
                Expression::Literal(Literal::Integer(_)) => {
                    errors.push(QueryError::type_check(
                        "vector_retrieval_ef_not_positive",
                        "VECTOR_NEAR `ef` argument must be a positive integer literal".to_string(),
                    ));
                }
                _ => {
                    errors.push(QueryError::type_check(
                        "vector_retrieval_ef_not_literal",
                        "VECTOR_NEAR `ef` argument must be a positive integer literal".to_string(),
                    ));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Imports used above — keep below the public surface to avoid polluting
// the top.
// ---------------------------------------------------------------------------

use crate::ontology::resource::Resource;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::eigon_json;
    use crate::query::lexer::tokenize;
    use crate::query::parser;
    use std::sync::Arc;

    fn build_core_layer() -> Arc<Layer> {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in resources {
            builder.add_resource(r).unwrap();
        }
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    fn check(layer: &Layer, query_str: &str) -> Vec<QueryError> {
        let tokens = tokenize(query_str).unwrap();
        let program = parser::parse(tokens).unwrap();
        type_check(&program, layer)
    }

    #[test]
    fn valid_query_no_errors() {
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            USING "urn:eigenius:core:Class"
            MATCH Class(?c) { short_name: ?name }
            RETURN [] { short_name: ?name }
            "#,
        );
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn unbound_variable_in_return() {
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x { short_name: ?name }
            RETURN [] { other: ?unknown }
            "#,
        );
        assert!(
            errors.iter().any(|e| e.rule == "unbound_variable"),
            "expected unbound_variable error"
        );
    }

    #[test]
    fn unbound_variable_in_where() {
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x { short_name: ?name }
            WHERE ?missing = "foo"
            "#,
        );
        assert!(errors.iter().any(|e| e.rule == "unbound_variable"));
    }

    #[test]
    fn using_resolves_to_class() {
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            USING "urn:eigenius:core:Class"
            MATCH Class(?c) {}
            "#,
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn using_unresolved() {
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            USING "urn:eigenius:nonexistent:Foo"
            MATCH Foo(?x) {}
            "#,
        );
        assert!(errors.iter().any(|e| e.rule == "using_unresolved"));
    }

    #[test]
    fn aggregate_in_where_rejected() {
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x { short_name: ?name }
            WHERE COUNT(?x) > 5
            RETURN [] { name: ?name }
            "#,
        );
        assert!(errors.iter().any(|e| e.rule == "aggregate_in_where"));
    }

    #[test]
    fn not_exists_on_bound_variable() {
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x { short_name: ?name, domain: ?d }
            WHERE NOT EXISTS(?d)
            "#,
        );
        // ?d is bound in MATCH, NOT EXISTS is valid
        assert!(
            !errors.iter().any(|e| e.rule == "not_exists_unbound"),
            "NOT EXISTS on bound variable should not error"
        );
    }

    #[test]
    fn not_exists_on_unbound_variable() {
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x { short_name: ?name }
            WHERE NOT EXISTS(?missing)
            "#,
        );
        assert!(errors.iter().any(|e| e.rule == "not_exists_unbound"));
    }

    // ─── D2 v2 §5.7–5.9 — D14 institution-surface rules ────────────────

    /// Build a layer with the dock-assay demo ontology stacked on top
    /// of the bootstrap chain. Provides a realistic InstitutionIndex
    /// for the FIBER / qualified-call type-check tests.
    fn build_demo_layer() -> Arc<Layer> {
        let demo_ontology =
            include_str!("../../../ontologies/examples/d14-dock-assay/dock-assay.json");
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut builder = LayerBuilder::new("type-check-demo", Some(parent));
        for r in eigon_json::parse_document(demo_ontology).expect("parse demo") {
            builder.add_resource(r).expect("add demo resource");
        }
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn using_institution_unresolved_when_iri_not_indexed() {
        let layer = build_demo_layer();
        let errors = check(
            &layer,
            r#"
            USING INSTITUTION "urn:eigenius:nonexistent:institution" AS bogus
            MATCH ?x {}
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "using_institution_unresolved"),
            "expected using_institution_unresolved; got {errors:?}"
        );
    }

    #[test]
    fn fiber_query_class_must_resolve_as_query_class() {
        let layer = build_demo_layer();
        let errors = check(
            &layer,
            r#"
            USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay
            MATCH ?x {}
            FIBER assay:not_a_real_query_class { } AS ?v
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "fiber_query_class_not_query_class"),
            "expected fiber_query_class_not_query_class; got {errors:?}"
        );
    }

    #[test]
    fn fiber_query_class_must_have_on_demand_role() {
        let layer = build_demo_layer();
        // `within_tolerance` is Decidable-only — FIBER should reject it.
        let errors = check(
            &layer,
            r#"
            USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay
            MATCH ?x {}
            FIBER assay:within_tolerance {
                "urn:eigenius:demo:d14:predicted_ic50": 1.0,
                "urn:eigenius:demo:d14:target_ic50": 1.0,
                "urn:eigenius:demo:d14:tolerance": 0.5
            } AS ?v
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "fiber_query_class_not_on_demand"),
            "expected fiber_query_class_not_on_demand; got {errors:?}"
        );
    }

    #[test]
    fn fiber_institution_mismatch_when_alias_disagrees() {
        let layer = build_demo_layer();
        // Aliasing the dock institution but FIBERing the assay-owned
        // QueryClass triggers the institution-agreement rule.
        let errors = check(
            &layer,
            r#"
            USING INSTITUTION "urn:eigenius:demo:d14:dock" AS dock
            MATCH ?x {}
            FIBER dock:validate_prediction {
                candidate: "urn:eigenius:demo:d14:dock_to_assay"(?x)
            } AS ?v
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "fiber_institution_mismatch"),
            "expected fiber_institution_mismatch; got {errors:?}"
        );
    }

    #[test]
    fn comorphism_coercion_unresolved() {
        let layer = build_demo_layer();
        let errors = check(
            &layer,
            r#"
            USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay
            MATCH ?x {}
            FIBER assay:validate_prediction {
                candidate: "urn:eigenius:demo:d14:nonexistent_comorphism"(?x)
            } AS ?v
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            errors.iter().any(|e| e.rule == "comorphism_unresolved"),
            "expected comorphism_unresolved; got {errors:?}"
        );
    }

    #[test]
    fn qualified_call_must_be_decidable() {
        // Calling the OnDemand-only `validate_prediction` QueryClass in
        // expression position should fire the rule (it's not Decidable).
        let layer = build_demo_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x {}
            WHERE "urn:eigenius:demo:d14:validate_prediction"(?x)
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "qualified_call_not_decidable"),
            "expected qualified_call_not_decidable; got {errors:?}"
        );
    }

    #[test]
    fn fiber_decidable_only_call_unaffected_by_qualified_call_rule() {
        // Sanity: a qualified call that resolves to a Decidable QueryClass
        // type-checks cleanly (no qualified_call_not_decidable).
        let layer = build_demo_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x {}
            WHERE "urn:eigenius:demo:d14:within_tolerance"(1.0, 1.0, 0.5) HOLDS
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            !errors
                .iter()
                .any(|e| e.rule == "qualified_call_not_decidable"),
            "Decidable QueryClass call should not trigger the rule; got {errors:?}"
        );
    }

    #[test]
    fn bare_verdict_qualified_call_in_where_rejected() {
        // A Decidable QueryClass call directly in WHERE without a
        // postfix predicate fires bare_verdict_in_boolean_position.
        let layer = build_demo_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x {}
            WHERE "urn:eigenius:demo:d14:within_tolerance"(1.0, 1.0, 0.5)
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "bare_verdict_in_boolean_position"),
            "expected bare_verdict_in_boolean_position; got {errors:?}"
        );
    }

    #[test]
    fn bare_verdict_fiber_var_in_where_rejected() {
        // A FIBER-bound Verdict variable used directly in WHERE fires
        // bare_verdict_in_boolean_position. The user should project
        // it through HOLDS / FAILS / UNDECIDABLE.
        let layer = build_demo_layer();
        let errors = check(
            &layer,
            r#"
            USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay
            MATCH ?x {}
            FIBER assay:validate_prediction {
                candidate: "urn:eigenius:demo:d14:dock_to_assay"(?x)
            } AS ?v
            WHERE ?v
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "bare_verdict_in_boolean_position"),
            "expected bare_verdict_in_boolean_position; got {errors:?}"
        );
    }

    #[test]
    fn projected_verdict_in_where_accepted() {
        // The HOLDS-projected form of the same FIBER-bound Verdict
        // should type-check cleanly — neither rule fires.
        let layer = build_demo_layer();
        let errors = check(
            &layer,
            r#"
            USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay
            MATCH ?x {}
            FIBER assay:validate_prediction {
                candidate: "urn:eigenius:demo:d14:dock_to_assay"(?x)
            } AS ?v
            WHERE ?v HOLDS
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            !errors
                .iter()
                .any(|e| e.rule == "bare_verdict_in_boolean_position"),
            "projected Verdict should be accepted; got {errors:?}"
        );
        assert!(
            !errors
                .iter()
                .any(|e| e.rule == "verdict_predicate_non_verdict_operand"),
            "FIBER-bound Verdict is a Verdict source; should not trigger; got {errors:?}"
        );
    }

    #[test]
    fn verdict_predicate_on_non_verdict_operand_rejected() {
        // `?name HOLDS` where ?name is bound to a string property
        // fires verdict_predicate_non_verdict_operand.
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            USING "urn:eigenius:core:Class"
            MATCH Class(?c) { short_name: ?name }
            WHERE ?name HOLDS
            RETURN [] { x: ?name }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "verdict_predicate_non_verdict_operand"),
            "expected verdict_predicate_non_verdict_operand; got {errors:?}"
        );
    }

    #[test]
    fn verdict_predicate_on_literal_rejected() {
        // `42 HOLDS` is structurally non-sensical; the rule fires.
        let layer = build_core_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x {}
            WHERE 42 HOLDS
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "verdict_predicate_non_verdict_operand"),
            "expected verdict_predicate_non_verdict_operand; got {errors:?}"
        );
    }

    #[test]
    fn not_bare_verdict_rejected() {
        // `WHERE NOT qc:check(?x)` — Verdict in NOT operand position
        // fires bare_verdict_in_boolean_position. The user must
        // project first: `NOT qc:check(?x) HOLDS`.
        let layer = build_demo_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?x {}
            WHERE NOT "urn:eigenius:demo:d14:within_tolerance"(1.0, 1.0, 0.5)
            RETURN [] { x: ?x }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "bare_verdict_in_boolean_position"),
            "expected bare_verdict_in_boolean_position under NOT; got {errors:?}"
        );
    }

    // ─── D43 §4.6 — TEXT_MATCH / TEXT_SCORE typing tests ────────────────

    /// Build a layer with three test-only Properties and a
    /// `core:TextIndex` Resource targeting one of them, stacked on the
    /// bootstrap chain so `core:string` / `core:Property` /
    /// `core:TextIndex` resolve. Used by the §4.6 typing tests below.
    ///
    /// Short names are deliberately namespaced (`test_body`,
    /// `test_count`, `test_title`) to avoid collision with the core
    /// ontology's own `description` Property — the merged-view
    /// short-name resolver returns the IRI-sort-order-first match,
    /// so a colliding short name would silently route to the wrong
    /// Property.
    fn build_text_index_layer() -> Arc<Layer> {
        use crate::ontology::iri::Iri;
        use crate::ontology::resource::{Resource, Value};
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("text-index-fixture", Some(parent));

        // Property: urn:eigenius:test:body, data_type: string,
        // short_name "test_body" — TextIndex targets this one.
        let mut body_prop = Resource::new(Iri::parse("urn:eigenius:test:body").unwrap());
        body_prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(Iri::parse(wk::PROPERTY).unwrap())]),
        );
        body_prop.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("test_body".into()),
        );
        body_prop.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::ResourceRef(Iri::parse(wk::STRING).unwrap()),
        );
        b.add_resource(body_prop).unwrap();

        // Integer-typed Property for the negative test
        // (TEXT_MATCH on non-string property).
        let mut count_prop = Resource::new(Iri::parse("urn:eigenius:test:count").unwrap());
        count_prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(Iri::parse(wk::PROPERTY).unwrap())]),
        );
        count_prop.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("test_count".into()),
        );
        count_prop.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::ResourceRef(Iri::parse(wk::INTEGER).unwrap()),
        );
        b.add_resource(count_prop).unwrap();

        // Property without a TextIndex — for the no-active-index test.
        let mut bare_prop = Resource::new(Iri::parse("urn:eigenius:test:title").unwrap());
        bare_prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(Iri::parse(wk::PROPERTY).unwrap())]),
        );
        bare_prop.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("test_title".into()),
        );
        bare_prop.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::ResourceRef(Iri::parse(wk::STRING).unwrap()),
        );
        b.add_resource(bare_prop).unwrap();

        // The TextIndex targeting `test:body`.
        let mut ti = Resource::new(Iri::parse("urn:eigenius:test:ti_body").unwrap());
        ti.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse(wk::TEXT_INDEX_CLASS).unwrap(),
            )]),
        );
        ti.set(
            Iri::parse(wk::TARGET_PROPERTY).unwrap(),
            Value::ResourceRef(Iri::parse("urn:eigenius:test:body").unwrap()),
        );
        ti.set(
            Iri::parse(wk::TEXT_ANALYZER).unwrap(),
            Value::String("en-stem-v1".into()),
        );
        b.add_resource(ti).unwrap();

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn text_match_well_formed_passes() {
        let layer = build_text_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { test_body: ?desc }
            WHERE TEXT_MATCH(?desc, "wal truncation")
            RETURN [] { d: ?d }
            "#,
        );
        let related: Vec<_> = errors
            .iter()
            .filter(|e| e.rule.starts_with("text_retrieval"))
            .collect();
        assert!(
            related.is_empty(),
            "expected no text_retrieval_* errors, got {related:?}"
        );
    }

    #[test]
    fn text_score_well_formed_passes() {
        let layer = build_text_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { test_body: ?desc }
            RETURN [] { d: ?d, s: TEXT_SCORE(?desc, "wal") }
            "#,
        );
        let related: Vec<_> = errors
            .iter()
            .filter(|e| e.rule.starts_with("text_retrieval"))
            .collect();
        assert!(
            related.is_empty(),
            "expected no text_retrieval_* errors, got {related:?}"
        );
    }

    #[test]
    fn text_match_with_full_iri_property_passes() {
        let layer = build_text_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?desc }
            WHERE TEXT_MATCH(?desc, "wal")
            RETURN [] { d: ?d }
            "#,
        );
        let related: Vec<_> = errors
            .iter()
            .filter(|e| e.rule.starts_with("text_retrieval"))
            .collect();
        assert!(
            related.is_empty(),
            "FullIri MATCH binding should typecheck; got {related:?}"
        );
    }

    #[test]
    fn text_match_wrong_arity_rejected() {
        let layer = build_text_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { test_body: ?desc }
            WHERE TEXT_MATCH(?desc)
            RETURN [] { d: ?d }
            "#,
        );
        assert!(
            errors.iter().any(|e| e.rule == "text_retrieval_arity"),
            "expected text_retrieval_arity; got {errors:?}"
        );
    }

    #[test]
    fn text_match_on_unbound_variable_rejected() {
        let layer = build_text_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d {}
            WHERE TEXT_MATCH(?d, "wal")
            RETURN [] { d: ?d }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "text_retrieval_arg0_not_property_bound"),
            "expected text_retrieval_arg0_not_property_bound; got {errors:?}"
        );
    }

    #[test]
    fn text_match_on_non_variable_rejected() {
        let layer = build_text_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { test_body: ?desc }
            WHERE TEXT_MATCH("not a var", "wal")
            RETURN [] { d: ?d }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "text_retrieval_arg0_not_variable"),
            "expected text_retrieval_arg0_not_variable; got {errors:?}"
        );
    }

    #[test]
    fn text_match_non_string_query_rejected() {
        let layer = build_text_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { test_body: ?desc }
            WHERE TEXT_MATCH(?desc, 42)
            RETURN [] { d: ?d }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "text_retrieval_arg1_not_string_literal"),
            "expected text_retrieval_arg1_not_string_literal; got {errors:?}"
        );
    }

    #[test]
    fn text_match_on_non_string_property_rejected() {
        let layer = build_text_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { test_count: ?n }
            WHERE TEXT_MATCH(?n, "wal")
            RETURN [] { d: ?d }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "text_retrieval_property_not_string"),
            "expected text_retrieval_property_not_string; got {errors:?}"
        );
    }

    #[test]
    fn text_match_without_active_index_rejected() {
        let layer = build_text_index_layer();
        // `test_title` is a string Property but has no TextIndex
        // targeting it.
        let errors = check(
            &layer,
            r#"
            MATCH ?d { test_title: ?t }
            WHERE TEXT_MATCH(?t, "wal")
            RETURN [] { d: ?d }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "text_retrieval_no_active_index"),
            "expected text_retrieval_no_active_index; got {errors:?}"
        );
    }

    // ─── D43 §4.5 — VECTOR_NEAR `ef` typing (M6.5) ──────────────

    fn build_vector_index_layer() -> Arc<Layer> {
        use crate::ontology::iri::Iri;
        use crate::ontology::resource::{Resource, Value};
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("vector-index-fixture", Some(parent));

        let body_iri = "urn:eigenius:test:body";
        let model = "urn:eigenius:embed:dummy:v1";

        let mut body_prop = Resource::new(Iri::parse(body_iri).unwrap());
        body_prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(Iri::parse(wk::PROPERTY).unwrap())]),
        );
        body_prop.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::ResourceRef(Iri::parse(wk::STRING).unwrap()),
        );
        b.add_resource(body_prop).unwrap();

        let mut vi = Resource::new(Iri::parse("urn:eigenius:test:vi").unwrap());
        vi.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse(wk::VECTOR_INDEX_CLASS).unwrap(),
            )]),
        );
        vi.set(
            Iri::parse(wk::TARGET_PROPERTY).unwrap(),
            Value::ResourceRef(Iri::parse(body_iri).unwrap()),
        );
        vi.set(
            Iri::parse(wk::VEC_MODEL).unwrap(),
            Value::ResourceRef(Iri::parse(model).unwrap()),
        );
        vi.set(Iri::parse(wk::VEC_DIM).unwrap(), Value::Integer(8));
        b.add_resource(vi).unwrap();

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn vector_near_with_ef_passes() {
        let layer = build_vector_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            WHERE VECTOR_NEAR(?vec, EMBED("hello"), 5, 32)
            RETURN [] { d: ?d }
            "#,
        );
        let related: Vec<_> = errors
            .iter()
            .filter(|e| e.rule.starts_with("vector_retrieval"))
            .collect();
        assert!(
            related.is_empty(),
            "VECTOR_NEAR with ef should typecheck; got {related:?}"
        );
    }

    #[test]
    fn vector_near_ef_must_be_positive() {
        let layer = build_vector_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            WHERE VECTOR_NEAR(?vec, EMBED("hello"), 5, 0)
            RETURN [] { d: ?d }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "vector_retrieval_ef_not_positive"),
            "expected vector_retrieval_ef_not_positive; got {errors:?}"
        );
    }

    #[test]
    fn vector_near_ef_must_be_integer_literal() {
        let layer = build_vector_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            WHERE VECTOR_NEAR(?vec, EMBED("hello"), 5, "not a number")
            RETURN [] { d: ?d }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "vector_retrieval_ef_not_literal"),
            "expected vector_retrieval_ef_not_literal; got {errors:?}"
        );
    }

    #[test]
    fn vector_near_with_5_args_rejected() {
        // 5 args is out of range. v1 supports only 3 or 4.
        let layer = build_vector_index_layer();
        let errors = check(
            &layer,
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            WHERE VECTOR_NEAR(?vec, EMBED("hello"), 5, 32, 100)
            RETURN [] { d: ?d }
            "#,
        );
        assert!(
            errors.iter().any(|e| e.rule == "vector_retrieval_arity"),
            "expected vector_retrieval_arity for 5-arg VECTOR_NEAR; got {errors:?}"
        );
    }
}
