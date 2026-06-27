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
pub mod rank;
pub mod resolve;
pub mod stratify;
pub mod text;
pub mod type_check;
pub mod vector;

use crate::layer::Layer;
use crate::observability::{field, operation};
use crate::ontology::resource::Resource;
use document::QueryFingerprint;
use error::QueryError;

/// Outcome of executing an EigenQL program: the wrapped result
/// document, plus the chain-commit resources accumulated by `FIBER ...
/// INTO "<iri>"` clauses (D14 §9.3 chain-reinsertion via EigenQL).
///
/// Server-side callers commit `into_resources` to the regular chain
/// and surface their IRIs to clients via `QueryResponse.output_resource_iris`.
/// Local callers (CLI, in-process tests) typically discard them via
/// the [`execute`] / [`execute_with`] convenience wrappers.
#[derive(Debug, Default)]
pub struct QueryOutcome {
    /// Eigon document (array of resources) shaped per D2 Appendix A.
    pub document: Vec<Resource>,
    /// Resources the query accumulated under `FIBER ... INTO "<iri>"`,
    /// each carrying the caller-named `@id`. Empty when no FIBER
    /// clause used INTO.
    pub into_resources: Vec<Resource>,
}

/// Execute an EigenQL program against a layer chain.
///
/// Convenience wrapper for callers that don't dispatch FIBER clauses
/// (CLI local mode, tests). See [`execute_with`] for the full surface.
pub fn execute(program_str: &str, layer: &Layer) -> Result<Vec<Resource>, Vec<QueryError>> {
    execute_with(program_str, layer, evaluate::FiberRuntime::default())
}

/// Execute an EigenQL program, optionally supplying an institution
/// registry + execution context so FIBER clauses can dispatch.
///
/// Returns just the wrapped document; any `FIBER ... INTO "<iri>"`
/// resources the query produced are discarded. Callers that need the
/// chain-commit list (server-side `Query` RPC) should use
/// [`execute_with_into`].
pub fn execute_with(
    program_str: &str,
    layer: &Layer,
    runtime: evaluate::FiberRuntime<'_>,
) -> Result<Vec<Resource>, Vec<QueryError>> {
    execute_with_into(program_str, layer, runtime).map(|outcome| outcome.document)
}

/// Execute an EigenQL program and return both the wrapped result
/// document and the chain-commit resources accumulated by
/// `FIBER ... INTO "<iri>"` clauses.
///
/// Pipeline: lex → parse → stratify → type_check → evaluate → document wrap.
/// The returned [`QueryOutcome::document`] follows D2 Appendix A:
/// synthesized Property resources, a row Class, and a ResultSet
/// referencing them. [`QueryOutcome::into_resources`] is the list of
/// FIBER responses that the caller declared with `INTO`, ready for
/// the server's commit cycle.
pub fn execute_with_into(
    program_str: &str,
    layer: &Layer,
    runtime: evaluate::FiberRuntime<'_>,
) -> Result<QueryOutcome, Vec<QueryError>> {
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

    // 5. Evaluate — row resources with synthesized Property IRIs;
    //    INTO-named FIBER responses bubble up alongside.
    let fp = QueryFingerprint::of(program_str);
    let (rows, into_resources) =
        evaluate::evaluate(&program, layer, &fp, runtime).map_err(|e| vec![e])?;

    tracing::debug!(
        { field::OPERATION } = operation::QUERY_EVALUATE,
        { field::COUNT } = rows.len(),
        { field::SIZE_BYTES } = program_str.len(),
        "EigenQL query evaluated"
    );

    // 6. Wrap into a self-describing document (Appendix A).
    let document = document::wrap(&program.query, program_str, rows);
    Ok(QueryOutcome {
        document,
        into_resources,
    })
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

        lb.build(crate::layer::LayerStorage::in_memory())
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
            USING NAMESPACE "urn:test:regression:"
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

    // Smoke test for the FIBER-decomposition design proposal (#10):
    //
    //     MATCH ?a { ref: ?b }, ?b { name: ?n } RETURN [] { n: ?n }
    //
    // confirms that EigenQL's pattern-chain mechanism — two patterns in
    // one MATCH clause sharing a variable via implicit equi-join —
    // already lets us decompose a resource bound in one pattern via a
    // follow-up pattern. The same mechanism would let a FIBER-bound
    // variable be decomposed by a subsequent pattern, *if* the FIBER
    // result is reachable the same way (bound to an IRI that resolves
    // in the layer, or directly to a Resource value the evaluator can
    // dereference).
    //
    // This test validates step one: both resources in the layer.
    #[test]
    fn match_pattern_chain_across_shared_variable() {
        let mut lb = LayerBuilder::new("chain-test", None);

        let a_iri = Iri::parse("urn:chain:a").unwrap();
        let b_iri = Iri::parse("urn:chain:b").unwrap();

        let mut a = Resource::new(a_iri);
        a.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String("urn:chain:A".to_string())]),
        );
        // ResourceRef-valued cross-reference. The evaluator's
        // `values_equal` must treat this as equal to the resource's
        // String-form IRI so the equi-join across patterns succeeds.
        a.set(
            Iri::parse("urn:chain:ref").unwrap(),
            Value::ResourceRef(b_iri.clone()),
        );
        lb.add_resource(a).unwrap();

        let mut b = Resource::new(b_iri);
        b.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String("urn:chain:B".to_string())]),
        );
        b.set(
            Iri::parse("urn:chain:name").unwrap(),
            Value::String("hello".to_string()),
        );
        lb.add_resource(b).unwrap();

        let layer = lb.build(crate::layer::LayerStorage::in_memory());

        let query_str = r#"
            MATCH ?a { "urn:chain:ref": ?b },
                  ?b { "urn:chain:name": ?n }
            RETURN [] { n: ?n }
        "#;
        let document = execute(query_str, &layer).expect("query should succeed");

        // Find the ResultSet and confirm one row with the 'n' short-name
        // mapped to 'hello'.
        let is_a = Iri::parse(wk::IS_A).unwrap();
        let result_set = document
            .iter()
            .find(|r| match r.get(&is_a) {
                Some(Value::Array(a)) => a.iter().any(|v| match v {
                    Value::String(s) => s == "urn:eigenius:query:ResultSet",
                    _ => false,
                }),
                _ => false,
            })
            .expect("ResultSet in document");
        let row_count = match result_set.get(&Iri::parse("urn:eigenius:query:row_count").unwrap()) {
            Some(Value::Integer(n)) => *n,
            _ => panic!("missing row_count"),
        };
        assert_eq!(row_count, 1, "expected exactly one row");

        let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
            Some(Value::Array(a)) => a,
            _ => panic!("missing rows"),
        };
        let row = match &rows[0] {
            Value::Embedded(r) => r,
            _ => panic!("row must be embedded"),
        };

        // Find the row Property with short_name "n" to discover its IRI,
        // then read the row's value under that IRI.
        let prop_iri = document
            .iter()
            .find(|r| {
                matches!(r.get(&Iri::parse(wk::SHORT_NAME).unwrap()),
                    Some(Value::String(s)) if s == "n")
            })
            .and_then(|r| r.id().cloned())
            .expect("Property resource with short_name 'n' must exist");

        let n_value = row.get(&prop_iri).expect("row should have the 'n' value");
        assert!(
            matches!(n_value, Value::String(s) if s == "hello"),
            "expected n=\"hello\", got {n_value:?}"
        );
    }
}
