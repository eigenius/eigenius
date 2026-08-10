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

//! Recursive descent parser for ESL.
//!
//! Consumes tokens from the lexer and produces an AST.
//! Grammar follows D7 §7 with the decisions from design review:
//! - Semicolons for let bindings (not `in`)
//! - Implicit body in program blocks (no `body =`)
//! - Braces + semicolons, not indentation-sensitive

use crate::esl::ast::*;
use crate::esl::error::{EslError, Position};
use crate::esl::lexer::{Token, TokenKind};

/// Parse a token stream into an ESL file AST.
pub fn parse(tokens: &[Token]) -> Result<File, EslError> {
    let mut p = Parser::new(tokens);
    p.parse_file()
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    // --- Token navigation ---

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    /// Look ahead `offset` tokens past the current position. `offset = 0`
    /// is the same as `peek()`. Returns the EOF kind if the lookahead
    /// would step past the end.
    fn peek_at(&self, offset: usize) -> &TokenKind {
        let idx = self.pos.saturating_add(offset);
        if idx < self.tokens.len() {
            &self.tokens[idx].kind
        } else {
            &TokenKind::Eof
        }
    }

    fn current_pos(&self) -> Position {
        self.tokens[self.pos].pos.clone()
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if tok.kind != TokenKind::Eof {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<&Token, EslError> {
        if self.peek() == kind {
            Ok(self.advance())
        } else {
            Err(EslError::parser(
                Some(self.current_pos()),
                format!("expected {:?}, found {:?}", kind, self.peek()),
            ))
        }
    }

    fn expect_ident(&mut self) -> Result<String, EslError> {
        // Accept keywords as identifiers (e.g., `core:resource`, `core:property`).
        // The eigenius#72 sort-literal keywords (Prop, Set, Type) are also
        // accepted because the core ontology has resources at IRIs like
        // `urn:eigenius:core:Set` whose local name parses as the keyword
        // token. Same applies to `axiom` / `forall` which a user might
        // happen to use as a name fragment.
        let name = match self.peek().clone() {
            TokenKind::Ident(name) => name,
            TokenKind::Namespace => "namespace".to_string(),
            TokenKind::Class => "class".to_string(),
            TokenKind::Property => "property".to_string(),
            TokenKind::Resource => "resource".to_string(),
            TokenKind::Program => "program".to_string(),
            TokenKind::Let => "let".to_string(),
            TokenKind::Case => "case".to_string(),
            TokenKind::Construct => "Construct".to_string(),
            TokenKind::Map => "map".to_string(),
            TokenKind::Reduce => "reduce".to_string(),
            TokenKind::Prop => "Prop".to_string(),
            TokenKind::SetKw => "Set".to_string(),
            TokenKind::TypeKw => "Type".to_string(),
            TokenKind::Axiom => "axiom".to_string(),
            TokenKind::Def => "def".to_string(),
            TokenKind::Forall => "forall".to_string(),
            _ => {
                return Err(EslError::parser(
                    Some(self.current_pos()),
                    format!("expected identifier, found {:?}", self.peek()),
                ))
            }
        };
        self.advance();
        Ok(name)
    }

    fn expect_string(&mut self) -> Result<String, EslError> {
        match self.peek().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                Ok(s)
            }
            _ => Err(EslError::parser(
                Some(self.current_pos()),
                format!("expected string literal, found {:?}", self.peek()),
            )),
        }
    }

    fn expect_semicolon(&mut self) -> Result<(), EslError> {
        self.expect(&TokenKind::Semicolon)?;
        Ok(())
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek() == kind
    }

    fn at_eof(&self) -> bool {
        self.at(&TokenKind::Eof)
    }

    // --- Qualified names ---

    /// Parse a qualified name: `ns:name` or bare `name`.
    fn parse_qualified_name(&mut self) -> Result<QualifiedName, EslError> {
        let pos = self.current_pos();
        // A qualified name `ns:name` is one atomic `QualName` token; a bare name
        // is an `Ident`. The standalone `Colon` is never consumed here — it is
        // reserved for the binder / annotation colon (`x : T`, `(e : T)`), so a
        // term ending in a bare identifier followed by ` : T` is unambiguous.
        if let TokenKind::QualName(ns, name) = self.peek() {
            let qn = QualifiedName {
                namespace: Some(ns.clone()),
                name: name.clone(),
                pos,
            };
            self.advance();
            return Ok(qn);
        }
        let first = self.expect_ident()?;
        Ok(QualifiedName {
            namespace: None,
            name: first,
            pos,
        })
    }

    /// Parse a comma-separated list of qualified names.
    fn parse_qualified_name_list(&mut self) -> Result<Vec<QualifiedName>, EslError> {
        let mut names = vec![self.parse_qualified_name()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            names.push(self.parse_qualified_name()?);
        }
        Ok(names)
    }

    // --- File ---

    fn parse_file(&mut self) -> Result<File, EslError> {
        let mut namespaces = Vec::new();
        let mut declarations = Vec::new();

        while !self.at_eof() {
            match self.peek() {
                TokenKind::Namespace => namespaces.push(self.parse_namespace()?),
                TokenKind::Class => declarations.push(Declaration::Class(self.parse_class()?)),
                TokenKind::Property => {
                    declarations.push(Declaration::Property(self.parse_property()?))
                }
                TokenKind::Resource => {
                    declarations.push(Declaration::Resource(self.parse_resource()?))
                }
                TokenKind::Program => {
                    declarations.push(Declaration::Program(self.parse_program()?))
                }
                TokenKind::Codata => {
                    declarations.push(Declaration::Codata(self.parse_codata()?))
                }
                TokenKind::Data => declarations.push(Declaration::Data(self.parse_data()?)),
                TokenKind::MergeComorphism => declarations.push(Declaration::MergeComorphism(
                    self.parse_merge_comorphism()?,
                )),
                TokenKind::TextIndex => {
                    declarations.push(Declaration::TextIndex(self.parse_text_index()?))
                }
                TokenKind::VectorIndex => {
                    declarations.push(Declaration::VectorIndex(self.parse_vector_index()?))
                }
                TokenKind::Axiom => {
                    declarations.push(Declaration::Axiom(self.parse_axiom()?))
                }
                TokenKind::Def => declarations.push(Declaration::Def(self.parse_def()?)),
                TokenKind::Macro => {
                    declarations.push(Declaration::Macro(self.parse_macro()?))
                }
                _ => {
                    return Err(EslError::parser(
                        Some(self.current_pos()),
                        format!(
                            "expected top-level declaration (namespace, class, property, resource, program, codata, data, merge_comorphism, text_index, vector_index, axiom, def, macro), found {:?}",
                            self.peek()
                        ),
                    ))
                }
            }
        }

        Ok(File {
            namespaces,
            declarations,
        })
    }

    // --- Namespace ---

    /// `namespace core = "urn:eigenius:core";`
    fn parse_namespace(&mut self) -> Result<NamespaceDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Namespace)?;
        let alias = self.expect_ident()?;
        self.expect(&TokenKind::Eq)?;
        let uri = self.expect_string()?;
        self.expect_semicolon()?;
        Ok(NamespaceDecl { alias, uri, pos })
    }

    // --- Class ---

    /// `class ex:Dog : ex:Animal { ... }` or, for multiple parents,
    /// `class ex:HybridCell : ex:Cell, ex:Visualisable { ... }`
    /// (eigenius#29). The colon + class list is optional; an empty
    /// list means the class has no superclasses authored at the
    /// header. Body-level `subclass_of` items extend the same set.
    fn parse_class(&mut self) -> Result<ClassDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Class)?;
        let name = self.parse_qualified_name()?;

        let parents = if self.at(&TokenKind::Colon) {
            self.advance();
            self.parse_qualified_name_list()?
        } else {
            Vec::new()
        };

        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            body.push(self.parse_class_item()?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(ClassDecl {
            name,
            parents,
            body,
            pos,
        })
    }

    fn parse_class_item(&mut self) -> Result<ClassItem, EslError> {
        let name = match self.peek().clone() {
            TokenKind::Ident(n) => n,
            _ => {
                return Err(EslError::parser(
                    Some(self.current_pos()),
                    format!(
                        "expected class item (description, requires, recommends), found {:?}",
                        self.peek()
                    ),
                ))
            }
        };

        match name.as_str() {
            "description" => {
                self.advance();
                self.expect(&TokenKind::Eq)?;
                let s = self.expect_string()?;
                self.expect_semicolon()?;
                Ok(ClassItem::Description(s))
            }
            "requires" => {
                self.advance();
                let names = self.parse_qualified_name_list()?;
                self.expect_semicolon()?;
                Ok(ClassItem::Requires(names))
            }
            "recommends" => {
                self.advance();
                let names = self.parse_qualified_name_list()?;
                self.expect_semicolon()?;
                Ok(ClassItem::Recommends(names))
            }
            _ => Err(EslError::parser(
                Some(self.current_pos()),
                format!("unknown class item: '{name}'"),
            )),
        }
    }

    // --- Property ---

    /// `property ex:name : core:string { ... }`
    fn parse_property(&mut self) -> Result<PropertyDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Property)?;
        let name = self.parse_qualified_name()?;
        self.expect(&TokenKind::Colon)?;
        let data_type = self.parse_qualified_name()?;

        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            body.push(self.parse_property_item()?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(PropertyDecl {
            name,
            data_type,
            body,
            pos,
        })
    }

    fn parse_property_item(&mut self) -> Result<PropertyItem, EslError> {
        let name = match self.peek().clone() {
            TokenKind::Ident(n) => n,
            _ => {
                return Err(EslError::parser(
                    Some(self.current_pos()),
                    format!("expected property item, found {:?}", self.peek()),
                ))
            }
        };

        match name.as_str() {
            "description" => {
                self.advance();
                self.expect(&TokenKind::Eq)?;
                let s = self.expect_string()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::Description(s))
            }
            "min_value" => {
                self.advance();
                self.expect(&TokenKind::Eq)?;
                let v = self.parse_number_f64()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::MinValue(v))
            }
            "max_value" => {
                self.advance();
                self.expect(&TokenKind::Eq)?;
                let v = self.parse_number_f64()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::MaxValue(v))
            }
            "min_length" => {
                self.advance();
                self.expect(&TokenKind::Eq)?;
                let v = self.parse_number_i64()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::MinLength(v))
            }
            "max_length" => {
                self.advance();
                self.expect(&TokenKind::Eq)?;
                let v = self.parse_number_i64()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::MaxLength(v))
            }
            "pattern" => {
                self.advance();
                self.expect(&TokenKind::Eq)?;
                let s = self.expect_string()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::Pattern(s))
            }
            "format" => {
                self.advance();
                self.expect(&TokenKind::Eq)?;
                let f = self.parse_qualified_name()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::Format(f))
            }
            "allows_only" => {
                self.advance();
                let names = self.parse_qualified_name_list()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::AllowsOnly(names))
            }
            "domain" => {
                self.advance();
                let names = self.parse_qualified_name_list()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::Domain(names))
            }
            "class_types" => {
                self.advance();
                let names = self.parse_qualified_name_list()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::ClassTypes(names))
            }
            "element_type" => {
                self.advance();
                self.expect(&TokenKind::Eq)?;
                let t = self.parse_qualified_name()?;
                self.expect_semicolon()?;
                Ok(PropertyItem::ElementType(t))
            }
            _ => Err(EslError::parser(
                Some(self.current_pos()),
                format!("unknown property item: '{name}'"),
            )),
        }
    }

    fn parse_number_f64(&mut self) -> Result<f64, EslError> {
        match self.peek().clone() {
            TokenKind::IntLit(n) => {
                self.advance();
                Ok(n as f64)
            }
            TokenKind::FloatLit(f) => {
                self.advance();
                Ok(f)
            }
            _ => Err(EslError::parser(
                Some(self.current_pos()),
                format!("expected number, found {:?}", self.peek()),
            )),
        }
    }

    fn parse_number_i64(&mut self) -> Result<i64, EslError> {
        match self.peek().clone() {
            TokenKind::IntLit(n) => {
                self.advance();
                Ok(n)
            }
            _ => Err(EslError::parser(
                Some(self.current_pos()),
                format!("expected integer, found {:?}", self.peek()),
            )),
        }
    }

    // --- Resource ---

    /// `resource ex:rex : ex:Dog { ... }` or
    /// `resource ex:rex : ex:Dog, ex:Pet { ... }` for multi-class
    /// resources (eigenius#29).
    fn parse_resource(&mut self) -> Result<ResourceDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Resource)?;
        let name = self.parse_qualified_name()?;
        self.expect(&TokenKind::Colon)?;
        let classes = self.parse_qualified_name_list()?;

        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            body.push(self.parse_resource_field()?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(ResourceDecl {
            name,
            classes,
            body,
            pos,
        })
    }

    fn parse_resource_field(&mut self) -> Result<ResourceField, EslError> {
        let property = self.parse_qualified_name()?;
        self.expect(&TokenKind::Eq)?;
        let value = self.parse_value()?;
        self.expect_semicolon()?;
        Ok(ResourceField { property, value })
    }

    fn parse_value(&mut self) -> Result<Value, EslError> {
        match self.peek().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                Ok(Value::String(s))
            }
            TokenKind::IntLit(n) => {
                self.advance();
                Ok(Value::Int(n))
            }
            TokenKind::FloatLit(f) => {
                self.advance();
                Ok(Value::Float(f))
            }
            TokenKind::BoolLit(b) => {
                self.advance();
                Ok(Value::Bool(b))
            }
            // Unary minus on a numeric literal — preserves the
            // pre-Phase-19f.3 shape `ex:value = -1.5;`. Lexer used to
            // sign-fold negative numbers; now it always emits `Minus`
            // (so `formula(x - 2)` lexes correctly), and the
            // negation is applied here. Only literals are accepted in
            // this position; arithmetic on arbitrary refs lives
            // inside `formula(...)`.
            TokenKind::Minus => {
                let pos = self.current_pos();
                self.advance();
                match self.peek().clone() {
                    TokenKind::IntLit(n) => {
                        self.advance();
                        Ok(Value::Int(-n))
                    }
                    TokenKind::FloatLit(f) => {
                        self.advance();
                        Ok(Value::Float(-f))
                    }
                    other => Err(EslError::parser(
                        Some(pos),
                        format!(
                            "unary `-` outside `formula(...)` only applies to numeric literals; got {other:?}"
                        ),
                    )),
                }
            }
            TokenKind::LBracket => self.parse_array_value(),
            TokenKind::LBrace => self.parse_block_value(),
            // A bare identifier OR a qualified name `ns:name` (one atomic
            // `QualName` token) in value position: a resource ref, an inductive
            // ctor application, a macro call, or the bare-keyword forms
            // `formula(...)` / `type_expr(...)`. The bare keywords are matched
            // first (they are never qualified); everything else goes through the
            // shared `parse_qualified_name` + ref/ctor/macro disambiguation.
            TokenKind::Ident(_) | TokenKind::QualName(..) => {
                let qn_pos = self.current_pos();
                let bare = match self.peek() {
                    TokenKind::Ident(name) => Some(name.clone()),
                    _ => None,
                };
                // `formula(...)` switches into the Pratt-parsed
                // math-expression sublanguage (Phase 19f.3): infix
                // arithmetic with conventional precedence, function
                // calls (`sin(x)`), parens for grouping, bare names
                // as `Var`, decimal numbers as `LitFloat`. The parsed
                // tree is a chain-typed FormulaTerm — same `Value::CtorApp`
                // shape the `App(...)` / `OpRef(...)` literal form
                // produces.
                if bare.as_deref() == Some("formula") && self.peek_at(1) == &TokenKind::LParen {
                    self.advance(); // consume `formula`
                    self.expect(&TokenKind::LParen)?;
                    let expr = self.parse_formula_expr(0)?;
                    self.expect(&TokenKind::RParen)?;
                    return Ok(expr);
                }
                // `type_expr(...)` — inline D47-encoded EigenTT type
                // expression. Parses the inner expression with the
                // same `parse_type_expr` used for `axiom` declarations
                // and `data` ctor types; the compile-side hook D47-
                // encodes the lowered Exp. Surface counterpart of
                // `formula(...)` for D47, used by D39 ReasoningSentence
                // authors who'd otherwise hand-write the verbose
                // `{"ctor":"App","args":[...]}` tagged-dict tree.
                if bare.as_deref() == Some("type_expr") && self.peek_at(1) == &TokenKind::LParen {
                    let pos = self.current_pos();
                    self.advance(); // consume `type_expr`
                    self.expect(&TokenKind::LParen)?;
                    let typ = self.parse_type_expr()?;
                    self.expect(&TokenKind::RParen)?;
                    return Ok(Value::TypeExpr { typ, pos });
                }
                let qn = self.parse_qualified_name()?;
                // Bare `Ident` followed by `(` is an inductive-ctor
                // application — `Foo(arg1, arg2, ...)` per D32
                // inductive-value literals. Nullary ctors require
                // empty parens (`Foo()`) so this peek is the only
                // place the parser disambiguates ctors vs. resource
                // refs. Namespace-qualified names (`ns:Foo`) followed
                // by `(` are not v1 — ctors live in an inductive's
                // per-type scope, not a global namespace, and
                // resolving `ns:Foo` to a ctor would require chain
                // context the parser doesn't have.
                if qn.namespace.is_none() && self.at(&TokenKind::LParen) {
                    self.expect(&TokenKind::LParen)?;
                    let mut args = Vec::new();
                    if !self.at(&TokenKind::RParen) {
                        args.push(self.parse_value()?);
                        while self.at(&TokenKind::Comma) {
                            self.advance();
                            if self.at(&TokenKind::RParen) {
                                break; // trailing comma
                            }
                            args.push(self.parse_value()?);
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    Ok(Value::CtorApp {
                        ctor: qn.name,
                        args,
                        pos: qn_pos,
                    })
                } else if qn.namespace.is_some() && self.at(&TokenKind::LParen) {
                    // D52 §12 — qualified `ns:Name(args)` parses as a
                    // macro call rather than a constructor application
                    // (which is unqualified) or a resource reference
                    // (which has no `(`). The compiler resolves the
                    // qualified name to a registered `MacroDecl` and
                    // expands the call site by AST substitution.
                    self.expect(&TokenKind::LParen)?;
                    let mut args = Vec::new();
                    if !self.at(&TokenKind::RParen) {
                        args.push(self.parse_value()?);
                        while self.at(&TokenKind::Comma) {
                            self.advance();
                            if self.at(&TokenKind::RParen) {
                                break; // trailing comma
                            }
                            args.push(self.parse_value()?);
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    Ok(Value::MacroCall {
                        name: qn,
                        args,
                        pos: qn_pos,
                    })
                } else {
                    Ok(Value::Ref(qn))
                }
            }
            _ => Err(EslError::parser(
                Some(self.current_pos()),
                format!("expected value, found {:?}", self.peek()),
            )),
        }
    }

    // ─── formula(...) sublanguage (Phase 19f.3) ─────────────────────────
    //
    // Pratt parser for math-style infix expressions inside
    // `formula(...)`. Output is a `Value::CtorApp` tree mirroring the
    // chain-typed FormulaTerm; the existing `compile_value` lowers it
    // to `Value::Json` carrying the `{ctor, args}` tagged-dict shape.
    //
    // Operator catalog (left-to-right precedence levels):
    //
    //   prec 4 (left assoc):  +    →  ops:add
    //   prec 4 (left assoc):  -    →  ops:sub  (binary)
    //   prec 5 (left assoc):  *    →  ops:mul
    //   prec 5 (left assoc):  /    →  ops:div
    //   prec 6 (prefix):       -   →  ops:neg  (or LitFloat sign-fold for literals)
    //   prec 7 (right assoc): ^    →  ops:pow
    //
    // Atoms:
    //   IntLit/FloatLit  →  LitFloat(value)
    //   Ident            →  Var("name")
    //   Ident "(" args ")" →  App-spine over `urn:eigenius:formulas:ops:<name>`
    //   "(" expr ")"     →  grouped expression

    /// Operator-IRI table for binary infix operators inside
    /// `formula(...)`. Returns `(precedence, right_associative, op_iri)`.
    fn formula_binop_table(token: &TokenKind) -> Option<(u8, bool, &'static str)> {
        match token {
            TokenKind::Plus => Some((4, false, "urn:eigenius:formulas:ops:add")),
            TokenKind::Minus => Some((4, false, "urn:eigenius:formulas:ops:sub")),
            TokenKind::Star => Some((5, false, "urn:eigenius:formulas:ops:mul")),
            TokenKind::Slash => Some((5, false, "urn:eigenius:formulas:ops:div")),
            TokenKind::Caret => Some((7, true, "urn:eigenius:formulas:ops:pow")),
            _ => None,
        }
    }

    /// Parse one formula expression at or above the given minimum
    /// precedence. Standard Pratt-parser shape — atom first, then
    /// fold binary operators of sufficient precedence into the
    /// running result.
    fn parse_formula_expr(&mut self, min_prec: u8) -> Result<Value, EslError> {
        let mut left = self.parse_formula_atom()?;
        while let Some((prec, right_assoc, op_iri)) = Self::formula_binop_table(self.peek()) {
            if prec < min_prec {
                break;
            }
            let op_pos = self.current_pos();
            self.advance(); // consume binop
            let next_min = if right_assoc { prec } else { prec + 1 };
            let right = self.parse_formula_expr(next_min)?;
            left = formula_binary_app(op_iri, left, right, op_pos);
        }
        Ok(left)
    }

    /// Parse a formula-expression atom: literal, var, function call,
    /// parens, or unary `+` / `-`.
    fn parse_formula_atom(&mut self) -> Result<Value, EslError> {
        let pos = self.current_pos();
        match self.peek().clone() {
            TokenKind::IntLit(n) => {
                self.advance();
                Ok(formula_lit_float(n as f64, pos))
            }
            TokenKind::FloatLit(f) => {
                self.advance();
                Ok(formula_lit_float(f, pos))
            }
            // Unary minus: fold sign into a numeric atom; otherwise
            // wrap with `urn:eigenius:formulas:ops:neg`.
            TokenKind::Minus => {
                self.advance();
                match self.peek().clone() {
                    TokenKind::IntLit(n) => {
                        self.advance();
                        Ok(formula_lit_float(-(n as f64), pos))
                    }
                    TokenKind::FloatLit(f) => {
                        self.advance();
                        Ok(formula_lit_float(-f, pos))
                    }
                    _ => {
                        let inner = self.parse_formula_atom()?;
                        Ok(Value::CtorApp {
                            ctor: "App".to_string(),
                            args: vec![
                                formula_op_ref("urn:eigenius:formulas:ops:neg", pos.clone()),
                                inner,
                            ],
                            pos,
                        })
                    }
                }
            }
            // Unary plus: just discard.
            TokenKind::Plus => {
                self.advance();
                self.parse_formula_atom()
            }
            // Parenthesised sub-expression.
            TokenKind::LParen => {
                self.advance();
                let inner = self.parse_formula_expr(0)?;
                self.expect(&TokenKind::RParen)?;
                Ok(inner)
            }
            // Identifier — either a function call (`sin(x)`) or a
            // bare `Var(name)`. Function names map directly to the
            // chain operator catalog at `urn:eigenius:formulas:ops:<name>`.
            TokenKind::Ident(name) => {
                self.advance();
                if self.at(&TokenKind::LParen) {
                    self.expect(&TokenKind::LParen)?;
                    let mut args: Vec<Value> = Vec::new();
                    if !self.at(&TokenKind::RParen) {
                        args.push(self.parse_formula_expr(0)?);
                        while self.at(&TokenKind::Comma) {
                            self.advance();
                            if self.at(&TokenKind::RParen) {
                                break;
                            }
                            args.push(self.parse_formula_expr(0)?);
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    let op_iri = format!("urn:eigenius:formulas:ops:{name}");
                    Ok(formula_function_call(&op_iri, args, pos))
                } else {
                    Ok(Value::CtorApp {
                        ctor: "Var".to_string(),
                        args: vec![Value::String(name)],
                        pos,
                    })
                }
            }
            other => Err(EslError::parser(
                Some(pos),
                format!("expected formula atom, found {other:?}"),
            )),
        }
    }

    fn parse_array_value(&mut self) -> Result<Value, EslError> {
        self.expect(&TokenKind::LBracket)?;
        let mut values = Vec::new();
        if !self.at(&TokenKind::RBracket) {
            values.push(self.parse_value()?);
            while self.at(&TokenKind::Comma) {
                self.advance();
                if self.at(&TokenKind::RBracket) {
                    break; // trailing comma
                }
                values.push(self.parse_value()?);
            }
        }
        self.expect(&TokenKind::RBracket)?;
        Ok(Value::Array(values))
    }

    fn parse_block_value(&mut self) -> Result<Value, EslError> {
        self.expect(&TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            fields.push(self.parse_resource_field()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Value::Block(fields))
    }

    // --- Program ---

    /// `program ex:summarize : ex:Document -> ex:Summary { expr }`
    fn parse_program(&mut self) -> Result<ProgramDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Program)?;
        let name = self.parse_qualified_name()?;
        self.expect(&TokenKind::Colon)?;
        let input_type = self.parse_qualified_name()?;
        self.expect(&TokenKind::Arrow)?;
        let output_type = self.parse_qualified_name()?;

        self.expect(&TokenKind::LBrace)?;

        // Parse optional attributes before the expression
        let mut attributes = Vec::new();
        while self.is_program_attribute() {
            attributes.push(self.parse_program_attribute()?);
        }

        // The rest of the block is the expression body
        let body = self.parse_expr()?;

        self.expect(&TokenKind::RBrace)?;

        Ok(ProgramDecl {
            name,
            input_type,
            output_type,
            attributes,
            body,
            pos,
        })
    }

    /// Check if the current position starts a program attribute (description = "...";)
    /// vs an expression. An attribute is `ident = string ;`.
    fn is_program_attribute(&self) -> bool {
        if let TokenKind::Ident(name) = self.peek() {
            if name == "description" {
                // Look ahead for `= "..."`
                if self.pos + 1 < self.tokens.len() {
                    return self.tokens[self.pos + 1].kind == TokenKind::Eq;
                }
            }
        }
        false
    }

    fn parse_program_attribute(&mut self) -> Result<ProgramAttribute, EslError> {
        let name = self.expect_ident()?;
        match name.as_str() {
            "description" => {
                self.expect(&TokenKind::Eq)?;
                let s = self.expect_string()?;
                self.expect_semicolon()?;
                Ok(ProgramAttribute::Description(s))
            }
            _ => Err(EslError::parser(
                Some(self.current_pos()),
                format!("unknown program attribute: '{name}'"),
            )),
        }
    }

    // --- Codata ---

    /// `codata ex:Stream { head : ex:Elem; tail : ex:Stream }`
    fn parse_codata(&mut self) -> Result<CodataDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Codata)?;
        let name = self.parse_qualified_name()?;

        // Optional type parameters, e.g. `(i : core:Size, A : core:Set)`.
        let params = if self.at(&TokenKind::LParen) {
            self.parse_data_params()?
        } else {
            Vec::new()
        };

        self.expect(&TokenKind::LBrace)?;

        let mut observations = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let obs_pos = self.current_pos();
            let obs_name = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            let typ = self.parse_type_expr()?;
            self.expect_semicolon()?;
            observations.push(ObservationDecl {
                name: obs_name,
                typ,
                pos: obs_pos,
            });
        }
        self.expect(&TokenKind::RBrace)?;

        if observations.is_empty() {
            return Err(EslError::parser(
                Some(pos.clone()),
                "codata type must declare at least one observation".to_string(),
            ));
        }

        Ok(CodataDecl {
            name,
            params,
            observations,
            pos,
        })
    }

    /// Parse a type expression (Phase 11b step 15h.3).
    ///
    /// Grammar (right-associative arrow, tight-binding ref):
    /// ```text
    /// TypeExpr ::= Atom '->' TypeExpr        -- arrow
    ///            | BinderArrow               -- `{j < i} -> …`
    ///            | Atom
    /// Atom     ::= QualifiedName [ '(' TypeExpr (',' TypeExpr)* ')' ]
    /// BinderArrow ::= '{' ident (':' QualifiedName)? ('<' QualifiedName)? '}' '->' TypeExpr
    /// ```
    fn parse_type_expr(&mut self) -> Result<TypeExpr, EslError> {
        let pos = self.current_pos();

        // D37 §3.5 — `pi x_1 : T_1, …, x_N : T_N => U`. The value-
        // typed Pi binder. Distinct from the size-binder form
        // (`{j : Size < bound} -> body`, below) which compiles to
        // `Exp::SizedPi` / `Exp::Pi` for size kinds, and from the
        // anonymous arrow `A -> B`.
        //
        // eigenius#72: `forall` is an alias for `pi` with the same
        // syntax — natural for proposition authoring.
        if self.at(&TokenKind::Pi) || self.at(&TokenKind::Forall) {
            return self.parse_pi_type();
        }

        // `exists x : T, y : U . B` — the Sigma binder, dual of `forall`. Terminated by `.`
        // rather than `=>` so it reads like the pair it builds; the DCG's own pretty-printer
        // uses the same shape (`SG#0:C. body`).
        if self.at(&TokenKind::Exists) {
            return self.parse_sigma_type();
        }

        // eigenius#72 Layer 3 — `fun (i_1 : T_1, …) => body` lambda in
        // type position. Used as a motive for `match … returning <motive>`
        // over indexed inductives. Parses through the same typed-param
        // list machinery as `pi`/`forall`.
        if self.at(&TokenKind::Fun) {
            return self.parse_fun_lambda();
        }

        // Compile-time aliases in type-expression position:
        // `alias name = expr, name = expr, ... in body`. The compiler
        // substitutes each name's bound expression at lowering time;
        // no D47 encoding is produced for the bindings themselves.
        // Distinct from `let` which marks a kernel-level binding.
        if self.at(&TokenKind::Alias) {
            return self.parse_alias_in_type_expr();
        }

        // eigenius#72 — sort literals at the head of a type
        // expression: `Prop`, `Set`, or `Type N`. May then be
        // followed by `-> ...` (an arrow whose domain is a sort).
        if self.at(&TokenKind::Prop) || self.at(&TokenKind::SetKw) || self.at(&TokenKind::TypeKw) {
            let sort = self.parse_sort_literal()?;
            if self.at(&TokenKind::Arrow) {
                self.advance();
                let codomain = self.parse_type_expr()?;
                return Ok(TypeExpr::Arrow {
                    domain: Box::new(sort),
                    codomain: Box::new(codomain),
                    pos,
                });
            }
            return Ok(sort);
        }

        // Size-binder arrow form — unambiguous because codata obs
        // types start fresh (no braces ever appear as a normal type
        // here).
        if self.at(&TokenKind::LBrace) {
            self.advance();
            let name = self.expect_ident()?;
            let (kind, bound) = if self.at(&TokenKind::Colon) {
                self.advance();
                let kind = self.parse_qualified_name()?;
                let bound = if self.at(&TokenKind::Less) {
                    self.advance();
                    Some(self.parse_qualified_name()?)
                } else {
                    None
                };
                (kind, bound)
            } else if self.at(&TokenKind::Less) {
                self.advance();
                let bound = self.parse_qualified_name()?;
                (
                    QualifiedName {
                        namespace: None,
                        name: "Size".to_string(),
                        pos: pos.clone(),
                    },
                    Some(bound),
                )
            } else {
                return Err(EslError::parser(
                    Some(self.current_pos()),
                    format!(
                        "expected ':' or '<' after binder name in type expression, \
                         found {:?}",
                        self.peek()
                    ),
                ));
            };
            self.expect(&TokenKind::RBrace)?;
            self.expect(&TokenKind::Arrow)?;
            let body = self.parse_type_expr()?;
            return Ok(TypeExpr::BinderArrow {
                name,
                kind,
                bound,
                body: Box::new(body),
                pos,
            });
        }

        // Atom, optionally followed by `->` for a non-dependent arrow.
        let atom = self.parse_type_atom()?;
        if self.at(&TokenKind::Arrow) {
            self.advance();
            let codomain = self.parse_type_expr()?;
            Ok(TypeExpr::Arrow {
                domain: Box::new(atom),
                codomain: Box::new(codomain),
                pos,
            })
        } else {
            Ok(atom)
        }
    }

    /// eigenius#72 — parse a sort literal as a `TypeExpr::Sort`.
    /// `Prop` / `Set` are nullary; `Type` requires a following
    /// integer level.
    fn parse_sort_literal(&mut self) -> Result<TypeExpr, EslError> {
        let pos = self.current_pos();
        if self.at(&TokenKind::Prop) {
            self.advance();
            return Ok(TypeExpr::Sort {
                kind: crate::esl::ast::SortKind::Prop,
                pos,
            });
        }
        if self.at(&TokenKind::SetKw) {
            self.advance();
            return Ok(TypeExpr::Sort {
                kind: crate::esl::ast::SortKind::Set,
                pos,
            });
        }
        if self.at(&TokenKind::TypeKw) {
            self.advance();
            let level = match self.peek().clone() {
                TokenKind::IntLit(n) if n >= 0 => {
                    self.advance();
                    n as usize
                }
                other => {
                    return Err(EslError::parser(
                        Some(self.current_pos()),
                        format!(
                            "expected non-negative integer level after `Type`, found {other:?}"
                        ),
                    ));
                }
            };
            return Ok(TypeExpr::Sort {
                kind: crate::esl::ast::SortKind::Type(level),
                pos,
            });
        }
        Err(EslError::parser(
            Some(pos),
            "expected a sort literal (Prop, Set, or Type N)".to_string(),
        ))
    }

    fn parse_type_atom(&mut self) -> Result<TypeExpr, EslError> {
        let pos = self.current_pos();
        // Parenthesized (grouped) type — lets a function type appear as an arrow
        // domain, e.g. `(A -> Prop) -> Prop` (higher-order types, as in
        // generalized-quantifier determiners). A leading `(` groups; a `(` *after*
        // a name (below) is constructor application, not grouping.
        if self.at(&TokenKind::LParen) {
            self.advance();
            // `()` — the unit value. Checked before parsing an inner type so the empty
            // parenthesis pair is not a parse error.
            if self.at(&TokenKind::RParen) {
                self.advance();
                return Ok(TypeExpr::Unit { pos });
            }
            let inner = self.parse_type_expr()?;
            // `(e : T)` — a type annotation (the bidirectional mode switch),
            // distinct from plain `(e)` grouping. Lets a checkable term (a `fun`
            // lambda) appear in inference position, e.g. a determiner's λ-sem.
            if self.at(&TokenKind::Colon) {
                self.advance();
                let typ = self.parse_type_expr()?;
                self.expect(&TokenKind::RParen)?;
                return Ok(TypeExpr::Ann {
                    expr: Box::new(inner),
                    typ: Box::new(typ),
                    pos,
                });
            }
            self.expect(&TokenKind::RParen)?;
            return Ok(inner);
        }
        // Literal in a type position — lowers to Exp::LitString /
        // LitInt / LitFloat. Required by `type_expr(...)` so authors
        // can write predicate applications with concrete arguments,
        // e.g. `Asserts("urn:foo")` or `Vec(3, A)`. Literals at the
        // head of a type-expression form an atom on their own; they
        // cannot take type-args, so the qualified-name path below
        // is bypassed entirely.
        match self.peek().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                return Ok(TypeExpr::LitString { value: s, pos });
            }
            TokenKind::IntLit(n) => {
                self.advance();
                return Ok(TypeExpr::LitInt { value: n, pos });
            }
            TokenKind::FloatLit(f) => {
                self.advance();
                return Ok(TypeExpr::LitFloat { value: f, pos });
            }
            _ => {}
        }
        let name = self.parse_qualified_name()?;
        let args = if self.at(&TokenKind::LParen) {
            self.advance();
            let mut args = Vec::new();
            while !self.at(&TokenKind::RParen) && !self.at_eof() {
                args.push(self.parse_type_expr()?);
                if self.at(&TokenKind::Comma) {
                    self.advance();
                } else if !self.at(&TokenKind::RParen) {
                    return Err(EslError::parser(
                        Some(self.current_pos()),
                        format!(
                            "expected ',' or ')' in type argument list, found {:?}",
                            self.peek()
                        ),
                    ));
                }
            }
            self.expect(&TokenKind::RParen)?;
            args
        } else {
            Vec::new()
        };
        Ok(TypeExpr::Ref { name, args, pos })
    }

    /// D37 §3.5 — `pi x_1 : T_1, …, x_N : T_N => U`. Value-typed
    /// Pi expression for forward-declared signatures and for the
    /// `program:type` slot on standalone Lambda resources.
    ///
    /// Single-parameter form is `pi x : T => U`. Multi-parameter is
    /// comma-separated; the compiler folds N binders into N nested
    /// single-parameter `Exp::Pi` nodes, with U as the innermost
    /// return type.
    fn parse_pi_type(&mut self) -> Result<TypeExpr, EslError> {
        let pos = self.current_pos();
        // Either `pi` (D37 §3.5) or `forall` (eigenius#72 alias) opens
        // the binder list. Both lower to the same `TypeExpr::Pi`.
        if self.at(&TokenKind::Pi) || self.at(&TokenKind::Forall) {
            self.advance();
        } else {
            return Err(EslError::parser(
                Some(pos.clone()),
                "expected `pi` or `forall` to open a Pi type".to_string(),
            ));
        }
        let params = self.parse_typed_param_list()?;
        self.expect(&TokenKind::FatArrow)?;
        let codomain = self.parse_type_expr()?;
        Ok(TypeExpr::Pi {
            params,
            codomain: Box::new(codomain),
            pos,
        })
    }

    /// `exists x_1 : T_1, ..., x_N : T_N . B` — nested `TypeExpr::Sigma`.
    ///
    /// Mirrors [`Self::parse_pi_type`], with `.` closing the binder list instead of `=>`.
    fn parse_sigma_type(&mut self) -> Result<TypeExpr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Exists)?;
        let params = self.parse_typed_param_list()?;
        // `=>`, exactly as `forall` — a second binder that terminated differently would be an
        // inconsistency with no payoff, and it would burn `.`, the one token a postfix `.1`/`.2`
        // projection form would need. (We chose `eigentt:fst(p)` over `p.1` precisely because
        // ESL has no postfix form yet; leave the door open.)
        self.expect(&TokenKind::FatArrow)?;
        let body = self.parse_type_expr()?;
        Ok(TypeExpr::Sigma {
            params,
            body: Box::new(body),
            pos,
        })
    }

    /// Compile-time aliases in `type_expr(...)` position:
    ///
    /// ```text
    /// alias NAME1 = expr1,
    ///       NAME2 = expr2,
    ///       ...
    /// in body
    /// ```
    ///
    /// Each binding's value is a full `TypeExpr` (an IRI literal, a
    /// proposition, a sub-justification term, anything). The compiler
    /// substitutes the bindings into the body at lowering time —
    /// purely surface sugar; the kernel's NbE never sees the `Alias`
    /// form. Sequential lexical scope: each later binding's value
    /// can reference earlier bindings.
    ///
    /// Used to factor out repeated IRI strings, propositions, and
    /// sub-justification spines in large certificate terms.
    ///
    /// Distinct from kernel-level `let` (`Decl::Def` in NbE; surface
    /// `let x : T = e; body` in the program-body parser). The two
    /// have separate keywords intentionally — `alias` is pure
    /// compile-time substitution, `let` is a real definitional
    /// binding the type theory sees.
    fn parse_alias_in_type_expr(&mut self) -> Result<TypeExpr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Alias)?;
        let mut bindings = Vec::new();
        loop {
            let binding_pos = self.current_pos();
            let name = self.expect_ident()?;
            self.expect(&TokenKind::Eq)?;
            let value = self.parse_type_expr()?;
            bindings.push(AliasBinding {
                name,
                value,
                pos: binding_pos,
            });
            if self.at(&TokenKind::Comma) {
                self.advance();
                continue;
            }
            break;
        }
        self.expect(&TokenKind::In)?;
        let body = self.parse_type_expr()?;
        Ok(TypeExpr::Alias {
            bindings,
            body: Box::new(body),
            pos,
        })
    }

    /// eigenius#72 Layer 3 — `fun (i_1 : T_1, …, i_n : T_n) => body`
    /// lambda in type position. Mirrors `parse_pi_type` exactly except
    /// for the keyword and the resulting AST variant.
    fn parse_fun_lambda(&mut self) -> Result<TypeExpr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Fun)?;
        let params = self.parse_typed_param_list()?;
        self.expect(&TokenKind::FatArrow)?;
        let body = self.parse_type_expr()?;
        Ok(TypeExpr::Lambda {
            params,
            body: Box::new(body),
            pos,
        })
    }

    /// `x_1 : T_1, x_2 : T_2, ...` — typed binder list shared by
    /// `pi` (D37 §3.5) and the typed `lambda` literal (D37 §3.1).
    /// Requires at least one binder; the list ends when the parser
    /// sees a non-comma token after a binder (typically `=>`).
    fn parse_typed_param_list(&mut self) -> Result<Vec<TypedParam>, EslError> {
        // eigenius#72: allow an outer `(...)` wrap around the binder
        // list — natural for proposition authoring
        // (`forall (P : Prop) => P -> P`). The existing D37 §3.5 `pi`
        // form (`pi x : T => U`) without parens stays valid too; we
        // peek for `(` and parse a parenthesised list when present.
        let parenthesised = self.at(&TokenKind::LParen);
        if parenthesised {
            self.advance();
        }
        let mut params = Vec::new();
        loop {
            let pos = self.current_pos();
            let name = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            let typ = self.parse_type_expr()?;
            params.push(TypedParam { name, typ, pos });
            if !self.at(&TokenKind::Comma) {
                break;
            }
            self.advance();
        }
        if parenthesised {
            self.expect(&TokenKind::RParen)?;
        }
        Ok(params)
    }

    /// D37 §3.3 — `merge_comorphism <iri> for <class> { <body> }`.
    ///
    /// Body is either:
    ///
    /// - **Inline:** `(a, b, opt) => <expr>` — parameter types are
    ///   inferred from the surrounding `for <class>` clause as
    ///   `(class, class, Option<class>)`. The expression's type is
    ///   inferred from the body.
    /// - **Reference:** `transformation = <iri>;` — references a
    ///   previously-declared standalone Lambda resource.
    ///
    /// Discriminator: an opening `(` after the `{` means inline; the
    /// contextual keyword `transformation` means reference. No
    /// ambiguity since the two forms can't overlap syntactically.
    fn parse_merge_comorphism(&mut self) -> Result<MergeComorphismDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::MergeComorphism)?;
        let name = self.parse_qualified_name()?;
        self.expect(&TokenKind::For)?;
        let target_class = self.parse_qualified_name()?;
        self.expect(&TokenKind::LBrace)?;

        let body = match self.peek() {
            TokenKind::LParen => self.parse_merge_comorphism_inline_body()?,
            TokenKind::Ident(s) if s == "transformation" => {
                self.parse_merge_comorphism_reference_body()?
            }
            other => {
                return Err(EslError::parser(
                    Some(self.current_pos()),
                    format!(
                        "expected '(' for an inline lambda body or 'transformation = <iri>' \
                         for a reference body, found {other:?}"
                    ),
                ));
            }
        };

        self.expect(&TokenKind::RBrace)?;
        Ok(MergeComorphismDecl {
            name,
            target_class,
            body,
            pos,
        })
    }

    fn parse_merge_comorphism_inline_body(&mut self) -> Result<MergeComorphismBody, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                params.push(self.expect_ident()?);
                if !self.at(&TokenKind::Comma) {
                    break;
                }
                self.advance();
            }
        }
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::FatArrow)?;
        let body = self.parse_expr()?;
        Ok(MergeComorphismBody::Inline { params, body, pos })
    }

    fn parse_merge_comorphism_reference_body(&mut self) -> Result<MergeComorphismBody, EslError> {
        let pos = self.current_pos();
        // The `transformation` keyword is contextual — match the
        // identifier rather than tokenising it globally. Anywhere
        // else in the grammar it stays a free identifier.
        let kw = self.expect_ident()?;
        if kw != "transformation" {
            return Err(EslError::parser(
                Some(pos.clone()),
                format!(
                    "expected 'transformation' keyword in merge_comorphism reference body, \
                     found {kw:?}"
                ),
            ));
        }
        self.expect(&TokenKind::Eq)?;
        let transformation = self.parse_qualified_name()?;
        // Optional trailing semicolon — matches the ESL style for
        // single-line property assignments.
        if self.at(&TokenKind::Semicolon) {
            self.advance();
        }
        Ok(MergeComorphismBody::Reference {
            transformation,
            pos,
        })
    }

    // --- D43 §3.1 — text_index / vector_index declarations ---

    /// `text_index <iri> { target_property = ...; text_analyzer = "..."; }`.
    /// Sugar over a `core:TextIndex` Resource declaration; the class
    /// is implicit in the keyword. The compiler is responsible for
    /// the lowering to a regular Resource.
    fn parse_text_index(&mut self) -> Result<TextIndexDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::TextIndex)?;
        let name = self.parse_qualified_name()?;
        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            body.push(self.parse_resource_field()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(TextIndexDecl { name, body, pos })
    }

    /// `vector_index <iri> { target_property = ...; vec_model = ...; vec_dim = ...; ... }`.
    /// Sugar over a `core:VectorIndex` Resource declaration; the
    /// class is implicit in the keyword. The compiler is responsible
    /// for the lowering to a regular Resource.
    fn parse_vector_index(&mut self) -> Result<VectorIndexDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::VectorIndex)?;
        let name = self.parse_qualified_name()?;
        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            body.push(self.parse_resource_field()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(VectorIndexDecl { name, body, pos })
    }

    // --- Data (Phase 11b step 7, D19 §10) ---

    /// `data Name { ctor, ctor(arg, ...), ... }` — non-parametric.
    /// `data Name(p1 : K1, p2 : K2, ...) { ... }` — parametric.
    ///
    /// v1 surface form (Haskell-style): constructors are named, with
    /// optional positional argument types in parentheses. Constructor
    /// argument types are parameterised name references; the implicit
    /// result type is the inductive itself applied to its parameters.
    /// eigenius#72 — `axiom Name : <type-expr> [ note "<justification>" ; ]`.
    /// Top-level declaration that commits a `core:Axiom` chain resource
    /// whose `axiom_statement` is the encoded type expression.
    fn parse_axiom(&mut self) -> Result<AxiomDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Axiom)?;
        let name = self.parse_qualified_name()?;
        self.expect(&TokenKind::Colon)?;
        let statement = self.parse_type_expr()?;
        // Optional `desc: "..."` and/or `note: "..."` annotation clauses, in either order,
        // before a closing semicolon. `desc:` → `core:description` (the gloss); `note:` →
        // `core:axiom_justification` (the warrant). Accepted as plain idents to avoid reserving
        // keywords. A repeated clause overwrites the earlier value (last wins).
        let mut description = None;
        let mut justification = None;
        while let TokenKind::Ident(name) = self.peek().clone() {
            match name.as_str() {
                "desc" => {
                    self.advance();
                    self.expect(&TokenKind::Colon)?;
                    description = Some(self.expect_string()?);
                }
                "note" => {
                    self.advance();
                    self.expect(&TokenKind::Colon)?;
                    justification = Some(self.expect_string()?);
                }
                _ => break,
            }
        }
        // Optional trailing semicolon.
        if self.at(&TokenKind::Semicolon) {
            self.advance();
        }
        Ok(AxiomDecl {
            name,
            statement,
            description,
            justification,
            pos,
        })
    }

    /// `def ex:F(m : Set, g : Set) : Prop = <type-expr>` (D66).
    ///
    /// Parameters reuse `parse_typed_param_list` — the production `forall` already uses — so a
    /// parameter's type may be any type expression, not just a class or sort as `data` allows.
    ///
    /// `opaque` is deliberately NOT accepted here. The property exists on the resource for #95, but
    /// an opaque definition is indistinguishable from an axiom until its body is type-checked at
    /// commit — until then it would be a body the kernel ignores, which is exactly the
    /// documentation-claiming-more-than-the-code pattern. Authors get `axiom` for that today.
    fn parse_def(&mut self) -> Result<DefDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Def)?;
        let name = self.parse_qualified_name()?;
        let params = if self.at(&TokenKind::LParen) {
            self.parse_typed_param_list()?
        } else {
            Vec::new()
        };
        self.expect(&TokenKind::Colon)?;
        let result = self.parse_type_expr()?;
        self.expect(&TokenKind::Eq)?;
        let body = self.parse_type_expr()?;
        let mut description = None;
        while let TokenKind::Ident(ident) = self.peek().clone() {
            if ident.as_str() != "desc" {
                break;
            }
            self.advance();
            self.expect(&TokenKind::Colon)?;
            description = Some(self.expect_string()?);
        }
        if self.at(&TokenKind::Semicolon) {
            self.advance();
        }
        Ok(DefDecl {
            name,
            params,
            result,
            body,
            description,
            pos,
        })
    }

    /// D52 §12 — `macro ns:Name(p1 : T1, p2 : T2, ...) : RetT => body;`
    /// Top-level smart-constructor declaration. Parses a parameter
    /// list, a return-type annotation, and a body `Value` expression
    /// after `=>`. The body is compile-time-substituted at each call
    /// site; the macro itself does not lower to a chain resource.
    fn parse_macro(&mut self) -> Result<MacroDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Macro)?;
        let name = self.parse_qualified_name()?;
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            params.push(self.parse_macro_param()?);
            while self.at(&TokenKind::Comma) {
                self.advance();
                if self.at(&TokenKind::RParen) {
                    break; // trailing comma
                }
                params.push(self.parse_macro_param()?);
            }
        }
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::Colon)?;
        let return_type = self.parse_type_expr()?;
        self.expect(&TokenKind::FatArrow)?;
        let body = self.parse_value()?;
        if self.at(&TokenKind::Semicolon) {
            self.advance();
        }
        Ok(MacroDecl {
            name,
            params,
            return_type,
            body,
            pos,
        })
    }

    /// Parse a single `name : Type` macro parameter.
    fn parse_macro_param(&mut self) -> Result<MacroParam, EslError> {
        let pos = self.current_pos();
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Colon)?;
        let typ = self.parse_type_expr()?;
        Ok(MacroParam { name, typ, pos })
    }

    fn parse_data(&mut self) -> Result<DataDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Data)?;
        let name = self.parse_qualified_name()?;

        let params = if self.at(&TokenKind::LParen) {
            self.parse_data_params()?
        } else {
            Vec::new()
        };

        // Optional `: <index-telescope> -> <sort>` clause (eigenius#72
        // Layer 2). Example: `data Vec(A : Set) : Nat -> Set { ... }`
        // parses one index `_ : Nat` and the result sort `Set`. The
        // telescope is `T1 -> T2 -> ... -> SortLiteral`; trailing piece
        // must be a sort literal (Prop / Set / Type N).
        //
        // The legacy form `data Name { ... }` and the parametric form
        // `data Name(params) { ... }` both omit this clause; `indices`
        // stays empty and `result_sort` stays `None` (compiler defaults
        // to `Set`).
        let (indices, result_sort) = if self.at(&TokenKind::Colon) {
            self.advance();
            self.parse_data_index_telescope()?
        } else {
            (Vec::new(), None)
        };

        // D52 §12 #8 — extra `is_a` classes for the emitted
        // inductive-type resource, written as a comma-separated list
        // after the result sort: `data X : Prop, stats:PopulationLevel,
        // OtherMarker { ... }`. Parallels the class-multi-parent
        // syntax `class X : P1, P2`. The structural terminator for
        // the type expression is `{` (data body opens) or `,` (next
        // extra class), so there's no ambiguity even when the result
        // type is a pi/forall whose binder list also uses commas —
        // those commas are inside a `=>`-terminated binder context.
        let mut extra_classes = Vec::new();
        while self.at(&TokenKind::Comma) {
            self.advance();
            extra_classes.push(self.parse_qualified_name()?);
        }

        self.expect(&TokenKind::LBrace)?;
        let mut ctors = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            ctors.push(self.parse_ctor_decl()?);
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else if !self.at(&TokenKind::RBrace) {
                return Err(EslError::parser(
                    Some(self.current_pos()),
                    format!(
                        "expected ',' or '}}' after constructor, found {:?}",
                        self.peek()
                    ),
                ));
            }
        }
        self.expect(&TokenKind::RBrace)?;

        // Zero-ctor inductives are valid in the underlying type theory:
        // in `Set`/`Type N` they're absurd (Void-shaped — inhabitable
        // only by absurd elimination); in `Prop` they're the canonical
        // shape for opaque predicates whose inhabitants are admitted by
        // the kernel rather than constructed by users (D49 §6's
        // `ChainWitness.IsDeclaredAs / IsObservedAs / IsDerivedAs /
        // IsVerifiedAs` and D39 §4.1's `core:Asserts` are all of this
        // shape). The parser does not police this — well-formedness is
        // a type-theory concern handled by the kernel checker, and
        // legitimate use cases would otherwise force users to either
        // declare a junk ctor or author the type in JSON to bypass ESL.

        Ok(DataDecl {
            name,
            params,
            indices,
            result_sort,
            extra_classes,
            ctors,
            pos,
        })
    }

    /// `(name : Kind, name : Kind, ...)` — parameter list for parametric data.
    fn parse_data_params(&mut self) -> Result<Vec<DataParam>, EslError> {
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        while !self.at(&TokenKind::RParen) && !self.at_eof() {
            let pos = self.current_pos();
            let name = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            // The kind is a qualified-name class OR a sort literal (`Prop`/`Set`/
            // `Type N`) — the latter for Lean-style sort-parametrized inductives
            // (`And (P : Prop, ...)`). Parse a type-expr (which recognizes sort
            // literals) and classify, mirroring the index-telescope
            // (`parse_data_index_telescope`). A param kind is terminated by `,`
            // or `)`, so no trailing arrow is consumed here.
            let kind = match self.parse_type_expr()? {
                TypeExpr::Ref { name, args, .. } if args.is_empty() => IndexKind::Named(name),
                TypeExpr::Sort { kind, .. } => IndexKind::Sort(kind),
                other => {
                    return Err(EslError::parser(
                        Some(other.pos().clone()),
                        "data parameter kind must be a qualified-name class or a sort \
                         literal (Prop / Set / Type N)"
                            .to_string(),
                    ));
                }
            };
            params.push(DataParam { name, kind, pos });
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else if !self.at(&TokenKind::RParen) {
                return Err(EslError::parser(
                    Some(self.current_pos()),
                    format!(
                        "expected ',' or ')' after data parameter, found {:?}",
                        self.peek()
                    ),
                ));
            }
        }
        self.expect(&TokenKind::RParen)?;
        if params.is_empty() {
            return Err(EslError::parser(
                Some(self.current_pos()),
                "empty data parameter list — write `data Name { ... }` for non-parametric \
                 inductives instead of `data Name() { ... }`"
                    .to_string(),
            ));
        }
        Ok(params)
    }

    /// Parse the `<index-telescope> -> <sort-literal>` clause that
    /// appears after the colon in `data Name(params) : ... { ... }`
    /// (eigenius#72 Layer 2). The colon has already been consumed by
    /// the caller.
    ///
    /// Grammar: one or more anonymous index types separated by `->`,
    /// terminated by a sort literal as the result sort. Each
    /// intermediate index type can be one of:
    ///
    /// - A bare qualified-name reference (parameter name, or a class
    ///   IRI) — the original v1 shape; carries through as
    ///   [`IndexKind::Named`].
    /// - A sort literal (`Prop` / `Set` / `Type N`) — needed for
    ///   indexed inductives that range over types as values, e.g.
    ///   D39 §5's `JustifiedBy : JustificationTerm → Prop → Type`
    ///   and `ChainWitness.IsDeclaredAs : core:string → Prop → Prop`;
    ///   carries through as [`IndexKind::Sort`].
    ///
    /// Arbitrary applied or function-typed index kinds are still
    /// deferred. A bare sort literal (no preceding `->`) is also
    /// accepted and produces an empty index telescope with the sort
    /// as `result_sort` — `data Foo : Set { ... }` is equivalent to
    /// `data Foo { ... }` but with the sort made explicit.
    fn parse_data_index_telescope(
        &mut self,
    ) -> Result<(Vec<DataIndex>, Option<SortKind>), EslError> {
        let mut indices = Vec::new();
        let te = self.parse_type_expr()?;
        let mut current = te;
        loop {
            match current {
                TypeExpr::Arrow {
                    domain, codomain, ..
                } => {
                    let (kind, dom_pos) = match *domain {
                        TypeExpr::Ref { name, args, pos } if args.is_empty() => {
                            (IndexKind::Named(name), pos)
                        }
                        TypeExpr::Sort { kind, pos } => (IndexKind::Sort(kind), pos),
                        other => {
                            return Err(EslError::parser(
                                Some(other.pos().clone()),
                                "data-index telescope: each index type must be a bare \
                                 qualified-name reference or a sort literal (Prop / Set / \
                                 Type N); arbitrary applied or function-typed index kinds \
                                 are not yet supported"
                                    .to_string(),
                            ));
                        }
                    };
                    indices.push(DataIndex {
                        name: "_".to_string(),
                        kind,
                        pos: dom_pos,
                    });
                    current = *codomain;
                }
                TypeExpr::Sort { kind, .. } => {
                    return Ok((indices, Some(kind)));
                }
                other => {
                    return Err(EslError::parser(
                        Some(other.pos().clone()),
                        "data-index telescope must end in a sort literal (Prop / Set / Type N); \
                         each preceding segment is an anonymous index type separated by `->`"
                            .to_string(),
                    ));
                }
            }
        }
    }

    /// Three surface forms (eigenius#72 Layer 2):
    ///
    /// - `name` — nullary positional.
    /// - `name(arg, arg, ...)` — positional with arg list.
    /// - `name : <type-expr>` — typed form carrying the full Π-telescope
    ///   (required for indexed inductives; optional but allowed for
    ///   non-indexed too).
    fn parse_ctor_decl(&mut self) -> Result<CtorDecl, EslError> {
        let pos = self.current_pos();
        let name = self.expect_ident()?;
        if self.at(&TokenKind::Colon) {
            self.advance();
            let typ = self.parse_type_expr()?;
            return Ok(CtorDecl::Typed { name, typ, pos });
        }
        let args = if self.at(&TokenKind::LParen) {
            self.advance();
            let mut args = Vec::new();
            while !self.at(&TokenKind::RParen) && !self.at_eof() {
                args.push(self.parse_ctor_arg()?);
                if self.at(&TokenKind::Comma) {
                    self.advance();
                } else if !self.at(&TokenKind::RParen) {
                    return Err(EslError::parser(
                        Some(self.current_pos()),
                        format!(
                            "expected ',' or ')' in constructor arg list, found {:?}",
                            self.peek()
                        ),
                    ));
                }
            }
            self.expect(&TokenKind::RParen)?;
            if args.is_empty() {
                return Err(EslError::parser(
                    Some(pos.clone()),
                    format!(
                        "empty constructor arg list — write `{name}` for nullary instead of `{name}()`"
                    ),
                ));
            }
            args
        } else {
            Vec::new()
        };
        Ok(CtorDecl::Positional { name, args, pos })
    }

    /// A single constructor argument.
    ///
    /// Two surface forms (Phase 11b step 15h):
    ///
    /// - Positional (legacy): `Name` or `Name(params, ...)`.
    /// - Brace-delimited named binder:
    ///   - `{j < i}` — sized binder, kind `Size` implicit; compiles
    ///     to `Exp::SizedPi` with upper bound `i`.
    ///   - `{j : Kind}` — named binder with explicit kind (no bound);
    ///     compiles to a plain Π binder.
    ///   - `{j : Kind < i}` — named binder with both explicit kind
    ///     and an upper bound. When `Kind` is `Size` this becomes
    ///     a `SizedPi`; otherwise bounded-binding on non-size kinds
    ///     is rejected at decode time.
    ///
    /// Braces disambiguate binders from positional qualified names
    /// (`ex:Nat`) — without them the two shapes are token-identical.
    fn parse_ctor_arg(&mut self) -> Result<CtorArg, EslError> {
        if self.at(&TokenKind::LBrace) {
            let pos = self.current_pos();
            self.advance();
            let name = self.expect_ident()?;
            let (kind, bound) = if self.at(&TokenKind::Colon) {
                self.advance();
                let kind = self.parse_qualified_name()?;
                let bound = if self.at(&TokenKind::Less) {
                    self.advance();
                    Some(self.parse_qualified_name()?)
                } else {
                    None
                };
                (kind, bound)
            } else if self.at(&TokenKind::Less) {
                // `{name < bound}` — implicit Size kind.
                self.advance();
                let bound = self.parse_qualified_name()?;
                (
                    QualifiedName {
                        namespace: None,
                        name: "Size".to_string(),
                        pos: pos.clone(),
                    },
                    Some(bound),
                )
            } else {
                return Err(EslError::parser(
                    Some(self.current_pos()),
                    format!(
                        "expected ':' or '<' after binder name in ctor arg, found {:?}",
                        self.peek()
                    ),
                ));
            };
            self.expect(&TokenKind::RBrace)?;
            Ok(CtorArg::Named {
                name,
                kind,
                bound,
                pos,
            })
        } else {
            Ok(CtorArg::Positional(self.parse_ctor_arg_type()?))
        }
    }

    /// A constructor argument type: `Name` or `Name(arg, ...)`.
    fn parse_ctor_arg_type(&mut self) -> Result<CtorArgType, EslError> {
        let pos = self.current_pos();
        let name = self.parse_qualified_name()?;
        let params = if self.at(&TokenKind::LParen) {
            self.advance();
            let mut params = Vec::new();
            while !self.at(&TokenKind::RParen) && !self.at_eof() {
                params.push(self.parse_ctor_arg_type()?);
                if self.at(&TokenKind::Comma) {
                    self.advance();
                } else if !self.at(&TokenKind::RParen) {
                    return Err(EslError::parser(
                        Some(self.current_pos()),
                        format!(
                            "expected ',' or ')' in type argument list, found {:?}",
                            self.peek()
                        ),
                    ));
                }
            }
            self.expect(&TokenKind::RParen)?;
            params
        } else {
            Vec::new()
        };
        Ok(CtorArgType { name, params, pos })
    }

    // --- Expressions ---

    /// Parse an expression. This is the top-level expression parser.
    /// Precedence (loosest to tightest):
    ///   let, case, corecord (extend rightward)
    ///   lambda
    ///   application f(arg)
    ///   projection e.prop
    ///   atoms (var, literal, construct, parens)
    fn parse_expr(&mut self) -> Result<Expr, EslError> {
        match self.peek() {
            TokenKind::Let => self.parse_let(),
            TokenKind::Case => self.parse_case(),
            TokenKind::Match => self.parse_match(),
            TokenKind::Corecord => self.parse_corecord(),
            TokenKind::Backslash | TokenKind::Lambda => self.parse_lambda(),
            // D37 §3.1 — typed lambda literal `lambda x : T => body`.
            // Distinct from the untyped `\x -> e` / `λx -> e` forms
            // above; the typed form's params carry annotations and
            // the body separator is `=>` instead of `->`.
            TokenKind::LambdaKw => self.parse_lambda_literal(),
            _ => self.parse_apply_or_atom(),
        }
    }

    /// `let x : T = e; body`
    fn parse_let(&mut self) -> Result<Expr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Let)?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Colon)?;
        let typ = self.parse_qualified_name()?;
        self.expect(&TokenKind::Eq)?;
        let value = self.parse_expr()?;
        self.expect_semicolon()?;
        let body = self.parse_expr()?;

        Ok(Expr::Let {
            name,
            typ,
            value: Box::new(value),
            body: Box::new(body),
            pos,
        })
    }

    /// `\x -> e` or `λx -> e`
    fn parse_lambda(&mut self) -> Result<Expr, EslError> {
        let pos = self.current_pos();
        // Accept either \ or λ
        match self.peek() {
            TokenKind::Backslash | TokenKind::Lambda => {
                self.advance();
            }
            _ => {
                return Err(EslError::parser(
                    Some(self.current_pos()),
                    "expected '\\' or 'λ'".to_string(),
                ))
            }
        }
        let param = self.expect_ident()?;
        self.expect(&TokenKind::Arrow)?;
        let body = self.parse_expr()?;

        Ok(Expr::Lambda {
            param,
            param_type: None,
            body: Box::new(body),
            pos,
        })
    }

    /// D37 §3.1 — typed lambda literal:
    ///   `lambda x_1 : T_1, …, x_N : T_N => body`
    ///
    /// Multi-parameter form folds into N nested single-parameter
    /// `Expr::Lambda` nodes. Each binder carries `param_type =
    /// Some(T_i)` so the validator can commit-time-check the body
    /// against its declared signature.
    ///
    /// The optional `=> body : ReturnType` suffix from D37 §3.4 is
    /// not yet accepted by the parser — the return type is inferred
    /// from the body at validation time. Adding the suffix is a
    /// pure grammar extension that doesn't change AST shape.
    fn parse_lambda_literal(&mut self) -> Result<Expr, EslError> {
        let outer_pos = self.current_pos();
        self.expect(&TokenKind::LambdaKw)?;
        let params = self.parse_typed_param_list()?;
        self.expect(&TokenKind::FatArrow)?;
        let body = self.parse_expr()?;

        // Fold N parameters into nested Lambda nodes. The rightmost
        // parameter wraps the body directly; the leftmost is the
        // outermost lambda.
        let mut expr = body;
        for param in params.into_iter().rev() {
            let pos = param.pos.clone();
            expr = Expr::Lambda {
                param: param.name,
                param_type: Some(param.typ),
                body: Box::new(expr),
                pos,
            };
        }
        // If we collapsed everything, the outermost lambda already
        // has its own pos. The `outer_pos` captures the `lambda`
        // keyword position itself — useful for diagnostics on an
        // empty parameter list (which `parse_typed_param_list`
        // rejects via `expect_ident` already, so this branch is
        // defensive).
        let _ = outer_pos;
        Ok(expr)
    }

    /// `corecord { head = e1; tail = e2 }`
    fn parse_corecord(&mut self) -> Result<Expr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Corecord)?;
        self.expect(&TokenKind::LBrace)?;

        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let name = self.expect_ident()?;
            self.expect(&TokenKind::Eq)?;
            let body = self.parse_expr()?;
            self.expect_semicolon()?;
            fields.push(CoField { name, body });
        }
        self.expect(&TokenKind::RBrace)?;

        if fields.is_empty() {
            return Err(EslError::parser(
                Some(pos.clone()),
                "corecord must have at least one field".to_string(),
            ));
        }

        Ok(Expr::CoRecord { fields, pos })
    }

    /// `case e { A -> e1; B -> e2 }`
    fn parse_case(&mut self) -> Result<Expr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Case)?;
        // No-trailing-block: the following `{` is the case body, not
        // a component config block on the scrutinee.
        let scrutinee = self.parse_apply_or_atom_no_trailing_block()?;
        self.expect(&TokenKind::LBrace)?;

        let mut branches = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let constructor = self.expect_ident()?;
            self.expect(&TokenKind::Arrow)?;
            let body = self.parse_expr()?;
            self.expect_semicolon()?;
            branches.push((constructor, body));
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(Expr::Case {
            scrutinee: Box::new(scrutinee),
            branches,
            pos,
        })
    }

    /// `match e returning T { ctor -> body; ctor(x, y) -> body; ... }`
    /// (Phase 11b step 11, D19 §10).
    ///
    /// `returning T` annotates the result type of every arm body —
    /// required for now (motive inference is a future extension).
    /// We use a dedicated keyword rather than `:` because the
    /// scrutinee is an arbitrary expression that may itself end in
    /// a qualified-name reference; `expr : T` is grammatically
    /// ambiguous when `expr` parses greedily for namespaced names.
    fn parse_match(&mut self) -> Result<Expr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Match)?;
        // No-trailing-block: a following `{` opens the match body, not
        // a component config block on the scrutinee.
        let scrutinee = self.parse_apply_or_atom_no_trailing_block()?;
        // `returning <motive>` is optional. When omitted, the
        // kernel-side type checker synthesises the motive from the
        // checking-mode expected type (Phase 11b step 12). When present
        // the expression builder desugars eagerly to
        // `Exp::InductiveRec` with the supplied motive (eigenius#72
        // Layer 3 — accepts the full type-expression surface, including
        // `fun (i : T) => body` motives for indexed inductives).
        let returning = if self.at(&TokenKind::Returning) {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.expect(&TokenKind::LBrace)?;

        let mut arms = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            arms.push(self.parse_match_arm()?);
        }
        self.expect(&TokenKind::RBrace)?;

        if arms.is_empty() {
            return Err(EslError::parser(
                Some(pos),
                "match expression must have at least one arm".to_string(),
            ));
        }

        Ok(Expr::Match {
            scrutinee: Box::new(scrutinee),
            returning,
            arms,
            pos,
        })
    }

    /// One arm of a match: `ctor -> body;` or `ctor(x, y, ...) -> body;`.
    fn parse_match_arm(&mut self) -> Result<MatchArm, EslError> {
        let pos = self.current_pos();
        let ctor_name = self.expect_ident()?;
        let bindings = if self.at(&TokenKind::LParen) {
            self.advance();
            let mut bindings = Vec::new();
            while !self.at(&TokenKind::RParen) && !self.at_eof() {
                bindings.push(self.expect_ident()?);
                if self.at(&TokenKind::Comma) {
                    self.advance();
                } else if !self.at(&TokenKind::RParen) {
                    return Err(EslError::parser(
                        Some(self.current_pos()),
                        format!(
                            "expected ',' or ')' in match arm bindings, found {:?}",
                            self.peek()
                        ),
                    ));
                }
            }
            self.expect(&TokenKind::RParen)?;
            if bindings.is_empty() {
                return Err(EslError::parser(
                    Some(pos.clone()),
                    format!(
                        "empty binding list — write `{ctor_name}` for nullary instead of `{ctor_name}()`"
                    ),
                ));
            }
            bindings
        } else {
            Vec::new()
        };
        self.expect(&TokenKind::Arrow)?;
        let body = self.parse_expr()?;
        self.expect_semicolon()?;
        Ok(MatchArm {
            ctor_name,
            bindings,
            body,
            pos,
        })
    }

    /// Parse function application or an atom, allowing the trailing
    /// `{ … }` config-block sugar for component dispatch (`f(arg) { … }`).
    fn parse_apply_or_atom(&mut self) -> Result<Expr, EslError> {
        self.parse_apply_or_atom_inner(true)
    }

    /// Like `parse_apply_or_atom` but with trailing config-block
    /// parsing disabled. Use this in positions where a following `{`
    /// belongs to the surrounding grammar — e.g. the scrutinee of
    /// `case` or `match`, where `{` opens the body, not a component
    /// configuration block.
    fn parse_apply_or_atom_no_trailing_block(&mut self) -> Result<Expr, EslError> {
        self.parse_apply_or_atom_inner(false)
    }

    fn parse_apply_or_atom_inner(&mut self, allow_trailing_block: bool) -> Result<Expr, EslError> {
        let mut expr = self.parse_atom()?;

        // Handle projection chains: e.prop.prop2
        while self.at(&TokenKind::Dot) {
            self.advance();
            let property = self.parse_qualified_name()?;
            let pos = property.pos.clone();
            expr = Expr::Project {
                expr: Box::new(expr),
                property,
                pos,
            };
        }

        // Handle function application: expr(args)
        if self.at(&TokenKind::LParen) {
            let pos = self.current_pos();
            self.advance(); // (

            // Collect arguments
            let mut args = Vec::new();
            if !self.at(&TokenKind::RParen) {
                args.push(self.parse_expr()?);
                while self.at(&TokenKind::Comma) {
                    self.advance();
                    args.push(self.parse_expr()?);
                }
            }
            self.expect(&TokenKind::RParen)?;

            // Determine what kind of application this is
            expr = match &expr {
                Expr::Var { name, .. } if name == "map" && args.len() == 2 => Expr::MapExpr {
                    function: Box::new(args.remove(0)),
                    collection: Box::new(args.remove(0)),
                    pos,
                },
                Expr::Var { name, .. } if name == "reduce" && args.len() == 3 => Expr::ReduceExpr {
                    function: Box::new(args.remove(0)),
                    initial: Box::new(args.remove(0)),
                    collection: Box::new(args.remove(0)),
                    pos,
                },
                _ => {
                    // Regular function application
                    let function = match expr {
                        Expr::Var { name, pos } => QualifiedName {
                            namespace: None,
                            name,
                            pos,
                        },
                        Expr::Project { property, .. } => property,
                        _ => {
                            return Err(EslError::parser(
                                Some(pos),
                                "function in application must be an identifier".to_string(),
                            ))
                        }
                    };

                    // Trailing block: f(args) { key = val; } supplies a
                    // component_argument (static config for IO components).
                    // Suppressed in contexts where a following `{`
                    // belongs to the surrounding grammar (e.g.
                    // `match expr { … }` — the brace opens the match
                    // body, not a config block).
                    let block_component_argument =
                        if allow_trailing_block && self.at(&TokenKind::LBrace) {
                            Some(Box::new(self.parse_block_expr()?))
                        } else {
                            None
                        };

                    Expr::Apply {
                        function,
                        args,
                        component_argument: block_component_argument,
                        pos,
                    }
                }
            };
        }

        Ok(expr)
    }

    /// Parse an atomic expression.
    fn parse_atom(&mut self) -> Result<Expr, EslError> {
        match self.peek().clone() {
            // Construct T { ... }
            TokenKind::Construct => self.parse_construct(),

            // Parenthesized expression or pair
            TokenKind::LParen => {
                let pos = self.current_pos();
                self.advance(); // (
                let first = self.parse_expr()?;
                if self.at(&TokenKind::Comma) {
                    self.advance();
                    let second = self.parse_expr()?;
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expr::Pair {
                        first: Box::new(first),
                        second: Box::new(second),
                        pos,
                    })
                } else {
                    self.expect(&TokenKind::RParen)?;
                    Ok(first)
                }
            }

            // String literal
            TokenKind::StringLit(s) => {
                let pos = self.current_pos();
                self.advance();
                Ok(Expr::Literal {
                    value: LiteralValue::String(s),
                    pos,
                })
            }

            // Integer literal
            TokenKind::IntLit(n) => {
                let pos = self.current_pos();
                self.advance();
                Ok(Expr::Literal {
                    value: LiteralValue::Int(n),
                    pos,
                })
            }

            // Float literal
            TokenKind::FloatLit(f) => {
                let pos = self.current_pos();
                self.advance();
                Ok(Expr::Literal {
                    value: LiteralValue::Float(f),
                    pos,
                })
            }

            // Boolean literal
            TokenKind::BoolLit(b) => {
                let pos = self.current_pos();
                self.advance();
                Ok(Expr::Literal {
                    value: LiteralValue::Bool(b),
                    pos,
                })
            }

            // Lambda (can appear as atom in nested position)
            TokenKind::Backslash | TokenKind::Lambda => self.parse_lambda(),
            // D37 §3.1 — typed lambda literal in atom position.
            TokenKind::LambdaKw => self.parse_lambda_literal(),

            // Keywords that are also valid in expression position
            TokenKind::Map | TokenKind::Reduce => {
                let pos = self.current_pos();
                let name = self.expect_ident()?;
                Ok(Expr::Var { name, pos })
            }

            // Identifier (variable) or qualified name `ns:name` (one atomic token)
            TokenKind::Ident(_) | TokenKind::QualName(..) => {
                let pos = self.current_pos();
                let qn = self.parse_qualified_name()?;
                if let Some(ns) = qn.namespace {
                    Ok(Expr::Var {
                        name: format!("{ns}:{}", qn.name),
                        pos,
                    })
                } else {
                    Ok(Expr::Var { name: qn.name, pos })
                }
            }

            _ => Err(EslError::parser(
                Some(self.current_pos()),
                format!("expected expression, found {:?}", self.peek()),
            )),
        }
    }

    /// `Construct ex:Dog { name = e1, breed = e2 }`
    fn parse_construct(&mut self) -> Result<Expr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Construct)?;
        let class = self.parse_qualified_name()?;
        self.expect(&TokenKind::LBrace)?;

        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let prop = self.parse_qualified_name()?;
            self.expect(&TokenKind::Eq)?;
            let value = self.parse_expr()?;
            fields.push((prop, value));

            // Comma is optional between fields
            if self.at(&TokenKind::Comma) {
                self.advance();
            }
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(Expr::ConstructExpr { class, fields, pos })
    }

    /// Parse a block expression: `{ key = value; key2 = value2; }`
    /// Used for component argument blocks. Produces an embedded resource
    /// with qualified-name keys and values (which can be strings, numbers,
    /// booleans, or nested blocks).
    fn parse_block_expr(&mut self) -> Result<Expr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::LBrace)?;

        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let prop = self.parse_qualified_name()?;
            self.expect(&TokenKind::Eq)?;
            let value = self.parse_block_value_expr()?;
            self.expect_semicolon()?;
            fields.push((prop, value));
        }
        self.expect(&TokenKind::RBrace)?;

        // Produce a ConstructExpr with no class (anonymous block)
        // The compiler will emit it as an embedded resource
        Ok(Expr::ConstructExpr {
            class: QualifiedName {
                namespace: None,
                name: String::new(),
                pos: pos.clone(),
            },
            fields,
            pos,
        })
    }

    /// Parse a value inside a block expression.
    /// Accepts literals, qualified names, and nested blocks.
    fn parse_block_value_expr(&mut self) -> Result<Expr, EslError> {
        match self.peek().clone() {
            TokenKind::StringLit(s) => {
                let pos = self.current_pos();
                self.advance();
                Ok(Expr::Literal {
                    value: LiteralValue::String(s),
                    pos,
                })
            }
            TokenKind::IntLit(n) => {
                let pos = self.current_pos();
                self.advance();
                Ok(Expr::Literal {
                    value: LiteralValue::Int(n),
                    pos,
                })
            }
            TokenKind::FloatLit(f) => {
                let pos = self.current_pos();
                self.advance();
                Ok(Expr::Literal {
                    value: LiteralValue::Float(f),
                    pos,
                })
            }
            TokenKind::BoolLit(b) => {
                let pos = self.current_pos();
                self.advance();
                Ok(Expr::Literal {
                    value: LiteralValue::Bool(b),
                    pos,
                })
            }
            TokenKind::LBrace => self.parse_block_expr(),
            _ => {
                // Qualified name as string reference
                let pos = self.current_pos();
                let qn = self.parse_qualified_name()?;
                if let Some(ns) = qn.namespace {
                    Ok(Expr::Var {
                        name: format!("{ns}:{}", qn.name),
                        pos,
                    })
                } else {
                    Ok(Expr::Var { name: qn.name, pos })
                }
            }
        }
    }
}

// ─── formula(...) tree builders (Phase 19f.3) ──────────────────────────
//
// Free helpers that produce `Value::CtorApp` nodes for the
// chain-typed FormulaTerm shape. Free rather than methods on Parser
// because they're pure — they don't touch parser state — and the
// formula sub-parser uses them at multiple call sites.

fn formula_lit_float(value: f64, pos: Position) -> Value {
    Value::CtorApp {
        ctor: "LitFloat".to_string(),
        args: vec![Value::Float(value)],
        pos,
    }
}

fn formula_op_ref(iri: &str, pos: Position) -> Value {
    Value::CtorApp {
        ctor: "OpRef".to_string(),
        args: vec![Value::String(iri.to_string())],
        pos,
    }
}

/// Build a binary `App(App(OpRef(op), lhs), rhs)` for an infix
/// operator at the given position.
fn formula_binary_app(op_iri: &str, lhs: Value, rhs: Value, pos: Position) -> Value {
    Value::CtorApp {
        ctor: "App".to_string(),
        args: vec![
            Value::CtorApp {
                ctor: "App".to_string(),
                args: vec![formula_op_ref(op_iri, pos.clone()), lhs],
                pos: pos.clone(),
            },
            rhs,
        ],
        pos,
    }
}

/// Build a function-call expression `f(arg1, arg2, …)` as a
/// left-spined App tree over `OpRef(op_iri)`. Mirrors what
/// `EigeniusCatalyst.num_to_formula` and `EigeniusSymbolics.num_to_formula`
/// emit when SymbolicUtils gives them n-ary calls (Phase 19f.1 fix):
/// each App node is binary, so a 3-arg function call becomes
/// `App(App(App(OpRef, a), b), c)` is wrong (3-arg App-spine the
/// chain validator rejects); the right shape is left-folding into
/// nested binary apps. Unary calls are a single binary App; nullary
/// is a special case (just the OpRef alone — no current operator
/// in the catalog needs this shape, but the helper handles it).
fn formula_function_call(op_iri: &str, args: Vec<Value>, pos: Position) -> Value {
    if args.is_empty() {
        return formula_op_ref(op_iri, pos);
    }
    let mut iter = args.into_iter();
    let first = iter.next().expect("non-empty");
    if iter.len() == 0 {
        // Unary call: `App(OpRef(op), arg1)`.
        return Value::CtorApp {
            ctor: "App".to_string(),
            args: vec![formula_op_ref(op_iri, pos.clone()), first],
            pos,
        };
    }
    // Binary or n-ary: left-fold so each App carries the operator's
    // declared arity at every step. `f(a, b, c)` →
    // `f(f(a, b), c)` — a fresh OpRef at each level.
    let mut iter = std::iter::once(first).chain(iter);
    let a = iter.next().expect("len ≥ 1");
    let b = iter.next().expect("len ≥ 2");
    let mut acc = formula_binary_app(op_iri, a, b, pos.clone());
    for next in iter {
        acc = formula_binary_app(op_iri, acc, next, pos.clone());
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esl::lexer::tokenize;

    fn parse_str(input: &str) -> Result<File, EslError> {
        let tokens = tokenize(input).map_err(|e| EslError::parser(e.position, e.message))?;
        parse(&tokens)
    }

    #[test]
    fn namespace_declaration() {
        let file = parse_str(r#"namespace core = "urn:eigenius:core";"#).unwrap();
        assert_eq!(file.namespaces.len(), 1);
        assert_eq!(file.namespaces[0].alias, "core");
        assert_eq!(file.namespaces[0].uri, "urn:eigenius:core");
    }

    #[test]
    fn class_declaration() {
        let file = parse_str(
            r#"
            class ex:Document {
                description = "A document";
                requires ex:text, ex:author;
                recommends ex:date;
            }
        "#,
        )
        .unwrap();
        assert_eq!(file.declarations.len(), 1);
        match &file.declarations[0] {
            Declaration::Class(c) => {
                assert_eq!(c.name.name, "Document");
                assert_eq!(c.name.namespace.as_deref(), Some("ex"));
                assert!(c.parents.is_empty());
                assert_eq!(c.body.len(), 3);
            }
            _ => panic!("expected class"),
        }
    }

    #[test]
    fn class_with_parent() {
        let file = parse_str(
            r#"
            class ex:Dog : ex:Animal {
                description = "A dog";
                requires ex:breed;
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Class(c) => {
                assert_eq!(c.parents.len(), 1);
                assert_eq!(c.parents[0].name, "Animal");
            }
            _ => panic!("expected class"),
        }
    }

    #[test]
    fn class_with_multiple_parents() {
        let file = parse_str(
            r#"
            class ex:HybridCell : ex:Cell, ex:Visualisable {
                description = "A hybrid cell.";
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Class(c) => {
                let names: Vec<&str> = c.parents.iter().map(|p| p.name.as_str()).collect();
                assert_eq!(names, vec!["Cell", "Visualisable"]);
            }
            _ => panic!("expected class"),
        }
    }

    #[test]
    fn property_declaration() {
        let file = parse_str(
            r#"
            property ex:count : core:integer {
                description = "Number of items";
                min_value = 0;
                max_value = 100;
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Property(p) => {
                assert_eq!(p.name.name, "count");
                assert_eq!(p.data_type.name, "integer");
                assert_eq!(p.body.len(), 3);
            }
            _ => panic!("expected property"),
        }
    }

    #[test]
    fn property_with_allows_only() {
        let file = parse_str(
            r#"
            property ex:status : core:resource {
                description = "Current status";
                allows_only ex:Active, ex:Inactive;
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Property(p) => {
                assert!(p
                    .body
                    .iter()
                    .any(|item| matches!(item, PropertyItem::AllowsOnly(v) if v.len() == 2)));
            }
            _ => panic!("expected property"),
        }
    }

    #[test]
    fn resource_declaration() {
        let file = parse_str(
            r#"
            resource ex:rex : ex:Dog {
                ex:name = "Rex";
                ex:breed = "German Shepherd";
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Resource(r) => {
                assert_eq!(r.name.name, "rex");
                assert_eq!(r.classes.len(), 1);
                assert_eq!(r.classes[0].name, "Dog");
                assert_eq!(r.body.len(), 2);
            }
            _ => panic!("expected resource"),
        }
    }

    #[test]
    fn resource_with_array() {
        let file = parse_str(
            r#"
            resource ex:test : ex:Thing {
                ex:tags = ["a", "b", "c"];
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Resource(r) => match &r.body[0].value {
                Value::Array(arr) => assert_eq!(arr.len(), 3),
                _ => panic!("expected array"),
            },
            _ => panic!("expected resource"),
        }
    }

    #[test]
    fn resource_with_nested_block() {
        let file = parse_str(
            r#"
            resource ex:test : ex:Thing {
                ex:config = {
                    ex:key = "value";
                };
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Resource(r) => {
                assert!(matches!(&r.body[0].value, Value::Block(_)));
            }
            _ => panic!("expected resource"),
        }
    }

    #[test]
    fn resource_with_inductive_ctor_app() {
        // D32 inductive-value literals: `Var("x")` is a 1-arg ctor;
        // `App(head, arg)` is a 2-arg ctor; literals nest.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = App(OpRef("urn:eigenius:formulas:ops:mul"), Var("x"));
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Resource(r) => {
                let body = &r.body[0];
                let Value::CtorApp { ctor, args, .. } = &body.value else {
                    panic!("expected CtorApp");
                };
                assert_eq!(ctor, "App");
                assert_eq!(args.len(), 2);
                let Value::CtorApp {
                    ctor: c1, args: a1, ..
                } = &args[0]
                else {
                    panic!("expected nested OpRef CtorApp");
                };
                assert_eq!(c1, "OpRef");
                assert_eq!(a1.len(), 1);
                assert!(matches!(&a1[0], Value::String(s) if s == "urn:eigenius:formulas:ops:mul"));
                let Value::CtorApp { ctor: c2, .. } = &args[1] else {
                    panic!("expected nested Var CtorApp");
                };
                assert_eq!(c2, "Var");
            }
            _ => panic!("expected resource"),
        }
    }

    #[test]
    fn nullary_ctor_requires_parens() {
        // `LE` (no parens) is a resource ref, NOT a ctor — the
        // disambiguation rule is "always require parens for ctors".
        let file = parse_str(
            r#"
            resource ex:c : ex:Constraint {
                ex:relation = LE();
                ex:other    = LE;
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Resource(r) => {
                // First field: `LE()` → CtorApp.
                let Value::CtorApp { ctor, args, .. } = &r.body[0].value else {
                    panic!("expected CtorApp");
                };
                assert_eq!(ctor, "LE");
                assert!(args.is_empty(), "nullary ctor should have empty args");
                // Second field: `LE` (no parens) → Ref.
                let Value::Ref(qn) = &r.body[1].value else {
                    panic!("expected Ref");
                };
                assert_eq!(qn.name, "LE");
                assert!(qn.namespace.is_none());
            }
            _ => panic!("expected resource"),
        }
    }

    #[test]
    fn qualified_call_parses_as_macro_call() {
        // D52 §12 — `ns:Name(args)` now parses as a macro call
        // (compile-time AST substitution), distinct from the
        // unqualified `Name(args)` shape which remains an inductive
        // constructor application. The compiler resolves the
        // qualified name against the per-file macro table and
        // rejects with a clean diagnostic if no macro is declared
        // under that IRI; at the parser layer we only confirm the
        // shape is recognised.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = formulas:App(Var("x"), Var("y"));
            }
        "#,
        )
        .expect("qualified name with `(` should parse as a macro call");
        match first_field_value(&file) {
            Value::MacroCall { name, args, .. } => {
                assert_eq!(name.namespace.as_deref(), Some("formulas"));
                assert_eq!(name.name, "App");
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected MacroCall, got {other:?}"),
        }
    }

    /// Helper: extract the parsed `Value::CtorApp` from the first
    /// resource-field's value position.
    fn first_field_value(file: &File) -> &Value {
        match &file.declarations[0] {
            Declaration::Resource(r) => &r.body[0].value,
            _ => panic!("expected resource"),
        }
    }

    #[test]
    fn formula_simple_binary() {
        // `x + 2` → `add(Var("x"), LitFloat(2.0))`.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = formula(x + 2);
            }
        "#,
        )
        .unwrap();
        let Value::CtorApp { ctor, args, .. } = first_field_value(&file) else {
            panic!("expected CtorApp");
        };
        assert_eq!(ctor, "App");
        assert_eq!(args.len(), 2);
        // args[0] should be App(OpRef("...:add"), Var("x")).
        let Value::CtorApp {
            ctor: c1, args: a1, ..
        } = &args[0]
        else {
            panic!("expected nested App on lhs");
        };
        assert_eq!(c1, "App");
        let Value::CtorApp {
            ctor: c_op,
            args: a_op,
            ..
        } = &a1[0]
        else {
            panic!("expected OpRef");
        };
        assert_eq!(c_op, "OpRef");
        assert!(matches!(&a_op[0], Value::String(s) if s == "urn:eigenius:formulas:ops:add"));
        let Value::CtorApp {
            ctor: c_var,
            args: a_var,
            ..
        } = &a1[1]
        else {
            panic!("expected Var");
        };
        assert_eq!(c_var, "Var");
        assert!(matches!(&a_var[0], Value::String(s) if s == "x"));
        // args[1] should be LitFloat(2.0).
        let Value::CtorApp {
            ctor: c_lit,
            args: a_lit,
            ..
        } = &args[1]
        else {
            panic!("expected LitFloat");
        };
        assert_eq!(c_lit, "LitFloat");
        assert!(matches!(&a_lit[0], Value::Float(f) if (*f - 2.0).abs() < f64::EPSILON));
    }

    #[test]
    fn formula_precedence_and_associativity() {
        // `1 + 2 * 3` should parse as `add(1, mul(2, 3))` — `*` binds
        // tighter than `+`.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = formula(1 + 2 * 3);
            }
        "#,
        )
        .unwrap();
        let Value::CtorApp { args, .. } = first_field_value(&file) else {
            panic!("expected CtorApp");
        };
        // args[0]'s App has op = add; args[1] should be the mul(2, 3).
        let Value::CtorApp { args: rhs_args, .. } = &args[1] else {
            panic!("expected nested mul");
        };
        let Value::CtorApp {
            args: rhs_inner, ..
        } = &rhs_args[0]
        else {
            panic!("expected App on rhs");
        };
        let Value::CtorApp { args: op_ref, .. } = &rhs_inner[0] else {
            panic!("expected OpRef");
        };
        assert!(matches!(&op_ref[0], Value::String(s) if s == "urn:eigenius:formulas:ops:mul"));
    }

    #[test]
    fn formula_pow_is_right_associative() {
        // `a ^ b ^ c` should parse as `pow(a, pow(b, c))` — Symbolics
        // / mathematical convention.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = formula(a ^ b ^ c);
            }
        "#,
        )
        .unwrap();
        let Value::CtorApp { args, .. } = first_field_value(&file) else {
            panic!("expected CtorApp");
        };
        // Outer App: pow(a, <inner pow>). args[1] is the inner pow.
        let Value::CtorApp {
            ctor: inner_ctor,
            args: inner_args,
            ..
        } = &args[1]
        else {
            panic!("expected inner App for pow");
        };
        assert_eq!(inner_ctor, "App");
        let Value::CtorApp { args: inner_op, .. } = &inner_args[0] else {
            panic!("expected App with OpRef");
        };
        let Value::CtorApp { args: op_args, .. } = &inner_op[0] else {
            panic!("expected OpRef");
        };
        assert!(matches!(&op_args[0], Value::String(s) if s == "urn:eigenius:formulas:ops:pow"));
    }

    #[test]
    fn formula_unary_minus_on_literal_folds() {
        // `-2` should fold the sign into the LitFloat rather than
        // wrap with neg.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = formula(-2);
            }
        "#,
        )
        .unwrap();
        let Value::CtorApp { ctor, args, .. } = first_field_value(&file) else {
            panic!("expected CtorApp");
        };
        assert_eq!(ctor, "LitFloat");
        assert!(matches!(&args[0], Value::Float(f) if (*f + 2.0).abs() < f64::EPSILON));
    }

    #[test]
    fn formula_unary_minus_on_var_wraps_in_neg() {
        // `-x` → `neg(Var("x"))`. Folding only applies to numeric atoms.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = formula(-x);
            }
        "#,
        )
        .unwrap();
        let Value::CtorApp { ctor, args, .. } = first_field_value(&file) else {
            panic!("expected CtorApp");
        };
        assert_eq!(ctor, "App");
        let Value::CtorApp { args: op_args, .. } = &args[0] else {
            panic!("expected OpRef");
        };
        assert!(matches!(&op_args[0], Value::String(s) if s == "urn:eigenius:formulas:ops:neg"));
        let Value::CtorApp {
            ctor: var_ctor,
            args: var_args,
            ..
        } = &args[1]
        else {
            panic!("expected Var");
        };
        assert_eq!(var_ctor, "Var");
        assert!(matches!(&var_args[0], Value::String(s) if s == "x"));
    }

    #[test]
    fn formula_function_call() {
        // `sin(x)` → `App(OpRef(...:sin), Var("x"))`.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = formula(sin(x));
            }
        "#,
        )
        .unwrap();
        let Value::CtorApp { args, .. } = first_field_value(&file) else {
            panic!("expected CtorApp");
        };
        let Value::CtorApp { args: op_args, .. } = &args[0] else {
            panic!("expected OpRef");
        };
        assert!(matches!(&op_args[0], Value::String(s) if s == "urn:eigenius:formulas:ops:sin"));
    }

    #[test]
    fn formula_parens_override_precedence() {
        // `(1 + 2) * 3` should parse as `mul(add(1, 2), 3)` — parens
        // beat * over +.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = formula((1 + 2) * 3);
            }
        "#,
        )
        .unwrap();
        let Value::CtorApp { args, .. } = first_field_value(&file) else {
            panic!("expected CtorApp");
        };
        // Outer App is mul; args[0] is the App(App(OpRef(mul),
        // App(...)), …); descend into its first leaf which is the
        // mul OpRef.
        let Value::CtorApp { args: outer_op, .. } = &args[0] else {
            panic!("expected outer App");
        };
        let Value::CtorApp { args: op_args, .. } = &outer_op[0] else {
            panic!("expected OpRef");
        };
        assert!(matches!(&op_args[0], Value::String(s) if s == "urn:eigenius:formulas:ops:mul"));
        // outer_op[1] should be the (1+2) sub-add.
        let Value::CtorApp { args: lhs_op, .. } = &outer_op[1] else {
            panic!("expected lhs sub-expression");
        };
        let Value::CtorApp {
            args: lhs_inner, ..
        } = &lhs_op[0]
        else {
            panic!("expected nested App for add");
        };
        let Value::CtorApp {
            args: lhs_op_args, ..
        } = &lhs_inner[0]
        else {
            panic!("expected OpRef");
        };
        assert!(
            matches!(&lhs_op_args[0], Value::String(s) if s == "urn:eigenius:formulas:ops:add")
        );
    }

    #[test]
    fn formula_kinase_ki_residual() {
        // The shape used in the kinase Ki-fit demo:
        //   pow(IC50_obs - c * Ki, 2)
        // This previously took ~15 lines of nested ctor literals
        // (cell 8 of kinase-institutions.json); the formula(...)
        // sublanguage compresses it to one readable line.
        let file = parse_str(
            r#"
            resource ex:t : ex:Holder {
                ex:term = formula((4 - 2 * Ki) ^ 2);
            }
        "#,
        )
        .unwrap();
        let Value::CtorApp { ctor, .. } = first_field_value(&file) else {
            panic!("expected CtorApp");
        };
        assert_eq!(ctor, "App");
    }

    #[test]
    fn simple_program() {
        let file = parse_str(
            r#"
            program ex:identity : ex:Document -> ex:Document {
                input
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => {
                assert_eq!(p.name.name, "identity");
                assert_eq!(p.input_type.name, "Document");
                assert_eq!(p.output_type.name, "Document");
                assert!(matches!(&p.body, Expr::Var { name, .. } if name == "input"));
            }
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn program_with_let() {
        let file = parse_str(
            r#"
            program ex:summarize : ex:Document -> ex:Document {
                let summary : core:string = CompleteText(input);
                Construct ex:Document { text = summary }
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => {
                assert!(matches!(&p.body, Expr::Let { name, .. } if name == "summary"));
            }
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn program_with_description() {
        let file = parse_str(
            r#"
            program ex:summarize : ex:Document -> ex:Document {
                description = "Summarize a document";
                input
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => {
                assert_eq!(p.attributes.len(), 1);
                assert!(
                    matches!(&p.attributes[0], ProgramAttribute::Description(s) if s == "Summarize a document")
                );
            }
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn lambda_expression() {
        let file = parse_str(
            r#"
            program ex:test : ex:A -> ex:B {
                \x -> x
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => {
                assert!(matches!(&p.body, Expr::Lambda { param, .. } if param == "x"));
            }
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn projection() {
        let file = parse_str(
            r#"
            program ex:test : ex:A -> ex:B {
                input.ex:name
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => {
                assert!(matches!(&p.body, Expr::Project { .. }));
            }
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn construct_expression() {
        let file = parse_str(
            r#"
            program ex:test : ex:A -> ex:B {
                Construct ex:Dog { name = "Rex", breed = "GSD" }
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => match &p.body {
                Expr::ConstructExpr { class, fields, .. } => {
                    assert_eq!(class.name, "Dog");
                    assert_eq!(fields.len(), 2);
                }
                _ => panic!("expected construct"),
            },
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn pair_expression() {
        let file = parse_str(
            r#"
            program ex:test : ex:A -> ex:B {
                ("hello", 42)
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => {
                assert!(matches!(&p.body, Expr::Pair { .. }));
            }
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn map_expression() {
        let file = parse_str(
            r#"
            program ex:test : ex:A -> ex:B {
                map(\x -> x, items)
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => {
                assert!(matches!(&p.body, Expr::MapExpr { .. }));
            }
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn full_file() {
        let input = r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            class ex:Document {
                description = "A text document";
                requires ex:text;
            }

            property ex:text : core:string {
                description = "The text content";
            }

            resource ex:doc1 : ex:Document {
                ex:text = "Hello world";
            }

            program ex:summarize : ex:Document -> ex:Document {
                description = "Summarize a document";
                let summary : core:string = CompleteText(input);
                Construct ex:Document { text = summary }
            }
        "#;
        let file = parse_str(input).unwrap();
        assert_eq!(file.namespaces.len(), 2);
        assert_eq!(file.declarations.len(), 4);
    }

    #[test]
    fn case_expression() {
        let file = parse_str(
            r#"
            program ex:test : ex:A -> ex:B {
                case result {
                    Ok -> value;
                    Err -> fallback;
                }
            }
        "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => match &p.body {
                Expr::Case { branches, .. } => {
                    assert_eq!(branches.len(), 2);
                    assert_eq!(branches[0].0, "Ok");
                    assert_eq!(branches[1].0, "Err");
                }
                _ => panic!("expected case"),
            },
            _ => panic!("expected program"),
        }
    }

    // --- `data` declarations (Phase 11b step 7, D19 §10) ---

    #[test]
    fn data_nat_non_parametric_with_recursive_arg() {
        let file = parse_str(
            r#"
            data ex:Nat {
                zero,
                succ(ex:Nat),
            }
            "#,
        )
        .unwrap();
        assert_eq!(file.declarations.len(), 1);
        match &file.declarations[0] {
            Declaration::Data(d) => {
                assert_eq!(d.name.name, "Nat");
                assert!(d.params.is_empty());
                assert_eq!(d.ctors.len(), 2);
                assert_eq!(d.ctors[0].name(), "zero");
                assert!(d.ctors[0].args().is_empty());
                assert_eq!(d.ctors[1].name(), "succ");
                assert_eq!(d.ctors[1].args().len(), 1);
                match &d.ctors[1].args()[0] {
                    CtorArg::Positional(t) => {
                        assert_eq!(t.name.name, "Nat");
                        assert!(t.params.is_empty());
                    }
                    other => panic!("expected Positional, got {other:?}"),
                }
            }
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn data_bool_two_nullary_ctors() {
        let file = parse_str(
            r#"
            data ex:Bool {
                tt,
                ff,
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => {
                assert_eq!(d.name.name, "Bool");
                assert_eq!(d.ctors.len(), 2);
                assert!(d.ctors[0].args().is_empty());
                assert!(d.ctors[1].args().is_empty());
            }
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn data_list_parametric_with_self_reference() {
        let file = parse_str(
            r#"
            data ex:List(A : core:Set) {
                nil,
                cons(A, ex:List(A)),
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => {
                assert_eq!(d.name.name, "List");
                assert_eq!(d.params.len(), 1);
                assert_eq!(d.params[0].name, "A");
                match &d.params[0].kind {
                    IndexKind::Named(qn) => assert_eq!(qn.name, "Set"),
                    other => panic!("expected Named param kind, got {other:?}"),
                }
                assert_eq!(d.ctors.len(), 2);
                assert_eq!(d.ctors[0].name(), "nil");
                assert!(d.ctors[0].args().is_empty());
                assert_eq!(d.ctors[1].name(), "cons");
                assert_eq!(d.ctors[1].args().len(), 2);
                // First arg: bare `A` (param reference) — positional
                match &d.ctors[1].args()[0] {
                    CtorArg::Positional(t) => {
                        assert_eq!(t.name.name, "A");
                        assert!(t.params.is_empty());
                    }
                    other => panic!("expected Positional, got {other:?}"),
                }
                // Second arg: `ex:List(A)` — positional, applied to `A`.
                match &d.ctors[1].args()[1] {
                    CtorArg::Positional(t) => {
                        assert_eq!(t.name.name, "List");
                        assert_eq!(t.params.len(), 1);
                        assert_eq!(t.params[0].name.name, "A");
                    }
                    other => panic!("expected Positional, got {other:?}"),
                }
            }
            _ => panic!("expected data"),
        }
    }

    // --- Bounded binders in ctor args (Phase 11b step 15h.2) ---

    #[test]
    fn data_ctor_bounded_size_binder_implicit_kind() {
        // `{j < i}` — size kind implicit, bound to `i`.
        let file = parse_str(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat(i : core:Size) {
                zero,
                succ({j < i}, ex:Nat(j)),
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => {
                let succ = &d.ctors[1];
                assert_eq!(succ.args().len(), 2);
                match &succ.args()[0] {
                    CtorArg::Named {
                        name, kind, bound, ..
                    } => {
                        assert_eq!(name, "j");
                        assert_eq!(kind.name, "Size");
                        assert!(kind.namespace.is_none());
                        let b = bound.as_ref().expect("bound present");
                        assert_eq!(b.name, "i");
                    }
                    other => panic!("expected Named, got {other:?}"),
                }
                assert!(matches!(&succ.args()[1], CtorArg::Positional(_)));
            }
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn data_ctor_bounded_size_binder_explicit_kind() {
        // `{j : core:Size < i}` — same as implicit-kind form.
        let file = parse_str(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat(i : core:Size) {
                zero,
                succ({j : core:Size < i}, ex:Nat(j)),
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => match &d.ctors[1].args()[0] {
                CtorArg::Named {
                    name, kind, bound, ..
                } => {
                    assert_eq!(name, "j");
                    assert_eq!(kind.namespace.as_deref(), Some("core"));
                    assert_eq!(kind.name, "Size");
                    assert_eq!(bound.as_ref().unwrap().name, "i");
                }
                other => panic!("expected Named, got {other:?}"),
            },
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn data_ctor_unbounded_named_binder() {
        // `{A : core:Set}` — unbounded Pi binder, kind Set.
        let file = parse_str(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Wrap {
                mk({A : core:Set}, A),
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => match &d.ctors[0].args()[0] {
                CtorArg::Named {
                    name, kind, bound, ..
                } => {
                    assert_eq!(name, "A");
                    assert_eq!(kind.name, "Set");
                    assert!(bound.is_none());
                }
                other => panic!("expected Named, got {other:?}"),
            },
            _ => panic!("expected data"),
        }
    }

    // --- eigenius#72 Layer 2: indexed data declarations ---

    #[test]
    fn data_vec_indexed_with_typed_ctors() {
        let file = parse_str(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Vec(A : core:Set) : core:Nat -> Set {
                nil : ex:Vec(A, ex:zero),
                cons : forall (n : core:Nat) => A -> ex:Vec(A, n) -> ex:Vec(A, ex:succ(n)),
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => {
                assert_eq!(d.name.name, "Vec");
                assert_eq!(d.params.len(), 1);
                assert_eq!(d.indices.len(), 1);
                assert_eq!(d.indices[0].name, "_");
                match &d.indices[0].kind {
                    IndexKind::Named(qn) => assert_eq!(qn.name, "Nat"),
                    other => panic!("expected Named index kind, got {other:?}"),
                }
                assert_eq!(d.result_sort, Some(SortKind::Set));
                assert_eq!(d.ctors.len(), 2);
                assert!(matches!(&d.ctors[0], CtorDecl::Typed { name, .. } if name == "nil"));
                assert!(matches!(&d.ctors[1], CtorDecl::Typed { name, .. } if name == "cons"));
            }
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn data_eq_indexed_in_prop_with_two_indices() {
        let file = parse_str(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Eq(A : core:Set) : A -> A -> Prop {
                refl : forall (a : A) => ex:Eq(A, a, a),
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => {
                assert_eq!(d.indices.len(), 2);
                for idx in &d.indices {
                    match &idx.kind {
                        IndexKind::Named(qn) => assert_eq!(qn.name, "A"),
                        other => panic!("expected Named index kind, got {other:?}"),
                    }
                }
                assert_eq!(d.result_sort, Some(SortKind::Prop));
            }
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn data_indexed_rejects_arrow_in_index_telescope_without_sort() {
        // Trailing segment must be a sort literal.
        let err = parse_str(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Bad(A : core:Set) : core:Nat -> core:Nat {
                mk : ex:Bad(A, ex:zero, ex:zero),
            }
            "#,
        )
        .unwrap_err();
        assert!(
            err.message.contains("sort literal"),
            "expected sort-literal error, got: {}",
            err.message
        );
    }

    #[test]
    fn data_unindexed_with_result_sort_only() {
        // `data Foo : Set { ... }` is allowed — empty index telescope,
        // explicit result sort.
        let file = parse_str(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Unit : Set {
                tt,
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => {
                assert!(d.indices.is_empty());
                assert_eq!(d.result_sort, Some(SortKind::Set));
                assert_eq!(d.ctors.len(), 1);
                assert!(matches!(&d.ctors[0], CtorDecl::Positional { name, .. } if name == "tt"));
            }
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn data_indexed_accepts_sort_literals_in_intermediate_indices() {
        // D39 §5 JustifiedBy / D49 ChainWitness predicates need Sort
        // literals (Prop, Set, Type N) as intermediate index kinds, not
        // just bare names or class IRIs.
        let file = parse_str(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:JustifiedBy : ex:Term -> Prop -> Type 0 {
                mk : forall (t : ex:Term) => forall (P : Prop) => ex:JustifiedBy(t, P),
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => {
                assert_eq!(d.indices.len(), 2);
                match &d.indices[0].kind {
                    IndexKind::Named(qn) => assert_eq!(qn.name, "Term"),
                    other => panic!("expected Named, got {other:?}"),
                }
                match &d.indices[1].kind {
                    IndexKind::Sort(SortKind::Prop) => {}
                    other => panic!("expected Sort(Prop), got {other:?}"),
                }
                assert_eq!(d.result_sort, Some(SortKind::Type(0)));
            }
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn data_indexed_accepts_all_three_sort_literals_in_indices() {
        // Exercises Prop, Set, and Type N as intermediate index kinds in
        // a single declaration to confirm the parser/AST/encoder path
        // handles each shape.
        let file = parse_str(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Triple : Prop -> Set -> Type 2 -> Type 3 {
                mk : forall (p : Prop) => forall (s : Set) => forall (t : Type 2) => ex:Triple(p, s, t),
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Data(d) => {
                assert_eq!(d.indices.len(), 3);
                assert!(matches!(d.indices[0].kind, IndexKind::Sort(SortKind::Prop)));
                assert!(matches!(d.indices[1].kind, IndexKind::Sort(SortKind::Set)));
                assert!(matches!(
                    d.indices[2].kind,
                    IndexKind::Sort(SortKind::Type(2))
                ));
                assert_eq!(d.result_sort, Some(SortKind::Type(3)));
            }
            _ => panic!("expected data"),
        }
    }

    // --- eigenius#72 Layer 3: match returning with Lambda motive ---

    #[test]
    fn match_returning_with_fun_lambda_motive_parses() {
        let file = parse_str(
            r#"
            namespace ex = "urn:eigenius:example";
            namespace core = "urn:eigenius:core";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:identity : ex:Nat -> ex:Nat {
                match input returning fun (n : core:Nat) => ex:Nat {
                    zero -> input;
                    succ(k) -> input;
                }
            }
            "#,
        )
        .unwrap();
        assert_eq!(file.declarations.len(), 2);
        let prog = match &file.declarations[1] {
            Declaration::Program(p) => p,
            _ => panic!("expected program"),
        };
        match &prog.body {
            Expr::Match { returning, .. } => {
                assert!(
                    matches!(returning, Some(TypeExpr::Lambda { .. })),
                    "expected TypeExpr::Lambda motive, got {returning:?}"
                );
            }
            other => panic!("expected Match, got {other:?}"),
        }
    }

    #[test]
    fn match_returning_with_bare_ref_motive_parses_as_typeexpr_ref() {
        // The pre-Layer-3 form `returning T` is still accepted; it
        // now parses as `TypeExpr::Ref` instead of the old
        // `QualifiedName`.
        let file = parse_str(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }

            program ex:identity : ex:Nat -> ex:Nat {
                match input returning ex:Nat {
                    zero -> input;
                    succ(k) -> input;
                }
            }
            "#,
        )
        .unwrap();
        let prog = match &file.declarations[1] {
            Declaration::Program(p) => p,
            _ => panic!("expected program"),
        };
        match &prog.body {
            Expr::Match { returning, .. } => match returning {
                Some(TypeExpr::Ref { name, args, .. }) => {
                    assert_eq!(name.name, "Nat");
                    assert!(args.is_empty());
                }
                other => panic!("expected TypeExpr::Ref, got {other:?}"),
            },
            other => panic!("expected Match, got {other:?}"),
        }
    }

    #[test]
    fn data_zero_constructors_accepted() {
        // Zero-ctor inductives are valid: absurd in `Set`/`Type`, opaque
        // predicates in `Prop`. Required for D39 §4.1 `core:Asserts`,
        // D49 §6 `ChainWitness.Is*As`, and any future Void-shaped or
        // kernel-internal-predicate declaration. The parser does not
        // policing this; well-formedness is a kernel-checker concern.
        let file = parse_str(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Void : Set {
            }

            data ex:Opaque : core:string -> Prop {
            }
            "#,
        )
        .unwrap();
        assert_eq!(file.declarations.len(), 2);
        match &file.declarations[0] {
            Declaration::Data(d) => {
                assert_eq!(d.name.name, "Void");
                assert!(d.ctors.is_empty());
                assert_eq!(d.result_sort, Some(SortKind::Set));
            }
            _ => panic!("expected data"),
        }
        match &file.declarations[1] {
            Declaration::Data(d) => {
                assert_eq!(d.name.name, "Opaque");
                assert!(d.ctors.is_empty());
                assert_eq!(d.result_sort, Some(SortKind::Prop));
                assert_eq!(d.indices.len(), 1);
            }
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn data_empty_param_list_rejected() {
        // `data Name() { ... }` is not valid syntax — must omit `()`
        // for non-parametric inductives.
        let err = parse_str(
            r#"
            data ex:Foo() {
                mk,
            }
            "#,
        )
        .unwrap_err();
        assert!(err.message.contains("empty data parameter list"));
    }

    #[test]
    fn data_empty_ctor_arg_list_rejected() {
        // `mk()` is not valid — write `mk` for nullary.
        let err = parse_str(
            r#"
            data ex:Foo {
                mk(),
            }
            "#,
        )
        .unwrap_err();
        assert!(err.message.contains("empty constructor arg list"));
    }

    #[test]
    fn function_application_collects_all_positional_args() {
        // Parser captures every positional arg into the Vec — no silent
        // drop, no premature arity rejection. The compiler decides what
        // arities are valid based on whether the function is a ctor or
        // a component.
        let file = parse_str(
            r#"
            namespace ex = "urn:eigenius:example";

            program ex:demo : ex:Foo -> ex:Bar {
                f(a, b, c, d)
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => match &p.body {
                Expr::Apply {
                    args,
                    component_argument,
                    ..
                } => {
                    assert_eq!(args.len(), 4, "all 4 args preserved");
                    assert!(component_argument.is_none());
                }
                other => panic!("expected Apply, got {other:?}"),
            },
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn function_application_with_trailing_block_sets_component_argument() {
        // f(a) { key = val; } — block syntax for component config.
        let file = parse_str(
            r#"
            namespace ex = "urn:eigenius:example";

            program ex:demo : ex:Foo -> ex:Bar {
                f(a) { ex:key = "val"; }
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => match &p.body {
                Expr::Apply {
                    args,
                    component_argument,
                    ..
                } => {
                    assert_eq!(args.len(), 1);
                    assert!(component_argument.is_some());
                }
                other => panic!("expected Apply, got {other:?}"),
            },
            _ => panic!("expected program"),
        }
    }

    // --- D37: lambda / pi / merge_comorphism surface forms ---

    #[test]
    fn parses_typed_lambda_literal_with_single_param() {
        // Standalone-style typed lambda. Parses as a single
        // Lambda node with `param_type = Some(Ref(A))`.
        let file = parse_str(
            r#"
            namespace ex = "urn:ex";
            program ex:f : ex:A -> ex:A {
                lambda x : ex:A => x
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => match &p.body {
                Expr::Lambda {
                    param,
                    param_type: Some(TypeExpr::Ref { name, .. }),
                    ..
                } => {
                    assert_eq!(param, "x");
                    assert_eq!(name.name, "A");
                }
                other => panic!("expected typed Lambda, got {other:?}"),
            },
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn parses_typed_lambda_literal_multi_param_folds_to_nested_lambdas() {
        // `lambda a : ex:A, b : ex:A => a` lowers to
        // `Lambda{a, Lambda{b, Var(a)}}`. The leftmost binder is
        // the outermost lambda. Program signatures use
        // `QualifiedName` input/output (single-class on each side);
        // the typechecker would eventually disagree with the
        // multi-arg lambda body's shape, but the parser only builds
        // AST and is happy.
        let file = parse_str(
            r#"
            namespace ex = "urn:ex";
            program ex:f : ex:A -> ex:B {
                lambda a : ex:A, b : ex:A => a
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::Program(p) => match &p.body {
                Expr::Lambda { param, body, .. } => {
                    assert_eq!(param, "a");
                    match body.as_ref() {
                        Expr::Lambda {
                            param: inner_param,
                            body: inner_body,
                            ..
                        } => {
                            assert_eq!(inner_param, "b");
                            assert!(matches!(
                                inner_body.as_ref(),
                                Expr::Var { name, .. } if name == "a"
                            ));
                        }
                        other => panic!("expected inner Lambda, got {other:?}"),
                    }
                }
                other => panic!("expected outer Lambda, got {other:?}"),
            },
            _ => panic!("expected program"),
        }
    }

    #[test]
    fn parses_pi_type_expression() {
        // `pi a : ex:A, b : ex:A => ex:A` as a codata observation
        // type. Codata observation types accept full `TypeExpr`s
        // (unlike `program` whose input/output slots are
        // `QualifiedName` — a separate piece of work to extend),
        // which is the natural existing surface for testing the new
        // Pi expression.
        let file = parse_str(
            r#"
            namespace ex = "urn:ex";
            codata ex:F {
                run : pi a : ex:A, b : ex:A => ex:A;
            }
            "#,
        )
        .unwrap();
        let codata = match &file.declarations[0] {
            Declaration::Codata(c) => c,
            _ => panic!("expected codata"),
        };
        let obs_type = &codata.observations[0].typ;
        match obs_type {
            TypeExpr::Pi {
                params, codomain, ..
            } => {
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].name, "a");
                assert_eq!(params[1].name, "b");
                assert!(matches!(
                    codomain.as_ref(),
                    TypeExpr::Ref { name, .. } if name.name == "A"
                ));
            }
            other => panic!("expected Pi, got {other:?}"),
        }
    }

    #[test]
    fn parses_merge_comorphism_inline_body() {
        let file = parse_str(
            r#"
            namespace ex = "urn:ex";
            merge_comorphism ex:take_b for ex:Patient {
                (a, b, opt) => b
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::MergeComorphism(mc) => {
                assert_eq!(mc.name.name, "take_b");
                assert_eq!(mc.target_class.name, "Patient");
                match &mc.body {
                    MergeComorphismBody::Inline { params, body, .. } => {
                        assert_eq!(
                            params,
                            &["a".to_string(), "b".to_string(), "opt".to_string()]
                        );
                        assert!(matches!(body, Expr::Var { name, .. } if name == "b"));
                    }
                    other => panic!("expected inline body, got {other:?}"),
                }
            }
            _ => panic!("expected merge_comorphism declaration"),
        }
    }

    #[test]
    fn parses_merge_comorphism_reference_body() {
        let file = parse_str(
            r#"
            namespace ex = "urn:ex";
            merge_comorphism ex:take_b for ex:Patient {
                transformation = ex:take_b_term;
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::MergeComorphism(mc) => match &mc.body {
                MergeComorphismBody::Reference { transformation, .. } => {
                    assert_eq!(transformation.namespace.as_deref(), Some("ex"));
                    assert_eq!(transformation.name, "take_b_term");
                }
                other => panic!("expected reference body, got {other:?}"),
            },
            _ => panic!("expected merge_comorphism declaration"),
        }
    }

    #[test]
    fn merge_comorphism_rejects_missing_for_clause() {
        let result = parse_str(
            r#"
            namespace ex = "urn:ex";
            merge_comorphism ex:take_b { (a, b, opt) => b }
            "#,
        );
        assert!(result.is_err(), "missing 'for' clause should fail to parse");
    }

    // --- D43 §3.1 text_index / vector_index parsing (M1) ---

    /// `text_index ex:my_index { ... }` parses into a TextIndexDecl
    /// with the body fields preserved.
    #[test]
    fn parses_text_index_declaration() {
        let file = parse_str(
            r#"
            namespace ex = "urn:ex";
            namespace core = "urn:eigenius:core";
            text_index ex:description_en {
                core:target_property = ex:description;
                core:text_analyzer = "en-stem-v1";
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::TextIndex(ti) => {
                assert_eq!(ti.name.namespace.as_deref(), Some("ex"));
                assert_eq!(ti.name.name, "description_en");
                assert_eq!(ti.body.len(), 2);

                // First field: target_property = ex:description
                assert_eq!(ti.body[0].property.namespace.as_deref(), Some("core"));
                assert_eq!(ti.body[0].property.name, "target_property");
                match &ti.body[0].value {
                    Value::Ref(qn) => {
                        assert_eq!(qn.namespace.as_deref(), Some("ex"));
                        assert_eq!(qn.name, "description");
                    }
                    other => panic!("expected ref value for target_property, got {other:?}"),
                }

                // Second field: text_analyzer = "en-stem-v1"
                assert_eq!(ti.body[1].property.name, "text_analyzer");
                match &ti.body[1].value {
                    Value::String(s) => assert_eq!(s, "en-stem-v1"),
                    other => panic!("expected string value for text_analyzer, got {other:?}"),
                }
            }
            other => panic!("expected text_index declaration, got {other:?}"),
        }
    }

    /// `vector_index ex:my_index { ... }` parses into a VectorIndexDecl
    /// with all field shapes (Ref, String, Int) preserved. Nested-IRI
    /// scopes like `urn:eigenius:core:distances:cosine` are addressed
    /// via separate namespace aliases per ESL's single-colon
    /// QualifiedName grammar.
    #[test]
    fn parses_vector_index_declaration() {
        let file = parse_str(
            r#"
            namespace ex = "urn:ex";
            namespace core = "urn:eigenius:core";
            namespace cd = "urn:eigenius:core:distances";
            namespace cs = "urn:eigenius:core:strategies";
            vector_index ex:description_oai_v3 {
                core:target_property = ex:description;
                core:vec_model = ex:openai_text_embedding_3_large_v3;
                core:vec_dim = 1536;
                core:vec_distance = cd:cosine;
                core:vec_strategy = cs:auto;
            }
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::VectorIndex(vi) => {
                assert_eq!(vi.name.namespace.as_deref(), Some("ex"));
                assert_eq!(vi.name.name, "description_oai_v3");
                assert_eq!(vi.body.len(), 5);

                // Field types preserved per the §3.1 typing
                assert!(matches!(&vi.body[0].value, Value::Ref(_)));
                assert!(matches!(&vi.body[1].value, Value::Ref(_)));
                assert!(matches!(&vi.body[2].value, Value::Int(1536)));
                assert!(matches!(&vi.body[3].value, Value::Ref(_)));
                assert!(matches!(&vi.body[4].value, Value::Ref(_)));

                // Check distance resolves through the dedicated `cd` alias.
                match &vi.body[3].value {
                    Value::Ref(qn) => {
                        assert_eq!(qn.namespace.as_deref(), Some("cd"));
                        assert_eq!(qn.name, "cosine");
                    }
                    other => panic!("expected ref value for vec_distance, got {other:?}"),
                }
            }
            other => panic!("expected vector_index declaration, got {other:?}"),
        }
    }

    /// An empty body is valid syntax (no fields). Useful when an Index
    /// is being staged before its parameters are filled in.
    #[test]
    fn parses_text_index_with_empty_body() {
        let file = parse_str(
            r#"
            namespace ex = "urn:ex";
            text_index ex:stub {}
            "#,
        )
        .unwrap();
        match &file.declarations[0] {
            Declaration::TextIndex(ti) => {
                assert_eq!(ti.name.name, "stub");
                assert!(ti.body.is_empty());
            }
            other => panic!("expected text_index declaration, got {other:?}"),
        }
    }

    /// `text_index` without a name fails — the IRI is required for
    /// the Resource it lowers to.
    #[test]
    fn text_index_rejects_missing_name() {
        let result = parse_str(
            r#"
            namespace ex = "urn:ex";
            text_index { core:text_analyzer = "en-stem-v1"; }
            "#,
        );
        assert!(result.is_err(), "missing IRI should fail to parse");
    }

    /// `vector_index` without a name fails for the same reason.
    #[test]
    fn vector_index_rejects_missing_name() {
        let result = parse_str(
            r#"
            namespace ex = "urn:ex";
            vector_index { core:vec_dim = 1536; }
            "#,
        );
        assert!(result.is_err(), "missing IRI should fail to parse");
    }
}
