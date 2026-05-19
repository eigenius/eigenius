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
//!   ConservationLaw, the institution's `requires_environment` is a
//!   placeholder env IRI that doesn't need to resolve at validation
//!   time — the env Resource itself is committed during the
//!   live-stack demo, not by this test).

use eigenius_kernel::bootstrap::bootstrap;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;

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

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

#[test]
fn catalyst_ontology_and_institution_validate_cleanly() {
    let mut ctx = bootstrap().expect("bootstrap");

    for (label, json) in [
        ("diffeq_ontology", DIFFEQ_ONTOLOGY_JSON),
        ("catalyst_ontology", CATALYST_ONTOLOGY_JSON),
        ("catalyst_institution", CATALYST_INSTITUTION_JSON),
    ] {
        for r in eigon_json::parse_document(json).expect("parse") {
            ctx.add_resource(r).expect("add_resource");
        }
        ctx.commit(label).expect("commit");
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
