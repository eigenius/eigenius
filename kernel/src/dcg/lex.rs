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

//! D95 — the lexer: text → lexemes, deleting nothing.
//!
//! Every byte of the input is in exactly one lexeme, so whatever a later stage drops it drops as a
//! decision it can be asked about, not as a side effect of lexing. The classes are the ones the token
//! stream's edge trimming has always used: a run of alphanumerics, a run of whitespace, and one of
//! anything else. Every decision — asides, separators, trimming, commas — is
//! [`super::preprocess`]'s.

use std::ops::Range;

/// What a lexeme is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexClass {
    /// A maximal run of alphanumeric characters (`char::is_alphanumeric`): `BRCA1`, `0`, `mg`.
    Word,
    /// A maximal run of whitespace (`char::is_whitespace`).
    Space,
    /// One character that is neither: punctuation, a symbol, a combining mark. `0.56` is three
    /// lexemes and `°C` two.
    Punct,
}

/// A span of the lexed text and its class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexeme {
    /// Byte offsets into the lexed text.
    pub span: Range<usize>,
    pub class: LexClass,
}

impl Lexeme {
    /// The lexeme's text, read from the `source` it was lexed from.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.span.clone()]
    }
}

fn class_of(c: char) -> LexClass {
    if c.is_whitespace() {
        LexClass::Space
    } else if c.is_alphanumeric() {
        LexClass::Word
    } else {
        LexClass::Punct
    }
}

/// Lex `text`. The spans tile it: contiguous, in order, covering every byte.
pub fn lex(text: &str) -> Vec<Lexeme> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let class = class_of(c);
        let mut end = start + c.len_utf8();
        if class != LexClass::Punct {
            while let Some(&(i, d)) = chars.peek() {
                if class_of(d) != class {
                    break;
                }
                end = i + d.len_utf8();
                chars.next();
            }
        }
        out.push(Lexeme {
            span: start..end,
            class,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(text: &str) -> Vec<(&str, LexClass)> {
        lex(text).iter().map(|l| (l.text(text), l.class)).collect()
    }

    #[test]
    fn runs_of_words_and_spaces_and_single_punctuation() {
        use LexClass::*;
        assert_eq!(
            classes("37 °C, 0.56"),
            [
                ("37", Word),
                (" ", Space),
                ("°", Punct),
                ("C", Word),
                (",", Punct),
                (" ", Space),
                ("0", Word),
                (".", Punct),
                ("56", Word),
            ]
        );
        assert_eq!(classes("--"), [("-", Punct), ("-", Punct)]);
        assert!(lex("").is_empty());
    }

    #[test]
    fn the_spans_tile_the_input() {
        for text in [
            "HeLa depends on BRCA1.",
            "lethality\u{2014}an interaction\u{2014}can be exploited",
            "10 μg ml⁻¹ at 37 °C (Fig. 2g); café\u{301} a\u{a0}b",
            "",
        ] {
            let mut at = 0;
            for l in lex(text) {
                assert_eq!(l.span.start, at, "{text:?}");
                assert!(l.span.end > l.span.start, "{text:?}");
                at = l.span.end;
            }
            assert_eq!(at, text.len(), "{text:?}");
        }
    }
}
