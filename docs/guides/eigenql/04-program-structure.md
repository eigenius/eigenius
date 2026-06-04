# 4. Program structure

An EigenQL program has two parts: zero or more `DEFINE` rules, followed by a single top-level query. The grammar is authoritatively specified in [D2 §3](../../design/d2-eigenql-specification.md); this chapter focuses on semantics and the resulting [`Program`](../../../kernel/src/query/ast.rs) AST.

```
Program ::= Definition* Query
```

The entry points in [`kernel/src/query/parser.rs`](../../../kernel/src/query/parser.rs) are `parse_program()` (the full file), `parse_define()` (one rule), and `parse_query()` (the query).

## 4.1. The `Program` and `Query` AST

```rust
pub struct Program {
    pub definitions: Vec<RuleDefinition>,
    pub query: Query,
}

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
```

The query's evaluation order is: `body` → `group_by` → `result` → `distinct` → `order_by` → `offset` → `limit`. Each stage consumes the output of the previous one.

## 4.2. `USING` — class imports

```
UsingClause ::= 'USING' StringLit ( ',' StringLit )*
```

A `USING` clause imports one or more class IRIs. Inside the `MATCH` block, those classes can be referred to by their **short name** (the `short_name` property value) instead of the full IRI.

```eigenql
USING "urn:eigenius:core:Class", "urn:eigenius:example:Dog"

MATCH Class(?c) { ... }
MATCH Dog(?d) { ... }
```

Without the `USING`, you'd have to write `MATCH "urn:eigenius:core:Class"(?c) { ... }` explicitly.

**Type-check rule** ([type_check.rs](../../../kernel/src/query/type_check.rs)):

- Every `USING` IRI must resolve in the layer chain, and the resolved resource must have `is_a: Class`. Violations raise `using_unresolved` or `using_not_class`.

## 4.3. `USING INSTITUTION` — fiber aliases

```
UsingInstitution ::= 'USING' 'INSTITUTION' StringLit 'AS' ident
```

Declares a short alias for an institution's IRI, to be used in subsequent `FIBER` clauses in the same `MatchPart`.

```eigenql
USING INSTITUTION "urn:eigenius:institutions:docking" AS dock
USING INSTITUTION "urn:eigenius:institutions:assay" AS assay
```

Aliases are **scoped to their `MatchPart`** — a `DEFINE` body and the main query have separate scopes. Aliases must be unique within a scope.

The parsed AST:

```rust
pub struct InstitutionAlias {
    pub iri: Iri,
    pub alias: String,
}
```

**Type-check rules**:

- `duplicate_using_institution_alias` if an alias is reused
- `undeclared_institution_alias` if a `FIBER` clause uses an alias that wasn't declared

## 4.4. `DEFINE` — derived relations

```
Definition ::= 'DEFINE' ident '(' Variable (',' Variable)* ')' 'FROM' MatchPart
```

A `DEFINE` rule introduces a new named relation with a tuple of head variables. The body is a `MatchPart` (same shape as a query body, minus `FIBER`) that produces bindings; those bindings, projected onto the head variables, populate the relation.

```eigenql
DEFINE Ancestor(?x, ?z) FROM
    MATCH ?x { "urn:eigenius:test:reports_to": ?z }
```

`Ancestor` is now a relation with two arguments. Other rules and the main query can match against it:

```eigenql
Ancestor(?y) { "urn:eigenius:test:reports_to": ?z }
```

Multiple rules may define the same relation. Their bindings union. The ancestor example in [chapter 2 §2.4](02-quick-tour.md) uses two rules — a base case and a recursive step.

**Recursion** is allowed through positive dependencies only. If a `DEFINE` body negates a relation that transitively depends on the rule's head, **stratification fails**. See [chapter 10](10-stratification.md).

**Fiber restriction**: `DEFINE` bodies **cannot** contain `FIBER` clauses. The rule-fixpoint evaluator has no overlay context, so institution dispatch is disallowed by both the parser (`parse_match_part(allow_fiber=false)`) and the evaluator (defensive check in [`evaluate_match_part`](../../../kernel/src/query/evaluate.rs)).

## 4.5. `MATCH` — structural patterns

```
Clause ::= Pattern | FiberClause
MatchClause ::= 'MATCH' Pattern (',' Pattern)*
```

`MATCH` is the primary pattern clause. It's followed by one or more comma-separated `Pattern`s, each of which binds variables by unifying with resources in the layer.

```eigenql
MATCH Dog(?d) {
    breed: ?breed,
    name: ?name
}
```

The full treatment of patterns (subjects, class tags, property patterns, negation) is in [chapter 5](05-pattern-matching.md).

Multiple `MATCH` clauses compose by equi-join on shared variables:

```eigenql
MATCH Dog(?d) { owner: ?o }
MATCH Person(?o) { name: ?owner_name }
```

Here the second `MATCH` refines bindings from the first by requiring `?o` to also be a `Person` with a `name`.

## 4.6. `FIBER` — institution dispatch

```
FiberClause ::= 'FIBER' inst '.' QueryClass '{' Params '}' 'AS' Variable
```

A `FIBER` clause dispatches an `OnDemand` `QueryClass` to its institution and binds the response. Param values may use comorphism coercion (`comorphism_iri(source)`) to translate across an institution boundary inline.

```eigenql
FIBER assay:validate_prediction {
    candidate: dock_to_assay(?d)
} AS ?check
```

**What happens at evaluation** (D14 §9):

1. Resolve the QueryClass in the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs); confirm `OnDemand` is in its `dispatch_role` set.
2. Build an input resource of class `QueryClass.query_class` with each param's value filled in. Comorphism-coerced param values run the four-step extract → transform → reify pipeline (D14 §10.3).
3. Look up the QueryClass's `institution_ref` in the [`InstitutionRuntime`](../../../kernel/src/institution/runtime.rs) and call `Institution::query(query_handler, input, ctx)`.
4. Stamp the response with a deterministic transient IRI and attach it to the `FiberOverlay`.
5. Extend the binding with the AS-named variable → the response IRI (as a `Value::String`).

Subsequent `MATCH` clauses can pattern-match against the response through the overlay exactly as if it were a layer resource, scoped to this query only.

**Type-check rules** (D2 §5.8):

- `fiber_query_class_not_class` if the query class doesn't resolve to a `Class`
- `fiber_query_class_not_on_demand` if the QueryClass is declared but its `dispatch_role` set excludes `OnDemand`
- `fiber_institution_mismatch` if the QueryClass's `institution_ref` doesn't match the alias / inline IRI in the FIBER clause
- `fiber_param_short_name_unresolved` if a param's short name isn't a declared `requires` / `recommends` property of the query class
- `fiber_missing_required_param` if a `requires` property of the query class isn't supplied
- `comorphism_coercion_class_mismatch` if a comorphism coercion's `to_class` doesn't satisfy the FIBER input class for that property
- `comorphism_io_not_supported_in_v1` if a comorphism's transformation Component requires `IO` capability

Full FIBER semantics: [chapter 8](08-fiber-clauses.md).

## 4.7. `WHERE` — condition filter

```
WhereClause ::= 'WHERE' Expression
```

`WHERE` filters bindings by evaluating its expression against each one. A binding is retained if the expression evaluates to `Value::Boolean(true)`; dropped otherwise (including on type errors and `Undecidable` institution results).

```eigenql
WHERE ?age > 18 AND ?country = "DE"
```

The expression grammar is a full operator precedence hierarchy (see [chapter 7](07-expressions.md)). Aggregates are not permitted in `WHERE` — the type checker rejects them with `aggregate_in_where`.

**Decidable QueryClass predicates** (D14):

```eigenql
WHERE cap:within_tolerance(?delta, 0.1) HOLDS
```

Qualified-name function calls dispatch to `Institution::query` for the matching `Decidable` `QueryClass` and return a `Verdict`. The postfix `HOLDS` (or `FAILS` / `UNDECIDABLE`) projects the Verdict to a Boolean — only `Holds` survives the `... HOLDS` filter. A bare Verdict-typed expression in Boolean position is a static type error (`bare_verdict_in_boolean_position`); the conversion is always explicit.

## 4.8. `RETURN` — result shaping

```
ReturnClause ::= 'RETURN' '[' ClassList? ']' '{' ReturnItem (',' ReturnItem)* '}'
ReturnItem ::= Name ':' Expression
```

`RETURN` turns each surviving binding into a result resource.

```eigenql
RETURN [Prediction, AssayResult] {
    compound: ?cpd,
    predicted_affinity: ?pred,
    concentration: 10 * ?mw
}
```

**Behaviour**:

- The class list (between `[` and `]`) becomes the `is_a` array on each result row.
- Each `ReturnItem` is a name-expression pair. The expression is evaluated against the binding to produce a value.
- Name resolution: a `FullIri` is used verbatim as the property IRI; a `ShortName` is mapped to a synthesized per-query property IRI via [`QueryFingerprint::row_property_iri`](../../../kernel/src/query/document.rs). This keeps every query's result shape self-describing (see [chapter 11](11-result-format.md)).
- Aggregate expressions (see §4.9) are only permitted when `GROUP BY` is present or when the entire query has no non-aggregate return items.

**Empty return** (`RETURN [] {}`) produces a guard-style result — one empty row per binding. Useful for counting or existence checks.

## 4.9. `GROUP BY` — aggregation

```
GroupByClause ::= 'GROUP' 'BY' Expression (',' Expression)*
```

Partitions bindings by the values of the group-by expressions. Each partition becomes one output row, with aggregates computed per partition.

```eigenql
GROUP BY ?breed, ?country
RETURN [] {
    breed: ?breed,
    country: ?country,
    count: COUNT(?d),
    avg_age: AVG(?age)
}
```

**Type-check rule** (`aggregate_without_group_by`):

- Every non-aggregate expression in `RETURN` must appear verbatim in `GROUP BY`.
- If `RETURN` contains only aggregates (and class tags), `GROUP BY` may be empty — the entire binding set is one group.

## 4.10. `ORDER BY`, `LIMIT`, `TOP`, `OFFSET`, `DISTINCT`

```
OrderByClause ::= 'ORDER' 'BY' OrderItem (',' OrderItem)*
OrderItem    ::= Expression ('ASC' | 'DESC')?
LimitClause  ::= 'LIMIT' integer
TopClause    ::= 'TOP' integer
OffsetClause ::= 'OFFSET' integer
```

- **`DISTINCT`** (just the keyword, no argument) deduplicates result rows after `RETURN` shaping.
- **`ORDER BY`** sorts the result resources by the given expressions; default direction is `ASC`. Mixed `ASC`/`DESC` per column supported.
- **`OFFSET n`** skips the first `n` rows.
- **`LIMIT n`** truncates to `n` rows — un-ranked structural cutoff.
- **`TOP n`** truncates to `n` rows by **similarity ranking** when the query contains `~` operators (D43). Mutually exclusive with `LIMIT` and `ORDER BY`; requires at least one `~` in `WHERE`. See [chapter 6 §6.5](06-text-and-vector-retrieval.md#65-top-n--ranked-truncation) for the full surface and the typecheck rules.

Evaluation order: `RETURN` shape → `DISTINCT` → `ORDER BY` → `OFFSET` → `LIMIT`. The `TOP` reordering happens earlier — between GROUP BY and RETURN shaping — because it sorts by the per-binding similarity score before the binding-to-resource projection drops the subject IRI needed for the score lookup.

## 4.11. Typical clause order

```eigenql
-- DEFINE rules (0+)
DEFINE ...

-- Query
USING ...
USING INSTITUTION ... AS ...

MATCH ...      -- one or more clauses, interleaved with FIBER
FIBER ...
MATCH ...

WHERE ...      -- optional, may contain `~` similarity operators (chapter 13)

GROUP BY ...   -- optional
RETURN [...] { ... }

ORDER BY ...   -- mutually exclusive with TOP
LIMIT n        -- mutually exclusive with TOP
TOP n          -- mutually exclusive with LIMIT + ORDER BY; requires ~ in WHERE
OFFSET m
DISTINCT
```

The parser accepts the clauses in this order; it will error on unexpected sequencing.

---

Next: **[5. Pattern matching →](05-pattern-matching.md)**
