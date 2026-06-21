# 9. Stratification

`DEFINE` rules can recurse and can use negation, but not both arbitrarily. **Stratification** is the rule that makes the combination decidable: a relation may not depend on its own negation, directly or transitively. The stratifier is [`kernel/src/query/stratify.rs`](../../../kernel/src/query/stratify.rs), run as pipeline stage 3 in [`execute_with`](../../../kernel/src/query/mod.rs), between parsing and type-checking.

## 10.1. Why stratification exists

Naïve recursion with negation has ambiguous or contradictory semantics. Consider:

```eigenql
DEFINE P(?x) FROM MATCH NOT ?x (P) {}
```

Reads as "P includes everyone who is not in P" — self-contradictory. There's no fixpoint.

Stratification forbids this by structure. A rule may refer to another relation **positively** any number of times (including recursively), but may only refer to a relation **negatively** if that relation is in a strictly lower stratum — i.e. fully computed before the current rule runs.

Positive recursion is fine:

```eigenql
DEFINE Ancestor(?x, ?z) FROM MATCH ?x { reports_to: ?z }
DEFINE Ancestor(?x, ?z) FROM
    MATCH ?x { reports_to: ?y },
          Ancestor(?y) { reports_to: ?z }
```

Both rules depend on `Ancestor` positively. Stratification assigns `Ancestor` to a single stratum; the evaluator computes the fixpoint within that stratum.

Negation across strata is also fine:

```eigenql
DEFINE Reachable(?x, ?y) FROM ...
DEFINE Isolated(?x) FROM MATCH ?x {}, NOT ?x (Reachable) {}
```

`Isolated` depends on `Reachable` negatively but `Reachable` doesn't depend on `Isolated` at all. Stratum 0 = `{Reachable}`, stratum 1 = `{Isolated}`. The evaluator runs stratum 0 to fixpoint first, then stratum 1.

## 10.2. What the stratifier rejects

A **negation cycle** is a cycle in the dependency graph that includes at least one negative edge. The stratifier detects these via DFS ([`has_negation_cycle`](../../../kernel/src/query/stratify.rs)).

Self-reference through negation:

```eigenql
DEFINE Bad(?x) FROM MATCH NOT ?x (Bad) {}
```

Error: `negation cycle detected involving relation 'Bad'`.

Mutual recursion through negation:

```eigenql
DEFINE A(?x) FROM MATCH NOT ?x (B) {}
DEFINE B(?x) FROM MATCH NOT ?x (A) {}
```

Also rejected — the cycle `A → B → A` passes through negative edges.

## 10.3. How strata are assigned

The stratifier ([`stratify`](../../../kernel/src/query/stratify.rs)) runs this algorithm:

1. Collect all relation names.
2. Build two dependency maps: `pos_deps[name]` and `neg_deps[name]`.
3. Check for negation cycles; error if any.
4. Initialize every relation at stratum 0.
5. Iterate: for each relation, ensure `stratum[name] ≥ stratum[positive_dep]` (shared stratum OK) and `stratum[name] > stratum[negative_dep]` (strictly greater). Repeat until stable.

The final assignment groups relations into strata, returned as a `Vec<Stratum>`:

```rust
pub struct Stratum {
    pub relations: Vec<String>,
    pub order: usize,
}
```

Evaluation order is by `order` ascending. Relations in the same stratum are evaluated together via a shared seminaive fixpoint.

## 10.4. Evaluation consequences

The evaluator ([`evaluate`](../../../kernel/src/query/evaluate.rs) step 1) doesn't inspect the strata explicitly in the current implementation — it runs a single fixpoint over **all** `DEFINE` rules for up to 1000 iterations:

```rust
// Initial pass: evaluate all rules from base facts
for def in &program.definitions {
    let bindings = evaluate_match_part(&def.body, layer, &derived)?;
    entry.extend(bindings);
}

// Fixpoint loop
for _ in 0..max_iterations {
    let mut new_facts = false;
    for def in &program.definitions {
        ...
    }
    if !new_facts { break; }
}
```

Stratification ensures this converges because there are no negation cycles. The evaluator computes strata **in dependency order** (D59): it runs a seminaive fixpoint over each stratum's rules with all lower strata already fully computed, then moves to the next stratum. This ordering is **required for negation**, not merely an optimization — a relation that negates another (`NOT Reach(?x)`) sits in a strictly higher stratum, and the add-only fixpoint would otherwise see a *partial* negated relation in early iterations and add stale rows it never retracts. Within a single stratum only positive recursion appears (the stratifier guarantees it), so the monotonic fixpoint is sound there.

The **1000-iteration safety bound** is a defensive cap — ordinary DEFINE recursion converges in O(size-of-relation) iterations. If you hit the cap, something is wrong (non-terminating rule or unexpectedly large fixpoint), and the evaluator silently stops adding facts.

## 10.5. The `FIBER` restriction

FIBER clauses dispatch to institutions whose responses go into a transient overlay. Because the overlay doesn't exist during the `DEFINE` fixpoint (it's only built for the main query), `DEFINE` bodies cannot contain FIBER clauses.

The parser enforces this: [`parse_match_part`](../../../kernel/src/query/parser.rs) takes an `allow_fiber: bool` parameter, which is `false` inside `parse_define`. The evaluator defends against it too ([`evaluate_match_part`](../../../kernel/src/query/evaluate.rs) — if `part.has_fiber()`, return `"FIBER clauses are not allowed in DEFINE bodies"`).

If you need institution reasoning to feed a derived relation, do it in reverse: the main query runs FIBER, produces binding rows, and the calling application can store those as a new layer of resources before issuing a second query that `DEFINE`s over them.

## 10.6. Practical implications

- **Write positive recursion freely**. Transitive closure, reachability, subclass chains — all safe.
- **Separate "compute" and "negate" into different relations**. If you need negation, make sure what you negate is computed earlier — usually, don't name the same relation on both sides.
- **Guard clauses are fine**. `MATCH NOT ?x { retired: true }` in a rule body only negates a pattern against the layer, not a derived relation — no stratification concerns.
- **Mutual recursion without negation is allowed**. `A` depending on `B` and `B` depending on `A` through positive patterns produces a joint fixpoint.

## 10.7. Example: safely combining recursion and negation

```eigenql
DEFINE Employee(?e) FROM MATCH ?e (Person) { company: ?c }
DEFINE Manager(?m) FROM MATCH ?m (Person) {
    direct_reports: ?r
}
DEFINE Contributor(?e) FROM
    MATCH ?e (Employee) {},
    NOT ?e (Manager) {}

MATCH ?x (Contributor) {}
RETURN [] { contributor: ?x }
```

Dependency graph:
- `Contributor` depends positively on `Employee`, negatively on `Manager`
- `Employee`, `Manager` depend only on layer resources (no derived relations)

Strata: `{Employee, Manager}` at order 0, `{Contributor}` at order 1. No cycles. The stratifier accepts this, and the evaluator computes `Employee` and `Manager` first (from the layer), then `Contributor` (from those plus the layer).

Change any rule to reference `Contributor` negatively (or via a chain of negations back to `Contributor`), and the stratifier rejects the program.

---

Next: **[11. Result format →](11-result-format.md)**
