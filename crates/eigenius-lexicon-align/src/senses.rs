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

//! **The adjective sense judge** (D97 decision 7; eigenius#263) — which senses of an adjective take
//! the preposition SPECIALIST or the curated frames name for the lemma.
//!
//! SPECIALIST speaks for a lemma; the importer emits per WordNet sense. Where the evidence does not
//! decide — the lemma has several gradable senses, and the derivational pointers and WordNet's
//! convention pick all of them or none — the judge reads each sense's gloss and says whether the
//! adjective, in that sense, takes `<preposition> + noun phrase`.
//!
//! It follows the WordNet–UMLS alignment's protocol ([`crate::adjudicate`]):
//! - its verdicts are **recorded** (`adjective-senses.jsonl`), never taken at build time;
//! - it is **scored first** on the items the evidence decides, with the evidence hidden
//!   ([`score_gold`]): below 95% recall it is not trusted;
//! - a sense is placed at confidence ≥ 0.85; where the judge places none there, the senses it said
//!   yes to below it; where it said no to every sense, nothing, and the item is recorded as a gap —
//!   a use WordNet has no sense for, as the evaluative `it was good of you` (decided 2026-09-30) —
//!   [`resolve`] writes the placements the importer reads (`adjective-senses.tsv`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use eigenius_wordnet::governance::{classify, convention_prepositions, Classified};
use eigenius_wordnet::wndb::{read_data_file, Offset, Synset};
use serde::{Deserialize, Serialize};

/// A sense's confidence at or above which the judge places it.
pub const ACCEPT: f32 = 0.85;
/// The gold recall below which the judge is not trusted.
pub const MIN_RECALL: f64 = 0.95;

/// One sense, as the judge sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sense {
    pub offset: Offset,
    /// WordNet's gloss, its examples included.
    pub gloss: String,
}

/// A (lemma, preposition) whose senses the judge decides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub lemma: String,
    pub preposition: String,
    pub senses: Vec<Sense>,
    /// For a gold item, the senses the evidence placed; empty otherwise.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub gold: BTreeSet<Offset>,
}

/// The judge's inputs, read once.
pub struct Sources {
    pub adjectives: BTreeMap<Offset, Synset>,
    pub classified: Classified,
}

/// Read WordNet's adjectives and nouns and SPECIALIST, and classify every attested preposition.
pub fn load(dict: &Path, specialist: &Path) -> Result<Sources, String> {
    let read = |pos: &str| {
        let path = dict.join(format!("data.{pos}"));
        read_data_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    };
    let adjectives = read("adj")?;
    let lexicon = eigenius_specialist::Lexicon::read(specialist)
        .map_err(|e| format!("{}: {e}", specialist.display()))?;
    let classified = classify(
        &adjectives,
        Some(&lexicon),
        eigenius_wordnet::convert::adjective_frames(),
    );
    Ok(Sources {
        adjectives,
        classified,
    })
}

impl Sources {
    fn senses(&self, offsets: &[Offset]) -> Vec<Sense> {
        offsets
            .iter()
            .map(|o| Sense {
                offset: o.clone(),
                gloss: self.adjectives[o].gloss.clone(),
            })
            .collect()
    }

    /// The items on several senses: the judge's work.
    pub fn open_items(&self) -> Vec<Item> {
        self.classified
            .open
            .iter()
            .map(|o| Item {
                lemma: o.lemma.clone(),
                preposition: o.preposition.clone(),
                senses: self.senses(&o.senses),
                gold: BTreeSet::new(),
            })
            .collect()
    }

    /// The judge's gold: the open items with a sense whose own gloss names the preposition in
    /// WordNet's convention (``followed by `on'``) — a per-sense fact the judge must recover.
    pub fn gold_items(&self) -> Vec<Item> {
        self.open_items()
            .into_iter()
            .filter_map(|mut item| {
                item.gold = item
                    .senses
                    .iter()
                    .filter(|s| convention_prepositions(&s.gloss).contains(&item.preposition))
                    .map(|s| s.offset.clone())
                    .collect();
                (!item.gold.is_empty()).then_some(item)
            })
            .collect()
    }
}

/// One sense's verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SenseVerdict {
    pub offset: Offset,
    /// In this sense, does the adjective take `<preposition> + noun phrase`?
    pub fits: bool,
    /// 0–1.
    pub confidence: f32,
}

/// One item's recorded verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub lemma: String,
    pub preposition: String,
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
    /// Items not in the verdicts.
    pub missing: usize,
    /// `lemma preposition sense: gold / judge` for every disagreement — read these; they are where
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

/// Score `verdicts` against the gold items: a sense the evidence placed and the judge accepts at
/// `threshold` is a true positive.
pub fn score_gold(gold: &[Item], verdicts: &[Verdict], threshold: f32) -> GoldScore {
    let by_key: BTreeMap<(&str, &str), &Verdict> = verdicts
        .iter()
        .map(|v| ((v.lemma.as_str(), v.preposition.as_str()), v))
        .collect();
    let mut score = GoldScore::default();
    for item in gold {
        let Some(v) = by_key.get(&(item.lemma.as_str(), item.preposition.as_str())) else {
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
                (false, false) => {}
            }
            if g != j {
                score.disagreements.push(format!(
                    "{} {} {}: evidence {}, judge {} — {}",
                    item.lemma,
                    item.preposition,
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
    pub preposition: String,
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
    let by_key: BTreeMap<(&str, &str), &Verdict> = verdicts
        .iter()
        .map(|v| ((v.lemma.as_str(), v.preposition.as_str()), v))
        .collect();
    let mut out = Vec::new();
    let mut missing = Vec::new();
    for item in items {
        let Some(v) = by_key.get(&(item.lemma.as_str(), item.preposition.as_str())) else {
            missing.push(format!("{} {}", item.lemma, item.preposition));
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
            preposition: item.preposition.clone(),
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

/// The placements file the importer reads: `lemma <TAB> preposition <TAB> offset,…`, or `-` for a
/// gap.
pub fn render_placements(rows: &[Placement], threshold: f32) -> String {
    let count = |p: Placed| rows.iter().filter(|r| r.placed == p).count();
    let mut out = format!(
        "# The adjective sense judge's placements (D97 decision 7; eigenius#263) — generated by\n\
         # `specialist-senses resolve` from adjective-senses.jsonl; do not edit by hand.\n\
         # {} items: {} on the senses accepted at confidence >= {threshold}; {} on the senses the judge\n\
         # said yes to below it; {} gaps (`-`), where it said no to every sense — a use WordNet has no\n\
         # sense for (decided 2026-09-30).\n\
         # lemma <TAB> preposition <TAB> WordNet adjective offsets\n",
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
        out.push_str(&format!("{}\t{}\t{senses}\n", r.lemma, r.preposition));
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

#[cfg(feature = "use-llm")]
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ItemRow {
    /// The item's index in the batch (0-based).
    pub index: usize,
    /// One row per sense of the item.
    pub senses: Vec<SenseRow>,
}

#[cfg(feature = "use-llm")]
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SenseRow {
    /// The sense's index within its item (0-based).
    pub index: usize,
    /// In this sense, does the adjective take `<preposition> + noun phrase` as its complement?
    pub fits: bool,
    /// 0.0–1.0.
    pub confidence: f32,
}

/// The prompt for one batch of items. The evidence is never shown: a gold item looks like any other.
pub fn prompt(batch: &[&Item]) -> String {
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
            item.lemma, item.preposition, item.lemma, item.preposition
        ));
        for (j, s) in item.senses.iter().enumerate() {
            p.push_str(&format!("   ({j}) {}\n", s.gloss.trim()));
        }
        p.push('\n');
    }
    p.push_str(
        "Return one row per item, with its index, and in it one verdict per sense, with the \
         sense's index.",
    );
    p
}

/// Judge one batch. `Err` on any transport, API or decode failure, and on a reply that does not
/// answer every item and every sense exactly once — the caller records nothing for the batch.
#[cfg(feature = "use-llm")]
pub async fn judge_batch(
    api_key: &str,
    model: &str,
    batch: &[&Item],
) -> Result<Vec<Verdict>, String> {
    let reply: BatchReply = eigenius_kernel::dcg::anthropic_client::anthropic_structured(
        api_key,
        &eigenius_kernel::dcg::anthropic_client::ModelConfig::with_model(model),
        &prompt(batch),
    )
    .await?;
    let mut rows: BTreeMap<usize, &ItemRow> = BTreeMap::new();
    for r in &reply.items {
        if r.index >= batch.len() || rows.insert(r.index, r).is_some() {
            return Err(format!("item index {} out of range or repeated", r.index));
        }
    }
    let mut out = Vec::with_capacity(batch.len());
    for (i, item) in batch.iter().enumerate() {
        let row = rows.get(&i).ok_or_else(|| {
            format!(
                "no verdict for item {i} ({} {})",
                item.lemma, item.preposition
            )
        })?;
        let mut senses: BTreeMap<usize, &SenseRow> = BTreeMap::new();
        for s in &row.senses {
            if s.index >= item.senses.len() || senses.insert(s.index, s).is_some() {
                return Err(format!(
                    "{} {}: sense index {} out of range or repeated",
                    item.lemma, item.preposition, s.index
                ));
            }
        }
        if senses.len() != item.senses.len() {
            return Err(format!(
                "{} {}: {} of {} senses answered",
                item.lemma,
                item.preposition,
                senses.len(),
                item.senses.len()
            ));
        }
        out.push(Verdict {
            lemma: item.lemma.clone(),
            preposition: item.preposition.clone(),
            senses: item
                .senses
                .iter()
                .enumerate()
                .map(|(j, s)| SenseVerdict {
                    offset: s.offset.clone(),
                    fits: senses[&j].fits,
                    confidence: senses[&j].confidence.clamp(0.0, 1.0),
                })
                .collect(),
            model: model.to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(lemma: &str, prep: &str, senses: &[&str], gold: &[&str]) -> Item {
        Item {
            lemma: lemma.into(),
            preposition: prep.into(),
            senses: senses
                .iter()
                .map(|o| Sense {
                    offset: o.to_string(),
                    gloss: format!("gloss of {o}"),
                })
                .collect(),
            gold: gold.iter().map(|g| g.to_string()).collect(),
        }
    }

    fn verdict(lemma: &str, prep: &str, senses: &[(&str, bool, f32)]) -> Verdict {
        Verdict {
            lemma: lemma.into(),
            preposition: prep.into(),
            senses: senses
                .iter()
                .map(|(o, fits, confidence)| SenseVerdict {
                    offset: o.to_string(),
                    fits: *fits,
                    confidence: *confidence,
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
        let tsv = render_placements(&rows, ACCEPT);
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
        let p = prompt(&[&gold]);
        assert!(p.contains("[0] adjective: dependent   preposition: on"));
        assert!(p.contains("(1) gloss of 00000002"));
        assert!(!p.contains("gold") && !p.contains("evidence"));
    }
}
