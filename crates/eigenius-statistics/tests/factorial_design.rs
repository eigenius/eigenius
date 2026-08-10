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

//! D52 Phase 2.5 — Factorial (k-way omnibus ANOVA) end-to-end.
//!
//! Exercises:
//!  - The `stats:Factorial(k, factor_levels, observations, replication)`
//!    smart constructor expanding to a Bundle at product position
//!    `(CompleteRandom, Unblocked, FullFactorial(k), _, CrossSectional)`
//!  - The validator's new Factorial dispatch arm: decodes the
//!    `[factor_levels, flat_observations]` payload, chunks rows of
//!    `k+1` floats into `(cell_index, value)` pairs, runs the omnibus
//!    ANOVA, returns the F-statistic + one-sided p-value
//!  - A 2×2 design with cleanly-separated cell means Holds at α = 0.05
//!    (F ≈ 500, df = (3, 8), p ≪ 1e-8)

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

fn build_factorial_chain() -> ExecutionContext {
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

    let fixture_source = include_str!("fixtures/factorial_design.esl");
    let fixture_resources = esl::compile_against_layer(fixture_source, &stats_layer)
        .unwrap_or_else(|errs| {
            panic!(
                "factorial_design.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut fixture_builder = LayerBuilder::new("factorial-fixture", Some(stats_layer));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        "factorial-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn factorial_2x2_omnibus_recomputes_to_holds() {
    let ctx = build_factorial_chain();
    let claim_iri = Iri::parse("urn:eigenius:demo:kx:claim_cell_means_differ").expect("claim IRI");
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

    // 2×2 design with cell means (10, 20, 30, 40) and within-cell SD
    // ≈ 1: omnibus F ≈ 500, df = (3, 8), p ≪ 1e-8. Clear Holds.
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds — omnibus F-test on cleanly separated cells should reject H0; \
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
        "omnibus F on this design should give p ≪ 1e-6; got p = {p_value}"
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
        "F-statistic should be very large for this design; got F = {f_stat}"
    );

    // Per-effect shape: 2×2 → 3 derivations (main_A, main_B,
    // interaction_A_B). Main effects reject (Holds); interaction is
    // zero so its result Fails.
    assert_eq!(
        outcome.derivations.len(),
        3,
        "2×2 factorial yields 3 derivations"
    );

    let effect_names: Vec<String> = outcome
        .derivations
        .iter()
        .map(|d| {
            d.get(&Iri::parse(iris::PROP_EFFECT_NAME).unwrap())
                .and_then(Value::as_str)
                .expect("each result carries effect_name")
                .to_string()
        })
        .collect();
    assert_eq!(effect_names, vec!["main_A", "main_B", "interaction_A_B"]);

    let effect_ctors: Vec<String> = outcome
        .derivations
        .iter()
        .map(|d| {
            d.get(&Iri::parse(iris::PROP_VERDICT_CTOR).unwrap())
                .and_then(Value::as_str)
                .expect("each result carries verdict_ctor")
                .to_string()
        })
        .collect();
    assert_eq!(
        effect_ctors,
        vec![wk::VERDICT_HOLDS, wk::VERDICT_HOLDS, wk::VERDICT_FAILS]
    );
}

#[test]
fn factorial_data_lands_at_full_factorial_dispatch_position() {
    let ctx = build_factorial_chain();
    let claim_iri = Iri::parse("urn:eigenius:demo:kx:claim_cell_means_differ").expect("claim IRI");
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
    // args[2] is the factor axis. `FullFactorial(2)` is what
    // distinguishes the Factorial dispatch position; the integer arg
    // is the factor count k passed to the macro.
    let factor_ctor = bundle_json["args"][2]["ctor"]
        .as_str()
        .expect("factor arg has ctor");
    assert_eq!(
        factor_ctor, "FullFactorial",
        "stats:Factorial must land at the FullFactorial position; got factor = {factor_ctor}"
    );
    let factor_k = bundle_json["args"][2]["args"][0].as_i64();
    assert_eq!(
        factor_k,
        Some(2),
        "stats:Factorial(k=2, …) should produce FullFactorial(2); got k = {factor_k:?}"
    );
}
