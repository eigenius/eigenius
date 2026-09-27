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

//! D62 S0 — document segmentation (text-only).
//!
//! The front of the encoding pipeline: a document is split into the sentence units the parser can
//! attempt ([`segment_sentences`]). A sentence is split into tokens by [`super::lex`] and
//! [`super::preprocess`] (D95).
//!
//! Deterministic, no LLM. Verified on real paper prose in
//! `crates/eigenius-wordnet/tests/encoding_prototype.rs` (the cleaned WRN first page: a naive
//! `.`/`!`/`?` split over-segments 4 paragraphs into 47 units; this yields ~26).

/// Abbreviations (and, by the single-letter guard, initials / `e.g.` / `i.e.`) whose trailing
/// `.` is NOT a sentence boundary. Lowercased, alphanumerics only.
const ABBREV: &[&str] = &[
    "fig",
    "figs",
    "et",
    "al",
    "vs",
    "no",
    "ca",
    "approx",
    "etc",
    "cf",
    "ref",
    "eq",
    "exp",
    "data",
    "extended",
    "supplementary",
    "tab",
    "table",
    "eg",
    "ie",
    "dr",
    "mr",
    "vol",
    "ed",
    "pp",
];

/// Whether `word`'s trailing `.` is an abbreviation period (so not a sentence boundary): a known
/// abbreviation, or an initialism — a word whose dot-separated parts are each one letter (an initial
/// `e`, `e.g`, `r.p.m`, `s.e.m`). `next` is the next **non-whitespace** char after the period (or
/// `'\0'` at end-of-text). An initialism is an abbreviation UNLESS it is followed by a sentence start
/// (an uppercase letter) — that marks a real boundary, e.g. a figure-panel letter ending a clause:
/// `… (Extended Data Fig. 1d, e). MSI …` (the letter is `e)`, alnum-reduced to `e`; the following
/// `M` of `MSI` is the boundary signal). An initialism followed by a lowercase letter is the
/// abbreviation case (`e.g. in`, `1,000 r.p.m. for 5 min`).
fn is_abbrev(word: &str, next: char) -> bool {
    let w: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
    let w = w.to_lowercase();
    if ABBREV.contains(&w.as_str()) {
        return true;
    }
    let initialism = word
        .split('.')
        .all(|part| part.chars().filter(|c| c.is_alphanumeric()).count() == 1);
    initialism && !next.is_uppercase()
}

/// Split a document into sentence units. A `.` ends a sentence EXCEPT inside a word (a decimal
/// `0.56`, a host name `depmap.org`) or after an abbreviation or initialism (`Fig.`, `et al.`,
/// `e.g.`, `r.p.m.`);
/// `!` and `?` always end one. None of the three ends a sentence inside an open parenthesis or
/// bracket: a parenthetical belongs to the sentence around it, so `(Chr. 3+5)`, `(Extended Data
/// Figs. 6b, e)` and `(https://portals.broadinstitute.org/gpp/public/)` stay whole. (Text-only S0:
/// equation/citation/table routing is a later refinement; this is the prose path.)
pub fn segment_sentences(doc: &str) -> Vec<String> {
    let chars: Vec<char> = doc.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    for i in 0..chars.len() {
        let boundary = match chars[i] {
            '(' | '[' => {
                depth += 1;
                false
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ if depth > 0 => false,
            '!' | '?' => true,
            '.' => {
                let next = chars.get(i + 1).copied().unwrap_or(' ');
                if next.is_alphanumeric() {
                    // Word-internal: a decimal (`0.56`), a host or file name (`depmap.org`,
                    // `cloud.html`), a DOI, or inside an initialism (`r.p.m.`).
                    false
                } else {
                    // The next NON-whitespace char disambiguates a single-letter abbreviation/initial
                    // from a real boundary (an uppercase start). `'\0'` = end-of-text.
                    let next_word = chars[i + 1..]
                        .iter()
                        .copied()
                        .find(|c| !c.is_whitespace())
                        .unwrap_or('\0');
                    let seg: String = chars[start..i].iter().collect();
                    !is_abbrev(seg.split_whitespace().next_back().unwrap_or(""), next_word)
                }
            }
            _ => false,
        };
        if boundary {
            let s: String = chars[start..=i].iter().collect();
            if !s.trim().is_empty() {
                out.push(s.trim().to_string());
            }
            start = i + 1;
        }
    }
    let tail: String = chars[start..].iter().collect();
    if !tail.trim().is_empty() {
        out.push(tail.trim().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_on_real_boundaries_only() {
        assert_eq!(
            segment_sentences("A dog sees a bird. A cat sees a fish."),
            ["A dog sees a bird.", "A cat sees a fish."]
        );
    }

    #[test]
    fn does_not_split_decimals_or_abbreviations() {
        // decimal, "Fig.", "et al.", and "e.g." must not end a sentence.
        let s = segment_sentences(
            "We saw a 0.56-fold change (Fig. 1a). Chan et al. report this, e.g. in colon.",
        );
        assert_eq!(
            s.len(),
            2,
            "two sentences, not split on 0.56/Fig./et al./e.g.; got {s:?}"
        );
    }

    #[test]
    fn splits_after_a_figure_panel_letter_ending_a_sentence() {
        // D62 §2 S0-c: `… (Extended Data Fig. 1d, e). MSI …` — the panel letter `e)` was alnum-reduced
        // to a single `e` and treated as an initial, MERGING the two sentences (unit-10 over-merge).
        // A single letter followed by an UPPERCASE start is a real boundary.
        let s = segment_sentences(
            "We evaluated MSI (Extended Data Fig. 1d, e). MSI is most commonly observed in cancers.",
        );
        assert_eq!(
            s.len(),
            2,
            "the figure-panel letter `e).` ends the first sentence; got {s:?}"
        );
        // A bare single-letter clause-end before an uppercase start also splits.
        assert_eq!(
            segment_sentences("This is shown in panel d. The next result follows.").len(),
            2,
            "a panel letter `d.` before an uppercase start ends the sentence"
        );
    }

    #[test]
    fn a_parenthetical_does_not_end_its_sentence() {
        assert_eq!(
            segment_sentences(
                "HCT116 cells gained chromosomes 3 and 5 (Chr. 3+5), which include MLH1. \
                 The rest followed (see Extended Data Figs. 6b, e)."
            ),
            [
                "HCT116 cells gained chromosomes 3 and 5 (Chr. 3+5), which include MLH1.",
                "The rest followed (see Extended Data Figs. 6b, e)."
            ]
        );
    }

    #[test]
    fn a_host_name_or_an_initialism_does_not_end_its_sentence() {
        assert_eq!(
            segment_sentences(
                "Lines are listed at DepMap.org and are as follows. \
                 Cells were centrifuged at 1,000 r.p.m. for 5 min. \
                 Bars show the s.e.m. Code is at https://github.com/cancerdatasci/WRN."
            ),
            [
                "Lines are listed at DepMap.org and are as follows.",
                "Cells were centrifuged at 1,000 r.p.m. for 5 min.",
                "Bars show the s.e.m.",
                "Code is at https://github.com/cancerdatasci/WRN."
            ]
        );
        assert_eq!(
            segment_sentences("Blots are shown in Figs. 2e and 4a. Others were not.").len(),
            2
        );
    }

    /// Every WRN segment whose brackets do not balance, and the count. Run with
    /// `--ignored --nocapture`; it reads the gitignored texts under `references/`.
    #[test]
    #[ignore = "reads the gitignored WRN texts under references/"]
    fn list_the_wrn_segments_with_unbalanced_brackets() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../references/publications/WRN-Helicase-Nature-OCR"
        );
        for name in ["methods.txt", "letter-body.txt"] {
            let path = format!("{dir}/{name}");
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let sentences = segment_sentences(&text);
            let mut unbalanced = 0;
            for s in &sentences {
                let depth = s.chars().fold(0i32, |d, c| match c {
                    '(' | '[' => d + 1,
                    ')' | ']' => d - 1,
                    _ => d,
                });
                if depth != 0 {
                    unbalanced += 1;
                    println!("{name} [{depth:+}]: {s}");
                }
            }
            println!(
                "\n{name}: {unbalanced} of {} segments unbalanced\n",
                sentences.len()
            );
        }
    }
}
