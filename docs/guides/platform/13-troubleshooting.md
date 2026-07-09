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

**Cause:** `rustc` older than the workspace MSRV (1.97).

**Fix:** `rustup update` to get the latest stable.

### `cargo component: command not found`

**Cause:** WASM example builds need `cargo-component`.

**Fix:** `cargo install cargo-component`. If you're not building WASM examples, run `cargo build --workspace` directly to skip them.

### WASM target not installed

```
error: the 'wasm32-unknown-unknown' target may not be installed
```

**Fix:** `rustup target add wasm32-unknown-unknown`.

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

### `ANTHROPIC_API_KEY required for non-mock LLM mode`

**Cause:** Started without `EIGENIUS_MOCK_LLM=true` and without `ANTHROPIC_API_KEY` set.

**Fix:** Either export `ANTHROPIC_API_KEY=sk-ant-...` or start with `EIGENIUS_MOCK_LLM=true`.

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

## 13.5. Capability install

### `Error: WIT mismatch: expected 'eigenius-component', got '...'`

**Cause:** The WASM binary was built against a different WIT world than the kind/level you're installing. Common causes:
- Installing a `eigenius-component-io` binary as `--capability pure`.
- Installing a `eigenius-institution` binary as `--kind component`.

**Fix:** Match `--kind` and `--capability` to the WIT world the binary was built against. See the binary's source `Cargo.toml` `[package.metadata.component.target] world = "..."`.

### `Error: out of fuel`

**Cause:** Component execution exceeded the default 100M-instruction fuel budget.

**Fix:** For legitimate heavy computation, request a higher fuel limit in the capability's full-mode definition (see [chapter 9](09-wasm-components.md) §9.9). For unintentional infinite loops, fix the component code.

### `Error: memory limit exceeded`

**Cause:** Linear memory allocation over 64 MiB.

**Fix:** Process in smaller chunks, or request a higher memory limit.

### Install succeeds but `capability list` doesn't show the new IRI

**Cause:** The install went to the wrong host. IO-capability components install to the orchestrator; pure/read components install to the kernel. If the kernel and orchestrator are out of sync (e.g., the orchestrator's `EIGENIUS_KERNEL_ENDPOINT` is wrong), the registration may not propagate.

**Fix:** Verify the orchestrator can reach the kernel: from the orchestrator's container, `curl http://kernel:50051/health` (or whatever endpoint is configured). Restart both services with consistent endpoints.

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

**Fix:** Run `eigenius db compact <path>` on your database. If query time is dominated by deep layer-chain walks, consider periodically consolidating layers (planned for Phase 14; for now, the workaround is to re-load the consolidated state into a fresh database).

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

You don't, directly. Layers are immutable. The recommended pattern: load a new layer that supersedes the resource (Phase 14 will formalise this with explicit overlay/redaction semantics).

### Can I run multiple kernels against the same database?

No. RocksDB takes an exclusive directory lock; only one kernel process at a time can open `--db <path>`. For horizontal scaling, run multiple kernel instances each with its own database (and consider Phase 14's reconciliation work for keeping them consistent).

### Does the kernel auto-restart on crash?

The kernel itself doesn't supervise itself. Wrap it in a process supervisor:
- **systemd** with `Restart=always` for bare-metal hosts.
- **Docker Compose** with `restart: unless-stopped` for containerised setups.
- **ContainerApps** auto-restarts crashed containers by default.

### How do I rotate the `ANTHROPIC_API_KEY`?

In container environments, update the env var (or the Key Vault secret) and restart the orchestrator. The kernel doesn't see the key — only the orchestrator does.

### Where do reasoning traces live?

When `--db` is set, in the database under the `traces` column family. The trace shape is specified in [D6b — Reasoning trace schema](../../design/d6b-reasoning-trace-schema.md). Without `--db`, traces are in-memory and discarded on kernel exit.

### How do I clean out old traces?

There's currently no built-in trace eviction policy. For now: `eigenius db export` to capture what you want, drop the database, restart fresh. Trace lifecycle management is filed for Phase 14.

### Why does my `--endpoint` URL say `localhost` work locally but not from a container?

Containers don't share the host's `localhost` namespace. From inside a container, `localhost` refers to the *container's* network. To reach a service on the host: use the host's IP, or (in Docker Compose) use the service name (`http://kernel:50051`).

---

Next: **[14. Notebook →](14-notebook.md)**
