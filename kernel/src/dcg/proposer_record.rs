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

//! **Record / replay for the anaphora [`Proposer`]** (plan §2.4) — the third recorded LLM stage,
//! beside `sense_ranker` (ranks.json) and `reading_ranker` (selections.json). Same rationale:
//! the LLM is the one component that can answer differently for the same code against the same
//! store; recording turns it from an uncontrolled input into a recorded one, and a replay with
//! 0 misses IS the recorded experiment.
//!
//! The key covers everything the proposer was asked: the sentence, the surrounding document
//! (as SHA-256 — part of the key, not repeated verbatim per record), the prior-selection
//! glosses, the HOLE (var + pretty type + kind), and the PRESENTED candidates (stable key +
//! surface — the §2.4 type pre-filter runs before presentation, so the recorded question is the
//! filtered one). A changed document, a changed upstream selection, a differently-typed hole, or
//! a different candidate set is a different question and must MISS rather than silently replay a
//! stale answer. A replay miss returns the EMPTY proposal (the hole stays unresolved — the parse
//! stays `Open`, fail-closed) and is COUNTED; a recorded empty proposal ("none of these") is a
//! HIT that replays as the refusal it was.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::parse::{Proposal, ProposeCtx, Proposer};
use super::pretty::pretty_term;
use super::reading_ranker::PriorSelection;

/// One candidate as presented — its stable identity ([`super::parse::Candidate::key`]) and its
/// surface form. Both are part of the question: a same-key candidate with a changed surface was
/// presented DIFFERENTLY to the model.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RecordedProposalCandidate {
    pub key: String,
    pub surface: String,
}

/// A recorded proposal: the exact question put to the proposer, and the answer it gave. An
/// EMPTY `proposal.ranked` is a recorded refusal ("no presented candidate is the referent") —
/// stored like any answer, so a draw containing refusals still replays with 0 misses.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProposalRecord {
    pub sentence: String,
    pub document_sha256: String,
    #[serde(default)]
    pub prior_selections: Vec<PriorSelection>,
    pub hole_var: String,
    /// `pretty_term` of the hole's restrictor type — the veto the presented list was filtered by.
    pub hole_ty: String,
    /// `Debug` of the hole's [`super::parse::HoleKind`].
    pub hole_kind: String,
    pub candidates: Vec<RecordedProposalCandidate>,
    pub proposal: Proposal,
}

fn document_sha(document: &str) -> String {
    format!("{:x}", Sha256::digest(document.as_bytes()))
}

/// The lookup key — everything that was part of the question (module doc).
fn proposal_key(
    sentence: &str,
    document_sha256: &str,
    prior: &[PriorSelection],
    hole_var: &str,
    hole_ty: &str,
    hole_kind: &str,
    candidates: &[RecordedProposalCandidate],
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
    k.push_str(hole_var);
    k.push('\u{1e}');
    k.push_str(hole_ty);
    k.push('\u{1e}');
    k.push_str(hole_kind);
    k.push('\u{1c}');
    for c in candidates {
        k.push('\u{1f}');
        k.push_str(&c.key);
        k.push('\u{1e}');
        k.push_str(&c.surface);
    }
    k
}

fn recorded_candidates(ctx: &ProposeCtx) -> Vec<RecordedProposalCandidate> {
    ctx.candidates
        .iter()
        .map(|c| RecordedProposalCandidate {
            key: c.key(),
            surface: c.surface().to_string(),
        })
        .collect()
}

fn ctx_key(ctx: &ProposeCtx, candidates: &[RecordedProposalCandidate]) -> String {
    proposal_key(
        ctx.doc.sentence,
        &document_sha(ctx.doc.document),
        ctx.doc.prior_selections,
        &ctx.hole.var,
        &pretty_term(&ctx.hole.ty),
        &format!("{:?}", ctx.hole.kind),
        candidates,
    )
}

/// **Record** every proposal an inner proposer produces — rankings AND refusals. Flush with
/// [`Self::write`]. **Memoizing**: a repeated key returns the recorded answer WITHOUT re-asking
/// the inner proposer — the key defines the question, and one question has one answer per draw.
/// This matters structurally: several open parses of one sentence routinely carry the SAME hole
/// (same var, type, and presented candidates), and a live inner proposer would otherwise be
/// asked — and billed, and free to answer differently — once per parse (the worst unit has 48).
pub struct RecordingProposer<P: Proposer> {
    inner: P,
    log: Mutex<BTreeMap<String, ProposalRecord>>,
}

impl<P: Proposer> RecordingProposer<P> {
    pub fn new(inner: P) -> Self {
        Self {
            inner,
            log: Mutex::new(BTreeMap::new()),
        }
    }

    /// The recorded proposals as JSON (sorted by key — deterministic bytes).
    pub fn to_json(&self) -> std::io::Result<String> {
        let log = self.log.lock().expect("proposal log");
        let records: Vec<&ProposalRecord> = log.values().collect();
        serde_json::to_string_pretty(&records)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Write the recorded proposals as JSON (sorted by key — deterministic bytes).
    pub fn write(&self, path: &Path) -> std::io::Result<usize> {
        let json = self.to_json()?;
        let n = self.log.lock().expect("proposal log").len();
        std::fs::write(path, json)?;
        Ok(n)
    }

    /// The recorded proposals as chain-ready draws (D71 §9) — the same set `write` serialises,
    /// each paired with the replay key it answers.
    pub fn keyed_draws(&self) -> std::io::Result<Vec<crate::dcg::draw::KeyedDraw>> {
        let log = self.log.lock().expect("proposal log");
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

impl<P: Proposer> Proposer for RecordingProposer<P> {
    fn propose(&self, ctx: &ProposeCtx) -> Proposal {
        let candidates = recorded_candidates(ctx);
        let key = ctx_key(ctx, &candidates);
        if let Some(r) = self.log.lock().expect("proposal log").get(&key) {
            return r.proposal.clone(); // memoized — same question, same answer (module doc)
        }
        let proposal = self.inner.propose(ctx);
        let record = ProposalRecord {
            sentence: ctx.doc.sentence.to_string(),
            document_sha256: document_sha(ctx.doc.document),
            prior_selections: ctx.doc.prior_selections.to_vec(),
            hole_var: ctx.hole.var.clone(),
            hole_ty: pretty_term(&ctx.hole.ty),
            hole_kind: format!("{:?}", ctx.hole.kind),
            candidates,
            proposal: proposal.clone(),
        };
        self.log.lock().expect("proposal log").insert(key, record);
        proposal
    }
}

/// **Replay** proposals recorded by [`RecordingProposer`] — no LLM, no network, deterministic.
/// A miss returns the EMPTY proposal (the hole stays unresolved — fail-closed; a recording
/// cannot answer a question it was not asked) and is COUNTED: [`Self::misses`] must be 0 for a
/// replay to be a faithful reproduction of the recorded run.
pub struct ReplayProposer {
    by_key: BTreeMap<String, ProposalRecord>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

/// The same key, computed from a RECORDED exchange rather than a live one. One function, shared by
/// the replay loader and the D71 draw emitter.
pub(crate) fn record_key(r: &ProposalRecord) -> String {
    proposal_key(
        &r.sentence,
        &r.document_sha256,
        &r.prior_selections,
        &r.hole_var,
        &r.hole_ty,
        &r.hole_kind,
        &r.candidates,
    )
}

impl ReplayProposer {
    /// Load a recording written by [`RecordingProposer::write`].
    pub fn load(path: &Path) -> std::io::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    /// Load a recording from its JSON, wherever it came from — a draw file, or the run's
    /// `doc-<id>` branch via [`crate::dcg::draw::draws_from_layer`] (D71 §9).
    pub fn from_json(text: &str) -> std::io::Result<Self> {
        let records: Vec<ProposalRecord> = serde_json::from_str(text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut by_key = BTreeMap::new();
        for r in records {
            let k = record_key(&r);
            by_key.insert(k, r);
        }
        Ok(Self {
            by_key,
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        })
    }

    /// Proposals replayed from the recording.
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    /// Questions NOT found in the recording (answered empty, fail-closed). **Must be 0** for the
    /// replay to reproduce the recorded run.
    pub fn misses(&self) -> usize {
        self.misses.load(Ordering::Relaxed)
    }
}

impl Proposer for ReplayProposer {
    fn propose(&self, ctx: &ProposeCtx) -> Proposal {
        let candidates = recorded_candidates(ctx);
        let key = ctx_key(ctx, &candidates);
        match self.by_key.get(&key) {
            Some(r) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                r.proposal.clone()
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                Proposal::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse::{Candidate, HoleInfo, HoleKind};
    use super::super::reading_ranker::DocumentContext;
    use super::*;
    use crate::nbe::term::Exp;
    use crate::ontology::iri::Iri;

    fn hole() -> HoleInfo {
        HoleInfo {
            var: "$demref$0_0".to_string(),
            ty: Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:CellLine").unwrap()),
            kind: HoleKind::EntityRef,
        }
    }

    fn cands(n: usize) -> Vec<Candidate> {
        (0..n)
            .map(|i| Candidate::Individual {
                iri: Iri::parse(&format!("urn:eigenius:lexicon:c{i}")).unwrap(),
                surface: format!("candidate {i}"),
            })
            .collect()
    }

    fn doc<'a>(document: &'a str, sentence: &'a str) -> DocumentContext<'a> {
        DocumentContext {
            document,
            sentence,
            prior_selections: &[],
            concepts: &[],
        }
    }

    /// Chooses the LAST candidate (non-trivial, so a replay falling back to "first" is caught).
    struct Last;
    impl Proposer for Last {
        fn propose(&self, ctx: &ProposeCtx) -> Proposal {
            let n = ctx.candidates.len();
            Proposal {
                ranked: (0..n).rev().collect(),
                rationale: Some("last first".to_string()),
                confidence: Some(0.5),
            }
        }
    }

    #[test]
    fn a_replay_reproduces_the_recorded_proposal_exactly() {
        let c = cands(3);
        let h = hole();
        let rec = RecordingProposer::new(Last);
        let d = doc("Doc text.", "Doc text.");
        let live = rec.propose(&ProposeCtx {
            doc: &d,
            hole: &h,
            candidates: &c,
        });
        assert_eq!(live.ranked, vec![2, 1, 0]);

        let dir = std::env::temp_dir().join("eigenius-proposal-replay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("proposals.json");
        assert_eq!(rec.write(&path).unwrap(), 1);

        let replay = ReplayProposer::load(&path).unwrap();
        let got = replay.propose(&ProposeCtx {
            doc: &d,
            hole: &h,
            candidates: &c,
        });
        assert_eq!(got.ranked, live.ranked);
        assert_eq!(got.rationale, live.rationale);
        assert_eq!(got.confidence, live.confidence);
        assert_eq!(replay.hits(), 1);
        assert_eq!(replay.misses(), 0, "a faithful replay misses nothing");
    }

    #[test]
    fn the_recorder_memoizes_a_repeated_question() {
        // Several open parses of one sentence routinely carry the SAME hole over the same
        // presented candidates — the inner proposer (a live LLM) must be asked ONCE.
        struct Counting(std::sync::Arc<AtomicUsize>);
        impl Proposer for Counting {
            fn propose(&self, _ctx: &ProposeCtx) -> Proposal {
                self.0.fetch_add(1, Ordering::Relaxed);
                Proposal::ranked(vec![0])
            }
        }
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let rec = RecordingProposer::new(Counting(std::sync::Arc::clone(&calls)));
        let c = cands(2);
        let h = hole();
        let d = doc("Doc.", "Doc.");
        let first = rec.propose(&ProposeCtx {
            doc: &d,
            hole: &h,
            candidates: &c,
        });
        let second = rec.propose(&ProposeCtx {
            doc: &d,
            hole: &h,
            candidates: &c,
        });
        assert_eq!(first.ranked, second.ranked);
        assert_eq!(calls.load(Ordering::Relaxed), 1, "one question, one ask");
    }

    #[test]
    fn a_recorded_refusal_replays_as_a_refusal_hit() {
        struct Refuse;
        impl Proposer for Refuse {
            fn propose(&self, _ctx: &ProposeCtx) -> Proposal {
                Proposal::default()
            }
        }
        let c = cands(2);
        let h = hole();
        let rec = RecordingProposer::new(Refuse);
        let d = doc("Doc.", "Doc.");
        assert!(rec
            .propose(&ProposeCtx {
                doc: &d,
                hole: &h,
                candidates: &c,
            })
            .ranked
            .is_empty());
        let dir = std::env::temp_dir().join("eigenius-proposal-replay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("proposals-refuse.json");
        assert_eq!(rec.write(&path).unwrap(), 1, "the refusal IS recorded");
        let replay = ReplayProposer::load(&path).unwrap();
        assert!(replay
            .propose(&ProposeCtx {
                doc: &d,
                hole: &h,
                candidates: &c,
            })
            .ranked
            .is_empty());
        assert_eq!(replay.hits(), 1, "a recorded refusal is a HIT, not a miss");
        assert_eq!(replay.misses(), 0);
    }

    #[test]
    fn a_replay_miss_answers_empty_and_is_counted() {
        let c = cands(2);
        let h = hole();
        let rec = RecordingProposer::new(Last);
        let d = doc("Doc A.", "Doc A.");
        rec.propose(&ProposeCtx {
            doc: &d,
            hole: &h,
            candidates: &c,
        });
        let dir = std::env::temp_dir().join("eigenius-proposal-replay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("proposals-miss.json");
        rec.write(&path).unwrap();
        let replay = ReplayProposer::load(&path).unwrap();

        // A different DOCUMENT is a different question, even for an identical sentence.
        let d2 = doc("Doc B.", "Doc A.");
        assert!(replay
            .propose(&ProposeCtx {
                doc: &d2,
                hole: &h,
                candidates: &c,
            })
            .ranked
            .is_empty());
        assert_eq!(replay.misses(), 1, "counted, not hidden");

        // A differently-TYPED hole is a different question (the restrictor changed).
        let mut h2 = hole();
        h2.ty = Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:Gene").unwrap());
        assert!(replay
            .propose(&ProposeCtx {
                doc: &d,
                hole: &h2,
                candidates: &c,
            })
            .ranked
            .is_empty());

        // A different CANDIDATE SET is a different question (the discourse or filter changed).
        let c3 = cands(3);
        assert!(replay
            .propose(&ProposeCtx {
                doc: &d,
                hole: &h,
                candidates: &c3,
            })
            .ranked
            .is_empty());
        assert_eq!(replay.misses(), 3);

        // The recorded question still replays.
        assert!(!replay
            .propose(&ProposeCtx {
                doc: &d,
                hole: &h,
                candidates: &c,
            })
            .ranked
            .is_empty());
        assert_eq!(replay.hits(), 1);
    }
}
