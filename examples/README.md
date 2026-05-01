# Eigenius Examples

This directory holds worked examples that demonstrate specific Eigenius
features end-to-end. Each example is a standalone crate excluded from the
top-level Cargo workspace (they compile to `wasm32-unknown-unknown` with
their own build tooling) and has its own README walking through the code.

## WASM Extensibility (Phase 8)

Eigenius supports extending the kernel with WASM components and
institutions built against the WIT interfaces in [`wit/`](../wit/). These
examples show how to write such extensions using the
[`eigenius-wasm-sdk`](../sdk/wasm-sdk/) crate and how to install them
through the CLI.

### [wasm-doc-validator](wasm-doc-validator/)

A **pure component** that validates document structure. Takes a
`Document` resource (title, body, section_count), checks each field
against simple rules, and returns a `ValidationResult` with a boolean
`valid` flag and an optional array of error messages.

Demonstrates:

- Implementing the `eigenius-component` WIT world
- Reading typed properties from a CBOR-encoded input resource via the SDK
- Constructing an output resource with array-valued properties
- Installing into a running kernel via `eigenius capability install`

### [wasm-d14-echo](wasm-d14-echo/)

A **D14 institution smoke test** targeting the
`eigenius-institution-d14` WIT world. Echoes its inputs back with
provenance and stage markers so kernel host-bridge tests can verify
that `extract-typed`, `reify`, and `query` round-trip correctly.

Demonstrates:

- Implementing the triadic comorphism contract from
  [D14](../docs/design/d14-institution-realisation.md)
- Echoing CBOR resources without dropping properties
- Tagging the result with which export stage produced it

### [wasm-d14-dock](wasm-d14-dock/), [wasm-d14-assay](wasm-d14-assay/), [wasm-d14-arrhenius](wasm-d14-arrhenius/)

The **M8 dock-assay worked example** (D14 §5.1, §13.4 M8). Three crates
that together exercise the full D14 institution surface end-to-end:

- `wasm-d14-dock` — a source-side institution implementing
  `extract_typed` for an `ef_dock_to_dg` ExportFormat (reads `delta_g`
  off a `DockingResult`, returns it as a Float-typed payload).
- `wasm-d14-assay` — a target-side institution implementing `reify`
  (constructs an `AssayPrediction` with a Float `ic50`) plus `query`
  for three QueryClasses: `within_tolerance` (Decidable),
  `assay_prediction_validity` (AutoOnLoad), `validate_prediction`
  (OnDemand).
- `wasm-d14-arrhenius` — a Pure Component implementing the middle of
  the `dock_to_assay` Comorphism: Float → Float via
  `IC₅₀ ≈ exp(-ΔG / R·T) · 10⁹`.

The supporting ontology lives in
[`ontologies/examples/d14-dock-assay/dock-assay.json`](../ontologies/examples/d14-dock-assay/dock-assay.json).
Both an in-process variant
([`kernel/tests/d14_dock_assay_demo.rs`](../kernel/tests/d14_dock_assay_demo.rs))
and a WASM-hosted variant
([`kernel/tests/d14_dock_assay_demo_wasm.rs`](../kernel/tests/d14_dock_assay_demo_wasm.rs))
exercise the surface; the WASM test demonstrates auto-registration
from a child layer carrying `runtime: wasm` + inline `wasm_binary`.

### [wasm-http-shout](wasm-http-shout/)

An **IO component** that calls an LLM by dispatching to the native
`CompleteText` handler. Takes a `TextInput`, wraps the text in a prompt
asking for ALL CAPS, dispatches via `io-access.dispatch-component`, and
wraps the LLM response in a `ShoutedText` output.

Demonstrates:

- Implementing the `eigenius-component-io` WIT world (IO capability level)
- Calling the host's `io-access.dispatch-component` to reach another
  component (including native TS handlers) from inside WASM
- The full orchestrator-hosted flow: kernel install →
  `RegisterWasmComponent` RPC → orchestrator compile via napi-rs addon →
  kernel dispatch → guest runs → host callback → response

### [wasm-read-query-probe](wasm-read-query-probe/)

A **minimal IO component** used as a test fixture for the orchestrator's
`read-access.resolve` and `query-access.query` host imports. Calls both,
returns bytes-received / rows-received counts. Not intended for end users;
see [`orchestration/native/src/tests.rs`](../orchestration/native/src/tests.rs)
for its usage.

### [wasm-cbor-echo](wasm-cbor-echo/)

A **minimal echo component** used to verify CBOR interop between the
orchestrator's cbor-x codec and the SDK's ciborium codec. Decodes its
input resource, re-encodes unchanged, returns it. The round-trip test
at [`orchestration/tests/cbor_roundtrip_test.ts`](../orchestration/tests/cbor_roundtrip_test.ts)
passes twenty value variants through it (floats, booleans, large
integers, unicode strings, nested resources, …).

## Building an example

Each example requires:

- [`cargo-component`](https://github.com/bytecodealliance/cargo-component)
  (`cargo install cargo-component`)
- The `wasm32-unknown-unknown` Rust target
  (`rustup target add wasm32-unknown-unknown`)

Then from the example directory:

```bash
cargo component build
```

The resulting `.wasm` file is a [WebAssembly Component Model] binary that
conforms to one of the worlds defined in [`wit/eigenius-component.wit`](../wit/eigenius-component.wit).

[WebAssembly Component Model]: https://component-model.bytecodealliance.org/

## Installing into a running kernel

Start the kernel:

```bash
cargo run -p eigenius-cli -- serve
```

Install the built `.wasm` file in a separate terminal:

```bash
# Quick mode — good for trying things out
eigenius --endpoint http://localhost:50051 capability install \
    examples/wasm-doc-validator/target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm \
    --as-iri urn:example:components:DocValidator \
    --kind component \
    --input-type urn:example:doc:Document \
    --output-type urn:example:doc:ValidationResult

# Full mode — merges a user-supplied definition file with the binary
eigenius --endpoint http://localhost:50051 capability install \
    examples/wasm-doc-validator/target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm \
    --definition examples/wasm-doc-validator/component.esl
```

Verify the install:

```bash
eigenius --endpoint http://localhost:50051 capability list
eigenius --endpoint http://localhost:50051 capability inspect urn:example:components:DocValidator
```

Run a test input against it:

```bash
eigenius --endpoint http://localhost:50051 capability test \
    urn:example:components:DocValidator \
    --input some-document.json
```

## Kernel fixtures

Pre-built binaries of these examples live in
[`kernel/tests/fixtures/`](../kernel/tests/fixtures/) for use in kernel
integration tests. If you modify an example, rebuild and copy the binary
to the fixtures directory so the tests pick up the change:

```bash
cd examples/wasm-doc-validator
cargo component build
cp target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm \
   ../../kernel/tests/fixtures/
```

## Further reading

- **[D12: WASM Extensibility](../docs/design/d12-wasm-extensibility.md)** —
  design specification for the WASM hosting architecture, capability levels,
  and WIT interface contracts
- **[D14: Institution Realisation](../docs/design/d14-institution-realisation.md)** —
  the canonical institution model that the D14-flavoured examples target
  (Institution trait, declaration vocabulary, dispatch model). Supersedes D10.
- **[SDK reference](../sdk/wasm-sdk/src/lib.rs)** — `Resource` and `Value`
  APIs, CBOR helpers, and the `institution` submodule with builders for the
  D14 declaration shapes (`InstitutionDecl`, `ExportFormatDecl`,
  `ImportFormatDecl`, `QueryClassDecl`, `ComorphismDecl`)
