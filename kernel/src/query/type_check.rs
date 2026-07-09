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

    // Build the institution index once for the whole pass — every
    // FIBER / qualified-call check resolves through it. Index-driven
    // (`from_layer_indexed`, not the full-chain `from_layer`): this runs on
    // EVERY query, so on a large knowledge-graph chain the full scan was a
    // ~O(chain) per-query floor (≈3.5s on the UMLS chain). The query head is
    // stored, so the triple index covers it; identical result to the full scan
    // on a core-rooted chain (`indexed_rebuild_matches_full_scan`).
    let (index, _index_errors) = InstitutionIndex::from_layer_indexed(layer);

    // DEFINE relation names are valid pattern "classes" (they reference a derived
    // relation, not a chain class), so short-name class resolution must exempt them.
    let relation_names: BTreeSet<String> =
        program.definitions.iter().map(|d| d.name.clone()).collect();

    // Check DEFINE rules
    for def in &program.definitions {
        check_match_part(&def.body, layer, &relation_names, &mut errors);
    }

    // Check the query
    check_match_part(&program.query.body, layer, &relation_names, &mut errors);

    // FIBER-clause specifics: USING INSTITUTION alias + IRI resolution,
    // FIBER QueryClass / institution-agreement / OnDemand-role checks,
    // param scope + required coverage, comorphism coercion rules.
    check_fiber_clauses(&program.query.body, layer, &index, &mut errors);

    // Qualified-name function calls in expression position must
    // resolve to a Decidable QueryClass (D2 v2 §5.9).
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

    // Check aggregate/GROUP BY consistency
    check_aggregate_consistency(program, &mut errors);

    // D43 §4.3 / §4.4 — similarity operator typing rules. Walks
    // every expression position that can contain a `~` (WHERE,
    // RETURN, GROUP BY, ORDER BY, and rule bodies' WHERE) and
    // checks the LHS is property-bound, the property has an active
    // similarity index of the kind required by `via:` (or any kind
    // by default), and the hint set is internally consistent.
    let prop_var_index = match build_property_variable_index(program, layer) {
        Ok(m) => m,
        Err(e) => {
            // Ambiguous short name in a property position — surface as a type
            // error; continue with an empty typing view so other checks still run.
            errors.push(e);
            BTreeMap::new()
        }
    };
    let text_indexes = resolve_active_text_indexes(layer);
    let vector_indexes = resolve_active_vector_indexes(layer);
    let check_in_expr = |expr: &Expression, errs: &mut Vec<QueryError>| {
        check_similarity(
            expr,
            &prop_var_index,
            &text_indexes,
            &vector_indexes,
            layer,
            errs,
        );
    };
    for cond in &program.query.body.conditions {
        check_in_expr(cond, &mut errors);
    }
    for item in &program.query.result {
        check_in_expr(&item.expression, &mut errors);
    }
    for expr in &program.query.group_by {
        check_in_expr(expr, &mut errors);
    }
    for item in &program.query.order_by {
        check_in_expr(&item.expression, &mut errors);
    }
    for def in &program.definitions {
        for cond in &def.body.conditions {
            check_in_expr(cond, &mut errors);
        }
    }

    // D43 §3.3 — TOP K structural rules.
    //
    //   1. TOP is mutually exclusive with LIMIT (different surfaces:
    //      LIMIT is un-ranked truncation, TOP is similarity-ranked).
    //   2. TOP is mutually exclusive with ORDER BY (ranking comes
    //      from the similarity score, not a user expression).
    //   3. TOP requires at least one similarity operator in WHERE —
    //      without `~`, ranking has no source.
    //   4. TOP N requires N > 0.
    if let Some(n) = program.query.top {
        if n == 0 {
            errors.push(QueryError::type_check(
                "top_must_be_positive",
                "TOP N requires N to be a positive integer".to_string(),
            ));
        }
        if program.query.limit.is_some() {
            errors.push(QueryError::type_check(
                "top_with_limit",
                "TOP and LIMIT are mutually exclusive — use TOP for similarity-ranked truncation and LIMIT for un-ranked truncation".to_string(),
            ));
        }
        if !program.query.order_by.is_empty() {
            errors.push(QueryError::type_check(
                "top_with_order_by",
                "TOP and ORDER BY are mutually exclusive — TOP draws its ordering from the similarity score".to_string(),
            ));
        }
        if !any_similarity_in_where(&program.query.body) {
            errors.push(QueryError::type_check(
                "top_without_similarity",
                "TOP N requires at least one `~` similarity operator in WHERE (use LIMIT for un-ranked truncation)".to_string(),
            ));
        }
    }

    errors
}

/// Walk a MatchPart's WHERE conditions looking for a `~` operator
/// anywhere — including under boolean combinators and other nested
/// expressions. Used by the TOP K structural check to ensure ranking
/// has a source.
fn any_similarity_in_where(part: &MatchPart) -> bool {
    part.conditions.iter().any(expr_has_similarity)
}

fn expr_has_similarity(expr: &Expression) -> bool {
    match expr {
        Expression::Similarity { .. } => true,
        Expression::Binary { left, right, .. } => {
            expr_has_similarity(left) || expr_has_similarity(right)
        }
        Expression::Unary { operand, .. } | Expression::VerdictPredicate { operand, .. } => {
            expr_has_similarity(operand)
        }
        Expression::FunctionCall { args, .. } => args.iter().any(expr_has_similarity),
        Expression::Aggregate { arg, .. } => expr_has_similarity(arg),
        Expression::Array(es) => es.iter().any(expr_has_similarity),
        Expression::Object(pairs) => pairs.iter().any(|(_, v)| expr_has_similarity(v)),
        Expression::Literal(_)
        | Expression::Variable(_)
        | Expression::NotExists(_)
        | Expression::DotPath { .. } => false,
    }
}

/// Check a MatchPart: validate USING IRIs resolve to classes, and that every
/// short-name pattern class resolves to a class in scope (the implicit core
/// namespace + any `USING NAMESPACE`) or names a DEFINE relation. Fails closed:
/// an unresolvable short-name class is an error, not a silent degradation to an
/// untyped match-all.
fn check_match_part(
    part: &MatchPart,
    layer: &Layer,
    relation_names: &BTreeSet<String>,
    errors: &mut Vec<QueryError>,
) {
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

    // Check short-name pattern classes resolve in scope.
    for pattern in part.patterns() {
        if let Some(Name::ShortName(short)) = &pattern.class {
            if relation_names.contains(short) {
                continue; // a DEFINE relation reference, not a chain class
            }
            match crate::query::resolve::resolve_scoped_name(
                layer,
                &part.using_namespaces,
                &[wk::CLASS],
                short,
            ) {
                Ok(Some(_)) => {}
                Ok(None) => errors.push(QueryError::type_check(
                    "unknown_class",
                    format!(
                        "pattern class '{short}' does not resolve to a Class in the core namespace \
                         or any USING NAMESPACE; add `USING NAMESPACE \"<prefix>\"` or use a full IRI"
                    ),
                )),
                Err(e) => errors.push(e),
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
            match &prop.object {
                ValueOrVariable::Variable(v) => {
                    vars.insert(v.name.clone());
                }
                ValueOrVariable::Array(ap) => {
                    for v in ap.variables() {
                        vars.insert(v.name.clone());
                    }
                }
                ValueOrVariable::Literal(_) => {}
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
        Expression::Similarity {
            property, query, ..
        } => {
            if !bound.contains(&property.name) {
                errors.push(QueryError::type_check(
                    "unbound_variable",
                    format!(
                        "similarity LHS '?{}' is not bound in any MATCH pattern",
                        property.name
                    ),
                ));
            }
            check_expression_variables(query, bound, errors);
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
            Name::ShortName(short) => {
                match resolve_short_name_to_query_class(layer, &part.using_namespaces, short) {
                    Ok(v) => v,
                    Err(e) => {
                        errors.push(e);
                        None
                    }
                }
            }
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
/// resource. These are the only Verdict-typed `?var`
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
                    // Ambiguity here is reported by `check_fiber_clauses`; this
                    // best-effort verdict scan just skips an unresolved name.
                    Name::ShortName(short) => {
                        resolve_short_name_to_query_class(layer, &part.using_namespaces, short)
                            .ok()
                            .flatten()
                    }
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
fn resolve_short_name_to_query_class(
    layer: &Layer,
    namespaces: &[String],
    short: &str,
) -> Result<Option<Iri>, QueryError> {
    crate::query::resolve::resolve_scoped_name(layer, namespaces, &[wk::QUERY_CLASS_CLASS], short)
}

fn query_class_name_display(name: &Name) -> String {
    match name {
        Name::ShortName(s) => s.clone(),
        Name::FullIri(i) => i.as_str().to_string(),
    }
}

// ─── D43 §4 — similarity-operator typing ─────────────────────────────

/// Schema view a similarity call needs: which property a variable
/// was bound to. Reused from the deleted D43 §4.6 retrieval-primitive
/// pass — same shape, smaller surface.
struct PropertyBinding {
    property_iri: Iri,
}

/// Walk every MATCH pattern in the program and build the
/// `variable → property_iri` map. Property variables bound by
/// rule-derived patterns are included too — the rule's body is
/// typed against the same schema view.
fn build_property_variable_index(
    program: &Program,
    layer: &Layer,
) -> Result<BTreeMap<String, PropertyBinding>, QueryError> {
    let mut out: BTreeMap<String, PropertyBinding> = BTreeMap::new();
    let mut visit = |part: &MatchPart| -> Result<(), QueryError> {
        for pat in part.patterns() {
            for pp in &pat.properties {
                if let ValueOrVariable::Variable(var) = &pp.object {
                    if let Some(property_iri) =
                        resolve_property_name(&pp.property, layer, &part.using_namespaces)?
                    {
                        out.entry(var.name.clone())
                            .or_insert(PropertyBinding { property_iri });
                    }
                }
            }
        }
        Ok(())
    };
    visit(&program.query.body)?;
    for def in &program.definitions {
        visit(&def.body)?;
    }
    Ok(out)
}

/// Resolve a property `Name` to its IRI. `FullIri` returns the IRI
/// directly; `ShortName` scans the chain-merged view for a Property
/// Resource whose `short_name` matches.
fn resolve_property_name(
    name: &Name,
    layer: &Layer,
    namespaces: &[String],
) -> Result<Option<Iri>, QueryError> {
    match name {
        Name::FullIri(iri) => Ok(Some(iri.clone())),
        Name::ShortName(s) => {
            crate::query::resolve::resolve_scoped_name(layer, namespaces, &[wk::PROPERTY], s)
        }
    }
}

/// Does the Property Resource at `property_iri` declare
/// `data_type: core:string`? Defensive: properties without a
/// `data_type` slot are treated as non-string-typed.
fn property_is_string_typed(property_iri: &Iri, layer: &Layer) -> bool {
    use crate::ontology::resource::Value;
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
        Some(Value::ResourceRef(iri)) => *iri == string_iri,
        Some(v) => v
            .as_iri_str()
            .and_then(|s| Iri::parse(s).ok())
            .map(|iri| iri == string_iri)
            .unwrap_or(false),
        None => false,
    }
}

/// Recursively walk `expr`, checking every `Similarity` node against
/// the schema view per D43 §4.3 / §4.4.
fn check_similarity(
    expr: &Expression,
    prop_var_index: &BTreeMap<String, PropertyBinding>,
    text_indexes: &[ActiveTextIndex],
    vector_indexes: &[ActiveVectorIndex],
    layer: &Layer,
    errors: &mut Vec<QueryError>,
) {
    match expr {
        Expression::Similarity {
            property,
            query,
            hints,
        } => {
            check_similarity_node(
                property,
                query,
                hints,
                prop_var_index,
                text_indexes,
                vector_indexes,
                layer,
                errors,
            );
            check_similarity(
                query,
                prop_var_index,
                text_indexes,
                vector_indexes,
                layer,
                errors,
            );
        }
        Expression::Binary { left, right, .. } => {
            check_similarity(
                left,
                prop_var_index,
                text_indexes,
                vector_indexes,
                layer,
                errors,
            );
            check_similarity(
                right,
                prop_var_index,
                text_indexes,
                vector_indexes,
                layer,
                errors,
            );
        }
        Expression::Unary { operand, .. } | Expression::VerdictPredicate { operand, .. } => {
            check_similarity(
                operand,
                prop_var_index,
                text_indexes,
                vector_indexes,
                layer,
                errors,
            );
        }
        Expression::FunctionCall { args, .. } => {
            for a in args {
                check_similarity(
                    a,
                    prop_var_index,
                    text_indexes,
                    vector_indexes,
                    layer,
                    errors,
                );
            }
        }
        Expression::Aggregate { arg, .. } => {
            check_similarity(
                arg,
                prop_var_index,
                text_indexes,
                vector_indexes,
                layer,
                errors,
            );
        }
        Expression::Array(es) => {
            for e in es {
                check_similarity(
                    e,
                    prop_var_index,
                    text_indexes,
                    vector_indexes,
                    layer,
                    errors,
                );
            }
        }
        Expression::Object(pairs) => {
            for (_, v) in pairs {
                check_similarity(
                    v,
                    prop_var_index,
                    text_indexes,
                    vector_indexes,
                    layer,
                    errors,
                );
            }
        }
        Expression::Literal(_)
        | Expression::Variable(_)
        | Expression::NotExists(_)
        | Expression::DotPath { .. } => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn check_similarity_node(
    property: &Variable,
    query: &Expression,
    hints: &HintSet,
    prop_var_index: &BTreeMap<String, PropertyBinding>,
    text_indexes: &[ActiveTextIndex],
    vector_indexes: &[ActiveVectorIndex],
    layer: &Layer,
    errors: &mut Vec<QueryError>,
) {
    // §4.3.1 — LHS must be bound by a property pattern.
    let binding = match prop_var_index.get(&property.name) {
        Some(b) => b,
        None => {
            errors.push(QueryError::type_check(
                "similarity_lhs_not_property_bound",
                format!(
                    "left-hand side of `~` must be a property-bound variable (got '?{}')",
                    property.name
                ),
            ));
            return;
        }
    };
    let property_iri = &binding.property_iri;
    let property_display = property_iri.as_str();

    // §4.3.3 — property must be string-typed.
    if !property_is_string_typed(property_iri, layer) {
        errors.push(QueryError::type_check(
            "similarity_property_not_string",
            format!(
                "property '{property_display}' is not String-typed; similarity requires String-shaped"
            ),
        ));
    }

    let text_active: Option<&ActiveTextIndex> = text_indexes
        .iter()
        .find(|i| i.target_property == *property_iri);
    let vector_active: Option<&ActiveVectorIndex> = vector_indexes
        .iter()
        .find(|i| i.target_property == *property_iri);

    // §4.3.2 — at least one active similarity index.
    if text_active.is_none() && vector_active.is_none() {
        errors.push(QueryError::type_check(
            "similarity_no_active_index",
            format!("property '{property_display}' has no active similarity index at this head"),
        ));
    }

    // §4.3.4 — RHS must be a string literal in v1.
    if !matches!(query, Expression::Literal(Literal::String(_))) {
        errors.push(QueryError::type_check(
            "similarity_rhs_not_string_literal",
            "right-hand side of `~` must be a string literal".to_string(),
        ));
    }

    // §4.4 — hint validation.
    if let Some(via) = hints.via {
        match via {
            Via::Text => {
                if text_active.is_none() {
                    errors.push(QueryError::type_check(
                        "similarity_hint_via_text_no_text_index",
                        format!(
                            "via: text requires an active TextIndex on property '{property_display}' (none declared at head)"
                        ),
                    ));
                }
                if hints.model.is_some() {
                    errors.push(QueryError::type_check(
                        "similarity_hint_model_with_via_text",
                        "`model:` is incompatible with `via: text` — the model only applies to the vector path".to_string(),
                    ));
                }
            }
            Via::Vector => {
                if vector_active.is_none() {
                    errors.push(QueryError::type_check(
                        "similarity_hint_via_vector_no_vector_index",
                        format!(
                            "via: vector requires an active VectorIndex on property '{property_display}' (none declared at head)"
                        ),
                    ));
                }
            }
            Via::Hybrid => {
                if text_active.is_none() || vector_active.is_none() {
                    let have = match (text_active.is_some(), vector_active.is_some()) {
                        (true, false) => "only TextIndex",
                        (false, true) => "only VectorIndex",
                        _ => "neither",
                    };
                    errors.push(QueryError::type_check(
                        "similarity_hint_via_hybrid_missing_index",
                        format!(
                            "via: hybrid requires both a TextIndex and a VectorIndex on property '{property_display}' ({have} declared)"
                        ),
                    ));
                }
            }
        }
    }
    if let Some(model) = &hints.model {
        // model: implicitly forces vector path; in v1 (at most one
        // VectorIndex per property) we just check it matches.
        match vector_active {
            None => {
                errors.push(QueryError::type_check(
                    "similarity_hint_model_no_vector_index",
                    format!(
                        "`model:` requires an active VectorIndex on property '{property_display}' (none declared at head)"
                    ),
                ));
            }
            Some(vi) => {
                if vi.model.as_str() != model {
                    errors.push(QueryError::type_check(
                        "similarity_hint_model_mismatch",
                        format!(
                            "model '{model}' does not match the active VectorIndex on property '{property_display}' (which declares '{}')",
                            vi.model.as_str()
                        ),
                    ));
                }
            }
        }
    }
    if let Some(0) = hints.k {
        errors.push(QueryError::type_check(
            "similarity_hint_k_not_positive",
            "`k:` must be a positive integer".to_string(),
        ));
    }
    if let Some(0) = hints.limit {
        errors.push(QueryError::type_check(
            "similarity_hint_limit_not_positive",
            "`limit:` must be a positive integer".to_string(),
        ));
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

    // ─── D2 v2 §5.7–5.9 — institution-surface rules ────────────────

    /// Build a layer with the dock-assay demo ontology stacked on top
    /// of the bootstrap chain. Provides a realistic InstitutionIndex
    /// for the FIBER / qualified-call type-check tests.
    fn build_demo_layer() -> Arc<Layer> {
        let demo_ontology = include_str!("../../../ontologies/examples/dock-assay/dock-assay.json");
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
            USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
            USING NAMESPACE "urn:eigenius:demo:institutions:"
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
            USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
            USING NAMESPACE "urn:eigenius:demo:institutions:"
            MATCH ?x {}
            FIBER assay:within_tolerance {
                "urn:eigenius:demo:institutions:predicted_ic50": 1.0,
                "urn:eigenius:demo:institutions:target_ic50": 1.0,
                "urn:eigenius:demo:institutions:tolerance": 0.5
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
            USING INSTITUTION "urn:eigenius:demo:institutions:dock" AS dock
            USING NAMESPACE "urn:eigenius:demo:institutions:"
            MATCH ?x {}
            FIBER dock:validate_prediction {
                candidate: "urn:eigenius:demo:institutions:dock_to_assay"(?x)
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
            USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
            USING NAMESPACE "urn:eigenius:demo:institutions:"
            MATCH ?x {}
            FIBER assay:validate_prediction {
                candidate: "urn:eigenius:demo:institutions:nonexistent_comorphism"(?x)
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
            WHERE "urn:eigenius:demo:institutions:validate_prediction"(?x)
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
            WHERE "urn:eigenius:demo:institutions:within_tolerance"(1.0, 1.0, 0.5) HOLDS
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
            WHERE "urn:eigenius:demo:institutions:within_tolerance"(1.0, 1.0, 0.5)
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
            USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
            USING NAMESPACE "urn:eigenius:demo:institutions:"
            MATCH ?x {}
            FIBER assay:validate_prediction {
                candidate: "urn:eigenius:demo:institutions:dock_to_assay"(?x)
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
            USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
            USING NAMESPACE "urn:eigenius:demo:institutions:"
            MATCH ?x {}
            FIBER assay:validate_prediction {
                candidate: "urn:eigenius:demo:institutions:dock_to_assay"(?x)
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
            WHERE NOT "urn:eigenius:demo:institutions:within_tolerance"(1.0, 1.0, 0.5)
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

    // ─── D43 §4 — similarity operator typing tests ──────────────────────

    use crate::ontology::resource::{Resource, Value};

    fn iri(s: &str) -> Iri {
        Iri::parse(s).expect("valid iri")
    }

    fn make_resource(id: &str, class_iri: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(iri(class_iri))]),
        );
        for (k, v) in props {
            r.set(iri(k), v);
        }
        r
    }

    /// Build a layer carrying a test Property with `data_type: string`
    /// and (optionally) a TextIndex and/or VectorIndex targeting it.
    /// Bootstrap loads the core ontology so `is_a` resolves correctly.
    fn build_indexed_layer(text: bool, vector: bool) -> Arc<Layer> {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap should succeed");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let mut b = LayerBuilder::new("test-indexes", Some(head));

        // A test Property named "test_body" with data_type=string.
        b.add_resource(make_resource(
            "urn:ex:test_body",
            "urn:eigenius:core:Property",
            vec![
                (
                    "urn:eigenius:core:short_name",
                    Value::String("test_body".into()),
                ),
                (
                    "urn:eigenius:core:data_type",
                    Value::ResourceRef(iri("urn:eigenius:core:string")),
                ),
            ],
        ))
        .unwrap();
        // Also an int-typed property to test the string-typed check.
        b.add_resource(make_resource(
            "urn:ex:test_count",
            "urn:eigenius:core:Property",
            vec![
                (
                    "urn:eigenius:core:short_name",
                    Value::String("test_count".into()),
                ),
                (
                    "urn:eigenius:core:data_type",
                    Value::ResourceRef(iri("urn:eigenius:core:integer")),
                ),
            ],
        ))
        .unwrap();

        if text {
            b.add_resource(make_resource(
                "urn:ex:ti_body",
                "urn:eigenius:core:TextIndex",
                vec![
                    (
                        "urn:eigenius:core:target_property",
                        Value::ResourceRef(iri("urn:ex:test_body")),
                    ),
                    (
                        "urn:eigenius:core:text_analyzer",
                        Value::String("en-stem-v1".into()),
                    ),
                ],
            ))
            .unwrap();
        }
        if vector {
            b.add_resource(make_resource(
                "urn:ex:vi_body",
                "urn:eigenius:core:VectorIndex",
                vec![
                    (
                        "urn:eigenius:core:target_property",
                        Value::ResourceRef(iri("urn:ex:test_body")),
                    ),
                    (
                        "urn:eigenius:core:vec_model",
                        Value::ResourceRef(iri("urn:eigenius:embed:m1")),
                    ),
                    ("urn:eigenius:core:vec_dim", Value::Integer(8)),
                    (
                        "urn:eigenius:core:vec_distance",
                        Value::ResourceRef(iri("urn:eigenius:core:distances:cosine")),
                    ),
                ],
            ))
            .unwrap();
        }
        Arc::new(b.build(storage))
    }

    #[test]
    fn similarity_well_formed_text_passes() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "WAL truncation"
            "#,
        );
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn similarity_well_formed_vector_passes() {
        let layer = build_indexed_layer(false, true);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "kernel chain consolidation"
            "#,
        );
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn similarity_on_unbound_variable_rejected() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?other ~ "anything"
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "similarity_lhs_not_property_bound"
                    || e.rule == "unbound_variable"),
            "expected lhs-not-property-bound, got: {errors:?}"
        );
    }

    #[test]
    fn similarity_on_non_string_property_rejected() {
        let layer = build_indexed_layer(true, false);
        // Need an active similarity index on test_count too to isolate
        // the string-typed check from the no-index check.
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap should succeed");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let mut b = LayerBuilder::new("count-text", Some(head));
        b.add_resource(make_resource(
            "urn:ex:test_count",
            "urn:eigenius:core:Property",
            vec![
                (
                    "urn:eigenius:core:short_name",
                    Value::String("test_count".into()),
                ),
                (
                    "urn:eigenius:core:data_type",
                    Value::ResourceRef(iri("urn:eigenius:core:integer")),
                ),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:ti_count",
            "urn:eigenius:core:TextIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::ResourceRef(iri("urn:ex:test_count")),
                ),
                (
                    "urn:eigenius:core:text_analyzer",
                    Value::String("en-stem-v1".into()),
                ),
            ],
        ))
        .unwrap();
        let layer_count = Arc::new(b.build(storage));
        let errors = check(
            &layer_count,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_count: ?c }
            WHERE ?c ~ "anything"
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "similarity_property_not_string"),
            "expected similarity_property_not_string, got: {errors:?}"
        );
        let _ = layer; // unused but keeps the basic fixture in scope
    }

    #[test]
    fn similarity_without_active_index_rejected() {
        let layer = build_indexed_layer(false, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "query"
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "similarity_no_active_index"),
            "expected similarity_no_active_index, got: {errors:?}"
        );
    }

    #[test]
    fn similarity_rhs_must_be_string_literal() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ 42
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "similarity_rhs_not_string_literal"),
            "expected similarity_rhs_not_string_literal, got: {errors:?}"
        );
    }

    #[test]
    fn similarity_via_text_without_text_index_rejected() {
        let layer = build_indexed_layer(false, true);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "q" { via: text }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "similarity_hint_via_text_no_text_index"),
            "expected via-text-no-text-index, got: {errors:?}"
        );
    }

    #[test]
    fn similarity_via_vector_without_vector_index_rejected() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "q" { via: vector }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "similarity_hint_via_vector_no_vector_index"),
            "expected via-vector-no-vector-index, got: {errors:?}"
        );
    }

    #[test]
    fn similarity_via_hybrid_with_one_index_rejected() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "q" { via: hybrid }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "similarity_hint_via_hybrid_missing_index"),
            "expected hybrid-missing-index, got: {errors:?}"
        );
    }

    #[test]
    fn similarity_model_with_via_text_rejected() {
        let layer = build_indexed_layer(true, true);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "q" { via: text, model: "urn:eigenius:embed:m1" }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "similarity_hint_model_with_via_text"),
            "expected model-with-via-text, got: {errors:?}"
        );
    }

    #[test]
    fn similarity_model_mismatch_rejected() {
        let layer = build_indexed_layer(false, true);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "q" { model: "urn:eigenius:embed:other" }
            "#,
        );
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "similarity_hint_model_mismatch"),
            "expected model-mismatch, got: {errors:?}"
        );
    }

    // ─── D43 §3.3 — TOP K structural typing tests ──────────────────────

    #[test]
    fn top_with_similarity_passes() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "x"
            TOP 20
            "#,
        );
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn top_zero_rejected() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "x"
            TOP 0
            "#,
        );
        assert!(
            errors.iter().any(|e| e.rule == "top_must_be_positive"),
            "expected top_must_be_positive, got: {errors:?}"
        );
    }

    #[test]
    fn top_with_limit_rejected() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "x"
            LIMIT 10
            TOP 5
            "#,
        );
        assert!(
            errors.iter().any(|e| e.rule == "top_with_limit"),
            "expected top_with_limit, got: {errors:?}"
        );
    }

    #[test]
    fn top_with_order_by_rejected() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            WHERE ?desc ~ "x"
            ORDER BY ?desc
            TOP 5
            "#,
        );
        assert!(
            errors.iter().any(|e| e.rule == "top_with_order_by"),
            "expected top_with_order_by, got: {errors:?}"
        );
    }

    #[test]
    fn top_without_similarity_rejected() {
        let layer = build_indexed_layer(true, false);
        let errors = check(
            &layer,
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { test_body: ?desc }
            TOP 5
            "#,
        );
        assert!(
            errors.iter().any(|e| e.rule == "top_without_similarity"),
            "expected top_without_similarity, got: {errors:?}"
        );
    }
}
