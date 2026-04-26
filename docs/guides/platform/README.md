# Platform user guide

How to install, run, manage, and extend the Eigenius platform. This guide is the practical companion to the surface-language guides — it covers everything *around* writing ESL or EigenQL: the CLI, the kernel server, the orchestrator, persistence, WASM extensions, and deployment.

The guide is grounded in the implementation in [`cli/`](../../../cli/), [`kernel/src/server/`](../../../kernel/src/server/), [`orchestration/`](../../../orchestration/), [`storage/rocksdb/`](../../../storage/rocksdb/), [`examples/wasm-*`](../../../examples/), and [`deploy/`](../../../deploy/). Every command shape, env var, and config detail links to the source.

## How to read this guide

If you're new: read chapters 1–5 sequentially, then jump to **[chapter 13 — Notebook](13-notebook.md)** for the most accessible UX. After that, the guide is a reference — jump to the chapter for the question you have.

The most-used reference chapters are:

- **[13. Notebook](13-notebook.md)** — the React notebook UX, served by the orchestrator at `http://localhost:8080/notebooks/`
- **[14. TypeScript SDK](14-typescript-sdk.md)** — the `Eigen` class the notebook is built on, also usable from your own browser / Deno / Node code
- **[4. CLI reference](04-cli-reference.md)** — every `eigenius` subcommand
- **[6. Database management](06-database-management.md)** — durable mode, exports, backups
- **[9. Building WASM components](09-wasm-components.md)** and **[10. Building WASM institutions](10-wasm-institutions.md)** — the extension surface

## Chapters

1. **[Introduction](01-introduction.md)** — what this guide is for, system topology at a glance, the four ways to interact with the platform.

2. **[Installation and prerequisites](02-installation.md)** — Rust 1.95+, Deno, system packages, optional `just` and `cargo-component`. WSL 2 notes for Windows users.

3. **[Building and testing](03-building-and-testing.md)** — `just build`, `just test`, `just check`. What `just build` does that plain `cargo build` does not.

4. **[CLI reference](04-cli-reference.md)** — every `eigenius` subcommand, grouped by purpose: file commands, knowledge-graph commands, program commands, server, database, capability, tasks.

5. **[Running the platform locally](05-running-locally.md)** — three-terminal model (orchestrator + kernel + CLI), Docker Compose, what state survives restarts.

6. **[Database management](06-database-management.md)** — `serve --db <path>`, RocksDB persistence, drift refusal, `db stats`/`compact`/`export`, backup strategy.

7. **[The orchestrator](07-orchestrator.md)** — what it does (IO component dispatch + LLM adapter + MCP server), real vs. mock LLM mode, the built-in `CompleteText` and `CompleteJson` components, when you don't need it.

8. **[Worked demos](08-demos.md)** — step-throughs of `demo/run.sh`, `demo/patent/run.sh`, and `demo/wasm/run.sh`.

9. **[Building WASM components](09-wasm-components.md)** — pure / read-capability / IO components via `wasm-cbor-echo`, `wasm-doc-validator`, `wasm-read-query-probe`, `wasm-http-shout`. Build with `cargo-component`, install with `eigenius capability install`.

10. **[Building WASM institutions](10-wasm-institutions.md)** — `FiberReasoner` implementations via `wasm-ordering-institution`. Native institution alternative for non-sandboxed cases.

11. **[Deployment](11-deployment.md)** — Docker Compose (production-quality today), Azure ContainerApps via Bicep (preliminary; templates exist but haven't been deployed end-to-end yet), embedding the kernel as a library.

12. **[Troubleshooting and FAQ](12-troubleshooting.md)** — common build / runtime / connection issues.

13. **[Notebook](13-notebook.md)** — the React SPA the orchestrator serves at `/notebooks/`. Cell types (markdown / esl / eigenql / typescript / program-run / chart), the file format, publish-to-layer, the patent-analysis and kinase-screening demos, where the source lives.

14. **[TypeScript SDK](14-typescript-sdk.md)** — `@eigenius/client` and the `Eigen` class. The SDK that powers the notebook, also usable from your own code. Five-line examples for inspect / query / load / runProgramByIri / layerTopology / publishNotebook.

15. **[Appendix](15-appendix.md)** — environment variables, file locations, source index, related documents.

## Related documents

- [**ESL user guide**](../esl/README.md) — surface syntax for ontologies and programs
- [**EigenQL user guide**](../eigenql/README.md) — surface syntax for queries
- [**D13 Durable kernel state**](../../design/d13-durable-kernel-state.md) — `serve --db` spec
- [**D12 WASM extensibility**](../../design/d12-wasm-extensibility.md) — capability levels and host imports
- [**D6 Execution architecture**](../../design/d6-execution-architecture.md) — kernel ↔ orchestrator boundary

The full design-document set lives in [`docs/design/`](../../design/).

---

Ready to start? → **[1. Introduction](01-introduction.md)**
