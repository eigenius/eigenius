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
pub mod drops;
pub mod emit;
pub mod merge;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One (UMLS concept, WordNet synset) pair sharing a surface form, with everything an adjudicator
/// needs to decide whether they are the same concept — and the features to score it with.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Candidate {
    /// The surface string both sides spell (lowercased).
    pub surface: String,
    /// UMLS concept id.
    pub cui: String,
    /// UMLS definition (`MRDEF`) — **empty for 40% of the CANDIDATES here (40,736 of 102,292), and
    /// for 89% of the Metathesaurus at large** (267,162 of 2,509,295 CUIs carry one; measured
    /// 2026-07-29 against 2026AA) — which is why the fields below
    /// exist. UMLS simply never wrote a definition for `Deficiency` (C0011155), whose surface *is* a
    /// WordNet lemma (`lack / deficiency / want`); requiring a gloss silently excluded it, and with
    /// it the whole Functional/Qualitative-Concept bucket — precisely the abstract nouns that overlap
    /// WordNet most.
    pub umls_gloss: String,
    /// UMLS preferred name (`MTH/PN`, else `*/PT`). Present for every concept.
    #[serde(default)]
    pub umls_name: String,
    /// A few distinctive atoms as `TTY|STR`. The **fully-specified names** carry structure a gloss
    /// does not: `Deficiency (attribute)`, `Deficient (qualifier value)` say *what kind of thing*
    /// the concept is — which is how a real merge (`Deficiency`) is told apart from a metadata
    /// artefact (`Specialty Type - cancer`, a *discipline*, competing with the disease `cancer`).
    #[serde(default)]
    pub umls_atoms: Vec<String>,
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
    use eigenius_wordnet::morphy::{morphstr, ExcLists, LemmaSet};
    use eigenius_wordnet::wndb::{read_data_file, Pos};

    // UMLS definitions — present for only ~10.6% of concepts.
    let mut cui_gloss: BTreeMap<String, String> = BTreeMap::new();
    for line in std::fs::read_to_string(meta.join("MRDEF.RRF"))?.lines() {
        if let Some(d) = parse_mrdef_line(line) {
            if d.suppress == "N" && !d.def.is_empty() {
                cui_gloss.entry(d.cui).or_insert(d.def);
            }
        }
    }

    // Semantic types, for EVERY concept (not just the glossed ones) — for an un-glossed concept the
    // type is a large part of what identifies it.
    let mut cui_tuis: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in std::fs::read_to_string(meta.join("MRSTY.RRF"))?.lines() {
        if let Some(s) = parse_mrsty_line(line) {
            cui_tuis.entry(s.cui).or_default().push(s.sty);
        }
    }

    // WordNet nouns: lemma → [(offset, gloss)]. INSTANCE synsets (`@i`) are excluded — the importer
    // emits them as a `resource`, not a `class`, and an entry's `cat_n(C, num)` requires `C : Set`.
    // Pointing an entry at an individual is a type error the kernel validator rejects.
    let exc = ExcLists::load(dict)?;
    let nouns = read_data_file(&dict.join(Pos::Noun.data_file()))?;
    let lemma_set = LemmaSet::from_synsets([&nouns]);
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

    // One pass over MRCONSO: collect, per concept that shares a surface with a WordNet noun, its
    // surfaces + its atoms (`TTY|STR`) + its preferred name.
    struct Acc {
        name: String,
        atoms: Vec<String>,
        surfaces: BTreeSet<String>,
    }
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
    let conso = std::fs::read_to_string(meta.join("MRCONSO.RRF"))?;
    for line in conso.lines() {
        let Some(a) = parse_mrconso_line(line) else {
            continue;
        };
        if a.lat != "ENG" || a.suppress != "N" {
            continue;
        }
        let surface = a.str_.to_lowercase();
        // **Lemmatize before matching.** WordNet lemmas are SINGULAR; UMLS mints atoms for the
        // plural too (`Genes`, `Tumours`, `Therapies`). Matching the surface verbatim meant those
        // atoms never became candidates — so the plural ENTRY was never merged, even though the
        // singular of the SAME concept was, and it kept denoting `umlscui:…` and generating a
        // separate reading. Witnessed on the WRN page: `genes` had THREE competing senses
        // (n05436752 + C0017337 + junk) and `Thus, MSI tumours need novel therapies` had 8 readings
        // over ONE skeleton — pure sense ambiguity, entirely manufactured by unmerged plurals.
        //
        // The candidate keeps the ORIGINAL surface (the emitter finds the entry by `(cui, form)`,
        // and the entry's form IS the plural); only the WordNet lookup uses the lemma.
        let hits_wn = by_lemma.contains_key(&surface)
            || morphstr(&surface, Pos::Noun, &exc, &lemma_set)
                .iter()
                .any(|l| by_lemma.contains_key(&l.to_lowercase()));
        let e = acc.entry(a.cui.clone()).or_insert_with(|| Acc {
            name: String::new(),
            atoms: Vec::new(),
            surfaces: BTreeSet::new(),
        });
        // Preferred name: MTH/PN wins, else the first PT.
        if e.name.is_empty() && (a.tty == "PN" || a.tty == "PT") {
            e.name = a.str_.clone();
        }
        // Keep a handful of atoms, favouring the fully-specified names — they carry the structure.
        if e.atoms.len() < 8 {
            let row = format!("{}|{}", a.tty, a.str_);
            if !e.atoms.contains(&row) {
                e.atoms.push(row);
            }
        }
        if hits_wn {
            e.surfaces.insert(surface);
        }
    }

    let mut out: Vec<Candidate> = Vec::new();
    for (cui, e) in &acc {
        if e.surfaces.is_empty() {
            continue; // shares no surface with a WordNet noun
        }
        // Not every concept carries a `PN`/`PT` atom — fall back to the first atom's string rather
        // than handing the adjudicator an empty name.
        // Not every concept carries a `PN`/`PT` atom. Fall back to the first atom's string, and
        // failing that the surface itself — never hand the adjudicator an empty name.
        let name = if !e.name.is_empty() {
            e.name.clone()
        } else if let Some((_, s)) = e.atoms.first().and_then(|a| a.split_once('|')) {
            s.to_string()
        } else {
            String::new()
        };
        let ug = cui_gloss.get(cui).cloned().unwrap_or_default();
        let ut = gloss_tokens(&ug);
        for surface in &e.surfaces {
            // Resolve the WordNet synsets under the surface OR its lemma (see above).
            let syns: &Vec<(String, String)> = match by_lemma.get(surface) {
                Some(v) => v,
                None => {
                    let Some(v) = morphstr(surface, Pos::Noun, &exc, &lemma_set)
                        .iter()
                        .find_map(|l| by_lemma.get(&l.to_lowercase()))
                    else {
                        continue;
                    };
                    v
                }
            };
            for (off, wg) in syns {
                out.push(Candidate {
                    surface: surface.clone(),
                    cui: cui.clone(),
                    umls_gloss: ug.clone(),
                    umls_name: if name.is_empty() {
                        surface.clone()
                    } else {
                        name.clone()
                    },
                    umls_atoms: e.atoms.clone(),
                    tuis: cui_tuis.get(cui).cloned().unwrap_or_default(),
                    offset: off.clone(),
                    wn_gloss: wg.clone(),
                    gloss_jaccard: if ug.is_empty() {
                        0.0
                    } else {
                        jaccard(&ut, &gloss_tokens(wg))
                    },
                });
            }
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
