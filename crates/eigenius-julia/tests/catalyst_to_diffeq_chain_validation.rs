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

//! Chain-validation test for the Catalyst → DiffEq Comorphism
//! (D27 §4.4.4 / D32 §6 / Phase 19h.1). Loads the four declaration
//! files (diffeq + catalyst ontologies and institution descriptors)
//! plus the comorphism declaration, commits them onto a bootstrapped
//! chain, and asserts that the whole layer validates without errors
//! and the new resources resolve.

use eigenius_kernel::bootstrap::bootstrap_with_storage;
use eigenius_kernel::lattice::commit_layer_default;
use eigenius_kernel::layer::LayerStorage;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::memory::MemoryPersistentBackend;
use eigenius_kernel::storage::PersistentBackend;
use std::sync::Arc;

const DIFFEQ_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/diffeq/declarations/diffeq-ontology.eigon.json");
const DIFFEQ_INSTITUTION_JSON: &str =
    include_str!("../../../julia/institutions/diffeq/declarations/diffeq-institution.eigon.json");
const CATALYST_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/catalyst/declarations/catalyst-ontology.eigon.json");
const CATALYST_INSTITUTION_JSON: &str = include_str!(
    "../../../julia/institutions/catalyst/declarations/catalyst-institution.eigon.json"
);
const COMORPHISM_JSON: &str =
    include_str!("../../../julia/comorphisms/catalyst-to-diffeq.eigon.json");

mod common;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

#[test]
fn catalyst_to_diffeq_comorphism_validates_cleanly() {
    // D41 Phase G migration — see `diffeq_chain_validation.rs` for
    // the canonical migration note.
    let backend = Arc::new(MemoryPersistentBackend::new());
    let storage = LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
    let mut ctx = bootstrap_with_storage(storage).expect("bootstrap");

    // Commit order: DiffEq ontology first because Catalyst's
    // institution declarations reference `diffeq:OdeProblem` (in
    // `payload_type` and `result_class`); the validator resolves
    // those references at commit time.
    // Each institution's env committed before it (closed-world
    // `requires_environment`).
    let diffeq_env = common::stub_env_json("urn:eigenius:diffeq:env:v1", "julia");
    let catalyst_env = common::stub_env_json("urn:eigenius:catalyst:env:v1", "julia");
    for (label, json) in [
        ("diffeq_ontology", DIFFEQ_ONTOLOGY_JSON.to_string()),
        ("diffeq_env", diffeq_env),
        ("diffeq_institution", DIFFEQ_INSTITUTION_JSON.to_string()),
        ("catalyst_ontology", CATALYST_ONTOLOGY_JSON.to_string()),
        ("catalyst_env", catalyst_env),
        (
            "catalyst_institution",
            CATALYST_INSTITUTION_JSON.to_string(),
        ),
        ("comorphism", COMORPHISM_JSON.to_string()),
    ] {
        for r in eigon_json::parse_document(&json).expect("parse") {
            ctx.add_resource(r).expect("add_resource");
        }
        let working = ctx.take_working(label).expect("take_working");
        let layer =
            commit_layer_default(working, ctx.storage().clone(), backend.as_ref()).expect("commit");
        ctx.advance_head(layer, label).expect("advance_head");
    }

    for required in [
        "urn:eigenius:catalyst:formats:ef_cat_to_ode_input",
        "urn:eigenius:diffeq:formats:if_diffeq_problem",
        "urn:eigenius:comorphisms:catalyst_to_diffeq:m_id_ode_problem",
        "urn:eigenius:comorphisms:catalyst_to_diffeq",
    ] {
        assert!(
            ctx.head().resolve(&iri(required)).is_some(),
            "required resource {required} must resolve on head layer"
        );
    }

    let validator = eigenius_kernel::validation::Validator::new(std::sync::Arc::clone(ctx.head()));
    let errors = validator.validate();
    assert!(
        errors.is_empty(),
        "chain must validate cleanly; got errors:\n{}",
        errors
            .iter()
            .map(|e| format!("  [{:?}] {} on {:?}", e.rule, e.message, e.resource_id))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
