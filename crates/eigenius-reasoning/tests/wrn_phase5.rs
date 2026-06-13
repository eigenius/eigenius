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

//! WRN Phase-5: C-MMR (causal dissection) and C-MAIN (the thesis).
//!
//! C-MAIN reaches `SyntheticLethal(WRN, MSI)` by modus ponens over a
//! Declared synthesis implication (`SVD → IVD → RA → DL → CD →
//! SyntheticLethal`) applied to the five findings (C-VAL, C-VIVO,
//! D-HELICASE, C-MECH, C-MMR). Each antecedent is discharged by its own
//! warrant inlined into the certificate — a proven sentence is the
//! antecedent of the implication, not an evidence atom. (The lemma-citation
//! mechanism that would let C-MAIN reference the phase conclusions directly
//! — D39's planned `Asserts` wrapper — is a separate follow-up.)

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

fn verdict(
    ctx: &ExecutionContext,
    inst: &ReasoningInstitution,
    iri: &str,
) -> (String, Option<String>) {
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
    (ctor, diagnostic)
}

fn build_ctx() -> ExecutionContext {
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
    let phase3 = esl_against(
        include_str!("../../../experiments/publications/wrn-helicase/wrn-phase3.esl"),
        &phase2,
        "wrn-phase3",
    );
    let phase5 = esl_against(
        include_str!("../../../experiments/publications/wrn-helicase/wrn-phase5.esl"),
        &phase3,
        "wrn-phase5",
    );
    let _ = phase5.chain_witness_index();
    ExecutionContext::new(
        phase5,
        "wrn-phase5",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn wrn_phase5_cmmr_and_cmain_validate() {
    let ctx = build_ctx();
    let inst = ReasoningInstitution::new();

    let (ctor, diag) = verdict(&ctx, &inst, "urn:eigenius:pub:wrn:concl_mmr");
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "C-MMR should Hold; diagnostic: {diag:?}"
    );

    // C-MAIN: the thesis, by modus ponens over the synthesis implication.
    let (ctor, diag) = verdict(&ctx, &inst, "urn:eigenius:pub:wrn:concl_main");
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "C-MAIN should Hold; diagnostic: {diag:?}"
    );
}
