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

//! **Verdicts → the merge set.** Deterministic; no LLM. This is where the adjudicator's raw
//! `same/confidence` rows become the table the emitter rewrites entries from.
//!
//! Three rules, and each one exists because it was got wrong first:
//!
//! **1. A verdict is about `(cui, synset)`. The surface is only how the pair was found.**
//! `C0017337` ("Gene") and WordNet `n05436752` name the same concept *whatever string led you
//! there* — so the one verdict licenses the merge for **every** surface of that concept: `gene`
//! **and** `genes`. Keying merges on the surface the adjudicator happened to see silently dropped
//! every plural: the chain holds `e_C0017337_0` = "Genes" *and* `e_C0017337_1` = "Gene", and only
//! the second was ever rewritten. Expanding one verdict across the concept's surfaces took the
//! merge set from 26 690 to 38 397 for zero extra API spend.
//!
//! **2. Merge only at `confidence ≥ 0.85`** ([`MERGE_CONFIDENCE`]). The precision probe found a
//! real false merge below it (`attachment`: an *email* attachment vs *a supplementary part*). The
//! model proposed nothing at all below 0.70, so its own uncertainty is the usable signal.
//!
//! **3. One entry, one class — ties are DROPPED.** A `(cui, surface)` proposed for two different
//! synsets is resolved by highest confidence; an exact tie is *dropped*. With no basis to choose,
//! prefer to miss: a missed merge changes nothing, a wrong one points a word at the wrong concept.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::adjudicate::Verdict;
use crate::Candidate;

/// The confidence at or above which a `same=true` verdict becomes a merge. See rule 2.
pub const MERGE_CONFIDENCE: f32 = 0.85;

/// One row of `merges.json` — the emitter's input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Merge {
    pub cui: String,
    pub offset: String,
    /// Lowercased. The emitter matches it against the committed entry's `lexicon:form`.
    pub surface: String,
    pub confidence: f32,
}

/// What [`resolve`] did, so the pipeline can print it instead of guessing.
#[derive(Debug, Default, PartialEq)]
pub struct MergeStats {
    /// Distinct `(cui, synset)` pairs the adjudicator accepted at or above the threshold.
    pub accepted_concept_pairs: usize,
    /// `(cui, surface)` keys dropped for an exact confidence tie between two synsets (rule 3).
    pub ties_dropped: usize,
    /// Candidate pairs with no verdict at all — the adjudicator never returned one (it fails
    /// closed, so this is a real, visible gap, not a silent "different").
    pub unjudged: usize,
}

/// Turn the adjudicator's verdicts into the merge set.
///
/// `candidates` supplies the surfaces: every surface that ever produced the pair `(cui, offset)`
/// inherits that pair's verdict (rule 1).
pub fn resolve(candidates: &[Candidate], verdicts: &[Verdict]) -> (Vec<Merge>, MergeStats) {
    // Rule 1: index the verdict by the CONCEPT PAIR, not by the surface that surfaced it.
    let mut by_pair: BTreeMap<(&str, &str), &Verdict> = BTreeMap::new();
    for v in verdicts {
        let key = (v.cui.as_str(), v.offset.as_str());
        // A pair re-judged (a resumed run, a reprompt) keeps the most confident verdict.
        match by_pair.get(&key) {
            Some(prev) if prev.confidence >= v.confidence => {}
            _ => {
                by_pair.insert(key, v);
            }
        }
    }

    let mut stats = MergeStats {
        accepted_concept_pairs: by_pair
            .values()
            .filter(|v| v.same && v.confidence >= MERGE_CONFIDENCE)
            .count(),
        ..Default::default()
    };

    // Rule 3: a (cui, surface) may be claimed by several synsets. Keep the most confident; drop
    // exact ties. `Option<..>` = poisoned by a tie.
    let mut best: BTreeMap<(String, String), Option<(f32, String)>> = BTreeMap::new();

    for c in candidates {
        let Some(v) = by_pair.get(&(c.cui.as_str(), c.offset.as_str())) else {
            stats.unjudged += 1;
            continue;
        };
        if !v.same || v.confidence < MERGE_CONFIDENCE {
            continue;
        }
        let key = (c.cui.clone(), c.surface.to_lowercase());
        match best.get(&key) {
            // Same synset reached by two candidate rows — not a conflict, just the same merge.
            Some(Some((_, off))) if *off == c.offset => {}
            Some(Some((conf, _))) if *conf > v.confidence => {}
            Some(Some((conf, _))) if (*conf - v.confidence).abs() < f32::EPSILON => {
                best.insert(key, None); // tie between two synsets — drop, prefer to miss
            }
            Some(None) => {} // already poisoned
            _ => {
                best.insert(key, Some((v.confidence, c.offset.clone())));
            }
        }
    }

    let mut out = Vec::new();
    for ((cui, surface), slot) in best {
        match slot {
            Some((confidence, offset)) => out.push(Merge {
                cui,
                offset,
                surface,
                confidence,
            }),
            None => stats.ties_dropped += 1,
        }
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(surface: &str, cui: &str, offset: &str) -> Candidate {
        Candidate {
            surface: surface.into(),
            cui: cui.into(),
            offset: offset.into(),
            ..Default::default()
        }
    }
    fn verdict(cui: &str, offset: &str, surface: &str, same: bool, confidence: f32) -> Verdict {
        Verdict {
            cui: cui.into(),
            offset: offset.into(),
            surface: surface.into(),
            same,
            confidence,
            reason: String::new(),
        }
    }

    /// Rule 1, and the bug that motivated it: the adjudicator judged the pair once, having seen
    /// only the singular. The plural surface of the *same concept* must merge too — the chain holds
    /// a separate entry for it (`e_C0017337_0` = "Genes").
    #[test]
    fn one_verdict_licenses_every_surface_of_the_concept() {
        let cands = [
            cand("gene", "C0017337", "05436752"),
            cand("genes", "C0017337", "05436752"),
        ];
        let verdicts = [verdict("C0017337", "05436752", "gene", true, 0.95)];
        let (merges, stats) = resolve(&cands, &verdicts);

        let surfaces: Vec<&str> = merges.iter().map(|m| m.surface.as_str()).collect();
        assert_eq!(surfaces, ["gene", "genes"]);
        // One CONCEPT pair was accepted; it produced two entry rewrites.
        assert_eq!(stats.accepted_concept_pairs, 1);
        assert_eq!(stats.unjudged, 0);
    }

    /// Rule 2: below the threshold nothing merges, however emphatic the `same`.
    #[test]
    fn a_verdict_below_the_confidence_threshold_does_not_merge() {
        let cands = [cand("attachment", "C0870313", "13792970")];
        let verdicts = [verdict("C0870313", "13792970", "attachment", true, 0.80)];
        let (merges, _) = resolve(&cands, &verdicts);
        assert!(merges.is_empty());
    }

    /// Rule 3: one surface claimed by two synsets. Higher confidence wins…
    #[test]
    fn a_surface_claimed_by_two_synsets_goes_to_the_more_confident_one() {
        let cands = [
            cand("state", "C1442792", "00024720"),
            cand("state", "C1442792", "08654360"),
        ];
        let verdicts = [
            verdict("C1442792", "00024720", "state", true, 0.95),
            verdict("C1442792", "08654360", "state", true, 0.88),
        ];
        let (merges, stats) = resolve(&cands, &verdicts);
        assert_eq!(merges.len(), 1);
        assert_eq!(merges[0].offset, "00024720");
        assert_eq!(stats.ties_dropped, 0);
    }

    /// …and an exact tie is DROPPED. With no basis to choose, prefer to miss.
    #[test]
    fn an_exact_tie_is_dropped_rather_than_guessed() {
        let cands = [
            cand("state", "C1442792", "00024720"),
            cand("state", "C1442792", "08654360"),
        ];
        let verdicts = [
            verdict("C1442792", "00024720", "state", true, 0.9),
            verdict("C1442792", "08654360", "state", true, 0.9),
        ];
        let (merges, stats) = resolve(&cands, &verdicts);
        assert!(merges.is_empty());
        assert_eq!(stats.ties_dropped, 1);
    }

    /// A candidate the adjudicator never returned a verdict for is COUNTED, not silently treated as
    /// "different". It fails closed and stays visible.
    #[test]
    fn an_unjudged_candidate_is_counted_not_silently_dropped() {
        let cands = [cand("epsilon toxin", "C0148554", "14806598")];
        let (merges, stats) = resolve(&cands, &[]);
        assert!(merges.is_empty());
        assert_eq!(stats.unjudged, 1);
    }
}
