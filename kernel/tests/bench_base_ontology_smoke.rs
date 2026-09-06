// `bench-core.esl` and `harness-ontology.esl` compile against the bootstrap chain and build into
// layers without validator failures.
//
// These two are not benchmark scaffolding despite where they live: the WRN publication chain loads
// both (`wrn_phase2`, `wrn_phase3`, `wrn_phase5`, `wrn_phase1_recompute`, `demo/wrn-helicase/run.sh`)
// for `bench:Measurement`, `bench:Dataset` and `bench:TaskOutput`. Those tests would also fail if
// either stopped compiling, but only after building a seven-layer chain; this says which file broke.
//
// The `mol.esl` module it also covered was deleted `2026-09-05` with the SAB tracer tasks — nothing
// outside them used the `mol:` namespace.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;

fn fail(stage: &str, errs: Vec<impl std::fmt::Debug>) -> ! {
    panic!(
        "{stage} failed:\n{}",
        errs.into_iter()
            .map(|e| format!("  - {e:?}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

#[test]
fn bench_core_and_harness_round_trip() {
    // core
    let core_json = include_str!("../../ontologies/core/core-ontology.json");
    let core_resources = eigon_json::parse_document(core_json).unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    // reflection (+ eigentt + institution), as in drug_screening.rs
    let reflection_json = include_str!("../../ontologies/reflection/reflection-ontology.json");
    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for r in eigon_json::parse_document(reflection_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let eigentt_json = include_str!("../../ontologies/eigentt/eigentt-type-fragment.json");
    for r in eigon_json::parse_document(eigentt_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let institution_json = include_str!("../../ontologies/institution/institution-ontology.json");
    for r in eigon_json::parse_document(institution_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    // bench-core, compiled against reflection
    let bench_core_src = include_str!("../../experiments/benchmark/base-ontologies/bench-core.esl");
    let bench_core_resources = esl::compile(bench_core_src, &reflection)
        .unwrap_or_else(|errs| fail("bench-core.esl compile", errs));
    assert!(
        !bench_core_resources.is_empty(),
        "bench-core produced no resources"
    );
    let mut bc_builder = LayerBuilder::new("bench-core", Some(reflection));
    for r in bench_core_resources {
        bc_builder.add_resource(r).unwrap();
    }
    let bench_core = Arc::new(bc_builder.build(LayerStorage::in_memory()));

    // harness-ontology (bench:TaskOutput), compiled against bench-core
    let harness_src = include_str!("../../experiments/benchmark/harness-ontology.esl");
    let harness_resources = esl::compile(harness_src, &bench_core)
        .unwrap_or_else(|errs| fail("harness-ontology.esl compile", errs));
    assert!(
        !harness_resources.is_empty(),
        "harness-ontology produced no resources"
    );
    let mut harness_builder = LayerBuilder::new("harness", Some(bench_core));
    for r in harness_resources {
        harness_builder.add_resource(r).unwrap();
    }
    let _harness = Arc::new(harness_builder.build(LayerStorage::in_memory()));
}
