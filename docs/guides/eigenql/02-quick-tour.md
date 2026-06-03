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

**Stratification** (chapter 10) ensures recursion stays decidable: a rule may not depend *negatively* on a relation that transitively depends on itself. Here both rules are positive, so the fixpoint converges in at most O(relation-size) iterations.

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
- Non-aggregate `RETURN` items must appear in `GROUP BY` (validated by the type checker — see chapter 10).
- `COUNT(?d)` returns the count per partition as an `Integer`.
- Supported aggregates: `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`.

## 2.6. FIBER clause: dispatch into an institution

Ask a registered institution to evaluate an `OnDemand` `QueryClass` and bind the response.

```eigenql
USING "urn:eigenius:demo:d14:DockingResult"
USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay

MATCH DockingResult(?d) {
    "urn:eigenius:demo:d14:delta_g": ?dg
}
FIBER assay:validate_prediction {
    candidate: dock_to_assay(?d)
} AS ?check

WHERE ?check HOLDS
RETURN [] {
    delta_g: ?dg
}
```

**What happens**:

- `USING INSTITUTION "urn:..." AS assay` aliases the assay institution within this `MatchPart`.
- The FIBER param value `dock_to_assay(?d)` is a **comorphism coercion** — the kernel runs the four-step pipeline (extract ΔG via the dock institution → apply the Arrhenius transformation Component → reify an `AssayPrediction` via the assay institution) and uses the reified resource as the `candidate` property.
- `FIBER assay:validate_prediction { … } AS ?check` builds the QueryClass's input resource, calls the assay institution's `query` handler, and binds the returned `Verdict` to `?check`. The response lives in a transient overlay (D2 §7.12) discarded when the query finishes.
- `WHERE ?check HOLDS` projects the bound Verdict to a Boolean — keeps rows where the assay institution accepted the prediction.

This requires the `FiberRuntime` to carry an `InstitutionIndex` + `InstitutionRuntime` + `ComponentRegistry`; without them the FIBER clause errors at dispatch time. See [chapter 8](08-fiber-clauses.md) and the M8 worked example in [`kernel/tests/d14_dock_assay_demo.rs`](../../../kernel/tests/d14_dock_assay_demo.rs).

## 2.7. Decidable QueryClass in `WHERE`

Filter bindings using a domain predicate answered by an institution. Under D14 the call returns a `Verdict` (not a Boolean); a postfix `HOLDS` (or `FAILS` / `UNDECIDABLE`) projects it.

```eigenql
USING "urn:eigenius:demo:d14:WithinToleranceInput"

MATCH WithinToleranceInput(?wt) {
    "urn:eigenius:demo:d14:predicted_ic50": ?p,
    "urn:eigenius:demo:d14:target_ic50":    ?t,
    "urn:eigenius:demo:d14:tolerance":      ?tol
}
WHERE assay:within_tolerance(?p, ?t, ?tol) HOLDS
RETURN [] {
    predicted: ?p,
    target: ?t
}
```

**What happens**:

- `assay:within_tolerance(...)` is a qualified-name function call. The evaluator resolves the IRI in the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs); it classifies as a `Decidable` `QueryClass`, dispatching as `Exp::NativeDecide`.
- The kernel marshals the positional args into a synthetic input resource (args attached as `urn:eigenius:institution:decide_args`), calls `Institution::query(query_handler, input, ctx)` on the assay institution, and reads the returned Verdict's `ctor_name`.
- The postfix `HOLDS` projects the Verdict to a Boolean: `true` when `ctor_name == "Holds"`, `false` otherwise.

A bare Verdict in Boolean position (`WHERE assay:within_tolerance(…)` with no postfix) is a static type error (`bare_verdict_in_boolean_position`) — the conversion is always explicit. See [chapter 9](09-institutions.md) for the full Verdict surface.

---

These seven cover the shapes that make up most EigenQL code. The remaining chapters drill into specifics: every clause, every expression form, and how evaluation threads bindings through the stages.

Next: **[3. Lexical structure →](03-lexical-structure.md)**
