# 13. Appendix

## 13.1. Environment variables

| Variable | Default | Used by | Effect |
|---|---|---|---|
| `EIGENIUS_DB` | (none, in-memory) | `eigenius serve` | Path to the RocksDB persistence directory |
| `EIGENIUS_ORCHESTRATOR_ENDPOINT` | (none) | `eigenius serve` | Kernel's URL for the orchestrator (alternative to `--orchestrator`) |
| `EIGENIUS_KERNEL_ENDPOINT` | `http://localhost:50051` | Orchestrator | Endpoint the orchestrator uses for kernel callbacks (read/query host imports) |
| `EIGENIUS_ORCHESTRATOR_PORT` | `8080` | Orchestrator | Port the orchestrator binds to |
| `EIGENIUS_MOCK_LLM` | `false` | Orchestrator | When `true`, swap real LLM handlers for canned mock responses |
| `ANTHROPIC_API_KEY` | (none) | Orchestrator | Required when `EIGENIUS_MOCK_LLM` is unset |
| `RUSTFLAGS` | (none) | `cargo` | `RUSTFLAGS="-D warnings"` upgrades clippy warnings to errors (used by `just check`) |

CLI commands also accept `--endpoint <url>` as an alternative to setting an env var; the flag takes precedence.

## 13.2. File and directory locations

| Location | Contents |
|---|---|
| `target/debug/` | Workspace build artifacts (debug profile) |
| `target/debug/eigenius` | The CLI binary |
| `target/release/` | Workspace build artifacts (release profile) |
| `examples/wasm-*/target/wasm32-unknown-unknown/debug/*.wasm` | WASM example binaries |
| `kernel/tests/fixtures/*.wasm` | Test fixtures copied from WASM examples |
| `~/.cache/deno/` | Deno-cached TypeScript dependencies |
| `<rocksdb-path>/` (e.g. `/var/lib/eigenius`) | Persisted state when `serve --db` is used |

## 13.3. Default ports

| Port | Service | Configuration |
|---|---|---|
| 50051 | Kernel gRPC | `eigenius serve --port <N>` |
| 8080 | Orchestrator HTTP | `EIGENIUS_ORCHESTRATOR_PORT=<N>` |

## 13.4. The four embedded ontology layers

Loaded at every kernel startup; their parent-pointer chain forms the bootstrap:

| Layer | IRI base | Source |
|---|---|---|
| core | `urn:eigenius:core` | [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json) |
| program | `urn:eigenius:program` | [`ontologies/program/program-ontology.json`](../../../ontologies/program/program-ontology.json) |
| reflection | `urn:eigenius:reflection` | (embedded — reasoning traces, epistemic categories) |
| institution | `urn:eigenius:institution` | (embedded — institution and comorphism classes) |

When `serve --db <path>` is used, a SHA-256 manifest of these is written on first start and verified on subsequent starts (drift refusal — see [chapter 6](06-database-management.md) §6.3).

## 13.5. Source index — implementation files referenced in this guide

### CLI

- [`cli/src/main.rs`](../../../cli/src/main.rs) — every subcommand, the `Commands` enum is the source of truth for command shapes

### Kernel

- [`kernel/src/server/`](../../../kernel/src/server/) — gRPC service definitions
- [`kernel/src/bootstrap/`](../../../kernel/src/bootstrap/) — embedded ontology loader
- [`kernel/src/storage/`](../../../kernel/src/storage/) — storage interface traits
- [`kernel/src/capability/`](../../../kernel/src/capability/) — WASM capability hosting, component/institution registries
- [`kernel/src/institution/mod.rs`](../../../kernel/src/institution/mod.rs) — `FiberReasoner` trait, `InstitutionRegistry`

### Storage backends

- [`storage/memory/`](../../../storage/memory/) — in-memory backend (default for `serve` without `--db`)
- [`storage/rocksdb/`](../../../storage/rocksdb/) — RocksDB backend (`serve --db`)
- [`storage/tikv/`](../../../storage/tikv/) — TiKV backend (placeholder)
- [`storage/indexing/`](../../../storage/indexing/) — SPO/POS/OPS triple index construction

### WASM runtime and SDK

- [`crates/wasm-runtime/`](../../../crates/wasm-runtime/) — Wasmtime integration, fuel/memory limits
- [`sdk/wasm-sdk/`](../../../sdk/wasm-sdk/) — Rust SDK for authoring components and institutions
- [`wit/eigenius-component.wit`](../../../wit/eigenius-component.wit) — WIT interface contracts

### Examples

- [`examples/wasm-cbor-echo/`](../../../examples/wasm-cbor-echo/) — minimum viable component
- [`examples/wasm-doc-validator/`](../../../examples/wasm-doc-validator/) — pure component with typed I/O
- [`examples/wasm-read-query-probe/`](../../../examples/wasm-read-query-probe/) — read-capability component
- [`examples/wasm-http-shout/`](../../../examples/wasm-http-shout/) — IO component dispatching `CompleteText`
- [`examples/wasm-ordering-institution/`](../../../examples/wasm-ordering-institution/) — institution implementation

### Orchestrator

- [`orchestration/src/main.ts`](../../../orchestration/src/main.ts) — entry point, registry setup
- [`orchestration/src/components/`](../../../orchestration/src/components/) — `CompleteText`, `CompleteJson`, registry
- [`orchestration/src/llm/adapter.ts`](../../../orchestration/src/llm/adapter.ts) — Anthropic adapter
- [`orchestration/src/mcp/server.ts`](../../../orchestration/src/mcp/server.ts) — MCP tool surface
- [`orchestration/src/wasm/`](../../../orchestration/src/wasm/) — WASM addon hosting (IO components)

### Demo scripts

- [`demo/run.sh`](../../../demo/run.sh) — basic document demo
- [`demo/patent/run.sh`](../../../demo/patent/run.sh) — patent analysis pipeline
- [`demo/wasm/run.sh`](../../../demo/wasm/run.sh) — WASM extensibility demo

### Deployment

- [`docker-compose.yml`](../../../docker-compose.yml) — local stack composition
- [`deploy/Dockerfile.kernel`](../../../deploy/Dockerfile.kernel) — kernel image
- [`deploy/Dockerfile.orchestration`](../../../deploy/Dockerfile.orchestration) — orchestrator image
- [`deploy/bicep/main.bicep`](../../../deploy/bicep/main.bicep) — Azure ContainerApps orchestrating template
- [`deploy/bicep/modules/`](../../../deploy/bicep/modules/) — per-resource Bicep modules
- [`deploy/bicep/parameters/`](../../../deploy/bicep/parameters/) — staging/production environment overrides

### Build / task automation

- [`justfile`](../../../justfile) — task recipes (`build`, `test`, `check`, `up`, `serve`, etc.)

## 13.6. Related documents

- [**ESL user guide**](../esl/README.md) — the surface language for ontologies and programs
- [**EigenQL user guide**](../eigenql/README.md) — the query language
- [**D1 — Eigon serialization format**](../../design/d1-eigon-serialization-format.md) — Eigon-JSON spec
- [**D2 — EigenQL specification**](../../design/d2-eigenql-specification.md) — EigenQL spec
- [**D6 — Execution architecture**](../../design/d6-execution-architecture.md) — kernel ↔ orchestrator boundary
- [**D6b — Reasoning trace schema**](../../design/d6b-reasoning-trace-schema.md) — trace storage
- [**D7 — ESL surface syntax**](../../design/d7-esl-surface-syntax.md) — ESL spec
- [**D8 — CompleteJson component**](../../design/d8-complete-json-component.md) — structured LLM output
- [**D10 — Grothendieck institution protocol**](../../design/d10-grothendieck-institution-protocol.md) — institution model
- [**D12 — WASM extensibility**](../../design/d12-wasm-extensibility.md) — capability levels, host imports, fuel/memory
- [**D13 — Durable kernel state**](../../design/d13-durable-kernel-state.md) — `serve --db` spec, restart re-registration
- [**D21 — Task traces and checkpointing**](../../design/d21-task-traces-and-checkpointing.md) — task model and resume sweep

The full design-document set lives in [`docs/design/`](../../design/).

## 13.7. Phase status

The platform is currently complete through Phase 11e (see top-level [README.md](../../../README.md)):

- Phases 0–9: kernel + orchestrator + LLM integration + WASM extensibility + persistence + tasks
- Phase 10: kernel completeness (ontology-as-types resolution)
- Phase 11a–e: type theory extensions (Map/Reduce, inductive types, institution decide procedures, comorphisms, ESL+EigenQL surfaces)

Next: Phase 12 (worked institution examples — life-science demos drawing on Phase 11's surface).

---

Return to **[README](README.md)**.
