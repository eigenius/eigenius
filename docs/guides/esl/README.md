# ESL user guide

ESL — the Eigenius Surface Language — is the human-friendly syntax for declaring ontologies, defining typed programs, and constructing resource instances. It compiles to Eigon-JSON resources that the EigenTT kernel type-checks and evaluates.

This guide is written against the implementation in [`kernel/src/esl/`](../../../kernel/src/esl/) and the type-theory kernel in [`kernel/src/nbe/`](../../../kernel/src/nbe/) and [`kernel/src/program/`](../../../kernel/src/program/). The specification it complements is [D7](../../design/d7-esl-surface-syntax.md). Every feature described here is grounded in source — chapters link directly to the modules that implement them.

## How to read this guide

The chapters are ordered for sequential reading: each builds on concepts introduced earlier. If you already know a typed functional language and a typed knowledge graph (RDF/SHACL/OWL), you can skim chapters 1–3, internalise [chapter 6 (Resources, types, and the layer)](06-resources-types-and-the-layer.md), and then dip into the per-construct references in chapters 4 and 5.

The single most important chapter for understanding *how Eigenius differs from a standalone type-theory or a standalone knowledge graph* is **[Chapter 6](06-resources-types-and-the-layer.md)** — the bridge between the resource graph and the type system. Several patterns in chapters 4, 5, 8, and 9 only make sense once that bridge is internalised.

## Chapters

1. **[Introduction](01-introduction.md)** — what ESL is, the two-layer design (HCL-style declarations + ML-style expressions), how it compiles to Eigon-JSON, and how it relates to the kernel and the rest of Eigenius.

2. **[Quick tour](02-quick-tour.md)** — six worked examples: minimal class+property+resource, a `program` with `let` and apply, sized inductive types, codata streams, component calls with config blocks, and an institution-dispatched decide predicate.

3. **[Lexical structure](03-lexical-structure.md)** — tokens, keywords (declaration / expression / type), identifiers and qualified names (`ns:local`), variables, literals, operators, the lambda forms (`\` and `λ`), comments.

4. **[Declarations](04-declarations.md)** — per-form reference: `namespace`, `class`, `property`, `resource`, `axiom` (postulated propositions), `def` (transparent definitions, D66), `data` (inductive types with bounded binders), `codata` (coinductive types), `program`. Syntax + emitted resource shape + kernel mapping for each.

5. **[Expressions](05-expressions.md)** — per-construct reference for the program-body sublanguage: `let`, `Apply`, `Lambda`, `Case`, `Match`, `Construct`, `Project`, `MapExpr`, `ReduceExpr`, `CoRecord`, `Pair`, `Literal`, `Var`, plus the `formula(...)` Pratt-parsed math sublanguage that lowers to chain-resident `formulas:FormulaTerm` values (full reference in the [formula language guide](../formula/README.md)). For each: syntax, kernel `Exp` it compiles to, type-check rule, evaluation rule, capability-mode notes.

6. **[Resources, types, and the layer](06-resources-types-and-the-layer.md)** — *the bridge chapter*. How a single IRI is simultaneously a resource you can query and a type you can ascribe. Ontology-as-types resolution. The mappings table (declaration ↔ resource shape ↔ kernel term/type). When and how the kernel calls back into the layer.

7. **[Type theory primer](07-type-theory-primer.md)** — universes, Π/Σ-types, inductive types, coinductive types, sized types, identity types, normalization-by-evaluation. Brief and pragmatic; cross-linked to chapter 6.

8. **[Capability modes](08-capability-modes.md)** — `Pure` / `Read` / `Check` / `IO` and how they gate which kernel AST nodes can produce values vs. stay neutral. Covers the `EigonClass` resolution rule, component dispatch, and institution constraint firing.

9. **[Institutions in ESL](09-institutions.md)** — the institution surface from a program-author perspective under D14: declarations (`Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism`), how `cap:predicate(args)` dispatches as `Exp::NativeDecide` returning a `Verdict`, why comorphisms aren't expression-level, and the life-science motivating example.

10. **[Error messages](10-error-messages.md)** — common compile-time and check-time errors with the messages you'll see, what they mean, and how to fix them.

11. **[Appendix](11-appendix.md)** — EBNF grammar reference (linked to D7 §7 as authoritative), keyword list, source file index.

## Related documents

- [**D7 ESL surface syntax**](../../design/d7-esl-surface-syntax.md) — the authoritative grammar and semantics. This guide derives from D7 but adds worked examples and implementation pointers.
- [**D18 Ontology-as-types resolution**](../../design/d18-ontology-as-types-resolution.md) — the bridge mechanism explained in chapter 6.
- [**D19 Inductive and sized types**](../../design/d19-inductive-types.md) — type theory underpinning chapters 4 (`data`/`codata`) and 7.
- [**D11 Codata, streams, and resumable execution**](../../design/d11-codata-streams.md) — coinductive type design.
- [**D14 Institution Realisation**](../../design/d14-institution-realisation.md) — institution mechanism dispatched in chapter 9. Supersedes D10.
- [**D1 Eigon serialization format**](../../design/d1-eigon-serialization-format.md) — the resource model ESL compiles into.
- [**EigenQL user guide**](../eigenql/README.md) — the companion query language; the two share the institution capability classification table.

## Source index

All implementation referenced in this guide lives in [`kernel/src/esl/`](../../../kernel/src/esl/), [`kernel/src/program/`](../../../kernel/src/program/), and [`kernel/src/nbe/`](../../../kernel/src/nbe/). See [§12 appendix](11-appendix.md) for the complete file-by-file list.

---

Ready to start? → **[1. Introduction](01-introduction.md)**
