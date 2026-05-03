# eigenius-orchestrator-runtime-substrate

napi-rs native addon hosting the
[Eigenius runtime substrate](../../crates/runtime-substrate/) dispatcher inside
the Deno orchestrator. The kernel dispatches `RunRuntimeScript` /
`CallRuntimeMethod` IO components over the `ComponentExecutor` gRPC path; the
orchestrator's TS handlers re-encode the JS-side input/argument as Eigon-CBOR
(matching the kernel ↔ orchestrator codec consolidated in Phase 18e) and cross
into Rust through this addon.

Companion design:
[docs/design/d26-runtime-substrate.md](../../docs/design/d26-runtime-substrate.md),
Phase 18a in
[docs/design/implementation-plan.md](../../docs/design/implementation-plan.md).

## Build

From this directory:

```bash
npm install                                # first time only
./node_modules/.bin/napi build --platform  # debug build
./node_modules/.bin/napi build --platform --release   # release build
```

This produces three artefacts next to `Cargo.toml`:

- `eigenius-orchestrator-runtime-substrate.linux-x64-gnu.node` — the compiled
  addon (platform-suffixed)
- `index.js` — napi-rs stub that loads the `.node` file
- `index.d.ts` — TypeScript declarations for the exports

The Deno orchestrator imports the addon via `napi/loadAddon.ts`-style
fallthrough — if the `.node` file is missing, substrate-routed components fail
to register and the orchestrator logs a one-line warning at startup. Build it
once with the commands above and Deno picks it up automatically.

## Exports

| Function                                        | Purpose                                                                                                                                |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `registerTestLanguageRuntime(workerBinaryPath)` | Register the bash-backed `TestLanguageRuntime` for dev / CI / smoke tests.                                                             |
| `dispatchRunRuntimeScript(input, argument)`     | Dispatch a `RunRuntimeScript` invocation. Eigon-CBOR `Buffer` in / out.                                                                |
| `dispatchCallRuntimeMethod(input, argument)`    | Dispatch a `CallRuntimeMethod` invocation. v1 returns `MethodSignatureMismatch` from `JobSpawner`-backed runtimes; lands fully in 19a. |
| `listRegisteredLanguages()`                     | List the language IDs of currently-registered runtimes.                                                                                |

## Distribution

Same model as `orchestration/native`: per-OS CI runners produce
platform-suffixed `.node` artefacts (`*.linux-x64-gnu.node`,
`*.darwin-arm64.node`, etc.). `index.js` is the napi-rs-generated loader stub
that picks the right `.node` at import time.
