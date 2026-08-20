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
//! The deterministic sense cap (`Parser::with_sense_cap`) keeps the top-`N` senses per lemma
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
    /// **What this sense DENOTES** — the pretty-printed `sem`.
    ///
    /// Recorded because the `sense` LABEL is not the concept. Cross-lexicon alignment redefines an
    /// entry's `cat`/`sem` to the WordNet class but deliberately leaves `sense` alone (the seed-time
    /// dedup keys on `(cat, sem)`, so rewriting the label would be busywork). A merged UMLS entry
    /// therefore still reports `umls:C1442792` here — which made `ranks.json` blind to the very
    /// merges it was being used to measure. Recording the `sem` makes two entries that now denote
    /// ONE concept visibly identical.
    pub sem: String,
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
    /// One ranking per word, or `None` when this ranker DID NOT ANSWER — a transport/API failure,
    /// a malformed reply, or a replay miss.
    ///
    /// The distinction is load-bearing and was missing until 2026-08-13. The failure path used to
    /// return the identity permutation, which is indistinguishable from a real answer that keeps
    /// every sense — so a failed call got RECORDED as a ranking and replayed forever as fact.
    /// Measured: in `ranks/2026-07-29-demonstratives.json`, «WRN was dispensable in models of
    /// microsatellite-stable cancers.» carries identity for all 20 senses of «models» and all 7
    /// of «cancers». That frozen failure is why the crab genus and the astrological sign reached
    /// the parse, and why the reading ranker was later asked to choose between them. A `None`
    /// cannot be mistaken for an answer: the caller falls back to seed order, and the RECORDER
    /// writes nothing, so a re-run retries instead of inheriting the failure.
    fn rank(&self, sentence: &str, context: &str, words: &[WordSenses]) -> Option<Vec<Vec<usize>>>;
}

/// The trivial deterministic ranker: keep each word's candidates in seed order (identity
/// permutation). The CI stand-in for the trait + the no-op default (equivalent to the static
/// `sense_rank` cap with no contextual reordering).
pub struct IdentityRanker;

impl SenseRanker for IdentityRanker {
    fn rank(
        &self,
        _sentence: &str,
        _context: &str,
        words: &[WordSenses],
    ) -> Option<Vec<Vec<usize>>> {
        Some(
            words
                .iter()
                .map(|w| (0..w.candidates.len()).collect())
                .collect(),
        )
    }
}

// ───────────────────────── record / replay (reproducibility) ─────────────────────────

/// A recorded ranking decision: the exact question put to the ranker, and the answer it gave.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RankRecord {
    /// The sentence the ranking was conditioned on.
    pub sentence: String,
    /// The surrounding PASSAGE the ranking was conditioned on (empty = ranked in isolation). Part of
    /// the question, so it is part of the replay key — a recording made with a different context
    /// window must MISS, not silently replay an answer to a different question.
    #[serde(default)]
    pub context: String,
    /// Per word: the surface form, its candidate sense labels **in seed order**, and the
    /// permutation the ranker returned (indices into `senses`, most-plausible-first).
    pub words: Vec<RankedWord>,
}

/// One word's recorded ranking.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RankedWord {
    pub surface: String,
    pub senses: Vec<String>,
    /// What each sense DENOTES (aligned with `senses`) — see [`SenseCandidate::sem`]. Two entries
    /// with different `senses` but the SAME `sems` are the same concept under two labels.
    #[serde(default)]
    pub sems: Vec<String>,
    pub order: Vec<usize>,
}

/// The lookup key for a ranking: the sentence plus every word's candidate sense-set **in seed
/// order**. Both matter — the same word ranks differently in a different sentence, and a different
/// candidate set is a different question. Two runs whose lexicon changed will therefore MISS the
/// cache rather than silently replay a stale answer.
fn rank_key(sentence: &str, context: &str, words: &[WordSenses]) -> String {
    // The CONTEXT is part of the question: the same sentence ranked with different surrounding
    // sentences is a different query and may get a different answer. Including it means a recording
    // made under a different context window MISSES (and `assert_replay_faithful` makes that fatal)
    // instead of silently replaying a context-free answer.
    let mut k = String::from(sentence);
    k.push('\u{1d}');
    k.push_str(context);
    for w in words {
        k.push('\u{1f}');
        // LOWERCASED, and it must stay that way. The ranking question is about the WORD, not its
        // casing, and every recording predates `tokenize` preserving case (2026-07-29) — so the
        // recorded surfaces are lowercase. Keying on the raw surface would make a capitalised
        // sentence-initial token miss its own recording, which `assert_replay_faithful` turns into a
        // hard failure. Normalising here (and identically in the replay's key rebuild) keeps every
        // committed recording valid across that change.
        k.push_str(&w.surface.to_lowercase());
        for c in w.candidates {
            k.push('\u{1e}');
            k.push_str(&c.sense);
        }
    }
    k
}

/// The same key, computed from a RECORDED exchange rather than a live one.
///
/// `load` used to re-derive this inline, which meant two copies of one contract with a comment
/// asking the reader to keep them in step. It is one function now, because the D71 draw emitter
/// needs the key too and a third copy would have been the point where they diverged.
pub(crate) fn record_key(r: &RankRecord) -> String {
    let mut k = r.sentence.clone();
    k.push('\u{1d}');
    k.push_str(&r.context);
    for w in &r.words {
        k.push('\u{1f}');
        k.push_str(&w.surface.to_lowercase()); // see the note in `rank_key`
        for s in &w.senses {
            k.push('\u{1e}');
            k.push_str(s);
        }
    }
    k
}

/// **Record** every ranking an inner ranker produces, so the run can later be replayed exactly.
///
/// The contextual reranker is an LLM: it is the one component that can return a different answer
/// for the same code and the same store, which makes any measurement that depends on it
/// irreproducible — and makes it impossible to A/B a parser change, because the LLM moves
/// underneath you. Recording turns it from an *uncontrolled* input into a *recorded* one:
/// [`ReplaySenseRanker`] then re-runs the identical decisions with no API calls at all.
///
/// Flush with [`Self::write`] (the harness does this at the end of a run).
pub struct RecordingSenseRanker<R: SenseRanker> {
    inner: R,
    log: std::sync::Mutex<std::collections::BTreeMap<String, RankRecord>>,
}

impl<R: SenseRanker> RecordingSenseRanker<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            log: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// The recorded decisions as JSON (sorted by key — deterministic bytes).
    pub fn to_json(&self) -> std::io::Result<String> {
        let log = self.log.lock().expect("rank log");
        let records: Vec<&RankRecord> = log.values().collect();
        serde_json::to_string_pretty(&records)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Write the recorded decisions as JSON (sorted by key — deterministic bytes).
    pub fn write(&self, path: &std::path::Path) -> std::io::Result<usize> {
        let json = self.to_json()?;
        let n = self.log.lock().expect("rank log").len();
        std::fs::write(path, json)?;
        Ok(n)
    }

    /// The recorded decisions as chain-ready draws (D71 §9) — the same set `write` serialises,
    /// each paired with the replay key it answers.
    pub fn keyed_draws(&self) -> std::io::Result<Vec<crate::dcg::draw::KeyedDraw>> {
        let log = self.log.lock().expect("rank log");
        log.values()
            .map(|r| {
                Ok(crate::dcg::draw::KeyedDraw {
                    key: record_key(r),
                    record: serde_json::to_value(r)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
                })
            })
            .collect()
    }
}

impl<R: SenseRanker> SenseRanker for RecordingSenseRanker<R> {
    fn rank(&self, sentence: &str, context: &str, words: &[WordSenses]) -> Option<Vec<Vec<usize>>> {
        // A NON-ANSWER IS NEVER RECORDED. Writing the fallback would freeze a failed call into the
        // artifact, where it replays as "keep every sense" and reports a HIT — the 2026-07-29 crab
        // bug. Nothing recorded ⇒ the next run asks again.
        let order = self.inner.rank(sentence, context, words)?;
        // An answer that eliminates NOTHING anywhere is either a ranker declining to work or a
        // failure that slipped through some other impl's fallback. Say so at record time, where
        // the run can still be repeated, rather than leaving it to be discovered in a draw months
        // later.
        if words.len() > 1
            && words
                .iter()
                .zip(order.iter())
                .all(|(w, o)| o.iter().copied().eq(0..w.candidates.len()))
        {
            eprintln!(
                "sense-ranks: SUSPICIOUS — every word of «{}» kept every sense in seed order \
                 ({} words, {} senses). That is the shape of a failed call, not a ranking.",
                sentence.trim(),
                words.len(),
                words.iter().map(|w| w.candidates.len()).sum::<usize>()
            );
        }
        let rec = RankRecord {
            sentence: sentence.to_string(),
            context: context.to_string(),
            words: words
                .iter()
                .zip(order.iter())
                .map(|(w, o)| RankedWord {
                    surface: w.surface.to_string(),
                    senses: w.candidates.iter().map(|c| c.sense.clone()).collect(),
                    sems: w.candidates.iter().map(|c| c.sem.clone()).collect(),
                    order: o.clone(),
                })
                .collect(),
        };
        self.log
            .lock()
            .expect("rank log")
            .insert(rank_key(sentence, context, words), rec);
        Some(order)
    }
}

/// **Replay** rankings recorded by [`RecordingSenseRanker`] — no LLM, no network, deterministic.
///
/// A miss (the sentence or a word's candidate set is not in the recording) falls back to the seed
/// order and is COUNTED, not hidden: [`Self::misses`] must be 0 for a replay to be a faithful
/// reproduction. A non-zero count means the lexicon or the page changed under the recording, and
/// the run is a different experiment.
pub struct ReplaySenseRanker {
    by_key: std::collections::BTreeMap<String, Vec<Vec<usize>>>,
    misses: std::sync::atomic::AtomicUsize,
    hits: std::sync::atomic::AtomicUsize,
}

impl ReplaySenseRanker {
    /// Load a recording written by [`RecordingSenseRanker::write`].
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    /// Load a recording from its JSON, wherever it came from — a draw file, or the run's
    /// `doc-<id>` branch via [`crate::dcg::draw::draws_from_layer`] (D71 §9). One path, so a
    /// chain-replayed run and a file-replayed run cannot key differently.
    pub fn from_json(text: &str) -> std::io::Result<Self> {
        let records: Vec<RankRecord> = serde_json::from_str(text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut by_key = std::collections::BTreeMap::new();
        for r in records {
            // The key comes from the recorded question, so it matches what `rank` will compute.
            let key = record_key(&r);
            by_key.insert(key, r.words.iter().map(|w| w.order.clone()).collect());
        }
        Ok(Self {
            by_key,
            misses: std::sync::atomic::AtomicUsize::new(0),
            hits: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Rankings replayed from the recording.
    pub fn hits(&self) -> usize {
        self.hits.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Rankings NOT found in the recording (fell back to seed order). **Must be 0** for the replay
    /// to reproduce the recorded run.
    pub fn misses(&self) -> usize {
        self.misses.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl SenseRanker for ReplaySenseRanker {
    fn rank(&self, sentence: &str, context: &str, words: &[WordSenses]) -> Option<Vec<Vec<usize>>> {
        match self.by_key.get(&rank_key(sentence, context, words)) {
            Some(order) => {
                self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Some(order.clone())
            }
            None => {
                self.misses
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                None // a miss is a NON-ANSWER; the caller falls back, nothing is recorded
            }
        }
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
        fn rank(
            &self,
            sentence: &str,
            context: &str,
            words: &[WordSenses],
        ) -> Option<Vec<Vec<usize>>> {
            if words.is_empty() {
                return Some(Vec::new());
            }
            // DOCUMENT CONTEXT (D63, 2026-07-21): the neighbouring sentences. A sense that is
            // plausible for an isolated sentence is often obviously wrong in the passage — "regions"
            // pulls UMLS "Geographic Locations" until the surrounding genomics text rules it out.
            let context_block = if context.trim().is_empty() {
                String::new()
            } else {
                format!("Passage (for context only — do NOT rank its words):\n  {context}\n\n")
            };
            let mut prompt = format!(
                "{context_block}In the sentence:\n  \"{sentence}\"\nrank each word's candidate senses by \
                 contextual plausibility (most-likely sense first). Return `rankings`: one list \
                 per word (in the given order), listing that word's candidate indices \
                 most-plausible first.\n\n\
                 IMPORTANT — you may ELIMINATE a sense by OMITTING its index. Omit any sense that \
                 is not a possible reading of the word in THIS sentence. Do not pad the list: if \
                 only one sense is possible, return only that one index. A grammatical word like \
                 \"of\", \"may\" or \"a\" usually has exactly one reading here, and the \
                 domain-specific noun senses of such a word are never right — omit them.\n\
                 Omit a sense only when it is impossible, not merely unlikely: a sense you omit \
                 cannot be recovered.\n\nWords and candidate senses:\n"
            );
            for (wi, w) in words.iter().enumerate() {
                prompt.push_str(&format!("Word {wi} = \"{}\":\n", w.surface));
                for (ci, c) in w.candidates.iter().enumerate() {
                    prompt.push_str(&format!("  [{ci}] {}\n", c.gloss));
                }
            }
            // `EIGENIUS_DUMP_RANK_PROMPT=1` prints the exact prompt sent for each sentence — the
            // reranker decides which senses reach the parser, so being able to READ what it was
            // asked is the difference between debugging it and guessing at it.
            if std::env::var("EIGENIUS_DUMP_RANK_PROMPT").is_ok() {
                eprintln!("\n===== SENSE-RANKER PROMPT =====\n{prompt}\n===== END PROMPT =====\n");
            }
            // A failed call and a malformed reply are NON-ANSWERS, not rankings. Returning the
            // identity permutation here is what froze a failure into `ranks/2026-07-29-…` and put
            // the crab genus in front of the reading ranker months later (D69 §7e).
            let reply = self.ask(&prompt)?;
            if reply.rankings.len() != words.len() {
                eprintln!(
                    "anthropic sense-ranker: malformed reply for «{}» ({} rankings for {} words) \
                     — treating as NO ANSWER",
                    sentence.trim(),
                    reply.rankings.len(),
                    words.len()
                );
                return None;
            }
            let ranked: Vec<Vec<usize>> = reply
                .rankings
                .into_iter()
                .zip(words)
                .map(|(ranking, w)| {
                    let n = w.candidates.len();
                    let valid: Vec<usize> = ranking.into_iter().filter(|&i| i < n).collect();
                    // **An index the model OMITTED is ELIMINATED.** It used to be appended back here
                    // ("preserving completeness"), which destroyed the only signal the ranker has for
                    // saying "this sense is impossible" — a permutation can reorder but never drop.
                    // That is how `of` kept a reading of `BRIP1 wt Allele` and `may` kept `Month of
                    // May`: the model ranked the correct sense #0, and the cap, obliged to fill its
                    // quota of 2, took the next one off the restored list.
                    //
                    // Eliminated indices are still appended — but AFTER every ranked one, so
                    // `sense_cap_key` sorts them last and `lookup_span` can cut at the ranked count
                    // (see its `effective cap`). They remain reachable by widen-on-failure, so a
                    // wrong elimination costs a slower parse, never a grammar gap.
                    let mut seen = vec![false; n];
                    let mut out = Vec::with_capacity(n);
                    for i in valid {
                        if !seen[i] {
                            seen[i] = true;
                            out.push(i);
                        }
                    }
                    // NOTE: omitted indices are NOT appended. They are absent from the flattened
                    // `sense → rank` map, so `sense_cap_key` sorts them after every ranked sense
                    // (its first key is `ctx.is_none()`), and `lookup_span` cuts at the ranked
                    // count. They remain seedable once widen-on-failure raises the cap, so a wrong
                    // elimination costs a slower parse, never a grammar gap.
                    out
                })
                .collect();
            Some(ranked)
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
                sem: String::new(),
            },
            SenseCandidate {
                sense: "b".into(),
                gloss: "y".into(),
                sem: String::new(),
            },
            SenseCandidate {
                sense: "c".into(),
                gloss: "z".into(),
                sem: String::new(),
            },
        ];
        let words = vec![WordSenses {
            surface: "w",
            candidates: &cands,
        }];
        assert_eq!(
            IdentityRanker.rank("s", "", &words),
            Some(vec![vec![0, 1, 2]])
        );
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
                sem: String::new(),
            },
            SenseCandidate {
                sense: "bank.n.09".into(),
                gloss: "sloping land beside a body of water".into(),
                sem: String::new(),
            },
        ];
        let words = vec![WordSenses {
            surface: "bank",
            candidates: &cands,
        }];
        let r = ranker.rank(
            "The bank approved the loan after reviewing the application.",
            "",
            &words,
        );
        let r = r.expect("the live ranker answered");
        assert_eq!(r.len(), 1, "one ranking for the one word");
        assert_eq!(r[0].len(), 2, "a permutation of both candidates");
        assert_eq!(
            r[0][0], 0,
            "the financial sense ranks first in a loan context, got {:?}",
            r[0]
        );
    }

    /// **A NON-ANSWER IS NEVER RECORDED** (D69 §7e). The regression this locks: the failure path
    /// used to return the identity permutation, the recorder wrote it as a ranking, and the draw
    /// replayed it forever as "keep every sense" while reporting a HIT. One such frozen failure
    /// in `ranks/2026-07-29-demonstratives.json` put the crab genus and the astrological sign in
    /// front of the reading ranker; re-asked, the live ranker keeps only the disease sense.
    #[test]
    fn a_ranker_that_does_not_answer_records_nothing() {
        struct NoAnswer;
        impl SenseRanker for NoAnswer {
            fn rank(&self, _s: &str, _c: &str, _w: &[WordSenses]) -> Option<Vec<Vec<usize>>> {
                None
            }
        }
        let c = cands(3);
        let words = vec![WordSenses {
            surface: "cancers",
            candidates: &c,
        }];
        let rec = RecordingSenseRanker::new(NoAnswer);
        assert_eq!(
            rec.rank("s", "", &words),
            None,
            "the non-answer propagates — the caller falls back to seed order itself"
        );
        let dir = std::env::temp_dir().join("eigenius-rank-nonanswer-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ranks.json");
        assert_eq!(
            rec.write(&path).unwrap(),
            0,
            "nothing recorded, so a re-run ASKS AGAIN instead of inheriting the failure"
        );
    }

    // ── record / replay ──────────────────────────────────────────────────────

    /// A ranker that reverses each word's candidates — a stand-in for the LLM: it returns a
    /// non-identity order, so a replay that silently fell back to the seed order would be caught.
    struct ReverseRanker;
    impl SenseRanker for ReverseRanker {
        fn rank(&self, _s: &str, _c: &str, words: &[WordSenses]) -> Option<Vec<Vec<usize>>> {
            Some(
                words
                    .iter()
                    .map(|w| (0..w.candidates.len()).rev().collect())
                    .collect(),
            )
        }
    }

    fn cands(n: usize) -> Vec<SenseCandidate> {
        (0..n)
            .map(|i| SenseCandidate {
                sense: format!("wn:s{i}"),
                gloss: format!("gloss {i}"),
                sem: format!("wn:n{i}"),
            })
            .collect()
    }

    #[test]
    fn a_replay_reproduces_the_recorded_rankings_exactly_and_makes_no_calls() {
        let c = cands(3);
        let words = vec![WordSenses {
            surface: "bank",
            candidates: &c,
        }];

        let rec = RecordingSenseRanker::new(ReverseRanker);
        let live = rec.rank("we sat on the bank", "", &words);
        assert_eq!(
            live,
            Some(vec![vec![2, 1, 0]]),
            "the inner ranker's answer passes through"
        );

        let dir = std::env::temp_dir().join("eigenius-rank-replay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ranks.json");
        assert_eq!(rec.write(&path).unwrap(), 1);

        // Replay: same question → the SAME answer, with no ranker behind it at all.
        let replay = ReplaySenseRanker::load(&path).unwrap();
        let got = replay.rank("we sat on the bank", "", &words);
        assert_eq!(
            got, live,
            "replay must reproduce the recorded ranking exactly"
        );
        assert_eq!(replay.hits(), 1);
        assert_eq!(replay.misses(), 0, "a faithful replay misses nothing");
    }

    #[test]
    fn a_replay_miss_is_counted_not_hidden() {
        let c = cands(2);
        let words = vec![WordSenses {
            surface: "bank",
            candidates: &c,
        }];
        let rec = RecordingSenseRanker::new(ReverseRanker);
        rec.rank("sentence A", "", &words);
        let dir = std::env::temp_dir().join("eigenius-rank-replay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ranks-miss.json");
        rec.write(&path).unwrap();

        let replay = ReplaySenseRanker::load(&path).unwrap();
        // A DIFFERENT sentence — the recording cannot answer it.
        let got = replay.rank("sentence B", "", &words);
        assert_eq!(
            got, None,
            "a miss is a NON-ANSWER — the caller falls back to seed order, and nothing about the \
             miss can be mistaken for a recorded ranking"
        );
        assert_eq!(
            replay.misses(),
            1,
            "and the miss is COUNTED — a replay with misses is not a reproduction"
        );

        // A different CANDIDATE SET is also a different question (the lexicon changed under it).
        let c2 = cands(3);
        let words2 = vec![WordSenses {
            surface: "bank",
            candidates: &c2,
        }];
        replay.rank("sentence A", "", &words2);
        assert_eq!(
            replay.misses(),
            2,
            "a changed sense-set must MISS, not replay a stale answer"
        );
    }
}
