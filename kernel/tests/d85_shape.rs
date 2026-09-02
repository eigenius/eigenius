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

/// The constructor class for `eigentt:Term.App` **exists, derived**, in the layer that declares
/// the inductive — nobody authored it.
///
/// `core:ctors` is the single declaration; the class is a projection of it, materialised at
/// layer build (D85 §6.1). Authoring one instead is what Rule 25 refuses, and
/// `a_constructor_class_from_a_later_layer_is_refused` below pins that.
#[test]
fn the_constructor_class_is_derived_into_the_inductives_layer() {
    let core = core_layer();
    let cls = core
        .get_resource(&iri("urn:eigenius:eigentt:Term-App"))
        .expect("eigentt:Term-App is derived into core, where eigentt:Term is declared");
    assert!(
        cls.get(&iri("urn:eigenius:core:subclass_of"))
            .map(|v| v.as_iri_array().contains(&iri("urn:eigenius:eigentt:Term")))
            .unwrap_or(false),
        "the derived class must name its inductive in subclass_of"
    );
    let errs: Vec<String> = eigenius_kernel::validation::Validator::new(Arc::clone(&core))
        .validate()
        .into_iter()
        .filter(|e| {
            e.resource_id
                .as_ref()
                .is_some_and(|i| i.as_str().contains("Term-App"))
        })
        .map(|e| e.message)
        .collect();
    assert!(
        errs.is_empty(),
        "the derived class must validate; got:\n{}",
        errs.join("\n")
    );
}

/// A test-local inductive, so these exercise step 1's derivation without reaching into step 3.
///
/// A resource-form value in an `eigentt:Term`-typed slot is still refused by the D47 codec
/// ("expected Value::Json, got Embedded") — that codec is what step 3 changes. Step 1 delivers
/// the STRUCTURE: the classes exist, arity is checked, argument types are checked.
fn colour_inductive() -> Resource {
    let mut ind = Resource::new(iri("urn:test:d85:Colour"));
    ind.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:core:InductiveType"),
    );
    ind.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("Colour".into()),
    );
    ind.set(
        iri("urn:eigenius:core:description"),
        Value::String("a test inductive".into()),
    );

    let mut red = Resource::new_embedded();
    red.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:core:InductiveCtor"),
    );
    red.set(
        iri("urn:eigenius:core:ctor_name"),
        Value::String("Red".into()),
    );

    let mut named = Resource::new_embedded();
    named.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:core:InductiveCtor"),
    );
    named.set(
        iri("urn:eigenius:core:ctor_name"),
        Value::String("Named".into()),
    );
    let mut arg = Resource::new_embedded();
    arg.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:core:InductiveArgType"),
    );
    arg.set(
        iri("urn:eigenius:core:arg_name"),
        Value::String("name".into()),
    );
    arg.set(
        iri("urn:eigenius:core:type_name"),
        Value::Json(
            serde_json::json!({"ctor": "ConstRef", "args": ["urn:eigenius:core:string", []]}),
        ),
    );
    named.set(
        iri("urn:eigenius:core:arg_types"),
        Value::Array(vec![Value::Embedded(Box::new(arg))]),
    );

    ind.set(
        iri("urn:eigenius:core:ctors"),
        Value::Array(vec![
            Value::Embedded(Box::new(red)),
            Value::Embedded(Box::new(named)),
        ]),
    );
    ind
}

/// A value names its constructor's class and supplies that constructor's arguments as the
/// properties the class requires.
///
/// **Arity is Rule 1 and argument types are Rule 5** — neither is written anywhere. They fall
/// out of the class and properties the derivation built from `core:ctors`.
#[test]
fn a_value_carries_its_constructors_class_and_arguments() {
    let mut v = Resource::new(iri("urn:test:d85:a_named_colour"));
    v.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:test:d85:Colour-Named"),
    );
    v.set(
        iri("urn:test:d85:Colour-Named-name"),
        Value::String("mauve".into()),
    );
    let errs = errors_for(vec![colour_inductive(), v]);
    assert!(
        errs.is_empty(),
        "a well-formed value must validate; got:\n{}",
        errs.join("\n")
    );
}

/// A nullary constructor's class requires nothing, so a value of it carries only its `is_a`.
#[test]
fn a_nullary_constructors_value_needs_no_arguments() {
    let mut v = Resource::new(iri("urn:test:d85:a_red"));
    v.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:test:d85:Colour-Red"),
    );
    let errs = errors_for(vec![colour_inductive(), v]);
    assert!(
        errs.is_empty(),
        "a nullary value must validate; got:\n{}",
        errs.join("\n")
    );
}

/// Omitting an argument is caught by Rule 1, with no rule of its own.
#[test]
fn a_value_missing_a_constructor_argument_is_refused() {
    let mut v = Resource::new(iri("urn:test:d85:missing_an_arg"));
    v.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:test:d85:Colour-Named"),
    );
    let errs = errors_for(vec![colour_inductive(), v]);
    assert!(
        errs.iter()
            .any(|e| e.contains("Colour-Named-name") && e.contains("missing")),
        "the missing argument must be named; got:\n{}",
        errs.join("\n")
    );
}

/// And an argument of the wrong type is caught by Rule 5, likewise.
#[test]
fn a_value_with_a_mistyped_argument_is_refused() {
    let mut v = Resource::new(iri("urn:test:d85:mistyped"));
    v.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:test:d85:Colour-Named"),
    );
    v.set(iri("urn:test:d85:Colour-Named-name"), Value::Integer(7));
    let errs = errors_for(vec![colour_inductive(), v]);
    assert!(
        !errs.is_empty(),
        "an integer where the constructor declares core:string must be refused"
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
    let print = |id: &str| eigenius_kernel::esl::print::print_document(&doc_for(id), &core_layer());

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

// ── Rule 25 — an inductive stays closed ──────────────────────────────

/// A constructor class written by hand in a LATER layer is refused.
///
/// This is the hole §6.1 opens and Rule 25 closes. Before §6.1, closedness was structural:
/// `core:ctors` holds embedded resources with no `@id`, so there was nowhere to add a
/// constructor. Giving constructors top-level classes makes them addable, and a value carrying
/// `is_a: [eigentt:Term-Bogus]` would satisfy every slot declaring `class_types eigentt:Term` —
/// subsumption walks `subclass_of` — while no match arm covers it and no eliminator handles it.
#[test]
fn a_constructor_class_from_a_later_layer_is_refused() {
    let core = core_layer();
    let mut bogus = Resource::new(iri("urn:eigenius:eigentt:Term-Bogus"));
    bogus.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:core:Class"),
    );
    bogus.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("Term-Bogus".into()),
    );
    bogus.set(
        iri("urn:eigenius:core:description"),
        Value::String("not a real ctor".into()),
    );
    bogus.set(
        iri("urn:eigenius:core:subclass_of"),
        arr("urn:eigenius:eigentt:Term"),
    );

    let mut b = LayerBuilder::new("later", Some(core));
    b.add_resource(bogus).unwrap();
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let errs: Vec<String> = eigenius_kernel::validation::Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert!(
        errs.iter().any(|m| m.contains("declared in a LOWER layer")),
        "a ctor class in a layer above its inductive must be refused; got:\n{}",
        errs.join("\n")
    );
}

/// And one written in the inductive's OWN layer, naming a constructor it does not declare.
///
/// Same-layer alone is not enough: `core:ctors` is the authority, and it is what exhaustiveness
/// reads, so a class outside that set would be a constructor the eliminator does not know about
/// even inside the declaring layer.
#[test]
fn a_constructor_class_naming_no_declared_ctor_is_refused() {
    let mut b = LayerBuilder::new("own-layer", None);
    // A minimal inductive declared in THIS layer, with one real constructor.
    let mut ind = Resource::new(iri("urn:test:d85:Colour"));
    ind.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:core:InductiveType"),
    );
    ind.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("Colour".into()),
    );
    let mut ctor = Resource::new_embedded();
    ctor.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:core:InductiveCtor"),
    );
    ctor.set(
        iri("urn:eigenius:core:ctor_name"),
        Value::String("Red".into()),
    );
    ind.set(
        iri("urn:eigenius:core:ctors"),
        Value::Array(vec![Value::Embedded(Box::new(ctor))]),
    );
    b.add_resource(ind).unwrap();

    // Same layer, but names a constructor `core:ctors` does not declare.
    let mut bogus = Resource::new(iri("urn:test:d85:Colour-Mauve"));
    bogus.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:core:Class"),
    );
    bogus.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("Colour-Mauve".into()),
    );
    bogus.set(
        iri("urn:eigenius:core:description"),
        Value::String("undeclared".into()),
    );
    bogus.set(
        iri("urn:eigenius:core:subclass_of"),
        arr("urn:test:d85:Colour"),
    );
    b.add_resource(bogus).unwrap();

    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let errs: Vec<String> = eigenius_kernel::validation::Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert!(
        errs.iter()
            .any(|m| m.contains("must be one of its constructors") && m.contains("Red")),
        "a ctor class naming no declared ctor must be refused, and the message must list what IS \
         declared; got:\n{}",
        errs.join("\n")
    );
}

/// The derived classes themselves satisfy Rule 25 — which is the point of deriving them.
#[test]
fn the_derived_constructor_classes_satisfy_rule_25() {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let errs: Vec<String> = eigenius_kernel::validation::Validator::new(ctx.head().clone())
        .validate()
        .into_iter()
        .filter(|e| format!("{:?}", e.rule).contains("InductiveNotClosed"))
        .map(|e| e.message)
        .collect();
    assert!(
        errs.is_empty(),
        "derived classes must satisfy Rule 25; got:\n{}",
        errs.join("\n")
    );
}

/// An INDUCTIVE-typed argument is declared `core:inductive`, not `core:resource`.
///
/// This follows the ontology's own convention — `core:type_name` and `core:param_kind` both hold
/// an `eigentt:Term` value and both declare `data_type: core:inductive` with the inductive in
/// `class_types`. A `core:resource` slot with an inductive in `class_types` takes a different
/// path: Rule 8 dispatches it to the JSON walker, which rejects the resource form outright.
///
/// **The value half of this waits for step 3.** A resource-form value in a `eigentt:Term`-ranged
/// slot is still refused by Rule 21, which routes such slots through the D47 codec — and that
/// codec reads `Value::Json` only. Rule 21 exempts exactly two properties by name
/// (`is_declaration_internal`: `core:type_name` and `core:param_kind`), which is why 98 of the
/// 123 authored values sit in slots that would accept the new shape today and the rest do not.
/// Step 3 is what makes `decode_type` read a value resource; until then this test pins the
/// DECLARATION the derivation produces, which is what step 1 owes.
#[test]
fn an_inductive_typed_argument_is_declared_core_inductive() {
    let core = core_layer();
    let head = core
        .get_resource(&iri("urn:eigenius:eigentt:Term-App-head"))
        .expect("derived");
    assert_eq!(
        head.get(&iri("urn:eigenius:core:data_type"))
            .and_then(|v| v.as_str()),
        Some("urn:eigenius:core:inductive"),
        "an eigentt:Term-typed argument must be declared core:inductive, not core:resource"
    );
    assert!(
        head.get(&iri("urn:eigenius:core:class_types"))
            .map(|v| v.as_iri_array().contains(&iri("urn:eigenius:eigentt:Term")))
            .unwrap_or(false),
        "and must name the inductive in class_types"
    );
}

// ── Step 2 — `decode_type` reads a value resource ────────────────────

/// A resource-form value validates in an `eigentt:Term`-ranged slot.
///
/// Before step 2 this failed with "expected Value::Json, got Embedded": Rule 21 routes every
/// `eigentt:Term`- or `Judgement`-ranged slot through the D47 codec, and the codec read the
/// tagged dict only. `decode_type` now reads both, by translating the resource form to the
/// tagged form and decoding that — so every constructor's decoding stays in one place and the
/// two shapes cannot drift while both are accepted.
///
/// `encode_type` still emits the dict. Expand before migrate: nothing is rewritten yet, so
/// nothing can break.
#[test]
fn a_value_resource_decodes_in_a_term_ranged_slot() {
    let leaf = || {
        let mut r = Resource::new_embedded();
        r.set(
            iri("urn:eigenius:core:is_a"),
            arr("urn:eigenius:eigentt:Term-UnitVal"),
        );
        Value::Embedded(Box::new(r))
    };
    let mut v = Resource::new(iri("urn:test:d85:app_of_two_leaves"));
    v.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:eigentt:Term-App"),
    );
    v.set(iri("urn:eigenius:eigentt:Term-App-head"), leaf());
    v.set(iri("urn:eigenius:eigentt:Term-App-arg"), leaf());
    let errs = errors_for(vec![v]);
    // What step 2 delivers is that the value DECODES. Whether an arbitrary term also
    // type-checks is Rule 21's second phase and a property of the term, not of its shape —
    // `App` of two units is ill-typed in either encoding.
    assert!(
        !errs.iter().any(|e| e.contains("failed to decode")),
        "a value resource must decode in an eigentt:Term-ranged slot; got:\n{}",
        errs.join("\n")
    );
}

/// And the two shapes decode to the SAME `Exp` — which is what makes the migration safe.
#[test]
fn both_shapes_decode_to_the_same_exp() {
    let core = core_layer();
    let tagged = Value::Json(serde_json::json!({
        "ctor": "ConstRef", "args": ["urn:eigenius:core:string", []]
    }));
    let mut res = Resource::new_embedded();
    res.set(
        iri("urn:eigenius:core:is_a"),
        arr("urn:eigenius:eigentt:Term-ConstRef"),
    );
    res.set(
        iri("urn:eigenius:eigentt:Term-ConstRef-iri"),
        Value::String("urn:eigenius:core:string".into()),
    );
    res.set(
        iri("urn:eigenius:eigentt:Term-ConstRef-levels"),
        Value::Array(Vec::new()),
    );

    let from_json = eigenius_kernel::program::eigentt_type_mirror::decode_type(&tagged, &core)
        .expect("the tagged dict decodes");
    let from_resource = eigenius_kernel::program::eigentt_type_mirror::decode_type(
        &Value::Embedded(Box::new(res)),
        &core,
    )
    .expect("the value resource decodes");
    assert_eq!(
        from_json, from_resource,
        "the two shapes must decode to the same Exp, or migrating a value changes its meaning"
    );
}
