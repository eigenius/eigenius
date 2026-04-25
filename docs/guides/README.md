# Eigenius user guides

Task-first documentation for using the Eigenius platform. The design specs in [`docs/design/`](../design/) are spec-first — these guides are written for developers actually using the system.

Every claim in the guides is grounded in source: each chapter links to the kernel module, CLI command, example crate, or test that implements the feature being described.

## Guides

### [Platform user guide →](platform/README.md)

How to install, run, manage, and extend the platform: the CLI, the kernel server, the orchestrator, RocksDB persistence, WASM components and institutions, deployment via Docker Compose or Azure ContainerApps.

**Thirteen chapters covering**: installation, build/test workflow, CLI reference (every `eigenius` subcommand), running locally (three-terminal model + Docker Compose), database management (`serve --db`, drift refusal, exports), the orchestrator (LLM dispatch + MCP server), three end-to-end demo walkthroughs, building WASM components (pure / read / IO levels), building WASM institutions, deployment, troubleshooting, environment-variable and source-file index.

Most important chapters: **[4. CLI reference](platform/04-cli-reference.md)** for everyday operations and **[9. Building WASM components](platform/09-wasm-components.md)** + **[10. Building WASM institutions](platform/10-wasm-institutions.md)** for extending the kernel.

### [ESL — Eigenius Surface Language →](esl/README.md)

The surface syntax for declaring ontologies, defining typed programs, and constructing resource instances. Compiles to Eigon-JSON resources that the Mini-TT kernel type-checks and evaluates.

**Eleven chapters covering**: HCL-style declarations (`namespace`, `class`, `property`, `resource`, `data`, `codata`, `program`); the ML-style expression sublanguage (`let`, lambdas, pattern match, constructor application, projection, `corecord`, etc.); the bridge between the resource graph and the kernel's type theory; the four capability modes (`Pure`/`Read`/`Check`/`IO`); institution-dispatched decide predicates and comorphisms; common error messages.

Most important chapter for understanding *how Eigenius differs from a standalone type-theory or a standalone knowledge graph*: **[chapter 6 — Resources, types, and the layer](esl/06-resources-types-and-the-layer.md)**.

### [EigenQL — query language →](eigenql/README.md)

The read-only query language over the layered Eigon knowledge graph. Pattern matching with `MATCH`, derived relations with `DEFINE`, institution dispatch via `FIBER` clauses and qualified-name function calls.

**Twelve chapters covering**: lexical structure; clause-by-clause program structure (`USING`, `MATCH`, `WHERE`, `FIBER`, `RETURN`, `GROUP BY`, etc.); pattern matching against typed and untyped resources; the expression sublanguage; FIBER clauses (institution dispatch with transient overlay); decide predicates and comorphisms in expression position; stratification rules for recursion + negation; the result-document format; error messages.

## How the guides relate

The **platform** guide is operational — it covers everything *around* writing ESL/EigenQL: installing, running, managing data, deploying, building WASM extensions. The **ESL** and **EigenQL** guides are surface-language references — they cover what you write *into* the system.

ESL **computes**; EigenQL **retrieves and filters**. They share the same kernel primitives — most importantly the institution capability classification, which means the same qualified-name IRI dispatches identically from both languages ([ESL §9.8](esl/09-institutions.md), [EigenQL §8](eigenql/08-institutions.md)).

If you're new to the platform: start with [platform chapters 1, 2, 5](platform/01-introduction.md) (orientation, install, run locally), then [ESL chapters 1, 6](esl/01-introduction.md) and [EigenQL chapters 1, 2](eigenql/01-introduction.md). That's enough to install the system, write small ontologies, declare resources, and query them.

## Related design documents

- [D7 ESL surface syntax](../design/d7-esl-surface-syntax.md) — authoritative grammar, complementary to the ESL guide
- [D2 EigenQL specification](../design/d2-eigenql-specification.md) — authoritative grammar and semantics, complementary to the EigenQL guide
- [D18 Ontology-as-types resolution](../design/d18-ontology-as-types-resolution.md) — the bridge mechanism explained in ESL chapter 6
- [D19 Inductive and sized types](../design/d19-inductive-types.md) — type theory underpinning ESL `data`/`codata` declarations
- [D10 Grothendieck institution protocol](../design/d10-grothendieck-institution-protocol.md) — institution mechanism dispatched in both guides

The full design-document set lives in [`docs/design/`](../design/).
