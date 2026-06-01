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
pub mod embed_infer;
pub mod error;
pub mod evaluate;
pub mod functions;
pub mod lexer;
pub mod parser;
pub mod rank;
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

    // 4b. D43 §4.4 — EMBED model inference. Rewrites 1-arg
    //     EMBED("text") calls into 2-arg EMBED("text", "<model>")
    //     using the surrounding VECTOR_NEAR / VECTOR_SIM context's
    //     active VectorIndex `model_iri`. The evaluator never sees
    //     the 1-arg form.
    let mut program = program;
    let infer_errors = embed_infer::infer_embed_models(&mut program, layer);
    if !infer_errors.is_empty() {
        return Err(infer_errors);
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

    /// D43 §3.7 / M7.1 — `TOP K BY ?score DESC` truncates the result
    /// to the K highest-scoring rows ordered descending. Equivalent
    /// to `ORDER BY ?score DESC LIMIT K` for v1 (the planner-side
    /// pushdown is M7.4); this test pins the row-count and ordering
    /// contract.
    #[test]
    fn top_k_by_truncates_to_k_rows_descending() {
        let mut lb = LayerBuilder::new("top-k-test", None);

        // Class declaration.
        let class_iri = Iri::parse("urn:test:topk:Item").unwrap();
        let mut cls = Resource::new(class_iri.clone());
        cls.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Item".to_string()),
        );
        lb.add_resource(cls).unwrap();

        // Five instances with scores 1..=5 — we'll TOP 3 BY ?score DESC
        // and expect rows with scores 5, 4, 3 in that order.
        let score_prop = Iri::parse("urn:test:topk:score").unwrap();
        for i in 1..=5 {
            let inst_iri = Iri::parse(&format!("urn:test:topk:item-{i}")).unwrap();
            let mut inst = Resource::new(inst_iri);
            inst.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String("urn:test:topk:Item".to_string())]),
            );
            inst.set(score_prop.clone(), Value::Integer(i));
            lb.add_resource(inst).unwrap();
        }
        let layer = lb.build(crate::layer::LayerStorage::in_memory());

        // RETURN names the column `s` so the variable in TOP K BY
        // matches the row's short-name on the post-shape sort
        // (matches the existing ORDER BY contract — the expression
        // must reference a variable that the RETURN exposes under
        // that same short-name).
        let query_str = r#"
            USING "urn:test:topk:Item"
            MATCH Item(?i) { "urn:test:topk:score": ?s }
            RETURN [] { s: ?s }
            TOP 3 BY ?s DESC
        "#;
        let document = execute(query_str, &layer).expect("query should succeed");

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
        assert_eq!(row_count, 3, "TOP 3 BY must truncate to 3 rows");

        let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
            Some(Value::Array(a)) => a,
            _ => panic!("missing rows"),
        };

        // Locate the synthesized Property for the row's "s" key.
        let score_row_prop = document
            .iter()
            .find(|r| {
                matches!(r.get(&Iri::parse(wk::SHORT_NAME).unwrap()),
                    Some(Value::String(s)) if s == "s")
            })
            .and_then(|r| r.id().cloned())
            .expect("Property resource with short_name 's' must exist");

        let scores: Vec<i64> = rows
            .iter()
            .map(|row| match row {
                Value::Embedded(r) => match r.get(&score_row_prop) {
                    Some(Value::Integer(n)) => *n,
                    other => panic!("row score not Integer: {other:?}"),
                },
                _ => panic!("row must be embedded"),
            })
            .collect();
        assert_eq!(
            scores,
            vec![5, 4, 3],
            "TOP 3 BY ?s DESC must yield the three highest scores in descending order"
        );
    }

    /// D43 §3.7 — `TOP K BY ?score ASC` returns the K *lowest*
    /// scores ascending. Mirrors the DESC case above to make sure
    /// direction is propagated end-to-end.
    #[test]
    fn top_k_by_ascending_yields_lowest_k() {
        let mut lb = LayerBuilder::new("top-k-asc-test", None);
        let class_iri = Iri::parse("urn:test:topk:Item").unwrap();
        let mut cls = Resource::new(class_iri);
        cls.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Item".to_string()),
        );
        lb.add_resource(cls).unwrap();

        let score_prop = Iri::parse("urn:test:topk:score").unwrap();
        for i in 1..=5 {
            let inst_iri = Iri::parse(&format!("urn:test:topk:item-{i}")).unwrap();
            let mut inst = Resource::new(inst_iri);
            inst.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String("urn:test:topk:Item".to_string())]),
            );
            inst.set(score_prop.clone(), Value::Integer(i));
            lb.add_resource(inst).unwrap();
        }
        let layer = lb.build(crate::layer::LayerStorage::in_memory());

        let query_str = r#"
            USING "urn:test:topk:Item"
            MATCH Item(?i) { "urn:test:topk:score": ?s }
            RETURN [] { s: ?s }
            TOP 2 BY ?s ASC
        "#;
        let document = execute(query_str, &layer).expect("query should succeed");

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
        assert_eq!(row_count, 2, "TOP 2 BY must truncate to 2 rows");

        let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
            Some(Value::Array(a)) => a,
            _ => panic!("missing rows"),
        };
        let score_row_prop = document
            .iter()
            .find(|r| {
                matches!(r.get(&Iri::parse(wk::SHORT_NAME).unwrap()),
                    Some(Value::String(s)) if s == "s")
            })
            .and_then(|r| r.id().cloned())
            .expect("Property resource with short_name 's' must exist");

        let scores: Vec<i64> = rows
            .iter()
            .map(|row| match row {
                Value::Embedded(r) => match r.get(&score_row_prop) {
                    Some(Value::Integer(n)) => *n,
                    other => panic!("row score not Integer: {other:?}"),
                },
                _ => panic!("row must be embedded"),
            })
            .collect();
        assert_eq!(
            scores,
            vec![1, 2],
            "TOP 2 BY ?s ASC must yield the two lowest scores"
        );
    }

    /// D43 §3.6 / §6.4 / M7.2 — `RRF(?a, ?b)` in RETURN materialises
    /// per-source ranks across all bindings and emits the §3.6 fused
    /// score per row. The score for binding `i` is
    /// `sum_j 1 / (k + rank_j(i))` with default `k = 60`.
    ///
    /// Setup: 3 instances with `a` and `b` Integer scores chosen so
    /// the two sources rank the rows differently. We verify that the
    /// RRF row matches the analytic formula for each binding.
    #[test]
    fn rrf_fuses_two_source_rankings_in_return() {
        let mut lb = LayerBuilder::new("rrf-test", None);
        let class_iri = Iri::parse("urn:test:rrf:Item").unwrap();
        let mut cls = Resource::new(class_iri);
        cls.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Item".to_string()),
        );
        lb.add_resource(cls).unwrap();

        // Three instances with two integer-scored properties. Higher
        // value → better rank (assign_ranks_desc).
        let a_prop = Iri::parse("urn:test:rrf:a").unwrap();
        let b_prop = Iri::parse("urn:test:rrf:b").unwrap();
        for (name, a, b) in &[("x", 3, 1), ("y", 2, 2), ("z", 1, 3)] {
            let inst_iri = Iri::parse(&format!("urn:test:rrf:item-{name}")).unwrap();
            let mut inst = Resource::new(inst_iri);
            inst.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String("urn:test:rrf:Item".to_string())]),
            );
            inst.set(a_prop.clone(), Value::Integer(*a as i64));
            inst.set(b_prop.clone(), Value::Integer(*b as i64));
            lb.add_resource(inst).unwrap();
        }
        let layer = lb.build(crate::layer::LayerStorage::in_memory());

        // RRF over two variable references — both are "recognised
        // score expressions" per the score-expression recognition
        // rule (§4.7) because they're variables.
        let query_str = r#"
            USING "urn:test:rrf:Item"
            MATCH Item(?i) { "urn:test:rrf:a": ?a, "urn:test:rrf:b": ?b }
            RETURN [] { a: ?a, b: ?b, fused: RRF(?a, ?b) }
        "#;
        let document = execute(query_str, &layer).expect("query should succeed");

        let is_a = Iri::parse(wk::IS_A).unwrap();
        let result_set = document
            .iter()
            .find(|r| match r.get(&is_a) {
                Some(Value::Array(arr)) => arr.iter().any(|v| match v {
                    Value::String(s) => s == "urn:eigenius:query:ResultSet",
                    _ => false,
                }),
                _ => false,
            })
            .expect("ResultSet in document");

        let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
            Some(Value::Array(arr)) => arr,
            _ => panic!("rows missing"),
        };
        assert_eq!(rows.len(), 3, "RRF must not change row cardinality");

        let prop_iri_for = |sn: &str| -> Iri {
            document
                .iter()
                .find(|r| {
                    matches!(r.get(&Iri::parse(wk::SHORT_NAME).unwrap()),
                        Some(Value::String(s)) if s == sn)
                })
                .and_then(|r| r.id().cloned())
                .unwrap_or_else(|| panic!("Property with short_name '{sn}' must exist"))
        };
        let prop_a = prop_iri_for("a");
        let prop_b = prop_iri_for("b");
        let prop_fused = prop_iri_for("fused");

        // Compute expected ranks: higher value → better rank.
        // Source ?a: 3→rank1, 2→rank2, 1→rank3. Same for ?b.
        let k_const = 60.0;
        for row in rows {
            let r = match row {
                Value::Embedded(r) => r,
                _ => panic!("row not embedded"),
            };
            let a = match r.get(&prop_a) {
                Some(Value::Integer(n)) => *n,
                other => panic!("a not Integer: {other:?}"),
            };
            let b = match r.get(&prop_b) {
                Some(Value::Integer(n)) => *n,
                other => panic!("b not Integer: {other:?}"),
            };
            let fused = match r.get(&prop_fused) {
                Some(Value::Float(f)) => *f,
                other => panic!("fused not Float: {other:?}"),
            };
            // rank = 4 - score (for scores in {1,2,3}).
            let rank_a = 4 - a as i64;
            let rank_b = 4 - b as i64;
            let expected = 1.0 / (k_const + rank_a as f64) + 1.0 / (k_const + rank_b as f64);
            assert!(
                (fused - expected).abs() < 1e-12,
                "row (a={a}, b={b}): expected fused={expected}, got {fused}"
            );
        }
    }

    /// D43 §3.6 — `RRF(s1, s2, k: N)` honours the user-supplied
    /// `k` constant. A row that ranks 1 in both sources at `k=10`
    /// should produce `2 / 11`, distinct from the default-k value
    /// `2 / 61`.
    #[test]
    fn rrf_honours_custom_k_named_argument() {
        let mut lb = LayerBuilder::new("rrf-k-test", None);
        let class_iri = Iri::parse("urn:test:rrf:Item").unwrap();
        let mut cls = Resource::new(class_iri);
        cls.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Item".to_string()),
        );
        lb.add_resource(cls).unwrap();

        let a_prop = Iri::parse("urn:test:rrf:a").unwrap();
        let b_prop = Iri::parse("urn:test:rrf:b").unwrap();
        let inst_iri = Iri::parse("urn:test:rrf:solo").unwrap();
        let mut inst = Resource::new(inst_iri);
        inst.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String("urn:test:rrf:Item".to_string())]),
        );
        inst.set(a_prop, Value::Integer(5));
        inst.set(b_prop, Value::Integer(7));
        lb.add_resource(inst).unwrap();
        let layer = lb.build(crate::layer::LayerStorage::in_memory());

        let query_str = r#"
            USING "urn:test:rrf:Item"
            MATCH Item(?i) { "urn:test:rrf:a": ?a, "urn:test:rrf:b": ?b }
            RETURN [] { fused: RRF(?a, ?b, k: 10) }
        "#;
        let document = execute(query_str, &layer).expect("query should succeed");

        let is_a = Iri::parse(wk::IS_A).unwrap();
        let result_set = document
            .iter()
            .find(|r| match r.get(&is_a) {
                Some(Value::Array(arr)) => arr.iter().any(|v| match v {
                    Value::String(s) => s == "urn:eigenius:query:ResultSet",
                    _ => false,
                }),
                _ => false,
            })
            .expect("ResultSet");
        let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
            Some(Value::Array(arr)) => arr,
            _ => panic!("rows missing"),
        };
        assert_eq!(rows.len(), 1);

        let prop_fused = document
            .iter()
            .find(|r| {
                matches!(r.get(&Iri::parse(wk::SHORT_NAME).unwrap()),
                    Some(Value::String(s)) if s == "fused")
            })
            .and_then(|r| r.id().cloned())
            .expect("fused Property");
        let row = match &rows[0] {
            Value::Embedded(r) => r,
            _ => panic!(),
        };
        let fused = match row.get(&prop_fused) {
            Some(Value::Float(f)) => *f,
            other => panic!("fused not Float: {other:?}"),
        };
        // Single row → ranks 1 in both sources; k=10. Fused = 2/11.
        let expected = 2.0 / 11.0;
        assert!(
            (fused - expected).abs() < 1e-12,
            "expected fused={expected} with k=10, got {fused}"
        );
    }

    /// D45 — `BIND(expr AS ?var)` evaluates `expr` per-binding and
    /// makes `?var` available to RETURN. End-to-end semantic check
    /// against the same per-row Item corpus the TOP K BY tests use.
    #[test]
    fn bind_introduces_row_local_variable_visible_to_return() {
        let mut lb = LayerBuilder::new("bind-e2e", None);
        let class_iri = Iri::parse("urn:test:bind:Item").unwrap();
        let mut cls = Resource::new(class_iri);
        cls.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Item".to_string()),
        );
        lb.add_resource(cls).unwrap();

        let score_prop = Iri::parse("urn:test:bind:score").unwrap();
        for i in [1, 4, 9] {
            let inst_iri = Iri::parse(&format!("urn:test:bind:item-{i}")).unwrap();
            let mut inst = Resource::new(inst_iri);
            inst.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String("urn:test:bind:Item".to_string())]),
            );
            inst.set(score_prop.clone(), Value::Integer(i));
            lb.add_resource(inst).unwrap();
        }
        let layer = lb.build(crate::layer::LayerStorage::in_memory());

        // Compute ?squared = ?s * ?s per-row via BIND, return it.
        // 1, 4, 9 → 1, 16, 81.
        let query_str = r#"
            USING "urn:test:bind:Item"
            MATCH Item(?i) { "urn:test:bind:score": ?s }
            WHERE BIND(?s * ?s AS ?squared)
            RETURN [] { squared: ?squared }
        "#;
        let document = execute(query_str, &layer).expect("BIND query should succeed");

        let is_a = Iri::parse(wk::IS_A).unwrap();
        let result_set = document
            .iter()
            .find(|r| match r.get(&is_a) {
                Some(Value::Array(arr)) => arr.iter().any(|v| match v {
                    Value::String(s) => s == "urn:eigenius:query:ResultSet",
                    _ => false,
                }),
                _ => false,
            })
            .expect("ResultSet");
        let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
            Some(Value::Array(a)) => a,
            _ => panic!("missing rows"),
        };
        assert_eq!(rows.len(), 3);

        let prop_squared = document
            .iter()
            .find(|r| {
                matches!(r.get(&Iri::parse(wk::SHORT_NAME).unwrap()),
                    Some(Value::String(s)) if s == "squared")
            })
            .and_then(|r| r.id().cloned())
            .expect("squared property");
        let mut squared: Vec<i64> = rows
            .iter()
            .map(|row| match row {
                Value::Embedded(r) => match r.get(&prop_squared) {
                    Some(Value::Integer(n)) => *n,
                    other => panic!("squared not Integer: {other:?}"),
                },
                _ => panic!("row not embedded"),
            })
            .collect();
        squared.sort();
        assert_eq!(squared, vec![1, 16, 81]);
    }

    /// D45 — a BIND result is referenceable from a subsequent
    /// filter clause in the same WHERE list. Filter drops rows
    /// that don't satisfy.
    #[test]
    fn bind_value_visible_to_subsequent_filter() {
        let mut lb = LayerBuilder::new("bind-filter-e2e", None);
        let class_iri = Iri::parse("urn:test:bind:Item").unwrap();
        let mut cls = Resource::new(class_iri);
        cls.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Item".to_string()),
        );
        lb.add_resource(cls).unwrap();

        let score_prop = Iri::parse("urn:test:bind:score").unwrap();
        for i in [1, 4, 9] {
            let inst_iri = Iri::parse(&format!("urn:test:bind:item-{i}")).unwrap();
            let mut inst = Resource::new(inst_iri);
            inst.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String("urn:test:bind:Item".to_string())]),
            );
            inst.set(score_prop.clone(), Value::Integer(i));
            lb.add_resource(inst).unwrap();
        }
        let layer = lb.build(crate::layer::LayerStorage::in_memory());

        // ?squared > 10 filters out the rows whose original score
        // was 1 or 4 (squared = 1, 16). Only 81 survives.
        let query_str = r#"
            USING "urn:test:bind:Item"
            MATCH Item(?i) { "urn:test:bind:score": ?s }
            WHERE BIND(?s * ?s AS ?squared),
                  ?squared > 10
            RETURN [] { squared: ?squared }
        "#;
        let document = execute(query_str, &layer).expect("BIND+filter query should succeed");

        let is_a = Iri::parse(wk::IS_A).unwrap();
        let result_set = document
            .iter()
            .find(|r| match r.get(&is_a) {
                Some(Value::Array(arr)) => arr.iter().any(|v| match v {
                    Value::String(s) => s == "urn:eigenius:query:ResultSet",
                    _ => false,
                }),
                _ => false,
            })
            .expect("ResultSet");
        let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
            Some(Value::Array(a)) => a,
            _ => panic!("missing rows"),
        };
        // Only s=9 (squared=81) and s=4 (squared=16) survive ?squared > 10.
        assert_eq!(
            rows.len(),
            2,
            "expected 2 rows after BIND+filter; got {}",
            rows.len()
        );
    }

    /// D43 §3.6 + §3.7 / M7.4 — `TOP K BY RRF(...) DESC` end-to-
    /// end. The sort key is the RRF call itself (inlined, not a
    /// row-renamed variable), exercising the M7.4 sort-against-
    /// binding restructure: the sort path evaluates the Rrf
    /// expression against the underlying binding with the rrf
    /// context, rather than looking up a row property short-name.
    #[test]
    fn top_k_by_rrf_sorts_by_fused_score() {
        let mut lb = LayerBuilder::new("top-k-rrf-test", None);
        let class_iri = Iri::parse("urn:test:rrf:Item").unwrap();
        let mut cls = Resource::new(class_iri);
        cls.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Item".to_string()),
        );
        lb.add_resource(cls).unwrap();

        // Three rows with *aligned* rankings (same row ranks 1st
        // in both ?a and ?b). RRF's mirror-image case is a tie
        // (1/61 + 1/63 = 2/62 for k=60), so this test uses an
        // aligned corpus where (10,10) is uniquely top under both
        // sources and therefore wins the fused ranking too.
        let a_prop = Iri::parse("urn:test:rrf:a").unwrap();
        let b_prop = Iri::parse("urn:test:rrf:b").unwrap();
        for (name, a, b) in &[("x", 10, 10), ("y", 5, 5), ("z", 1, 1)] {
            let inst_iri = Iri::parse(&format!("urn:test:rrf:item-{name}")).unwrap();
            let mut inst = Resource::new(inst_iri);
            inst.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String("urn:test:rrf:Item".to_string())]),
            );
            inst.set(a_prop.clone(), Value::Integer(*a as i64));
            inst.set(b_prop.clone(), Value::Integer(*b as i64));
            lb.add_resource(inst).unwrap();
        }
        let layer = lb.build(crate::layer::LayerStorage::in_memory());

        // Note the RRF call inlined directly in TOP K BY — no
        // RETURN-renamed-variable kludge. This is the §6.5 worked-
        // example shape (modulo the binding-introduction surface,
        // which D45 covers).
        let query_str = r#"
            USING "urn:test:rrf:Item"
            MATCH Item(?i) { "urn:test:rrf:a": ?a, "urn:test:rrf:b": ?b }
            RETURN [] { a: ?a, b: ?b }
            TOP 1 BY RRF(?a, ?b) DESC
        "#;
        let document = execute(query_str, &layer).expect("TOP K BY RRF should succeed");

        let is_a = Iri::parse(wk::IS_A).unwrap();
        let result_set = document
            .iter()
            .find(|r| match r.get(&is_a) {
                Some(Value::Array(arr)) => arr.iter().any(|v| match v {
                    Value::String(s) => s == "urn:eigenius:query:ResultSet",
                    _ => false,
                }),
                _ => false,
            })
            .expect("ResultSet");
        let row_count = match result_set.get(&Iri::parse("urn:eigenius:query:row_count").unwrap()) {
            Some(Value::Integer(n)) => *n,
            _ => panic!("missing row_count"),
        };
        assert_eq!(row_count, 1, "TOP 1 BY must truncate to one row");

        let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
            Some(Value::Array(a)) => a,
            _ => panic!("missing rows"),
        };

        let prop_iri_for = |sn: &str| -> Iri {
            document
                .iter()
                .find(|r| {
                    matches!(r.get(&Iri::parse(wk::SHORT_NAME).unwrap()),
                        Some(Value::String(s)) if s == sn)
                })
                .and_then(|r| r.id().cloned())
                .unwrap_or_else(|| panic!("Property with short_name '{sn}' must exist"))
        };
        let prop_a = prop_iri_for("a");
        let prop_b = prop_iri_for("b");

        let row = match &rows[0] {
            Value::Embedded(r) => r,
            _ => panic!("row not embedded"),
        };
        let a = match row.get(&prop_a) {
            Some(Value::Integer(n)) => *n,
            other => panic!("a not Integer: {other:?}"),
        };
        let b = match row.get(&prop_b) {
            Some(Value::Integer(n)) => *n,
            other => panic!("b not Integer: {other:?}"),
        };
        // (10,10) is rank 1 in both sources → 2/(60+1) fused.
        // (5,5)   is rank 2 in both → 2/(60+2).
        // (1,1)   is rank 3 in both → 2/(60+3).
        // (10,10) wins.
        assert_eq!(
            (a, b),
            (10, 10),
            "TOP 1 BY RRF(?a, ?b) DESC must surface the top-of-both row first"
        );
    }

    /// D45 + D43 §3.6 + §3.7 / M7.4 — `TOP K BY ?fused DESC` where
    /// `?fused` is BIND-introduced from an RRF call. The BIND
    /// rejects RRF directly (D45 §3), so the fused score must come
    /// from RETURN's `fused: RRF(...)`. The sort path's row-property
    /// fallback handles the variable-to-row-short-name lookup; this
    /// pins the existing TOP K BY-of-RETURN-renamed-RRF surface
    /// against accidental regression.
    #[test]
    #[ignore = "RETURN-renamed names aren't query variables; this surface needs BIND-of-RRF (deferred per D45 §3) or a SQL-style AS-binding extension"]
    fn top_k_by_variable_falls_back_to_row_property() {
        let mut lb = LayerBuilder::new("top-k-fallback-test", None);
        let class_iri = Iri::parse("urn:test:rrf:Item").unwrap();
        let mut cls = Resource::new(class_iri);
        cls.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("Item".to_string()),
        );
        lb.add_resource(cls).unwrap();

        let a_prop = Iri::parse("urn:test:rrf:a").unwrap();
        let b_prop = Iri::parse("urn:test:rrf:b").unwrap();
        for (name, a, b) in &[("x", 3, 1), ("y", 2, 2), ("z", 1, 3)] {
            let inst_iri = Iri::parse(&format!("urn:test:rrf:item-{name}")).unwrap();
            let mut inst = Resource::new(inst_iri);
            inst.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String("urn:test:rrf:Item".to_string())]),
            );
            inst.set(a_prop.clone(), Value::Integer(*a as i64));
            inst.set(b_prop.clone(), Value::Integer(*b as i64));
            lb.add_resource(inst).unwrap();
        }
        let layer = lb.build(crate::layer::LayerStorage::in_memory());

        let query_str = r#"
            USING "urn:test:rrf:Item"
            MATCH Item(?i) { "urn:test:rrf:a": ?a, "urn:test:rrf:b": ?b }
            RETURN [] { fused: RRF(?a, ?b), a: ?a, b: ?b }
            TOP 1 BY ?fused DESC
        "#;
        let document = execute(query_str, &layer).expect("TOP K BY ?fused should succeed");

        let is_a = Iri::parse(wk::IS_A).unwrap();
        let result_set = document
            .iter()
            .find(|r| match r.get(&is_a) {
                Some(Value::Array(arr)) => arr.iter().any(|v| match v {
                    Value::String(s) => s == "urn:eigenius:query:ResultSet",
                    _ => false,
                }),
                _ => false,
            })
            .expect("ResultSet");
        let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
            Some(Value::Array(a)) => a,
            _ => panic!("missing rows"),
        };
        assert_eq!(rows.len(), 1);

        let prop_a = document
            .iter()
            .find(|r| {
                matches!(r.get(&Iri::parse(wk::SHORT_NAME).unwrap()),
                    Some(Value::String(s)) if s == "a")
            })
            .and_then(|r| r.id().cloned())
            .expect("Property a");
        let row = match &rows[0] {
            Value::Embedded(r) => r,
            _ => panic!("row not embedded"),
        };
        let a = match row.get(&prop_a) {
            Some(Value::Integer(n)) => *n,
            other => panic!("a not Integer: {other:?}"),
        };
        // The (2, 2) row has the highest fused score; ?fused
        // resolves via row-property fallback to its row:fused
        // value, and the sort picks the (2, 2) row out.
        assert_eq!(a, 2, "expected (2,2) row to win on ?fused");
    }

    /// D43 §4.7 — RRF rejects a non-recognised score expression at
    /// typecheck time with the spec's diagnostic.
    #[test]
    fn rrf_rejects_non_score_argument_at_typecheck() {
        let layer = make_ontology_layer();
        // Literal `1.0` is a Float but not a recognised score
        // expression — only TEXT_SCORE / VECTOR_SIM / arithmetic /
        // variable bound to one of those are accepted.
        let query_str = r#"
            USING "urn:test:regression:Thing"
            MATCH Thing(?c) { "urn:eigenius:core:short_name": ?name }
            RETURN [] { iri: ?c, fused: RRF(?name, 1.0) }
        "#;
        let errors = execute(query_str, &layer).expect_err("must fail typecheck");
        let combined = errors
            .iter()
            .map(|e| format!("{e}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            combined.contains("not a recognised score expression"),
            "expected the §4.7 diagnostic, got: {combined}"
        );
    }
}
