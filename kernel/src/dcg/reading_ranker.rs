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

//! **Reading selection** (D63 reading-selection note; parser-pipeline plan Stage 1) — choose ONE
//! reading from a sentence's surviving parse forest, in document context.
//!
//! This is the post-parse sibling of [`crate::dcg::sense_ranker`]: the sense ranker reorders the
//! *pre-parse* seed beam per word; a [`ReadingRanker`] chooses among the *assembled* readings
//! (`Vec<Item>`) that survive parsing, felicity, and dedup. The two differ in their trust story:
//! a wrongly-ranked sense fails to parse (the kernel felicity gate vetoes), but **every reading
//! candidate here already type-checks — there is no kernel veto on selection.** The controls are
//! instead: the recorded decision + rationale (emitted as the claim's `enc:DecisionPoint`), the
//! offline faithfulness gate (`selection_accuracy` against the human pins in
//! `experiments/parsing/expected-readings.tsv`), and the adjudication ledger (an
//! `invalid`-adjudicated skeleton being selectable at all is a grammar bug to fix, not a runtime
//! filter).
//!
//! **Document context is part of the contract, not a hint.** Reading choice (PP attachment,
//! coordination scope, sense) is frequently decided by the surrounding prose, so `select` takes a
//! [`DocumentContext`] — the surrounding input text, the target sentence, and the glosses of
//! prior sentences' already-selected readings (sequential consistency as the discourse loop
//! advances). The record/replay key covers the whole context, so a context change is a counted
//! MISS, never a silent reuse.
//!
//! **Abstention is legal and fail-open.** `select` returns `None` to abstain; the sentence then
//! stays `Ambiguous` — never a forced wrong choice. A replay miss abstains (and is counted): a
//! recording cannot answer a question it was not asked, and unlike the sense ranker there is no
//! harmless seed order to fall back to — a fabricated selection would be a wrong *answer*, not a
//! slower parse.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

/// One reading of the sentence, as presented to the ranker. Candidates are presented grouped by
/// skeleton (the caller orders them); selection is flat over readings, but accuracy is measured at
/// skeleton granularity (gold labels exist only there).
#[derive(Clone, Debug)]
pub struct ReadingCandidate {
    /// `skeleton_of(item.sem())` — the sense-erased structure key the pins and the adjudication
    /// ledger are written in.
    pub skeleton: String,
    /// The verbalised gloss ([`crate::dcg::verbalize`]) — names concrete senses, so the ranker
    /// sees both ambiguity axes. Fail-honest: unrenderable structure appears as `⟦…⟧`.
    pub gloss: String,
    /// The pretty-printed λ-term — the reading's identity, for the record and the prompt appendix.
    pub sem: String,
}

/// A prior sentence's already-selected reading — part of the question for every later sentence
/// (sequential consistency: what "these lines" was taken to mean upstream constrains the reading
/// of the sentence that mentions them).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PriorSelection {
    /// 0-based sentence ordinal within the document.
    pub ordinal: usize,
    /// The selected reading's gloss.
    pub gloss: String,
}

/// The question's context: the surrounding input text with the target sentence identified, plus
/// the prior selections. The `document` is the input the discourse loop is iterating (for the
/// in-process pipeline: the segmented sentences, in order).
pub struct DocumentContext<'a> {
    /// The full surrounding input text (preceding AND following sentences).
    pub document: &'a str,
    /// The target sentence (verbatim, as it occurs in `document`).
    pub sentence: &'a str,
    /// Glosses of the already-selected readings of prior sentences, in document order.
    pub prior_selections: &'a [PriorSelection],
}

/// A ranker's answer: the chosen candidate plus the audit trail the caller records.
#[derive(Clone, Debug)]
pub struct ReadingSelection {
    /// Index into the candidate slice.
    pub chosen: usize,
    /// Why — recorded verbatim into the emitted decision record.
    pub rationale: String,
    /// The remaining candidates in preference order (chosen excluded), most-preferred first. May
    /// be empty when the impl has no meaningful order for the rest (e.g. a pin).
    pub runners_up: Vec<usize>,
}

/// The **untrusted** reading selector. Given the document context and the sentence's surviving
/// readings, choose one — or abstain (`None`), leaving the sentence `Ambiguous`. Implementations:
/// the live LLM ranker (`use-llm`), record/replay wrappers, the pin-backed gate arm, deterministic
/// mocks. No kernel veto exists on this choice — see the module doc for the controls.
pub trait ReadingRanker {
    fn select(
        &self,
        ctx: &DocumentContext,
        candidates: &[ReadingCandidate],
    ) -> Option<ReadingSelection>;
}

/// The pin-backed selector — the ground-truth/gate arm. Selects the candidate whose skeleton
/// equals the sentence's pinned skeleton; abstains when the sentence has no pin, the pin matches
/// no candidate, or **two or more candidates share the pinned skeleton** (sense-level ambiguity a
/// skeleton pin cannot adjudicate — the same fail-closed rule as `select_pinned` in
/// `eigenius-encoding`).
pub struct PinReadingRanker {
    /// sentence → pinned skeleton.
    pins: BTreeMap<String, String>,
}

impl PinReadingRanker {
    pub fn new(pins: BTreeMap<String, String>) -> Self {
        Self { pins }
    }
}

impl ReadingRanker for PinReadingRanker {
    fn select(
        &self,
        ctx: &DocumentContext,
        candidates: &[ReadingCandidate],
    ) -> Option<ReadingSelection> {
        let pin = self.pins.get(ctx.sentence.trim())?;
        let matches: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| &c.skeleton == pin)
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [one] => Some(ReadingSelection {
                chosen: *one,
                rationale: "pinned skeleton (expected-readings corpus)".to_string(),
                runners_up: Vec::new(),
            }),
            _ => None, // no match, or ≥2 readings share the pinned skeleton — abstain
        }
    }
}

// ───────────────────────── record / replay (reproducibility) ─────────────────────────

/// A recorded selection: the exact question put to the ranker, and the answer it gave.
///
/// The document is stored as its SHA-256 (hex) rather than verbatim — the surrounding text is
/// part of the KEY (a changed document must MISS), but repeating the full page in every record
/// would bloat a committed recording without adding information the run directory doesn't already
/// hold. Everything else the model saw — sentence, prior-selection glosses, candidate skeletons,
/// glosses, and sems — is recorded verbatim.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SelectionRecord {
    pub sentence: String,
    pub document_sha256: String,
    #[serde(default)]
    pub prior_selections: Vec<PriorSelection>,
    pub candidates: Vec<RecordedCandidate>,
    pub chosen: usize,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub runners_up: Vec<usize>,
}

/// One candidate as recorded — skeleton, gloss, and sem, exactly as presented.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RecordedCandidate {
    pub skeleton: String,
    pub gloss: String,
    pub sem: String,
}

fn document_sha(document: &str) -> String {
    format!("{:x}", Sha256::digest(document.as_bytes()))
}

/// The lookup key for a selection: everything that was part of the question. The same sentence
/// with a different surrounding document, different prior selections, or a different candidate
/// set (skeletons, glosses, OR sems — all three are presented to the model) is a different
/// question and must MISS rather than silently replay a stale answer.
fn selection_key(
    sentence: &str,
    document_sha256: &str,
    prior: &[PriorSelection],
    candidates: &[RecordedCandidate],
) -> String {
    let mut k = String::from(sentence);
    k.push('\u{1d}');
    k.push_str(document_sha256);
    for p in prior {
        k.push('\u{1f}');
        k.push_str(&p.ordinal.to_string());
        k.push('\u{1e}');
        k.push_str(&p.gloss);
    }
    k.push('\u{1c}');
    for c in candidates {
        k.push('\u{1f}');
        k.push_str(&c.skeleton);
        k.push('\u{1e}');
        k.push_str(&c.gloss);
        k.push('\u{1e}');
        k.push_str(&c.sem);
    }
    k
}

fn recorded_candidates(candidates: &[ReadingCandidate]) -> Vec<RecordedCandidate> {
    candidates
        .iter()
        .map(|c| RecordedCandidate {
            skeleton: c.skeleton.clone(),
            gloss: c.gloss.clone(),
            sem: c.sem.clone(),
        })
        .collect()
}

/// **Record** every selection an inner ranker produces (abstentions are not recorded — an absent
/// key replays as an abstention anyway). Flush with [`Self::write`]. Same rationale as
/// [`crate::dcg::sense_ranker::RecordingSenseRanker`]: the LLM is the one component that can
/// answer differently for the same code and store; recording turns it from an uncontrolled input
/// into a recorded one.
pub struct RecordingReadingRanker<R: ReadingRanker> {
    inner: R,
    log: Mutex<BTreeMap<String, SelectionRecord>>,
}

impl<R: ReadingRanker> RecordingReadingRanker<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            log: Mutex::new(BTreeMap::new()),
        }
    }

    /// Write the recorded selections as JSON (sorted by key — deterministic bytes).
    pub fn write(&self, path: &Path) -> std::io::Result<usize> {
        let log = self.log.lock().expect("selection log");
        let records: Vec<&SelectionRecord> = log.values().collect();
        let json = serde_json::to_string_pretty(&records)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)?;
        Ok(records.len())
    }
}

impl<R: ReadingRanker> ReadingRanker for RecordingReadingRanker<R> {
    fn select(
        &self,
        ctx: &DocumentContext,
        candidates: &[ReadingCandidate],
    ) -> Option<ReadingSelection> {
        let selection = self.inner.select(ctx, candidates)?;
        let recorded = recorded_candidates(candidates);
        let sha = document_sha(ctx.document);
        let key = selection_key(ctx.sentence, &sha, ctx.prior_selections, &recorded);
        self.log.lock().expect("selection log").insert(
            key,
            SelectionRecord {
                sentence: ctx.sentence.to_string(),
                document_sha256: sha,
                prior_selections: ctx.prior_selections.to_vec(),
                candidates: recorded,
                chosen: selection.chosen,
                rationale: selection.rationale.clone(),
                runners_up: selection.runners_up.clone(),
            },
        );
        Some(selection)
    }
}

/// **Replay** selections recorded by [`RecordingReadingRanker`] — no LLM, no network,
/// deterministic. A miss **abstains** (`None` — the sentence stays `Ambiguous`) and is COUNTED:
/// unlike the sense ranker there is no harmless fallback order, so a recording that cannot answer
/// the question must not invent one. [`Self::misses`] must be 0 for a replay to be a faithful
/// reproduction; a non-zero count means the document, lexicon, glosses, or an upstream selection
/// changed under the recording, and the run is a different experiment.
pub struct ReplayReadingRanker {
    by_key: BTreeMap<String, SelectionRecord>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl ReplayReadingRanker {
    /// Load a recording written by [`RecordingReadingRanker::write`].
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let records: Vec<SelectionRecord> = serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut by_key = BTreeMap::new();
        for r in records {
            // Rebuild the key from the recorded question, so it matches what `select` computes.
            let k = selection_key(
                &r.sentence,
                &r.document_sha256,
                &r.prior_selections,
                &r.candidates,
            );
            by_key.insert(k, r);
        }
        Ok(Self {
            by_key,
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        })
    }

    /// Selections replayed from the recording.
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    /// Questions NOT found in the recording (abstained). **Must be 0** for the replay to
    /// reproduce the recorded run.
    pub fn misses(&self) -> usize {
        self.misses.load(Ordering::Relaxed)
    }
}

impl ReadingRanker for ReplayReadingRanker {
    fn select(
        &self,
        ctx: &DocumentContext,
        candidates: &[ReadingCandidate],
    ) -> Option<ReadingSelection> {
        let recorded = recorded_candidates(candidates);
        let sha = document_sha(ctx.document);
        let key = selection_key(ctx.sentence, &sha, ctx.prior_selections, &recorded);
        match self.by_key.get(&key) {
            Some(r) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(ReadingSelection {
                    chosen: r.chosen,
                    rationale: r.rationale.clone(),
                    runners_up: r.runners_up.clone(),
                })
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands(n: usize) -> Vec<ReadingCandidate> {
        (0..n)
            .map(|i| ReadingCandidate {
                skeleton: format!("skel-{i}"),
                gloss: format!("gloss {i}"),
                sem: format!("sem {i}"),
            })
            .collect()
    }

    fn ctx<'a>(
        document: &'a str,
        sentence: &'a str,
        prior: &'a [PriorSelection],
    ) -> DocumentContext<'a> {
        DocumentContext {
            document,
            sentence,
            prior_selections: prior,
        }
    }

    /// A deterministic stand-in for the LLM: always chooses the LAST candidate (a non-trivial
    /// answer, so a replay that silently fell back to "first" would be caught).
    struct LastRanker;
    impl ReadingRanker for LastRanker {
        fn select(
            &self,
            _ctx: &DocumentContext,
            candidates: &[ReadingCandidate],
        ) -> Option<ReadingSelection> {
            let n = candidates.len();
            if n == 0 {
                return None;
            }
            Some(ReadingSelection {
                chosen: n - 1,
                rationale: "last".to_string(),
                runners_up: (0..n - 1).rev().collect(),
            })
        }
    }

    #[test]
    fn pin_ranker_selects_the_unique_pinned_skeleton_and_abstains_otherwise() {
        let mut pins = BTreeMap::new();
        pins.insert("S one.".to_string(), "skel-1".to_string());
        let ranker = PinReadingRanker::new(pins);
        let c = cands(3);

        let sel = ranker
            .select(&ctx("S one. S two.", "S one.", &[]), &c)
            .expect("pin matches");
        assert_eq!(sel.chosen, 1);

        // No pin for the sentence → abstain.
        assert!(ranker.select(&ctx("S two.", "S two.", &[]), &c).is_none());

        // Two candidates share the pinned skeleton → abstain (sense-level tie a pin cannot break).
        let mut tied = cands(2);
        tied[0].skeleton = "skel-1".to_string();
        tied[1].skeleton = "skel-1".to_string();
        assert!(ranker
            .select(&ctx("S one.", "S one.", &[]), &tied)
            .is_none());
    }

    #[test]
    fn a_replay_reproduces_the_recorded_selection_exactly() {
        let c = cands(3);
        let prior = vec![PriorSelection {
            ordinal: 0,
            gloss: "prior gloss".to_string(),
        }];
        let rec = RecordingReadingRanker::new(LastRanker);
        let live = rec
            .select(&ctx("Doc text.", "Doc text.", &prior), &c)
            .expect("inner ranker answers");
        assert_eq!(live.chosen, 2);

        let dir = std::env::temp_dir().join("eigenius-selection-replay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("selections.json");
        assert_eq!(rec.write(&path).unwrap(), 1);

        let replay = ReplayReadingRanker::load(&path).unwrap();
        let got = replay
            .select(&ctx("Doc text.", "Doc text.", &prior), &c)
            .expect("same question → recorded answer");
        assert_eq!(got.chosen, live.chosen);
        assert_eq!(got.runners_up, live.runners_up);
        assert_eq!(replay.hits(), 1);
        assert_eq!(replay.misses(), 0, "a faithful replay misses nothing");
    }

    #[test]
    fn a_replay_miss_abstains_and_is_counted() {
        let c = cands(2);
        let rec = RecordingReadingRanker::new(LastRanker);
        rec.select(&ctx("Doc A.", "Doc A.", &[]), &c).unwrap();
        let dir = std::env::temp_dir().join("eigenius-selection-replay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("selections-miss.json");
        rec.write(&path).unwrap();
        let replay = ReplayReadingRanker::load(&path).unwrap();

        // A different DOCUMENT is a different question, even for an identical sentence.
        assert!(replay.select(&ctx("Doc B.", "Doc A.", &[]), &c).is_none());
        assert_eq!(replay.misses(), 1, "counted, not hidden");

        // Different PRIOR SELECTIONS are a different question (an upstream choice changed).
        let prior = vec![PriorSelection {
            ordinal: 0,
            gloss: "changed".to_string(),
        }];
        assert!(replay
            .select(&ctx("Doc A.", "Doc A.", &prior), &c)
            .is_none());

        // A different CANDIDATE SET is a different question (the forest or lexicon changed).
        let c3 = cands(3);
        assert!(replay.select(&ctx("Doc A.", "Doc A.", &[]), &c3).is_none());
        assert_eq!(replay.misses(), 3);

        // The recorded question still replays.
        assert!(replay.select(&ctx("Doc A.", "Doc A.", &[]), &c).is_some());
        assert_eq!(replay.hits(), 1);
    }
}
