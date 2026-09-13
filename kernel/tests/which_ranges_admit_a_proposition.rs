//! Which declared ranges let a proposition reach the chain unchecked (eigenius#226).
//!
//! A D85-encoded term is an embedded resource whose `is_a` names its constructor class.
//! Structurally that is indistinguishable from any other embedded value, so whether the
//! kernel type-checks it as a TERM depends entirely on what the carrying property
//! declares. This pins which declarations admit one and which refuse.
//!
//! **The result is the measurement D90 rests on.** The `core:inductive` family is closed:
//! a term slot must name exactly one InductiveType range, and a term under the wrong
//! range is refused twice over. The door is open only where a slot declares NO range, on
//! `core:json` or `core:resource` — and `core:resource` is declared to mean "a reference
//! OR an embedded resource", so an unranged one cannot tell an inline program AST node
//! from an inline proposition.
//!
//! If this test starts failing, the boundary moved and D90 needs re-reading.
use std::sync::Arc;

use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// A property declared with the given data_type, and optionally a class_types range.
fn declare_prop(id: &str, data_type: &str, class_types: Option<&str>) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
    );
    r.set(
        iri(wk::DESCRIPTION),
        Value::String("probe property for eigenius#226".into()),
    );
    r.set(iri(wk::SHORT_NAME), Value::String("probe_prop".into()));
    r.set(iri(wk::DATA_TYPE_PROP), Value::String(data_type.into()));
    if let Some(ct) = class_types {
        r.set(
            iri(wk::CLASS_TYPES),
            Value::Array(vec![Value::String(ct.into())]),
        );
    }
    r
}

#[test]
fn which_declared_ranges_admit_a_proposition() {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let head = Arc::clone(ctx.head());

    // A real proposition in the current (D85) encoding: an embedded resource whose
    // `is_a` names the constructor class.
    let prop_exp = Exp::sort(0);
    let encoded = eigenius_kernel::program::eigentt_type_mirror::encode_type(
        &prop_exp,
        eigenius_kernel::testing::codec_names(),
    )
    .expect("encodes");
    println!("ENCODED SHAPE: {encoded:?}");

    // Each candidate range a smuggling property could be declared with.
    let cases: Vec<(&str, &str, Option<&str>)> = vec![
        ("urn:eigenius:test:p_json", "urn:eigenius:core:json", None),
        (
            "urn:eigenius:test:p_resource",
            "urn:eigenius:core:resource",
            None,
        ),
        (
            "urn:eigenius:test:p_inductive_norange",
            "urn:eigenius:core:inductive",
            None,
        ),
        (
            "urn:eigenius:test:p_inductive_wrongrange",
            "urn:eigenius:core:inductive",
            Some("urn:eigenius:core:Class"),
        ),
        (
            "urn:eigenius:test:p_string",
            "urn:eigenius:core:string",
            None,
        ),
        // Does a RANGE constrain the embedded case on a resource-typed slot, the way it
        // demonstrably does on an inductive one?
        (
            "urn:eigenius:test:p_resource_ranged",
            "urn:eigenius:core:resource",
            Some("urn:eigenius:core:Class"),
        ),
    ];

    for (pid, dt, ct) in cases {
        let mut b = LayerBuilder::new("probe", Some(Arc::clone(&head)));
        b.add_resource(declare_prop(pid, dt, ct)).unwrap();

        let mut carrier = Resource::new(iri("urn:eigenius:test:carrier"));
        carrier.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        carrier.set(iri(wk::DESCRIPTION), Value::String("probe carrier".into()));
        carrier.set(iri(wk::SHORT_NAME), Value::String("carrier".into()));
        carrier.set(iri(pid), encoded.clone());
        let added = b.add_resource(carrier);

        if let Err(e) = added {
            println!("{pid} ({dt}): REFUSED AT ADD — {e}");
            continue;
        }
        let layer = Arc::new(b.build(LayerStorage::in_memory()));
        let errs = eigenius_kernel::validation::Validator::new(layer).validate();
        let admitted = errs.is_empty();
        println!(
            "{pid} ({dt}, class_types={ct:?}): {}",
            if admitted {
                "ADMITTED".to_string()
            } else {
                format!("{} error(s)", errs.len())
            }
        );
        for e in errs.iter() {
            println!("      [{:?}] {}", e.rule, e.message);
        }
        // The pinned boundary: unranged opaque/resource slots admit a term; everything
        // that declares what it holds refuses one.
        let expected_admit = matches!(
            pid,
            "urn:eigenius:test:p_json" | "urn:eigenius:test:p_resource"
        );
        assert_eq!(
            admitted, expected_admit,
            "{pid} ({dt}, class_types={ct:?}) changed: admitted={admitted}, expected={expected_admit}"
        );
    }
}
