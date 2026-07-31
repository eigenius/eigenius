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

//! **Surfaces the closed-class layer owns** — the single list both lexicon importers consult.
//!
//! `ontologies/lexicon/closed-class.esl` supplies the grammatical reading of these words
//! (prepositions, conjunctions, determiners, the copula). A content-word importer must therefore NOT
//! seed a content noun/verb on them: the content senses that collide on these surfaces are
//! element-symbol and acronym homonyms (`As` = arsenic AND American Samoa, `In` = indium, `Be` =
//! beryllium, `At` = astatine) or terminology reifications of the function word itself (`For
//! (preposition)`, `Some (qualifier value)`, `RelationshipConjunction - and`). Seeded, they let a
//! function word **pile into a compound noun** instead of doing its grammatical job — measured on the
//! WRN reference page as "We evaluated MSI **as** a biomarker for WRN dependency" parsing as a compound
//! *"a Microsatellite-Instability **As** dependency"*, 19 structural readings.
//!
//! Dropping them cannot make a word unknown (the bootstrap covers it), and a document that genuinely
//! needs the symbol recovers it as a document-glossary entry — the same accepted tradeoff the UMLS
//! importer already documents for `as`=arsenic / `in`=indium.
//!
//! This list is deliberately **only** what the closed class owns. Importer-specific artefact lists
//! (UMLS's `lead`/`alone`/`negation` reifications) stay in that importer: `lead` is a legitimate
//! WordNet content noun and verb, so it must not be dropped corpus-wide.

/// Prepositions and conjunctions (D63 §5.3).
const PREPOSITIONS_AND_CONJUNCTIONS: &[&str] = &[
    "for", "from", "into", "as", "with", "on", "at", "by", "of", "in", "then", "than", "within",
    "upon", "onto", "unto", // prepositions
    "and", "or", "but", "nor", // coordinating conjunctions
];

/// Determiners and quantifiers the bootstrap ships (D63 §8.3).
const DETERMINERS: &[&str] = &[
    "some", "each", "every", "all", "any", "no", "several", "many", "few", "fewer", "most", "both",
];

/// **Demonstratives** — `this`/`that`/`these`/`those` (2026-07-29).
///
/// The closed-class layer has always claimed these (`closed-class.esl`: "A definite ('the',
/// 'this'/'that', 'these'/'those') denotes a fixed, presupposed referent") and ships determiner
/// entries for all four — `this` 2, `that` 3, `these` 2, `those` 2 — but they were missing from the
/// list above, so a content importer was free to seed a noun on them. UMLS does:
///
///   C0039828  CHV `SY` **these**   the MeSH PUBLICATION TYPE *Theses*
///   C1080058  CHV `PT` **this**    the insect genus *This <Coelopellini>* (NCBI `SCN` "This")
///
/// Three atoms, two concepts, and the `these` one alone accounted for **24 of the 48 skeletons** on
/// "MSI cell lines from these four lineages showed greater dependence on WRN than their MSS
/// counterparts." — `these four lineages` re-bracketed as a THREE-nominal compound (*a Theses four
/// line*) instead of the definite `the(Σ… four lineage)`. Page-wide the artefact reached 622 of 2871
/// printed readings across 3 units.
///
/// This is the same defect class the module header already names ("terminology reifications of the
/// function word itself"), and it is NOT reachable by the two mechanisms that look adjacent:
/// - `drops.rs` requires a WordNet-NOUN collision, and `these` is not a WordNet noun in any POS
///   (checked across index.{noun,verb,adj,adv} — all four demonstratives are absent), so it never
///   becomes a drop candidate;
/// - case-sensitive symbol matching ([`super::parse::all_caps_symbol`]) does not apply — `this` and
///   `These` are not all-caps, so both entries survive that filter.
///
/// Dropping the content sense cannot open a grammar gap: the closed class supplies all four, and
/// WordNet has no content entry on any of them to lose.
const DEMONSTRATIVES: &[&str] = &["this", "that", "these", "those"];

/// The copula and its inflections — the grammatical core of predication. `being` is EXCLUDED: it is a
/// legitimate common noun ("a living being"), and its progressive use needs no content sense.
const COPULA: &[&str] = &["be", "is", "are", "was", "were", "am", "been"];

/// Whether `form` is a surface the closed-class layer owns, so a content-word importer must not seed a
/// content entry for it. Case-insensitive, exact match on the WHOLE surface — a multiword form that
/// merely *contains* a function word (`act on`, `cell line`) is unaffected.
pub fn is_closed_class_surface(form: &str) -> bool {
    let f = form.trim().to_ascii_lowercase();
    PREPOSITIONS_AND_CONJUNCTIONS.contains(&f.as_str())
        || DETERMINERS.contains(&f.as_str())
        || DEMONSTRATIVES.contains(&f.as_str())
        || COPULA.contains(&f.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_function_words_determiners_and_the_copula() {
        for f in ["as", "As", "AS", "in", "at", "of", "and", "than"] {
            assert!(is_closed_class_surface(f), "{f} is closed-class");
        }
        for f in ["some", "each", "no", "both"] {
            assert!(is_closed_class_surface(f), "{f} is a determiner");
        }
        // Demonstratives, case-insensitively — a sentence-initial `These` must be caught too, and
        // since `tokenize` stopped lowercasing (2026-07-29) that is the form the importer sees.
        for f in ["this", "that", "these", "those", "These", "THOSE"] {
            assert!(is_closed_class_surface(f), "{f} is a demonstrative");
        }
        for f in ["be", "is", "were", "been"] {
            assert!(is_closed_class_surface(f), "{f} is a copula form");
        }
    }

    #[test]
    fn leaves_content_words_alone() {
        // `being` is a legitimate noun; `lead`/`alone` are legitimate WordNet content (their UMLS
        // reifications are dropped by that importer's own list, not here); modals/auxiliaries that the
        // corpus needs as content verbs are untouched.
        for f in [
            "being",
            "lead",
            "alone",
            "have",
            "do",
            "will",
            "can",
            "cell line",
            "act on",
            "arsenic",
        ] {
            assert!(!is_closed_class_surface(f), "{f} must NOT be dropped");
        }
    }
}
