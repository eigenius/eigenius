# 7. FIBER clauses

`FIBER` clauses extend a query with reasoning delegated to a registered institution. The specification is in [D2 §3.5 + §5.8 + §6.12](../../design/d2-eigenql-specification.md); the implementation is [`apply_fiber_clause`](../../../kernel/src/query/evaluate.rs) in `evaluate.rs`.

The pattern: you declare an institution alias, ask the institution to perform a typed query, and bind the response resource to a variable that subsequent clauses can match against.

## 7.1. Anatomy

```
UsingInstitution ::= 'USING' 'INSTITUTION' StringLit 'AS' ident

FiberClause ::= 'FIBER' institution_ref ':' QueryClass
                '{' ParamBinding (',' ParamBinding)* '}'
                'AS' Variable

ParamBinding ::= Name ':' Expression
```

Where `institution_ref` is either an alias (`ShortName`) or an inline full IRI (`FullIri`).

```eigenql
USING INSTITUTION "urn:eigenius:institutions:docking" AS dock

FIBER dock:PredictBinding {
    compound: ?cpd,
    receptor: ?rec
} AS ?pred
```

**`USING INSTITUTION`** introduces the alias at the top of a `MatchPart`. Aliases are scoped to their `MatchPart` and must be unique within that scope; re-declaring is a type error (`duplicate_using_institution_alias`).

**`FIBER`** performs the dispatch. Its three parts:

1. **`institution:query_class`** — who to ask and what question class.
2. **`{ param: value, ... }`** — the query resource's property bindings.
3. **`AS ?var`** — where to put the response IRI in the current binding.

## 7.2. Evaluation walk-through

For each binding in the current binding set, `apply_fiber_clause`:

1. **Resolves the institution**. Alias lookup in `aliases: BTreeMap<&str, &Iri>`, or direct IRI if the clause gave one inline. The institution must be registered in the `InstitutionRegistry`; otherwise `"no institution registered for IRI '...'"`.
2. **Resolves the query class**. `ShortName` → class IRI in the layer; `FullIri` → used as-is. Must be a `Class`.
3. **Builds a query resource**. Starts with `is_a: [query_class_iri]`. For each `param`:
   - The param name is resolved to a property IRI via [`build_param_iri_table`](../../../kernel/src/query/evaluate.rs) (walks the class's `requires` ∪ `recommends` looking for `short_name` matches).
   - The value is evaluated with the current binding via [`eval_expression`](../../../kernel/src/query/evaluate.rs) — so expressions in param slots see the binding exactly as `WHERE` would.
   - The resulting `(property_iri, value)` pair is set on the query resource.
4. **Dispatches**. Calls `reasoner.query(&query_resource, ctx)` on the institution. The reasoner returns a new `Resource` — its response.
5. **Stamps the response**. The response gets a deterministic IRI via `fp.fiber_response_iri(clause_idx, binding_idx)` — stable per (query text, clause, binding). This lets subsequent queries against the same inputs produce the same overlay resource identity.
6. **Attaches to the overlay**. The stamped resource is pushed into `FiberOverlay::entries` — visible to all subsequent pattern matching in this query only.
7. **Extends the binding**. `?pred` is bound to the response IRI as a `Value::String`.

Step 7 is why subsequent `MATCH` clauses can reference `?pred` just like any resource:

```eigenql
FIBER dock:PredictBinding { compound: ?cpd } AS ?pred
MATCH Prediction(?pred) {
    affinity: ?aff
}
```

The second `MATCH` sees `?pred` as bound, looks up the resource in the overlay (or layer, but the overlay wins for this IRI since it's deterministic per-query), and unifies.

## 7.3. Requirements on the runtime

`FIBER` clauses require both fields of `FiberRuntime` to be present:

```rust
pub struct FiberRuntime<'a> {
    pub institutions: Option<&'a InstitutionRegistry>,
    pub ctx: Option<&'a ExecutionContext>,
}
```

If `institutions` is `None` the clause errors with `"FIBER requires an institution registry — not available in this execution context"`. If `ctx` is `None`, `"FIBER requires an execution context — not available in this execution context"`.

Queries executed via the convenience `execute(program_str, layer)` (no runtime) **cannot use `FIBER`**. Use `execute_with(program_str, layer, FiberRuntime { institutions: Some(reg), ctx: Some(exec_ctx) })` when your query needs it.

## 7.4. The transient overlay

The overlay model (D2 §6.12) isolates FIBER responses from the persistent layer:

- Responses live in `FiberOverlay` for the duration of a single query evaluation.
- Pattern matching in subsequent clauses scans the overlay in addition to the layer (and derived relations).
- The overlay is discarded when `evaluate()` returns. Nothing is committed to the layer.

This lets queries use institution reasoning without side effects. Two successive runs of the same query produce identical results only if the institution's `query()` is deterministic — the overlay IRIs themselves are stable because `fp.fiber_response_iri` is deterministic, but the institution is free to return different responses.

## 7.5. FIBER vs. decide vs. comorphism

Three ways a query can involve an institution. They have different shapes and semantics:

| Mechanism | Syntax | Returns | When to use |
|---|---|---|---|
| `FIBER` clause | `FIBER inst:QueryClass { params } AS ?var` | Binds a response resource | When the institution's result is a structured resource with multiple properties that subsequent clauses want to pattern-match |
| Decide predicate | `inst:predicate(args)` in `WHERE` | `Value::Boolean` | When you just want yes/no filtering |
| Comorphism | `inst:translate(source)` in `RETURN` | `Value::Embedded(resource)` | When you need to translate a resource across an institution boundary and include the result in output |

A query can mix all three. FIBER is the most powerful shape because the bound variable participates in later pattern matching; decide/comorphism are simpler calls that produce one value.

## 7.6. Type-checking rules

The type checker ([kernel/src/query/type_check.rs](../../../kernel/src/query/type_check.rs)) validates:

- **`undeclared_institution_alias`** — the alias in `FIBER alias:Q { … }` wasn't declared with `USING INSTITUTION`.
- **`duplicate_using_institution_alias`** — two `USING INSTITUTION` clauses in the same `MatchPart` use the same alias.
- **`fiber_query_class_not_class`** — the query class IRI doesn't resolve, or resolves to something that isn't a `Class`.
- **`fiber_param_short_name_unresolved`** — a param short name isn't a property that `requires` or `recommends` declares on the query class.
- **`fiber_missing_required_param`** — a `requires` property of the query class wasn't supplied in the param list.

The check runs before evaluation, so malformed FIBER clauses fail fast at compile time.

## 7.7. Determinism and the response IRI

Response IRIs are generated by [`QueryFingerprint::fiber_response_iri(clause_idx, binding_idx)`](../../../kernel/src/query/document.rs) — a deterministic function of the query text (hashed to a fingerprint), the clause index within the query, and the binding index within the clause's iteration.

This matters because:

1. **Reproducibility**: re-running the same query over the same layer produces the same overlay IRIs (institution response content may vary if the institution is non-deterministic, but IRIs are stable).
2. **Caching**: higher layers that cache query results can key on both the query text and the overlay IRI scheme.
3. **Debugging**: a response IRI contains the query hash, so error messages reference traceable identities.

The response IRI format is `urn:eigenius:query:gen:<8-hex>:fiber:<clause>:<binding>`.

## 7.8. Example: multi-step institution query

```eigenql
USING "urn:eigenius:example:Compound"
USING INSTITUTION "urn:eigenius:institutions:docking" AS dock
USING INSTITUTION "urn:eigenius:institutions:assay" AS assay

MATCH Compound(?c) {
    smiles: ?smiles
}
FIBER dock:PredictBinding {
    compound: ?c
} AS ?pred
FIBER assay:EstimateIC50 {
    docking_prediction: ?pred
} AS ?ic50

RETURN [CombinedPrediction] {
    compound: ?c,
    docking_pred: ?pred,
    assay_estimate: ?ic50
}
```

For each compound: ask the docking institution to predict binding affinity, then pass that prediction to the assay institution to estimate IC₅₀. Both responses flow into the result row.

Behaviour notes:

- The second `FIBER` sees `?pred` (bound by the first) as a regular variable — the overlay response is referenceable by IRI anywhere a resource would be expected.
- If `?c` yields 100 bindings, 100 `PredictBinding` dispatches + 100 `EstimateIC50` dispatches happen sequentially. FIBER is not batched yet.

---

Next: **[8. Institutions →](08-institutions.md)**
