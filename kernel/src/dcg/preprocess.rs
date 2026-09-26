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
//! place, in this order:
//!
//! 1. **Brackets.** Each closer pairs with the latest open opener, whatever the bracket types.
//!    - A pair whose opener follows a word lexeme directly is an **argument** — `log2(copy number)`,
//!      `poly(ADP-ribose)`: its content stays, and its brackets become [`TokenKind::NonProse`]
//!      tokens, so the sentence reaches no parse rather than a parse without the argument.
//!    - Any other pair is a **gloss** — `microsatellite instability (MSI)`, `(Fig. 1a)` — and is
//!      dropped with its content, leaving a separator (D62 S0).
//!    - An unmatched opener or closer is a `NonProse` token, and the text around it stays.
//! 2. **Paired em-dash appositives.** With an even number (at least two) of U+2014, the odd segments
//!    and the dashes are dropped and the rest joined; a lone em-dash stays a separator.
//! 3. **Separators.** Whitespace, `—`, `–`, `‒`, `―` and `/` end a token. A comma is a token of its
//!    own, so list coordination can key on it — except between a digit group and exactly three digits
//!    (`1,200`), where it groups digits.
//! 4. **Edge trimming.** Leading and trailing non-alphanumerics are dropped, and a token left empty is
//!    dropped with them, with two exceptions: an operator (`<`, `≤`, `=`, `±`, `×`, …) becomes a
//!    `NonProse` token of its own, and a sign directly before a numeral joins it (`−1`).
//! 5. **Commas.** Leading and trailing commas are dropped and a run collapses to one: a comma separates
//!    content tokens, and a stray one would block a full-span parse.
//! 6. **Kinds.** A numeral is [`TokenKind::Numeral`]. A token that starts with a digit and is not one
//!    (`53BP1`, `5-fold`, `1a`) is a [`TokenKind::Word`], so a lexicon without it reports it missing
//!    (D95, decision 3). A token with no ASCII letter is `NonProse`.
//!
//! **Case is preserved.** Consumers fold where they need a lowercase key ([`Parser::has_token`],
//! `lookup_span`, the [`Lemmatizer`], `ReservedTable::kind`, `rank_key`); `all_caps_symbol` needs the
//! original, to tell the symbol `CELL` from the noun `cell` (2026-07-29).
//!
//! [`Parser::has_token`]: super::parse::Parser::has_token
//! [`Lemmatizer`]: super::lemmatizer::Lemmatizer

use std::ops::Range;

use super::lex::{lex, LexClass, Lexeme};
use crate::numeric::Rational;

/// One token of a sentence, as the parser sees it: a chart position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    surface: String,
    span: Range<usize>,
    kind: TokenKind,
}

/// What a token is to the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// A token the lexicon is asked about. With no entry it is a missing lexeme.
    Word,
    /// The list separator `,`. It has no lexical entry; coordination, apposition and comma absorption
    /// key on its position.
    Comma,
    /// A number, exactly: `37`, `0.56`, `−1`, `1,200`.
    Numeral(Rational),
    /// A token the grammar has no reading for and the lexicon is not expected to know: an operator
    /// (`<`, `=`, `±`), a bracket kept around an argument or left unmatched, a token with no ASCII
    /// letter (`μ`).
    NonProse,
}

impl Token {
    /// The text the parser reads: the token's lexemes concatenated, with whatever the preprocessor
    /// dropped inside it left out.
    pub fn surface(&self) -> &str {
        &self.surface
    }

    /// Byte offsets of the token in the text it was preprocessed from, from its first kept lexeme to
    /// its last.
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }

    pub fn kind(&self) -> &TokenKind {
        &self.kind
    }

    pub fn is_word(&self) -> bool {
        self.kind == TokenKind::Word
    }

    pub fn is_comma(&self) -> bool {
        self.kind == TokenKind::Comma
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
    let pieces = drop_appositives(text, resolve_brackets(text, lexemes));

    let mut tokens: Vec<Token> = Vec::new();
    let mut run: Vec<&Lexeme> = Vec::new();
    for (k, piece) in pieces.iter().enumerate() {
        let lexeme = match piece {
            Piece::Break => {
                flush(text, &mut run, &mut tokens);
                continue;
            }
            Piece::Symbol(l) => {
                flush(text, &mut run, &mut tokens);
                tokens.push(symbol(text, l));
                continue;
            }
            Piece::Lexeme(l) => *l,
        };
        let s = lexeme.text(text);
        if lexeme.class == LexClass::Space || is_separator(s) {
            flush(text, &mut run, &mut tokens);
        } else if s == "," && groups_digits(text, &run, pieces.get(k + 1)) {
            run.push(lexeme);
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
    let start = tokens
        .iter()
        .position(|t| !t.is_comma())
        .unwrap_or(tokens.len());
    tokens.drain(..start);
    while tokens.last().is_some_and(Token::is_comma) {
        tokens.pop();
    }
    tokens.dedup_by(|a, b| a.is_comma() && b.is_comma());
    tokens
}

/// A lexeme to consider, a separator left where a gloss or appositive was dropped, or a lexeme that
/// is a token of its own (a kept or unmatched bracket).
enum Piece<'a> {
    Lexeme(&'a Lexeme),
    Break,
    Symbol(&'a Lexeme),
}

fn is_opener(s: &str) -> bool {
    matches!(s, "(" | "[" | "{")
}

fn is_closer(s: &str) -> bool {
    matches!(s, ")" | "]" | "}")
}

/// Decision 1.
fn resolve_brackets<'a>(text: &str, lexemes: &'a [Lexeme]) -> Vec<Piece<'a>> {
    let mut partner: Vec<Option<usize>> = vec![None; lexemes.len()];
    let mut open: Vec<usize> = Vec::new();
    for (i, l) in lexemes.iter().enumerate() {
        let s = l.text(text);
        if is_opener(s) {
            open.push(i);
        } else if is_closer(s) {
            if let Some(o) = open.pop() {
                partner[o] = Some(i);
                partner[i] = Some(o);
            }
        }
    }
    let mut out = Vec::with_capacity(lexemes.len());
    let mut i = 0;
    while i < lexemes.len() {
        let l = &lexemes[i];
        let s = l.text(text);
        if is_opener(s) {
            let argument = i > 0 && lexemes[i - 1].class == LexClass::Word;
            match partner[i] {
                Some(close) if !argument => {
                    out.push(Piece::Break);
                    i = close + 1;
                    continue;
                }
                _ => out.push(Piece::Symbol(l)),
            }
        } else if is_closer(s) {
            // A gloss's closer was skipped with it, so this closes an argument or nothing.
            out.push(Piece::Symbol(l));
        } else {
            out.push(Piece::Lexeme(l));
        }
        i += 1;
    }
    out
}

const EM_DASH: &str = "\u{2014}";

/// Decision 2: with an even number of em-dashes, drop the odd segments and the dashes.
fn drop_appositives<'a>(text: &str, pieces: Vec<Piece<'a>>) -> Vec<Piece<'a>> {
    let is_dash = |p: &Piece| matches!(p, Piece::Lexeme(l) if l.text(text) == EM_DASH);
    let dashes = pieces.iter().filter(|p| is_dash(p)).count();
    if dashes < 2 || !dashes.is_multiple_of(2) {
        return pieces;
    }
    let mut out = Vec::with_capacity(pieces.len());
    let mut segment = 0usize;
    for p in pieces {
        if is_dash(&p) {
            segment += 1;
            if segment.is_multiple_of(2) {
                out.push(Piece::Break);
            }
        } else if segment.is_multiple_of(2) {
            out.push(p);
        }
    }
    out
}

/// Decision 3's separators other than whitespace and the comma.
fn is_separator(s: &str) -> bool {
    matches!(s, "—" | "–" | "‒" | "―" | "/")
}

fn is_digits(text: &str, l: &Lexeme) -> bool {
    l.class == LexClass::Word && l.text(text).bytes().all(|b| b.is_ascii_digit())
}

/// Whether a comma after `run` groups digits: the run from its first word lexeme is digit groups
/// separated by commas — the first of one to three digits, the rest of three — and the next piece
/// is exactly three digits.
fn groups_digits(text: &str, run: &[&Lexeme], next: Option<&Piece>) -> bool {
    let Some(Piece::Lexeme(next)) = next else {
        return false;
    };
    if !is_digits(text, next) || next.text(text).len() != 3 {
        return false;
    }
    let Some(first) = run.iter().position(|l| l.class == LexClass::Word) else {
        return false;
    };
    let body = &run[first..];
    body.iter().enumerate().all(|(k, l)| {
        if k % 2 == 1 {
            return l.text(text) == ",";
        }
        let n = l.text(text).len();
        is_digits(text, l) && if k == 0 { (1..=3).contains(&n) } else { n == 3 }
    }) && body.len() % 2 == 1
}

/// A symbol that states a relation or an operation, which edge trimming must not drop: dropping it
/// turns `< −1` into the number `−1`.
fn is_operator(s: &str) -> bool {
    matches!(
        s,
        "<" | ">" | "≤" | "≥" | "=" | "≠" | "≈" | "~" | "±" | "×" | "\u{2212}"
    )
}

fn is_sign(s: &str) -> bool {
    matches!(s, "-" | "\u{2212}")
}

fn symbol(text: &str, l: &Lexeme) -> Token {
    Token {
        surface: l.text(text).to_string(),
        span: l.span.clone(),
        kind: TokenKind::NonProse,
    }
}

/// Close the current run. Decision 4: the token spans its first word lexeme to its last, taking a
/// sign directly before a numeral; operators at its edges become tokens of their own; the other
/// non-alphanumerics at its edges are dropped, and a run with no word lexeme leaves only its operators.
fn flush(text: &str, run: &mut Vec<&Lexeme>, tokens: &mut Vec<Token>) {
    let first = run.iter().position(|l| l.class == LexClass::Word);
    let last = run.iter().rposition(|l| l.class == LexClass::Word);
    let (Some(first), Some(last)) = (first, last) else {
        for l in run.iter().filter(|l| is_operator(l.text(text))) {
            tokens.push(symbol(text, l));
        }
        run.clear();
        return;
    };
    let concat = |ls: &[&Lexeme]| -> String { ls.iter().map(|l| l.text(text)).collect() };
    let signed = first > 0
        && is_sign(run[first - 1].text(text))
        && numeral_value(&concat(&run[first - 1..=last])).is_some();
    let start = if signed { first - 1 } else { first };
    for l in run[..start].iter().filter(|l| is_operator(l.text(text))) {
        tokens.push(symbol(text, l));
    }
    let surface = concat(&run[start..=last]);
    let kind = match numeral_value(&surface) {
        Some(value) => TokenKind::Numeral(value),
        None if surface.starts_with(|c: char| c.is_ascii_digit()) => TokenKind::Word,
        None if !surface.chars().any(|c| c.is_ascii_alphabetic()) => TokenKind::NonProse,
        None => TokenKind::Word,
    };
    tokens.push(Token {
        span: run[start].span.start..run[last].span.end,
        surface,
        kind,
    });
    for l in run[last + 1..].iter().filter(|l| is_operator(l.text(text))) {
        tokens.push(symbol(text, l));
    }
    run.clear();
}

/// The value of a numeral: an optional sign (`-`, `−`), digits — plain, or grouped by commas in
/// threes — and an optional decimal part. `None` for anything else, including a numeral too large to
/// admit exactly.
fn numeral_value(s: &str) -> Option<Rational> {
    let (negative, body) = match s.strip_prefix('-').or_else(|| s.strip_prefix('\u{2212}')) {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let all_digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let mut groups = int.split(',');
    let head = groups.next()?;
    let rest: Vec<&str> = groups.collect();
    let grouped = !rest.is_empty();
    if !all_digits(head)
        || (grouped && head.len() > 3)
        || !rest.iter().all(|g| all_digits(g) && g.len() == 3)
    {
        return None;
    }
    if frac.is_some_and(|f| !all_digits(f)) {
        return None;
    }
    let mut normal = String::with_capacity(s.len());
    if negative {
        normal.push('-');
    }
    normal.push_str(head);
    for g in rest {
        normal.push_str(g);
    }
    if let Some(f) = frac {
        normal.push('.');
        normal.push_str(f);
    }
    Rational::parse_decimal(&normal).ok()
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

    fn rat(s: &str) -> Rational {
        Rational::parse_decimal(s).unwrap()
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
    fn drops_glosses_and_appositives() {
        assert_eq!(
            surfaces("microsatellite instability (MSI) results"),
            ["microsatellite", "instability", "results"]
        );
        assert_eq!(
            surfaces("lethality\u{2014}an interaction here\u{2014}can be exploited"),
            ["lethality", "can", "be", "exploited"]
        );
        assert_eq!(surfaces("not\u{2014}can"), ["not", "can"]);
        // A dropped gloss leaves a separator.
        assert_eq!(surfaces("x.(y)z"), ["x", "z"]);
    }

    #[test]
    fn keeps_an_argument_and_its_brackets() {
        let toks = tokenize("log2(copy number) < -1");
        assert_eq!(
            toks.iter().map(Token::surface).collect::<Vec<_>>(),
            ["log2", "(", "copy", "number", ")", "<", "-1"]
        );
        assert_eq!(toks[1].kind(), &TokenKind::NonProse);
        assert_eq!(toks[4].kind(), &TokenKind::NonProse);
        assert_eq!(toks[5].kind(), &TokenKind::NonProse);
        assert_eq!(toks[6].kind(), &TokenKind::Numeral(rat("-1")));
        assert_eq!(
            surfaces("poly(ADP-ribose) polymerase"),
            ["poly", "(", "ADP-ribose", ")", "polymerase"]
        );
        // A gloss inside an argument is still dropped.
        assert_eq!(
            surfaces("log2(copy number (CN))"),
            ["log2", "(", "copy", "number", ")"]
        );
    }

    #[test]
    fn an_unmatched_bracket_is_a_token_and_the_text_stays() {
        assert_eq!(
            surfaces("unclosed (aside runs on"),
            ["unclosed", "(", "aside", "runs", "on"]
        );
        assert_eq!(surfaces("a stray ) closer"), ["a", "stray", ")", "closer"]);
    }

    #[test]
    fn numerals() {
        for (text, value) in [
            ("37", "37"),
            ("0.56", "0.56"),
            ("-1", "-1"),
            ("\u{2212}1", "-1"),
            ("1,200", "1200"),
            ("12,345,678.5", "12345678.5"),
            ("at 30.", "30"),
        ] {
            let toks = tokenize(text);
            let last = toks.last().unwrap();
            assert_eq!(last.kind(), &TokenKind::Numeral(rat(value)), "{text:?}");
        }
        // A comma that does not group digits separates.
        assert_eq!(
            surfaces("genes 1,2 and 3"),
            ["genes", "1", ",", "2", "and", "3"]
        );
        assert_eq!(surfaces("1,2345"), ["1", ",", "2345"]);
        assert_eq!(surfaces("1234,567"), ["1234", ",", "567"]);
        // A hyphen inside a token is not a sign.
        assert_eq!(tokenize("5-3")[0].kind(), &TokenKind::Word);
    }

    #[test]
    fn a_digit_initial_token_that_is_not_a_numeral_is_a_word() {
        for word in [
            "53BP1",
            "5-fold",
            "1a",
            "0.56-fold",
            "10\u{207b}\u{b9}\u{b3}",
            "45-60",
        ] {
            assert_eq!(tokenize(word)[0].kind(), &TokenKind::Word, "{word}");
        }
        for word in ["MLH1", "BRCA1", "WRN", "HEK293T"] {
            assert_eq!(tokenize(word)[0].kind(), &TokenKind::Word, "{word}");
        }
    }

    #[test]
    fn operators_are_tokens_of_their_own() {
        let toks = tokenize("P = 4.2 × 10, n ≈ ~5 and 5 ± 1");
        assert_eq!(
            toks.iter().map(Token::surface).collect::<Vec<_>>(),
            ["P", "=", "4.2", "×", "10", ",", "n", "≈", "~", "5", "and", "5", "±", "1"]
        );
        for i in [1, 3, 7, 8, 12] {
            assert_eq!(
                toks[i].kind(),
                &TokenKind::NonProse,
                "{}",
                toks[i].surface()
            );
        }
        // Inside a token an operator stays in it.
        assert_eq!(surfaces("P<0.05"), ["P<0.05"]);
        // `<−1`: the operator splits off and the sign joins the numeral.
        let toks = tokenize("<\u{2212}1");
        assert_eq!(toks[0].surface(), "<");
        assert_eq!(toks[1].kind(), &TokenKind::Numeral(rat("-1")));
    }

    #[test]
    fn kinds() {
        assert_eq!(tokenize("μ")[0].kind(), &TokenKind::NonProse);
        assert_eq!(tokenize("helicase")[0].kind(), &TokenKind::Word);
        assert_eq!(tokenize("a, b")[1].kind(), &TokenKind::Comma);
    }

    #[test]
    fn a_span_reaches_back_into_the_text() {
        let text = "Cells, at 37 °C (MSI) log(x)";
        let toks = tokenize(text);
        assert_eq!(
            toks.iter().map(|t| &text[t.span()]).collect::<Vec<_>>(),
            ["Cells", ",", "at", "37", "C", "log", "(", "x", ")"]
        );
    }

    // ── The legacy tokenizer ────────────────────────────────────────────────────────────────────
    // `segment::tokenize` as it stood before D95 split lexing from preprocessing, verbatim. Slice 1
    // equalled it on every input. Slice 2 changes brackets, operators, signs, digit grouping and
    // kinds, so it is an oracle only over text with none of those; over the WRN texts it is the
    // reference the changes are listed against.

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

    /// Every character class the old tokenizer treated differently, less the ones slice 2 changes:
    /// no digit, no bracket, no operator. (A combining mark and `μ` are letterless, so they exercise
    /// `NonProse`.)
    const ALPHABET: &[char] = &[
        'a', 'B', ' ', '\u{a0}', ',', '\u{2014}', '–', '‒', '―', '/', '-', '.', '°', '\u{301}',
        'μ', '\'',
    ];

    fn assert_matches_legacy(text: &str) {
        let toks = tokenize(text);
        let got: Vec<&str> = toks.iter().map(Token::surface).collect();
        assert_eq!(got, legacy_tokenize(text), "{text:?}");
        for t in &toks {
            let kind = match t.surface() {
                "," => TokenKind::Comma,
                s if !s.chars().any(|c| c.is_ascii_alphabetic()) => TokenKind::NonProse,
                _ => TokenKind::Word,
            };
            assert_eq!(t.kind(), &kind, "{text:?}: {:?}", t.surface());
        }
    }

    #[test]
    fn matches_the_legacy_tokenizer_where_slice_2_changes_nothing() {
        let n = ALPHABET.len();
        for len in 0..=4u32 {
            for mut code in 0..n.pow(len) {
                let mut s = String::new();
                for _ in 0..len {
                    s.push(ALPHABET[code % n]);
                    code /= n;
                }
                assert_matches_legacy(&s);
            }
        }
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        for _ in 0..20_000 {
            let len = next() % 40;
            let s: String = (0..len).map(|_| ALPHABET[next() % n]).collect();
            assert_matches_legacy(&s);
        }
    }

    /// Every WRN sentence whose token stream slice 2 changes, with the legacy stream beside it.
    /// Run with `--ignored --nocapture`; it reads the gitignored texts under `references/`.
    #[test]
    #[ignore = "reads the gitignored WRN texts under references/"]
    fn list_the_wrn_sentences_slice_2_changes() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../references/publications/WRN-Helicase-Nature-OCR"
        );
        for name in ["methods.txt", "letter-body.txt"] {
            let path = format!("{dir}/{name}");
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let sentences = super::super::segment::segment_sentences(&text);
            let mut changed = 0;
            for s in &sentences {
                let now = tokenize(s);
                let was = legacy_tokenize(s);
                if now
                    .iter()
                    .map(Token::surface)
                    .ne(was.iter().map(String::as_str))
                {
                    changed += 1;
                    println!("\n{name}: {s}");
                    println!("  was: {}", was.join(" "));
                    println!(
                        "  now: {}",
                        now.iter()
                            .map(|t| match t.kind() {
                                TokenKind::Numeral(_) => format!("#{}", t.surface()),
                                TokenKind::NonProse => format!("!{}", t.surface()),
                                _ => t.surface().to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                }
            }
            println!(
                "\n{name}: {changed} of {} sentences changed",
                sentences.len()
            );
        }
    }
}
