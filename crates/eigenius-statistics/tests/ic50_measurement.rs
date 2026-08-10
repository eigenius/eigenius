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

//! D52 Phase 1 end-to-end fixture: IC50 measurement claim.
//!
//! Exercises the full vertical:
//!  - ESL `macro` extension (compile-time AST substitution) via
//!    `stats:SingleSampleEstimate(...)` expanding to the Bundle ctor.
//!  - The statistics ontology compiling against the reflection +
//!    institution + eigentt + core layers.
//!  - The `StatisticsInstitution::query` dispatch resolving the claim's
//!    sample_set, decoding the Bundle product position, running the
//!    one-sample t-test, and emitting a verdict.
//!  - The §7.4 epistemic-scope check admitting the population-level
//!    `HasLowIC50` claim from BiologicalReplication.
//!
//! The 3 IC50 readings (72, 85, 100 nM) tested against H0: μ = 100 nM
//! give a two-sided p-value of ~0.218 (Student's t with df = 2,
//! t ≈ -1.78). At α = 0.05, **the test does not cross the threshold**
//! — so the Phase 1 verdict is Fails(AlphaNotCrossed). This matches
//! the (correct, honest) statistical reading: n = 3 is too small to
//! confidently distinguish 85 nM from 100 nM with this variance.
//!
//! The fixture is therefore wired to expect **Verdict::Fails** with a
//! diagnostic naming the computed p-value and the threshold. Proving
//! the rejection-path is more informative than constructing a contrived
//! n that always passes — it exercises §6's structured-diagnostic
//! requirement.

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

fn build_ic50_chain() -> ExecutionContext {
    // Layer stack: core → reflection → eigentt → institution → statistics → fixture
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

    let fixture_source = include_str!("fixtures/ic50_measurement.esl");
    let fixture_resources = esl::compile_against_layer(fixture_source, &stats_layer)
        .unwrap_or_else(|errs| {
            panic!(
                "ic50_measurement.esl failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut fixture_builder = LayerBuilder::new("ic50-fixture", Some(stats_layer));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        "ic50-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn ic50_measurement_claim_recomputes_to_verdict() {
    let ctx = build_ic50_chain();
    let claim_iri =
        Iri::parse("urn:eigenius:demo:screen:claim_eig0291_lowic50").expect("claim IRI");
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

    // Phase 1's IC50 case: 3 replicate readings (72, 85, 100) tested
    // against threshold 100 nM yields two-sided p ≈ 0.22, which does
    // not cross α = 0.05. Expected verdict is Fails with an
    // AlphaNotCrossed diagnostic. This is the *correct* statistical
    // reading — and proves the verifier wired through end-to-end.
    assert_eq!(
        ctor,
        wk::VERDICT_FAILS,
        "expected Fails (n=3 too small to confidently reject H0: μ=100); \
         got {ctor}, diagnostic: {diagnostic:?}"
    );
    let diag = diagnostic.expect("Fails verdict must carry a diagnostic");
    assert!(
        diag.contains("AlphaNotCrossed"),
        "diagnostic should name the AlphaNotCrossed failure mode: {diag}"
    );

    // The verdict's computed numerics must be attached (D52 §6 — Holds
    // *and* Fails verdicts that actually ran the test carry the
    // intermediate numerics for audit).
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
        p_value > 0.05 && p_value < 0.5,
        "computed p-value should be in (0.05, 0.5) for the IC50 case; got {p_value}"
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
        (t_stat - (-1.776)).abs() < 1e-2,
        "t-statistic should match R's t.test(c(72,85,100), mu=100); got {t_stat}"
    );
}

#[test]
fn confirmatory_claim_recomputes_to_holds() {
    // The confirmatory n=6 dataset (clustered around 85 nM) is what
    // the screening n=3 reading would hand off to in a real workflow:
    // generate hypothesis from the screen, confirm in a larger run.
    // The one-sample t-test against 100 nM here has |t| ≈ 7.5 with
    // df=5; p ≪ 0.05, so the verdict is Holds — proving the success
    // path of the verifier's dispatch, complementary to the Fails
    // path the screening claim exercises.
    let ctx = build_ic50_chain();
    let claim_iri = Iri::parse("urn:eigenius:demo:screen:claim_eig0291_confirmatory_holds")
        .expect("confirmatory claim IRI");
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

    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds on the n=6 confirmatory dataset; \
         got {ctor}, diagnostic: {diagnostic:?}"
    );

    // Holds verdicts must also carry the computed numerics so audit
    // consumers see *why* the test crossed alpha.
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
        "Holds requires p < alpha = 0.05; got p = {p_value}"
    );
}

#[test]
fn sar_admits_is_derived_as_witness_via_institution_emitted_marker() {
    // Post-step-1E: the IsDerivedAs witness is admitted off the
    // `StatisticalAnalysisResult` derivation's
    // `reflection:InstitutionEmittedDerivation` marker, NOT off a
    // separate ProgramTrace. The witness emitter walks every
    // `InstitutionEmittedDerivation` and indexes by
    // `(resource_iri, canonical_proposition)`.
    //
    // The witness admission is independent of the institution's
    // verdict outcome — the index is built from chain shapes
    // (`is_a InstitutionEmittedDerivation` + `canonical_proposition`),
    // not from runtime verifier outputs. The test below commits the
    // pre-authored confirmatory SAR (via fixture load) and confirms
    // the index sees the witness keyed on the SAR's IRI.
    use eigenius_kernel::layer::lookup_chain_witness;
    use eigenius_kernel::witness::{WitnessCategory, WitnessKey};

    let ctx = build_ic50_chain();
    let sar_iri =
        Iri::parse("urn:eigenius:demo:screen:claim_eig0291_confirmatory_holds:result:main_effect")
            .expect("SAR IRI");
    let sar_arc = ctx
        .resolve(&sar_iri)
        .unwrap_or_else(|| panic!("pre-authored SAR `{sar_iri}` should be on chain"));

    // Read the SAR's canonical_proposition — the strictly-statistical
    // claim the verifier emits for SingleSampleEstimate +
    // OneSidedWitnessed + Absolute(T), pre-authored here to match
    // what the institution would produce.
    let canonical_prop = sar_arc
        .get(&Iri::parse("urn:eigenius:reflection:canonical_proposition").unwrap())
        .expect("SAR must carry canonical_proposition for witness admission")
        .clone();

    // Build the lookup key the way D49 §6 builds it from the emitter
    // side, using the same hash_proposition_value the index uses.
    let expected_key =
        WitnessKey::from_encoded(WitnessCategory::Derived, sar_iri.clone(), &canonical_prop);

    assert!(
        lookup_chain_witness(ctx.head().as_ref(), &expected_key),
        "IsDerivedAs witness for {sar_iri} with the SAR's canonical_proposition \
         must be in the chain witness index (the pre-authored SAR carries both \
         `is_a InstitutionEmittedDerivation` and `canonical_proposition` — the \
         two preconditions the D49 emitter requires)"
    );
}
