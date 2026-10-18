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

//! D43 §2.3 / M3.2 — chain-aware BM25 scorer.
//!
//! Implements the BM25 ranking function with one structural
//! modification documented in D43 §2.3: per-document term frequency
//! is implicit (`tf = 1` when the term is in the doc per the
//! posting bitmap, `0` otherwise). The Roaring-bitmap posting list
//! stored at `text_term:<I>:<T>:<L>` is a set, not a multiset, so
//! we don't carry per-`(term, doc)` counts.
//!
//! The simplified per-term contribution under that assumption is:
//!
//! ```text
//!   score_term = IDF(t) * (k1 + 1) / (1 + k1 * (1 - b + b * |d| / avgdl))
//! ```
//!
//! and the full BM25 score for a document is the sum of `score_term`
//! over the query terms the document actually contains.
//!
//! Chain-aware IDF (D43 §2.3 query path step 6) sums `N` and
//! per-term `df` across the visible layer chain rooted at the
//! query head — not within a single segment, the way Tantivy
//! computes BM25 per-segment. This is the structural payoff of
//! rolling our own inverted index: scores from different layers
//! share the same IDF baseline and compose by direct top-k merge
//! without per-segment renormalisation (D43 §7.4).
//!
//! M3.3 wraps this with the chain-walk + posting-intersection +
//! shadow-check orchestration. M3.2's surface is the pure math:
//! IDF computation, length normalisation, per-term and per-doc
//! scoring.

use crate::layer::{collect_ancestors, Layer, LayerId, TermHit, TextIndex};
use crate::ontology::iri::Iri;
use std::collections::BTreeSet;

/// BM25 free parameters. Lucene's `BM25Similarity` defaults are
/// the recommended starting point for general-purpose English text.
#[derive(Debug, Clone, Copy)]
pub struct Bm25Params {
    /// Term-frequency saturation parameter. Higher values push the
    /// score to grow more linearly with `tf`; values close to 0
    /// effectively reduce BM25 to a binary-presence scorer. We
    /// don't use `tf > 1` per the structural simplification above,
    /// but the formula's `k1` still affects how aggressively long
    /// documents are penalised relative to short ones.
    pub k1: f32,
    /// Length-normalisation parameter in `[0, 1]`. `b = 0` removes
    /// length normalisation entirely; `b = 1` applies it in full.
    /// Lucene's default is `0.75`.
    pub b: f32,
}

impl Bm25Params {
    /// Lucene's defaults (`k1 = 1.2`, `b = 0.75`). The starting
    /// point unless workload-specific tuning suggests otherwise.
    pub const LUCENE_DEFAULT: Self = Self { k1: 1.2, b: 0.75 };
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self::LUCENE_DEFAULT
    }
}

/// Per-query chain-aware statistics for one TextIndex Resource.
///
/// Computed once at query-time (D43 §2.3 step 6) by
/// [`compute_chain_stats`]: it sums document counts and per-term
/// DFs across every layer in the visible chain that contributes
/// under the active TextIndex. Cached for the lifetime of the
/// query so multi-term queries don't re-walk the chain per term.
///
/// **`n` (total document count).** Sum of
/// `text_stats:<I>:<L>.doc_count` for every `L` in the chain with
/// any contribution under `I`. This is the BM25 corpus-size term;
/// because the chain is the corpus, this is the right value.
///
/// **`avg_doc_length`.** Length-weighted average of per-layer
/// `avg_doc_length`s. Same as `total_token_count / n` over the
/// visible chain.
///
/// **`term_df`.** Per-query-term map to global document frequency
/// summed across visible layers. Drives the IDF computation in
/// [`Bm25Scorer::idf_for`].
#[derive(Debug, Clone)]
pub struct Bm25ChainStats {
    /// Total document count across the visible chain.
    pub n: u64,
    /// Average document length across the visible chain.
    pub avg_doc_length: f32,
    /// Per-term document frequency across the visible chain. The
    /// map is keyed by analyzed-term string and carries the
    /// summed DF.
    pub term_df: std::collections::BTreeMap<String, u64>,
}

/// Compute the chain-aware BM25 statistics for a query against a
/// specific TextIndex Resource at the given head.
///
/// Walks the visible ancestor set, sums per-layer `text_stats` for
/// the global `N` and length-weighted `avg_doc_length`, and sums
/// per-term `df` from the posting-list value prefixes (no Roaring
/// bitmap deserialisation — that happens in M3.3 when scoring
/// individual docs).
///
/// `query_terms` is the post-analyzer token list. Repeats in the
/// query don't compound DF (querying for `"alpha alpha"` is
/// semantically the same as `"alpha"` for IDF purposes).
pub fn compute_chain_stats(
    head: &Layer,
    text_index: &dyn TextIndex,
    index_iri: &Iri,
    query_terms: &[String],
) -> Bm25ChainStats {
    let chain = collect_ancestors(head);

    // Sum per-layer doc_count and weighted length sum.
    let mut n: u64 = 0;
    let mut weighted_len_sum: f64 = 0.0;
    for layer in &chain {
        if let Ok(Some(stats)) = text_index.get_layer_stats(index_iri, layer) {
            let dc = stats.doc_count as u64;
            n += dc;
            weighted_len_sum += dc as f64 * stats.avg_doc_length as f64;
        }
    }
    let avg_doc_length = if n > 0 {
        (weighted_len_sum / n as f64) as f32
    } else {
        0.0
    };

    // Sum per-term DF across visible layers. Repeated query terms
    // are deduplicated before the scan because each unique term
    // produces a single DF lookup against the index.
    let mut term_df = std::collections::BTreeMap::new();
    let unique_terms: BTreeSet<&str> = query_terms.iter().map(|s| s.as_str()).collect();
    for term in unique_terms {
        let mut df_sum: u64 = 0;
        for hit in text_index.scan_term(index_iri, term).flatten() {
            if chain.contains(&hit.layer) {
                df_sum += hit.df as u64;
            }
        }
        term_df.insert(term.to_string(), df_sum);
    }

    Bm25ChainStats {
        n,
        avg_doc_length,
        term_df,
    }
}

/// BM25 scorer parameterised by `(k1, b)` and chain-aware stats.
///
/// Construct once per query via [`Bm25Scorer::new`] from a
/// [`Bm25ChainStats`] computed by [`compute_chain_stats`]. Score
/// individual documents with [`Bm25Scorer::score_doc`].
#[derive(Debug, Clone)]
pub struct Bm25Scorer {
    params: Bm25Params,
    stats: Bm25ChainStats,
}

impl Bm25Scorer {
    pub fn new(params: Bm25Params, stats: Bm25ChainStats) -> Self {
        Self { params, stats }
    }

    /// IDF for a single term per BM25's standard formula:
    ///
    /// ```text
    ///   IDF(t) = ln((N - df + 0.5) / (df + 0.5) + 1)
    /// ```
    ///
    /// The `+ 1` inside the logarithm guarantees `IDF(t) >= 0`
    /// even for `df > N/2` — matches Lucene's `BM25Similarity`
    /// and avoids the negative-IDF artefact of classical Robertson
    /// BM25 for very-common terms.
    ///
    /// Unknown terms (not in [`Bm25ChainStats::term_df`]) score as
    /// 0; treating them as if they had `df = 0` would yield a
    /// large positive IDF and bias the score towards typos.
    pub fn idf_for(&self, term: &str) -> f32 {
        let df = match self.stats.term_df.get(term) {
            Some(df) if *df > 0 => *df as f64,
            _ => return 0.0,
        };
        let n = self.stats.n as f64;
        let ratio = (n - df + 0.5) / (df + 0.5) + 1.0;
        ratio.ln() as f32
    }

    /// Score one document for a set of matched query terms.
    ///
    /// `matched_terms` is the subset of the query that this
    /// document actually contains (per the posting-list
    /// intersection in M3.3). Terms the document doesn't contain
    /// contribute 0 and aren't passed in.
    ///
    /// `doc_length` is the document's token count from
    /// `text_docs.doc_lengths[doc_id]`.
    pub fn score_doc(&self, matched_terms: &[&str], doc_length: u32) -> f32 {
        if matched_terms.is_empty() {
            return 0.0;
        }
        let dl = doc_length as f32;
        let avgdl = self.stats.avg_doc_length.max(1.0); // avoid /0 in empty-corpus edge
        let length_factor = self.params.k1 * (1.0 - self.params.b + self.params.b * dl / avgdl);
        let saturation = (self.params.k1 + 1.0) / (1.0 + length_factor);

        let mut sum = 0.0_f32;
        for term in matched_terms {
            sum += self.idf_for(term) * saturation;
        }
        sum
    }

    /// Access the underlying parameters (used by §6 planner code
    /// for cost estimation in M3.6+).
    pub fn params(&self) -> Bm25Params {
        self.params
    }

    /// Access the chain-aware stats this scorer was built with.
    pub fn stats(&self) -> &Bm25ChainStats {
        &self.stats
    }
}

/// Helper for [`compute_chain_stats`] tests and external chain-walk
/// integration. Reduces verbose imports.
#[doc(hidden)]
pub fn for_test_chain_terms(hits: &[TermHit], chain: &BTreeSet<LayerId>) -> u64 {
    hits.iter()
        .filter(|h| chain.contains(&h.layer))
        .map(|h| h.df as u64)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{MemoryTextIndex, TextDoc};
    use std::collections::BTreeMap;

    /// IDF formula matches Lucene's: `(N - df + 0.5) / (df + 0.5) + 1`,
    /// then `ln`. Common terms (high df) → small IDF; rare terms →
    /// large IDF.
    #[test]
    fn idf_formula_matches_lucene() {
        let stats = Bm25ChainStats {
            n: 1000,
            avg_doc_length: 8.0,
            term_df: BTreeMap::from([("common".into(), 500), ("rare".into(), 1)]),
        };
        let scorer = Bm25Scorer::new(Bm25Params::LUCENE_DEFAULT, stats);

        // Rare term IDF should exceed common term IDF.
        let idf_rare = scorer.idf_for("rare");
        let idf_common = scorer.idf_for("common");
        assert!(
            idf_rare > idf_common,
            "{idf_rare} should exceed {idf_common}"
        );

        // Unknown term → IDF=0 (no contribution).
        assert_eq!(scorer.idf_for("unknown"), 0.0);

        // Concrete numbers for the rare term at N=1000, df=1:
        // ratio = (1000 - 1 + 0.5) / (1 + 0.5) + 1 = 999.5 / 1.5 + 1 = 667.333
        // ln(667.333) ≈ 6.503
        assert!((idf_rare - 6.503).abs() < 0.01, "got {idf_rare}");
    }

    /// Document-length penalty: longer documents matching the same
    /// term get a lower score than shorter documents.
    #[test]
    fn length_normalisation_penalises_long_docs() {
        let stats = Bm25ChainStats {
            n: 100,
            avg_doc_length: 10.0,
            term_df: BTreeMap::from([("alpha".into(), 5)]),
        };
        let scorer = Bm25Scorer::new(Bm25Params::LUCENE_DEFAULT, stats);
        let short_doc = scorer.score_doc(&["alpha"], 5);
        let avg_doc = scorer.score_doc(&["alpha"], 10);
        let long_doc = scorer.score_doc(&["alpha"], 100);
        assert!(short_doc > avg_doc, "{short_doc} > {avg_doc}");
        assert!(avg_doc > long_doc, "{avg_doc} > {long_doc}");
    }

    /// Empty matched-terms list yields a zero score.
    #[test]
    fn empty_match_yields_zero_score() {
        let stats = Bm25ChainStats {
            n: 100,
            avg_doc_length: 10.0,
            term_df: BTreeMap::from([("alpha".into(), 5)]),
        };
        let scorer = Bm25Scorer::new(Bm25Params::default(), stats);
        assert_eq!(scorer.score_doc(&[], 10), 0.0);
    }

    /// Multi-term scoring sums contributions across matched terms.
    /// A document matching two terms with the same IDF scores 2×
    /// what a document matching one of them scores (modulo length
    /// normalisation — same doc length here).
    #[test]
    fn multi_term_scoring_sums_contributions() {
        let stats = Bm25ChainStats {
            n: 100,
            avg_doc_length: 10.0,
            term_df: BTreeMap::from([
                ("alpha".into(), 5),
                ("beta".into(), 5), // same df → same IDF
            ]),
        };
        let scorer = Bm25Scorer::new(Bm25Params::default(), stats);
        let single = scorer.score_doc(&["alpha"], 10);
        let both = scorer.score_doc(&["alpha", "beta"], 10);
        assert!((both - 2.0 * single).abs() < 1e-5, "{both} ≈ 2 * {single}");
    }

    /// `b = 0` removes length normalisation; all matching docs at
    /// the same term get the same score regardless of length.
    #[test]
    fn b_zero_removes_length_normalisation() {
        let stats = Bm25ChainStats {
            n: 100,
            avg_doc_length: 10.0,
            term_df: BTreeMap::from([("alpha".into(), 5)]),
        };
        let scorer = Bm25Scorer::new(Bm25Params { k1: 1.2, b: 0.0 }, stats);
        let short = scorer.score_doc(&["alpha"], 1);
        let long = scorer.score_doc(&["alpha"], 1000);
        assert!((short - long).abs() < 1e-5, "{short} ≈ {long}");
    }

    /// Chain-aware stats sum doc_count and DF across all visible
    /// layers under one TextIndex. Layers outside the visible
    /// chain don't contribute.
    #[test]
    fn compute_chain_stats_sums_across_chain() {
        use crate::bootstrap::bootstrap;
        use crate::layer::LayerBuilder;
        use crate::ontology::iri::Iri;
        use std::sync::Arc;

        let ctx = bootstrap().unwrap();
        let head_seed = Arc::clone(ctx.head());

        // Build two layers under one TextIndex; populate the memory
        // backend's text index via the storage handle.
        let storage = head_seed.storage().clone();
        let text_index = Arc::clone(&storage.text_index);

        let mut l1_b = LayerBuilder::new("l1", Some(Arc::clone(&head_seed)));
        let dummy =
            crate::ontology::resource::Resource::new(Iri::parse("urn:eigenius:test:r1").unwrap());
        l1_b.add_resource(dummy).unwrap();
        let l1 = Arc::new(l1_b.build(storage.clone()));

        let mut l2_b = LayerBuilder::new("l2", Some(Arc::clone(&l1)));
        let dummy2 =
            crate::ontology::resource::Resource::new(Iri::parse("urn:eigenius:test:r2").unwrap());
        l2_b.add_resource(dummy2).unwrap();
        let l2 = Arc::new(l2_b.build(storage.clone()));

        let index_iri = Iri::parse("urn:eigenius:test:idx").unwrap();
        let s1 = Iri::parse("urn:eigenius:test:s1").unwrap();
        let s2 = Iri::parse("urn:eigenius:test:s2").unwrap();
        let s3 = Iri::parse("urn:eigenius:test:s3").unwrap();
        let toks_1 = vec!["alpha".to_string(), "beta".to_string()];
        let toks_2 = vec!["beta".to_string(), "gamma".to_string()];
        let toks_3 = vec!["alpha".to_string(); 4];

        // L1 has 2 docs: (s1, [alpha, beta]), (s2, [beta, gamma])
        text_index
            .extend_layer(
                &index_iri,
                l1.id(),
                "en-stem-v1",
                &[
                    TextDoc {
                        subject: &s1,
                        tokens: &toks_1,
                    },
                    TextDoc {
                        subject: &s2,
                        tokens: &toks_2,
                    },
                ],
            )
            .unwrap();

        // L2 has 1 doc: (s3, [alpha alpha alpha alpha])
        text_index
            .extend_layer(
                &index_iri,
                l2.id(),
                "en-stem-v1",
                &[TextDoc {
                    subject: &s3,
                    tokens: &toks_3,
                }],
            )
            .unwrap();

        // Compute chain stats for query terms ["alpha", "beta"].
        let query = vec!["alpha".to_string(), "beta".to_string()];
        let stats = compute_chain_stats(&l2, text_index.as_ref(), &index_iri, &query);

        // N: 2 (L1) + 1 (L2) = 3.
        assert_eq!(stats.n, 3);

        // avg_doc_length: weighted by per-layer doc_count.
        // L1 has 2 docs, each length 2 → avg 2.0.
        // L2 has 1 doc, length 4 → avg 4.0.
        // Chain-aware avg = (2.0 × 2 + 4.0 × 1) / 3 = 8/3 ≈ 2.667.
        assert!(
            (stats.avg_doc_length - 8.0 / 3.0).abs() < 0.01,
            "got {}",
            stats.avg_doc_length
        );

        // DF for "alpha": s1 contains alpha (L1), s3 contains alpha (L2) → df=2.
        // DF for "beta":  s1 contains beta (L1), s2 contains beta (L1) → df=2.
        assert_eq!(stats.term_df.get("alpha").copied(), Some(2));
        assert_eq!(stats.term_df.get("beta").copied(), Some(2));
    }

    /// IDF is layer-aware: a term that appears in 1 doc out of 3
    /// has a different IDF than the same term appearing in 1 doc
    /// out of 30 — because `N` is summed across the visible chain.
    #[test]
    fn chain_aware_idf_changes_with_chain_size() {
        let stats_small = Bm25ChainStats {
            n: 3,
            avg_doc_length: 10.0,
            term_df: BTreeMap::from([("alpha".into(), 1)]),
        };
        let stats_large = Bm25ChainStats {
            n: 30,
            avg_doc_length: 10.0,
            term_df: BTreeMap::from([("alpha".into(), 1)]),
        };
        let scorer_small = Bm25Scorer::new(Bm25Params::default(), stats_small);
        let scorer_large = Bm25Scorer::new(Bm25Params::default(), stats_large);
        assert!(scorer_large.idf_for("alpha") > scorer_small.idf_for("alpha"));
    }

    /// Tests for the deduplicated DF helper.
    #[test]
    fn dedup_helper_sums_in_chain_only() {
        use crate::layer::LayerId;
        let l1 = LayerId([1u8; 32]);
        let l2 = LayerId([2u8; 32]);
        let l3_out = LayerId([3u8; 32]);
        let hits = vec![
            TermHit {
                layer: l1.clone(),
                df: 5,
                postings: vec![],
            },
            TermHit {
                layer: l2.clone(),
                df: 7,
                postings: vec![],
            },
            TermHit {
                layer: l3_out,
                df: 100,
                postings: vec![],
            },
        ];
        let chain = BTreeSet::from([l1, l2]);
        assert_eq!(for_test_chain_terms(&hits, &chain), 12);
    }

    /// MemoryTextIndex round-trip — the BM25 scorer doesn't care
    /// which `TextIndex` impl it consults, but verify against the
    /// in-memory one for completeness.
    #[test]
    fn smoke_test_with_memory_text_index() {
        let idx = MemoryTextIndex::new();
        let index_iri = Iri::parse("urn:eigenius:test:idx").unwrap();
        let layer = crate::layer::LayerId([42u8; 32]);
        let s = Iri::parse("urn:eigenius:test:s").unwrap();
        let toks = vec!["alpha".to_string(), "beta".to_string(), "alpha".to_string()];
        idx.extend_layer(
            &index_iri,
            &layer,
            "en-stem-v1",
            &[TextDoc {
                subject: &s,
                tokens: &toks,
            }],
        )
        .unwrap();

        // Synthetic stats with one doc, query "alpha beta".
        let stats = Bm25ChainStats {
            n: 1,
            avg_doc_length: 3.0,
            term_df: BTreeMap::from([("alpha".into(), 1), ("beta".into(), 1)]),
        };
        let scorer = Bm25Scorer::new(Bm25Params::default(), stats);
        let score = scorer.score_doc(&["alpha", "beta"], 3);
        // Non-zero score for a matching doc.
        assert!(score > 0.0, "expected positive score, got {score}");
    }
}
