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

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::error::QueryError;
use std::collections::BTreeSet;

/// Type-check a parsed EigenQL program against a layer.
///
/// Returns a list of errors (empty if valid).
pub fn type_check(program: &Program, layer: &Layer) -> Vec<QueryError> {
    let mut errors = Vec::new();

    // Check DEFINE rules
    for def in &program.definitions {
        check_match_part(&def.body, layer, &mut errors);
    }

    // Check the query
    check_match_part(&program.query.body, layer, &mut errors);

    // FIBER-clause specifics: USING INSTITUTION alias uniqueness, FIBER
    // referent resolution, required-param coverage, short-name scope.
    check_fiber_clauses(&program.query.body, layer, &mut errors);

    // Collect all bound variables from MATCH patterns
    let bound_vars = collect_bound_variables(program);

    // Check variables used in WHERE are bound
    for condition in &program.query.body.conditions {
        check_expression_variables(condition, &bound_vars, &mut errors);
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

/// Collect all variable names bound in MATCH patterns across the program.
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
        Expression::FunctionCall { args, .. } => args.iter().any(expr_has_aggregate),
        Expression::Array(elements) => elements.iter().any(expr_has_aggregate),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// FIBER-clause checks (D2 §5.8)
// ---------------------------------------------------------------------------

fn check_fiber_clauses(part: &MatchPart, layer: &Layer, errors: &mut Vec<QueryError>) {
    // 1. USING INSTITUTION aliases: uniqueness among themselves and with
    //    class-name USING imports (bare identifiers in queries).
    let mut alias_set: BTreeSet<&str> = BTreeSet::new();
    for alias in &part.using_institutions {
        if !alias_set.insert(alias.alias.as_str()) {
            errors.push(QueryError::type_check(
                "duplicate_using_institution_alias",
                format!("duplicate USING INSTITUTION alias: '{}'", alias.alias),
            ));
        }
    }

    // 2. Each FIBER clause.
    let class_iri = Iri::parse(wk::CLASS).unwrap();
    let requires_prop = Iri::parse(wk::REQUIRES).unwrap();
    let recommends_prop = Iri::parse(wk::RECOMMENDS).unwrap();
    let short_name_prop = Iri::parse(wk::SHORT_NAME).unwrap();

    for c in &part.clauses {
        let fc = match c {
            Clause::Fiber(fc) => fc,
            _ => continue,
        };

        // 2a. Institution ref — if it's a ShortName, alias must be declared.
        //     If it's a FullIri, we trust it resolves at runtime (institution
        //     resources aren't always layer-resident).
        if let Name::ShortName(ref alias) = fc.institution {
            if !alias_set.contains(alias.as_str()) {
                errors.push(QueryError::type_check(
                    "undeclared_institution_alias",
                    format!(
                        "FIBER refers to undeclared institution alias '{alias}' — \
                         add `USING INSTITUTION \"...\" AS {alias}` or use an inline IRI"
                    ),
                ));
            }
        }

        // 2b. Resolve the query class. If resolvable in the layer, it must
        //     be a Class; we also use it to scope short-name params.
        let query_class_iri = match &fc.query_class {
            Name::FullIri(iri) => Some(iri.clone()),
            Name::ShortName(name) => resolve_short_name_to_class(layer, name),
        };

        let class_resource = query_class_iri
            .as_ref()
            .and_then(|iri| layer.resolve(iri).map(|arc| (*arc).clone()));

        if let Some(ref cr) = class_resource {
            if !cr.is_instance_of(&class_iri) {
                errors.push(QueryError::type_check(
                    "fiber_query_class_not_class",
                    format!(
                        "FIBER query class '{}' does not resolve to a Class",
                        query_class_name_display(&fc.query_class)
                    ),
                ));
            }
        } else {
            // v1 lenient: if we can't resolve, skip downstream checks and
            // let the evaluator fail with a clearer message at dispatch
            // time (institution may have registered the class dynamically).
            continue;
        }
        let class_resource = class_resource.unwrap();

        // Collect the class's allowed property IRIs (requires ∪ recommends)
        // and build a short_name → IRI map for short-name resolution.
        let mut allowed_prop_iris: BTreeSet<String> = BTreeSet::new();
        let mut required_prop_iris: BTreeSet<String> = BTreeSet::new();
        let mut short_to_iri: BTreeMap<String, String> = BTreeMap::new();

        let required_list = collect_property_iris(&class_resource, &requires_prop);
        let recommended_list = collect_property_iris(&class_resource, &recommends_prop);

        for iri in &required_list {
            allowed_prop_iris.insert(iri.as_str().to_string());
            required_prop_iris.insert(iri.as_str().to_string());
        }
        for iri in &recommended_list {
            allowed_prop_iris.insert(iri.as_str().to_string());
        }
        for iri in allowed_prop_iris.iter() {
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

        // 2c. Validate each param.
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
                                 query class '{}' (requires ∪ recommends)",
                                query_class_name_display(&fc.query_class)
                            ),
                        ));
                        None
                    }
                }
            };
            if let Some(iri) = resolved_iri {
                supplied_iris.insert(iri);
            }
        }

        // 2d. Required-property coverage.
        for req in &required_prop_iris {
            if !supplied_iris.contains(req) {
                errors.push(QueryError::type_check(
                    "fiber_missing_required_param",
                    format!(
                        "FIBER for query class '{}' is missing required param '{}'",
                        query_class_name_display(&fc.query_class),
                        req
                    ),
                ));
            }
        }
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

fn resolve_short_name_to_class(layer: &Layer, short: &str) -> Option<Iri> {
    use crate::ontology::resource::Value;
    let class_iri = Iri::parse(wk::CLASS).unwrap();
    let short_prop = Iri::parse(wk::SHORT_NAME).unwrap();
    for (iri, res) in layer.iter_all_resources() {
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

fn query_class_name_display(name: &Name) -> String {
    match name {
        Name::ShortName(s) => s.clone(),
        Name::FullIri(i) => i.as_str().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Imports used above — keep below the public surface to avoid polluting
// the top.
// ---------------------------------------------------------------------------

use crate::ontology::resource::Resource;
use std::collections::BTreeMap;

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
        Arc::new(builder.build(
            std::sync::Arc::new(crate::layer::MemoryResourceCache::new()),
            std::sync::Arc::new(crate::layer::MemoryResourceBackend::new()),
        ))
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
}
