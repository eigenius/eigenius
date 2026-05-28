# Eigenius — Coding Conventions

Mandatory reading before writing code. The conventions here are how the
codebase is actually written; new code must match.

For architectural shape and the design-first workflow, read
[`CLAUDE.md`](CLAUDE.md) and [`AGENTS.md`](AGENTS.md) first — they cover
*what* and *why*. This document covers *how*.

## Build, test, lint

Use the `/build`, `/test`, and `/lint` skills (under
[`.agents/skills/`](.agents/skills/)). Each points at the canonical CI
invocation:

- `/build` — `cargo build --workspace`
- `/test` — `cargo test --workspace`
- `/lint` — `cargo fmt --all -- --check` and
  `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`

`-D warnings` is the project's clippy policy: every clippy warning is an
error. The codebase does not maintain a permitted-warnings allowlist —
fix the code, do not silence the lint. If a lint genuinely doesn't apply
to a specific call site, use `#[allow(clippy::<name>)]` with a one-line
comment explaining why. Project-wide `#![allow(...)]` is not used.

## Workspace shape

The workspace at [`Cargo.toml`](Cargo.toml) declares the members.

- **[`kernel/`](kernel/)** — core library. Ontology validation, layer
  management, query, NbE, validator, capability dispatch, execution
  contexts, reflection, bootstrap. Library only; no binary.
- **[`crates/`](crates/)** — supporting libraries: `eigenius-config` for
  configuration, `runtime-substrate` for execution substrate,
  `wasm-runtime` for Wasmtime component integration, `eigenius-julia` and
  `eigenius-lean*` for institution bindings.
- **[`storage/`](storage/)** — pluggable backends: `memory`, `rocksdb`,
  `tikv`. Each implements the same kernel-side storage interface.
- **[`cli/`](cli/)** — the `eigenius` binary; gRPC client linked against
  kernel + default rocksdb backend + institution crates for in-process
  verification.
- **[`sdk/wasm-sdk/`](sdk/wasm-sdk/)** — language-agnostic CBOR codec for
  WASM component authoring. Compiles to `wasm32-unknown-unknown` and
  native.

Edition: **2021**, workspace-unified at `[workspace.package]`. There is no
declared MSRV — CI uses stable.

## Core data model

These are the load-bearing patterns. New code that touches the data model
must follow them.

### Everything is a Resource

There are no separate `Class`, `Property`, or `DataType` Rust types. The
`Resource` struct at [`kernel/src/ontology/resource.rs:174`](kernel/src/ontology/resource.rs#L174)
carries all three:

```rust
pub struct Resource {
    id: Option<Iri>,
    properties: BTreeMap<Iri, Value>,
}
```

A class definition and a property definition are both resources — the
difference is which properties they declare. The validator walks the
parent chain (see *Layers* below) to resolve a resource's type.

`Value` is an 8-variant enum (`String`, `Integer`, `Float`, `Boolean`,
`ResourceRef`, `Embedded`, `Array`, `Json`). Predicates like
`as_iri()`, `as_str()`, `as_integer()` extract typed views; use them
instead of pattern-matching the enum directly when reading a property.

**Property storage is `BTreeMap<Iri, Value>` — not `HashMap`.** The
rationale is in the code: deterministic ordering for canonical hashing,
plus cache-friendly sequential access. Use `BTreeMap` throughout the
codebase for any map keyed by data that participates in hashing or
serialisation. `HashMap` is acceptable only when keys are ephemeral and
ordering genuinely doesn't matter (e.g., a transient lookup table that
never gets serialised).

### IRI scheme

The `Iri` struct at [`kernel/src/ontology/iri.rs:27`](kernel/src/ontology/iri.rs#L27)
is the only legal way to construct an IRI. There is no free-form
construction — string concatenation does not become an IRI:

```rust
pub struct Iri(String);
impl Iri {
    pub fn parse(s: &str) -> Result<Self, IriError> { /* validates */ }
    pub fn namespace(&self) -> &str { ... }
    pub fn local_name(&self) -> &str { ... }
}
```

Scheme: `urn:eigenius:<namespace>:<local-name>`. The core ontology lives
under `urn:eigenius:core:*`. When you need a new IRI:

- Always go through `Iri::parse()`, never wrap a `String` directly.
- Validate at the boundary (deserialisation, FFI, user input). Once a
  value is an `Iri`, treat it as trusted internally.
- Property local names are **snake_case** (`institution_iri`,
  `wasm_binary_ref`, `memory_limit_pages`).
- Class local names are **PascalCase** (`Institution`, `Property`,
  `Class`).
- See [`kernel/src/ontology/well_known.rs`](kernel/src/ontology/well_known.rs)
  for constants and helpers for canonical core IRIs.

### Layer chain

Layers are immutable once built and form a DAG via `Arc<Layer>` parent
pointers. From [`kernel/src/layer/mod.rs:162`](kernel/src/layer/mod.rs#L162):

```rust
pub struct Layer {
    id: LayerId,
    content_hash: ContentHash,
    supporting_layer: Option<LayerId>,
    name: String,
    parents: Vec<Arc<Layer>>,
    defined_iris: BTreeSet<Iri>,
    /* ... */
}
```

Multiple parents are supported (per design `d33-partial-order-chains.md`
and the Phase-14e merge work). `parents.first()` is the canonical
single-parent walk for chain resolution; multi-parent walks must visit
every parent.

Construct via `LayerBuilder::build()` and wire chains together with
`build_chain()`. Never mutate a layer after construction — if you need a
new state, build a new layer with the modified one as a parent.

## Error handling

[`thiserror`](https://docs.rs/thiserror) is ubiquitous. The pattern,
seen across `kernel/src/runtime/boundary.rs`, `kernel/src/task/mod.rs`,
`crates/eigenius-config/src/loader.rs`, and `crates/runtime-substrate/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum FooError {
    #[error("could not read {path}: {source}")]
    Read { path: String, #[source] source: std::io::Error },

    #[error("invalid format")]
    InvalidFormat,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

Rules:

- **Each logical subsystem defines its own `pub enum *Error`** — boundary,
  task, loader, build, spawn, run, etc. Don't dump everything into a
  single project-wide error.
- **`#[error(...)]` messages are mandatory** on every variant. They must
  carry enough context to diagnose without a stack trace.
- **`#[from]` for common conversions** (especially `std::io::Error`).
  Don't repeat `.map_err(|e| FooError::Io(e))` everywhere.
- **Structured fields, not formatted strings.** A variant with
  `{ path: String, source: ... }` is better than a single `String` carrying
  a pre-formatted message — downstream code can inspect the fields.
- **`anyhow` is not used in library code.** It's acceptable in tests and
  the CLI's outermost error reporting, where dynamic dispatch over error
  types is genuinely the right tool. Library APIs return concrete error
  enums.
- **No project-level `Result<T>` alias.** Each crate uses
  `Result<T, ThatCrate'sError>` directly. There is no `prelude::Result`.

## Async

[`tokio`](https://docs.rs/tokio) is the async runtime, pinned in
`[workspace.dependencies]` with `features = ["full"]`. Async is the
default.

The following crates are intentionally sync — don't add a tokio dep to
them without a documented reason:

- `eigenius-config` — blocking config file I/O is fine
- `eigenius-julia` — pure sync API
- `eigenius-lean`, `eigenius-lean-runtime`, `eigenius-lean-worker` —
  Lean interop via tempfiles / FFI
- `wasm-runtime` — optional tokio behind the `docker-spawner` feature

**Naming convention for async functions: no `_async` suffix.** An async
function is named the same as its conceptual sync counterpart
(`fn connect_orchestrator(...) -> ...` is async if it's the only flavour
that exists). For trait methods, use `async-trait` rather than maintaining
parallel `*_async` variants.

## Test organisation

Two layouts coexist and both are correct:

- **Inline `#[cfg(test)] mod tests { ... }`** at the bottom of the module
  being tested. Used for ~64 modules in `kernel/src` alone. Right for
  unit tests that exercise private functions or want close coupling to
  the implementation.
- **Separate `tests/` directory** (e.g. `kernel/tests/`,
  `crates/*/tests/`). Right for integration tests that exercise the
  crate's *public* API and don't need access to internals.

[`proptest`](https://docs.rs/proptest) (v1) is in
`[workspace.dependencies]` and is used in kernel property tests. Use it
for invariants over a range of inputs; capture the seed when reproducing
locally (`CARGO_PROPTEST_SEED=...`).

[`criterion`](https://docs.rs/criterion) (v0.5, with HTML reports) is
available for benchmarks. Don't add benchmarks for hot paths reflexively
— add them only when there's a concrete performance question to answer.

Both `arrange / act / assert` and direct linear test bodies are
acceptable. Prefer the form that reads clearest for the specific test.
Don't introduce a test-helper abstraction for a pattern that appears
only twice.

## Documentation in code

### Module-level doc comments are mandatory for public modules

Every `mod.rs` (and every top-level `<module>.rs` that exposes a public
API) starts with a `//!` block describing the module's role. Pattern,
from [`kernel/src/layer/mod.rs`](kernel/src/layer/mod.rs):

```rust
//! Layer system for stratified ontology composition.
//!
//! Layers hold resources and form a chain via parent pointers.
//! ...
```

The doc should answer: *what does this module own, what is the entry
point, what design doc covers it.*

### Function docs on public APIs

Public functions, structs, methods, and traits carry `///` doc comments.
Documentation explains:

- What the function does (one sentence).
- Parameter meanings (when not obvious from the name).
- Return semantics (especially for `Option`, `Result`, or sentinel
  values).
- Error conditions for functions returning `Result`.

Private items don't need doc comments unless the *why* is non-obvious.
Default to no comment when the function name says it all.

### Design doc references in comments

When code implements a specific design decision, reference the design
doc inline. Two patterns coexist:

- Short form: `// D13 §4.2` (design doc d13, section 4.2). Most common.
- Long form: `// See docs/design/d24-schema-versioning.md §6.1`. Use
  when the short form would be ambiguous.

This convention makes it trivial for a reader (or an agent) to find the
*why* without grepping. Maintain it for any code that materialises a
design decision from `docs/design/`.

## Module layout

The pattern that has emerged across kernel sub-systems: **`mod.rs`
holds the public API; submodules are concern-scoped leaves**.

Example: [`kernel/src/ontology/`](kernel/src/ontology/):

- `mod.rs` — public re-exports (`pub use resource::{Resource, Value}`,
  `pub use iri::Iri`).
- `resource.rs` — `Resource` struct, `Value` enum, predicates.
- `iri.rs` — `Iri` struct, `IriError`, parsing/validation.
- `eigon_json.rs` — JSON serialisation/deserialisation for Eigon.
- `eigon_cbor.rs` — CBOR serialisation, including the CBOR tag 27182
  used to distinguish JSON from embedded resources.
- `well_known.rs` — constants for core IRIs.

For larger sub-systems ([`kernel/src/layer/`](kernel/src/layer/) is the
fully-worked example), `mod.rs` still owns the public API and submodules
fan out by concern (`bloom.rs`, `cache.rs`, `consolidate.rs`,
`handle.rs`, `index.rs`, `merge.rs`, `redirect.rs`, `storage.rs`,
`supporting.rs`).

Don't move public types into submodules and re-export them from `mod.rs`
just to spread file sizes. Move types into a submodule when they have
real internal complexity that benefits from isolation.

## Naming

| Pattern | When to use | Example |
| --- | --- | --- |
| `<Thing>Builder` | Multi-step construction with non-trivial validation | `LayerBuilder`, `BuildahImageBuilder` |
| `<Thing>Config` | Immutable configuration struct, often loaded from env/file | `SubstrateConfig`, `DockerSpawnerConfig`, `GcConfig` |
| `<Thing>Handle` | Cheap-to-clone reference to a heavyweight resource | `LayerHandle` |
| `<Thing>Error` | Each subsystem's error enum | `BoundaryError`, `BuildError`, `LoaderError` |
| `from_*` | Conversion **into** `Self` from another type | `from_bytes`, `from_cbor`, `from_handle` |
| `to_*` | Conversion **from** `&self` to a new owned value | `to_json`, `to_cbor`, `to_f64` |
| `into_*` | Conversion **consuming** `self` (rare in this codebase) | `into_server` |
| `new` | Direct constructor | `Resource::new(iri)`, `Resource::new_embedded()` |
| `set_*` (on a builder) | Set a field during builder chaining | `LayerBuilder::set_name(...)` |

Functions named `parse` (taking `&str`) are typed-constructor flavours of
`from_str`; they return a custom error rather than implementing
`FromStr`. See `Iri::parse` for the pattern.

`with_*` is rarely used in this codebase — builders use `.set_*().build()`
rather than `.with_*().build()`. Don't introduce `.with_*` chains for
new builders.

## Misc

- `Arc<T>` is the default for shared, immutable, reference-counted state
  (layers, ontology fragments). `Rc<T>` is rare; use it only when single-
  threaded restriction is intentional.
- Avoid `unsafe` outside of FFI shims (`crates/eigenius-lean-worker/src/`,
  `crates/wasm-runtime/`, anything bridging to a C ABI). If you need
  `unsafe` in pure-Rust code, that's a sign to revisit the design.
- Don't add backwards-compatibility code paths for shapes you've decided
  to retire. Per `AGENTS.md`, if you know the shape is wrong, fix it now
  — don't ship a bridge.
- Avoid premature abstraction. Three similar lines is fine; reach for a
  helper at five or more, and only when the helper genuinely clarifies.
- Prefer `BTreeSet` over `HashSet` for the same reasons `BTreeMap` is
  preferred over `HashMap`.

## What this file does not cover

- **Architecture and design** — see [`docs/design/`](docs/design/).
- **Workflow ordering** (when to draft a design doc first, when to
  escalate) — see [`AGENTS.md`](AGENTS.md).
- **Build commands and toolchain expectations** — see [`CLAUDE.md`](CLAUDE.md)
  and the skills at [`.agents/skills/`](.agents/skills/).

If a convention here conflicts with current code in a load-bearing way,
the *code* may be right and this document may be stale. Flag the
discrepancy, propose a fix or a doc update, and resolve it before
adding more code that follows the wrong convention.
