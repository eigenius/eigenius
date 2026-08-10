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

//! D53 §6.1 — native recompute over a `PinnedExternalFile` (storage ⊥ grade).
//!
//! The same one-sample test that `ic50_measurement.rs` runs over an **inline**
//! SampleSet (`confirmatory_claim_recomputes_to_holds`: 6 readings vs μ=100 →
//! Holds, t = -8.056) is run here with the observations read from a **file** — a
//! `PinnedExternalFile` column the kernel content-verifies and reads. The
//! verdict + numerics must be **identical**: the method (deterministic Rust) and
//! the content-addressed bytes set the grade, not where the bytes were stored.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::institution::runtime::Institution;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_statistics::institution::iris;
use eigenius_statistics::StatisticsInstitution;
use sha2::{Digest, Sha256};

const FILE_IRI: &str = "urn:eigenius:demo:screen:ic50_file";
const PLAN_IRI: &str = "urn:eigenius:demo:screen:claim_file_backed";

/// The confirmatory dataset from `ic50_measurement.rs`, in a CSV column.
const CSV_BODY: &str = "id,ic50\nr1,78.0\nr2,82.0\nr3,85.0\nr4,88.0\nr5,91.0\nr6,86.0\n";

/// Inline ESL fixture: the impossibility witness, a file-backed
/// SampleSetResource (empty inline observations + `observations_source` /
/// `observations_column`), and the one-sided SAP — same parameters as the
/// inline confirmatory claim.
const FIXTURE: &str = r#"
namespace core       = "urn:eigenius:core";
namespace formats    = "urn:eigenius:core:formats";
namespace reflection = "urn:eigenius:reflection";
namespace stats      = "urn:eigenius:measurements";
namespace screen     = "urn:eigenius:demo:screen";

resource screen:witness_kinaseglo_floor : stats:ImpossibilityWitness {
    reflection:declared_by = "methodology:kinase-glo-emax-floor";
    reflection:rationale   = "E_max plateau excludes the inverse direction; licenses the one-sided path.";
}

resource screen:ss_file_backed : stats:SampleSetResource {
    reflection:source      = "depmap-slice:ic50-confirmatory";
    reflection:observed_at = "2026-03-11T10:18:42Z";

    // Observations live off-chain (D53 §6.1): the inline slot is empty;
    // the verifier reads the `ic50` column of the PinnedExternalFile.
    stats:sample_set_value     = stats:SingleSampleEstimate([], BiologicalReplication());
    stats:observations_source  = "urn:eigenius:demo:screen:ic50_file";
    stats:observations_column  = "ic50";
}

resource screen:claim_file_backed : stats:StatisticalAnalysisPlan {
    stats:sample_set = screen:ss_file_backed;

    stats:alpha               = 0.05;
    stats:effect_size         = Absolute(100.0, "nM");
    stats:directionality      = OneSidedWitnessed("urn:eigenius:demo:screen:witness_kinaseglo_floor");
    stats:variance_assumption = WelchUnequal();
    stats:outlier_exclusion   = Identity();
}
"#;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI")
}

/// Build core→reflection→eigentt→institution→statistics→ingest→fixture, with a
/// programmatically-added `PinnedExternalFile` pointing at the temp CSV.
fn build_chain(csv_path: &str, content_hash: &str) -> ExecutionContext {
    let core = {
        let rs =
            eigon_json::parse_document(include_str!("../../../ontologies/core/core-ontology.json"))
                .unwrap();
        let mut b = LayerBuilder::new("core", None);
        for r in rs {
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
        let rs = esl::compile_against_layer(
            include_str!("../../../ontologies/statistics/statistics.esl"),
            &reflection,
        )
        .expect("statistics.esl compiles");
        let mut b = LayerBuilder::new("statistics", Some(reflection));
        for r in rs {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };

    let ingest = {
        let rs = eigon_json::parse_document(include_str!(
            "../../../ontologies/ingest/ingest-ontology.json"
        ))
        .unwrap();
        let mut b = LayerBuilder::new("ingest", Some(stats_layer));
        for r in rs {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };

    let fixture = {
        let rs = esl::compile_against_layer(FIXTURE, &ingest).unwrap_or_else(|errs| {
            panic!(
                "fixture failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
        let mut b = LayerBuilder::new("file-backed-fixture", Some(ingest));
        for r in rs {
            b.add_resource(r).unwrap();
        }
        // The PinnedExternalFile, built programmatically so its reference +
        // content_hash track the temp CSV.
        let mut file = Resource::new(iri(FILE_IRI));
        file.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:ingest:PinnedExternalFile",
            ))]),
        );
        file.set(
            iri("urn:eigenius:reflection:source"),
            Value::String("depmap-slice:ic50-confirmatory".into()),
        );
        file.set(
            iri("urn:eigenius:ingest:reference"),
            Value::String(format!("file://{csv_path}")),
        );
        file.set(
            iri("urn:eigenius:ingest:content_hash"),
            Value::String(content_hash.into()),
        );
        file.set(
            iri("urn:eigenius:ingest:media_type"),
            Value::String("text/csv".into()),
        );
        b.add_resource(file).unwrap();
        Arc::new(b.build(LayerStorage::in_memory()))
    };

    ExecutionContext::new(
        fixture,
        "file-backed-fixture",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn file_backed_observations_match_inline_recompute() {
    // Materialize the off-chain column to a temp file + compute its hash.
    let csv_path = std::env::temp_dir().join(format!("eig_p4_ic50_{}.csv", std::process::id()));
    std::fs::write(&csv_path, CSV_BODY).unwrap();
    let content_hash = format!("sha256:{:x}", Sha256::digest(CSV_BODY.as_bytes()));

    let ctx = build_chain(&csv_path.to_string_lossy(), &content_hash);

    let claim = (*ctx.resolve(&iri(PLAN_IRI)).expect("plan on chain")).clone();
    let inst = StatisticsInstitution::new(); // file:// store — no cache root needed
    let outcome = inst
        .query(&iri(iris::PROC_VALIDATE_ANALYSIS_PLAN), &claim, &ctx)
        .expect("validate returns an outcome");
    let result = outcome
        .derivations
        .first()
        .expect("a StatisticalAnalysisResult was emitted");

    let ctor = result
        .get(&iri(iris::PROP_VERDICT_CTOR))
        .and_then(Value::as_str)
        .expect("verdict ctor");
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "file-backed confirmatory data must Hold, same as the inline case"
    );

    // Deterministic one-sample t for [78,82,85,88,91,86] vs μ=100:
    // mean 85, sd ≈ 4.561, t = -15 / (sd/√6) ≈ -8.056 — the same value the
    // inline path computes (identical `one_sample_t_test` over the same
    // numbers), confirming the file column fed the recompute correctly.
    let t_stat = match result.get(&iri(iris::PROP_COMPUTED_STATISTIC)) {
        Some(Value::Float(f)) => *f,
        other => panic!("expected computed_statistic float, got {other:?}"),
    };
    assert!(
        (t_stat - (-8.056)).abs() < 1e-2,
        "t-statistic must match the deterministic recompute (-8.056); got {t_stat}"
    );
    let p_value = match result.get(&iri(iris::PROP_COMPUTED_P_VALUE)) {
        Some(Value::Float(f)) => *f,
        other => panic!("expected computed_p_value float, got {other:?}"),
    };
    assert!(p_value < 0.05, "one-sided p must cross α; got {p_value}");

    let _ = std::fs::remove_file(&csv_path);
}

#[test]
fn file_backed_observations_fail_closed_on_tamper() {
    let csv_path = std::env::temp_dir().join(format!("eig_p4_tamper_{}.csv", std::process::id()));
    std::fs::write(&csv_path, CSV_BODY).unwrap();
    // Pin a WRONG hash — the content-verify must fail closed and the verifier
    // must not produce a Holds verdict over unverified bytes.
    let wrong_hash = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    let ctx = build_chain(&csv_path.to_string_lossy(), wrong_hash);
    let claim = (*ctx.resolve(&iri(PLAN_IRI)).expect("plan on chain")).clone();
    let inst = StatisticsInstitution::new();
    let outcome = inst
        .query(&iri(iris::PROC_VALIDATE_ANALYSIS_PLAN), &claim, &ctx)
        .expect("validate returns an outcome");
    // A content-hash mismatch fails *before* the test runs (a gate failure):
    // the Fails verdict lands in `output`, and no Holds derivation is emitted.
    let ctor = outcome
        .output
        .get(&iri(wk::CTOR_NAME))
        .and_then(Value::as_str)
        .expect("gate verdict ctor");
    assert_eq!(
        ctor,
        wk::VERDICT_FAILS,
        "a content-hash mismatch must fail closed, not Hold"
    );
    assert!(
        outcome.derivations.is_empty(),
        "no statistical result should be emitted over unverified bytes"
    );

    let _ = std::fs::remove_file(&csv_path);
}
