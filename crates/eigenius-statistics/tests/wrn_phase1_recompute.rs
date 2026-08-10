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

//! Two-institution composition end-to-end — the full WRN Phase-1
//! computational-discovery spine, kernel-recomputed.
//!
//! The **statistics** institution recomputes four `StatisticalAnalysisPlan`s
//! from chain-resident data and emits `StatisticalAnalysisResult`s, each
//! carrying a canonical proposition + an `IsDerivedAs` witness:
//! 1. C-WRN — MSI-vs-MSS WRN dependency (Wilcoxon, 37 vs 91) → `lt(mean_diff_of(s), 0)`.
//! 2. D-REFINE — WRN-dep ~ #MS-deletions (Spearman, 51 pairs) → `lt(spearman_rho(s), 0)`.
//! 3. D-RECQ — WRN MSI-vs-MSS in the RecQ cohort (Wilcoxon, 32 vs 413) → `lt(mean_diff_of(s), 0)`.
//! 4. D-BIOM — a threshold classifier (ClassificationAnalysisPlan) emits two results: `ge(ppv(s), 0.7)` + `ge(sensitivity(s), 0.9)`.
//!
//! The **reasoning** institution then type-checks every conclusion, each
//! certificate composing a declared statistical→domain bridge with
//! `DerivedEvidence` on the matching result(s):
//! - `C-WRN` → `SelectivelyEssential(WRN, MSI)`
//! - `D-REFINE` → `DependencyCorrelatesWithMutatorLoad(WRN, MSI)`
//! - `D-RECQ` → `OnlyMSISelectiveInFamily(WRN, RecQ_helicases)` — WRN derived + declared uniqueness over the kernel-computed n.s. p-values of the other RecQ helicases (a null is not derivable, so the negatives are an explicit judgment)
//! - `D-BIOM` → `StrongBiomarker(MSI, WRN_dependency)` (two derived facts)
//!
//! It also checks the **linked-external** two-screen `concl_wrn_selective`
//! (the paper's independent-replication argument; the limma genome-wide
//! ranking awaits Phase 2.5) and that the Phase-1 `bench:TaskOutput`
//! deliverable cites the four recomputed conclusions.
//!
//! The two institutions never call each other — they compose through the
//! shared chain witness index. This turns the institution-recomputable
//! tier from agent-attested (recorded ToolArtifacts) into kernel-recomputed.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::institution::runtime::Institution;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::ReasoningInstitution;
use eigenius_statistics::institution::iris;
use eigenius_statistics::StatisticsInstitution;

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

/// Dispatch the statistics institution on `plan_iri`, assert every
/// emitted derivation Holds, and finalize each (replicating the kernel's
/// `finalize_emitted_derivation`, which the raw `query()` skips) so the D49
/// witness emitter will admit its `IsDerivedAs` once committed. Returns
/// all finalized result resources — one for single-effect plans, two for
/// the classification plan (`:result:ppv` + `:result:sensitivity`).
fn recompute_finalized(
    stats_inst: &StatisticsInstitution,
    layer: &Arc<Layer>,
    plan_iri: &str,
) -> Vec<Resource> {
    let plan = (*layer
        .resolve(&Iri::parse(plan_iri).unwrap())
        .unwrap_or_else(|| panic!("plan `{plan_iri}` on chain")))
    .clone();
    let ctx = ExecutionContext::new(
        layer.clone(),
        "recompute",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );
    let outcome = stats_inst
        .query(
            &Iri::parse(iris::PROC_VALIDATE_ANALYSIS_PLAN).unwrap(),
            &plan,
            &ctx,
        )
        .expect("validate_analysis_plan outcome");
    assert!(
        !outcome.derivations.is_empty(),
        "plan `{plan_iri}` should emit at least one StatisticalAnalysisResult"
    );
    let is_a = Iri::parse(wk::IS_A).unwrap();
    outcome
        .derivations
        .iter()
        .map(|d| {
            let mut result = d.clone();
            let ctor = result
                .get(&Iri::parse(iris::PROP_VERDICT_CTOR).unwrap())
                .and_then(Value::as_str)
                .expect("verdict ctor");
            assert_eq!(
                ctor,
                wk::VERDICT_HOLDS,
                "recompute result of `{plan_iri}` should Hold"
            );
            let mut classes = match result.get(&is_a) {
                Some(Value::Array(a)) => a.clone(),
                Some(o) => vec![o.clone()],
                None => Vec::new(),
            };
            for marker in [wk::DERIVED_RESOURCE, wk::INSTITUTION_EMITTED_DERIVATION] {
                if !classes
                    .iter()
                    .any(|v| matches!(v, Value::String(s) if s == marker))
                {
                    classes.push(Value::String(marker.to_string()));
                }
            }
            result.set(is_a.clone(), Value::Array(classes));
            result
        })
        .collect()
}

/// Type-check a reasoning sentence against the (recomputed-result-bearing)
/// context and assert Holds.
fn assert_reasoning_holds(ctx: &ExecutionContext, sentence_iri: &str) {
    let inst = ReasoningInstitution::new();
    let sentence = (*ctx
        .resolve(&Iri::parse(sentence_iri).unwrap())
        .unwrap_or_else(|| panic!("sentence `{sentence_iri}` on chain")))
    .clone();
    let outcome = do_validate_justification(&inst, &sentence, ctx).expect("validate outcome");
    let ctor = outcome
        .output
        .get(&Iri::parse(wk::CTOR_NAME).unwrap())
        .and_then(Value::as_str)
        .expect("verdict ctor")
        .to_string();
    let diagnostic = outcome
        .output
        .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned);
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "`{sentence_iri}` should type-check against the kernel-recomputed result; \
         got {ctor}, diagnostic: {diagnostic:?}"
    );
}

#[test]
fn wrn_warrants_kernel_recomputed() {
    // core → reflection(+eigentt+institution) → reasoning → statistics →
    // bench-core → harness → onco → wrn-recompute → wrn-phase1.
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
    // D54 two-phase load: the plans (emitters: SampleSets +
    // StatisticalAnalysisPlans + ImpossibilityWitnesses + DeclaredResource
    // bridges) must load before the conclusions (consumers: the
    // `concl_*_recomputed` ReasoningSentences citing the emitted witnesses).
    let recompute_plans = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl"
        ),
        &onco,
        "wrn-recompute-plans",
    );
    let recompute = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/04-phase1-recompute-conclusions.esl"
        ),
        &recompute_plans,
        "wrn-recompute-conclusions",
    );
    let phase1 = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/05-phase1-discovery.esl"
        ),
        &recompute,
        "wrn-phase1",
    );

    // ── Step 1: the statistics institution recomputes every plan ──
    //   - MSI-vs-MSS Wilcoxon (37 vs 91)   → C-WRN selective essentiality
    //   - mutator-load Spearman (51 pairs) → D-REFINE correlation
    //   - RecQ-cohort Wilcoxon (32 vs 413) → D-RECQ family uniqueness
    //   - p53-stratified Wilcoxon (23 vs 13) → C-MECH p53 modulation
    //   - threshold classifier (PPV/sens)  → D-BIOM strong biomarker (2 results)
    //   - nested two-way ANOVA × 2 (KM12, OVK18) → C-VAL wet-lab viability
    let stats_inst = StatisticsInstitution::new();
    let mut emitted: Vec<Resource> = Vec::new();
    for plan_iri in [
        "urn:eigenius:pub:wrn:wrn_dep_plan",
        "urn:eigenius:pub:wrn:wrn_corr_plan",
        "urn:eigenius:pub:wrn:mutator_load_plan",
        "urn:eigenius:pub:wrn:coloc_plan",
        "urn:eigenius:pub:wrn:hcr_plan",
        "urn:eigenius:pub:wrn:apop_shrna_KM12_plan",
        "urn:eigenius:pub:wrn:wrn_recq_plan",
        "urn:eigenius:pub:wrn:p53_dep_plan",
        "urn:eigenius:pub:wrn:biomarker_plan",
        "urn:eigenius:pub:wrn:viab_KM12_plan",
        "urn:eigenius:pub:wrn:viab_OVK18_plan",
        // C-MECH recomputed DDR endpoints: cell-cycle arrest (ED Fig 4b) +
        // apoptosis (ED Fig 4c), three MSI lines each (nested two-way ANOVA).
        "urn:eigenius:pub:wrn:cc_KM12_plan",
        "urn:eigenius:pub:wrn:cc_SW48_plan",
        "urn:eigenius:pub:wrn:cc_OVK18_plan",
        "urn:eigenius:pub:wrn:apop_KM12_plan",
        "urn:eigenius:pub:wrn:apop_SW48_plan",
        "urn:eigenius:pub:wrn:apop_OVK18_plan",
        // C-MMR recomputed MMR-restoration (ED Fig 10c), three crossed two-way
        // ANOVA contrasts: rescue + two MLH1-KO re-sensitization controls.
        "urn:eigenius:pub:wrn:mmr_rescue_plan",
        "urn:eigenius:pub:wrn:mmr_resens1_plan",
        "urn:eigenius:pub:wrn:mmr_resens2_plan",
        // D-ONTARGET / D-HELICASE(exo) recomputed: the Fig 2c cDNA-rescue
        // two-sample t-tests (WT + E84A rescue sgWRN-EIJ).
        "urn:eigenius:pub:wrn:rescue_wt_plan",
        "urn:eigenius:pub:wrn:rescue_e84a_plan",
    ] {
        emitted.extend(recompute_finalized(&stats_inst, &phase1, plan_iri));
    }
    assert_eq!(
        emitted.len(),
        23,
        "21 single-effect results + the classification plan's 2 (ppv + sensitivity)"
    );

    // ── Step 2: commit every institution-emitted result onto the chain ──
    let with_results = {
        let mut b = LayerBuilder::new("wrn-stat-results", Some(phase1.clone()));
        for r in emitted {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let ctx = ExecutionContext::new(
        with_results,
        "wrn-stat-results",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );

    // ── Step 3: the reasoning institution type-checks every kernel-
    //            recomputed conclusion against the IsDerivedAs-bearing
    //            results, plus the linked-external two-screen C-WRN ──
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_wrn_selective_recomputed");
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_refine_recomputed");
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_recq_recomputed");
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_biomarker_recomputed");
    // D-LINEAGE recomputed (ED Fig 2b, Wilcoxon P = 1.7e-9): common MSI lineages
    // carry a higher mutator load than uncommon ones.
    assert_reasoning_holds(
        &ctx,
        "urn:eigenius:pub:wrn:concl_lineage_mutator_recomputed",
    );
    // C-LOC recomputed (ED Fig 8d, t-test): WRN delocalized from nucleolus in MSI.
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_coloc_recomputed");
    // C-MMR-FN recomputed (ED Fig 10a, t-test): MMR restoration restores repair.
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_hcr_recomputed");
    // C-APOP-shRNA recomputed (ED Fig 4d, t-test): apoptosis confirmed via shRNA.
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_apop_shrna_recomputed");
    // C-MECH recomputed sub-warrant: p53 status modulates WRN dependence.
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_p53_modulates");
    // C-VAL recomputed: wet-lab competition-assay nested ANOVA (KM12 + OVK18).
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_val_recomputed");
    // C-MECH recomputed DDR endpoints (ED Fig 4b/4c, three MSI lines each):
    // WRN depletion causes MSI-selective cell-cycle arrest + apoptosis.
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_cellcycle_recomputed");
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_apoptosis_recomputed");
    // C-MMR recomputed (ED Fig 10c, crossed two-way ANOVA, three contrasts):
    // restoring MMR partially rescues WRN dependence.
    assert_reasoning_holds(
        &ctx,
        "urn:eigenius:pub:wrn:concl_mmr_restoration_recomputed",
    );
    // D-ONTARGET / D-HELICASE(exo) recomputed (Fig 2c cDNA rescue):
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_rescue_wt_recomputed");
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_rescue_e84a_recomputed");
    // The paper's two-screen independent-replication argument (D-DIFF →
    // discovery rule), warranted by the linked-external ToolArtifacts.
    assert_reasoning_holds(&ctx, "urn:eigenius:pub:wrn:concl_wrn_selective");

    // ── Step 4: the Phase-1 deliverable resolves and cites the six
    //            kernel-recomputed discovery conclusions (the core four plus the
    //            lineage-restriction [ED2b] and TP53-modulation characterizations) ──
    let finding = ctx
        .resolve(&Iri::parse("urn:eigenius:pub:wrn:discovery_finding").unwrap())
        .expect("discovery_finding TaskOutput on chain");
    let n_chain = match finding.get(&Iri::parse("urn:eigenius:benchmark:reasoning_chain").unwrap())
    {
        Some(Value::Array(a)) => a.len(),
        other => panic!("reasoning_chain not an array: {other:?}"),
    };
    assert_eq!(n_chain, 6, "discovery_finding should cite 6 conclusions");
}
