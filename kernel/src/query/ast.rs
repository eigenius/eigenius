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
//!
//! # The `S` parameter (D92)
//!
//! Every type that holds a reference to declared vocabulary is generic in how that
//! reference is spelled:
//!
//! - `Program<Parsed>` is what the parser builds. A [`Name`] is a short name or a full
//!   IRI, as written in the query text, and nothing has checked that it names anything.
//! - `Program<Resolved>` is what resolution produces. Every reference has been looked up
//!   against the chain, or the program does not exist.
//!
//! **The point is not that the unresolved state is checked. It is that a missed position
//! does not compile.** Resolution is a total function between the two instantiations, so
//! to produce the second it must produce a resolved reference for *every* parameterised
//! position — there is no arm it can skip.
//!
//! That is not hypothetical. Before D92 the resolution pass matched `Clause::Pattern` and
//! let `Clause::Fiber` fall through an `else { continue }`, so a dot-path inside a FIBER
//! param kept its short names, type-checked with zero errors, and failed at evaluation
//! with a message naming the resolution pass. Under the parameterised AST that `continue`
//! has no resolved reference to put in the clause it skipped.
//!
//! `S` defaults to [`Parsed`] so that code working on parsed programs reads unchanged.
//!
//! **A [`ColumnLabel`] is deliberately not parameterised.** It is a name the query author
//! invents for an output column, not a reference to anything — see its own documentation.

use crate::ontology::iri::Iri;

/// Which stage of the pipeline an AST belongs to, and therefore how the references it
/// holds are spelled (D92).
///
/// **Two associated types, not one, because the positions do not resolve alike.** A
/// pattern class may name a chain class *or* a `DEFINE` relation, so it resolves to a
/// [`ClassRef`]; every other reference resolves to the `Iri` of a declared resource. A
/// single parameter would force one resolved type on both, and that type would have to be
/// a sum — which would let a property key hold a relation: representable and invalid,
/// which is the thing this design exists to remove.
///
/// The associated types carry the `Debug + Clone + PartialEq` bounds so the AST's derives
/// hold for every stage.
pub trait Stage {
    /// A `MATCH` pattern's class: a chain class or a `DEFINE` relation.
    type PatternClass: std::fmt::Debug + Clone + PartialEq;
    /// Every other reference to declared vocabulary — a property key, a dot-path segment,
    /// a `RETURN` result class, a FIBER institution, query class, param or comorphism.
    ///
    /// They share a type because they resolve alike, to the `Iri` of a declared resource.
    /// Distinguishing them further — a `PropertyIri` that cannot hold a class — wants a
    /// newtype per metaclass, which is a larger change than D92 and buys nothing this one
    /// needs.
    type Ref: std::fmt::Debug + Clone + PartialEq;
}

/// The stage the parser produces: references as written, none of them checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parsed;

/// The stage resolution produces: every reference looked up against the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved;

impl Stage for Parsed {
    type PatternClass = Name;
    type Ref = Name;
}

impl Stage for Resolved {
    type PatternClass = ClassRef;
    type Ref = Iri;
}

/// What a resolved `MATCH` pattern class names.
///
/// `MATCH Dog(?d)` names a chain class; `MATCH ancestor(?x, ?y)` names a rule defined by
/// this program. `check_match_part` carried that distinction as an exemption list inside
/// one checking function; here it is a variant every consumer has to handle.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassRef {
    /// A `core:Class` on the chain.
    Chain(Iri),
    /// A `DEFINE` relation, by [`RelationId`].
    ///
    /// An id rather than a name, because a name leaves a lookup at every use — the
    /// second-resolution pattern D92 removes. An id rather than the `RuleDefinition`
    /// itself, because `stratify` rejects only *negation* cycles: ordinary positive
    /// recursion is legal Datalog, and a `ClassRef` embedding its own definition would be
    /// an infinite value for exactly the rules the feature exists for. And an id per
    /// RELATION rather than per definition — see [`relation_ids`].
    Relation(RelationId),
}

/// Identifies one derived relation within a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelationId(pub usize);

/// The id of each distinct relation, assigned by first appearance.
///
/// **A relation may be defined by several rules**, and the id identifies the RELATION,
/// not the rule. `DEFINE Reach(?t) FROM …` followed by `DEFINE Reach(?n) FROM MATCH
/// Reach(?m) …` is one relation with a base case and a recursive case; keying derived
/// facts per definition splits it into two, and the closure then never accumulates.
pub fn relation_ids<S: Stage>(
    definitions: &[RuleDefinition<S>],
) -> std::collections::BTreeMap<String, RelationId> {
    let mut out = std::collections::BTreeMap::new();
    let mut next = 0usize;
    for d in definitions {
        if !out.contains_key(&d.name) {
            out.insert(d.name.clone(), RelationId(next));
            next += 1;
        }
    }
    out
}

/// A complete EigenQL program: zero or more rule definitions + a query.
#[derive(Debug, Clone, PartialEq)]
pub struct Program<S: Stage = Parsed> {
    pub definitions: Vec<RuleDefinition<S>>,
    pub query: Query<S>,
}

/// A DEFINE clause: names a derived relation.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleDefinition<S: Stage = Parsed> {
    pub name: String,
    pub variables: Vec<Variable>,
    pub body: MatchPart<S>,
}

/// The USING + MATCH + (optional FIBER) + WHERE portion, shared by DEFINE and Query.
///
/// Clauses preserve textual order so FIBER dispatches can consume
/// bindings from preceding MATCH/FIBER clauses and subsequent patterns
/// can consume bindings produced by FIBER — see D2 §3.5, §6.12.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchPart<S: Stage = Parsed> {
    pub using: Vec<Iri>,
    pub using_institutions: Vec<InstitutionAlias>,
    /// `USING NAMESPACE "<prefix>"` declarations — the vocabulary namespaces
    /// (verbatim IRI prefixes) that bare short names in this part's
    /// classes/properties/query-classes resolve within. See
    /// [`crate::query::resolve`].
    pub using_namespaces: Vec<String>,
    pub clauses: Vec<Clause<S>>,
    pub conditions: Vec<Expression<S>>,
}

impl<S: Stage> MatchPart<S> {
    /// Iterate over just the MATCH patterns, ignoring FIBER clauses.
    /// Adapter for callers that predate FIBER support (DEFINE bodies,
    /// stratification, etc.). Use `.clauses` directly when FIBER matters.
    pub fn patterns(&self) -> impl Iterator<Item = &Pattern<S>> {
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
pub enum Clause<S: Stage = Parsed> {
    /// One structural pattern. Multiple consecutive Pattern clauses
    /// correspond to comma-separated patterns in one MATCH clause, but
    /// separating them into multiple MATCH clauses is equivalent
    /// (equi-join over shared variables).
    Pattern(Pattern<S>),
    /// A FIBER dispatch to a registered institution. See D2 §3.5.
    Fiber(FiberClause<S>),
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
pub struct FiberClause<S: Stage = Parsed> {
    /// Institution reference — either a USING INSTITUTION alias
    /// (ShortName) or an inline full IRI (FullIri).
    pub institution: S::Ref,
    /// Query class name (must appear in the institution's declared
    /// query_types). Short name or full IRI.
    pub query_class: S::Ref,
    /// Parameter bindings passed as properties on the query resource.
    pub params: Vec<ParamBinding<S>>,
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
pub struct ParamBinding<S: Stage = Parsed> {
    pub name: S::Ref,
    pub value: ParamValue<S>,
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
pub enum ParamValue<S: Stage = Parsed> {
    Expression(Expression<S>),
    Comorphism { name: S::Ref, source: Expression<S> },
}

/// A complete query with all clauses.
#[derive(Debug, Clone, PartialEq)]
pub struct Query<S: Stage = Parsed> {
    pub body: MatchPart<S>,
    pub group_by: Vec<Expression<S>>,
    pub result_classes: Vec<S::Ref>,
    pub result: Vec<ReturnItem<S>>,
    pub order_by: Vec<OrderItem<S>>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub distinct: bool,
    /// D43 §3.3 — ranked truncation. `TOP N` is the user-facing
    /// surface for "give me the N most relevant rows." When the
    /// query contains a similarity operator, ordering is the fused
    /// similarity score. Without `~`, `TOP N` parses and is then
    /// rejected at *typecheck* (`top_without_similarity`); the parser
    /// enforces only position — `TOP` sits between `LIMIT` and
    /// `OFFSET`. Use `LIMIT` for un-ranked truncation. Mutually
    /// exclusive with `LIMIT` (`top_with_limit`) and with `ORDER BY`
    /// (`top_with_order_by`) in the same query.
    pub top: Option<usize>,
}

/// A MATCH pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern<S: Stage = Parsed> {
    pub subject: Variable,
    pub class: Option<S::PatternClass>,
    pub properties: Vec<PropertyPattern<S>>,
    pub negated: bool,
}

/// A property binding within a pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyPattern<S: Stage = Parsed> {
    pub property: S::Ref,
    pub object: ValueOrVariable,
}

/// A name: either a bare shortname or a full IRI.
#[derive(Debug, Clone, PartialEq)]
pub enum Name {
    ShortName(String),
    FullIri(Iri),
}

/// The name a `RETURN` item gives one output column.
///
/// **Not a [`Name`], though it is spelled like one.** A `Name` is a reference to
/// something the chain already declares, which resolution must turn into an `Iri` or
/// reject. A column label is invented by the query author on the spot: `RETURN [] { total:
/// SUM(?x) }` does not assert that anything called `total` is declared anywhere, and there
/// is nothing to resolve it against.
///
/// The two shared a type until D92, which is how the resolution discipline for one came to
/// read as the discipline for the other — and why six of the eight `Name` positions were
/// not following it. Splitting them makes "is this a reference or a label?" a question the
/// compiler asks at every position rather than one a contributor has to think to ask.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnLabel {
    /// A bare name. The result document synthesises a per-query property IRI for it,
    /// `{fingerprint}:row:{name}` — see `QueryFingerprint::row_property_iri`.
    Synthesised(String),
    /// An explicit IRI, used verbatim as the column's property.
    Explicit(Iri),
}

impl ColumnLabel {
    /// The bare text of the label, for a diagnostic or a `short_name`. An explicit IRI
    /// contributes its last colon-separated segment.
    pub fn text(&self) -> String {
        match self {
            ColumnLabel::Synthesised(s) => s.clone(),
            ColumnLabel::Explicit(iri) => iri
                .as_str()
                .rsplit(':')
                .next()
                .unwrap_or(iri.as_str())
                .to_string(),
        }
    }
}

impl std::fmt::Display for ColumnLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColumnLabel::Synthesised(s) => f.write_str(s),
            ColumnLabel::Explicit(i) => f.write_str(i.as_str()),
        }
    }
}

impl Name {
    /// The bare text of the name, for a diagnostic or a `short_name`. A full IRI
    /// contributes its last colon-separated segment.
    pub fn text(&self) -> String {
        match self {
            Name::ShortName(s) => s.clone(),
            Name::FullIri(iri) => iri
                .as_str()
                .rsplit(':')
                .next()
                .unwrap_or(iri.as_str())
                .to_string(),
        }
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Name::ShortName(s) => f.write_str(s),
            Name::FullIri(i) => f.write_str(i.as_str()),
        }
    }
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
pub struct ReturnItem<S: Stage = Parsed> {
    pub name: ColumnLabel,
    pub expression: Expression<S>,
}

/// An ORDER BY item.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderItem<S: Stage = Parsed> {
    pub expression: Expression<S>,
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
pub enum Expression<S: Stage = Parsed> {
    Literal(Literal),
    Variable(Variable),
    Binary {
        op: BinaryOp,
        left: Box<Expression<S>>,
        right: Box<Expression<S>>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expression<S>>,
    },
    /// Postfix Verdict projection (D2 v2 §3.7 / §3.8): `?v HOLDS`,
    /// `?v FAILS`, `?v UNDECIDABLE`. The operand must evaluate to a
    /// `Verdict`-typed resource carrying `ctor_name`; the result is a
    /// `Boolean` true iff the constructor matches.
    VerdictPredicate {
        kind: VerdictPredicate,
        operand: Box<Expression<S>>,
    },
    /// `NOT EXISTS(e)` — true when `e` has no value.
    ///
    /// Held as an expression, not a bare `Variable`, so it reaches a dot-path:
    /// `NOT EXISTS(?n.title)` asks whether the resource carries the property, which is the
    /// question this was always meant to answer. Over a bare variable it asks whether the
    /// variable is bound, which under a strictly conjunctive `MATCH` is always true — so
    /// that form matched nothing and was dead (eigenius#124).
    NotExists(Box<Expression<S>>),
    FunctionCall {
        name: String,
        args: Vec<Expression<S>>,
    },
    Aggregate {
        op: AggregateOp,
        arg: Box<Expression<S>>,
    },
    /// `?root.seg.seg` — property traversal from a bound resource.
    ///
    /// A segment is a reference of the same kind a `MATCH` brace key holds, because it
    /// names the same thing: a declared `core:Property`. In a parsed program that is a
    /// [`Name`] — a short name resolving against the root's class where the query states
    /// one and against the imported namespaces otherwise, or a full IRI naming the
    /// property outright where neither scope reaches it. In a resolved program it is the
    /// property it resolved to.
    DotPath {
        root: Variable,
        segments: Vec<S::Ref>,
    },
    Array(Vec<Expression<S>>),
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
        query: Box<Expression<S>>,
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
