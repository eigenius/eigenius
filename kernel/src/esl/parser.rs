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
        // Accept keywords as identifiers (e.g., `core:resource`, `core:property`)
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
        let first = self.expect_ident()?;

        if self.at(&TokenKind::Colon) {
            self.advance(); // :
            let second = self.expect_ident()?;
            Ok(QualifiedName {
                namespace: Some(first),
                name: second,
                pos,
            })
        } else {
            Ok(QualifiedName {
                namespace: None,
                name: first,
                pos,
            })
        }
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
                _ => {
                    return Err(EslError::parser(
                        Some(self.current_pos()),
                        format!(
                            "expected top-level declaration (namespace, class, property, resource, program), found {:?}",
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

    /// `class ex:Dog : ex:Animal { ... }`
    fn parse_class(&mut self) -> Result<ClassDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Class)?;
        let name = self.parse_qualified_name()?;

        let parent = if self.at(&TokenKind::Colon) {
            self.advance();
            Some(self.parse_qualified_name()?)
        } else {
            None
        };

        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            body.push(self.parse_class_item()?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(ClassDecl {
            name,
            parent,
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

    /// `resource ex:rex : ex:Dog { ... }`
    fn parse_resource(&mut self) -> Result<ResourceDecl, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Resource)?;
        let name = self.parse_qualified_name()?;
        self.expect(&TokenKind::Colon)?;
        let class = self.parse_qualified_name()?;

        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            body.push(self.parse_resource_field()?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(ResourceDecl {
            name,
            class,
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
            TokenKind::LBracket => self.parse_array_value(),
            TokenKind::LBrace => self.parse_block_value(),
            TokenKind::Ident(_) => {
                let qn = self.parse_qualified_name()?;
                Ok(Value::Ref(qn))
            }
            _ => Err(EslError::parser(
                Some(self.current_pos()),
                format!("expected value, found {:?}", self.peek()),
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

    // --- Expressions ---

    /// Parse an expression. This is the top-level expression parser.
    /// Precedence (loosest to tightest):
    ///   let, case (extend rightward)
    ///   lambda
    ///   application f(arg)
    ///   projection e.prop
    ///   atoms (var, literal, construct, parens)
    fn parse_expr(&mut self) -> Result<Expr, EslError> {
        match self.peek() {
            TokenKind::Let => self.parse_let(),
            TokenKind::Case => self.parse_case(),
            TokenKind::Backslash | TokenKind::Lambda => self.parse_lambda(),
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
            body: Box::new(body),
            pos,
        })
    }

    /// `case e { A -> e1; B -> e2 }`
    fn parse_case(&mut self) -> Result<Expr, EslError> {
        let pos = self.current_pos();
        self.expect(&TokenKind::Case)?;
        let scrutinee = self.parse_apply_or_atom()?;
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

    /// Parse function application or an atom.
    /// `f(arg)` or `map(f, coll)` or `reduce(f, init, coll)` or plain atom.
    fn parse_apply_or_atom(&mut self) -> Result<Expr, EslError> {
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

                    let argument = if args.is_empty() {
                        Box::new(Expr::Literal {
                            value: LiteralValue::Bool(true), // unit placeholder
                            pos: pos.clone(),
                        })
                    } else {
                        Box::new(args.remove(0))
                    };

                    let component_argument = if args.is_empty() {
                        None
                    } else {
                        Some(Box::new(args.remove(0)))
                    };

                    // Check for trailing block: f(arg) { key = val; }
                    // This is the component argument (static configuration)
                    let component_argument =
                        if component_argument.is_none() && self.at(&TokenKind::LBrace) {
                            Some(Box::new(self.parse_block_expr()?))
                        } else {
                            component_argument
                        };

                    Expr::Apply {
                        function,
                        argument,
                        component_argument,
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

            // Keywords that are also valid in expression position
            TokenKind::Map | TokenKind::Reduce => {
                let pos = self.current_pos();
                let name = self.expect_ident()?;
                Ok(Expr::Var { name, pos })
            }

            // Identifier (variable or qualified name)
            TokenKind::Ident(_) => {
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
                assert!(c.parent.is_none());
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
                assert_eq!(c.parent.as_ref().unwrap().name, "Animal");
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
                assert_eq!(r.class.name, "Dog");
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
}
