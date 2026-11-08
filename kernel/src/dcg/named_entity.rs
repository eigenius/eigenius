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

/// Does the text between token `k` and token `k + 1` carry a clause boundary?
///
/// The parser drops `:` and a sentence-final `.` instead of emitting them, so two tokens can be
/// adjacent in the stream with a boundary between them in the text.
fn punctuation_between(text: &str, toks: &[super::preprocess::Token], k: usize) -> bool {
    let Some(a) = toks.get(k) else { return false };
    let Some(b) = toks.get(k + 1) else {
        return false;
    };
    let (lo, hi) = (a.span().end, b.span().start);
    if lo > hi || hi > text.len() || !text.is_char_boundary(lo) || !text.is_char_boundary(hi) {
        return false;
    }
    text[lo..hi]
        .chars()
        .any(|c| matches!(c, '.' | '!' | '?' | ';' | ':' | ','))
}

/// Is every letter of `s` upper-case (`DRIVE`, `CAT`)? A single lower-case letter makes it false.
fn all_caps(s: &str) -> bool {
    let mut saw = false;
    for c in s.chars().filter(|c| c.is_alphabetic()) {
        if !c.is_uppercase() {
            return false;
        }
        saw = true;
    }
    saw
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
    // ALL-CAPS acronym, or Title-case: first upper, no interior upper (a single capital then lower-case
    // letters/digits — rejects `mRNA`, `HeLa`, admitted only when fully upper).
    all_caps(bare)
        || bare
            .chars()
            .skip(1)
            .filter(|c| c.is_alphabetic())
            .all(|c| c.is_lowercase())
}

/// Extract `<head> <Name…>` apposition candidates from a document's units, **already tokenized by the
/// parser**. The `head` is an alphabetic token that is NOT a function word ([`HEAD_STOP_WORDS`]), is
/// not marked as code by the author (`marked_as_code`), and passes the injected `head_ok` predicate
/// (a noun that is not an adjective — see the layer wrapper in [`super::glossary`]). The `name` is the maximal following run
/// of proper-name tokens ([`name_token`]) within the unit. A candidate is admitted iff its surface
/// **recurs** (≥2 occurrences, case-insensitively) — the precision signal separating a
/// repeatedly-referenced named entity from a one-off verb+object. First-seen wins per surface.
///
/// **Tokens, not `split_whitespace`.** The minted entry is a MULTIWORD key, and the parser matches it
/// by joining its own tokens — so a key built from anything else is a key it can never match. The
/// hand-rolled version read raw whitespace runs and stripped surrounding punctuation itself, which
/// made it blind to markup: `axis (Damgaard et al. 2012)` yielded the entity `axis Damgaard`, and
/// `` the `CAT` PAM `` yielded `CAT PAM`. Both are strings the parser never forms — the parenthetical
/// is resolved away before tokens exist — so both were DEAD entries claiming coverage that did not
/// exist. This was the fourth time a glossary source re-derived the tokenizer (D102 decision 7).
///
/// A clause boundary is read from the GAP between two tokens. The parser emits `,` as a non-word
/// token but drops `:` and a sentence-final `.` entirely, so adjacency in the token stream is not
/// adjacency in the text: `Two errata: Nat Commun 2025` tokenizes as `errata`, `Nat`, `Commun`, and
/// reading that as one apposition produced `errata Nat Commun`, which split `Nat Commun`'s own
/// recurrence count and lost it. Each unit's text comes in with its tokens so the slice between
/// consecutive spans can be inspected; that is the parser's own data, not a second tokenizer.
///
/// The predicate is injected (not a `&Layer` parameter) so the extraction logic is unit-testable with a
/// closure; the layer-backed entry point that supplies the real check + mints/emits lives in
/// [`super::glossary`], where the chain is available.
pub fn extract_named_entities_with(
    units: &[(&str, Vec<super::preprocess::Token>)],
    head_ok: impl Fn(&str) -> bool,
    marked_as_code: impl Fn(usize, &super::preprocess::Token) -> bool,
) -> Vec<NamedEntity> {
    // Case-insensitive surface-frequency table for the recurrence guard.
    let mut freq: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut candidates: Vec<NamedEntity> = Vec::new();

    for (u, (text, toks)) in units.iter().enumerate() {
        extract_in_unit(
            u,
            text,
            toks,
            &head_ok,
            &marked_as_code,
            &mut freq,
            &mut candidates,
        );
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

/// One unit's tokens, in order. Factored out so the recurrence table spans the whole document while
/// the adjacency scan never crosses a unit boundary.
///
/// A NON-WORD token breaks a run rather than being skipped. The parser emits `,` as one, and a comma
/// is exactly what used to separate `axis` from `Damgaard`; dropping non-words would restore the
/// adjacency the boundary rule exists to deny. `:` and a sentence-final `.` are dropped by the
/// parser rather than emitted, so those are read from the text between the two spans.
fn extract_in_unit(
    unit: usize,
    text: &str,
    toks: &[super::preprocess::Token],
    head_ok: &impl Fn(&str) -> bool,
    marked_as_code: &impl Fn(usize, &super::preprocess::Token) -> bool,
    freq: &mut std::collections::BTreeMap<String, usize>,
    candidates: &mut Vec<NamedEntity>,
) {
    let raw: Vec<&str> = toks.iter().map(|t| t.surface()).collect();
    let word: Vec<bool> = toks.iter().map(|t| t.is_word()).collect();
    let mut i = 0;
    while i + 1 < raw.len() {
        if !word[i] {
            i += 1;
            continue;
        }
        let head = raw[i];
        // A head is an alphabetic, non-stop word that passes the injected admissibility check (a noun
        // that is not an adjective — see the layer wrapper in `glossary`) and that the author has NOT
        // marked as code.
        //
        // Markup, not orthography. `` the `CAT` PAM `` yielded the entity `CAT PAM`: `CAT` is a
        // spacer's PAM sequence quoted as a literal, not a noun typing `PAM`. Rejecting an ALL-CAPS
        // head would also catch it, and would be wrong — this page's all-caps-headed appositions are
        // `CFTR W1282X`, `ACMG PM2`, `MANE CDS` and `REST API`, every one of them real, and
        // `CFTR W1282X` types a variant by its gene. The author's backticks say which is which.
        //
        // THIS OCCURRENCE, not the surface. Asking whether the surface appears in any span rejects a
        // head document-wide, and on this page that is `gene`, `protein`, `variant`, `region`,
        // `codon`, `sequence`, `position`, `score` and 60 more — every one an ordinary common noun
        // that also sits inside a quoted URL or query string.
        let head_lc = head.to_ascii_lowercase();
        let admissible = head.chars().count() >= 2
            && head.chars().all(|c| c.is_alphabetic())
            && !HEAD_STOP_WORDS.contains(&head_lc.as_str())
            && !marked_as_code(unit, &toks[i])
            && head_ok(&head_lc);
        if !admissible {
            i += 1;
            continue;
        }
        // Maximal following run of name tokens, broken by a non-word token, by punctuation in the
        // gap between two tokens, and by the end of the unit.
        if punctuation_between(text, toks, i) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut names: Vec<&str> = Vec::new();
        while j < raw.len() {
            if !word[j]
                || !name_token(raw[j])
                || (j > i + 1 && punctuation_between(text, toks, j - 1))
            {
                break;
            }
            names.push(raw[j]);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Admissible apposition heads for the tests — the injected stand-in for the layer wrapper's
    /// "noun that is not an adjective". EXCLUDES verbs ("identified"/"evaluated") and adjectives
    /// ("somatic"/"other"/"deficient"); "vitamin"/"type" are heads so the single-letter-name guard, not a
    /// missing head, is what rejects "vitamin D". "errata"/"nat" are admitted so the COLON, not the
    /// head test, is what breaks `Two errata: Nat Commun`.
    fn head_ok(w: &str) -> bool {
        matches!(
            w,
            "project" | "gene" | "vitamin" | "type" | "screen" | "errata" | "nat"
        )
    }

    /// No markup in these fixtures, so only the head test and orthography can reject anything.
    fn no_code(_u: usize, _t: &crate::dcg::preprocess::Token) -> bool {
        false
    }

    /// `text` as the pipeline delivers it: segmented into units, each tokenized by the parser.
    fn units(text: &str) -> Vec<(String, Vec<crate::dcg::preprocess::Token>)> {
        let prose = crate::dcg::quantity::ProseUnits::none();
        crate::dcg::segment::segment_sentences(text)
            .into_iter()
            .map(|u| {
                let toks = crate::dcg::preprocess::tokenize(&u, &prose);
                (u, toks)
            })
            .collect()
    }

    /// The borrowed view `extract_named_entities_with` takes.
    fn view(
        us: &[(String, Vec<crate::dcg::preprocess::Token>)],
    ) -> Vec<(&str, Vec<crate::dcg::preprocess::Token>)> {
        us.iter().map(|(t, k)| (t.as_str(), k.clone())).collect()
    }

    fn surfaces(text: &str) -> Vec<String> {
        extract_named_entities_with(&view(&units(text)), head_ok, no_code)
            .into_iter()
            .map(|e| e.surface)
            .collect()
    }

    /// A boundary the parser DROPS still breaks the apposition.
    ///
    /// `,` arrives as a non-word token, but `:` and a sentence-final `.` are dropped, so
    /// `Two errata: Nat Commun 2025` tokenizes as `errata`, `Nat`, `Commun` — adjacent in the
    /// stream, separated in the text. Reading that as one apposition yielded `errata Nat Commun`,
    /// which took one of `Nat Commun`'s two occurrences with it and dropped the real entity below
    /// the recurrence threshold. The gap between the two spans is where the colon is.
    #[test]
    fn a_boundary_the_parser_drops_still_breaks_the_apposition() {
        let got = surfaces("Two errata: Nat Commun 2025. Two errata: Nat Commun 2026.");
        assert!(
            !got.iter().any(|s| s == "errata Nat Commun"),
            "a colon separates the head from the name: {got:?}"
        );
        assert_eq!(
            got,
            vec!["Nat Commun".to_string()],
            "and the real apposition keeps both of its occurrences"
        );
    }

    /// A head the author quoted as a literal is not a head — and ALL-CAPS alone is not the signal.
    ///
    /// `` the `CAT` PAM `` yielded the entity `CAT PAM`: `CAT` is a PAM sequence in backticks, not a
    /// noun typing `PAM`. Rejecting all-caps heads would catch it, and would be wrong — the XIAP
    /// page's all-caps-headed appositions are `CFTR W1282X`, `ACMG PM2`, `MANE CDS` and `REST API`,
    /// all four real, and `CFTR W1282X` types a variant by its gene.
    #[test]
    fn a_head_the_author_marked_as_code_is_not_a_head() {
        // Same words, same shape; only the backticks differ. `code_span_ranges` lives with the
        // layer-backed wrapper, so the fixture marks the occurrence by its byte span here.
        let bare = "As was the CAT PAM. As was the CAT PAM.";
        let marked = "As was the `CAT` PAM. As was the `CAT` PAM.";
        let surfaces_of = |text: &str| -> Vec<String> {
            let spans: Vec<Vec<std::ops::Range<usize>>> =
                crate::dcg::segment::segment_sentences(text)
                    .iter()
                    .map(|u| crate::dcg::glossary::code_span_ranges(u))
                    .collect();
            extract_named_entities_with(
                &view(&units(text)),
                |_| true,
                |u, t| {
                    let s = t.span();
                    spans[u]
                        .iter()
                        .any(|r| r.start <= s.start && s.end <= r.end)
                },
            )
            .into_iter()
            .map(|e| e.surface)
            .collect()
        };
        assert_eq!(
            surfaces_of(bare),
            vec!["CAT PAM".to_string()],
            "the premise: nothing but the markup distinguishes this"
        );
        assert!(
            surfaces_of(marked).is_empty(),
            "the author marked CAT as code: {:?}",
            surfaces_of(marked)
        );

        // An all-caps head the author did NOT mark stays admissible.
        let gene = extract_named_entities_with(
            &view(&units(
                "The CFTR W1282X allele recurs. The CFTR W1282X allele recurs.",
            )),
            |_| true,
            no_code,
        );
        assert!(
            gene.iter().any(|e| e.surface == "CFTR W1282X"),
            "an unmarked all-caps head is a head: {gene:?}"
        );
    }

    #[test]
    fn a_recurring_apposition_is_recognised() {
        // "project DRIVE" — admissible head + name, occurring twice, is recognized (fields exposed).
        let got = extract_named_entities_with(
            &view(&units(
                "We used project DRIVE to screen. Then project DRIVE ran again.",
            )),
            head_ok,
            no_code,
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
