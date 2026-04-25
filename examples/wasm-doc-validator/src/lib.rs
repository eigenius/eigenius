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

// Document validator: a pure WASM component that checks document
// structure using the eigenius-wasm-sdk.
//
// - Input: a Document resource with title, body, section_count
// - Output: a ValidationResult with a boolean 'valid' and optional 'errors' array
//
// Rules:
//   - title must not be empty
//   - body must be at least 100 characters
//   - section_count must be >= 1

use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-component",
});

const TITLE: &str = "urn:example:doc:title";
const BODY: &str = "urn:example:doc:body";
const SECTION_COUNT: &str = "urn:example:doc:section_count";
const VALID: &str = "urn:example:doc:valid";
const ERRORS: &str = "urn:example:doc:errors";
const IS_A: &str = "urn:eigenius:core:is_a";
const VALIDATION_RESULT_CLASS: &str = "urn:example:doc:ValidationResult";

struct DocValidator;

impl Guest for DocValidator {
    fn execute(input: Vec<u8>, _argument: Vec<u8>) -> Result<ComponentResult, String> {
        let doc = Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;

        let mut errors: Vec<String> = Vec::new();

        match doc.get_string(TITLE) {
            Some(t) if t.is_empty() => errors.push("title must not be empty".into()),
            None => errors.push("title is missing".into()),
            _ => {}
        }

        match doc.get_string(BODY) {
            Some(b) if b.len() < 100 => {
                errors.push("body must be at least 100 characters".into())
            }
            None => errors.push("body is missing".into()),
            _ => {}
        }

        match doc.get_integer(SECTION_COUNT) {
            Some(n) if n < 1 => errors.push("must have at least one section".into()),
            None => errors.push("section_count is missing".into()),
            _ => {}
        }

        let mut output = Resource::new();
        output.set(
            IS_A,
            Value::Array(vec![Value::String(VALIDATION_RESULT_CLASS.into())]),
        );
        output.set(VALID, Value::Boolean(errors.is_empty()));
        if !errors.is_empty() {
            output.set(
                ERRORS,
                Value::Array(errors.into_iter().map(Value::String).collect()),
            );
        }

        Ok(ComponentResult {
            output: output.to_cbor(),
        })
    }

    fn component_iri() -> String {
        "urn:example:components:DocValidator".to_string()
    }
}

export!(DocValidator);
