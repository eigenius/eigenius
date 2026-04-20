# wasm-http-shout

An **IO WASM component** that calls an LLM by dispatching to the native
`CompleteText` handler. Implements the
[`eigenius-component-io`](../../wit/eigenius-component.wit) WIT world and
runs inside the **orchestrator** (not the kernel), via the
`eigenius-orchestrator-wasm` napi-rs addon.

Companion design: [D12](../../docs/design/d12-wasm-extensibility.md) and
[D12b implementation plan](../../docs/design/d12b-orchestrator-wasm-plan.md).

## What it does

Takes a `TextInput` resource with a `text` property, wraps the text in a
prompt asking for ALL CAPS, dispatches to `CompleteText`, and wraps the
LLM response in a `ShoutedText` output resource.

| Property | Type | Role |
|---|---|---|
| `urn:example:shout:text` | string | input text to transform |
| → `urn:example:shout:shouted` | string | uppercased output |

## Key SDK patterns shown

- Implementing the `eigenius-component-io` WIT world (IO capability level)
- Calling the host's `io-access.dispatch-component` to invoke another
  component by IRI from inside WASM
- Building a nested argument resource (`CompleteText` takes an embedded
  `request_parameters` sub-resource)
- Installing into a running orchestrator via `eigenius capability install`

## Building

```bash
cd examples/wasm-http-shout
cargo component build --release
```

Output: `target/wasm32-unknown-unknown/release/eigenius_wasm_http_shout.wasm`

## Prerequisites for running

Both the kernel and orchestrator must be up, and the orchestrator must
have the napi-rs addon built.

```bash
# 1. Build the native addon (first time only)
(cd orchestration && deno task build:addon)

# 2. Start the orchestrator (mock-LLM mode — no API key needed)
(cd orchestration && EIGENIUS_MOCK_LLM=true deno task start)

# 3. Start the kernel in a second terminal, pointing at the orchestrator
cargo run -p eigenius-cli -- serve \
    --port 50051 \
    --orchestrator http://localhost:8080
```

You should see the orchestrator log:

```
WASM IO components: enabled (native addon loaded)
```

If you see `disabled (addon not loaded …)` instead, step 1 didn't run —
go build the addon.

## Installing

```bash
cargo run -p eigenius-cli -- \
    --endpoint http://localhost:50051 \
    capability install \
    examples/wasm-http-shout/target/wasm32-unknown-unknown/release/eigenius_wasm_http_shout.wasm \
    --as-iri urn:example:components:HttpShout \
    --kind component \
    --capability io
```

The kernel scans the uploaded layer, sees `capability_level=io`, and
forwards the binary to the orchestrator via `RegisterWasmComponent`.
The orchestrator:

1. Hashes the binary and checks the disk cache at `~/.cache/eigenius/wasm/`
2. Either deserialises a cached `.cwasm` or compiles fresh from bytes
3. Inserts the handle into `WasmComponentRegistry`
4. Registers a `ComponentRegistry` handler that wraps the WASM

On success you should see `[wasm] registered urn:example:components:HttpShout`
in the orchestrator log.

## Testing

```bash
cat > /tmp/shout-input.json <<'EOF'
{
  "@id": "urn:example:shout:demo",
  "urn:example:shout:text": "hello from wasm"
}
EOF

cargo run -p eigenius-cli -- \
    --endpoint http://localhost:50051 \
    capability test \
    urn:example:components:HttpShout \
    --input /tmp/shout-input.json
```

Expected output (in mock mode):

```json
{
  "urn:eigenius:core:is_a": ["urn:example:shout:ShoutedText"],
  "urn:example:shout:shouted": "HELLO FROM WASM"
}
```

The call path this exercised:

```
cli capability test
    │  gRPC ComponentRequest (Eigon-JSON)
    ▼
kernel → orchestrator (RemoteComponent)
    │  gRPC ComponentExecutor.Execute
    ▼
orchestrator ComponentRegistry handler (WASM-backed)
    │  CBOR encode
    ▼
addon.executeComponent → wasmtime → guest.execute
    │  host import: dispatch-component(CompleteText, …)
    ▼
HostBridge.dispatch → ComponentRegistry.execute("CompleteText")
    │  mock or real LLM call
    ▼
CompleteText handler returns LLM output
    │  CBOR encode → back through the stack
    ▼
guest wraps output as ShoutedText → response to CLI
```

## Source walkthrough

See [`src/lib.rs`](src/lib.rs):

- `wit_bindgen::generate!` brings in the full `eigenius-component-io`
  world. This pulls in `io_access::dispatch_component` which is the host
  import the guest uses to reach `CompleteText`.
- `impl Guest for HttpShout` provides the two exports.
- `dispatch_component(iri, input, argument)` wraps the generated binding
  with a simpler signature; the host (orchestrator) bridges it to a TS
  async callback via napi-rs `ThreadsafeFunction`.

## Troubleshooting

**Orchestrator log says `WASM IO components: disabled`**
→ The addon isn't built. Run `deno task build:addon` from `orchestration/`.

**CLI says `orchestrator WASM support is disabled (native addon not loaded …)`**
→ Same cause as above.

**`cargo-component` not found**
→ `cargo install cargo-component`.

**Component compiles but instantiation fails with `wasi:cli/environment` missing**
→ The build target drifted to `wasm32-wasip1`. Make sure
`examples/wasm-http-shout/.cargo/config.toml` sets
`target = "wasm32-unknown-unknown"`.

## Related

- [examples/README.md](../README.md) — top-level overview
- [orchestration/native/README.md](../../orchestration/native/README.md) — addon build + runtime
- [docs/design/d12b-orchestrator-wasm-plan.md](../../docs/design/d12b-orchestrator-wasm-plan.md) — implementation plan
- Rust-level tests: [orchestration/native/src/tests.rs](../../orchestration/native/src/tests.rs)
- TS smoke test: [orchestration/tests/wasm_shout_test.ts](../../orchestration/tests/wasm_shout_test.ts)
