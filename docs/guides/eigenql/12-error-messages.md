# 11. Error messages

EigenQL errors are structured: each carries a phase, a rule identifier, an optional source position, and a message. The type is [`QueryError`](../../../kernel/src/query/error.rs):

```rust
pub struct QueryError {
    pub position: Option<Position>,
    pub phase: ErrorPhase,    // Lexer | Parser | TypeCheck | Stratification | Evaluation
    pub rule: String,
    pub message: String,
}
```

Errors are grouped by the pipeline phase that produced them. This chapter walks through common errors in each phase with typical messages and how to fix them.

## 12.1. Lexer errors

Thrown by [`lexer::tokenize`](../../../kernel/src/query/lexer.rs). Rare in practice; most surface-level typos parse fine at the lexer level and fail later.

**Unterminated string literal**

```
3:14: [Lexer] unterminated string literal
```

Close the string with a matching `"`. Escapes: `\"`, `\\`, `\n`, `\r`, `\t`, `\uXXXX`.

**Invalid escape**

```
2:22: [Lexer] invalid escape sequence: \q
```

Only the five escapes listed above are recognized.

**Unexpected character**

```
1:5: [Lexer] unexpected character: '@'
```

Characters outside the token set ([chapter 3](03-lexical-structure.md)) are rejected. `@` isn't part of EigenQL.

## 12.2. Parser errors

Thrown during AST construction. These are usually the most common in practice because the grammar is strict about clause order.

**Expected token**

```
5:12: [Parser] expected '{' but found Identifier("retirement")
```

The parser expected a specific token shape and got something else. Most common when:

- `RETURN` is forgotten
- A comma-list is missing a comma
- A pattern's property block is missing `{`
- A qualified name is mistyped (`cap;predicate` instead of `cap:predicate`)

**Expected clause**

```
[Parser] expected MATCH or FIBER clause
```

`MatchPart` saw neither a pattern nor a FIBER keyword. Check the clause at the position.

**Unexpected end of input**

```
[Parser] unexpected end of input; expected '}'
```

The parser ran off the end of the token stream before finishing a structural token. Usually a missing closing brace.

## 12.3. Type-check errors

Thrown by [`type_check::type_check`](../../../kernel/src/query/type_check.rs). All type-check errors are collected and returned together — you'll see multiple in one go.

**`using_unresolved`**

```
[TypeCheck] USING resource 'urn:example:Foo' not found in layer chain
```

The IRI in `USING` doesn't exist in the layer. Check the namespace and spelling, or confirm the layer chain you're querying against actually contains the class.

**`using_not_class`**

```
[TypeCheck] USING resource 'urn:example:Foo' exists but is not a Class
```

The IRI resolves but doesn't have `is_a: Class` in its metadata. `USING` only accepts Classes; for properties or datatype references, use the full IRI directly in patterns.

**`unbound_variable`**

```
[TypeCheck] variable '?breed' used in WHERE but never bound by any MATCH or FIBER
```

Every variable referenced in `WHERE`, `RETURN`, `GROUP BY`, or `ORDER BY` must be bound by a pattern or FIBER clause. Check for typos (`?bread` vs `?breed`) or missing MATCH clauses.

**`aggregate_in_where`**

```
[TypeCheck] aggregate functions (COUNT, SUM, …) are not allowed in WHERE
```

Aggregates only work in `RETURN` (and only when `GROUP BY` is present or the entire query has no non-aggregate items). In `WHERE`, filter first, then aggregate. For "filter groups after aggregation", aggregate in `RETURN` and use `ORDER BY`/`LIMIT` instead.

**`aggregate_without_group_by`**

```
[TypeCheck] RETURN mixes aggregate and non-aggregate expressions without a GROUP BY
```

If `RETURN` has any aggregate, every non-aggregate item in `RETURN` must appear verbatim in `GROUP BY`. Add the missing keys or remove the aggregate.

**`duplicate_using_institution_alias`**

```
[TypeCheck] institution alias 'dock' declared twice
```

Aliases are unique within a `MatchPart`. Rename one.

**`undeclared_institution_alias`**

```
[TypeCheck] FIBER uses institution alias 'dock' which was not declared with USING INSTITUTION
```

Add `USING INSTITUTION "urn:..." AS dock` before the `FIBER` clause.

**`fiber_query_class_not_class`**

```
[TypeCheck] FIBER query class 'PredictBinding' does not resolve to a Class
```

The query class must be a declared `Class` resource. If you're using a short name, check `USING`. If you're using a full IRI, check it exists in the layer.

**`fiber_param_short_name_unresolved`**

```
[TypeCheck] FIBER param 'compund' not declared on query class 'PredictBinding' (requires/recommends)
```

A short-name param must match a `requires` or `recommends` property of the query class. Typical cause: typo ("compund" vs "compound"). Either fix the typo, use the full IRI if you know the property exists under a different short name, or update the class's `requires`/`recommends`.

**`fiber_missing_required_param`**

```
[TypeCheck] FIBER clause missing required param 'receptor' of class 'PredictBinding'
```

Every property in the query class's `requires` must be supplied. Add the missing binding.

## 12.4. Stratification errors

Thrown by [`stratify::stratify`](../../../kernel/src/query/stratify.rs) before type-check. Only one rule:

**`stratification`**

```
[Stratification] negation cycle detected involving relation 'Bad'
```

A `DEFINE` rule depends on its own negation, directly or transitively. See [chapter 10](10-stratification.md). Rewrite to separate the "compute" and "negate" steps into different relations.

## 12.5. Evaluation errors

Thrown during [`evaluate`](../../../kernel/src/query/evaluate.rs). These are the runtime errors that survive type-check — structurally valid queries that fail at data access.

**Unbound variable** (shouldn't happen if type-check passed)

```
[Evaluation] unbound variable: ?name
```

If you see this after type-check, it's a bug — please file an issue.

**FIBER requires registry**

```
[Evaluation] FIBER requires an institution registry — not available in this execution context
```

You called `execute(program, layer)` (no runtime) on a query that uses `FIBER`. Switch to `execute_with(program, layer, FiberRuntime { index: Some(...), runtime: Some(...), components: Some(...), ctx: Some(...), overlay: None })`.

**Institution not registered**

```
[Evaluation] no institution registered for IRI 'urn:eigenius:institutions:docking'
```

The `USING INSTITUTION` alias maps to an IRI but no `Institution` is registered against it in the [`InstitutionRuntime`](../../../kernel/src/institution/runtime.rs). For WASM-runtime institutions the auto-registration scan ([`build_wasm_institution_runtime`](../../../kernel/src/capability/registration.rs)) needs an `Institution` declaration with `runtime: wasm` + inline `wasm_binary` on the chain; for in-process / external institutions, register the runtime entry programmatically before running the query.

**FIBER param unresolvable**

```
[Evaluation] FIBER param 'compound' unresolvable against query class 'PredictBinding'
```

Like the type-check version, but deferred — can happen if the class schema changes between type-check and eval (unusual in practice, since both see the same layer).

**FIBER dispatch failed**

```
[Evaluation] fiber dispatch failed (clause 0): <institution error>
```

The institution's `query()` method returned an error. The inner message comes from the institution — consult its documentation.

**Comorphism arity**

```
[Evaluation] comorphism 'urn:...' expects exactly 1 source argument, got 3
```

Comorphism function calls take exactly one argument. If you need multiple, package them in an embedded resource ahead of time and pass that.

**Unknown function**

```
[Evaluation] unknown function: cap:something
```

Three cases:

1. The IRI isn't registered as a decide predicate or comorphism.
2. The institution registered but the specific procedure IRI wasn't in `decide_procedures` / `comorphism_types`.
3. Typo in the function name.

**Resource / property not found**

```
[Evaluation] resource 'urn:example:missing' not found in layer chain
[Evaluation] property 'name' not found on resource 'urn:example:x'
```

Dot-path (`?x.name`) failed. Check the data — the layer is immutable, so this is always a data issue.

**Variable is not a resource reference**

```
[Evaluation] variable ?x is not a resource IRI
[Evaluation] property 'country' on 'urn:...' is not a resource reference
```

Dot-paths walk resources through properties. At each step the target must be an IRI string. If a segment's value is a literal scalar (string not matching IRI format, integer, etc.), the walk fails.

**WHERE condition must be boolean**

```
[Evaluation] WHERE condition must be boolean
```

A `WHERE` expression evaluated to a non-boolean. Typical cause: using `?x` directly instead of a comparison (`?x = "..."`). Wrap in a comparison, `IN`, `LIKE`, `NOT EXISTS`, etc.

**Arithmetic / type errors**

```
[Evaluation] cannot add String and Integer
[Evaluation] division by zero
[Evaluation] NOT operand must be boolean, got Integer
```

Operand type or value violated. Check the expression with concrete values in mind.

## 12.6. Error phases as a debugging flow

When a query fails, the phase tells you where to look:

| Phase | Likely cause | What to check |
|---|---|---|
| Lexer | Unbalanced string, invalid escape, stray character | The line/column cited |
| Parser | Missing punctuation, wrong clause order, malformed expression | The token structure near the cited position |
| TypeCheck | Undeclared reference, wrong structural shape | USING clauses, variable bindings, FIBER schemas |
| Stratification | Negation cycle in DEFINE rules | Rule dependencies; separate compute/negate |
| Evaluation | Missing data, runtime type mismatch, institution issue | The layer contents, the runtime, the institution state |

When in doubt, start the query without `WHERE`, `GROUP BY`, or `ORDER BY`, and add them back one at a time until the error reappears — this isolates which clause introduced the problem.

---

Next: **[13. Appendix →](13-appendix.md)**
