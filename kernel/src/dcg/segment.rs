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
//! The front of the encoding pipeline: **splitting text into the units the parser can attempt**, at
//! both granularities. A document is split into sentence units ([`segment_sentences`]); a sentence is
//! split into word tokens ([`tokenize`]). Tokens that are not prose (statistics, figure references) are
//! flagged so the parser skips them ([`is_nonprose`]).
//!
//! Both are segmentation, and they were in different modules — `tokenize` sat inside the parser, which
//! meant the parser owned a decision about *text* rather than about *grammar*.
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

/// Whether `word`'s trailing `.` is an abbreviation period (so not a sentence boundary): a known
/// abbreviation, or a single letter (an initial, or one half of `e.g.`/`i.e.`). `next` is the next
/// **non-whitespace** char after the period (or `'\0'` at end-of-text). A single letter is an
/// abbreviation/initial UNLESS it is followed by a sentence start (an uppercase letter) — that marks
/// a real boundary, e.g. a figure-panel letter ending a clause: `… (Extended Data Fig. 1d, e). MSI …`
/// (the letter is `e)`, alnum-reduced to `e`; the following `M` of `MSI` is the boundary signal). A
/// single letter followed by a lowercase letter is the abbreviation case (`e.g.` → `g`).
fn is_abbrev(word: &str, next: char) -> bool {
    let w: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
    let w = w.to_lowercase();
    if ABBREV.contains(&w.as_str()) {
        return true;
    }
    w.chars().count() == 1 && !next.is_uppercase()
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

/// Whether a (already-tokenized, lowercased) token is **non-prose** — a number, statistic,
/// percentage, or figure reference — and should be routed out of the parse rather than
/// treated as a lexeme. These start with a digit or carry no letters (`10−13`, `0.56`,
/// `1a`, `398`, `45`). Gene-like letter+digit symbols (`mlh1`, `msh2`, `brca1`, `parp`) start
/// with a letter and are NOT non-prose — they are content the domain lexicon resolves.
pub fn is_nonprose(token: &str) -> bool {
    let first = token.chars().next().unwrap_or(' ');
    first.is_ascii_digit() || !token.chars().any(|c| c.is_ascii_alphabetic())
}

/// Split prose into word tokens, **preserving the source's case**. Token-internal **separators** —
/// em/en-dashes (`—`/`–`), slashes, and brackets — are normalised to spaces first, so `"not—can"` →
/// `["not", "can"]` and `"and/or"` → `["and", "or"]` (D62 S0). Hyphens (`-`) are kept, so
/// hyphenated compounds (`"double-stranded"`) stay intact. Each token is then trimmed of
/// leading/trailing non-alphanumerics (so `"BRCA1,"` → `"BRCA1"`); empties are dropped.
/// Multiword forms are recovered by re-joining spans at lookup time, not here.
///
/// **CASE IS PRESERVED HERE, and folded where a lowercase key is actually wanted** (2026-07-29).
/// This function used to `to_lowercase()` every token, which destroyed the distinction between a
/// nomenclature SYMBOL and the common noun it spells before any consumer could see it: `CELL` (HGNC
/// `NS` for the CELP pseudogene) became indistinguishable from `cell`, so `MSI cell lines…` read
/// `cell` as a GENE in 16 of its 48 skeletons. Case-insensitive LOOKUP is still right — the lexical
/// index stays lowercase-keyed, and sentence-initial `Cell lines…` must reach the lemma `cell` — so
/// every consumer that needs a key lowercases at the point of use ([`Parser::has_token`],
/// `lookup_span`'s `s_lc`, the [`Lemmatizer`](super::lemmatizer::Lemmatizer), `ReservedTable::kind`,
/// `rank_key`). The one consumer that needs the ORIGINAL is
/// [`all_caps_symbol`](super::parse::all_caps_symbol), which is the whole point.
pub fn tokenize(text: &str) -> Vec<String> {
    // Bracket/dash/slash separators → spaces; the **comma** is preserved as a standalone `,` token
    // (D62 S0) so the parser can key multi-item list coordination on it. Other punctuation is still
    // trimmed off token edges.
    let mut spaced = String::with_capacity(text.len());
    for c in strip_bracketed_asides(text).chars() {
        match c {
            '—' | '–' | '‒' | '―' | '/' | '(' | ')' | '[' | ']' | '{' | '}' => {
                spaced.push(' ')
            }
            ',' => spaced.push_str(" , "),
            other => spaced.push(other),
        }
    }
    let mut toks: Vec<String> = spaced
        .split_whitespace()
        .filter_map(|t| {
            if t == "," {
                Some(",".to_string())
            } else {
                let s = t.trim_matches(|c: char| !c.is_alphanumeric());
                (!s.is_empty()).then_some(s.to_string())
            }
        })
        .collect();
    // A comma is only a separator BETWEEN content tokens: drop dangling (leading/trailing) commas
    // and collapse runs, so a stray `,` never blocks a full-span parse.
    while toks.first().is_some_and(|t| t == ",") {
        toks.remove(0);
    }
    while toks.last().is_some_and(|t| t == ",") {
        toks.pop();
    }
    toks.dedup_by(|a, b| a == "," && b == ",");
    toks
}

/// Drop **bracketed asides** before tokenizing (D62 S0): parenthetical `(…)`/`[…]`/`{…}` glosses
/// (depth-aware) and **em-dash-bracketed appositives** `—…—` (paired U+2014). These are droppable
/// for a *scientific claim* — an abbreviation gloss (`microsatellite instability (MSI)`), a figure
/// ref (`(Fig. 1a)`), or a defining appositive (`lethality—an interaction…—can be exploited`) leaves
/// the head + matrix asserting the same fact. A deliberate, recorded cut (apposition-as-renaming is
/// discourse-level, out of scope for the claim — `docs/notes/d62-grammar-gap-analysis.md`). Content
/// punctuation (commas/lists) is NOT dropped here — that is the marker-keyed list slice.
/// A single (unpaired) em-dash is left for the tokenizer to split (it isn't a bracketing pair).
fn strip_bracketed_asides(text: &str) -> String {
    // 1. Parentheticals/brackets, depth-aware (handles nesting like `poly(ADP(x))`).
    let mut no_parens = String::with_capacity(text.len());
    let mut depth = 0u32;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => no_parens.push(c),
            _ => {}
        }
    }
    // 2. Paired em-dash appositives: with an even number of `—`, the bracketed asides are the
    // odd-indexed segments; keep the even-indexed matrix. An odd count (a lone `—`) is left as-is.
    let parts: Vec<&str> = no_parens.split('\u{2014}').collect();
    if parts.len() >= 3 && parts.len() % 2 == 1 {
        parts
            .iter()
            .step_by(2) // 0, 2, 4, … = the matrix segments
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        no_parens
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokenize_preserves_case_and_strips_edge_punctuation() {
        // CASE IS PRESERVED (2026-07-29): `BRCA1` must stay distinguishable from a lowercase
        // common noun, or an all-caps nomenclature symbol becomes reachable from ordinary prose
        // (`CELL` the CELP pseudogene vs the noun `cell`). Consumers fold where they need a key.
        assert_eq!(
            tokenize("HeLa depends on BRCA1."),
            ["HeLa", "depends", "on", "BRCA1"]
        );
        // The comma between content tokens is preserved as a `,` token (D62 S0 list coordination).
        assert_eq!(tokenize("  A,  b!  "), ["A", ",", "b"]);
        assert!(tokenize("   ").is_empty());
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn tokenize_preserves_list_commas_and_drops_dangling() {
        // Internal commas survive as separators; leading/trailing/duplicate commas are dropped.
        assert_eq!(
            tokenize("a, b, c and d"),
            ["a", ",", "b", ",", "c", "and", "d"]
        );
        assert_eq!(tokenize("a,, b,"), ["a", ",", "b"]); // collapsed run + trailing dropped
        assert_eq!(tokenize(", a"), ["a"]); // leading dropped
    }

    #[test]
    fn tokenize_keeps_internal_alphanumerics() {
        // intra-token digits/letters survive; only the edges are trimmed. The `(BRCA1)` is now a
        // dropped parenthetical aside (D62 S0), so only `p53` survives.
        assert_eq!(tokenize("p53, (BRCA1)"), ["p53"]);
    }

    #[test]
    fn tokenize_drops_bracketed_asides() {
        // Parenthetical gloss dropped, head + matrix kept.
        assert_eq!(
            tokenize("microsatellite instability (MSI) results"),
            ["microsatellite", "instability", "results"]
        );
        // Nested parens dropped wholesale.
        assert_eq!(
            tokenize("poly(ADP(x)-ribose) polymerase"),
            ["poly", "polymerase"]
        );
        // Paired em-dash appositive dropped; head + matrix kept.
        assert_eq!(
            tokenize("lethality\u{2014}an interaction here\u{2014}can be exploited"),
            ["lethality", "can", "be", "exploited"]
        );
        // A single (unpaired) em-dash is NOT a bracket pair → split, both sides kept.
        assert_eq!(tokenize("not\u{2014}can"), ["not", "can"]);
    }

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
    fn nonprose_routes_stats_keeps_genes() {
        for stat in ["10", "0.56", "1a", "398", "45"] {
            assert!(is_nonprose(stat), "{stat} should be non-prose");
        }
        for gene in ["mlh1", "msh2", "brca1", "parp", "wrn", "helicase"] {
            assert!(!is_nonprose(gene), "{gene} should be kept as a lexeme");
        }
    }
}
