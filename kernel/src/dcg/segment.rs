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

//! D62 S0 — document segmentation + non-prose classification (text-only).
//!
//! The front of the encoding pipeline: a document is split into sentence units the parser
//! can attempt, and tokens that are not prose (statistics, figure references) are flagged so
//! the parser/encoder skips them. Tokenization itself (em-dash/slash/bracket splitting) lives
//! in [`super::tokenize`]; this module owns the *sentence-boundary* and *non-prose* decisions.
//!
//! Deterministic, no LLM. Verified on real paper prose in
//! `crates/eigenius-wordnet/tests/encoding_prototype.rs` (the cleaned WRN first page: a naive
//! `.`/`!`/`?` split over-segments 4 paragraphs into 47 units; this yields ~26, and routes the
//! stat/figure-ref tokens out while keeping gene symbols like `MLH1`/`MSH2`).

/// Abbreviations (and, by the single-letter guard, initials / `e.g.` / `i.e.`) whose trailing
/// `.` is NOT a sentence boundary. Lowercased, alphanumerics only.
const ABBREV: &[&str] = &[
    "fig",
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

/// Whether `word`'s trailing `.` is an abbreviation period (so not a sentence boundary): a
/// single letter (an initial, or one half of `e.g.`/`i.e.`) or a known abbreviation.
fn is_abbrev(word: &str) -> bool {
    let w: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
    let w = w.to_lowercase();
    w.chars().count() <= 1 || ABBREV.contains(&w.as_str())
}

/// Split a document into sentence units. A `.` ends a sentence EXCEPT inside a decimal
/// (`0.56`) or after an abbreviation / single-letter initial (`Fig.`, `et al.`, `e.g.`);
/// `!` and `?` always end one. (Text-only S0: equation/citation/table routing is a later
/// refinement; this is the prose path.)
pub fn segment_sentences(doc: &str) -> Vec<String> {
    let chars: Vec<char> = doc.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        let boundary = match chars[i] {
            '!' | '?' => true,
            '.' => {
                let prev = if i > 0 { chars[i - 1] } else { ' ' };
                let next = chars.get(i + 1).copied().unwrap_or(' ');
                if prev.is_ascii_digit() && next.is_ascii_digit() {
                    false // decimal point
                } else {
                    let seg: String = chars[start..i].iter().collect();
                    !is_abbrev(seg.split_whitespace().next_back().unwrap_or(""))
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

/// Whether a (already-tokenized, lowercased) token is **non-prose** — a number, statistic,
/// percentage, or figure reference — and should be routed out of the parse rather than
/// treated as a lexeme. These start with a digit or carry no letters (`10−13`, `0.56`,
/// `1a`, `398`, `45`). Gene-like letter+digit symbols (`mlh1`, `msh2`, `brca1`, `parp`) start
/// with a letter and are NOT non-prose — they are content the domain lexicon resolves.
pub fn is_nonprose(token: &str) -> bool {
    let first = token.chars().next().unwrap_or(' ');
    first.is_ascii_digit() || !token.chars().any(|c| c.is_ascii_alphabetic())
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
    fn nonprose_routes_stats_keeps_genes() {
        for stat in ["10", "0.56", "1a", "398", "45"] {
            assert!(is_nonprose(stat), "{stat} should be non-prose");
        }
        for gene in ["mlh1", "msh2", "brca1", "parp", "wrn", "helicase"] {
            assert!(!is_nonprose(gene), "{gene} should be kept as a lexeme");
        }
    }
}
