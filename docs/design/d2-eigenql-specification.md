# D2: EigenQL v1 Specification

*Design document for the Eigenius project — April 2026*

**Status:** Draft
**Required before:** Phase 1 implementation
**Resolves:** Final EBNF grammar, keyword choices, escaping rules, type checking rules, error message format

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

A query without a RETURN clause is a **match query** — it evaluates to a boolean (does any matching assignment exist?). This form is used as guard conditions in DAG Select constructs.

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
MATCH  WHERE  RETURN  USING  AS  DEFINE  FROM
AND  OR  NOT  IN  LIKE  EXISTS
GROUP  BY  ORDER  ASC  DESC
DISTINCT  LIMIT  OFFSET
```

Built-in function names are also keywords:

```
DATE  TIMESTAMP  REGEX  LENGTH  CONTAINS  CONCAT
COUNT  SUM  AVG  MIN  MAX
```

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
match_part  ::= using_clause? match_clause where_clause?
```

### 3.2 DEFINE clause

```ebnf
define_clause ::= 'DEFINE' IDENTIFIER '(' variable_list ')' 'FROM' match_part
variable_list ::= variable (',' variable)*
```

A DEFINE clause introduces a named derived relation. The relation name is an identifier. The variable list declares the relation's arity and column names.

Multiple DEFINE clauses with the same name define a **union** — a resource matches the relation if it matches any definition. Self-reference in a DEFINE's MATCH clause creates **recursion**.

**Negation in MATCH patterns.** Within a DEFINE's MATCH clause (and in the final query's MATCH clause), a pattern may be prefixed with `NOT` to express negation:

```ebnf
pattern ::= typed_pattern | untyped_pattern | negated_pattern
negated_pattern ::= 'NOT' (typed_pattern | untyped_pattern)
```

A negated pattern succeeds when no matching resource exists. Negation is subject to stratification checking (see §6.9).

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
using_clause       ::= single_using+
single_using       ::= 'USING' string_list
string_list        ::= STRING (',' STRING)*
```

The USING clause imports ontology classes by IRI for shortname reference within the query. Each IRI must resolve to a valid Class resource in the current layer chain. Shortnames are derived from the `short_name` property of the resolved resource.

USING is a convenience — it enables bare identifier references like `Dog` instead of `"urn:eigenius:example:Dog"`. Without USING, any class or property can still be referenced by its full IRI as a quoted string.

Multiple USING clauses are allowed and their imports are merged (duplicates removed). Shortnames must be unique within the query scope; a duplicate shortname is an error.

### 3.4 MATCH clause

```ebnf
match_clause  ::= 'MATCH' pattern_list
pattern_list  ::= pattern (',' pattern)*

pattern       ::= typed_pattern | untyped_pattern
typed_pattern ::= name '(' variable ')' object_pattern
untyped_pattern ::= variable object_pattern

object_pattern ::= '{' property_list? '}'
property_list  ::= property (',' property)*
property       ::= name ':' value

name          ::= IDENTIFIER | STRING
```

**Typed patterns** constrain matches to instances of the named class (and its subclasses via `subclass_of`). The class is referenced by shortname (resolved from USING imports) or by full IRI (as a string literal). Properties referenced in a typed pattern must be valid for the specified class, accounting for property inheritance through the subclass chain.

**Untyped patterns** match any resource. Properties are not validated against a class schema.

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

### 3.5 WHERE clause

```ebnf
where_clause    ::= 'WHERE' expression_list
expression_list ::= expression (',' expression)*
```

Multiple expressions in the WHERE clause are implicitly ANDed.

### 3.6 Expressions

Operator precedence (highest to lowest):

| Precedence | Operators | Associativity |
|------------|-----------|---------------|
| 1 | Unary `NOT`, `+`, `-` | Right |
| 2 | `**` | Left |
| 3 | `*`, `/`, `%` | Left |
| 4 | `+`, `-`, `\|\|` | Left |
| 5 | `<`, `<=`, `>`, `>=`, `IN`, `NOT IN`, `LIKE`, `NOT LIKE` | Left |
| 6 | `=`, `<>` | Left |
| 7 | `AND` | Left |
| 8 | `OR` | Left |

```ebnf
expression      ::= or_expr
or_expr         ::= and_expr ('OR' and_expr)*
and_expr        ::= equality_expr ('AND' equality_expr)*
equality_expr   ::= relational_expr (('=' | '<>') relational_expr)*
relational_expr ::= additive_expr (('<' | '<=' | '>' | '>=' | comparison_op) additive_expr)*
additive_expr   ::= mult_expr (('+' | '-' | '||') mult_expr)*
mult_expr       ::= power_expr (('*' | '/' | '%') power_expr)*
power_expr      ::= unary_expr ('**' unary_expr)*
unary_expr      ::= primary_expr
                   | 'NOT' unary_expr
                   | 'NOT' 'EXISTS' '(' variable ')'
                   | '+' unary_expr
                   | '-' unary_expr

comparison_op   ::= 'IN' | 'NOT' 'IN' | 'LIKE' | 'NOT' 'LIKE'

primary_expr    ::= value
                   | '(' expression ')'
                   | array_literal
                   | object_literal
                   | function_call
                   | aggregate_call
                   | variable '.' IDENTIFIER ('.' IDENTIFIER)*
```

### 3.7 Function calls

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

### 3.8 Aggregate functions

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

### 3.9 Literals and values

```ebnf
value         ::= literal | variable
literal       ::= STRING | NUMBER | BOOLEAN

variable      ::= VARIABLE

array_literal  ::= '[' ']' | '[' expression_list ']'
object_literal ::= '{' '}' | '{' object_property_list '}'

object_property_list ::= object_property (',' object_property)*
object_property      ::= name ':' expression
```

### 3.10 RETURN clause

```ebnf
return_clause      ::= 'RETURN' return_class_names '{' return_list '}'
return_class_names ::= name | '[' name_list ']' | '[' ']'
return_list        ::= return_item (',' return_item)*
return_item        ::= name ':' expression
name_list          ::= name (',' name)*
```

The return class determines which properties are valid in the output. The result class may be a single class name, an array of class names (for resources with multiple class membership), or empty (untyped result).

Each return item maps a property name to an expression. Expression types must match the declared property data types.

### 3.11 GROUP BY clause

```ebnf
group_by_clause ::= 'GROUP' 'BY' expression_list
```

Groups result rows by the specified expressions before applying aggregation. All non-aggregated expressions in the RETURN clause must appear in the GROUP BY clause.

### 3.12 ORDER BY clause

```ebnf
order_by_clause ::= 'ORDER' 'BY' order_item (',' order_item)*
order_item      ::= expression ('ASC' | 'DESC')?
```

Sorts results by the specified expressions. Default sort direction is ASC. Expressions must reference variables or properties present in the RETURN clause.

### 3.13 LIMIT and OFFSET

```ebnf
limit_clause  ::= 'LIMIT' NUMBER
offset_clause ::= 'OFFSET' NUMBER
```

LIMIT restricts the number of results returned. OFFSET skips the first N results. Both values must be non-negative integers. OFFSET without LIMIT is allowed.

### 3.14 DISTINCT

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
  match: MatchPart;             // The match_part (using + match + where)
}

interface MatchPart {
  using: string[];              // IRI strings from USING clause
  patterns: Pattern[];          // MATCH patterns
  conditions: Expression[];     // WHERE conditions
}

interface Query {
  match: MatchPart;             // USING + MATCH + WHERE
  groupBy: Expression[];        // GROUP BY expressions
  resultClasses: Name[];        // RETURN class names
  result: RenameResult[];       // RETURN property mappings
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

interface RenameResult {
  name: Name;
  expression: Expression;
}

type Expression =
  | { value: unknown }                                    // Literal value
  | { variable: Variable }                                // Variable reference
  | { binary: BinaryOperator; left: Expression; right: Expression }
  | { unary: UnaryOperator; operand: Expression }
  | { notExists: Variable }                               // NOT EXISTS(?var)
  | { function: string; arguments: Expression[] }         // Function call
  | { aggregate: AggregateOp; argument: Expression }      // Aggregate function
  | { path: Variable; segments: string[] }                // Dot-path: ?var.a.b
  | { array: Expression[] }                               // Array literal
  | { object: { key: Name; value: Expression }[] }        // Object literal

type BinaryOperator =
  | '=' | '<>' | '<' | '<=' | '>' | '>='
  | '+' | '-' | '*' | '/' | '%' | '**'
  | '||'
  | 'and' | 'or'
  | 'in' | 'not in' | 'like' | 'not like';

type UnaryOperator = 'not' | '+' | '-';

type AggregateOp = 'count' | 'sum' | 'avg' | 'min' | 'max';
```

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

---

## 6. Evaluation Semantics

### 6.1 Pattern matching

Each MATCH pattern generates a set of **bindings** — assignments of variables to values. Multiple patterns are joined by shared variables (equi-join). The result is the set of all consistent binding combinations across all patterns.

For a typed pattern `ClassName(?var) { prop1: ?a, prop2: ?b }`:
1. Iterate over all resources in the layer chain where `is_a` includes `ClassName` (or a subclass)
2. For each matching resource, bind `?var` to the resource's IRI
3. For each property pattern, bind the variable to the resource's property value
4. If the property is missing on the resource, the pattern does not match that resource (the variable is unbound for that resource)

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

For each surviving binding (or group), the RETURN clause constructs a result resource:
1. Create a new resource with `is_a` set to the result class(es)
2. For each return item, evaluate the expression against the binding and set the property

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

---

## 7. Error Format

Query errors are reported as structured objects:

```typescript
interface QueryError {
  position: { line: number; column: number } | null;
  phase: 'lexer' | 'parser' | 'type_check' | 'evaluation';
  rule: string;
  message: string;
}
```

Error phases:
- **lexer** — unrecognized token, unterminated string
- **parser** — syntax error, unexpected token
- **type_check** — type mismatch, unbound variable, invalid property reference, aggregate without GROUP BY
- **stratification** — negation cycle in DEFINE rules
- **evaluation** — runtime errors (division by zero, invalid date parse, fixpoint non-convergence)

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
