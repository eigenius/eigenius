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

//! IO WASM component that invokes CompleteText via `dispatch-component`.
//!
//! Takes a `TextInput` resource with a `text` property. Dispatches to the
//! `CompleteText` component (hosted natively in the orchestrator as a TS
//! handler) with a prompt asking for the text uppercased. Returns a
//! `ShoutedText` resource with the LLM response.
//!
//! This exercises the full IO WASM path:
//!   kernel install → orchestrator register → WASM invoke →
//!   dispatch-component → CompleteText → LLM → back through the stack.

use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-component-io",
});

const TEXT_IN: &str = "urn:example:shout:text";
const SHOUT_OUT: &str = "urn:example:shout:shouted";
const IS_A: &str = "urn:eigenius:core:is_a";
const SHOUTED_TEXT_CLASS: &str = "urn:example:shout:ShoutedText";

const COMPLETE_TEXT_IRI: &str = "urn:eigenius:program:components:CompleteText";
const USER_PROMPT: &str = "urn:eigenius:program:components:completion:user_prompt";
const SYSTEM_PROMPT: &str = "urn:eigenius:program:components:completion:system_prompt";
const REQUEST_PARAMS: &str = "urn:eigenius:program:components:completion:request_parameters";
const MODEL_PROP: &str = "urn:eigenius:program:request:model";
const TEMP_PROP: &str = "urn:eigenius:program:request:temperature";
const MAX_TOKENS_PROP: &str = "urn:eigenius:program:request:max_tokens";

struct HttpShout;

impl Guest for HttpShout {
    fn execute(input: Vec<u8>, _argument: Vec<u8>) -> Result<ComponentResult, String> {
        let input_resource =
            Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;

        let text = input_resource
            .get_string(TEXT_IN)
            .ok_or("input missing 'text' property")?;

        // Build a CompleteText argument resource. Its required property
        // (user_prompt) is a template string; we inject the input text
        // directly here since the template is resolved client-side.
        let mut arg = Resource::new();
        arg.set(
            USER_PROMPT,
            Value::String(format!(
                "Rewrite the following text in ALL CAPS, preserving words exactly. \
                 Respond with only the rewritten text, no commentary.\n\nText: {text}"
            )),
        );
        arg.set(
            SYSTEM_PROMPT,
            Value::String("You are a precise text transformer.".to_string()),
        );
        let mut params = Resource::new();
        params.set(MODEL_PROP, Value::String("claude-haiku-4-5".to_string()));
        params.set(TEMP_PROP, Value::Float(0.0));
        params.set(MAX_TOKENS_PROP, Value::Integer(500));
        arg.set(REQUEST_PARAMS, Value::Embedded(Box::new(params)));

        // CompleteText's handler doesn't use the input (only the argument),
        // so we pass an empty resource.
        let inner_input = Resource::new();

        // Dispatch via the host import.
        let output_bytes = dispatch_component(
            COMPLETE_TEXT_IRI,
            &inner_input.to_cbor(),
            &arg.to_cbor(),
        )
        .map_err(|e| format!("dispatch-component failed: {e}"))?;

        let lllm_out = Resource::from_cbor(&output_bytes)
            .map_err(|e| format!("parse LLM output: {e}"))?;

        // CompleteText returns a resource with a single string property.
        // Pull the text out and wrap in a ShoutedText resource.
        let shouted = lllm_out
            .properties()
            .find_map(|(_, v)| v.as_string().map(str::to_string))
            .unwrap_or_else(|| "(empty LLM response)".to_string());

        let mut output = Resource::new();
        output.set(
            IS_A,
            Value::Array(vec![Value::String(SHOUTED_TEXT_CLASS.into())]),
        );
        output.set(SHOUT_OUT, Value::String(shouted));

        Ok(ComponentResult {
            output: output.to_cbor(),
        })
    }

    fn component_iri() -> String {
        "urn:example:components:HttpShout".to_string()
    }
}

/// Call the host's dispatch-component import.
fn dispatch_component(
    component_iri: &str,
    input: &[u8],
    argument: &[u8],
) -> Result<Vec<u8>, String> {
    use crate::eigenius::component::io_access;
    match io_access::dispatch_component(component_iri, input, argument) {
        Ok(bytes) => Ok(bytes),
        Err(msg) => Err(msg),
    }
}

export!(HttpShout);
