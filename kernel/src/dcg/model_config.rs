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
    /// Anthropic model id.
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
    /// The default configuration with a different model.
    pub fn with_model(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Self::default()
        }
    }
}
