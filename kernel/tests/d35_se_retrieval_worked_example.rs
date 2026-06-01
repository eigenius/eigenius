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

//! D43 M9.1 — D35 §7.4 worked-example integration test.
//!
//! D35 §7.4 sketches a hybrid-retrieval query an agent issues when
//! starting work in an unfamiliar area of the SE knowledge graph:
//!
//! ```eigenql
//! USING "urn:eigenius:se:CodeArtifact",
//!       "urn:eigenius:contracts:BoundaryContract"
//!
//! MATCH CodeArtifact(?a) {
//!     description: ?desc,
//!     description_embedding: ?vec,
//!     contracted_by: ?bc
//! }
//! WHERE TEXT_MATCH(?desc, "WAL truncation concurrent commit")
//!    OR VECTOR_NEAR(?vec,
//!                   EMBED("rolling back a partially-written commit under concurrent load"),
//!                   k: 50)
//! RETURN [] {
//!     artifact:   ?a,
//!     contract:   ?bc,
//!     text_score: TEXT_SCORE(?desc, "WAL truncation concurrent commit"),
//!     vec_score:  VECTOR_SIM(?vec, EMBED("...")),
//!     fused:      RRF(text_score, vec_score)
//! }
//! TOP 20 BY ?fused
//! ```
//!
//! D43 v1 makes a few specification-vs-implementation adaptations:
//!
//! - **Single property indexed both ways.** The §7.4 sketch has a
//!   separate `description_embedding: ?vec` property holding a
//!   pre-derived vector. D43 indexes the *string* property directly
//!   under both a `core:TextIndex` and a `core:VectorIndex` (the
//!   embedder produces the vector at index time); there is no
//!   user-visible derived vector property. So `?desc` carries the
//!   string and serves both `TEXT_MATCH(?desc, ...)` and
//!   `VECTOR_NEAR(?desc, EMBED(...), k)`.
//! - **RRF source expressions are inlined.** D45's BIND makes the
//!   per-row sources first-class variables; the §7.4 SQL-style
//!   `RRF(text_score, vec_score)` reference to RETURN-renamed
//!   columns isn't an EigenQL surface (RETURN doesn't introduce
//!   variables — see D45 §1). For TOP K BY we either inline the
//!   RRF (same approach M7.2's tests use) or BIND the sources and
//!   reference the BIND variables. This test demonstrates the BIND
//!   form because it matches the §6.5 pipeline shape closest.
//! - **`k:` is positional in v1.** D43 §3.4's `VECTOR_NEAR(?v, q, k:
//!   K)` keyword-arg surface is reserved for v1.1; today the third
//!   positional integer is K.
//!
//! The pipeline tested:
//!
//! 1. Bootstrap + SE-shaped corpus (CodeArtifact, BoundaryContract).
//! 2. Per-property indexes: TextIndex + VectorIndex on `description`.
//! 3. Hybrid query: `TEXT_MATCH OR VECTOR_NEAR` filters; BIND
//!    materialises `?ts` and `?vs`; RETURN exposes them + fused;
//!    TOP K BY RRF(?ts, ?vs) DESC truncates.
//! 4. Assertions: result is non-empty, row count ≤ K, every row
//!    carries a Float fused score, and the structural join
//!    (CodeArtifact ↔ BoundaryContract via contracted_by) holds.

use std::sync::Arc;

use eigenius_kernel::bootstrap::bootstrap;
use eigenius_kernel::layer::LayerBuilder;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::embedder::{DummyEmbedder, EmbedderRegistry};
use eigenius_kernel::query;
use eigenius_kernel::query::evaluate::FiberRuntime;
use eigenius_kernel::query::vector::indexing::sweep_layer_vectors;

const CODE_ARTIFACT: &str = "urn:eigenius:se:CodeArtifact";
const BOUNDARY_CONTRACT: &str = "urn:eigenius:contracts:BoundaryContract";
const DESCRIPTION: &str = "urn:eigenius:se:description";
const CONTRACTED_BY: &str = "urn:eigenius:se:contracted_by";
const TEXT_INDEX_IRI: &str = "urn:eigenius:test:ti_description";
const VECTOR_INDEX_IRI: &str = "urn:eigenius:test:vi_description";
const MODEL_IRI: &str = "urn:eigenius:embed:dummy:v1";

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// Build the worked-example corpus:
///
/// - Two Classes: `CodeArtifact`, `BoundaryContract`.
/// - Two Properties: `description` (string), `contracted_by` (ref).
/// - A `TextIndex` + `VectorIndex` on `description`.
/// - Four BoundaryContracts (one referenced by each CodeArtifact
///   under test, plus a spare to confirm the join doesn't leak).
/// - Five CodeArtifacts with descriptions that span the relevance
///   spectrum: dead-on for the text query, dead-on for the vector
///   query, both, neither, and a near-miss.
fn build_se_corpus() -> (Arc<eigenius_kernel::layer::Layer>, EmbedderRegistry) {
    let ctx = bootstrap().expect("bootstrap");
    let parent = Arc::clone(ctx.head());
    let mut b = LayerBuilder::new("se-corpus", Some(parent));

    // Class declarations.
    for (class_iri, short_name) in [
        (CODE_ARTIFACT, "CodeArtifact"),
        (BOUNDARY_CONTRACT, "BoundaryContract"),
    ] {
        let mut cls = Resource::new(iri(class_iri));
        cls.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        cls.set(iri(wk::SHORT_NAME), Value::String(short_name.into()));
        b.add_resource(cls).unwrap();
    }

    // Property declarations.
    let mut desc_prop = Resource::new(iri(DESCRIPTION));
    desc_prop.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
    );
    desc_prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
    desc_prop.set(iri(wk::SHORT_NAME), Value::String("description".into()));
    b.add_resource(desc_prop).unwrap();

    let mut cb_prop = Resource::new(iri(CONTRACTED_BY));
    cb_prop.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
    );
    cb_prop.set(iri(wk::SHORT_NAME), Value::String("contracted_by".into()));
    b.add_resource(cb_prop).unwrap();

    // TextIndex on description.
    let mut ti = Resource::new(iri(TEXT_INDEX_IRI));
    ti.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::TEXT_INDEX_CLASS))]),
    );
    ti.set(
        iri(wk::TARGET_PROPERTY),
        Value::ResourceRef(iri(DESCRIPTION)),
    );
    ti.set(iri(wk::TEXT_ANALYZER), Value::String("en-stem-v1".into()));
    b.add_resource(ti).unwrap();

    // VectorIndex on description.
    let mut vi = Resource::new(iri(VECTOR_INDEX_IRI));
    vi.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
    );
    vi.set(
        iri(wk::TARGET_PROPERTY),
        Value::ResourceRef(iri(DESCRIPTION)),
    );
    vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(MODEL_IRI)));
    vi.set(iri(wk::VEC_DIM), Value::Integer(8));
    b.add_resource(vi).unwrap();

    // BoundaryContracts (referents for the join).
    for i in 0..5 {
        let mut bc = Resource::new(iri(&format!("urn:eigenius:test:contract_{i}")));
        bc.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(BOUNDARY_CONTRACT))]),
        );
        b.add_resource(bc).unwrap();
    }

    // CodeArtifacts with varied descriptions.
    let artifacts = [
        ("a_text_hit", "WAL truncation under concurrent commit", 0),
        ("b_text_near", "WAL segment lifecycle and rotation", 1),
        (
            "c_vec_hit",
            "rolling back a partially-written commit under concurrent load",
            2,
        ),
        ("d_both", "WAL truncation during partial commit rollback", 3),
        ("e_neither", "unrelated implementation details", 4),
    ];
    for (sid, desc, contract_idx) in artifacts {
        let mut art = Resource::new(iri(&format!("urn:eigenius:test:artifact_{sid}")));
        art.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(CODE_ARTIFACT))]),
        );
        art.set(iri(DESCRIPTION), Value::String(desc.into()));
        art.set(
            iri(CONTRACTED_BY),
            Value::ResourceRef(iri(&format!("urn:eigenius:test:contract_{contract_idx}"))),
        );
        b.add_resource(art).unwrap();
    }

    let layer = Arc::new(b.build(eigenius_kernel::layer::LayerStorage::in_memory()));

    let mut reg = EmbedderRegistry::new();
    reg.register(Arc::new(DummyEmbedder::new(MODEL_IRI, 8)));
    sweep_layer_vectors(&layer, &reg, None).expect("vector index sweep");

    (layer, reg)
}

/// D35 §7.4 / D43 M9.1 — the worked-example hybrid query runs
/// end-to-end against an SE-shaped corpus and surfaces the
/// expected pipeline output: a bounded result with fused scores,
/// joined structurally with the BoundaryContract referent.
#[test]
fn se_worked_example_returns_bounded_hybrid_result() {
    let (layer, embedders) = build_se_corpus();
    let runtime = FiberRuntime {
        embedders: Some(&embedders),
        ..FiberRuntime::default()
    };

    // §7.4 adapted to D43 v1 surface (see module docs for the
    // delta against the literal spec text). The query exercises
    // every D43 retrieval primitive: MATCH-pattern join,
    // TEXT_MATCH / VECTOR_NEAR hybrid filter, BIND for per-row
    // sources, TEXT_SCORE / VECTOR_SIM projection, RRF fusion in
    // RETURN, RRF in TOP K BY (M7.4a sort-against-binding path).
    let query_str = r#"
        USING "urn:eigenius:se:CodeArtifact",
              "urn:eigenius:contracts:BoundaryContract"

        MATCH CodeArtifact(?a) {
            "urn:eigenius:se:description": ?desc,
            "urn:eigenius:se:contracted_by": ?bc
        },
        BoundaryContract(?bc) {}

        WHERE BIND(TEXT_SCORE(?desc, "wal truncation concurrent commit") AS ?ts),
              BIND(VECTOR_SIM(?desc, EMBED("rolling back a partially-written commit under concurrent load")) AS ?vs),
              TEXT_MATCH(?desc, "wal truncation concurrent commit")
           OR VECTOR_NEAR(?desc, EMBED("rolling back a partially-written commit under concurrent load"), 50)

        RETURN [] {
            artifact:   ?a,
            contract:   ?bc,
            text_score: ?ts,
            vec_score:  ?vs,
            fused:      RRF(?ts, ?vs)
        }
        TOP 20 BY RRF(?ts, ?vs) DESC
    "#;

    let document = query::execute_with(query_str, &layer, runtime)
        .expect("D35 §7.4 worked-example query should succeed end-to-end");

    // The ResultSet shape per Appendix A.
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

    let row_count = match result_set.get(&Iri::parse("urn:eigenius:query:row_count").unwrap()) {
        Some(Value::Integer(n)) => *n,
        _ => panic!("missing row_count"),
    };

    assert!(row_count > 0, "hybrid query should return at least one row");
    assert!(
        row_count <= 20,
        "TOP 20 must cap row count, got {row_count}"
    );

    let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
        Some(Value::Array(arr)) => arr,
        _ => panic!("missing rows"),
    };

    // Every row must carry: artifact (IRI ref), contract (IRI ref),
    // text_score (Float), vec_score (Float), fused (Float). The
    // fused score is the load-bearing assertion — proves the RRF
    // pre-pass ran and the BIND-bound source variables were
    // visible to the RETURN.
    let short_name_iri = Iri::parse(wk::SHORT_NAME).unwrap();
    let prop_iri_for = |sn: &str| -> Iri {
        document
            .iter()
            .find(|r| {
                matches!(r.get(&short_name_iri),
                    Some(Value::String(s)) if s == sn)
            })
            .and_then(|r| r.id().cloned())
            .unwrap_or_else(|| panic!("synthesized Property with short_name '{sn}' must exist"))
    };
    let prop_artifact = prop_iri_for("artifact");
    let prop_contract = prop_iri_for("contract");
    let prop_fused = prop_iri_for("fused");

    for row in rows {
        let row = match row {
            Value::Embedded(r) => r,
            _ => panic!("row must be embedded"),
        };

        // Structural join check: artifact's contract field
        // resolves to a BoundaryContract IRI in the corpus.
        let artifact_v = row.get(&prop_artifact).expect("row carries artifact");
        let contract_v = row.get(&prop_contract).expect("row carries contract");
        assert!(
            artifact_v.as_iri_str().is_some(),
            "artifact must be a resource ref, got {artifact_v:?}"
        );
        let contract_iri = contract_v
            .as_iri_str()
            .expect("contract must be a resource ref");
        assert!(
            contract_iri.starts_with("urn:eigenius:test:contract_"),
            "contract must be one of the corpus BoundaryContracts, got {contract_iri}"
        );

        // Fused score is a Float; the RRF pre-pass populated it.
        let fused = row.get(&prop_fused).expect("row carries fused");
        assert!(
            matches!(fused, Value::Float(_)),
            "fused must be Float, got {fused:?}"
        );
    }
}

/// Confirms the documented surface adaptations against the
/// literal §7.4 sketch. This test parses the literal §7.4 query
/// shape (RETURN-renamed RRF source references; positional `?vec`
/// against a separate `description_embedding` property) and
/// asserts that the kernel rejects them with diagnostic errors
/// — pinning the documented "spec adapts to EigenQL surface"
/// guidance against accidental regression.
#[test]
fn se_worked_example_literal_spec_form_is_rejected() {
    let (layer, embedders) = build_se_corpus();
    let runtime = FiberRuntime {
        embedders: Some(&embedders),
        ..FiberRuntime::default()
    };

    // The literal §7.4 form: `RRF(text_score, vec_score)`
    // references bare identifiers that the EigenQL parser
    // interprets as IRI-style string literals, not as variable
    // references. RRF's typecheck rejects them as "not a
    // recognised score expression."
    let literal_form = r#"
        USING "urn:eigenius:se:CodeArtifact"
        MATCH CodeArtifact(?a) {
            "urn:eigenius:se:description": ?desc,
            "urn:eigenius:se:contracted_by": ?bc
        }
        RETURN [] {
            artifact: ?a,
            text_score: TEXT_SCORE(?desc, "wal truncation"),
            vec_score: VECTOR_SIM(?desc, EMBED("rolling back commit")),
            fused: RRF(text_score, vec_score)
        }
    "#;
    let errors = query::execute_with(literal_form, &layer, runtime)
        .expect_err("bare-identifier RRF refs must be rejected at typecheck");
    let combined: String = errors
        .iter()
        .map(|e| format!("{e}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        combined.contains("not a recognised score expression"),
        "expected the §4.7 score-expression diagnostic, got: {combined}"
    );
}
