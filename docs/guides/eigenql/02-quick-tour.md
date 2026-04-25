# 2. Quick tour

Seven worked examples covering the shapes you'll encounter most often. Each example is drawn from or closely mirrors a test in [kernel/src/query/evaluate.rs](../../../kernel/src/query/evaluate.rs) — you can run them against the same test layer (core ontology + `ontologies/examples/animals.json`).

## 2.1. List all classes in the layer

The simplest meaningful query: find every `Class` resource and return its short name.

```eigenql
USING "urn:eigenius:core:Class"

MATCH Class(?c) {
    short_name: ?name
}
RETURN [] {
    short_name: ?name
}
```

**What happens**:

- `USING "urn:eigenius:core:Class"` imports the `Class` class so we can refer to it by its short name (`Class`) in the `MATCH`.
- `MATCH Class(?c) { short_name: ?name }` scans the layer for resources whose `is_a` includes `Class`, binds the resource IRI to `?c`, and the value of its `short_name` property to `?name`.
- `RETURN [] { short_name: ?name }` emits one result row per binding. `[]` means "no class tag on the result rows" (aka a plain row-shape); the object `{ short_name: ?name }` gives the row one property named `short_name`.

**Where it lives in the code**: this is the [`find_all_classes`](../../../kernel/src/query/evaluate.rs) test.

## 2.2. Filter matches by a property value

Match every `Dog` resource, then filter to the German Shepherd breed.

```eigenql
MATCH "urn:eigenius:example:Dog"(?d) {
    "urn:eigenius:example:breed": ?breed
}
WHERE ?breed = "German Shepherd"
RETURN [] {
    "urn:eigenius:example:breed": ?breed
}
```

**What happens**:

- No `USING` is strictly required when using full IRIs directly — `"urn:eigenius:example:Dog"` is a `Name::FullIri`.
- The `MATCH` binds `?d` to each Dog resource and `?breed` to its breed value.
- `WHERE ?breed = "German Shepherd"` filters to only bindings where the breed equals the literal string.
- `RETURN` emits one row per surviving binding with the breed as its single property.

This is the [`where_filtering`](../../../kernel/src/query/evaluate.rs) test.

## 2.3. Pattern matching on property names with `LIKE`

Find properties whose short name starts with `data_`.

```eigenql
USING "urn:eigenius:core:Property"

MATCH Property(?p) {
    short_name: ?name
}
WHERE ?name LIKE "data_%"
RETURN [] {
    short_name: ?name
}
```

**What happens**: `LIKE` is SQL-style pattern matching — `%` matches any sequence, `_` matches a single character. See [`like_match`](../../../kernel/src/query/functions.rs) for the implementation. Picks up `data_type`, etc.

## 2.4. Recursive derivation with `DEFINE`

Build the ancestor relation from a direct `reports_to` chain.

```eigenql
DEFINE Ancestor(?x, ?z) FROM
    MATCH ?x { "urn:eigenius:test:reports_to": ?z }

DEFINE Ancestor(?x, ?z) FROM
    MATCH ?x { "urn:eigenius:test:reports_to": ?y },
          Ancestor(?y) { "urn:eigenius:test:reports_to": ?z }

MATCH ?person {}
WHERE ?person = "urn:eigenius:test:alice"
RETURN [] {}
```

**What happens**:

- The first `DEFINE` rule says "Alice is an ancestor of Bob if Alice reports to Bob".
- The second rule is recursive: "`?x` is an ancestor of `?z` if `?x` reports to some `?y` and `?y` is already an ancestor of `?z`". The `Ancestor(?y) { ... }` notation matches against the derived relation, not the layer.
- Both rules together compute the transitive closure via a **seminaive fixpoint** in the evaluator.
- The final `MATCH ?person {}` and `WHERE ?person = "urn:eigenius:test:alice"` is a guard query — no `RETURN` content, just checking Alice exists.

**Stratification** (chapter 9) ensures recursion stays decidable: a rule may not depend *negatively* on a relation that transitively depends on itself. Here both rules are positive, so the fixpoint converges in at most O(relation-size) iterations.

This is [`recursive_define_ancestor`](../../../kernel/src/query/evaluate.rs).

## 2.5. Aggregation: count instances per class

Group dogs by breed and count how many of each.

```eigenql
MATCH "urn:eigenius:example:Dog"(?d) {
    "urn:eigenius:example:breed": ?breed
}
GROUP BY ?breed
RETURN [] {
    "urn:eigenius:example:breed": ?breed,
    count: COUNT(?d)
}
```

**What happens**:

- Bindings are partitioned by the `GROUP BY` key (`?breed`). Each partition becomes one output row.
- Non-aggregate `RETURN` items must appear in `GROUP BY` (validated by the type checker — see chapter 9).
- `COUNT(?d)` returns the count per partition as an `Integer`.
- Supported aggregates: `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`.

## 2.6. FIBER clause: dispatch into an institution

Ask a registered institution for additional reasoning over a binding.

```eigenql
USING "urn:eigenius:example:Assay"
USING INSTITUTION "urn:eigenius:institutions:docking" AS dock

MATCH Assay(?a) {
    "urn:eigenius:example:compound": ?cpd
}
FIBER dock:PredictBinding {
    compound: ?cpd
} AS ?pred

RETURN [Prediction] {
    compound: ?cpd,
    predicted_affinity: ?pred
}
```

**What happens**:

- `USING INSTITUTION "urn:..." AS dock` declares a short alias `dock` for the `docking` institution.
- `FIBER dock:PredictBinding { compound: ?cpd } AS ?pred` builds a `PredictBinding` query resource with `compound` bound to the value of `?cpd`, dispatches it to the `docking` institution's `query()` method, and binds the response to `?pred` (which refers to the response IRI in subsequent clauses).
- The response is attached to a transient **overlay** (D2 §6.12) that's discarded when the query finishes — it does not persist to the layer.
- `RETURN [Prediction]` tags each row with the `Prediction` class.

Running this requires `FiberRuntime::institutions` populated; otherwise the `FIBER` clause errors at dispatch time. See [chapter 7](07-fiber-clauses.md) for full details.

## 2.7. Institution-dispatched decide predicate in `WHERE`

Filter bindings using a domain predicate answered by an institution.

```eigenql
USING "urn:eigenius:example:Docking"

MATCH Docking(?d) {
    "urn:eigenius:example:delta_g": ?dg
}
WHERE docking:within_tolerance(?dg, 2.0)
RETURN [] {
    delta_g: ?dg
}
```

**What happens** (Phase 11e.2):

- `docking:within_tolerance(?dg, 2.0)` is a qualified-name function call. Because it contains `:`, the evaluator parses it as an IRI (`urn:eigenius:docking:within_tolerance`) and consults the institution registry via [`InstitutionRegistry::classify`](../../../kernel/src/institution/mod.rs).
- When the IRI classifies as a `DecidePredicate`, the evaluator calls `FiberReasoner::decide(iri, args, ctx)` — the institution returns `DecResult::Holds` (include the binding), `DecResult::Fails` (drop the binding), or `DecResult::Undecidable` (also dropped, for `WHERE`'s boolean semantics).
- Returns `Value::Boolean(true)` for `Holds` and `false` otherwise.

A comorphism invocation in `RETURN` works symmetrically and produces a `Value::Embedded(resource)` — see [chapter 8](08-institutions.md).

---

These seven cover the shapes that make up most EigenQL code. The remaining chapters drill into specifics: every clause, every expression form, and how evaluation threads bindings through the stages.

Next: **[3. Lexical structure →](03-lexical-structure.md)**
