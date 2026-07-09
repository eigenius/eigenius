# 18. Appendix

## 18.1. Environment variables

| Variable | Default | Used by | Effect |
|---|---|---|---|
| `EIGENIUS_DB` | (none, in-memory) | `eigenius serve` | Path to the RocksDB persistence directory |
| `EIGENIUS_HOME` | workspace root in dev / `/opt/eigenius` in containers | `eigenius env build` | Base path for resolving the default Julia worker source (`$EIGENIUS_HOME/julia/runtime-worker/`). Overridden per invocation by `--worker-source-dir`. |
| `EIGENIUS_ORCHESTRATOR_ENDPOINT` | (none) | `eigenius serve` | Kernel's URL for the orchestrator (alternative to `--orchestrator`) |
| `EIGENIUS_KERNEL_ENDPOINT` | `http://localhost:50051` | Orchestrator | Endpoint the orchestrator uses for kernel callbacks (read/query host imports) |
| `EIGENIUS_ORCHESTRATOR_PORT` | `8080` | Orchestrator | Port the orchestrator binds to |
| `EIGENIUS_MOCK_LLM` | `false` | Orchestrator | When `true`, swap real LLM handlers for canned mock responses |
| `ANTHROPIC_API_KEY` | (none) | Orchestrator | Required when `EIGENIUS_MOCK_LLM` is unset |
| `EIGENIUS_EXTFILE_CACHE_DIR` | (none) | `eigenius data provision`, kernel recompute | Depot extfile-cache root where pinned external files materialize (`<cache>/<sha256>/<name>`). Default cache root for `data provision` when `--cache-root` is omitted; the directory the kernel reads for native file-backed `SampleSet` recompute (D53 §6.1 / §7). |
| `EIGENIUS_OXEN_BIN` | `oxen` | Runtime substrate (Oxen fetch) | Path to the prebuilt `oxen` client binary used to fetch `oxen://` references (D53 §2). |
| `EIGENIUS_OXEN_SCHEME` | `https` | Runtime substrate (Oxen fetch) | URL scheme the Oxen client uses for the remote host (set `http` for a local/plaintext Oxen server). |
| `OXEN_CONFIG_DIR` | Oxen client default | Runtime substrate (Oxen auth) | Oxen config dir holding `auth_config.toml` (per-host bearer token). The token is substrate-side only and never enters a worker image (D53 §10). |
| `RUSTFLAGS` | (none) | `cargo` | `RUSTFLAGS="-D warnings"` upgrades clippy warnings to errors (used by `just check`) |

CLI commands also accept `--endpoint <url>` as an alternative to setting an env var; the flag takes precedence.

## 18.2. File and directory locations

| Location | Contents |
|---|---|
| `target/debug/` | Workspace build artifacts (debug profile) |
| `target/debug/eigenius` | The CLI binary |
| `target/release/` | Workspace build artifacts (release profile) |
| `~/.cache/deno/` | Deno-cached TypeScript dependencies |
| `<rocksdb-path>/` (e.g. `/var/lib/eigenius`) | Persisted state when `serve --db` is used |
| `/var/lib/eigenius/substrate-depot/` | Runtime-substrate depot path (chapter 11). Bind-mounted into the orchestrator at the *same* path so worker UDS sockets are reachable from both host and orchestrator container. |
| `julia/runtime-worker/` | Julia worker source baked into substrate env images |
| `julia/common/EigeniusJuliaCommon/` | Substrate-side Julia utilities shared across institutions |
| `julia/institutions/<institution>/` | Per-institution handler package (`Project.toml` + `src/`) and chain declarations |
| `julia/comorphisms/` | Cross-institution comorphism declarations |

## 18.3. Default ports

| Port | Service | Configuration |
|---|---|---|
| 50051 | Kernel gRPC | `eigenius serve --port <N>` |
| 8080 | Orchestrator HTTP | `EIGENIUS_ORCHESTRATOR_PORT=<N>` |

## 18.4. The four embedded ontology layers

Loaded at every kernel startup; their parent-pointer chain forms the bootstrap:

| Layer | IRI base | Source |
|---|---|---|
| core | `urn:eigenius:core` | [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json) |
| program | `urn:eigenius:program` | [`ontologies/program/program-ontology.json`](../../../ontologies/program/program-ontology.json) |
| reflection | `urn:eigenius:reflection` | (embedded — reasoning traces, epistemic categories) |
| institution | `urn:eigenius:institution` | (embedded — institution and comorphism classes) |

When `serve --db <path>` is used, a SHA-256 manifest of these is written on first start and verified on subsequent starts (drift refusal — see [chapter 6](06-database-management.md) §6.3).

## 18.5. Source index — implementation files referenced in this guide

### CLI

- [`cli/src/main.rs`](../../../cli/src/main.rs) — every subcommand, the `Commands` enum is the source of truth for command shapes

### Kernel

- [`kernel/src/server/`](../../../kernel/src/server/) — gRPC service definitions
- [`kernel/src/bootstrap/`](../../../kernel/src/bootstrap/) — embedded ontology loader
- [`kernel/src/storage/`](../../../kernel/src/storage/) — storage interface traits
- [`kernel/src/capability/`](../../../kernel/src/capability/) — institution capability registration; `registration.rs` does chain-scan auto-registration for external + in-process backends
- [`kernel/src/institution/runtime.rs`](../../../kernel/src/institution/runtime.rs) — D14 `Institution` trait, `InstitutionRuntime`
- [`kernel/src/institution/registry.rs`](../../../kernel/src/institution/registry.rs) — `InstitutionIndex` (derived from chain scan)

### Storage backends

- [`storage/memory/`](../../../storage/memory/) — in-memory backend (default for `serve` without `--db`)
- [`storage/rocksdb/`](../../../storage/rocksdb/) — RocksDB backend (`serve --db`)
- [`storage/tikv/`](../../../storage/tikv/) — TiKV backend (placeholder)
- [`kernel/src/layer/index.rs`](../../../kernel/src/layer/index.rs) — per-layer triple index trait + in-memory impl (Phase 14h)
- [`storage/rocksdb/src/triple_index.rs`](../../../storage/rocksdb/src/triple_index.rs) — RocksDB-backed triple index (Phase 14h)

### Runtime substrate (chapter 11)

- [`crates/runtime-substrate/`](../../../crates/runtime-substrate/) — substrate hosting layer (loaded into the orchestrator)
- [`crates/runtime-substrate/src/language_runtime.rs`](../../../crates/runtime-substrate/src/language_runtime.rs) — `LanguageRuntime` trait
- [`crates/runtime-substrate/src/spawner/`](../../../crates/runtime-substrate/src/spawner/) — container lifecycle
- [`crates/runtime-substrate/src/image_build/`](../../../crates/runtime-substrate/src/image_build/) — buildah-driven env image construction
- [`crates/runtime-substrate/src/rpc/`](../../../crates/runtime-substrate/src/rpc/) — Eigon-CBOR-over-UDS protocol
- [`crates/runtime-substrate/src/mirror_generator.rs`](../../../crates/runtime-substrate/src/mirror_generator.rs) — closure walker for chain shapes
- [`crates/eigenius-julia/`](../../../crates/eigenius-julia/) — Julia v1 instantiation of `LanguageRuntime`
- [`julia/runtime-worker/`](../../../julia/runtime-worker/) — Julia worker (PID 1 in env images)
- [`julia/institutions/`](../../../julia/institutions/) — five v1 Julia institutions (Symbolics, IntervalArithmetic, Catalyst, DiffEq, JuMP-HiGHS)
- [`julia/comorphisms/`](../../../julia/comorphisms/) — cross-institution comorphism declarations

### Examples

WASM example components (`examples/wasm-*`) were removed on 2026-07-08 along
with the rest of WASM extensibility.

### Orchestrator

- [`orchestration/src/main.ts`](../../../orchestration/src/main.ts) — entry point, registry setup
- [`orchestration/src/components/`](../../../orchestration/src/components/) — `CompleteText`, `CompleteJson`, registry
- [`orchestration/src/llm/adapter.ts`](../../../orchestration/src/llm/adapter.ts) — Anthropic adapter
- [`orchestration/src/mcp/server.ts`](../../../orchestration/src/mcp/server.ts) — MCP tool surface

### Demo scripts

- [`demo/run.sh`](../../../demo/run.sh) — basic document demo
- [`demo/patent/run.sh`](../../../demo/patent/run.sh) — patent analysis pipeline

### Deployment

- [`docker-compose.yml`](../../../docker-compose.yml) — local stack composition
- [`deploy/Dockerfile.kernel`](../../../deploy/Dockerfile.kernel) — kernel image
- [`deploy/Dockerfile.orchestration`](../../../deploy/Dockerfile.orchestration) — orchestrator image
- [`deploy/bicep/main.bicep`](../../../deploy/bicep/main.bicep) — Azure ContainerApps orchestrating template
- [`deploy/bicep/modules/`](../../../deploy/bicep/modules/) — per-resource Bicep modules
- [`deploy/bicep/parameters/`](../../../deploy/bicep/parameters/) — staging/production environment overrides

### Build / task automation

- [`justfile`](../../../justfile) — task recipes (`build`, `test`, `check`, `up`, `serve`, etc.)

## 18.6. Related documents

- [**ESL user guide**](../esl/README.md) — the surface language for ontologies and programs
- [**EigenQL user guide**](../eigenql/README.md) — the query language
- [**D1 — Eigon serialization format**](../../design/d1-eigon-serialization-format.md) — Eigon-JSON spec
- [**D2 — EigenQL specification**](../../design/d2-eigenql-specification.md) — EigenQL spec
- [**D6 — Execution architecture**](../../design/d6-execution-architecture.md) — kernel ↔ orchestrator boundary
- [**D6b — Reasoning trace schema**](../../design/d6b-reasoning-trace-schema.md) — trace storage
- [**D7 — ESL surface syntax**](../../design/d7-esl-surface-syntax.md) — ESL spec
- [**D8 — CompleteJson component**](../../design/d8-complete-json-component.md) — structured LLM output
- [**D14 — Institution Realisation**](../../design/d14-institution-realisation.md) — institution model (supersedes D10); §9.3 covers comorphism chain reinsertion
- [**D12 — WASM extensibility**](../../design/d12-wasm-extensibility.md) — capability levels, host imports, fuel/memory
- [**D13 — Durable kernel state**](../../design/d13-durable-kernel-state.md) — `serve --db` spec, restart re-registration
- [**D21 — Task traces and checkpointing**](../../design/d21-task-traces-and-checkpointing.md) — task model and resume sweep
- [**D26 — Runtime substrate**](../../design/d26-runtime-substrate.md) — substrate hosting layer, `LanguageRuntime` trait
- [**D29 — Mirror generator**](../../design/d29-runtime-mirror-generator.md) — closure walker, content-addressed mirror IRIs
- [**D31 — Runtime-language-substrate institution lifecycle**](../../design/d31-runtime-language-substrate-institution-lifecycle.md) — install flow, env image lifecycle
- [**D32 — Chain-mirrored EigenTT inductives**](../../design/d32-chain-mirrored-mini-tt-inductives.md) — `formulas:FormulaTerm` as a EigenTT fragment on the chain

The full design-document set lives in [`docs/design/`](../../design/).

## 18.7. Phase status

The platform is currently complete through Phase 11e (see top-level [README.md](../../../README.md)):

- Phases 0–9: kernel + orchestrator + LLM integration + WASM extensibility + persistence + tasks
- Phase 10: kernel completeness (ontology-as-types resolution)
- Phase 11a–e: type theory extensions (Map/Reduce, inductive types, institution decide procedures, comorphisms, ESL+EigenQL surfaces)

Next: Phase 12 (worked institution examples — life-science demos drawing on Phase 11's surface).

---

Return to **[README](README.md)**.
