# wasm-ordering-institution

A WASM **fiber reasoner** (institution) that provides domain-specific
reasoning over refinement morphisms. Implements the
[`eigenius-institution`](../../wit/eigenius-component.wit) WIT world and
runs inside the Eigenius kernel via `WasmFiberReasoner`.

This mirrors the kernel's built-in test `OrderingInstitution` (in
[`kernel/src/institution/mod.rs`](../../kernel/src/institution/mod.rs))
but hosted as an external WASM binary rather than a Rust trait object.
It's a good reference for building your own domain institution.

## What it does

Declares a fiber with:

- **One morphism class** — `Refinement`: relates a `source` resource to a
  `target` resource with a `delta` property measuring the refinement
  magnitude (e.g., a convergence delta, error reduction, etc.)
- **One query class** — `ConvergenceQuery`: takes a `tolerance` and a
  `latest_delta` parameter and returns whether the refinement has settled
  below the tolerance

Provides the four fiber reasoner behaviors:

| Method                | Behavior                                                                          |
|-----------------------|-----------------------------------------------------------------------------------|
| `fiber-declaration`   | returns the institution IRI, name, morphism types (with required properties), query types (with required parameters) |
| `query`               | reads `tolerance` and `latest_delta` from the query resource, returns `converged: true` iff `|latest_delta| ≤ tolerance`, echoes the checked values |
| `validate-morphism`   | returns `valid` iff `delta > 0`; `invalid` with reason otherwise                  |
| `discover-morphisms`  | returns an empty list (this institution doesn't discover morphisms)               |

The validation logic mirrors the built-in test institution: a refinement
is only valid if it strictly improves the underlying objective. Zero or
negative delta values are rejected.

### Parameterized queries

`ConvergenceQuery` demonstrates the general shape of a parameterized
institution query. Its parameters live as ordinary properties on the query
resource alongside the `is_a`:

| Property                                  | Type      | Role                                |
|-------------------------------------------|-----------|-------------------------------------|
| `urn:eigenius:core:is_a`                  | [IRI]     | `[urn:eigenius:test:wasm:ConvergenceQuery]` |
| `urn:eigenius:test:wasm:tolerance`        | number    | threshold; non-negative             |
| `urn:eigenius:test:wasm:latest_delta`     | number    | most recent refinement's delta      |

Both parameters are declared as `urn:eigenius:core:requires` on the query
class in `fiber-declaration`, which means the kernel validates them
structurally *before* the query reaches the institution. If a caller
forgets a parameter, the kernel rejects the query at the load/validate
stage — the guest doesn't need to defensively check for missing
properties (though this example does, as a safety belt).

The result is an embedded resource:

| Property                                  | Type     | Meaning                                     |
|-------------------------------------------|----------|---------------------------------------------|
| `urn:eigenius:test:wasm:converged`        | boolean  | `|latest_delta| ≤ tolerance`                |
| `urn:eigenius:test:wasm:checked_delta`    | number   | echoes `latest_delta` — lets callers audit  |
| `urn:eigenius:test:wasm:checked_tolerance`| number   | echoes `tolerance`                          |

## Key SDK patterns shown

- **Full institution lifecycle** — all four FiberReasoner methods
  implemented end-to-end
- **`FiberDeclaration` construction** via the
  [`institution` submodule](../../sdk/wasm-sdk/src/institution.rs),
  including building `Resource` objects for each morphism class and
  query class with their required-property metadata
- **Using the WIT `validation-result` enum** — the return type for
  `validate-morphism` is `result<tuple<validation-result, string>,
  string>`, so guest code returns `(ValidationResult::Valid, "")` or
  `(ValidationResult::Invalid, reason)` depending on the check
- **Matching on CBOR-decoded primitive values** — the `delta` property
  may arrive as either `Value::Float` or `Value::Integer` depending on
  how the caller serialized it; the guest handles both

## Building

```bash
cd examples/wasm-ordering-institution
cargo component build
```

Output: `target/wasm32-unknown-unknown/debug/eigenius_wasm_ordering_institution.wasm`

## Installing

From a running Eigenius kernel (`cargo run -p eigenius-cli -- serve`):

```bash
eigenius --endpoint http://localhost:50051 capability install \
    examples/wasm-ordering-institution/target/wasm32-unknown-unknown/debug/eigenius_wasm_ordering_institution.wasm \
    --as-iri urn:eigenius:test:wasm:ordering \
    --kind institution
```

**Note on IRIs:** The WASM binary declares its institution IRI in
`fiber-declaration` (here, `urn:eigenius:test:wasm:ordering`). The CLI's
`--as-iri` flag sets the ontology resource's `@id`, which should match
the binary-declared IRI. If they differ, the kernel prints a warning and
uses the binary's IRI as authoritative.

Verify the install:

```bash
eigenius --endpoint http://localhost:50051 capability list
```

You should see the institution appear:

```
Institutions:
  ordering (urn:eigenius:test:wasm:ordering)
```

## Testing

### Quick end-to-end run

All of the steps below — install, list, two convergence queries, inspect,
and an EigenQL `FIBER` query (#10) that dispatches into the institution —
are scripted in [`run.sh`](run.sh). Build the release CLI once and execute:

```bash
cargo build --release -p eigenius-cli
./examples/wasm-ordering-institution/run.sh
```

The script starts a kernel on port 50099 (override with `PORT=<port>`),
tears it down on exit, and prints each step's output. Use this as a
smoke test after kernel or SDK changes. The final step loads a couple of
`Refinement` instances plus the Property definitions the type checker
needs, then runs:

```eigenql
USING INSTITUTION "urn:eigenius:test:wasm:ordering" AS ord
USING "urn:eigenius:test:wasm:Refinement"
MATCH Refinement(?m) { delta: ?d }
FIBER ord:ConvergenceQuery { tolerance: 0.01, latest_delta: ?d } AS ?conv
MATCH ?conv { "urn:eigenius:test:wasm:converged": ?c }
WHERE ?c = true
RETURN [] { refinement: ?m, delta: ?d }
```

which asks the institution "converged?" once per `Refinement` and keeps
only the bindings it reports as converged. See D2 Appendix B for the
FIBER clause semantics.

### Durability smoke (Phase 9a)

[`run_durable.sh`](run_durable.sh) is the CLI-surface counterpart to the
in-process test at `storage/rocksdb/tests/durability_test.rs`. It starts
the kernel with `--db <tempdir>`, installs the institution, kills the
kernel, restarts it against the same DB, and verifies the institution
still dispatches — no re-install. Use it after changes to the SEED /
RESUME paths. See D13.

```bash
cargo build -p eigenius-cli
./examples/wasm-ordering-institution/run_durable.sh
```

### Converged case

Save a convergence query where the latest delta is well below the
tolerance:

```bash
cat > /tmp/conv-query.json <<'EOF'
{
  "@id": "urn:example:q1",
  "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:ConvergenceQuery"],
  "urn:eigenius:test:wasm:tolerance": 0.01,
  "urn:eigenius:test:wasm:latest_delta": 0.001
}
EOF
```

Dispatch:

```bash
eigenius --endpoint http://localhost:50051 capability test \
    urn:eigenius:test:wasm:ordering \
    --input /tmp/conv-query.json
```

Expected output:

```json
{
  "urn:eigenius:test:wasm:checked_delta": 0.001,
  "urn:eigenius:test:wasm:checked_tolerance": 0.01,
  "urn:eigenius:test:wasm:converged": true
}
```

### Not-yet-converged case

Raise the latest delta above the tolerance:

```bash
cat > /tmp/not-converged.json <<'EOF'
{
  "@id": "urn:example:q2",
  "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:ConvergenceQuery"],
  "urn:eigenius:test:wasm:tolerance": 0.01,
  "urn:eigenius:test:wasm:latest_delta": 0.5
}
EOF

eigenius --endpoint http://localhost:50051 capability test \
    urn:eigenius:test:wasm:ordering \
    --input /tmp/not-converged.json
```

Returns `"urn:eigenius:test:wasm:converged": false`.

### Missing required parameter

Omit `latest_delta`:

```bash
cat > /tmp/missing-param.json <<'EOF'
{
  "@id": "urn:example:q3",
  "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:ConvergenceQuery"],
  "urn:eigenius:test:wasm:tolerance": 0.01
}
EOF

eigenius --endpoint http://localhost:50051 capability test \
    urn:eigenius:test:wasm:ordering \
    --input /tmp/missing-param.json
```

The institution rejects the query because the required parameter is
missing:

```
Fiber query failed: computation failed: missing or non-numeric 'urn:eigenius:test:wasm:latest_delta' parameter
```

### Unknown query type

```bash
cat > /tmp/bad-query.json <<'EOF'
{
  "@id": "urn:example:bad",
  "urn:eigenius:core:is_a": ["urn:example:UnknownQuery"]
}
EOF

eigenius --endpoint http://localhost:50051 capability test \
    urn:eigenius:test:wasm:ordering \
    --input /tmp/bad-query.json
```

The institution rejects queries whose class it doesn't recognize:

```
Fiber query failed: computation failed: unknown query type: ["urn:example:UnknownQuery"]
```

### Morphism validation

Morphism validation happens automatically when you `load` resources of a
registered morphism class — no CLI command is needed. If you load a
`Refinement` morphism with a non-positive `delta`, the load is rejected
with a validation error from the institution.

## Source walkthrough

See [`src/lib.rs`](src/lib.rs).

- **`fiber_declaration`** builds two class resources (`Refinement` and
  `ConvergenceQuery`) via the SDK's `Resource::with_id` / `set_is_a` /
  `set` builders. Both carry `core:requires` listing their required
  properties — the kernel validates these structurally before a query
  or morphism reaches the institution. The two classes are wrapped in
  a [`FiberDeclaration`](../../sdk/wasm-sdk/src/institution.rs) which
  serializes to the CBOR-encoded resource shape the kernel expects.
- **`query`** decodes the input, checks its `is_a` for
  `ConvergenceQuery`, then pulls the typed parameters (`tolerance`,
  `latest_delta`) using the shared `extract_number` helper. The helper
  accepts both `Float` and `Integer` values since CBOR may round-trip
  numeric literals either way. The result resource echoes the checked
  inputs alongside the boolean — useful for auditability.
- **`validate_morphism`** reads the `delta` property (via the same
  helper) and returns `(ValidationResult::Valid, "")` or
  `(ValidationResult::Invalid, reason)`.
- **`discover_morphisms`** returns an empty list — this institution
  doesn't infer new morphisms.

## Related

- [examples/README.md](../README.md) — top-level examples overview
- [docs/design/d10-grothendieck-institution-protocol.md](../../docs/design/d10-grothendieck-institution-protocol.md) —
  the institution protocol design
- [docs/design/d12-wasm-extensibility.md](../../docs/design/d12-wasm-extensibility.md) —
  the WASM hosting architecture
- [sdk/wasm-sdk/src/institution.rs](../../sdk/wasm-sdk/src/institution.rs) —
  the institution SDK helpers
- Kernel integration tests for this institution live in
  [kernel/src/capability/tests.rs](../../kernel/src/capability/tests.rs)
  (`wasm_institution_*` tests)
