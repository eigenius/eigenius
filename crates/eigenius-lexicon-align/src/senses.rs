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

//! **The sense judge** (D97 decisions 1 and 7; eigenius#263) — which senses of an adjective or a
//! verb take the complement SPECIALIST (or, for adjectives, the curated frames) names for the lemma.
//!
//! SPECIALIST speaks for a lemma; the importer emits per WordNet sense. Where the evidence does not
//! decide — the lemma has several senses — the judge reads each sense's gloss and says whether the
//! word, in that sense, takes the complement: for an adjective `<preposition> + noun phrase`; for a
//! verb that, a direct object, or a that-clause ([`Part`]).
//!
//! It follows the WordNet–UMLS alignment's protocol ([`crate::adjudicate`]):
//! - its verdicts are **recorded** (`{part}-senses.jsonl`), never taken at build time;
//! - it is **scored first** on the items WordNet decides, with WordNet's evidence hidden
//!   ([`score_gold`]): below 95% recall it is not trusted. For an adjective the evidence is the
//!   gloss's ``followed by `on'`` convention; for a verb, its frames of the complement's kind;
//! - a sense is placed at confidence ≥ 0.85; where the judge places none there, the senses it said
//!   yes to below it; where it said no to every sense, nothing, and the item is recorded as a gap —
//!   a use WordNet has no sense for, as the evaluative `it was good of you` (decided 2026-09-30) —
//!   [`resolve`] writes the placements the importer reads (`{part}-senses.tsv`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use eigenius_wordnet::governance::{classify, convention_prepositions};
use eigenius_wordnet::morphy::{morphstr, ExcLists, LemmaSet};
use eigenius_wordnet::verb_governance::{classify_verbs, label, shown_frames, OpenVerbItem};
use eigenius_wordnet::wndb::{read_data_file, Offset, Synset};
use serde::{Deserialize, Serialize};

/// A sense's confidence at or above which the judge places it.
pub const ACCEPT: f32 = 0.85;
/// The gold recall below which the judge is not trusted.
pub const MIN_RECALL: f64 = 0.95;

/// Which judge: the adjectives' (D97 decision 7) or the verbs' (decision 1, revised 2026-10-01).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Adjective,
    Verb,
}

impl Part {
    pub fn name(self) -> &'static str {
        match self {
            Part::Adjective => "adjective",
            Part::Verb => "verb",
        }
    }
}

/// One sense, as the judge sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sense {
    pub offset: Offset,
    /// WordNet's gloss, its examples included.
    pub gloss: String,
    /// A verb sense's WordNet patterns (`Somebody ----s something`), less those of the kind judged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<String>,
}

/// A (lemma, complement) whose senses the judge decides. The complement is a preposition, or for a
/// verb `object` or `clause`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub lemma: String,
    pub complement: String,
    pub senses: Vec<Sense>,
    /// For a gold item, the senses WordNet's evidence places; empty otherwise.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub gold: BTreeSet<Offset>,
}

/// The judge's items, read once.
pub struct Sources {
    pub part: Part,
    /// The items on several senses: the judge's work.
    pub open: Vec<Item>,
    /// The items it is scored on.
    pub gold: Vec<Item>,
    /// What the classification counted, one line each.
    pub summary: Vec<String>,
    /// WordNet's verb morphology, for the verbs' adjacency test; none for adjectives.
    pub forms: Option<VerbForms>,
}

impl Sources {
    /// `verdicts` with the senses whose sentence does not show their frame turned to no
    /// ([`VerbForms::require_frame`]), and how many were; unchanged for adjectives, whose judge
    /// writes no sentence.
    pub fn require_frame(&self, verdicts: &[Verdict]) -> (Vec<Verdict>, usize) {
        match &self.forms {
            Some(f) => f.require_frame(verdicts),
            None => (verdicts.to_vec(), 0),
        }
    }
}

/// Read WordNet's synsets of `part` and SPECIALIST, and sort every attested complement.
pub fn load(dict: &Path, specialist: &Path, part: Part) -> Result<Sources, String> {
    let read = |pos: &str| {
        let path = dict.join(format!("data.{pos}"));
        read_data_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    };
    let lexicon = eigenius_specialist::Lexicon::read(specialist)
        .map_err(|e| format!("{}: {e}", specialist.display()))?;
    match part {
        Part::Adjective => Ok(adjective_sources(&read("adj")?, &lexicon)),
        Part::Verb => {
            let verbs = read("verb")?;
            let exc = ExcLists::load(dict).map_err(|e| format!("{}: {e}", dict.display()))?;
            let mut sources = verb_sources(&verbs, &lexicon);
            sources.forms = Some(VerbForms::new(exc, &verbs));
            Ok(sources)
        }
    }
}

fn adjective_sources(
    adjectives: &BTreeMap<Offset, Synset>,
    lexicon: &eigenius_specialist::Lexicon,
) -> Sources {
    let classified = classify(
        adjectives,
        Some(lexicon),
        eigenius_wordnet::convert::adjective_frames(),
    );
    let open: Vec<Item> = classified
        .open
        .iter()
        .map(|o| Item {
            lemma: o.lemma.clone(),
            complement: o.preposition.clone(),
            senses: o
                .senses
                .iter()
                .map(|off| Sense {
                    offset: off.clone(),
                    gloss: adjectives[off].gloss.clone(),
                    frames: Vec::new(),
                })
                .collect(),
            gold: BTreeSet::new(),
        })
        .collect();
    // The gold: the open items with a sense whose own gloss names the preposition in WordNet's
    // convention (``followed by `on'``) — a per-sense fact the judge must recover.
    let gold = open
        .iter()
        .cloned()
        .filter_map(|mut item| {
            item.gold = item
                .senses
                .iter()
                .filter(|s| convention_prepositions(&s.gloss).contains(&item.complement))
                .map(|s| s.offset.clone())
                .collect();
            (!item.gold.is_empty()).then_some(item)
        })
        .collect();
    let c = &classified.counts;
    Sources {
        part: Part::Adjective,
        open,
        gold,
        summary: vec![format!(
            "{} adjective lemmas attested; {} items in lexicon:Prep: {} on one sense, {} for the \
             judge; outside lexicon:Prep: {:?}",
            c.lemmas_attested, c.items, c.one_sense, c.open, c.outside
        )],
        forms: None,
    }
}

fn verb_sources(
    verbs: &BTreeMap<Offset, Synset>,
    lexicon: &eigenius_specialist::Lexicon,
) -> Sources {
    let classified = classify_verbs(verbs, lexicon);
    let item = |o: &OpenVerbItem| Item {
        lemma: o.lemma.clone(),
        complement: label(&o.complement).to_string(),
        senses: o
            .senses
            .iter()
            .map(|off| Sense {
                offset: off.clone(),
                gloss: verbs[off].gloss.clone(),
                frames: shown_frames(&verbs[off], &o.lemma, &o.complement)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            })
            .collect(),
        gold: BTreeSet::new(),
    };
    let open = classified.open.iter().map(item).collect();
    let gold = classified
        .gold
        .iter()
        .map(|g| Item {
            gold: g.gold.clone(),
            ..item(&g.item)
        })
        .collect();
    let c = &classified.counts;
    let mut summary = vec![format!(
        "{} verb lemmas with a SPECIALIST complement; outside lexicon:Prep: {:?}",
        c.lemmas_attested, c.outside
    )];
    for (kind, k) in &c.by_kind {
        summary.push(format!(
            "  {kind}: {} on one sense, {} for the judge, {} met by WordNet's frames",
            k.one_sense, k.open, k.met_by_wordnet
        ));
    }
    Sources {
        part: Part::Verb,
        open,
        gold,
        summary,
        forms: None,
    }
}

/// The kind of an item's complement, for reporting: `preposition`, `object` or `clause`.
pub fn kind_of(complement: &str) -> &'static str {
    match complement {
        "object" => "object",
        "clause" => "clause",
        _ => "preposition",
    }
}

/// One sense's verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SenseVerdict {
    pub offset: Offset,
    /// In this sense, does the word take the complement?
    pub fits: bool,
    /// 0–1.
    pub confidence: f32,
    /// The verb judge's sentence using the word in this sense with the complement, where it wrote
    /// one; the adjective judge writes none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

/// One item's recorded verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub lemma: String,
    pub complement: String,
    pub senses: Vec<SenseVerdict>,
    pub model: String,
}

impl Verdict {
    /// The senses placed at `threshold`.
    pub fn accepted(&self, threshold: f32) -> BTreeSet<Offset> {
        self.senses
            .iter()
            .filter(|s| s.fits && s.confidence >= threshold)
            .map(|s| s.offset.clone())
            .collect()
    }
}

/// The judge's score on the gold items, per sense.
#[derive(Debug, Default, PartialEq)]
pub struct GoldScore {
    pub true_pos: usize,
    pub false_pos: usize,
    pub false_neg: usize,
    /// Senses the evidence does not place and the judge does not accept. Not a known negative —
    /// WordNet's frames are not complete — so `false_pos` over `false_pos + true_neg` is how often
    /// the judge goes beyond WordNet, not its error rate.
    pub true_neg: usize,
    /// Items not in the verdicts.
    pub missing: usize,
    /// `lemma complement sense: gold / judge` for every disagreement — read these; they are where
    /// the judge is wrong, or the evidence is.
    pub disagreements: Vec<String>,
}

impl GoldScore {
    pub fn recall(&self) -> f64 {
        ratio(self.true_pos, self.true_pos + self.false_neg)
    }

    pub fn precision(&self) -> f64 {
        ratio(self.true_pos, self.true_pos + self.false_pos)
    }
}

fn ratio(n: usize, d: usize) -> f64 {
    if d == 0 {
        0.0
    } else {
        n as f64 / d as f64
    }
}

/// The union of several judges' verdicts: per (item, sense), the most confident yes if any judge
/// said yes, else the most confident no — so a sense is accepted at a threshold wherever any of them
/// accepts it there.
pub fn union(runs: &[Vec<Verdict>]) -> Vec<Verdict> {
    let mut merged: BTreeMap<(String, String), Verdict> = BTreeMap::new();
    for v in runs.iter().flatten() {
        let key = (v.lemma.clone(), v.complement.clone());
        let Some(m) = merged.get_mut(&key) else {
            merged.insert(key, v.clone());
            continue;
        };
        for s in &v.senses {
            match m.senses.iter_mut().find(|t| t.offset == s.offset) {
                None => m.senses.push(s.clone()),
                Some(t) => {
                    if (s.fits, s.confidence) > (t.fits, t.confidence) {
                        *t = s.clone();
                    }
                }
            }
        }
    }
    merged.into_values().collect()
}

fn by_key(verdicts: &[Verdict]) -> BTreeMap<(&str, &str), &Verdict> {
    verdicts
        .iter()
        .map(|v| ((v.lemma.as_str(), v.complement.as_str()), v))
        .collect()
}

/// Score `verdicts` against the gold items: a sense the evidence placed and the judge accepts at
/// `threshold` is a true positive.
pub fn score_gold(gold: &[Item], verdicts: &[Verdict], threshold: f32) -> GoldScore {
    let by_key = by_key(verdicts);
    let mut score = GoldScore::default();
    for item in gold {
        let Some(v) = by_key.get(&(item.lemma.as_str(), item.complement.as_str())) else {
            score.missing += 1;
            continue;
        };
        let accepted = v.accepted(threshold);
        for s in &item.senses {
            let (g, j) = (item.gold.contains(&s.offset), accepted.contains(&s.offset));
            match (g, j) {
                (true, true) => score.true_pos += 1,
                (false, true) => score.false_pos += 1,
                (true, false) => score.false_neg += 1,
                (false, false) => score.true_neg += 1,
            }
            if g != j {
                score.disagreements.push(format!(
                    "{} {} {}: evidence {}, judge {} — {}",
                    item.lemma,
                    item.complement,
                    s.offset,
                    if g { "yes" } else { "no" },
                    if j { "yes" } else { "no" },
                    s.gloss
                ));
            }
        }
    }
    score
}

/// How an open item was placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Placed {
    /// On the senses the judge accepts at the threshold.
    Accepted,
    /// On the senses it said yes to, all below the threshold.
    BelowThreshold,
    /// On no sense: the judge said no to every one.
    Gap,
}

/// One resolved placement.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Placement {
    pub lemma: String,
    pub complement: String,
    pub senses: BTreeSet<Offset>,
    pub placed: Placed,
}

/// The placements the importer reads: for each open item, the senses the judge accepts at
/// `threshold`; where it accepts none, the senses it said yes to below it; where it said no to every
/// sense, none — a gap. `Err` names the open items with no verdict: the placements are written whole
/// or not at all.
pub fn resolve(
    items: &[Item],
    verdicts: &[Verdict],
    threshold: f32,
) -> Result<Vec<Placement>, Vec<String>> {
    let by_key = by_key(verdicts);
    let mut out = Vec::new();
    let mut missing = Vec::new();
    for item in items {
        let Some(v) = by_key.get(&(item.lemma.as_str(), item.complement.as_str())) else {
            missing.push(format!("{} {}", item.lemma, item.complement));
            continue;
        };
        let all: BTreeSet<Offset> = item.senses.iter().map(|s| s.offset.clone()).collect();
        let accepted: BTreeSet<Offset> =
            v.accepted(threshold).intersection(&all).cloned().collect();
        let yes: BTreeSet<Offset> = v.accepted(0.0).intersection(&all).cloned().collect();
        let (senses, placed) = if !accepted.is_empty() {
            (accepted, Placed::Accepted)
        } else if !yes.is_empty() {
            (yes, Placed::BelowThreshold)
        } else {
            (BTreeSet::new(), Placed::Gap)
        };
        out.push(Placement {
            lemma: item.lemma.clone(),
            complement: item.complement.clone(),
            senses,
            placed,
        });
    }
    if missing.is_empty() {
        out.sort();
        Ok(out)
    } else {
        Err(missing)
    }
}

/// The placements file the importer reads: `lemma <TAB> complement <TAB> offset,…`, or `-` for a
/// gap.
pub fn render_placements(part: Part, rows: &[Placement], threshold: f32) -> String {
    let count = |p: Placed| rows.iter().filter(|r| r.placed == p).count();
    let (title, command, column) = match part {
        Part::Adjective => (
            "adjective sense judge's placements (D97 decision 7; eigenius#263)",
            "`specialist-senses resolve` from adjective-senses.jsonl",
            "lemma <TAB> preposition <TAB> WordNet adjective offsets",
        ),
        Part::Verb => (
            "verb sense judge's placements (D97 decisions 1 and 6, slice 2)",
            "`specialist-senses --pos verb resolve` from verb-senses.jsonl; a preposition is placed\n\
             # only where the judge's sentence shows it right after the verb",
            "lemma <TAB> complement (a preposition, `object`, `clause`) <TAB> WordNet verb offsets",
        ),
    };
    let mut out = format!(
        "# The {title} — generated by\n\
         # {command}; do not edit by hand.\n\
         # {} items: {} on the senses accepted at confidence >= {threshold}; {} on the senses the judge\n\
         # said yes to below it; {} gaps (`-`), where it said no to every sense — a use WordNet has no\n\
         # sense for (decided 2026-09-30).\n\
         # {column}\n",
        rows.len(),
        count(Placed::Accepted),
        count(Placed::BelowThreshold),
        count(Placed::Gap)
    );
    for r in rows {
        let senses: Vec<&str> = r.senses.iter().map(String::as_str).collect();
        let senses = if senses.is_empty() {
            "-".to_string()
        } else {
            senses.join(",")
        };
        out.push_str(&format!("{}\t{}\t{senses}\n", r.lemma, r.complement));
    }
    out
}

/// Read recorded verdicts (JSON lines); a malformed line is an error.
pub fn read_verdicts(path: &Path) -> Result<Vec<Verdict>, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| {
            serde_json::from_str(l).map_err(|e| format!("{} line {}: {e}", path.display(), i + 1))
        })
        .collect()
}

/// What the model returns for one batch.
#[cfg(feature = "use-llm")]
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchReply {
    pub items: Vec<ItemRow>,
}

/// One item's row, in the batch's order, checked by the echoed word. No item index: on
/// `claude-opus-5-5` a constrained item index ran away (`"index":110123456789…`) until the reply hit
/// `max_tokens` (2026-10-01). The senses keep theirs: without it `claude-sonnet-4-6` skipped a sense
/// of long items (22 of `drop`'s 23).
#[cfg(feature = "use-llm")]
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ItemRow {
    /// The item's word, as given.
    pub word: String,
    /// One verdict per sense of the item, in the order given.
    pub senses: Vec<SenseRow>,
}

#[cfg(feature = "use-llm")]
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SenseRow {
    /// The sense's index within its item (0-based).
    pub index: usize,
    /// In this sense, does the word take the item's complement?
    pub fits: bool,
    /// 0.0–1.0.
    pub confidence: f32,
    /// A sentence using the word in this sense with the complement, when it fits.
    pub example: Option<String>,
}

/// How the verb judge's instructions are worded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wording {
    /// One text naming all three kinds of complement.
    Shared,
    /// A text per kind; a batch holds one kind ([`batches`]). The clause text says a sense shown
    /// with a noun object can take a clause; the object text names the causative alternation.
    PerKind,
}

/// `items` in batches of at most `size`, each of one kind of complement, so a batch has one prompt.
pub fn batches(items: &[Item], size: usize) -> Vec<Vec<Item>> {
    let mut by_kind: BTreeMap<&str, Vec<Item>> = BTreeMap::new();
    for i in items {
        by_kind
            .entry(kind_of(&i.complement))
            .or_default()
            .push(i.clone());
    }
    by_kind
        .into_values()
        .flat_map(|v| {
            v.chunks(size.max(1))
                .map(<[Item]>::to_vec)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The prompt for one batch of items. The evidence is never shown: a gold item looks like any other.
pub fn prompt(part: Part, wording: Wording, batch: &[&Item]) -> String {
    match part {
        Part::Adjective => adjective_prompt(batch),
        Part::Verb => verb_prompt(wording, batch),
    }
}

const RETURN_ROWS: &str =
    "Return one row per item, in the order given, with the item's word, and in \
     it one verdict per sense, with the sense's number";

fn adjective_prompt(batch: &[&Item]) -> String {
    let mut p = String::from(
        "You decide which senses of an English adjective take a prepositional complement.\n\
         Each item is an adjective, a preposition, and the adjective's senses from WordNet: a \
         definition, often with examples in quotes.\n\n\
         For EACH sense, answer whether the adjective, IN THAT SENSE, takes a complement made of \
         the preposition and a noun phrase — as in `dependent on the drug`, `resistant to \
         treatment`, `responsible for the delay`, `concordant with the results` — with a confidence \
         from 0 to 1.\n\
         - A sense fits when the noun phrase after the preposition completes THAT meaning: \
           `dependent on X` fits \"relying on something for support\", not \"(of a clause) \
           subordinate\".\n\
         - A preposition that only adds a time, place or manner adjunct is not a complement.\n\
         - Judge each sense on its own. Several senses may fit; none may.\n\
         - If you cannot tell, answer fits=false with a low confidence.\n\n",
    );
    for (i, item) in batch.iter().enumerate() {
        p.push_str(&format!(
            "[{i}] adjective: {}   preposition: {}   example: something is {} {} something\n",
            item.lemma, item.complement, item.lemma, item.complement
        ));
        for (j, s) in item.senses.iter().enumerate() {
            p.push_str(&format!("   ({j}) {}\n", s.gloss.trim()));
        }
        p.push('\n');
    }
    p.push_str(RETURN_ROWS);
    p.push('.');
    p
}

const VERB_INTRO: &str = "You decide which senses of an English verb take a given complement.\n\
     Each item is a verb, a complement, and the verb's senses from WordNet: a definition, often with \
     examples in quotes, and some of the sentence patterns WordNet lists for the sense (`----` stands \
     for the verb).\n\n";

/// The steps every verb wording ends with; `shape` is the sentence the judge tries to write.
fn verb_steps(shape: &str) -> String {
    format!(
        "For EACH sense:\n\
         1. Try to write one natural English sentence {shape} that uses the verb IN THAT SENSE.\n\
         2. If you can, answer fits=true and give the sentence as the example. It does not matter \
            whether the sentence can also be read in another of the verb's senses: WordNet's senses \
            overlap, and a complement often belongs to several. If no such sentence exists — the \
            verb does not take the complement in this sense, or it would only be an adjunct — answer \
            fits=false with no example.\n\
         3. Give a confidence from 0 to 1.\n"
    )
}

const VERB_NOTES: &str =
    "- Definitions rarely mention complements, and the patterns listed are not all \
       a sense's patterns: decide from how the verb is used in that sense.\n\
     - Judge each sense on its own. Several senses may fit; none may.\n\n";

fn verb_instructions(wording: Wording, kind: &str) -> String {
    match (wording, kind) {
        (Wording::Shared, _) => format!(
            "The complement is one of:\n\
             - a preposition right after the verb, and a noun phrase that the verb selects — \
               `consist of two parts`, `rely on the assay`, `interact with the protein`, `belong to \
               the family`. A prepositional phrase that only adds a time, place, manner, duration or \
               purpose (`grow in the lab`, `wait for hours`) is not one; nor is a preposition after an \
               object (`insert the probe into the cell`, `attribute the effect to the drug`), which is \
               another construction;\n\
             - a direct object: a noun phrase right after the verb — `inhibit the enzyme`, `express \
               the gene`;\n\
             - a that-clause — `report that the drug failed`, `find that the levels were high`.\n\n\
             {}Notes:\n\
             - A verb of saying, thinking, knowing, finding or showing takes a that-clause in each \
               sense whose content can be a statement.\n\
             {VERB_NOTES}",
            verb_steps("with the complement")
        ),
        (Wording::PerKind, "clause") => format!(
            "The complement is a that-clause stating what is said, thought, known, found, decided, \
             shown or demanded — `report that the drug failed`.\n\n\
             {}Notes:\n\
             - A sense shown only with a noun object can still take a clause: «She noticed the \
               stain» and «She noticed that the stain had spread» use one sense of `notice`, as do \
               «The test detected lead» and «The test detected that lead was present».\n\
             {VERB_NOTES}",
            verb_steps("`<subject> <verb> that <clause>`")
        ),
        (Wording::PerKind, "object") => format!(
            "The complement is a direct object: a noun phrase right after the verb, with no \
             preposition before it — `inhibit the enzyme`, `express the gene`, `weigh ten grams`.\n\n\
             {}Notes:\n\
             - Many verbs alternate between a use without an object and a causative use with the \
               same meaning: «The ice thawed» / «The sun thawed the ice», «The glass shattered» / \
               «The blast shattered the glass». A sense shown only without an object takes one when \
               its causative use keeps that meaning.\n\
             - A noun phrase after a preposition (`point at the chart`) is not a direct object, nor \
               is one after a particle the verb needs (`burn off calories`).\n\
             {VERB_NOTES}",
            verb_steps("`<subject> <verb> <noun phrase>`")
        ),
        (Wording::PerKind, _) => format!(
            "The complement is the item's preposition and a noun phrase that the verb selects, \
             completing its meaning — `consist of two parts`, `rely on the assay`, `interact with \
             the protein`, `belong to the family`.\n\n\
             {}Notes:\n\
             - A prepositional phrase that only adds a time, place, manner, duration or purpose \
               (`grow in the lab`, `wait for hours`) is not a complement.\n\
             - Nothing comes between the verb and the preposition: a preposition after an object \
               (`insert the probe into the cell`, `attribute the effect to the drug`) is another \
               construction.\n\
             {VERB_NOTES}",
            verb_steps("`<subject> <verb> <preposition> <noun phrase>`")
        ),
    }
}

fn verb_prompt(wording: Wording, batch: &[&Item]) -> String {
    let kind = batch
        .first()
        .map_or("preposition", |i| kind_of(&i.complement));
    debug_assert!(
        wording == Wording::Shared || batch.iter().all(|i| kind_of(&i.complement) == kind),
        "a per-kind batch holds one kind"
    );
    let mut p = format!("{VERB_INTRO}{}", verb_instructions(wording, kind));
    for (i, item) in batch.iter().enumerate() {
        let (complement, pattern) = match item.complement.as_str() {
            "object" => (
                "a direct object".to_string(),
                format!("{} <noun phrase>", item.lemma),
            ),
            "clause" => (
                "a that-clause".to_string(),
                format!("{} that <clause>", item.lemma),
            ),
            prep => (
                format!("`{prep}` + noun phrase"),
                format!("{} {prep} <noun phrase>", item.lemma),
            ),
        };
        p.push_str(&format!(
            "[{i}] verb: {}   complement: {complement}   pattern: {pattern}\n",
            item.lemma
        ));
        for (j, s) in item.senses.iter().enumerate() {
            p.push_str(&format!("   ({j}) {}\n", s.gloss.trim()));
            if !s.frames.is_empty() {
                p.push_str(&format!("       patterns: {}\n", s.frames.join("; ")));
            }
        }
        p.push('\n');
    }
    p.push_str(RETURN_ROWS);
    p.push_str(" and, where it fits, the example.");
    p
}

/// The reply cap. A batch of verbs with dozens of senses, a sentence for each, passed the default
/// 4,096 tokens on 12 of 209 batches (2026-10-01).
#[cfg(feature = "use-llm")]
const REPLY_TOKENS: u32 = 16_000;

/// Whether a row's `word` names `lemma`: the lemma itself, or the lemma followed by what the item
/// adds (`come [before]` for `come`'s `before` item).
#[cfg(any(feature = "use-llm", test))]
fn echoes(word: &str, lemma: &str) -> bool {
    let w = word.trim().to_lowercase();
    let l = lemma.to_lowercase();
    w == l
        || w.strip_prefix(&l)
            .is_some_and(|rest| rest.starts_with(|c: char| !c.is_alphanumeric()))
}

/// Judge one batch. `Err` on any transport, API or decode failure, and on a reply that does not
/// answer every item and every sense exactly once — the caller records nothing for the batch.
#[cfg(feature = "use-llm")]
pub async fn judge_batch(
    api_key: &str,
    model: &str,
    part: Part,
    wording: Wording,
    batch: &[&Item],
) -> Result<Vec<Verdict>, String> {
    let reply: BatchReply = eigenius_kernel::dcg::anthropic_client::anthropic_structured(
        api_key,
        &eigenius_kernel::dcg::anthropic_client::ModelConfig::requested(model, model, REPLY_TOKENS),
        &prompt(part, wording, batch),
    )
    .await?;
    if reply.items.len() != batch.len() {
        return Err(format!(
            "{} rows for {} items",
            reply.items.len(),
            batch.len()
        ));
    }
    let mut out = Vec::with_capacity(batch.len());
    for (item, row) in batch.iter().zip(&reply.items) {
        if !echoes(&row.word, &item.lemma) {
            return Err(format!(
                "row for `{}` where `{}` was asked",
                row.word, item.lemma
            ));
        }
        let mut senses: BTreeMap<usize, &SenseRow> = BTreeMap::new();
        for r in &row.senses {
            if r.index >= item.senses.len() || senses.insert(r.index, r).is_some() {
                return Err(format!(
                    "{} {}: sense index {} out of range or repeated",
                    item.lemma, item.complement, r.index
                ));
            }
        }
        if senses.len() != item.senses.len() {
            return Err(format!(
                "{} {}: {} of {} senses answered",
                item.lemma,
                item.complement,
                senses.len(),
                item.senses.len()
            ));
        }
        out.push(Verdict {
            lemma: item.lemma.clone(),
            complement: item.complement.clone(),
            senses: item
                .senses
                .iter()
                .zip(senses.values())
                .map(|(s, r)| SenseVerdict {
                    offset: s.offset.clone(),
                    fits: r.fits,
                    confidence: r.confidence.clamp(0.0, 1.0),
                    example: r
                        .example
                        .as_ref()
                        .map(|e| e.trim().to_string())
                        .filter(|e| !e.is_empty()),
                })
                .collect(),
            model: model.to_string(),
        });
    }
    Ok(out)
}

/// WordNet's verb morphology, for reading the judge's sentences: the exception lists and the verb
/// lemmas ([`eigenius_wordnet::morphy`]).
pub struct VerbForms {
    exc: ExcLists,
    lemmas: LemmaSet,
}

impl VerbForms {
    pub fn new(exc: ExcLists, verbs: &BTreeMap<Offset, Synset>) -> VerbForms {
        VerbForms {
            exc,
            lemmas: LemmaSet::from_synsets([verbs]),
        }
    }

    /// Whether `word` is a form of the verb `base`: `flew` of `fly`, `starred` of `star`.
    fn is_form_of(&self, word: &str, base: &str) -> bool {
        word == base
            || morphstr(
                word,
                eigenius_wordnet::wndb::Pos::Verb,
                &self.exc,
                &self.lemmas,
            )
            .iter()
            .any(|b| b == base)
    }

    /// Whether `sentence` shows the frame a placement of `complement` puts on `lemma`'s sense. For
    /// a preposition, `(S\NP)/PP`: a form of the verb (its head inflected, the rest of a multiword
    /// lemma as is) directly followed by the preposition. An object or a clause is not tested.
    ///
    /// The judge writes a sentence for each sense it accepts. Where an object comes between the
    /// verb and the preposition — «The doctor referred the patient to a specialist» — the sentence
    /// shows the object + PP frame of D97 slice 4, and a bare PP frame on the sense would be wrong.
    /// Measured 2026-10-01: 372 of 2,676 accepted preposition senses fail; 24 of a random 25 are
    /// object + PP, the 25th an adverb between («danced gracefully with»). It costs nothing on the
    /// gold: every sense WordNet's `to` and `on` frames confirm passes. It does not catch a particle
    /// («box up the old books»), a passive by-phrase or a sibling sense.
    pub fn shows_frame(&self, lemma: &str, complement: &str, sentence: &str) -> bool {
        if matches!(complement, "object" | "clause") {
            return true;
        }
        let words: Vec<String> = sentence
            .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        let lemma = lemma.to_lowercase();
        let mut parts = lemma.split(' ');
        let head = parts.next().unwrap_or("");
        let after: Vec<&str> = parts.chain(complement.split(' ')).collect();
        (0..words.len()).any(|i| {
            self.is_form_of(&words[i], head)
                && words.len() > i + after.len()
                && words[i + 1..=i + after.len()]
                    .iter()
                    .zip(&after)
                    .all(|(w, a)| w == a)
        })
    }

    /// `verdicts` with every sense whose sentence does not show its frame turned to no, and how many
    /// were.
    pub fn require_frame(&self, verdicts: &[Verdict]) -> (Vec<Verdict>, usize) {
        let mut withdrawn = 0;
        let out = verdicts
            .iter()
            .map(|v| {
                let mut v = v.clone();
                for s in &mut v.senses {
                    if s.fits
                        && !self.shows_frame(
                            &v.lemma,
                            &v.complement,
                            s.example.as_deref().unwrap_or(""),
                        )
                    {
                        s.fits = false;
                        withdrawn += 1;
                    }
                }
                v
            })
            .collect();
        (out, withdrawn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(lemma: &str, complement: &str, senses: &[&str], gold: &[&str]) -> Item {
        Item {
            lemma: lemma.into(),
            complement: complement.into(),
            senses: senses
                .iter()
                .map(|o| Sense {
                    offset: o.to_string(),
                    gloss: format!("gloss of {o}"),
                    frames: Vec::new(),
                })
                .collect(),
            gold: gold.iter().map(|g| g.to_string()).collect(),
        }
    }

    fn verdict(lemma: &str, complement: &str, senses: &[(&str, bool, f32)]) -> Verdict {
        Verdict {
            lemma: lemma.into(),
            complement: complement.into(),
            senses: senses
                .iter()
                .map(|(o, fits, confidence)| SenseVerdict {
                    offset: o.to_string(),
                    fits: *fits,
                    confidence: *confidence,
                    example: None,
                })
                .collect(),
            model: "test".into(),
        }
    }

    #[test]
    fn a_sense_is_placed_at_the_threshold_then_below_it_else_the_item_is_a_gap() {
        let items = [
            item("responsible", "for", &["00000004", "00000005"], &[]),
            item("confident", "in", &["00000006", "00000007"], &[]),
            item("good", "of", &["00000008", "00000009"], &[]),
        ];
        let verdicts = [
            verdict(
                "responsible",
                "for",
                &[("00000004", false, 0.9), ("00000005", true, 0.9)],
            ),
            verdict(
                "confident",
                "in",
                &[("00000006", true, 0.75), ("00000007", false, 0.9)],
            ),
            verdict(
                "good",
                "of",
                &[("00000008", false, 0.95), ("00000009", false, 0.9)],
            ),
        ];
        let rows = resolve(&items, &verdicts, ACCEPT).unwrap();
        let by: BTreeMap<&str, &Placement> = rows.iter().map(|r| (r.lemma.as_str(), r)).collect();
        assert_eq!(
            by["responsible"].senses,
            BTreeSet::from(["00000005".to_string()])
        );
        assert_eq!(by["responsible"].placed, Placed::Accepted);
        assert_eq!(
            by["confident"].senses,
            BTreeSet::from(["00000006".to_string()])
        );
        assert_eq!(by["confident"].placed, Placed::BelowThreshold);
        assert!(by["good"].senses.is_empty());
        assert_eq!(by["good"].placed, Placed::Gap);
        let tsv = render_placements(Part::Adjective, &rows, ACCEPT);
        assert!(tsv.contains("responsible\tfor\t00000005\n"));
        assert!(tsv.contains("good\tof\t-\n"));
        let parsed = eigenius_wordnet::governance::Placements::parse(&tsv).unwrap();
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn an_open_item_without_a_verdict_stops_the_resolution() {
        let items = [item("responsible", "for", &["00000004", "00000005"], &[])];
        assert_eq!(
            resolve(&items, &[], ACCEPT).unwrap_err(),
            ["responsible for"]
        );
    }

    #[test]
    fn the_gold_score_counts_senses() {
        let gold = [item(
            "dependent",
            "on",
            &["00000001", "00000002", "00000003"],
            &["00000001"],
        )];
        let verdicts = [verdict(
            "dependent",
            "on",
            &[
                ("00000001", true, 0.95),
                ("00000002", true, 0.9),
                ("00000003", false, 0.9),
            ],
        )];
        let score = score_gold(&gold, &verdicts, ACCEPT);
        assert_eq!(
            (score.true_pos, score.false_pos, score.false_neg),
            (1, 1, 0)
        );
        assert_eq!(score.recall(), 1.0);
        assert_eq!(score.precision(), 0.5);
        assert_eq!(score.disagreements.len(), 1);
    }

    #[test]
    fn the_prompt_hides_the_evidence() {
        let gold = item("dependent", "on", &["00000001", "00000002"], &["00000001"]);
        let p = prompt(Part::Adjective, Wording::Shared, &[&gold]);
        assert!(p.contains("[0] adjective: dependent   preposition: on"));
        assert!(p.contains("(1) gloss of 00000002"));
        assert!(!p.contains("gold") && !p.contains("evidence"));
    }

    #[test]
    fn a_sentence_shows_the_frame_where_the_preposition_follows_the_verb() {
        let exc = ExcLists::parse("", "flew fly\ngave give\nreferred refer\n", "", "");
        let verbs: BTreeMap<Offset, Synset> = ["fly", "refer", "give", "give rise", "dance"]
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let s = Synset {
                    offset: format!("{:08}", i + 1),
                    pos: eigenius_wordnet::wndb::Pos::Verb,
                    words: vec![w.to_string()],
                    gloss: String::new(),
                    hypernyms: vec![],
                    instance_of: vec![],
                    frames: vec![],
                    relational: false,
                    derivational: BTreeSet::new(),
                };
                (s.offset.clone(), s)
            })
            .collect();
        let f = VerbForms::new(exc, &verbs);
        assert!(f.shows_frame("fly", "at", "She flew at him in a fury."));
        assert!(f.shows_frame("refer", "to", "She referred to the report."));
        assert!(!f.shows_frame(
            "refer",
            "to",
            "The doctor referred the patient to a specialist."
        ));
        assert!(f.shows_frame("give rise", "to", "Mutations gave rise to resistance."));
        assert!(!f.shows_frame("dance", "with", "She danced gracefully with him."));
        assert!(f.shows_frame("refer", "object", "anything"));
        let v = Verdict {
            lemma: "refer".into(),
            complement: "to".into(),
            senses: vec![SenseVerdict {
                offset: "00000002".into(),
                fits: true,
                confidence: 0.9,
                example: Some("He referred the case to a court.".into()),
            }],
            model: "test".into(),
        };
        let (out, withdrawn) = f.require_frame(&[v]);
        assert_eq!(withdrawn, 1);
        assert!(out[0].accepted(0.0).is_empty());
    }

    #[test]
    fn a_row_echoes_its_lemma_with_or_without_what_the_item_adds() {
        assert!(echoes("Come", "come"));
        assert!(echoes("come [before]", "come"));
        assert!(echoes("give rise", "give rise"));
        assert!(!echoes("comes", "come"));
        assert!(!echoes("go", "come"));
    }

    #[test]
    fn a_union_accepts_a_sense_any_run_accepts() {
        let a = verdict(
            "bend",
            "object",
            &[("00000001", true, 0.9), ("00000002", false, 0.9)],
        );
        let b = verdict(
            "bend",
            "object",
            &[("00000001", false, 0.6), ("00000002", true, 0.86)],
        );
        let u = union(&[vec![a], vec![b]]);
        assert_eq!(u.len(), 1);
        assert_eq!(
            u[0].accepted(ACCEPT),
            BTreeSet::from(["00000001".to_string(), "00000002".to_string()])
        );
    }

    #[test]
    fn a_batch_holds_one_kind_of_complement() {
        let items = [
            item("respond", "to", &["00000001"], &[]),
            item("report", "clause", &["00000002"], &[]),
            item("depend", "on", &["00000003"], &[]),
            item("bend", "object", &["00000004"], &[]),
        ];
        let batched = batches(&items, 8);
        let kinds: Vec<Vec<&str>> = batched
            .iter()
            .map(|b| b.iter().map(|i| i.complement.as_str()).collect())
            .collect();
        assert_eq!(kinds, [vec!["clause"], vec!["object"], vec!["to", "on"]]);
    }

    #[test]
    fn the_per_kind_wording_names_its_kind_only() {
        let bend = item("bend", "object", &["00000004"], &[]);
        let p = prompt(Part::Verb, Wording::PerKind, &[&bend]);
        assert!(p.contains("«The sun thawed the ice»"));
        assert!(!p.contains("that-clause stating"));
        let report = item("report", "clause", &["00000002"], &[]);
        let p = prompt(Part::Verb, Wording::PerKind, &[&report]);
        assert!(p.contains("«She noticed that the stain had spread»"));
        assert!(!p.contains("thawed"));
    }

    #[test]
    fn the_verb_prompt_names_the_complement_and_the_shown_patterns() {
        let mut respond = item("respond", "to", &["00000001", "00000002"], &["00000001"]);
        respond.senses[0].frames = vec!["Somebody ----s".into()];
        let incubate = item("incubate", "object", &["00000003", "00000004"], &[]);
        let p = prompt(Part::Verb, Wording::Shared, &[&respond, &incubate]);
        assert!(p.contains(
            "[0] verb: respond   complement: `to` + noun phrase   pattern: respond to <noun phrase>"
        ));
        assert!(p.contains("       patterns: Somebody ----s\n"));
        assert!(p.contains("[1] verb: incubate   complement: a direct object"));
        assert!(!p.contains("gold") && !p.contains("evidence"));
    }
}
