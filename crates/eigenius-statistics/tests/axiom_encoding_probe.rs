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

//! Probe: does the ESL `type_expr` encoder produce the same D47 JSON for an
//! `axiom`-headed predicate application as the kernel verifier produces when
//! emitting a `StatisticalAnalysisResult.canonical_proposition`?
//!
//! Both ends of the D49 witness machinery must hash to the same value:
//!
//! - Producer: `crates/eigenius-statistics/src/validate.rs`'s
//!   `derive_canonical_proposition_singlesample` for `OneSidedWitnessed`
//!   emits `stats:lt(stats:mean_of(s), T)` as a D47 JSON tree using its
//!   own inline `encode_app` / `encode_const_ref` / `encode_lit_string` /
//!   `encode_lit_float` helpers.
//!
//! - Consumer: an author writes the same proposition inside a
//!   `reflection:canonical_proposition = type_expr(...)` slot on a
//!   `reflection:DeclaredResource` bridge. The ESL compiler walks the
//!   `TypeExpr::Ref { name = stats:lt, args = [...] }` tree, resolves
//!   each axiom reference against the chain layer, and emits a D47
//!   JSON tree via the kernel's shared D47 codec.
//!
//! If both encoders agree on the JSON shape, the witness index keyed
//! against the verdict's canonical_proposition will match the bridge's
//! antecedent — and the reasoning institution's `JustifiedBy.derived`
//! grounding ctor can synthesise the witness against the same hash.
//!
//! If they disagree, the bridge restructure cannot work without an ESL
//! parser change.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;

fn build_stats_layer() -> Arc<eigenius_kernel::layer::Layer> {
    let core_json = include_str!("../../../ontologies/core/core-ontology.json");
    let core_resources = eigon_json::parse_document(core_json).unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let reflection_json = include_str!("../../../ontologies/reflection/reflection-ontology.json");
    let reflection_resources = eigon_json::parse_document(reflection_json).unwrap();
    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for r in reflection_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let eigentt_json = include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json");
    for r in eigon_json::parse_document(eigentt_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let institution_json =
        include_str!("../../../ontologies/institution/institution-ontology.json");
    for r in eigon_json::parse_document(institution_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    let stats_source = include_str!("../../../ontologies/statistics/statistics.esl");
    let stats_resources = esl::compile_against_layer(stats_source, &reflection)
        .expect("statistics.esl compiles against reflection layer");
    let mut stats_builder = LayerBuilder::new("statistics", Some(reflection));
    for r in stats_resources {
        stats_builder.add_resource(r).unwrap();
    }
    Arc::new(stats_builder.build(LayerStorage::in_memory()))
}

/// Reproduce the kernel verifier's encoding for a OneSidedWitnessed
/// SingleSampleEstimate canonical_proposition.
///
/// Mirrors `derive_canonical_proposition_singlesample` +
/// `derive_factor_effect_proposition`-shaped helpers in
/// `crates/eigenius-statistics/src/validate.rs`, kept inline here so
/// the probe is self-contained — any drift in the verifier's
/// encoding belongs in a separate change.
fn verifier_emit_stats_lt_mean_of(sample_set_iri: &str, threshold: f64) -> serde_json::Value {
    let stats_lt_iri = "urn:eigenius:measurements:lt";
    let stats_mean_of_iri = "urn:eigenius:measurements:mean_of";
    let mean_of_s = encode_app(
        encode_const_ref(stats_mean_of_iri),
        encode_lit_string(sample_set_iri),
    );
    encode_app(
        encode_app(encode_const_ref(stats_lt_iri), mean_of_s),
        encode_lit_float(threshold),
    )
}

fn encode_const_ref(iri: &str) -> serde_json::Value {
    serde_json::json!({"ctor": "ConstRef", "args": [iri]})
}

fn encode_lit_string(s: &str) -> serde_json::Value {
    serde_json::json!({"ctor": "LitString", "args": [s]})
}

fn encode_lit_float(f: f64) -> serde_json::Value {
    serde_json::json!({"ctor": "LitFloat", "args": [f]})
}

fn encode_app(head: serde_json::Value, arg: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"ctor": "App", "args": [head, arg]})
}

/// Author the bridge proposition in ESL and recover its D47-encoded
/// JSON via the chain layer.
fn esl_canonical_proposition(
    stats_layer: &Arc<eigenius_kernel::layer::Layer>,
) -> serde_json::Value {
    let bridge_source = r#"
namespace core       = "urn:eigenius:core";
namespace reflection = "urn:eigenius:reflection";
namespace stats      = "urn:eigenius:measurements";
namespace probe      = "urn:eigenius:probe";

resource probe:bridge_proposition : reflection:DeclaredResource {
    reflection:declared_by = "probe:axiom-encoding";

    reflection:canonical_proposition = type_expr(
        stats:lt(
            stats:mean_of("urn:eigenius:probe:sample"),
            100.0
        )
    );
}
"#;
    let resources = esl::compile_against_layer(bridge_source, stats_layer)
        .unwrap_or_else(|errs| panic!("probe ESL failed to compile: {errs:?}"));
    let mut layer_builder = LayerBuilder::new("probe-layer", Some(Arc::clone(stats_layer)));
    for r in resources {
        layer_builder.add_resource(r).unwrap();
    }
    let layer = layer_builder.build(LayerStorage::in_memory());
    let bridge_iri = Iri::parse("urn:eigenius:probe:bridge_proposition").unwrap();
    let bridge = layer
        .resolve(&bridge_iri)
        .expect("bridge resource committed");
    let prop_iri = Iri::parse("urn:eigenius:reflection:canonical_proposition").unwrap();
    match bridge.get(&prop_iri) {
        Some(Value::Json(j)) => j.clone(),
        other => panic!("canonical_proposition is not Value::Json: {other:?}"),
    }
}

#[test]
fn axiom_predicate_application_round_trips_through_esl_and_kernel_emitter() {
    let stats_layer = build_stats_layer();
    let expected = verifier_emit_stats_lt_mean_of("urn:eigenius:probe:sample", 100.0);
    let actual = esl_canonical_proposition(&stats_layer);

    if expected != actual {
        panic!(
            "axiom-encoding probe FAILED — the ESL `type_expr` encoder and the kernel \
             verifier's `derive_canonical_proposition_singlesample` produce DIFFERENT \
             D47 JSON for the same proposition. The bridge-resource restructure cannot \
             work without aligning the two paths.\n\
             \n\
             expected (verifier-side, what the StatisticalAnalysisResult.canonical_proposition \
             gets when OneSidedWitnessed dispatch fires):\n{}\n\
             \n\
             actual (ESL-side, what `type_expr(stats:lt(stats:mean_of(\"...\"), 100.0))` \
             encodes to):\n{}\n",
            serde_json::to_string_pretty(&expected).unwrap(),
            serde_json::to_string_pretty(&actual).unwrap(),
        );
    }
}
