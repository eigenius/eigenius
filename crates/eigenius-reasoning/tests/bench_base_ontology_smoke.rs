// TEMPORARY smoke test for the benchmark base ontologies (D51 gap 5).
//
// Confirms `experiments/benchmark/base-ontologies/{bench-core,mol}.esl`
// compile against the bootstrap chain and build into layers without
// validator failures — the D51 gap-5 "rounds-trips through the commit
// pipeline cleanly" quality check.
//
// This lives in the production reasoning crate's test dir only for
// bring-up. When the benchmark harness gets its own crate (D51 gap 7),
// move it there and drop the include_str! dependency on experiments/.

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
fn bench_core_and_mol_round_trip() {
    // core
    let core_json = include_str!("../../../ontologies/core/core-ontology.json");
    let core_resources = eigon_json::parse_document(core_json).unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    // reflection (+ eigentt + institution), as in drug_screening.rs
    let reflection_json = include_str!("../../../ontologies/reflection/reflection-ontology.json");
    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for r in eigon_json::parse_document(reflection_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let eigentt_json = include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json");
    for r in eigon_json::parse_document(eigentt_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let institution_json =
        include_str!("../../../ontologies/institution/institution-ontology.json");
    for r in eigon_json::parse_document(institution_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    // bench-core, compiled against reflection
    let bench_core_src =
        include_str!("../../../experiments/benchmark/base-ontologies/bench-core.esl");
    let bench_core_resources = esl::compile_against_layer(bench_core_src, &reflection)
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
    let harness_src = include_str!("../../../experiments/benchmark/harness-ontology.esl");
    let harness_resources = esl::compile_against_layer(harness_src, &bench_core)
        .unwrap_or_else(|errs| fail("harness-ontology.esl compile", errs));
    assert!(
        !harness_resources.is_empty(),
        "harness-ontology produced no resources"
    );
    let mut harness_builder = LayerBuilder::new("harness", Some(bench_core));
    for r in harness_resources {
        harness_builder.add_resource(r).unwrap();
    }
    let harness = Arc::new(harness_builder.build(LayerStorage::in_memory()));

    // mol, compiled against harness (linear chain: bench-core → harness → mol)
    let mol_src = include_str!("../../../experiments/benchmark/base-ontologies/mol.esl");
    let mol_resources = esl::compile_against_layer(mol_src, &harness)
        .unwrap_or_else(|errs| fail("mol.esl compile", errs));
    assert!(!mol_resources.is_empty(), "mol produced no resources");
    let mut mol_builder = LayerBuilder::new("mol", Some(harness));
    for r in mol_resources {
        mol_builder.add_resource(r).unwrap();
    }
    let _mol = Arc::new(mol_builder.build(LayerStorage::in_memory()));
}
