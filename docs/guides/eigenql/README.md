# EigenQL user guide

EigenQL is the read-only query language for the Eigenius knowledge graph. It lets you match patterns against layered Eigon resources, derive relations with `DEFINE`, dispatch to institution reasoners via `FIBER` clauses and qualified-name function calls, and shape the results into self-describing Eigon documents.

This guide is written against the implementation in [`kernel/src/query/`](../../../kernel/src/query/) and the specification in [D2](../../design/d2-eigenql-specification.md). Every feature described here is grounded in source — chapters link directly to the modules that implement them.

## How to read this guide

The chapters are ordered for sequential reading: each builds on concepts introduced earlier. If you already know SQL, Cypher, or Datalog, you can skim [chapter 2](02-quick-tour.md) to calibrate, then jump to whichever later chapter matches your question.

As a reference, the most useful chapters are:

- **[4. Program structure](04-program-structure.md)** — what clauses exist, in what order, and how they compose
- **[6. Text and vector retrieval](06-text-and-vector-retrieval.md)** — the D43 fuzzy-retrieval surface (`~` operator, indexes, `TOP N`)
- **[7. Expressions](07-expressions.md)** — the expression sublanguage used in `WHERE`, `RETURN`, `GROUP BY`, `ORDER BY`
- **[13. Appendix](13-appendix.md)** — EBNF grammar, keyword list, built-in function table, operator precedence, source index

## Chapters

1. **[Introduction](01-introduction.md)** — what EigenQL is, how it sits beside ESL, the six-stage execution pipeline, and the single entry point (`execute_with`).

2. **[Quick tour](02-quick-tour.md)** — seven worked examples drawn from the test suite: listing classes, filtering, recursive `DEFINE` (transitive closure), `GROUP BY` with aggregates, a `FIBER` clause, and a decide predicate in `WHERE`.

3. **[Lexical structure](03-lexical-structure.md)** — tokens, keywords (structural, operator, built-in function, literal), identifiers, variables (`?name`), qualified names, literals, operators, punctuation, comments, and whitespace rules.

4. **[Program structure](04-program-structure.md)** — clause-by-clause reference: `USING`, `USING INSTITUTION`, `DEFINE`, `MATCH`, `WHERE`, `FIBER`, `RETURN`, `GROUP BY`, `ORDER BY`, `LIMIT` / `TOP` / `OFFSET`/`DISTINCT`, and how `Query` / `MatchPart` are shaped internally.

5. **[Pattern matching](05-pattern-matching.md)** — how `MATCH` works: subject variables, class predicates with subclass-chain walking, property patterns against IRI / literal / variable targets, negation semantics, and multi-pattern equi-joins.

6. **[Text and vector retrieval](06-text-and-vector-retrieval.md)** — the D43 surface: the `~` similarity operator, schema-side `TextIndex` / `VectorIndex` declarations, the `{ via, model, k, limit }` hint block, `TOP N` ranked truncation, hybrid retrieval and internal RRF fusion, embedder registration, worked examples (text-only / vector-only / hybrid / structural composition), and the failure-mode catalogue.

7. **[Expressions](07-expressions.md)** — per-variant reference for the expression AST: literals, variables, binary ops (11 operators), unary ops, `NOT EXISTS`, function calls (three dispatch paths), aggregates, dot-paths, arrays, precedence, and type coercion rules.

8. **[FIBER clauses](08-fiber-clauses.md)** — structured institution dispatch: query-class-typed parameter objects, the transient overlay, the determinism guarantee via synthesized response IRIs, and a full multi-step example.

9. **[Institutions in EigenQL](09-institutions.md)** — how EigenQL dispatches to registered institutions under D14: the `InstitutionIndex` derived from the chain, Decidable `QueryClass` calls returning `Verdict`, the postfix `HOLDS` / `FAILS` / `UNDECIDABLE` predicates, comorphism coercion in FIBER params, and the classification table shared with ESL.

10. **[Stratification](10-stratification.md)** — why the stratifier exists, what a negation cycle looks like, how strata are assigned, the `FIBER`-in-`DEFINE` restriction, and safe patterns for combining recursion and negation.

11. **[Result format](11-result-format.md)** — the two result shapes (with / without `RETURN` items), synthesized IRIs via `QueryFingerprint`, a full walk-through of a result document, data-type inference for `Property` resources, and how to consume results programmatically.

12. **[Error messages](12-error-messages.md)** — the `QueryError` structure, common errors per phase (Lexer / Parser / TypeCheck / Stratification / Evaluation), and a debugging flow that uses the phase tag to localise faults.

13. **[Appendix](13-appendix.md)** — EBNF grammar reference, keyword list, built-in function table, operator precedence, result-document IRI constants, institution capability method reference, execution entry points, related documents, and a full source index.

## Related documents

- [**EigenQL specification (D2)**](../../design/d2-eigenql-specification.md) — the authoritative grammar and semantics. This guide is derived from D2 but adds worked examples and implementation pointers.
- [**Institution Realisation (D14)**](../../design/d14-institution-realisation.md) — the institution model `FIBER` and qualified-name calls dispatch through: ontology-first declarations (`Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism`), the three-method `Institution` trait (`extract_typed` / `reify` / `query`), triadic comorphism realisation, and the dispatch model. Supersedes D10.
- [**Eigon serialization format (D1)**](../../design/d1-eigon-serialization-format.md) — the resource / value model that EigenQL reads from and writes to.
- [**ESL user guide**](../esl/README.md) — the companion surface language for declaring ontologies and programs.

## Source index

All implementation referenced in this guide lives in [`kernel/src/query/`](../../../kernel/src/query/) and [`kernel/src/institution/`](../../../kernel/src/institution/). See [appendix §13.9](13-appendix.md#129-source-index) for the complete file-by-file list.

---

Ready to start? → **[1. Introduction](01-introduction.md)**
