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

//! Dock institution for the D14 worked example (D14 §5.1).
//!
//! Source side of the dock→assay comorphism. Targets the
//! `eigenius-institution-d14` WIT world with one boundary export
//! (`extract_typed`) implementing the `ef_dock_to_dg` ExportFormat
//! procedure: read a `DockingResult` resource and extract its
//! `delta_g` (kcal/mol) as a Float-typed payload.
//!
//! The reify and query exports are stubs — Dock is source-side only;
//! it neither reifies nor answers queries. The reified target lives in
//! the assay institution; the QueryClasses live there too.

use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-institution-d14",
});

const DELTA_G_PROP: &str = "urn:eigenius:demo:d14:delta_g";
const VALUE_PROP: &str = "urn:eigenius:core:value";
const EXTRACT_DG_PROC: &str = "urn:eigenius:demo:d14:proc:extract_dg";

struct DockInstitution;

impl Guest for DockInstitution {
    fn extract_typed(procedure_iri: String, input: Vec<u8>) -> Result<Vec<u8>, String> {
        if procedure_iri != EXTRACT_DG_PROC {
            return Err(format!(
                "dock institution does not implement procedure `{procedure_iri}`"
            ));
        }
        let resource = Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;
        let delta_g = resource
            .get(DELTA_G_PROP)
            .and_then(|v| v.as_float())
            .or_else(|| resource.get(DELTA_G_PROP).and_then(|v| v.as_integer()).map(|n| n as f64))
            .ok_or_else(|| {
                format!("DockingResult is missing required `{DELTA_G_PROP}` (Float)")
            })?;

        // M4 marshalling: typed value is wrapped as a single-Float
        // resource (the kernel's Val::ResourceVal carrier shape).
        let mut wrapper = Resource::new();
        wrapper.set(VALUE_PROP, Value::Float(delta_g));
        Ok(wrapper.to_cbor())
    }

    fn reify(procedure_iri: String, _value: Vec<u8>) -> Result<Vec<u8>, String> {
        Err(format!(
            "dock institution does not implement reify (`{procedure_iri}`)"
        ))
    }

    fn query(procedure_iri: String, _input: Vec<u8>) -> Result<Vec<u8>, String> {
        Err(format!(
            "dock institution does not implement query (`{procedure_iri}`)"
        ))
    }
}

export!(DockInstitution);
