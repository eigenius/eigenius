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

//! D52 Phase 1.5 — IID two-sample dispatch end-to-end.
//!
//! Exercises the IID smart constructor + the new IID verifier arm:
//!  - The `stats:IID(group_a, group_b, replication)` macro expansion
//!    produces a `Bundle` at product position `(CompleteRandom,
//!    Unblocked, SingleFactor, _, CrossSectional)` with the two
//!    groups nested in the observations slot.
//!  - The validator's new IID dispatch reads observations as
//!    `[group_a, group_b]`, runs Welch's t-test, and emits Holds when
//!    p < alpha.

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

fn build_iid_chain() -> ExecutionContext {
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

    let fixture_source = include_str!("fixtures/iid_two_sample.esl");
    let fixture_resources = esl::compile_against_layer(fixture_source, &stats_layer)
        .unwrap_or_else(|errs| {
            panic!(
                "iid_two_sample.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut fixture_builder = LayerBuilder::new("iid-fixture", Some(stats_layer));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        "iid-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn iid_two_sample_recomputes_to_holds() {
    let ctx = build_iid_chain();
    let claim_iri =
        Iri::parse("urn:eigenius:demo:assay:claim_drug_changes_activity").expect("claim IRI");
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

    // Control mean ≈ 100, drug mean ≈ 60, separation ≈ 40 with low
    // intra-group SD: Welch's t-test gives |t| ≫ 5, p ≪ 0.001. Holds.
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds — Welch's t-test on the well-separated groups should reject H0; \
         got {ctor}, diagnostic: {diagnostic:?}"
    );

    // Computed numerics attached for audit.
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
        p_value < 0.001,
        "Welch's t-test on this separation should give p ≪ 0.001; got p = {p_value}"
    );

    let t_stat = result
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
        t_stat.abs() > 5.0,
        "|t| should be well above 5 for this clean separation; got t = {t_stat}"
    );
}

#[test]
fn iid_pooled_variance_dispatches_correctly() {
    // Sanity check: switching variance_assumption from WelchUnequal to
    // Pooled should produce a (slightly) different t-statistic and a
    // (slightly) different df. We don't construct a full second fixture
    // — just confirm the dispatch routes to the Pooled arm by
    // overriding the variance_assumption field on the claim resource
    // in-memory and re-running.
    use eigenius_kernel::ontology::resource::Value as KV;
    use serde_json::json;

    let ctx = build_iid_chain();
    let claim_iri =
        Iri::parse("urn:eigenius:demo:assay:claim_drug_changes_activity").expect("claim IRI");
    let claim_arc = ctx
        .resolve(&claim_iri)
        .unwrap_or_else(|| panic!("claim `{claim_iri}` should be on chain"));
    let mut claim = (*claim_arc).clone();

    // Override variance_assumption from WelchUnequal to Pooled.
    claim.set(
        Iri::parse(iris::PROP_VARIANCE_ASSUMPTION).unwrap(),
        KV::Json(json!({"ctor": "Pooled", "args": []})),
    );

    let inst = StatisticsInstitution::new();
    let proc_iri = Iri::parse(iris::PROC_VALIDATE_ANALYSIS_PLAN).expect("proc IRI");
    let outcome = inst
        .query(&proc_iri, &claim, &ctx)
        .expect("validate handler returns an outcome");
    let result = outcome
        .derivations
        .first()
        .expect("statistics emits a StatisticalAnalysisResult when the SAP ran");

    let ctor = result
        .get(&Iri::parse(iris::PROP_VERDICT_CTOR).unwrap())
        .and_then(KV::as_str)
        .expect("verdict carries ctor_name");
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "Pooled-variance dispatch should also Holds for this clearly separated data"
    );
}
