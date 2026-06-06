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

//! Assay institution for the D14 worked example (D14 §5.1).
//!
//! Target side of the dock→assay comorphism. Targets the
//! `eigenius-institution-d14` WIT world and implements:
//!
//! - `reify` — `if_assay_from_ic50` ImportFormat procedure: take a
//!   Float-typed payload and construct an `AssayPrediction` resource
//!   with the value as `ic50` (nM).
//! - `query` — handles three QueryClass procedures owned by this
//!   institution:
//!   - `within_tolerance` (Decidable) — three-arg predicate over IC50,
//!     target IC50, and tolerance; returns Verdict.
//!   - `assay_prediction_validity` (AutoOnLoad) — single-arg check
//!     that an AssayPrediction has positive IC50; returns Verdict.
//!   - `validate_prediction` (OnDemand) — wraps an AssayPrediction
//!     candidate in the same validity check, surfaced via FIBER.
//!
//! `extract_typed` is a stub — Assay is target-side only.

use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-institution-d14",
});

const IS_A: &str = "urn:eigenius:core:is_a";
const VERDICT_CLASS: &str = "urn:eigenius:institution:Verdict";
const CTOR_NAME: &str = "urn:eigenius:core:ctor_name";

const VALUE_PROP: &str = "urn:eigenius:core:value";
const ASSAY_PREDICTION_CLASS: &str = "urn:eigenius:demo:d14:AssayPrediction";
const IC50_PROP: &str = "urn:eigenius:demo:d14:ic50";
const PREDICTED_IC50_PROP: &str = "urn:eigenius:demo:d14:predicted_ic50";
const TARGET_IC50_PROP: &str = "urn:eigenius:demo:d14:target_ic50";
const TOLERANCE_PROP: &str = "urn:eigenius:demo:d14:tolerance";
const CANDIDATE_PROP: &str = "urn:eigenius:demo:d14:candidate";

const REIFY_IC50_PROC: &str = "urn:eigenius:demo:d14:proc:reify_ic50";
const WITHIN_TOLERANCE_PROC: &str = "urn:eigenius:demo:d14:proc:within_tolerance";
const CHECK_ASSAY_PREDICTION_PROC: &str = "urn:eigenius:demo:d14:proc:check_assay_prediction";
const VALIDATE_PREDICTION_PROC: &str = "urn:eigenius:demo:d14:proc:validate_prediction";

struct AssayInstitution;

fn extract_float(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    v.as_float()
        .or_else(|| v.as_integer().map(|n| n as f64))
        .or_else(|| {
            // Nested wrapper resource carrying a single Float (the
            // EigenTT typed-value carrier shape).
            v.as_embedded().and_then(first_float_property)
        })
}

fn first_float_property(resource: &Resource) -> Option<f64> {
    for (_, v) in resource.properties() {
        if let Some(f) = v.as_float() {
            return Some(f);
        }
        if let Some(n) = v.as_integer() {
            return Some(n as f64);
        }
    }
    None
}

fn verdict_resource(ctor: &str) -> Resource {
    let mut r = Resource::new();
    r.set(IS_A, Value::Array(vec![Value::String(VERDICT_CLASS.into())]));
    r.set(CTOR_NAME, Value::String(ctor.into()));
    r
}

fn assay_prediction_verdict(input: &Resource) -> &'static str {
    match input.get(IC50_PROP).and_then(|v| v.as_float()) {
        Some(v) if v.is_finite() && v > 0.0 => "Holds",
        Some(_) => "Fails",
        None => "Undecidable",
    }
}

fn within_tolerance_verdict(input: &Resource) -> &'static str {
    // Decidable QueryClass dispatch (D14 §9.2): the kernel populates
    // the input class's typed required properties from positional
    // ESL args in `requires` declaration order (Phase 19d.7). For
    // `WithinToleranceInput` the kernel sets `predicted_ic50`,
    // `target_ic50`, `tolerance` from `decide(predicted, target,
    // tol)`. Args arrive as wrapper resources (the kernel marshals
    // `Val::ResourceVal` as `Value::Embedded`), so `extract_float`
    // digs through the wrapper.
    let predicted = extract_float(input.get(PREDICTED_IC50_PROP));
    let target = extract_float(input.get(TARGET_IC50_PROP));
    let tolerance = extract_float(input.get(TOLERANCE_PROP));
    match (predicted, target, tolerance) {
        (Some(p), Some(t), Some(tol)) if tol >= 0.0 => {
            if (p - t).abs() <= tol {
                "Holds"
            } else {
                "Fails"
            }
        }
        _ => "Undecidable",
    }
}

impl Guest for AssayInstitution {
    fn extract_typed(procedure_iri: String, _input: Vec<u8>) -> Result<Vec<u8>, String> {
        Err(format!(
            "assay institution does not implement extract_typed (`{procedure_iri}`)"
        ))
    }

    fn reify(procedure_iri: String, value: Vec<u8>) -> Result<Vec<u8>, String> {
        if procedure_iri != REIFY_IC50_PROC {
            return Err(format!(
                "assay institution does not implement procedure `{procedure_iri}`"
            ));
        }
        let payload = Resource::from_cbor(&value).map_err(|e| format!("parse value: {e}"))?;
        let ic50 = first_float_property(&payload)
            .or_else(|| payload.get(VALUE_PROP).and_then(|v| v.as_float()))
            .ok_or_else(|| "assay reify: payload carries no Float".to_string())?;

        let mut prediction = Resource::new();
        prediction.set(
            IS_A,
            Value::Array(vec![Value::String(ASSAY_PREDICTION_CLASS.into())]),
        );
        prediction.set(IC50_PROP, Value::Float(ic50));
        Ok(prediction.to_cbor())
    }

    fn query(procedure_iri: String, input: Vec<u8>) -> Result<Vec<u8>, String> {
        let resource = Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;
        let ctor = match procedure_iri.as_str() {
            WITHIN_TOLERANCE_PROC => within_tolerance_verdict(&resource),
            CHECK_ASSAY_PREDICTION_PROC => assay_prediction_verdict(&resource),
            VALIDATE_PREDICTION_PROC => {
                // OnDemand: input.candidate is the AssayPrediction.
                let candidate = resource
                    .get(CANDIDATE_PROP)
                    .and_then(|v| v.as_embedded())
                    .ok_or_else(|| {
                        "validate_prediction: input is missing `candidate` (Embedded resource)"
                            .to_string()
                    })?;
                assay_prediction_verdict(candidate)
            }
            other => {
                return Err(format!(
                    "assay institution does not implement procedure `{other}`"
                ));
            }
        };
        Ok(verdict_resource(ctor).to_cbor())
    }
}

export!(AssayInstitution);
