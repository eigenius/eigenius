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

//! Chain-validation test for the JuMP-HiGHS institution declarations
//! (Phase 19f / D27 §4.2). Loads the ontology + institution
//! declarations onto a bootstrapped chain and asserts that the v1
//! resources (OptimisationProblem class + 5 properties; VariableBound
//! class + 3 properties; Constraint class + 3 properties;
//! ConstraintRelation inductive with 3 ctors; OptimisesTo class +
//! 6 properties; Institution; 2 RuntimeMethodSignatures; 2
//! QueryClasses) are all present and the validator accepts the layer
//! cleanly.

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

mod common;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

#[test]
fn jump_ontology_and_highs_institution_validate_cleanly() {
    // D41 Phase G migration — see `diffeq_chain_validation.rs`.
    let backend = Arc::new(MemoryPersistentBackend::new());
    let storage = LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
    let mut ctx = bootstrap_with_storage(storage).expect("bootstrap");

    // Env before institution (closed-world `requires_environment`).
    let jump_env = common::stub_env_json("urn:eigenius:jump_highs:env:v1", "julia");
    for (label, json) in [
        ("jump_ontology", JUMP_ONTOLOGY_JSON.to_string()),
        ("jump_highs_env", jump_env),
        (
            "jump_highs_institution",
            JUMP_HIGHS_INSTITUTION_JSON.to_string(),
        ),
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
        // OptimisationProblem class + properties
        "urn:eigenius:jump:OptimisationProblem",
        "urn:eigenius:jump:variable_names",
        "urn:eigenius:jump:variable_bounds",
        "urn:eigenius:jump:objective",
        "urn:eigenius:jump:sense",
        "urn:eigenius:jump:constraints",
        // VariableBound class + properties
        "urn:eigenius:jump:VariableBound",
        "urn:eigenius:jump:variable_name",
        "urn:eigenius:jump:lower",
        "urn:eigenius:jump:upper",
        // Constraint class + properties
        "urn:eigenius:jump:Constraint",
        "urn:eigenius:jump:lhs",
        "urn:eigenius:jump:relation",
        "urn:eigenius:jump:rhs",
        // ConstraintRelation inductive
        "urn:eigenius:jump:ConstraintRelation",
        // OptimisesTo class + properties
        "urn:eigenius:jump:OptimisesTo",
        "urn:eigenius:jump:problem",
        "urn:eigenius:jump:termination_status",
        "urn:eigenius:jump:objective_value",
        "urn:eigenius:jump:variable_values",
        "urn:eigenius:jump:abstol",
        "urn:eigenius:jump:reltol",
        // Institution + signatures + query classes
        "urn:eigenius:institutions:jump_highs",
        "urn:eigenius:jump_highs:signatures:validate_optimum",
        "urn:eigenius:jump_highs:query_classes:optimum_validity",
        "urn:eigenius:jump_highs:signatures:solve_problem",
        "urn:eigenius:jump_highs:query_classes:qc_jump_solve",
    ] {
        assert!(
            ctx.head().resolve(&iri(required)).is_some(),
            "required JuMP-HiGHS resource {required} must resolve on head layer"
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
