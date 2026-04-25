# 1. Introduction

This guide explains how to **operate** the Eigenius platform — install it, run it, manage data, write WASM extensions, and deploy. It is the practical companion to the surface-language guides ([ESL](../esl/README.md), [EigenQL](../eigenql/README.md)) which target program/query authors. Where this guide is operational, the language guides are reference material for what you write *into* the system.

## 1.1. System topology

Eigenius is a small set of cooperating processes:

```
┌─────────┐  CLI commands         ┌──────────────────┐
│   CLI   │ ──────────────────►  │  Kernel (Rust)   │
└─────────┘     gRPC :50051       │  - Layer chain    │
                                  │  - Type theory    │
                                  │  - EigenQL        │
                                  │  - WASM runtime   │
                                  └────┬─────────────┘
                                       │ component dispatch
                                       │ over Connect RPC :8080
                                  ┌────▼─────────────┐
                                  │  Orchestrator    │
                                  │  (Deno)          │
                                  │  - LLM adapter    │
                                  │  - MCP server     │
                                  │  - IO components  │
                                  └──────────────────┘
```

**Kernel** ([`kernel/`](../../../kernel/)) — Rust binary. Holds the layer chain (knowledge graph), type-checks programs via the Mini-TT kernel, evaluates programs and queries, dispatches IO components to the orchestrator. Exposes a gRPC service on port 50051. Optionally persists state to RocksDB via `serve --db <path>`.

**Orchestrator** ([`orchestration/`](../../../orchestration/)) — Deno/TypeScript service. Provides IO component implementations the kernel cannot embed directly (LLM adapters, HTTP-bearing components). Exposes the Model Context Protocol (MCP) server surface for external LLM agents to call kernel operations as tools. Listens on port 8080.

**CLI** ([`cli/`](../../../cli/)) — Rust binary (`eigenius`). Two operation modes:

- **In-process** — for file commands and read-only ontology inspection. No kernel server needed.
- **Remote** — for live operations against a running kernel. Pass `--endpoint http://localhost:50051`.

## 1.2. Four ways to interact with the platform

You'll touch the platform through one of four interfaces depending on what you're doing:

1. **The CLI** — for ad-hoc operations: load a file, run a program, query the graph, inspect a resource, install a WASM capability. The `eigenius` binary in [`cli/`](../../../cli/) is the everyday tool. See [chapter 4](04-cli-reference.md).

2. **The gRPC API** — for programmatic clients. The kernel exposes a tonic-based gRPC service at `--port` (default 50051) when running under `eigenius serve`. Protobuf definitions live in [`proto/`](../../../proto/). The CLI itself is a client of this API.

3. **The kernel as a library** — for embedding the kernel in another Rust process. Add `eigenius_kernel` as a Cargo dependency and use the modules under [`kernel/src/`](../../../kernel/src/) directly. This is what the kernel server itself does — the gRPC layer is a thin wrapper over the in-process API.

4. **WASM extensions** — for adding domain-specific dispatch logic. Custom components and institutions are built as WASM binaries against the [`eigenius-component`](../../../wit/eigenius-component.wit) and `eigenius-institution` WIT worlds, using the [`eigenius-wasm-sdk`](../../../sdk/wasm-sdk/) crate. Installed at runtime via `eigenius capability install`. See [chapter 9](09-wasm-components.md) and [chapter 10](10-wasm-institutions.md).

## 1.3. The four bootstrap layers

When the kernel starts, it loads four immutable layers in a parent-pointer chain:

```
core (root)
  └─ program
      └─ reflection
          └─ institution
```

These are baked into the kernel binary as the **embedded ontology**. The corresponding source-of-truth JSON files live in [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json) and equivalents for the other three. Every subsequent layer (loaded via `eigenius load`) sits on top of these four.

When running with `--db <path>`, the kernel also seeds a SHA-256 manifest of the embedded ontology on first start. Subsequent restarts verify that the embedded ontologies haven't drifted from the persisted manifest — see [chapter 6](06-database-management.md) and [D13](../../design/d13-durable-kernel-state.md).

## 1.4. What this guide covers

The chapters in order:

- **[Chapters 2–3](02-installation.md)** — get a development environment building.
- **[Chapter 4](04-cli-reference.md)** — every CLI command, with what it does in process vs. against a running kernel.
- **[Chapters 5–7](05-running-locally.md)** — running locally, persistence, the orchestrator.
- **[Chapter 8](08-demos.md)** — three worked end-to-end demos.
- **[Chapters 9–10](09-wasm-components.md)** — extending the kernel with WASM components and institutions.
- **[Chapter 11](11-deployment.md)** — Docker Compose and Azure ContainerApps deployment.
- **[Chapters 12–13](12-troubleshooting.md)** — troubleshooting and reference appendix.

## 1.5. What this guide does not cover

- **ESL syntax / semantics** — see the [ESL user guide](../esl/README.md).
- **EigenQL syntax / semantics** — see the [EigenQL user guide](../eigenql/README.md).
- **Kernel internals** — type theory, NbE, ontology-as-types resolution, etc. See the design documents in [`docs/design/`](../../design/).
- **Designing new institutions for new domains** — that's domain-modelling work, ongoing as Phases 12 and 15.

---

Next: **[2. Installation and prerequisites →](02-installation.md)**
