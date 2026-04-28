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

//! D14 smoke-test institution.
//!
//! Targets the `eigenius-institution-d14` WIT world (D14 §11). The
//! three exports do simple pass-through with provenance markers so
//! tests on the host side can verify that the wire format and
//! procedure-IRI dispatch round-trip end-to-end:
//!
//! - `extract_typed` echoes the input resource as the typed-value
//!   payload, with a `provenance` property recording the procedure
//!   IRI it was dispatched on.
//! - `reify` echoes the typed-value back as a resource, similarly
//!   tagged.
//! - `query` echoes the input resource with a marker.
//!
//! The institution declares no real domain logic — it exists only to
//! prove the dispatch path is wired correctly. M5 will exercise this
//! fixture from the InstitutionInvoke evaluator; M8 will replace it
//! with the rewritten `wasm-ordering-institution`.

use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-institution-d14",
});

const PROVENANCE_PROP: &str = "urn:eigenius:test:d14_echo:provenance";
const STAGE_PROP: &str = "urn:eigenius:test:d14_echo:stage";

struct EchoInstitution;

impl Guest for EchoInstitution {
    fn extract_typed(procedure_iri: String, input: Vec<u8>) -> Result<Vec<u8>, String> {
        let mut r = Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;
        r.set(PROVENANCE_PROP, Value::String(procedure_iri));
        r.set(STAGE_PROP, Value::String("extract_typed".into()));
        Ok(r.to_cbor())
    }

    fn reify(procedure_iri: String, value: Vec<u8>) -> Result<Vec<u8>, String> {
        let mut r = Resource::from_cbor(&value).map_err(|e| format!("parse value: {e}"))?;
        r.set(PROVENANCE_PROP, Value::String(procedure_iri));
        r.set(STAGE_PROP, Value::String("reify".into()));
        Ok(r.to_cbor())
    }

    fn query(procedure_iri: String, input: Vec<u8>) -> Result<Vec<u8>, String> {
        let mut r = Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;
        r.set(PROVENANCE_PROP, Value::String(procedure_iri));
        r.set(STAGE_PROP, Value::String("query".into()));
        Ok(r.to_cbor())
    }
}

export!(EchoInstitution);
