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

//! D95 — the preprocessor: lexemes → the tokens the parser seeds from.
//!
//! [`super::lex`] deletes nothing, so every decision about the token stream is made here, in one
//! place, where it used to be fused into the tokenizer. These are the decisions the fused tokenizer
//! made (D62 S0), in its order:
//!
//! 1. **Bracketed asides.** `(…)`, `[…]` and `{…}`, depth-aware and brackets included, are dropped —
//!    an abbreviation gloss (`microsatellite instability (MSI)`), a figure reference (`(Fig. 1a)`).
//!    Nothing takes their place, so `a(b)c` is `ac`, and an unclosed opener drops the rest of the text.
//! 2. **Paired em-dash appositives.** With an even number (at least two) of U+2014, the odd segments
//!    and the dashes are dropped and the rest joined; a lone em-dash stays a separator.
//! 3. **Separators.** Whitespace, `—`, `–`, `‒`, `―` and `/` end a token. A comma is a token of its
//!    own, so list coordination can key on it.
//! 4. **Edge trimming.** Leading and trailing non-alphanumerics are dropped, and a token left empty is
//!    dropped with them.
//! 5. **Commas.** Leading and trailing commas are dropped and a run collapses to one: a comma separates
//!    content tokens, and a stray one would block a full-span parse.
//!
//! **Case is preserved.** Consumers fold where they need a lowercase key ([`Parser::has_token`],
//! `lookup_span`, the [`Lemmatizer`], `ReservedTable::kind`, `rank_key`); `all_caps_symbol` needs the
//! original, to tell the symbol `CELL` from the noun `cell` (2026-07-29).
//!
//! [`Parser::has_token`]: super::parse::Parser::has_token
//! [`Lemmatizer`]: super::lemmatizer::Lemmatizer

use std::ops::Range;

use super::lex::{lex, LexClass, Lexeme};

/// One token of a sentence, as the parser sees it: a chart position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    surface: String,
    span: Range<usize>,
    kind: TokenKind,
}

/// What a token is to the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A token the lexicon is asked about.
    Word,
    /// The list separator `,`. It has no lexical entry; coordination, apposition and comma absorption
    /// key on its position.
    Comma,
    /// A token that starts with a digit or carries no ASCII letter — `0.56`, `398`, `1a`, `10−13`: a
    /// number, a statistic or a figure panel (D62 S0). It is not a missing lexeme when the lexicon does
    /// not know it, and it still reaches the chart, where it seeds whatever the lexicon has for it.
    NonProse,
}

impl Token {
    /// The text the parser reads: the token's lexemes concatenated, with whatever the preprocessor
    /// dropped inside it left out.
    pub fn surface(&self) -> &str {
        &self.surface
    }

    /// Byte offsets of the token in the text it was preprocessed from, from its first kept lexeme to
    /// its last — so an aside dropped inside a token is inside its span.
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }

    pub fn kind(&self) -> TokenKind {
        self.kind
    }
}

/// The surfaces of `tokens`, joined by single spaces — the key a multiword lexical form is stored
/// under.
pub fn join_surfaces(tokens: &[Token]) -> String {
    let mut out = String::new();
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&t.surface);
    }
    out
}

/// Lex and preprocess `text`.
pub fn tokenize(text: &str) -> Vec<Token> {
    preprocess(text, &lex(text))
}

/// Preprocess `lexemes`, which [`lex`] made from `text`.
pub fn preprocess(text: &str, lexemes: &[Lexeme]) -> Vec<Token> {
    let kept = drop_asides(text, lexemes);
    let pieces = drop_appositives(text, &kept);

    let mut tokens: Vec<Token> = Vec::new();
    let mut run: Vec<&Lexeme> = Vec::new();
    for piece in pieces {
        let lexeme = match piece {
            Piece::Break => {
                flush(text, &mut run, &mut tokens);
                continue;
            }
            Piece::Lexeme(l) => l,
        };
        let s = lexeme.text(text);
        if lexeme.class == LexClass::Space || is_separator(s) {
            flush(text, &mut run, &mut tokens);
        } else if s == "," {
            flush(text, &mut run, &mut tokens);
            tokens.push(Token {
                surface: ",".to_string(),
                span: lexeme.span.clone(),
                kind: TokenKind::Comma,
            });
        } else {
            run.push(lexeme);
        }
    }
    flush(text, &mut run, &mut tokens);

    // Decision 5: a comma only separates content tokens.
    let is_comma = |t: &Token| t.kind == TokenKind::Comma;
    let start = tokens
        .iter()
        .position(|t| !is_comma(t))
        .unwrap_or(tokens.len());
    tokens.drain(..start);
    while tokens.last().is_some_and(is_comma) {
        tokens.pop();
    }
    tokens.dedup_by(|a, b| is_comma(a) && is_comma(b));
    tokens
}

/// A kept lexeme, or the separator that joins the segments left by dropping an appositive.
enum Piece<'a> {
    Lexeme(&'a Lexeme),
    Break,
}

/// Decision 1: drop every lexeme inside brackets, and the brackets.
fn drop_asides<'a>(text: &str, lexemes: &'a [Lexeme]) -> Vec<&'a Lexeme> {
    let mut depth = 0u32;
    let mut kept = Vec::with_capacity(lexemes.len());
    for l in lexemes {
        match l.text(text) {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth = depth.saturating_sub(1),
            _ if depth == 0 => kept.push(l),
            _ => {}
        }
    }
    kept
}

const EM_DASH: &str = "\u{2014}";

/// Decision 2: with an even number of em-dashes, drop the odd segments and the dashes.
fn drop_appositives<'a>(text: &str, kept: &[&'a Lexeme]) -> Vec<Piece<'a>> {
    let dashes = kept.iter().filter(|l| l.text(text) == EM_DASH).count();
    if dashes < 2 || !dashes.is_multiple_of(2) {
        return kept.iter().map(|l| Piece::Lexeme(l)).collect();
    }
    let mut out = Vec::with_capacity(kept.len());
    let mut segment = 0usize;
    for l in kept {
        if l.text(text) == EM_DASH {
            segment += 1;
            if segment.is_multiple_of(2) {
                out.push(Piece::Break);
            }
        } else if segment.is_multiple_of(2) {
            out.push(Piece::Lexeme(l));
        }
    }
    out
}

/// Decision 3's separators other than whitespace and the comma. (Brackets are gone by decision 1.)
fn is_separator(s: &str) -> bool {
    matches!(s, "—" | "–" | "‒" | "―" | "/")
}

/// Close the current run as a token. Decision 4: it spans its first word lexeme to its last, so the
/// non-alphanumeric lexemes at its edges are trimmed; a run with no word lexeme is dropped.
fn flush(text: &str, run: &mut Vec<&Lexeme>, tokens: &mut Vec<Token>) {
    let first = run.iter().position(|l| l.class == LexClass::Word);
    let last = run.iter().rposition(|l| l.class == LexClass::Word);
    if let (Some(first), Some(last)) = (first, last) {
        let kept = &run[first..=last];
        let surface: String = kept.iter().map(|l| l.text(text)).collect();
        let kind = if is_nonprose(&surface) {
            TokenKind::NonProse
        } else {
            TokenKind::Word
        };
        tokens.push(Token {
            span: kept[0].span.start..kept[kept.len() - 1].span.end,
            surface,
            kind,
        });
    }
    run.clear();
}

/// A number, statistic or figure panel: it starts with a digit or carries no ASCII letter. Gene-like
/// letter+digit symbols (`MLH1`, `BRCA1`) start with a letter and are words.
fn is_nonprose(surface: &str) -> bool {
    let first = surface.chars().next().unwrap_or(' ');
    first.is_ascii_digit() || !surface.chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surfaces(text: &str) -> Vec<String> {
        tokenize(text)
            .into_iter()
            .map(|t| t.surface().to_string())
            .collect()
    }

    #[test]
    fn preserves_case_and_strips_edge_punctuation() {
        assert_eq!(
            surfaces("HeLa depends on BRCA1."),
            ["HeLa", "depends", "on", "BRCA1"]
        );
        assert_eq!(surfaces("  A,  b!  "), ["A", ",", "b"]);
        assert!(tokenize("   ").is_empty());
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn preserves_list_commas_and_drops_dangling() {
        assert_eq!(
            surfaces("a, b, c and d"),
            ["a", ",", "b", ",", "c", "and", "d"]
        );
        assert_eq!(surfaces("a,, b,"), ["a", ",", "b"]);
        assert_eq!(surfaces(", a"), ["a"]);
    }

    #[test]
    fn keeps_internal_alphanumerics() {
        assert_eq!(surfaces("p53, (BRCA1)"), ["p53"]);
        assert_eq!(surfaces("0.56-fold WRN's"), ["0.56-fold", "WRN's"]);
    }

    #[test]
    fn drops_bracketed_asides() {
        assert_eq!(
            surfaces("microsatellite instability (MSI) results"),
            ["microsatellite", "instability", "results"]
        );
        assert_eq!(
            surfaces("poly(ADP(x)-ribose) polymerase"),
            ["poly", "polymerase"]
        );
        assert_eq!(
            surfaces("lethality\u{2014}an interaction here\u{2014}can be exploited"),
            ["lethality", "can", "be", "exploited"]
        );
        assert_eq!(surfaces("not\u{2014}can"), ["not", "can"]);
    }

    #[test]
    fn kinds_follow_the_nonprose_rule() {
        for stat in ["10", "0.56", "1a", "398", "45", "10−13"] {
            assert_eq!(tokenize(stat)[0].kind(), TokenKind::NonProse, "{stat}");
        }
        for word in ["MLH1", "msh2", "BRCA1", "PARP", "WRN", "helicase"] {
            assert_eq!(tokenize(word)[0].kind(), TokenKind::Word, "{word}");
        }
        assert_eq!(tokenize("a, b")[1].kind(), TokenKind::Comma);
    }

    #[test]
    fn a_span_reaches_back_into_the_text() {
        let text = "Cells, at 37 °C (MSI) a(b)c";
        let toks = tokenize(text);
        assert_eq!(
            toks.iter().map(|t| &text[t.span()]).collect::<Vec<_>>(),
            ["Cells", ",", "at", "37", "C", "a(b)c"]
        );
        assert_eq!(toks[5].surface(), "ac");
    }

    // ── The differential oracle ─────────────────────────────────────────────────────────────────
    // `legacy_tokenize` is `segment::tokenize` as it stood before D95 split lexing from preprocessing,
    // verbatim. Slice 1 is behaviour-preserving, so the preprocessed surfaces must equal it on every
    // input. Slice 2 changes behaviour and retires the oracle.

    fn legacy_tokenize(text: &str) -> Vec<String> {
        let mut spaced = String::with_capacity(text.len());
        for c in legacy_strip_bracketed_asides(text).chars() {
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
        while toks.first().is_some_and(|t| t == ",") {
            toks.remove(0);
        }
        while toks.last().is_some_and(|t| t == ",") {
            toks.pop();
        }
        toks.dedup_by(|a, b| a == "," && b == ",");
        toks
    }

    fn legacy_strip_bracketed_asides(text: &str) -> String {
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
        let parts: Vec<&str> = no_parens.split('\u{2014}').collect();
        if parts.len() >= 3 && parts.len() % 2 == 1 {
            parts
                .iter()
                .step_by(2)
                .copied()
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            no_parens
        }
    }

    fn legacy_is_nonprose(token: &str) -> bool {
        let first = token.chars().next().unwrap_or(' ');
        first.is_ascii_digit() || !token.chars().any(|c| c.is_ascii_alphabetic())
    }

    fn assert_matches_legacy(text: &str) {
        let toks = tokenize(text);
        let got: Vec<&str> = toks.iter().map(Token::surface).collect();
        assert_eq!(got, legacy_tokenize(text), "{text:?}");
        for t in &toks {
            let kind = match t.surface() {
                "," => TokenKind::Comma,
                s if legacy_is_nonprose(s) => TokenKind::NonProse,
                _ => TokenKind::Word,
            };
            assert_eq!(t.kind(), kind, "{text:?}: {:?}", t.surface());
        }
    }

    /// Every character class the old tokenizer treated differently: letters, digits, whitespace
    /// (including a no-break space), each separator, each bracket, the comma, edge punctuation, a
    /// combining mark, and a non-ASCII letter and digit.
    const ALPHABET: &[char] = &[
        'a', 'B', '1', ' ', '\u{a0}', ',', '(', ')', '[', '}', '\u{2014}', '–', '‒', '―', '/', '-',
        '.', '°', '\u{301}', 'μ', '¹', '\'',
    ];

    #[test]
    fn matches_the_legacy_tokenizer_on_every_string_up_to_length_four() {
        let n = ALPHABET.len();
        let mut total = 0usize;
        for len in 0..=4u32 {
            for mut code in 0..n.pow(len) {
                let mut s = String::new();
                for _ in 0..len {
                    s.push(ALPHABET[code % n]);
                    code /= n;
                }
                assert_matches_legacy(&s);
                total += 1;
            }
        }
        assert_eq!(total, (0..=4u32).map(|l| n.pow(l)).sum::<usize>());
    }

    #[test]
    fn matches_the_legacy_tokenizer_on_long_random_strings() {
        // A fixed LCG, so a failure reproduces.
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        for _ in 0..20_000 {
            let len = next() % 40;
            let s: String = (0..len)
                .map(|_| ALPHABET[next() % ALPHABET.len()])
                .collect();
            assert_matches_legacy(&s);
        }
    }

    #[test]
    fn matches_the_legacy_tokenizer_on_prose() {
        for s in [
            "HeLa depends on BRCA1.",
            "These classifications were highly concordant with PCR-based MSI phenotyping and with predicted MMR deficiency.",
            "Germline mutations in the MMR genes (MLH1, MSH2, MSH6 and PMS2) cause Lynch syndrome.",
            "lethality\u{2014}an interaction in which the loss of either gene is tolerated\u{2014}can be exploited",
            "…the plates were spun at 931g for 2 h at 30 °C (Fig. 2g).",
            "a dose of 5 mg/dL, 20–30% and 0.56-fold; log2(copy number) < -1",
            "P = 4.2 × 10⁻¹³, 45-60% of such cancers, 10 μg ml⁻¹ of gentamicin",
            "unclosed (aside runs to the end",
            "stray ) closer and {mixed] brackets",
            "colon, gastric, endometrial and ovarian cancers",
            "WRN , which is a helicase , affects HeLa",
        ] {
            assert_matches_legacy(s);
        }
    }

    /// The gitignored WRN texts, sentence by sentence and whole, when a checkout has them.
    #[test]
    #[ignore = "reads the gitignored WRN texts under references/"]
    fn matches_the_legacy_tokenizer_on_the_wrn_texts() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../references/publications/WRN-Helicase-Nature-OCR"
        );
        for name in ["methods.txt", "letter-body.txt"] {
            let path = format!("{dir}/{name}");
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_matches_legacy(&text);
            for s in super::super::segment::segment_sentences(&text) {
                assert_matches_legacy(&s);
            }
        }
    }
}
