# Platform user guide

How to install, run, manage, and extend the Eigenius platform. This guide is the practical companion to the surface-language guides — it covers everything *around* writing ESL, EigenQL, or formula values: the CLI, the kernel server, the orchestrator, persistence, WASM and substrate extensions, and deployment.

The guide is grounded in the implementation in [`cli/`](../../../cli/), [`kernel/src/server/`](../../../kernel/src/server/), [`orchestration/`](../../../orchestration/), [`storage/rocksdb/`](../../../storage/rocksdb/), [`crates/runtime-substrate/`](../../../crates/runtime-substrate/), [`examples/wasm-*`](../../../examples/), [`julia/`](../../../julia/), and [`deploy/`](../../../deploy/). Every command shape, env var, and config detail links to the source.

## How to read this guide

If you're new: read chapters 1–5 sequentially, then jump to **[chapter 14 — Notebook](14-notebook.md)** for the most accessible UX. After that, the guide is a reference — jump to the chapter for the question you have.

The most-used reference chapters are:

- **[14. Notebook](14-notebook.md)** — the React notebook UX, served by the orchestrator at `http://localhost:8080/notebooks/`
- **[15. Tags, branches, and history](15-tags-branches-history.md)** — the workspace panels that drive named refs and chain navigation
- **[16. Merge resolution](16-merge-resolution.md)** — picking a per-conflict strategy, the cascade gate, provenance records
- **[17. TypeScript SDK](17-typescript-sdk.md)** — the `Eigen` class the notebook is built on, also usable from your own browser / Deno / Node code
- **[4. CLI reference](04-cli-reference.md)** — every `eigenius` subcommand
- **[6. Database management](06-database-management.md)** — durable mode, exports, backups
- **[9. Building WASM components](09-wasm-components.md)** and **[10. Building WASM institutions](10-wasm-institutions.md)** — the sandboxed extension surface
- **[11. Runtime substrate](11-runtime-substrate.md)** — the language-runtime extension surface (Julia in v1)
- **[`julia-institutions/`](julia-institutions/)** — slow-walk tutorials for each of the v1 Julia institutions
- **[`lean-institution/`](lean-institution/)** — the platform's first verification institution: Lean 4 in-process via `nanoda_lib`

## Chapters

1. **[Introduction](01-introduction.md)** — what this guide is for, system topology at a glance, the seven ways to interact with the platform.

2. **[Installation and prerequisites](02-installation.md)** — Rust 1.95+, Deno, system packages, optional `just` and `cargo-component`. WSL 2 notes for Windows users.

3. **[Building and testing](03-building-and-testing.md)** — `just build`, `just test`, `just check`. What `just build` does that plain `cargo build` does not.

4. **[CLI reference](04-cli-reference.md)** — every `eigenius` subcommand, grouped by purpose: file commands, knowledge-graph commands, program commands, server, database, branch, mirror, env, institution, capability, tasks.

5. **[Running the platform locally](05-running-locally.md)** — three-terminal model (orchestrator + kernel + CLI), Docker Compose, what state survives restarts.

6. **[Database management](06-database-management.md)** — `serve --db <path>`, RocksDB persistence, drift refusal, `db stats`/`compact`/`export`, backup strategy.

7. **[The orchestrator](07-orchestrator.md)** — what it does (IO component dispatch + LLM adapter + MCP server + substrate addon), real vs. mock LLM mode, the built-in `CompleteText` and `CompleteJson` components, when you don't need it.

8. **[Worked demos](08-demos.md)** — step-throughs of `demo/run.sh`, `demo/patent/run.sh`, `demo/wasm/run.sh`, and the multi-institution kinase-institutions notebook.

9. **[Building WASM components](09-wasm-components.md)** — pure / read-capability / IO components via `wasm-cbor-echo`, `wasm-doc-validator`, `wasm-read-query-probe`, `wasm-http-shout`. Build with `cargo-component`, install with `eigenius capability install`.

10. **[Building WASM institutions](10-wasm-institutions.md)** — D14 `Institution` trait implementations against the `eigenius-institution-d14` WIT world (`extract-typed` / `reify` / `query`). Auto-registration from chain scan via `runtime: wasm` + inline `wasm_binary`. Walked through with the M8 dock-assay worked example.

11. **[Runtime substrate](11-runtime-substrate.md)** — the orchestrator-spawned, container-hosted runtime layer for institutions backed by full language ecosystems (Julia in v1, Python and others tracked). The `mirror create → env build → env create → institution install` lifecycle. WASM vs. substrate trade-off table. Cross-links to [`julia-institutions/`](julia-institutions/).

12. **[Deployment](12-deployment.md)** — Docker Compose (production-quality today), Azure ContainerApps via Bicep (preliminary; templates exist but haven't been deployed end-to-end yet), embedding the kernel as a library.

13. **[Troubleshooting and FAQ](13-troubleshooting.md)** — common build / runtime / connection issues.

14. **[Notebook](14-notebook.md)** — the React SPA the orchestrator serves at `/notebooks/`. Cell types (markdown / esl / eigenql / typescript / program-run / chart), the file format, publish-to-layer, the patent-analysis and kinase-institutions demos, KaTeX math rendering.

15. **[Tags, branches, and history](15-tags-branches-history.md)** — the workspace rail's chain destinations: Branches (switch / create / delete), Tags (immutable named refs that pin against GC), History (chain walker + time-travel read-pin). The BranchBar at the top, the create-branch dialog's four start-from modes, mental model of mutable vs. immutable.

16. **[Merge resolution](16-merge-resolution.md)** — folding one branch into another when contributions conflict. The six-state flow (loading → picking → previewing → acknowledging → committing → done), the four strategies (Witness / Rename / SchemaQuotient / Restructure with KeepBoth/KeepOne/KeepNeither sub-flavours), the cascade gate, merge-resolution provenance records, off-span witness discovery, worked examples, CLI mirror.

17. **[TypeScript SDK](17-typescript-sdk.md)** — `@eigenius/client` and the `Eigen` class. The SDK that powers the notebook, also usable from your own code. Five-line examples for inspect / query / load / runProgramByIri / layerTopology / publishNotebook.

18. **[Appendix](18-appendix.md)** — environment variables, file locations, source index, related documents.

## Julia institution tutorials

Slow-walk tutorials for the five v1 Julia institutions live in
[`julia-institutions/`](julia-institutions/). Read the
[intervals tutorial](julia-institutions/intervals-institution-tutorial.md)
first for the substrate plumbing slow-walk; then the others go domain-specific.

## Lean institution tutorial

The platform's first verification institution (D28). In-process via
`nanoda_lib` for the verification side; substrate-hosted for the
authoring side. Walks the closed audit chain D28 §5.7 promises against
the [`lean-verification`](../../../notebooks/examples/lean-verification.json)
notebook. See [`lean-institution/`](lean-institution/).

When bumping the pinned Lean toolchain, follow the checklist at
[`docs/notes/lean-toolchain-upgrade.md`](../../notes/lean-toolchain-upgrade.md)
— the substrate's image-digest model treats every toolchain change as
a new content-addressed `LeanEnvironment`, so existing verified proofs
stay valid against their original env digest.

## Related documents

- [**ESL user guide**](../esl/README.md) — surface syntax for ontologies and programs
- [**EigenQL user guide**](../eigenql/README.md) — surface syntax for queries
- [**Formula language guide**](../formula/README.md) — chain-mirrored EigenTT fragment, shared by every numerical institution
- [**D13 Durable kernel state**](../../design/d13-durable-kernel-state.md) — `serve --db` spec
- [**D12 WASM extensibility**](../../design/d12-wasm-extensibility.md) — capability levels and host imports
- [**D14 Institution Realisation**](../../design/d14-institution-realisation.md) — institution model (supersedes D10), the protocol contract for chapters 10 and 11
- [**D26 Runtime substrate**](../../design/d26-runtime-substrate.md), [**D29 Mirror generator**](../../design/d29-runtime-mirror-generator.md), [**D31 Institution lifecycle**](../../design/d31-runtime-language-substrate-institution-lifecycle.md) — the substrate specs
- [**D32 Chain-mirrored EigenTT inductives**](../../design/d32-chain-mirrored-mini-tt-inductives.md) — the formula-language design spec
- [**D6 Execution architecture**](../../design/d6-execution-architecture.md) — kernel ↔ orchestrator boundary

The full design-document set lives in [`docs/design/`](../../design/).

---

Ready to start? → **[1. Introduction](01-introduction.md)**
