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

//! Chain-validation test for the Catalyst institution declarations
//! (Phase 19h / D27 §4.4). Loads the ontology + institution
//! declarations onto a bootstrapped chain and asserts that:
//!
//! - all ten v1 resources are present on the head layer
//!   (ReactionNetwork class + 3 properties; ConservationLaw class +
//!   2 properties; Institution; RuntimeMethodSignature; QueryClass),
//! - the validator accepts the whole chain without errors,
//! - typed cross-references resolve (the QueryClass's `query_class`
//!   points at ConservationLaw, the signature's `input_types` reach
//!   ConservationLaw, the institution's `requires_environment` resolves
//!   to a committed `RuntimeEnvironment`).
//!
//! Closed-world reference integrity (D62 Rule 22) requires every typed
//! reference to resolve on the chain — including the institution's
//! `requires_environment`. The live-stack demo (`demo/catalyst/run.sh`)
//! commits the `RuntimeEnvironment` Resource (step 5) *before* installing
//! the institution (step 6); this test mirrors that ordering by committing
//! a stub env at `catalyst:env:v1` first. The stub carries only the
//! declaration-time `requires` fields — the deploy-time `image_digest`
//! (a `recommends` field produced by `env build`) is intentionally absent.

use eigenius_kernel::bootstrap::bootstrap_with_storage;
use eigenius_kernel::lattice::commit_layer_default;
use eigenius_kernel::layer::LayerStorage;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::memory::MemoryPersistentBackend;
use eigenius_kernel::storage::PersistentBackend;
use std::sync::Arc;

const CATALYST_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/catalyst/declarations/catalyst-ontology.eigon.json");
const CATALYST_INSTITUTION_JSON: &str = include_str!(
    "../../../julia/institutions/catalyst/declarations/catalyst-institution.eigon.json"
);
// Phase 19h.1: the Catalyst institution declarations now reference
// `diffeq:OdeProblem` as `payload_type` of `ef_cat_to_ode_input`
// and `result_class` of `qc_cat_to_ode`, so the DiffEq ontology
// must be on the chain before the Catalyst institution validates.
const DIFFEQ_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/diffeq/declarations/diffeq-ontology.eigon.json");

mod common;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

#[test]
fn catalyst_ontology_and_institution_validate_cleanly() {
    // D41 Phase G migration: bootstrap with a memory-backed
    // `PersistentBackend` so layer commits go through
    // `commit_layer_default` — the D41 supported single-layer-commit
    // surface. `ExecutionContext::commit` was retired in D41 Phase G.
    let backend = Arc::new(MemoryPersistentBackend::new());
    let storage = LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
    let mut ctx = bootstrap_with_storage(storage).expect("bootstrap");

    // Commit the env before the institution (mirrors the demo's
    // step-5-before-step-6 ordering) so `requires_environment` resolves.
    let catalyst_env = common::stub_env_json("urn:eigenius:catalyst:env:v1", "julia");
    for (label, json) in [
        ("diffeq_ontology", DIFFEQ_ONTOLOGY_JSON.to_string()),
        ("catalyst_ontology", CATALYST_ONTOLOGY_JSON.to_string()),
        ("catalyst_env", catalyst_env),
        (
            "catalyst_institution",
            CATALYST_INSTITUTION_JSON.to_string(),
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
        "urn:eigenius:catalyst:ReactionNetwork",
        "urn:eigenius:catalyst:network_source",
        "urn:eigenius:catalyst:species_declared",
        "urn:eigenius:catalyst:parameters_declared",
        "urn:eigenius:catalyst:ConservationLaw",
        "urn:eigenius:catalyst:network",
        "urn:eigenius:catalyst:coefficients",
        "urn:eigenius:institutions:catalyst",
        "urn:eigenius:catalyst:signatures:validate_conservation_law",
        "urn:eigenius:catalyst:query_classes:conservation_law_validity",
        // Phase 19h.1 — Catalyst → DiffEq comorphism source side.
        "urn:eigenius:catalyst:CatalystToOdeInput",
        "urn:eigenius:catalyst:initial_conditions",
        "urn:eigenius:catalyst:parameter_values",
        "urn:eigenius:catalyst:time_span_start",
        "urn:eigenius:catalyst:time_span_end",
        "urn:eigenius:catalyst:signatures:compile_to_ode",
        "urn:eigenius:catalyst:query_classes:qc_cat_to_ode",
        "urn:eigenius:catalyst:formats:ef_cat_to_ode_input",
    ] {
        assert!(
            ctx.head().resolve(&iri(required)).is_some(),
            "required Catalyst resource {required} must resolve on head layer"
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
