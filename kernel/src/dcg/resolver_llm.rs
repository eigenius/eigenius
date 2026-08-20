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

use super::pretty_term;
use super::{Proposal, ProposeCtx, Proposer};

/// The model's structured reply: candidate indices, most-likely antecedent first, plus the
/// audit fields the proposal record stores verbatim (plan §2.4).
#[derive(Deserialize, JsonSchema)]
struct Ranking {
    /// Indices into the presented candidate list, ranked most-likely-antecedent first.
    /// Empty if no candidate is a plausible antecedent.
    ranked_candidate_indices: Vec<usize>,
    /// One sentence: why the top pick is the referent (or why none is).
    rationale: String,
    /// Confidence in the TOP pick, 0.0 to 1.0.
    confidence: f64,
}

/// A [`Proposer`] backed by Anthropic Claude via the direct tool-use client. Ranks the in-scope candidate
/// antecedents for a referent hole; on any error (no answer, transport, deserialize) it
/// proposes nothing — i.e. *unresolvable* — so the resolve loop fails closed rather than
/// guessing.
pub struct AnthropicProposer {
    api_key: String,
    model: crate::dcg::anthropic_client::ModelConfig,
}

impl AnthropicProposer {
    /// Build from an explicit key + model.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::with_config(
            api_key,
            crate::dcg::anthropic_client::ModelConfig::with_model(model),
        )
    }

    /// Build with an explicit [`ModelConfig`] — how a formalization run selects the model it
    /// wants, and what a recorded draw names as the answerer (D71 §7.1 / §9).
    pub fn with_config(
        api_key: impl Into<String>,
        model: crate::dcg::anthropic_client::ModelConfig,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            model,
        }
    }

    /// Build from `$ANTHROPIC_API_KEY` (the standard shell env), defaulting to a fast model.
    /// `None` if the key is unset.
    pub fn from_env() -> Option<Self> {
        Self::from_env_with(Default::default())
    }

    /// From `$ANTHROPIC_API_KEY` with an explicit [`ModelConfig`]. The formalization service
    /// threads one config to every proposer in a run, so a draw's recorded model is the run's,
    /// not a per-seam default (D71 §7.1 / §9).
    pub fn from_env_with(cfg: crate::dcg::anthropic_client::ModelConfig) -> Option<Self> {
        std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .map(|k| Self::with_config(k, cfg.clone()))
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
    fn propose(&self, ctx: &ProposeCtx) -> Proposal {
        if ctx.candidates.is_empty() {
            return Proposal::default();
        }
        let candidate_list = ctx
            .candidates
            .iter()
            .enumerate()
            .map(|(i, c)| format!("[{i}] {}", c.surface()))
            .collect::<Vec<_>>()
            .join("\n");
        // Prior selections — the discourse the referent must be consistent with (same block the
        // reading ranker presents).
        let mut prior_block = String::new();
        if !ctx.doc.prior_selections.is_empty() {
            prior_block
                .push_str("Readings already selected for earlier sentences (the discourse):\n");
            for p in ctx.doc.prior_selections {
                prior_block.push_str(&format!("  sentence {}: \"{}\"\n", p.ordinal, p.gloss));
            }
            prior_block.push('\n');
        }
        let instructions = format!(
            "A parser is resolving an anaphor (a pronoun, possessor, or demonstrative like \
             \"these X\") to its antecedent in the document below.\n\n\
             Document:\n{}\n\n{prior_block}\
             The sentence containing the anaphor:\n  \"{}\"\n\n\
             The anaphor's referent must be of type `{}`. The candidates (already filtered to \
             that type — earlier discourse referents, most recent first):\n{}\n\n\
             Return `ranked_candidate_indices` (most-likely antecedent first; empty if none is \
             the referent), `rationale` (one sentence), and `confidence` in the top pick (0-1).",
            ctx.doc.document.trim(),
            ctx.doc.sentence.trim(),
            pretty_term(&ctx.hole.ty),
            candidate_list,
        );
        // `EIGENIUS_DUMP_PROPOSE_PROMPT=1` prints the exact prompt per hole — same rationale as
        // the reading ranker's dump: the proposer decides which referent lands, so being able to
        // READ what it was asked matters.
        if std::env::var("EIGENIUS_DUMP_PROPOSE_PROMPT").is_ok() {
            eprintln!("\n===== PROPOSER PROMPT =====\n{instructions}\n===== END PROMPT =====\n");
        }
        let Some(ranking) = self.ask(&instructions) else {
            return Proposal::default();
        };
        // Out-of-range indices are dropped here for a cleaner record; `resolve_with` would
        // ignore them anyway (the proposer is untrusted input).
        Proposal {
            ranked: ranking
                .ranked_candidate_indices
                .into_iter()
                .filter(|&i| i < ctx.candidates.len())
                .collect(),
            rationale: Some(ranking.rationale),
            confidence: Some(ranking.confidence.clamp(0.0, 1.0)),
        }
    }
}
