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

//! D52 Phase 4.9 — RepeatedMeasures (CompoundSymmetry) end-to-end.
//!
//! Exercises:
//!  - The `stats:RepeatedMeasures(n_subjects, n_timepoints, observations,
//!    replication)` smart constructor expanding to a Bundle at product
//!    position `(CompleteRandom, Unblocked, SingleFactor, _,
//!    Longitudinal(n_timepoints))`
//!  - The validator's new RM dispatch arm: reads
//!    `autocorrelation_structure` from the claim, routes to the
//!    CompoundSymmetry numerics arm, decodes `[subject, time, value]`
//!    rows, runs univariate RM-ANOVA, reports the time F-test
//!  - The "AR1 / Unstructured not yet wired" diagnostic paths (the
//!    surface that documents what's not yet supported)
//!  - A 5-subject × 4-timepoint clinical clearance fixture yields Holds
//!    with F_time ≫ 100 and p ≪ 1e-6

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

fn build_rm_chain() -> ExecutionContext {
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

    let fixture_source = include_str!("fixtures/repeated_measures_design.esl");
    let fixture_resources = esl::compile_against_layer(fixture_source, &stats_layer)
        .unwrap_or_else(|errs| {
            panic!(
                "repeated_measures_design.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut fixture_builder = LayerBuilder::new("rm-fixture", Some(stats_layer));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        "rm-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn rm_compound_symmetry_recomputes_to_holds() {
    let ctx = build_rm_chain();
    let claim_iri =
        Iri::parse("urn:eigenius:demo:pk:claim_drug_clears_over_time").expect("claim IRI");
    let claim_arc = ctx
        .resolve(&claim_iri)
        .unwrap_or_else(|| panic!("claim `{claim_iri}` should be on chain"));
    let claim = (*claim_arc).clone();

    let inst = StatisticsInstitution::new();
    let proc_iri = Iri::parse(iris::PROC_VALIDATE_ANALYSIS_PLAN).expect("proc IRI");
    let outcome = inst
        .query(&proc_iri, &claim, &ctx)
        .expect("validate_analysis_plan returns an outcome");
    let result = outcome
        .derivations
        .first()
        .expect("statistics emits a StatisticalAnalysisResult when the SAP ran");

    let ctor = result
        .get(&Iri::parse(iris::PROP_VERDICT_CTOR).unwrap())
        .and_then(Value::as_str)
        .expect("verdict carries ctor_name")
        .to_string();
    let diagnostic = result
        .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned);

    // 5×4 RM with clean monotone decline → F_time huge, p ≪ 1e-6.
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds — RM-ANOVA on the monotone-decline clearance design should reject H0; \
         got {ctor}, diagnostic: {diagnostic:?}"
    );

    let p_value = result
        .get(&Iri::parse(iris::PROP_COMPUTED_P_VALUE).unwrap())
        .and_then(|v| {
            if let Value::Float(f) = v {
                Some(*f)
            } else {
                None
            }
        })
        .expect("verdict carries computed_p_value");
    assert!(
        p_value < 1e-6,
        "RM-ANOVA time F-test should give p ≪ 1e-6; got p = {p_value}"
    );

    let f_stat = result
        .get(&Iri::parse(iris::PROP_COMPUTED_STATISTIC).unwrap())
        .and_then(|v| {
            if let Value::Float(f) = v {
                Some(*f)
            } else {
                None
            }
        })
        .expect("verdict carries computed_statistic");
    assert!(
        f_stat > 100.0,
        "F_time should be very large for this clean monotone decline; got F = {f_stat}"
    );

    // Holds verdict carries the RM diagnostic naming the test
    // configuration for audit.
    let diag = diagnostic.expect("RM verdict should carry a CompoundSymmetry diagnostic");
    assert!(
        diag.contains("CompoundSymmetry"),
        "diagnostic should name the autocorrelation structure used; got: {diag}"
    );
}

#[test]
fn rm_data_lands_at_longitudinal_dispatch_position() {
    let ctx = build_rm_chain();
    let claim_iri =
        Iri::parse("urn:eigenius:demo:pk:claim_drug_clears_over_time").expect("claim IRI");
    let claim_arc = ctx
        .resolve(&claim_iri)
        .unwrap_or_else(|| panic!("claim `{claim_iri}` should be on chain"));
    let claim = (*claim_arc).clone();
    let sample_set_iri_str = claim
        .get(&Iri::parse(iris::PROP_SAMPLE_SET).unwrap())
        .and_then(|v| {
            if let Value::ResourceRef(i) = v {
                Some(i.as_str().to_string())
            } else if let Value::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .expect("claim has sample_set");
    let sample_set_iri = Iri::parse(&sample_set_iri_str).expect("sample_set IRI parses");
    let sample_set_res = ctx.resolve(&sample_set_iri).expect("SampleSet on chain");
    let sample_set_value = sample_set_res
        .get(&Iri::parse("urn:eigenius:measurements:sample_set_value").unwrap())
        .expect("sample_set_value set");
    let bundle_json = if let Value::Json(j) = sample_set_value {
        j
    } else {
        panic!("sample_set_value is not Value::Json");
    };
    // args[4] = Longitudinal(4) — RM-specific repeated-measures axis
    // with the timepoint count carried in args[0] of the inner ctor.
    let rm_ctor = bundle_json["args"][4]["ctor"].as_str();
    assert_eq!(rm_ctor, Some("Longitudinal"));
    let n_timepoints = bundle_json["args"][4]["args"][0].as_i64();
    assert_eq!(
        n_timepoints,
        Some(4),
        "stats:RepeatedMeasures(_, n_timepoints=4, …) should produce Longitudinal(4); got {n_timepoints:?}"
    );

    // args[2] = FullFactorial(0) — RM uses FullFactorial(k_between)
    // uniformly across the (k=0 / k=1 / k≥2) shapes per the §5.2.3
    // dispatch-matrix encoding (k=0 is "no between-subjects factor").
    let factor_ctor = bundle_json["args"][2]["ctor"].as_str();
    assert_eq!(factor_ctor, Some("FullFactorial"));
    let k_between = bundle_json["args"][2]["args"][0].as_i64();
    assert_eq!(
        k_between,
        Some(0),
        "time-only RM fixture should produce FullFactorial(0); got {k_between:?}"
    );

    // args[8] = [factor_levels, observations] wrapper mirroring
    // Factorial. For the k=0 fixture, factor_levels = [] and the
    // inner observations array has 60 floats (3 floats × 5 subjects ×
    // 4 timepoints).
    let wrapper = &bundle_json["args"][8];
    let outer_len = wrapper.as_array().map(Vec::len);
    assert_eq!(
        outer_len,
        Some(2),
        "observations slot should be a [factor_levels, observations] wrapper"
    );
    let factor_levels_len = wrapper[0].as_array().map(Vec::len);
    assert_eq!(
        factor_levels_len,
        Some(0),
        "k=0 fixture should have factor_levels = []"
    );
    let inner_len = wrapper[1].as_array().map(Vec::len);
    assert_eq!(
        inner_len,
        Some(60),
        "5 subjects × 4 timepoints × 3 floats/row = 60 inner observation floats"
    );
}

#[test]
fn rm_ar1_returns_not_yet_wired_diagnostic() {
    // Override the claim's autocorrelation_structure to AR1 and
    // confirm the dispatch rejects with the documented diagnostic.
    use eigenius_kernel::ontology::resource::Value as KV;
    use serde_json::json;

    let ctx = build_rm_chain();
    let claim_iri =
        Iri::parse("urn:eigenius:demo:pk:claim_drug_clears_over_time").expect("claim IRI");
    let claim_arc = ctx
        .resolve(&claim_iri)
        .unwrap_or_else(|| panic!("claim should be on chain"));
    let mut claim = (*claim_arc).clone();
    claim.set(
        Iri::parse(iris::PROP_AUTOCORRELATION_STRUCTURE).unwrap(),
        KV::Json(json!({"ctor": "AR1", "args": []})),
    );

    let inst = StatisticsInstitution::new();
    let proc_iri = Iri::parse(iris::PROC_VALIDATE_ANALYSIS_PLAN).expect("proc IRI");
    let outcome = inst
        .query(&proc_iri, &claim, &ctx)
        .expect("validate handler returns an outcome");

    // AR1 is structurally not wired — the SAP can't run, so the gate
    // verdict Fails and no StatisticalAnalysisResult derivation is emitted.
    assert!(
        outcome.derivations.is_empty(),
        "gate Fails (SAP couldn't run) must not emit derivations; got {} derivations",
        outcome.derivations.len()
    );
    let ctor = outcome
        .output
        .get(&Iri::parse(wk::CTOR_NAME).unwrap())
        .and_then(KV::as_str)
        .expect("gate verdict carries ctor_name")
        .to_string();
    let diagnostic = outcome
        .output
        .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(KV::as_str)
        .map(str::to_owned);
    assert_eq!(ctor, wk::VERDICT_FAILS);
    let diag = diagnostic.expect("gate Fails carries a diagnostic");
    assert!(
        diag.contains("AR1") && diag.contains("not yet wired"),
        "diagnostic should explain AR1 is not yet wired; got: {diag}"
    );
}
