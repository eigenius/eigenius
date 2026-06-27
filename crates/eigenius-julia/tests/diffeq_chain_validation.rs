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

//! Chain-validation test for the DiffEq institution declarations
//! (Phase 19g / D27 §4.5). Loads the ontology + institution
//! declarations onto a bootstrapped chain and asserts that the v1
//! resources (OdeProblem class + 6 properties; OdeSolution class +
//! 5 properties; Institution; RuntimeMethodSignature; QueryClass)
//! are all present and the validator accepts the layer cleanly.

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

mod common;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

#[test]
fn diffeq_ontology_and_institution_validate_cleanly() {
    // D41 Phase G migration: bootstrap with a memory-backed
    // `PersistentBackend` so layer commits go through
    // `commit_layer_default` — the D41 supported single-layer-commit
    // surface. `ExecutionContext::commit` was retired in D41 Phase G.
    let backend = Arc::new(MemoryPersistentBackend::new());
    let storage = LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
    let mut ctx = bootstrap_with_storage(storage).expect("bootstrap");

    // Env before institution (closed-world `requires_environment`).
    let diffeq_env = common::stub_env_json("urn:eigenius:diffeq:env:v1", "julia");
    for (label, json) in [
        ("diffeq_ontology", DIFFEQ_ONTOLOGY_JSON.to_string()),
        ("diffeq_env", diffeq_env),
        ("diffeq_institution", DIFFEQ_INSTITUTION_JSON.to_string()),
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
        "urn:eigenius:diffeq:OdeProblem",
        "urn:eigenius:diffeq:state_names",
        "urn:eigenius:diffeq:parameter_names",
        "urn:eigenius:diffeq:rhs",
        "urn:eigenius:diffeq:initial_conditions",
        "urn:eigenius:diffeq:parameters",
        "urn:eigenius:diffeq:time_span_start",
        "urn:eigenius:diffeq:time_span_end",
        "urn:eigenius:diffeq:RhsComponent",
        "urn:eigenius:diffeq:term",
        "urn:eigenius:diffeq:OdeSolution",
        "urn:eigenius:diffeq:problem",
        "urn:eigenius:diffeq:algorithm",
        "urn:eigenius:diffeq:abstol",
        "urn:eigenius:diffeq:reltol",
        "urn:eigenius:diffeq:final_state",
        "urn:eigenius:institutions:diffeq",
        "urn:eigenius:diffeq:signatures:validate_solution",
        "urn:eigenius:diffeq:query_classes:solution_validity",
        // Phase 19h.1 — Catalyst → DiffEq comorphism target side.
        "urn:eigenius:diffeq:formats:if_diffeq_problem",
    ] {
        assert!(
            ctx.head().resolve(&iri(required)).is_some(),
            "required DiffEq resource {required} must resolve on head layer"
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
