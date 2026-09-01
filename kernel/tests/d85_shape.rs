//! D85 §6.1 — the shapes step 1 makes expressible, and the one it must still reject.
//!
//! Nothing produces these yet; this pins that the ontology admits the right ones, which is
//! what makes step 1 additive rather than a cut-over.

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

fn core_layer() -> Arc<eigenius_kernel::layer::Layer> {
    let core_json = include_str!("../../ontologies/core/core-ontology.json");
    let mut b = LayerBuilder::new("core", None);
    for r in eigon_json::parse_document(core_json).unwrap() {
        b.add_resource(r).unwrap();
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn errors_for(rs: Vec<Resource>) -> Vec<String> {
    let mut b = LayerBuilder::new("d85-shape", Some(core_layer()));
    for r in rs {
        b.add_resource(r).unwrap();
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    eigenius_kernel::validation::Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| format!("[{:?}] {}", e.resource_id, e.message))
        .collect()
}

/// A constructor class names its inductive in `subclass_of`.
///
/// Rejected before step 1 by Rule 22's reference check, which held ONE expected class —
/// which is why `class_types` had open-coded its Class-or-InductiveType case around it.
fn ctor_class() -> Resource {
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
    c
}

#[test]
fn a_constructor_class_may_subclass_its_inductive() {
    let errs = errors_for(vec![ctor_class()]);
    assert!(
        errs.is_empty(),
        "a ctor class must validate; got:\n{}",
        errs.join("\n")
    );
}

#[test]
fn a_value_carries_its_constructors_class_in_is_a() {
    let mut v = Resource::new(iri("urn:test:d85:an_app_value"));
    v.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:eigentt:Term.App"),
    );
    let errs = errors_for(vec![ctor_class(), v]);
    assert!(
        errs.is_empty(),
        "a value naming its ctor class must validate; got:\n{}",
        errs.join("\n")
    );
}

/// **And a value may NOT name the inductive directly.**
///
/// `is_a: [eigentt:Term]` says "some Term, constructor unspecified" — a shape with no arity
/// and no argument types to check, so Rule 1 and Rules 5/6 have nothing to work with. Step 1
/// briefly widened `core:is_a` to admit an `InductiveType` and permitted exactly this; the
/// widening turned out to be unnecessary for the ctor-class shape above, so it was withdrawn.
/// `core:subclass_of` and `core:class_types` DO admit an inductive — they name a type, where
/// `is_a` names the class an instance inhabits.
#[test]
fn a_value_may_not_name_the_inductive_itself() {
    let mut v = Resource::new(iri("urn:test:d85:underspecified"));
    v.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:eigentt:Term"),
    );
    let errs = errors_for(vec![v]);
    assert!(
        errs.iter()
            .any(|e| e.contains("must be an instance of one of")),
        "naming the inductive without a constructor must be rejected; got:\n{}",
        errs.join("\n")
    );
}

/// **The derived names must be spellable in ESL**, and `.` is not.
///
/// A constructor class is `<inductive>-<Ctor>` and an argument property
/// `<inductive>-<Ctor>-<arg>` (§6.1). The separator is forced, not chosen: ESL admits
/// `[A-Za-z0-9_]` bare and `[A-Za-z0-9_-]` quoted, so a dotted name is unspellable either way and
/// `esl::print` hard-errors rather than emit one — which would break printing of every chain
/// carrying a derived class. `_` is in the charset but ambiguous, because constructor names in the
/// tree contain underscores (`cat_np`, `conn_and`): inductive `A_B` + ctor `C` and inductive `A` +
/// ctor `B_C` would collide on `A_B_C`.
///
/// `-` is unambiguous because no component may contain one — every inductive, constructor and
/// argument name in the tree matches `[A-Za-z0-9_]+`.
#[test]
fn derived_names_are_esl_spellable() {
    let doc_for = |id: &str| {
        serde_json::json!([{
            "@id": id,
            "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
            "urn:eigenius:core:short_name": "probe",
            "urn:eigenius:core:description": "probe"
        }])
    };
    let print = |id: &str| eigenius_kernel::esl::print::print_document(&doc_for(id));

    assert!(
        print("urn:eigenius:eigentt:Term-App").is_ok(),
        "the hyphenated constructor-class name must print"
    );
    assert!(
        print("urn:eigenius:eigentt:Term-App-fn").is_ok(),
        "and so must the argument-property name"
    );
    let dotted = print("urn:eigenius:eigentt:Term.App");
    assert!(
        dotted.is_err(),
        "a DOTTED name must not print — no ESL identifier can spell it, which is why `-` is the \
         separator; got: {dotted:?}"
    );
}
