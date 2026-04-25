# 10. Building WASM institutions

Institutions are domain-specific reasoning modules — fiber reasoners over the knowledge graph. The same WASM hosting machinery that runs components also hosts institutions, but against a different WIT world (`eigenius-institution`) with a different surface (`fiber-declaration`, `query`, `validate-morphism`, `discover-morphisms`, plus optional `decide` and `translate`).

Cross-link: this chapter is the **implementer** view. The **user** view (how programs and queries invoke institutions) is in [ESL chapter 9](../esl/09-institutions.md) and [EigenQL chapter 8](../eigenql/08-institutions.md).

## 10.1. The institution model

An institution declares (at registration time) the morphism classes and query classes it answers, and implements four required operations:

| Operation | Purpose |
|---|---|
| `fiber-declaration` | Returns metadata: institution IRI, name, morphism/query/comorphism types, decide procedures |
| `query` | Answers a typed query resource (the `FIBER` clause's request) |
| `validate-morphism` | Domain-specific morphism validation (returns `valid`/`invalid`/`undecidable`) |
| `discover-morphisms` | Infers new morphisms from a given resource set |

Plus two optional operations from Phase 11c–11d:

| Operation | Purpose |
|---|---|
| `decide` | Evaluates a registered decide-predicate IRI against args; returns `Holds`/`Fails`/`Undecidable` |
| `translate` | Translates a resource across an institution boundary via a registered comorphism IRI |

The full kernel-side trait is [`FiberReasoner`](../../../kernel/src/institution/mod.rs); the WASM-side surface is [`wit/eigenius-component.wit`](../../../wit/eigenius-component.wit) under the `eigenius-institution` world.

## 10.2. Project setup

Same `cargo-component` pattern as components. The `Cargo.toml` selects the institution world:

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
world = "eigenius-institution"
path = "<repo>/wit"
```

The SDK provides institution-specific helpers in [`sdk/wasm-sdk/src/institution.rs`](../../../sdk/wasm-sdk/src/institution.rs):

- `FiberDeclaration` — builder for the resource the kernel expects from `fiber-declaration`.
- `MorphismValidation` — Rust-side enum mirroring the `validation-result` WIT enum (`Valid`, `Invalid(String)`, `Undecidable`).

## 10.3. Implementing the four required operations

The Guest trait that `wit-bindgen::generate!` creates has methods for each operation. Stubbed shape:

```rust
use eigenius_wasm_sdk::institution::FiberDeclaration;
use eigenius_wasm_sdk::{Resource, Value};

wit_bindgen::generate!({ path: "../../wit", world: "eigenius-institution" });

struct MyInstitution;

impl Guest for MyInstitution {
    fn fiber_declaration() -> Vec<u8> {
        // Build morphism class resources
        let mut my_morphism = Resource::with_id("urn:example:institutions:MyMorphism");
        my_morphism.set_is_a(["urn:eigenius:core:Class"]);
        // ... set short_name, description, requires ...

        let decl = FiberDeclaration {
            institution_iri: "urn:example:institutions:MyInst".into(),
            name: "My Institution".into(),
            morphism_types: vec![my_morphism],
            query_types: vec![],
            structural_properties: vec![],
        };
        decl.into_resource().to_cbor()
    }

    fn query(q: Vec<u8>) -> Result<Vec<u8>, String> {
        let query = Resource::from_cbor(&q).map_err(|e| format!("parse: {e}"))?;
        // ... handle the query, build response ...
        let mut response = Resource::new();
        // ... set response fields ...
        Ok(response.to_cbor())
    }

    fn validate_morphism(m: Vec<u8>) -> ValidationResult {
        let morphism = match Resource::from_cbor(&m) {
            Ok(r) => r,
            Err(_) => return ValidationResult::Invalid,
        };
        // ... domain-specific check ...
        if /* valid */ { ValidationResult::Valid }
        else { ValidationResult::Invalid }
    }

    fn discover_morphisms(_resources: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>, String> {
        Ok(vec![])  // Implement morphism discovery if applicable
    }
}

export!(MyInstitution);
```

Most institutions don't need `discover_morphisms` (it's for systems that infer relationships from existing resources); returning an empty Vec is the inert default.

## 10.4. Worked example: ordering institution

Source: [`examples/wasm-ordering-institution/`](../../../examples/wasm-ordering-institution/).

Domain: refinement steps in iterative computations. Declares two classes:

- **`Refinement`** — a morphism between two results, carrying `source`, `target`, `delta` properties.
- **`ConvergenceQuery`** — asks whether the latest refinement step converged below a tolerance, with `tolerance` and `latest_delta` parameters.

### `fiber_declaration`

The institution declares both classes plus the institution metadata. Excerpt:

```rust
fn fiber_declaration() -> Vec<u8> {
    let mut refinement = Resource::with_id(REFINEMENT_CLASS);
    refinement.set_is_a(["urn:eigenius:core:Class"]);
    refinement.set("urn:eigenius:core:short_name", Value::String("Refinement".into()));
    refinement.set("urn:eigenius:core:requires", Value::Array(vec![
        Value::String(SOURCE.into()),
        Value::String(TARGET.into()),
        Value::String(DELTA.into()),
    ]));

    let mut query_class = Resource::with_id(CONVERGENCE_QUERY_CLASS);
    query_class.set_is_a(["urn:eigenius:core:Class"]);
    query_class.set("urn:eigenius:core:short_name", Value::String("ConvergenceQuery".into()));
    query_class.set("urn:eigenius:core:requires", Value::Array(vec![
        Value::String(TOLERANCE.into()),
        Value::String(LATEST_DELTA.into()),
    ]));

    let decl = FiberDeclaration {
        institution_iri: INSTITUTION_IRI.into(),
        name: "WASM Ordering Institution".into(),
        morphism_types: vec![refinement],
        query_types: vec![query_class],
        structural_properties: vec![],
    };
    decl.into_resource().to_cbor()
}
```

The kernel parses this declaration at registration time and:

- Indexes `Refinement` in its **morphism dispatch table** — any morphism resource with `is_a` including `Refinement` routes to this institution.
- Indexes `ConvergenceQuery` in its **query dispatch table** — any `FIBER` clause naming this query class routes here.

### `validate_morphism`

```rust
fn validate_morphism(m: Vec<u8>) -> ValidationResult {
    let morphism = Resource::from_cbor(&m).expect("parse morphism");
    let delta = morphism.get_float(DELTA).unwrap_or(0.0);
    if delta > 0.0 { ValidationResult::Valid }
    else { ValidationResult::Invalid }
}
```

A simple domain rule: refinements must have strictly positive `delta`.

### `query`

```rust
fn query(q: Vec<u8>) -> Result<Vec<u8>, String> {
    let query = Resource::from_cbor(&q).map_err(|e| format!("parse query: {e}"))?;
    let tolerance = query.get_float(TOLERANCE).ok_or("missing tolerance")?;
    let latest_delta = query.get_float(LATEST_DELTA).ok_or("missing latest_delta")?;

    let converged = latest_delta <= tolerance;

    let mut response = Resource::new();
    response.set(CONVERGED, Value::Boolean(converged));
    response.set(CHECKED_DELTA, Value::Float(latest_delta));
    response.set(CHECKED_TOLERANCE, Value::Float(tolerance));
    Ok(response.to_cbor())
}
```

Plain answer to the convergence question, with the input parameters echoed back as part of the response (useful for downstream pattern matching in EigenQL).

When this institution is registered and EigenQL runs:

```eigenql
USING INSTITUTION "urn:eigenius:test:wasm:ordering" AS ord

MATCH Refinement(?m) { latest_delta: ?d, target: ?t }
FIBER  ord:ConvergenceQuery { tolerance: 0.01, latest_delta: ?d } AS ?conv
MATCH  ?conv { "urn:eigenius:test:wasm:converged": ?c }
WHERE  ?c = true
RETURN [] { m: ?m, t: ?t }
```

…the `FIBER` clause dispatches one query per binding through this institution's `query` function.

## 10.5. Installing, listing, testing

```bash
# Build
cd examples/wasm-ordering-institution
cargo component build

# Install
eigenius --endpoint http://localhost:50051 capability install \
    target/wasm32-unknown-unknown/debug/eigenius_wasm_ordering_institution.wasm \
    --as-iri urn:eigenius:test:wasm:ordering \
    --kind institution \
    --capability pure

# Verify
eigenius --endpoint http://localhost:50051 list-institutions

# Test fiber query
cat > /tmp/conv-query.json <<'EOF'
{
  "@id": "urn:example:queries:test1",
  "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:ConvergenceQuery"],
  "urn:eigenius:test:wasm:tolerance": 0.01,
  "urn:eigenius:test:wasm:latest_delta": 0.005
}
EOF
eigenius --endpoint http://localhost:50051 capability test \
    urn:eigenius:test:wasm:ordering \
    --input /tmp/conv-query.json \
    --mode query
```

The two `--mode` choices for institutions:

| Mode | Dispatches |
|---|---|
| `query` (default) | `query` operation — runs a fiber query |
| `discover` | `discover-morphisms` operation — infers morphisms from the input resources |

## 10.6. Native institutions (without WASM)

For institutions that need full Rust dependencies, performance-critical paths, or first-party integration with the kernel, you can write a **native institution** as a Rust crate that links into the kernel directly.

The native trait is [`FiberReasoner`](../../../kernel/src/institution/mod.rs):

```rust
pub trait FiberReasoner: Send + Sync {
    fn fiber_declaration(&self) -> FiberDeclaration;
    fn query(&self, query: &Resource, ctx: &ExecutionContext)
        -> Result<Resource, InstitutionError>;
    fn validate_morphism(&self, morphism: &Resource, ctx: &ExecutionContext)
        -> Result<MorphismValidation, InstitutionError>;
    fn discover_morphisms(&self, resources: &[Resource], ctx: &ExecutionContext)
        -> Result<Vec<Resource>, InstitutionError>;

    // Optional (Phase 11c–d):
    fn decide(&self, iri: &Iri, args: &[Value], ctx: &ExecutionContext)
        -> Result<DecResult, InstitutionError> { ... }
    fn translate(&self, iri: &Iri, source: &Resource, ctx: &ExecutionContext)
        -> Result<Resource, InstitutionError> { ... }
}
```

Trade-offs vs. WASM:

| Aspect | WASM institution | Native institution |
|---|---|---|
| Sandboxed | Yes (Wasmtime) | No |
| Fuel/memory limits | Yes | No |
| Portability | Single `.wasm` binary | Per-platform crate |
| Dependency tree | Restricted (no_std-ish) | Full Cargo dependency tree |
| Performance | WASM JIT overhead per call | Direct Rust calls |
| Hot-installable | Yes (via `capability install`) | No (compiled into the kernel binary) |
| Best for | Untrusted / 3rd-party reasoners | First-party reasoners with heavy dependencies |

Native institutions are registered at kernel startup via [`InstitutionRegistry::register`](../../../kernel/src/institution/mod.rs). To register a custom native institution, you'd modify the kernel binary (or run a fork). For most use cases, WASM is the right path.

## 10.7. Phase 11c: decide procedures

If your institution declares decide procedures in `decide_procedures`, programs can call them by qualified name (`cap:within_tolerance(input.delta, 0.1)`) and EigenQL queries can use them in `WHERE` clauses. The `decide` operation gets called at type-check time (Check capability mode) and/or runtime (IO mode).

Three-valued result:

| Result | Effect in `WHERE` / type-check |
|---|---|
| `Holds` | Predicate passes |
| `Fails` | Predicate rejects (filter out / type-check failure) |
| `Undecidable` | Defer to runtime (in check mode) or treat as `false` (in `WHERE`) |

Cross-link: [ESL chapter 9](../esl/09-institutions.md), [EigenQL chapter 8](../eigenql/08-institutions.md).

## 10.8. Phase 11d: comorphisms

If your institution declares comorphism types in `comorphism_types`, the `translate` operation gets called when programs invoke the comorphism by qualified name (`cap:dock_to_assay(docking_result)`).

Comorphisms take exactly one source resource and return one resource. The compiler enforces this arity rule.

## 10.9. The institution chapter cross-references

For the *user* perspective on what institutions look like from the surface languages:

- **[ESL chapter 9 — Institutions in ESL](../esl/09-institutions.md)** — invoking decide predicates and comorphisms from program bodies.
- **[EigenQL chapter 7 — FIBER clauses](../eigenql/07-fiber-clauses.md)** — invoking institution queries from EigenQL.
- **[EigenQL chapter 8 — Institutions in EigenQL](../eigenql/08-institutions.md)** — the classification table that ESL and EigenQL share.

For the *protocol* specification:

- **[D10 Grothendieck institution protocol](../../design/d10-grothendieck-institution-protocol.md)** — full design rationale.

---

Next: **[11. Deployment →](11-deployment.md)**
