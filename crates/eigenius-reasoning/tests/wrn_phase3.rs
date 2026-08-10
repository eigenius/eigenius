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

//! WRN Phase-3 in-vivo + mechanism chain — the Declared reasoning
//! (C911 seed-control logic, DSB→DDR mechanism, telomere-defect rejection),
//! kernel-type-checked.
//!
//! Builds core → … → onco → wrn-phase1-recompute-{plans,conclusions} →
//! wrn-phase1 → wrn-phase2
//! → wrn-phase3 and runs ValidateJustification on the five Phase-3
//! conclusions, asserting Holds:
//! - C-VIVO `InVivoDependence(WRN, MSI)`
//! - C-VIVO `OnTarget(WRN, xenograft_growth)` (C911 seed-control logic)
//! - C-MECH `CausesDSBs(WRN, MSI)`
//! - C-MECH `DSBDrivenLethality(WRN, MSI)`
//! - C-MECH `NotViaTelomereDefect(WRN, MSI)` (tested-and-rejected hypothesis)
//!
//! Phase-3 statistics are linked-external (xenograft lme4 LRT, DSB/IF foci,
//! GSEA). The one recomputable sub-claim — p53 modulates WRN dependence
//! (Wilcoxon 23 vs 13) — is kernel-recomputed in the statistics layer and
//! validated by `eigenius-statistics`'s `wrn_phase1_recompute` test instead.

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

/// Statistics-institution-recomputed conclusions whose DerivedEvidence witnesses
/// are emitted out of band (not in this reasoning harness); validated for real in
/// eigenius-statistics/tests/wrn_phase1_recompute.rs. See esl_against_pending.
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

/// Conclusions whose witnesses come from the R runtime (a DerivedResource a
/// wrapped-R program commits) — absent in-process; covered live by the demo.
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
/// `Holds`, else the live loader would reject the layer (so a downstream lemma
/// citation of it would be unsound). Panics on a non-`Holds` sentence unless its
/// IRI is in `pending` — exceptions whose witnesses are produced out of band (the
/// R runtime, or the statistics institution's AutoOnLoad, not run here). Without
/// this gate a layer could commit a never-validated conclusion and a later
/// sentence would trust it by IRI — the gap that let wrn_phase5 pass without
/// wrn-literature.
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

/// Builds the full WRN chain up to phase-3 in-process and returns a read-only
/// execution context plus a Reasoning institution to validate against.
fn build_phase3_ctx() -> (ExecutionContext, ReasoningInstitution) {
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
    // Literature layer: references + imported-claim warrants (reference:Citation),
    // composed as premises by the seed-control rule [16] etc.
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

    let ctx = ExecutionContext::new(
        phase3,
        "wrn-phase3",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );
    (ctx, ReasoningInstitution::new())
}

/// The mechanism chain validates hermetically: every conclusion here discharges
/// against witnesses emitted in-process (declarations and in-file program
/// traces), so no external runtime is required.
///
/// `concl_vivo_ontarget` discharges `DerivedEvidence(vivo_seed_control)`, whose
/// witness comes from `vivo_seed_control`'s in-file `ProgramTrace` — not from R.
#[test]
fn wrn_phase3_mechanism_chain_validates() {
    let (ctx, inst) = build_phase3_ctx();
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_vivo_ontarget");
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_dsb");
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_mech");
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_not_telomere");
}

/// `concl_vivo` (in-vivo WRN dependence) discharges
/// `DerivedEvidence(vivo_lme4:result)`, whose `IsDerivedAs` witness is produced
/// only by running the xenograft lme4 program through the R language runtime
/// (the Docker-hosted Bioconductor container). That witness cannot exist in this
/// in-process harness, so the assertion is necessarily absent here.
///
/// The R-backed leg is covered end-to-end by `demo/wrn-helicase/run.sh`, which
/// runs the lme4 program for real (p ≈ 0.048, `onco:InVivoDependence` Holds).
/// This test exists to document that boundary; do not "fix" it by hand-authoring
/// a `vivo_lme4:result` resource — that would fabricate the very derivation the
/// witness is meant to attest. Unignore it only when the harness can drive the
/// runtime substrate (see eigenius#85's two-phase load discussion for the
/// surrounding load-ordering context).
#[test]
#[ignore = "needs the R runtime to emit the vivo_lme4 witness; covered live by demo/wrn-helicase/run.sh"]
fn wrn_phase3_invivo_validates() {
    let (ctx, inst) = build_phase3_ctx();
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_vivo");
}

/// `concl_p53_activation` (C-MECH p53 arm) discharges
/// `DerivedEvidence(if_ed5:result)`, whose `IsDerivedAs` witness is produced only
/// by running the ED Fig 5 IF `emmeans` lsmeans program through the R language
/// runtime. Like `concl_vivo`, that witness cannot exist in this in-process
/// harness, so the assertion is necessarily absent here and the leg is covered
/// end-to-end by `demo/wrn-helicase/run.sh` (Step 3g: ActivatesP53Response Holds,
/// p-p53 +0.155 / p21 +0.310, p53-null KM12 control p21_null_logfc < 0). Do not
/// "fix" it by hand-authoring an `if_ed5:result` — that fabricates the very
/// derivation the witness attests.
#[test]
#[ignore = "needs the R runtime to emit the if_ed5 witness; covered live by demo/wrn-helicase/run.sh"]
fn wrn_phase3_p53_activation_validates() {
    let (ctx, inst) = build_phase3_ctx();
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_p53_activation");
}

/// `concl_dsb_foci` (the reproduced-external 53BP1 DSB-foci corroboration of
/// `concl_dsb`) discharges `DerivedEvidence(foci_dsb:result)`, whose `IsDerivedAs`
/// witness is produced only by running the ED Fig 6f/6h foci-count program through
/// the R language runtime. Like `concl_vivo`, that witness is absent in this
/// in-process harness; the leg is covered live by `demo/wrn-helicase/run.sh`
/// (Step 3h: CausesDSBs Holds, condition×MSI interaction +1.82, p ≈ 2.6e-142).
/// The linked-external `concl_dsb`/`concl_mech` chain stays verifiable in-process
/// (it cites the full-panel `mech_dsb` ToolArtifact), so this is purely additive.
#[test]
#[ignore = "needs the R runtime to emit the foci_dsb witness; covered live by demo/wrn-helicase/run.sh"]
fn wrn_phase3_dsb_foci_validates() {
    let (ctx, inst) = build_phase3_ctx();
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_dsb_foci");
}

/// `concl_dsb_gh2ax` (the reproduced-external γH2AX-intensity leg of `CausesDSBs`,
/// ED Fig 6c) discharges `DerivedEvidence(gh2ax:result)`, whose `IsDerivedAs`
/// witness is produced only by running the ED Fig 6c emmeans intensity program
/// through the R runtime. Absent in-process; covered live by
/// `demo/wrn-helicase/run.sh` (γH2AX intensity: log10 FC 0.055 ES2 / 0.144 OVK18,
/// MSI-vs-MSS contrast P < 2e-16 — the paper's published statistic).
#[test]
#[ignore = "needs the R runtime to emit the gh2ax witness; covered live by demo/wrn-helicase/run.sh"]
fn wrn_phase3_dsb_gh2ax_validates() {
    let (ctx, inst) = build_phase3_ctx();
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_dsb_gh2ax");
}

/// `concl_dsb_gh2ax_foci` (the reproduced-external γH2AX-foci leg of `CausesDSBs`,
/// ED Fig 6a/6d) discharges `DerivedEvidence(gh2ax_foci:result)`, whose witness is
/// produced only by the R runtime (foci interaction lm with pan-nuclear saturated
/// cells counted at a ceiling). Absent in-process; covered live by
/// `demo/wrn-helicase/run.sh` (interaction +7.3, foci ×3.4 MSI vs ×1.0 MSS).
#[test]
#[ignore = "needs the R runtime to emit the gh2ax_foci witness; covered live by demo/wrn-helicase/run.sh"]
fn wrn_phase3_dsb_gh2ax_foci_validates() {
    let (ctx, inst) = build_phase3_ctx();
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_dsb_gh2ax_foci");
}

/// `concl_ddr_signaling` (the reproduced-external DDR-signaling leg,
/// `ActivatesDSBResponse`, ED Fig 7b/7d) discharges `DerivedEvidence(patm:result)`,
/// whose witness is produced only by the R runtime (pATM(S1981) foci interaction
/// lm). Absent in-process; covered live by `demo/wrn-helicase/run.sh` (pATM foci
/// ×1.74 MSI vs ×1.11 MSS, interaction p≈0). This is the ATM-activation bridge the
/// paper draws from DSBs to p53.
#[test]
#[ignore = "needs the R runtime to emit the patm witness; covered live by demo/wrn-helicase/run.sh"]
fn wrn_phase3_ddr_signaling_validates() {
    let (ctx, inst) = build_phase3_ctx();
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_ddr_signaling");
}

/// `concl_paralog` (ED Fig 9a specificity) discharges
/// `DerivedEvidence(paralog_ctrl:result)`, whose `IsDerivedAs` witness is produced
/// only by running the paralogue co-loss program through the R language runtime
/// over the 1.6 GB DepMap rds (the large multi-schema D53 container path). Like
/// `concl_vivo`, that witness is absent in this in-process harness; covered live
/// by `demo/wrn-helicase/run.sh` (Step 3i: NotExplainedByParalogLoss Holds, MSI
/// β = −0.667 baseline / stays significant controlling for each paralogue's loss).
#[test]
#[ignore = "needs the R runtime + 1.6GB rds to emit the paralog_ctrl witness; covered live by demo/wrn-helicase/run.sh"]
fn wrn_phase3_paralog_validates() {
    let (ctx, inst) = build_phase3_ctx();
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_paralog");
}
