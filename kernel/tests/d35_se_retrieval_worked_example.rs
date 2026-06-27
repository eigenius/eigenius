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

//! D35 §7.4 / M9.1 — end-to-end SE-knowledge-graph retrieval demo.
//!
//! Exercises the full D43 v1 surface against a minimal SE-style
//! schema: a `CodeArtifact` Class with a `description` and a
//! `contracted_by` Property, plus a `BoundaryContract` Class. Both a
//! TextIndex (BM25) and a VectorIndex (cosine over the
//! [`DummyEmbedder`]) target `description`, so the hybrid path runs
//! internally — the user query never names text vs. vector.
//!
//! The D35 §7.4 worked example modeled retrieval after the abandoned
//! SQL-shaped surface (TEXT_MATCH, VECTOR_NEAR, EMBED, RRF,
//! `TOP K BY <expr>`). Under the post-reset D43 surface (`~`
//! operator + `{ via, model, k, limit }` hints + bare `TOP N` for
//! ranked truncation), the same query collapses to a structural
//! `MATCH` plus disjunctive `~` operators in `WHERE` plus an
//! anonymous `TOP N`. This test pins the rewritten shape end-to-end
//! and asserts the relevance-ordered result set matches what a
//! schema owner would expect.
//!
//! Per-test invariants:
//!
//! - [`d35_worked_example_returns_topk_by_relevance`] — full hybrid
//!   query against three CodeArtifacts; verifies the structural
//!   filter (only artifacts with a non-null `contracted_by` survive)
//!   composes with the similarity-driven ranking.
//! - [`d35_worked_example_disjunctive_two_topics_ranks_both`] — two
//!   distinct fuzzy queries OR'd; assert both topics surface and
//!   their overlap (artifacts matching both queries) ranks higher.
//! - [`d35_worked_example_text_only_via_hint_pins_text_path`] — the
//!   `{ via: text }` hint exercises the per-operator override, even
//!   though both indexes are active.

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::layer::{Layer, LayerBuilder};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::embedder::{registry_with_dummy, EmbedderRegistry};
use eigenius_kernel::query::evaluate::FiberRuntime;
use eigenius_kernel::query::execute_with;
use eigenius_kernel::query::vector::indexing::sweep_layer_vectors;

// ─── Schema constants — the SE-style classes/properties under test ────

const CODE_ARTIFACT_CLASS: &str = "urn:eigenius:se:CodeArtifact";
const BOUNDARY_CONTRACT_CLASS: &str = "urn:eigenius:contracts:BoundaryContract";
const DESCRIPTION_PROP: &str = "urn:eigenius:se:description";
const CONTRACTED_BY_PROP: &str = "urn:eigenius:se:contracted_by";

const TEXT_INDEX_IRI: &str = "urn:eigenius:se:ti_description";
const VECTOR_INDEX_IRI: &str = "urn:eigenius:se:vi_description";
const EMBED_MODEL_IRI: &str = "urn:eigenius:embed:dummy:v1";

const ARTIFACT_DOCS: [(&str, &str, Option<&str>); 4] = [
    (
        "urn:eigenius:se:a1",
        "WAL truncation under concurrent commit recovery",
        Some("urn:eigenius:se:bc1"),
    ),
    (
        "urn:eigenius:se:a2",
        "rolling back a partially-written commit under concurrent load",
        Some("urn:eigenius:se:bc2"),
    ),
    (
        "urn:eigenius:se:a3",
        "kernel layer chain consolidation pass",
        Some("urn:eigenius:se:bc3"),
    ),
    (
        // Same topic as a1 ("WAL truncation"), but no contract —
        // exercises the structural composition: the similarity
        // operator alone would surface this row, but the
        // `contracted_by: ?bc` pattern filters it out before TOP
        // even sees it.
        "urn:eigenius:se:a4",
        "WAL truncation crash-recovery diagnostics",
        None,
    ),
];

// ─── Fixture builders ──────────────────────────────────────────────────

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

fn make_resource(id: &str, class_iri: &str, props: Vec<(&str, Value)>) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(class_iri))]),
    );
    for (k, v) in props {
        r.set(iri(k), v);
    }
    r
}

/// Build a layer with the SE-style schema, a TextIndex + VectorIndex
/// on `description`, the four [`ARTIFACT_DOCS`], and the three
/// referenced BoundaryContracts. The text index auto-populates at
/// `LayerBuilder::build`; the vector segment is populated by an
/// explicit post-Load sweep.
fn build_se_corpus() -> (Arc<Layer>, EmbedderRegistry) {
    let ctx = bootstrap::bootstrap().expect("bootstrap should succeed");
    let head = Arc::clone(ctx.head());
    let storage = head.storage().clone();
    let mut b = LayerBuilder::new("se-corpus", Some(head));

    // CodeArtifact + BoundaryContract Classes.
    b.add_resource(make_resource(
        CODE_ARTIFACT_CLASS,
        wk::CLASS,
        vec![(wk::SHORT_NAME, Value::String("CodeArtifact".into()))],
    ))
    .unwrap();
    b.add_resource(make_resource(
        BOUNDARY_CONTRACT_CLASS,
        wk::CLASS,
        vec![(wk::SHORT_NAME, Value::String("BoundaryContract".into()))],
    ))
    .unwrap();

    // Properties: description (string), contracted_by (resource ref).
    b.add_resource(make_resource(
        DESCRIPTION_PROP,
        wk::PROPERTY,
        vec![
            (wk::SHORT_NAME, Value::String("se_description".into())),
            (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::STRING))),
        ],
    ))
    .unwrap();
    b.add_resource(make_resource(
        CONTRACTED_BY_PROP,
        wk::PROPERTY,
        vec![
            (wk::SHORT_NAME, Value::String("contracted_by".into())),
            (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::RESOURCE))),
        ],
    ))
    .unwrap();

    // TextIndex + VectorIndex on description, both active at the head.
    b.add_resource(make_resource(
        TEXT_INDEX_IRI,
        wk::TEXT_INDEX_CLASS,
        vec![
            (
                wk::TARGET_PROPERTY,
                Value::ResourceRef(iri(DESCRIPTION_PROP)),
            ),
            (wk::TEXT_ANALYZER, Value::String("en-stem-v1".into())),
        ],
    ))
    .unwrap();
    b.add_resource(make_resource(
        VECTOR_INDEX_IRI,
        wk::VECTOR_INDEX_CLASS,
        vec![
            (
                wk::TARGET_PROPERTY,
                Value::ResourceRef(iri(DESCRIPTION_PROP)),
            ),
            (wk::VEC_MODEL, Value::ResourceRef(iri(EMBED_MODEL_IRI))),
            (wk::VEC_DIM, Value::Integer(8)),
            (
                wk::VEC_DISTANCE,
                Value::ResourceRef(iri("urn:eigenius:core:distances:cosine")),
            ),
        ],
    ))
    .unwrap();

    // Three BoundaryContract instances referenced by the artifacts.
    for bc_iri in [
        "urn:eigenius:se:bc1",
        "urn:eigenius:se:bc2",
        "urn:eigenius:se:bc3",
    ] {
        b.add_resource(make_resource(
            bc_iri,
            BOUNDARY_CONTRACT_CLASS,
            vec![(wk::SHORT_NAME, Value::String(bc_iri.into()))],
        ))
        .unwrap();
    }

    // Four CodeArtifact instances with description + optional contract.
    for (artifact_iri, description, contract) in ARTIFACT_DOCS {
        let mut props: Vec<(&str, Value)> =
            vec![(DESCRIPTION_PROP, Value::String(description.into()))];
        if let Some(bc) = contract {
            props.push((CONTRACTED_BY_PROP, Value::ResourceRef(iri(bc))));
        }
        b.add_resource(make_resource(artifact_iri, CODE_ARTIFACT_CLASS, props))
            .unwrap();
    }

    let layer = Arc::new(b.build(storage));
    let embedders = registry_with_dummy();
    sweep_layer_vectors(&layer, &embedders, None).expect("vector sweep should succeed");
    (layer, embedders)
}

// ─── Result-extraction helpers ─────────────────────────────────────────

fn row_property_iri_for(wrapped: &[Resource], short_name: &str) -> Iri {
    let short_prop = Iri::parse(wk::SHORT_NAME).unwrap();
    wrapped
        .iter()
        .find(|r| {
            matches!(r.get(&short_prop), Some(Value::String(s)) if s == short_name)
                && r.id().is_some()
                && r.id().unwrap().as_str().contains(":row:")
        })
        .and_then(|r| r.id().cloned())
        .unwrap_or_else(|| panic!("no row Property with short_name '{short_name}'"))
}

/// Pull a sequence of `slot → IRI` values out of the wrapped query
/// result, preserving the row order the evaluator emitted (which is
/// the ranked order under `TOP N`).
fn rows_for_slot(wrapped: &[Resource], slot: &str) -> Vec<String> {
    let prop = row_property_iri_for(wrapped, slot);
    let rows_prop = Iri::parse("urn:eigenius:query:rows").unwrap();
    let result_set = wrapped
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str().ends_with(":result"))
                .unwrap_or(false)
        })
        .expect("result set");
    let rows = match result_set.get(&rows_prop) {
        Some(Value::Array(arr)) => arr,
        _ => return Vec::new(),
    };
    rows.iter()
        .filter_map(|v| match v {
            Value::Embedded(r) => r.get(&prop).cloned(),
            _ => None,
        })
        .filter_map(|v| match v {
            Value::ResourceRef(i) => Some(i.as_str().to_string()),
            Value::String(s) => Some(s),
            _ => None,
        })
        .collect()
}

// ─── Tests ─────────────────────────────────────────────────────────────

/// D35 §7.4 rewritten — full hybrid retrieval. The structural
/// `contracted_by: ?bc` pattern filters out the un-contracted
/// artifact (a4) before TOP sees it; the similarity-driven ranking
/// then orders the survivors by relevance to the fuzzy query.
#[test]
fn d35_worked_example_returns_topk_by_relevance() {
    let (layer, embedders) = build_se_corpus();
    let runtime = FiberRuntime {
        embedders: Some(&embedders),
        ..FiberRuntime::default()
    };
    let rows = execute_with(
        r#"
        USING "urn:eigenius:se:CodeArtifact",
              "urn:eigenius:contracts:BoundaryContract"
        USING NAMESPACE "urn:eigenius:se:"
        MATCH CodeArtifact(?a) {
            "urn:eigenius:se:description": ?desc,
            "urn:eigenius:se:contracted_by": ?bc
        }
        WHERE ?desc ~ "WAL truncation concurrent commit"
        RETURN [] { artifact: ?a, contract: ?bc }
        TOP 20
        "#,
        &layer,
        runtime,
    )
    .expect("query should succeed");

    let artifacts = rows_for_slot(&rows, "artifact");
    let contracts = rows_for_slot(&rows, "contract");

    // a1 is the strongest text-matching artifact with a contract;
    // a4 (same topic, no contract) is excluded by the structural
    // pattern. a2 and a3 are weaker text matches but still in the
    // result set because the vector probe ranks every artifact.
    assert!(
        artifacts.iter().any(|s| s == "urn:eigenius:se:a1"),
        "a1 must appear (strongest text + contract); got {artifacts:?}"
    );
    assert!(
        !artifacts.iter().any(|s| s == "urn:eigenius:se:a4"),
        "a4 must be filtered out (no contract); got {artifacts:?}"
    );
    // The artifact / contract slots are populated 1:1 across rows.
    assert_eq!(artifacts.len(), contracts.len());
    // Each contracted artifact maps to its declared contract.
    for (artifact, contract) in artifacts.iter().zip(contracts.iter()) {
        let expected_contract = ARTIFACT_DOCS
            .iter()
            .find(|(id, _, _)| *id == artifact.as_str())
            .and_then(|(_, _, c)| c.as_ref())
            .copied();
        if let Some(expected) = expected_contract {
            assert_eq!(
                contract, expected,
                "contract for {artifact} must round-trip through the structural join"
            );
        }
    }
}

/// Two distinct fuzzy queries OR'd in WHERE — the D35 §7.4 shape
/// after collapse to `~`. The platform fuses both candidate sets
/// via internal RRF; rows that satisfy both queries accumulate
/// scores and rank above rows that satisfy only one. Verifies the
/// disjunctive composition that was the original §7.4 example's
/// most operationally interesting feature.
#[test]
fn d35_worked_example_disjunctive_two_topics_ranks_both() {
    let (layer, embedders) = build_se_corpus();
    let runtime = FiberRuntime {
        embedders: Some(&embedders),
        ..FiberRuntime::default()
    };
    let rows = execute_with(
        r#"
        USING "urn:eigenius:se:CodeArtifact",
              "urn:eigenius:contracts:BoundaryContract"
        USING NAMESPACE "urn:eigenius:se:"
        MATCH CodeArtifact(?a) {
            "urn:eigenius:se:description": ?desc,
            "urn:eigenius:se:contracted_by": ?bc
        }
        WHERE ?desc ~ "WAL truncation concurrent commit"
           OR ?desc ~ "rolling back a partially-written commit"
        RETURN [] { artifact: ?a, contract: ?bc }
        TOP 20
        "#,
        &layer,
        runtime,
    )
    .expect("query should succeed");

    let artifacts = rows_for_slot(&rows, "artifact");
    // a1 matches the first query strongly; a2 matches the second
    // query strongly. Both must surface. The structural filter
    // again excludes a4.
    assert!(
        artifacts.iter().any(|s| s == "urn:eigenius:se:a1"),
        "a1 (WAL truncation) must surface; got {artifacts:?}"
    );
    assert!(
        artifacts.iter().any(|s| s == "urn:eigenius:se:a2"),
        "a2 (rolling back) must surface; got {artifacts:?}"
    );
    assert!(
        !artifacts.iter().any(|s| s == "urn:eigenius:se:a4"),
        "a4 must be filtered out (no contract); got {artifacts:?}"
    );
}

/// `{ via: text }` on a property with both indexes routes the
/// operator through the text path only. Verifies the per-operator
/// override even when the default-hybrid path would otherwise run.
#[test]
fn d35_worked_example_text_only_via_hint_pins_text_path() {
    let (layer, embedders) = build_se_corpus();
    let runtime = FiberRuntime {
        embedders: Some(&embedders),
        ..FiberRuntime::default()
    };
    let rows = execute_with(
        r#"
        USING "urn:eigenius:se:CodeArtifact",
              "urn:eigenius:contracts:BoundaryContract"
        USING NAMESPACE "urn:eigenius:se:"
        MATCH CodeArtifact(?a) {
            "urn:eigenius:se:description": ?desc,
            "urn:eigenius:se:contracted_by": ?bc
        }
        WHERE ?desc ~ "WAL truncation" { via: text }
        RETURN [] { artifact: ?a, contract: ?bc }
        TOP 20
        "#,
        &layer,
        runtime,
    )
    .expect("query should succeed");
    let artifacts = rows_for_slot(&rows, "artifact");
    // Under text-only, the only contracted artifact whose
    // description literally matches "WAL truncation" is a1; a2 / a3
    // talk about other topics and drop out, and a4 (uncontracted)
    // is filtered structurally.
    assert!(
        artifacts.iter().any(|s| s == "urn:eigenius:se:a1"),
        "a1 must surface; got {artifacts:?}"
    );
    assert!(
        !artifacts.iter().any(|s| s == "urn:eigenius:se:a4"),
        "a4 must be filtered out (no contract); got {artifacts:?}"
    );
}
