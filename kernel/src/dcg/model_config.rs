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

//! Model selection policy, separate from the transport that applies it.
//!
//! [`super::anthropic_client`] is `use-llm`-gated because it carries an HTTP client. The CONFIG is
//! not: a formalization request names a model whether or not this binary can call one, and a
//! recorded draw names the model that answered it long after the run (D71 §7.1 / §9). Gating the
//! value with the transport would make the request type unbuildable in the default build, which is
//! precisely the build the deterministic replay arms run in.

/// Default reply cap. Structured replies here are rankings and classifications, not prose.
const MAX_TOKENS: u32 = 4096;

/// Reply cap for a model that thinks by default: its thinking counts against `max_tokens`. The
/// largest a non-streaming request should ask for.
const MAX_TOKENS_THINKING: u32 = 16_000;

/// The model id used by the reasoning-layer proposers when none is given (`from_env`). Matches the
/// model the `allms` path used, so behaviour is unchanged apart from the transport.
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

/// How one run's untrusted proposers call the model.
///
/// Carried per RUN rather than compiled in, so a formalization request can select the model and a
/// recorded draw can say which one answered (`enc:draw_model`, D71 §9). A draw is keyed on the
/// QUESTION, not on who answered it, so changing the model does not invalidate a recording — it
/// means the recorded answer is the previous model's, which is exactly what the field records.
///
/// **`temperature` is deliberately NOT a field.** See [`TEMPERATURE`]: it is pinned at 0 because
/// every caller here ranks or classifies rather than writing prose, and sampling made the canonical
/// parse-rate measurement irreproducible between runs of identical code against an identical store.
/// Exposing it would hand a caller a switch that silently destroys that property. Making it
/// configurable is a decision to take deliberately, not a field to add in passing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelConfig {
    /// Model id: an Anthropic model (`claude-…`), or a TypeSafe one (`jev-…`) — the provider follows
    /// from the id ([`ModelConfig::provider`]).
    pub model: String,
    /// Reply cap. Structured replies here are rankings and classifications, not prose, so the
    /// default is generous; a document with very large candidate pools can need more.
    pub max_tokens: u32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            max_tokens: MAX_TOKENS,
        }
    }
}

impl ModelConfig {
    /// The default configuration with a different model, with room for its thinking if it thinks by
    /// default.
    pub fn with_model(model: impl Into<String>) -> Self {
        let model = model.into();
        let max_tokens = if thinks_by_default(&model) {
            MAX_TOKENS_THINKING
        } else {
            MAX_TOKENS
        };
        Self { model, max_tokens }
    }

    /// Who serves the model.
    pub fn provider(&self) -> Provider {
        if self.model.starts_with("jev-") {
            Provider::TypeSafe
        } else {
            Provider::Anthropic
        }
    }

    /// How the model is asked for a structured reply. The forced `emit` tool everywhere it is
    /// accepted — every measurement before eigenius#264 used it — and the JSON-schema output mode on
    /// the models that reject forced tool use with an HTTP 400 ("tool_choice: type tool and any are
    /// not supported for this model"). The schema mode is not offered on `claude-sonnet-4-6`.
    pub fn structured_output(&self) -> StructuredOutput {
        if FORCED_TOOL_REJECTED.contains(&self.model.as_str()) {
            StructuredOutput::JsonSchema
        } else {
            StructuredOutput::ForcedTool
        }
    }

    /// Whether the model takes `temperature`. Pinned at 0 where it does; from Opus 4.7 and in the
    /// Claude 5 family a sampling parameter is an HTTP 400, so those models run at their default.
    pub fn accepts_temperature(&self) -> bool {
        !SAMPLING_REJECTED.iter().any(|p| self.model.starts_with(p))
    }
}

/// Who serves a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    TypeSafe,
}

/// How a model is asked for a structured reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructuredOutput {
    /// One `emit` tool whose input schema is the reply's, forced with `tool_choice`.
    ForcedTool,
    /// `output_config.format` with the reply's JSON schema.
    JsonSchema,
}

/// The models that reject a forced `tool_choice` (the Claude API reference, 2026-09-25).
const FORCED_TOOL_REJECTED: &[&str] = &[
    "claude-fable-5-1",
    "claude-mythos-5-1",
    "claude-opus-5-5",
    "claude-sonnet-5-5",
];

/// The model id prefixes that reject a sampling parameter.
const SAMPLING_REJECTED: &[&str] = &[
    "claude-opus-4-7",
    "claude-opus-4-8",
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-fable-",
    "claude-mythos-",
];

/// The model families that think unless told not to.
fn thinks_by_default(model: &str) -> bool {
    [
        "claude-opus-5",
        "claude-sonnet-5",
        "claude-fable-",
        "claude-mythos-",
    ]
    .iter()
    .any(|p| model.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// eigenius#264: the Claude 5 models that reject forced tool use get the schema mode and no
    /// temperature; every earlier measurement's model keeps the forced tool at temperature 0.
    #[test]
    fn each_model_is_asked_the_way_it_accepts() {
        let sonnet46 = ModelConfig::with_model("claude-sonnet-4-6");
        assert_eq!(sonnet46.structured_output(), StructuredOutput::ForcedTool);
        assert!(sonnet46.accepts_temperature());
        assert_eq!(sonnet46.max_tokens, MAX_TOKENS);
        let sonnet55 = ModelConfig::with_model("claude-sonnet-5-5");
        assert_eq!(sonnet55.structured_output(), StructuredOutput::JsonSchema);
        assert!(!sonnet55.accepts_temperature());
        assert_eq!(sonnet55.max_tokens, MAX_TOKENS_THINKING);
        assert_eq!(sonnet55.provider(), Provider::Anthropic);
        assert_eq!(
            ModelConfig::with_model("jev-latest").provider(),
            Provider::TypeSafe
        );
    }
}
