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

//! **Discourse-kind assignment** (D68 §4) — what a landed claim IS in the document's own terms
//! (a finding / observation / classification / hypothesis / suggestion / assertion), carried as
//! a second `is_a` class beside `enc:EncodedClaim`.
//!
//! Three-step assignment, per claim:
//! 1. **Matrix-frame evidence** ([`frame_kind`]) — deterministic: a sentence whose surface
//!    carries a marked frame («We **hypothesized** that…») wears its kind on its sleeve.
//! 2. **The recorded classifier** ([`KindClassifier`]) for unmarked declaratives — a judgment,
//!    treated like every judgment in this pipeline: an untrusted impl behind a trait, RECORDED
//!    (`kinds.json`, the ranks/selections/proposals sibling), replay-only in artifact
//!    generation, adjudicable. There is no kernel veto on kind assignment; the restrictor check
//!    downstream is sound RELATIVE to assigned kinds (the honest statement — D68 §4).
//! 3. **Default `enc:Assertion`** — unmarked and unclassified: the claim lands unreferable
//!    (Assertion is deliberately aligned to no lexicon class; fail-closed).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use eigenius_kernel::ontology::iri::Iri;

/// `urn:eigenius:encoding:<Kind>` — the closed kind classes (encoding.esl; D68 §2).
pub const KIND_FINDING: &str = "urn:eigenius:encoding:Finding";
pub const KIND_OBSERVATION: &str = "urn:eigenius:encoding:Observation";
pub const KIND_CLASSIFICATION: &str = "urn:eigenius:encoding:Classification";
pub const KIND_HYPOTHESIS: &str = "urn:eigenius:encoding:Hypothesis";
pub const KIND_SUGGESTION: &str = "urn:eigenius:encoding:Suggestion";
pub const KIND_ASSERTION: &str = "urn:eigenius:encoding:Assertion";

/// Deterministic matrix-frame evidence: the closed frame table (D68 §4 step 1). Surface-level
/// on purpose — the frame IS in the surface, and a table over lemmas keeps the deterministic
/// arm free of sense-atom conventions. `None` = unmarked (the classifier's question).
pub fn frame_kind(sentence: &str) -> Option<Iri> {
    let s = sentence.to_lowercase();
    let kind = if s.contains("hypothesized that") || s.contains("hypothesize that") {
        KIND_HYPOTHESIS
    } else if s.contains("suggest that") || s.contains("suggests that") {
        KIND_SUGGESTION
    } else {
        return None;
    };
    Some(Iri::parse(kind).expect("static kind IRI"))
}

/// The **untrusted** kind classifier for UNMARKED declaratives: which discourse-kind class(es)
/// the claim carries. Empty ⇒ no verdict ⇒ the lander defaults to `enc:Assertion`
/// (unreferable). Impls: the live LLM classifier (`use-llm`), the record/replay pair, and
/// [`NoKindClassifier`] (always abstains — the deterministic floor).
pub trait KindClassifier {
    /// The kinds this claim carries, and — when the classifier is a judgment rather than a
    /// lookup — one sentence saying why. The rationale is RECORDED, not consumed: a kind verdict
    /// is an untrusted judgment awaiting human sign-off, and a draw without its reasoning cannot
    /// be reviewed (the same discipline the selection and proposal draws already follow).
    fn classify(&self, ordinal: usize, sentence: &str, gloss: &str) -> KindVerdict;
}

/// A classifier's answer: the kinds, plus the reasoning when there is any.
#[derive(Debug, Default, Clone)]
pub struct KindVerdict {
    pub kinds: Vec<Iri>,
    pub rationale: Option<String>,
}

impl KindVerdict {
    /// A verdict with no stated reasoning (the deterministic arms).
    pub fn bare(kinds: Vec<Iri>) -> Self {
        Self {
            kinds,
            rationale: None,
        }
    }
}

/// Always abstains — every unmarked claim lands `enc:Assertion`. The deterministic floor: with
/// it, only frame-marked sentences are discourse-referable.
pub struct NoKindClassifier;
impl KindClassifier for NoKindClassifier {
    fn classify(&self, _ordinal: usize, _sentence: &str, _gloss: &str) -> KindVerdict {
        KindVerdict::default()
    }
}

impl<T: KindClassifier + ?Sized> KindClassifier for Box<T> {
    fn classify(&self, ordinal: usize, sentence: &str, gloss: &str) -> KindVerdict {
        (**self).classify(ordinal, sentence, gloss)
    }
}

impl<T: KindClassifier + ?Sized> KindClassifier for std::sync::Arc<T> {
    fn classify(&self, ordinal: usize, sentence: &str, gloss: &str) -> KindVerdict {
        (**self).classify(ordinal, sentence, gloss)
    }
}

// ───────────────────────── record / replay (the ranks/selections discipline) ─────────────────

/// One recorded verdict: the question (sentence + the chosen reading's gloss — the gloss pins
/// WHICH reading was classified) and the kinds assigned. An empty `kinds` is a recorded
/// abstention (the claim landed Assertion) — recorded, so a draw replays with 0 misses.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct KindRecord {
    pub sentence: String,
    pub gloss: String,
    pub kinds: Vec<String>,
    /// The classifier's stated reasoning, when it gave one. `default` so draws recorded before
    /// this field still replay (the replay key is sentence+gloss; the rationale is for review).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

fn kind_key(sentence: &str, gloss: &str) -> String {
    format!("{sentence}\u{1d}{gloss}")
}

/// Record every verdict an inner classifier produces. Flush with [`Self::write`].
pub struct RecordingKindClassifier<C: KindClassifier> {
    inner: C,
    log: Mutex<BTreeMap<String, KindRecord>>,
}

impl<C: KindClassifier> RecordingKindClassifier<C> {
    pub fn new(inner: C) -> Self {
        Self {
            inner,
            log: Mutex::new(BTreeMap::new()),
        }
    }

    /// Write the recorded verdicts as JSON (sorted by key — deterministic bytes).
    pub fn write(&self, path: &Path) -> std::io::Result<usize> {
        let log = self.log.lock().expect("kind log");
        let records: Vec<&KindRecord> = log.values().collect();
        let json = serde_json::to_string_pretty(&records)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)?;
        Ok(records.len())
    }
}

impl<C: KindClassifier> KindClassifier for RecordingKindClassifier<C> {
    fn classify(&self, ordinal: usize, sentence: &str, gloss: &str) -> KindVerdict {
        let key = kind_key(sentence, gloss);
        if let Some(r) = self.log.lock().expect("kind log").get(&key) {
            // Memoized — same question, same answer (one ask per sentence per draw).
            return KindVerdict {
                kinds: r.kinds.iter().filter_map(|k| Iri::parse(k).ok()).collect(),
                rationale: r.rationale.clone(),
            };
        }
        let verdict = self.inner.classify(ordinal, sentence, gloss);
        self.log.lock().expect("kind log").insert(
            key,
            KindRecord {
                sentence: sentence.to_string(),
                gloss: gloss.to_string(),
                kinds: verdict
                    .kinds
                    .iter()
                    .map(|k| k.as_str().to_string())
                    .collect(),
                rationale: verdict.rationale.clone(),
            },
        );
        verdict
    }
}

/// Replay a recorded draw — no LLM, deterministic. A miss answers EMPTY (the claim lands
/// Assertion, unreferable — fail-closed) and is COUNTED: [`Self::misses`] must be 0 for a
/// replay to reproduce the recorded run.
pub struct ReplayKindClassifier {
    by_key: BTreeMap<String, KindRecord>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl ReplayKindClassifier {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let records: Vec<KindRecord> = serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut by_key = BTreeMap::new();
        for r in records {
            by_key.insert(kind_key(&r.sentence, &r.gloss), r);
        }
        Ok(Self {
            by_key,
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        })
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    /// Questions not in the recording (answered empty). **Must be 0** for a faithful replay.
    pub fn misses(&self) -> usize {
        self.misses.load(Ordering::Relaxed)
    }
}

impl KindClassifier for ReplayKindClassifier {
    fn classify(&self, _ordinal: usize, sentence: &str, gloss: &str) -> KindVerdict {
        match self.by_key.get(&kind_key(sentence, gloss)) {
            Some(r) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                KindVerdict {
                    kinds: r.kinds.iter().filter_map(|k| Iri::parse(k).ok()).collect(),
                    rationale: r.rationale.clone(),
                }
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                KindVerdict::default()
            }
        }
    }
}

// ───────────────────────── live Anthropic classifier (use-llm) ─────────────────────────

#[cfg(feature = "use-llm")]
mod anthropic {
    use super::*;
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// The model's structured reply: at most one kind, or abstain.
    #[derive(Deserialize, JsonSchema)]
    struct KindReply {
        /// One of: finding, observation, classification, hypothesis, suggestion — or "none"
        /// when no kind confidently applies (the claim stays an unmarked assertion).
        kind: String,
        /// One sentence: why.
        rationale: String,
    }

    /// The live kind classifier (D68 §4 step 2): an untrusted judgment in document context —
    /// wrap in [`RecordingKindClassifier`] so a draw is recorded; on any error it abstains
    /// (the claim lands Assertion, fail-closed).
    pub struct AnthropicKindClassifier {
        api_key: String,
        model: String,
        /// The whole document — the classifier judges the sentence IN CONTEXT.
        document: String,
    }

    impl AnthropicKindClassifier {
        pub fn from_env(document: &str) -> Option<Self> {
            std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .map(|k| Self {
                    api_key: k,
                    model: eigenius_kernel::dcg::anthropic_client::DEFAULT_MODEL.to_string(),
                    document: document.to_string(),
                })
        }
    }

    impl KindClassifier for AnthropicKindClassifier {
        fn classify(&self, _ordinal: usize, sentence: &str, gloss: &str) -> KindVerdict {
            let prompt = format!(
                "A scientific document is being encoded claim by claim. Classify what the \
                 sentence below IS, in the document's own discourse terms — the kind a later \
                 sentence would refer back to it as («these findings…», «these observations…»).\n\n\
                 Document:\n{}\n\n\
                 The sentence to classify:\n  \"{}\"\n\
                 (its encoded reading: \"{}\")\n\n\
                 Return `kind`: one of finding | observation | classification | hypothesis | \
                 suggestion — or \"none\" if no kind confidently applies (it stays an unmarked \
                 assertion). A finding is an established result of the document's own \
                 investigation; an observation is something observed/measured; a classification \
                 assigns items to categories. Return `rationale`: one sentence why.",
                self.document.trim(),
                sentence.trim(),
                gloss,
            );
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return KindVerdict::default(),
            };
            let reply = match rt.block_on(
                eigenius_kernel::dcg::anthropic_client::anthropic_structured::<KindReply>(
                    &self.api_key,
                    &self.model,
                    &prompt,
                ),
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("anthropic kind-classifier error: {e}");
                    return KindVerdict::default();
                }
            };
            // An abstention is a RECORDED answer, and its reasoning is the part a reviewer
            // most needs — keep the rationale on both branches.
            let rationale = Some(reply.rationale);
            let iri = match reply.kind.trim().to_lowercase().as_str() {
                "finding" => KIND_FINDING,
                "observation" => KIND_OBSERVATION,
                "classification" => KIND_CLASSIFICATION,
                "hypothesis" => KIND_HYPOTHESIS,
                "suggestion" => KIND_SUGGESTION,
                _ => {
                    return KindVerdict {
                        kinds: Vec::new(),
                        rationale,
                    }
                }
            };
            KindVerdict {
                kinds: vec![Iri::parse(iri).expect("static kind IRI")],
                rationale,
            }
        }
    }
}

#[cfg(feature = "use-llm")]
pub use anthropic::AnthropicKindClassifier;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_table_marks_hypotheses_and_suggestions() {
        assert_eq!(
            frame_kind("We hypothesized that MSI creates vulnerabilities.")
                .unwrap()
                .as_str(),
            KIND_HYPOTHESIS
        );
        assert_eq!(
            frame_kind("These observations suggest that WRN dependency is real.")
                .unwrap()
                .as_str(),
            KIND_SUGGESTION
        );
        assert!(frame_kind("WRN was selectively essential in MSI models.").is_none());
    }

    #[test]
    fn a_kind_replay_reproduces_the_draw_and_counts_misses() {
        struct AlwaysFinding;
        impl KindClassifier for AlwaysFinding {
            fn classify(&self, _o: usize, _s: &str, _g: &str) -> KindVerdict {
                KindVerdict {
                    kinds: vec![Iri::parse(KIND_FINDING).unwrap()],
                    rationale: Some("it reports the study's own result".to_string()),
                }
            }
        }
        let rec = RecordingKindClassifier::new(AlwaysFinding);
        assert_eq!(rec.classify(0, "S.", "g").kinds.len(), 1);
        let dir = std::env::temp_dir().join("eigenius-kind-replay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kinds.json");
        assert_eq!(rec.write(&path).unwrap(), 1);

        let replay = ReplayKindClassifier::load(&path).unwrap();
        let hit = replay.classify(0, "S.", "g");
        assert_eq!(hit.kinds[0].as_str(), KIND_FINDING);
        assert_eq!(
            hit.rationale.as_deref(),
            Some("it reports the study's own result"),
            "the reasoning survives the round-trip — it is what a reviewer signs off on"
        );
        assert_eq!(replay.hits(), 1);
        // A different gloss is a different question (a different reading was classified).
        assert!(replay.classify(0, "S.", "other gloss").kinds.is_empty());
        assert_eq!(replay.misses(), 1, "counted, not hidden");
    }
}
