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
//! Impls: a deterministic mock ([`IdentityRanker`]) for CI, and the live ranker
//! ([`DecisionSenseRanker`]): one question per word put to a [`Decider`] (eigenius#264's decision
//! interface), so any provider answers it — Anthropic with a ranking, TypeSafe with a probability
//! per sense. A ranking carries a weight per sense, and the parser eliminates a sense whose weight
//! is below its floor (`Parser::with_sense_floor`), so the floor is tuned on a replay, not
//! re-asked.

use crate::dcg::decision::{Choice, Decider, Question};

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

/// One word's ranking: candidate indices, most plausible in context first, and — where the ranker
/// gives them — a weight per candidate, aligned with the candidates: the provider's probability,
/// or for a ranking without probabilities 1 for a ranked sense and 0 for one it leaves out.
///
/// Without weights, a candidate left out of `order` is ELIMINATED (the rankers before 2026-10-01,
/// and every recording they made). With weights, `order` holds every candidate and the parser
/// eliminates the ones whose weight is below its floor.
#[derive(Clone, Debug, PartialEq)]
pub struct WordRanking {
    pub order: Vec<usize>,
    pub weights: Vec<f64>,
}

impl WordRanking {
    /// A ranking without weights: `order`'s candidates kept, the rest eliminated.
    pub fn ordered(order: Vec<usize>) -> Self {
        WordRanking {
            order,
            weights: Vec::new(),
        }
    }

    /// The candidates kept at `floor`: `order`, less those weighted below it.
    pub fn kept(&self, floor: f64) -> impl Iterator<Item = usize> + '_ {
        self.order
            .iter()
            .copied()
            .filter(move |&i| self.weights.get(i).is_none_or(|&w| w >= floor))
    }
}

/// The **untrusted** contextual sense reranker. Given the `sentence` and one [`WordSenses`] per
/// content word, return a [`WordRanking`] per word, aligned with `words` (callers must tolerate a
/// malformed reply by falling back to the seed order).
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
    fn rank(&self, sentence: &str, context: &str, words: &[WordSenses])
        -> Option<Vec<WordRanking>>;

    /// The model that answers, for a ranker that asks one — what a recording names (D71 §9).
    fn model(&self) -> Option<String> {
        None
    }
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
    ) -> Option<Vec<WordRanking>> {
        Some(
            words
                .iter()
                .map(|w| WordRanking::ordered((0..w.candidates.len()).collect()))
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
    /// The model that answered, where the ranker names one (recordings before 2026-10-01 do not).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
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
    /// The ranker's weight per sense, aligned with `senses` — see [`WordRanking`]. Empty in a
    /// recording whose ranker gave none: there a sense left out of `order` was eliminated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weights: Vec<f64>,
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
    fn model(&self) -> Option<String> {
        self.inner.model()
    }

    fn rank(
        &self,
        sentence: &str,
        context: &str,
        words: &[WordSenses],
    ) -> Option<Vec<WordRanking>> {
        // A NON-ANSWER IS NEVER RECORDED. Writing the fallback would freeze a failed call into the
        // artifact, where it replays as "keep every sense" and reports a HIT — the 2026-07-29 crab
        // bug. Nothing recorded ⇒ the next run asks again.
        let order = self.inner.rank(sentence, context, words)?;
        // An answer that eliminates NOTHING anywhere is either a ranker declining to work or a
        // failure that slipped through some other impl's fallback. Say so at record time, where
        // the run can still be repeated, rather than leaving it to be discovered in a draw months
        // later.
        if words.len() > 1
            && words.iter().zip(order.iter()).all(|(w, o)| {
                o.weights.is_empty() && o.order.iter().copied().eq(0..w.candidates.len())
            })
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
                    order: o.order.clone(),
                    weights: o.weights.clone(),
                })
                .collect(),
            model: self.inner.model().unwrap_or_default(),
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
    by_key: std::collections::BTreeMap<String, Vec<WordRanking>>,
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
            by_key.insert(
                key,
                r.words
                    .iter()
                    .map(|w| WordRanking {
                        order: w.order.clone(),
                        weights: w.weights.clone(),
                    })
                    .collect(),
            );
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
    fn rank(
        &self,
        sentence: &str,
        context: &str,
        words: &[WordSenses],
    ) -> Option<Vec<WordRanking>> {
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

// ───────────────────────── the live ranker: a decision per word ─────────────────────────

/// A sense call's question about one word. A sense the sentence rules out is left out of a
/// ranking: Anthropic's models rank, and an unranked sense weighs least; TypeSafe's give it a low
/// probability. Either way the parser's floor eliminates it.
fn sense_question(surface: &str) -> String {
    format!(
        "Which sense of «{surface}» does `the_sentence` use? A sense `the_sentence` rules out is \
         not a runner-up."
    )
}

/// The sentence's sense ranking as one [`Choice`]: the passage (when the parser's context window
/// gives one) and the sentence, and one question per word, its senses keyed `1`, `2`, … by their
/// glosses.
pub fn sense_choice(sentence: &str, context: &str, words: &[WordSenses]) -> Choice {
    Choice {
        context: vec![
            ("document".to_string(), context.to_string()),
            ("the_sentence".to_string(), sentence.trim().to_string()),
        ],
        questions: words
            .iter()
            .map(|w| Question {
                question: sense_question(w.surface),
                notes: Vec::new(),
                options: w
                    .candidates
                    .iter()
                    .enumerate()
                    .map(|(i, c)| ((i + 1).to_string(), c.gloss.trim().into()))
                    .collect(),
            })
            .collect(),
    }
}

/// **The live sense ranker** (eigenius#264's decision interface; D63 phase 2): the sentence's words
/// as one [`Choice`], one question per word, put to a [`Decider`]. Each answer becomes a
/// [`WordRanking`] over every sense: with the provider's probabilities, ordered and weighted by
/// them; with a ranking, its order, a ranked sense weighing 1 and a left-out one 0. The parser's
/// floor does the eliminating. A decider error or an answer that does not cover every
/// word is NO ANSWER (D69 §7e): the caller falls back to seed order and nothing is recorded.
pub struct DecisionSenseRanker<D: Decider> {
    decider: D,
}

impl<D: Decider> DecisionSenseRanker<D> {
    pub fn new(decider: D) -> Self {
        Self { decider }
    }
}

impl<D: Decider> SenseRanker for DecisionSenseRanker<D> {
    fn model(&self) -> Option<String> {
        Some(self.decider.model().to_string())
    }

    fn rank(
        &self,
        sentence: &str,
        context: &str,
        words: &[WordSenses],
    ) -> Option<Vec<WordRanking>> {
        if words.is_empty() {
            return Some(Vec::new());
        }
        let choice = sense_choice(sentence, context, words);
        // `EIGENIUS_DUMP_RANK_PROMPT=1` prints what the ranker was asked: it decides which senses
        // reach the parser, so reading the question is how it is debugged.
        if std::env::var("EIGENIUS_DUMP_RANK_PROMPT").is_ok() {
            eprintln!(
                "\n===== SENSE-RANKER CHOICE =====\n{}\n===== END =====\n",
                crate::dcg::decision::render_prompt(&choice)
            );
        }
        let decided = match self.decider.choose(&choice) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("sense-ranker ({}): {e}", self.decider.model());
                return None;
            }
        };
        if decided.len() != words.len() {
            eprintln!(
                "sense-ranker: {} answers for {} words of «{}» — treating as NO ANSWER",
                decided.len(),
                words.len(),
                sentence.trim()
            );
            return None;
        }
        Some(
            words
                .iter()
                .zip(&decided)
                .map(|(w, d)| word_ranking(d, w.candidates.len()))
                .collect(),
        )
    }
}

/// One answer as a ranking of `n` senses keyed `1`…`n`. With probabilities: every sense, by
/// probability. Without: the choice and the runners-up in the answer's order, weighing 1, then the
/// senses the answer leaves out, weighing 0.
fn word_ranking(d: &crate::dcg::decision::Decided, n: usize) -> WordRanking {
    let key = |i: usize| (i + 1).to_string();
    if !d.probabilities.is_empty() {
        let weights: Vec<f64> = (0..n)
            .map(|i| d.probabilities.get(&key(i)).copied().unwrap_or(0.0))
            .collect();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| weights[b].total_cmp(&weights[a]).then(a.cmp(&b)));
        return WordRanking { order, weights };
    }
    let mut order: Vec<usize> = std::iter::once(&d.choice)
        .chain(&d.runners_up)
        .filter_map(|k| (0..n).find(|&i| key(i) == *k))
        .collect();
    let mut weights = vec![0.0; n];
    for &i in &order {
        weights[i] = 1.0;
    }
    order.extend((0..n).filter(|i| weights[*i] == 0.0));
    WordRanking { order, weights }
}

/// A live sense ranker, whichever provider answers it.
pub type LiveSenseRanker = Box<dyn SenseRanker + Send + Sync>;

/// The live sense ranker for `cfg`'s model, over its provider's decider; `None` without the
/// provider's key in the environment.
#[cfg(feature = "use-llm")]
pub fn live_sense_ranker(cfg: &crate::dcg::model_config::ModelConfig) -> Option<LiveSenseRanker> {
    crate::dcg::decision::decider_from_env(cfg)
        .map(|d| Box::new(DecisionSenseRanker::new(d)) as LiveSenseRanker)
}

/// [`live_sense_ranker`] for the model in `EIGENIUS_SENSE_MODEL`, else
/// [`DEFAULT_MODEL`](crate::dcg::model_config::DEFAULT_MODEL).
#[cfg(feature = "use-llm")]
pub fn live_sense_ranker_from_env() -> Option<LiveSenseRanker> {
    use crate::dcg::model_config::{ModelConfig, DEFAULT_MODEL};
    let model = std::env::var("EIGENIUS_SENSE_MODEL").unwrap_or_default();
    live_sense_ranker(&ModelConfig::requested(&model, DEFAULT_MODEL, 0))
}

impl<T: SenseRanker + ?Sized> SenseRanker for Box<T> {
    fn rank(
        &self,
        sentence: &str,
        context: &str,
        words: &[WordSenses],
    ) -> Option<Vec<WordRanking>> {
        (**self).rank(sentence, context, words)
    }
    fn model(&self) -> Option<String> {
        (**self).model()
    }
}

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
            Some(vec![WordRanking::ordered(vec![0, 1, 2])])
        );
    }

    /// A decider that answers each word with fixed probabilities over its senses: `1` most likely,
    /// the last sense near zero.
    struct Probabilities;
    impl Decider for Probabilities {
        fn choose(&self, choice: &Choice) -> Result<Vec<crate::dcg::decision::Decided>, String> {
            Ok(choice
                .questions
                .iter()
                .map(|q| {
                    let n = q.options.len();
                    let probabilities = (1..=n)
                        .map(|k| {
                            let p = if k == n { 0.001 } else { 1.0 / k as f64 };
                            (k.to_string(), p)
                        })
                        .collect();
                    crate::dcg::decision::Decided {
                        choice: "1".into(),
                        runners_up: (2..=n).map(|k| k.to_string()).collect(),
                        probabilities,
                        rationale: String::new(),
                        model: "fixed".into(),
                    }
                })
                .collect())
        }
        fn model(&self) -> &str {
            "fixed"
        }
    }

    /// The decision ranker asks one question per word and orders every sense by its weight; the
    /// floor, not the ranker, eliminates.
    #[test]
    fn the_decision_ranker_weights_every_sense_and_the_floor_eliminates() {
        let c = cands(3);
        let words = vec![
            WordSenses {
                surface: "respond",
                candidates: &c,
            },
            WordSenses {
                surface: "cancers",
                candidates: &c[..2],
            },
        ];
        let choice = sense_choice("Some cancers do not respond.", "", &words);
        assert_eq!(choice.questions.len(), 2);
        assert_eq!(
            choice.questions[0].question,
            "Which sense of «respond» does `the_sentence` use? A sense `the_sentence` rules out \
             is not a runner-up."
        );
        assert_eq!(
            choice.questions[0].options[1],
            ("2".to_string(), "gloss 1".into())
        );
        let r = DecisionSenseRanker::new(Probabilities)
            .rank("Some cancers do not respond.", "", &words)
            .unwrap();
        assert_eq!(r[0].order, [0, 1, 2]);
        assert_eq!(r[0].weights, [1.0, 0.5, 0.001]);
        assert_eq!(r[0].kept(0.02).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(
            r[1].kept(0.02).collect::<Vec<_>>(),
            [0],
            "0.001 is below the floor"
        );
        assert_eq!(
            WordRanking::ordered(vec![2, 0])
                .kept(0.5)
                .collect::<Vec<_>>(),
            [2, 0],
            "without weights, `order` is what is kept"
        );
        // A ranking without probabilities: the ranked senses weigh 1, a left-out one 0, so any
        // floor above 0 eliminates exactly what the ranker left out, however long the ranking.
        let ranked = crate::dcg::decision::Decided {
            choice: "3".into(),
            runners_up: vec!["1".into()],
            probabilities: Default::default(),
            rationale: "loans".into(),
            model: "claude".into(),
        };
        let r = word_ranking(&ranked, 3);
        assert_eq!(r.order, [2, 0, 1]);
        assert_eq!(r.weights, [1.0, 0.0, 1.0]);
        assert_eq!(r.kept(DEFAULT_FLOOR_FOR_TEST).collect::<Vec<_>>(), [2, 0]);
    }

    const DEFAULT_FLOOR_FOR_TEST: f64 = crate::dcg::parse::DEFAULT_SENSE_FLOOR;

    /// Live WSD through the decision interface: a real model must put the contextual sense first.
    /// Skips without a key; runs live with `--features use-llm` and the provider's key.
    #[cfg(feature = "use-llm")]
    #[test]
    fn live_sense_ranker_picks_the_contextual_sense() {
        let Some(ranker) = live_sense_ranker_from_env() else {
            eprintln!("SKIP live_sense_ranker: no key for the sense model");
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
        let r = ranker
            .rank(
                "The bank approved the loan after reviewing the application.",
                "",
                &words,
            )
            .expect("the live ranker answered");
        assert_eq!(r.len(), 1, "one ranking for the one word");
        assert_eq!(
            r[0].order[0], 0,
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
            fn rank(&self, _s: &str, _c: &str, _w: &[WordSenses]) -> Option<Vec<WordRanking>> {
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
        fn rank(&self, _s: &str, _c: &str, words: &[WordSenses]) -> Option<Vec<WordRanking>> {
            Some(
                words
                    .iter()
                    .map(|w| WordRanking::ordered((0..w.candidates.len()).rev().collect()))
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
            Some(vec![WordRanking::ordered(vec![2, 1, 0])]),
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
