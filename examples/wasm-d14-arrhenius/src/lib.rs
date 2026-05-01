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

//! Arrhenius transformation Component for the D14 worked example
//! (D14 §5.1).
//!
//! Pure scalar transformation Float → Float — the middle of the
//! `dock_to_assay` comorphism. Reads the single Float property off the
//! input wrapper resource (the carrier shape `extract_typed` returns),
//! applies `IC50 (nM) = exp(-ΔG / R·T) · 1e9` with R·T at 310 K, and
//! emits the result as the same single-Float wrapper shape that
//! `reify` consumes.
//!
//! Capability: Pure (no read / no IO). The kernel hosts this directly
//! via Wasmtime — no orchestrator round-trip.

use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-component",
});

const COMPONENT_IRI: &str = "urn:eigenius:demo:d14:cm_arrhenius";
const VALUE_PROP: &str = "urn:eigenius:core:value";

const RT_KCAL_PER_MOL: f64 = 0.616; // R·T at ~310 K, in kcal/mol
const IC50_SCALE_NM: f64 = 1.0e9;

fn arrhenius_ic50_nm(delta_g_kcal: f64) -> f64 {
    (-delta_g_kcal / RT_KCAL_PER_MOL).exp() * IC50_SCALE_NM
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

struct ArrheniusComponent;

impl Guest for ArrheniusComponent {
    fn execute(input: Vec<u8>, _argument: Vec<u8>) -> Result<ComponentResult, String> {
        let r = Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;
        let delta_g = first_float_property(&r).ok_or_else(|| {
            "cm_arrhenius: input wrapper resource carries no Float payload".to_string()
        })?;
        let ic50_nm = arrhenius_ic50_nm(delta_g);

        let mut output = Resource::new();
        output.set(VALUE_PROP, Value::Float(ic50_nm));
        Ok(ComponentResult {
            output: output.to_cbor(),
        })
    }

    fn component_iri() -> String {
        COMPONENT_IRI.to_string()
    }
}

export!(ArrheniusComponent);
