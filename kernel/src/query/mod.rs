//! EigenQL query language: lexer, parser, stratification, type checker, and evaluator.
//!
//! Implements the EigenQL specification from design doc D2.

pub mod ast;
pub mod document;
pub mod error;
pub mod evaluate;
pub mod functions;
pub mod lexer;
pub mod parser;
pub mod stratify;
pub mod type_check;

use crate::layer::Layer;
use crate::ontology::resource::Resource;
use document::QueryFingerprint;
use error::QueryError;

/// Execute an EigenQL program against a layer chain.
///
/// Pipeline: lex → parse → stratify → type_check → evaluate → document wrap.
/// Returns an Eigon document (array of resources) shaped per D2 Appendix A:
/// synthesized Property resources, a row Class, and a ResultSet referencing
/// them. Callers typically hand this straight to
/// `eigon_cbor::serialize_document`.
pub fn execute(program_str: &str, layer: &Layer) -> Result<Vec<Resource>, Vec<QueryError>> {
    // 1. Lex
    let tokens = lexer::tokenize(program_str).map_err(|e| vec![e])?;

    // 2. Parse
    let program = parser::parse(tokens).map_err(|e| vec![e])?;

    // 3. Stratification check
    stratify::stratify(&program.definitions).map_err(|e| vec![e])?;

    // 4. Type check
    let type_errors = type_check::type_check(&program, layer);
    if !type_errors.is_empty() {
        return Err(type_errors);
    }

    // 5. Evaluate — row resources with synthesized Property IRIs
    let fp = QueryFingerprint::of(program_str);
    let rows = evaluate::evaluate(&program, layer, &fp).map_err(|e| vec![e])?;

    // 6. Wrap into a self-describing document (Appendix A).
    Ok(document::wrap(&program.query, program_str, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;

    fn make_ontology_layer() -> Layer {
        // A minimal layer with a single Class the query can match.
        let mut lb = LayerBuilder::new("test-regression-9", None);
        let class_iri = Iri::parse("urn:test:regression:Thing").unwrap();
        let mut cls = Resource::new(class_iri.clone());
        cls.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Thing".to_string()),
        );
        lb.add_resource(cls).unwrap();

        // An instance with a short_name.
        let inst_iri = Iri::parse("urn:test:regression:thing-1").unwrap();
        let mut inst = Resource::new(inst_iri);
        inst.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String("urn:test:regression:Thing".to_string())]),
        );
        inst.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("first".to_string()),
        );
        lb.add_resource(inst).unwrap();

        lb.build()
    }

    /// Regression for issue #9: short-name RETURN keys (`{ iri: ?c, name: ?name }`)
    /// must no longer be silently prefixed with `urn:query:result:`. The
    /// user-facing short names appear on synthesized Property resources,
    /// not on row property keys.
    #[test]
    fn issue_9_return_short_names_drive_property_shortnames() {
        let layer = make_ontology_layer();
        let query_str = r#"
            USING "urn:test:regression:Thing"
            MATCH Thing(?c) { "urn:eigenius:core:short_name": ?name }
            RETURN [] { iri: ?c, name: ?name }
        "#;

        let document = execute(query_str, &layer).expect("query should succeed");

        // 1. No resource in the document may carry an IRI starting with
        //    the old `urn:query:result:` prefix — that was the bug.
        for res in &document {
            if let Some(id) = res.id() {
                assert!(
                    !id.as_str().starts_with("urn:query:result:"),
                    "found stale prefix on resource id: {}",
                    id.as_str()
                );
            }
        }

        // 2. The document should contain Property resources with the
        //    short_names the user typed in RETURN.
        let short_name_prop = Iri::parse(wk::SHORT_NAME).unwrap();
        let property_class = wk::PROPERTY;
        let is_a = Iri::parse(wk::IS_A).unwrap();

        let property_short_names: Vec<String> = document
            .iter()
            .filter(|r| match r.get(&is_a) {
                Some(Value::Array(a)) => a.iter().any(|v| match v {
                    Value::String(s) => s == property_class,
                    _ => false,
                }),
                _ => false,
            })
            .filter_map(|r| match r.get(&short_name_prop) {
                Some(Value::String(s)) => Some(s.clone()),
                _ => None,
            })
            .collect();

        assert!(
            property_short_names.contains(&"iri".to_string()),
            "expected a Property with short_name='iri', got {property_short_names:?}"
        );
        assert!(
            property_short_names.contains(&"name".to_string()),
            "expected a Property with short_name='name', got {property_short_names:?}"
        );

        // 3. The ResultSet must reference a row class whose property
        //    list covers both Properties.
        let result_set = document
            .iter()
            .find(|r| match r.get(&is_a) {
                Some(Value::Array(a)) => a.iter().any(|v| match v {
                    Value::String(s) => s == "urn:eigenius:query:ResultSet",
                    _ => false,
                }),
                _ => false,
            })
            .expect("ResultSet must be in the document");

        let row_count = match result_set.get(&Iri::parse("urn:eigenius:query:row_count").unwrap()) {
            Some(Value::Integer(n)) => *n,
            _ => panic!("ResultSet missing row_count"),
        };
        assert_eq!(row_count, 1, "expected one row");

        // 4. The embedded row's keys are the synthesized Property IRIs
        //    (the same ones the Property resources in the document
        //    describe) — NOT user-typed short names as raw keys.
        let rows_prop = Iri::parse("urn:eigenius:query:rows").unwrap();
        let rows = match result_set.get(&rows_prop) {
            Some(Value::Array(a)) => a,
            _ => panic!("ResultSet missing rows array"),
        };
        assert_eq!(rows.len(), 1);
        let row = match &rows[0] {
            Value::Embedded(r) => r,
            _ => panic!("row must be embedded"),
        };

        // Gather the Property IRIs the document declares.
        let property_iris: Vec<String> = document
            .iter()
            .filter(|r| match r.get(&is_a) {
                Some(Value::Array(a)) => a.iter().any(|v| match v {
                    Value::String(s) => s == property_class,
                    _ => false,
                }),
                _ => false,
            })
            .filter_map(|r| r.id().map(|i| i.as_str().to_string()))
            .collect();

        // Each row key (aside from is_a) should be one of the Property IRIs.
        for key in row.properties().keys() {
            if key.as_str() == wk::IS_A {
                continue;
            }
            assert!(
                property_iris.contains(&key.as_str().to_string()),
                "row key {} is not one of the declared Property IRIs {:?}",
                key.as_str(),
                property_iris
            );
        }
    }
}
