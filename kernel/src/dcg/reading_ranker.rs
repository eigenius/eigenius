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
#[cfg(feature = "use-llm")]
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
    /// The structure alone, in the sentence's own words
    /// ([`crate::dcg::verbalize::Register::Structural`]): the same for every reading of one
    /// structure. What the two-call ranker's structure call shows (eigenius#264). A presentation
    /// of `sem`, like the prompt's wording, so it is not part of the selection key.
    pub structure: String,
    /// How the reading's phrases attach ([`crate::dcg::verbalize::structure_links`]) — from
    /// which the structure call states how the structures differ.
    pub links: Vec<crate::dcg::verbalize::Link>,
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

    /// The recorded selections as JSON (sorted by key — deterministic bytes).
    pub fn to_json(&self) -> std::io::Result<String> {
        let log = self.log.lock().expect("selection log");
        let records: Vec<&SelectionRecord> = log.values().collect();
        serde_json::to_string_pretty(&records)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Write the recorded selections as JSON (sorted by key — deterministic bytes).
    pub fn write(&self, path: &Path) -> std::io::Result<usize> {
        let json = self.to_json()?;
        let n = self.log.lock().expect("selection log").len();
        std::fs::write(path, json)?;
        Ok(n)
    }

    /// The recorded selections as chain-ready draws (D71 §9) — the same set `write` serialises,
    /// each paired with the replay key it answers.
    pub fn keyed_draws(&self) -> std::io::Result<Vec<crate::dcg::draw::KeyedDraw>> {
        let log = self.log.lock().expect("selection log");
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

/// The same key, computed from a RECORDED exchange rather than a live one. One function, shared by
/// the replay loader and the D71 draw emitter — a second copy is where the two would diverge.
pub(crate) fn record_key(r: &SelectionRecord) -> String {
    selection_key(
        &r.sentence,
        &r.document_sha256,
        &r.prior_selections,
        &r.candidates,
    )
}

impl ReplayReadingRanker {
    /// Load a recording written by [`RecordingReadingRanker::write`].
    pub fn load(path: &Path) -> std::io::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    /// Load a recording from its JSON, wherever it came from — a draw file, or the run's
    /// `doc-<id>` branch via [`crate::dcg::draw::draws_from_layer`] (D71 §9).
    pub fn from_json(text: &str) -> std::io::Result<Self> {
        let records: Vec<SelectionRecord> = serde_json::from_str(text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut by_key = BTreeMap::new();
        for r in records {
            // The key comes from the recorded question, so it matches what `select` computes.
            let k = record_key(&r);
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
#[cfg(any(feature = "use-llm", test))]
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
#[cfg(any(feature = "use-llm", test))]
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

/// eigenius#264 — the candidates as structure choices: indices grouped by skeleton, in the
/// caller's order, and skeletons whose structural renderings coincide merged into one choice, so
/// the sense call still sees both. Grouping on the skeleton first means a sense the rendering fails
/// to hide can never split one structure into two.
fn structure_groups(candidates: &[ReadingCandidate]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut by_skeleton: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_rendering: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, c) in candidates.iter().enumerate() {
        if let Some(&g) = by_skeleton.get(c.skeleton.as_str()) {
            groups[g].push(i);
            continue;
        }
        let g = *by_rendering.entry(&c.structure).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        by_skeleton.insert(&c.skeleton, g);
        groups[g].push(i);
    }
    groups
}

/// The structure call's question without its instructions: each structure once, in the
/// sentence's words, then how the structures differ. What the harness prints for every ambiguous
/// unit under `EIGENIUS_DUMP_STRUCTURES`, without calling a model (eigenius#264).
pub fn structure_question(candidates: &[ReadingCandidate]) -> String {
    let groups = structure_groups(candidates);
    render_structures(candidates, &groups)
}

/// The shown structures, numbered from 1, and the lines saying how they differ.
fn render_structures(candidates: &[ReadingCandidate], groups: &[Vec<usize>]) -> String {
    let mut out = String::new();
    for (n, g) in groups.iter().enumerate() {
        out.push_str(&format!(
            "Structure {}: {}\n",
            n + 1,
            candidates[g[0]].structure
        ));
    }
    let contrasts = structure_contrasts(candidates, groups);
    if !contrasts.is_empty() {
        out.push_str("\nHow the structures differ:\n");
        for l in contrasts {
            out.push_str(&format!("  - {l}\n"));
        }
    }
    out
}

/// The most contrast lines a structure call shows; the rest are counted in a closing line.
const MAX_CONTRASTS: usize = 16;

/// How the shown structures differ, as lines the structure call must decide: one per phrase whose
/// role is not the same in every structure, saying what it does in each. Structures whose phrases
/// all play the same roles differ in grouping or scope, which links do not see; the first few such
/// pairs are named with the words where their renderings part.
fn structure_contrasts(candidates: &[ReadingCandidate], groups: &[Vec<usize>]) -> Vec<String> {
    use std::collections::BTreeSet;
    let reps: Vec<&ReadingCandidate> = groups.iter().map(|g| &candidates[g[0]]).collect();
    let mut roles: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
    for (n, rep) in reps.iter().enumerate() {
        for l in &rep.links {
            let (phrase, role) = phrase_and_role(l);
            roles
                .entry(phrase)
                .or_insert_with(|| vec![BTreeSet::new(); reps.len()])[n]
                .insert(role);
        }
    }
    let mut lines = Vec::new();
    for (phrase, per) in &roles {
        if per.iter().all(|r| *r == per[0]) {
            continue;
        }
        // The structures, grouped by the phrase's role, in the order they are shown.
        let mut by_role: Vec<(Vec<&str>, Vec<usize>)> = Vec::new();
        for (n, r) in per.iter().enumerate() {
            let r: Vec<&str> = r.iter().map(String::as_str).collect();
            match by_role.iter_mut().find(|(d, _)| *d == r) {
                Some((_, ns)) => ns.push(n + 1),
                None => by_role.push((r, vec![n + 1])),
            }
        }
        let parts: Vec<String> = by_role
            .iter()
            .map(|(r, ns)| match r.as_slice() {
                [] => format!("no such link in {}", numbered(ns)),
                r => format!("{} in {}", r.join(" and "), numbered(ns)),
            })
            .collect();
        lines.push(format!("{phrase}: {}", parts.join("; ")));
    }
    let mut alike = 0;
    for a in 0..reps.len() {
        for b in a + 1..reps.len() {
            if reps[a].links == reps[b].links && alike < MAX_ALIKE {
                alike += 1;
                let (x, y) = word_diff(&reps[a].structure, &reps[b].structure);
                lines.push(format!(
                    "structures {} and {} link every phrase alike and part here: «{x}» in {}, \
                     «{y}» in {}",
                    a + 1,
                    b + 1,
                    a + 1,
                    b + 1
                ));
            }
        }
    }
    if lines.len() > MAX_CONTRASTS {
        let more = lines.len() - MAX_CONTRASTS;
        lines.truncate(MAX_CONTRASTS);
        lines.push(format!("({more} further difference(s) not listed)"));
    }
    lines
}

/// The most structure pairs named as linking alike; the words where two renderings part are a
/// weaker contrast than a link, and every further pair repeats them.
const MAX_ALIKE: usize = 4;

/// A link as the contrast names it: the phrase, and what it does in one structure.
fn phrase_and_role(l: &crate::dcg::verbalize::Link) -> (String, String) {
    match l.relation.as_str() {
        "compound" => (
            format!("«{}»", l.dependent),
            format!("a noun modifier of «{}»", l.host),
        ),
        "modifier" => (
            format!("«{}»", l.dependent),
            format!("an adjective on «{}»", l.host),
        ),
        "count" => ("a count".to_string(), format!("of «{}»", l.host)),
        "argument" => (
            format!("«{}»", l.dependent),
            format!("an argument of «{}»", l.host),
        ),
        "subject" => (
            format!("«{}»", l.dependent),
            format!("the subject of «{}»", l.host),
        ),
        p => (
            format!("«{p} {}»", l.dependent),
            format!("attaches to «{}»", l.host),
        ),
    }
}

fn numbered(ns: &[usize]) -> String {
    let s: Vec<String> = ns.iter().map(usize::to_string).collect();
    match s.as_slice() {
        [one] => format!("structure {one}"),
        _ => format!("structures {}", s.join(", ")),
    }
}

/// Where two renderings part: the words between their longest common prefix and suffix, at most
/// 12 of them.
fn word_diff(a: &str, b: &str) -> (String, String) {
    let (wa, wb): (Vec<&str>, Vec<&str>) = (
        a.split_whitespace().collect(),
        b.split_whitespace().collect(),
    );
    let pre = wa.iter().zip(&wb).take_while(|(x, y)| x == y).count();
    let room = wa.len().min(wb.len()) - pre;
    let suf = wa
        .iter()
        .rev()
        .zip(wb.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    let mid = |w: &[&str]| match &w[pre..w.len() - suf] {
        [] => "—".to_string(),
        m if m.len() > 12 => format!("{} …", m[..12].join(" ")),
        m => m.join(" "),
    };
    (mid(&wa), mid(&wb))
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
        model: crate::dcg::anthropic_client::ModelConfig,
    }

    impl AnthropicReadingRanker {
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

        /// From `$ANTHROPIC_API_KEY`, defaulting to the shared client model. `None` if unset.
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

        fn ask<T: JsonSchema + serde::de::DeserializeOwned>(
            &self,
            instructions: &str,
        ) -> Option<T> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            match rt.block_on(crate::dcg::anthropic_client::anthropic_structured::<T>(
                &self.api_key,
                &self.model,
                instructions,
            )) {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("anthropic reading-ranker error: {e}");
                    None
                }
            }
        }

        /// eigenius#264 — two calls. The structure call sees each structure once, in the
        /// sentence's own words, with how the structures differ, and argues that difference; the
        /// sense call sees the chosen structure's readings as the flat listing presents them. A
        /// pool with one structure skips the first call, a structure with one reading the second.
        fn select_two_call(
            &self,
            ctx: &DocumentContext,
            candidates: &[ReadingCandidate],
        ) -> Option<ReadingSelection> {
            let mut groups = super::structure_groups(candidates);
            let total = groups.len();
            let kept = total.min(super::MAX_STRUCTURES_SHOWN);
            let dropped: usize = groups[kept..].iter().map(Vec::len).sum();
            groups.truncate(kept);
            if dropped > 0 {
                eprintln!(
                    "reading-ranker: TRUNCATED «{}» — showed {kept} of {total} structures, \
                     omitting {dropped} reading(s); the omitted ones cannot be chosen",
                    ctx.sentence.trim()
                );
            }
            let (group, structure_rationale, other_structures) = if groups.len() == 1 {
                (0, None, Vec::new())
            } else {
                let prompt = structure_prompt(ctx, candidates, &groups, total - kept);
                dump_prompt(&prompt);
                let reply: StructureReply = self.ask(&prompt)?;
                if reply.verdict == Verdict::NoneFaithful {
                    none_faithful(ctx, &reply.rationale, reply.missing.as_deref());
                    return None;
                }
                // Numbered from 1 as shown; anything else is malformed ⇒ fail closed, and say so.
                let Some(n) = reply.structure.filter(|n| (1..=groups.len()).contains(n)) else {
                    malformed(
                        ctx,
                        &format!("structure {:?} of {}", reply.structure, groups.len()),
                    );
                    return None;
                };
                let others: Vec<usize> = reply
                    .runners_up
                    .iter()
                    .filter(|&&r| r != n && (1..=groups.len()).contains(&r))
                    .map(|&r| r - 1)
                    .collect();
                (
                    n - 1,
                    Some(format!("STRUCTURE {n}: {}", reply.rationale)),
                    others,
                )
            };
            let members = &groups[group];
            let (chosen, sense_rationale, sense_runners) = if members.len() == 1 {
                (members[0], None, Vec::new())
            } else {
                let prompt = sense_prompt(ctx, candidates, members);
                dump_prompt(&prompt);
                let reply: ReadingSelectionReply = self.ask(&prompt)?;
                if reply.verdict == Verdict::NoneFaithful {
                    none_faithful(ctx, &reply.rationale, reply.missing_sense.as_deref());
                    return None;
                }
                // The readings are numbered from 0 as shown; map back to the candidate list.
                let Some(c) = reply.chosen.and_then(|c| members.get(c)) else {
                    malformed(
                        ctx,
                        &format!("reading {:?} of {}", reply.chosen, members.len()),
                    );
                    return None;
                };
                let runners = reply
                    .runners_up
                    .iter()
                    .filter_map(|&r| members.get(r).copied());
                (
                    *c,
                    Some(format!("SENSES: {}", reply.rationale)),
                    runners.collect(),
                )
            };
            // The chosen structure's other readings first, then each other structure's first.
            let mut seen = vec![false; candidates.len()];
            seen[chosen] = true;
            let runners_up: Vec<usize> = sense_runners
                .into_iter()
                .filter(|i| members.contains(i))
                .chain(other_structures.iter().map(|&g| groups[g][0]))
                .filter(|&i| !std::mem::replace(&mut seen[i], true))
                .collect();
            let rationale: Vec<String> = [structure_rationale, sense_rationale]
                .into_iter()
                .flatten()
                .collect();
            Some(ReadingSelection {
                chosen,
                rationale: rationale.join(" | "),
                runners_up,
            })
        }
    }

    /// The structure call's reply (eigenius#264).
    #[derive(Deserialize, JsonSchema)]
    struct StructureReply {
        /// `chose` when one structure matches the sentence; `none_faithful` when none does.
        verdict: Verdict,
        /// The chosen structure's number as shown, from 1. Required when `verdict` is `chose`.
        structure: Option<usize>,
        /// When `verdict` is `none_faithful`: the grouping the structures lack.
        missing: Option<String>,
        /// One or two sentences that decide the listed differences between the structures.
        rationale: String,
        /// The other structure numbers, most plausible first.
        #[serde(default)]
        runners_up: Vec<usize>,
    }

    /// `EIGENIUS_DUMP_SELECT_PROMPT=1` prints each prompt the ranker sends.
    fn dump_prompt(prompt: &str) {
        if std::env::var("EIGENIUS_DUMP_SELECT_PROMPT").is_ok() {
            eprintln!("\n===== READING-RANKER PROMPT =====\n{prompt}\n===== END PROMPT =====\n");
        }
    }

    /// A reply naming nothing that was shown: the ranker abstains, and the log says why.
    fn malformed(ctx: &DocumentContext, what: &str) {
        eprintln!(
            "reading-ranker: MALFORMED reply on «{}» — {what}; abstained",
            ctx.sentence.trim()
        );
    }

    /// "No candidate is faithful" is a result, and its diagnostic names what the pool lacks.
    fn none_faithful(ctx: &DocumentContext, rationale: &str, missing: Option<&str>) {
        eprintln!(
            "reading-ranker: NONE FAITHFUL on «{}» — {rationale}{}",
            ctx.sentence.trim(),
            missing
                .map(|m| format!("  [missing: {m}]"))
                .unwrap_or_default()
        );
    }

    /// The readings already selected for earlier sentences, for consistency.
    fn prior_block(ctx: &DocumentContext) -> String {
        let mut out = String::new();
        if !ctx.prior_selections.is_empty() {
            out.push_str(
                "Readings already selected for earlier sentences (stay consistent with them):\n",
            );
            for p in ctx.prior_selections {
                out.push_str(&format!("  sentence {}: \"{}\"\n", p.ordinal, p.gloss));
            }
            out.push('\n');
        }
        out
    }

    /// The structure call: each shown structure once, in the sentence's words, and the lines
    /// saying how they differ, which the rationale must decide.
    fn structure_prompt(
        ctx: &DocumentContext,
        candidates: &[ReadingCandidate],
        groups: &[Vec<usize>],
        dropped_structures: usize,
    ) -> String {
        let mut shown = super::render_structures(candidates, groups);
        if dropped_structures > 0 {
            shown.push_str(&format!(
                "(NOTE: {dropped_structures} further structure(s) are not shown and cannot be \
                 chosen. If none of the above is faithful, say so rather than picking the \
                 closest.)\n"
            ));
        }
        format!(
            "A parser read the document below and found several STRUCTURES for one of its \
             sentences — different ways the sentence's words combine. Choose the structure that \
             matches what the sentence means in the context of the document.\n\n\
             Word senses are not the question here. Every word is shown as the sentence writes \
             it; a second step chooses the senses within the structure you pick.\n\n\
             Document:\n{}\n\n{}\
             The sentence:\n  \"{}\"\n\n\
             Structures. `«…»` is a word of the sentence; `+ X` is a relation the structure \
             asserts of the phrase before it; `compound-with` marks a noun modifier whose \
             relation the sentence leaves unspecified; `and` joins separate claims.\n\
             {shown}\n\
             Return verdict `chose` with `structure` = the number of the structure whose grouping \
             the sentence means, `rationale` = one or two sentences that decide the differences \
             listed above (where each phrase attaches, and why), and `runners_up` = the other \
             structure numbers in preference order. If no structure is faithful, return verdict \
             `none_faithful` and say in `missing` which grouping the structures lack.",
            ctx.document.trim(),
            prior_block(ctx),
            ctx.sentence.trim(),
        )
    }

    /// The sense call: the chosen structure's readings, flat, with the legend of the concepts
    /// they name.
    fn sense_prompt(
        ctx: &DocumentContext,
        candidates: &[ReadingCandidate],
        members: &[usize],
    ) -> String {
        let listing: String = members
            .iter()
            .enumerate()
            .map(|(n, &i)| format!("  [{n}] {}\n", candidates[i].gloss))
            .collect();
        let named: Vec<super::ConceptNote> = ctx
            .concepts
            .iter()
            .filter(|c| {
                let id = format!("[{}]", c.id);
                members.iter().any(|&i| candidates[i].gloss.contains(&id))
            })
            .cloned()
            .collect();
        format!(
            "A parser read the document below and produced several candidate READINGS \
             (interpretations) of one sentence. Their structure is settled:\n  {}\n\
             The readings below share it and differ in word sense. Choose the reading whose word \
             senses match what the sentence means in the context of the document.\n\n\
             Document:\n{}\n\n{}\
             The sentence to disambiguate:\n  \"{}\"\n\n\
             Candidate readings. `«label» [id]` names a concept, `+ relation X` is an explicit \
             relation the reading asserts, and `⟦…⟧` marks a fragment that could not be \
             rendered.\n{listing}{}\n\
             Return verdict `chose` with `chosen` = the index of the reading whose word senses \
             match the sentence's intended meaning, `rationale` = one sentence why, and \
             `runners_up` = the remaining indices in preference order. If no reading is faithful, \
             return verdict `none_faithful` and say in `missing_sense` which sense is missing.",
            candidates[members[0]].structure,
            ctx.document.trim(),
            prior_block(ctx),
            ctx.sentence.trim(),
            concept_legend(&named),
        )
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
            // eigenius#264: the two-call ranker is OPT-IN until its A/B against this flat listing
            // (three live draws per arm on one snapshot) decides the default.
            if std::env::var("EIGENIUS_SELECT_TWO_CALL").is_ok() {
                return self.select_two_call(ctx, candidates);
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
            let reply: ReadingSelectionReply = self.ask(&prompt)?;
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
                structure: format!("structure {i}"),
                links: Vec::new(),
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
                structure: "we saw the man by using a telescope".to_string(),
                links: Vec::new(),
            },
            ReadingCandidate {
                skeleton: "see(§)(we, man_with(telescope))".to_string(),
                gloss: "we saw the man who was holding a telescope".to_string(),
                sem: String::new(),
                structure: "we saw the man who was holding a telescope".to_string(),
                links: Vec::new(),
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
            structure: String::new(),
            links: Vec::new(),
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

#[cfg(test)]
mod two_call_tests {
    use super::{structure_contrasts, structure_groups, word_diff, ReadingCandidate};
    use crate::dcg::verbalize::Link;

    fn link(relation: &str, dependent: &str, host: &str) -> Link {
        Link {
            relation: relation.into(),
            dependent: dependent.into(),
            host: host.into(),
        }
    }

    fn cand(structure: &str, links: Vec<Link>) -> ReadingCandidate {
        ReadingCandidate {
            skeleton: structure.into(),
            gloss: String::new(),
            sem: String::new(),
            structure: structure.into(),
            links,
        }
    }

    /// eigenius#264's witness: «The MSI relationship compared favourably to other strong
    /// biomarkers for vulnerabilities.» — two structures, one link apart.
    #[test]
    fn the_contrast_names_the_attachment_that_differs() {
        let shared = || {
            vec![
                link("compound", "MSI", "relationship"),
                link("to", "biomarkers", "relationship"),
            ]
        };
        let mut flat = shared();
        flat.push(link("for", "vulnerabilities", "relationship"));
        let mut nested = shared();
        nested.push(link("for", "vulnerabilities", "biomarkers"));
        let c = [
            cand(
                "the «relationship» … and the «relationship» for «vulnerabilities»",
                flat.clone(),
            ),
            cand(
                "the «relationship» … «biomarkers» + for «vulnerabilities»",
                nested.clone(),
            ),
            cand(
                "the «relationship» … «biomarkers» + for «vulnerabilities»",
                nested,
            ),
        ];
        let groups = structure_groups(&c);
        assert_eq!(
            groups,
            vec![vec![0], vec![1, 2]],
            "alike renderings share a group"
        );
        assert_eq!(
            structure_contrasts(&c, &groups),
            vec![
                "«for vulnerabilities»: attaches to «relationship» in structure 1; attaches to \
                 «biomarkers» in structure 2"
                    .to_string()
            ]
        );
    }

    /// Grouping is by skeleton first: a sense the rendering fails to hide cannot split a structure.
    #[test]
    fn one_skeleton_is_one_structure_whatever_it_renders_as() {
        let mut leaky = cand("x «a02734544»", Vec::new());
        leaky.skeleton = "S".into();
        let mut other = cand("x «a02734192»", Vec::new());
        other.skeleton = "S".into();
        assert_eq!(structure_groups(&[leaky, other]), vec![vec![0, 1]]);
    }

    /// Structures no link separates differ in grouping or scope; the contrast shows where their
    /// renderings part.
    #[test]
    fn structures_alike_in_links_are_named_by_where_they_part() {
        let c = [
            cand("possibly ( «A» or «B» )", Vec::new()),
            cand("possibly «A» or possibly «B»", Vec::new()),
        ];
        let groups = structure_groups(&c);
        assert_eq!(
            structure_contrasts(&c, &groups),
            vec![
                "structures 1 and 2 link every phrase alike and part here: «( «A» or «B» )» in 1, \
                 ««A» or possibly «B»» in 2"
                    .to_string()
            ]
        );
        assert_eq!(
            word_diff("a b c d", "a x d"),
            ("b c".to_string(), "x".to_string())
        );
        assert_eq!(
            word_diff("a b", "a b c"),
            ("—".to_string(), "c".to_string())
        );
    }
}
