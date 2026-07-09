# 10. Building WASM institutions

> **REMOVED (2026-07-08).** WASM-backed institutions were removed. The
> institution *framework* (D14) is intact — institutions are still declared
> as ontology resources and implemented as runtimes — but the WASM backend
> is gone. Implement institutions as in-process Rust (`runtime: in_process`,
> e.g. `eigenius-reasoning`, `eigenius-lean`) or as external institutions via
> the runtime substrate (`runtime: external`, D31). This chapter is retained
> as historical record only. Background:
> [D12](../../design/d12-wasm-extensibility.md).

Institutions are domain-specific reasoning systems — typed reasoners that contribute structured fibres to the knowledge graph. Under D14 ([Institution Realisation](../../design/d14-institution-realisation.md)) they are *declared* as ontology resources committed to the layer chain (`Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism`) and *implemented* as a runtime that handles boundary translations and any opaque reasoning. The same WASM hosting machinery that runs components also hosts institutions, but against the dedicated `eigenius-institution-d14` WIT world (D14 §13).

Cross-link: this chapter is the **implementer** view. The **user** view (how programs and queries invoke institutions) is in [ESL §9](../esl/09-institutions.md) and [EigenQL §8](../eigenql/08-institutions.md).

## 10.1. The institution model under D14

A WASM institution exports three operations to the kernel:

| Operation | Purpose |
|---|---|
| `extract-typed` | Boundary: read a source-class resource and emit a EigenTT-typed payload (the `S` of a comorphism, or the input side of a Component-implemented QueryClass). |
| `reify` | Boundary: take a EigenTT-typed payload and construct a target-class resource (the `T` of a comorphism, or the output side of a Component-implemented QueryClass). |
| `query` | Optional escape hatch: answer an institution-defined query by returning a result resource. Required only for QueryClasses whose `query_handler` is the institution's own procedure (not a Component IRI). |

These are the only methods. No `fiber_declaration()`, no `validate-morphism`, no `discover-morphisms` — the four-method D10 trichotomy collapses into "boundary translation + optional reasoning". Operational profile (validates on Load? answers EigenQL FIBER? gets called from `NativeDecide`?) is determined by the QueryClass's `dispatch_role` set on the chain, not by which method the kernel calls.

The trait surface on the kernel side ([`kernel/src/institution/runtime.rs`](../../../kernel/src/institution/runtime.rs)):

```rust
pub trait Institution: Send + Sync {
    fn institution_iri(&self) -> &Iri;

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError>;

    fn reify(
        &self,
        procedure_iri: &Iri,
        value: &Val,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError>;

    /// Optional. Default returns NotImplemented.
    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> { … }
}
```

The WASM-host bridge that implements this trait against a Wasmtime-loaded binary is [`WasmInstitution`](../../../kernel/src/capability/wasm_institution_d14.rs).

## 10.2. The ontology-first principle

Every concept the institution exposes is a typed Resource on the chain. The kernel's [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) is built by scanning that chain (D14 §3, §9). For a WASM institution, the typical packaging is a WASM binary plus a sibling Eigon-JSON document declaring the institution's surface; the two are loaded together and the kernel auto-registers the binary against the `Institution` resource it finds (see §10.4).

The five resource shapes (D14 §4):

| Shape | Properties | What it does |
|---|---|---|
| `Institution` | `institution_iri`, `name`, `runtime`, optional `wasm_binary` | Identifies the institution and tells the kernel how to reach it. `runtime: wasm` + inline `wasm_binary` (base64 or `hex:` prefix) makes it auto-register. |
| `ExportFormat` | `from_class`, `payload_type`, `institution_ref`, `procedure` | "When you need a Float-typed view of a `DockingResult`, call procedure `extract_dg`." |
| `ImportFormat` | `to_class`, `payload_type`, `institution_ref`, `procedure` | "To construct an `AssayPrediction` from a Float, call procedure `reify_ic50`." |
| `QueryClass` | `query_class`, `result_class`, `dispatch_role`, `query_handler`, `institution_ref` | The institution's typed function. `dispatch_role` is one or more of `OnDemand` (FIBER), `AutoOnLoad` (Load gate), `Decidable` (NativeDecide). |
| `Comorphism` | `export_format`, `transformation`, `import_format`, `exact` | The triadic translation across an institution boundary; the transformation is a EigenTT Component IRI. |

The `Comorphism`'s well-typedness is checked at commit time (D14 §4.5): the `transformation`'s signature must equal `(payload_type(export_format)) → (payload_type(import_format))`. Mismatches are rejected by structural validation.

## 10.3. Project setup

A D14 institution is a `cargo-component` crate targeting the `eigenius-institution-d14` world. Skeleton `Cargo.toml`:

```toml
[package]
name = "my-institution"
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
world = "eigenius-institution-d14"
path = "<repo>/wit"
```

The SDK provides [resource builders](../../../sdk/wasm-sdk/src/institution.rs) for each declaration shape:

- `InstitutionDecl`, `ExportFormatDecl`, `ImportFormatDecl`, `QueryClassDecl`, `ComorphismDecl` — each exposes a typed builder API and emits a typed Resource on `.build()`.
- `RuntimeKind` (Wasm / External / InProcess) and `DispatchRole` (OnDemand / AutoOnLoad / Decidable) for the enum-valued properties.

These are *for guest-side use* (you can use them inside the WASM crate to construct your declaration document) but they are equally usable from the host or from any tool that builds Eigon documents. Declaration construction has no kernel-side magic; the resources can also be hand-written in JSON (the dock-assay demo's [`dock-assay.json`](../../../ontologies/examples/d14-dock-assay/dock-assay.json) does exactly this).

## 10.4. Implementing the three operations

The Guest trait that `wit-bindgen::generate!` creates has three exports. The dock side of the M8 demo ([`examples/wasm-d14-dock/src/lib.rs`](../../../examples/wasm-d14-dock/src/lib.rs)) implements only `extract_typed`:

```rust
use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({
    path: "../../wit",
    world: "eigenius-institution-d14",
});

const DELTA_G_PROP: &str = "urn:eigenius:demo:d14:delta_g";
const VALUE_PROP: &str = "urn:eigenius:core:value";
const EXTRACT_DG_PROC: &str = "urn:eigenius:demo:d14:proc:extract_dg";

struct DockInstitution;

impl Guest for DockInstitution {
    fn extract_typed(procedure_iri: String, input: Vec<u8>) -> Result<Vec<u8>, String> {
        if procedure_iri != EXTRACT_DG_PROC {
            return Err(format!("dock does not implement `{procedure_iri}`"));
        }
        let resource = Resource::from_cbor(&input).map_err(|e| format!("parse: {e}"))?;
        let delta_g = resource
            .get(DELTA_G_PROP)
            .and_then(|v| v.as_float())
            .ok_or_else(|| "DockingResult missing delta_g".to_string())?;

        // The EigenTT typed-value carrier shape: a single-Float wrapper resource.
        let mut wrapper = Resource::new();
        wrapper.set(VALUE_PROP, Value::Float(delta_g));
        Ok(wrapper.to_cbor())
    }

    fn reify(procedure_iri: String, _value: Vec<u8>) -> Result<Vec<u8>, String> {
        Err(format!("dock does not implement reify (`{procedure_iri}`)"))
    }

    fn query(procedure_iri: String, _input: Vec<u8>) -> Result<Vec<u8>, String> {
        Err(format!("dock does not implement query (`{procedure_iri}`)"))
    }
}

export!(DockInstitution);
```

The assay side ([`examples/wasm-d14-assay/src/lib.rs`](../../../examples/wasm-d14-assay/src/lib.rs)) is the dual — it implements `reify` (constructing an `AssayPrediction` from a Float-payload) plus `query` to handle three QueryClasses (`within_tolerance` Decidable, `assay_prediction_validity` AutoOnLoad, `validate_prediction` OnDemand). It dispatches on `procedure_iri` inside `query`:

```rust
fn query(procedure_iri: String, input: Vec<u8>) -> Result<Vec<u8>, String> {
    let resource = Resource::from_cbor(&input).map_err(|e| format!("parse: {e}"))?;
    let ctor = match procedure_iri.as_str() {
        WITHIN_TOLERANCE_PROC => within_tolerance_verdict(&resource),
        CHECK_ASSAY_PREDICTION_PROC => assay_prediction_verdict(&resource),
        VALIDATE_PREDICTION_PROC => {
            let candidate = resource.get(CANDIDATE_PROP).and_then(|v| v.as_embedded())
                .ok_or("validate_prediction: candidate missing")?;
            assay_prediction_verdict(candidate)
        }
        other => return Err(format!("assay does not implement `{other}`")),
    };
    Ok(verdict_resource(ctor).to_cbor())
}
```

The full dock-assay ontology that ties these together (Institution + ExportFormat + ImportFormat + 3× QueryClass + Comorphism + supporting classes) lives in [`ontologies/examples/d14-dock-assay/dock-assay.json`](../../../ontologies/examples/d14-dock-assay/dock-assay.json). The same JSON drives both the in-process test ([`d14_dock_assay_demo.rs`](../../../kernel/tests/d14_dock_assay_demo.rs)) and the WASM-hosted test ([`d14_dock_assay_demo_wasm.rs`](../../../kernel/tests/d14_dock_assay_demo_wasm.rs)) — the WASM test layers a child layer on top with `runtime: wasm` + inline `wasm_binary` overrides.

### 10.4.1. Verdict result resources

QueryClasses with `result_class: urn:eigenius:institution:Verdict` return a Verdict-shaped resource. Construct it with `is_a: [Verdict]` and `urn:eigenius:core:ctor_name` set to one of `Holds`, `Fails`, `Undecidable`. The kernel reads `ctor_name` to project the verdict (D14 §6.1).

```rust
fn verdict_resource(ctor: &str) -> Resource {
    let mut r = Resource::new();
    r.set("urn:eigenius:core:is_a",
          Value::Array(vec![Value::String("urn:eigenius:institution:Verdict".into())]));
    r.set("urn:eigenius:core:ctor_name", Value::String(ctor.into()));
    r
}
```

### 10.4.2. EigenTT typed-value carrier shape

`extract_typed` and `reify` exchange CBOR-encoded EigenTT typed values (`typed-value` in the WIT world — distinct from `resource-data` even though the on-the-wire form is the same). The kernel marshals primitives like `Float` as a single-property wrapper resource carrying the value at `urn:eigenius:core:value`. The dock and assay implementations both work with this shape: dock wraps `Float(delta_g)` into a wrapper on extract; assay reads the wrapper's first Float on reify. The Arrhenius transformation Component ([`examples/wasm-d14-arrhenius/src/lib.rs`](../../../examples/wasm-d14-arrhenius/src/lib.rs)) follows the same convention.

For richer types — tuples, records, inductive values — the SDK provides round-trip helpers between Rust types and `typed-value`. (See the SDK's `typed_value` module.)

## 10.5. Auto-registration from the layer chain

Under D14, the kernel doesn't need a bespoke `capability install --kind institution` flow — `Institution` declarations on the chain are automatically registered in the `InstitutionRuntime` at commit time. The mechanism ([`build_wasm_institution_runtime`](../../../kernel/src/capability/registration.rs)) walks every `Institution` resource whose `runtime` is `urn:eigenius:institution:runtimes:wasm`, decodes the inline `wasm_binary` (base64 or `hex:` prefix), and constructs a `WasmInstitution` keyed by the institution IRI.

Concretely, an `Institution` declaration with WASM packaging looks like:

```json
{
  "@id": "urn:eigenius:demo:d14:dock",
  "urn:eigenius:core:is_a": ["urn:eigenius:institution:Institution"],
  "urn:eigenius:institution:institution_iri": "urn:eigenius:demo:d14:dock",
  "urn:eigenius:institution:institution_name": "Dock",
  "urn:eigenius:institution:runtime": "urn:eigenius:institution:runtimes:wasm",
  "urn:eigenius:institution:wasm_binary": "hex:<long-hex-string>"
}
```

When this is loaded into the kernel via `eigenius load`, the EigeniusService's per-commit hook rebuilds both the `InstitutionIndex` (from the new ExportFormat / ImportFormat / QueryClass / Comorphism declarations) and the `InstitutionRuntime` (from the WASM binaries). All ESL programs and EigenQL queries that reference the institution's IRIs are immediately routed through it.

In-process and external runtimes (`runtime: in_process` or `runtime: external`) are skipped by the auto-registration scan — those are the caller's responsibility and remain a programmatic concern (kernel-embedded Rust integrations or out-of-process gRPC services, respectively).

## 10.6. Building, loading, and inspecting

```bash
# Build the WASM crate
cd examples/wasm-d14-dock
cargo component build

# Build sibling institutions and the transformation Component
cd ../wasm-d14-assay && cargo component build && cd -
cd examples/wasm-d14-arrhenius && cargo component build && cd -

# Load the declaration document (Institution + ExportFormat + … +
# Comorphism + supporting classes). Auto-registration kicks in on commit.
cargo run -q -p eigenius-cli -- \
    --endpoint http://localhost:50051 \
    load ontologies/examples/d14-dock-assay/dock-assay.json

# List registered institutions (kernel logs each one at info level on
# rebuild — see the EigeniusService::rebuild_institution_index hook).
```

For development the easiest path is to use the existing `just build-wasm` target plus the dock-assay test as a fixture-driven harness. The WASM test ([`kernel/tests/d14_dock_assay_demo_wasm.rs`](../../../kernel/tests/d14_dock_assay_demo_wasm.rs)) builds a layer with `runtime: wasm` + inline `wasm_binary` overrides for each institution and the transformation Component, runs `build_wasm_institution_runtime` and `scan_and_register`, and exercises the four-step pipeline + Decidable + AutoOnLoad dispatch end-to-end. Reading that test is the fastest way to internalise the surface.

## 10.7. Native (in-process) institutions

For institutions that need full Rust dependencies, performance-critical paths, or first-party integration with the kernel, you can implement the `Institution` trait directly as a Rust struct linked into the kernel binary. Declare with `runtime: urn:eigenius:institution:runtimes:in_process`; the auto-registration scan will skip it; register the `Box<dyn Institution>` programmatically via `InstitutionRuntime::register` at startup.

Trade-offs vs. WASM:

| Aspect | WASM institution | In-process institution |
|---|---|---|
| Sandboxed | Yes (Wasmtime) | No |
| Fuel/memory limits | Yes | No |
| Portability | Single `.wasm` binary | Per-platform crate |
| Dependency tree | Restricted (no_std-ish) | Full Cargo dependency tree |
| Performance | Wasmtime overhead per call | Direct Rust calls |
| Hot-installable | Yes (auto-registration on commit) | No (compiled into the kernel binary) |
| Best for | Untrusted / 3rd-party reasoners | First-party reasoners with heavy dependencies |

For most use cases, WASM is the right path. Native institutions are appropriate when the institution's reasoning code is already a kernel-internal Rust crate and the sandboxing / fuel discipline isn't worth the marshalling overhead.

## 10.8. Cross-references

For the *user* perspective on what institutions look like from the surface languages:

- [**ESL §9 — Institutions in ESL**](../esl/09-institutions.md) — invoking Decidable QueryClasses from program bodies; how comorphisms surface (and don't) in ESL.
- [**EigenQL §7 — FIBER clauses**](../eigenql/07-fiber-clauses.md) — invoking OnDemand QueryClasses from EigenQL.
- [**EigenQL §8 — Institutions in EigenQL**](../eigenql/08-institutions.md) — the full classification table, postfix Verdict predicates, comorphism coercion in FIBER params.

For the *protocol* specification:

- [**D14 — Institution Realisation**](../../design/d14-institution-realisation.md) — the canonical reference for the institution surface in Eigenius. (Supersedes D10.)
- [**D12 — WASM extensibility**](../../design/d12-wasm-extensibility.md) — the underlying WASM hosting machinery that institutions share with components.

For example code:

- [`examples/wasm-d14-dock/`](../../../examples/wasm-d14-dock/), [`examples/wasm-d14-assay/`](../../../examples/wasm-d14-assay/), [`examples/wasm-d14-arrhenius/`](../../../examples/wasm-d14-arrhenius/) — the M8 worked example.
- [`examples/wasm-d14-echo/`](../../../examples/wasm-d14-echo/) — the smallest viable D14 institution (smoke test of the WIT bindings).
- [`kernel/tests/d14_dock_assay_demo.rs`](../../../kernel/tests/d14_dock_assay_demo.rs) and [`kernel/tests/d14_dock_assay_demo_wasm.rs`](../../../kernel/tests/d14_dock_assay_demo_wasm.rs) — end-to-end harnesses.

---

Next: **[11. Runtime substrate →](11-runtime-substrate.md)**
