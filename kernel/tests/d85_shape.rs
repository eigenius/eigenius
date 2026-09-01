//! D85 §6.1 — the two shapes step 1 makes expressible.
//!
//! Nothing produces them yet; this pins that the ontology admits them, which is what
//! makes step 1 additive rather than a cut-over.

use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use std::sync::Arc;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}
fn arr(v: &str) -> Value {
    Value::Array(vec![Value::String(v.into())])
}

#[test]
fn a_value_may_carry_is_a_naming_an_inductive_and_a_ctor_class_may_subclass_one() {
    let core_json = include_str!("../../ontologies/core/core-ontology.json");
    let mut b = LayerBuilder::new("core", None);
    for r in eigon_json::parse_document(core_json).unwrap() {
        b.add_resource(r).unwrap();
    }
    let core = Arc::new(b.build(LayerStorage::in_memory()));

    // (1) A VALUE stating its own type (R1): `is_a` names the inductive it inhabits.
    // Rejected before D85 §6.1 with ClassTypeMismatch, because `is_a` admitted only
    // `core:Class` and `is_subclass_of` walks `subclass_of`, never `is_a`.
    let mut v = Resource::new(iri("urn:test:d85:a_term_value"));
    v.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:eigentt:Term"),
    );

    // (2) A CONSTRUCTOR CLASS naming its inductive in `subclass_of`. Rejected before by a
    // DIFFERENT rule — Rule 22's reference check, which held one expected class.
    let mut c = Resource::new(iri("urn:eigenius:eigentt:Term.App"));
    c.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:core:Class"),
    );
    c.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("Term.App".into()),
    );
    c.set(
        iri("urn:eigenius:core:description"),
        Value::String("Application, as a constructor class over eigentt:Term.".into()),
    );
    c.set(
        iri("urn:eigenius:core:subclass_of"),
        arr("urn:eigenius:eigentt:Term"),
    );

    let mut b2 = LayerBuilder::new("d85-shape", Some(core));
    b2.add_resource(v).unwrap();
    b2.add_resource(c).unwrap();
    let layer = Arc::new(b2.build(LayerStorage::in_memory()));

    let errs: Vec<String> = eigenius_kernel::validation::Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| format!("[{:?}] {}", e.resource_id, e.message))
        .collect();
    assert!(
        errs.is_empty(),
        "both shapes must validate; got:\n{}",
        errs.join("\n")
    );
}
