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

//! D43 §4.6 / M3.7 — query-scoped retrieval context for `TEXT_MATCH`
//! and `TEXT_SCORE` evaluation.
//!
//! The retrieval primitives need three things the row-by-row
//! expression evaluator doesn't otherwise carry:
//!
//! 1. **A property-variable → (subject-variable, property-IRI) map**.
//!    `TEXT_MATCH(?desc, "q")` is syntactically a row-local check, but
//!    semantically it's asking *"is this row's source subject in the
//!    text-index hits for `q` against the TextIndex on `?desc`'s
//!    property?"*. The map walks the program's MATCH patterns once
//!    so per-row evaluation can recover both ends of that question.
//! 2. **The active-TextIndex set at the query head**, computed once
//!    via `resolve_active_text_indexes`.
//! 3. **A per-query cache of text-search results**, keyed by
//!    `(index_iri, query_string)`. A query that mentions both
//!    `TEXT_MATCH(?desc, "q")` and `TEXT_SCORE(?desc, "q")` runs the
//!    index probe once and serves both calls (D43 §4.6 — "the
//!    parsed query for TEXT_MATCH and TEXT_SCORE with identical
//!    arguments runs once").
//!
//! Constructed at the top of [`crate::query::evaluate::evaluate`]
//! once per program; threaded through `FiberRuntime::retrieval` into
//! the expression evaluator.

use crate::layer::{
    resolve_active_text_indexes, resolve_active_vector_indexes, ActiveTextIndex, ActiveVectorIndex,
    Layer,
};
use crate::ontology::iri::Iri;
use crate::query::ast::{Clause, MatchPart, Pattern, Program, ValueOrVariable};
use crate::query::text::analyzer::registry;
use crate::query::text::search::{run_text_search, TextScoredHit, TextSearchError};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Captured binding of one property-bound variable. The
/// row-evaluator uses both fields: `subject_variable` to look up the
/// row's source subject in the binding map, `property_iri` to find
/// the active TextIndex and its analyzer.
#[derive(Debug, Clone)]
pub(super) struct PropertyBindingInfo {
    pub subject_variable: String,
    pub property_iri: Iri,
}

/// Query-scoped retrieval state. `'a` borrows from the program and
/// the layer; the inner `RefCell` exists so per-row evaluation can
/// memoise probe results without needing `&mut`.
pub struct RetrievalContext<'a> {
    /// `?var → (subject_variable, property_iri)` for every property-
    /// bound variable in the program (DEFINE bodies + main query).
    /// Empty for queries that don't reference any property-bound
    /// variable through retrieval primitives — the typechecker has
    /// already validated that `TEXT_MATCH`'s first argument *is* a
    /// property-bound variable, so absence here means the call was
    /// rejected at typecheck and should not reach the evaluator.
    property_bindings: BTreeMap<String, PropertyBindingInfo>,
    /// Snapshot of the active TextIndex set at the query head.
    text_indexes: Vec<ActiveTextIndex>,
    /// Snapshot of the active VectorIndex set at the query head.
    /// Used by `VECTOR_NEAR` / `VECTOR_SIM` (M5.5) to resolve a
    /// property-bound `?var` to its active VectorIndex Resource —
    /// same shape as `text_indexes`.
    vector_indexes: Vec<ActiveVectorIndex>,
    /// Per-`(index_iri, query)` memoisation. `Arc<Vec<…>>` so
    /// multiple TEXT_MATCH / TEXT_SCORE calls in the same query
    /// share the same allocation across many row evaluations.
    cache: RefCell<BTreeMap<(Iri, String), Arc<TextSearchOutcome>>>,
    /// Borrow of the layer so probes run against the same head the
    /// rest of the query is evaluated against.
    layer: &'a Layer,
}

/// Either the probe's hits, or the error it raised. Cached so a
/// failing probe doesn't get retried per row.
#[derive(Debug)]
pub(super) enum TextSearchOutcome {
    Ok(Vec<TextScoredHit>),
    Err(TextSearchError),
}

impl<'a> RetrievalContext<'a> {
    /// Build the retrieval context for `program` against `layer`.
    /// Walks every MATCH pattern (both DEFINE bodies and the main
    /// query body) and records each property-bound variable,
    /// resolving short-name properties against the layer chain.
    pub fn new(program: &Program, layer: &'a Layer) -> Self {
        let mut property_bindings: BTreeMap<String, PropertyBindingInfo> = BTreeMap::new();
        for part in
            std::iter::once(&program.query.body).chain(program.definitions.iter().map(|d| &d.body))
        {
            collect_property_bindings(part, layer, &mut property_bindings);
        }
        let text_indexes = resolve_active_text_indexes(layer);
        let vector_indexes = resolve_active_vector_indexes(layer);
        Self {
            property_bindings,
            text_indexes,
            vector_indexes,
            cache: RefCell::new(BTreeMap::new()),
            layer,
        }
    }

    /// Find the active VectorIndex Resource targeting `property_iri`.
    /// Returns `None` if no active VectorIndex covers the property —
    /// the typechecker has already enforced this is not the case
    /// for any `VECTOR_NEAR` / `VECTOR_SIM` call that reaches eval.
    pub(super) fn active_vector_index_for(&self, property_iri: &Iri) -> Option<&ActiveVectorIndex> {
        self.vector_indexes
            .iter()
            .find(|vi| vi.target_property == *property_iri)
    }

    /// Borrow of the layer the context was constructed against.
    /// Vector-retrieval needs it to issue probes (the text path
    /// also uses it inside `probe()`).
    pub(super) fn layer(&self) -> &'a Layer {
        self.layer
    }

    /// Look up the binding info for a property-bound variable.
    pub(super) fn binding_for(&self, var_name: &str) -> Option<&PropertyBindingInfo> {
        self.property_bindings.get(var_name)
    }

    /// Find the active TextIndex Resource targeting `property_iri`.
    /// Returns `None` if no active TextIndex covers the property —
    /// the typechecker has already enforced this is not the case
    /// for any `TEXT_MATCH` / `TEXT_SCORE` call that reaches eval.
    pub(super) fn active_index_for(&self, property_iri: &Iri) -> Option<&ActiveTextIndex> {
        self.text_indexes
            .iter()
            .find(|ti| ti.target_property == *property_iri)
    }

    /// Run `run_text_search` once per `(index_iri, query)` pair and
    /// memoise the result. Subsequent row evaluations share the
    /// cached probe.
    pub(super) fn probe(&self, active: &ActiveTextIndex, query: &str) -> Arc<TextSearchOutcome> {
        let key = (active.iri.clone(), query.to_string());
        if let Some(entry) = self.cache.borrow().get(&key) {
            return Arc::clone(entry);
        }
        let outcome = match registry::analyzer_for(&active.analyzer) {
            Some(analyzer) => match run_text_search(
                self.layer,
                self.layer.storage().text_index.as_ref(),
                &active.iri,
                analyzer.as_ref(),
                query,
            ) {
                Ok(hits) => TextSearchOutcome::Ok(hits),
                Err(e) => TextSearchOutcome::Err(e),
            },
            None => TextSearchOutcome::Err(TextSearchError::UnknownAnalyzer {
                analyzer_id: active.analyzer.clone(),
            }),
        };
        let arc = Arc::new(outcome);
        self.cache.borrow_mut().insert(key, Arc::clone(&arc));
        arc
    }
}

/// Walk a single MatchPart's patterns and add an entry per
/// property-bound variable. Subsequent rebindings of the same
/// variable name are ignored — the typechecker has already
/// validated each variable has a unique binding source in MATCH.
fn collect_property_bindings(
    part: &MatchPart,
    layer: &Layer,
    out: &mut BTreeMap<String, PropertyBindingInfo>,
) {
    for clause in &part.clauses {
        if let Clause::Pattern(p) = clause {
            collect_from_pattern(p, layer, out);
        }
    }
}

fn collect_from_pattern(
    pattern: &Pattern,
    layer: &Layer,
    out: &mut BTreeMap<String, PropertyBindingInfo>,
) {
    for pp in &pattern.properties {
        if let ValueOrVariable::Variable(var) = &pp.object {
            if let Some(iri) = resolve_property_name_in_layer(&pp.property, layer) {
                out.entry(var.name.clone()).or_insert(PropertyBindingInfo {
                    subject_variable: pattern.subject.name.clone(),
                    property_iri: iri,
                });
            }
        }
    }
}

/// Resolve a property `Name` to its IRI. Mirrors the typechecker's
/// resolver (`type_check::resolve_property_name`) — kept in this
/// module to avoid a cross-module dependency on a private item.
fn resolve_property_name_in_layer(name: &crate::query::ast::Name, layer: &Layer) -> Option<Iri> {
    use crate::ontology::resource::Value;
    use crate::ontology::well_known as wk;
    use crate::query::ast::Name;
    match name {
        Name::FullIri(iri) => Some(iri.clone()),
        Name::ShortName(s) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;
    use crate::query::execute;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a layer chain with: bootstrap + a child layer that
    /// declares (a) a string Property `urn:eigenius:test:body` with
    /// short_name `test_body`, (b) a TextIndex Resource targeting it,
    /// and (c) two indexable Document Resources whose body values
    /// have distinct token content.
    fn build_corpus() -> Arc<crate::layer::Layer> {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("corpus", Some(parent));

        // String-typed Property with short_name "test_body".
        let mut body_prop = Resource::new(iri("urn:eigenius:test:body"));
        body_prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        body_prop.set(iri(wk::SHORT_NAME), Value::String("test_body".into()));
        body_prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        b.add_resource(body_prop).unwrap();

        // TextIndex targeting test:body.
        let mut ti = Resource::new(iri("urn:eigenius:test:ti_body"));
        ti.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::TEXT_INDEX_CLASS))]),
        );
        ti.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri("urn:eigenius:test:body")),
        );
        ti.set(iri(wk::TEXT_ANALYZER), Value::String("en-stem-v1".into()));
        b.add_resource(ti).unwrap();

        // Two documents — one matches "wal truncation", one doesn't.
        let mut d1 = Resource::new(iri("urn:eigenius:test:doc1"));
        d1.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
        );
        d1.set(
            iri("urn:eigenius:test:body"),
            Value::String("WAL truncation under concurrent commit".into()),
        );
        b.add_resource(d1).unwrap();

        let mut d2 = Resource::new(iri("urn:eigenius:test:doc2"));
        d2.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
        );
        d2.set(
            iri("urn:eigenius:test:body"),
            Value::String("rolling back a partial commit".into()),
        );
        b.add_resource(d2).unwrap();

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    /// `execute()` returns a self-describing document: Property
    /// declarations + a row Class + a single ResultSet whose
    /// `urn:eigenius:query:rows` array holds the actual row
    /// Resources as embedded values. Pull those out.
    fn data_rows(document: &[Resource]) -> Vec<Resource> {
        let rows_prop = Iri::parse("urn:eigenius:query:rows").unwrap();
        for r in document {
            if let Some(Value::Array(arr)) = r.properties().get(&rows_prop) {
                return arr
                    .iter()
                    .filter_map(|v| match v {
                        Value::Embedded(res) => Some(*res.clone()),
                        _ => None,
                    })
                    .collect();
            }
        }
        Vec::new()
    }

    /// Pull the doc-subject IRI string out of one row.
    fn row_subject(row: &Resource) -> Option<String> {
        row.properties().iter().find_map(|(_, v)| {
            v.as_iri_str()
                .filter(|s| s.starts_with("urn:eigenius:test:doc"))
                .map(|s| s.to_string())
        })
    }

    fn row_subjects(document: &[Resource]) -> Vec<String> {
        let mut out: Vec<String> = data_rows(document).iter().filter_map(row_subject).collect();
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn match_finds_both_docs_without_filter() {
        // Sanity: the MATCH pattern alone, without retrieval, should
        // bind both Documents. Helps localise a failure to the
        // retrieval evaluator vs the candidate scan.
        let layer = build_corpus();
        let rows = execute(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?desc }
            RETURN [] { d: ?d }
            "#,
            &layer,
        )
        .expect("query should succeed");
        assert_eq!(
            row_subjects(&rows),
            vec![
                "urn:eigenius:test:doc1".to_string(),
                "urn:eigenius:test:doc2".to_string(),
            ],
            "MATCH alone should return both docs"
        );
    }

    #[test]
    fn text_match_filters_to_hits_only() {
        let layer = build_corpus();
        let rows = execute(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?desc }
            WHERE TEXT_MATCH(?desc, "wal truncation")
            RETURN [] { d: ?d }
            "#,
            &layer,
        )
        .expect("query should succeed");

        let subjects = row_subjects(&rows);
        assert_eq!(
            subjects,
            vec!["urn:eigenius:test:doc1".to_string()],
            "only doc1 contains both 'wal' and 'truncation'; got {subjects:?}"
        );
    }

    #[test]
    fn text_match_with_no_query_terms_returns_empty() {
        let layer = build_corpus();
        let rows = execute(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?desc }
            WHERE TEXT_MATCH(?desc, "elephant")
            RETURN [] { d: ?d }
            "#,
            &layer,
        )
        .expect("query should succeed");

        assert!(
            row_subjects(&rows).is_empty(),
            "no document contains 'elephant'"
        );
    }

    #[test]
    fn text_score_returns_positive_for_hits_zero_for_misses() {
        let layer = build_corpus();
        let rows = execute(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?desc }
            RETURN [] { d: ?d, s: TEXT_SCORE(?desc, "wal") }
            "#,
            &layer,
        )
        .expect("query should succeed");

        // Two rows (no WHERE filter); doc1's score should be > 0
        // and doc2's should be 0.0.
        let mut by_doc: BTreeMap<String, f64> = BTreeMap::new();
        for row in data_rows(&rows) {
            let mut doc: Option<String> = None;
            let mut score: Option<f64> = None;
            for val in row.properties().values() {
                if let Some(s) = val.as_iri_str() {
                    if s.starts_with("urn:eigenius:test:doc") {
                        doc = Some(s.to_string());
                    }
                }
                if let Value::Float(f) = val {
                    score = Some(*f);
                }
            }
            if let (Some(d), Some(s)) = (doc, score) {
                by_doc.insert(d, s);
            }
        }
        assert_eq!(
            by_doc.len(),
            2,
            "expected one row per document; got {by_doc:?}"
        );
        let d1 = by_doc
            .get("urn:eigenius:test:doc1")
            .copied()
            .unwrap_or(-1.0);
        let d2 = by_doc
            .get("urn:eigenius:test:doc2")
            .copied()
            .unwrap_or(-1.0);
        assert!(d1 > 0.0, "doc1 should have positive score; got {d1}");
        assert_eq!(d2, 0.0, "doc2 should have zero score; got {d2}");
    }

    #[test]
    fn text_match_and_text_score_share_one_probe() {
        // Verifies the per-(index, query) cache: both calls succeed
        // and produce coherent results without a re-probe. We can't
        // observe the probe count directly here, but a correctness
        // check that both `TEXT_MATCH` and `TEXT_SCORE` on the same
        // arguments agree on which row is the hit is the meaningful
        // black-box equivalent.
        let layer = build_corpus();
        let rows = execute(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?desc }
            WHERE TEXT_MATCH(?desc, "wal")
            RETURN [] { d: ?d, s: TEXT_SCORE(?desc, "wal") }
            "#,
            &layer,
        )
        .expect("query should succeed");
        let data = data_rows(&rows);
        assert_eq!(data.len(), 1, "exactly one document matches 'wal'");
        let row = &data[0];
        let mut saw_doc1 = false;
        let mut saw_positive_score = false;
        for val in row.properties().values() {
            if let Some(s) = val.as_iri_str() {
                if s == "urn:eigenius:test:doc1" {
                    saw_doc1 = true;
                }
            }
            if let Value::Float(f) = val {
                if *f > 0.0 {
                    saw_positive_score = true;
                }
            }
        }
        assert!(
            saw_doc1 && saw_positive_score,
            "row should reference doc1 with positive score"
        );
    }

    // ─── D43 §3.4 / §4.5 — VECTOR_NEAR / VECTOR_SIM e2e tests ─────

    /// Build a layer chain with a string Property + a `core:VectorIndex`
    /// Resource targeting it + N Documents whose body strings each
    /// embed under the dummy embedder to a distinct vector. Run the
    /// sweep so the vector segments are populated, returning the
    /// head + EmbedderRegistry the evaluator needs.
    fn build_vector_corpus(
        n_docs: usize,
    ) -> (
        Arc<crate::layer::Layer>,
        crate::program::embedder::EmbedderRegistry,
    ) {
        use crate::program::embedder::{DummyEmbedder, EmbedderRegistry};
        use crate::query::vector::indexing::sweep_layer_vectors;

        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("vec-corpus", Some(parent));

        let body_iri = "urn:eigenius:test:body";
        let model_iri = "urn:eigenius:embed:dummy:v1";

        let mut body_prop = Resource::new(iri(body_iri));
        body_prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        body_prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        b.add_resource(body_prop).unwrap();

        let mut vi = Resource::new(iri("urn:eigenius:test:vi_body"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        b.add_resource(vi).unwrap();

        for i in 0..n_docs {
            let mut d = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
            d.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
            );
            d.set(iri(body_iri), Value::String(format!("text {i}")));
            b.add_resource(d).unwrap();
        }

        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model_iri, 8)));
        sweep_layer_vectors(&layer, &reg, None).expect("sweep");
        (layer, reg)
    }

    #[test]
    fn vector_near_returns_top_k_subjects() {
        use crate::query;
        use crate::query::evaluate::FiberRuntime;

        let (layer, embedders) = build_vector_corpus(5);
        let runtime = FiberRuntime {
            embedders: Some(&embedders),
            ..FiberRuntime::default()
        };
        // EMBED("text 2") matches doc2's exact body, so doc2 should
        // be the top-1 hit. k=1 → only doc2 satisfies VECTOR_NEAR.
        let document = query::execute_with(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            WHERE VECTOR_NEAR(?vec, EMBED("text 2", "urn:eigenius:embed:dummy:v1"), 1)
            RETURN [] { d: ?d }
            "#,
            &layer,
            runtime,
        )
        .expect("query should succeed");
        let subjects = row_subjects(&document);
        assert_eq!(
            subjects,
            vec!["urn:eigenius:test:doc2".to_string()],
            "VECTOR_NEAR k=1 should return only doc2; got {subjects:?}"
        );
    }

    #[test]
    fn vector_sim_returns_one_for_exact_match() {
        use crate::query;
        use crate::query::evaluate::FiberRuntime;

        let (layer, embedders) = build_vector_corpus(3);
        let runtime = FiberRuntime {
            embedders: Some(&embedders),
            ..FiberRuntime::default()
        };
        // EMBED("text 0") is doc0's body verbatim → cosine similarity
        // should be 1.0 for doc0 and < 1.0 for the others.
        let document = query::execute_with(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            RETURN [] { d: ?d, s: VECTOR_SIM(?vec, EMBED("text 0", "urn:eigenius:embed:dummy:v1")) }
            "#,
            &layer,
            runtime,
        )
        .expect("query should succeed");
        let mut scores: BTreeMap<String, f64> = BTreeMap::new();
        for row in data_rows(&document) {
            let mut subj: Option<String> = None;
            let mut sc: Option<f64> = None;
            for val in row.properties().values() {
                if let Some(s) = val.as_iri_str() {
                    if s.starts_with("urn:eigenius:test:doc") {
                        subj = Some(s.to_string());
                    }
                }
                if let Value::Float(f) = val {
                    sc = Some(*f);
                }
            }
            if let (Some(d), Some(s)) = (subj, sc) {
                scores.insert(d, s);
            }
        }
        let doc0 = scores
            .get("urn:eigenius:test:doc0")
            .copied()
            .expect("doc0 score present");
        let doc1 = scores
            .get("urn:eigenius:test:doc1")
            .copied()
            .expect("doc1 score present");
        let doc2 = scores
            .get("urn:eigenius:test:doc2")
            .copied()
            .expect("doc2 score present");
        assert!(
            (doc0 - 1.0).abs() < 1e-5,
            "doc0 should score ~1.0; got {doc0}"
        );
        assert!(doc1 < doc0, "doc1 ({doc1}) should be below doc0 ({doc0})");
        assert!(doc2 < doc0, "doc2 ({doc2}) should be below doc0 ({doc0})");
    }

    #[test]
    fn vector_near_with_inferred_embed_model_works_end_to_end() {
        // The user writes `EMBED("text 0")` without an explicit
        // model IRI. The §4.4 inference pass copies the active
        // VectorIndex's model_iri into the call before evaluation;
        // the row matches doc0 exactly so the top-1 hit is doc0.
        use crate::query;
        use crate::query::evaluate::FiberRuntime;

        let (layer, embedders) = build_vector_corpus(3);
        let runtime = FiberRuntime {
            embedders: Some(&embedders),
            ..FiberRuntime::default()
        };
        let document = query::execute_with(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            WHERE VECTOR_NEAR(?vec, EMBED("text 0"), 1)
            RETURN [] { d: ?d }
            "#,
            &layer,
            runtime,
        )
        .expect("query should succeed under inferred EMBED model");
        let subjects = row_subjects(&document);
        assert_eq!(
            subjects,
            vec!["urn:eigenius:test:doc0".to_string()],
            "inferred-model EMBED should still rank doc0 first; got {subjects:?}"
        );
    }

    #[test]
    fn vector_near_filters_consistently_with_vector_sim() {
        use crate::query;
        use crate::query::evaluate::FiberRuntime;

        // VECTOR_NEAR(k=2) ∩ VECTOR_SIM(>0.99) under cosine: only
        // documents with cosine similarity within the top-2 *and*
        // numerically close to 1.0 should remain. The dummy
        // embedder is deterministic, so a query for `"text 1"`
        // returns exactly doc1 in the top-1 / top-2.
        let (layer, embedders) = build_vector_corpus(4);
        let runtime = FiberRuntime {
            embedders: Some(&embedders),
            ..FiberRuntime::default()
        };
        let document = query::execute_with(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            WHERE VECTOR_NEAR(?vec, EMBED("text 1", "urn:eigenius:embed:dummy:v1"), 2)
              AND VECTOR_SIM(?vec, EMBED("text 1", "urn:eigenius:embed:dummy:v1")) > 0.99
            RETURN [] { d: ?d }
            "#,
            &layer,
            runtime,
        )
        .expect("query should succeed");
        let subjects = row_subjects(&document);
        assert_eq!(
            subjects,
            vec!["urn:eigenius:test:doc1".to_string()],
            "exactly doc1 should survive both filters; got {subjects:?}"
        );
    }
}
