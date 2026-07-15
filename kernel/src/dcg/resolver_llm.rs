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

//! D64 §4 — a **live-LLM** [`Proposer`] for anaphora resolution, behind the `use-llm` feature.
//!
//! Opt-in and dev/experimentation only: it lets us validate *resolution quality* with a real
//! model in-process before the production path (the orchestrator across the process boundary).
//! Default builds stay LLM-free — the kernel is the trusted oracle; the resolve loop runs
//! against the abstract [`Proposer`] trait, and the LLM only ever *proposes* (the kernel
//! re-gates every suggestion via [`super::Parser::resolve_open`]). The proposer never
//! decides felicity, so a hallucinated or type-wrong antecedent is vetoed downstream.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::ontology::Iri;

use super::{ProposeCtx, Proposer};

/// The model's structured reply: candidate indices, most-likely antecedent first.
#[derive(Deserialize, JsonSchema)]
struct Ranking {
    /// Indices into the presented candidate list, ranked most-likely-antecedent first.
    /// Empty if no candidate is a plausible antecedent.
    ranked_candidate_indices: Vec<usize>,
}

/// A [`Proposer`] backed by Anthropic Claude via the direct tool-use client. Ranks the in-scope candidate
/// antecedents for a referent hole; on any error (no answer, transport, deserialize) it
/// proposes nothing — i.e. *unresolvable* — so the resolve loop fails closed rather than
/// guessing.
pub struct AnthropicProposer {
    api_key: String,
    model: String,
}

impl AnthropicProposer {
    /// Build from an explicit key + model.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// Build from `$ANTHROPIC_API_KEY` (the standard shell env), defaulting to a fast model.
    /// `None` if the key is unset.
    pub fn from_env() -> Option<Self> {
        std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .map(|k| Self::new(k, crate::dcg::anthropic_client::DEFAULT_MODEL))
    }

    fn ask(&self, instructions: &str) -> Option<Ranking> {
        // The client is async; bridge to the sync `Proposer` trait with a transient current-thread
        // runtime (the resolve loop is sync). Any failure → `None` (the loop fails closed).
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        match rt.block_on(
            crate::dcg::anthropic_client::anthropic_structured::<Ranking>(
                &self.api_key,
                &self.model,
                instructions,
            ),
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("anthropic proposer error: {e}");
                None
            }
        }
    }
}

impl Proposer for AnthropicProposer {
    fn propose(&self, ctx: &ProposeCtx) -> Vec<Iri> {
        if ctx.candidates.is_empty() {
            return Vec::new();
        }
        let candidate_list = ctx
            .candidates
            .iter()
            .enumerate()
            .map(|(i, c)| format!("[{i}] {}", c.surface))
            .collect::<Vec<_>>()
            .join("\n");
        let instructions = format!(
            "In the sentence:\n  \"{}\"\nan anaphor (pronoun or possessor) refers to an earlier \
             entity. Choose its most likely antecedent from these candidates:\n{}\n\nReturn \
             `ranked_candidate_indices`: the candidate indices, most-likely antecedent first \
             (empty if none is plausible).",
            ctx.sentence, candidate_list,
        );
        let Some(ranking) = self.ask(&instructions) else {
            return Vec::new();
        };
        ranking
            .ranked_candidate_indices
            .into_iter()
            .filter_map(|i| ctx.candidates.get(i).map(|c| c.iri.clone()))
            .collect()
    }
}
