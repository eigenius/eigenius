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

//! **Named-entity recognition** — the *fourth* document-glossary extraction source (D63,
//! `docs/notes/d63-named-entity-glossary-source.md`). Deterministic **apposition**: a common-noun HEAD
//! immediately followed by a proper NAME — a run of Capitalized / ALL-CAPS tokens ("project Achilles",
//! "project DRIVE"). A recognized name is later minted as a doc-local **named individual** and emitted
//! as a `cat_np` proper-noun alias (grounding + emission live in `super::glossary`, layer-backed).
//!
//! Unlike [`super::abbrev`] (Schwartz-Hearst is a purely orthographic `Long Form (SHORT)` pattern),
//! apposition is NOT decidable from orthography alone: the head must be a NOUN, separating "project
//! DRIVE" (noun+name) from "identified WRN" (verb+object) and "in DRIVE" (prep+name). But in a
//! 7.6M-entry lexicon almost every surface has *some* noun sense, so "is a common noun" does not
//! discriminate. Two signals that DO:
//!
//! - **Recurrence** — a genuine named entity is referred to repeatedly; a one-off verb+object or
//!   adjective+noun bigram is not. The surface must occur **≥2 times** in the document.
//! - **Head is not an adjective** — rejects "somatic MMR", "other DNA", "deficient DNA" (which recur or
//!   not) where the "head" is really an attributive adjective, not the noun an apposition needs.
//!
//! The head admissibility (a noun that is not an adjective) is an **injected predicate**
//! ([`extract_named_entities_with`]) so the logic is unit-testable with a closure; the layer-backed
//! predicate + minting/emission live in [`super::glossary`]. Orthography decides the rest: the name
//! shape (all-caps acronym or Title-case, ≥2 letters), a function-word head stop-list, and the
//! clause-boundary stop.

use std::collections::BTreeSet;

/// One recognized named-entity candidate: the full `surface` ("Project Achilles"), the `head` common
/// noun as written ("Project" — already checked common-noun by the recognizer's predicate), and the
/// proper `name` ("Achilles" — its not-a-common-noun status is re-checked against the lexicon at
/// grounding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedEntity {
    pub surface: String,
    pub head: String,
    pub name: String,
}

/// Closed-class words that may orthographically precede a capitalized token but are never the head of a
/// `<common-noun> <Name>` apposition ("the DRIVE", "in Achilles"). A coarse stop-list — the real
/// common-noun test is at grounding; this only trims obvious noise so the candidate set stays small.
const HEAD_STOP_WORDS: &[&str] = &[
    "the", "a", "an", "of", "in", "on", "at", "to", "for", "with", "by", "from", "as", "and", "or",
    "but", "nor", "so", "yet", "into", "onto", "than", "that", "this", "these", "those", "is",
    "are", "was", "were", "be", "been", "being", "we", "it", "its", "our", "their", "his", "her",
    "no", "not", "if", "then", "when", "where", "which", "who", "whom", "whose", "both", "either",
    "neither", "each", "any", "all", "some",
];

/// The bare word of a raw whitespace token: leading/trailing non-alphanumerics stripped ("project," →
/// "project", "(DRIVE)" → "DRIVE"). Interior punctuation is kept (hyphens, digits).
fn bare(tok: &str) -> &str {
    tok.trim_matches(|c: char| !c.is_alphanumeric())
}

/// Does the raw token carry a **sentence/clause boundary** — a trailing `.`, `!`, `?`, `;`, `:` or `,`
/// (so the following token starts a new clause and must not join this one as a name)?
fn ends_clause(tok: &str) -> bool {
    tok.trim_end()
        .chars()
        .last()
        .is_some_and(|c| matches!(c, '.' | '!' | '?' | ';' | ':' | ','))
}

/// Is `bare` a proper-**name** token: ALL-CAPS (an acronym like `DRIVE`, `CRISPR`) or Title-case
/// (`Achilles`), at least two letters? Rejects single letters (`vitamin D`, `type X`), all-lower words,
/// and digit/symbol tokens.
fn name_token(bare: &str) -> bool {
    let letters = bare.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 2 {
        return false;
    }
    if !bare.chars().next().is_some_and(|c| c.is_uppercase()) {
        return false;
    }
    let all_caps = bare
        .chars()
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_uppercase());
    // ALL-CAPS acronym, or Title-case: first upper, no interior upper (a single capital then lower-case
    // letters/digits — rejects `mRNA`, `HeLa`, admitted only when fully upper).
    all_caps
        || bare
            .chars()
            .skip(1)
            .filter(|c| c.is_alphabetic())
            .all(|c| c.is_lowercase())
}

/// Extract `<head> <Name…>` apposition candidates from raw document text. The `head` is an alphabetic
/// token that is NOT a function word ([`HEAD_STOP_WORDS`]) and passes the injected `head_ok` predicate
/// (a noun that is not an adjective — see the layer wrapper in [`super::glossary`]). The `name` is the
/// maximal following run of proper-name tokens ([`name_token`]) with no clause boundary crossed. A
/// candidate is admitted iff its surface **recurs** (≥2 occurrences, case-insensitively) — the
/// precision signal separating a repeatedly-referenced named entity from a one-off verb+object. First-
/// seen wins per surface.
///
/// The predicate is injected (not a `&Layer` parameter) so the extraction logic is unit-testable with a
/// closure; the layer-backed entry point that supplies the real check + mints/emits lives in
/// [`super::glossary`], where the chain is available.
pub fn extract_named_entities_with(text: &str, head_ok: impl Fn(&str) -> bool) -> Vec<NamedEntity> {
    let raw: Vec<&str> = text.split_whitespace().collect();
    // Case-insensitive surface-frequency table for the recurrence guard.
    let mut freq: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut candidates: Vec<NamedEntity> = Vec::new();

    let mut i = 0;
    while i + 1 < raw.len() {
        let head_raw = raw[i];
        let head = bare(head_raw);
        // A head is an alphabetic, non-stop word that ends no clause and passes the injected admissibility
        // check (a noun that is not an adjective — see the layer wrapper in `glossary`).
        let head_lc = head.to_ascii_lowercase();
        let admissible = head.chars().count() >= 2
            && head.chars().all(|c| c.is_alphabetic())
            && !HEAD_STOP_WORDS.contains(&head_lc.as_str())
            && !ends_clause(head_raw)
            && head_ok(&head_lc);
        if !admissible {
            i += 1;
            continue;
        }
        // Maximal following run of name tokens, stopping at a clause boundary (the boundary token is
        // NOT included).
        let mut j = i + 1;
        let mut names: Vec<&str> = Vec::new();
        while j < raw.len() {
            let nb = bare(raw[j]);
            if !name_token(nb) {
                break;
            }
            names.push(nb);
            if ends_clause(raw[j]) {
                break; // this name ends the clause — keep it, stop the run
            }
            j += 1;
        }
        if names.is_empty() {
            i += 1;
            continue;
        }
        let name = names.join(" ");
        let surface = format!("{head} {name}");
        *freq.entry(surface.to_ascii_lowercase()).or_default() += 1;
        candidates.push(NamedEntity {
            surface,
            head: head.to_string(),
            name,
        });
        i = j.max(i + 1);
    }

    // Admit iff the surface RECURS (≥2 occurrences) — a genuine named entity is referred to repeatedly,
    // while a one-off verb+object ("identified WRN") or adjective+noun ("other DNA") is not. Dedupe by
    // surface, first-seen wins.
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for ent in candidates {
        let key = ent.surface.to_ascii_lowercase();
        if freq.get(&key).copied().unwrap_or(0) < 2 {
            continue;
        }
        if seen.insert(key) {
            out.push(ent);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Admissible apposition heads for the tests — the injected stand-in for the layer wrapper's
    /// "noun that is not an adjective". EXCLUDES verbs ("identified"/"evaluated") and adjectives
    /// ("somatic"/"other"/"deficient"); "vitamin"/"type" are heads so the single-letter-name guard, not a
    /// missing head, is what rejects "vitamin D".
    fn head_ok(w: &str) -> bool {
        matches!(w, "project" | "gene" | "vitamin" | "type" | "screen")
    }

    fn surfaces(text: &str) -> Vec<String> {
        extract_named_entities_with(text, head_ok)
            .into_iter()
            .map(|e| e.surface)
            .collect()
    }

    #[test]
    fn a_recurring_apposition_is_recognised() {
        // "project DRIVE" — admissible head + name, occurring twice, is recognized (fields exposed).
        let got = extract_named_entities_with(
            "We used project DRIVE to screen. Then project DRIVE ran again.",
            head_ok,
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].surface, "project DRIVE");
        assert_eq!(got[0].head, "project");
        assert_eq!(got[0].name, "DRIVE");
    }

    #[test]
    fn a_one_off_apposition_is_not_admitted() {
        // A single occurrence — even a clean head+name — is NOT a named entity (recurrence is required);
        // the same surface twice IS. Holds for both Title-case and all-caps names.
        assert!(surfaces("Project Achilles found a thing.").is_empty());
        assert!(surfaces("We used project DRIVE once.").is_empty());
        let twice = surfaces("Project Achilles found a thing. Project Achilles is a screen.");
        assert_eq!(twice, vec!["Project Achilles".to_string()]);
    }

    #[test]
    fn non_noun_head_is_rejected_even_when_recurring() {
        // "identified WRN" (verb head), "somatic MMR" (adjective head), "the DRIVE" (function word) —
        // never appositions, even repeated. `head_ok` rejects the verb/adjective; the stop-list the
        // function word.
        let text =
            "We identified WRN and somatic MMR and the DRIVE. We identified WRN and somatic \
                    MMR and the DRIVE.";
        assert!(surfaces(text).is_empty());
    }

    #[test]
    fn single_letter_name_is_not_a_name() {
        // "vitamin D", "type X" — single-letter designators, not proper names in v1 (heads ARE ok).
        let text = "A vitamin D and type X assay. A vitamin D and type X assay.";
        assert!(surfaces(text).is_empty());
    }

    #[test]
    fn clause_boundary_is_not_crossed() {
        // Head at a clause end must not bind the next clause's initial capital.
        let text = "We used a project. Achilles was here. We used a project. Achilles was here.";
        assert!(surfaces(text).is_empty());
    }

    #[test]
    fn both_paper_names_from_one_passage() {
        // The two real names recur; the verb+object "identified WRN" does not.
        let text = "Project Achilles and project DRIVE identified WRN. Project Achilles screened \
                    genes; project DRIVE analysed genes.";
        let mut got = surfaces(text);
        got.sort();
        assert_eq!(
            got,
            vec!["Project Achilles".to_string(), "project DRIVE".to_string()]
        );
    }
}
