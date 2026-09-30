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
//! 3. **Separators.** Whitespace, `—`, `–`, `‒`, `―` and `/` end a token, except an en-dash between
//!    digits, which joins a range (`4–12`). A comma is a token of its own, so list coordination can
//!    key on it — except between a digit group and exactly three digits (`1,200`, `1,000g`), where it
//!    groups digits.
//! 4. **Edge trimming.** Leading and trailing non-alphanumerics are dropped, and a token left empty is
//!    dropped with them, with two exceptions: an operator (`<`, `≤`, `=`, `±`, `×`, …) becomes a
//!    `NonProse` token of its own, and a sign directly before a numeral joins it (`−1`).
//! 5. **Commas.** Leading and trailing commas are dropped and a run collapses to one: a comma separates
//!    content tokens, and a stray one would block a full-span parse.
//! 6. **Kinds.** A numeral is [`TokenKind::Numeral`]. A token that starts with a digit and is not one
//!    (`53BP1`, `5-fold`, `1a`) is a [`TokenKind::Word`], so a lexicon without it reports it missing
//!    (D95, decision 3). A digit pair joined by an en-dash (`4–12`) and a token with no ASCII letter
//!    are `NonProse`; step 9 makes the pair a range when a unit follows it.
//! 7. **Scientific notation.** A mantissa, `×` or `x`, and a power of ten — `2 × 10⁻¹⁶`, `1.5 x 10³`,
//!    `2.2× 10-16` — are one numeral, and so is a power of ten written with a superscript or a caret
//!    (`10³`, `10^6`). After `×` the exponent may be written with a plain minus, as extracted text
//!    writes it (`10-16`, `10−16`); alone, `10-16` is not a power.
//! 8. **Quantities.** A numeral and the unit written after it — `37 °C`, `10 μg ml⁻¹`, `10%`, or a
//!    unit attached to its digits, `931g` — are one [`TokenKind::Quantity`] token, carrying every
//!    reading of the unit ([`super::quantity`]). The unit is read from the text, not the tokens, since
//!    edge trimming has dropped the `°` and the `%`. The expression must end where a token ends:
//!    `5′-UTR` is not five arcminutes and a suffix.
//! 9. **Ranges.** A digit pair joined by an en-dash or a hyphen, with a unit or `%` after it — `2–3
//!    days`, `80–90%`, `45-60%` — is one [`TokenKind::Range`] token, both endpoints read in that unit
//!    (D95 implementation plan, slice 6, decision 8). An en-dash pair with no unit is a count range,
//!    read at the dimensionless unit (`4–7 foci`, slice 7, decision 5); a hyphen pair with no unit is
//!    not a range, since that is how a catalogue number is written (`926-68021`).
//!
//! **Case is preserved.** Consumers fold where they need a lowercase key ([`Parser::has_token`],
//! `lookup_span`, the [`Lemmatizer`], `ReservedTable::kind`, `rank_key`); `all_caps_symbol` needs the
//! original, to tell the symbol `CELL` from the noun `cell` (2026-07-29).
//!
//! [`Parser::has_token`]: super::parse::Parser::has_token
//! [`Lemmatizer`]: super::lemmatizer::Lemmatizer

use std::ops::Range;

use super::lex::{lex, LexClass, Lexeme};
use super::quantity::{ProseUnits, Quantity};
use crate::numeric::Rational;

/// One token of a sentence, as the parser sees it: a chart position.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    surface: String,
    span: Range<usize>,
    kind: TokenKind,
}

/// A range's endpoints, each a [`Quantity`] read in the unit written after the pair.
#[derive(Debug, Clone, PartialEq)]
pub struct QuantityRange {
    pub low: Quantity,
    pub high: Quantity,
    /// No unit followed the pair (`4–7 foci`): its endpoints are bare numbers, read at the
    /// dimensionless unit, and the range counts (D95 slice 7) rather than measures.
    pub unitless: bool,
}

/// What a token is to the parser.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// A token the lexicon is asked about. With no entry it is a missing lexeme.
    Word,
    /// The list separator `,`. It has no lexical entry; coordination, apposition and comma absorption
    /// key on its position.
    Comma,
    /// A number, exactly: `37`, `0.56`, `−1`, `1,200`.
    Numeral(Rational),
    /// A numeral with its unit: `37 °C`, `931g` (two readings), `10%`.
    Quantity(Quantity),
    /// Two numerals with the unit written once after them: `2–3 days`, `80–90%`, `45-60%`. The
    /// endpoints carry the same readings, in the same order.
    Range(QuantityRange),
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

/// Lex and preprocess `text`, reading units against `units`. The parser's own tokenization is
/// [`Parser::tokenize`](super::parse::Parser::tokenize), with the chain's vocabulary.
pub fn tokenize(text: &str, units: &ProseUnits) -> Vec<Token> {
    preprocess(text, &lex(text), units)
}

/// Preprocess `lexemes`, which [`lex`] made from `text`.
pub fn preprocess(text: &str, lexemes: &[Lexeme], units: &ProseUnits) -> Vec<Token> {
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
        if s == EN_DASH && joins_range(text, &run, pieces.get(k + 1)) {
            run.push(lexeme);
        } else if lexeme.class == LexClass::Space || is_separator(s) {
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
    let tokens = join_scientific(text, tokens);
    recognise_quantities(text, tokens, units)
}

/// Decision 7: scientific notation is one numeral — a mantissa, `×` or `x`, and a power of ten, or a
/// power of ten alone written with a superscript or a caret.
fn join_scientific(text: &str, tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if let (TokenKind::Numeral(mantissa), Some(times), Some(power)) =
            (&tokens[i].kind, tokens.get(i + 1), tokens.get(i + 2))
        {
            let value = matches!(times.surface.as_str(), "×" | "x" | "X")
                .then(|| power_of_ten(&power.surface, true))
                .flatten()
                .and_then(|k| scaled(mantissa, k));
            if let Some(value) = value {
                let span = tokens[i].span.start..power.span.end;
                out.push(Token {
                    surface: text[span.clone()].to_string(),
                    span,
                    kind: TokenKind::Numeral(value),
                });
                i += 3;
                continue;
            }
        }
        let alone = (tokens[i].kind == TokenKind::Word)
            .then(|| power_of_ten(&tokens[i].surface, false))
            .flatten()
            .and_then(|k| scaled(&Rational::from_integer(1.into()).ok()?, k));
        match alone {
            Some(value) => out.push(Token {
                kind: TokenKind::Numeral(value),
                ..tokens[i].clone()
            }),
            None => out.push(tokens[i].clone()),
        }
        i += 1;
    }
    out
}

/// The exponent `k` of a power of ten written `10` and then a superscript exponent (`10⁻¹⁶`, `10³`) or
/// a caret (`10^-16`), or — when `after_times`, where nothing else can be meant — a plain minus
/// (`10-16`, `10−16`). Exponents beyond ±400 are refused, since the value is admitted exactly.
fn power_of_ten(surface: &str, after_times: bool) -> Option<i32> {
    let rest = surface.strip_prefix("10")?;
    let (negative, digits): (bool, String) = if let Some(r) = rest.strip_prefix('^') {
        let (negative, r) = match r.strip_prefix(['-', '\u{2212}']) {
            Some(r) => (true, r),
            None => (false, r.strip_prefix('+').unwrap_or(r)),
        };
        (negative, r.to_string())
    } else if rest.starts_with(|c: char| superscript_digit(c).is_some() || c == '⁻' || c == '⁺')
    {
        let negative = rest.starts_with('⁻');
        let r = rest.trim_start_matches(['⁻', '⁺']);
        let mut digits = String::new();
        for c in r.chars() {
            digits.push(superscript_digit(c)?);
        }
        (negative, digits)
    } else if after_times {
        let r = rest.strip_prefix(['-', '\u{2212}'])?;
        (true, r.to_string())
    } else {
        return None;
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.len() > 3 {
        return None;
    }
    let k: i32 = digits.parse().ok()?;
    (k <= 400).then_some(if negative { -k } else { k })
}

/// The ASCII digit a superscript digit writes.
fn superscript_digit(c: char) -> Option<char> {
    Some(match c {
        '⁰' => '0',
        '¹' => '1',
        '²' => '2',
        '³' => '3',
        '⁴' => '4',
        '⁵' => '5',
        '⁶' => '6',
        '⁷' => '7',
        '⁸' => '8',
        '⁹' => '9',
        _ => return None,
    })
}

/// `mantissa × 10^k`, exactly.
fn scaled(mantissa: &Rational, k: i32) -> Option<Rational> {
    let power = num_bigint::BigInt::from(10).pow(k.unsigned_abs());
    if k >= 0 {
        Rational::new(mantissa.numer() * power, mantissa.denom().clone()).ok()
    } else {
        Rational::new(mantissa.numer().clone(), mantissa.denom() * power).ok()
    }
}

/// Decisions 8 and 9: a numeral and the unit written after it become one quantity token, and a digit
/// pair with a unit after it one range token.
fn recognise_quantities(text: &str, tokens: Vec<Token>, units: &ProseUnits) -> Vec<Token> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if let Some((end, range)) = range_at(text, &tokens[i], units) {
            let mut j = i + 1;
            while j < tokens.len() && tokens[j].span.start < end {
                j += 1;
            }
            if tokens[i..j].iter().all(|t| t.span.end <= end) {
                let start = tokens[i].span.start;
                out.push(Token {
                    surface: text[start..end].to_string(),
                    span: start..end,
                    kind: TokenKind::Range(range),
                });
                i = j;
                continue;
            }
        }
        if let Some((end, quantity)) = quantity_at(text, &tokens[i], units) {
            let mut j = i + 1;
            while j < tokens.len() && tokens[j].span.start < end {
                j += 1;
            }
            if tokens[i..j].iter().all(|t| t.span.end <= end) {
                let start = tokens[i].span.start;
                out.push(Token {
                    surface: text[start..end].to_string(),
                    span: start..end,
                    kind: TokenKind::Quantity(quantity),
                });
                i = j;
                continue;
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    out
}

/// The range a token is, if it is a digit pair joined by an en-dash or a hyphen and a unit follows
/// it: both endpoints read in that unit, with their readings in the same order.
fn range_at(text: &str, t: &Token, units: &ProseUnits) -> Option<(usize, QuantityRange)> {
    if matches!(t.kind, TokenKind::Numeral(_) | TokenKind::Comma) {
        return None;
    }
    let (low, high) = t
        .surface
        .split_once(EN_DASH)
        .or_else(|| t.surface.split_once('-'))?;
    let (low, high) = (numeral_value(low)?, numeral_value(high)?);
    let (end, high_readings, low_readings, unitless) = match (
        units.read(text, t.span.end, false, &high),
        units.read(text, t.span.end, false, &low),
    ) {
        (Some((end, high_readings)), Some((low_end, low_readings))) if low_end == end => {
            (end, high_readings, low_readings, false)
        }
        // An en-dash pair with no unit is a count range (`4–7 foci`); a hyphen pair with no unit
        // stays a word, since that is how a catalogue number is written (`926-68021`).
        (None, None) if t.surface.contains(EN_DASH) => (
            t.span.end,
            vec![units.bare(&high)?],
            vec![units.bare(&low)?],
            true,
        ),
        _ => return None,
    };
    if low_readings.len() != high_readings.len() {
        return None;
    }
    Some((
        end,
        QuantityRange {
            low: Quantity {
                value: low,
                readings: low_readings,
            },
            high: Quantity {
                value: high,
                readings: high_readings,
            },
            unitless,
        },
    ))
}

/// The quantity a token starts, if a unit follows its numeral: a numeral token and the unit after
/// it, or a word token whose leading digits carry the unit directly (`931g`, `5mg/kg`).
fn quantity_at(text: &str, t: &Token, units: &ProseUnits) -> Option<(usize, Quantity)> {
    match &t.kind {
        TokenKind::Numeral(value) => {
            let (end, readings) = units.read(text, t.span.end, false, value)?;
            Some((
                end,
                Quantity {
                    value: value.clone(),
                    readings,
                },
            ))
        }
        // The surface must be the text itself: a dropped gloss inside it would misplace the unit.
        TokenKind::Word
            if t.surface.starts_with(|c: char| c.is_ascii_digit())
                && text.get(t.span()) == Some(t.surface.as_str()) =>
        {
            let digits = numeral_prefix(&t.surface);
            let value = numeral_value(&t.surface[..digits])?;
            let (end, readings) = units.read(text, t.span.start + digits, true, &value)?;
            Some((end, Quantity { value, readings }))
        }
        _ => None,
    }
}

/// The byte length of the numeral that starts `s`: digits, grouped by commas in threes or not, and a
/// decimal part if a digit follows the point.
fn numeral_prefix(s: &str) -> usize {
    let mut int = leading_digits(s);
    while s[int..].starts_with(',') && leading_digits(&s[int + 1..]) == 3 {
        int += 4;
    }
    let rest = &s.as_bytes()[int..];
    if rest.first() == Some(&b'.') {
        let frac = rest[1..].iter().take_while(|b| b.is_ascii_digit()).count();
        if frac > 0 {
            return int + 1 + frac;
        }
    }
    int
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

const EN_DASH: &str = "\u{2013}";

/// Whether an en-dash after `run` joins a range: digits directly before it and after it (`4–12%`,
/// `2–3 days`, `0.1–0.5`).
fn joins_range(text: &str, run: &[&Lexeme], next: Option<&Piece>) -> bool {
    matches!(next, Some(Piece::Lexeme(n)) if is_digits(text, n))
        && run.last().is_some_and(|l| is_digits(text, l))
}

/// Whether `s` is two numerals joined by an en-dash. Such a pair is non-prose rather than a numeral and
/// a quantity (`4–12% gels` is not four gels of 12%); with a unit after it, it is a range (decision 9).
fn is_range(s: &str) -> bool {
    s.split_once(EN_DASH)
        .is_some_and(|(a, b)| numeral_value(a).is_some() && numeral_value(b).is_some())
}

fn is_digits(text: &str, l: &Lexeme) -> bool {
    l.class == LexClass::Word && l.text(text).bytes().all(|b| b.is_ascii_digit())
}

fn leading_digits(s: &str) -> usize {
    s.bytes().take_while(u8::is_ascii_digit).count()
}

/// Whether a comma after `run` groups digits: the run from its first word lexeme is digit groups
/// separated by commas — the first of one to three digits, the rest of three — and the next piece
/// starts with exactly three digits, alone or with a unit attached (`1,200`, `1,000g`).
fn groups_digits(text: &str, run: &[&Lexeme], next: Option<&Piece>) -> bool {
    let Some(Piece::Lexeme(next)) = next else {
        return false;
    };
    if next.class != LexClass::Word || leading_digits(next.text(text)) != 3 {
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
        None if is_range(&surface) => TokenKind::NonProse,
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

    /// No unit vocabulary: these tests are about everything but quantities.
    fn tokenize(text: &str) -> Vec<Token> {
        super::tokenize(text, &ProseUnits::none())
    }

    /// Decision 7: scientific notation is one numeral; a power of ten alone needs a superscript or a
    /// caret, and after `×` a plain minus writes the exponent. `4.2 × 10` is a product, not a power.
    #[test]
    fn scientific_notation_is_one_numeral() {
        let rat = |s: &str| Rational::parse_canonical(s).unwrap();
        let tokens = tokenize("P < 2 × 10⁻¹⁶");
        assert_eq!(surfaces_of(&tokens), ["P", "<", "2 × 10⁻¹⁶"]);
        assert_eq!(
            tokens[2].kind(),
            &TokenKind::Numeral(rat("1/5000000000000000"))
        );
        assert_eq!(
            tokenize("2.2× 10-16")[0].kind(),
            &TokenKind::Numeral(rat("11/50000000000000000"))
        );
        assert_eq!(
            tokenize("10³ cells")[0].kind(),
            &TokenKind::Numeral(rat("1000"))
        );
        assert_eq!(
            tokenize("10^6 cells")[0].kind(),
            &TokenKind::Numeral(rat("1000000"))
        );
        assert!(tokenize("10-16 cells")[0].is_word());
        assert_eq!(surfaces_of(&tokenize("4.2 × 10")), ["4.2", "×", "10"]);
    }

    fn surfaces_of(tokens: &[Token]) -> Vec<&str> {
        tokens.iter().map(Token::surface).collect()
    }

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
        // A group before an attached unit still groups: `1,000g` is one token.
        assert_eq!(surfaces("spun at 1,000g"), ["spun", "at", "1,000g"]);
        assert_eq!(surfaces("genes 1,23a"), ["genes", "1", ",", "23a"]);
    }

    /// Two numerals joined by an en-dash are a range, one non-prose token; letters on either side
    /// leave the en-dash a separator.
    #[test]
    fn a_range_is_non_prose() {
        for (text, range) in [
            ("in 4\u{2013}12% gels", "4\u{2013}12"),
            ("every 2\u{2013}3 days", "2\u{2013}3"),
            ("at 0.1\u{2013}0.5", "0.1\u{2013}0.5"),
        ] {
            let toks = tokenize(text);
            let t = toks.iter().find(|t| t.surface() == range).expect(text);
            assert_eq!(t.kind(), &TokenKind::NonProse, "{text:?}");
        }
        assert_eq!(surfaces("Fig. 10a\u{2013}d"), ["Fig", "10a", "d"]);
        assert_eq!(surfaces("exon\u{2013}intron"), ["exon", "intron"]);
    }

    #[test]
    fn a_digit_initial_token_that_is_not_a_numeral_is_a_word() {
        for word in ["53BP1", "5-fold", "1a", "0.56-fold", "45-60"] {
            assert_eq!(tokenize(word)[0].kind(), &TokenKind::Word, "{word}");
        }
        // A power of ten written with a superscript is a numeral (decision 7), no longer a word.
        assert!(matches!(
            tokenize("10\u{207b}\u{b9}\u{b3}")[0].kind(),
            TokenKind::Numeral(_)
        ));
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
