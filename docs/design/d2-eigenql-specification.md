# D2: EigenQL v1 Specification

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 1; D14-aligned)
**Required before:** Phase 1 implementation
**Resolves:** Final EBNF grammar, keyword choices, escaping rules, type checking rules, error message format

### Revision: D14 alignment

This revision updates EigenQL's institution surface against
[D14 (Institution Realisation)](d14-institution-realisation.md), which
supersedes the D10 surface the prior draft was wired against. The
substantive shape changes:

- The dispatch backbone is the derived [`InstitutionIndex`] over
  declared `Institution`, `QueryClass`, `Comorphism`, `ExportFormat`,
  and `ImportFormat` resources, plus an `InstitutionRuntime` that
  supplies the `Institution` trait implementations. There is no
  `InstitutionRegistry` and no `FiberReasoner` trait.
- A `FIBER` clause dispatches a registered `QueryClass` whose
  `dispatch_role` includes `OnDemand`, via `Institution::query`.
- Comorphisms surface as **FIBER parameter coercions**:
  `param: comorphism_iri(source)` runs the four-step pipeline
  (extract → transformation → reify) inline as part of FIBER input
  marshalling. They are not free-standing expressions.
- A qualified-name function call in expression position dispatches
  only as a `Decidable` `QueryClass` invocation, and returns a
  typed `Verdict` value (not Boolean).
- New postfix predicates `HOLDS`, `FAILS`, `UNDECIDABLE` project
  a Verdict to Boolean. They are reserved keywords.
- `FIBER`-bound response resources of `result_class = Verdict` are
  pattern-matchable on `ctor_name` in subsequent `MATCH` clauses.

[`InstitutionIndex`]: d14-institution-realisation.md

---

## 1. Overview

EigenQL is a typed semantic query language for pattern matching and retrieval over the Eigon knowledge graph. It operates within an execution context and sees exactly the resources visible within that context's layer chain.

EigenQL is a **typed Datalog** — it supports conjunctive queries with recursive rule definitions, aggregation, and typed result shaping. Non-recursive queries evaluate in a single pass. Recursive queries use bottom-up seminaive fixpoint evaluation. Negation in MATCH patterns is supported with stratification checking to prevent paradoxes.

### 1.1 Program structure

An EigenQL program consists of zero or more **rule definitions** followed by a **query**:

```
Program ::= DEFINE* Query
```

### 1.2 Rule definitions

A `DEFINE` clause names a query result as a **derived relation** — a virtual relation that can be referenced in MATCH patterns of other rules (including itself, for recursion).

```
DEFINE RelationName(variable_list) FROM match_part
```

Multiple DEFINE clauses for the same relation name provide **union semantics** — a resource matches the relation if it matches any of the definitions. Self-reference enables recursion.

### 1.3 Query structure

```
Query ::= [USING] MATCH [WHERE] [GROUP BY] [RETURN] [ORDER BY] [LIMIT] [OFFSET] [DISTINCT]
```

A query without a RETURN clause is a **match query** — it evaluates to a boolean (does any matching assignment exist?). This form is used as guard conditions in program Select constructs.

A query with a RETURN clause is a **result query** — it produces typed resources shaped by the RETURN clause.

Every standalone (non-recursive) query is equivalent to a single non-recursive rule — existing v1 queries are valid EigenQL programs without modification.

---

## 2. Lexical Grammar

The lexical grammar defines the tokens recognized by the lexer. Whitespace and comments are discarded before parsing.

### 2.1 Whitespace and comments

```
WHITESPACE  ::= [ \t\n\r]+
LINE_COMMENT ::= "//" [^\n]*
BLOCK_COMMENT ::= "/*" .* "*/"
```

Both comment forms are discarded. Block comments may span multiple lines.

### 2.2 Keywords

Keywords are case-sensitive and uppercase:

```
MATCH  WHERE  RETURN  USING  INSTITUTION  AS  DEFINE  FROM  FIBER
AND  OR  NOT  IN  LIKE  EXISTS
GROUP  BY  ORDER  ASC  DESC
DISTINCT  LIMIT  OFFSET
HOLDS  FAILS  UNDECIDABLE
```

Built-in function names are also keywords:

```
DATE  TIMESTAMP  REGEX  LENGTH  CONTAINS  CONCAT
COUNT  SUM  AVG  MIN  MAX
```

`INSTITUTION` and `FIBER` are reserved for institution dispatch
(§3.3.1, §3.5). `HOLDS`, `FAILS`, and `UNDECIDABLE` are reserved as
postfix predicates that project a `Verdict`-typed expression to a
boolean (§3.8).

### 2.3 Identifiers and variables

```
IDENTIFIER ::= [a-zA-Z] [a-zA-Z0-9_-]*
VARIABLE   ::= "?" [a-zA-Z] [a-zA-Z0-9_]*
```

Identifiers are used for shortnames (property and class references resolved against the ontology). Variables bind values during pattern matching and are prefixed with `?`.

### 2.4 Literals

```
STRING  ::= '"' (ESCAPE | [^"\\])* '"'
ESCAPE  ::= '\\' ["\\/bfnrt] | '\\u' HEX{4}
HEX     ::= [0-9a-fA-F]

NUMBER  ::= '-'? ('0' | [1-9] DIGIT*) ('.' DIGIT+)? ([eE] [+-]? DIGIT+)?

BOOLEAN ::= 'true' | 'false'
```

String literals use JSON escaping rules (same as Eigon-JSON). Numbers follow JSON number syntax.

There is no `undefined` or `null` literal. Use `NOT EXISTS(?var)` to test for property absence.

### 2.5 Operators

```
COMPARISON ::= '=' | '<>' | '<' | '<=' | '>' | '>='
ARITHMETIC ::= '+' | '-' | '*' | '/' | '%' | '**'
STRING_OP  ::= '||'
```

### 2.6 Structural tokens

```
LPAREN    ::= '('
RPAREN    ::= ')'
LBRACE    ::= '{'
RBRACE    ::= '}'
LBRACKET  ::= '['
RBRACKET  ::= ']'
COLON     ::= ':'
COMMA     ::= ','
DOT       ::= '.'
```

### 2.7 IRI literals

IRIs appear as string literals (double-quoted). There is no special IRI token — the parser distinguishes IRIs from plain strings by context (USING clause, class references, property references). Any resource can always be referenced by its full IRI as a quoted string, without requiring USING.

```
USING "urn:eigenius:example:Dog", "urn:eigenius:example:Animal"
```

---

## 3. Parser Grammar (EBNF)

### 3.1 Top-level

```ebnf
program     ::= define_clause* query
query       ::= match_part group_by_clause? return_clause? order_by_clause?
                limit_clause? offset_clause? 'DISTINCT'?
match_part  ::= (using_clause | using_institution)* clause+ where_clause?
clause      ::= match_clause | fiber_clause
```

A `match_part` accepts any interleaving of plain `USING` clauses and
`USING INSTITUTION` clauses, followed by one or more `MATCH` or `FIBER`
clauses in textual order, optionally followed by a `WHERE`. Clauses are
processed left to right: each `MATCH` or `FIBER` may bind variables that
subsequent clauses consume.

**DEFINE bodies are restricted.** The `match_part` inside a `DEFINE`
body permits plain `USING` clauses and exactly one `MATCH` clause; it
forbids both `USING INSTITUTION` and `FIBER`. See §3.2 and §6.7.

### 3.2 DEFINE clause

```ebnf
define_clause   ::= 'DEFINE' IDENTIFIER '(' variable_list ')' 'FROM' define_body
define_body     ::= using_clause* match_clause where_clause?
variable_list   ::= variable (',' variable)*
```

A DEFINE clause introduces a named derived relation. The relation name is an identifier. The variable list declares the relation's arity and column names.

Multiple DEFINE clauses with the same name define a **union** — a resource matches the relation if it matches any definition. Self-reference in a DEFINE's MATCH clause creates **recursion**.

**DEFINE body restrictions.** `define_body` is the restricted form of
`match_part`: it permits plain `USING` clauses and exactly one `MATCH`,
but forbids both `FIBER` and `USING INSTITUTION`. Institution dispatch
is reserved for the top-level query so that derived relations remain
pure and stratifiable. To feed institution responses into a derived
relation, run an outer query that issues `FIBER` and stores the
responses as resources, then run a follow-up query whose `DEFINE`s
operate on those.

**Negation in MATCH patterns.** Within a DEFINE's MATCH clause (and in
the final query's MATCH clauses), a pattern may be prefixed with `NOT`
to express negation. The grammar is defined in §3.4. A negated pattern
succeeds when no matching resource exists; negation is subject to
stratification checking (see §6.9).

**Example — transitive ancestor relation:**

```
DEFINE Ancestor(?x, ?z) FROM
    MATCH Employee(?x) { reports_to: ?z }

DEFINE Ancestor(?x, ?z) FROM
    MATCH Employee(?x) { reports_to: ?y },
    Ancestor(?y, ?z)

MATCH Ancestor(?a) {
    ?person: "urn:example:alice"
}
RETURN [] { ancestor: ?a }
```

**Example — negation with stratification:**

```
DEFINE Orphan(?x) FROM
    MATCH Animal(?x) { name: ?name },
    NOT Animal(?parent) { offspring: ?x }

MATCH Orphan(?o) { name: ?name }
RETURN [] { name: ?name }
```

### 3.3 USING clause

```ebnf
using_clause       ::= 'USING' string_list
string_list        ::= STRING (',' STRING)*
```

The USING clause imports ontology classes by IRI for shortname reference within the query. Each IRI must resolve to a valid Class resource in the current layer chain. Shortnames are derived from the `short_name` property of the resolved resource.

USING is a convenience — it enables bare identifier references like `Dog` instead of `"urn:eigenius:example:Dog"`. Without USING, any class or property can still be referenced by its full IRI as a quoted string.

Multiple USING clauses are allowed and their imports are merged (duplicates removed). Shortnames must be unique within the query scope; a duplicate shortname is an error.

#### 3.3.1 USING INSTITUTION

```ebnf
using_institution ::= 'USING' 'INSTITUTION' STRING 'AS' IDENTIFIER
```

A `USING INSTITUTION` clause binds a short alias to a declared
institution IRI for use in subsequent `FIBER` clauses (§3.5). The IRI
must resolve in the layer chain to a resource whose `is_a` includes
`urn:eigenius:institution:Institution` — D14 institutions ride into
the chain as ordinary ontology resources. The kernel's
`InstitutionIndex` (built from the chain) is the authority for the
lookup.

Beyond alias introduction, `USING INSTITUTION` carries a structural
assertion: the QueryClass cited by every subsequent `FIBER` clause
referencing this alias must declare the aliased institution as its
`institution_ref`. The assertion is enforced at type check (§5.7).
Likewise, when a FIBER parameter uses comorphism coercion (§3.5), the
comorphism's `import_format`'s `institution_ref` must match the
aliased institution.

The alias inhabits a separate namespace from `USING` class shortnames
and from variable names — it is only valid as the institution-reference
position in `FIBER`. Two `USING INSTITUTION` clauses may not declare
the same alias. Type-check errors for these conditions are listed in §5.

`USING` and `USING INSTITUTION` clauses may appear in any order and may
be interleaved (per the §3.1 `match_part` grammar).

`USING INSTITUTION` is only valid in the top-level query's `match_part`,
not in a `DEFINE` body — see §3.2.

### 3.4 MATCH clause

```ebnf
match_clause    ::= 'MATCH' pattern_list
pattern_list    ::= pattern (',' pattern)*

pattern         ::= 'NOT'? (typed_pattern | untyped_pattern)
typed_pattern   ::= name '(' variable ')' object_pattern
untyped_pattern ::= variable object_pattern

object_pattern  ::= '{' property_list? '}'
property_list   ::= property (',' property)*
property        ::= name ':' value

name            ::= IDENTIFIER | STRING
```

**Typed patterns** constrain matches to instances of the named class (and its subclasses via `subclass_of`). The class is referenced by shortname (resolved from USING imports) or by full IRI (as a string literal). Properties referenced in a typed pattern must be valid for the specified class, accounting for property inheritance through the subclass chain.

A typed pattern's class name may also be the name of a `DEFINE`d
relation. Patterns of the form `Relation(?var) { ... }` look up bindings
of the derived relation, not class instances. The parser does not
syntactically distinguish the two — resolution is by name in the
type-checker's scope (USING-imported class shortname vs. DEFINEd
relation name).

**Untyped patterns** match any resource. Properties are not validated against a class schema.

**Negated patterns** (`NOT` prefix) succeed when no resource matches the
inner pattern shape. Negation is subject to stratification (§6.9).

**Name resolution.** A `name` is either:
- A bare identifier — resolved as a shortname against USING imports (for class names) or against the matched class's property set (for property names)
- A quoted string — interpreted as a full IRI, resolved directly from the layer chain

**Dot-path navigation.** Property paths like `?person.address.city` are supported as shortname-only sugar. Each segment is resolved as a shortname against the type of the preceding segment. For full IRI precision, decompose into multiple patterns:

```
MATCH ?person {
    "urn:eigenius:example:address": ?addr
},
?addr {
    "urn:eigenius:example:city": ?city
}
```

### 3.5 FIBER clause

```ebnf
fiber_clause      ::= 'FIBER' institution_ref ':' query_class_ref
                      '{' param_list? '}' 'AS' variable
institution_ref   ::= IDENTIFIER | STRING
query_class_ref   ::= IDENTIFIER | STRING
param_list        ::= param (',' param)*
param             ::= name ':' param_value
param_value       ::= expression | comorphism_coercion
comorphism_coercion ::= qualified_name '(' expression ')'
qualified_name    ::= IDENTIFIER ':' IDENTIFIER | STRING
```

A `FIBER` clause dispatches a `QueryClass` declared in the chain that
includes `OnDemand` in its `dispatch_role` set (D14 §6.2). The dispatch
is `Institution::query(query_handler, input_resource, ctx)` where:

- The `Institution` is looked up by the `institution_ref` of the
  cited `QueryClass` in the runtime's `InstitutionRuntime`.
- The `query_handler` is the `QueryClass`'s declared procedure IRI.
- The `input_resource` is constructed from the FIBER param block,
  with `is_a` set to the `QueryClass`'s `query_class` input class.

The clause binds the response resource (or a synthesized handle for
it; see §6.12) to a variable that subsequent clauses can decompose
with `MATCH`.

- `institution_ref` is either a `USING INSTITUTION` alias
  (`IDENTIFIER`) or an inline full IRI (quoted `STRING`).
- `query_class_ref` is the QueryClass — a short name resolved against
  the QueryClass declarations in the index, or a full IRI. The cited
  QueryClass's `institution_ref` must equal the aliased / inline
  institution (§5.8).
- The brace block contains parameter bindings (`name : param_value`)
  passed as properties on the constructed input resource.
- `AS variable` binds the response resource to a variable usable in
  later clauses.

A param's value is normally an expression (literal, variable, scalar
function call). A param can also use **comorphism coercion**:
`name: comorphism_iri(source)` runs the named comorphism's four-step
pipeline (D14 §9.3) — extract a typed value from the source resource
via the source institution's `ExportFormat`, apply the comorphism's
`transformation` Component, reify into a target-class resource via the
target institution's `ImportFormat` — and uses the reified value as
the parameter input. The coercion's target class must match (or
satisfy `subclass_of` against) the FIBER QueryClass's input class for
that property; see §5.8.

**v1 restriction: no IO in the comorphism transformation.** The cited
comorphism's `transformation` Component must have a `capability_level`
of `Pure` or `Read`. A comorphism whose transformation requires `IO`
is rejected with the typed error `comorphism_io_not_supported_in_v1`
(§7). This restriction lets the EigenQL evaluator run a coercion
inline without standing up an IO evaluator backing for the FIBER path.
A future revision can lift the restriction; the surface syntax is
unchanged when it does.

`FIBER` is only valid in the top-level query's `match_part`, never in
a `DEFINE` body. See §5.7–5.9 for type rules and §6.12 for evaluation.

**Examples**

With alias:

```
USING INSTITUTION "urn:eigenius:dock:Inst"  AS dock
USING INSTITUTION "urn:eigenius:assay:Inst" AS assay

MATCH DockingResult(?d) { binding_pose: ?p }
FIBER assay:CheckBindingAffinity {
    candidate: dock_to_assay(?d),    // comorphism coercion
    threshold: 0.85
} AS ?check
WHERE ?check HOLDS
RETURN [] { d: ?d }
```

Inline IRI (one-shot, no coercion):

```
MATCH Refinement(?m) { latest_delta: ?d }
FIBER "urn:eigenius:test:Ordering":ConvergenceQuery
      { tolerance: 0.01, latest_delta: ?d } AS ?conv
MATCH ?conv { "urn:eigenius:test:converged": ?c }
WHERE ?c = true
RETURN [] { m: ?m }
```

Mixed param names — required short name, recommended short name, full-IRI pass-through:

```
FIBER ord:ConvergenceQuery {
    tolerance: 0.01,                           // short name in `requires`: must be present
    latest_delta: ?d,                          // short name in `requires`: must be present
    window_hint: 50,                           // short name in `recommends`: optional
    "urn:example:client:correlation_id": ?cid  // full IRI: pass-through, no scope check
} AS ?conv
```

The first two are in the query class's `requires` list and must be
supplied. The third is in `recommends` — an extra hint the class
knows about but doesn't demand. The fourth is outside the class
scope entirely; it is included in the query resource as-is. Typos in
the short-name positions are caught at type-check time because they
fail to resolve in the class's declared property set.

### 3.6 WHERE clause

```ebnf
where_clause    ::= 'WHERE' expression_list
expression_list ::= expression (',' expression)*
```

Multiple expressions in the WHERE clause are implicitly ANDed.

### 3.7 Expressions

Operator precedence is given below the grammar (after the operator
list), once the postfix Verdict predicate has been introduced.

`**` is conventionally right-associative in mathematics (`2**3**2 = 512`)
but the current parser folds it left (`(2**3)**2 = 64`). Always parenthesise
stacked exponents when the order matters.

```ebnf
expression      ::= or_expr
or_expr         ::= and_expr ('OR' and_expr)*
and_expr        ::= equality_expr ('AND' equality_expr)*
equality_expr   ::= relational_expr (('=' | '<>') relational_expr)*
relational_expr ::= additive_expr ((cmp_op | comparison_op) additive_expr)*
additive_expr   ::= mult_expr (('+' | '-' | '||') mult_expr)*
mult_expr       ::= power_expr (('*' | '/' | '%') power_expr)*
power_expr      ::= unary_expr ('**' unary_expr)*
unary_expr      ::= verdict_term
                   | 'NOT' unary_expr
                   | 'NOT' 'EXISTS' '(' variable ')'
                   | '+' unary_expr
                   | '-' unary_expr
verdict_term    ::= primary_expr (verdict_predicate)?

cmp_op            ::= '<' | '<=' | '>' | '>='
comparison_op     ::= 'IN' | 'NOT' 'IN' | 'LIKE' | 'NOT' 'LIKE'
verdict_predicate ::= 'HOLDS' | 'FAILS' | 'UNDECIDABLE'

primary_expr    ::= value
                   | '(' expression ')'
                   | array_literal
                   | function_call
                   | aggregate_call
                   | qualified_call
                   | shortname_literal
                   | variable '.' IDENTIFIER ('.' IDENTIFIER)*

shortname_literal ::= IDENTIFIER     // bare identifier — string-typed shortname literal
```

`verdict_predicate` is a postfix operator that binds tighter than `NOT`
and looser than primary-expression construction. Its operand must
evaluate to a `Verdict` inductive value (§3.8); the result is
`boolean`. Because it is non-associative, expressions like
`?v HOLDS FAILS` are syntactically rejected.

The placement gives the natural reading on `NOT`:

```
NOT qc:check(?x) HOLDS    ≡    NOT (qc:check(?x) HOLDS)
                          ≡    "either Fails or Undecidable"
```

And on `AND`/`OR`:

```
qc:a(?x) HOLDS AND qc:b(?y) FAILS
    ≡  (qc:a(?x) HOLDS) AND (qc:b(?y) FAILS)
```

The operator-precedence table is updated accordingly:

| Precedence | Operators | Associativity |
|------------|-----------|---------------|
| 1 | Postfix `HOLDS`, `FAILS`, `UNDECIDABLE` | Non-associative |
| 2 | Unary `NOT`, `+`, `-` | Right |
| 3 | `**` | Left (parser quirk; see §3.7 note) |
| 4 | `*`, `/`, `%` | Left |
| 5 | `+`, `-`, `\|\|` | Left |
| 6 | `<`, `<=`, `>`, `>=`, `IN`, `NOT IN`, `LIKE`, `NOT LIKE` | Left |
| 7 | `=`, `<>` | Left |
| 8 | `AND` | Left |
| 9 | `OR` | Left |

A bare `IDENTIFIER` in expression position evaluates to the identifier
text as a string-typed scalar. This is the same form used to pass class
or property short names as scalar values (e.g. in `RETURN`).

### 3.8 Function calls

EigenQL has three categories of function call. The first is the closed
set of built-in scalar functions:

```ebnf
function_call ::= DATE '(' value ')'
                | TIMESTAMP '(' value ')'
                | REGEX '(' value ')'
                | LENGTH '(' expression ')'
                | CONTAINS '(' expression ',' expression ')'
                | CONCAT '(' expression ',' expression ')'
```

| Function | Arguments | Returns | Description |
|----------|-----------|---------|-------------|
| `DATE` | string | date | Parse an ISO 8601 date string |
| `TIMESTAMP` | string | datetime | Parse an ISO 8601 datetime string |
| `REGEX` | string | regex | Compile a regular expression for LIKE matching |
| `LENGTH` | string or array | integer | String character count or array element count |
| `CONTAINS` | array, value | boolean | Test if array contains value |
| `CONCAT` | array, array | array | Concatenate two arrays |

The second category is institution-dispatched function calls: a
`qualified_call` invokes a `Decidable` `QueryClass` declared by some
institution (D14 §6.2). The result is a typed `Verdict` value, not a
Boolean.

```ebnf
qualified_call ::= qualified_name '(' arg_list? ')'
qualified_name ::= IDENTIFIER ':' IDENTIFIER
arg_list       ::= expression (',' expression)*
```

The qualified name resolves to a full IRI and is looked up in the
`InstitutionIndex`. The IRI must reference a `QueryClass` declaration
whose `dispatch_role` set includes `Decidable` (§5.9). If the IRI is
not a Decidable QueryClass — or no institution is in scope — the
qualified-name fall-through is **`unknown function: ns:local`**
(§6.13). Comorphism dispatch is **not** available in expression
position; comorphisms surface only in FIBER parameter coercion (§3.5).

The result of a Decidable call is a `Verdict` resource (constructors
`Holds | Fails | Undecidable`). To use that result as a Boolean, apply
a postfix Verdict predicate:

```
?v HOLDS         // true iff ctor_name = "Holds"
?v FAILS         // true iff ctor_name = "Fails"
?v UNDECIDABLE   // true iff ctor_name = "Undecidable"
```

The postfix predicates also accept any expression that evaluates to a
Verdict — most commonly the `qualified_call` itself, or a variable
bound by a `FIBER` clause whose `result_class` is `Verdict`. Examples:

```
WHERE qc:check(?x) HOLDS                // common case: Decidable in WHERE
WHERE NOT qc:check(?x) HOLDS            // "either Fails or Undecidable"
WHERE ?conv HOLDS AND ?other FAILS      // FIBER-bound Verdicts
RETURN [] { ok: qc:check(?x) HOLDS }    // boolean projection in RETURN
```

A bare Verdict expression in Boolean position (e.g. `WHERE qc:check(?x)`
without a postfix predicate) is a type error (§5.9). This is
deliberate — it forces an explicit choice on how Undecidable should
behave, rather than silently collapsing it.

The postfix predicates are Verdict-specific in v1: they reject any
operand whose static type isn't `Verdict`. A future revision could
generalise them to project any inductive value to a constructor-name
match without breaking existing source.

The third category is the aggregate calls covered in §3.9.

### 3.9 Aggregate functions

```ebnf
aggregate_call ::= COUNT '(' expression ')'
                 | SUM '(' expression ')'
                 | AVG '(' expression ')'
                 | MIN '(' expression ')'
                 | MAX '(' expression ')'
```

| Function | Arguments | Returns | Description |
|----------|-----------|---------|-------------|
| `COUNT` | any | integer | Count of values (including duplicates) |
| `SUM` | numeric | numeric | Sum of values |
| `AVG` | numeric | float | Arithmetic mean |
| `MIN` | numeric, string, or date | same type | Minimum value |
| `MAX` | numeric, string, or date | same type | Maximum value |

Aggregate functions may only appear in the RETURN clause. A query that uses aggregates in RETURN must either:
- Aggregate all result properties (no grouping needed), or
- Include a GROUP BY clause specifying the non-aggregated properties

### 3.10 Literals and values

```ebnf
value         ::= literal | variable
literal       ::= STRING | NUMBER | BOOLEAN

variable      ::= VARIABLE

array_literal ::= '[' ']' | '[' expression_list ']'
```

The grammar does not include an object literal in expression position;
embedded resource construction is performed only by the kernel during
RETURN shaping (§6.5) and FIBER param materialisation (§6.12).

### 3.11 RETURN clause

```ebnf
return_clause      ::= 'RETURN' return_class_names? '{' return_list? '}'
return_class_names ::= name | '[' name_list ']' | '[' ']'
return_list        ::= return_item (',' return_item)*
return_item        ::= name ':' expression
name_list          ::= name (',' name)*
```

The class spec preceding the body has three forms: a single bare name
(`RETURN Person { ... }`), a bracketed list (`RETURN [Person, Animal] { ... }`,
including the empty list `[]` for an untyped result), or omitted entirely
(braces directly after `RETURN`).

The body may be empty (`{ }`) — useful for guard queries that return only
the boolean `matched` and `row_count` (see §6.5 and Appendix A).

The return class determines which properties are valid in the output. The result class may be a single class name, an array of class names (for resources with multiple class membership), or empty (untyped result).

Each return item maps a property name to an expression. Expression types must match the declared property data types.

The wire shape of a RETURN-bearing response — row resources plus a
synthesized row class and Property resources wrapped in a `ResultSet` —
is specified in [Appendix A](#appendix-a-result-documents). Consumers
drive lookups through the included class, not through fixed IRI
conventions.

### 3.12 GROUP BY clause

```ebnf
group_by_clause ::= 'GROUP' 'BY' expression_list
```

Groups result rows by the specified expressions before applying aggregation. All non-aggregated expressions in the RETURN clause must appear in the GROUP BY clause.

### 3.13 ORDER BY clause

```ebnf
order_by_clause ::= 'ORDER' 'BY' order_item (',' order_item)*
order_item      ::= expression ('ASC' | 'DESC')?
```

Sorts results by the specified expressions. Default sort direction is ASC. Expressions must reference variables or properties present in the RETURN clause.

### 3.14 LIMIT and OFFSET

```ebnf
limit_clause  ::= 'LIMIT' NUMBER
offset_clause ::= 'OFFSET' NUMBER
```

LIMIT restricts the number of results returned. OFFSET skips the first N results. Both values must be non-negative integers. OFFSET without LIMIT is allowed.

### 3.15 DISTINCT

```ebnf
distinct_clause ::= 'DISTINCT'
```

Deduplicates result resources. Two results are considered duplicates if all their return properties have equal values.

---

## 4. Abstract Syntax Tree

The AST produced by the parser:

```typescript
interface Program {
  definitions: RuleDefinition[];  // DEFINE clauses
  query: Query;                   // Final query
}

interface RuleDefinition {
  name: string;                 // Relation name
  variables: Variable[];        // Declared variables (arity)
  body: MatchPart;              // The DEFINE body (USING + MATCH + WHERE)
}

interface MatchPart {
  using: string[];              // IRI strings from USING clauses
  usingInstitutions: InstitutionAlias[];  // USING INSTITUTION clauses
  clauses: Clause[];            // MATCH and FIBER clauses, in textual order
  conditions: Expression[];     // WHERE conditions
}

interface InstitutionAlias {
  iri: string;                  // Institution IRI
  alias: string;                // Identifier used in subsequent FIBER clauses
}

type Clause =
  | { kind: 'pattern'; pattern: Pattern }
  | { kind: 'fiber'; fiber: FiberClause };

interface FiberClause {
  institution: Name;            // Alias (shortname) or full IRI
  queryClass: Name;             // Class shortname or full IRI
  params: ParamBinding[];       // Property bindings for the query resource
  binding: Variable;            // ?var bound to the response resource IRI
}

interface ParamBinding {
  name: Name;
  value: ParamValue;
}

type ParamValue =
  | { kind: 'expression'; expression: Expression }
  | { kind: 'comorphism'; comorphism: Name; source: Expression };

interface Query {
  body: MatchPart;              // USING + USING INSTITUTION + (MATCH | FIBER)+ + WHERE
  groupBy: Expression[];        // GROUP BY expressions
  resultClasses: Name[];        // RETURN class names
  result: ReturnItem[];         // RETURN property mappings
  orderBy: OrderItem[];         // ORDER BY items
  limit?: number;               // LIMIT value
  offset?: number;              // OFFSET value
  distinct: boolean;            // DISTINCT flag
}

interface OrderItem {
  expression: Expression;
  direction: 'asc' | 'desc';
}

interface Pattern {
  subject: { variable: Variable };
  class?: Name;                 // Present for typed patterns
  properties: PropertyPattern[];
  negated: boolean;             // true for NOT patterns
}

interface PropertyPattern {
  property: Name;
  object: VariableOrValue<unknown>;
}

interface Name {
  shortname?: string;           // Bare identifier
  uri?: string;                 // Full IRI (quoted string)
}

interface Variable {
  name: string;                 // Without the '?' prefix
}

type VariableOrValue<T> = { variable: Variable } | { value: T };

interface ReturnItem {
  name: Name;
  expression: Expression;
}

type Expression =
  | { value: unknown }                                    // Literal value
  | { variable: Variable }                                // Variable reference
  | { binary: BinaryOperator; left: Expression; right: Expression }
  | { unary: UnaryOperator; operand: Expression }
  | { verdictPredicate: VerdictPredicate; operand: Expression } // ?v HOLDS / FAILS / UNDECIDABLE
  | { notExists: Variable }                               // NOT EXISTS(?var)
  | { function: string; arguments: Expression[] }         // Function call (builtin or qualified)
  | { aggregate: AggregateOp; argument: Expression }      // Aggregate function
  | { path: Variable; segments: string[] }                // Dot-path: ?var.a.b
  | { array: Expression[] }                               // Array literal

type BinaryOperator =
  | '=' | '<>' | '<' | '<=' | '>' | '>='
  | '+' | '-' | '*' | '/' | '%' | '**'
  | '||'
  | 'and' | 'or'
  | 'in' | 'not in' | 'like' | 'not like';

type UnaryOperator = 'not' | '+' | '-';

type VerdictPredicate = 'holds' | 'fails' | 'undecidable';

type AggregateOp = 'count' | 'sum' | 'avg' | 'min' | 'max';
```

The `function` Expression variant carries the textual function name —
either a builtin (`"DATE"`, `"LENGTH"`, …) or a qualified name
(`"docking:within_tolerance"`). Qualified names dispatch only to
`Decidable` `QueryClass` declarations under D14; resolution and
dispatch happen at evaluate time per §6.13. Comorphism dispatch lives
on `ParamValue.kind = 'comorphism'`, not on the `function` variant.

---

## 5. Type Checking Rules

Type checking is performed at query submission time, before evaluation begins.

### 5.1 Variable typing

- A variable bound to a property in MATCH inherits the property's declared data type from the ontology.
- A variable used across multiple patterns must have a consistent type across all uses.
- Variables must be defined in MATCH before use in WHERE, GROUP BY, RETURN, or ORDER BY.
- Unbound variables in RETURN are an error.

### 5.2 Expression type rules

| Expression | Type constraint |
|------------|----------------|
| `a = b`, `a <> b` | Operands must have the same data type |
| `a < b`, `a <= b`, `a > b`, `a >= b` | Operands must be numeric, string, or date/datetime |
| `a + b`, `a - b`, `a * b`, `a / b`, `a % b` | Operands must be numeric; result is numeric |
| `a ** b` | Operands must be numeric; result is float |
| `a \|\| b` | Operands must be strings; result is string |
| `a AND b`, `a OR b` | Operands must be boolean; result is boolean |
| `NOT a` | Operand must be boolean; result is boolean |
| `NOT EXISTS(?var)` | Variable must be bound in MATCH; result is boolean |
| `a IN b` | `b` must be a resource_array or value_array |
| `a LIKE b` | Both operands must be strings |
| `a NOT IN b`, `a NOT LIKE b` | Same as `IN`, `LIKE` respectively |

No implicit type coercion is performed. A type mismatch is a query error.

### 5.3 Aggregate type rules

| Aggregate | Argument type | Result type |
|-----------|--------------|-------------|
| `COUNT` | any | integer |
| `SUM` | numeric | same numeric type |
| `AVG` | numeric | float |
| `MIN` | numeric, string, or date | same type |
| `MAX` | numeric, string, or date | same type |

Aggregates may only appear in RETURN expressions. Non-aggregated RETURN expressions must appear in GROUP BY.

### 5.4 Pattern type rules

- In a typed pattern `ClassName(?var) { prop: ?val }`, `prop` must be a valid property for `ClassName` (directly declared or inherited through `subclass_of`).
- The class must resolve to a resource with `is_a` including `urn:eigenius:core:Class`.
- The property must resolve to a resource with `is_a` including `urn:eigenius:core:Property`.
- Full IRI references (quoted strings) are resolved directly from the layer chain, bypassing USING.

### 5.5 RETURN type rules

- Each property in the RETURN clause must be valid for the declared result class.
- The expression type for each property must match the property's declared data type.
- If the result class is empty (`[]`), no property validation is performed.

### 5.6 Dot-path type rules

- Each segment in a dot-path `?var.a.b` is resolved as a shortname against the type of the preceding segment.
- The root variable must be bound to a resource in MATCH.
- Each intermediate segment must resolve to a property with data type `resource`.
- The final segment may resolve to a property of any data type.
- Dot-paths are unavailable for full IRI property references — use multi-pattern decomposition instead.

### 5.7 USING INSTITUTION type rules

- The IRI in `USING INSTITUTION "iri" AS alias` must resolve to a
  resource in the layer chain whose `is_a` includes
  `urn:eigenius:institution:Institution` and which is indexed by the
  `InstitutionIndex`. Failure rule: `using_institution_unresolved`.
- An alias must be unique within a `match_part`. Failure rule:
  `duplicate_using_institution_alias`.
- Aliases inhabit a separate namespace from `USING` class shortnames
  and from variable names; collisions across namespaces are not
  detected at type check (the parser distinguishes them by context).

### 5.8 FIBER type rules

For each `FIBER inst_ref : QueryClass { params } AS ?var` clause:

1. **Institution resolution.** `inst_ref` is either an alias declared by
   a `USING INSTITUTION` (rule `undeclared_institution_alias` if
   missing) or an inline IRI (rule `using_institution_unresolved` if
   not indexed). The resolved IRI is the *aliased institution*.

2. **QueryClass.** `QueryClass` must resolve to a `QueryClass`
   declaration in the `InstitutionIndex` (i.e. an indexed resource
   whose `is_a` includes
   `urn:eigenius:institution:QueryClass`). Failure rule:
   `fiber_query_class_not_query_class`.

3. **Dispatch role.** The cited QueryClass's `dispatch_role` set must
   include `OnDemand`. Failure rule:
   `fiber_query_class_not_on_demand` — a Decidable-only QueryClass
   cannot be dispatched via FIBER even though it has the same shape;
   the user surfaces it via §3.8 instead.

4. **Institution agreement.** The cited QueryClass's `institution_ref`
   must equal the aliased institution. Failure rule:
   `fiber_institution_mismatch`.

5. **Short-name parameter scope.** A short-name parameter must resolve
   against the union of the QueryClass's `urn:eigenius:core:requires`
   and `urn:eigenius:core:recommends` property lists. A short name
   that fails to resolve is rule `fiber_param_short_name_unresolved`.
   This is the rule that catches typos like `tolerence` for `tolerance`.

6. **Required-property coverage.** Every property in the QueryClass's
   `requires` list must have a matching entry in `params`. Missing
   coverage is rule `fiber_missing_required_param`.

7. **Full-IRI parameters** (quoted-string names) bypass class-scope
   validation — the open-world type system permits resources to carry
   properties beyond their declared class, and FIBER params follow
   the same rule. Type-checker does not warn on these.

8. **Parameter expression types.** Each plain-expression param's
   inferred type must match the declared `data_type` on the resolved
   Property resource. Lenient v1: if the type checker cannot infer an
   expression's type, the check is skipped (runtime catches genuine
   mismatches).

9. **Comorphism coercion params.** When a param's value is a
   `comorphism_coercion` of shape `comorphism_ref(source_expr)`:

   a. `comorphism_ref` must resolve to a `Comorphism` declaration in
      the `InstitutionIndex`. Failure rule:
      `comorphism_unresolved`.
   b. The resolved comorphism's `import_format`'s `institution_ref`
      must equal the aliased institution. Failure rule:
      `comorphism_target_mismatch`.
   c. The resolved comorphism's `transformation` Component must have
      `capability_level` `Pure` or `Read`. Failure rule:
      `comorphism_io_not_supported_in_v1`.
   d. The reified target class (from the comorphism's `import_format`
      `to_class`) must equal or `subclass_of` the FIBER QueryClass's
      input class for that property. Failure rule:
      `comorphism_target_class_mismatch`.
   e. `source_expr` evaluates to a resource value (variable bound by
      MATCH, embedded literal, or another expression of resource
      type). Non-resource source values fail rule
      `comorphism_source_not_resource`.

10. **Variable binding.** `?var` must not shadow an existing variable.

The response resource bound to `?var` carries the QueryClass's
`result_class` as its `is_a`. When that class is `Verdict`, subsequent
clauses may use `?var HOLDS` / `?var FAILS` / `?var UNDECIDABLE` for
projection (§3.8) or pattern-match `MATCH ?var { ctor_name: ?c }`
directly. For other result classes, the response resource is treated
as an untyped resource for the purposes of short-name dot-paths on
`?var` (use full-IRI property references when decomposing).

### 5.9 Institution-dispatched function-call type rules

For each expression of the form `qualified_name(args...)` where
`qualified_name = ns : local`:

1. **Classification.** The implementation joins `ns` and `local` to
   form an IRI string and looks it up in the `InstitutionIndex`. The
   IRI must reference a `QueryClass` declaration whose `dispatch_role`
   set includes `Decidable`. Unrecognised qualified names are not a
   type-check error — they fall through to evaluate-time builtin
   dispatch and surface as evaluation error `unknown function`
   (see §6.13). A qualified name that resolves to a `QueryClass`
   *without* `Decidable` in its dispatch role is rule
   `qualified_call_not_decidable` — to dispatch an OnDemand-only
   QueryClass, use FIBER (§3.5).

2. **Decidable QueryClass.** No static arity or argument-type check
   on the args themselves — the institution's query handler validates
   shape against the QueryClass's input class. Result type is
   `Verdict`.

3. **Postfix predicates.** A `verdict_predicate` (`HOLDS` / `FAILS`
   / `UNDECIDABLE`) requires its operand's inferred type to be
   `Verdict`. Otherwise rule `verdict_predicate_non_verdict_operand`.
   The result is `boolean`.

4. **Bare Verdict in Boolean position.** A `Verdict`-typed expression
   used directly where a `boolean` is required (WHERE, AND/OR
   operands, RETURN's boolean projection sites, etc.) is rule
   `bare_verdict_in_boolean_position`. The user surfaces a Boolean by
   applying a postfix predicate.

The closed-set built-in function calls of §3.8 retain their existing
arity-checked rules; only qualified names dispatch through the
`InstitutionIndex`.

---

## 6. Evaluation Semantics

### 6.1 Pattern matching

Each MATCH pattern generates a set of **bindings** — assignments of variables to values. Multiple patterns are joined by shared variables (equi-join). The result is the set of all consistent binding combinations across all patterns.

For a typed pattern `ClassName(?var) { prop1: ?a, prop2: ?b }`:
1. Iterate over all resources in the layer chain where `is_a` includes `ClassName` (or a subclass)
2. For each matching resource, bind `?var` to the resource's IRI
3. For each property pattern, bind the variable to the resource's property value
4. If the property is missing on the resource, the pattern does not match that resource (the variable is unbound for that resource)

**Value equality for equi-joins.** When the evaluator joins two
patterns on a shared variable, two bound values are considered equal
under the same rules as the `=` operator, with one cross-type
exception: `Value::ResourceRef(iri)` and `Value::String(s)` compare
equal when `iri.as_str() == s`. This bridges resources cross-referenced
via typed `ResourceRef` properties against resources whose IRI
appears as a `String`-typed property value (e.g. when the layer
encodes some references as raw strings).

### 6.2 NOT EXISTS

`NOT EXISTS(?var)` evaluates to `true` when the variable's associated property has no value on the matched resource. This allows testing for property absence without an `undefined` literal.

A variable used in `NOT EXISTS` must be bound in a MATCH pattern. The semantics: the pattern matches the resource, but the specific property has no value, so the variable is unbound. `NOT EXISTS(?var)` detects this unbound state.

### 6.3 WHERE filtering

After pattern matching produces bindings, WHERE expressions filter them. A binding is retained only if all WHERE conditions evaluate to `true` for that binding.

### 6.4 GROUP BY and aggregation

If a GROUP BY clause is present:
1. Partition bindings into groups where all GROUP BY expressions have equal values
2. For each group, evaluate RETURN expressions:
   - Non-aggregated expressions take the value from the group key
   - Aggregated expressions (COUNT, SUM, AVG, MIN, MAX) are computed over all bindings in the group

If no GROUP BY is present but aggregates are used in RETURN, all bindings form a single group.

### 6.5 RETURN shaping

For each surviving binding (or group), the RETURN clause constructs a
row resource:
1. Create a new resource with `is_a` set to the result class(es) (or the
   synthesized row class when `RETURN []` is used)
2. For each return item, evaluate the expression against the binding and
   set the property on the row under its synthesized Property IRI

Row resources are wrapped in a `ResultSet` together with a synthesized
row class and Property resources; the complete wire shape is specified
in [Appendix A](#appendix-a-result-documents).

### 6.6 Result modifiers

After RETURN shaping, result modifiers are applied in this order:
1. **DISTINCT** — remove duplicate result resources
2. **ORDER BY** — sort by the specified expressions
3. **OFFSET** — skip the first N results
4. **LIMIT** — take at most N results

### 6.7 DEFINE and fixpoint evaluation

**Non-recursive rules.** A DEFINE without self-reference evaluates in a single pass — the match_part runs once against the layer chain and the results form the derived relation.

**Recursive rules.** A DEFINE that references itself (directly or transitively through other DEFINEs) is evaluated using **bottom-up seminaive evaluation**:

1. Start with base facts from the layer chain
2. Apply all rules, adding newly derived facts to the derived relations
3. Repeat until no new facts are derived (fixpoint reached)

Seminaive optimization: in each iteration, only process combinations involving at least one fact derived in the previous iteration, avoiding redundant recomputation.

**Termination.** Fixpoint evaluation terminates because:
- The set of possible derived facts is bounded (finite resources, finite property values)
- Each iteration adds at least one new fact or terminates
- Negation is stratified (no negation cycles)

### 6.8 Union semantics

Multiple DEFINE clauses with the same relation name define a union. A tuple is in the derived relation if it is produced by any of the definitions. This is standard Datalog union semantics.

### 6.9 Stratified negation

Negated patterns (`NOT ClassName(?var) { ... }`) are subject to **stratification checking**:

1. Build the dependency graph: for each DEFINE, record which relations it references positively and negatively
2. Check for negation cycles: a cycle in the dependency graph that passes through a negative edge is an error
3. Compute strata: order rule evaluation so that negated relations are fully computed before being negated

A program that fails stratification checking is rejected before evaluation begins.

**Stratification example (valid):**

```
DEFINE HasParent(?x) FROM
    MATCH Animal(?parent) { offspring: ?x }

DEFINE Orphan(?x) FROM
    MATCH Animal(?x) {},
    NOT HasParent(?x) {}
```

`Orphan` negates `HasParent`, but `HasParent` does not reference `Orphan` — no negation cycle. `HasParent` is computed first (stratum 1), then `Orphan` (stratum 2).

**Stratification example (invalid — rejected):**

```
DEFINE A(?x) FROM
    MATCH Foo(?x) {},
    NOT B(?x) {}

DEFINE B(?x) FROM
    MATCH Bar(?x) {},
    NOT A(?x) {}
```

`A` negates `B` and `B` negates `A` — negation cycle. This program is rejected.

### 6.10 Layer-aware resolution

Queries execute within an execution context and see exactly the resources visible through the layer chain. Resource resolution follows the same parent-chain walk: the query evaluator checks the top layer first, then walks parents.

### 6.11 Monotonicity

Queries without negation (no `NOT EXISTS`, no negated patterns) are monotonic — adding resources to the layer chain can only increase the result set, never decrease it. This property is significant for caching and incremental evaluation.

Queries using `NOT EXISTS` or negated MATCH patterns (`NOT ClassName(...)`) are not monotonic — adding facts can decrease the result set. The evaluator tracks which queries use negation so that non-monotonic queries can be flagged for full re-evaluation on layer changes.

### 6.12 FIBER evaluation — transient overlay

`FIBER` clauses are processed in textual order interleaved with `MATCH`
clauses. For each surviving binding produced by preceding clauses:

1. Resolve the QueryClass via the `InstitutionIndex`. The cited
   institution agreement (§5.8 step 4) is enforced at type check, so
   evaluation only re-fetches the indexed entry.
2. Look up the declaring `Institution` in the `InstitutionRuntime` by
   the QueryClass's `institution_ref`. A missing runtime registration
   is evaluation error `institution_runtime_missing` (§7).
3. Construct the input resource: `is_a = [query_class.query_class]`
   (the QueryClass's declared input class), then evaluate each `param`
   against the current binding:
   - **Plain expression** params evaluate as in §3.7. The result is
     stored as a property value under the resolved Property IRI
     (short-name params resolved through the QueryClass's
     `requires`/`recommends`; full-IRI params passed through verbatim).
   - **Comorphism coercion** params run the four-step pipeline (D14
     §9.3) inline, see "Comorphism coercion" below.
4. Invoke
   `Institution::query(query_class.query_handler, &input, ctx)`.
5. Attach the response resource to a **query-scoped transient overlay
   layer** sitting on top of the evaluation layer chain. The overlay
   is private to the current query and is discarded when evaluation
   ends. The response resource's IRI is synthesized as
   `urn:eigenius:query:gen:<query-hash>:fiber:<clause-ordinal>:<binding-ordinal>`,
   deterministic within one query but never persisted.
6. Bind `?var` to that response IRI.

Subsequent clauses — including `MATCH ?var { ... }` decomposition —
see the response via normal layer iteration, reusing EigenQL's existing
pattern-match semantics. The overlay reuses the existing layer
machinery rather than introducing a parallel "bound-to-embedded-
resource" code path in the pattern matcher.

**Verdict-shaped responses.** When `query_class.result_class` is
`Verdict`, the response resource's `ctor_name` property carries the
constructor (`"Holds"` / `"Fails"` / `"Undecidable"`) and the IRI is
typed as `Verdict`. The bound variable can then be projected with the
postfix predicate (`?var HOLDS`) or matched directly (`MATCH ?var
{ ctor_name: ?c }`). Both are equivalent ways of reading the
constructor; `?var HOLDS` is the more ergonomic of the two.

**Comorphism coercion in params.** When a param's value is
`comorphism_iri(source_expr)`:

1. Evaluate `source_expr` against the current binding to yield a
   source resource. Non-resource sources are caught at type check
   (§5.8 step 9e).
2. Look up the comorphism in the `InstitutionIndex`. The capability-
   level guard (§5.8 step 9c) ensures the transformation is `Pure` or
   `Read` — IO transformations are rejected before evaluation begins.
3. Run the four-step pipeline:
   - `Institution::extract_typed(export_format.procedure, source, ctx)`
     against the source institution to yield a typed value.
   - Apply the comorphism's `transformation` Component to the typed
     value.
   - `Institution::reify(import_format.procedure, transformed, ctx)`
     against the target institution to yield a target-class resource.
4. Use the reified resource as the param value.

The four-step pipeline runs once per binding per param coercion. There
is no caching across bindings in v1 — the same memoization caveat as
plain FIBER dispatch applies.

**Stratification.** Each FIBER clause forms its own evaluation step.
The total per-query order is:

```
(MATCH | FIBER)+  →  WHERE  →  GROUP BY  →  RETURN  →  ORDER BY  →  LIMIT/OFFSET/DISTINCT
```

Within `(MATCH | FIBER)+`, clauses are processed left to right and a
`FIBER` clause may consume any variable bound by a preceding `MATCH`
or `FIBER`. Forward references are caught at type check.

**Dispatch frequency.** Fiber dispatches happen **once per binding in
the current candidate set**. Comorphism coercions inside a FIBER
clause dispatch alongside it — once per binding, per coercion.
Callers who care about dispatch cost should constrain the candidate
set upstream via `MATCH`.

**Memoization is not supported in v1.** Two identical `FIBER` clauses
with identical parameters dispatch twice. Future work: integrate with
the trace store so repeated fiber dispatches (and the comorphism
sub-dispatches inside them) are cached by `(institution, query class,
param hash)` and recovered on subsequent evaluations.

**Error handling.** A fiber dispatch (or any of its sub-coercions)
that returns `Err` aborts the whole query with that error surfaced as
the query's error message. Per-binding fallbacks (e.g. "filter out
bindings whose fiber query failed") are not supported — predictability
over cleverness.

### 6.13 Institution function-call evaluation

Expressions of the form `ns:local(args...)` dispatch a Decidable
QueryClass through the `InstitutionIndex`. For each evaluated call:

1. Construct the IRI string `ns:local` (or treat the qualified name
   as a full IRI if it parses as one).
2. Look up the IRI in the `InstitutionIndex` as a `QueryClass`. If
   the entry is missing, or its `dispatch_role` set does not include
   `Decidable`, fall through to builtin function dispatch — the
   builtin path then raises evaluation error
   `unknown function: ns:local`.
3. Look up the declaring `Institution` in the `InstitutionRuntime` by
   the QueryClass's `institution_ref`. A missing runtime registration
   is `institution_runtime_missing`.
4. Construct the input resource: `is_a = [query_class.query_class]`,
   plus an `urn:eigenius:institution:decide_args` array property
   carrying the evaluated argument values in positional order. (The
   institution's query handler unpacks this array against its declared
   input class — D14 §9.2.)
5. Invoke
   `Institution::query(query_class.query_handler, &input, ctx)`.
6. Read the response resource's `ctor_name` property to determine the
   `Verdict` constructor. Result is a `Verdict`-typed value carrying
   the constructor and the original response (so subsequent property
   access via `MATCH` / dot-path still works on FIBER-bound Verdicts;
   the in-expression form is typically projected immediately via a
   postfix predicate).

Result type is `Verdict`. Use a postfix predicate (§3.8) to project
to a `boolean`. A bare Verdict expression in a position requiring
Boolean is type error `bare_verdict_in_boolean_position`.

This evaluation requires the kernel runtime's `InstitutionIndex` and
`InstitutionRuntime` to be available. Calls invoked through
`execute(program, layer)` (no runtime) raise `unknown function`
because the classification step has nothing to look up against.

Comorphism dispatch is **not** available in expression position;
comorphisms surface only in FIBER parameter coercion (§3.5, §6.12).

---

## 7. Error Format

Query errors are reported as structured objects:

```typescript
interface QueryError {
  position: { line: number; column: number } | null;
  phase: 'lexer' | 'parser' | 'type_check' | 'stratification' | 'evaluation';
  rule: string;
  message: string;
}
```

Error phases:
- **lexer** — unrecognized token, unterminated string, invalid escape
- **parser** — syntax error, unexpected token, malformed clause
- **type_check** — type mismatch, unbound variable, invalid property
  reference, aggregate without GROUP BY, FIBER schema violation,
  comorphism-coercion well-formedness, Verdict-position rules
  (see §5.7–5.9 for rule names)
- **stratification** — negation cycle in DEFINE rules (§6.9)
- **evaluation** — runtime errors (division by zero, invalid date
  parse, fiber dispatch failure, comorphism dispatch failure, missing
  institution runtime registration, unknown qualified-name function)

Institution-surface rule identifiers used by §5.7–5.9 and §6.12–6.13:

| Rule | Phase | Where |
|------|-------|-------|
| `using_institution_unresolved` | type_check | §5.7 |
| `duplicate_using_institution_alias` | type_check | §5.7 |
| `undeclared_institution_alias` | type_check | §5.8 |
| `fiber_query_class_not_query_class` | type_check | §5.8 |
| `fiber_query_class_not_on_demand` | type_check | §5.8 |
| `fiber_institution_mismatch` | type_check | §5.8 |
| `fiber_param_short_name_unresolved` | type_check | §5.8 |
| `fiber_missing_required_param` | type_check | §5.8 |
| `comorphism_unresolved` | type_check | §5.8 |
| `comorphism_target_mismatch` | type_check | §5.8 |
| `comorphism_io_not_supported_in_v1` | type_check | §5.8 |
| `comorphism_target_class_mismatch` | type_check | §5.8 |
| `comorphism_source_not_resource` | type_check | §5.8 |
| `qualified_call_not_decidable` | type_check | §5.9 |
| `verdict_predicate_non_verdict_operand` | type_check | §5.9 |
| `bare_verdict_in_boolean_position` | type_check | §5.9 |
| `institution_runtime_missing` | evaluation | §6.12, §6.13 |
| `unknown function` | evaluation | §6.13 |

Each error carries a `rule` identifier so consumers can react
programmatically without parsing the human-readable `message`.

---

## 8. Examples

### 8.1 Find all classes

```
USING "urn:eigenius:core:Class"
MATCH Class(?c) {
    description: ?desc,
    short_name: ?name
}
RETURN Class {
    short_name: ?name,
    description: ?desc
}
```

### 8.2 Find all dogs (with inheritance)

```
USING "urn:eigenius:example:Dog"
MATCH Dog(?d) {
    name: ?name,
    breed: ?breed
}
RETURN Dog {
    name: ?name,
    breed: ?breed
}
```

### 8.3 Guard query (no RETURN)

```
USING "urn:eigenius:example:Dog"
MATCH Dog(?d) {
    breed: ?breed
}
WHERE ?breed = "German Shepherd"
```

### 8.4 Cross-pattern join

```
USING "urn:eigenius:example:Person", "urn:eigenius:example:Dog"
MATCH Person(?p) {
    name: ?owner_name,
    pet: ?d
},
Dog(?d) {
    name: ?dog_name,
    breed: ?breed
}
RETURN [] {
    owner: ?owner_name,
    dog: ?dog_name,
    breed: ?breed
}
```

### 8.5 Full IRI references (no USING)

```
MATCH "urn:eigenius:example:Dog"(?d) {
    "urn:eigenius:example:name": ?name,
    "urn:eigenius:example:breed": ?breed
}
RETURN [] {
    "urn:eigenius:example:name": ?name,
    "urn:eigenius:example:breed": ?breed
}
```

### 8.6 Dot-path navigation

```
USING "urn:eigenius:example:Person"
MATCH Person(?p) {
    name: ?name
}
RETURN [] {
    name: ?name,
    city: ?p.address.city
}
```

Equivalent decomposed form:

```
USING "urn:eigenius:example:Person"
MATCH Person(?p) {
    name: ?name,
    address: ?addr
},
?addr {
    city: ?city
}
RETURN [] {
    name: ?name,
    city: ?city
}
```

### 8.7 NOT EXISTS (property absence)

```
USING "urn:eigenius:core:Property"
MATCH Property(?p) {
    short_name: ?name,
    domain: ?domain
}
WHERE NOT EXISTS(?domain)
RETURN [] {
    short_name: ?name
}
```

### 8.8 Aggregation with GROUP BY

```
USING "urn:eigenius:example:Dog"
MATCH Dog(?d) {
    breed: ?breed
}
GROUP BY ?breed
RETURN [] {
    breed: ?breed,
    count: COUNT(?d)
}
ORDER BY COUNT(?d) DESC
LIMIT 10
```

### 8.9 Aggregation without GROUP BY

```
USING "urn:eigenius:example:Dog"
MATCH Dog(?d) {
    name: ?name
}
RETURN [] {
    total: COUNT(?d),
    names: CONCAT(?name)
}
```

### 8.10 Result modifiers

```
USING "urn:eigenius:core:Property"
MATCH Property(?p) {
    short_name: ?name,
    data_type: ?dt
}
RETURN [] {
    short_name: ?name,
    data_type: ?dt
}
ORDER BY ?name ASC
LIMIT 20
OFFSET 10
DISTINCT
```

### 8.11 Recursive rule (transitive closure)

```
USING "urn:eigenius:example:Employee"

DEFINE Ancestor(?person, ?ancestor) FROM
    MATCH Employee(?person) {
        reports_to: ?ancestor
    }

DEFINE Ancestor(?person, ?ancestor) FROM
    MATCH Employee(?person) {
        reports_to: ?manager
    },
    Ancestor(?manager, ?ancestor) {}

MATCH Ancestor(?a) {}
WHERE ?a = "urn:eigenius:example:alice"
RETURN [] {
    ancestor: ?a
}
```

### 8.12 Negated pattern

```
USING "urn:eigenius:example:Animal"

DEFINE HasOffspring(?parent) FROM
    MATCH Animal(?child) {
        parent: ?parent
    }

DEFINE ChildlessAnimal(?a) FROM
    MATCH Animal(?a) { name: ?name },
    NOT HasOffspring(?a) {}

MATCH ChildlessAnimal(?a) { name: ?name }
RETURN [] {
    name: ?name
}
```

### 8.13 Decidable QueryClass with postfix predicate

A `Decidable` `QueryClass` invoked in expression position returns a
`Verdict`. The postfix predicate projects to Boolean for use in
`WHERE`:

```
USING "urn:eigenius:example:DockingResult" AS dock_class

MATCH dock_class(?d) { score: ?s }
WHERE  dock:within_tolerance(?d, 0.85) HOLDS
RETURN [] { d: ?d, score: ?s }
```

`dock:within_tolerance` resolves through the `InstitutionIndex` to a
`QueryClass` whose `dispatch_role` includes `Decidable`. The
institution returns `Holds`/`Fails`/`Undecidable`; only `Holds`
survives the WHERE filter. To accept Undecidable results too, the
user writes `NOT dock:within_tolerance(?d, 0.85) FAILS`.

### 8.14 Comorphism coercion in a FIBER parameter

A FIBER param coerces a value across an institution boundary by
running a comorphism inline. `?dock_result` is a resource in the
docking institution's fiber; the comorphism translates it into the
class the assay institution's QueryClass expects:

```
USING INSTITUTION "urn:eigenius:dock:Inst"  AS dock
USING INSTITUTION "urn:eigenius:assay:Inst" AS assay

MATCH DockingResult(?d) { binding_pose: ?p }
FIBER assay:CheckBindingAffinity {
    candidate: dock:dock_to_assay(?d),       // comorphism coercion
    threshold: 0.85
} AS ?check
WHERE ?check HOLDS
RETURN [] { d: ?d }
```

The comorphism `dock:dock_to_assay` must declare an `import_format`
whose `institution_ref` is `assay` (so the coercion lands in the
right fibre) and whose `to_class` matches the type
`assay:CheckBindingAffinity` declares for `candidate`. Both
constraints are checked at type-check time (§5.8 step 9). The
transformation Component must be Pure or Read in v1 (no IO).

`?check` carries `result_class = Verdict`. The FIBER-bound variable
can be projected with `?check HOLDS` (§3.8) or matched directly:
`MATCH ?check { ctor_name: ?c }`.

---

## 9. Decisions Log

| Question | Decision | Rationale |
|----------|----------|-----------|
| `undefined` literal | Removed; use `NOT EXISTS(?var)` instead | Eigon-JSON has no null; testing for absence is clearer with explicit syntax |
| Property reference without USING | Always available via full IRI as quoted string; USING enables shortname convenience | Full IRI is the canonical form; USING is sugar |
| Dot-path navigation | Shortname-only sugar over multi-pattern joins; full IRI uses decomposed patterns | Keeps grammar simple; dot-paths resolve against class property sets |
| Result modifiers | DISTINCT, ORDER BY, LIMIT, OFFSET included in v1 | Essential for practical use |
| Aggregation | COUNT, SUM, AVG, MIN, MAX with GROUP BY in v1 | Essential for analytics queries |
| String concatenation `\|\|` | Added at additive precedence level | Consistent with SQL convention |
| `AS` keyword | Reserved but unused in grammar | Available for future aliasing syntax |
| Recursive rules | DEFINE with self-reference, seminaive fixpoint evaluation | Standard Datalog; enables transitive closure and derived relations |
| Negated patterns | `NOT ClassName(...)` in MATCH, with stratification checking | Stratified negation prevents paradoxes; well-understood theory |
| Monotonicity tracking | Queries with negation flagged as non-monotonic | Enables correct cache invalidation on layer changes |
| FIBER clause placement | Top-level query only; not in DEFINE bodies | Keeps derived relations pure and stratifiable; institution dispatch is an orchestration concern, not a fixpoint participant |
| FIBER overlay model | Response resources attached to a transient overlay layer; subsequent MATCH iterates layers normally | Reuses existing layer machinery rather than special-casing FIBER-bound variables in the pattern matcher |
| Full-IRI FIBER params | Bypass class-scope validation (open-world) | Consistent with §3.4's treatment of full-IRI property references in MATCH; clients can pass extra info the institution opted into |
| FIBER memoization | Not in v1 — every binding triggers a fresh dispatch | Predictable cost model; future trace-store integration tracked separately |
| FIBER dispatch order | Textual order, interleaved with MATCH | Forward references caught at type check; lets queries stage results from one institution into the next |
| Decidable-call result type | `Verdict`, not Boolean | D14 keeps Holds/Fails/Undecidable distinct. Boolean is recovered explicitly with a postfix predicate (`HOLDS`/`FAILS`/`UNDECIDABLE`); the user can no longer accidentally fold Undecidable into "false" |
| Bare Verdict in Boolean position | Type error `bare_verdict_in_boolean_position` | Forces the user to choose how Undecidable behaves rather than collapsing silently. Pairs with the postfix predicate as the canonical projection |
| Postfix predicate scope (v1) | Verdict-specific | Generalising to any inductive value's `ctor_name` is future-extensible; v1 keeps the type rule sharp |
| Comorphism in expression position | Dropped (use FIBER param coercion) | The four-step pipeline (extract → transformation → reify) is the wrong shape for an expression-position function call. FIBER parameter coercion is the natural surface — comorphism feeds a value from one institution into a query in another |
| Comorphism IO transformation in v1 | Rejected (`comorphism_io_not_supported_in_v1`) | Initial version restricts comorphism transformations to Pure/Read so EigenQL can run the coercion inline without standing up an IO evaluator. The surface is unchanged when this lifts |
| Qualified-name unrecognised IRIs | Evaluation error `unknown function`, not type error | Allows late registration of institutions and gives the same diagnostic shape as misspelt builtins |
| ESL/EigenQL classification | Both consult the `InstitutionIndex`; classification is per-call (Comorphism vs. Decidable QueryClass) rather than via a separate "classify" table | Single source of truth (the index, derived from the chain) supersedes the prior `InstitutionRegistry::classify` route. ESL and EigenQL agree because they read the same data |
| `**` associativity | Left in current parser (`(a**b)**c`) | Implementation pragmatism; convention is right-associative — users should parenthesise stacked exponents |
| `IN` / `LIKE` RHS | Any `additive_expr` (not restricted to array / string literal) | Allows variables and computed values; rejected types fail at evaluation |

---

## Appendix A. Result Documents

A query response is an **Eigon document** — the same wire shape as a Load
payload, a CBOR array of top-level resources — but it is not added to any
layer. Consumers treat it as transient data; clients that want to make it
durable can feed the identical bytes back into `Load`.

Everything in the response is first-class knowledge-graph content: row
values, their class, and the class's property definitions are all
resources with their own IRIs. Clients do not hardcode result keys; they
discover them by walking the included class. This closes
[issue #9](https://github.com/eigenius/eigenius/issues/9).

### A.1 Resources in the document

A response to a `RETURN`-bearing query contains four categories of
resource:

1. **Row `Property` resources** — one per `RETURN` item. Each carries
   `short_name` (the bare name the user typed), `datatype` (inferred
   per §5), and sits under `urn:eigenius:core:Property`.
2. **Row `Class` resource** — lists the properties above and carries a
   `short_name` derived from the `RETURN` class names (or a generated
   `QueryRow` if the `RETURN []` form is used).
3. **Row resources** — one per surviving binding. `is_a` includes the row
   class IRI. Each property value is stored under the synthesized
   Property IRI.
4. **`ResultSet` resource** — wraps the response. Carries:
   - `urn:eigenius:query:result_class` → row class IRI
   - `urn:eigenius:query:rows` → list of row resource IRIs (or embedded
     row resources; see A.4)
   - `urn:eigenius:query:row_count`, `urn:eigenius:query:elapsed_ms`, etc.
     for introspection

Match-only queries (no `RETURN`) return a `ResultSet` with an empty row
class and a boolean `urn:eigenius:query:matched` property. No Property
resources are synthesized.

### A.2 IRI synthesis

All synthesized IRIs live under the `urn:eigenius:query:gen:<hash>:`
namespace. `<hash>` is derived from the query text (a stable hash of the
canonicalized AST in v1; the exact hash function is an implementation
concern, not part of this spec). Within one response:

- `urn:eigenius:query:gen:<hash>:result` — the ResultSet
- `urn:eigenius:query:gen:<hash>:row_class` — the row class
- `urn:eigenius:query:gen:<hash>:row:<short_name>` — each Property
- `urn:eigenius:query:gen:<hash>:row:<n>` — the nth row resource

Re-running the same query produces identical IRIs. This is not a
persistence guarantee — nothing in the kernel retains these IRIs between
requests.

### A.3 Property datatype derivation

Each row Property's `datatype` is the inferred type of its `RETURN`
expression per §5. Aggregate functions have fixed datatypes:

| Expression | Property datatype |
|------------|-------------------|
| `COUNT(_)` | `urn:eigenius:core:Integer` |
| `SUM(?x)` where `?x : Integer` | `urn:eigenius:core:Integer` |
| `SUM(?x)` otherwise | `urn:eigenius:core:Float` |
| `AVG(?x)` | `urn:eigenius:core:Float` |
| `MIN(?x)`, `MAX(?x)` | same as datatype of `?x` |

Non-aggregate expressions inherit their datatype from the type checker's
existing inference for that expression shape.

### A.4 Access pattern

A consumer reads a result document like any other Eigon document:

1. Parse the response bytes as an Eigon document.
2. Find the resource with `is_a` including `urn:eigenius:query:ResultSet`.
3. Follow `urn:eigenius:query:result_class` to the row class.
4. Read the class's `urn:eigenius:core:properties` — a list of Property
   IRIs. For each, read its `urn:eigenius:core:short_name` to build a
   `short_name → iri` map.
5. Iterate `urn:eigenius:query:rows` and, for each row, look up values
   by IRI (not short name). Map them to short names using the table
   from step 4 when displaying to users or building client-side
   structures.

Clients never have to guess IRI conventions. Removing a `RETURN` item
changes the included Property resources, which consumers see
immediately; adding a computed column gives it a Property resource with
the inferred datatype.

### A.5 Non-goals for v1

The following are out of scope for this spec and tracked as future work:

- **Persistence of result classes.** Result documents are transient.
  Adding the same query's class to a durable layer for re-use is a
  separate concern (candidate future issue: "promote a ResultSet to a
  layer").
- **Cross-query class deduplication.** Two queries with the same RETURN
  shape generate distinct row classes today. Deduplication requires a
  canonicalization policy over the AST and is deferred.
- **Streaming pagination.** The wire transport returns a whole document
  per response. Large result sets may later require chunking, which is
  an implementation detail not mandated by this spec.
- **Result-set arithmetic.** Combining, filtering, or joining result
  sets is the job of a follow-up query; there is no fused "run these
  two queries and join their results" operator in v1.

### A.6 Evolution

If a future version of the spec introduces durable result classes,
cross-query deduplication, or streamed pagination, the wire shape can
evolve without breaking consumers that treat responses as opaque
documents. Client code that drives from the class's property list,
rather than from hardcoded IRIs, continues to work across both models.

