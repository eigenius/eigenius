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
//!   - eigenius_wasm_ordering_institution.wasm: ordering/refinement institution

use super::wasm_component::{CapabilityLevel, WasmComponent, WasmComponentConfig};
use super::wasm_institution::WasmFiberReasoner;
use crate::context::{ExecutionContext, ExecutionMode};
use crate::institution::error::MorphismValidation;
use crate::institution::FiberReasoner;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::program::component::BuiltinComponent;
use std::sync::Arc;

fn empty_layer() -> crate::layer::Layer {
    crate::layer::LayerBuilder::new("empty", None).build(
        std::sync::Arc::new(crate::layer::MemoryResourceCache::new()),
        std::sync::Arc::new(crate::layer::MemoryResourceBackend::new()),
    )
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

// --- Ordering institution integration tests ---

const INSTITUTION_IRI: &str = "urn:eigenius:test:wasm:ordering";
const REFINEMENT_CLASS: &str = "urn:eigenius:test:wasm:Refinement";
const CONVERGENCE_QUERY_CLASS: &str = "urn:eigenius:test:wasm:ConvergenceQuery";
const DELTA_PROP: &str = "urn:eigenius:test:wasm:delta";
const TOLERANCE_PROP: &str = "urn:eigenius:test:wasm:tolerance";
const LATEST_DELTA_PROP: &str = "urn:eigenius:test:wasm:latest_delta";
const CONVERGED_PROP: &str = "urn:eigenius:test:wasm:converged";
const CHECKED_DELTA_PROP: &str = "urn:eigenius:test:wasm:checked_delta";

fn load_ordering_institution() -> WasmFiberReasoner {
    let wasm_bytes =
        include_bytes!("../../../kernel/tests/fixtures/eigenius_wasm_ordering_institution.wasm");
    WasmFiberReasoner::from_bytes(wasm_bytes, WasmComponentConfig::default())
        .expect("failed to load ordering institution")
}

fn test_context() -> ExecutionContext {
    let cache = std::sync::Arc::new(crate::layer::MemoryResourceCache::new())
        as std::sync::Arc<dyn crate::layer::ResourceCache>;
    let backend = std::sync::Arc::new(crate::layer::MemoryResourceBackend::new())
        as std::sync::Arc<dyn crate::storage::ResourceBackend>;
    let layer = Arc::new(crate::layer::LayerBuilder::new("empty", None).build(
        std::sync::Arc::clone(&cache),
        std::sync::Arc::clone(&backend),
    ));
    ExecutionContext::new(layer, "test", ExecutionMode::ReadOnly, cache, backend)
}

#[test]
fn wasm_institution_extracts_declaration() {
    let reasoner = load_ordering_institution();
    let decl = reasoner.fiber_declaration();
    assert_eq!(decl.institution_iri.as_str(), INSTITUTION_IRI);
    assert_eq!(decl.name, "WASM Ordering Institution");
    assert_eq!(decl.morphism_types.len(), 1);
    assert_eq!(decl.query_types.len(), 1);

    // Verify the morphism type is the Refinement class
    let morphism = &decl.morphism_types[0];
    assert_eq!(morphism.id().unwrap().as_str(), REFINEMENT_CLASS);
}

#[test]
fn wasm_institution_iri_accessor() {
    let reasoner = load_ordering_institution();
    assert_eq!(reasoner.institution_iri().as_str(), INSTITUTION_IRI);
}

#[test]
fn wasm_institution_validates_positive_delta() {
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    let mut morphism = Resource::new_embedded();
    morphism.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(REFINEMENT_CLASS.into())]),
    );
    morphism.set(Iri::parse(DELTA_PROP).unwrap(), Value::Float(0.5));

    let result = reasoner.validate_morphism(&morphism, &ctx).unwrap();
    assert!(matches!(result, MorphismValidation::Valid));
}

#[test]
fn wasm_institution_rejects_zero_delta() {
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    let mut morphism = Resource::new_embedded();
    morphism.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(REFINEMENT_CLASS.into())]),
    );
    morphism.set(Iri::parse(DELTA_PROP).unwrap(), Value::Float(0.0));

    let result = reasoner.validate_morphism(&morphism, &ctx).unwrap();
    match result {
        MorphismValidation::Invalid(reason) => {
            assert!(reason.contains("positive"), "got: {reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn wasm_institution_rejects_negative_delta() {
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    let mut morphism = Resource::new_embedded();
    morphism.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(REFINEMENT_CLASS.into())]),
    );
    morphism.set(Iri::parse(DELTA_PROP).unwrap(), Value::Integer(-5));

    let result = reasoner.validate_morphism(&morphism, &ctx).unwrap();
    assert!(matches!(result, MorphismValidation::Invalid(_)));
}

#[test]
fn wasm_institution_rejects_missing_delta() {
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    let mut morphism = Resource::new_embedded();
    morphism.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(REFINEMENT_CLASS.into())]),
    );
    // No delta property

    let result = reasoner.validate_morphism(&morphism, &ctx).unwrap();
    match result {
        MorphismValidation::Invalid(reason) => {
            assert!(reason.contains("delta"), "got: {reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn wasm_institution_query_returns_converged_when_delta_below_tolerance() {
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    let mut query = Resource::new_embedded();
    query.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(CONVERGENCE_QUERY_CLASS.into())]),
    );
    query.set(Iri::parse(TOLERANCE_PROP).unwrap(), Value::Float(0.01));
    query.set(Iri::parse(LATEST_DELTA_PROP).unwrap(), Value::Float(0.001));

    let result = reasoner.query(&query, &ctx).unwrap();
    let converged = result
        .get(&Iri::parse(CONVERGED_PROP).unwrap())
        .unwrap()
        .as_boolean();
    assert_eq!(converged, Some(true));

    // Verify the institution echoed the inputs it checked
    let checked = result
        .get(&Iri::parse(CHECKED_DELTA_PROP).unwrap())
        .unwrap()
        .as_float();
    assert_eq!(checked, Some(0.001));
}

#[test]
fn wasm_institution_query_returns_not_converged_above_tolerance() {
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    let mut query = Resource::new_embedded();
    query.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(CONVERGENCE_QUERY_CLASS.into())]),
    );
    query.set(Iri::parse(TOLERANCE_PROP).unwrap(), Value::Float(0.01));
    query.set(Iri::parse(LATEST_DELTA_PROP).unwrap(), Value::Float(0.5));

    let result = reasoner.query(&query, &ctx).unwrap();
    let converged = result
        .get(&Iri::parse(CONVERGED_PROP).unwrap())
        .unwrap()
        .as_boolean();
    assert_eq!(converged, Some(false));
}

#[test]
fn wasm_institution_query_fails_on_missing_parameter() {
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    // Omit the required latest_delta parameter
    let mut query = Resource::new_embedded();
    query.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(CONVERGENCE_QUERY_CLASS.into())]),
    );
    query.set(Iri::parse(TOLERANCE_PROP).unwrap(), Value::Float(0.01));

    let result = reasoner.query(&query, &ctx);
    assert!(
        result.is_err(),
        "expected error for missing query parameter"
    );
}

#[test]
fn wasm_institution_rejects_unknown_query() {
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    let mut query = Resource::new_embedded();
    query.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String("urn:example:UnknownQuery".into())]),
    );

    let result = reasoner.query(&query, &ctx);
    assert!(result.is_err(), "expected error for unknown query type");
}

#[test]
fn wasm_institution_discover_returns_empty() {
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    // Pass a few arbitrary resources — this institution doesn't discover anything
    let mut r1 = Resource::new_embedded();
    r1.set(
        Iri::parse("urn:example:foo").unwrap(),
        Value::String("bar".into()),
    );

    let discovered = reasoner.discover_morphisms(&[r1], &ctx).unwrap();
    assert!(discovered.is_empty());
}

#[test]
fn wasm_institution_can_validate_repeatedly() {
    // Fresh instance per call (D12 §6.2) — verify multiple validations work
    let reasoner = load_ordering_institution();
    let ctx = test_context();

    for delta in [0.1, 0.5, 1.0] {
        let mut morphism = Resource::new_embedded();
        morphism.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String(REFINEMENT_CLASS.into())]),
        );
        morphism.set(Iri::parse(DELTA_PROP).unwrap(), Value::Float(delta));

        let result = reasoner.validate_morphism(&morphism, &ctx).unwrap();
        assert!(
            matches!(result, MorphismValidation::Valid),
            "delta {delta} should be valid"
        );
    }
}

// ---------------------------------------------------------------------------
// #10 — EigenQL FIBER clause end-to-end (D2 §3.5, §6.12)
// ---------------------------------------------------------------------------

/// Build a layer containing:
///   - the ordering institution's query_types (from its fiber declaration),
///     so the EigenQL type checker can validate FIBER short-name params
///   - a Refinement class + two Refinement instances with different deltas,
///     so MATCH has something to iterate over
fn build_fiber_test_layer() -> (
    std::sync::Arc<crate::layer::Layer>,
    crate::institution::InstitutionRegistry,
) {
    let reasoner = load_ordering_institution();
    let mut registry = crate::institution::InstitutionRegistry::new();
    let published = registry.register(Box::new(reasoner)).unwrap();

    let mut lb = crate::layer::LayerBuilder::new("fiber-test", None);

    // The institution's query_types (incl. ConvergenceQuery with its
    // required-property declarations) — needed by the type checker.
    for res in published {
        lb.add_resource(res).unwrap();
    }

    // The institution publishes its query class but not the Property
    // resources its `requires` list references. The EigenQL type checker
    // needs those Properties (specifically their `short_name`) to
    // validate FIBER's short-name params (tolerance, latest_delta).
    for (iri, short) in [
        (TOLERANCE_PROP, "tolerance"),
        (LATEST_DELTA_PROP, "latest_delta"),
    ] {
        let mut p = Resource::new(Iri::parse(iri).unwrap());
        p.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String("urn:eigenius:core:Property".into())]),
        );
        p.set(
            Iri::parse("urn:eigenius:core:short_name").unwrap(),
            Value::String(short.into()),
        );
        p.set(
            Iri::parse("urn:eigenius:core:data_type").unwrap(),
            Value::String("urn:eigenius:core:float".into()),
        );
        lb.add_resource(p).unwrap();
    }

    // A Refinement class (so MATCH Refinement(?m) resolves).
    let ref_class_iri = Iri::parse(REFINEMENT_CLASS).unwrap();
    let mut ref_class = Resource::new(ref_class_iri.clone());
    ref_class.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
    );
    ref_class.set(
        Iri::parse("urn:eigenius:core:short_name").unwrap(),
        Value::String("Refinement".into()),
    );
    // Declare delta as a required property so the evaluator can
    // short-name-resolve `delta: ?d` inside a Refinement MATCH.
    let delta_prop_iri = Iri::parse(DELTA_PROP).unwrap();
    ref_class.set(
        Iri::parse("urn:eigenius:core:requires").unwrap(),
        Value::Array(vec![Value::String(DELTA_PROP.into())]),
    );
    lb.add_resource(ref_class).unwrap();

    // The delta property resource (required so short-name resolution works).
    let mut delta_prop = Resource::new(delta_prop_iri.clone());
    delta_prop.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String("urn:eigenius:core:Property".into())]),
    );
    delta_prop.set(
        Iri::parse("urn:eigenius:core:short_name").unwrap(),
        Value::String("delta".into()),
    );
    delta_prop.set(
        Iri::parse("urn:eigenius:core:data_type").unwrap(),
        Value::String("urn:eigenius:core:float".into()),
    );
    lb.add_resource(delta_prop).unwrap();

    // Two Refinement instances — one with delta well below the tolerance
    // we'll use (converged), one above it (not converged).
    for (suffix, delta) in [("converged", 0.005), ("far", 0.5)] {
        let iri = Iri::parse(&format!("urn:test:refinement:{suffix}")).unwrap();
        let mut r = Resource::new(iri);
        r.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String(REFINEMENT_CLASS.into())]),
        );
        r.set(delta_prop_iri.clone(), Value::Float(delta));
        lb.add_resource(r).unwrap();
    }

    (
        std::sync::Arc::new(lb.build(
            std::sync::Arc::new(crate::layer::MemoryResourceCache::new()),
            std::sync::Arc::new(crate::layer::MemoryResourceBackend::new()),
        )),
        registry,
    )
}

#[test]
fn fiber_clause_converged_only() {
    use crate::context::{ExecutionContext, ExecutionMode};
    use crate::query;

    let (layer, institutions) = build_fiber_test_layer();
    let cache = Arc::clone(layer.cache());
    let backend = Arc::clone(layer.backend());
    let ctx = ExecutionContext::new(
        Arc::clone(&layer),
        "fiber-test",
        ExecutionMode::ReadOnly,
        cache,
        backend,
    );

    let runtime = query::evaluate::FiberRuntime {
        institutions: Some(&institutions),
        ctx: Some(&ctx),
    };

    // Query: for each Refinement, dispatch to the ordering institution's
    // ConvergenceQuery; project the binding that reports `converged: true`.
    let query_str = r#"
        USING INSTITUTION "urn:eigenius:test:wasm:ordering" AS ord
        USING "urn:eigenius:test:wasm:Refinement"
        MATCH Refinement(?m) { delta: ?d }
        FIBER ord:ConvergenceQuery { tolerance: 0.01, latest_delta: ?d } AS ?conv
        MATCH ?conv { "urn:eigenius:test:wasm:converged": ?c }
        WHERE ?c = true
        RETURN [] { m: ?m }
    "#;

    let document =
        query::execute_with(query_str, &layer, runtime).expect("FIBER query should succeed");

    // Expect exactly one row — the Refinement whose delta is within tolerance.
    let result_set = document
        .iter()
        .find(
            |r| match r.get(&Iri::parse("urn:eigenius:core:is_a").unwrap()) {
                Some(Value::Array(a)) => a
                    .iter()
                    .any(|v| matches!(v, Value::String(s) if s == "urn:eigenius:query:ResultSet")),
                _ => false,
            },
        )
        .expect("document must contain a ResultSet");
    let row_count = match result_set.get(&Iri::parse("urn:eigenius:query:row_count").unwrap()) {
        Some(Value::Integer(n)) => *n,
        _ => panic!("missing row_count"),
    };
    assert_eq!(row_count, 1, "expected exactly the converged refinement");

    // The single row's `m` property should reference the converged Refinement.
    let rows = match result_set.get(&Iri::parse("urn:eigenius:query:rows").unwrap()) {
        Some(Value::Array(a)) => a,
        _ => panic!("missing rows"),
    };
    let row = match &rows[0] {
        Value::Embedded(r) => r,
        _ => panic!("row must be embedded"),
    };
    // Find the row Property with short_name "m" and read the value under it.
    let m_prop_iri = document
        .iter()
        .find(|r| {
            matches!(r.get(&Iri::parse("urn:eigenius:core:short_name").unwrap()),
            Some(Value::String(s)) if s == "m")
        })
        .and_then(|r| r.id().cloned())
        .expect("Property 'm' must exist");
    let m_value = row.get(&m_prop_iri).expect("row must have 'm'");
    let m_str = match m_value {
        Value::String(s) => s.as_str(),
        Value::ResourceRef(i) => i.as_str(),
        other => panic!("unexpected m value: {other:?}"),
    };
    assert_eq!(m_str, "urn:test:refinement:converged");
}

#[test]
fn fiber_clause_missing_required_param_rejected_by_type_checker() {
    use crate::query;

    let (layer, _institutions) = build_fiber_test_layer();

    // `latest_delta` is required but omitted here.
    let query_str = r#"
        USING INSTITUTION "urn:eigenius:test:wasm:ordering" AS ord
        MATCH ?m { "urn:eigenius:test:wasm:delta": ?d }
        FIBER ord:ConvergenceQuery { tolerance: 0.01 } AS ?conv
        RETURN [] { conv: ?conv }
    "#;

    let result = query::execute(query_str, &layer);
    let errors = result.expect_err("type checker should reject missing required param");
    assert!(
        errors
            .iter()
            .any(|e| e.rule == "fiber_missing_required_param"),
        "expected fiber_missing_required_param error, got {errors:?}"
    );
}
