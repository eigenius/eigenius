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

//! End-to-end D39 / D49 demo: a drug-screening scenario authored
//! entirely in ESL via the new `type_expr(...)` surface.
//!
//! The fixture at [`tests/fixtures/drug_screening.esl`](fixtures/drug_screening.esl)
//! commits four artifacts:
//!
//! 1. Domain vocabulary (`HasLowIC50`, `StrongInhibitor` predicates).
//! 2. A literature rule as a `DeclaredResource` + `DeclarationTrace`,
//!    with explicit `canonical_proposition` = `HasLowIC50(EIG_0291)
//!    -> StrongInhibitor(EIG_0291)`.
//! 3. A bench measurement as an `ObservedResource` + `ObservationTrace`,
//!    with explicit `canonical_proposition` = `HasLowIC50(EIG_0291)`.
//! 4. A `ReasoningSentence` claiming `StrongInhibitor(EIG_0291)`,
//!    justified by `App(DeclaredEvidence(rule), ObservedEvidence(obs))`,
//!    with a `JustifiedBy.app` certificate composing the two
//!    grounding constructors.
//!
//! This test compiles the fixture, builds the layer chain, walks the
//! D49 witness index, runs the D39 ValidateJustification handler, and
//! asserts `Verdict::Holds` — proving the full chain
//! authoring → witness emission → kernel synthesis → certificate
//! type-check pipeline works end-to-end through the surface syntax
//! a real chain author would use.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::ReasoningInstitution;

/// Stand up the standard reasoning chain (core → reflection → eigentt
/// → institution → reasoning) plus a user layer compiled from the
/// drug-screening fixture. The fixture's `type_expr(...)` certificates
/// reference reasoning-layer ctors (`app`, `declared`, `observed`,
/// `App`, `DeclaredEvidence`, `ObservedEvidence`), so the user layer
/// must be compiled with [`esl::compile_against_layer`] — that seeds
/// the compiler's ctor table from the parent chain.
fn build_drug_screening_chain() -> ExecutionContext {
    let core_json = include_str!("../../../ontologies/core/core-ontology.json");
    let core_resources = eigon_json::parse_document(core_json).unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let reflection_json = include_str!("../../../ontologies/reflection/reflection-ontology.json");
    let reflection_resources = eigon_json::parse_document(reflection_json).unwrap();
    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for r in reflection_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let eigentt_json = include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json");
    let eigentt_resources = eigon_json::parse_document(eigentt_json).unwrap();
    for r in eigentt_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let institution_json =
        include_str!("../../../ontologies/institution/institution-ontology.json");
    let institution_resources = eigon_json::parse_document(institution_json).unwrap();
    for r in institution_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    let reasoning_source = include_str!("../../../ontologies/reasoning/reasoning.esl");
    let reasoning_resources = esl::compile(reasoning_source).expect("reasoning.esl compiles");
    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(reflection));
    for r in reasoning_resources {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    // The fixture compiles AGAINST the reasoning layer so its
    // `type_expr(...)` bodies can reference reasoning-layer ctors
    // (`app`, `declared`, `observed`, `App`, `DeclaredEvidence`,
    // `ObservedEvidence`) by their short names. The ctor table seed
    // is what gh #75's IRI-discipline split makes possible — the
    // chain's `core:InductiveType` resources carry full IRIs, so
    // the seed walks the chain unambiguously.
    let fixture_source = include_str!("fixtures/drug_screening.esl");
    let fixture_resources =
        esl::compile_against_layer(fixture_source, &reasoning).unwrap_or_else(|errs| {
            panic!(
                "drug_screening.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut fixture_builder = LayerBuilder::new("drug-screening-demo", Some(reasoning));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    // Force the witness index to populate from the two trace resources
    // the fixture committed. After this, `IsDeclaredAs(rule_iri,
    // rule_prop)` and `IsObservedAs(obs_iri, obs_prop)` are admissible.
    let _ = fixture_layer.chain_witness_index();

    ExecutionContext::new(
        fixture_layer,
        "drug-screening-demo",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn drug_screening_scenario_validates_to_holds() {
    let ctx = build_drug_screening_chain();

    // Fetch the ReasoningSentence the fixture authored, by IRI.
    let sentence_iri =
        Iri::parse("urn:eigenius:demo:screen:concl_eig0291_strong").expect("sentence IRI");
    let sentence_arc = ctx
        .resolve(&sentence_iri)
        .unwrap_or_else(|| panic!("sentence `{sentence_iri}` should be on the chain"));
    let sentence = (*sentence_arc).clone();

    // Dispatch through the institution exactly as the AutoOnLoad gate
    // would at commit time.
    let inst = ReasoningInstitution::new();
    let outcome = do_validate_justification(&inst, &sentence, &ctx)
        .expect("validate handler returns an outcome");

    let ctor = outcome
        .output
        .get(&Iri::parse(wk::CTOR_NAME).unwrap())
        .and_then(Value::as_str)
        .expect("verdict carries ctor_name")
        .to_string();
    let diagnostic = outcome
        .output
        .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned);

    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds; got {ctor}, diagnostic: {diagnostic:?}"
    );
}
