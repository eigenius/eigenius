# EigeniusRuntimeWorker

Julia worker speaking the Eigenius substrate's CBOR RPC over a Unix
domain socket. Phase 18d capstone; Phase 19a inherits this as the seed
of `eigenius-julia`'s production worker.

## What it does

- On startup: cross-checks `EIGENIUS_RUNTIME_ENV_DIGEST` /
  `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH` env vars against
  `/etc/eigenius-runtime-env/manifest-hash` (D26 §9.3). Exit 78 on
  mismatch.
- Binds the UDS at `EIGENIUS_TEST_WORKER_UDS`, chmod 0o666 so any host
  UID can connect.
- Speaks the substrate's 5-verb CBOR RPC with length-prefixed framing
  (4-byte big-endian length + CBOR body):
  - `Health` — reports baked-in digest + `numerical_metadata`
  - `Instantiate` — no-op (returns `ready: true`); 19a fills this in
    with real Julia env warm-up
  - `RegisterMirror` — no-op echo; 19b uses this for mirror payload
  - `DispatchMethod` — `eval`s the supplied Julia source, returns
    captured stdout (or the expression's value) as a CBOR-encoded
    `String`
  - `Evict` — clean exit
- Multi-connection accept loop: substrate may open separate
  connections for `Health` and `DispatchMethod` (Phase 18c.5); worker
  exits only on explicit `Evict`.

## Build and run

For local development outside a container:

```bash
julia --project=. -e 'using Pkg; Pkg.instantiate()'

# Worker won't start without the cross-check env vars set.
mkdir -p /tmp/wj-prov
echo "test-manifest" > /tmp/wj-prov/manifest-hash
EIGENIUS_TEST_WORKER_UDS=/tmp/wj-uds \
EIGENIUS_RUNTIME_ENV_DIGEST=sha256:0000000000000000000000000000000000000000000000000000000000000000 \
EIGENIUS_RUNTIME_ENV_MANIFEST_HASH=test-manifest \
EIGENIUS_RUNTIME_ENV_DIR=/tmp/wj-prov \
julia --project=. src/JuliaWorker.jl
```

Production (Phase 18d's `TestLanguageRuntimeJulia` and Phase 19a's
`eigenius-julia`) runs the worker inside a container built from
`julia:1.12-bookworm`, where the manifest-hash file lives at the
spec'd `/etc/eigenius-runtime-env/manifest-hash` — no
`EIGENIUS_RUNTIME_ENV_DIR` override needed.

## Pinning discipline

`Manifest.toml` is committed. Regenerate with:

```bash
julia --project=. -e 'using Pkg; Pkg.update(); Pkg.precompile()'
```

…then commit the updated `Manifest.toml`. Substrate-managed image
builds use `Pkg.instantiate` against the committed manifest so two
builds with the same source tree produce the same dependency closure.

## Protocol shape

Wire format mirrors the Rust enums in
[`crates/runtime-substrate/src/rpc/protocol.rs`](../../crates/runtime-substrate/src/rpc/protocol.rs).
Externally-tagged CBOR encoding (serde default for Rust enums):

- Unit variants (`Health`, `Evict`, `Evicted`) → bare CBOR text string
- Struct variants (`DispatchMethod{...}`, etc.) → `{"<VariantName>": {<fields>}}` map

`output` and `target` are typed as Rust `ByteBuf` (CBOR major type 2 —
byte string). The bytes' interpretation is up to the language runtime;
the test worker uses CBOR-encoded `String` for both directions.
