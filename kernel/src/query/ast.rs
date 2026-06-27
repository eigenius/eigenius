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
    /// `USING NAMESPACE "<prefix>"` declarations — the vocabulary namespaces
    /// (verbatim IRI prefixes) that bare short names in this part's
    /// classes/properties/query-classes resolve within. See
    /// [`crate::query::resolve`].
    pub using_namespaces: Vec<String>,
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
    /// Optional `INTO "<iri>"` suffix (D14 §9.3 chain-reinsertion via
    /// EigenQL). When `Some`, the FIBER response is committed to the
    /// regular chain at the named IRI as part of the query's commit
    /// cycle, and the binding variable resolves to that IRI rather
    /// than to the transient query-overlay IRI. When `None`, the
    /// response stays in the per-query overlay and disappears at
    /// query end.
    pub into: Option<Iri>,
}

/// A single `name: <value>` param inside a FIBER clause's braces. The
/// value is either a plain expression or a comorphism coercion
/// (D2 v2 §3.5).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamBinding {
    pub name: Name,
    pub value: ParamValue,
}

/// Two shapes for a FIBER param value (D2 v2 §3.5 / §4):
///
/// - `Expression(e)` — the value is the result of evaluating `e`
///   against the current binding (literal, variable, scalar function
///   call, dot-path, …).
/// - `Comorphism { name, source }` — `name(source)` runs the named
///   comorphism's four-step pipeline (extract_typed → transformation
///   → reify) inline, and the reified target resource is used as the
///   param value.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Expression(Expression),
    Comorphism { name: Name, source: Expression },
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
    /// D43 §3.3 — ranked truncation. `TOP N` is the user-facing
    /// surface for "give me the N most relevant rows." When the
    /// query contains a similarity operator, ordering is the fused
    /// similarity score; without `~`, `TOP N` is rejected at parse
    /// (use `LIMIT` for un-ranked truncation). Mutually exclusive
    /// with `LIMIT` and with `ORDER BY` in the same query.
    pub top: Option<usize>,
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

/// Either a variable reference, a literal value, or an array pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueOrVariable {
    Variable(Variable),
    Literal(Literal),
    /// An array pattern (D59) — matches against an array-valued property,
    /// binding/iterating its elements.
    Array(ArrayPattern),
}

/// A pattern over an array-valued property (D59).
#[derive(Debug, Clone, PartialEq)]
pub enum ArrayPattern {
    /// `[]`, `[?a]`, `[?a, ?b]` — exactly N elements, bound positionally.
    Exact(Vec<Variable>),
    /// `[?a, ...]`, `[?a, ?b, ...]` — at least N elements; the first N bound
    /// positionally, the remainder unconstrained.
    AtLeast(Vec<Variable>),
    /// `[... ?e ...]` — iterate: one binding per array element.
    Each(Variable),
}

impl ArrayPattern {
    /// The variables this pattern binds (for bound-ness tracking).
    pub fn variables(&self) -> Vec<&Variable> {
        match self {
            ArrayPattern::Exact(vs) | ArrayPattern::AtLeast(vs) => vs.iter().collect(),
            ArrayPattern::Each(v) => vec![v],
        }
    }
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
    /// Postfix Verdict projection (D2 v2 §3.7 / §3.8): `?v HOLDS`,
    /// `?v FAILS`, `?v UNDECIDABLE`. The operand must evaluate to a
    /// `Verdict`-typed resource carrying `ctor_name`; the result is a
    /// `Boolean` true iff the constructor matches.
    VerdictPredicate {
        kind: VerdictPredicate,
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
    /// D43 §3.3 — similarity operator `?prop ~ "query" { hints }`.
    ///
    /// `property` is the property-bound LHS; `query` is the RHS
    /// expression (a literal string in v1, more general in later
    /// revisions); `hints` is the optional trailing-braces hint set
    /// (§3.4). Returns a boolean at the row level (the row passes the
    /// platform-chosen relevance threshold); the per-row relevance
    /// score it contributes is held by the evaluator's fusion table,
    /// not the AST.
    Similarity {
        property: Variable,
        query: Box<Expression>,
        hints: HintSet,
    },
}

/// D43 §3.4 — optional trailing-braces hints on the `~` operator.
///
/// All fields are `Option`; absence means "use the platform default."
/// Validated at typecheck (§4.4): unknown keys reject; `via`/`model`
/// combinations are checked for consistency; `k` and `limit` must be
/// positive integer literals.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HintSet {
    /// `via: text | vector | hybrid`.
    pub via: Option<Via>,
    /// `model: "<iri>"` — overrides the embedder for the vector path.
    /// Implicitly forces `via: vector` when set.
    pub model: Option<String>,
    /// `k: <int>` — RRF smoothing constant (default 60).
    pub k: Option<usize>,
    /// `limit: <int>` — probe-side candidate-set cap.
    pub limit: Option<usize>,
}

/// D43 §3.4 — strategy selector for the `~` operator's `via:` hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Text,
    Vector,
    Hybrid,
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

/// Postfix Verdict predicates (D2 v2 §3.7 / §3.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictPredicate {
    Holds,
    Fails,
    Undecidable,
}

impl VerdictPredicate {
    /// Constructor-name string the predicate matches against.
    pub fn ctor_name(self) -> &'static str {
        match self {
            VerdictPredicate::Holds => "Holds",
            VerdictPredicate::Fails => "Fails",
            VerdictPredicate::Undecidable => "Undecidable",
        }
    }
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
