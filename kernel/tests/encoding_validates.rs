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

//! The D62 `encoding` ontology (`ontologies/encoding/encoding.esl`) is **Expressible** and
//! **validates** over its documented parent chain (core → reflection/eigentt → logic →
//! lexicon-schema → reference). Until this test, the demo's `eig load` was the file's only
//! check — an edit could break it with nothing failing before the demo ran.
//!
//! Also pins the slice-5 selection vocabulary (d63-reading-selection.md): the
//! `enc:SelectionAuthority` enumeration is CLOSED — `enc:selected_by` carries `allows_only`
//! over exactly the three authority individuals (the `reflection:epistemic_status` pattern),
//! so a new authority is a deliberate vocabulary edit, never a silent mint.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::validation::Validator;

fn json_layer(name: &str, parent: Option<Arc<Layer>>, sources: &[&str]) -> Arc<Layer> {
    let mut b = LayerBuilder::new(name, parent);
    for src in sources {
        for r in eigon_json::parse_document(src).expect("ontology parses") {
            b.add_resource(r).expect("ontology resource adds");
        }
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn esl_layer(name: &str, src: &str, parent: Arc<Layer>) -> Arc<Layer> {
    let resources = esl::compile_against_layer(src, &parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile (not Expressible):\n{}",
            errs.into_iter()
                .map(|e| format!("  - {e:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in &resources {
        b.add_resource(r.clone())
            .unwrap_or_else(|e| panic!("{name}: add_resource failed: {e:?}"));
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// core → reflection(+eigentt, institution, ingest) → prov → logic → lexicon-schema → reference —
/// the chain `ontologies/encoding/encoding.esl`'s header documents it loads after.
fn parent_chain() -> Arc<Layer> {
    let core = json_layer(
        "core",
        None,
        &[include_str!("../../ontologies/core/core-ontology.json")],
    );
    let refl = json_layer(
        "reflection",
        Some(core),
        &[
            include_str!("../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../ontologies/institution/institution-ontology.json"),
            include_str!("../../ontologies/ingest/ingest-ontology.json"),
        ],
    );
    // `prov` sits above reflection and below everything that names an agent, a
    // trace or an attribution — which is most of the stack.
    let prov = esl_layer("prov", include_str!("../../ontologies/prov/prov.esl"), refl);
    let logic = esl_layer(
        "logic",
        include_str!("../../ontologies/logic/logic.esl"),
        prov,
    );
    let lexicon = esl_layer(
        "lexicon-schema",
        include_str!("../../ontologies/lexicon/lexicon-ontology.esl"),
        logic,
    );
    esl_layer(
        "reference",
        include_str!("../../ontologies/reference/reference.esl"),
        lexicon,
    )
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-formed IRI")
}

#[test]
fn encoding_ontology_is_expressible_and_validates() {
    let encoding = esl_layer(
        "encoding",
        include_str!("../../ontologies/encoding/encoding.esl"),
        parent_chain(),
    );
    let errors = Validator::new(Arc::clone(&encoding)).validate();
    assert!(
        errors.is_empty(),
        "encoding.esl validates with 0 errors, got {}:\n{errors:#?}",
        errors.len()
    );

    // The selection-authority enumeration is CLOSED: selected_by carries allows_only over
    // exactly the three authorities, and each authority individual is on the layer.
    let p = encoding
        .resolve(&iri("urn:eigenius:encoding:selected_by"))
        .expect("enc:selected_by is declared");
    let allows = p
        .get(&iri("urn:eigenius:core:allows_only"))
        .expect("selected_by carries allows_only (the closed-enumeration pattern)");
    let Value::Array(vals) = allows else {
        panic!("allows_only is an array, got {allows:?}");
    };
    let mut got: Vec<String> = vals
        .iter()
        // The IRI, not the variant that carries it — `Value::as_iri` parses. Nothing upgrades
        // a parsed `String` any more, because that upgrade never survived a storage round trip.
        .map(|v| {
            v.as_iri()
                .unwrap_or_else(|| panic!("allows_only entry should be an IRI, got {v:?}"))
                .as_str()
                .to_string()
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            "urn:eigenius:encoding:authority_pin".to_string(),
            "urn:eigenius:encoding:authority_ranker".to_string(),
            "urn:eigenius:encoding:authority_sole".to_string(),
        ],
        "the authority enumeration is exactly pin | ranker | sole"
    );
    for a in ["authority_pin", "authority_ranker", "authority_sole"] {
        assert!(
            encoding
                .resolve(&iri(&format!("urn:eigenius:encoding:{a}")))
                .is_some(),
            "enc:{a} is on the layer"
        );
    }

    // The AnaphorBinding vocabulary (D67 §3): the binding-authority enumeration is CLOSED the
    // same way, and the machine-readable antecedent properties are declared.
    let p = encoding
        .resolve(&iri("urn:eigenius:encoding:bound_by"))
        .expect("enc:bound_by is declared");
    let allows = p
        .get(&iri("urn:eigenius:core:allows_only"))
        .expect("bound_by carries allows_only (the closed-enumeration pattern)");
    let Value::Array(vals) = allows else {
        panic!("allows_only is an array, got {allows:?}");
    };
    let mut got: Vec<String> = vals
        .iter()
        // The IRI, not the variant that carries it — `Value::as_iri` parses. Nothing upgrades
        // a parsed `String` any more, because that upgrade never survived a storage round trip.
        .map(|v| {
            v.as_iri()
                .unwrap_or_else(|| panic!("allows_only entry should be an IRI, got {v:?}"))
                .as_str()
                .to_string()
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            "urn:eigenius:encoding:binding_proposer".to_string(),
            "urn:eigenius:encoding:binding_recency".to_string(),
            "urn:eigenius:encoding:binding_replay".to_string(),
        ],
        "the binding-authority enumeration is exactly recency | proposer | replay"
    );
    for a in [
        "AnaphorBinding",
        "binding_unit",
        "hole_var",
        "antecedent_surface",
        "antecedent_resource",
        "antecedent_term",
        "binding_recency",
        "binding_proposer",
        "binding_replay",
    ] {
        assert!(
            encoding
                .resolve(&iri(&format!("urn:eigenius:encoding:{a}")))
                .is_some(),
            "enc:{a} is on the layer"
        );
    }

    // The discourse-kind axis (D68 §2): the closed kind lattice under the enc:Claim root. Each
    // kind subclasses enc:Claim ON THIS LAYER; the per-kind lexicon alignment lives in the
    // separate chain-loaded claim-kind-alignment.esl (its targets are seeded sense classes this
    // chain does not have).
    for k in [
        "Claim",
        "Finding",
        "Observation",
        "Classification",
        "Hypothesis",
        "Suggestion",
        "Assertion",
    ] {
        assert!(
            encoding
                .resolve(&iri(&format!("urn:eigenius:encoding:{k}")))
                .is_some(),
            "enc:{k} is on the layer"
        );
    }
    for k in [
        "Finding",
        "Observation",
        "Classification",
        "Hypothesis",
        "Suggestion",
        "Assertion",
    ] {
        assert!(
            encoding.is_subclass_of(
                &iri(&format!("urn:eigenius:encoding:{k}")),
                &iri("urn:eigenius:encoding:Claim")
            ),
            "enc:{k} ⊑ enc:Claim"
        );
    }
}
