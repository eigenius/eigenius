# Eigenius documentation

This directory holds all written documentation for the Eigenius platform. It is organised by audience and document kind:

| Subdirectory | Audience | Document kind |
|---|---|---|
| **[`guides/`](guides/README.md)** | Developers and operators using the platform | Task-first user guides |
| **[`design/`](design/README.md)** | Contributors and reviewers of the platform | Spec-first design documents |
| **`papers/`** | Academic / publication audiences | LaTeX sources for technical writeups, work-in-progress |

## [User guides →](guides/README.md)

Three guides, grounded in the implementation:

- **[Platform user guide](guides/platform/README.md)** — installing, running, managing, and extending the platform; the CLI; the kernel server; the orchestrator; persistence; WASM components and institutions; deployment; **the React notebook UX** (chapter 13); **the `@eigenius/client` TypeScript SDK** (chapter 14).
- **[ESL — Eigenius Surface Language](guides/esl/README.md)** — the surface syntax for declaring ontologies and writing typed programs.
- **[EigenQL — query language](guides/eigenql/README.md)** — the read-only query language over the layered Eigon knowledge graph.

The guides are task-first. They link into source for every claim about behaviour.

For most users, the notebook is the lowest-friction first touch — it's the React SPA bundled into the orchestrator image and served at `http://localhost:8080/notebooks/`. See [platform chapter 13](guides/platform/13-notebook.md) and the screenshot in [`guides/assets/eigenius_notebook_ux.png`](guides/assets/eigenius_notebook_ux.png).

## [Design documents →](design/README.md)

The architecture specification and design documents (D1–D28):

- **[architecture-v0.3.md](design/architecture-v0.3.md)** — current architecture specification.
- **[implementation-plan.md](design/implementation-plan.md)** — phased build plan (Phases 0–21).
- **D1–D28** — per-subsystem design notes: Eigon serialization (D1), EigenQL (D2), program model (D3), storage encoding (D4), gRPC API (D5), execution architecture (D6/D6b), ESL surface (D7), structured LLM output (D8), NbE and type extensions (D9), codata (D11), WASM extensibility (D12), durable kernel state (D13), **institutions (D14, supersedes D10)**, ontology-as-types (D18), inductive and sized types (D19), task traces (D21), notebook UX and TypeScript SDK (D22), runtime substrate (D26), Julia institutions (D27), Lean 4 as verification institution (D28).
- Plus standalone notes: [life-science requirements](design/life-science-requirements.md), [boundary contracts](design/boundary-contracts.md), [vision](design/vision.md), [manifesto](design/manifesto.md).

The design documents are spec-first. They define what should exist and why; the user guides explain what does exist and how to use it.

## `papers/`

LaTeX sources for technical writeups intended for eventual publication. Currently work-in-progress drafts:

- `eigenius-early.tex` — early-system overview
- `eigenius-institutions.tex` — institutions in Eigenius

Not built into the documentation site; build with a standard `pdflatex` toolchain when needed.

## How to navigate

- **New to the platform?** Start with the [notebook chapter](guides/platform/13-notebook.md) — the lowest-friction first touch — then read the [platform user guide](guides/platform/README.md) for installation, build, and operational reference, and dip into the [ESL](guides/esl/README.md) and [EigenQL](guides/eigenql/README.md) guides when you need to write your own ontologies, programs, and queries.
- **Want to understand a design decision?** Check the design document for the relevant subsystem ([design/](design/README.md)).
- **Working on the implementation?** Both — design docs for the spec, guides for the user-facing behaviour, source comments for the implementation detail.
