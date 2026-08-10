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

//! D52 Phase 4.5 — Split-Plot end-to-end.
//!
//! Exercises:
//!  - The `stats:SplitPlot(a, b, r, observations, replication)` smart
//!    constructor expanding to a Bundle at product position
//!    `(Restricted, SplitPlotBlocking(a, r), FullFactorial(2), _,
//!    CrossSectional)`
//!  - The validator's new SplitPlot dispatch arm: decodes
//!    `[whole_plot, w, s, value]` rows, runs split-plot ANOVA with
//!    nested error strata, reports the smallest p-value across the
//!    three F-tests (W main, S main, W×S interaction) with a
//!    diagnostic naming which effect produced it
//!  - A 2×2×3 design with clear whole-plot temperature effect and
//!    subplot drug effect yields Holds with the diagnostic naming the
//!    most-significant effect

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

fn build_splitplot_chain() -> ExecutionContext {
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

    let fixture_source = include_str!("fixtures/splitplot_design.esl");
    let fixture_resources = esl::compile_against_layer(fixture_source, &stats_layer)
        .unwrap_or_else(|errs| {
            panic!(
                "splitplot_design.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut fixture_builder = LayerBuilder::new("splitplot-fixture", Some(stats_layer));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        "splitplot-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn splitplot_2x2x3_recomputes_to_holds_with_effect_diagnostic() {
    let ctx = build_splitplot_chain();
    let claim_iri = Iri::parse("urn:eigenius:demo:tx:claim_effect_exists").expect("claim IRI");
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

    // 2×2×3 split-plot: both temperature (whole-plot) and drug
    // (subplot) main effects are clean and detectable. The first
    // result derivation in canonical order is main_whole_plot.
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds on main_whole_plot — split-plot's whole-plot \
         effect should reject; got {ctor}, diagnostic: {diagnostic:?}"
    );

    // Per-effect diagnostic carries the effect name + F/p numerics.
    let diag = diagnostic.expect("StatisticalAnalysisResult must carry a per-effect diagnostic");
    assert!(
        diag.contains("SplitPlot effect"),
        "diagnostic should name the per-effect shape; got: {diag}"
    );
    assert!(
        diag.contains("main_whole_plot"),
        "first derivation should be main_whole_plot; got: {diag}"
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
        p_value < 0.05,
        "SplitPlot main_whole_plot p should reject at α = 0.05; got p = {p_value}"
    );

    // Per-effect shape: 3 derivations (whole_plot, subplot, interaction).
    assert_eq!(
        outcome.derivations.len(),
        3,
        "SplitPlot emits exactly 3 per-effect StatisticalAnalysisResults"
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
    assert_eq!(
        effect_names,
        vec![
            "main_whole_plot",
            "main_subplot",
            "interaction_whole_plot_subplot"
        ]
    );
}

#[test]
fn splitplot_data_lands_at_split_plot_dispatch_position() {
    let ctx = build_splitplot_chain();
    let claim_iri = Iri::parse("urn:eigenius:demo:tx:claim_effect_exists").expect("claim IRI");
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
    // args[0] = Restricted
    let randomization = bundle_json["args"][0]["ctor"].as_str();
    assert_eq!(randomization, Some("Restricted"));
    // args[1] = SplitPlotBlocking(a, r)
    let blocking_ctor = bundle_json["args"][1]["ctor"].as_str();
    assert_eq!(
        blocking_ctor,
        Some("SplitPlotBlocking"),
        "stats:SplitPlot must land at SplitPlotBlocking; got {blocking_ctor:?}"
    );
    let a = bundle_json["args"][1]["args"][0].as_i64();
    let r = bundle_json["args"][1]["args"][1].as_i64();
    assert_eq!(a, Some(2), "a should be 2");
    assert_eq!(r, Some(3), "r should be 3");
    // args[2] = FullFactorial(2)
    let factor_ctor = bundle_json["args"][2]["ctor"].as_str();
    assert_eq!(factor_ctor, Some("FullFactorial"));
}
