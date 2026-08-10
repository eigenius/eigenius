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

//! Institution-recompute end-to-end — Spearman correlation on the Paired
//! dispatch with `variance_assumption = RankBased`.
//!
//! Recomputes the REAL WRN dependency ~ #MS-deletions correlation (51 MSI
//! cell lines, the corrected n; finding F1) through the statistics
//! institution and confirms it (a) emits a `Holds` result, (b) carries the
//! verifier-derived `¬(spearman_rho(s) = 0)` proposition (IsDerivedAs-
//! admissible), and (c) reproduces R's rho = -0.7412 (diagnostic) at a
//! highly significant p. The kernel-recomputed form of `D-REFINE`.

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
        let src = include_str!("fixtures/spearman_wrn.esl");
        let resources = esl::compile_against_layer(src, &stats_layer).unwrap_or_else(|errs| {
            panic!(
                "spearman_wrn.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
        let mut b = LayerBuilder::new("spearman-wrn-fixture", Some(stats_layer));
        for r in resources {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    ExecutionContext::new(
        fixture_layer,
        "spearman-wrn-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn spearman_recomputes_wrn_mutator_load_correlation_to_holds() {
    let ctx = build_chain();
    let claim = (*ctx
        .resolve(
            &Iri::parse("urn:eigenius:demo:wrncorr:claim_wrn_dep_tracks_mutator_load").unwrap(),
        )
        .expect("claim on chain"))
    .clone();

    let inst = StatisticsInstitution::new();
    let proc_iri = Iri::parse(iris::PROC_VALIDATE_ANALYSIS_PLAN).unwrap();
    let outcome = inst
        .query(&proc_iri, &claim, &ctx)
        .expect("validate_analysis_plan returns an outcome");
    let result = outcome
        .derivations
        .first()
        .expect("statistics emits a StatisticalAnalysisResult");

    let ctor = result
        .get(&Iri::parse(iris::PROP_VERDICT_CTOR).unwrap())
        .and_then(Value::as_str)
        .expect("verdict ctor")
        .to_string();
    let p_value = result
        .get(&Iri::parse(iris::PROP_COMPUTED_P_VALUE).unwrap())
        .and_then(|v| match v {
            Value::Float(f) => Some(*f),
            _ => None,
        })
        .expect("computed p-value");
    let diagnostic = result
        .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let has_canonical = result
        .get(&Iri::parse("urn:eigenius:reflection:canonical_proposition").unwrap())
        .is_some();

    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds; got {ctor}, diagnostic: {diagnostic:?}"
    );
    assert!(
        has_canonical,
        "Holds correlation result must carry a canonical_proposition (for IsDerivedAs)"
    );
    assert!(
        p_value > 0.0 && p_value < 1e-6,
        "anti-correlation should be highly significant; got {p_value:e}"
    );
    assert!(
        diagnostic.contains("Spearman") && diagnostic.contains("-0.74"),
        "diagnostic should record the Spearman path + rho ~ -0.74; got {diagnostic:?}"
    );
}
