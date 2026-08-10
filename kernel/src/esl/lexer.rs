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

//! Hand-written lexer for ESL (Eigenius Surface Language).
//!
//! Tokenizes ESL source into a stream of tokens with position tracking.
//! Whitespace and comments (// line, /* block */) are discarded.

use crate::esl::error::{EslError, Position};

/// Token types for ESL.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Top-level keywords
    Namespace,
    Class,
    Property,
    Resource,
    Program,
    Codata,
    Data,
    /// D37 §3.3 — `merge_comorphism <iri> for <class> { … }`.
    MergeComorphism,
    /// D37 §3.3 — the `for` keyword separating a merge_comorphism's
    /// IRI from its target class. Reserved globally so it can't be
    /// shadowed by user-defined identifiers.
    For,
    /// D43 §3.1 — `text_index <iri> { target_property = ...; text_analyzer = "..."; }`.
    /// Sugar over a `core:TextIndex` Resource declaration.
    TextIndex,
    /// D43 §3.1 — `vector_index <iri> { target_property = ...; vec_model = ...; ... }`.
    /// Sugar over a `core:VectorIndex` Resource declaration.
    VectorIndex,

    // Expression keywords
    Let,
    /// `alias name = expr, name = expr, ... in body` —
    /// compile-time substitution form in `type_expr(...)` position.
    /// Distinct from `Let` which marks a kernel-level definitional
    /// binding (`Decl::Def` in NbE; the `let x : T = e; body` form
    /// in the program-body parser). Aliases inline at lowering time
    /// and never appear in `Exp`. Future type-position kernel-let
    /// (real δ-binding) would reuse `Let`; reserving `alias` now
    /// keeps the two distinguishable forever.
    Alias,
    /// `in` keyword — terminates the binding list of an
    /// `alias name = expr, ... in body` form. Two letters; not used
    /// as an identifier anywhere in the existing ESL codebase, so the
    /// reservation costs nothing.
    In,
    Case,
    Match,
    Returning,
    Construct,
    Map,
    Reduce,
    Corecord,
    /// D37 §3.1 — `lambda x : T => body` (the typed, English-word
    /// form). Distinct from `Backslash` (`\`) and `Lambda` (`λ`),
    /// which produce the untyped lambda surface used in `program`
    /// bodies.
    LambdaKw,
    /// D37 §3.5 — `pi x : T => U` value-typed Pi expression.
    Pi,
    /// eigenius#72 — `forall (P : Prop) => body`. Alias for `pi` with
    /// proposition-authoring semantics; parses to the same AST.
    Forall,
    /// `exists` — opens a Sigma (dependent pair) binder, the dual of `forall`.
    Exists,
    /// eigenius#72 Layer 3 — `fun (i : T) => body` lambda in type
    /// position. Used as a motive for `match … returning <motive>`
    /// over indexed inductives. Compiles to `Exp::Lam`.
    Fun,
    /// eigenius#72 — `axiom Name : <type-expr>` top-level declaration.
    /// Commits a `core:Axiom` chain resource with the type expression
    /// encoded via the D47 codec.
    Axiom,
    /// `def name(p : T, ...) : R = <type-expr>` — a chain-resident TRANSPARENT definition (D66).
    /// Commits an `eigentt:Definition` whose body decode substitutes at each use, unlike
    /// [`TokenKind::Axiom`], whose IRI stays rigid and never unfolds.
    Def,
    /// D52 §12 — `macro ns:Name(p1 : T1, ...) : RetT => body`
    /// top-level smart-constructor declaration. Compile-time AST
    /// substitution only; not a runtime function. Distinct from
    /// `Fun` (`fun (x : T) => body` inside `type_expr(...)` is a
    /// type-level lambda).
    Macro,
    /// eigenius#72 — sort literal `Prop` (= `Sort(0)`).
    Prop,
    /// eigenius#72 — sort literal `Set` (= `Sort(1)`).
    SetKw,
    /// eigenius#72 — sort literal `Type` (followed by an integer level
    /// in `Type N` parses; `Type 0` is `Sort(1)`, `Type N` is `Sort(N+1)`).
    TypeKw,
    /// D37 §3.3 — `=>` returns/produces. Used in `lambda` bodies
    /// (after the parameter list) and inline `merge_comorphism`
    /// bodies. Distinct from `Arrow` (`->`) which separates the
    /// untyped lambda's parameter from its body and types in
    /// non-dependent arrows.
    FatArrow,

    // Literals
    StringLit(String),
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),

    // Identifier (bare word: name, breed, short_name)
    Ident(String),
    /// A qualified name `ns:name`, lexed atomically (tight `:`, no surrounding
    /// whitespace). Making qualified names a single token frees the standalone
    /// `:` (`Colon`) to mean *only* the binder / annotation colon (`x : T`,
    /// `(e : T)`), removing the namespace-vs-annotation ambiguity (D63 §8.2).
    QualName(String, String),

    // Operators
    Eq,        // =
    Arrow,     // ->
    Backslash, // \ (lambda)
    Lambda,    // λ (lambda, unicode)
    Dot,       // .
    Semicolon, // ;
    Colon,     // :
    Comma,     // ,
    Less,      // < (size bound in `j : Size < i`, Phase 11b step 15h)
    // Arithmetic operators — used by the formula(...) sublanguage
    // (Phase 19f.3) and by parse_value for unary minus on numeric
    // literals (`ex:value = -1.5;` preserves its old shape via the
    // parser). Tokens emitted unconditionally; consumers outside the
    // formula path only ever see Minus, never the others.
    Plus,  // +
    Minus, // -
    Star,  // *
    Slash, // /
    Caret, // ^

    // Structural
    LParen,   // (
    RParen,   // )
    LBrace,   // {
    RBrace,   // }
    LBracket, // [
    RBracket, // ]

    // End of input
    Eof,
}

/// A token with its source position.
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub pos: Position,
}

/// Tokenize an ESL source string.
pub fn tokenize(input: &str) -> Result<Vec<Token>, EslError> {
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
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
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
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let ch = self.bytes.get(self.pos).copied()?;
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
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    // Line comment
                    while let Some(ch) = self.advance() {
                        if ch == b'\n' {
                            break;
                        }
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
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
                }
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, EslError> {
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

        // FatArrow `=>` vs single Eq `=`. The `=>` token (D37 §3.1)
        // marks the lambda-body separator in typed lambda literals
        // and inline `merge_comorphism` bodies. Same lookahead shape
        // as `-` / `->` below.
        if ch == b'=' {
            if self.peek_at(1) == Some(b'>') {
                self.advance();
                self.advance();
                return Ok(Token {
                    kind: TokenKind::FatArrow,
                    pos,
                });
            }
            self.advance();
            return Ok(Token {
                kind: TokenKind::Eq,
                pos,
            });
        }

        // Single-character tokens
        let simple = match ch {
            b'(' => Some(TokenKind::LParen),
            b')' => Some(TokenKind::RParen),
            b'{' => Some(TokenKind::LBrace),
            b'}' => Some(TokenKind::RBrace),
            b'[' => Some(TokenKind::LBracket),
            b']' => Some(TokenKind::RBracket),
            b';' => Some(TokenKind::Semicolon),
            b',' => Some(TokenKind::Comma),
            b'.' => Some(TokenKind::Dot),
            b'\\' => Some(TokenKind::Backslash),
            b'<' => Some(TokenKind::Less),
            b'+' => Some(TokenKind::Plus),
            b'*' => Some(TokenKind::Star),
            b'/' => Some(TokenKind::Slash),
            b'^' => Some(TokenKind::Caret),
            _ => None,
        };

        if let Some(kind) = simple {
            self.advance();
            return Ok(Token { kind, pos });
        }

        // Colon — could be standalone : or part of an identifier (ns:name handled in lex_ident)
        if ch == b':' {
            self.advance();
            return Ok(Token {
                kind: TokenKind::Colon,
                pos,
            });
        }

        // Arrow ->  /  Minus -
        //
        // `-` followed by `>` is the function-arrow token used in
        // program type signatures and lambda binders. Any other `-`
        // emits as a `Minus` token; the parser decides whether it's
        // unary minus on a numeric literal (existing
        // `ex:value = -1.5;` shape, handled in `parse_value`),
        // unary or binary inside `formula(...)` (the Pratt parser
        // handles both cases), or an error in any other position.
        // Older sign-folding-at-lex behaviour (`-1.5` → single
        // `FloatLit(-1.5)` token) was retired in Phase 19f.3 because
        // it produced surprises inside `formula(x-2)` (the lexer
        // would have consumed `-2` as a signed literal, leaving no
        // operator between `x` and `2`).
        if ch == b'-' {
            if self.peek_at(1) == Some(b'>') {
                self.advance();
                self.advance();
                return Ok(Token {
                    kind: TokenKind::Arrow,
                    pos,
                });
            }
            self.advance();
            return Ok(Token {
                kind: TokenKind::Minus,
                pos,
            });
        }

        // String literal
        if ch == b'"' {
            return self.lex_string(pos);
        }

        // Number
        if ch.is_ascii_digit() {
            return self.lex_number(pos);
        }

        // Lambda unicode: λ is U+03BB, encoded as CE BB in UTF-8
        if ch == 0xCE && self.peek_at(1) == Some(0xBB) {
            self.advance();
            self.advance();
            return Ok(Token {
                kind: TokenKind::Lambda,
                pos,
            });
        }

        // Identifier or keyword
        if ch.is_ascii_alphabetic() || ch == b'_' {
            return self.lex_identifier(pos);
        }

        Err(EslError::lexer(
            pos,
            format!("unexpected character: '{}'", ch as char),
        ))
    }

    fn lex_string(&mut self, pos: Position) -> Result<Token, EslError> {
        self.advance(); // opening "
                        // Accumulate RAW BYTES and decode UTF-8 once at the end. Pushing each byte as `c as char`
                        // would be Latin-1 decoding: a multibyte char (`—` = E2 80 94, `β`, an accent) would split
                        // into per-byte mojibake (`â…`), corrupting every non-ASCII gloss and surface form on load.
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match self.advance() {
                None => return Err(EslError::lexer(pos, "unterminated string literal")),
                Some(b'"') => break,
                Some(b'\\') => match self.advance() {
                    Some(b'"') => bytes.push(b'"'),
                    Some(b'\\') => bytes.push(b'\\'),
                    Some(b'n') => bytes.push(b'\n'),
                    Some(b'r') => bytes.push(b'\r'),
                    Some(b't') => bytes.push(b'\t'),
                    Some(c) => {
                        return Err(EslError::lexer(
                            pos,
                            format!("invalid escape: '\\{}'", c as char),
                        ))
                    }
                    None => return Err(EslError::lexer(pos, "unterminated escape")),
                },
                Some(c) => bytes.push(c),
            }
        }
        let s = String::from_utf8(bytes)
            .map_err(|_| EslError::lexer(pos.clone(), "invalid UTF-8 in string literal"))?;
        Ok(Token {
            kind: TokenKind::StringLit(s),
            pos,
        })
    }

    fn lex_number(&mut self, pos: Position) -> Result<Token, EslError> {
        // `-` is no longer consumed here — the lexer always emits a
        // separate `Minus` token (Phase 19f.3). Sign folding now
        // happens in `parse_value` for the `ex:value = -1.5;` shape
        // (unary minus on a numeric literal), or implicitly in the
        // formula(...) Pratt parser's prefix-minus rule.
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.advance();
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            self.advance();
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.advance();
            }
        }
        if self.peek().is_some_and(|c| c == b'e' || c == b'E') {
            is_float = true;
            self.advance();
            if self.peek().is_some_and(|c| c == b'+' || c == b'-') {
                self.advance();
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.advance();
            }
        }

        let text = &self.input[start..self.pos];
        if is_float {
            let val: f64 = text
                .parse()
                .map_err(|_| EslError::lexer(pos.clone(), format!("invalid float: {text}")))?;
            Ok(Token {
                kind: TokenKind::FloatLit(val),
                pos,
            })
        } else {
            let val: i64 = text
                .parse()
                .map_err(|_| EslError::lexer(pos.clone(), format!("invalid integer: {text}")))?;
            Ok(Token {
                kind: TokenKind::IntLit(val),
                pos,
            })
        }
    }

    fn lex_identifier(&mut self, pos: Position) -> Result<Token, EslError> {
        let mut word = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                word.push(ch as char);
                self.advance();
            } else {
                break;
            }
        }

        let kind = match word.as_str() {
            // Top-level keywords
            "namespace" => TokenKind::Namespace,
            "class" => TokenKind::Class,
            "property" => TokenKind::Property,
            "resource" => TokenKind::Resource,
            "program" => TokenKind::Program,
            "codata" => TokenKind::Codata,
            "data" => TokenKind::Data,
            // D37 §3.3 — typed merge witness declaration.
            "merge_comorphism" => TokenKind::MergeComorphism,
            "for" => TokenKind::For,
            // D43 §3.1 — index Resource declarations as top-level keywords.
            // Sugar over `resource <iri> : core:TextIndex { ... }` /
            // `resource <iri> : core:VectorIndex { ... }`.
            "text_index" => TokenKind::TextIndex,
            "vector_index" => TokenKind::VectorIndex,
            // Expression keywords
            "let" => TokenKind::Let,
            "alias" => TokenKind::Alias,
            "in" => TokenKind::In,
            "case" => TokenKind::Case,
            "match" => TokenKind::Match,
            "returning" => TokenKind::Returning,
            "Construct" => TokenKind::Construct,
            "map" => TokenKind::Map,
            "reduce" => TokenKind::Reduce,
            "corecord" => TokenKind::Corecord,
            // D37 §3.1, §3.5 — typed lambda literal + Pi type expr.
            // Distinct from `\` / `λ` (Backslash / Lambda tokens),
            // which produce the untyped lambda surface.
            "lambda" => TokenKind::LambdaKw,
            "pi" => TokenKind::Pi,
            // eigenius#72 — proposition / indexed-type authoring.
            "forall" => TokenKind::Forall,
            "exists" => TokenKind::Exists,
            "fun" => TokenKind::Fun,
            "axiom" => TokenKind::Axiom,
            "def" => TokenKind::Def,
            "macro" => TokenKind::Macro,
            "Prop" => TokenKind::Prop,
            "Set" => TokenKind::SetKw,
            "Type" => TokenKind::TypeKw,
            // Literals
            "true" => TokenKind::BoolLit(true),
            "false" => TokenKind::BoolLit(false),
            // Identifier
            _ => TokenKind::Ident(word),
        };

        // Tight qualified name `ns:name`: a plain identifier immediately
        // followed (no whitespace) by `:` and another identifier lexes as a
        // single `QualName` token. This is what frees a space-surrounded `:`
        // to mean the binder / annotation colon (`x : T`, `(e : T)`) without
        // colliding with the namespace separator. Only plain identifiers form a
        // namespace (keywords never do), so keyword tokens are left untouched.
        if let TokenKind::Ident(ns) = &kind {
            if self.peek() == Some(b':')
                && self
                    .peek_at(1)
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            {
                let ns = ns.clone();
                self.advance(); // consume ':'
                let mut name = String::new();
                while let Some(ch) = self.peek() {
                    if ch.is_ascii_alphanumeric() || ch == b'_' {
                        name.push(ch as char);
                        self.advance();
                    } else {
                        break;
                    }
                }
                return Ok(Token {
                    kind: TokenKind::QualName(ns, name),
                    pos,
                });
            }
        }

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
    fn top_level_keywords() {
        assert_eq!(
            kinds("namespace class property resource program codata"),
            vec![
                TokenKind::Namespace,
                TokenKind::Class,
                TokenKind::Property,
                TokenKind::Resource,
                TokenKind::Program,
                TokenKind::Codata,
            ]
        );
    }

    #[test]
    fn expression_keywords() {
        assert_eq!(
            kinds("let case Construct map reduce corecord"),
            vec![
                TokenKind::Let,
                TokenKind::Case,
                TokenKind::Construct,
                TokenKind::Map,
                TokenKind::Reduce,
                TokenKind::Corecord,
            ]
        );
    }

    #[test]
    fn identifiers() {
        assert_eq!(
            kinds("name breed short_name Dog"),
            vec![
                TokenKind::Ident("name".into()),
                TokenKind::Ident("breed".into()),
                TokenKind::Ident("short_name".into()),
                TokenKind::Ident("Dog".into()),
            ]
        );
    }

    #[test]
    fn qualified_name_tokens() {
        // A tight `ns:name` lexes as one atomic `QualName` token (the namespace
        // separator), distinct from a space-surrounded binder/annotation `:`.
        assert_eq!(
            kinds("core:string ex:Dog"),
            vec![
                TokenKind::QualName("core".into(), "string".into()),
                TokenKind::QualName("ex".into(), "Dog".into()),
            ]
        );
        // A space-surrounded `:` stays a standalone `Colon` (binder / annotation).
        assert_eq!(
            kinds("x : Set"),
            vec![
                TokenKind::Ident("x".into()),
                TokenKind::Colon,
                TokenKind::SetKw,
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
        // Phase 19f.3: `-` no longer folds into the numeric literal at
        // the lexer level — it always emits `Minus` (sign folding
        // happens in the parser, where context determines whether it's
        // unary minus on a literal or the binary subtraction operator
        // inside a formula(...) expression).
        assert_eq!(
            kinds("42 2.72 -7 1e10"),
            vec![
                TokenKind::IntLit(42),
                TokenKind::FloatLit(2.72),
                TokenKind::Minus,
                TokenKind::IntLit(7),
                TokenKind::FloatLit(1e10),
            ]
        );
    }

    #[test]
    fn arithmetic_operators_emit_distinct_tokens() {
        // The formula(...) sublanguage uses these directly (Pratt
        // parser); outside formula(...) only `Minus` is consumed by
        // `parse_value` for unary-minus on numeric literals.
        assert_eq!(
            kinds("+ - * / ^"),
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Caret,
            ]
        );
    }

    #[test]
    fn arrow_still_takes_priority_over_minus() {
        // `->` must continue to parse as Arrow even after the lexer
        // started emitting bare `-` as Minus.
        assert_eq!(
            kinds("a -> b"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Arrow,
                TokenKind::Ident("b".into()),
            ]
        );
    }

    #[test]
    fn booleans() {
        assert_eq!(
            kinds("true false"),
            vec![TokenKind::BoolLit(true), TokenKind::BoolLit(false)]
        );
    }

    #[test]
    fn operators_and_structural() {
        assert_eq!(
            kinds("= -> ; : , . ( ) { } [ ] \\"),
            vec![
                TokenKind::Eq,
                TokenKind::Arrow,
                TokenKind::Semicolon,
                TokenKind::Colon,
                TokenKind::Comma,
                TokenKind::Dot,
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::LBracket,
                TokenKind::RBracket,
                TokenKind::Backslash,
            ]
        );
    }

    #[test]
    fn lambda_unicode() {
        assert_eq!(
            kinds("λx"),
            vec![TokenKind::Lambda, TokenKind::Ident("x".into())]
        );
    }

    #[test]
    fn comments() {
        assert_eq!(
            kinds("class // line comment\nproperty /* block */ resource"),
            vec![TokenKind::Class, TokenKind::Property, TokenKind::Resource]
        );
    }

    #[test]
    fn position_tracking() {
        let tokens = tokenize("class\n  property").unwrap();
        assert_eq!(tokens[0].pos, Position { line: 1, column: 1 });
        assert_eq!(tokens[1].pos, Position { line: 2, column: 3 });
    }

    #[test]
    fn full_program() {
        let input = r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Document {
                description = "A document";
                requires ex:text;
            }

            property ex:text : core:string {
                description = "Text content";
            }

            program ex:summarize : ex:Document -> ex:Document {
                let summary : core:string = CompleteText(input);
                Construct ex:Document { text = summary }
            }
        "#;
        let tokens = tokenize(input).unwrap();
        assert!(tokens.len() > 30);
        assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof);
    }

    #[test]
    fn unterminated_string_error() {
        let result = tokenize(r#""hello"#);
        assert!(result.is_err());
    }

    #[test]
    fn string_literal_preserves_utf8() {
        // Multibyte chars must survive lexing intact — before this, `c as char` decoded each UTF-8
        // byte as Latin-1, so `—`/`β` came out as per-byte mojibake (`â…`) and corrupted every
        // non-ASCII gloss and surface form on load.
        assert_eq!(
            kinds(r#""Repeat Pattern — β-catenin café""#),
            vec![TokenKind::StringLit(
                "Repeat Pattern — β-catenin café".into()
            )]
        );
    }

    #[test]
    fn namespace_declaration() {
        assert_eq!(
            kinds(r#"namespace core = "urn:eigenius:core" ;"#),
            vec![
                TokenKind::Namespace,
                TokenKind::Ident("core".into()),
                TokenKind::Eq,
                TokenKind::StringLit("urn:eigenius:core".into()),
                TokenKind::Semicolon,
            ]
        );
    }

    #[test]
    fn program_with_lambda() {
        assert_eq!(
            kinds(r#"\x -> x"#),
            vec![
                TokenKind::Backslash,
                TokenKind::Ident("x".into()),
                TokenKind::Arrow,
                TokenKind::Ident("x".into()),
            ]
        );
    }

    #[test]
    fn construct_with_fields() {
        assert_eq!(
            kinds("Construct ex:Dog { name = x , breed = y }"),
            vec![
                TokenKind::Construct,
                TokenKind::QualName("ex".into(), "Dog".into()),
                TokenKind::LBrace,
                TokenKind::Ident("name".into()),
                TokenKind::Eq,
                TokenKind::Ident("x".into()),
                TokenKind::Comma,
                TokenKind::Ident("breed".into()),
                TokenKind::Eq,
                TokenKind::Ident("y".into()),
                TokenKind::RBrace,
            ]
        );
    }
}
