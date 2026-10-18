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

use eigenius_kernel::bootstrap::bootstrap_with_storage;
use eigenius_kernel::lattice::commit_layer_default;
use eigenius_kernel::layer::LayerStorage;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::memory::MemoryPersistentBackend;
use eigenius_kernel::storage::PersistentBackend;
use std::sync::Arc;

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
// Phase 19f.1: symbolics ontology now references jump:VariableBound and
// jump:Constraint via SymbolicsToJuMPInput's framing properties (the
// Symbolics → JuMP comorphism), so the JuMP ontology must be on the
// chain before symbolics validates.
const JUMP_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/jump/declarations/jump-ontology.eigon.json");
const COMORPHISM_JSON: &str =
    include_str!("../../../julia/comorphisms/symbolics-to-intervals.eigon.json");

mod common;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

#[test]
fn symbolics_to_intervals_comorphism_validates_cleanly() {
    // D41 Phase G migration — see `diffeq_chain_validation.rs`.
    let backend = Arc::new(MemoryPersistentBackend::new());
    let storage = LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
    let mut ctx = bootstrap_with_storage(storage).expect("bootstrap");

    // Commit order matters:
    //   - intervals' BoundsRequest references SymbolicExpression as a
    //     `class_types`, so the symbolics ontology must be on the chain
    //     before intervals attempts to commit.
    //   - symbolics' SymbolicsToJuMPInput references jump:VariableBound
    //     and jump:Constraint via class_types, so jump ontology must be
    //     on the chain before symbolics validates.
    // Each institution's env committed before it (closed-world
    // `requires_environment`).
    let intervals_env = common::stub_env_json("urn:eigenius:intervals:env:v1", "julia");
    let symbolics_env = common::stub_env_json("urn:eigenius:symbolics:env:v1", "julia");
    for (label, json) in [
        ("jump_ontology", JUMP_ONTOLOGY_JSON.to_string()),
        ("symbolics_ontology", SYMBOLICS_ONTOLOGY_JSON.to_string()),
        ("intervals_ontology", INTERVALS_ONTOLOGY_JSON.to_string()),
        ("intervals_env", intervals_env),
        (
            "intervals_institution",
            INTERVALS_INSTITUTION_JSON.to_string(),
        ),
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

    // The Comorphism, both formats, the identity Lambda, the
    // IntervalFunction class, and the Symbolics deeper-surface
    // QueryClasses + their input classes must all be present on the
    // head layer.
    for required in [
        "urn:eigenius:intervals:IntervalFunction",
        "urn:eigenius:symbolics:formats:ef_symb_expr",
        "urn:eigenius:intervals:formats:if_intv_function",
        "urn:eigenius:comorphisms:symbolics_to_intervals:m_id_formula_term",
        "urn:eigenius:comorphisms:symbolics_to_intervals",
        // Phase 19d.3 — Symbolics deeper surface.
        "urn:eigenius:symbolics:SimplifyRequest",
        "urn:eigenius:symbolics:EquivalenceCheck",
        "urn:eigenius:symbolics:signatures:simplify_expression",
        "urn:eigenius:symbolics:signatures:check_equivalence",
        "urn:eigenius:symbolics:query_classes:qc_symb_simplify",
        "urn:eigenius:symbolics:query_classes:qc_symb_check_equivalence",
        // Phase 19d.4 — SatisfiesEquation chain claim + supporting
        // SymbolicEquation / VariableBinding shapes.
        "urn:eigenius:symbolics:SymbolicEquation",
        "urn:eigenius:symbolics:VariableBinding",
        "urn:eigenius:symbolics:SatisfiesEquation",
        "urn:eigenius:symbolics:signatures:validate_satisfies_equation",
        "urn:eigenius:symbolics:query_classes:satisfies_equation_validity",
        // Phase 19d.5 — Substitutes chain claim.
        "urn:eigenius:symbolics:Substitutes",
        "urn:eigenius:symbolics:signatures:validate_substitutes",
        "urn:eigenius:symbolics:query_classes:substitutes_validity",
        // Phase 19d.6 — SymbolicallyReducesTo + ReductionStrategy
        // (strategy-parametric reduction claim).
        "urn:eigenius:symbolics:ReductionStrategy",
        "urn:eigenius:symbolics:SymbolicallyReducesTo",
        "urn:eigenius:symbolics:signatures:validate_symbolically_reduces_to",
        "urn:eigenius:symbolics:query_classes:symbolically_reduces_to_validity",
    ] {
        assert!(
            ctx.head().resolve(&iri(required)).is_some(),
            "required resource {required} must resolve on head layer"
        );
    }

    // The validator must accept the whole chain without errors.
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
