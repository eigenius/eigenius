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

### [wasm-ordering-institution](wasm-ordering-institution/)

A **fiber reasoner** (institution) that provides domain-specific reasoning
over ordering/refinement morphisms. Declares a `Refinement` morphism class
and a `ConvergenceQuery` query class, validates that morphisms have a
positive `delta`, and answers convergence queries.

Demonstrates:

- Implementing the `eigenius-institution` WIT world with all four
  fiber reasoner methods (fiber-declaration, query, validate-morphism,
  discover-morphisms)
- Constructing a `FiberDeclaration` with morphism/query type definitions
- Returning typed validation results via the `validation-result` enum

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
- **[SDK reference](../sdk/wasm-sdk/src/lib.rs)** — `Resource` and `Value`
  APIs, CBOR helpers, and the `institution` submodule for fiber reasoner
  declarations
