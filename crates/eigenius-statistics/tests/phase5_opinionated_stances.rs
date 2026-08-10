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

//! D52 Phase 5 — Opinionated-stance hardening end-to-end.
//!
//! Exercises:
//!  - §7.1 OneSidedWitnessed directionality with an ImpossibilityWitness:
//!    Holds path with halved p-value; Fails path when the witness is
//!    not committed on chain.
//!  - §7.2 Dual-verdict ESD outlier exclusion on SingleSampleEstimate:
//!    primary numerics are the with-exclusion verdict; diagnostic
//!    enumerates both branches.
//!  - §7.3 MethodComparisonAnalysisPlan + Passing-Bablok dispatch: Holds when
//!    methods agree (slope CI ∋ 1.0 AND intercept CI ∋ 0.0); Fails
//!    with the MethodComparisonDisagreement diagnostic on proportional
//!    bias.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::institution::runtime::Institution;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_statistics::institution::iris;
use eigenius_statistics::StatisticsInstitution;

fn build_phase5_chain() -> ExecutionContext {
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

    let stats_source = include_str!("../../../ontologies/statistics/statistics.esl");
    let stats_resources = esl::compile_against_layer(stats_source, &reflection)
        .expect("statistics.esl compiles against reflection layer");
    let mut stats_builder = LayerBuilder::new("statistics", Some(reflection));
    for r in stats_resources {
        stats_builder.add_resource(r).unwrap();
    }
    let stats_layer = Arc::new(stats_builder.build(LayerStorage::in_memory()));

    let fixture_source = include_str!("fixtures/phase5_opinionated_stances.esl");
    let fixture_resources = esl::compile_against_layer(fixture_source, &stats_layer)
        .unwrap_or_else(|errs| {
            panic!(
                "phase5_opinionated_stances.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut fixture_builder = LayerBuilder::new("phase5-fixture", Some(stats_layer));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        "phase5-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

fn validate_claim(ctx: &ExecutionContext, claim_iri: &str) -> (String, Option<String>) {
    let iri = Iri::parse(claim_iri).expect("claim IRI parses");
    let claim_arc = ctx
        .resolve(&iri)
        .unwrap_or_else(|| panic!("claim `{claim_iri}` should be on chain"));
    let claim = (*claim_arc).clone();
    let inst = StatisticsInstitution::new();
    let proc_iri = Iri::parse(iris::PROC_VALIDATE_ANALYSIS_PLAN).expect("proc IRI");
    let outcome = inst
        .query(&proc_iri, &claim, ctx)
        .expect("validate handler returns an outcome");
    // Composite (ctor, diagnostic) — when the SAP ran, both come from
    // the per-effect StatisticalAnalysisResult derivation (the verdict_ctor +
    // its diagnostic). When the SAP couldn't run (structural Fails),
    // they come from the gate verdict — no derivation was emitted.
    let diag_iri = Iri::parse("urn:eigenius:institution:diagnostic").unwrap();
    match outcome.derivations.first() {
        Some(result) => {
            let ctor = result
                .get(&Iri::parse(iris::PROP_VERDICT_CTOR).unwrap())
                .and_then(Value::as_str)
                .expect("StatisticalAnalysisResult carries verdict_ctor")
                .to_string();
            let diagnostic = result
                .get(&diag_iri)
                .and_then(Value::as_str)
                .map(str::to_owned);
            (ctor, diagnostic)
        }
        None => {
            let ctor = outcome
                .output
                .get(&Iri::parse(wk::CTOR_NAME).unwrap())
                .and_then(Value::as_str)
                .expect("gate verdict carries ctor_name")
                .to_string();
            let diagnostic = outcome
                .output
                .get(&diag_iri)
                .and_then(Value::as_str)
                .map(str::to_owned);
            (ctor, diagnostic)
        }
    }
}

// ── §7.1 OneSidedWitnessed ────────────────────────────────────────────

#[test]
fn one_sided_witnessed_with_valid_witness_holds_with_halved_p() {
    let ctx = build_phase5_chain();
    let (ctor, diagnostic) = validate_claim(
        &ctx,
        "urn:eigenius:demo:decay:claim_short_half_life_onesided",
    );
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds with a valid impossibility witness; got {ctor}, diagnostic: {diagnostic:?}"
    );
    let diag = diagnostic.expect("Holds verdict should carry a OneSided derivation note");
    assert!(
        diag.contains("OneSidedWitnessed") && diag.contains("p_one_sided"),
        "diagnostic should explain the one-sided p halving; got: {diag}"
    );
}

#[test]
fn one_sided_witnessed_with_missing_witness_fails() {
    let ctx = build_phase5_chain();
    let (ctor, diagnostic) = validate_claim(
        &ctx,
        "urn:eigenius:demo:decay:claim_short_half_life_bad_witness",
    );
    assert_eq!(
        ctor,
        wk::VERDICT_FAILS,
        "expected Fails when the witness is not committed on chain; got {ctor}"
    );
    let diag = diagnostic.expect("Fails verdict should carry a witness-resolution diagnostic");
    assert!(
        diag.contains("not committed on chain") && diag.contains("OneSidedWitnessed"),
        "diagnostic should name the unresolved witness path; got: {diag}"
    );
}

// ── §7.2 Dual-verdict ESD outlier exclusion ───────────────────────────

#[test]
fn esd_outlier_exclusion_emits_dual_verdict_diagnostic() {
    let ctx = build_phase5_chain();
    let (ctor, diagnostic) = validate_claim(&ctx, "urn:eigenius:demo:assay:claim_low_readout_esd");
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "with-exclusion verdict should Holds (filtered samples cluster near 50, well below \
         threshold 100); got {ctor}, diagnostic: {diagnostic:?}"
    );
    let diag = diagnostic.expect("dual-verdict path should carry a DualVerdict diagnostic");
    assert!(
        diag.contains("DualVerdict") && diag.contains("ESD"),
        "diagnostic should name the dual-verdict shape and the ESD functor; got: {diag}"
    );
    assert!(
        diag.contains("with-exclusion") && diag.contains("without-exclusion"),
        "diagnostic should enumerate both branches; got: {diag}"
    );
    assert!(
        diag.contains("[10, 11]") || diag.contains("[11, 10]") || diag.contains("[10,11]"),
        "diagnostic should name the excluded indices (10 and 11 — the 200 and 250 outliers); \
         got: {diag}"
    );
}

// ── §7.3 MethodComparisonAnalysisPlan + Passing-Bablok ───────────────────────

#[test]
fn method_comparison_agreement_holds() {
    let ctx = build_phase5_chain();
    let (ctor, diagnostic) = validate_claim(&ctx, "urn:eigenius:demo:methods:claim_methods_agree");
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "concordant methods (slope ≈ 1.0, intercept ≈ 0.0) should Holds; got {ctor}, \
         diagnostic: {diagnostic:?}"
    );
    let diag = diagnostic.expect("PB Holds verdict should carry CIs in the diagnostic");
    assert!(
        diag.contains("Passing-Bablok") && diag.contains("agree"),
        "diagnostic should name Passing-Bablok and confirm agreement; got: {diag}"
    );
}

#[test]
fn method_comparison_proportional_bias_fails() {
    let ctx = build_phase5_chain();
    let (ctor, diagnostic) =
        validate_claim(&ctx, "urn:eigenius:demo:methods:claim_methods_disagree");
    assert_eq!(
        ctor,
        wk::VERDICT_FAILS,
        "1.5x proportional bias should produce MethodComparisonDisagreement; got {ctor}, \
         diagnostic: {diagnostic:?}"
    );
    let diag = diagnostic.expect("PB Fails verdict should carry the disagreement diagnostic");
    assert!(
        diag.contains("MethodComparisonDisagreement") && diag.contains("disagree"),
        "diagnostic should name the disagreement path; got: {diag}"
    );
}
