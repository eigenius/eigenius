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

//! eigenius#142 — a `program:Literal` must carry its value all the way
//! to the caller of `execute_program_nbe`, not decode to the literal's
//! primitive type.
//!
//! Before the fix `parse_literal` returned `Exp::EigonPrimitive(T)` for
//! every string / integer / float / boolean literal, and
//! `val_to_resource_value` had no arm for the literal `Val`s, so a
//! `Construct` field fed by a literal came back as
//! `Embedded(Resource { properties: {} })` — an empty resource, for
//! every literal, with no error raised.

use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::program::component::ComponentRegistry;
use eigenius_kernel::program::eval_io::execute_program_nbe;
use std::sync::Arc;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

fn empty_layer() -> Arc<eigenius_kernel::layer::Layer> {
    Arc::new(LayerBuilder::new("empty", None).build(LayerStorage::in_memory()))
}

/// A program that ignores its input and constructs a resource whose
/// fields are literals of each primitive kind.
fn literal_program_json(count: &str, label: &str, ratio: &str, flag: &str) -> String {
    format!(
        r#"{{
            "@id": "urn:eigenius:test:litprog",
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
            "urn:eigenius:program:body": {{
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Construct"],
                "urn:eigenius:program:class": "urn:eigenius:example:Reading",
                "urn:eigenius:program:fields": {{
                    "urn:eigenius:example:count": {{
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Literal"],
                        "urn:eigenius:program:value": {count}
                    }},
                    "urn:eigenius:example:label": {{
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Literal"],
                        "urn:eigenius:program:value": {label}
                    }},
                    "urn:eigenius:example:ratio": {{
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Literal"],
                        "urn:eigenius:program:value": {ratio}
                    }},
                    "urn:eigenius:example:flag": {{
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Literal"],
                        "urn:eigenius:program:value": {flag}
                    }}
                }}
            }}
        }}"#
    )
}

fn run(json: &str) -> Resource {
    let program = eigon_json::parse_document(json).unwrap().remove(0);
    let input = Resource::new_embedded();
    let registry = Arc::new(ComponentRegistry::default());
    execute_program_nbe(&program, &input, empty_layer(), registry, None)
        .expect("program executes")
        .output
}

/// Caller-level: the literal's value, not its type, reaches
/// `NbeExecutionResult::output`.
#[test]
fn literal_values_reach_the_program_output() {
    let out = run(&literal_program_json("42", r#""hello""#, "1.5", "true"));

    assert_eq!(
        out.get(&iri("urn:eigenius:example:count")),
        Some(&Value::Integer(42)),
        "integer literal must arrive as 42"
    );
    assert_eq!(
        out.get(&iri("urn:eigenius:example:label")),
        Some(&Value::String("hello".to_string())),
        "string literal must arrive as \"hello\""
    );
    assert_eq!(
        out.get(&iri("urn:eigenius:example:ratio")),
        Some(&Value::Float(1.5)),
        "float literal must arrive as 1.5"
    );

    // eigenius#142, still open: there is no `Exp::LitBool`, so a
    // boolean literal decodes to `Exp::EigonPrimitive(Boolean)` and
    // marshals to an empty embedded resource. Pinned so the gap is
    // visible and so adding `LitBool` forces this assertion to be
    // updated to `Value::Boolean(true)`.
    assert!(
        matches!(
            out.get(&iri("urn:eigenius:example:flag")),
            Some(Value::Embedded(r)) if r.properties().is_empty()
        ),
        "boolean literals have no value-carrying term (eigenius#142), got {:?}",
        out.get(&iri("urn:eigenius:example:flag"))
    );
}

/// Two programs differing only in their constants must not produce the
/// same output. This is the failure mode #142 describes: same term,
/// same value, indistinguishable to the checker and to any memo keyed
/// on the term.
#[test]
fn programs_with_different_constants_produce_different_outputs() {
    let a = run(&literal_program_json("42", r#""alpha""#, "1.5", "true"));
    let b = run(&literal_program_json("7", r#""beta""#, "2.5", "true"));

    assert_ne!(
        a.get(&iri("urn:eigenius:example:count")),
        b.get(&iri("urn:eigenius:example:count"))
    );
    assert_ne!(
        a.get(&iri("urn:eigenius:example:label")),
        b.get(&iri("urn:eigenius:example:label"))
    );
    assert_ne!(
        a.get(&iri("urn:eigenius:example:ratio")),
        b.get(&iri("urn:eigenius:example:ratio"))
    );
}

/// Term level: the decoded body must be the value-carrying `Lit*`
/// constructors, and two different constants must be different terms.
#[test]
fn literal_terms_carry_their_payload() {
    let layer = empty_layer();

    let program =
        eigon_json::parse_document(&literal_program_json("42", r#""hello""#, "1.5", "true"))
            .unwrap()
            .remove(0);
    // `parse_program` needs input/output types on the chain; go
    // through the body directly, which is what `execute_program_nbe`
    // parses too.
    let body = match program.get(&iri("urn:eigenius:program:body")) {
        Some(Value::Embedded(b)) => b.as_ref().clone(),
        other => panic!("body must be embedded, got {other:?}"),
    };
    let exp = eigenius_kernel::program::expr::parse_expression(&body, &layer).unwrap();
    let fields = match exp {
        Exp::Construct(_, fields) => fields,
        other => panic!("expected Construct, got {other:?}"),
    };
    let field = |name: &str| {
        fields
            .iter()
            .find(|(p, _)| p.as_str() == name)
            .map(|(_, e)| e.as_ref().clone())
            .unwrap()
    };

    assert_eq!(field("urn:eigenius:example:count"), Exp::LitInt(42));
    assert_eq!(
        field("urn:eigenius:example:label"),
        Exp::LitString("hello".to_string())
    );
    assert_eq!(field("urn:eigenius:example:ratio"), Exp::LitFloat(1.5));
}

/// A string literal that is a `urn:` / `http` IRI still decodes to
/// `Exp::Var` — the pre-canonicalisation resource-reference heuristic.
/// Unchanged by #142; pinned so the fix is not read as having removed
/// it.
#[test]
fn iri_shaped_string_literal_still_decodes_as_a_reference() {
    let mut r = Resource::new_embedded();
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::String(
            "urn:eigenius:program:Literal".to_string(),
        )]),
    );
    r.set(
        iri("urn:eigenius:program:value"),
        Value::String("urn:eigenius:example:thing".to_string()),
    );
    let exp = eigenius_kernel::program::expr::parse_expression(&r, &empty_layer()).unwrap();
    assert_eq!(
        exp,
        Exp::Var("urn:eigenius:example:thing".to_string()),
        "IRI-shaped string literals remain resource references"
    );
}
