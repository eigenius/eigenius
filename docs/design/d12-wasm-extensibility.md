# D12: WASM Extensibility

*Design document for the Eigenius project — April 2026*

> **Status: REMOVED (2026-07-08).** WASM extensibility was removed from
> the project. WASM was one of three institution/component backends
> (`RuntimeKind::Wasm | External | InProcess`) and carried the `wasmtime`
> dependency, but no production ontology declared `runtime: wasm` — only
> tests/demos exercised it. The institution framework and its other two
> backends (external via the D31 runtime substrate; in-process Rust) are
> unaffected. This document is retained as historical design record. See
> `docs/notes/wasm-removal-analysis.md` for the teardown scope.

**Status (historical):** Implemented. Kernel hosting of pure/read components landed
in Phase 8.0; orchestrator hosting of IO components landed via
[D12b](d12b-orchestrator-wasm-plan.md) (napi-rs + wasmtime). Decision #19
below is resolved.
**Depends on:** D6 (execution architecture), D9 (NbE unification), D10 (institution protocol)
**Companion:** [D12b — Orchestrator-Side WASM Implementation Plan](d12b-orchestrator-wasm-plan.md)
**Supersedes:** Previously planned D12 (Capability SDK) and D13 (Wire Format), merged here.

---

## 1. Overview

Eigenius supports two kinds of extension points: **components** (called during
program execution) and **fiber reasoners** (called during institution queries
and morphism validation). Phases 4-7 implement these as Rust trait objects
compiled into the kernel or dispatched to the Deno orchestrator via gRPC.

Phase 8 adds a third hosting option: **WASM modules** running in a sandbox.
The hosting location follows the capability level:

- **Pure and read** WASM modules run in the **kernel** via Wasmtime. They
  have direct access to the layer chain — no serialization round-trip for
  resource resolution. The kernel stays side-effect-free in its WASM
  hosting: no IO imports are ever linked.
- **IO** WASM modules run in the **orchestrator**. They have access to
  LLM adapters and external APIs directly, without a gRPC round-trip
  back to the orchestrator. The orchestrator is already the untrusted-code
  boundary for IO operations.
- **Institution fiber reasoners** run in the **kernel** (they need read
  and query access but never IO).

A WASM module is a self-contained binary that the host loads, instantiates,
and invokes within a sandbox. The module has no access to the host
filesystem, network, or memory outside its own linear memory. Execution
is bounded by a fuel budget — an infinite loop is terminated, not
tolerated.

Why WASM rather than another extension mechanism:

- **Sandboxing.** WASM linear memory isolation and capability-based imports
  mean untrusted code cannot read kernel memory, access the network, or
  corrupt state.
- **Language-agnostic.** Authors can write extensions in Rust, C, Go, or
  any language that compiles to WASM. The interface is defined in WIT
  (WASM Interface Types), not Rust traits.
- **Deterministic metering.** Wasmtime's fuel mechanism provides precise
  execution budgets. A module that exceeds its budget is terminated with
  an error, not a panic.
- **Portability.** The same WASM binary runs on any platform the kernel
  runs on — no recompilation needed when moving between Linux, macOS, or
  edge deployments.

### 1.1 What this document covers

This document specifies:

1. How WASM modules integrate with the existing `BuiltinComponent` and
   `FiberReasoner` interfaces (no new dispatch paths)
2. The WIT interface definition for components and institutions
3. Resource serialization across the WASM boundary (Eigon-CBOR)
4. Capability levels and their mapping to WASM imports
5. Registration via ontology resources
6. Fuel and memory limits
7. The SDK crate for Rust authors

### 1.2 What this document does not cover

- The security model for namespace delegation and trust chains (D14)
- Capability versioning and backward compatibility (D17)
- External-service hosting via gRPC (already implemented, D6)

---

## 2. Integration with Existing Interfaces

WASM is a hosting mechanism, not a new kind of extension. The same
`BuiltinComponent` and `FiberReasoner` traits are used regardless of
hosting. WASM modules register into the existing registries — the
dispatch path is unchanged.

```
                  BuiltinComponent
                  /       |       \
        Identity    Remote      Wasm
        (in-kernel) (gRPC)      (kernel or orchestrator)

                  FiberReasoner
                  /           \
        Rust trait obj      Wasm
        (in-kernel)         (kernel, Wasmtime)
```

**In the kernel:** pure/read WASM components register as `WasmComponent`
(implementing `BuiltinComponent`). WASM institutions register as
`WasmFiberReasoner` (implementing `FiberReasoner`). The kernel's evaluator
dispatches to them by IRI, same as any other component.

**In the orchestrator:** IO WASM components register in the orchestrator's
`ComponentRegistry` alongside `CompleteText` and `CompleteJson`. The kernel
dispatches to them via the existing `ComponentExecutor` gRPC service — the
kernel doesn't know whether the orchestrator runs the component as
TypeScript or WASM.

The program author doesn't know or care where a component runs. The
`capability_level` on the ontology resource determines the hosting
location. The same WASM binary works in either host — the WIT interface
is identical.

### 2.1 Terminology

The architecture document uses "capability" for sandboxed extension code.
The implementation uses "component" for program-level extensions. This
document uses both terms with their established meanings:

- **Component**: an extension called during program execution via `Apply`.
  Registered in `ComponentRegistry`. Has an IRI, input/output types,
  optional `argument_type`.
- **Fiber reasoner / institution**: an extension that provides domain-specific
  reasoning. Registered in `InstitutionRegistry`. Has morphism types,
  query types, structural properties.
- **Capability level**: the permission tier (pure/read/IO) that determines
  what host functions a WASM module can call.

---

## 3. The WASM Component Model

Eigenius uses the **WASM Component Model** (W3C standard) rather than a
custom host-guest protocol. The Component Model defines:

- **WIT (WASM Interface Types):** a typed interface definition language
  for declaring imports and exports
- **Canonical ABI:** standard memory layout for passing structured data
  (strings, lists, records, variants) across the boundary
- **Components:** self-describing WASM binaries that declare their imports
  and exports in terms of WIT interfaces

Wasmtime supports the Component Model natively. Using it means:

- No custom `alloc`/`dealloc` protocol — the canonical ABI handles memory
  management
- No manual CBOR serialization in the guest — WIT types map to native
  language types via code generators (`wit-bindgen` for Rust, `wit-bindgen`
  for C, etc.)
- Interface evolution via WIT versioning, not ad-hoc protocol changes

### 3.1 Why not raw WASM modules?

Raw WASM core modules require manual memory management: the host allocates
a buffer in the guest's linear memory, writes data, passes the pointer.
The guest must export `alloc` and `dealloc` functions. This is error-prone,
security-sensitive, and not standardized.

The Component Model eliminates this entire class of bugs. The tradeoff is
that guest code must be compiled as a WASM component (not a core module),
which requires `cargo component` or equivalent tooling. This is a
reasonable requirement for an SDK-based workflow.

---

## 4. WIT Interface Definition

### 4.1 Shared types

```wit
package eigenius:extension@0.1.0;

/// Types shared across all extension interfaces.
interface types {
    /// A CBOR-encoded Eigon resource.
    /// The canonical serialization format for crossing the boundary.
    type resource-data = list<u8>;

    /// An IRI string.
    type iri = string;

    /// Result of a component execution.
    record component-result {
        /// CBOR-encoded output resource.
        output: resource-data,
        /// Whether this component performed IO (for trace classification).
        is-io: bool,
    }

    /// Metrics from a component execution.
    record component-metrics {
        provider: string,
        model: string,
        prompt-tokens: s64,
        completion-tokens: s64,
        latency-ms: s64,
    }

    /// Result of morphism validation.
    enum validation-result {
        valid,
        invalid,
        undecidable,
    }
}
```

### 4.2 Host imports (kernel to WASM)

The kernel exports three tiers of host functions, matching the capability
modes from D9:

```wit
/// Read-only access to the knowledge graph.
/// Available to all capability levels.
interface read-access {
    use types.{resource-data, iri};

    /// Resolve a resource by IRI from the layer chain.
    /// Returns none if the resource doesn't exist.
    resolve: func(iri: iri) -> option<resource-data>;

    /// Get a property value from a CBOR-encoded resource.
    /// Convenience function to avoid full CBOR parsing in the guest
    /// for simple property lookups.
    get-property: func(resource: resource-data, property: iri)
        -> option<resource-data>;
}

/// Query access to the knowledge graph.
/// Available to read and IO capability levels.
interface query-access {
    use types.{resource-data};

    /// Execute an EigenQL query. Returns results as a list of
    /// CBOR-encoded resources.
    query: func(eigenql: string) -> result<list<resource-data>, string>;
}

/// IO access: dispatch to other components.
/// Available to IO capability level only.
interface io-access {
    use types.{resource-data, iri};

    /// Dispatch to another component by IRI.
    /// This allows WASM components to compose with other components
    /// (including remote orchestrator components like CompleteText).
    dispatch-component: func(
        component-iri: iri,
        input: resource-data,
        argument: resource-data,
    ) -> result<resource-data, string>;
}
```

### 4.3 Component world

```wit
/// A pure WASM component. Hosted in the kernel.
/// Has read-only access to the layer chain.
world eigenius-component {
    import read-access;

    /// Execute the component with input and optional argument.
    export execute: func(
        input: resource-data,
        argument: resource-data,
    ) -> result<component-result, string>;

    /// Return metadata about this component.
    /// Called once at registration time.
    export component-info: func() -> tuple<iri, bool>;
    // Returns: (component IRI, is_io flag)
}

/// A read-level WASM component. Hosted in the kernel.
/// Has read-only access plus query capability.
world eigenius-component-read {
    import read-access;
    import query-access;

    export execute: func(
        input: resource-data,
        argument: resource-data,
    ) -> result<component-result, string>;

    export component-info: func() -> tuple<iri, bool>;
}

/// An IO-level WASM component. Hosted in the orchestrator.
/// Has read and query access plus the ability to dispatch
/// to other components (including LLM adapters).
world eigenius-component-io {
    import read-access;
    import query-access;
    import io-access;

    export execute: func(
        input: resource-data,
        argument: resource-data,
    ) -> result<component-result, string>;

    export component-info: func() -> tuple<iri, bool>;
}
```

### 4.4 Institution world

```wit
/// A WASM fiber reasoner for a Grothendieck institution.
world eigenius-institution {
    import read-access;
    import query-access;

    /// Declare this institution's fiber structure.
    /// Called once at registration time. Returns CBOR-encoded
    /// FiberDeclaration (institution IRI, morphism types,
    /// query types, structural properties).
    export fiber-declaration: func() -> resource-data;

    /// Execute a fiber query.
    export query: func(query: resource-data) -> result<resource-data, string>;

    /// Validate a morphism.
    export validate-morphism: func(morphism: resource-data)
        -> result<validation-result, string>;

    /// Discover morphisms between resources.
    export discover-morphisms: func(resources: list<resource-data>)
        -> result<list<resource-data>, string>;
}
```

---

## 5. Resource Serialization

Resources cross the WASM boundary as **Eigon-CBOR** encoded bytes
(`resource-data = list<u8>`). CBOR is already implemented in the kernel
(`eigon_cbor.rs`) and is the canonical wire format for gRPC and storage.
Using it for WASM means:

- No new serialization format to implement or maintain
- Deterministic encoding (important for trace cache keys)
- Compact representation (less data to copy across the boundary)

The Component Model's canonical ABI handles the `list<u8>` transfer —
the host and guest never share raw pointers. The ABI copies the bytes
into the recipient's memory via a well-defined protocol.

### 5.1 What the guest sees

A guest component receives `resource-data` (opaque bytes). Using the SDK
crate (§10), the guest deserializes this into a `Resource` struct with
typed property access:

```rust
use eigenius_wasm_sdk::{Resource, Value};

fn execute(input: Vec<u8>, argument: Vec<u8>) -> Result<Vec<u8>, String> {
    let input = Resource::from_cbor(&input)?;
    let name = input.get_string("urn:example:name")?;

    let mut output = Resource::new();
    output.set("urn:example:greeting", Value::String(format!("Hello, {name}")));
    Ok(output.to_cbor())
}
```

The SDK crate provides `Resource` and `Value` types that mirror the
kernel's types but are independent (no shared memory, no kernel dependency).

### 5.2 Performance considerations

CBOR serialization adds overhead compared to shared-memory approaches.
For the expected use cases (domain validators, institution reasoners),
resources are typically small (< 10 KB). The serialization cost is
negligible compared to the computation the module performs.

For large resources (e.g., processing a collection of thousands of items),
the `get-property` host function allows targeted access without
deserializing the entire resource in the guest.

---

## 6. WASM Module Lifecycle

### 6.1 Loading

WASM modules are loaded when:

1. The kernel starts and processes bootstrap layers (for built-in WASM
   extensions)
2. A `load` RPC commits a layer containing a WASM component or institution
   resource

The kernel reads the WASM binary from the ontology resource (inline bytes
or blob store reference) and compiles it via Wasmtime's ahead-of-time
compiler. The compiled module is cached by content hash.

### 6.2 Instantiation

**Phase 8: fresh instance per invocation.** Each call to `execute` (for
components) or `query`/`validate_morphism`/`discover_morphisms` (for
institutions) creates a new Wasmtime instance. This guarantees:

- No mutable state leaks between invocations
- No shared references to prior inputs
- Clean fuel budget for each call

The overhead of instantiation is low (~microseconds for the Component
Model) because the module is pre-compiled and the instance only needs
memory allocation and import linking.

**Future optimization:** Instance pooling with state reset. The Component
Model's stateless design makes this safe — components have no mutable
globals by default. This optimization is deferred until profiling shows
instantiation is a bottleneck.

### 6.3 Invocation

1. Kernel creates a new Wasmtime `Store` with fuel limit and memory cap
2. Kernel instantiates the component, linking host imports based on
   capability level
3. Kernel serializes the input/argument resources to CBOR
4. Kernel calls the guest's exported function via the Component Model ABI
5. Guest executes, calling host imports as needed
6. Guest returns the result (CBOR bytes or error)
7. Kernel deserializes the output, creates the `ComponentResult` or
   `MorphismValidation`
8. Store is dropped (instance freed)

### 6.4 Error handling

| Condition | Kernel behavior |
|-----------|----------------|
| Guest returns `Err(message)` | Propagated as `Err(String)` to the caller |
| Fuel exhausted | Wasmtime trap → `Err("fuel exhausted")` |
| Memory limit exceeded | Wasmtime trap → `Err("memory limit exceeded")` |
| Guest panics (unreachable) | Wasmtime trap → `Err("wasm trap: ...")` |
| CBOR deserialization fails | `Err("invalid output: ...")` |

No WASM error can panic the kernel. All traps are caught and converted
to `Result::Err`.

---

## 7. Capability Levels and Import Sets

Each WASM module declares a capability level that determines which host
functions are available and where the module runs. This mirrors the NbE
capability modes (D9 §7):

| Capability level | WIT world | Host | Host imports | Use case |
|-----------------|-----------|------|--------------|----------|
| `pure` | `eigenius-component` | Kernel | `read-access` | Validators, transformers, formatters |
| `read` | `eigenius-component-read` | Kernel | `read-access`, `query-access` | Analyzers that query the graph |
| `io` | `eigenius-component-io` | Orchestrator | `read-access`, `query-access`, `io-access` | Components that call LLMs or external APIs |

Institutions always run in the kernel with `read-access` and
`query-access` (they need to resolve resources for morphism validation
and queries). They do not get `io-access` — institutions reason about
the graph, they don't call LLMs.

The capability level is declared on the component's ontology resource (§8).
For pure/read components, the kernel enforces it at instantiation time by
only linking the permitted imports. For IO components, the kernel sends
the WASM binary to the orchestrator via the `ComponentExecutor` gRPC
service, and the orchestrator handles instantiation with IO imports linked.

---

## 8. Registration via Ontology

WASM modules are registered as ordinary ontology resources. No special
loading mechanism — a `load` RPC that commits a layer with a WASM component
resource triggers registration.

### 8.1 Component registration

```json
{
  "@id": "urn:example:components:DocValidator",
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Component"],
  "urn:eigenius:core:short_name": "DocValidator",
  "urn:eigenius:core:description": "Validates document structure.",
  "urn:eigenius:program:component:input_type": "urn:example:Document",
  "urn:eigenius:program:component:output_type": "urn:example:ValidationResult",
  "urn:eigenius:program:component:capability_level":
    "urn:eigenius:program:capability_levels:pure",
  "urn:eigenius:program:component:implementation": "wasm",
  "urn:eigenius:program:component:wasm_binary": "<base64-encoded WASM>",
  "urn:eigenius:program:component:deterministic": true,
  "urn:eigenius:program:component:fallible": true
}
```

For large modules, use a blob store reference instead of inline bytes:

```json
{
  "urn:eigenius:program:component:wasm_binary_ref":
    "urn:eigenius:blob:sha256:abc123..."
}
```

The kernel tries `wasm_binary` first (inline), then `wasm_binary_ref`
(blob store lookup).

### 8.2 Institution registration

```json
{
  "@id": "urn:example:institutions:FEA",
  "urn:eigenius:core:is_a": ["urn:eigenius:institution:Institution"],
  "urn:eigenius:core:short_name": "FEA",
  "urn:eigenius:core:description": "Finite Element Analysis institution.",
  "urn:eigenius:institution:implementation": "wasm",
  "urn:eigenius:institution:wasm_binary_ref":
    "urn:eigenius:blob:sha256:def456..."
}
```

### 8.3 Registration flow

1. `load` RPC receives a layer with a component/institution resource
2. Kernel validates the resource against the ontology (standard validation)
3. Kernel checks `implementation` property:
   - `"builtin"` → already registered at startup (CompleteText, etc.)
   - `"wasm"` → load and compile the WASM binary, register as component
     or institution
   - absent or `"remote"` → dispatch via gRPC (existing behavior)
4. For WASM: kernel calls `component-info` (or `fiber-declaration`) to
   verify the module's self-declared IRI matches the resource's `@id`
5. Module is registered in the appropriate registry

### 8.4 Namespace protection

WASM modules cannot register under protected namespaces:

- `urn:eigenius:core:` — core ontology
- `urn:eigenius:program:` — program ontology (except under
  `urn:eigenius:program:components:` for new components)
- `urn:eigenius:reflection:` — reflection ontology
- `urn:eigenius:institution:` — institution ontology

This is enforced at load time, before the WASM module is instantiated.
The existing namespace protection in the layer system applies.

---

## 9. Fuel and Memory Limits

### 9.1 Fuel

Wasmtime's fuel mechanism counts instructions executed. Each WASM
instruction consumes one unit of fuel. When fuel runs out, execution
traps immediately.

Default fuel budget: **10,000,000** (~10M instructions). This is enough
for complex validation and reasoning but prevents infinite loops and
runaway computation.

The fuel budget can be configured per component/institution via an
ontology property:

```json
{
  "urn:eigenius:program:component:fuel_limit": 50000000
}
```

### 9.2 Memory

Wasmtime allows setting a maximum linear memory size per instance.

Default: **64 MB**. Configurable per module:

```json
{
  "urn:eigenius:program:component:memory_limit_mb": 128
}
```

### 9.3 Timeout

In addition to fuel, a wall-clock timeout prevents WASM modules from
blocking the kernel indefinitely (e.g., via host function calls that
take a long time):

Default: **30 seconds**.

---

## 10. SDK Crate

The `eigenius-wasm-sdk` crate provides Rust authors with ergonomic
bindings for writing WASM components and institutions. It is a
compile-time dependency — it produces no runtime overhead beyond the
WIT-generated glue code.

### 10.1 Phase 8: manual bindings

For Phase 8, the SDK provides:

- `Resource` and `Value` types mirroring the kernel's types
- CBOR serialization/deserialization (`Resource::from_cbor`, `to_cbor`)
- `HostContext` wrapper for calling host imports (`resolve`, `query`, etc.)
- WIT-generated bindings via `wit-bindgen`

The author writes a Rust crate that depends on `eigenius-wasm-sdk`,
implements the WIT-generated trait, and compiles with
`cargo component build`:

```rust
use eigenius_wasm_sdk::{Resource, Value, HostContext};

// WIT-generated trait
impl eigenius::extension::component::Guest for MyComponent {
    fn execute(
        input: Vec<u8>,
        argument: Vec<u8>,
    ) -> Result<eigenius::extension::types::ComponentResult, String> {
        let ctx = HostContext::new();
        let input = Resource::from_cbor(&input)
            .map_err(|e| format!("bad input: {e}"))?;

        // Access a property
        let text = input.get_string("urn:example:text")
            .ok_or("missing text property")?;

        // Resolve another resource from the graph
        let schema = ctx.resolve("urn:example:Schema")
            .ok_or("schema not found")?;

        // Build output
        let mut output = Resource::new();
        output.set("urn:example:valid", Value::Boolean(true));
        output.set("urn:example:message", Value::String("OK".into()));

        Ok(eigenius::extension::types::ComponentResult {
            output: output.to_cbor(),
            is_io: false,
        })
    }

    fn component_info() -> (String, bool) {
        ("urn:example:components:DocValidator".into(), false)
    }
}
```

### 10.2 Future: proc-macro SDK

A future version of the SDK adds `#[eigenius_component]` and
`#[eigenius_institution]` proc macros that generate the WIT trait
implementation, CBOR serialization, and boilerplate:

```rust
#[eigenius_component(
    iri = "urn:example:components:DocValidator",
    capability = "pure",
)]
fn validate(input: Resource, _arg: Resource, ctx: &ReadContext)
    -> Result<Resource, String>
{
    let text = input.get_string("urn:example:text")?;
    let mut output = Resource::new();
    output.set("urn:example:valid", Value::Boolean(!text.is_empty()));
    Ok(output)
}
```

The proc macro:

1. Generates the `Guest` trait implementation
2. Handles CBOR serialization/deserialization
3. Wraps the function in error handling
4. Generates `component-info` from the attribute

This is deferred to a later phase — manual bindings work first, proc
macros add ergonomics.

---

## 11. Kernel Implementation (Pure/Read Components + Institutions)

The kernel hosts pure/read WASM components and all WASM institutions.
IO WASM components are hosted by the orchestrator (§11.5).

### 11.1 New types

```rust
/// A pure/read component backed by a WASM module, hosted in the kernel.
pub struct WasmComponent {
    engine: wasmtime::Engine,
    component: wasmtime::component::Component,
    component_iri: String,
    capability_level: CapabilityLevel,  // Pure or Read only
    fuel_limit: u64,
    memory_limit_mb: u32,
}

impl BuiltinComponent for WasmComponent {
    fn is_io(&self) -> bool { false }  // Never IO — those go to orchestrator

    fn execute(
        &self,
        input: &Resource,
        argument: Option<&Resource>,
        layer: &Layer,
    ) -> Result<ComponentResult, String> {
        // 1. Create Store with fuel + memory limits
        // 2. Instantiate component with capability-appropriate imports
        // 3. Serialize input/argument to CBOR
        // 4. Call guest execute()
        // 5. Deserialize output from CBOR
        // 6. Return ComponentResult
    }
}

/// A fiber reasoner backed by a WASM module, hosted in the kernel.
pub struct WasmFiberReasoner {
    engine: wasmtime::Engine,
    component: wasmtime::component::Component,
    fuel_limit: u64,
    memory_limit_mb: u32,
}

impl FiberReasoner for WasmFiberReasoner {
    // Each method: instantiate, serialize, call, deserialize
}
```

### 11.2 Module loading

```rust
/// Load a WASM component from an ontology resource.
/// Only pure/read components are loaded in the kernel.
/// IO components are forwarded to the orchestrator.
pub fn load_wasm_component(
    resource: &Resource,
    layer: &Layer,
) -> Result<WasmComponent, String> {
    let binary = get_wasm_binary(resource, layer)?;
    let engine = wasmtime::Engine::new(
        wasmtime::Config::new().wasm_component_model(true),
    )?;
    let component = wasmtime::component::Component::new(&engine, &binary)?;

    // Call component-info to get IRI and is_io flag
    // Verify IRI matches resource @id
    // Read capability_level, fuel_limit, memory_limit_mb from resource

    Ok(WasmComponent { engine, component, ... })
}
```

### 11.3 Host function linking

The kernel only links pure and read imports — no IO imports are ever
provided by the kernel's WASM host:

```rust
fn link_imports(
    linker: &mut Linker<HostState>,
    capability_level: CapabilityLevel,
) {
    // Always link read-access
    linker.func_wrap("read-access", "resolve", |ctx, iri| { ... });
    linker.func_wrap("read-access", "get-property", |ctx, res, prop| { ... });

    // Link query-access for read level
    if capability_level >= CapabilityLevel::Read {
        linker.func_wrap("query-access", "query", |ctx, eigenql| { ... });
    }

    // io-access is never linked in the kernel — IO modules
    // are hosted by the orchestrator
}
```

### 11.4 Integration with server startup

```rust
// In start_server(), after loading bootstrap layers:
// Scan all committed layers for WASM component/institution resources
for resource in layer_chain.all_resources() {
    if is_wasm_component(resource) {
        let cap_level = get_capability_level(resource);
        if cap_level == CapabilityLevel::IO {
            // IO WASM components are forwarded to the orchestrator
            // via the existing remote component registration path
            continue;
        }
        let wasm_comp = load_wasm_component(resource, &layer)?;
        registry.register(wasm_comp.component_iri.clone(), Box::new(wasm_comp));
    }
    if is_wasm_institution(resource) {
        let wasm_inst = load_wasm_institution(resource, &layer)?;
        institution_registry.register(Box::new(wasm_inst))?;
    }
}
```

### 11.5 Orchestrator implementation (IO components)

IO WASM components run in the orchestrator alongside TypeScript
components like CompleteText and CompleteJson. The orchestrator:

1. Receives the WASM binary from the kernel (via a new
   `RegisterWasmComponent` RPC, or bundled with the layer data)
2. Compiles and caches the module
3. On each `ComponentExecutor.Execute` call, instantiates the module
   with all three import tiers linked (`read-access`, `query-access`,
   `io-access`)
4. For `read-access` and `query-access` host functions, the orchestrator
   proxies back to the kernel via the existing gRPC connection
5. For `io-access`, the orchestrator implements `dispatch-component`
   locally — calling other registered handlers (including LLM adapters)
   without a round-trip

The orchestrator hosts WASM components via **wasmtime embedded through a
napi-rs native addon** (see [D12b](d12b-orchestrator-wasm-plan.md) for the
implementation plan; a [spike report](../../spikes/napi-rs-async/REPORT.md)
records the decision rationale over jco/JSPI and Atomics.wait alternatives).
The WIT interface is runtime-agnostic — the same WASM binary works in both
kernel and orchestrator.

From the kernel's perspective, an IO WASM component is just another
remote component dispatched via `ComponentExecutor` gRPC. The kernel
doesn't know the orchestrator is running it as WASM vs. TypeScript.

---

## 12. Trace Integration

WASM components produce traces identical in structure to orchestrator
components. The trace cache key is computed the same way:

```
SHA-256(component_iri || canonicalize(input) || canonicalize(argument))
```

For IO WASM components, the kernel checks the trace store before
instantiation. On cache hit, the WASM module is never loaded — the
cached output is returned directly.

Pure and read WASM components do not produce traces (they are
deterministic given the same input and layer state).

---

## 13. Example: Document Validator

A complete worked example of a WASM component that validates document
structure.

### 13.1 Ontology (ESL)

```esl
namespace core = "urn:eigenius:core";
namespace doc = "urn:example:doc";

class doc:Document {
    description = "A structured document.";
    requires doc:title, doc:body, doc:section_count;
}

property doc:title : core:string {
    description = "Document title.";
    min_length = 1;
    max_length = 200;
}

property doc:body : core:string {
    description = "Document body text.";
}

property doc:section_count : core:integer {
    description = "Number of sections.";
    min_value = 1;
}
```

### 13.2 WASM component (Rust)

```rust
use eigenius_wasm_sdk::{Resource, Value};

impl Guest for DocValidator {
    fn execute(input: Vec<u8>, _argument: Vec<u8>)
        -> Result<ComponentResult, String>
    {
        let doc = Resource::from_cbor(&input)?;

        let title = doc.get_string("urn:example:doc:title")
            .ok_or("missing title")?;
        let body = doc.get_string("urn:example:doc:body")
            .ok_or("missing body")?;
        let sections = doc.get_integer("urn:example:doc:section_count")
            .ok_or("missing section_count")?;

        let mut errors = Vec::new();

        if title.is_empty() {
            errors.push("title must not be empty");
        }
        if body.len() < 100 {
            errors.push("body must be at least 100 characters");
        }
        if sections < 1 {
            errors.push("must have at least one section");
        }

        let mut output = Resource::new();
        output.set("urn:example:doc:valid", Value::Boolean(errors.is_empty()));
        if !errors.is_empty() {
            output.set(
                "urn:example:doc:errors",
                Value::Array(errors.into_iter()
                    .map(|e| Value::String(e.into()))
                    .collect()),
            );
        }

        Ok(ComponentResult {
            output: output.to_cbor(),
            is_io: false,
        })
    }

    fn component_info() -> (String, bool) {
        ("urn:example:components:DocValidator".into(), false)
    }
}
```

### 13.3 Registration

```json
{
  "@id": "urn:example:components:DocValidator",
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Component"],
  "urn:eigenius:core:short_name": "DocValidator",
  "urn:eigenius:program:component:input_type": "urn:example:doc:Document",
  "urn:eigenius:program:component:output_type": "urn:example:doc:ValidationResult",
  "urn:eigenius:program:component:capability_level":
    "urn:eigenius:program:capability_levels:pure",
  "urn:eigenius:program:component:implementation": "wasm",
  "urn:eigenius:program:component:wasm_binary_ref":
    "urn:eigenius:blob:sha256:abc123...",
  "urn:eigenius:program:component:deterministic": true
}
```

### 13.4 Usage in a program

```esl
program doc:check : doc:Document -> doc:ValidationResult {
    DocValidator(input)
}
```

The kernel dispatches to the WASM module just like any other component.
The program author doesn't know or care that it's WASM.

---

## 14. Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Host-guest data protocol | WASM Component Model (WIT) | Standard ABI, no manual memory management |
| Serialization format | Eigon-CBOR | Already implemented, compact, deterministic |
| Instantiation model | Fresh instance per invocation | Safe, simple; pooling deferred until profiling shows need |
| Module storage | Inline bytes or blob store reference | Small modules inline, large modules via blob store |
| Capability enforcement | Import linking at instantiation | Pure modules don't see query/IO imports; link-time enforcement |
| Hosting split | Pure/read in kernel, IO in orchestrator | Kernel stays side-effect-free; orchestrator already has LLM adapters |
| Default fuel limit | 10M instructions | Sufficient for validators; configurable per module |
| Default memory limit | 64 MB | Sufficient for typical resources; configurable per module |
| SDK approach (Phase 8) | Manual bindings via wit-bindgen | Ship fast; proc-macro ergonomics deferred |
| Institution IO access | Not provided | Institutions reason about the graph, not call LLMs |
| Namespace protection | Existing layer-system enforcement | No new mechanism needed |
| Orchestrator WASM runtime | Wasmtime via napi-rs native addon | Resolved 2026-04-19 after jco/JSPI spike failed and Atomics.wait was rejected on complexity grounds; see [D12b](d12b-orchestrator-wasm-plan.md) and the [spike report](../../spikes/napi-rs-async/REPORT.md) |

---

## 15. Implementation Steps

1. Add `wasmtime` dependency with `component-model` feature to `kernel/Cargo.toml`
2. Define WIT interfaces in `wit/` directory
3. Implement `WasmComponent` struct wrapping Wasmtime component instantiation
4. Implement host function linking for each capability tier
5. Implement `WasmFiberReasoner` struct
6. Add WASM detection to the `load` path (check `implementation: "wasm"`)
7. Add WASM component/institution scanning at server startup
8. Create the `eigenius-wasm-sdk` crate with `Resource`, `Value`, CBOR support
9. Build the document validator example as a WASM component
10. Integration tests: sandbox isolation, fuel exhaustion, capability enforcement
11. CLI `capability list` / `capability test` subcommands

---

## 16. References

- [WASM Component Model](https://component-model.bytecodealliance.org/) — W3C specification
- [WIT specification](https://component-model.bytecodealliance.org/design/wit.html) — Interface definition language
- [Wasmtime Component Model support](https://docs.wasmtime.dev/api/wasmtime/component/) — Rust API
- [wit-bindgen](https://github.com/bytecodealliance/wit-bindgen) — Code generator for multiple languages
- [cargo-component](https://github.com/bytecodealliance/cargo-component) — Build tool for WASM components
- D6: Execution Architecture — kernel/orchestrator boundary
- D9: NbE Unification — capability modes (Pure/Read/IO)
- D10: Grothendieck Institution Protocol — FiberReasoner trait
