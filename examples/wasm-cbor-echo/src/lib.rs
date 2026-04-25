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

//! Test fixture: decodes the input resource via the SDK (ciborium) and
//! re-encodes it unchanged. Lets us verify that bytes encoded by one CBOR
//! library survive a round-trip through the other.
//!
//! Used by orchestration tests to cross-check cbor-x ↔ ciborium interop
//! for value types the regular suites don't exercise (floats, booleans,
//! nulls, large integers, empty collections, non-ASCII strings).

use eigenius_wasm_sdk::Resource;

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-component-io",
});

struct Echo;

impl Guest for Echo {
    fn execute(input: Vec<u8>, _argument: Vec<u8>) -> Result<ComponentResult, String> {
        let parsed =
            Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;
        Ok(ComponentResult {
            output: parsed.to_cbor(),
        })
    }

    fn component_iri() -> String {
        "urn:test:components:CborEcho".to_string()
    }
}

export!(Echo);
