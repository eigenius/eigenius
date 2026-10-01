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

//! A minimal direct Anthropic Messages API client for **structured output** — the one thing the
//! reasoning-layer LLM calls (sense ranker, reading ranker, anaphora and abbreviation proposers)
//! need. Replaces the `allms` `Completions::get_answer` path, which prompt-injected the JSON schema
//! and then `serde_json::from_str`'d the model's free-form text: the model could emit a valid JSON
//! object *and then* trailing commentary ("Wait, let me recheck…"), which broke the deserializer
//! and silently degraded that call to its fallback.
//!
//! Two transports, chosen per model ([`ModelConfig::structured_output`]): forcing `tool_choice`
//! onto a single `emit` tool whose `input_schema` is the reply's JSON Schema, which makes the model
//! return a `tool_use` block whose `input` the API itself parses; or, on the models that reject
//! forced tool use (the Claude 5 generation, eigenius#264), `output_config.format` with the same
//! schema. Neither admits surrounding prose. Feature-gated behind `use-llm`. The provider-neutral
//! interface the reading ranker uses is [`super::decision`]; this is one transport under it.

pub use super::model_config::{ModelConfig, StructuredOutput, DEFAULT_MODEL};

use schemars::{schema_for, JsonSchema};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const TOOL_NAME: &str = "emit";

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

/// Call Anthropic for a reply of type `T`: its JSON Schema derived by `schemars`, the reply
/// deserialized into `T`. Async — callers run it on their own tokio runtime (as the proposers
/// already do). `Err(String)` on any transport / API / decode failure, so every caller can fail
/// closed (the LLM only ever *proposes*; the kernel gates).
pub async fn anthropic_structured<T: JsonSchema + DeserializeOwned>(
    api_key: &str,
    cfg: &ModelConfig,
    prompt: &str,
) -> Result<T, String> {
    let schema = serde_json::to_value(schema_for!(T)).map_err(|e| e.to_string())?;
    let reply = anthropic_json(api_key, cfg, prompt, schema).await?;
    serde_json::from_value(reply).map_err(|e| format!("reply did not match the schema: {e}"))
}

/// Call Anthropic for a JSON reply conforming to `schema`, in the way the model accepts
/// ([`ModelConfig::structured_output`]): the forced `emit` tool, whose `tool_use.input` the API
/// parses, or `output_config.format`, whose text block is the JSON. Either way no surrounding prose
/// is possible. Temperature 0 where the model takes it ([`ModelConfig::accepts_temperature`]).
pub async fn anthropic_json(
    api_key: &str,
    cfg: &ModelConfig,
    prompt: &str,
    mut schema: Value,
) -> Result<Value, String> {
    // Anthropic wants the schema object itself, not a meta-schema reference.
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$schema");
    }
    let mut body = json!({
        "model": cfg.model,
        "max_tokens": cfg.max_tokens,
        "messages": [{ "role": "user", "content": prompt }],
    });
    if cfg.accepts_temperature() {
        body["temperature"] = json!(TEMPERATURE);
    }
    let structured = cfg.structured_output();
    match structured {
        StructuredOutput::ForcedTool => {
            body["tools"] = json!([{
                "name": TOOL_NAME,
                "description": "Emit the structured result.",
                "input_schema": schema,
            }]);
            body["tool_choice"] = json!({ "type": "tool", "name": TOOL_NAME });
        }
        StructuredOutput::JsonSchema => {
            restrict_for_output_format(&mut schema);
            body["output_config"] =
                json!({ "format": { "type": "json_schema", "schema": schema } });
        }
    }

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
    if let Some(stop @ ("refusal" | "max_tokens")) =
        payload.get("stop_reason").and_then(Value::as_str)
    {
        return Err(format!("anthropic stopped on {stop}: {payload}"));
    }
    let blocks = payload
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("no content in response: {payload}"))?;
    match structured {
        // The forced tool call: the `tool_use` block named `emit`, and its `input`.
        StructuredOutput::ForcedTool => blocks
            .iter()
            .find(|b| {
                b.get("type").and_then(Value::as_str) == Some("tool_use")
                    && b.get("name").and_then(Value::as_str) == Some(TOOL_NAME)
            })
            .and_then(|b| b.get("input"))
            .cloned()
            .ok_or_else(|| format!("no `{TOOL_NAME}` tool_use in response: {payload}")),
        // The schema-constrained reply: the text block, after any thinking blocks.
        StructuredOutput::JsonSchema => {
            let text = blocks
                .iter()
                .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|b| b.get("text"))
                .and_then(Value::as_str)
                .ok_or_else(|| format!("no text block in response: {payload}"))?;
            serde_json::from_str(text).map_err(|e| format!("reply text is not JSON: {e}: {text}"))
        }
    }
}

/// The JSON-schema output mode takes a subset of JSON Schema: every object closed
/// (`additionalProperties: false`), and no numeric, string-length or array-size constraints, nor
/// formats beyond its own list. `schemars` emits `minimum: 0` and `format: "uint"` for a `usize`;
/// the bound is the caller's to check (every caller already range-checks an index it is given).
fn restrict_for_output_format(schema: &mut Value) {
    const DROPPED: &[&str] = &[
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "uniqueItems",
    ];
    const FORMATS: &[&str] = &[
        "date-time",
        "time",
        "date",
        "duration",
        "email",
        "hostname",
        "uri",
        "ipv4",
        "ipv6",
        "uuid",
    ];
    match schema {
        Value::Object(obj) => {
            for k in DROPPED {
                obj.remove(*k);
            }
            if obj
                .get("format")
                .and_then(Value::as_str)
                .is_some_and(|f| !FORMATS.contains(&f))
            {
                obj.remove("format");
            }
            if obj.get("type").and_then(Value::as_str) == Some("object")
                || obj.contains_key("properties")
            {
                obj.insert("additionalProperties".into(), Value::Bool(false));
            }
            for v in obj.values_mut() {
                restrict_for_output_format(v);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(restrict_for_output_format),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `usize` field's bound and format go, and every object is closed — the schema-mode subset.
    #[test]
    fn a_schemars_schema_is_restricted_to_the_output_format_subset() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "chosen": { "type": ["integer", "null"], "format": "uint", "minimum": 0.0 },
                "at": { "type": "string", "format": "date" },
                "nested": { "type": "object", "properties": { "n": { "type": "integer" } } },
            },
        });
        restrict_for_output_format(&mut schema);
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(
            schema["properties"]["nested"]["additionalProperties"],
            json!(false)
        );
        assert!(schema["properties"]["chosen"].get("minimum").is_none());
        assert!(schema["properties"]["chosen"].get("format").is_none());
        assert_eq!(schema["properties"]["at"]["format"], json!("date"));
    }
}
