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

//! Chain-validation test for the Symbolics → JuMP Comorphism
//! (D27 §4.2 / D32 §6 / Phase 19f.1). Loads the four declaration
//! files (jump + symbolics ontologies and institution descriptors)
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

const JUMP_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/jump/declarations/jump-ontology.eigon.json");
const JUMP_HIGHS_INSTITUTION_JSON: &str =
    include_str!("../../../julia/institutions/jump/declarations/jump-highs-institution.eigon.json");
const SYMBOLICS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/symbolics/declarations/symbolics-ontology.eigon.json"
);
const SYMBOLICS_INSTITUTION_JSON: &str = include_str!(
    "../../../julia/institutions/symbolics/declarations/symbolics-institution.eigon.json"
);
const COMORPHISM_JSON: &str =
    include_str!("../../../julia/comorphisms/symbolics-to-jump.eigon.json");

mod common;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

#[test]
fn symbolics_to_jump_comorphism_validates_cleanly() {
    // D41 Phase G migration — see `diffeq_chain_validation.rs`.
    let backend = Arc::new(MemoryPersistentBackend::new());
    let storage = LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
    let mut ctx = bootstrap_with_storage(storage).expect("bootstrap");

    // Commit order: JuMP ontology first because Symbolics's
    // SymbolicsToJuMPInput references jump:VariableBound /
    // jump:Constraint (in framing properties) and jump:OptimisationProblem
    // (in qc_symb_to_jump's result_class + ef_symb_to_jump_input's
    // payload_type), so those resolve at commit time.
    // Each institution's env committed before it (closed-world
    // `requires_environment`).
    let jump_env = common::stub_env_json("urn:eigenius:jump_highs:env:v1", "julia");
    let symbolics_env = common::stub_env_json("urn:eigenius:symbolics:env:v1", "julia");
    for (label, json) in [
        ("jump_ontology", JUMP_ONTOLOGY_JSON.to_string()),
        ("jump_highs_env", jump_env),
        (
            "jump_highs_institution",
            JUMP_HIGHS_INSTITUTION_JSON.to_string(),
        ),
        ("symbolics_ontology", SYMBOLICS_ONTOLOGY_JSON.to_string()),
        ("symbolics_env", symbolics_env),
        (
            "symbolics_institution",
            SYMBOLICS_INSTITUTION_JSON.to_string(),
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
        // Symbolics-side composite + properties + signature + QC + ExportFormat.
        "urn:eigenius:symbolics:SymbolicsToJuMPInput",
        "urn:eigenius:symbolics:objective",
        "urn:eigenius:symbolics:variable_names",
        "urn:eigenius:symbolics:framing_variable_bounds",
        "urn:eigenius:symbolics:sense",
        "urn:eigenius:symbolics:framing_constraints",
        "urn:eigenius:symbolics:signatures:frame_as_optimisation_problem",
        "urn:eigenius:symbolics:query_classes:qc_symb_to_jump",
        "urn:eigenius:symbolics:formats:ef_symb_to_jump_input",
        // JuMP-side ImportFormat.
        "urn:eigenius:jump_highs:formats:if_jump_optimisation_problem",
        // Comorphism triple.
        "urn:eigenius:comorphisms:symbolics_to_jump:m_id_optimisation_problem",
        "urn:eigenius:comorphisms:symbolics_to_jump",
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
