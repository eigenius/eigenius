# 1. Introduction

This guide explains how to **operate** the Eigenius platform — install it, run it, manage data, write WASM and substrate extensions, and deploy. It is the practical companion to the surface-language guides ([ESL](../esl/README.md), [EigenQL](../eigenql/README.md), [Formula](../formula/README.md)) which target program/query authors. Where this guide is operational, the language guides are reference material for what you write *into* the system.

## 1.1. System topology

Eigenius is a small set of cooperating processes:

```
┌──────────────┐  Connect-RPC                     ┌──────────────────┐
│  Notebook    │ ────────────────────────────►   │  Orchestrator    │
│  (browser)   │  /notebooks/* (static SPA)       │  (Deno)          │
└──────────────┘  /eigenius.v1.* (RPC)            │  - LLM adapter   │
                                                  │  - MCP server    │
┌─────────┐  CLI commands         ┌──────────────┤  - IO components │
│   CLI   │ ──────────────────►  │   Kernel     │  - Notebook SPA  │
└─────────┘     gRPC :50051       │   (Rust)     │  - Substrate     │
                                  │  - Layer chain    addon         │
                                  │  - Type theory    Connect :8080│
                                  │  - EigenQL        └────┬───────┘
                                  │  - WASM runtime        │
                                  └────┬─────────────┘     │ spawn
                                       ▲                   ▼ workers
                                       │ component   ┌──────────────┐
                                       │ dispatch    │   Worker     │
                                       │             │  containers  │
                                       │             │  (Julia,     │
                                       │             │   Python…)   │
                                       │             │  UDS +       │
                                       │             │  Eigon-CBOR  │
                                       │             └──────────────┘
                                       │
                                       │             /var/lib/eigenius/
                                       │             substrate-depot/
                                       │             (host bind-mount,
                                       │              UDS sockets)
```

**Kernel** ([`kernel/`](../../../kernel/)) — Rust binary. Holds the layer chain (knowledge graph), type-checks programs via the EigenTT kernel, evaluates programs and queries, dispatches IO components and external runtimes to the orchestrator. Exposes a gRPC service on port 50051. Optionally persists state to RocksDB via `serve --db <path>`.

**Orchestrator** ([`orchestration/`](../../../orchestration/)) — Deno/TypeScript service. Four roles: (a) hosts IO component implementations the kernel cannot embed directly (LLM adapters, HTTP-bearing components); (b) exposes the Model Context Protocol (MCP) server surface for external LLM agents to call kernel operations as tools; (c) serves the notebook SPA at `/notebooks/*` and proxies the kernel's RPC surface to browsers via Connect-RPC under `/eigenius.v1.*`; (d) hosts the **runtime substrate addon** that spawns sibling worker containers for substrate-hosted institutions (Julia in v1; see [chapter 11](11-runtime-substrate.md)). Listens on port 8080.

**Worker containers** — when the kernel dispatches against a substrate-hosted institution, the orchestrator's substrate addon spawns (or reuses, from a per-image cache) a sibling container running the institution's language runtime. Workers are *siblings* of the orchestrator container — Docker-out-of-Docker via the bind-mounted Docker socket — and communicate over a Unix domain socket whose path lives under the depot bind-mount (`/var/lib/eigenius/substrate-depot/`, reachable at the same path on host and orchestrator). See [chapter 11 §11.1](11-runtime-substrate.md#111-the-substrate-concept).

**Notebook** ([`notebooks/`](../../../notebooks/)) — React + TypeScript single-page app. Bundled into the orchestrator image at build time, served at `http://localhost:8080/notebooks/` from a single origin. Cells run ESL, EigenQL, TypeScript, and program invocations against the live kernel; outputs auto-render as typed inspectors, result tables, layer-stack diagrams, and program-trace trees. See [chapter 14](14-notebook.md).

**CLI** ([`cli/`](../../../cli/)) — Rust binary (`eigenius`). Two operation modes:

- **In-process** — for file commands and read-only ontology inspection. No kernel server needed.
- **Remote** — for live operations against a running kernel. Pass `--endpoint http://localhost:50051`.

**TypeScript SDK** ([`clients/eigenius-ts/`](../../../clients/eigenius-ts/)) — `@eigenius/client`. Wraps the orchestrator's two Connect-RPC services in a single typed `Eigen` class. The notebook uses it; you can use it programmatically from any TypeScript runtime (browser, Deno, Node). See [chapter 17](17-typescript-sdk.md).

## 1.2. Seven ways to interact with the platform

You'll touch the platform through one of seven interfaces depending on what you're doing:

1. **The notebook** — the most accessible UX. A React SPA the orchestrator serves at `http://localhost:8080/notebooks/` once `docker compose up -d` is running; cells (markdown, ESL, EigenQL, TypeScript, program-run) run against the live kernel and auto-render typed outputs. Notebook documents are JSON files and can be saved, loaded, or published into the kernel as queryable resources. See [chapter 14](14-notebook.md).

2. **The TypeScript SDK** — for programmatic browser / Deno / Node use of the same RPC surface the notebook drives. The `Eigen` class wraps `inspect` / `query` / `load` / `runProgram` / `runProgramByIri` / `layerTopology` / `publishNotebook` and a few more. See [chapter 17](17-typescript-sdk.md).

3. **The CLI** — for ad-hoc operations: load a file, run a program, query the graph, inspect a resource, install a WASM capability or substrate institution. The `eigenius` binary in [`cli/`](../../../cli/) is the everyday tool. See [chapter 4](04-cli-reference.md).

4. **The gRPC API** — for programmatic non-TS clients. The kernel exposes a tonic-based gRPC service at `--port` (default 50051) when running under `eigenius serve`. Protobuf definitions live in [`proto/`](../../../proto/). The CLI and the orchestrator's TypeScript SDK are both clients of this API.

5. **The kernel as a library** — for embedding the kernel in another Rust process. Add `eigenius_kernel` as a Cargo dependency and use the modules under [`kernel/src/`](../../../kernel/src/) directly. This is what the kernel server itself does — the gRPC layer is a thin wrapper over the in-process API.

6. **WASM extensions** — for adding *sandboxed* domain-specific dispatch logic. Custom components and institutions are built as WASM binaries against the [`eigenius-component`](../../../wit/eigenius-component.wit) and `eigenius-institution-d14` WIT worlds, using the [`eigenius-wasm-sdk`](../../../sdk/wasm-sdk/) crate. Components are installed via `eigenius capability install`; D14 institutions auto-register from chain scan when their `Institution` declaration carries `runtime: wasm` + inline `wasm_binary`. See [chapter 9](09-wasm-components.md) and [chapter 10](10-wasm-institutions.md).

7. **Runtime substrate institutions** — for institutions backed by *full language ecosystems* (Julia in v1; Python and others planned). Sibling worker containers spawned by the orchestrator's substrate addon, communicating with the kernel via Eigon-CBOR over UDS. The right path when the institution wants to use a heavy native library that doesn't compile to WASM cleanly — `Symbolics.jl`, `OrdinaryDiffEq.jl`, `JuMP+HiGHS`, etc. The chain shapes (`RuntimePackageMirror`, `RuntimeEnvironment`, `RuntimeMethodSignature`, `Institution { runtime: external }`) are committed via `eigenius mirror create` / `env build` / `env create` / `institution install`. See [chapter 11](11-runtime-substrate.md).

## 1.3. The five bootstrap layers

When the kernel starts, it loads five immutable layers in a parent-pointer chain:

```
core (root)
  └─ program
      └─ reflection
          └─ institution
              └─ notebook
```

These are baked into the kernel binary as the **embedded ontology**. The corresponding source-of-truth JSON files live in [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json) and equivalents for the other four. Every subsequent layer (loaded via `eigenius load` or `eigen.load(...)`) sits on top of these five. The notebook layer ([`ontologies/notebook/notebook-ontology.json`](../../../ontologies/notebook/notebook-ontology.json)) defines `Notebook`, `Cell`, and `CellType` so notebooks published from the UI ([chapter 14](14-notebook.md) §14.5) validate against a live, queryable schema without having to load anything first.

When running with `--db <path>`, the kernel also seeds a SHA-256 manifest of the embedded ontology on first start. Subsequent restarts verify that the embedded ontologies haven't drifted from the persisted manifest — see [chapter 6](06-database-management.md) and [D13](../../design/d13-durable-kernel-state.md).

## 1.4. What this guide covers

The chapters in order:

- **[Chapters 2–3](02-installation.md)** — get a development environment building.
- **[Chapter 4](04-cli-reference.md)** — every CLI command, with what it does in process vs. against a running kernel.
- **[Chapters 5–7](05-running-locally.md)** — running locally, persistence, the orchestrator.
- **[Chapter 8](08-demos.md)** — three worked end-to-end demos.
- **[Chapters 9–10](09-wasm-components.md)** — extending the kernel with WASM components and institutions.
- **[Chapter 11](11-runtime-substrate.md)** — extending the kernel with runtime-substrate-hosted institutions (Julia in v1).
- **[Chapter 12](12-deployment.md)** — Docker Compose and Azure ContainerApps deployment.
- **[Chapters 13](13-troubleshooting.md)** — troubleshooting.
- **[Chapter 14](14-notebook.md)** — the notebook UX (cells, file format, publish-to-layer, the patent and kinase-institutions demos).
- **[Chapter 15](15-tags-branches-history.md)** — tags, branches, and history in the notebook (the workspace rail's chain destinations + time-travel read-pin).
- **[Chapter 16](16-merge-resolution.md)** — merge resolution: picking strategies (Witness / Rename / SchemaQuotient / Restructure), the cascade gate, provenance records.
- **[Chapter 17](17-typescript-sdk.md)** — the TypeScript SDK that powers the notebook and that you can use programmatically.
- **[Chapter 18](18-appendix.md)** — appendix (env vars, file locations, source index).

## 1.5. What this guide does not cover

- **ESL syntax / semantics** — see the [ESL user guide](../esl/README.md).
- **EigenQL syntax / semantics** — see the [EigenQL user guide](../eigenql/README.md).
- **Formula language** — the chain-mirrored EigenTT fragment used by every numerical institution; see the [formula language guide](../formula/README.md).
- **Kernel internals** — type theory, NbE, ontology-as-types resolution, etc. See the design documents in [`docs/design/`](../../design/).

---

Next: **[2. Installation and prerequisites →](02-installation.md)**
