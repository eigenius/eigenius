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

//! WRN Phase-2 wet-lab validation chain — the Declared experimental-design
//! reasoning (the rescue / control logic), kernel-type-checked.
//!
//! Builds core → reflection → reasoning → statistics → bench-core → harness
//! → onco → wrn-phase1-recompute → wrn-phase1 → wrn-phase2, then runs
//! ValidateJustification on the four Phase-2 conclusions and asserts Holds:
//! - C-VAL `SelectiveViabilityDependence(WRN, MSI)`
//! - D-ONTARGET `OnTarget(WRN, MSI_viability)` (sgWRN-EIJ rescue logic)
//! - D-HELICASE `RequiresActivity(WRN, helicase)` (K577M fails to rescue)
//! - D-HELICASE `DispensableActivity(WRN, exonuclease)` (E84A rescues)
//!
//! Phase-2 statistics are linked-external (the authors' wet-lab assays,
//! recorded as bench:ToolArtifacts with ProgramTrace → IsDerivedAs); the
//! Declared rules lift those readouts into the conclusions. No statistics
//! institution is needed here — the warrants are Declared + linked-external.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::ReasoningInstitution;

fn esl_against(source: &str, parent: &Arc<Layer>, name: &str) -> Arc<Layer> {
    let resources = esl::compile_against_layer(source, parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile:\n{}",
            errs.into_iter()
                .map(|e| format!("  - {e:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent.clone()));
    for r in resources {
        b.add_resource(r).unwrap();
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn assert_holds(ctx: &ExecutionContext, inst: &ReasoningInstitution, iri: &str) {
    let sentence = (*ctx
        .resolve(&Iri::parse(iri).expect("sentence IRI"))
        .unwrap_or_else(|| panic!("sentence `{iri}` should be on the chain")))
    .clone();
    let outcome =
        do_validate_justification(inst, &sentence, ctx).expect("validate handler returns outcome");
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
        "expected Holds for `{iri}`; got {ctor}, diagnostic: {diagnostic:?}"
    );
}

#[test]
fn wrn_phase2_validation_chain_validates() {
    let core = {
        let mut b = LayerBuilder::new("core", None);
        for r in
            eigon_json::parse_document(include_str!("../../../ontologies/core/core-ontology.json"))
                .unwrap()
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reflection = {
        let mut b = LayerBuilder::new("reflection", Some(core));
        for src in [
            include_str!("../../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../../ontologies/institution/institution-ontology.json"),
        ] {
            for r in eigon_json::parse_document(src).unwrap() {
                b.add_resource(r).unwrap();
            }
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reasoning = {
        let mut b = LayerBuilder::new("reasoning", Some(reflection));
        for r in esl::compile(include_str!("../../../ontologies/reasoning/reasoning.esl"))
            .expect("reasoning.esl compiles")
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let statistics = esl_against(
        include_str!("../../../ontologies/statistics/statistics.esl"),
        &reasoning,
        "statistics",
    );
    let bench_core = esl_against(
        include_str!("../../../experiments/benchmark/base-ontologies/bench-core.esl"),
        &statistics,
        "bench-core",
    );
    let harness = esl_against(
        include_str!("../../../experiments/benchmark/harness-ontology.esl"),
        &bench_core,
        "harness",
    );
    let onco = esl_against(
        include_str!("../../../experiments/publications/wrn-helicase/onco.esl"),
        &harness,
        "onco",
    );
    let recompute = esl_against(
        include_str!("../../../experiments/publications/wrn-helicase/wrn-phase1-recompute.esl"),
        &onco,
        "wrn-recompute",
    );
    let phase1 = esl_against(
        include_str!("../../../experiments/publications/wrn-helicase/wrn-phase1.esl"),
        &recompute,
        "wrn-phase1",
    );
    let phase2 = esl_against(
        include_str!("../../../experiments/publications/wrn-helicase/wrn-phase2.esl"),
        &phase1,
        "wrn-phase2",
    );

    let _ = phase2.chain_witness_index();
    let ctx = ExecutionContext::new(
        phase2,
        "wrn-phase2",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );
    let inst = ReasoningInstitution::new();

    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_val");
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_ontarget");
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_helicase_required");
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_exo_dispensable");
}
