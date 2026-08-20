# 9. Stratification

`DEFINE` rules can recurse and can use negation, but not both arbitrarily. **Stratification** is the rule that makes the combination decidable: a relation may not depend on its own negation, directly or transitively. The stratifier is [`kernel/src/query/stratify.rs`](../../../kernel/src/query/stratify.rs), run as pipeline stage 3 in [`execute_with`](../../../kernel/src/query/mod.rs), between parsing and type-checking.

## 10.1. Why stratification exists

Naïve recursion with negation has ambiguous or contradictory semantics. Consider:

```eigenql
DEFINE P(?x) FROM MATCH NOT P(?x) {}
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

One caveat on n-ary heads: a `DEFINE` head may declare any number of variables and the evaluator stores them all as positional columns, but a *pattern referencing* a derived relation binds exactly one variable and candidate collection reads column `"0"` only ([`evaluate/pattern.rs`](../../../kernel/src/query/evaluate/pattern.rs)). `Ancestor(?y) { reports_to: ?z }` above binds `?y` from column 0 and then refines against the resource `?y` resolves to — it does not read the stored second column. A genuinely binary derived relation is therefore not yet expressible; write the second component as a property of the subject resource instead.

Negation across strata is also fine:

```eigenql
DEFINE Reachable(?x, ?y) FROM ...
DEFINE Isolated(?x) FROM MATCH ?x {}, NOT Reachable(?x) {}
```

`Isolated` depends on `Reachable` negatively but `Reachable` doesn't depend on `Isolated` at all. Stratum 0 = `{Reachable}`, stratum 1 = `{Isolated}`. The evaluator runs stratum 0 to fixpoint first, then stratum 1.

## 10.2. What the stratifier rejects

A **negation cycle** is a cycle in the dependency graph that includes at least one negative edge. The stratifier detects these via DFS ([`has_negation_cycle`](../../../kernel/src/query/stratify.rs)).

Self-reference through negation:

```eigenql
DEFINE Bad(?x) FROM MATCH NOT Bad(?x) {}
```

Error: `negation cycle detected involving relation 'Bad'`.

Mutual recursion through negation:

```eigenql
DEFINE A(?x) FROM MATCH NOT B(?x) {}
DEFINE B(?x) FROM MATCH NOT A(?x) {}
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

Evaluation order is by `order` ascending. Relations in the same stratum are evaluated together in one shared fixpoint loop.

## 10.4. Evaluation consequences

The evaluator ([`kernel/src/query/evaluate/mod.rs`](../../../kernel/src/query/evaluate/mod.rs), step 1) evaluates the strata **in dependency order**, running a fixpoint over each stratum's rules with every lower stratum already fully computed:

```rust
let strata = crate::query::stratify::stratify(&program.definitions)?;
let max_iterations = 1000; // Safety bound
for stratum in &strata {
    let rules = /* the definitions whose name is in this stratum */;
    for _ in 0..=max_iterations {
        let mut new_facts = false;
        for def in &rules {
            let bindings = evaluate_match_part(&def.body, layer, &derived)?;
            let projected = project_onto_head(bindings, &def.variables);
            let entry = derived.entry(def.name.clone()).or_default();
            for binding in projected {
                if !entry.contains(&binding) {
                    entry.push(binding);
                    new_facts = true;
                }
            }
        }
        if !new_facts { break; }
    }
}
```

Three things to read out of that loop.

**Stratum ordering is required for negation**, not merely an optimization. A relation that negates another (`NOT Reach(?x)`) sits in a strictly higher stratum, and the add-only fixpoint would otherwise see a *partial* negated relation in early iterations and add rows it never retracts. Within a single stratum only positive recursion appears (the stratifier guarantees it), so the monotonic fixpoint is sound there.

**The iteration is naive, not semi-naive.** Every round re-evaluates each rule's *entire* body against the whole accumulated `derived` map; there is no delta relation, and duplicates are discarded on insertion by the `entry.contains(&binding)` linear scan. Rows already derived are therefore recomputed once per round, and the cost per round is quadratic in the size of the relation. This is correct but not fast; do not size a workload on the assumption that a semi-naive evaluator is underneath.

**Each rule's body bindings are projected onto the rule's head variables**, re-keyed by positional index (`"0"`, `"1"`, …), before being stored. A relation defined by several rules may use differently-named head variables (`Reach(?t)` in one rule, `Reach(?n)` in another), so positional projection gives one canonical tuple shape per relation and drops rule-local variables. A row that fails to bind every head variable is dropped rather than reported.

The **1000-iteration safety bound** is a defensive cap — ordinary DEFINE recursion converges in O(size-of-relation) iterations. If you hit the cap, something is wrong (non-terminating rule or unexpectedly large fixpoint), and the evaluator silently stops adding facts and proceeds with the under-approximated relation.

## 10.5. The `FIBER` restriction

FIBER clauses dispatch to institutions whose responses go into a transient overlay. Because the overlay doesn't exist during the `DEFINE` fixpoint (it's only built for the main query), `DEFINE` bodies cannot contain FIBER clauses.

The parser enforces this: [`parse_match_part`](../../../kernel/src/query/parser.rs) takes an `allow_fiber: bool` parameter, which is `false` inside `parse_define`. The evaluator defends against it too ([`evaluate_match_part`](../../../kernel/src/query/evaluate/fiber.rs) — if `part.has_fiber()`, return `"FIBER clauses are not allowed in DEFINE bodies"`).

If you need institution reasoning to feed a derived relation, do it in reverse: the main query runs FIBER, produces binding rows, and the calling application can store those as a new layer of resources before issuing a second query that `DEFINE`s over them.

## 10.6. Practical implications

- **Write positive recursion freely**. Transitive closure, reachability, subclass chains — all safe.
- **Separate "compute" and "negate" into different relations**. If you need negation, make sure what you negate is computed earlier — usually, don't name the same relation on both sides.
- **Guard clauses are fine**. `MATCH NOT ?x { retired: true }` in a rule body only negates a pattern against the layer, not a derived relation — no stratification concerns.
- **Mutual recursion without negation is allowed**. `A` depending on `B` and `B` depending on `A` through positive patterns produces a joint fixpoint.

## 10.7. Example: safely combining recursion and negation

```eigenql
DEFINE Employee(?e) FROM MATCH Person(?e) { company: ?c }
DEFINE Manager(?m) FROM MATCH Person(?m) {
    direct_reports: ?r
}
DEFINE Contributor(?e) FROM
    MATCH Employee(?e) {},
    NOT Manager(?e) {}

MATCH Contributor(?x) {}
RETURN [] { contributor: ?x }
```

Dependency graph:
- `Contributor` depends positively on `Employee`, negatively on `Manager`
- `Employee`, `Manager` depend only on layer resources (no derived relations)

Strata: `{Employee, Manager}` at order 0, `{Contributor}` at order 1. No cycles. The stratifier accepts this, and the evaluator computes `Employee` and `Manager` first (from the layer), then `Contributor` (from those plus the layer).

Change any rule to reference `Contributor` negatively (or via a chain of negations back to `Contributor`), and the stratifier rejects the program.

---

Next: **[11. Result format →](11-result-format.md)**
