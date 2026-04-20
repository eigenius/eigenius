# wasm-doc-validator

A pure WASM component that checks document structure. Implements the
[`eigenius-component`](../../wit/eigenius-component.wit) WIT world and
runs inside the Eigenius kernel via `WasmComponent`.

## What it does

Takes a `Document` resource with three required properties:

| Property                                  | Type      | Rule                                  |
|-------------------------------------------|-----------|---------------------------------------|
| `urn:example:doc:title`                   | string    | must not be empty                     |
| `urn:example:doc:body`                    | string    | must be ≥ 100 characters              |
| `urn:example:doc:section_count`           | integer   | must be ≥ 1                           |

Returns a `ValidationResult` resource:

| Property                                  | Type            | Meaning                                |
|-------------------------------------------|-----------------|----------------------------------------|
| `urn:eigenius:core:is_a`                  | [IRI]           | always `[urn:example:doc:ValidationResult]` |
| `urn:example:doc:valid`                   | boolean         | `true` iff no errors                   |
| `urn:example:doc:errors`                  | array of string | absent when valid; list of failed rules |

All three rules are checked on every invocation, so the `errors` array
can contain up to three entries when multiple rules fail.

## Key SDK patterns shown

- **Typed property access** on the input resource via
  [`Resource::get_string`](../../sdk/wasm-sdk/src/lib.rs) and
  `Resource::get_integer`, with graceful handling of missing properties
- **Constructing output** with `Resource::set`, including array-valued
  properties via `Value::Array`
- **Class tagging** — setting `urn:eigenius:core:is_a` so the output is a
  proper typed Eigon resource and not just a blob
- **CBOR round-trip** at the WASM boundary via `Resource::from_cbor` /
  `Resource::to_cbor` — the SDK handles all the byte-level detail

## Building

```bash
cd examples/wasm-doc-validator
cargo component build
```

Output: `target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm`

## Installing

From a running Eigenius kernel (`cargo run -p eigenius-cli -- serve`):

```bash
eigenius --endpoint http://localhost:50051 capability install \
    examples/wasm-doc-validator/target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm \
    --as-iri urn:example:components:DocValidator \
    --kind component \
    --input-type urn:example:doc:Document \
    --output-type urn:example:doc:ValidationResult
```

## Testing

Save a valid document:

```bash
cat > /tmp/doc-valid.json <<'EOF'
{
  "@id": "urn:example:doc:demo",
  "urn:example:doc:title": "Hello World",
  "urn:example:doc:body": "This document has enough body text to pass the minimum 100-character requirement that the validator enforces. Padding to reach the threshold.",
  "urn:example:doc:section_count": 3
}
EOF
```

Run it through the component:

```bash
eigenius --endpoint http://localhost:50051 capability test \
    urn:example:components:DocValidator \
    --input /tmp/doc-valid.json
```

Expected output:

```json
{
  "urn:eigenius:core:is_a": ["urn:example:doc:ValidationResult"],
  "urn:example:doc:valid": true
}
```

Try an invalid document with multiple failures:

```bash
cat > /tmp/doc-invalid.json <<'EOF'
{
  "@id": "urn:example:doc:bad",
  "urn:example:doc:title": "",
  "urn:example:doc:body": "short",
  "urn:example:doc:section_count": 0
}
EOF

eigenius --endpoint http://localhost:50051 capability test \
    urn:example:components:DocValidator \
    --input /tmp/doc-invalid.json
```

Expected output:

```json
{
  "urn:eigenius:core:is_a": ["urn:example:doc:ValidationResult"],
  "urn:example:doc:valid": false,
  "urn:example:doc:errors": [
    "title must not be empty",
    "body must be at least 100 characters",
    "must have at least one section"
  ]
}
```

## Source walkthrough

See [`src/lib.rs`](src/lib.rs). The key pieces:

- `wit_bindgen::generate!` generates the `Guest` trait from
  `wit/eigenius-component.wit`. The generated `ComponentResult` type maps
  to the WIT record `{ output: list<u8> }`.
- `impl Guest for DocValidator` provides the two exported functions:
  `execute` and `component_iri`.
- `Resource::from_cbor` decodes the input bytes the host passes in;
  `output.to_cbor()` encodes the result bytes the host reads back.
- `export!(DocValidator)` registers the implementation with
  `cargo-component`'s runtime — this is what turns the crate into a
  Component Model component.

## Related

- [examples/README.md](../README.md) — top-level examples overview
- [docs/design/d12-wasm-extensibility.md](../../docs/design/d12-wasm-extensibility.md) — the architectural design
- [sdk/wasm-sdk/src/lib.rs](../../sdk/wasm-sdk/src/lib.rs) — the SDK API
- Kernel integration tests for this component live in
  [kernel/src/capability/tests.rs](../../kernel/src/capability/tests.rs)
  (`doc_validator_*` tests)
