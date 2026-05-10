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

//! Phase 19a.8 — mirror regeneration determinism anchor.
//!
//! Verifies the chain-side guarantee D31 §3.3 makes: regenerating a
//! mirror against an unchanged ontology layer is byte-identical, and a
//! changed ontology produces a different mirror IRI. Both are
//! load-bearing for the chain's content-addressing of mirrors and for
//! the substrate's `MirrorVersionMismatch` failure path.
//!
//! Cheap test — no Docker, no buildah. Just chain → generator →
//! assert-on-hash. Runs in milliseconds.

use eigenius_julia::mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::mirror_generator::{MirrorGenerationRequest, MirrorGenerator};
use std::collections::HashMap;

const KINASE_ONTOLOGY_JSON: &str =
    include_str!("../../../ontologies/examples/kinase/kinase-ontology.json");

const COMPOUND_CLASS_IRI: &str = "urn:eigenius:demo:assay:Compound";
const TARGET_CLASS_IRI: &str = "urn:eigenius:demo:assay:Target";

/// Tiny in-memory `ChainAccessor` over the kinase ontology Resources.
/// Same shape the intervals e2e test uses.
struct KinaseChain {
    resources: HashMap<Iri, Resource>,
}

impl KinaseChain {
    fn from_ontology_json(json: &str) -> Self {
        let mut resources = HashMap::new();
        for r in eigon_json::parse_document(json).expect("kinase ontology must parse") {
            if let Some(id) = r.id() {
                resources.insert(id.clone(), r);
            }
        }
        Self { resources }
    }

    /// Return a copy with one property's `data_type` swapped from
    /// `string` to `float`. Used by the change-detection assertion —
    /// a property edit must surface as a different mirror IRI.
    fn with_compound_id_data_type_changed(&self) -> Self {
        let mut resources = self.resources.clone();
        let prop_iri = Iri::parse("urn:eigenius:demo:assay:compound_id").unwrap();
        let prop = resources
            .get(&prop_iri)
            .cloned()
            .expect("compound_id property present");
        let mut tweaked = prop.clone();
        tweaked.set(
            Iri::parse("urn:eigenius:core:data_type").unwrap(),
            Value::ResourceRef(Iri::parse("urn:eigenius:core:float").unwrap()),
        );
        resources.insert(prop_iri, tweaked);
        Self { resources }
    }
}

impl ChainAccessor for KinaseChain {
    fn resolve(&self, _claim_layer: &Iri, target: &Iri) -> Option<Resource> {
        self.resources.get(target).cloned()
    }
    fn is_ancestor_or_equal(&self, _: &Iri, _: &Iri) -> bool {
        true
    }
    fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
        true
    }
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// Generate the kinase mirror against the supplied chain and lift it
/// through `mirror_to_resource` so the test can read the
/// `library_content_hash` and IRI off the resulting Resource.
fn generate_kinase_mirror(chain: &KinaseChain) -> Resource {
    let g = JuliaMirrorGenerator::new();
    let layer = iri("urn:eigenius:test:kinase:layer");
    let seed = vec![iri(COMPOUND_CLASS_IRI), iri(TARGET_CLASS_IRI)];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain,
        })
        .expect("kinase mirror generation");
    // `generated_at` is fixed so the determinism check isn't
    // confounded by wall-clock drift. The mirror IRI itself derives
    // from `library_content_hash`, not from `generated_at`, so this
    // is belt-and-suspenders — but pinning keeps the equality check
    // honest if the IRI derivation rule ever changes.
    mirror_to_resource(&g, &out, &layer, Some("1970-01-01T00:00:00Z"))
}

fn read_library_content_hash(mirror: &Resource) -> &str {
    mirror
        .get(&iri("urn:eigenius:runtime:library_content_hash"))
        .and_then(Value::as_str)
        .expect("mirror must carry library_content_hash")
}

fn read_mirror_iri(mirror: &Resource) -> &str {
    mirror.id().expect("mirror has IRI").as_str()
}

#[test]
fn unchanged_ontology_produces_byte_identical_mirror() {
    let chain = KinaseChain::from_ontology_json(KINASE_ONTOLOGY_JSON);

    let m1 = generate_kinase_mirror(&chain);
    let m2 = generate_kinase_mirror(&chain);

    assert_eq!(
        read_library_content_hash(&m1),
        read_library_content_hash(&m2),
        "library_content_hash must be deterministic across regenerations"
    );
    assert_eq!(
        read_mirror_iri(&m1),
        read_mirror_iri(&m2),
        "content-addressed mirror IRI must match across regenerations"
    );
}

#[test]
fn modified_ontology_produces_different_mirror() {
    // The substrate's `MirrorVersionMismatch` failure path depends on
    // a class edit producing a fresh mirror IRI — otherwise a
    // re-dispatched invocation would silently use a stale mirror.
    let original = KinaseChain::from_ontology_json(KINASE_ONTOLOGY_JSON);
    let modified = original.with_compound_id_data_type_changed();

    let m_original = generate_kinase_mirror(&original);
    let m_modified = generate_kinase_mirror(&modified);

    assert_ne!(
        read_library_content_hash(&m_original),
        read_library_content_hash(&m_modified),
        "modifying a property's data_type must produce a different library_content_hash"
    );
    assert_ne!(
        read_mirror_iri(&m_original),
        read_mirror_iri(&m_modified),
        "modifying a property's data_type must produce a different mirror IRI"
    );
}
