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

//! Institution-recompute smoke test — the ClassificationAnalysisPlan
//! dispatch (D52 §2.2). Confirms the threshold classifier emits two
//! StatisticalAnalysisResult derivations (`:result:ppv`, `:result:sensitivity`),
//! each Holds with the metric value and an `IsDerivedAs`-admissible
//! `stats:ge(...)` canonical proposition.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::institution::runtime::Institution;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_statistics::institution::iris;
use eigenius_statistics::StatisticsInstitution;

fn build_chain() -> ExecutionContext {
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
    let stats_layer: Arc<Layer> = {
        let src = include_str!("../../../ontologies/statistics/statistics.esl");
        let resources =
            esl::compile_against_layer(src, &reflection).expect("statistics.esl compiles");
        let mut b = LayerBuilder::new("statistics", Some(reflection));
        for r in resources {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let fixture_layer = {
        let src = include_str!("fixtures/classification_smoke.esl");
        let resources = esl::compile_against_layer(src, &stats_layer).unwrap_or_else(|errs| {
            panic!(
                "classification_smoke.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
        let mut b = LayerBuilder::new("classification-fixture", Some(stats_layer));
        for r in resources {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    ExecutionContext::new(
        fixture_layer,
        "classification-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn classification_plan_emits_ppv_and_sensitivity_results() {
    let ctx = build_chain();
    let plan = (*ctx
        .resolve(&Iri::parse("urn:eigenius:demo:classify:plan").unwrap())
        .expect("plan on chain"))
    .clone();

    let inst = StatisticsInstitution::new();
    let outcome = inst
        .query(
            &Iri::parse(iris::PROC_VALIDATE_ANALYSIS_PLAN).unwrap(),
            &plan,
            &ctx,
        )
        .expect("validate_analysis_plan returns an outcome");

    assert_eq!(
        outcome.derivations.len(),
        2,
        "classification plan emits ppv + sensitivity results"
    );

    let metric = |suffix: &str| {
        outcome
            .derivations
            .iter()
            .find(|d| {
                d.id()
                    .map(|i| i.as_str().ends_with(suffix))
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| panic!("result `{suffix}` emitted"))
    };
    for (suffix, value) in [(":result:ppv", 0.75_f64), (":result:sensitivity", 1.0)] {
        let r = metric(suffix);
        let ctor = r
            .get(&Iri::parse(iris::PROP_VERDICT_CTOR).unwrap())
            .and_then(Value::as_str)
            .expect("verdict ctor");
        assert_eq!(ctor, wk::VERDICT_HOLDS, "{suffix} should Hold");
        let stat = r
            .get(&Iri::parse(iris::PROP_COMPUTED_STATISTIC).unwrap())
            .and_then(|v| match v {
                Value::Float(f) => Some(*f),
                _ => None,
            })
            .expect("computed statistic");
        assert!(
            (stat - value).abs() < 1e-12,
            "{suffix} = {stat}, expected {value}"
        );
        assert!(
            r.get(&Iri::parse(iris::PROP_CANONICAL_PROPOSITION).unwrap())
                .is_some(),
            "{suffix} Holds → must carry a canonical proposition (for IsDerivedAs)"
        );
    }
}
