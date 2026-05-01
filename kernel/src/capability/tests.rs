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

//! Tests for WASM capability hosting.
//!
//! These tests use pre-built guest components at
//! `kernel/tests/fixtures/*.wasm`:
//!   - eigenius_wasm_doc_validator.wasm: document validator using the SDK
//!
//! D14 institution-side host-bridge tests live in
//! `kernel/src/capability/wasm_institution_d14.rs::tests`.

use super::wasm_component::{CapabilityLevel, WasmComponent, WasmComponentConfig};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::program::component::BuiltinComponent;

fn empty_layer() -> crate::layer::Layer {
    crate::layer::LayerBuilder::new("empty", None).build(crate::layer::LayerStorage::in_memory())
}

#[test]
fn wasm_component_default_config() {
    let cfg = WasmComponentConfig::default();
    assert_eq!(cfg.fuel_limit, 10_000_000);
    assert_eq!(cfg.memory_limit_pages, 1024);
}

#[test]
fn wasm_component_rejects_invalid_binary() {
    let bogus = b"not a wasm binary";
    let result =
        WasmComponent::from_bytes(bogus, CapabilityLevel::Pure, WasmComponentConfig::default());
    match result {
        Err(e) => assert!(e.contains("compilation failed"), "got: {e}"),
        Ok(_) => panic!("expected compilation failure for bogus bytes"),
    }
}

// --- Document validator integration tests ---
// Uses the eigenius-wasm-doc-validator component (built with the SDK) to
// verify real CBOR round-trips, nested types, and validation logic.

fn load_doc_validator() -> WasmComponent {
    let wasm_bytes =
        include_bytes!("../../../kernel/tests/fixtures/eigenius_wasm_doc_validator.wasm");
    WasmComponent::from_bytes(
        wasm_bytes,
        CapabilityLevel::Pure,
        WasmComponentConfig::default(),
    )
    .expect("failed to load doc validator")
}

const TITLE: &str = "urn:example:doc:title";
const BODY: &str = "urn:example:doc:body";
const SECTION_COUNT: &str = "urn:example:doc:section_count";
const VALID: &str = "urn:example:doc:valid";
const ERRORS: &str = "urn:example:doc:errors";

#[test]
fn doc_validator_has_correct_iri() {
    let component = load_doc_validator();
    assert_eq!(component.iri(), "urn:example:components:DocValidator");
}

#[test]
fn doc_validator_accepts_valid_document() {
    let component = load_doc_validator();
    let layer = empty_layer();

    let long_body = "a".repeat(150);
    let mut input = Resource::new_embedded();
    input.set(Iri::parse(TITLE).unwrap(), Value::String("My Doc".into()));
    input.set(Iri::parse(BODY).unwrap(), Value::String(long_body));
    input.set(Iri::parse(SECTION_COUNT).unwrap(), Value::Integer(3));

    let result = component.execute(&input, None, &layer).unwrap();
    let valid = result
        .output
        .get(&Iri::parse(VALID).unwrap())
        .unwrap()
        .as_boolean();
    assert_eq!(valid, Some(true));
    assert!(result.output.get(&Iri::parse(ERRORS).unwrap()).is_none());
}

#[test]
fn doc_validator_rejects_empty_title() {
    let component = load_doc_validator();
    let layer = empty_layer();

    let long_body = "a".repeat(150);
    let mut input = Resource::new_embedded();
    input.set(Iri::parse(TITLE).unwrap(), Value::String("".into()));
    input.set(Iri::parse(BODY).unwrap(), Value::String(long_body));
    input.set(Iri::parse(SECTION_COUNT).unwrap(), Value::Integer(3));

    let result = component.execute(&input, None, &layer).unwrap();
    let valid = result
        .output
        .get(&Iri::parse(VALID).unwrap())
        .unwrap()
        .as_boolean();
    assert_eq!(valid, Some(false));

    // Errors should be present and mention the title
    let errors_val = result.output.get(&Iri::parse(ERRORS).unwrap()).unwrap();
    let errors = match errors_val {
        Value::Array(items) => items,
        _ => panic!("expected array of errors"),
    };
    assert!(!errors.is_empty());
    let has_title_error = errors
        .iter()
        .any(|v| v.as_str().map(|s| s.contains("title")).unwrap_or(false));
    assert!(has_title_error);
}

#[test]
fn doc_validator_rejects_short_body() {
    let component = load_doc_validator();
    let layer = empty_layer();

    let mut input = Resource::new_embedded();
    input.set(Iri::parse(TITLE).unwrap(), Value::String("OK".into()));
    input.set(Iri::parse(BODY).unwrap(), Value::String("too short".into()));
    input.set(Iri::parse(SECTION_COUNT).unwrap(), Value::Integer(1));

    let result = component.execute(&input, None, &layer).unwrap();
    let valid = result
        .output
        .get(&Iri::parse(VALID).unwrap())
        .unwrap()
        .as_boolean();
    assert_eq!(valid, Some(false));
}

#[test]
fn doc_validator_rejects_zero_sections() {
    let component = load_doc_validator();
    let layer = empty_layer();

    let long_body = "a".repeat(150);
    let mut input = Resource::new_embedded();
    input.set(Iri::parse(TITLE).unwrap(), Value::String("OK".into()));
    input.set(Iri::parse(BODY).unwrap(), Value::String(long_body));
    input.set(Iri::parse(SECTION_COUNT).unwrap(), Value::Integer(0));

    let result = component.execute(&input, None, &layer).unwrap();
    let valid = result
        .output
        .get(&Iri::parse(VALID).unwrap())
        .unwrap()
        .as_boolean();
    assert_eq!(valid, Some(false));
}

#[test]
fn doc_validator_collects_multiple_errors() {
    let component = load_doc_validator();
    let layer = empty_layer();

    let mut input = Resource::new_embedded();
    input.set(Iri::parse(TITLE).unwrap(), Value::String("".into()));
    input.set(Iri::parse(BODY).unwrap(), Value::String("short".into()));
    input.set(Iri::parse(SECTION_COUNT).unwrap(), Value::Integer(0));

    let result = component.execute(&input, None, &layer).unwrap();
    let errors = match result.output.get(&Iri::parse(ERRORS).unwrap()).unwrap() {
        Value::Array(items) => items,
        _ => panic!("expected errors array"),
    };
    // All three rules should fire
    assert_eq!(errors.len(), 3);
}

#[test]
fn doc_validator_output_is_typed() {
    let component = load_doc_validator();
    let layer = empty_layer();

    let long_body = "a".repeat(150);
    let mut input = Resource::new_embedded();
    input.set(Iri::parse(TITLE).unwrap(), Value::String("OK".into()));
    input.set(Iri::parse(BODY).unwrap(), Value::String(long_body));
    input.set(Iri::parse(SECTION_COUNT).unwrap(), Value::Integer(1));

    let result = component.execute(&input, None, &layer).unwrap();

    // Output should have is_a = [ValidationResult]
    let is_a_iris = result.output.is_a();
    assert!(is_a_iris
        .iter()
        .any(|iri| iri.as_str() == "urn:example:doc:ValidationResult"));
}

#[test]
fn wasm_component_respects_low_fuel() {
    // Very low fuel should trap during component_iri extraction or execute.
    let wasm_bytes =
        include_bytes!("../../../kernel/tests/fixtures/eigenius_wasm_doc_validator.wasm");
    let component = WasmComponent::from_bytes(
        wasm_bytes,
        CapabilityLevel::Pure,
        WasmComponentConfig {
            fuel_limit: 100,
            memory_limit_pages: 1024,
        },
    );

    match component {
        Err(e) => {
            assert!(
                e.contains("fuel")
                    || e.contains("trap")
                    || e.contains("instantiation")
                    || e.contains("component-iri"),
                "expected fuel-related error, got: {e}"
            );
        }
        Ok(c) => {
            let layer = empty_layer();
            let long_body = "a".repeat(150);
            let mut input = Resource::new_embedded();
            input.set(Iri::parse(TITLE).unwrap(), Value::String("OK".into()));
            input.set(Iri::parse(BODY).unwrap(), Value::String(long_body));
            input.set(Iri::parse(SECTION_COUNT).unwrap(), Value::Integer(1));
            let result = c.execute(&input, None, &layer);
            assert!(
                result.is_err(),
                "expected fuel exhaustion error during execute"
            );
        }
    }
}

#[test]
fn wasm_component_fresh_instance_per_invocation() {
    // D12 §6.2 — each invocation must use a fresh Wasmtime instance
    // so no mutable state leaks between calls.
    let component = load_doc_validator();
    let layer = empty_layer();
    let long_body = "a".repeat(150);

    for i in 0..3 {
        let mut input = Resource::new_embedded();
        input.set(
            Iri::parse(TITLE).unwrap(),
            Value::String(format!("Doc {i}")),
        );
        input.set(Iri::parse(BODY).unwrap(), Value::String(long_body.clone()));
        input.set(Iri::parse(SECTION_COUNT).unwrap(), Value::Integer(1));

        let result = component.execute(&input, None, &layer).unwrap();
        let valid = result
            .output
            .get(&Iri::parse(VALID).unwrap())
            .unwrap()
            .as_boolean();
        assert_eq!(valid, Some(true), "iteration {i} should be valid");
    }
}
