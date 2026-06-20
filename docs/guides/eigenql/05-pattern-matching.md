# 5. Pattern matching

Patterns are how EigenQL binds variables to resources. Every `MATCH` clause contains one or more patterns, and each pattern unifies with candidate resources in the layer (or in a `DEFINE`-derived relation, or in the `FIBER` overlay) to extend the current set of bindings.

The core unification function is [`try_match_resource`](../../../kernel/src/query/evaluate.rs) in `evaluate.rs`, driven by [`apply_pattern`](../../../kernel/src/query/evaluate.rs) / [`apply_negated_pattern`](../../../kernel/src/query/evaluate.rs).

## 5.1. Pattern anatomy

```rust
pub struct Pattern {
    pub subject: Variable,              // ?x
    pub class: Option<Name>,            // optional: Dog
    pub properties: Vec<PropertyPattern>,
    pub negated: bool,                  // NOT ?x { ... }
}

pub struct PropertyPattern {
    pub property: Name,                 // prop name or IRI
    pub object: ValueOrVariable,        // literal or ?var
}
```

Surface syntax:

```
ClassName( ?subject ) { prop1: ?var, prop2: "literal", ... }
```

or untyped:

```
?subject { prop: ?v }
```

The three parts:

1. **Subject** — always a variable. The resource being described.
2. **Class predicate** — optional; if present, the resource's `is_a` must include the named class (subclass chain walked via [`is_subclass_instance`](../../../kernel/src/query/evaluate.rs)).
3. **Property patterns** — pairs of `property: target` where the target is either a literal (the resource's property value must equal it) or a variable (bound to the value).

## 5.2. Subject variable semantics

The subject is always a `Variable`. It plays one of two roles depending on whether it already has a binding in scope:

- **Free**: no prior binding exists. The pattern scans the layer for resources and binds the subject variable to each one that matches.
- **Bound**: the variable is already bound (from an earlier pattern or FIBER clause). The pattern restricts to the specific resource — if that resource matches the class and property patterns, the binding survives; otherwise it's dropped.

This is how patterns compose. Given:

```eigenql
MATCH Dog(?d) { breed: ?breed }
MATCH Animal(?d) { diet: ?diet }
```

the second pattern restricts `?d` (already bound to a specific dog) to those dogs that also happen to be `Animal` instances with a `diet` property. The binding set narrows.

Conflict detection is implemented in [`try_match_resource`](../../../kernel/src/query/evaluate.rs): when a variable is already bound and the candidate resource disagrees (via `values_equal`), the match fails and that candidate is discarded.

## 5.3. Class predicates

Class predicates use the `Name` enum:

```rust
pub enum Name {
    ShortName(String),    // Dog
    FullIri(Iri),         // "urn:eigenius:example:Dog"
}
```

`ShortName`s are resolved against classes imported by `USING` (chapter 4 §4.2). `FullIri`s are used directly.

**Subclass matching** is enabled: a pattern `Animal(?x)` matches any resource whose `is_a` chain leads to `Animal` through the `subclass_of` property. See [`is_subclass_instance`](../../../kernel/src/query/evaluate.rs).

Without a class predicate (`?x { ... }`), the pattern matches any resource regardless of class; only the property patterns constrain it.

## 5.4. Property patterns

A property pattern is:

```
property: target
```

where `property` is a `Name` and `target` is a literal value or a variable.

### Property name resolution

At the point a property pattern is evaluated, the target resource's properties are a `BTreeMap<Iri, Value>`. The property `Name` is resolved in one of two ways:

1. **`FullIri(iri)`** — used directly: the resource must have that exact IRI key.
2. **`ShortName(s)`** — the evaluator scans the resource's property keys and finds the IRI whose local name equals `s`. See [`find_property_by_shortname`](../../../kernel/src/query/evaluate.rs). If multiple keys could match, the first wins (the map is `BTreeMap`-ordered, i.e. sorted by full IRI).

A property pattern that references a property the resource doesn't carry causes the match to fail — the pattern drops that resource from the candidate set.

### Literal targets

```eigenql
MATCH Dog(?d) { breed: "German Shepherd" }
```

The property pattern matches only resources whose `breed` value equals the literal, using [`values_equal`](../../../kernel/src/query/functions.rs). `values_equal` handles cross-type comparisons (integer vs float, IRI-string vs resource-reference) sensibly.

### Variable targets

```eigenql
MATCH Dog(?d) { breed: ?breed }
```

Variables may be free or bound:

- **Free**: the variable is assigned to the property's value. In the result binding, `?breed` → value of the `breed` property.
- **Bound**: the property's value is compared via `values_equal` against the existing binding. Disagreement → the pattern fails.

### Array targets (D59)

A property whose value is an array (`resource_array` / `value_array`) can be matched with an **array pattern** in the target position, instead of binding the array as a whole:

```eigenql
MATCH ?team { members: [] }                 -- exactly empty
MATCH ?team { members: [?only] }            -- exactly one element, bound to ?only
MATCH ?team { members: [?a, ?b] }           -- exactly two, bound positionally
MATCH ?team { members: [?a, ?b, ...] }      -- at least two; ?a,?b = the first two
MATCH ?team { members: [... ?m ...] }       -- iterate: one binding per element
```

- `[v0, v1, …]` — **exact arity**: matches only when the array has exactly that many elements; binds each variable positionally (ordered arrays, so positions are well-defined). `[]` matches the empty array.
- `[v0, …, vN, ...]` — **at least N**: matches when the array has at least the listed elements; binds the first N, leaves the rest unconstrained.
- `[... ?e ...]` — **iterate**: produces *one binding per element* (an unnest). This turns an array property into a relation, so it composes with recursion for graph traversal:

```eigenql
-- reachability over an array-valued `depends_on` edge
DEFINE Reach(?n) FROM MATCH "...:Objective"(?o) { thesis: ?n }
DEFINE Reach(?n) FROM MATCH Reach(?m) { depends_on: [... ?n ...] }
```

Positional binds equi-join on any variable already bound, exactly like a scalar target. An array pattern against a non-array (or absent) property simply doesn't match.

## 5.5. Multiple patterns in a single `MATCH`

```eigenql
MATCH Dog(?d) { owner: ?o },
      Person(?o) { country: "DE" }
```

Comma-separated patterns in a `MATCH` clause are evaluated in order. Each subsequent pattern extends the current binding set. Bindings that fail any pattern are dropped. Effectively this is an equi-join on shared variables.

**Order matters for efficiency** but not for result semantics — the final binding set is the same regardless of pattern order, as long as you're matching against the same fixed layer.

## 5.6. Negation: `NOT { ... }`

A negated pattern **drops** bindings where a candidate resource matches.

```eigenql
MATCH ?x {}
MATCH NOT ?x { retired: true }
```

Reads as "for every resource `?x`, require that no property `retired: true` exists on it". Equivalent to: `?x` is not retired.

Implementation: [`apply_negated_pattern`](../../../kernel/src/query/evaluate.rs). For each existing binding, the evaluator checks whether any candidate resource matches the inner pattern (against current bindings). If none match, the binding survives; if at least one matches, the binding is dropped.

### The `NOT EXISTS` expression (distinct from pattern negation)

`NOT EXISTS(?var)` is an **expression** (for use in `WHERE`), not a pattern. It tests whether `?var` is currently bound — returning `true` if the variable is not in the binding, `false` if it is. See [chapter 7 §7.5](07-expressions.md). This overlaps conceptually with negated patterns but is evaluated differently.

### Interaction with stratification

If a `DEFINE` rule body contains a negated pattern on a derived relation, the stratifier ensures no negative cycles are introduced. See [chapter 10](10-stratification.md).

## 5.7. Where patterns can reference

A pattern's subject is scanned over candidate resources drawn from **three sources** in combination:

1. **The layer chain** — the canonical resources via `Layer::all_resources()`.
2. **Derived relations** (`DEFINE` rules) — a pattern whose class is a relation name (e.g. `Ancestor(?y)`) scans that relation's current binding set.
3. **The FIBER overlay** — transient resources produced by `FIBER` clauses in the current query. Visible only within this query, not persisted.

The three are scanned in source order; a resource appearing in more than one source is considered once.

## 5.8. Empty property lists

`MATCH ?x {}` is valid — it matches every resource in scope, binding `?x` to each. Use it when you want to iterate all resources and let subsequent clauses or `WHERE` narrow the set.

```eigenql
MATCH ?x {}
WHERE ?x = "urn:eigenius:test:alice"
```

## 5.9. Class-only matching

`MATCH Dog(?d) {}` binds `?d` to every Dog in the layer without examining its properties.

## 5.10. A concrete example

Given the test layer (core ontology + `animals.json`):

```eigenql
USING "urn:eigenius:example:Dog", "urn:eigenius:example:Person"

MATCH Dog(?d) {
    name: ?dog_name,
    owner: ?o
}
MATCH Person(?o) {
    name: ?owner_name,
    country: ?country
}
WHERE ?country IN ["DE", "FR"]
```

What happens:

1. First `MATCH`: scan all Dog resources, bind `?d` / `?dog_name` / `?o` for each.
2. Second `MATCH`: for each binding so far, restrict to rows where `?o` (already bound) resolves to a Person with a `name` and `country`. Bind `?owner_name` and `?country`.
3. `WHERE`: keep only bindings where `?country` is in the given array.

After this chain, the surviving bindings have all five variables populated and can be shaped by `RETURN`.

---

Next: **[6. Text and vector retrieval →](06-text-and-vector-retrieval.md)**
