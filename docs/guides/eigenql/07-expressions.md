# 6. Expressions

Expressions appear in `WHERE` conditions, `RETURN` values, `GROUP BY` keys, `ORDER BY` sort keys, and `FIBER` param bindings. They produce a `Value` — the same resource-value type used by Eigon resources: string, integer, float, boolean, array, or embedded resource.

The expression AST is the `Expression` enum in [kernel/src/query/ast.rs](../../../kernel/src/query/ast.rs); the evaluator is [`eval_expression`](../../../kernel/src/query/evaluate.rs) in `evaluate.rs`. The evaluator threads through the [`FiberRuntime`](../../../kernel/src/query/evaluate.rs) so that institution-dispatched calls and postfix Verdict predicates can resolve against the `InstitutionIndex` + `InstitutionRuntime` and the FIBER overlay.

A binding is a `BTreeMap<String, Value>` — the current variable environment from the surrounding `MATCH` / `FIBER` clauses.

## 7.1. Literals

```rust
Expression::Literal(Literal)
```

Literals convert directly to `Value` via [`literal_to_value`](../../../kernel/src/query/evaluate.rs):

- `Literal::String(s)` → `Value::String(s)`
- `Literal::Integer(n)` → `Value::Integer(n)`
- `Literal::Float(f)` → `Value::Float(f)`
- `Literal::Boolean(b)` → `Value::Boolean(b)`

Examples: `"hello"`, `42`, `3.14`, `true`, `false`.

## 7.2. Variables

```rust
Expression::Variable(Variable)
```

Looks up `binding[var.name]`. Returns the value if bound; raises `QueryError::evaluation("unbound variable: ?name")` if not.

```eigenql
WHERE ?breed = "German Shepherd"
```

Variables appearing in `WHERE` / `RETURN` / `GROUP BY` / `ORDER BY` are checked at type-check time for binding. Unbound variables produce a static error (`unbound_variable`) before evaluation.

## 7.3. Binary operators

```rust
Expression::Binary { op: BinaryOp, left: Box<Expression>, right: Box<Expression> }
```

`BinaryOp` has four groups.

### Comparison

| Op | Symbol | Behavior |
|---|---|---|
| `Eq` | `=` | Equal via [`values_equal`](../../../kernel/src/query/functions.rs) |
| `Neq` | `<>` | Not equal |
| `Lt` | `<` | Less than, via [`values_compare`](../../../kernel/src/query/functions.rs) |
| `Lte` | `<=` | ≤ |
| `Gt` | `>` | > |
| `Gte` | `>=` | ≥ |

`values_equal` and `values_compare` handle cross-type comparisons: integers and floats compare numerically; strings/IRIs compare case-sensitively; arrays and embedded resources compare structurally.

### Arithmetic

| Op | Symbol | Behavior |
|---|---|---|
| `Add` | `+` | Numeric add |
| `Sub` | `-` | Numeric subtract |
| `Mul` | `*` | Numeric multiply |
| `Div` | `/` | Numeric divide |
| `Mod` | `%` | Modulo |
| `Pow` | `**` | Exponentiation (always returns `Float`) |

Type rules: both operands must be numeric (Integer or Float). If both are integers and the result is an integer exactly, the result is `Integer`; otherwise `Float`. `Pow` is the only arithmetic that always returns `Float`. Division by zero produces an evaluation error.

### String

| Op | Symbol | Behavior |
|---|---|---|
| `StringConcat` | `\|\|` | String concatenation; both operands must be `String` |

### Logical and collection

| Op | Symbol | Behavior |
|---|---|---|
| `And` | `AND` | Boolean AND; both operands must be `Boolean` |
| `Or` | `OR` | Boolean OR; same |
| `In` | `IN` | Right operand must be `Array`; checks membership with `values_equal` |
| `NotIn` | `NOT IN` | Negation of `In` |
| `Like` | `LIKE` | SQL-style pattern match via [`like_match`](../../../kernel/src/query/functions.rs); `%` = any sequence, `_` = single char |
| `NotLike` | `NOT LIKE` | Negation of `Like` |

Example:

```eigenql
WHERE ?age > 18 AND ?country IN ["DE", "FR", "IT"]
```

## 7.4. Unary operators

```rust
Expression::Unary { op: UnaryOp, operand: Box<Expression> }
```

| Op | Symbol | Behavior |
|---|---|---|
| `Not` | `NOT` | Boolean negation; operand must be `Boolean` |
| `Pos` | `+` | Converts numeric to `Float` |
| `Neg` | `-` | Arithmetic negation; `Integer` stays integer, `Float` stays float |

```eigenql
WHERE NOT ?retired
WHERE -?delta < 0.1
```

## 7.5. `NOT EXISTS`

```rust
Expression::NotExists(Variable)
```

Returns `Value::Boolean(!binding.contains_key(&var.name))`.

This checks **variable binding presence**, not resource existence. It's true when the variable is *not* bound in the current binding — useful when a pattern produces optional bindings and you want to distinguish bound vs missing.

Contrast with **pattern negation** (`MATCH NOT ?x { ... }`, chapter 5 §5.6), which is a binding-level operation that drops bindings where a pattern matches.

## 7.6. Function calls

```rust
Expression::FunctionCall { name: String, args: Vec<Expression> }
```

A function call has three dispatch paths, tried in order:

1. **Built-in function** — match against a closed set: `DATE`, `TIMESTAMP`, `REGEX`, `LENGTH`, `CONTAINS`, `CONCAT` (§7.6.2). Aggregate functions (`COUNT`, `SUM`, …) parse as a separate `Aggregate` AST node and are covered in §7.7.
2. **Institution-dispatched Decidable QueryClass** — if `name` is a qualified name (`ns:local`) that resolves to a `Decidable` `QueryClass` in the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs), dispatch as `Exp::NativeDecide` (§7.6.1).
3. **Error** — if neither path matches, `call_function` returns `"unknown function: {name}"`.

### 6.6.1. Decidable QueryClass dispatch

When the function name is a qualified name like `cap:within_tolerance` and resolves to a `Decidable` `QueryClass`, the call evaluates to a `Verdict` value (D14 §7.1) — not a Boolean. To use the result as a Boolean, apply a postfix Verdict predicate:

```eigenql
WHERE docking:within_tolerance(?delta, 2.0) HOLDS       -- Boolean true iff Holds
WHERE NOT docking:within_tolerance(?delta, 2.0) HOLDS   -- "Fails or Undecidable"
RETURN [] {
    is_valid: docking:within_tolerance(?delta, 2.0) HOLDS
}
```

Mechanics:

- The kernel resolves the constraint IRI in the `InstitutionIndex`, marshals the positional args into a synthetic input resource (the args attached as `urn:eigenius:institution:decide_args`), looks up the QueryClass's `institution_ref` in the `InstitutionRuntime`, and calls `Institution::query(query_handler, input, ctx)`.
- The institution returns a Verdict resource with `urn:eigenius:core:ctor_name` set to one of `Holds`, `Fails`, `Undecidable`.
- The postfix predicate (`HOLDS` / `FAILS` / `UNDECIDABLE`) projects the Verdict to a Boolean.

A bare Verdict-typed expression in Boolean position (a `qualified_call` not followed by a postfix predicate, used inside `WHERE`/`AND`/`OR`/`NOT`) is a static type error (`bare_verdict_in_boolean_position`). The conversion is always explicit.

The postfix predicate's operand type is checked by the static type checker (`verdict_predicate_non_verdict_operand`) — only two source forms produce a `Verdict`:

1. A `qualified_call` resolving to a Decidable QueryClass.
2. A variable bound by a `FIBER` clause whose `result_class` is `urn:eigenius:institution:Verdict`.

**Comorphisms are not callable from expression position** under D14 — comorphism dispatch surfaces only inside FIBER param value coercion (see [chapter 8 §8.5](08-fiber-clauses.md) and [chapter 9 §9.6](09-institutions.md)).

For the full institution surface, see [chapter 9](09-institutions.md).

### 6.6.2. Built-in functions

All case-sensitive uppercase names.

| Function | Signature | Returns | Behavior |
|---|---|---|---|
| `DATE(s)` | `String` → `String` | Validates ISO 8601 date (`YYYY-MM-DD`). Passes the string through if valid; errors otherwise. |
| `TIMESTAMP(s)` | `String` → `String` | Validates ISO 8601 datetime with timezone. |
| `REGEX(s)` | `String` → `String` | Validates regex syntax. Returns the pattern on success. |
| `LENGTH(x)` | `String` or `Array` → `Integer` | Unicode char count for strings; element count for arrays. |
| `CONTAINS(arr, v)` | `Array × Value` → `Boolean` | Membership check with `values_equal`. |
| `CONCAT(a, b)` | `Array × Array` → `Array` | Concatenates two arrays. Not for strings — use `\|\|` for strings. |

## 7.7. Aggregates

```rust
Expression::Aggregate { op: AggregateOp, arg: Box<Expression> }
```

| Op | Signature | Returns |
|---|---|---|
| `Count` | `Variable` or expression | `Integer` — non-null count |
| `Sum` | `Integer` / `Float` | `Integer` (if all inputs integer and sum exact) or `Float` |
| `Avg` | numeric | `Float` |
| `Min` | any | Min by `values_compare` |
| `Max` | any | Max by `values_compare` |

**Where aggregates are allowed**:

- **Yes**: in `RETURN` expressions, provided `GROUP BY` covers all non-aggregate return items (or the entire query is a single-group aggregation).
- **No**: in `WHERE` (rejected at type-check with `aggregate_in_where`).

Implementation: [`eval_aggregate`](../../../kernel/src/query/evaluate.rs) runs after `apply_group_by` partitions bindings. For each group, the aggregate expression is evaluated across all bindings in the group to produce a single value. The value is stored under a synthetic key (`__agg_<Op>`) in a representative binding and looked up by [`shape_result`](../../../kernel/src/query/evaluate.rs) when constructing the output row.

Attempting to evaluate an `Aggregate` expression outside a `GROUP BY` context produces `"aggregate function outside GROUP BY context"`.

## 7.8. Dot-paths

```rust
Expression::DotPath { root: Variable, segments: Vec<String> }
```

Walks property chains through resources. The root variable must be bound to a resource IRI (as a `Value::String`). Each segment is a property short-name; the evaluator resolves each one against the current resource, moves to the referenced resource for the next segment, and returns the final value.

```eigenql
RETURN [] {
    owner_country: ?dog.owner.country
}
```

**Walk mechanics**:

1. Resolve `?dog` to an IRI.
2. Look up the resource in the layer.
3. Find the property matching `owner` by short name (same lookup as pattern matching — [`find_property_by_shortname`](../../../kernel/src/query/evaluate.rs)).
4. The value must be a resource reference (IRI string). Repeat step 2 with the new IRI.
5. After the final segment, return the raw value — may be a literal or another IRI string.

Errors with `"unbound variable"`, `"resource not found in layer chain"`, `"property 'X' not found on resource 'Y'"`, or `"property is not a resource reference"` when the walk fails.

## 7.9. Arrays

```rust
Expression::Array(Vec<Expression>)
```

Evaluates each element and returns a `Value::Array`. Useful in `IN` predicates and array-typed results.

```eigenql
WHERE ?country IN ["DE", "FR", "IT"]
```

## 7.10. Similarity operator (`~`)

```rust
Expression::Similarity { property: Variable, query: Box<Expression>, hints: HintSet }
```

The `~` binary operator is the D43 fuzzy-retrieval surface: `?property_var ~ "query string"` is true for rows where the property's value is similar to the query under whichever indexes (TextIndex, VectorIndex, or both) the schema declared on that property.

```eigenql
WHERE ?desc ~ "concurrent commit recovery"
```

An optional trailing `{ via, model, k, limit }` hint block overrides individual defaults:

```eigenql
WHERE ?desc ~ "WAL truncation" { via: text }
WHERE ?desc ~ "WAL truncation" { k: 30, limit: 50 }
```

The operator returns Boolean for filtering; the platform-internal similarity score it computed feeds `TOP N`'s implicit ranking (see [chapter 4 §4.10](04-program-structure.md#410-order-by-limit-top-offset-distinct)).

Precedence: relational tier (§7.12). `~` is non-chaining on the left — `?a ~ "x" ~ "y"` is rejected by the parser. The full surface, the hint catalogue, the typecheck rules, and worked examples live in **[chapter 6](06-text-and-vector-retrieval.md)**.

## 7.11. Objects

```rust
Expression::Object(Vec<(Name, Expression)>)
```

Object literals in expression position are **not yet supported** by the evaluator — `eval_expression` returns `"object literals in expressions not yet implemented"`. They're reserved in the AST for future use.

`RETURN [] { ... }` uses a similar-looking object syntax but that's a distinct grammar production: a list of `ReturnItem`, not an expression.

## 7.12. Precedence

From tightest to loosest binding, implemented as the [`parse_*_expr`](../../../kernel/src/query/parser.rs) ladder:

1. **Primary**: literals, variables, function calls, aggregates, arrays, parenthesized, dot-paths
2. **Postfix Verdict predicate** (`HOLDS`, `FAILS`, `UNDECIDABLE`) — non-associative
3. **Unary** (`NOT`, `+`, `-`, `NOT EXISTS`)
4. **Power** (`**`) — left-associative (parser quirk; parenthesise stacked exponents)
5. **Multiplicative** (`*`, `/`, `%`)
6. **Additive** (`+`, `-`, `\|\|`)
7. **Relational** (`<`, `<=`, `>`, `>=`, `IN`, `NOT IN`, `LIKE`, `NOT LIKE`, `~`)
8. **Equality** (`=`, `<>`)
9. **AND**
10. **OR**

The postfix Verdict predicate sits between primary and unary so that `NOT qc:check(?x) HOLDS` parses as `NOT (qc:check(?x) HOLDS)`. Parentheses override precedence: `(?a + ?b) * ?c`.

---

Next: **[8. FIBER clauses →](08-fiber-clauses.md)**
