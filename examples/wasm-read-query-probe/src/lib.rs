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

//! Test fixture that exercises the `read-access.resolve` and
//! `query-access.query` host imports. Used by the orchestrator's native-addon
//! tests to verify the full callback wiring beyond `dispatch-component`.
//!
//! On `execute`, the guest:
//!   1. Calls `resolve(input["urn:test:probe:resolve_iri"])`
//!   2. Calls `query(input["urn:test:probe:query_text"])`
//!   3. Returns a resource summarising what the host returned:
//!        urn:test:probe:resolved_len  = bytes length of resolved resource, or -1 if None
//!        urn:test:probe:query_rows    = number of rows returned by the query

use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-component-io",
});

const RESOLVE_IRI_PROP: &str = "urn:test:probe:resolve_iri";
const QUERY_TEXT_PROP: &str = "urn:test:probe:query_text";
const RESOLVED_LEN_PROP: &str = "urn:test:probe:resolved_len";
const QUERY_ROWS_PROP: &str = "urn:test:probe:query_rows";

struct Probe;

impl Guest for Probe {
    fn execute(input: Vec<u8>, _argument: Vec<u8>) -> Result<ComponentResult, String> {
        let input_resource =
            Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;

        let resolve_iri = input_resource
            .get_string(RESOLVE_IRI_PROP)
            .ok_or_else(|| format!("input missing '{RESOLVE_IRI_PROP}' property"))?;
        let query_text = input_resource
            .get_string(QUERY_TEXT_PROP)
            .ok_or_else(|| format!("input missing '{QUERY_TEXT_PROP}' property"))?;

        let resolved_len: i64 = match eigenius::component::read_access::resolve(resolve_iri) {
            Some(bytes) => bytes.len() as i64,
            None => -1,
        };

        let query_rows: i64 = match eigenius::component::query_access::query(query_text) {
            Ok(rows) => rows.len() as i64,
            Err(msg) => return Err(format!("query failed: {msg}")),
        };

        let mut output = Resource::new();
        output.set(RESOLVED_LEN_PROP, Value::Integer(resolved_len));
        output.set(QUERY_ROWS_PROP, Value::Integer(query_rows));

        Ok(ComponentResult {
            output: output.to_cbor(),
        })
    }

    fn component_iri() -> String {
        "urn:test:components:ReadQueryProbe".to_string()
    }
}

export!(Probe);
