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

//! Ordering/refinement institution implemented as a WASM component.
//!
//! Mirrors the kernel's test `OrderingInstitution` (in
//! `kernel/src/institution/mod.rs`) but hosted via `WasmFiberReasoner`.
//!
//! Defines:
//!   - `Refinement` morphism class with `source`, `target`, `delta` properties
//!   - `ConvergenceQuery` query class with `tolerance` and `latest_delta`
//!     parameters — demonstrates how queries can carry typed arguments
//!
//! Validates that a Refinement's `delta` is strictly positive.
//! Answers ConvergenceQuery by comparing `latest_delta <= tolerance`.
//! discover_morphisms returns an empty list.

use eigenius_wasm_sdk::institution::FiberDeclaration;
use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-institution",
});

const INSTITUTION_IRI: &str = "urn:eigenius:test:wasm:ordering";
const REFINEMENT_CLASS: &str = "urn:eigenius:test:wasm:Refinement";
const CONVERGENCE_QUERY_CLASS: &str = "urn:eigenius:test:wasm:ConvergenceQuery";

// Refinement properties
const SOURCE: &str = "urn:eigenius:test:wasm:source";
const TARGET: &str = "urn:eigenius:test:wasm:target";
const DELTA: &str = "urn:eigenius:test:wasm:delta";

// ConvergenceQuery parameters
const TOLERANCE: &str = "urn:eigenius:test:wasm:tolerance";
const LATEST_DELTA: &str = "urn:eigenius:test:wasm:latest_delta";

// ConvergenceQuery result properties
const CONVERGED: &str = "urn:eigenius:test:wasm:converged";
const CHECKED_DELTA: &str = "urn:eigenius:test:wasm:checked_delta";
const CHECKED_TOLERANCE: &str = "urn:eigenius:test:wasm:checked_tolerance";

struct OrderingInstitution;

impl Guest for OrderingInstitution {
    fn fiber_declaration() -> Vec<u8> {
        // Refinement morphism class
        let mut refinement = Resource::with_id(REFINEMENT_CLASS);
        refinement.set_is_a(["urn:eigenius:core:Class"]);
        refinement.set(
            "urn:eigenius:core:short_name",
            Value::String("Refinement".into()),
        );
        refinement.set(
            "urn:eigenius:core:description",
            Value::String("A refinement morphism between two results.".into()),
        );
        refinement.set(
            "urn:eigenius:core:requires",
            Value::Array(vec![
                Value::String(SOURCE.into()),
                Value::String(TARGET.into()),
                Value::String(DELTA.into()),
            ]),
        );

        // ConvergenceQuery class — declares its parameters as required
        // properties. The kernel validates these structurally before
        // the query reaches this institution.
        let mut query_class = Resource::with_id(CONVERGENCE_QUERY_CLASS);
        query_class.set_is_a(["urn:eigenius:core:Class"]);
        query_class.set(
            "urn:eigenius:core:short_name",
            Value::String("ConvergenceQuery".into()),
        );
        query_class.set(
            "urn:eigenius:core:description",
            Value::String(
                "Query whether the latest refinement step converged below a tolerance.".into(),
            ),
        );
        query_class.set(
            "urn:eigenius:core:requires",
            Value::Array(vec![
                Value::String(TOLERANCE.into()),
                Value::String(LATEST_DELTA.into()),
            ]),
        );

        // Declare the properties referenced from the `requires` lists
        // above so the kernel's class-definition validator
        // (eigenius#26) can resolve them. The kernel commits these
        // alongside the morphism + query classes when the institution
        // is installed.
        let structural_properties = vec![
            property(SOURCE, "source", "Source endpoint of a refinement.", "urn:eigenius:core:resource"),
            property(TARGET, "target", "Target endpoint of a refinement.", "urn:eigenius:core:resource"),
            property(DELTA, "delta", "Magnitude of the refinement step.", "urn:eigenius:core:float"),
            property(TOLERANCE, "tolerance", "Convergence tolerance threshold.", "urn:eigenius:core:float"),
            property(LATEST_DELTA, "latest_delta", "Most recent observed delta to compare against the tolerance.", "urn:eigenius:core:float"),
        ];

        let decl = FiberDeclaration {
            institution_iri: INSTITUTION_IRI.into(),
            name: "WASM Ordering Institution".into(),
            morphism_types: vec![refinement],
            query_types: vec![query_class],
            structural_properties,
        };

        decl.into_resource().to_cbor()
    }

    fn query(q: Vec<u8>) -> Result<Vec<u8>, String> {
        let query = Resource::from_cbor(&q).map_err(|e| format!("parse query: {e}"))?;

        if !query.is_a().iter().any(|i| *i == CONVERGENCE_QUERY_CLASS) {
            return Err(format!("unknown query type: {:?}", query.is_a()));
        }

        // Pull out the query parameters. Accept both float and integer
        // encodings since CBOR may round-trip numeric literals either way.
        let tolerance = extract_number(&query, TOLERANCE)
            .ok_or_else(|| format!("missing or non-numeric '{TOLERANCE}' parameter"))?;
        let latest_delta = extract_number(&query, LATEST_DELTA)
            .ok_or_else(|| format!("missing or non-numeric '{LATEST_DELTA}' parameter"))?;

        if tolerance < 0.0 {
            return Err(format!("tolerance must be non-negative, got {tolerance}"));
        }

        let converged = latest_delta.abs() <= tolerance;

        let mut result = Resource::new();
        result.set(CONVERGED, Value::Boolean(converged));
        // Echo the inputs so callers can confirm what was compared.
        result.set(CHECKED_DELTA, Value::Float(latest_delta));
        result.set(CHECKED_TOLERANCE, Value::Float(tolerance));
        Ok(result.to_cbor())
    }

    fn validate_morphism(morphism: Vec<u8>) -> Result<(ValidationResult, String), String> {
        let m = Resource::from_cbor(&morphism).map_err(|e| format!("parse morphism: {e}"))?;

        match extract_number(&m, DELTA) {
            Some(d) if d > 0.0 => Ok((ValidationResult::Valid, String::new())),
            Some(d) => Ok((
                ValidationResult::Invalid,
                format!("delta must be positive, got {d}"),
            )),
            None => Ok((
                ValidationResult::Invalid,
                "missing delta property".to_string(),
            )),
        }
    }

    fn discover_morphisms(_resources: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>, String> {
        // This institution does not discover new morphisms.
        Ok(Vec::new())
    }
}

/// Read a numeric property as f64, accepting both Float and Integer encodings.
fn extract_number(resource: &Resource, property: &str) -> Option<f64> {
    match resource.get(property)? {
        Value::Float(f) => Some(*f),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

/// Build a `core:Property` resource declaration for `iri` with the given
/// short-name, description, and data-type. Used to declare the
/// properties referenced from this institution's morphism / query
/// `requires` lists so the kernel can resolve them at validation time.
fn property(iri: &str, short_name: &str, description: &str, data_type: &str) -> Resource {
    let mut r = Resource::with_id(iri);
    r.set_is_a(["urn:eigenius:core:Property"]);
    r.set("urn:eigenius:core:short_name", Value::String(short_name.into()));
    r.set("urn:eigenius:core:description", Value::String(description.into()));
    r.set("urn:eigenius:core:data_type", Value::String(data_type.into()));
    r
}

export!(OrderingInstitution);
