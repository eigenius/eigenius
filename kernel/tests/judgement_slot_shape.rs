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

//! **A judgement is written at the type its slot declares** (D83 §4.2).
//!
//! `justification:judgement` is declared `eigentt:Judgement`, so its value is that
//! inductive's own constructor in D32 §3.7 form. It used to be
//! `CtorApp(eigentt:Judgement, "holds", …)` — an `eigentt:Term` value restating, inside
//! the value, the type the slot already names.
//!
//! That redundancy was not free. Rule 16 walks a `core:inductive` slot against its
//! declared inductive, so it read `App` as the constructor and reported "ctor `App` not
//! declared on InductiveType `eigentt:Judgement`" for every judgement on every chain. P5
//! suppressed the report by exempting `eigentt:Judgement` from Rule 16 entirely. This file
//! pins the shape that makes the exemption unnecessary, and pins that it is gone.

use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::validation::{ValidationRule, Validator};
use std::sync::Arc;

const SRC: &str = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace justification = "urn:eigenius:justification";
namespace jt            = "urn:eigenius:test:judgement";

class jt:Probe {
    description = "a holder for one judgement";
}

resource jt:probe : jt:Probe {
    justification:judgement = type_expr(
        holds( eigentt:logic_kernel,
               declared("urn:eigenius:test:judgement:claim", Prop, UnitVal),
               justification:Certificate(Declared("urn:eigenius:test:judgement:claim"), Prop) )
    );
}
"#;

fn compiled_judgement() -> serde_json::Value {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("in-memory bootstrap");
    let resources = eigenius_kernel::esl::compile_against_layer(SRC, ctx.head())
        .unwrap_or_else(|e| panic!("source must compile: {e:?}"));
    let probe = resources
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str() == "urn:eigenius:test:judgement:probe")
                .unwrap_or(false)
        })
        .expect("jt:probe committed");
    match probe
        .get(&Iri::parse("urn:eigenius:justification:judgement").unwrap())
        .expect("the judgement slot is set")
    {
        Value::Json(j) => j.clone(),
        other => panic!("a judgement is a JSON value, got {other:?}"),
    }
}

/// The slot declares the inductive, so the value names only the constructor.
#[test]
fn a_judgement_is_the_bare_constructor_of_the_inductive_its_slot_declares() {
    let j = compiled_judgement();
    assert_eq!(
        j["ctor"], "holds",
        "the value must be `holds`, not a `CtorApp` naming eigentt:Judgement; got {j}"
    );
    assert_eq!(
        j["args"].as_array().map(Vec::len),
        Some(3),
        "holds(logic, term, type)"
    );
}

/// `holds`'s first argument is typed `eigentt:Logic`, a CLASS. D32 §3.7 encodes a class
/// reference as an IRI string — not as a `ConstRef` term, which is what it became while
/// the whole judgement went through the eigentt codec.
#[test]
fn the_logic_argument_is_a_bare_iri_string() {
    let j = compiled_judgement();
    assert_eq!(
        j["args"][0],
        serde_json::json!("urn:eigenius:eigentt:logic_kernel"),
        "the logic is a class reference, so it is an IRI string"
    );
}

/// The other two arguments are `eigentt:Term`s and keep the D47 encoding — D32 §3.7's
/// argument table dispatches per argument, and this constructor's arguments are not all
/// of one kind.
#[test]
fn the_term_arguments_keep_the_eigentt_encoding() {
    let j = compiled_judgement();
    for i in [1usize, 2] {
        assert!(
            j["args"][i].get("ctor").is_some(),
            "argument {i} must be an encoded term, got {}",
            j["args"][i]
        );
    }
}

fn errors_for(judgement: serde_json::Value) -> Vec<String> {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("in-memory bootstrap");
    let base: Arc<Layer> = Arc::clone(ctx.head());
    let mut top = LayerBuilder::new("judgement_probe", Some(base));
    let mut r = Resource::new(Iri::parse("urn:eigenius:test:judgement:bad").unwrap());
    r.set(
        Iri::parse("urn:eigenius:justification:judgement").unwrap(),
        Value::Json(judgement),
    );
    top.add_resource(r).expect("resource adds");
    let layer = Arc::new(top.build(LayerStorage::in_memory()));
    Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
        .map(|e| e.message)
        .collect()
}

/// The exemption is withdrawn: Rule 16 reads a judgement like any other inductive value,
/// so a constructor `eigentt:Judgement` does not declare is rejected at commit. While the
/// exemption stood, this value reached the chain unexamined.
#[test]
fn rule_16_rejects_a_constructor_the_judgement_inductive_does_not_declare() {
    let errors = errors_for(serde_json::json!({"ctor": "asserts", "args": []}));
    assert!(
        errors
            .iter()
            .any(|m| m.contains("`asserts` not declared") && m.contains("eigentt:Judgement")),
        "expected Rule 16 to name the undeclared ctor and the inductive; got {errors:?}"
    );
}

/// Arity is checked too — `holds` takes three arguments.
#[test]
fn rule_16_rejects_a_judgement_of_the_wrong_arity() {
    let errors = errors_for(serde_json::json!({
        "ctor": "holds",
        "args": ["urn:eigenius:eigentt:logic_kernel"],
    }));
    assert!(
        errors.iter().any(|m| m.contains("expects 3 arg(s), got 1")),
        "expected an arity error, got {errors:?}"
    );
}
