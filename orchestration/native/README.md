# eigenius-orchestrator-wasm

napi-rs native addon that hosts IO WASM components for the Eigenius
orchestrator. Compiles Component Model binaries via wasmtime, caches them
to disk, and bridges the guest's host-import surface (dispatch, resolve,
query) to the orchestrator's TypeScript layer.

Companion design: [docs/design/d12b-orchestrator-wasm-plan.md](../../docs/design/d12b-orchestrator-wasm-plan.md).

## Build

From this directory:

```bash
npm install                                # first time only
./node_modules/.bin/napi build --platform  # debug build
./node_modules/.bin/napi build --platform --release   # release build
```

Or from `../` (Deno task):

```bash
deno task build:addon
```

This produces three artefacts next to `Cargo.toml`:

- `eigenius-orchestrator-wasm.linux-x64-gnu.node` — the compiled addon
- `index.js` — napi-rs stub that loads the `.node` file
- `index.d.ts` — TypeScript type definitions

All three are git-ignored.

## Test

Rust-level tests cover the full load + execute flow against fixtures in
`../../kernel/tests/fixtures/`:

```bash
cargo test
```

This exercises:

- `wasm-http-shout` → CBOR round-trip through a synthetic `HostBridge`
- `wasm-read-query-probe` → resolve/query callbacks and the not-found path
- Cache load/evict, handle lifecycle, unknown-handle error

## Runtime flags (Deno)

The addon is loaded through Node's CJS shim (`createRequire`). Deno needs
these flags to resolve it:

```
--allow-ffi --allow-env --allow-sys \
--unstable-node-globals --unstable-detect-cjs
```

The orchestration `deno task dev` / `deno task start` tasks already include
all of them.

## Architecture summary

```
  TS: registerComponentExecutor  ──────┐
                                       │  registerWasmComponent RPC
                                       ▼
  TS: WasmComponentRegistry  ── addon.loadComponent ──┐
                                                      │
  TS: createWasmComponentHandler  ◄─── ComponentRegistry.register
                │  (invoked on Execute RPC)
                ▼
  TS: CBOR encode input/argument
                │
                ▼
  addon.executeComponent(handle, input, argument, dispatch, resolve, query)
                │
                ▼
  Rust: execute::execute → wasmtime instantiate_async → guest `execute`
                │
                ▼  (guest calls host imports)
  Rust: linker::build_io_linker → HostBridge trait
                │
                ▼
  NapiBridge: wraps each callback in ThreadsafeFunction::call_async
                │
                ▼
  TS: hostBridge.dispatch / resolve / query
                │
                ├─ dispatch: ComponentRegistry.execute (other TS or WASM handler)
                ├─ resolve:  KernelClient.inspect
                └─ query:    KernelClient.query
```

## Disk cache

Compiled components are serialised to
`$EIGENIUS_WASM_CACHE` (default `~/.cache/eigenius/wasm/`):

```
<sha256_hex>.cwasm   ← wasmtime-serialised Component
<sha256_hex>.meta    ← "wasmtime-43" version tag
```

On re-load the `.cwasm` is deserialised directly (sub-millisecond) instead
of re-compiling (~225ms for a 4.7MB component). The tag check protects
against loading a cwasm built by a different wasmtime version.

The cache directory is `dirs::cache_dir()` with `/eigenius/wasm` appended,
so it's platform-appropriate out of the box:

| Platform | Default location |
|---|---|
| Linux | `~/.cache/eigenius/wasm/` |
| macOS | `~/Library/Caches/eigenius/wasm/` |
| Windows | `%LOCALAPPDATA%\eigenius\wasm\` |

Override via the `EIGENIUS_WASM_CACHE` env var.

## Distribution

The addon is a platform-specific compiled artefact. `package.json` declares
five target triples that napi-rs knows how to build:

| Triple | OS / arch |
|---|---|
| `x86_64-unknown-linux-gnu` | Linux x86_64 (reference build) |
| `aarch64-unknown-linux-gnu` | Linux ARM64 |
| `x86_64-apple-darwin` | macOS Intel |
| `aarch64-apple-darwin` | macOS Apple Silicon |
| `x86_64-pc-windows-msvc` | Windows x86_64 |

### Local development

`napi build --platform` (what `deno task build:addon` runs) builds for the
**host** triple only. Cross-compilation from Linux to macOS/Windows is not
part of the local workflow — it requires either [osxcross](https://github.com/tpoechtrager/osxcross)
for macOS or a native runner for each target, neither of which we ship.

If you're on a different host, `deno task build:addon` should Just Work
provided you have the Rust toolchain + `cargo-component` installed.

### Release builds

For a release, each target must be built on (or cross-compiled to) its own
runner. A typical CI matrix:

- `ubuntu-latest` → `x86_64-unknown-linux-gnu` + `aarch64-unknown-linux-gnu` (via `cross` or aarch64 runner)
- `macos-latest` → `x86_64-apple-darwin` + `aarch64-apple-darwin`
- `windows-latest` → `x86_64-pc-windows-msvc`

napi-rs produces one `.node` per triple (e.g., `eigenius-orchestrator-wasm.darwin-arm64.node`).
The generated `index.js` shim picks the right one at load time based on
`process.platform` + `process.arch`. Publishing is out of scope for this
document — revisit when the orchestrator ships as a distributable package.

### What's known to work today

Only `x86_64-unknown-linux-gnu`. The Rust code has no platform-specific
cfg gates or paths; the other targets are declared but unverified. When a
maintainer on another OS exercises `deno task build:addon` and reports
back, we can remove the "unverified" caveat.
