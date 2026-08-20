# 13. Troubleshooting and FAQ

Common issues organised by symptom. For each, the diagnosis and the fix.

## 13.1. Build failures

### `error: failed to run custom build command for prost-build`

**Cause:** `protoc` not installed.

**Fix:** Install `protobuf-compiler` (Ubuntu/WSL: `apt install protobuf-compiler`; macOS: `brew install protobuf`).

### `error: linker 'cc' not found`

**Cause:** No C/C++ toolchain (Ubuntu/WSL).

**Fix:** `sudo apt-get install -y build-essential`.

### `error: failed to run custom build command for librocksdb-sys`

**Cause:** `libclang-dev` missing — bindgen can't read RocksDB's headers.

**Fix:** `sudo apt-get install -y libclang-dev`.

### `error[E0658]: ...` referencing a Rust feature

**Cause:** `rustc` older than 1.97, the version `deploy/Dockerfile.kernel` builds with. Nothing enforces it locally — there is no `rust-version` key in `Cargo.toml` and no `rust-toolchain` file — so an older toolchain shows up as a dependency compile error rather than an MSRV message.

**Fix:** `rustup update` to get the latest stable.

### `cannot locate Lean's include directory` from `eigenius-lean-worker`'s build script

**Cause:** No Lean toolchain on the host. `crates/eigenius-lean-worker/build.rs` compiles a C bridge against Lean's `lean.h` and panics when it cannot find it. This fails the whole `cargo build --workspace`, not just the Lean crate.

**Fix:** Install elan and the pinned toolchain (see [chapter 2 §2.1](02-installation.md#21-required-toolchain)), or point `EIGENIUS_LEAN_INCLUDE_DIR` at a directory containing `lean/lean.h`.

### Deno `Specifier "..." was not found`

**Cause:** Deno cache stale or partial.

**Fix:** `cd orchestration && deno cache --reload src/main.ts`.

## 13.2. Server startup

### `Error: Address already in use (os error 98)` on port 50051

**Cause:** Another `eigenius serve` process is already running on the port.

**Fix:** Find it with `lsof -i :50051` (or `ss -tlnp | grep 50051`) and kill it, or start with `--port <other>`.

### `Error: IO error: While lock file: ... LOCK: Resource temporarily unavailable`

**Cause:** RocksDB lock file held by another process. Common causes:
- `eigenius serve --db <path>` already running.
- Previous `serve` crashed and left the lock (rare; RocksDB usually cleans up).
- `eigenius db stats/compact/export` running concurrently.

**Fix:** Stop the holding process. If you're sure nothing's holding it, delete the `LOCK` file in the database directory and retry.

### Drift refusal: `embedded ontology hash mismatch`

```
Error: embedded ontology 'urn:eigenius:core' hash differs from persisted manifest
  expected: ...
  found:    ...
```

**Cause:** You upgraded `eigenius` to a version with a different embedded ontology, and tried to start it against an existing database.

**Fix:** Either roll back to the prior version, or migrate. The migration path: export with the prior version (`eigenius db export`), delete the database, restart with the new version (re-seeds the manifest), re-load the export.

### Kernel hangs on startup with `Connection refused (orchestrator)`

**Cause:** `--orchestrator <url>` points at an orchestrator that isn't running, or that's reachable but on a different port.

**Fix:** Start the orchestrator first; verify with `curl http://localhost:8080/health`. The kernel won't fail outright on a missing orchestrator (some operations work without it), but it will retry the connection — which can look like a hang.

## 13.3. Orchestrator

### `Error: Module not found "ai" or "@ai-sdk/anthropic"`

**Cause:** Deno hasn't cached the dependency tree.

**Fix:** `cd orchestration && deno cache src/main.ts`.

### A provider auth error partway through a run, not at startup

**Cause:** Started without `EIGENIUS_MOCK_LLM=true` and without `ANTHROPIC_API_KEY` set.

There is **no startup check**. No orchestrator source file reads `ANTHROPIC_API_KEY` — its only occurrence under `orchestration/src/` is a doc comment; the Anthropic SDK picks the variable out of the environment itself, at the first dispatch. So a missing key starts cleanly, passes `/health`, and then fails inside the first `CompleteText` or `CompleteJson` call with whatever the provider returns. Earlier versions of this guide documented a startup error reading `ANTHROPIC_API_KEY required for non-mock LLM mode`; no such message exists anywhere in the tree.

**Fix:** Either export `ANTHROPIC_API_KEY=sk-ant-...` or start with `EIGENIUS_MOCK_LLM=true`. `EIGENIUS_MOCK_LLM` is compared strictly against the string `true` — `1` or `TRUE` do not enable mock mode.

### Orchestrator port already in use (8080)

**Cause:** Another service on 8080 (common — many development servers default to it).

**Fix:** Run with a different port: `EIGENIUS_ORCHESTRATOR_PORT=8081 deno run ...` and tell the kernel to use that endpoint: `eigenius serve --orchestrator http://localhost:8081`.

### Mock LLM responses not what you expected

**Cause:** Mock mode returns canned strings — not actual completions.

**Fix:** Switch to real mode (set `ANTHROPIC_API_KEY`, unset `EIGENIUS_MOCK_LLM`) for end-to-end testing of LLM behaviour.

## 13.4. CLI ↔ kernel connection

### `Error: connection error` when running CLI commands

**Cause:** Kernel server not running or `--endpoint` URL wrong.

**Fix:** Verify the kernel is up: `eigenius --endpoint http://localhost:50051 inspect "urn:eigenius:core:Class"`. If that succeeds, your CLI command's URL is wrong; if it fails, the kernel isn't running.

### `Error: gRPC status: ... INTERNAL ...`

**Cause:** Kernel-side error during the operation. The status message usually contains the underlying issue.

**Fix:** Check the kernel's stdout for an `ERROR` log line that fired at the same time. Common underlying causes: validation failure, type-check failure, missing layer, unregistered capability.

### CLI command says "load successful" but query returns nothing

**Cause:** In-process mode (no `--endpoint`) — the load happened against an ephemeral in-memory chain that's discarded when the CLI exits.

**Fix:** Either use `--endpoint <url>` with a running kernel (load and query against the same chain), or use the `--file` option of `query` to load and query in one invocation.

## 13.5. Capabilities

### `capability install` / `--capability` / `--kind` are not recognised

**Cause:** They were removed with the WASM extensibility path (2026-07-08). `CapabilityCommands` has three variants — `list`, `inspect`, `test` — and no `install`; there is no `--capability` flag and no `--kind` flag anywhere in the CLI. `wasmtime` is not a workspace dependency, so there is no fuel budget, no linear-memory limit and no WIT-world check to hit either. See [chapter 9](09-wasm-components.md) for the historical record.

**Fix:** Components and institutions are declared as ontology resources and committed with `eigenius load`; execution goes through the runtime substrate ([chapter 11](11-runtime-substrate.md)). See [chapter 4 §4.10](04-cli-reference.md#410-capability-commands).

### `capability list` doesn't show a component you committed

**Cause:** The declaration did not commit, or it committed to a branch the kernel is not serving from, or the component IRI is unregistered — an unregistered component IRI is not an error at dispatch, it returns its input unchanged.

**Fix:** `eigenius --endpoint <url> inspect <iri>` to confirm the resource is in the chain the kernel resolves against, and check the branch you loaded onto.

### The orchestrator cannot reach the kernel

**Fix:** From the orchestrator container, check the *kernel's* port with a gRPC call, not an HTTP one. **The kernel serves no HTTP endpoint** — `curl http://kernel:50051/health` cannot work; the port speaks gRPC and gRPC-Web only. The working probe is the one the compose file's healthcheck uses:

```bash
eigenius --endpoint http://kernel:50051 inspect "urn:eigenius:core:Class"
```

`/health` exists on the *orchestrator* (port 8080), not the kernel.

## 13.6. Layer / data issues

### `Validation failed: required property 'X' missing on resource 'Y'`

**Cause:** A loaded resource doesn't carry every property its class declares as `requires`.

**Fix:** Add the missing property, or change the class's `requires` to `recommends` if it should be optional.

### `Error: class 'urn:...' not found in layer chain`

**Cause:** A resource references a class IRI that isn't loaded into the layer chain.

**Fix:** Load the class's defining file before the resource that uses it. Order matters at load time, even though resolution at query time walks the full chain.

### `Error: subclass cycle detected`

**Cause:** A class declares a `subclass_of` that transitively reaches back to itself.

**Fix:** Find the cycle in your ontology and break it. Subclass chains must be acyclic.

## 13.7. Performance

### Queries slow down over time

**Cause:** RocksDB needs compaction (with persistent mode), or the layer chain has grown deep.

**Fix:** Run `eigenius db compact <path>` on your database, with the kernel stopped. If query time is dominated by deep layer-chain walks, consolidate the chain: `eigenius --endpoint <url> db consolidate <from-hex>..<to-hex>` collapses an inclusive layer range into one resolve-equivalent layer. It ships — re-loading into a fresh database is no longer the workaround. Run it with `--dry-run` first to see the cost and the predicted layer id. See [chapter 4 §4.5](04-cli-reference.md#45-database-commands) and [D25](../../design/d25-chain-consolidation.md).

### `eigenius run` is slow on programs with many IO calls

**Cause:** Each component dispatch involves a kernel→orchestrator→LLM round trip. With cold caches and a real LLM, latency dominates.

**Fix:** For repeated runs of the same program over the same input, the kernel's trace store memoises component dispatches — the second run is much faster than the first. For development iteration, use `EIGENIUS_MOCK_LLM=true` to skip the LLM round-trip.

## 13.8. Frequently-asked questions

### Can I run the kernel without the orchestrator?

Yes, for read-only operations (queries, inspection, type-check). For programs that dispatch IO components (`CompleteText`, `CompleteJson`, custom IO WASM components), the orchestrator is required.

### Can I use a different LLM provider?

The orchestrator currently ships with the Anthropic adapter. The Vercel AI SDK supports other providers (OpenAI, Google, etc.); adding one means writing a new adapter in [`orchestration/src/llm/`](../../../orchestration/src/llm/) and swapping it in `main.ts`. Filed under future work.

### How do I version my ontologies?

Conventionally, version through the URI: `urn:my-org:ontology:v1`, `urn:my-org:ontology:v2`. Layers are immutable, so loading the v2 ontology adds new resources without disturbing v1 — both versions are queryable in parallel against the chain.

### How do I delete a resource?

Layers are immutable, so nothing is deleted in place. Two shapes exist. Load a new layer that supersedes the resource — later layers shadow earlier ones on resolve. Or tombstone it: `eigenius load <file> --explicit-tombstone <iri>` commits a tombstone alongside the new layer, and `--commit-policy cascade` additionally tombstones lower-layer resources that the new layer's class redefinitions retroactively invalidate (D41 §3.3, §10.1).

### Can I run multiple kernels against the same database?

No. RocksDB takes an exclusive directory lock; only one process at a time can open `--db <path>`. This is also why `db stats`, `db compact` and `db export` need the kernel stopped. For horizontal scaling, run multiple kernel instances each with its own database.

### Does the kernel auto-restart on crash?

The kernel itself doesn't supervise itself. Wrap it in a process supervisor:
- **systemd** with `Restart=always` for bare-metal hosts.
- **Docker Compose** with `restart: unless-stopped` for containerised setups.
- **ContainerApps** auto-restarts crashed containers by default.

### How do I rotate the `ANTHROPIC_API_KEY`?

In container environments, update the env var (or the Key Vault secret) and restart the orchestrator. The kernel doesn't see the key — only the orchestrator does.

### Where do reasoning traces live?

When `--db` is set, in the **default** column family under the key prefix `trace:<key_hex>`. There is no `traces` column family: the only column families the store opens are `cf_text`, `cf_vec` and `cf_embed_cache`. The trace shape is specified in [D6b — Reasoning trace schema](../../design/d6b-reasoning-trace-schema.md). Without `--db`, traces are in-memory and discarded on kernel exit.

### How do I clean out old traces?

There is no trace eviction policy and no CLI surface for garbage collection. For now: `eigenius db export` to capture what you want, drop the database, restart fresh.

### Why does my `--endpoint` URL say `localhost` work locally but not from a container?

Containers don't share the host's `localhost` namespace. From inside a container, `localhost` refers to the *container's* network. To reach a service on the host: use the host's IP, or (in Docker Compose) use the service name (`http://kernel:50051`).

---

Next: **[14. Notebook →](14-notebook.md)**
