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

//! Chain-validation test for the Symbolics → IntervalArithmetic
//! Comorphism (D14 §5 / D32 §6.2). Loads the four declaration files —
//! intervals + symbolics ontologies and institution descriptors — plus
//! the cross-institution comorphism declaration, commits them onto a
//! bootstrapped chain, and asserts that the whole layer validates
//! without errors. The probe in `cross_institution_probe.rs`
//! demonstrates the *operational* identity-on-FormulaTerm story; this
//! test pins the *declarative* form: the chain itself accepts the
//! triple `(ef_symb_expr, m_id_formula_term, if_intv_function)` and
//! type-checks all the cross-references.

use eigenius_kernel::bootstrap::bootstrap;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;

const INTERVALS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/intervals/declarations/intervals-ontology.eigon.json"
);
const INTERVALS_INSTITUTION_JSON: &str = include_str!(
    "../../../julia/institutions/intervals/declarations/intervals-institution.eigon.json"
);
const SYMBOLICS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/symbolics/declarations/symbolics-ontology.eigon.json"
);
const SYMBOLICS_INSTITUTION_JSON: &str = include_str!(
    "../../../julia/institutions/symbolics/declarations/symbolics-institution.eigon.json"
);
const COMORPHISM_JSON: &str =
    include_str!("../../../julia/comorphisms/symbolics-to-intervals.eigon.json");

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

#[test]
fn symbolics_to_intervals_comorphism_validates_cleanly() {
    let mut ctx = bootstrap().expect("bootstrap");

    for (label, json) in [
        ("intervals_ontology", INTERVALS_ONTOLOGY_JSON),
        ("symbolics_ontology", SYMBOLICS_ONTOLOGY_JSON),
        ("intervals_institution", INTERVALS_INSTITUTION_JSON),
        ("symbolics_institution", SYMBOLICS_INSTITUTION_JSON),
        ("comorphism", COMORPHISM_JSON),
    ] {
        for r in eigon_json::parse_document(json).expect("parse") {
            ctx.add_resource(r).expect("add_resource");
        }
        ctx.commit(label).expect("commit");
    }

    // The Comorphism, both formats, the identity Lambda, and the
    // IntervalFunction class must all be present on the head layer.
    for required in [
        "urn:eigenius:intervals:IntervalFunction",
        "urn:eigenius:symbolics:formats:ef_symb_expr",
        "urn:eigenius:intervals:formats:if_intv_function",
        "urn:eigenius:comorphisms:symbolics_to_intervals:m_id_formula_term",
        "urn:eigenius:comorphisms:symbolics_to_intervals",
    ] {
        assert!(
            ctx.head().resolve(&iri(required)).is_some(),
            "required resource {required} must resolve on head layer"
        );
    }

    // The validator must accept the whole chain without errors.
    let validator = eigenius_kernel::validation::Validator::new(ctx.head());
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
