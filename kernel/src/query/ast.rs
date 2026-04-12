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

/// The USING + MATCH + WHERE portion, shared by DEFINE and Query.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchPart {
    pub using: Vec<Iri>,
    pub patterns: Vec<Pattern>,
    pub conditions: Vec<Expression>,
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
