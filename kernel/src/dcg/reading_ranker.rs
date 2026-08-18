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

use crate::dcg::verbalize::ConceptNote;
/// D69-B truncation cap: how many STRUCTURES a single prompt may show.
///
/// Chosen so the worst case on the corpus page stays legible — «The use of immune checkpoint blockade
/// can be limited by toxicity.» reaches 171 readings — while leaving every structure that IS shown
/// complete, since a half-shown structure would make the sense table lie. Dropping is logged and
/// stated in the prompt; a silent cap would let the model pick "the best of what it saw" and report it
/// as the best reading (the D62 no-silent-caps rule).
const MAX_STRUCTURES_SHOWN: usize = 12;

/// One reading of the sentence, as presented to the ranker. Candidates are presented grouped by
/// skeleton for legibility (the caller orders them), but the choice — and its evaluation — are
/// per READING: structure and word senses together. The reading-level gold ledger
/// (`experiments/parsing/reading-adjudications.tsv`) is keyed on the `sem`.
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
    /// The concepts the candidates name, each once, with the chain's definition where it has one
    /// (D69 §4). Printed as a legend beside the candidates: the definitions are what separate a
    /// named concept from a compound that says the same words, and repeating them on every
    /// candidate line would be unreadable. Empty is fine — the caller may not have a chain.
    pub concepts: &'a [ConceptNote],
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

impl<T: ReadingRanker + ?Sized> ReadingRanker for Box<T> {
    fn select(
        &self,
        ctx: &DocumentContext,
        candidates: &[ReadingCandidate],
    ) -> Option<ReadingSelection> {
        (**self).select(ctx, candidates)
    }
}

impl<T: ReadingRanker + ?Sized> ReadingRanker for std::sync::Arc<T> {
    fn select(
        &self,
        ctx: &DocumentContext,
        candidates: &[ReadingCandidate],
    ) -> Option<ReadingSelection> {
        (**self).select(ctx, candidates)
    }
}

/// The pin-backed selector — the ground-truth/gate arm. Selects the candidate whose skeleton
/// equals the sentence's pinned skeleton; abstains when the sentence has no pin, the pin matches
/// no candidate, or **two or more candidates share the pinned skeleton** (sense-level ambiguity a
/// skeleton pin cannot adjudicate — fail-closed; the encoding CLI turns such an abstention into
/// a hard error with the pin diagnostics).
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
    /// True when the ranker ABSTAINED on this question. Recorded (not omitted): an unrecorded
    /// abstention would be indistinguishable from a changed question on replay, so a draw with
    /// abstentions could never replay with 0 misses. `chosen`/`rationale`/`runners_up` are
    /// meaningless when set.
    #[serde(default)]
    pub abstained: bool,
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

/// **Record** every decision an inner ranker produces — selections AND abstentions (an
/// abstention is an answer to the question; leaving it out would make a draw with abstentions
/// unable to replay with 0 misses). Flush with [`Self::write`]. Same rationale as
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
        let selection = self.inner.select(ctx, candidates);
        let recorded = recorded_candidates(candidates);
        let sha = document_sha(ctx.document);
        let key = selection_key(ctx.sentence, &sha, ctx.prior_selections, &recorded);
        let record = match &selection {
            Some(s) => SelectionRecord {
                sentence: ctx.sentence.to_string(),
                document_sha256: sha,
                prior_selections: ctx.prior_selections.to_vec(),
                candidates: recorded,
                abstained: false,
                chosen: s.chosen,
                rationale: s.rationale.clone(),
                runners_up: s.runners_up.clone(),
            },
            None => SelectionRecord {
                sentence: ctx.sentence.to_string(),
                document_sha256: sha,
                prior_selections: ctx.prior_selections.to_vec(),
                candidates: recorded,
                abstained: true,
                chosen: 0,
                rationale: String::new(),
                runners_up: Vec::new(),
            },
        };
        self.log.lock().expect("selection log").insert(key, record);
        selection
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
                if r.abstained {
                    return None; // a RECORDED abstention — a hit, replayed as the abstention it was
                }
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

// ───────────────────────── live Anthropic ranker (use-llm feature) ─────────────────────────

/// Split a gloss into its `«label» [id]` atoms, in order, with the literal text between them.
///
/// Returns `(frame, atoms)`: `frame` is the gloss with each atom replaced by `{}` — the part that is
/// INVARIANT across readings of one structure — and `atoms` are the `(label, id)` pairs in order.
fn split_atoms(gloss: &str) -> (String, Vec<(String, String)>) {
    let mut frame = String::new();
    let mut atoms = Vec::new();
    let mut rest = gloss;
    while let Some(open) = rest.find('«') {
        let after = &rest[open + '«'.len_utf8()..];
        let Some(close) = after.find('»') else { break };
        let label = &after[..close];
        let tail = &after[close + '»'.len_utf8()..];
        // The id must follow immediately as ` [id]`, else this is not an atom.
        let Some(stripped) = tail.strip_prefix(" [") else {
            frame.push_str(&rest[..open + '«'.len_utf8()]);
            rest = after;
            continue;
        };
        let Some(idend) = stripped.find(']') else {
            break;
        };
        frame.push_str(&rest[..open]);
        frame.push_str("{}");
        atoms.push((label.to_string(), stripped[..idend].to_string()));
        rest = &stripped[idend + 1..];
    }
    frame.push_str(rest);
    (frame, atoms)
}

/// D69-B: render one structure ONCE with its varying slots marked, then the slot options, then the
/// readings as slot assignments rather than as repeated full glosses.
///
/// The pool factorizes — structures × sense assignments — and repeating every invariant part once
/// per reading is what made the prompt 13 KB of near-duplicates (§1). Here the invariant frame is
/// stated once and only the positions that actually differ are enumerated. Falls back to the flat
/// listing when the group's glosses do not align (different atom counts ⇒ no positional slots).
fn render_structure_group(n: usize, group: &[(usize, &ReadingCandidate)]) -> String {
    let mut out = format!("Structure {n}:\n");
    let parsed: Vec<(String, Vec<(String, String)>)> =
        group.iter().map(|(_, c)| split_atoms(&c.gloss)).collect();
    let aligned = parsed
        .windows(2)
        .all(|w| w[0].0 == w[1].0 && w[0].1.len() == w[1].1.len());
    if !aligned || group.len() < 2 {
        for (i, c) in group {
            out.push_str(&format!("  [{i}] {}\n", c.gloss));
        }
        return out;
    }
    let natoms = parsed[0].1.len();
    let varying: Vec<usize> = (0..natoms)
        .filter(|&k| parsed.iter().any(|p| p.1[k] != parsed[0].1[k]))
        .collect();
    // The frame, with fixed atoms filled in and varying ones left as named slots.
    let mut shown = String::new();
    let mut slot_no = 0usize;
    let mut names: Vec<String> = Vec::new();
    for (k, piece) in parsed[0].0.split("{}").enumerate() {
        shown.push_str(piece);
        if k < natoms {
            let (label, id) = &parsed[0].1[k];
            if varying.contains(&k) {
                slot_no += 1;
                let nm = format!("{}", (b'A' + (slot_no as u8 - 1)) as char);
                shown.push_str(&format!("{{{nm}}}"));
                names.push(nm);
            } else {
                shown.push_str(&format!("«{label}» [{id}]"));
            }
        }
    }
    out.push_str(&format!("  {shown}\n"));
    // The options per slot — this is the only place a sense is named more than once.
    for (si, &k) in varying.iter().enumerate() {
        let mut seen: Vec<&(String, String)> = Vec::new();
        for p in &parsed {
            if !seen.iter().any(|o| **o == p.1[k]) {
                seen.push(&p.1[k]);
            }
        }
        let opts: Vec<String> = seen.iter().map(|(l, i)| format!("«{l}» [{i}]")).collect();
        out.push_str(&format!("    {} = {}\n", names[si], opts.join(" | ")));
    }
    // Each reading as its slot assignment, so the index the model must return is unambiguous.
    for (p, (i, _)) in parsed.iter().zip(group.iter()) {
        let asg: Vec<String> = varying
            .iter()
            .enumerate()
            .map(|(si, &k)| format!("{}=[{}]", names[si], p.1[k].1))
            .collect();
        out.push_str(&format!("      [{i}] {}\n", asg.join(" ")));
    }
    out
}

#[cfg(feature = "use-llm")]
mod anthropic {
    use super::{DocumentContext, ReadingCandidate, ReadingRanker, ReadingSelection};
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// What the ranker concluded. A STRING ENUM, not a flag beside a mandatory `chosen`: the old
    /// shape required an index even when abstaining, so "none of these is right" could only be
    /// said while also naming a winner. Measured consequence (D69 §7d): on a pool where every
    /// reading was unfaithful, three different models all chose and justified, one of them
    /// reporting `abstain: false` alongside a full 19-deep ranking.
    #[derive(Deserialize, JsonSchema, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    enum Verdict {
        /// One candidate faithfully expresses the sentence; `chosen` names it.
        Chose,
        /// NO candidate does. A correct answer, not a failure — `missing_sense` says what the
        /// candidate set lacks.
        NoneFaithful,
    }

    /// The model's structured reply. `none_faithful` ⇒ no selection (the sentence stays
    /// Ambiguous); otherwise `chosen` indexes the candidate list.
    #[derive(Deserialize, JsonSchema)]
    struct ReadingSelectionReply {
        /// `chose` when one candidate faithfully expresses the sentence; `none_faithful` when
        /// none does.
        verdict: Verdict,
        /// The index of the faithful reading. Required when `verdict` is `chose`; omit otherwise.
        chosen: Option<usize>,
        /// When `verdict` is `none_faithful`: which sense or structure the candidates lack, e.g.
        /// "no disease sense of «cancer» — only the crab genus and the astrological sign".
        /// Recorded as the diagnostic that points at the gap.
        missing_sense: Option<String>,
        /// One sentence: why this reading, or why none is faithful.
        rationale: String,
        /// The remaining reading indices in preference order, most plausible first. May be empty
        /// — a full ranking of readings you did not discriminate is noise, not information.
        #[serde(default)]
        runners_up: Vec<usize>,
    }

    /// A [`ReadingRanker`] backed by Anthropic Claude via the direct tool-use client
    /// ([`crate::dcg::anthropic_client`]). On any error it ABSTAINS (`None`) — unlike the sense
    /// ranker there is no harmless fallback order; a fabricated selection would be a wrong
    /// answer, not a slower parse. A malformed reply (index out of range) likewise abstains.
    pub struct AnthropicReadingRanker {
        api_key: String,
        model: String,
    }

    impl AnthropicReadingRanker {
        pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
            Self {
                api_key: api_key.into(),
                model: model.into(),
            }
        }

        /// From `$ANTHROPIC_API_KEY`, defaulting to the shared client model. `None` if unset.
        pub fn from_env() -> Option<Self> {
            std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .map(|k| Self::new(k, crate::dcg::anthropic_client::DEFAULT_MODEL))
        }

        fn ask(&self, instructions: &str) -> Option<ReadingSelectionReply> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            match rt.block_on(crate::dcg::anthropic_client::anthropic_structured::<
                ReadingSelectionReply,
            >(&self.api_key, &self.model, instructions))
            {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("anthropic reading-ranker error: {e}");
                    None
                }
            }
        }
    }

    /// The concept legend: each concept the candidates name, once, with its definition. Empty string
    /// when there is nothing to say, so the prompt gains no dangling header.
    fn concept_legend(concepts: &[super::ConceptNote]) -> String {
        let with_defs: Vec<&super::ConceptNote> =
            concepts.iter().filter(|c| c.definition.is_some()).collect();
        if with_defs.is_empty() {
            return String::new();
        }
        let mut out = String::from("\nWhat the concepts mean (from the knowledge graph):\n");
        for c in with_defs {
            let d = c.definition.as_deref().unwrap_or_default();
            let d: String = if d.chars().count() > 240 {
                format!("{}…", d.chars().take(240).collect::<String>())
            } else {
                d.to_string()
            };
            out.push_str(&format!("  [{}] «{}» — {}\n", c.id, c.label, d));
        }
        out
    }

    impl ReadingRanker for AnthropicReadingRanker {
        fn select(
            &self,
            ctx: &DocumentContext,
            candidates: &[ReadingCandidate],
        ) -> Option<ReadingSelection> {
            if candidates.len() < 2 {
                return None; // nothing to disambiguate
            }
            // Prior selections — the discourse the ranker must stay consistent with.
            let mut prior_block = String::new();
            if !ctx.prior_selections.is_empty() {
                prior_block.push_str(
                    "Readings already selected for earlier sentences (stay consistent with them):\n",
                );
                for p in ctx.prior_selections {
                    prior_block.push_str(&format!("  sentence {}: \"{}\"\n", p.ordinal, p.gloss));
                }
                prior_block.push('\n');
            }
            // Candidates, grouped by skeleton (the caller sorts them): a structure header per
            // distinct skeleton, then each reading's index + gloss. The gloss names concrete
            // senses, so readings within one structure differ by word sense.
            // D69-B — TWO-LEVEL presentation. Group by skeleton (the caller sorts), render each
            // structure's invariant frame ONCE, and enumerate only the slots whose sense actually
            // varies. §1's prompt spent 13 KB repeating near-identical glosses 120 times; the pool
            // factorizes, so the repetition was pure noise around the two real axes.
            let mut groups: Vec<Vec<(usize, &ReadingCandidate)>> = Vec::new();
            let mut last_skel: Option<&str> = None;
            for (i, c) in candidates.iter().enumerate() {
                if last_skel != Some(c.skeleton.as_str()) {
                    groups.push(Vec::new());
                    last_skel = Some(c.skeleton.as_str());
                }
                groups
                    .last_mut()
                    .expect("a group was just pushed")
                    .push((i, c));
            }
            // TRUNCATION, if any, is EXPLICIT and LOGGED — never silent (the D62 rule). Structures
            // are dropped whole, lowest-ranked last, so a kept structure is always shown complete.
            let total_structures = groups.len();
            let kept = groups.len().min(super::MAX_STRUCTURES_SHOWN);
            let dropped_readings: usize = groups[kept..].iter().map(Vec::len).sum();
            groups.truncate(kept);
            let mut cand_block = String::new();
            // TWO-LEVEL (D69-B) is OPT-IN and OFF by default — the A/B rejected it. Measured
            // 2026-08-17 on the same forest and the same ranks, fully adjudicated both ways:
            // flat 30/40 correct with structure 33/40, two-level 24/40 with structure 29/40. §4a of
            // the note says the surface is settled by selection accuracy and not by taste, so the
            // flat listing stays. Kept behind a flag because the A/B should be repeatable and the
            // idea may be right with a different realisation (a two-CALL ranker rather than a
            // two-level prompt — the note's own preferred shape, which this was the cheap proxy for).
            let two_level = std::env::var("EIGENIUS_SELECT_TWO_LEVEL").is_ok();
            for (n, g) in groups.iter().enumerate() {
                if two_level {
                    cand_block.push_str(&super::render_structure_group(n + 1, g));
                } else {
                    cand_block.push_str(&format!("Structure {}:\n", n + 1));
                    for (i, c) in g {
                        cand_block.push_str(&format!("  [{i}] {}\n", c.gloss));
                    }
                }
            }
            if dropped_readings > 0 {
                let msg = format!(
                    "reading-ranker: TRUNCATED «{}» — showed {kept} of {total_structures} structures, \
                     omitting {dropped_readings} reading(s); the omitted ones cannot be chosen",
                    ctx.sentence.trim()
                );
                eprintln!("{msg}");
                cand_block.push_str(&format!(
                    "\n(NOTE: {dropped_readings} further reading(s) in {} more structure(s) are not \
                     shown and cannot be chosen. If none of the above is faithful, say so rather than \
                     picking the closest.)\n",
                    total_structures - kept
                ));
            }
            let legend = concept_legend(ctx.concepts);
            let prompt = format!(
                "A parser read the document below and produced several candidate READINGS \
                 (interpretations) of one sentence. Choose the reading that expresses what the \
                 sentence actually means in the context of the document.\n\n\
                 Document:\n{}\n\n{prior_block}\
                 The sentence to disambiguate:\n  \"{}\"\n\n\
                 Candidate readings, grouped by grammatical STRUCTURE. Each structure is shown \
                 once, as what it COMMITS TO: `«label» [id]` names a concept, `+ relation X` is an \
                 explicit relation the reading asserts, and `⟦…⟧` marks a fragment that could not \
                 be rendered. Readings within one structure differ only in word \
                 sense.\n\n{cand_block}\n{legend}\
                 Return `chosen` = the index of the reading whose structure AND word senses match \
                 the sentence's intended meaning, `rationale` = one sentence why, and `runners_up` \
                 = the remaining indices in preference order. Set `abstain` = true only if no \
                 reading can be identified as the intended one — prefer choosing when one reading \
                 is clearly best.",
                ctx.document.trim(),
                ctx.sentence.trim(),
            );
            // `EIGENIUS_DUMP_SELECT_PROMPT=1` prints the exact prompt per sentence — the ranker
            // decides which reading lands on the chain, so being able to READ what it was asked
            // is the difference between debugging it and guessing at it.
            if std::env::var("EIGENIUS_DUMP_SELECT_PROMPT").is_ok() {
                eprintln!(
                    "\n===== READING-RANKER PROMPT =====\n{prompt}\n===== END PROMPT =====\n"
                );
            }
            let reply = self.ask(&prompt)?;
            // "No candidate is faithful" is a RESULT, and its diagnostic is the valuable half:
            // it names the sense the pool lacks, which is the upstream bug (a sense that exists
            // in the lexicon but never entered this sentence's candidate set — D69 §7d).
            if reply.verdict == Verdict::NoneFaithful {
                eprintln!(
                    "reading-ranker: NONE FAITHFUL on «{}» — {}{}",
                    ctx.sentence.trim(),
                    reply.rationale,
                    reply
                        .missing_sense
                        .as_deref()
                        .map(|m| format!("  [missing: {m}]"))
                        .unwrap_or_default()
                );
                return None;
            }
            let chosen = reply.chosen?; // `chose` without an index is malformed ⇒ fail closed
            if chosen >= candidates.len() {
                return None; // out-of-range index from untrusted input
            }
            let n = candidates.len();
            let mut seen = vec![false; n];
            seen[chosen] = true;
            let runners_up: Vec<usize> = reply
                .runners_up
                .into_iter()
                .filter(|&i| i < n && !std::mem::replace(&mut seen[i], true))
                .collect();
            Some(ReadingSelection {
                chosen,
                rationale: reply.rationale,
                runners_up,
            })
        }
    }
}

#[cfg(feature = "use-llm")]
pub use anthropic::AnthropicReadingRanker;

/// The first pair of candidates that render identically (D69 §3) — `None` when the pool is
/// injective, which is the required state before any pool is put to a ranker.
///
/// Checked by the CALLER, before any ranker sees the pool: the invariant is about what may be
/// ASKED, so it must hold for the pin-backed and replay arms too, not only the live one.
pub fn first_collision(candidates: &[ReadingCandidate]) -> Option<(usize, usize)> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, c) in candidates.iter().enumerate() {
        if let Some(&j) = seen.get(c.gloss.as_str()) {
            return Some((j, i));
        }
        seen.insert(c.gloss.as_str(), i);
    }
    None
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
            concepts: &[],
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

    /// Live reading disambiguation: the document context must decide between two structurally
    /// distinct readings. Skips without a key; runs with `--features use-llm` + `ANTHROPIC_API_KEY`.
    #[cfg(feature = "use-llm")]
    #[test]
    fn live_anthropic_reading_ranker_picks_the_contextual_reading() {
        let Some(ranker) = AnthropicReadingRanker::from_env() else {
            eprintln!("SKIP live_anthropic_reading_ranker: ANTHROPIC_API_KEY unset");
            return;
        };
        let candidates = vec![
            ReadingCandidate {
                skeleton: "see_with(§)(we, telescope, man)".to_string(),
                gloss: "we saw the man by using a telescope".to_string(),
                sem: String::new(),
            },
            ReadingCandidate {
                skeleton: "see(§)(we, man_with(telescope))".to_string(),
                gloss: "we saw the man who was holding a telescope".to_string(),
                sem: String::new(),
            },
        ];
        let ctx = DocumentContext {
            document: "We set up our new telescope on the balcony at dusk. \
                       We saw the man with the telescope. \
                       The optics were remarkably sharp for the price.",
            sentence: "We saw the man with the telescope.",
            prior_selections: &[],
            concepts: &[],
        };
        let sel = ranker
            .select(&ctx, &candidates)
            .expect("a clearly-contextual reading should be chosen, not abstained");
        assert_eq!(
            sel.chosen, 0,
            "the document (we set up a telescope, its optics were sharp) selects the \
             instrumental reading; rationale: {}",
            sel.rationale
        );
        assert!(!sel.rationale.is_empty());
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
    fn a_recorded_abstention_replays_as_an_abstention_hit() {
        struct Abstain;
        impl ReadingRanker for Abstain {
            fn select(
                &self,
                _ctx: &DocumentContext,
                _c: &[ReadingCandidate],
            ) -> Option<ReadingSelection> {
                None
            }
        }
        let c = cands(2);
        let rec = RecordingReadingRanker::new(Abstain);
        assert!(rec.select(&ctx("Doc.", "Doc.", &[]), &c).is_none());
        let dir = std::env::temp_dir().join("eigenius-selection-replay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("selections-abstain.json");
        assert_eq!(rec.write(&path).unwrap(), 1, "the abstention IS recorded");
        let replay = ReplayReadingRanker::load(&path).unwrap();
        assert!(replay.select(&ctx("Doc.", "Doc.", &[]), &c).is_none());
        assert_eq!(
            replay.hits(),
            1,
            "a recorded abstention is a HIT, not a miss"
        );
        assert_eq!(
            replay.misses(),
            0,
            "a draw with abstentions still replays with 0 misses"
        );
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

#[cfg(test)]
mod d69b_tests {
    use super::{render_structure_group, split_atoms, ReadingCandidate};

    fn cand(gloss: &str) -> ReadingCandidate {
        ReadingCandidate {
            skeleton: "§(§, §)".into(),
            gloss: gloss.into(),
            sem: String::new(),
        }
    }

    #[test]
    fn split_atoms_separates_the_invariant_frame_from_the_senses() {
        let (frame, atoms) = split_atoms("a «target» [n05981230] + of «WRN» [C0388246] here");
        assert_eq!(frame, "a {} + of {} here");
        assert_eq!(
            atoms,
            vec![
                ("target".to_string(), "n05981230".to_string()),
                ("WRN".to_string(), "C0388246".to_string())
            ]
        );
    }

    /// The point of D69-B: the invariant part is stated ONCE and only the differing position is
    /// enumerated, instead of repeating the whole gloss per reading (§1's 13 KB of near-duplicates).
    #[test]
    fn only_the_varying_slot_is_enumerated() {
        let group = [
            cand("a «target» [n05981230] + of «WRN protein» [C0388246]"),
            cand("a «target» [n05981230] + of «WRN gene» [C1337007]"),
        ];
        let g: Vec<(usize, &ReadingCandidate)> = group.iter().enumerate().collect();
        let out = render_structure_group(1, &g);
        assert!(
            out.contains("a «target» [n05981230] + of {A}"),
            "frame once: {out}"
        );
        assert!(
            out.contains("A = «WRN protein» [C0388246] | «WRN gene» [C1337007]"),
            "{out}"
        );
        assert!(
            out.contains("[0] A=[C0388246]") && out.contains("[1] A=[C1337007]"),
            "{out}"
        );
        assert_eq!(
            out.matches("«target»").count(),
            1,
            "the shared slot is not repeated: {out}"
        );
    }

    /// Misaligned glosses (different atom counts) have no positional slots, so the flat listing is
    /// kept rather than inventing an alignment.
    #[test]
    fn unalignable_groups_fall_back_to_the_flat_listing() {
        let group = [
            cand("a «target» [n05981230]"),
            cand("a «target» [n05981230] + of «WRN» [C0388246]"),
        ];
        let g: Vec<(usize, &ReadingCandidate)> = group.iter().enumerate().collect();
        let out = render_structure_group(1, &g);
        assert!(out.contains("[0] a «target» [n05981230]\n"), "{out}");
        assert!(!out.contains("{A}"), "no slots when unaligned: {out}");
    }
}
