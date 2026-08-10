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

/// Statistics-institution-recomputed conclusions (DerivedEvidence witnesses
/// emitted out of band); validated in wrn_phase1_recompute.rs. See esl_against_pending.
const STATS_RECOMPUTED: &[&str] = &[
    "urn:eigenius:pub:wrn:concl_wrn_selective_recomputed",
    "urn:eigenius:pub:wrn:concl_refine_recomputed",
    "urn:eigenius:pub:wrn:concl_lineage_mutator_recomputed",
    "urn:eigenius:pub:wrn:concl_coloc_recomputed",
    "urn:eigenius:pub:wrn:concl_apop_shrna_recomputed",
    "urn:eigenius:pub:wrn:concl_hcr_recomputed",
    "urn:eigenius:pub:wrn:concl_recq_recomputed",
    "urn:eigenius:pub:wrn:concl_biomarker_recomputed",
    "urn:eigenius:pub:wrn:concl_p53_modulates",
    "urn:eigenius:pub:wrn:concl_val_recomputed",
    "urn:eigenius:pub:wrn:concl_cellcycle_recomputed",
    "urn:eigenius:pub:wrn:concl_apoptosis_recomputed",
    "urn:eigenius:pub:wrn:concl_mmr_restoration_recomputed",
    "urn:eigenius:pub:wrn:concl_rescue_wt_recomputed",
    "urn:eigenius:pub:wrn:concl_rescue_e84a_recomputed",
];

/// Conclusions whose witnesses come from the R runtime; covered live by the demo.
const R_RUNTIME: &[&str] = &[
    "urn:eigenius:pub:wrn:concl_vivo",
    "urn:eigenius:pub:wrn:concl_p53_activation",
    "urn:eigenius:pub:wrn:concl_dsb_foci",
    "urn:eigenius:pub:wrn:concl_dsb_gh2ax",
    "urn:eigenius:pub:wrn:concl_dsb_gh2ax_foci",
    "urn:eigenius:pub:wrn:concl_ddr_signaling",
    "urn:eigenius:pub:wrn:concl_paralog",
];

fn esl_against(source: &str, parent: &Arc<Layer>, name: &str) -> Arc<Layer> {
    esl_against_pending(source, parent, name, &[])
}

/// Build a layer from ESL, then replicate the live commit pipeline's AutoOnLoad
/// gate: every `reasoning:ReasoningSentence` this layer adds MUST validate to
/// `Holds`, else the live loader would reject it (and a downstream lemma citation
/// would be unsound). Panics on a non-`Holds` sentence unless its IRI is in
/// `pending` (witnesses produced out of band — R runtime / statistics institution).
fn esl_against_pending(
    source: &str,
    parent: &Arc<Layer>,
    name: &str,
    pending: &[&str],
) -> Arc<Layer> {
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
    for r in &resources {
        b.add_resource(r.clone()).unwrap();
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let ctx = ExecutionContext::new(
        layer.clone(),
        name,
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );
    let inst = ReasoningInstitution::new();
    let sentence_class = "urn:eigenius:reasoning:ReasoningSentence";
    for r in &resources {
        if !r.is_a().iter().any(|c| c.as_str() == sentence_class) {
            continue;
        }
        let iri = r.id().map(|i| i.as_str().to_string()).unwrap_or_default();
        let outcome =
            do_validate_justification(&inst, r, &ctx).expect("validate handler returns outcome");
        let ctor = outcome
            .output
            .get(&Iri::parse(wk::CTOR_NAME).unwrap())
            .and_then(Value::as_str)
            .unwrap_or("<none>");
        if ctor != wk::VERDICT_HOLDS && !pending.iter().any(|p| *p == iri) {
            let diag = outcome
                .output
                .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
                .and_then(Value::as_str)
                .unwrap_or("");
            panic!(
                "esl_against({name}): conclusion `{iri}` did not Hold (got {ctor}) — the live \
                 AutoOnLoad gate would reject this layer, so a downstream lemma citation of it \
                 would be unsound. diagnostic: {diag}\n  If its witness is produced out of band \
                 (R runtime / statistics institution AutoOnLoad, not run in this harness), add \
                 its IRI to `pending`."
            );
        }
    }
    layer
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
        include_str!("../../../experiments/publications/wrn-helicase/chain/01-onco.esl"),
        &harness,
        "onco",
    );
    // Literature layer: phase2/phase3 rules compose its warrants as premises.
    let literature = esl_against(
        include_str!("../../../experiments/publications/wrn-helicase/chain/02-literature.esl"),
        &onco,
        "wrn-literature",
    );
    // D54 two-phase load: plans (emitters) before conclusions (consumers).
    let recompute_plans = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl"
        ),
        &literature,
        "wrn-recompute-plans",
    );
    let recompute = esl_against_pending(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/04-phase1-recompute-conclusions.esl"
        ),
        &recompute_plans,
        "wrn-recompute-conclusions",
        STATS_RECOMPUTED,
    );
    let phase1 = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/05-phase1-discovery.esl"
        ),
        &recompute,
        "wrn-phase1",
    );
    let phase2 = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/07-phase2-validation.esl"
        ),
        &phase1,
        "wrn-phase2",
    );
    let phase3 = esl_against_pending(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/08-phase3-invivo-mechanism.esl"
        ),
        &phase2,
        "wrn-phase3",
        R_RUNTIME,
    );
    let phase5 = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/09-phase5-synthesis.esl"
        ),
        &phase3,
        "wrn-phase5",
    );
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
