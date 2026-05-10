<p align="center">
  <img src="assets/eigenius_logo_400x400.png" alt="Eigenius" width="200">
</p>

# Eigenius — AI Platform for Science and Engineering

An open-source **AI platform for science and engineering** built on a typed, queryable knowledge graph.

Contemporary LLMs produce text that reads like knowledge but carries no epistemic warranty — there is no structural way to distinguish a correct derivation from a convincing hallucination. Eigenius addresses this by anchoring knowledge in a typed, queryable knowledge graph where every fact has tracked provenance, every derivation is replayable through a typed pipeline, and formal proofs provide machine-checked certainty.

The platform maintains four epistemic categories:

- **Declared** — human assertions
- **Observed** — facts with provenance
- **Derived** — conclusions from typed pipelines with full audit trails
- **Verified** — derivations with machine-checked formal proofs

For frontier research in quantum physics, life sciences, materials science, and beyond, this distinction makes it possible to know what has been truly verified versus what is plausible-sounding text without proper grounding.

**Current status:** Phases 0–11e + D22 complete; Phase 12 in progress. The platform is operational end-to-end — kernel, orchestrator, LLM integration, and CLI connected via gRPC; type-checked programs with dependent types, sized inductives and codata; **institution dispatch under D14** (declarations as ontology resources, the three-method `Institution` trait, triadic comorphisms, Verdict-typed Decidable QueryClasses, FIBER param coercion, auto-registration of WASM institutions from chain scan); the React notebook + TypeScript SDK; durable RocksDB persistence; WASM-sandboxed extensions; deployable via Docker Compose. See the [implementation plan](../design/implementation-plan.md) for the full phased build plan and the [top-level README](https://github.com/eigenius/eigenius#readme) for the live capability list.

> This is still a very early stage of this project. Anticipate
> features not working or missing functionality overall. Our goal
> is to close those quality gaps rather aggressively. Feel free
> to submit issues in the discussion forum or directly as issue.

---

## Start here — the notebook

For most users, the notebook is the most accessible way to use the platform. A React SPA bundled into the orchestrator image and served at `http://localhost:8080/notebooks/` once `docker compose up -d` is running. Cells (markdown, ESL, EigenQL, TypeScript, program-run) drive the kernel; outputs auto-render as typed inspectors, result tables, layer-stack diagrams, and program-trace trees.

<p align="center">
  <img src="assets/eigenius_notebook_ux.png" alt="The Eigenius notebook — top of the patent-analysis demo" width="900">
</p>

→ **[Platform guide chapter 14 — Notebook](platform/14-notebook.md)** for the full reference.
→ **[Platform guide chapter 15 — TypeScript SDK](platform/15-typescript-sdk.md)** if you want to drive the kernel programmatically with the same `Eigen` class the notebook uses.

## User guides

Five task-first guides, grounded in the implementation. Every claim links to the kernel module, CLI command, example crate, or test that implements it.

### [Platform user guide →](platform/README.md)

How to install, run, manage, and extend the platform: the CLI, the kernel server, the orchestrator, RocksDB persistence, WASM components and institutions, the runtime substrate (Julia v1), deployment via Docker Compose or Azure ContainerApps, the notebook UX, the TypeScript SDK.

**Sixteen chapters covering**: installation, build/test workflow, CLI reference (every `eigenius` subcommand), running locally (three-terminal model + Docker Compose), database management (`serve --db`, drift refusal, exports), the orchestrator (LLM dispatch + MCP server + substrate addon), four end-to-end demo walkthroughs, building WASM components (pure / read / IO levels), building WASM institutions, the runtime substrate (`mirror create → env build → env create → institution install` flow), deployment, troubleshooting, environment-variable and source-file index, the React notebook (cell types + file format + publish-to-layer + patent and kinase-institutions demos + KaTeX), the TypeScript SDK (`@eigenius/client` API + worked examples). Plus per-Julia-institution slow-walk tutorials under [`platform/julia-institutions/`](platform/julia-institutions/).

Most important chapters: **[14. Notebook](platform/14-notebook.md)** + **[15. TypeScript SDK](platform/15-typescript-sdk.md)** for the typical first-touch UX, **[4. CLI reference](platform/04-cli-reference.md)** for everyday CLI operations, **[9. Building WASM components](platform/09-wasm-components.md)** + **[10. Building WASM institutions](platform/10-wasm-institutions.md)** for sandboxed extensions, and **[11. Runtime substrate](platform/11-runtime-substrate.md)** for language-runtime-hosted institutions.

### [ESL — Eigenius Surface Language →](esl/README.md)

The surface syntax for declaring ontologies, defining typed programs, and constructing resource instances. Compiles to Eigon-JSON resources that the Mini-TT kernel type-checks and evaluates.

**Eleven chapters covering**: HCL-style declarations (`namespace`, `class`, `property`, `resource`, `data`, `codata`, `program`); the ML-style expression sublanguage (`let`, lambdas, pattern match, constructor application, projection, `corecord`, etc.); the bridge between the resource graph and the kernel's type theory; the four capability modes (`Pure`/`Read`/`Check`/`IO`); institution-dispatched decide predicates and comorphism program-invocations; common error messages.

Most important chapter for understanding *how Eigenius differs from a standalone type-theory or a standalone knowledge graph*: **[chapter 6 — Resources, types, and the layer](esl/06-resources-types-and-the-layer.md)**.

### [EigenQL — query language →](eigenql/README.md)

The read-only query language over the layered Eigon knowledge graph. Pattern matching with `MATCH`, derived relations with `DEFINE`, institution dispatch via `FIBER` clauses and qualified-name function calls.

**Twelve chapters covering**: lexical structure; clause-by-clause program structure (`USING`, `MATCH`, `WHERE`, `FIBER`, `RETURN`, `GROUP BY`, etc.); pattern matching against typed and untyped resources; the expression sublanguage; FIBER clauses (institution dispatch with transient overlay or `INTO`-pinned chain reinsertion); decide predicates and comorphism coercion; stratification rules for recursion + negation; the result-document format; error messages.

### [Formula language →](formula/README.md)

The chain-mirrored Mini-TT fragment shared by every numerical institution on the platform. A small typed expression-tree language at `urn:eigenius:formulas:` that Symbolics, IntervalArithmetic, Catalyst, DiffEq, and JuMP-HiGHS all consume directly.

**Eight chapters covering**: the three-surface mental model (Mini-TT fragment / Eigon-JSON encoding / ESL `formula(...)` sublanguage); the six constructors and why two are binders; the tagged-dict embedding and validator's inductive-value rule; the operator catalog and signature-driven arity check; the Pratt-parsed `formula(...)` ESL sublanguage; identity-comorphism collapse when both endpoints share FormulaTerm; common failure modes; appendix.

### [Composing institutions →](composition/README.md)

The cross-cutting story none of the per-host chapters tell on their own — what happens when *several institutions* cooperate over a shared payload through declared comorphisms. The kinase-institutions notebook is the running example: five Julia institutions, three cross-institution comorphisms, two storylines, and the [D14 §9.3](../design/d14-institution-realisation.md) chain-reinsertion contract closed through both ESL `Exp::InstitutionInvoke` and EigenQL `FIBER ... INTO`.

**Nine chapters covering**: the three layers of composition (shared payload / declared comorphisms / coordinated dispatch roles); shared payload languages and identity-comorphism collapse; the triadic structure of comorphisms with the four-step dispatch pipeline; the three dispatch roles (AutoOnLoad / OnDemand / Decidable) in concert; chain reinsertion of comorphism outputs through both surfaces; an end-to-end walkthrough of the kinase notebook; composition patterns and anti-patterns; cross-composition failure modes; appendix with a research-direction note on translating institution theory from set + model theory into constructive type theory.

### [References →](references/README.md)

A consolidated bibliography for the platform — what we cite, what we depend on, what came before us, and what we share contemporary ground with. Generated from the BibTeX files in [`docs/references/`](../references/) by `scripts/bib-to-md.py`.

**Four lists covering**: cited references (used in design docs / papers / guides), foundational works the system relies on, philosophical and methodological precursors (MKM, Suppes structuralism, formal ontologies in science, the reproducibility movement), and contemporary related work (institution theory in physics and engineering, HOL for the natural sciences, HoTT and its extensions, the epistemology of formal proof). Companion tooling: `scripts/verify-citations.py` cross-checks each entry against Crossref / arXiv / live URLs.

## How the guides relate

The **platform** guide is operational — it covers everything *around* writing ESL / EigenQL / formulas: installing, running, managing data, deploying, building WASM and substrate extensions. The **ESL**, **EigenQL**, and **formula** guides are surface-language references — they cover what you write *into* the system. The **composition** guide is the cross-cutting story — what happens when multiple institutions cooperate, the topic neither the per-host platform chapters nor the surface-language guides can cover cleanly on their own.

ESL **computes**; EigenQL **retrieves and filters**; formula **expresses typed expression trees** consumed by every numerical institution. The three share the same kernel primitives — most importantly the [`InstitutionIndex`](../../kernel/src/institution/registry.rs) classification (D14 §9.5), which means the same qualified-name IRI dispatches identically from ESL and EigenQL ([ESL §9.8](esl/09-institutions.md), [EigenQL §8](eigenql/08-institutions.md)); and the chain-resident `formulas:FormulaTerm` shape is the payload language every numerical institution speaks ([formula §1](formula/01-introduction.md)).

If you're new to the platform: start with [platform chapter 14](platform/14-notebook.md) (the notebook UX) — it's the lowest-friction first touch. Then read [platform chapters 1, 2, 5](platform/01-introduction.md) for orientation, install, and the kernel/orchestrator topology under the notebook, and dip into [ESL chapters 1, 6](esl/01-introduction.md) + [EigenQL chapters 1, 2](eigenql/01-introduction.md) + [formula chapter 1](formula/01-introduction.md) when you want to write your own ontologies, programs, queries, and typed expression trees.

## Beyond the guides

Spec-first design documents in [`docs/design/`](../design/) cover the underlying architecture and the per-subsystem decisions:

- [D7 ESL surface syntax](../design/d7-esl-surface-syntax.md) — authoritative grammar, complementary to the ESL guide
- [D2 EigenQL specification](../design/d2-eigenql-specification.md) — authoritative grammar and semantics, complementary to the EigenQL guide
- [D18 Ontology-as-types resolution](../design/d18-ontology-as-types-resolution.md) — the bridge mechanism explained in ESL chapter 6
- [D19 Inductive and sized types](../design/d19-inductive-types.md) — type theory underpinning ESL `data`/`codata` declarations and chain-resident inductives
- [D14 Institution Realisation](../design/d14-institution-realisation.md) — institution mechanism dispatched in three guides (ESL, EigenQL, platform); supersedes D10. §9.3 covers comorphism chain reinsertion.
- [D22 Notebook UX and TypeScript SDK](../design/d22-notebook-and-typescript-sdk.md) — spec for the notebook + `@eigenius/client`, complementary to platform chapters 14 + 15
- [D26 Runtime Substrate](../design/d26-runtime-substrate.md), [D29 Mirror Generator](../design/d29-runtime-mirror-generator.md), [D31 Institution Lifecycle](../design/d31-runtime-language-substrate-institution-lifecycle.md) — the substrate hosting layer and lifecycle, complementary to platform chapter 11
- [D27 Julia Institutions](../design/d27-julia-institutions.md) — the v1 Julia institution suite, complementary to the per-institution tutorials under [`platform/julia-institutions/`](platform/julia-institutions/)
- [D32 Chain-mirrored Mini-TT inductives](../design/d32-chain-mirrored-mini-tt-inductives.md) — the formula-language design spec, complementary to the formula guide

The full set (plus standalone notes including [D28 Lean 4 as Verification Institution](../design/d28-lean-4-as-institution.md)) lives at [`docs/design/`](../design/).

Source code: [github.com/eigenius/eigenius](https://github.com/eigenius/eigenius).
