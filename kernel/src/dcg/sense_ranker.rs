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

//! Contextual **sense reranking** (D63 parsing-scale plan / GH #97) — the *strong* form of the
//! adaptive-supertagging lever.
//!
//! The deterministic sense cap (`LexicalIndex::with_sense_cap`) keeps the top-`N` senses per lemma
//! by static `sense_rank` (global WordNet frequency). A [`SenseRanker`] makes that prior
//! **contextual**: given a sentence and each content word's candidate senses, it returns a per-word
//! ranking, so the kept top-`N` are the senses most plausible *in this sentence*. This is zero-shot
//! neural contextual supertagging (cf. Xu/Auli/Clark 2015) and it reuses the resolver's
//! **proposer-behind-oracle** pattern (D64 §4): an *untrusted* ranker only reorders the seed beam;
//! the kernel felicity gate still decides validity, and widen-on-failure recovers a wrongly
//! down-ranked sense (a bad rank costs a re-parse, never a missed parse).
//!
//! Impls: a deterministic mock ([`IdentityRanker`]) for CI, and a feature-gated live Anthropic
//! ranker ([`AnthropicSenseRanker`], `use-llm` feature, tool-use-constrained). Both behind the one
//! [`SenseRanker`] trait, so the (future) parser-cap integration is impl-agnostic.

/// One candidate sense of a content word: its lexicon `sense` label (e.g. `wn:bank.n.01`) and a
/// short human-readable gloss the ranker reasons over.
#[derive(Clone, Debug)]
pub struct SenseCandidate {
    pub sense: String,
    pub gloss: String,
}

/// One word's sense-ranking request: the surface form and its candidate senses (in seed order).
pub struct WordSenses<'a> {
    pub surface: &'a str,
    pub candidates: &'a [SenseCandidate],
}

/// The **untrusted** contextual sense reranker. Given the `sentence` and one [`WordSenses`] per
/// content word, return a **ranking per word**: a permutation of that word's candidate indices,
/// most-plausible-in-context first. The returned `Vec` is aligned with `words` (one inner `Vec`
/// per word); each inner `Vec` should be a permutation of `0..candidates.len()` (callers must
/// tolerate a malformed reply — e.g. an LLM omission — by falling back to the seed order).
pub trait SenseRanker {
    fn rank(&self, sentence: &str, words: &[WordSenses]) -> Vec<Vec<usize>>;
}

/// The trivial deterministic ranker: keep each word's candidates in seed order (identity
/// permutation). The CI stand-in for the trait + the no-op default (equivalent to the static
/// `sense_rank` cap with no contextual reordering).
pub struct IdentityRanker;

impl SenseRanker for IdentityRanker {
    fn rank(&self, _sentence: &str, words: &[WordSenses]) -> Vec<Vec<usize>> {
        words
            .iter()
            .map(|w| (0..w.candidates.len()).collect())
            .collect()
    }
}

// ───────────────────────── live Anthropic ranker (use-llm feature) ─────────────────────────

#[cfg(feature = "use-llm")]
mod anthropic {
    use super::{SenseRanker, WordSenses};
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// The model's structured reply: one ranking per word (each a list of candidate indices,
    /// most-plausible-first), aligned with the request order.
    #[derive(Deserialize, JsonSchema)]
    struct SenseRankingReply {
        /// One ranking per word, in the same order the words were given; each is that word's
        /// candidate indices reordered most-plausible-in-context first.
        rankings: Vec<Vec<usize>>,
    }

    /// A [`SenseRanker`] backed by Anthropic Claude via the direct tool-use client
    /// ([`crate::dcg::anthropic_client`]). On any error it returns the **seed order** (identity) so
    /// the caller degrades gracefully — the reranker only reorders a beam, never gates validity.
    pub struct AnthropicSenseRanker {
        api_key: String,
        model: String,
    }

    impl AnthropicSenseRanker {
        pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
            Self {
                api_key: api_key.into(),
                model: model.into(),
            }
        }

        /// From `$ANTHROPIC_API_KEY`, defaulting to a fast model. `None` if the key is unset.
        pub fn from_env() -> Option<Self> {
            std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .map(|k| Self::new(k, crate::dcg::anthropic_client::DEFAULT_MODEL))
        }

        fn ask(&self, instructions: &str) -> Option<SenseRankingReply> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            match rt.block_on(crate::dcg::anthropic_client::anthropic_structured::<
                SenseRankingReply,
            >(&self.api_key, &self.model, instructions))
            {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("anthropic sense-ranker error: {e}");
                    None
                }
            }
        }
    }

    impl SenseRanker for AnthropicSenseRanker {
        fn rank(&self, sentence: &str, words: &[WordSenses]) -> Vec<Vec<usize>> {
            let identity = || -> Vec<Vec<usize>> {
                words
                    .iter()
                    .map(|w| (0..w.candidates.len()).collect())
                    .collect()
            };
            if words.is_empty() {
                return Vec::new();
            }
            let mut prompt = format!(
                "In the sentence:\n  \"{sentence}\"\nrank each word's candidate senses by \
                 contextual plausibility (most-likely sense first). Return `rankings`: one list \
                 per word (in the given order), each a permutation of that word's candidate \
                 indices, most-plausible first.\n\nWords and candidate senses:\n"
            );
            for (wi, w) in words.iter().enumerate() {
                prompt.push_str(&format!("Word {wi} = \"{}\":\n", w.surface));
                for (ci, c) in w.candidates.iter().enumerate() {
                    prompt.push_str(&format!("  [{ci}] {}\n", c.gloss));
                }
            }
            let Some(reply) = self.ask(&prompt) else {
                return identity();
            };
            // Accept only well-formed per-word permutations; fall back to seed order otherwise.
            if reply.rankings.len() != words.len() {
                return identity();
            }
            reply
                .rankings
                .into_iter()
                .zip(words)
                .map(|(ranking, w)| {
                    let n = w.candidates.len();
                    let valid: Vec<usize> = ranking.into_iter().filter(|&i| i < n).collect();
                    // Append any indices the model omitted, preserving completeness.
                    let mut seen = vec![false; n];
                    let mut out = Vec::with_capacity(n);
                    for i in valid {
                        if !seen[i] {
                            seen[i] = true;
                            out.push(i);
                        }
                    }
                    for (i, s) in seen.iter().enumerate() {
                        if !s {
                            out.push(i);
                        }
                    }
                    out
                })
                .collect()
        }
    }
}

#[cfg(feature = "use-llm")]
pub use anthropic::AnthropicSenseRanker;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_ranker_keeps_seed_order() {
        let cands = vec![
            SenseCandidate {
                sense: "a".into(),
                gloss: "x".into(),
            },
            SenseCandidate {
                sense: "b".into(),
                gloss: "y".into(),
            },
            SenseCandidate {
                sense: "c".into(),
                gloss: "z".into(),
            },
        ];
        let words = vec![WordSenses {
            surface: "w",
            candidates: &cands,
        }];
        assert_eq!(IdentityRanker.rank("s", &words), vec![vec![0, 1, 2]]);
    }

    /// Live WSD: a real model must pick the contextual sense (JSON-Schema-constrained). Skips
    /// without a key; runs live with `--features use-llm` + `ANTHROPIC_API_KEY`.
    #[cfg(feature = "use-llm")]
    #[test]
    fn live_anthropic_sense_ranker_picks_the_contextual_sense() {
        let Some(ranker) = AnthropicSenseRanker::from_env() else {
            eprintln!("SKIP live_anthropic_sense_ranker: ANTHROPIC_API_KEY unset");
            return;
        };
        let cands = vec![
            SenseCandidate {
                sense: "bank.n.01".into(),
                gloss: "a financial institution that accepts deposits and makes loans".into(),
            },
            SenseCandidate {
                sense: "bank.n.09".into(),
                gloss: "sloping land beside a body of water".into(),
            },
        ];
        let words = vec![WordSenses {
            surface: "bank",
            candidates: &cands,
        }];
        let r = ranker.rank(
            "The bank approved the loan after reviewing the application.",
            &words,
        );
        assert_eq!(r.len(), 1, "one ranking for the one word");
        assert_eq!(r[0].len(), 2, "a permutation of both candidates");
        assert_eq!(
            r[0][0], 0,
            "the financial sense ranks first in a loan context, got {:?}",
            r[0]
        );
    }
}
