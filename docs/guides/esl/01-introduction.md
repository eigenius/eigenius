# 1. Introduction

ESL — the **Eigenius Surface Language** — is the surface syntax developers write to declare ontologies, define typed programs, and construct resource instances in Eigenius. It compiles to Eigon-JSON, the canonical resource format ([D1](../../design/d1-eigon-serialization-format.md)). Once compiled, the resources are loaded into a layer and become subject to all the same machinery as any other resource — type-checking, querying via [EigenQL](../eigenql/README.md), institution dispatch, persistence.

Eigon-JSON is a derivation of [**JSON-AD**](https://docs.atomicdata.dev/), the JSON serialization of Atomic Data — the resource format whose property keys are themselves IRIs and whose schema is itself queryable resources. The lineage and the deliberate divergences (URN-scheme IRIs instead of fetchable HTTPS URLs, no `@context`, layer-resolved namespaces, three-layer type system) are documented in [D1 §1.1–1.2](../../design/d1-eigon-serialization-format.md).

Two design decisions shape the rest of the guide:

1. **ESL is sugar over Eigon-JSON.** Every ESL construct has a 1:1 mapping to an Eigon-JSON resource. The compiler never invents semantics; it only restates the source in resource form. The runtime never sees ESL — only resources.
2. **The kernel is dependent-typed.** Programs you write in ESL aren't checked by an ad-hoc validator; they're checked by the EigenTT kernel — a normalization-by-evaluation type-checker for a Π/Σ/Inductive/Coinductive/Universe type theory. ESL is the surface for those types.

These two decisions meet at chapter 6: the resource graph and the type theory are not separate worlds. A class declared in ESL becomes a `Class` resource that the kernel resolves *as a type* during type-check, via the ontology-as-types bridge ([D18](../../design/d18-ontology-as-types-resolution.md)). A property carrying a `data_type` IRI is consulted by the kernel during field access. An institution-registered decide predicate fires automatically when its constraint appears in scope. Internalising this bridge is the single most important precondition for understanding the rest of the language; **chapter 6 explains it directly, and chapters 4, 5, 7, and 9 lean on it heavily**.

## 1.1. The two-layer design

ESL has two structurally distinct surfaces:

**Declarative (HCL-style).** Top-level forms — `namespace`, `class`, `property`, `resource`, `data`, `codata`, `program` — declare named entities. Each becomes one or more resources in the output. Order doesn't matter at the file level; declarations are independent until something references another by IRI.

**Expression (ML-style).** Inside `program` bodies you write ML-style expressions: `let`, lambdas (`\x -> e` or `λx -> e`), application, pattern match, constructor calls, projections. These compile to a kernel `Exp` that the type-checker sees and the evaluator runs.

The split exists because the two surfaces have different jobs. Declarations describe **what exists** in the knowledge graph. Expressions describe **how to compute** with what exists. Both compile through the same compiler, both end up as resources, but each has its own grammar and its own check rules.

## 1.2. The compile pipeline

```
.esl source
   │
   ▼
lexer::tokenize  ─►  Vec<Token>
   │
   ▼
parser::parse    ─►  ast::File (declarations + expressions)
   │
   ▼
compile::compile_file  ─►  Vec<Resource>   (Eigon-JSON, ready for layer load)
   │
   ▼
(optional) institution-aware variant: compile_file_with_institutions
   for compile-time classification of qualified-name function calls
   (Phase 11e.1 — see chapter 9).
```

The end product is a `Vec<Resource>`. Loading those resources into a layer makes them visible to:

- **The type-checker** ([`kernel/src/nbe/check.rs`](../../../kernel/src/nbe/check.rs)) — when a `program`'s body type-checks, the kernel walks the layer to resolve types referenced by IRI ([D18](../../design/d18-ontology-as-types-resolution.md)).
- **The evaluator** ([`kernel/src/nbe/eval.rs`](../../../kernel/src/nbe/eval.rs)) — when the program runs, the kernel may resolve resources, dispatch to components and institutions, and produce a result value.
- **EigenQL** — once loaded, every class, property, and resource declared in ESL is queryable via [the EigenQL surface](../eigenql/README.md).

The two compile entry points live in [`kernel/src/esl/mod.rs`](../../../kernel/src/esl/mod.rs):

```rust
pub fn compile(source: &str) -> Result<Vec<Resource>, Vec<EslError>>;

pub fn compile_with_institutions(
    source: &str,
    index: Arc<InstitutionIndex>,
) -> Result<Vec<Resource>, Vec<EslError>>;
```

Use the basic `compile` for ontologies and resources that don't reference institution capabilities. Use `compile_with_institutions` when a `program` body invokes qualified-name function calls referencing a `Decidable` `QueryClass` — the chain-derived [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) classifies the IRI at compile time. See chapter 9 (and note that under D14 comorphisms are not callable from ESL expression position — they surface inside EigenQL FIBER param coercion).

## 1.3. What this guide covers

The rest of the guide proceeds bottom-up:

- **[Chapter 2](02-quick-tour.md)** — six worked examples to give you a feel for the language before any reference material.
- **[Chapter 3](03-lexical-structure.md)** — tokens and lexical conventions.
- **[Chapter 4](04-declarations.md)** — every declaration form, with the resource shape it emits and the kernel mapping.
- **[Chapter 5](05-expressions.md)** — every expression form, with its kernel `Exp` representation, type-check rule, and evaluation rule.
- **[Chapter 6](06-resources-types-and-the-layer.md)** — *the bridge*. Read this before chapters 7–9; it makes everything that follows sit naturally.
- **[Chapter 7](07-type-theory-primer.md)** — the underlying type theory, in the language of users not theorists.
- **[Chapter 8](08-capability-modes.md)** — `Pure` / `Read` / `Check` / `IO` and what kernel nodes can or can't do in each.
- **[Chapter 9](09-institutions.md)** — how programs reach into institutions for domain reasoning.
- **[Chapter 10](10-error-messages.md)** — common errors and fixes.
- **[Chapter 12](11-appendix.md)** — grammar reference and source index.

## 1.4. What this guide does not cover

- **Implementing a new institution.** That's a separate audience (institution authors, not program authors). The relevant interfaces are the [`Institution` trait](../../../kernel/src/institution/runtime.rs) and [D14](../../design/d14-institution-realisation.md); the WASM-flavoured implementer guide is [platform §10](../platform/10-wasm-institutions.md).
- **Kernel internals beyond the user-visible surface.** This guide explains what the type-checker enforces and what the evaluator computes; it does not explain how NbE works internally beyond chapter 7's primer.
- **Operational concerns** — deploying eigenius, configuring storage, managing layer chains across services. Those belong in operations docs.

---

Next: **[2. Quick tour →](02-quick-tour.md)**
