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

//! A minimal direct Anthropic Messages API client for **structured output via forced tool-use** —
//! the one thing the reasoning-layer LLM calls (sense ranker, anaphora proposer, abbreviation
//! proposer) need. Replaces the `allms` `Completions::get_answer` path, which prompt-injected the
//! JSON schema and then `serde_json::from_str`'d the model's free-form text: the model could emit a
//! valid JSON object *and then* trailing commentary ("Wait, let me recheck…"), which broke the
//! deserializer and silently degraded that call to its fallback.
//!
//! Forcing `tool_choice` onto a single `emit` tool whose `input_schema` is the JSON Schema of `T`
//! makes the model return a `tool_use` block whose `input` is a JSON object the API itself parses —
//! **no surrounding prose is possible**. We only target Anthropic, so this is a ~one-endpoint client
//! (no multi-provider abstraction), feature-gated behind `use-llm`.

use schemars::{schema_for, JsonSchema};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

/// The model id used by the reasoning-layer proposers when none is given (`from_env`). Matches the
/// model the `allms` path used, so behaviour is unchanged apart from the transport.
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const TOOL_NAME: &str = "emit";
const MAX_TOKENS: u32 = 4096;

/// **Temperature 0 — pinned, and load-bearing for reproducibility.**
///
/// Every caller here asks the model to *rank* or *classify* (which senses survive the cap, which
/// anaphor binds where), not to write prose. We want the model's best ordering, not a sample from
/// its distribution. The Messages API defaults to `temperature: 1.0`, so omitting this field made
/// the sense reranker — and therefore the canonical parse-rate measurement it feeds — **randomized
/// between runs of identical code against an identical store**: different senses survive the cap →
/// a different chart → a different parse. A measurement that cannot be reproduced is not a
/// measurement.
///
/// (0 buys stability, not a guarantee: the API does not promise bitwise determinism. The
/// reproducible *gate* is the cap-only arm, `measure-parse-rate.sh --no-llm`, which uses no LLM at
/// all; the reranked run is the headline number.)
const TEMPERATURE: f32 = 0.0;

/// Call Anthropic with a forced single-tool `emit` whose `input_schema` is the JSON Schema derived
/// from `T`, and deserialize the returned `tool_use.input` into `T`. Async — callers run it on their
/// own tokio runtime (as the proposers already do). `Err(String)` on any transport / API / decode
/// failure, so every caller can fail closed (the LLM only ever *proposes*; the kernel gates).
pub async fn anthropic_structured<T: JsonSchema + DeserializeOwned>(
    api_key: &str,
    model: &str,
    prompt: &str,
) -> Result<T, String> {
    // JSON Schema of the reply type. Strip `$schema` — Anthropic's `input_schema` wants the schema
    // object itself, not a meta-schema reference.
    let mut schema = serde_json::to_value(schema_for!(T)).map_err(|e| e.to_string())?;
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$schema");
    }
    let body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "temperature": TEMPERATURE,
        "tools": [{
            "name": TOOL_NAME,
            "description": "Emit the structured result.",
            "input_schema": schema,
        }],
        "tool_choice": { "type": "tool", "name": TOOL_NAME },
        "messages": [{ "role": "user", "content": prompt }],
    });

    let resp = reqwest::Client::new()
        .post(API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("anthropic request failed: {e}"))?;

    let status = resp.status();
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| format!("anthropic response not JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!("anthropic API {status}: {payload}"));
    }

    // The forced tool call: find the `tool_use` block named `emit` and take its `input`.
    let input = payload
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| {
            blocks.iter().find(|b| {
                b.get("type").and_then(Value::as_str) == Some("tool_use")
                    && b.get("name").and_then(Value::as_str) == Some(TOOL_NAME)
            })
        })
        .and_then(|b| b.get("input"))
        .ok_or_else(|| format!("no `{TOOL_NAME}` tool_use in response: {payload}"))?;

    serde_json::from_value(input.clone())
        .map_err(|e| format!("tool_use input did not match schema: {e}"))
}
