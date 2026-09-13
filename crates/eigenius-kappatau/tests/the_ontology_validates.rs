//! The κ–τ ontology compiles and validates against the bootstrap chain (D91).
//!
//! `every_ontology_is_validated` globs `ontologies/*.json`, so an ESL ontology outside
//! the bootstrap chain reaches a validator only through its own crate's tests — the
//! same arrangement `statistics.esl` has. `the_commitment_condition.rs` validates the
//! FIXTURE layer, which sits above this one, and layer validation covers a layer's own
//! resources rather than its parents', so it does not cover this.
use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};

#[test]
fn the_kappa_tau_ontology_compiles_and_validates() {
    let boot = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let head = Arc::clone(boot.head());
    let source = include_str!("../../../ontologies/kappatau/kappatau.esl");
    let resources = match esl::compile(source, &head) {
        Ok(r) => r,
        Err(e) => panic!("kappatau.esl does not compile: {e:?}"),
    };
    let mut b = LayerBuilder::new("kappatau", Some(head));
    for r in resources {
        b.add_resource(r).expect("resource adds");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let errors = eigenius_kernel::validation::Validator::new(layer).validate();
    assert!(
        errors.is_empty(),
        "{} validation error(s):\n{}",
        errors.len(),
        errors
            .iter()
            .map(|e| format!(
                "  [{:?}] {} — {}",
                e.rule,
                e.resource_id.as_ref().map(|i| i.as_str()).unwrap_or("?"),
                e.message
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
