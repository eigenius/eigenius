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

//! Hand-written lexer for EigenQL.
//!
//! Tokenizes an EigenQL source string into a stream of tokens
//! with position tracking. Whitespace and comments are discarded.

use crate::query::error::{Position, QueryError};

/// Token types.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    Match,
    Where,
    Return,
    Using,
    As,
    Define,
    From,
    And,
    Or,
    Not,
    In,
    Like,
    Exists,
    Group,
    By,
    Order,
    Asc,
    Desc,
    Distinct,
    Limit,
    Offset,
    // FIBER-clause keywords (D2 §3.3.1, §3.5)
    Fiber,
    Institution,
    /// `USING NAMESPACE "<prefix>"` — declares a vocabulary namespace whose
    /// classes/properties bare short names resolve within (D2 short-name scoping).
    Namespace,
    /// Optional FIBER suffix that names a chain-resident IRI for the
    /// response resource. Without `INTO`, the FIBER response stays in
    /// the transient query overlay; with `INTO "<iri>"`, the response
    /// is committed to the regular chain at the named IRI as part of
    /// the query's commit cycle (D14 §9.3 chain-reinsertion).
    Into,
    // Postfix Verdict predicates (D2 v2 §3.7, §3.8)
    Holds,
    Fails,
    Undecidable,

    // Built-in functions
    DateFn,
    TimestampFn,
    RegexFn,
    LengthFn,
    ContainsFn,
    ConcatFn,

    // Aggregates
    CountFn,
    SumFn,
    AvgFn,
    MinFn,
    MaxFn,

    // D43 §3.7 — ranked-retrieval clause
    /// `TOP K BY ?score [DESC|ASC]` (D43 §3.7). Mutually exclusive
    /// with `ORDER BY` / `LIMIT` in the same query.
    Top,

    // Literals
    StringLit(String),
    NumberInt(i64),
    NumberFloat(f64),
    BooleanLit(bool),

    // Variable (?name)
    Variable(String),

    // Identifier
    Identifier(String),

    // Operators
    Eq,         // =
    Neq,        // <>
    Lt,         // <
    Lte,        // <=
    Gt,         // >
    Gte,        // >=
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    DoubleStar, // **
    Pipe2,      // ||
    Tilde,      // ~  (D43 §3.3 similarity operator)

    // Structural
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Dot,
    /// `...` — the rest/iteration marker in array patterns (D59).
    Ellipsis,

    // End of input
    Eof,
}

/// A token with its position in the source.
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub pos: Position,
}

/// Tokenize an EigenQL source string.
pub fn tokenize(input: &str) -> Result<Vec<Token>, QueryError> {
    let mut lexer = Lexer::new(input);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let is_eof = token.kind == TokenKind::Eof;
        tokens.push(token);
        if is_eof {
            break;
        }
    }
    Ok(tokens)
}

struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn position(&self) -> Position {
        Position {
            line: self.line,
            column: self.col,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.input.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let ch = self.input.get(self.pos).copied()?;
        self.pos += 1;
        if ch == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => {
                    self.advance();
                }
                Some(b'/') => {
                    if self.peek_at(1) == Some(b'/') {
                        // Line comment
                        while let Some(ch) = self.advance() {
                            if ch == b'\n' {
                                break;
                            }
                        }
                    } else if self.peek_at(1) == Some(b'*') {
                        // Block comment
                        self.advance(); // /
                        self.advance(); // *
                        loop {
                            match self.advance() {
                                Some(b'*') if self.peek() == Some(b'/') => {
                                    self.advance();
                                    break;
                                }
                                None => break,
                                _ => {}
                            }
                        }
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, QueryError> {
        self.skip_whitespace_and_comments();

        let pos = self.position();

        let ch = match self.peek() {
            None => {
                return Ok(Token {
                    kind: TokenKind::Eof,
                    pos,
                })
            }
            Some(ch) => ch,
        };

        // Structural tokens
        match ch {
            b'(' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::LParen,
                    pos,
                });
            }
            b')' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::RParen,
                    pos,
                });
            }
            b'{' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::LBrace,
                    pos,
                });
            }
            b'}' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::RBrace,
                    pos,
                });
            }
            b'[' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::LBracket,
                    pos,
                });
            }
            b']' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::RBracket,
                    pos,
                });
            }
            b':' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::Colon,
                    pos,
                });
            }
            b',' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::Comma,
                    pos,
                });
            }
            b'.' => {
                // `...` (ellipsis) — the rest/iteration marker in array
                // patterns (D59); otherwise a single `.` (dot-path).
                if self.peek_at(1) == Some(b'.') && self.peek_at(2) == Some(b'.') {
                    self.advance();
                    self.advance();
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::Ellipsis,
                        pos,
                    });
                }
                self.advance();
                return Ok(Token {
                    kind: TokenKind::Dot,
                    pos,
                });
            }
            b'+' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::Plus,
                    pos,
                });
            }
            b'%' => {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::Percent,
                    pos,
                });
            }
            _ => {}
        }

        // Slash — division or comment start (comments already handled in skip_whitespace)
        if ch == b'/' {
            self.advance();
            return Ok(Token {
                kind: TokenKind::Slash,
                pos,
            });
        }

        // Multi-character operators
        if ch == b'*' {
            self.advance();
            if self.peek() == Some(b'*') {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::DoubleStar,
                    pos,
                });
            }
            return Ok(Token {
                kind: TokenKind::Star,
                pos,
            });
        }

        if ch == b'|' && self.peek_at(1) == Some(b'|') {
            self.advance();
            self.advance();
            return Ok(Token {
                kind: TokenKind::Pipe2,
                pos,
            });
        }

        if ch == b'<' {
            self.advance();
            if self.peek() == Some(b'=') {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::Lte,
                    pos,
                });
            }
            if self.peek() == Some(b'>') {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::Neq,
                    pos,
                });
            }
            return Ok(Token {
                kind: TokenKind::Lt,
                pos,
            });
        }

        if ch == b'>' {
            self.advance();
            if self.peek() == Some(b'=') {
                self.advance();
                return Ok(Token {
                    kind: TokenKind::Gte,
                    pos,
                });
            }
            return Ok(Token {
                kind: TokenKind::Gt,
                pos,
            });
        }

        if ch == b'=' {
            self.advance();
            return Ok(Token {
                kind: TokenKind::Eq,
                pos,
            });
        }

        if ch == b'-' {
            // Could be a negative number or minus operator
            // If followed by a digit, lex as number
            if let Some(next) = self.peek_at(1) {
                if next.is_ascii_digit() {
                    return self.lex_number(pos);
                }
            }
            self.advance();
            return Ok(Token {
                kind: TokenKind::Minus,
                pos,
            });
        }

        // D43 §3.3 similarity operator `~`
        if ch == b'~' {
            self.advance();
            return Ok(Token {
                kind: TokenKind::Tilde,
                pos,
            });
        }

        // String literal
        if ch == b'"' {
            return self.lex_string(pos);
        }

        // Variable
        if ch == b'?' {
            self.advance();
            return self.lex_variable(pos);
        }

        // Number
        if ch.is_ascii_digit() {
            return self.lex_number(pos);
        }

        // Identifier or keyword
        if ch.is_ascii_alphabetic() || ch == b'_' {
            return self.lex_identifier_or_keyword(pos);
        }

        Err(QueryError::lexer(
            pos,
            format!("unexpected character: '{}'", ch as char),
        ))
    }

    fn lex_string(&mut self, pos: Position) -> Result<Token, QueryError> {
        self.advance(); // opening "
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err(QueryError::lexer(pos, "unterminated string literal")),
                Some(b'"') => break,
                Some(b'\\') => match self.advance() {
                    Some(b'"') => s.push('"'),
                    Some(b'\\') => s.push('\\'),
                    Some(b'/') => s.push('/'),
                    Some(b'b') => s.push('\u{08}'),
                    Some(b'f') => s.push('\u{0C}'),
                    Some(b'n') => s.push('\n'),
                    Some(b'r') => s.push('\r'),
                    Some(b't') => s.push('\t'),
                    Some(b'u') => {
                        let mut hex = String::with_capacity(4);
                        for _ in 0..4 {
                            match self.advance() {
                                Some(c) if (c as char).is_ascii_hexdigit() => {
                                    hex.push(c as char);
                                }
                                _ => return Err(QueryError::lexer(pos, "invalid unicode escape")),
                            }
                        }
                        let code = u32::from_str_radix(&hex, 16).map_err(|_| {
                            QueryError::lexer(pos.clone(), "invalid unicode escape")
                        })?;
                        let ch = char::from_u32(code).ok_or_else(|| {
                            QueryError::lexer(pos.clone(), "invalid unicode code point")
                        })?;
                        s.push(ch);
                    }
                    Some(c) => {
                        return Err(QueryError::lexer(
                            pos,
                            format!("invalid escape character: '{}'", c as char),
                        ))
                    }
                    None => return Err(QueryError::lexer(pos, "unterminated string escape")),
                },
                Some(c) => s.push(c as char),
            }
        }
        Ok(Token {
            kind: TokenKind::StringLit(s),
            pos,
        })
    }

    fn lex_variable(&mut self, pos: Position) -> Result<Token, QueryError> {
        let mut name = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                name.push(ch as char);
                self.advance();
            } else {
                break;
            }
        }
        if name.is_empty() {
            return Err(QueryError::lexer(pos, "expected variable name after '?'"));
        }
        Ok(Token {
            kind: TokenKind::Variable(name),
            pos,
        })
    }

    fn lex_number(&mut self, pos: Position) -> Result<Token, QueryError> {
        let start = self.pos;
        // Optional minus
        if self.peek() == Some(b'-') {
            self.advance();
        }
        // Integer part
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        let mut is_float = false;
        // Decimal part
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            self.advance(); // .
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        // Exponent
        if let Some(b'e' | b'E') = self.peek() {
            is_float = true;
            self.advance();
            if let Some(b'+' | b'-') = self.peek() {
                self.advance();
            }
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        let text = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| QueryError::lexer(pos.clone(), "invalid number"))?;

        if is_float {
            let val: f64 = text
                .parse()
                .map_err(|_| QueryError::lexer(pos.clone(), format!("invalid float: {text}")))?;
            Ok(Token {
                kind: TokenKind::NumberFloat(val),
                pos,
            })
        } else {
            let val: i64 = text
                .parse()
                .map_err(|_| QueryError::lexer(pos.clone(), format!("invalid integer: {text}")))?;
            Ok(Token {
                kind: TokenKind::NumberInt(val),
                pos,
            })
        }
    }

    fn lex_identifier_or_keyword(&mut self, pos: Position) -> Result<Token, QueryError> {
        let mut word = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-' {
                word.push(ch as char);
                self.advance();
            } else {
                break;
            }
        }

        let kind = match word.as_str() {
            // Keywords
            "MATCH" => TokenKind::Match,
            "WHERE" => TokenKind::Where,
            "RETURN" => TokenKind::Return,
            "USING" => TokenKind::Using,
            "AS" => TokenKind::As,
            "DEFINE" => TokenKind::Define,
            "FROM" => TokenKind::From,
            "AND" => TokenKind::And,
            "OR" => TokenKind::Or,
            "NOT" => TokenKind::Not,
            "IN" => TokenKind::In,
            "LIKE" => TokenKind::Like,
            "EXISTS" => TokenKind::Exists,
            "GROUP" => TokenKind::Group,
            "BY" => TokenKind::By,
            "ORDER" => TokenKind::Order,
            "ASC" => TokenKind::Asc,
            "DESC" => TokenKind::Desc,
            "DISTINCT" => TokenKind::Distinct,
            "LIMIT" => TokenKind::Limit,
            "OFFSET" => TokenKind::Offset,
            "FIBER" => TokenKind::Fiber,
            "INSTITUTION" => TokenKind::Institution,
            "NAMESPACE" => TokenKind::Namespace,
            "INTO" => TokenKind::Into,
            "HOLDS" => TokenKind::Holds,
            "FAILS" => TokenKind::Fails,
            "UNDECIDABLE" => TokenKind::Undecidable,
            // Functions
            "DATE" => TokenKind::DateFn,
            "TIMESTAMP" => TokenKind::TimestampFn,
            "REGEX" => TokenKind::RegexFn,
            "LENGTH" => TokenKind::LengthFn,
            "CONTAINS" => TokenKind::ContainsFn,
            "CONCAT" => TokenKind::ConcatFn,
            // Aggregates
            "COUNT" => TokenKind::CountFn,
            "SUM" => TokenKind::SumFn,
            "AVG" => TokenKind::AvgFn,
            "MIN" => TokenKind::MinFn,
            "MAX" => TokenKind::MaxFn,
            // D43 §3 — retrieval primitives.
            "TOP" => TokenKind::Top,
            // Booleans
            "true" => TokenKind::BooleanLit(true),
            "false" => TokenKind::BooleanLit(false),
            // Identifier
            _ => TokenKind::Identifier(word),
        };

        Ok(Token { kind, pos })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(input: &str) -> Vec<TokenKind> {
        tokenize(input)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .filter(|k| *k != TokenKind::Eof)
            .collect()
    }

    #[test]
    fn keywords() {
        assert_eq!(
            kinds("MATCH WHERE RETURN"),
            vec![TokenKind::Match, TokenKind::Where, TokenKind::Return,]
        );
    }

    #[test]
    fn define_from() {
        assert_eq!(
            kinds("DEFINE FROM"),
            vec![TokenKind::Define, TokenKind::From,]
        );
    }

    #[test]
    fn variable() {
        assert_eq!(
            kinds("?name ?x123"),
            vec![
                TokenKind::Variable("name".into()),
                TokenKind::Variable("x123".into()),
            ]
        );
    }

    #[test]
    fn string_literal() {
        assert_eq!(
            kinds(r#""hello" "world\n""#),
            vec![
                TokenKind::StringLit("hello".into()),
                TokenKind::StringLit("world\n".into()),
            ]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("42 2.72 -7 1e10"),
            vec![
                TokenKind::NumberInt(42),
                TokenKind::NumberFloat(2.72),
                TokenKind::NumberInt(-7),
                TokenKind::NumberFloat(1e10),
            ]
        );
    }

    #[test]
    fn booleans() {
        assert_eq!(
            kinds("true false"),
            vec![TokenKind::BooleanLit(true), TokenKind::BooleanLit(false),]
        );
    }

    #[test]
    fn operators() {
        assert_eq!(
            kinds("= <> < <= > >= + - * / % ** ||"),
            vec![
                TokenKind::Eq,
                TokenKind::Neq,
                TokenKind::Lt,
                TokenKind::Lte,
                TokenKind::Gt,
                TokenKind::Gte,
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Percent,
                TokenKind::DoubleStar,
                TokenKind::Pipe2,
            ]
        );
    }

    #[test]
    fn structural() {
        assert_eq!(
            kinds("( ) { } [ ] : , ."),
            vec![
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::LBracket,
                TokenKind::RBracket,
                TokenKind::Colon,
                TokenKind::Comma,
                TokenKind::Dot,
            ]
        );
    }

    #[test]
    fn identifiers() {
        assert_eq!(
            kinds("name breed short_name"),
            vec![
                TokenKind::Identifier("name".into()),
                TokenKind::Identifier("breed".into()),
                TokenKind::Identifier("short_name".into()),
            ]
        );
    }

    #[test]
    fn aggregates() {
        assert_eq!(
            kinds("COUNT SUM AVG MIN MAX"),
            vec![
                TokenKind::CountFn,
                TokenKind::SumFn,
                TokenKind::AvgFn,
                TokenKind::MinFn,
                TokenKind::MaxFn,
            ]
        );
    }

    #[test]
    fn comments_discarded() {
        assert_eq!(
            kinds("MATCH // comment\n WHERE /* block */ RETURN"),
            vec![TokenKind::Match, TokenKind::Where, TokenKind::Return]
        );
    }

    #[test]
    fn position_tracking() {
        let tokens = tokenize("MATCH\n  WHERE").unwrap();
        assert_eq!(tokens[0].pos, Position { line: 1, column: 1 });
        assert_eq!(tokens[1].pos, Position { line: 2, column: 3 });
    }

    #[test]
    fn full_query() {
        let input = r#"
            USING "urn:eigenius:core:Class"
            MATCH Class(?c) {
                short_name: ?name
            }
            RETURN Class {
                short_name: ?name
            }
        "#;
        let tokens = tokenize(input).unwrap();
        assert!(tokens.len() > 10);
        assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof);
    }

    #[test]
    fn unterminated_string_error() {
        let result = tokenize(r#""hello"#);
        assert!(result.is_err());
    }

    #[test]
    fn not_exists_tokens() {
        assert_eq!(
            kinds("NOT EXISTS"),
            vec![TokenKind::Not, TokenKind::Exists,]
        );
    }

    #[test]
    fn group_by_order_by() {
        assert_eq!(
            kinds("GROUP BY ORDER BY ASC DESC"),
            vec![
                TokenKind::Group,
                TokenKind::By,
                TokenKind::Order,
                TokenKind::By,
                TokenKind::Asc,
                TokenKind::Desc,
            ]
        );
    }

    #[test]
    fn distinct_limit_offset() {
        assert_eq!(
            kinds("DISTINCT LIMIT OFFSET"),
            vec![TokenKind::Distinct, TokenKind::Limit, TokenKind::Offset,]
        );
    }

    // --- D43 §3 retrieval primitives — M1 lexer reservation ---

    /// Each D43 retrieval keyword tokenises to its dedicated TokenKind.
    /// M1's load-bearing invariant: the keywords are reserved (cannot
    /// be used as user identifiers) so the surface stays stable through
    /// to M3 / M5 / M7 implementation.
    /// Only `TOP` remains as a reserved retrieval-clause keyword
    /// after the D43 surface reset; the six function-shaped
    /// primitives (TEXT_MATCH / TEXT_SCORE / VECTOR_NEAR / VECTOR_SIM
    /// / EMBED / RRF) collapsed into the `~` operator added in
    /// Phase 5.
    #[test]
    fn d43_retrieval_keywords_tokenize() {
        assert_eq!(kinds("TOP"), vec![TokenKind::Top]);
    }

    /// D43 §3.3 — the similarity operator `~` tokenises to
    /// `TokenKind::Tilde`. Standalone and adjacent to identifiers /
    /// variables / strings, the lexer doesn't merge it into anything.
    #[test]
    fn d43_tilde_tokenizes() {
        assert_eq!(kinds("~"), vec![TokenKind::Tilde]);
        assert_eq!(
            kinds("?desc ~ \"q\""),
            vec![
                TokenKind::Variable("desc".into()),
                TokenKind::Tilde,
                TokenKind::StringLit("q".into()),
            ]
        );
    }

    /// EigenQL keywords are uppercase-only per D2 §2.2. Lowercase
    /// `top` stays an identifier — keyword status is case-sensitive.
    #[test]
    fn d43_retrieval_keywords_are_case_sensitive() {
        assert_eq!(
            kinds("text_match vector_near embed rrf top"),
            vec![
                TokenKind::Identifier("text_match".into()),
                TokenKind::Identifier("vector_near".into()),
                TokenKind::Identifier("embed".into()),
                TokenKind::Identifier("rrf".into()),
                TokenKind::Identifier("top".into()),
            ]
        );
    }
}
