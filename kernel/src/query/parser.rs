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

//! Recursive descent parser for EigenQL.
//!
//! Parses a token stream into the AST defined in `ast.rs`.
//! Follows the EBNF grammar in D2 §3.

use crate::ontology::iri::Iri;
use crate::query::ast::*;
use crate::query::error::{Position, QueryError};
use crate::query::lexer::{Token, TokenKind};

/// Parse an EigenQL program from a token stream.
pub fn parse(tokens: Vec<Token>) -> Result<Program, QueryError> {
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program()?;
    parser.expect_eof()?;
    Ok(program)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &TokenKind {
        self.tokens
            .get(self.pos)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    /// Look `offset` tokens ahead of the current position. Returns
    /// `None` past end-of-input (distinct from `TokenKind::Eof`).
    fn peek_at(&self, offset: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + offset).map(|t| &t.kind)
    }

    fn position(&self) -> Option<Position> {
        self.tokens.get(self.pos).map(|t| t.pos.clone())
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &TokenKind) -> Result<&Token, QueryError> {
        if self.peek() == expected {
            Ok(self.advance())
        } else {
            Err(QueryError::parser(
                self.position(),
                format!("expected {expected:?}, got {:?}", self.peek()),
            ))
        }
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek() == kind
    }

    fn at_any(&self, kinds: &[TokenKind]) -> bool {
        kinds.contains(self.peek())
    }

    /// Verify all tokens have been consumed. Called from the top-level
    /// `parse` entry point after `parse_program` returns. Without this,
    /// typos in trailing clauses (`ORDR BY`, `LIMT 5`, raw garbage)
    /// silently terminate the query before the typo and the user sees
    /// a successful partial parse — see eigenius#27.
    fn expect_eof(&self) -> Result<(), QueryError> {
        if self.peek() == &TokenKind::Eof {
            return Ok(());
        }
        let unexpected = self.peek();
        let suggestion = trailing_keyword_typo_hint(unexpected);
        let msg = match suggestion {
            Some(kw) => {
                format!("unexpected token after query body: {unexpected:?} — did you mean `{kw}`?")
            }
            None => format!("unexpected token after query body: {unexpected:?}"),
        };
        Err(QueryError::parser(self.position(), msg))
    }

    // --- Top-level ---

    fn parse_program(&mut self) -> Result<Program, QueryError> {
        let mut definitions = Vec::new();
        while self.at(&TokenKind::Define) {
            definitions.push(self.parse_define()?);
        }
        let query = self.parse_query()?;
        Ok(Program { definitions, query })
    }

    fn parse_define(&mut self) -> Result<RuleDefinition, QueryError> {
        self.expect(&TokenKind::Define)?;
        let name = self.parse_identifier()?;
        self.expect(&TokenKind::LParen)?;
        let variables = self.parse_variable_list()?;
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::From)?;
        let body = self.parse_match_part(/* allow_fiber */ false)?;
        Ok(RuleDefinition {
            name,
            variables,
            body,
        })
    }

    fn parse_variable_list(&mut self) -> Result<Vec<Variable>, QueryError> {
        let mut vars = vec![self.parse_variable()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            vars.push(self.parse_variable()?);
        }
        Ok(vars)
    }

    fn parse_query(&mut self) -> Result<Query, QueryError> {
        let body = self.parse_match_part(/* allow_fiber */ true)?;

        let group_by = if self.at(&TokenKind::Group) {
            self.parse_group_by()?
        } else {
            vec![]
        };

        let (result_classes, result) = if self.at(&TokenKind::Return) {
            self.parse_return()?
        } else {
            (vec![], vec![])
        };

        let order_by = if self.at(&TokenKind::Order) {
            self.parse_order_by()?
        } else {
            vec![]
        };

        let limit = if self.at(&TokenKind::Limit) {
            self.advance();
            Some(self.parse_usize()?)
        } else {
            None
        };

        // D43 §3.3 — `TOP N` is the ranked-truncation surface. The
        // grammar allows it anywhere in the trailing clause set so
        // queries like `WHERE ?d ~ "x" TOP 20` and
        // `WHERE ?d ~ "x" RETURN [] {...} TOP 20` both parse;
        // typecheck enforces the structural constraints (no LIMIT,
        // no ORDER BY, at least one `~` in WHERE).
        let top = if self.at(&TokenKind::Top) {
            self.advance();
            Some(self.parse_usize()?)
        } else {
            None
        };

        let offset = if self.at(&TokenKind::Offset) {
            self.advance();
            Some(self.parse_usize()?)
        } else {
            None
        };

        let distinct = if self.at(&TokenKind::Distinct) {
            self.advance();
            true
        } else {
            false
        };

        Ok(Query {
            body,
            group_by,
            result_classes,
            result,
            order_by,
            limit,
            offset,
            distinct,
            top,
        })
    }

    // --- MATCH part (shared by DEFINE and Query) ---

    /// Parse a MatchPart shared by DEFINE and the top-level query.
    ///
    /// DEFINE bodies (`allow_fiber = false`) get exactly one MATCH
    /// clause and no FIBER — fiber queries are orchestrator-scoped,
    /// relation rules are pure. Top-level queries (`allow_fiber = true`)
    /// may interleave multiple MATCH and FIBER clauses per D2 §3.1.
    fn parse_match_part(&mut self, allow_fiber: bool) -> Result<MatchPart, QueryError> {
        let mut using = Vec::new();
        let mut using_institutions = Vec::new();
        let mut using_namespaces = Vec::new();
        while self.at(&TokenKind::Using) {
            // Peek past USING to distinguish plain USING from USING INSTITUTION
            // and USING NAMESPACE.
            match self.peek_at(1) {
                Some(TokenKind::Institution) => {
                    if !allow_fiber {
                        return Err(QueryError::parser(
                            self.position(),
                            "USING INSTITUTION is only valid in the top-level query, not in DEFINE"
                                .to_string(),
                        ));
                    }
                    using_institutions.push(self.parse_using_institution()?);
                }
                Some(TokenKind::Namespace) => {
                    using_namespaces.extend(self.parse_using_namespace()?);
                }
                _ => {
                    let more = self.parse_using()?;
                    using.extend(more);
                }
            }
        }

        let mut clauses = Vec::new();
        if allow_fiber {
            loop {
                match self.peek() {
                    TokenKind::Match => {
                        for p in self.parse_match_clause()? {
                            clauses.push(Clause::Pattern(p));
                        }
                    }
                    TokenKind::Fiber => {
                        clauses.push(Clause::Fiber(self.parse_fiber_clause()?));
                    }
                    _ => break,
                }
            }
        } else {
            // DEFINE body: exactly one MATCH clause.
            for p in self.parse_match_clause()? {
                clauses.push(Clause::Pattern(p));
            }
        }

        if clauses.is_empty() {
            return Err(QueryError::parser(
                self.position(),
                "expected MATCH or FIBER clause".to_string(),
            ));
        }

        let conditions = if self.at(&TokenKind::Where) {
            self.parse_where()?
        } else {
            vec![]
        };

        Ok(MatchPart {
            using,
            using_institutions,
            using_namespaces,
            clauses,
            conditions,
        })
    }

    /// Parse a single `USING` clause (possibly with multiple comma-separated
    /// IRIs). Caller has already peeked that we're at `USING` (not
    /// `USING INSTITUTION`).
    fn parse_using(&mut self) -> Result<Vec<Iri>, QueryError> {
        let mut iris = Vec::new();
        self.expect(&TokenKind::Using)?;
        loop {
            let s = self.parse_string_lit()?;
            let iri = Iri::parse(&s).map_err(|e| {
                QueryError::parser(self.position(), format!("invalid IRI in USING: {e}"))
            })?;
            iris.push(iri);
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(iris)
    }

    /// `USING NAMESPACE "<prefix>"` (one or more comma-separated prefixes).
    /// Each prefix is the IRI-string prefix of a vocabulary namespace
    /// (e.g. `"urn:eigenius:core:"`) that bare short names resolve within.
    /// Caller has already peeked that we're at `USING NAMESPACE`.
    fn parse_using_namespace(&mut self) -> Result<Vec<String>, QueryError> {
        self.expect(&TokenKind::Using)?;
        self.expect(&TokenKind::Namespace)?;
        let mut prefixes = Vec::new();
        loop {
            let prefix = self.parse_string_lit()?;
            if prefix.is_empty() {
                return Err(QueryError::parser(
                    self.position(),
                    "USING NAMESPACE prefix must be non-empty".to_string(),
                ));
            }
            prefixes.push(prefix);
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(prefixes)
    }

    /// `USING INSTITUTION "iri" AS alias`
    fn parse_using_institution(&mut self) -> Result<InstitutionAlias, QueryError> {
        self.expect(&TokenKind::Using)?;
        self.expect(&TokenKind::Institution)?;
        let iri_str = self.parse_string_lit()?;
        let iri = Iri::parse(&iri_str).map_err(|e| {
            QueryError::parser(
                self.position(),
                format!("invalid IRI in USING INSTITUTION: {e}"),
            )
        })?;
        self.expect(&TokenKind::As)?;
        let alias = self.parse_identifier()?;
        Ok(InstitutionAlias { iri, alias })
    }

    /// `FIBER institution_ref : QueryClass { params } AS ?var [INTO "<iri>"]`
    ///
    /// The optional `INTO` suffix names a chain-resident IRI for the
    /// FIBER's response resource (D14 §9.3 chain-reinsertion via
    /// EigenQL). With `INTO`, the response is committed to the regular
    /// chain as part of the query's commit cycle.
    fn parse_fiber_clause(&mut self) -> Result<FiberClause, QueryError> {
        self.expect(&TokenKind::Fiber)?;
        let institution = self.parse_name()?;
        self.expect(&TokenKind::Colon)?;
        let query_class = self.parse_name()?;
        let params = self.parse_fiber_params()?;
        self.expect(&TokenKind::As)?;
        let binding = self.parse_variable()?;
        let into = if self.at(&TokenKind::Into) {
            self.advance();
            let iri_str = match self.peek().clone() {
                TokenKind::StringLit(s) => {
                    self.advance();
                    s
                }
                other => {
                    return Err(QueryError::parser(
                        self.position(),
                        format!("expected quoted IRI string after FIBER `INTO`, got {other:?}"),
                    ));
                }
            };
            let iri = Iri::parse(&iri_str).map_err(|e| {
                QueryError::parser(
                    self.position(),
                    format!("FIBER INTO target `{iri_str}` is not a valid IRI: {e}"),
                )
            })?;
            Some(iri)
        } else {
            None
        };
        Ok(FiberClause {
            institution,
            query_class,
            params,
            binding,
            into,
        })
    }

    fn parse_fiber_params(&mut self) -> Result<Vec<ParamBinding>, QueryError> {
        self.expect(&TokenKind::LBrace)?;
        if self.at(&TokenKind::RBrace) {
            self.advance();
            return Ok(vec![]);
        }
        let mut params = vec![self.parse_param_binding()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            if self.at(&TokenKind::RBrace) {
                break; // trailing comma
            }
            params.push(self.parse_param_binding()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(params)
    }

    fn parse_param_binding(&mut self) -> Result<ParamBinding, QueryError> {
        let name = self.parse_name()?;
        self.expect(&TokenKind::Colon)?;
        let expression = self.parse_expression()?;
        // D2 v2 §3.5 — disambiguate comorphism coercion from a plain
        // expression. A single-arg qualified-name function call in
        // FIBER param value position is treated as a comorphism
        // coercion (the type checker validates it actually resolves
        // to a Comorphism declaration). Anything else is a plain
        // expression.
        let value = match expression {
            Expression::FunctionCall {
                name: fname,
                mut args,
            } if fname.contains(':') && args.len() == 1 => {
                let source = args.pop().expect("len == 1 checked above");
                let coercion = if let Ok(iri) = Iri::parse(&fname) {
                    crate::query::ast::Name::FullIri(iri)
                } else {
                    crate::query::ast::Name::ShortName(fname)
                };
                crate::query::ast::ParamValue::Comorphism {
                    name: coercion,
                    source,
                }
            }
            other => crate::query::ast::ParamValue::Expression(other),
        };
        Ok(ParamBinding { name, value })
    }

    fn parse_match_clause(&mut self) -> Result<Vec<Pattern>, QueryError> {
        self.expect(&TokenKind::Match)?;
        let mut patterns = vec![self.parse_pattern()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            patterns.push(self.parse_pattern()?);
        }
        Ok(patterns)
    }

    fn parse_pattern(&mut self) -> Result<Pattern, QueryError> {
        // Check for negated pattern
        let negated = if self.at(&TokenKind::Not) {
            self.advance();
            true
        } else {
            false
        };

        // Typed pattern: Name(variable) { ... }
        // Untyped pattern: variable { ... }
        // Derived relation: Name(variable, ...) { ... }  (from DEFINE)
        match self.peek().clone() {
            TokenKind::Variable(_) => {
                let subject = self.parse_variable()?;
                let properties = self.parse_object_pattern()?;
                Ok(Pattern {
                    subject,
                    class: None,
                    properties,
                    negated,
                })
            }
            TokenKind::Identifier(_) | TokenKind::StringLit(_) => {
                let class = self.parse_name()?;
                self.expect(&TokenKind::LParen)?;
                let subject = self.parse_variable()?;
                self.expect(&TokenKind::RParen)?;
                let properties = self.parse_object_pattern()?;
                Ok(Pattern {
                    subject,
                    class: Some(class),
                    properties,
                    negated,
                })
            }
            _ => Err(QueryError::parser(
                self.position(),
                format!("expected pattern, got {:?}", self.peek()),
            )),
        }
    }

    fn parse_object_pattern(&mut self) -> Result<Vec<PropertyPattern>, QueryError> {
        self.expect(&TokenKind::LBrace)?;
        if self.at(&TokenKind::RBrace) {
            self.advance();
            return Ok(vec![]);
        }
        let mut props = vec![self.parse_property_pattern()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            if self.at(&TokenKind::RBrace) {
                break; // trailing comma
            }
            props.push(self.parse_property_pattern()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(props)
    }

    fn parse_property_pattern(&mut self) -> Result<PropertyPattern, QueryError> {
        let property = self.parse_name()?;
        self.expect(&TokenKind::Colon)?;
        let object = self.parse_value_or_variable()?;
        Ok(PropertyPattern { property, object })
    }

    // --- WHERE ---

    fn parse_where(&mut self) -> Result<Vec<Expression>, QueryError> {
        self.expect(&TokenKind::Where)?;
        self.parse_expression_list()
    }

    // --- Expressions (precedence climbing) ---

    fn parse_expression_list(&mut self) -> Result<Vec<Expression>, QueryError> {
        let mut exprs = vec![self.parse_expression()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            exprs.push(self.parse_expression()?);
        }
        Ok(exprs)
    }

    fn parse_expression(&mut self) -> Result<Expression, QueryError> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> Result<Expression, QueryError> {
        let mut left = self.parse_and_expr()?;
        while self.at(&TokenKind::Or) {
            self.advance();
            let right = self.parse_and_expr()?;
            left = Expression::Binary {
                op: BinaryOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and_expr(&mut self) -> Result<Expression, QueryError> {
        let mut left = self.parse_equality_expr()?;
        while self.at(&TokenKind::And) {
            self.advance();
            let right = self.parse_equality_expr()?;
            left = Expression::Binary {
                op: BinaryOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality_expr(&mut self) -> Result<Expression, QueryError> {
        let mut left = self.parse_relational_expr()?;
        while self.at_any(&[TokenKind::Eq, TokenKind::Neq]) {
            let op = match self.advance().kind {
                TokenKind::Eq => BinaryOp::Eq,
                TokenKind::Neq => BinaryOp::Neq,
                _ => unreachable!(),
            };
            let right = self.parse_relational_expr()?;
            left = Expression::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_relational_expr(&mut self) -> Result<Expression, QueryError> {
        let mut left = self.parse_additive_expr()?;
        // D43 §3.3 — similarity operator `~`. Sits at relational
        // precedence so `?a ~ "x" AND ?b ~ "y"` and
        // `?a ~ "x" OR ?b ~ "y"` parse as expected; `~` itself does
        // not chain on the left, so we handle it before the loop and
        // do not re-enter it.
        if matches!(self.peek(), TokenKind::Tilde) {
            return self.parse_similarity_continuation(left);
        }
        loop {
            let op = match self.peek() {
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Lte => BinaryOp::Lte,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::Gte => BinaryOp::Gte,
                TokenKind::In => BinaryOp::In,
                TokenKind::Like => BinaryOp::Like,
                TokenKind::Not => {
                    // NOT IN or NOT LIKE
                    if let Some(next) = self.tokens.get(self.pos + 1) {
                        match next.kind {
                            TokenKind::In => {
                                self.advance(); // NOT
                                self.advance(); // IN
                                let right = self.parse_additive_expr()?;
                                left = Expression::Binary {
                                    op: BinaryOp::NotIn,
                                    left: Box::new(left),
                                    right: Box::new(right),
                                };
                                continue;
                            }
                            TokenKind::Like => {
                                self.advance(); // NOT
                                self.advance(); // LIKE
                                let right = self.parse_additive_expr()?;
                                left = Expression::Binary {
                                    op: BinaryOp::NotLike,
                                    left: Box::new(left),
                                    right: Box::new(right),
                                };
                                continue;
                            }
                            _ => break,
                        }
                    } else {
                        break;
                    }
                }
                _ => break,
            };
            self.advance();
            let right = self.parse_additive_expr()?;
            left = Expression::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// D43 §3.3 — finish parsing the similarity operator after the
    /// LHS has been consumed at relational precedence and `~` has
    /// been peeked. Syntactic rules enforced here:
    ///
    /// 1. The LHS must be a bare property-bound variable
    ///    (`Expression::Variable`). Anything else fails with a parse
    ///    error pointing at the `~`. Semantic checks (was the var
    ///    bound, is the property similarity-indexed) run at
    ///    typecheck.
    /// 2. The RHS is parsed at additive precedence so hint
    ///    delimiters (`{`, `}`) and boolean combinators (`AND`,
    ///    `OR`) terminate the operand cleanly without requiring
    ///    parentheses.
    /// 3. An optional trailing `{ hint, hint }` block is parsed
    ///    via [`parse_hint_set`]; the trailing braces are
    ///    distinguishable from a code-block context because the
    ///    relational-expr parser doesn't expect `{` here.
    fn parse_similarity_continuation(
        &mut self,
        left: Expression,
    ) -> Result<Expression, QueryError> {
        let tilde_pos = self.position();
        let property = match left {
            Expression::Variable(v) => v,
            _ => {
                return Err(QueryError::parser(
                    tilde_pos,
                    "similarity LHS must be a property-bound variable (`?var ~ \"query\"`)"
                        .to_string(),
                ));
            }
        };
        self.advance(); // consume `~`
        let query = self.parse_additive_expr()?;
        let hints = if matches!(self.peek(), TokenKind::LBrace) {
            self.parse_hint_set()?
        } else {
            HintSet::default()
        };
        Ok(Expression::Similarity {
            property,
            query: Box::new(query),
            hints,
        })
    }

    /// D43 §3.4 — parse `{ key: value, key: value, ... }` immediately
    /// after a similarity RHS. Allowed keys: `via`, `model`, `k`,
    /// `limit`. Repeated keys overwrite (the typechecker will catch
    /// the more interesting semantic conflicts; the parser only
    /// enforces shape).
    ///
    /// On `via`: the parser accepts the bare identifiers `text`,
    /// `vector`, `hybrid`; anything else is a parse error. On
    /// `model`: a string literal (an IRI in user space). On `k` /
    /// `limit`: a positive integer literal — zero or negative is
    /// rejected at typecheck so the parser stays a shape-only pass.
    fn parse_hint_set(&mut self) -> Result<HintSet, QueryError> {
        self.expect(&TokenKind::LBrace)?;
        let mut hints = HintSet::default();
        loop {
            if matches!(self.peek(), TokenKind::RBrace) {
                self.advance();
                return Ok(hints);
            }
            let key_pos = self.position();
            let key = self.parse_identifier()?;
            self.expect(&TokenKind::Colon)?;
            match key.as_str() {
                "via" => {
                    let v_pos = self.position();
                    let v = self.parse_identifier()?;
                    hints.via = Some(match v.as_str() {
                        "text" => Via::Text,
                        "vector" => Via::Vector,
                        "hybrid" => Via::Hybrid,
                        other => {
                            return Err(QueryError::parser(
                                v_pos,
                                format!(
                                    "hint `via` must be `text`, `vector`, or `hybrid` (got `{other}`)"
                                ),
                            ));
                        }
                    });
                }
                "model" => {
                    hints.model = Some(self.parse_string_lit()?);
                }
                "k" => {
                    hints.k = Some(self.parse_usize()?);
                }
                "limit" => {
                    hints.limit = Some(self.parse_usize()?);
                }
                other => {
                    return Err(QueryError::parser(
                        key_pos,
                        format!(
                            "unknown similarity hint `{other}` (allowed: via, model, k, limit)"
                        ),
                    ));
                }
            }
            match self.peek() {
                TokenKind::Comma => {
                    self.advance();
                }
                TokenKind::RBrace => {}
                _ => {
                    return Err(QueryError::parser(
                        self.position(),
                        "expected `,` or `}` in similarity hint set".to_string(),
                    ));
                }
            }
        }
    }

    fn parse_additive_expr(&mut self) -> Result<Expression, QueryError> {
        let mut left = self.parse_multiplicative_expr()?;
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                TokenKind::Pipe2 => BinaryOp::StringConcat,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative_expr()?;
            left = Expression::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative_expr(&mut self) -> Result<Expression, QueryError> {
        let mut left = self.parse_power_expr()?;
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_power_expr()?;
            left = Expression::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_power_expr(&mut self) -> Result<Expression, QueryError> {
        let mut left = self.parse_unary_expr()?;
        while self.at(&TokenKind::DoubleStar) {
            self.advance();
            let right = self.parse_unary_expr()?;
            left = Expression::Binary {
                op: BinaryOp::Pow,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary_expr(&mut self) -> Result<Expression, QueryError> {
        match self.peek() {
            TokenKind::Not => {
                self.advance();
                // NOT EXISTS(?var)
                if self.at(&TokenKind::Exists) {
                    self.advance();
                    self.expect(&TokenKind::LParen)?;
                    let var = self.parse_variable()?;
                    self.expect(&TokenKind::RParen)?;
                    return Ok(Expression::NotExists(var));
                }
                let operand = self.parse_unary_expr()?;
                Ok(Expression::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(operand),
                })
            }
            TokenKind::Plus => {
                self.advance();
                let operand = self.parse_unary_expr()?;
                Ok(Expression::Unary {
                    op: UnaryOp::Pos,
                    operand: Box::new(operand),
                })
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_unary_expr()?;
                Ok(Expression::Unary {
                    op: UnaryOp::Neg,
                    operand: Box::new(operand),
                })
            }
            _ => self.parse_verdict_term(),
        }
    }

    /// `verdict_term ::= primary_expr (verdict_predicate)?`. The postfix
    /// Verdict predicate (HOLDS / FAILS / UNDECIDABLE) is non-associative
    /// — `?v HOLDS FAILS` is rejected by the consume-once shape below
    /// (the second predicate keyword would not match any continuation
    /// in the grammar above this position).
    fn parse_verdict_term(&mut self) -> Result<Expression, QueryError> {
        let primary = self.parse_primary_expr()?;
        let kind = match self.peek() {
            TokenKind::Holds => crate::query::ast::VerdictPredicate::Holds,
            TokenKind::Fails => crate::query::ast::VerdictPredicate::Fails,
            TokenKind::Undecidable => crate::query::ast::VerdictPredicate::Undecidable,
            _ => return Ok(primary),
        };
        self.advance();
        Ok(Expression::VerdictPredicate {
            kind,
            operand: Box::new(primary),
        })
    }

    fn parse_primary_expr(&mut self) -> Result<Expression, QueryError> {
        match self.peek().clone() {
            // Parenthesized expression
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(&TokenKind::RParen)?;
                Ok(expr)
            }

            // Array literal
            TokenKind::LBracket => {
                self.advance();
                if self.at(&TokenKind::RBracket) {
                    self.advance();
                    return Ok(Expression::Array(vec![]));
                }
                let elements = self.parse_expression_list()?;
                self.expect(&TokenKind::RBracket)?;
                Ok(Expression::Array(elements))
            }

            // Function calls and aggregates
            TokenKind::DateFn
            | TokenKind::TimestampFn
            | TokenKind::RegexFn
            | TokenKind::LengthFn
            | TokenKind::ContainsFn
            | TokenKind::ConcatFn => {
                let name = match self.advance().kind {
                    TokenKind::DateFn => "DATE",
                    TokenKind::TimestampFn => "TIMESTAMP",
                    TokenKind::RegexFn => "REGEX",
                    TokenKind::LengthFn => "LENGTH",
                    TokenKind::ContainsFn => "CONTAINS",
                    TokenKind::ConcatFn => "CONCAT",
                    _ => unreachable!(),
                }
                .to_string();
                self.expect(&TokenKind::LParen)?;
                let args = self.parse_expression_list()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expression::FunctionCall { name, args })
            }

            TokenKind::CountFn
            | TokenKind::SumFn
            | TokenKind::AvgFn
            | TokenKind::MinFn
            | TokenKind::MaxFn => {
                let op = match self.advance().kind {
                    TokenKind::CountFn => AggregateOp::Count,
                    TokenKind::SumFn => AggregateOp::Sum,
                    TokenKind::AvgFn => AggregateOp::Avg,
                    TokenKind::MinFn => AggregateOp::Min,
                    TokenKind::MaxFn => AggregateOp::Max,
                    _ => unreachable!(),
                };
                self.expect(&TokenKind::LParen)?;
                let arg = self.parse_expression()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expression::Aggregate {
                    op,
                    arg: Box::new(arg),
                })
            }

            // Variable (possibly with dot-path)
            TokenKind::Variable(_) => {
                let var = self.parse_variable()?;
                if self.at(&TokenKind::Dot) {
                    let mut segments = Vec::new();
                    while self.at(&TokenKind::Dot) {
                        self.advance();
                        segments.push(self.parse_identifier()?);
                    }
                    Ok(Expression::DotPath {
                        root: var,
                        segments,
                    })
                } else {
                    Ok(Expression::Variable(var))
                }
            }

            // String literal — and, if directly followed by `(`, a
            // function call whose name is the literal IRI string. This
            // matches D2 v2 §3.5 / §3.8's `qualified_name ::= IDENTIFIER ':' IDENTIFIER | STRING`,
            // allowing comorphism coercion / decide-predicate calls
            // written with a full quoted IRI rather than a namespace
            // alias.
            TokenKind::StringLit(_) => {
                let s = self.parse_string_lit()?;
                if self.at(&TokenKind::LParen) {
                    self.advance();
                    let args = self.parse_expression_list()?;
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expression::FunctionCall { name: s, args })
                } else {
                    Ok(Expression::Literal(Literal::String(s)))
                }
            }

            // Number literal
            TokenKind::NumberInt(n) => {
                let expr = Expression::Literal(Literal::Integer(n));
                self.advance();
                Ok(expr)
            }
            TokenKind::NumberFloat(f) => {
                let expr = Expression::Literal(Literal::Float(f));
                self.advance();
                Ok(expr)
            }

            // Boolean literal
            TokenKind::BooleanLit(b) => {
                let expr = Expression::Literal(Literal::Boolean(b));
                self.advance();
                Ok(expr)
            }

            // Identifier as expression — a bare shortname literal, or a
            // qualified-name function call (Phase 11e.2).
            //
            // `ns:local(args)` is dispatched through the institution
            // registry at evaluate time: if the resolved IRI classifies
            // as a decide predicate, the call returns a boolean; if it
            // classifies as a comorphism, the call returns a resource.
            // Unrecognised IRIs fall through to builtin dispatch which
            // produces a "no such function" error.
            TokenKind::Identifier(_) => {
                let first = self.parse_identifier()?;
                // `ident : ident` — qualified name. Otherwise bare shortname.
                let full_name = if self.at(&TokenKind::Colon) {
                    self.advance();
                    let local = self.parse_identifier()?;
                    format!("{first}:{local}")
                } else {
                    first
                };
                if self.at(&TokenKind::LParen) {
                    self.advance();
                    let args = self.parse_expression_list()?;
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expression::FunctionCall {
                        name: full_name,
                        args,
                    })
                } else {
                    Ok(Expression::Literal(Literal::String(full_name)))
                }
            }

            _ => Err(QueryError::parser(
                self.position(),
                format!("expected expression, got {:?}", self.peek()),
            )),
        }
    }

    // --- RETURN ---

    fn parse_return(&mut self) -> Result<(Vec<Name>, Vec<ReturnItem>), QueryError> {
        self.expect(&TokenKind::Return)?;

        // Result class names
        let classes = match self.peek() {
            TokenKind::LBracket => {
                self.advance();
                if self.at(&TokenKind::RBracket) {
                    self.advance();
                    vec![]
                } else {
                    let names = self.parse_name_list()?;
                    self.expect(&TokenKind::RBracket)?;
                    names
                }
            }
            TokenKind::LBrace => vec![], // No class name, go straight to body
            _ => vec![self.parse_name()?],
        };

        self.expect(&TokenKind::LBrace)?;
        let mut items = Vec::new();
        if !self.at(&TokenKind::RBrace) {
            items.push(self.parse_return_item()?);
            while self.at(&TokenKind::Comma) {
                self.advance();
                if self.at(&TokenKind::RBrace) {
                    break;
                }
                items.push(self.parse_return_item()?);
            }
        }
        self.expect(&TokenKind::RBrace)?;

        Ok((classes, items))
    }

    fn parse_return_item(&mut self) -> Result<ReturnItem, QueryError> {
        let name = self.parse_name()?;
        self.expect(&TokenKind::Colon)?;
        let expression = self.parse_expression()?;
        Ok(ReturnItem { name, expression })
    }

    // --- GROUP BY ---

    fn parse_group_by(&mut self) -> Result<Vec<Expression>, QueryError> {
        self.expect(&TokenKind::Group)?;
        self.expect(&TokenKind::By)?;
        self.parse_expression_list()
    }

    // --- ORDER BY ---

    fn parse_order_by(&mut self) -> Result<Vec<OrderItem>, QueryError> {
        self.expect(&TokenKind::Order)?;
        self.expect(&TokenKind::By)?;
        let mut items = vec![self.parse_order_item()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            items.push(self.parse_order_item()?);
        }
        Ok(items)
    }

    fn parse_order_item(&mut self) -> Result<OrderItem, QueryError> {
        let expression = self.parse_expression()?;
        let direction = match self.peek() {
            TokenKind::Asc => {
                self.advance();
                SortDirection::Asc
            }
            TokenKind::Desc => {
                self.advance();
                SortDirection::Desc
            }
            _ => SortDirection::Asc,
        };
        Ok(OrderItem {
            expression,
            direction,
        })
    }

    // --- Helpers ---

    fn parse_name(&mut self) -> Result<Name, QueryError> {
        match self.peek().clone() {
            TokenKind::Identifier(s) => {
                self.advance();
                Ok(Name::ShortName(s))
            }
            TokenKind::StringLit(s) => {
                self.advance();
                let iri = Iri::parse(&s).map_err(|e| {
                    QueryError::parser(self.position(), format!("invalid IRI: {e}"))
                })?;
                Ok(Name::FullIri(iri))
            }
            _ => Err(QueryError::parser(
                self.position(),
                format!(
                    "expected name (identifier or string), got {:?}",
                    self.peek()
                ),
            )),
        }
    }

    fn parse_name_list(&mut self) -> Result<Vec<Name>, QueryError> {
        let mut names = vec![self.parse_name()?];
        while self.at(&TokenKind::Comma) {
            self.advance();
            names.push(self.parse_name()?);
        }
        Ok(names)
    }

    fn parse_variable(&mut self) -> Result<Variable, QueryError> {
        match self.peek().clone() {
            TokenKind::Variable(name) => {
                self.advance();
                Ok(Variable::new(&name))
            }
            _ => Err(QueryError::parser(
                self.position(),
                format!("expected variable (?name), got {:?}", self.peek()),
            )),
        }
    }

    /// Parse an array pattern (D59) in a property-object position.
    /// Forms: `[]`, `[?a]`, `[?a, ?b]` (Exact); `[?a, ...]`, `[?a, ?b, ...]`
    /// (AtLeast); `[... ?e ...]` (Each — iterate one binding per element).
    fn parse_array_pattern(&mut self) -> Result<ArrayPattern, QueryError> {
        self.expect(&TokenKind::LBracket)?;
        // `[]`
        if matches!(self.peek(), TokenKind::RBracket) {
            self.advance();
            return Ok(ArrayPattern::Exact(vec![]));
        }
        // `[... ?e ...]`
        if matches!(self.peek(), TokenKind::Ellipsis) {
            self.advance();
            let var = self.parse_variable()?;
            self.expect(&TokenKind::Ellipsis)?;
            self.expect(&TokenKind::RBracket)?;
            return Ok(ArrayPattern::Each(var));
        }
        // `[?a]`, `[?a, ?b]`, or `[?a, ?b, ...]`
        let mut vars = Vec::new();
        loop {
            vars.push(self.parse_variable()?);
            if matches!(self.peek(), TokenKind::Comma) {
                self.advance();
                if matches!(self.peek(), TokenKind::Ellipsis) {
                    self.advance();
                    self.expect(&TokenKind::RBracket)?;
                    return Ok(ArrayPattern::AtLeast(vars));
                }
                continue;
            }
            break;
        }
        self.expect(&TokenKind::RBracket)?;
        Ok(ArrayPattern::Exact(vars))
    }

    fn parse_value_or_variable(&mut self) -> Result<ValueOrVariable, QueryError> {
        match self.peek().clone() {
            TokenKind::LBracket => Ok(ValueOrVariable::Array(self.parse_array_pattern()?)),
            TokenKind::Variable(_) => {
                let var = self.parse_variable()?;
                Ok(ValueOrVariable::Variable(var))
            }
            TokenKind::StringLit(s) => {
                self.advance();
                Ok(ValueOrVariable::Literal(Literal::String(s)))
            }
            TokenKind::NumberInt(n) => {
                self.advance();
                Ok(ValueOrVariable::Literal(Literal::Integer(n)))
            }
            TokenKind::NumberFloat(f) => {
                self.advance();
                Ok(ValueOrVariable::Literal(Literal::Float(f)))
            }
            TokenKind::BooleanLit(b) => {
                self.advance();
                Ok(ValueOrVariable::Literal(Literal::Boolean(b)))
            }
            _ => Err(QueryError::parser(
                self.position(),
                format!("expected value or variable, got {:?}", self.peek()),
            )),
        }
    }

    fn parse_string_lit(&mut self) -> Result<String, QueryError> {
        match self.peek().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                Ok(s)
            }
            _ => Err(QueryError::parser(
                self.position(),
                format!("expected string literal, got {:?}", self.peek()),
            )),
        }
    }

    fn parse_identifier(&mut self) -> Result<String, QueryError> {
        match self.peek().clone() {
            TokenKind::Identifier(s) => {
                self.advance();
                Ok(s)
            }
            _ => Err(QueryError::parser(
                self.position(),
                format!("expected identifier, got {:?}", self.peek()),
            )),
        }
    }

    fn parse_usize(&mut self) -> Result<usize, QueryError> {
        match self.peek() {
            TokenKind::NumberInt(n) => {
                let n = *n;
                self.advance();
                if n < 0 {
                    return Err(QueryError::parser(
                        self.position(),
                        "expected non-negative integer",
                    ));
                }
                Ok(n as usize)
            }
            _ => Err(QueryError::parser(
                self.position(),
                format!("expected integer, got {:?}", self.peek()),
            )),
        }
    }
}

/// Suggest a trailing-clause keyword if the unexpected token is a
/// near-miss for one of `ORDER` / `LIMIT` / `OFFSET` / `DISTINCT`. The
/// lexer turns these typos into `Ident`s; we only consider that case.
/// Returns the suggested keyword (uppercase) or `None` if the token
/// isn't an identifier or none of the keywords are within Levenshtein
/// distance 2.
fn trailing_keyword_typo_hint(tok: &TokenKind) -> Option<&'static str> {
    let candidate = match tok {
        TokenKind::Identifier(s) => s.to_ascii_uppercase(),
        _ => return None,
    };
    const KEYWORDS: &[&str] = &["ORDER", "LIMIT", "OFFSET", "DISTINCT"];
    let max_dist = match candidate.len() {
        0..=4 => 1,
        _ => 2,
    };
    KEYWORDS
        .iter()
        .copied()
        .filter_map(|kw| {
            let d = levenshtein(&candidate, kw);
            if d == 0 || d > max_dist {
                None
            } else {
                Some((d, kw))
            }
        })
        .min_by_key(|(d, _)| *d)
        .map(|(_, kw)| kw)
}

/// Plain Levenshtein distance — only used for typo hints in parser
/// errors, so the O(m·n) cost on tiny strings is fine.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let (m, n) = (a_chars.len(), b_chars.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::lexer::tokenize;

    fn parse_str(input: &str) -> Result<Program, QueryError> {
        let tokens = tokenize(input)?;
        parse(tokens)
    }

    fn patterns_of(q: &Query) -> Vec<&Pattern> {
        q.body.patterns().collect()
    }

    #[test]
    fn simple_match_query() {
        let prog = parse_str(r#"MATCH ?x { name: ?n }"#).unwrap();
        assert!(prog.definitions.is_empty());
        let pats = patterns_of(&prog.query);
        assert_eq!(pats.len(), 1);
        assert!(pats[0].class.is_none());
        assert!(!pats[0].negated);
    }

    #[test]
    fn using_namespace_parses() {
        let prog = parse_str(
            r#"USING NAMESPACE "urn:eigenius:core:", "urn:ex:" MATCH Widget(?w) { name: ?n }"#,
        )
        .unwrap();
        assert_eq!(
            prog.query.body.using_namespaces,
            vec!["urn:eigenius:core:".to_string(), "urn:ex:".to_string()]
        );
        // Plain USING and USING NAMESPACE are distinct lists.
        assert!(prog.query.body.using.is_empty());
    }

    #[test]
    fn typed_pattern() {
        let prog =
            parse_str(r#"USING "urn:eigenius:core:Class" MATCH Class(?c) { short_name: ?name }"#)
                .unwrap();
        assert_eq!(prog.query.body.using.len(), 1);
        let pats = patterns_of(&prog.query);
        assert!(pats[0].class.is_some());
        assert_eq!(pats[0].properties.len(), 1);
    }

    #[test]
    fn full_iri_pattern() {
        let prog = parse_str(
            r#"MATCH "urn:eigenius:core:Class"(?c) { "urn:eigenius:core:short_name": ?name }"#,
        )
        .unwrap();
        let pats = patterns_of(&prog.query);
        let pat = pats[0];
        assert!(matches!(pat.class, Some(Name::FullIri(_))));
        assert!(matches!(pat.properties[0].property, Name::FullIri(_)));
    }

    #[test]
    fn with_return() {
        let prog = parse_str(
            r#"
            USING "urn:eigenius:core:Class"
            MATCH Class(?c) { short_name: ?name }
            RETURN Class { short_name: ?name }
            "#,
        )
        .unwrap();
        assert_eq!(prog.query.result_classes.len(), 1);
        assert_eq!(prog.query.result.len(), 1);
    }

    #[test]
    fn where_clause() {
        let prog = parse_str(
            r#"
            MATCH ?x { name: ?n, age: ?a }
            WHERE ?a > 18 AND ?n LIKE "A%"
            "#,
        )
        .unwrap();
        assert_eq!(prog.query.body.conditions.len(), 1); // single AND expression
        assert!(matches!(
            &prog.query.body.conditions[0],
            Expression::Binary {
                op: BinaryOp::And,
                ..
            }
        ));
    }

    #[test]
    fn not_exists() {
        let prog = parse_str(
            r#"
            MATCH ?x { name: ?n, domain: ?d }
            WHERE NOT EXISTS(?d)
            "#,
        )
        .unwrap();
        assert!(matches!(
            &prog.query.body.conditions[0],
            Expression::NotExists(_)
        ));
    }

    #[test]
    fn aggregation_with_group_by() {
        let prog = parse_str(
            r#"
            MATCH ?x { breed: ?b }
            GROUP BY ?b
            RETURN [] { breed: ?b, count: COUNT(?x) }
            "#,
        )
        .unwrap();
        assert_eq!(prog.query.group_by.len(), 1);
        assert_eq!(prog.query.result.len(), 2);
    }

    #[test]
    fn order_by_limit_offset_distinct() {
        let prog = parse_str(
            r#"
            MATCH ?x { name: ?n }
            RETURN [] { name: ?n }
            ORDER BY ?n DESC
            LIMIT 10
            OFFSET 5
            DISTINCT
            "#,
        )
        .unwrap();
        assert_eq!(prog.query.order_by.len(), 1);
        assert_eq!(prog.query.order_by[0].direction, SortDirection::Desc);
        assert_eq!(prog.query.limit, Some(10));
        assert_eq!(prog.query.offset, Some(5));
        assert!(prog.query.distinct);
    }

    #[test]
    fn define_rule() {
        let prog = parse_str(
            r#"
            DEFINE Ancestor(?x, ?z) FROM
                MATCH ?x { reports_to: ?z }
            DEFINE Ancestor(?x, ?z) FROM
                MATCH ?x { reports_to: ?y },
                Ancestor(?y) { }
            MATCH Ancestor(?a) {}
            RETURN [] { ancestor: ?a }
            "#,
        )
        .unwrap();
        assert_eq!(prog.definitions.len(), 2);
        assert_eq!(prog.definitions[0].name, "Ancestor");
        assert_eq!(prog.definitions[0].variables.len(), 2);
    }

    #[test]
    fn negated_pattern() {
        let prog = parse_str(
            r#"
            MATCH ?x { name: ?n },
            NOT ?parent { offspring: ?x }
            "#,
        )
        .unwrap();
        let pats = patterns_of(&prog.query);
        assert!(!pats[0].negated);
        assert!(pats[1].negated);
    }

    #[test]
    fn dot_path() {
        let prog = parse_str(
            r#"
            MATCH ?p { name: ?n }
            RETURN [] { city: ?p.address.city }
            "#,
        )
        .unwrap();
        assert!(matches!(
            &prog.query.result[0].expression,
            Expression::DotPath { segments, .. } if segments == &["address", "city"]
        ));
    }

    #[test]
    fn multiple_patterns_join() {
        let prog = parse_str(
            r#"
            MATCH ?p { pet: ?d },
            ?d { name: ?dog_name }
            "#,
        )
        .unwrap();
        assert_eq!(patterns_of(&prog.query).len(), 2);
    }

    #[test]
    fn empty_return_classes() {
        let prog = parse_str(r#"MATCH ?x {} RETURN [] { name: ?x }"#).unwrap();
        assert!(prog.query.result_classes.is_empty());
    }

    #[test]
    fn function_calls() {
        let prog = parse_str(
            r#"
            MATCH ?x { name: ?n }
            WHERE LENGTH(?n) > 3 AND CONTAINS([1, 2, 3], 2)
            "#,
        )
        .unwrap();
        assert!(!prog.query.body.conditions.is_empty());
    }

    #[test]
    fn arithmetic_precedence() {
        let prog = parse_str(r#"MATCH ?x {} WHERE ?x = 1 + 2 * 3"#).unwrap();
        // Should parse as ?x = (1 + (2 * 3))
        let cond = &prog.query.body.conditions[0];
        assert!(matches!(
            cond,
            Expression::Binary {
                op: BinaryOp::Eq,
                ..
            }
        ));
    }

    #[test]
    fn string_concat() {
        let prog = parse_str(r#"MATCH ?x {} RETURN [] { full: ?x || " suffix" }"#).unwrap();
        assert!(matches!(
            &prog.query.result[0].expression,
            Expression::Binary {
                op: BinaryOp::StringConcat,
                ..
            }
        ));
    }

    // -----------------------------------------------------------------
    // #10 — FIBER clause + USING INSTITUTION (D2 §3.3.1, §3.5)
    // -----------------------------------------------------------------

    #[test]
    fn fiber_with_using_institution_alias() {
        let prog = parse_str(
            r#"
            USING INSTITUTION "urn:eigenius:test:wasm:ordering" AS ord
            MATCH ?m { latest_delta: ?d }
            FIBER ord:ConvergenceQuery { tolerance: 0.01, latest_delta: ?d } AS ?conv
            MATCH ?conv { "urn:eigenius:test:wasm:converged": ?c }
            WHERE ?c = true
            "#,
        )
        .unwrap();

        // USING INSTITUTION alias captured separately from USING class imports.
        assert_eq!(prog.query.body.using.len(), 0);
        assert_eq!(prog.query.body.using_institutions.len(), 1);
        assert_eq!(prog.query.body.using_institutions[0].alias, "ord");
        assert_eq!(
            prog.query.body.using_institutions[0].iri.as_str(),
            "urn:eigenius:test:wasm:ordering"
        );

        // Clauses preserved in textual order: Match, Fiber, Match.
        let clauses = &prog.query.body.clauses;
        assert_eq!(clauses.len(), 3);
        assert!(matches!(clauses[0], Clause::Pattern(_)));
        match &clauses[1] {
            Clause::Fiber(fc) => {
                assert!(matches!(fc.institution, Name::ShortName(ref s) if s == "ord"));
                assert!(
                    matches!(fc.query_class, Name::ShortName(ref s) if s == "ConvergenceQuery")
                );
                assert_eq!(fc.params.len(), 2);
                assert!(matches!(fc.params[0].name, Name::ShortName(ref s) if s == "tolerance"));
                assert_eq!(fc.binding.name, "conv");
            }
            _ => panic!("expected Fiber clause at index 1"),
        }
        assert!(matches!(clauses[2], Clause::Pattern(_)));
        assert!(prog.query.body.has_fiber());
    }

    #[test]
    fn fiber_with_into_clause_parses() {
        let prog = parse_str(
            r#"
            USING INSTITUTION "urn:eigenius:test:wasm:ordering" AS ord
            MATCH ?m { latest_delta: ?d }
            FIBER ord:ConvergenceQuery { tolerance: 0.01, latest_delta: ?d } AS ?conv
                INTO "urn:eigenius:test:wasm:my_conv"
            "#,
        )
        .unwrap();
        let clauses = &prog.query.body.clauses;
        match &clauses[1] {
            Clause::Fiber(fc) => {
                let into = fc.into.as_ref().expect("INTO clause should be parsed");
                assert_eq!(into.as_str(), "urn:eigenius:test:wasm:my_conv");
            }
            _ => panic!("expected Fiber clause at index 1"),
        }
    }

    #[test]
    fn fiber_without_into_clause_has_none() {
        // Sanity check: FIBER without INTO leaves the field as None
        // (back-compat with all pre-Phase 19i queries).
        let prog = parse_str(
            r#"
            MATCH ?m { latest_delta: ?d }
            FIBER "urn:eigenius:test:wasm:ordering":ConvergenceQuery
                { tolerance: 0.01, latest_delta: ?d } AS ?conv
            "#,
        )
        .unwrap();
        let clauses = &prog.query.body.clauses;
        match &clauses[1] {
            Clause::Fiber(fc) => assert!(fc.into.is_none()),
            _ => panic!("expected Fiber clause"),
        }
    }

    #[test]
    fn fiber_into_rejects_non_string_argument() {
        let err = parse_str(
            r#"
            MATCH ?m { latest_delta: ?d }
            FIBER "urn:eigenius:test:wasm:ordering":ConvergenceQuery
                { tolerance: 0.01, latest_delta: ?d } AS ?conv INTO ?out
            "#,
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("expected quoted IRI string"),
            "expected `INTO` to require a quoted IRI string, got: {msg}"
        );
    }

    #[test]
    fn fiber_with_inline_iri() {
        let prog = parse_str(
            r#"
            MATCH ?m { latest_delta: ?d }
            FIBER "urn:eigenius:test:wasm:ordering":ConvergenceQuery
                { tolerance: 0.01, latest_delta: ?d } AS ?conv
            "#,
        )
        .unwrap();

        let fc = match &prog.query.body.clauses[1] {
            Clause::Fiber(fc) => fc,
            _ => panic!("expected Fiber clause"),
        };
        match &fc.institution {
            Name::FullIri(iri) => {
                assert_eq!(iri.as_str(), "urn:eigenius:test:wasm:ordering")
            }
            other => panic!("expected FullIri institution, got {other:?}"),
        }
    }

    #[test]
    fn fiber_with_mixed_short_name_and_full_iri_params() {
        let prog = parse_str(
            r#"
            USING INSTITUTION "urn:eigenius:test:wasm:ordering" AS ord
            MATCH ?m { latest_delta: ?d }
            FIBER ord:ConvergenceQuery {
                tolerance: 0.01,
                "urn:example:client:correlation_id": "abc"
            } AS ?conv
            "#,
        )
        .unwrap();

        let fc = match &prog.query.body.clauses[1] {
            Clause::Fiber(fc) => fc,
            _ => panic!("expected Fiber clause"),
        };
        assert_eq!(fc.params.len(), 2);
        assert!(matches!(fc.params[0].name, Name::ShortName(ref s) if s == "tolerance"));
        assert!(matches!(
            fc.params[1].name,
            Name::FullIri(ref i) if i.as_str() == "urn:example:client:correlation_id"
        ));
    }

    #[test]
    fn using_and_using_institution_coexist() {
        let prog = parse_str(
            r#"
            USING "urn:eigenius:test:wasm:Refinement"
            USING INSTITUTION "urn:eigenius:test:wasm:ordering" AS ord
            MATCH Refinement(?m) { latest_delta: ?d }
            FIBER ord:ConvergenceQuery { tolerance: 0.01, latest_delta: ?d } AS ?conv
            "#,
        )
        .unwrap();

        assert_eq!(prog.query.body.using.len(), 1);
        assert_eq!(prog.query.body.using_institutions.len(), 1);
    }

    #[test]
    fn fiber_alone_without_match_errors() {
        // A query that has only a FIBER clause and no MATCH should still
        // parse — both kinds are clauses. But a clause list that starts
        // without any MATCH/FIBER errors.
        let err = parse_str(r#"WHERE ?x = 1"#);
        assert!(err.is_err());
    }

    // --- eigenius#27: trailing junk after RETURN ---

    #[test]
    fn trailing_typo_after_return_errors_with_hint() {
        let err = parse_str(
            r#"USING "urn:eigenius:core:Class"
            MATCH Class(?c) { short_name: ?n }
            RETURN [] { name: ?n }
            ORDR BY ?n"#,
        )
        .unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("ORDR"),
            "expected the offending token in the error: {msg}"
        );
        assert!(
            msg.contains("ORDER"),
            "expected the typo hint to mention ORDER: {msg}"
        );
    }

    #[test]
    fn trailing_limit_typo_errors_with_hint() {
        let err = parse_str(
            r#"USING "urn:eigenius:core:Class"
            MATCH Class(?c) { short_name: ?n }
            RETURN [] { name: ?n }
            LIMT 5"#,
        )
        .unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("LIMT"));
        assert!(msg.contains("LIMIT"));
    }

    #[test]
    fn trailing_garbage_after_return_errors() {
        let err = parse_str(
            r#"USING "urn:eigenius:core:Class"
            MATCH Class(?c) { short_name: ?n }
            RETURN [] { name: ?n }
            ZZZZZZ"#,
        )
        .unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("ZZZZZZ"),
            "expected the unrecognized token to surface verbatim: {msg}"
        );
    }

    #[test]
    fn legitimate_trailing_clauses_still_parse() {
        // Sanity: the EOF check shouldn't break valid trailing clauses.
        parse_str(
            r#"USING "urn:eigenius:core:Class"
            MATCH Class(?c) { short_name: ?n }
            RETURN [] { name: ?n }
            ORDER BY ?n
            LIMIT 5"#,
        )
        .unwrap();
    }

    // ─── D43 §3.3 — similarity operator parser tests ───────────────────

    fn first_where(prog: &Program) -> &Expression {
        prog.query
            .body
            .conditions
            .first()
            .expect("expected at least one WHERE condition")
    }

    #[test]
    fn similarity_op_parses_bare() {
        let prog = parse_str(
            r#"
            MATCH ?d { description: ?desc }
            WHERE ?desc ~ "concurrent commit recovery"
            "#,
        )
        .unwrap();
        match first_where(&prog) {
            Expression::Similarity {
                property,
                query,
                hints,
            } => {
                assert_eq!(property.name, "desc");
                assert!(matches!(
                    **query,
                    Expression::Literal(Literal::String(ref s)) if s == "concurrent commit recovery"
                ));
                assert_eq!(hints, &HintSet::default());
            }
            other => panic!("expected Similarity, got {other:?}"),
        }
    }

    #[test]
    fn similarity_op_disjunction_composes_with_or() {
        let prog = parse_str(
            r#"
            MATCH ?d { title: ?t, body: ?b }
            WHERE ?t ~ "alpha" OR ?b ~ "beta"
            "#,
        )
        .unwrap();
        match first_where(&prog) {
            Expression::Binary {
                op: BinaryOp::Or,
                left,
                right,
            } => {
                assert!(matches!(**left, Expression::Similarity { .. }));
                assert!(matches!(**right, Expression::Similarity { .. }));
            }
            other => panic!("expected OR, got {other:?}"),
        }
    }

    #[test]
    fn similarity_hint_via_text() {
        let prog = parse_str(
            r#"
            MATCH ?s { name: ?n }
            WHERE ?n ~ "Foo::bar" { via: text }
            "#,
        )
        .unwrap();
        let Expression::Similarity { hints, .. } = first_where(&prog) else {
            panic!("expected Similarity");
        };
        assert_eq!(hints.via, Some(Via::Text));
        assert!(hints.model.is_none());
        assert!(hints.k.is_none());
        assert!(hints.limit.is_none());
    }

    #[test]
    fn similarity_hint_full_set() {
        let prog = parse_str(
            r#"
            MATCH ?d { description: ?desc }
            WHERE ?desc ~ "kernel chain" { via: hybrid, model: "urn:eigenius:embed:m1", k: 30, limit: 50 }
            "#,
        )
        .unwrap();
        let Expression::Similarity { hints, .. } = first_where(&prog) else {
            panic!("expected Similarity");
        };
        assert_eq!(hints.via, Some(Via::Hybrid));
        assert_eq!(hints.model.as_deref(), Some("urn:eigenius:embed:m1"));
        assert_eq!(hints.k, Some(30));
        assert_eq!(hints.limit, Some(50));
    }

    #[test]
    fn similarity_lhs_must_be_variable() {
        let err = parse_str(
            r#"
            MATCH ?d { description: ?desc }
            WHERE ("foo" || ?desc) ~ "query"
            "#,
        )
        .expect_err("non-variable LHS should fail at parse");
        let msg = format!("{err}");
        assert!(msg.contains("similarity LHS"), "unexpected message: {msg}");
    }

    #[test]
    fn similarity_unknown_hint_key_rejected() {
        let err = parse_str(
            r#"
            MATCH ?d { description: ?desc }
            WHERE ?desc ~ "query" { weights: 1 }
            "#,
        )
        .expect_err("unknown hint key should fail at parse");
        let msg = format!("{err}");
        assert!(
            msg.contains("unknown similarity hint"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn similarity_bad_via_value_rejected() {
        let err = parse_str(
            r#"
            MATCH ?d { description: ?desc }
            WHERE ?desc ~ "query" { via: graph }
            "#,
        )
        .expect_err("bad via value should fail at parse");
        let msg = format!("{err}");
        assert!(msg.contains("via"), "unexpected message: {msg}");
    }

    #[test]
    fn top_n_parses_into_query_field() {
        let prog = parse_str(
            r#"
            MATCH ?d { description: ?desc }
            WHERE ?desc ~ "q"
            TOP 20
            "#,
        )
        .unwrap();
        assert_eq!(prog.query.top, Some(20));
        assert!(prog.query.limit.is_none());
    }
}
