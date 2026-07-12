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

//! **Cross-lexicon concept unification** (D63,
//! `docs/notes/d63-wordnet-umls-concept-unification.md`): find the places where WordNet and UMLS
//! name the *same concept*, so the lexicon can denote **one** concept instead of two.
//!
//! Why it matters, measured rather than assumed. WordNet and UMLS each mint their own class for a
//! shared meaning — `state` is `wn:n00024720` **and** `umlscui:C1442792`, with **verbatim-identical
//! glosses**. The parser builds a reading for each; they are not `Exp`-equal (the IRIs differ), so
//! nothing collapses them and they *multiply*. Over the WRN page (`experiments/parsing`,
//! 2026-07-11) **47% of ranked words spent BOTH `SENSE_CAP` slots on such a cross-lexicon pair** —
//! so no genuine alternative sense could seed at all.
//!
//! This crate is the **deterministic half**: it generates the candidate pairs and extracts a
//! high-confidence *gold* subset. It calls no model. The adjudicator (does one concept underlie
//! both glosses?) is judged against that gold set before it is trusted on anything else.

pub mod adjudicate;
pub mod emit;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One (UMLS concept, WordNet synset) pair sharing a surface form, with everything an adjudicator
/// needs to decide whether they are the same concept — and the features to score it with.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    /// The surface string both sides spell (lowercased).
    pub surface: String,
    /// UMLS concept id.
    pub cui: String,
    /// UMLS definition (`MRDEF`).
    pub umls_gloss: String,
    /// UMLS semantic type(s) (`MRSTY` TUI).
    pub tuis: Vec<String>,
    /// WordNet synset offset (noun).
    pub offset: String,
    /// WordNet gloss.
    pub wn_gloss: String,
    /// Token Jaccard of the two normalized glosses. **The gold signal, not the verdict**: a high
    /// value means the two definitions are worded alike, which is near-conclusive for *same*; a low
    /// value is NOT evidence of *different* — it may just be different wording (`congenital
    /// abnormality` scores 0.0 against WordNet's phrasing and is plainly the same concept). This is
    /// exactly why the adjudicator exists.
    pub gloss_jaccard: f32,
}

/// Normalize a gloss to a content-word token set: lowercase, drop parentheticals (WordNet's
/// `(genetics)` topic prefixes), drop punctuation and short function words.
pub fn gloss_tokens(g: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut depth = 0i32;
    let mut word = String::new();
    for c in g.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            c if c.is_ascii_alphanumeric() => word.push(c.to_ascii_lowercase()),
            _ => {
                if word.len() > 2 {
                    out.insert(std::mem::take(&mut word));
                } else {
                    word.clear();
                }
            }
        }
    }
    if word.len() > 2 {
        out.insert(word);
    }
    out
}

/// Token Jaccard of two gloss token sets; `0.0` if either is too short to judge.
pub fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.len() < 4 || b.len() < 4 {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Generate every candidate pair: a surface string that is BOTH a UMLS concept's English atom and a
/// WordNet noun lemma, where **both sides carry a gloss** (without two glosses there is nothing to
/// adjudicate).
///
/// **No pre-filter on semantic type.** Requiring the UMLS TUI and the WordNet supersense to agree
/// was measured (5-fold cross-validated, 2026-07-11) and is too lossy to be a gate: keeping 93% of
/// known duplicates removes only 23% of the work, and cutting 61% of the work **discards a quarter
/// of the duplicates** — silently, and a dropped duplicate is one that is never merged. The TUI is
/// carried on the candidate as a *feature* for the adjudicator to weigh, never as a filter.
///
/// **Gloss coverage is the real bound**, and it is narrower than it looks: only ~10.6% of UMLS CUIs
/// have an `MRDEF` definition. That is not fatal — every duplicate witnessed in the corpus is
/// glossed (`events`, `genes`, `DNA repair`, `cell death`). Prose uses the well-described concepts;
/// the un-glossed 89% is a long tail of source-specific codes that never surface in text.
pub fn candidates(meta: &Path, dict: &Path) -> std::io::Result<Vec<Candidate>> {
    use eigenius_umls::rrf::{parse_mrconso_line, parse_mrdef_line, parse_mrsty_line};
    use eigenius_wordnet::wndb::{read_data_file, Pos};

    // UMLS: CUI → definition (first, unsuppressed).
    let mut cui_gloss: BTreeMap<String, String> = BTreeMap::new();
    for line in std::fs::read_to_string(meta.join("MRDEF.RRF"))?.lines() {
        if let Some(d) = parse_mrdef_line(line) {
            if d.suppress == "N" && !d.def.is_empty() {
                cui_gloss.entry(d.cui).or_insert(d.def);
            }
        }
    }

    // …their semantic types (a feature, not a filter).
    let mut cui_tuis: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in std::fs::read_to_string(meta.join("MRSTY.RRF"))?.lines() {
        if let Some(s) = parse_mrsty_line(line) {
            if cui_gloss.contains_key(&s.cui) {
                cui_tuis.entry(s.cui).or_default().push(s.tui);
            }
        }
    }

    // WordNet nouns: lemma → [(offset, gloss)].
    //
    // **INSTANCE synsets are excluded.** A synset with an `@i` (instance-hypernym) pointer is a
    // proper-noun individual — `Africa`, `Alabama` — and the importer emits it as a `resource`, not
    // a `class`. A lexical entry's category is `cat_n(C, num)` where `C : Set`, so pointing an entry
    // at an individual is a TYPE ERROR: the individual's type is its class (`EigonClass(…)`), not
    // `Sort(1)`. The kernel validator rejects it (`TypeExprIllTyped`), which is exactly how this was
    // caught — 405 such merges produced 721 violations on the first load attempt. The symmetric
    // exclusion on the UMLS side is the `cat_np` skip in [`crate::emit`].
    let nouns = read_data_file(&dict.join(Pos::Noun.data_file()))?;
    let mut by_lemma: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (off, syn) in &nouns {
        if syn.gloss.is_empty() || !syn.instance_of.is_empty() {
            continue;
        }
        for w in &syn.words {
            by_lemma
                .entry(w.to_lowercase())
                .or_default()
                .push((off.clone(), syn.gloss.clone()));
        }
    }

    // Join on the surface. Stream MRCONSO (2.3 GB) rather than materialize it.
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let conso = std::fs::read_to_string(meta.join("MRCONSO.RRF"))?;
    for line in conso.lines() {
        let Some(a) = parse_mrconso_line(line) else {
            continue;
        };
        if a.lat != "ENG" || a.suppress != "N" {
            continue;
        }
        let Some(ug) = cui_gloss.get(&a.cui) else {
            continue;
        };
        let surface = a.str_.to_lowercase();
        let Some(syns) = by_lemma.get(&surface) else {
            continue;
        };
        let ut = gloss_tokens(ug);
        for (off, wg) in syns {
            if !seen.insert((a.cui.clone(), off.clone())) {
                continue;
            }
            out.push(Candidate {
                surface: surface.clone(),
                cui: a.cui.clone(),
                umls_gloss: ug.clone(),
                tuis: cui_tuis.get(&a.cui).cloned().unwrap_or_default(),
                offset: off.clone(),
                wn_gloss: wg.clone(),
                gloss_jaccard: jaccard(&ut, &gloss_tokens(wg)),
            });
        }
    }
    out.sort_by(|a, b| (&a.cui, &a.offset).cmp(&(&b.cui, &b.offset)));
    Ok(out)
}

/// The **gold** threshold: same surface AND normalized-gloss token Jaccard ≥ this ⇒ near-certainly
/// the same concept. Used to *validate the adjudicator*, not to do the alignment — it is far too
/// strict to find the duplicates that matter (`congenital abnormality` scores 0.0 and is the same
/// concept), and precision at this threshold is what makes it a usable answer key.
pub const GOLD_JACCARD: f32 = 0.75;

/// The gold subset of `cands` (see [`GOLD_JACCARD`]).
pub fn gold(cands: &[Candidate]) -> Vec<&Candidate> {
    cands
        .iter()
        .filter(|c| c.gloss_jaccard >= GOLD_JACCARD)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gloss_tokens_drops_parentheticals_and_short_words() {
        // WordNet's `(genetics)` topic prefix must not count as content, or every genetics gloss
        // looks alike.
        let t = gloss_tokens("(genetics) a segment of DNA on a chromosome");
        assert!(!t.contains("genetics"), "parenthetical dropped");
        assert!(t.contains("segment") && t.contains("chromosome"));
        assert!(
            !t.contains("of") && !t.contains("on"),
            "short words dropped"
        );
    }

    #[test]
    fn jaccard_scores_the_real_state_pair_as_gold() {
        // The pair that motivates the whole exercise — UMLS C1442792 vs WordNet n00024720.
        let umls = gloss_tokens("The way something is with respect to its main attributes.");
        let wn = gloss_tokens("the way something is with respect to its main attributes");
        assert!(
            jaccard(&umls, &wn) >= GOLD_JACCARD,
            "verbatim-identical glosses must land in the gold set"
        );
    }

    #[test]
    fn jaccard_is_not_evidence_of_difference() {
        // The load-bearing caveat: a LOW score means "worded differently", NOT "different concept".
        // `congenital abnormality` is the same concept in both and shares almost no wording — which
        // is precisely why an adjudicator is needed instead of a threshold.
        let umls = gloss_tokens("An abnormality present at birth.");
        let wn = gloss_tokens("a physical abnormality existing from birth or before birth");
        let j = jaccard(&umls, &wn);
        assert!(
            j < GOLD_JACCARD,
            "same concept, different wording, low score"
        );
    }

    #[test]
    fn a_short_gloss_is_never_gold() {
        // Too little text to judge — do not let a two-word gloss score 1.0 by accident.
        let a = gloss_tokens("a cell");
        let b = gloss_tokens("a cell");
        assert_eq!(jaccard(&a, &b), 0.0);
    }
}
