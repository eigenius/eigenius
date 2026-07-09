# D12b — Orchestrator-Side WASM Implementation Plan

> **Status: REMOVED (2026-07-08).** The orchestrator-side WASM IO host
> (napi-rs + wasmtime addon at `orchestration/native/`) was removed along
> with the rest of WASM extensibility (see [D12](d12-wasm-extensibility.md)).
> The orchestrator's runtime substrate (D26/D31 — Docker/OCI external
> institutions) is a separate mechanism and is unaffected. Retained as
> historical record.

**Status (historical):** Implemented (2026-04-20). All M1–M5 milestones landed and
every §12 open question is resolved. Cross-platform builds are declared
in the napi-rs config but only the Linux x86_64 target has been exercised
so far; `deno task build:addon` should Just Work on other hosts.
**Companion to:** D12 (WASM Extensibility)
**Spike report:** [spikes/napi-rs-async/REPORT.md](../../spikes/napi-rs-async/REPORT.md)

This plan operationalises D12 for the orchestrator side. D12 said *what*
orchestrator-hosted IO WASM components must do; this document says *how* we
build them into the production orchestrator given the spike's results.

## 1 · Goals and non-goals

**In scope for this phase:**
- Replace the `registerWasmComponent` stub with a real napi-rs + wasmtime
  path that compiles, caches, and executes `eigenius-component-io` world
  components.
- Wire the guest's `dispatch-component` host import to the orchestrator's
  existing `ComponentRegistry` so WASM can call `CompleteText`, `CompleteJSON`,
  or any other registered handler.
- Extract the small shared surface between kernel and orchestrator into a
  new `eigenius-wasm-runtime` crate.
- End-to-end test: `wasm-http-shout` installed into a layer, kernel forwards
  to orchestrator, orchestrator runs it, round-trips through the JS
  `CompleteText` handler.

**Additionally in scope (decided 2026-04-19):**
- `read-access` and `query-access` host imports bridged from the orchestrator
  back into the kernel via a gRPC callback. Done in M3/M4, not deferred.

**Deferred to later phases:**
- Automatic re-registration after orchestrator restart (tracked as a GitHub
  issue). v1 requires restarting both kernel and orchestrator together.
- Persistent binary storage on the orchestrator (v1 keeps binaries in memory).
- Dynamic reloading / hot-swap.
- Component metrics and fuel accounting exposed via `ComponentMetrics`.
- Parallel instance pools (single instance per call is fine at projected
  volumes; revisit when we have load data).
- Darwin / Windows distribution triples — v1 ships `x86_64-unknown-linux-gnu`
  only. Cross-compilation from Linux to macOS requires osxcross or a mac
  runner; revisit at release time.

## 2 · Architecture recap

```
┌─ kernel ─────────────────────────────────────────────────────────┐
│   Layer.scan → PendingIoComponent → RegisterWasmComponent ───┐   │
│                                                              │   │
│   Program dispatch ─→ RemoteComponent (gRPC) ──────────┐     │   │
└────────────────────────────────────────────────────────┼─────┼───┘
                                                        │     │ gRPC
                                         ComponentRequest│     │(binary)
                                                        ▼     ▼
┌─ orchestrator (Deno) ────────────────────────────────────────────┐
│   ComponentExecutor service (TS)                                 │
│      ├─ execute: routes to ComponentRegistry                     │
│      │     ├─ native handlers (CompleteText, CompleteJSON, …)    │
│      │     └─ WASM handler (calls into addon) ──────────┐        │
│      │                                                  │        │
│      └─ registerWasmComponent: ──────┐                  │        │
│                                      ▼                  ▼        │
│         ┌─ wasm/ (TS) ──────────────────────────────────────┐    │
│         │  WasmComponentRegistry  ─ stores IRI → handle     │    │
│         │  dispatchBridge        ─ routes dispatch-component│    │
│         │  cborBridge           ─ CBOR ↔ Eigon-JSON         │    │
│         │  loadAddon            ─ requires the .node file   │    │
│         └──────────────────────────┬──────────────────────┘      │
│                                    │ napi-rs FFI                 │
│         ┌─ native/ (Rust cdylib) ──▼──────────────────────┐      │
│         │  src/lib.rs: load_component, execute_component  │      │
│         │  depends on: eigenius-wasm-runtime, napi, wasmtime│    │
│         └────────────────────┬────────────────────────────┘      │
│                              │                                   │
└──────────────────────────────┼───────────────────────────────────┘
                               │
                    ┌──────────▼──────────────┐
                    │ eigenius-wasm-runtime   │ ← new shared crate
                    │  (used by kernel too)   │
                    └─────────────────────────┘
```

## 3 · Shared crate: `eigenius-wasm-runtime`

**New location:** `crates/wasm-runtime/` (workspace member).

Extracted from [kernel/src/capability/wasm_component.rs](../../kernel/src/capability/wasm_component.rs)
and [spikes/napi-rs-async/src/lib.rs](../../spikes/napi-rs-async/src/lib.rs). Keep
the surface small — interfaces differ enough between kernel and orchestrator
that forcing more into the shared crate costs more than it saves.

**Surface:**

```rust
// Engine construction (sync or async).
pub fn new_engine(async_support: bool) -> anyhow::Result<Engine>;

// Compile a component from bytes. Caller provides the Engine so that
// caching is the caller's concern.
pub fn compile_component(engine: &Engine, binary: &[u8]) -> anyhow::Result<Component>;

// Read the `component-iri` export (one-shot instantiate, no host imports
// beyond the core world). Returns the declared IRI.
pub fn read_component_iri<T>(engine: &Engine, component: &Component, state: T)
    -> anyhow::Result<String>;

// Config shared by kernel and orchestrator.
#[derive(Debug, Clone)]
pub struct WasmComponentConfig {
    pub fuel_limit: u64,
    pub memory_limit_pages: u32,
}

// Parse the result<component-result, string> return of `execute`.
pub fn parse_execute_result(val: &Val) -> anyhow::Result<Vec<u8>>;

// Turn a byte slice into the Val::List arguments that `execute` expects.
pub fn encode_execute_params(input: &[u8], argument: &[u8]) -> Vec<Val>;

// Capability level (shared identifier; kernel ignores Io, orchestrator only
// handles Io).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityLevel { Pure, Read, Io }
```

**Explicitly not in the shared crate:**
- Linker setup (kernel wires `read-access` against layer state; orchestrator
  wires `io-access` against a JS callback — the signatures don't compose)
- HostState / ResourceTable contents
- BuiltinComponent trait impl
- napi-rs types

**Refactor steps:**
1. Create `crates/wasm-runtime/` with the surface above, copying from kernel.
2. Update workspace `Cargo.toml` to include the new crate.
3. Switch `kernel/src/capability/wasm_component.rs` to use
   `eigenius_wasm_runtime::*` for the extracted pieces.
4. Switch `spikes/napi-rs-async/src/lib.rs` to use it (optional — the spike
   stays as-is for reference).
5. Verify `cargo test --workspace` still passes.

## 4 · Orchestrator native addon: `orchestration/native/`

**New crate location:** `orchestration/native/` — sibling of `orchestration/src/`.

**Cargo.toml (sketch):**
```toml
[package]
name = "eigenius-orchestrator-wasm"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
napi = { version = "3", default-features = false, features = ["napi8", "tokio_rt", "async"] }
napi-derive = "3"
tokio = { version = "1", features = ["rt-multi-thread", "sync", "time"] }
wasmtime = { version = "43", features = ["component-model", "async"] }
eigenius-wasm-runtime = { path = "../../crates/wasm-runtime", features = ["async"] }
anyhow = "1"
sha2 = "0.10"
```

**Exports (typed on the TS side):**

```ts
// Opaque handle to a compiled component. Internally a u64 id or Box index.
export type ComponentHandle = number;

// Compile a binary; may hit the on-disk cache or compile fresh. Idempotent
// when called with the same binary bytes. Returns the declared IRI and handle.
export function loadComponent(
  binary: Uint8Array,
  opts: { fuelLimit: number; memoryLimitPages: number },
): Promise<{ handle: ComponentHandle; componentIri: string }>;

// Release the compiled component and free its cache slot.
export function unloadComponent(handle: ComponentHandle): void;

// Execute a loaded component. All three callbacks correspond to the
// `io-access`, `read-access`, and `query-access` host imports. Errors
// from any callback surface to the guest as `err(string)` on the
// respective return type.
export interface WasmHostCallbacks {
  /** `io-access.dispatch-component` — route to other components. */
  dispatch: (iri: string, input: Uint8Array, argument: Uint8Array) => Promise<Uint8Array>;
  /** `read-access.resolve` — fetch a resource by IRI from the kernel. */
  resolve: (iri: string) => Promise<Uint8Array | null>;
  /** `query-access.query` — run an EigenQL query against the kernel. */
  query: (eigenql: string) => Promise<Uint8Array[]>;
}

export function executeComponent(
  handle: ComponentHandle,
  input: Uint8Array,
  argument: Uint8Array,
  callbacks: WasmHostCallbacks,
): Promise<Uint8Array>;
```

**Internal design:**

- **Engine lifecycle:** one `wasmtime::Engine` per addon-load, stored in a
  `OnceCell<Engine>`. `Config::async_support(true)`, `consume_fuel(true)`,
  `wasm_component_model(true)`.
- **Component cache:** `DashMap<ComponentHandle, Arc<CompiledComponent>>`
  where `CompiledComponent { component: Component, iri: String, config: WasmComponentConfig }`.
  Handle is a monotonically-incrementing u64.
- **Binary cache on disk:** before compiling, hash the binary (sha256), look
  up `~/.cache/eigenius/wasm/<hex>.cwasm`. If present, `Component::deserialize_file`;
  else `Component::from_binary` then `Component::serialize_file`.
- **Per-call Store:** fresh `Store` each `executeComponent` call. Set fuel
  from the component's `WasmComponentConfig`. Wire the `io-access` linker
  that forwards to the JS dispatch callback via `ThreadsafeFunction` + `FnArgs`
  (learnings from spike).
- **Callback types:** all three host imports use `ThreadsafeFunction` with
  `FnArgs<T>` tuples to spread into multiple JS args. Without `FnArgs`,
  napi-rs packs the tuple into a single JS array — the single largest
  debugging cost of the spike.
  - `dispatch`: `FnArgs<(String, Buffer, Buffer)>` → `Promise<Buffer>`
  - `resolve`: `FnArgs<(String,)>` → `Promise<Option<Buffer>>`
  - `query`: `FnArgs<(String,)>` → `Promise<Vec<Buffer>>`
- **Error mapping:** JS rejection inside the dispatch callback → return
  `Err::<_, String>(e.to_string())` from the wasmtime host function → guest
  observes `err(string)` branch. Don't swallow, don't return empty bytes.

## 5 · Orchestrator TS layer: `orchestration/src/wasm/`

**New files:**

- `wasm/loadAddon.ts` — `createRequire` + locate the built `.node` for the
  current platform. Throws a helpful error if the addon isn't built.
- `wasm/registry.ts` — `WasmComponentRegistry` keyed by component IRI:
  `register(iri, handle, metadata)`, `has(iri)`, `execute(iri, input, argument, callbacks)`.
- `wasm/hostBridge.ts` — builds the three host callbacks (`dispatch`,
  `resolve`, `query`) given a `ComponentRegistry` and a kernel gRPC client:
  - `dispatch` routes by IRI:
    - If another WASM component: call `WasmComponentRegistry.execute` (CBOR).
    - If a native TS handler: decode CBOR → Eigon-JSON → call handler →
      encode output back to CBOR. **This is the canonical boundary.**
  - `resolve` calls the kernel's existing resource-lookup RPC and returns
    the CBOR-encoded resource (or null).
  - `query` calls the kernel's EigenQL RPC and returns the result list.
- `wasm/cbor.ts` — thin wrapper over [`cbor-x`](https://www.npmjs.com/package/cbor-x)
  (JSR mirror) with encode/decode helpers specific to Eigon resources.
  Handles the property-keyed map layout from `eigon_cbor` so encode/decode
  round-trips cleanly with the SDK.

**Kernel-side additions:** resolve/query bridge needs the orchestrator to
have a gRPC client to the kernel (currently only kernel→orch). Add a
`KernelClient` to orchestrator startup; use the same resilient-startup
pattern (`connect_lazy`) the kernel uses for its `SharedOrchestratorClient`.

RPC surface already exists — reuse:
- `Inspect(iri) -> { found, resource: CBOR bytes }` → `read-access.resolve`.
  Return `None` to the guest when `found == false`.
- `Query(eigenql) -> QueryResponse { document: CBOR bytes }` →
  `query-access.query`. The TS client decodes the document, finds the
  ResultSet, and hands the guest the embedded rows as a list of CBOR
  buffers. See D2 Appendix A for the document shape.

Both RPCs already return CBOR, so no format conversion on the kernel side.

**Changes to existing files:**

- [orchestration/src/server/component_executor.ts](../../orchestration/src/server/component_executor.ts):
  - Replace `registerWasmComponent` stub: call `addon.loadComponent`, store
    resulting handle in `WasmComponentRegistry`, register a handler in the
    existing `ComponentRegistry` that dispatches to the WASM side with the
    bridge.
  - On execute for a WASM-backed IRI, the path is: `ComponentRegistry.execute`
    → WASM handler → `addon.executeComponent(handle, input, argument, dispatch)`.

- [orchestration/src/components/registry.ts](../../orchestration/src/components/registry.ts):
  No structural change expected — the WASM handler plugs in via the existing
  `register(iri, handler)` mechanism.

- [orchestration/src/main.ts](../../orchestration/src/main.ts):
  On startup, try loading the native addon. If missing, log a warning and
  reject WASM registrations at the RPC boundary — the non-WASM path still
  works.

## 6 · CBOR ↔ Eigon-JSON boundary

The WASM side speaks CBOR (matches the SDK). Existing TS component handlers
speak Eigon-JSON. The boundary sits at **dispatchBridge** when a WASM guest
calls into a non-WASM handler.

**Rules:**
- WASM → WASM: CBOR in, CBOR out. No conversion.
- WASM → TS native: CBOR decoded to JS object → Eigon-JSON string → `JSON.parse`
  → handler → output object → `JSON.stringify` → CBOR-encode.
- External caller → WASM (kernel gRPC path): kernel-sent payload may already
  be CBOR. If `content_type == "application/cbor"`, pass through; if
  `"application/eigon+json"`, transcode before calling `executeComponent`.

This mirrors the duality that already exists in the kernel: the WASM SDK is
CBOR-only, the rest of the system is Eigon-JSON, and the transcoder lives at
the boundary.

## 7 · Component cache design

**Location:** `~/.cache/eigenius/wasm/`. Override via env var
`EIGENIUS_WASM_CACHE` for tests and CI.

**Layout:**
```
~/.cache/eigenius/wasm/
  <sha256_hex>.cwasm    ← serialised Component
  <sha256_hex>.meta     ← JSON { componentIri, wasmtimeVersion, createdAt }
```

**Algorithm (on `loadComponent`):**
1. Compute `h = sha256(binary)`.
2. If `<h>.cwasm` exists and `<h>.meta.wasmtimeVersion` matches current
   wasmtime version: `Component::deserialize_file`. Else fall through.
3. Compile fresh: `Component::from_binary`. Serialize to `<h>.cwasm.tmp` then
   atomic rename (handles concurrent loads). Write `<h>.meta`.
4. Memoise in process: `DashMap<sha256, Arc<Component>>` so repeat loads in the
   same process don't touch disk.

**Wasmtime version check:** on mismatch, recompile. Never trust a cwasm from a
different wasmtime version — deserialisation is not guaranteed compatible.

**No eviction policy in v1.** Realistic cache size stays small; deferred until
we have numbers.

## 8 · Error semantics

| Source | Surface |
|---|---|
| Binary fails to compile (on register) | `RegisterWasmComponentResponse { success: false, error }` → kernel returns error |
| Component load succeeds, IRI mismatch with kernel's expectation | Reject registration with explicit IRI-mismatch error |
| Execute: guest trap (fuel / memory / panic) | `ComponentResponse { success: false, error: "trap: …" }` |
| Execute: host dispatch callback rejects | Guest observes `err(string)` branch; WASM code decides how to surface |
| Execute: native addon crash | TS catches, logs, returns failure; does not take down the orchestrator process |

**Fuel accounting:** record `fuel_consumed = before - after` per call. Return
through `ComponentMetrics` when it's plumbed (later milestone).

## 9 · Observability

Per WASM invocation, log at debug level:
- Component IRI, input byte size, argument byte size
- Number of dispatch callbacks made
- Total wall-clock elapsed
- Fuel consumed

Surface the same info in a `tracing`-style span on the Rust side once we hook
up structured tracing; for v1, `console.debug` in TS is enough.

## 10 · Milestones

Each milestone ends with a working, tested increment.

### M1 — Shared crate extraction ✓

- [x] Create `crates/wasm-runtime/` with the surface from §3.
- [x] Migrate `kernel/src/capability/wasm_component.rs` to use it.
- [x] Verify `cargo test --workspace` passes, kernel WASM tests still green.

### M2 — Native addon skeleton ✓

- [x] Create `orchestration/native/` crate (napi-rs + wasmtime + shared crate).
- [x] Implement `loadComponent`, `executeComponent`, `unloadComponent`.
- [x] Implement Component disk cache (§7).
- [x] Unit-test from Rust side using the existing `wasm-http-shout` fixture.

### M3 — TS integration (io-access path) ✓

- [x] `orchestration/src/wasm/` with `loadAddon`, `registry`, `hostBridge`,
  `cbor`.
- [x] Replace `registerWasmComponent` stub.
- [x] Wire `ComponentRegistry` so the dispatch bridge can route to TS handlers.
- [x] Smoke test: register `wasm-http-shout` via TS, invoke `execute`, verify
  dispatch to the mock `CompleteText`.
  ([orchestration/tests/wasm_shout_test.ts](../../orchestration/tests/wasm_shout_test.ts))

### M3b — read-access / query-access bridge ✓

- [x] Reused existing `KernelClient` (already had `inspect` + `query`).
- [x] `resolve` callback wired to the kernel's `Inspect` RPC.
- [x] `query` callback wired to the kernel's streaming `Query` RPC.
- [x] Addon's wasmtime linker now wires all three interfaces
  ([orchestration/native/src/linker.rs](../../orchestration/native/src/linker.rs)).
- [x] WASM test fixture
  ([examples/wasm-read-query-probe/](../../examples/wasm-read-query-probe/))
  exercises both imports end-to-end.

### M4 — End-to-end kernel integration ✓

- [x] Deno tasks carry the required runtime flags; `deno task build:addon`
  covers the build step.
- [x] Runbook documented in
  [examples/wasm-http-shout/README.md](../../examples/wasm-http-shout/README.md):
  kernel serve → orchestrator start → `eigenius capability install` →
  `capability test`.
- [x] CLI quick-mode `--capability io` path verified.
- [x] Programmatic spawn-both-processes E2E test:
  [orchestration/tests/wasm_e2e_test.ts](../../orchestration/tests/wasm_e2e_test.ts)
  spawns the kernel binary and a Deno orchestrator, installs
  `wasm-http-shout` via the kernel's `Load` RPC, invokes it via
  `RunProgram`, and verifies the response flows all the way back through
  `RemoteComponent` → orchestrator → WASM → mock `CompleteText` →
  ShoutedText. Stable across 4 consecutive runs at ~750ms each.

### M5 — Hardening ✓

- [x] Error paths covered by tests in
  [orchestration/native/src/tests.rs](../../orchestration/native/src/tests.rs):
  - `guest_trap_on_fuel_exhaustion_surfaces_error`
  - `dispatch_rejection_surfaces_to_guest`
  - `corrupt_cache_entry_triggers_recompile` (also hardened `execute::load`
    to self-heal a bad cache entry by recompiling AND re-caching)
- [x] `with_isolated_cache` test helper serialises env-var mutation with a
  module mutex.
- [x] D12 + this plan updated to reflect shipped state.
- [x] File GitHub issue for automatic re-registration on orchestrator
  restart (see §12 — deferred for v1). → [#11](https://github.com/eigenius/eigenius/issues/11)

## 11 · Testing strategy (as shipped)

- **Rust unit tests** — 3 in
  [crates/wasm-runtime/src/lib.rs](../../crates/wasm-runtime/src/lib.rs)
  and 10 in
  [orchestration/native/src/tests.rs](../../orchestration/native/src/tests.rs)
  covering: load, execute, unload, unknown-handle, cache round-trip,
  resolve/query callback wiring (with canned + not-found branches),
  guest trap, dispatch rejection, corrupt cache recovery.
- **TS integration test** —
  [orchestration/tests/wasm_shout_test.ts](../../orchestration/tests/wasm_shout_test.ts)
  registers `wasm-http-shout` and exercises the dispatch-to-TS-handler path.
- **Concurrency coverage** — the spike already proved 50 concurrent calls
  via `echoWithCallback` and 100 concurrent `executeComponent` invocations
  (see spike REPORT §Checkpoint 3). We chose not to duplicate at the
  orchestrator unit-test level since the concurrency primitive is unchanged
  from the spike.
- **Full spawn-both-processes E2E** —
  [orchestration/tests/wasm_e2e_test.ts](../../orchestration/tests/wasm_e2e_test.ts)
  brings up kernel + orchestrator as subprocesses, installs
  `wasm-http-shout`, invokes it, and asserts the mock `CompleteText`
  response round-trips. Prerequisites: `cargo build` + `deno task
  build:addon`; the test skips cleanly if either is missing.

## 12 · Open questions

### Resolved 2026-04-19

1. **Orchestrator restart re-registration.** Deferred — v1 requires
   restarting both kernel and orchestrator together. Delta tracked as
   [issue #11](https://github.com/eigenius/eigenius/issues/11).
2. **Scope of `read-access` / `query-access` for IO components.** Included
   in v1. Kernel-callback path via existing `Inspect` and `Query` RPCs
   (milestone M3b).
3. **Cross-compilation for distribution.** Linux x86_64 only for v1. Darwin
   requires osxcross or a mac runner — deferred to a post-release task.

### Resolved 2026-04-20

4. **Vendored vs rebuild-on-install.** Resolved: pre-built per-platform
   `.node` files, produced by a CI matrix that runs `napi build --platform
   --release` on each target's native runner. Same pattern as any napi-rs
   package. `package.json` declares the five supported triples
   (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, both Darwin
   variants, `x86_64-pc-windows-msvc`). Only the Linux x86_64 target is
   currently exercised; others are declared but unverified until a
   maintainer on the target platform runs `deno task build:addon`. See
   [orchestration/native/README.md §Distribution](../../orchestration/native/README.md#distribution).

5. **Eigon-JSON / CBOR alignment.** Resolved by a round-trip test that
   pushes twenty value variants (floats, booleans, large integers, unicode,
   empty collections, nested resources) through
   [wasm-cbor-echo](../../examples/wasm-cbor-echo/) — cbor-x encodes,
   ciborium in the guest decodes and re-encodes, cbor-x decodes. All pass.
   Incidental finding: the Eigon model rejects null-valued properties
   outright (guest returns `null values not allowed`), so those are
   filtered at the handler boundary rather than silently dropped. See
   [orchestration/tests/cbor_roundtrip_test.ts](../../orchestration/tests/cbor_roundtrip_test.ts).

## 13 · Incorporated learnings from the spike

Summarised here so nobody has to re-derive them:

1. **`FnArgs<T>` is mandatory** for multi-arg JS callbacks via
   `ThreadsafeFunction` — spread via `FnArgs::from(tuple)` on the Rust side.
2. **Deno needs `--unstable-node-globals --unstable-detect-cjs`** plus
   `--allow-ffi --allow-env --allow-sys` to load the napi-rs addon.
3. **Component compilation dominates latency** (~226ms per 4.7MB component).
   Disk + in-process cache is not optional; it's what makes this architecture
   viable.
4. **Wasmtime error conversion:** host-import closures return `wasmtime::Result`,
   not `anyhow::Result`. Use `wasmtime::Error::msg(format!(…))`.
5. **Memory growth is bounded** (+79MB over 1000 sequential calls, wasmtime
   internal caching) — no leak signature. Concurrent execution scales
   (15.4s for 100 calls, ~6.5× amortisation).
