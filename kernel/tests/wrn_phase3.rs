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

/// Statistics-institution-recomputed conclusions. Their plan declarations and input
/// observations are committed by the statistics layer's own AutoOnLoad, which this
/// reasoning harness does not run; validated for real in
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

fn esl_against(source: &str, parent: &Arc<Layer>, name: &str) -> Arc<Layer> {
    esl_against_pending(source, parent, name, &[])
}

/// Build a layer from ESL, then replicate the live commit pipeline's AutoOnLoad
/// gate: every `justification:Conclusion` this layer adds MUST validate to
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
    let resources = esl::compile(source, parent).unwrap_or_else(|errs| {
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

    // STRUCTURAL validation, which `LayerBuilder::build` does not run and
    // `esl::compile_against_layer` does not either — it compiles, it does not
    // validate. Without this the chain is checked only for its conclusions'
    // certificates, so a resource-typed property pointing at a name nothing
    // resolves, a required property left off, or a class that no longer exists
    // all commit clean here and fail on a real load.
    //
    // `00-wrn-vocabulary.esl`'s own header records this costing two months:
    // 33 undeclared output keys meant the chain had not loaded since Rule 22
    // landed, and nothing noticed because these tests build layers this way.
    let structural = eigenius_kernel::validation::Validator::new(layer.clone()).validate();
    assert!(
        structural.is_empty(),
        "{name}: the layer must validate structurally — the live loader runs this and \
         these tests did not, which is how the chain silently stopped loading once before. \
         {} error(s): {:#?}",
        structural.len(),
        structural.iter().take(10).collect::<Vec<_>>()
    );

    // Every conclusion's judgement must type-check. This ran `do_validate_justification` and
    // read a `Verdict`; the check it was reading moved to commit at P2, so it now reads the
    // errors Rule 21 produces — same check, taken from where it lives. The `pending` skip is
    // preserved exactly: those conclusions' witnesses arrive out of band.
    let sentence_class = "urn:eigenius:justification:Conclusion";
    let all_errors = eigenius_kernel::validation::Validator::new(layer.clone()).validate();
    for r in &resources {
        if !r.is_a().iter().any(|c| c.as_str() == sentence_class) {
            continue;
        }
        let iri = r.id().map(|i| i.as_str().to_string()).unwrap_or_default();
        let diag = all_errors
            .iter()
            .filter(|e| e.resource_id.as_ref().is_some_and(|i| i.as_str() == iri))
            .map(|e| e.message.clone())
            .collect::<Vec<_>>()
            .join("\n");
        if !diag.is_empty() && !pending.iter().any(|p| *p == iri) {
            panic!(
                "esl_against({name}): conclusion `{iri}` did not type-check — the live \
                 AutoOnLoad gate would reject this layer, so a downstream lemma citation of it \
                 would be unsound. diagnostic: {diag}\n  If its witness is produced out of band \
                 (R runtime / statistics institution AutoOnLoad, not run in this harness), add \
                 its IRI to `pending`."
            );
        }
    }
    layer
}

/// The conclusion's judgement type-checks — no validation error names it.
///
/// Read a `Verdict` off `do_validate_justification` until P7. The check moved to commit at
/// P2, so this reads Rule 21's errors instead: it decodes the judgement, checks its `type` is
/// a type, and checks its `term` against it.
fn assert_holds(ctx: &ExecutionContext, iri: &str) {
    ctx.resolve(&Iri::parse(iri).expect("sentence IRI"))
        .unwrap_or_else(|| panic!("sentence `{iri}` should be on the chain"));
    let diagnostic = eigenius_kernel::validation::Validator::new(ctx.head().clone())
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref().is_some_and(|i| i.as_str() == iri))
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        diagnostic.is_empty(),
        "expected `{iri}` to type-check; got: {diagnostic}"
    );
}

/// Builds the full WRN chain up to phase-3 in-process and returns a read-only
/// execution context plus a Reasoning institution to validate against.
fn build_phase3_ctx() -> ExecutionContext {
    let core = {
        let mut b = LayerBuilder::new("core", None);
        for r in
            eigon_json::parse_document(include_str!("../../ontologies/core/core-ontology.json"))
                .unwrap()
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reflection = {
        let mut b = LayerBuilder::new("reflection", Some(core));
        for src in [
            include_str!("../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../ontologies/institution/institution-ontology.json"),
            include_str!("../../ontologies/ingest/ingest-ontology.json"),
        ] {
            for r in eigon_json::parse_document(src).unwrap() {
                b.add_resource(r).unwrap();
            }
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    // `prov` — the provenance axis, above reflection and below everything that
    // names an agent, a trace or an attribution.
    let prov = {
        let mut b = LayerBuilder::new("prov", Some(reflection));
        for r in esl::compile(
            include_str!("../../ontologies/prov/prov.esl"),
            &eigenius_kernel::layer::Layer::empty(),
        )
        .unwrap()
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reasoning = {
        let mut b = LayerBuilder::new("reasoning", Some(prov));
        for r in esl::compile(
            include_str!("../../ontologies/justification/justification.esl"),
            &eigenius_kernel::layer::Layer::empty(),
        )
        .expect("reasoning.esl compiles")
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let statistics = esl_against(
        include_str!("../../ontologies/statistics/statistics.esl"),
        &reasoning,
        "statistics",
    );
    // `reference` — the literature layer uses reference:Citation, and this chain
    // never carried the ontology declaring it. Nothing noticed because the layers
    // were built without structural validation.
    let reference = esl_against(
        include_str!("../../ontologies/reference/reference.esl"),
        &statistics,
        "reference",
    );
    let bench_core = esl_against(
        include_str!("../../experiments/benchmark/base-ontologies/bench-core.esl"),
        &reference,
        "bench-core",
    );
    let harness = esl_against(
        include_str!("../../experiments/benchmark/harness-ontology.esl"),
        &bench_core,
        "harness",
    );
    let onco = esl_against(
        include_str!("../../experiments/publications/wrn-helicase/chain/01-onco.esl"),
        &harness,
        "onco",
    );
    // Literature layer: references + imported-claim warrants (reference:Citation),
    // composed as premises by the seed-control rule [16] etc.
    let literature = esl_against(
        include_str!("../../experiments/publications/wrn-helicase/chain/02-literature.esl"),
        &onco,
        "wrn-literature",
    );
    // D54 two-phase load: plans (emitters) before conclusions (consumers).
    let recompute_plans = esl_against(
        include_str!(
            "../../experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl"
        ),
        &literature,
        "wrn-recompute-plans",
    );
    let recompute = esl_against_pending(
        include_str!(
            "../../experiments/publications/wrn-helicase/chain/04-phase1-recompute-conclusions.esl"
        ),
        &recompute_plans,
        "wrn-recompute-conclusions",
        STATS_RECOMPUTED,
    );
    let phase1 = esl_against(
        include_str!("../../experiments/publications/wrn-helicase/chain/05-phase1-discovery.esl"),
        &recompute,
        "wrn-phase1",
    );
    let phase2 = esl_against(
        include_str!("../../experiments/publications/wrn-helicase/chain/07-phase2-validation.esl"),
        &phase1,
        "wrn-phase2",
    );
    // 08a commits the out-of-band programs' inputs (as observations) and their
    // reproducibility claims (as declarations). It must precede 08, whose
    // conclusions cite both: `emit_from_trace` resolves a trace's target on the
    // chain, so the targets have to be in an ancestor layer.
    // The programs' real inputs: content-addressed PinnedExternalFiles with
    // reference + content_hash + media_type, declared beside each program.
    // 08a used to mint hash-less stand-ins whose IRIs were TRUNCATED PREFIXES of
    // these — `wrn:input_8d26fbb8aafb610a` for
    // `ingest:file:8d26fbb8aafb610a4952…` — so an observation named 16 hex digits
    // of a hash instead of pointing at the bytes.
    let program_inputs = {
        let mut b = LayerBuilder::new("wrn-program-inputs", Some(phase2.clone()));
        for src in [
            include_str!("../../experiments/publications/wrn-helicase/programs/invivo/xenograft-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/invivo/km12-competition-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/foci-ed6-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/gh2ax-foci-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/gh2ax-intensity-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/if-ed5-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/patm-foci-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/specificity/paralog-ed9a-input.json"),
        ] {
            for r in eigon_json::parse_document(src).unwrap() {
                b.add_resource(r).unwrap();
            }
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let provenance = esl_against(
        include_str!(
            "../../experiments/publications/wrn-helicase/chain/08a-program-provenance.esl"
        ),
        &program_inputs,
        "wrn-08a",
    );
    let phase3 = esl_against_pending(
        include_str!(
            "../../experiments/publications/wrn-helicase/chain/08-phase3-invivo-mechanism.esl"
        ),
        &provenance,
        "wrn-phase3",
        // No pending allowance. Under the three grounds a computed conclusion is
        // `App(Declared(plan), Observed(input))`, and BOTH witnesses are chain-resident
        // facts that 08a commits — so these type-check with no runtime at all. They used
        // to need one: `DerivedEvidence(<program>:result)` cited the program's OUTPUT
        // resource, which only the R runtime could commit, so type-checking the chain
        // required having run the analysis.
        &[],
    );

    ExecutionContext::new(
        phase3,
        "wrn-phase3",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

/// The mechanism chain validates hermetically: every conclusion here discharges
/// against witnesses emitted in-process, so no external runtime is required.
///
/// `concl_vivo_ontarget` discharges `Declared(vivo_seed_control)` — an author
/// asserting that the seed control ran and was inert, which is exactly what a
/// transcribed run establishes.
#[test]
fn wrn_phase3_mechanism_chain_validates() {
    let ctx = build_phase3_ctx();
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_vivo_ontarget");
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_dsb");
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_mech");
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_not_telomere");
}

/// `concl_vivo` (in-vivo WRN dependence) grounds on
/// `App(Declared(vivo_lme4_yields_result), Observed(vivo_xenograft_table))` — the
/// xenograft lme4 plan, declared to denote a function of its input, applied to that
/// input. Both witnesses are chain-resident (08a), so this type-checks with no
/// runtime.
///
/// It was `DerivedEvidence(vivo_lme4:result)`, which cited the program's OUTPUT
/// resource — a thing only the R runtime could commit — so the assertion was
/// `#[ignore]`d here and covered by `demo/wrn-helicase/run.sh`. Dropping the ignore
/// loosens nothing: the demo still runs lme4 for real (p ≈ 0.048,
/// `onco:InVivoDependence` Holds) and that is what checks the NUMBER. This test
/// checks that the chain's reasoning is well-typed, which never depended on the run
/// and only appeared to because the citation named a run artifact.
#[test]
fn wrn_phase3_invivo_validates() {
    let ctx = build_phase3_ctx();
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_vivo");
}

/// `concl_p53_activation` (C-MECH p53 arm) grounds on
/// `App(Declared(if_ed5_yields_result), Observed(input_8d26fbb8aafb610a))`, both
/// witnesses chain-resident (08a). The ED Fig 5 IF `emmeans` run is covered by
/// `demo/wrn-helicase/run.sh` (Step 3g: ActivatesP53Response Holds, p-p53 +0.155 /
/// p21 +0.310, p53-null KM12 control p21_null_logfc < 0).
#[test]
fn wrn_phase3_p53_activation_validates() {
    let ctx = build_phase3_ctx();
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_p53_activation");
}

/// `concl_dsb_foci` (the reproduced-external 53BP1 DSB-foci corroboration of
/// `concl_dsb`) grounds on
/// `App(Declared(foci_dsb_yields_result), Observed(input_1ba6dc6f78b10cee))`. The
/// ED Fig 6f/6h foci-count run is covered live by `demo/wrn-helicase/run.sh`
/// (Step 3h: CausesDSBs Holds, condition×MSI interaction +1.82, p ≈ 2.6e-142).
#[test]
fn wrn_phase3_dsb_foci_validates() {
    let ctx = build_phase3_ctx();
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_dsb_foci");
}

/// `concl_dsb_gh2ax` (the γH2AX-intensity leg of `CausesDSBs`, ED Fig 6c) grounds
/// on `App(Declared(gh2ax_yields_result), Observed(<the pinned intensity table>))`.
/// The emmeans run is covered live by `demo/wrn-helicase/run.sh` (γH2AX intensity:
/// log10 FC 0.055 ES2 / 0.144 OVK18, MSI-vs-MSS contrast P < 2e-16 — the paper's
/// published statistic).
#[test]
fn wrn_phase3_dsb_gh2ax_validates() {
    let ctx = build_phase3_ctx();
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_dsb_gh2ax");
}

/// `concl_dsb_gh2ax_foci` (the γH2AX-foci leg of `CausesDSBs`, ED Fig 6a/6d)
/// grounds on
/// `App(Declared(gh2ax_foci_yields_result), Observed(input_70abbad2f5319ae1))`. The
/// foci interaction lm (pan-nuclear saturated cells counted at a ceiling) is covered
/// live by `demo/wrn-helicase/run.sh` (interaction +7.3, foci ×3.4 MSI vs ×1.0 MSS).
#[test]
fn wrn_phase3_dsb_gh2ax_foci_validates() {
    let ctx = build_phase3_ctx();
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_dsb_gh2ax_foci");
}

/// `concl_ddr_signaling` (the DDR-signaling leg, `ActivatesDSBResponse`, ED Fig
/// 7b/7d) grounds on
/// `App(Declared(patm_yields_result), Observed(input_9a718df80087dece))`. The
/// pATM(S1981) foci interaction lm is covered live by `demo/wrn-helicase/run.sh`
/// (pATM foci ×1.74 MSI vs ×1.11 MSS, interaction p≈0). This is the ATM-activation
/// bridge the paper draws from DSBs to p53.
#[test]
fn wrn_phase3_ddr_signaling_validates() {
    let ctx = build_phase3_ctx();
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_ddr_signaling");
}

/// `concl_paralog` (ED Fig 9a specificity) grounds on
/// `App(Declared(paralog_ctrl_yields_result), Observed(input_14e82c398188b9f6))`.
/// The paralogue co-loss run over the 1.6 GB DepMap rds (the large multi-schema D53
/// container path) is covered live by `demo/wrn-helicase/run.sh` (Step 3i:
/// NotExplainedByParalogLoss Holds, MSI β = −0.667 baseline / stays significant
/// controlling for each paralogue's loss).
#[test]
fn wrn_phase3_paralog_validates() {
    let ctx = build_phase3_ctx();
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_paralog");
}
