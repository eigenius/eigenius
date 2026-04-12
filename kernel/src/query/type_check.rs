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
        collect_pattern_vars(&def.body.patterns, &mut vars);
        for v in &def.variables {
            vars.insert(v.name.clone());
        }
    }

    collect_pattern_vars(&program.query.body.patterns, &mut vars);
    vars
}

fn collect_pattern_vars(patterns: &[Pattern], vars: &mut BTreeSet<String>) {
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
        Arc::new(builder.build())
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
