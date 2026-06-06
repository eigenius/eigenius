# 1. Introduction

**EigenQL** is the query language for eigenius. It is read-only: queries retrieve resources from the layered knowledge graph, shape them into result documents, and optionally dispatch to registered *institutions* for domain-specific reasoning. Unlike ESL, EigenQL does not declare types or compute new values that persist — every query is a well-typed view over a fixed layer chain.

## Where EigenQL sits

The eigenius platform has two surface languages with complementary roles:

| | ESL | EigenQL |
|---|---|---|
| Role | Declares ontology + programs | Queries and filters |
| Side effects | Write: new Resources, trace layers | Read: no mutation |
| Type theory | Fully dependent (EigenTT) | Relational / first-order |
| Institutions | Invoke via expression syntax | Invoke via expression syntax + `FIBER` clauses |

EigenQL queries operate against a `Layer` — a chain of immutable resource sets rooted in the core ontology. A query is lexed, parsed, stratified, type-checked, evaluated against the layer chain, and finally wrapped into a self-describing result document per [D2 Appendix A](../../design/d2-eigenql-specification.md). The entry point is [`kernel/src/query/mod.rs::execute_with`](../../../kernel/src/query/mod.rs):

```rust
pub fn execute_with(
    program_str: &str,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Vec<Resource>, Vec<QueryError>>
```

The six pipeline stages are:

1. **Lex** — tokenize the source ([kernel/src/query/lexer.rs](../../../kernel/src/query/lexer.rs))
2. **Parse** — build the AST ([kernel/src/query/parser.rs](../../../kernel/src/query/parser.rs))
3. **Stratify** — validate DEFINE rule negation-cycles ([kernel/src/query/stratify.rs](../../../kernel/src/query/stratify.rs))
4. **Type-check** — validate classes, variables, FIBER clauses ([kernel/src/query/type_check.rs](../../../kernel/src/query/type_check.rs))
5. **Evaluate** — match patterns, run DEFINE fixpoint, group, aggregate, shape ([kernel/src/query/evaluate.rs](../../../kernel/src/query/evaluate.rs))
6. **Wrap** — produce the result document ([kernel/src/query/document.rs](../../../kernel/src/query/document.rs))

Type-check and stratification errors are reported all at once when possible; evaluation errors stop at the first failure.

## What a query looks like

A minimal query has three pieces: a `USING` import, a `MATCH` pattern, and a `RETURN` clause.

```eigenql
USING "urn:eigenius:core:Class"

MATCH Class(?c) {
    short_name: ?name
}
RETURN [] {
    short_name: ?name
}
```

That query returns a row for every `Class` resource in the layer chain, binding its `short_name` property into a result column named `short_name`. The `USING` import announces which classes the query talks about; `MATCH` defines a pattern with a variable (`?c`) bound to every matching resource; `RETURN` shapes each match into an output row.

More complex queries can:

- Interleave `FIBER` clauses that dispatch to an institution `OnDemand` `QueryClass` and bind the response (D14 §9)
- Invoke **Decidable QueryClasses** registered by institutions: `cap:within_tolerance(delta, 0.1) HOLDS` in a `WHERE` filter — returns a `Verdict` projected to a Boolean by the postfix predicate
- **Coerce values across institution boundaries** inside FIBER param values: `param: comorphism_iri(source)` runs the four-step extract → transform → reify pipeline (D14 §10.3) inline
- Derive new relations with `DEFINE ... FROM` (including recursion through stratified negation)
- Aggregate with `GROUP BY` + `COUNT` / `SUM` / `AVG` / `MIN` / `MAX`
- Sort, limit, and deduplicate results

## What this guide covers

Each chapter is self-contained and linked to the corresponding implementation file:

- **[2. Quick tour](02-quick-tour.md)** — seven worked examples covering the common shapes
- **[3. Lexical structure](03-lexical-structure.md)** — tokens, keywords, literals, identifier forms
- **[4. Program structure](04-program-structure.md)** — the top-level clauses in order
- **[5. Pattern matching](05-pattern-matching.md)** — `MATCH`, subjects, property patterns, negation
- **[6. Text and vector retrieval](06-text-and-vector-retrieval.md)** — the `~` similarity operator, `TextIndex` / `VectorIndex` declarations, hint block, `TOP N`, hybrid RRF
- **[7. Expressions](07-expressions.md)** — every `Expression` variant with its evaluation rule
- **[8. FIBER clauses](08-fiber-clauses.md)** — institution dispatch via the transient overlay
- **[9. Institutions](09-institutions.md)** — Decidable QueryClasses, postfix Verdict predicates, comorphism coercion, the D14 classification table
- **[10. Stratification](10-stratification.md)** — recursion + negation semantics
- **[11. Result format](11-result-format.md)** — how `RETURN` shapes become Eigon documents
- **[12. Error messages](12-error-messages.md)** — common errors and how to fix them
- **[13. Appendix](13-appendix.md)** — grammar reference, keyword list, built-in functions

## What EigenQL deliberately doesn't do

- **No mutation** — queries never commit to a layer. Writing new resources is ESL's job.
- **No dependent types** — EigenQL's type checker validates relational structure (classes, variables, aggregates) but the value-level universe is flat: strings, numbers, booleans, arrays, embedded resources. Dependent types live in ESL and the kernel.
- **No user-defined functions** — built-ins (`DATE`, `LENGTH`, `CONCAT`, ...) plus institution-registered capabilities are the extension points. There is no `def f(x) = ...` in queries.
- **No side-effectful dispatch beyond institutions** — EigenQL has no component registry, no IO effects, no trace emission. Everything a query reads is already in the layer or produced by a `FIBER` clause into the transient overlay.

## Prerequisites

The guide assumes familiarity with the Eigon serialization format ([D1](../../design/d1-eigon-serialization-format.md) — resources, IRIs, properties, embedded values) and the institution protocol ([D14 Institution Realisation](../../design/d14-institution-realisation.md) — institutions as ontology declarations + boundary trait, comorphisms as triadic translations, the Verdict shape). Reading [D2](../../design/d2-eigenql-specification.md) is helpful but not required; this guide re-derives the query semantics from the implementation and cites D2 where the spec is authoritative (grammar appendix, result-document shape).

---

Next: **[2. Quick tour →](02-quick-tour.md)**
