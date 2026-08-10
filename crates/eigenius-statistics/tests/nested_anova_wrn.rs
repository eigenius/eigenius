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

//! Institution-recompute test — the nested two-way ANOVA dispatch (D52
//! §2.2), keyed on a `stats:Nested(...)` SampleSet's NestedBlocking ctor
//! under a plain StatisticalAnalysisPlan, on the REAL WRN ED Fig 3b day-10
//! competition-assay data. Reproduces
//! the paper's two-way ANOVA `value ~ is_WRN + guide`:
//!   - KM12 (MSI): paper p = 2.7e-19 → Holds, carries `lt(mean_diff_of(s), 0)`.
//!   - ES2  (MSS): paper p = 0.37    → Fails (no canonical proposition).

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
        let src = include_str!("fixtures/nested_anova_wrn.esl");
        let resources = esl::compile_against_layer(src, &stats_layer).unwrap_or_else(|errs| {
            panic!(
                "nested_anova_wrn.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
        let mut b = LayerBuilder::new("nested-anova-fixture", Some(stats_layer));
        for r in resources {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    ExecutionContext::new(
        fixture_layer,
        "nested-anova-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

fn recompute(
    ctx: &ExecutionContext,
    plan_iri: &str,
) -> eigenius_kernel::ontology::resource::Resource {
    let plan = (*ctx
        .resolve(&Iri::parse(plan_iri).unwrap())
        .unwrap_or_else(|| panic!("plan `{plan_iri}` on chain")))
    .clone();
    let inst = StatisticsInstitution::new();
    let outcome = inst
        .query(
            &Iri::parse(iris::PROC_VALIDATE_ANALYSIS_PLAN).unwrap(),
            &plan,
            ctx,
        )
        .expect("validate_analysis_plan outcome");
    outcome
        .derivations
        .into_iter()
        .next()
        .expect("nested ANOVA emits a StatisticalAnalysisResult")
}

fn field<'a>(r: &'a eigenius_kernel::ontology::resource::Resource, iri: &str) -> Option<&'a Value> {
    r.get(&Iri::parse(iri).unwrap())
}

#[test]
fn nested_anova_reproduces_wrn_competition_assay() {
    let ctx = build_chain();

    // KM12 (MSI): the paper's 2.7e-19 → Holds, directional lt proposition.
    let km12 = recompute(&ctx, "urn:eigenius:demo:nested:plan_KM12");
    let ctor = field(&km12, iris::PROP_VERDICT_CTOR)
        .and_then(Value::as_str)
        .unwrap();
    let diag = field(&km12, "urn:eigenius:institution:diagnostic")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "KM12 should Hold; diagnostic: {diag}"
    );
    assert!(
        field(&km12, iris::PROP_CANONICAL_PROPOSITION).is_some(),
        "KM12 Holds → must carry lt(mean_diff_of(s),0) for IsDerivedAs"
    );
    // p reported (one-sided) should be ~1.4e-19 (paper two-sided 2.7e-19).
    let p = field(&km12, iris::PROP_COMPUTED_P_VALUE)
        .and_then(|v| match v {
            Value::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap();
    assert!(p < 1e-15, "KM12 p should be ≪ 1e-15; got {p:e}");
    assert!(
        diag.contains("Nested two-way ANOVA") && diag.contains("group_a mean"),
        "diagnostic should record the nested-ANOVA path; got {diag}"
    );

    // ES2 (MSS): paper 0.37 → Fails, no canonical proposition.
    let es2 = recompute(&ctx, "urn:eigenius:demo:nested:plan_ES2");
    let ctor = field(&es2, iris::PROP_VERDICT_CTOR)
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(
        ctor,
        wk::VERDICT_FAILS,
        "ES2 (MSS) should not reach significance"
    );
    assert!(
        field(&es2, iris::PROP_CANONICAL_PROPOSITION).is_none(),
        "ES2 Fails → no canonical proposition (no IsDerivedAs)"
    );
}
