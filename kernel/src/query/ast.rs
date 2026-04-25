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

//! AST types for EigenQL programs.
//!
//! Matches the grammar in design doc D2 §3 and §4.

use crate::ontology::iri::Iri;

/// A complete EigenQL program: zero or more rule definitions + a query.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub definitions: Vec<RuleDefinition>,
    pub query: Query,
}

/// A DEFINE clause: names a derived relation.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleDefinition {
    pub name: String,
    pub variables: Vec<Variable>,
    pub body: MatchPart,
}

/// The USING + MATCH + (optional FIBER) + WHERE portion, shared by DEFINE and Query.
///
/// Clauses preserve textual order so FIBER dispatches can consume
/// bindings from preceding MATCH/FIBER clauses and subsequent patterns
/// can consume bindings produced by FIBER — see D2 §3.5, §6.12.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchPart {
    pub using: Vec<Iri>,
    pub using_institutions: Vec<InstitutionAlias>,
    pub clauses: Vec<Clause>,
    pub conditions: Vec<Expression>,
}

impl MatchPart {
    /// Iterate over just the MATCH patterns, ignoring FIBER clauses.
    /// Adapter for callers that predate FIBER support (DEFINE bodies,
    /// stratification, etc.). Use `.clauses` directly when FIBER matters.
    pub fn patterns(&self) -> impl Iterator<Item = &Pattern> {
        self.clauses.iter().filter_map(|c| match c {
            Clause::Pattern(p) => Some(p),
            Clause::Fiber(_) => None,
        })
    }

    /// True if this MatchPart contains any FIBER clauses.
    pub fn has_fiber(&self) -> bool {
        self.clauses.iter().any(|c| matches!(c, Clause::Fiber(_)))
    }
}

/// A single clause inside a MatchPart.
#[derive(Debug, Clone, PartialEq)]
pub enum Clause {
    /// One structural pattern. Multiple consecutive Pattern clauses
    /// correspond to comma-separated patterns in one MATCH clause, but
    /// separating them into multiple MATCH clauses is equivalent
    /// (equi-join over shared variables).
    Pattern(Pattern),
    /// A FIBER dispatch to a registered institution. See D2 §3.5.
    Fiber(FiberClause),
}

/// `USING INSTITUTION "<iri>" AS <alias>` — binds a short name to an
/// institution IRI for use in subsequent FIBER clauses.
#[derive(Debug, Clone, PartialEq)]
pub struct InstitutionAlias {
    pub iri: Iri,
    pub alias: String,
}

/// A FIBER clause. Per D2 §3.5: dispatches to a registered
/// institution's fiber reasoner with a typed query resource built from
/// `params`, binds the response resource to `binding` so subsequent
/// MATCH clauses can decompose it.
#[derive(Debug, Clone, PartialEq)]
pub struct FiberClause {
    /// Institution reference — either a USING INSTITUTION alias
    /// (ShortName) or an inline full IRI (FullIri).
    pub institution: Name,
    /// Query class name (must appear in the institution's declared
    /// query_types). Short name or full IRI.
    pub query_class: Name,
    /// Parameter bindings passed as properties on the query resource.
    pub params: Vec<ParamBinding>,
    /// Variable the response resource is bound to.
    pub binding: Variable,
}

/// A single `name: expression` param inside a FIBER clause's braces.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamBinding {
    pub name: Name,
    pub expression: Expression,
}

/// A complete query with all clauses.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub body: MatchPart,
    pub group_by: Vec<Expression>,
    pub result_classes: Vec<Name>,
    pub result: Vec<ReturnItem>,
    pub order_by: Vec<OrderItem>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub distinct: bool,
}

/// A MATCH pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub subject: Variable,
    pub class: Option<Name>,
    pub properties: Vec<PropertyPattern>,
    pub negated: bool,
}

/// A property binding within a pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyPattern {
    pub property: Name,
    pub object: ValueOrVariable,
}

/// A name: either a bare shortname or a full IRI.
#[derive(Debug, Clone, PartialEq)]
pub enum Name {
    ShortName(String),
    FullIri(Iri),
}

/// A query variable (without the `?` prefix).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable {
    pub name: String,
}

impl Variable {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

/// Either a variable reference or a literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueOrVariable {
    Variable(Variable),
    Literal(Literal),
}

/// A literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
}

/// A RETURN item: maps a property name to an expression.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnItem {
    pub name: Name,
    pub expression: Expression,
}

/// An ORDER BY item.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderItem {
    pub expression: Expression,
    pub direction: SortDirection,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// An expression in WHERE, RETURN, GROUP BY, or ORDER BY.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Literal(Literal),
    Variable(Variable),
    Binary {
        op: BinaryOp,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expression>,
    },
    NotExists(Variable),
    FunctionCall {
        name: String,
        args: Vec<Expression>,
    },
    Aggregate {
        op: AggregateOp,
        arg: Box<Expression>,
    },
    DotPath {
        root: Variable,
        segments: Vec<String>,
    },
    Array(Vec<Expression>),
    Object(Vec<(Name, Expression)>),
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // Comparison
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    // String
    StringConcat,
    // Logical
    And,
    Or,
    // Collection/pattern
    In,
    NotIn,
    Like,
    NotLike,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Pos,
    Neg,
}

/// Aggregate operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateOp {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}
