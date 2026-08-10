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

//! D52 §8 end-to-end: statistics verdict → D39 reasoning composition.
//!
//! Layer stack: core → reflection → eigentt → institution → reasoning
//!   → statistics → ic50-fixture → d39-composition-fixture.
//!
//! Validates that:
//!  1. The IC50 confirmatory StatisticalAnalysisPlan's IsDerivedAs witness
//!     (admitted via its ProgramTrace, exercised in
//!     `ic50_measurement.rs`) is visible to the D39 reasoning
//!     validator when it processes a sentence using
//!     `DerivedEvidence(claim_iri)`.
//!  2. The reasoning sentence `App(SpecStr(DeclaredEvidence(rule),
//!     EIG_0291), DerivedEvidence(claim))` type-checks against
//!     `JustifiedBy(_, StrongInhibitor(EIG_0291))`.
//!
//! This is the proof point that D52 §8 actually works end-to-end —
//! the statistics institution produces a chain artifact that D39
//! reasoning consumes via the normal grounding-evidence path.

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

fn build_composition_chain() -> ExecutionContext {
    // Core + reflection + eigentt + institution.
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

    // Reasoning layer — provides JustifiedBy + JustificationTerm
    // inductives the certificate type-checks against.
    let reasoning_source = include_str!("../../../ontologies/reasoning/reasoning.esl");
    let reasoning_resources = esl::compile(reasoning_source).expect("reasoning.esl compiles");
    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(reflection));
    for r in reasoning_resources {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    // Statistics layer — provides StatisticalAnalysisPlan, SampleSet, axis
    // enums, and the PopulationLevel scope marker class the
    // composition fixture references.
    let stats_source = include_str!("../../../ontologies/statistics/statistics.esl");
    let stats_resources = esl::compile_against_layer(stats_source, &reasoning)
        .expect("statistics.esl compiles against reasoning layer");
    let mut stats_builder = LayerBuilder::new("statistics", Some(reasoning));
    for r in stats_resources {
        stats_builder.add_resource(r).unwrap();
    }
    let stats_layer = Arc::new(stats_builder.build(LayerStorage::in_memory()));

    // IC50 fixture layer — provides the screening + confirmatory
    // SampleSets + claims + traces. The confirmatory StatisticalAnalysisPlan
    // is the IsDerivedAs witness target for the DerivedEvidence used
    // by the composition fixture's ReasoningSentence.
    let ic50_source = include_str!("fixtures/ic50_measurement.esl");
    let ic50_resources =
        esl::compile_against_layer(ic50_source, &stats_layer).unwrap_or_else(|errs| {
            panic!(
                "ic50_measurement.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut ic50_builder = LayerBuilder::new("ic50-fixture", Some(stats_layer));
    for r in ic50_resources {
        ic50_builder.add_resource(r).unwrap();
    }
    let ic50_layer = Arc::new(ic50_builder.build(LayerStorage::in_memory()));

    // Composition fixture layer — adds the literature rule (universal)
    // + its DeclarationTrace + the ReasoningSentence that derives
    // StrongInhibitor(EIG_0291) via App(SpecStr, DerivedEvidence).
    let composition_source = include_str!("fixtures/d39_composition.esl");
    let composition_resources = esl::compile_against_layer(composition_source, &ic50_layer)
        .unwrap_or_else(|errs| {
            panic!(
                "d39_composition.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut composition_builder = LayerBuilder::new("d39-composition-fixture", Some(ic50_layer));
    for r in composition_resources {
        composition_builder.add_resource(r).unwrap();
    }
    let composition_layer = Arc::new(composition_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        composition_layer,
        "d39-composition-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn statistics_verdict_composes_with_universal_rule_via_d39() {
    let ctx = build_composition_chain();
    let sentence_iri =
        Iri::parse("urn:eigenius:demo:screen:concl_eig0291_strong").expect("sentence IRI");
    let sentence_arc = ctx
        .resolve(&sentence_iri)
        .unwrap_or_else(|| panic!("sentence `{sentence_iri}` should be on chain"));
    let sentence = (*sentence_arc).clone();

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

    // The IsDerivedAs witness for the confirmatory claim is admitted
    // by its ProgramTrace (see ic50_measurement.rs's
    // `claim_admits_is_derived_as_witness_via_program_trace` test).
    // The IsDeclaredAs witness for the universal rule is admitted by
    // its DeclarationTrace. SpecStr specializes the rule at
    // EIG_0291; App composes the specialized implication with the
    // derived evidence; the result type-checks against
    // `JustifiedBy(_, StrongInhibitor(EIG_0291))`. Holds.
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds — the universal rule applied to the confirmatory \
         IC50 claim should derive StrongInhibitor(EIG_0291); got {ctor}, \
         diagnostic: {diagnostic:?}"
    );
}
