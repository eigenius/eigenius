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

//! **An inductive value can name another value instead of containing it** (D83 §3.2/§3.3).
//!
//! This is the asymmetry D32 §3.7 left behind: it gave CLASS-typed arguments both "a
//! `ResourceRef` … or an embedded resource map" and gave inductive-typed arguments only the
//! inline form. A value becomes nameable by acquiring an `@id` and declaring its type —
//! the same relationship `Embedded` and `ResourceRef` already have for resources.
//!
//! The property that makes a reference a SHARING device rather than a distinct term is
//! that it expands: a referencing value and its fully inlined twin are the same value, so
//! they decode to the same `Exp` and hash to the same witness key. Anything else would make
//! deduplication change witness keys and break live citations.

use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::program::eigentt_type_mirror::decode_type;
use eigenius_kernel::validation::{ValidationRule, Validator};
use eigenius_kernel::witness::hash_proposition_exp;
use std::sync::Arc;

const TERM: &str = "urn:eigenius:eigentt:Term";
const PROP: &str = "urn:eigenius:reflection:canonical_proposition";

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// `UnitVal`, a nullary `eigentt:Term` constructor — the simplest thing a value resource
/// can hold. `Fst` below is the one-argument constructor whose slot is declared
/// `eigentt:Term`, so a reference standing in that slot goes through the term decoder.
fn unit_inline() -> serde_json::Value {
    serde_json::json!({"ctor": "UnitVal", "args": []})
}

/// A §3.2 chain-resident value: an `@id`, the InductiveType it inhabits, its constructor
/// and its arguments.
fn value_resource(id: &str, ctor: &str, args: Vec<Value>) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(TERM))]),
    );
    r.set(iri("urn:eigenius:core:ctor"), Value::String(ctor.into()));
    r.set(iri("urn:eigenius:core:args"), Value::Array(args));
    r
}

fn build(resources: Vec<Resource>) -> Arc<Layer> {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("in-memory bootstrap");
    let mut b = LayerBuilder::new("value-ref-probe", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("resource adds");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn errors(layer: &Arc<Layer>) -> Vec<String> {
    Validator::new(Arc::clone(layer))
        .validate()
        .into_iter()
        .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
        .map(|e| e.message)
        .collect()
}

/// A holder whose `canonical_proposition` is `Fst(<target>)` — the argument reached
/// through a reference rather than inlined.
fn holder_naming(target: &str) -> Resource {
    let mut h = Resource::new(iri("urn:eigenius:test:vref:holder"));
    h.set(
        iri(PROP),
        Value::Json(serde_json::json!({"ctor": "Fst", "args": [target]})),
    );
    h
}

/// Rule 24: a resource whose `is_a` names an InductiveType is a VALUE of it, and validates
/// against that inductive's constructor schema.
#[test]
fn a_chain_resident_value_validates_against_its_inductive() {
    let layer = build(vec![value_resource(
        "urn:eigenius:test:vref:one",
        "Fst",
        vec![Value::Json(unit_inline())],
    )]);
    assert!(errors(&layer).is_empty(), "{:?}", errors(&layer));
}

/// A value has ONE type. The array shape of `is_a` is inherited from resources generally,
/// not a licence to give a value two — a walk cannot check one value against two schemas.
#[test]
fn a_value_resource_may_not_declare_two_types() {
    let mut r = value_resource("urn:eigenius:test:vref:two", "UnitVal", vec![]);
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![
            Value::ResourceRef(iri(TERM)),
            Value::ResourceRef(iri("urn:eigenius:core:Class")),
        ]),
    );
    let layer = build(vec![r]);
    assert!(
        errors(&layer)
            .iter()
            .any(|m| m.contains("exactly one type")),
        "{:?}",
        errors(&layer)
    );
}

/// Declaring the type without the constructor leaves nothing to expand.
#[test]
fn a_value_resource_without_a_constructor_is_rejected() {
    let mut r = value_resource("urn:eigenius:test:vref:bare", "UnitVal", vec![]);
    r.remove(&iri("urn:eigenius:core:ctor"));
    let layer = build(vec![r]);
    assert!(
        errors(&layer).iter().any(|m| m.contains("`core:ctor`")),
        "{:?}",
        errors(&layer)
    );
}

/// The reference and its inlined twin are the SAME value: same `Exp`, same witness hash.
/// This is what makes a reference a sharing device rather than a distinct term.
#[test]
fn a_reference_and_its_inlined_twin_are_one_value() {
    let layer = build(vec![
        value_resource("urn:eigenius:test:vref:unit", "UnitVal", vec![]),
        holder_naming("urn:eigenius:test:vref:unit"),
    ]);
    let referencing = decode_type(
        &Value::Json(serde_json::json!({
            "ctor": "Fst", "args": ["urn:eigenius:test:vref:unit"],
        })),
        &layer,
    )
    .expect("the referencing value decodes");
    let twin = decode_type(
        &Value::Json(serde_json::json!({"ctor": "Fst", "args": [unit_inline()]})),
        &layer,
    )
    .expect("the inlined twin decodes");
    assert_eq!(referencing, twin, "a reference expands to its target");
    assert_eq!(
        hash_proposition_exp(&referencing).unwrap(),
        hash_proposition_exp(&twin).unwrap(),
        "expansion identity: deduplication must not change a witness key"
    );
}

/// A reference into thin air is a malformed value, not a silently empty one.
///
/// Reported by Rule 21, not Rule 16: the holding slot is `reflection:canonical_proposition`,
/// ranged at `eigentt:Term`, and Rule 16 skips that type so the two do not produce duplicate
/// diagnostics. Rule 21 decodes, and decode is where a reference is expanded.
#[test]
fn a_reference_to_a_resource_that_is_not_there_is_rejected() {
    let layer = build(vec![holder_naming("urn:eigenius:test:vref:absent")]);
    let all: Vec<String> = Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| format!("{:?}: {}", e.rule, e.message))
        .collect();
    assert!(
        all.iter().any(|m| m.contains("is not in the chain")),
        "{all:?}"
    );
}

/// A cycle does not terminate under expansion, and a value's identity IS its expansion.
/// The validator must say so rather than hang — it runs at commit, before any decode.
#[test]
fn a_reference_cycle_is_rejected_by_the_validator() {
    let a = value_resource(
        "urn:eigenius:test:vref:a",
        "Fst",
        vec![Value::String("urn:eigenius:test:vref:b".into())],
    );
    let b = value_resource(
        "urn:eigenius:test:vref:b",
        "Fst",
        vec![Value::String("urn:eigenius:test:vref:a".into())],
    );
    let layer = build(vec![a, b]);
    assert!(
        errors(&layer).iter().any(|m| m.contains("cycle")),
        "{:?}",
        errors(&layer)
    );
}

/// And the decoder says so too, since it is what would otherwise hash a value that does not
/// terminate. Cycle detection has to run BEFORE hashing (D83 §3.3).
#[test]
fn a_reference_cycle_is_rejected_by_the_decoder() {
    let a = value_resource(
        "urn:eigenius:test:vref:ca",
        "Fst",
        vec![Value::String("urn:eigenius:test:vref:cb".into())],
    );
    let b = value_resource(
        "urn:eigenius:test:vref:cb",
        "Fst",
        vec![Value::String("urn:eigenius:test:vref:ca".into())],
    );
    let layer = build(vec![a, b]);
    let err = decode_type(
        &Value::Json(serde_json::json!("urn:eigenius:test:vref:ca")),
        &layer,
    )
    .expect_err("a cyclic reference must not decode");
    assert!(
        format!("{err}").contains("cycle"),
        "expected a cycle diagnostic, got {err}"
    );
}
