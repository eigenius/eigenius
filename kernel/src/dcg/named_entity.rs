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
//! apposition is NOT decidable from orthography alone: its defining feature — the head is a *common
//! noun* — is lexical, separating "project DRIVE" (noun+name) from "identified WRN" (verb+object) and
//! "in DRIVE" (prep+name). So the head common-noun test is part of recognition, taken as an **injected
//! predicate** ([`extract_named_entities_with`]) rather than a `&Layer` parameter, keeping the logic
//! unit-testable with a closure; the layer-backed entry point that supplies the real check + mints/emits
//! lives in [`super::glossary`]. Orthography + the document decide the rest: the name shape, a
//! function-word head stop-list, and the guard against one-off sentence-initial Title Case (an all-caps
//! name is a strong proper signal; a Title-case name must **recur** in the document). One layer-backed
//! filter is still deferred to grounding — the NAME is not itself a common noun, so "DNA polymerase"
//! stays a common-noun compound, not a spurious individual.

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

/// A proper-**name** token: ALL-CAPS (an acronym like `DRIVE`, `CRISPR`) or Title-case (`Achilles`),
/// at least two letters. Rejects single letters (`vitamin D`, `type X`), all-lower words, and
/// digit/symbol tokens. Returns `(is_name, is_all_caps)` — the all-caps flag is the strong proper
/// signal used by the recurrence guard.
fn name_token(bare: &str) -> Option<bool> {
    let letters = bare.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 2 {
        return None;
    }
    let first_upper = bare.chars().next().is_some_and(|c| c.is_uppercase());
    if !first_upper {
        return None;
    }
    let all_caps = bare
        .chars()
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_uppercase());
    if all_caps {
        return Some(true);
    }
    // Title-case: first upper, no interior upper (rejects `mRNA`, `HeLa` — handled as acronym-ish only
    // when fully upper). A single capital followed by lower-case letters/digits.
    let rest_lower = bare
        .chars()
        .skip(1)
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_lowercase());
    rest_lower.then_some(false)
}

/// Extract `<common-noun-head> <Name…>` apposition candidates from raw document text. The `head` is an
/// alphabetic token that is NOT a function word ([`HEAD_STOP_WORDS`]) and IS a known common noun per the
/// injected `is_common_noun` predicate (called on the lower-cased head) — this is the load-bearing
/// apposition signal, separating "project DRIVE" (noun+name) from "identified WRN" (verb+object) and "in
/// DRIVE" (prep+name), which pure orthography cannot. The `name` is the maximal following run of
/// proper-name tokens ([`name_token`]) with no clause boundary crossed. A candidate is admitted iff the
/// name run contains an ALL-CAPS token **or** the whole surface recurs in the document (case-insensitively)
/// — the guard against a one-off sentence-initial Title-case coincidence. First-seen wins per surface
/// (deduped, case-insensitively).
///
/// The predicate is injected (not a `&Layer` parameter) so the extraction logic is unit-testable with a
/// closure; the layer-backed entry point that supplies the real check + mints/emits lives in
/// [`super::glossary`], where the chain is available.
pub fn extract_named_entities_with(
    text: &str,
    is_common_noun: impl Fn(&str) -> bool,
) -> Vec<NamedEntity> {
    let raw: Vec<&str> = text.split_whitespace().collect();
    // Case-insensitive surface-frequency table for the recurrence guard.
    let mut freq: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut candidates: Vec<(usize, NamedEntity, bool)> = Vec::new(); // (idx, entity, name_has_all_caps)

    let mut i = 0;
    while i + 1 < raw.len() {
        let head_raw = raw[i];
        let head = bare(head_raw);
        // A head is an alphabetic, non-stop, non-name-cased word (so the FIRST token of a name run is
        // never mistaken for a head), and it must not end its clause.
        let head_lc = head.to_ascii_lowercase();
        let head_ok = head.chars().count() >= 2
            && head.chars().all(|c| c.is_alphabetic())
            && !HEAD_STOP_WORDS.contains(&head_lc.as_str())
            && !ends_clause(head_raw)
            && is_common_noun(&head_lc);
        if !head_ok {
            i += 1;
            continue;
        }
        // Maximal following run of name tokens, stopping at a clause boundary (the boundary token is
        // NOT included).
        let mut j = i + 1;
        let mut names: Vec<&str> = Vec::new();
        let mut any_all_caps = false;
        while j < raw.len() {
            let nb = bare(raw[j]);
            let Some(all_caps) = name_token(nb) else {
                break;
            };
            names.push(nb);
            any_all_caps |= all_caps;
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
        candidates.push((
            i,
            NamedEntity {
                surface,
                head: head.to_string(),
                name,
            },
            any_all_caps,
        ));
        i = j.max(i + 1);
    }

    // Admit by the guard (all-caps OR recurs); dedupe by surface, first-seen wins.
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (_, ent, all_caps) in candidates {
        let key = ent.surface.to_ascii_lowercase();
        let recurs = freq.get(&key).copied().unwrap_or(0) >= 2;
        if !(all_caps || recurs) {
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

    /// The corpus's common nouns for the tests — the injected stand-in for the layer-backed check.
    /// Deliberately EXCLUDES "identified"/"screened"/"analysed" (verbs) and "vitamin"/"type" is
    /// included so the single-letter-name guard, not a missing head, is what rejects "vitamin D".
    fn is_noun(w: &str) -> bool {
        matches!(
            w,
            "project" | "gene" | "vitamin" | "type" | "screen" | "line"
        )
    }

    fn surfaces(text: &str) -> Vec<String> {
        extract_named_entities_with(text, is_noun)
            .into_iter()
            .map(|e| e.surface)
            .collect()
    }

    #[test]
    fn all_caps_name_is_recognised_on_first_sight() {
        // "project DRIVE" — common-noun head + all-caps acronym name; the all-caps signal admits it
        // without needing a second occurrence.
        let got =
            extract_named_entities_with("We used project DRIVE to screen the lines.", is_noun);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].surface, "project DRIVE");
        assert_eq!(got[0].head, "project");
        assert_eq!(got[0].name, "DRIVE");
    }

    #[test]
    fn title_case_name_needs_recurrence() {
        // A one-off Title-case apposition is NOT admitted (could be a sentence-initial coincidence);
        // the same surface twice IS.
        assert!(surfaces("Project Achilles found a thing.").is_empty());
        let twice = surfaces("Project Achilles found a thing. Project Achilles is a screen.");
        assert_eq!(twice, vec!["Project Achilles".to_string()]);
    }

    #[test]
    fn non_noun_head_is_rejected() {
        // "identified WRN" (verb+object), "the DRIVE"/"in DRIVE" (function word) — never appositions,
        // even though orthographically they are word+CAPS. The common-noun predicate is what rejects the
        // verb; the stop-list rejects the function words.
        assert!(surfaces("We identified WRN and the DRIVE and in DRIVE too.").is_empty());
    }

    #[test]
    fn single_letter_name_is_not_a_name() {
        // "vitamin D", "type X" — single-letter designators, not proper names in v1 (heads ARE nouns).
        assert!(surfaces("A vitamin D and type X assay. A vitamin D and type X assay.").is_empty());
    }

    #[test]
    fn clause_boundary_is_not_crossed() {
        // Head at a clause end must not bind the next clause's initial capital.
        assert!(surfaces("We used a project. Achilles was the target.").is_empty());
    }

    #[test]
    fn both_paper_names_from_one_passage() {
        let text = "Project Achilles and project DRIVE identified WRN. Project Achilles screened \
                    lines; project DRIVE analysed lines.";
        let mut got = surfaces(text);
        got.sort();
        assert_eq!(
            got,
            vec!["Project Achilles".to_string(), "project DRIVE".to_string()]
        );
    }
}
