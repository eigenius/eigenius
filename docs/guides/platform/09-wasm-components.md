# 9. Building WASM components

> **REMOVED (2026-07-08).** WASM components were removed from the platform.
> `eigenius capability install`, the `wit/` interfaces, the `examples/wasm-*`
> crates, and in-kernel WASM hosting no longer exist. This chapter is retained
> as historical record only. For live extension mechanisms, use in-process Rust
> institutions or external institutions via the runtime substrate (see
> [11. Runtime substrate](11-runtime-substrate.md)). Background:
> [D12](../../design/d12-wasm-extensibility.md).

WASM components extend the platform with custom dispatch logic. Each component is a sandboxed `.wasm` binary that implements the [`eigenius-component`](../../../wit/eigenius-component.wit) WIT interface, built with `cargo-component`, and installed at runtime via `eigenius capability install`.

This chapter walks through the four worked examples in [`examples/wasm-*`](../../../examples/), in order of increasing capability level. Each example is exercised by an integration test in the kernel suite, so the patterns shown work end-to-end.

The full design rationale (capability levels, host imports, fuel/memory limits) is in [D12 — WASM extensibility](../../design/d12-wasm-extensibility.md).

## 9.1. Capability levels

Three levels, from least to most privileged:

| Level | Where hosted | Imports available |
|---|---|---|
| `pure` | Kernel | None — pure computation only |
| `read` | Kernel | `read-access` (resolve resource by IRI), `query-access` (run EigenQL) |
| `io` | **Orchestrator** | `read-access`, `query-access`, `io-access` (dispatch other components) |

Pick the lowest capability level that does the job. Pure components have the fastest startup (no host calls), are easiest to test, and run in the kernel directly — no orchestrator round-trip per dispatch. IO components must run in the orchestrator because the `io-access` host imports are orchestrator-side (they let WASM code dispatch to other components, including the orchestrator's LLM components).

## 9.2. Project setup

Every WASM component is its own crate. The minimum `Cargo.toml`:

```toml
[package]
name = "my-component"
version = "0.1.0"
edition = "2021"

[dependencies]
eigenius-wasm-sdk = { path = "<repo>/sdk/wasm-sdk" }
wit-bindgen = "0.41"

[lib]
crate-type = ["cdylib"]

[package.metadata.component]
package = "eigenius:component"

[package.metadata.component.target]
world = "eigenius-component"     # or "eigenius-component-io" for IO components
path = "<repo>/wit"
```

The `[package.metadata.component]` entries are read by `cargo component build` to know which WIT world to generate bindings for. For IO components, change `world = "eigenius-component-io"`.

Build:

```bash
cargo component build
```

Output: `target/wasm32-unknown-unknown/debug/<crate_name>.wasm` (replace `debug` with `release` for an optimised build).

## 9.3. The eigenius-wasm-sdk

The [`eigenius-wasm-sdk`](../../../sdk/wasm-sdk/) crate provides typed access to CBOR-encoded resources at the WASM boundary. The two main types:

- **`Resource`** — keyed property bag with IRI-typed property names; `Resource::from_cbor` to parse the input bytes, `Resource::to_cbor` to emit output bytes.
- **`Value`** — the resource's value type: `Value::String`, `Value::Integer`, `Value::Float`, `Value::Boolean`, `Value::Array`, `Value::Embedded(Box<Resource>)`, `Value::ResourceRef(String)`.

Typed accessors avoid manual `Value::*` matching for common cases:

```rust
let title:    Option<String> = doc.get_string("urn:example:doc:title");
let count:    Option<i64>    = doc.get_integer("urn:example:doc:section_count");
let active:   Option<bool>   = doc.get_boolean("urn:example:doc:active");
```

These are a thin wrapper around `Resource::get`; the SDK also exposes `set`, `set_array`, etc., for output construction.

For institutions, the `eigenius_wasm_sdk::institution` module provides the matching types. See [chapter 10](10-wasm-institutions.md).

## 9.4. Pure components

Pure components do nothing but compute. No layer access, no dispatch. The simplest possible WASM extension shape.

### `wasm-cbor-echo` — minimum viable component

Source: [`examples/wasm-cbor-echo/`](../../../examples/wasm-cbor-echo/).

Echoes the input as the output. About 30 lines of code; the smallest crate that successfully implements the WIT contract.

```rust
impl Guest for CborEcho {
    fn execute(input: Vec<u8>, _argument: Vec<u8>) -> Result<ComponentResult, String> {
        Ok(ComponentResult { output: input })
    }
    fn component_iri() -> String { "urn:example:components:CborEcho".into() }
}
```

Use it as a template for new pure components. Replace `execute`'s body with your computation; replace `component_iri` with your chosen IRI.

### `wasm-doc-validator` — typed input/output

Source: [`examples/wasm-doc-validator/`](../../../examples/wasm-doc-validator/) and its [README](../../../examples/wasm-doc-validator/README.md).

Validates a `Document` resource against three rules: non-empty title, body ≥ 100 characters, `section_count` ≥ 1. Returns a `ValidationResult` resource with a boolean `valid` flag and an optional `errors` array.

The full body (~50 lines):

```rust
const TITLE: &str = "urn:example:doc:title";
const BODY: &str = "urn:example:doc:body";
const SECTION_COUNT: &str = "urn:example:doc:section_count";
const VALID: &str = "urn:example:doc:valid";
const ERRORS: &str = "urn:example:doc:errors";
const IS_A: &str = "urn:eigenius:core:is_a";
const VALIDATION_RESULT_CLASS: &str = "urn:example:doc:ValidationResult";

impl Guest for DocValidator {
    fn execute(input: Vec<u8>, _argument: Vec<u8>) -> Result<ComponentResult, String> {
        let doc = Resource::from_cbor(&input).map_err(|e| format!("parse input: {e}"))?;
        let mut errors: Vec<String> = Vec::new();

        match doc.get_string(TITLE) {
            Some(t) if t.is_empty() => errors.push("title must not be empty".into()),
            None => errors.push("title is missing".into()),
            _ => {}
        }
        match doc.get_string(BODY) {
            Some(b) if b.len() < 100 => errors.push("body must be at least 100 characters".into()),
            None => errors.push("body is missing".into()),
            _ => {}
        }
        match doc.get_integer(SECTION_COUNT) {
            Some(n) if n < 1 => errors.push("must have at least one section".into()),
            None => errors.push("section_count is missing".into()),
            _ => {}
        }

        let mut output = Resource::new();
        output.set(IS_A, Value::Array(vec![Value::String(VALIDATION_RESULT_CLASS.into())]));
        output.set(VALID, Value::Boolean(errors.is_empty()));
        if !errors.is_empty() {
            output.set(ERRORS, Value::Array(errors.into_iter().map(Value::String).collect()));
        }
        Ok(ComponentResult { output: output.to_cbor() })
    }

    fn component_iri() -> String { "urn:example:components:DocValidator".into() }
}
```

Patterns demonstrated:

- **CBOR round-trip at the boundary** — `Resource::from_cbor` / `to_cbor` is all the byte-handling needed.
- **Typed property access** — `get_string`, `get_integer` return `Option<T>`; pattern-match for both presence and validation in one go.
- **Output class tagging** — set `urn:eigenius:core:is_a` on the output so it's a proper typed resource, not an opaque blob.
- **Array-valued outputs** — `Value::Array(vec![Value::String(...)])` for collections.

## 9.5. Read-capability components

Read-capability components can resolve resources from the layer chain and run EigenQL queries. Use this when your component needs context beyond its direct input.

### `wasm-read-query-probe`

Source: [`examples/wasm-read-query-probe/`](../../../examples/wasm-read-query-probe/).

A diagnostic component that exercises both `read-access.resolve` and `query-access.query` from inside WASM, returning a result resource containing what was found.

The host-import call shape:

```rust
use crate::eigenius::component::read_access;
use crate::eigenius::component::query_access;

let bytes: Option<Vec<u8>> = read_access::resolve("urn:example:foo");
let resource = bytes.and_then(|b| Resource::from_cbor(&b).ok());

let results: Result<Vec<Vec<u8>>, String> = query_access::query(
    "MATCH ?x { } RETURN [] { x: ?x }".to_string()
);
```

The Cargo.toml enables read-capability by selecting the same `eigenius-component` world but installing with `--capability read`. The WIT world includes both `read-access` and `query-access` import declarations; the bindings expose them as Rust modules.

When installing, use `--capability read`:

```bash
eigenius --endpoint http://localhost:50051 capability install \
    examples/wasm-read-query-probe/target/wasm32-unknown-unknown/debug/eigenius_wasm_read_query_probe.wasm \
    --as-iri urn:example:components:ReadQueryProbe \
    --kind component \
    --capability read \
    --input-type urn:example:Probe \
    --output-type urn:example:ProbeResult
```

## 9.6. IO components

IO components run in the **orchestrator** (not the kernel) because their `io-access` host imports allow them to dispatch to other components — including the orchestrator's built-in `CompleteText` and `CompleteJson` LLM components.

### `wasm-http-shout`

Source: [`examples/wasm-http-shout/`](../../../examples/wasm-http-shout/).

Takes a `TextInput`, wraps it in a prompt asking the LLM to return the text in ALL CAPS, dispatches `CompleteText`, and returns the response wrapped in a `ShoutedText` resource.

The relevant snippet:

```rust
use crate::eigenius::component::io_access;

const COMPLETE_TEXT: &str = "urn:eigenius:program:components:CompleteText";

impl Guest for HttpShout {
    fn execute(input: Vec<u8>, _argument: Vec<u8>) -> Result<ComponentResult, String> {
        let inp = Resource::from_cbor(&input).map_err(|e| format!("parse: {e}"))?;
        let text = inp.get_string("urn:example:text:body")
            .ok_or("missing text.body")?;

        // Build the input + argument resources for CompleteText
        let mut prompt_input = Resource::new();
        prompt_input.set("urn:example:text:body",
            Value::String(format!("Reply with this text in ALL CAPS: {text}")));
        let prompt_arg = Resource::new();

        let response_bytes = io_access::dispatch_component(
            COMPLETE_TEXT.to_string(),
            prompt_input.to_cbor(),
            prompt_arg.to_cbor(),
        ).map_err(|e| format!("dispatch CompleteText: {e}"))?;

        let response = Resource::from_cbor(&response_bytes)
            .map_err(|e| format!("parse response: {e}"))?;

        let mut output = Resource::new();
        output.set("urn:eigenius:core:is_a",
            Value::Array(vec![Value::String("urn:example:text:ShoutedText".into())]));
        // Forward the LLM's output text as-is
        if let Some(t) = response.get_string("urn:example:text:body") {
            output.set("urn:example:text:body", Value::String(t));
        }
        Ok(ComponentResult { output: output.to_cbor() })
    }
    // ...
}
```

The Cargo.toml selects `world = "eigenius-component-io"`. Install with `--capability io`:

```bash
eigenius --endpoint http://localhost:50051 capability install \
    examples/wasm-http-shout/target/wasm32-unknown-unknown/debug/eigenius_wasm_http_shout.wasm \
    --as-iri urn:example:components:HttpShout \
    --kind component \
    --capability io \
    --input-type urn:example:text:TextInput \
    --output-type urn:example:text:ShoutedText
```

The CLI routes IO-capability installs to the orchestrator's WASM addon registry; the kernel is informed of the registration so its dispatcher knows to delegate to the orchestrator when the component IRI is invoked.

## 9.7. Building, installing, testing

The full lifecycle for a new component:

```bash
# 1. Build
cd examples/wasm-doc-validator   # or your own crate
cargo component build

# 2. Install
eigenius --endpoint http://localhost:50051 capability install \
    target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm \
    --as-iri urn:example:components:DocValidator \
    --kind component \
    --capability pure \
    --input-type urn:example:doc:Document \
    --output-type urn:example:doc:ValidationResult

# 3. Verify
eigenius --endpoint http://localhost:50051 capability list
eigenius --endpoint http://localhost:50051 capability inspect \
    urn:example:components:DocValidator

# 4. Test
cat > /tmp/doc.json <<'EOF'
{
  "@id": "urn:example:doc:demo",
  "urn:example:doc:title": "Hello World",
  "urn:example:doc:body": "Long enough body... (100+ chars)",
  "urn:example:doc:section_count": 3
}
EOF
eigenius --endpoint http://localhost:50051 capability test \
    urn:example:components:DocValidator \
    --input /tmp/doc.json
```

If `--db` is enabled, the WASM binary and its capability declaration persist across kernel restarts — see [chapter 6](06-database-management.md) §6.9 (restart re-registration).

## 9.8. Quick mode vs. full mode

The `capability install` flags fall into two modes:

**Quick mode** (above) — pass IRI, kind, capability, and input/output types as flags. The CLI fills in a synthetic capability declaration. Suitable for one-off installs and ad-hoc testing.

**Full mode** — provide a `--definition <file>` whose content is an Eigon-JSON or ESL file declaring the capability resource. The CLI fills in only `wasm_binary` and `implementation: "wasm"`. Suitable for production installs where the capability metadata (description, version, additional declarations) lives in source control.

```bash
eigenius --endpoint http://localhost:50051 capability install \
    my-component.wasm \
    --definition my-component-decl.esl
```

The definition file's resource shape mirrors what's stored in the capability registry — see [`kernel/src/capability/`](../../../kernel/src/capability/) for the schema.

## 9.9. Fuel and memory limits

Each WASM execution is bounded by:

- **Fuel** — an instruction count budget enforced by Wasmtime. Default: 100 million instructions per dispatch (configurable per-capability when needed). Exceeded → execution aborts with `OutOfFuel` error.
- **Memory** — bounded linear memory (default: 64 MiB). Exceeded → execution aborts with `MemoryLimitExceeded`.

The defaults are sized for typical structural-validation workloads. Components that do heavy computation (large transforms, JSON parsing of huge documents) may need higher fuel limits — request via the capability declaration's `fuel_limit` property.

The fuel-and-memory machinery lives in [`crates/wasm-runtime/`](../../../crates/wasm-runtime/).

## 9.10. Testing components in isolation

For unit testing a WASM component without standing up a full kernel, the SDK exposes the same `Resource` and `Value` types as a Rust library. Compile the component's logic into a non-WASM binary and exercise the `execute` function directly:

```rust
#[cfg(test)]
mod tests {
    use eigenius_wasm_sdk::{Resource, Value};

    #[test]
    fn rejects_empty_title() {
        let mut input = Resource::new();
        input.set("urn:example:doc:title", Value::String("".into()));
        // ... call your validation logic ...
    }
}
```

The SDK is `no_std`-compatible by feature flag (`default-features = false`) so it links cleanly into both the WASM target and a host test binary.

## 9.11. Test paths

The kernel test suite covers the WASM dispatch paths, all in `kernel/tests/`:

1. Pure component (kernel-hosted) — `wasm-doc-validator`
2. Read-capability component — `wasm-read-query-probe`
3. IO component (orchestrator-hosted) — `wasm-http-shout`
4. D14 institution (kernel-hosted, three WASM crates: `wasm-d14-dock`, `wasm-d14-assay`, plus the `wasm-d14-arrhenius` transformation Component) — see [chapter 10](10-wasm-institutions.md) and [`kernel/tests/d14_dock_assay_demo_wasm.rs`](../../../kernel/tests/d14_dock_assay_demo_wasm.rs)
5. Component install + dispatch round-trip
6. Capability persistence across restart (with `--db`)
7. Fuel-exhaustion handling
8. Memory-limit handling

When extending the WASM machinery, run `cargo test --workspace` to exercise all of these.

---

Next: **[10. Building WASM institutions →](10-wasm-institutions.md)**
