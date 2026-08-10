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

//! Institution-recompute end-to-end — Wilcoxon rank-sum on the IID
//! dispatch with `variance_assumption = RankBased`.
//!
//! Recomputes the REAL WRN MSI-vs-MSS dependency comparison (37 vs 91
//! common-MSI-lineage cell lines, pinned snapshot) through the statistics
//! institution and confirms it (a) emits a `Holds` StatisticalAnalysisResult,
//! (b) carries the verifier-derived two-sample canonical proposition
//! (`¬(mean_diff_of(s) = 0)`) so the D49 witness emitter would admit
//! `IsDerivedAs`, and (c) reproduces the paper's P = 4.2e-13 (the diagnostic
//! note + computed p-value). This turns the recorded WRN dependency warrant
//! into a kernel-recomputed one.

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
    let stats_layer = {
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
        let src = include_str!("fixtures/wilcoxon_wrn.esl");
        let resources = esl::compile_against_layer(src, &stats_layer).unwrap_or_else(|errs| {
            panic!(
                "wilcoxon_wrn.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
        let mut b = LayerBuilder::new("wilcoxon-wrn-fixture", Some(stats_layer));
        for r in resources {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    ExecutionContext::new(
        fixture_layer,
        "wilcoxon-wrn-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn wilcoxon_recomputes_wrn_msi_vs_mss_to_holds() {
    let ctx = build_chain();
    let claim = (*ctx
        .resolve(&Iri::parse("urn:eigenius:demo:wrndep:claim_msi_more_wrn_dependent").unwrap())
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

    // (a) Holds — MSI lines are far more WRN-dependent; Wilcoxon rejects.
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds; got {ctor}, diagnostic: {diagnostic:?}"
    );
    // (b) The result carries the verifier-derived two-sample proposition,
    //     so D49 admits IsDerivedAs against it.
    assert!(
        has_canonical,
        "Holds two-sample result must carry a canonical_proposition (for IsDerivedAs)"
    );
    // (c) Reproduces the paper's P = 4.2e-13 (§5.1 Class-C: log-scale).
    assert!(
        p_value > 0.0 && p_value < 1e-10,
        "expected P ~ 4e-13 (paper); got {p_value:e}"
    );
    // The Wilcoxon path ran (not the t-test).
    assert!(
        diagnostic.contains("Wilcoxon"),
        "diagnostic should record the Wilcoxon rank-sum path; got {diagnostic:?}"
    );
}
