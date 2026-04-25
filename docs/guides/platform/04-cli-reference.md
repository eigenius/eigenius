# 4. CLI reference

The `eigenius` CLI is the primary developer interface. The binary lives at [`cli/`](../../../cli/) and ships as one of the workspace's outputs (`target/debug/eigenius` after `cargo build`).

```bash
eigenius [--json] [--endpoint URL] <subcommand> [args...]
```

Two **global flags**:

| Flag | Effect |
|---|---|
| `--json` | Emit machine-readable JSON instead of human-formatted output |
| `--endpoint URL` | Connect to a remote kernel via gRPC instead of running in process |

In-process commands operate against an in-memory layer chain bootstrapped from the embedded core ontologies. Remote commands (`--endpoint http://localhost:50051`) talk to a running `eigenius serve` instance and operate against its persistent or in-memory state.

The full source of truth for command shapes is the `Commands` enum in [`cli/src/main.rs`](../../../cli/src/main.rs) (line 27).

## 4.1. File commands (in-process)

These commands operate on local files without needing a running kernel.

### `validate <FILE>`

Validate an Eigon-JSON or ESL file against the bootstrapped core ontology stack.

```bash
eigenius validate ontologies/examples/animals.json
eigenius validate demo/document.esl
```

ESL files (extension `.esl`) are compiled to Eigon-JSON in memory before validation. The validator runs all 12 ontology rules ([D1](../../design/d1-eigon-serialization-format.md)) and reports failures with rule names and resource IRIs.

### `compile <FILE>`

Compile an ESL file to Eigon-JSON, write to stdout.

```bash
eigenius compile demo/document.esl > demo/document.json
```

Pure surface-language transformation — no validation, no layer load.

### `inspect <IRI> [--at-layer <LAYER_ID>]`

Print a resource by IRI. Resolves through the in-process layer chain (or through a remote kernel's chain when combined with `--endpoint`).

```bash
eigenius inspect "urn:eigenius:core:Class"
eigenius --endpoint http://localhost:50051 inspect "urn:example:Dog"
```

`--at-layer` (remote mode only) resolves at a specific historical layer rather than the current top — useful for reaching a forked task result layer (D21 §3.6).

## 4.2. Knowledge-graph commands

Read or modify the layer chain. In-process operations get a fresh in-memory chain each invocation; remote operations work against the running kernel's persistent state.

### `load <FILE>`

Load an Eigon-JSON or ESL file as a new layer on top of the current chain. Validates first; rejects on validation failure.

```bash
eigenius --endpoint http://localhost:50051 load demo/document.json
eigenius --endpoint http://localhost:50051 load demo/document.esl
```

In-process `load` is mostly useful with `--json` for scripting; the new layer is in-memory and discarded when the command exits.

### `query <EIGENQL> [--file <PATH>] [--at-layer <LAYER_ID>]`

Execute an EigenQL query.

```bash
# Against the in-process bootstrap
eigenius query 'USING "urn:eigenius:core:Class" MATCH Class(?c) { short_name: ?n } RETURN [] { name: ?n }'

# Load a file first (in-process)
eigenius query --file ontologies/examples/animals.json \
    'MATCH "urn:eigenius:example:Dog"(?d) { "urn:eigenius:example:name": ?name } RETURN [] { name: ?name }'

# Against a running kernel
eigenius --endpoint http://localhost:50051 query \
    'MATCH "urn:eigenius:core:Class"(?c) { short_name: ?n } RETURN [] { name: ?n }'
```

`--at-layer` (remote mode only) targets a specific historical layer.

EigenQL syntax: see the [EigenQL guide](../eigenql/README.md).

## 4.3. Program commands

### `program-validate <PROGRAM_FILE> [--ontology <FILE>]` (in-process)

Type-check a program. The optional `--ontology` loads supporting class/property declarations before checking.

```bash
eigenius program-validate ontologies/examples/simple-program.json \
    --ontology ontologies/examples/animals.json
```

### `run <PROGRAM_FILE> <INPUT_FILE>` (requires `--endpoint`)

Execute a program against an input. Requires a running kernel because programs may dispatch IO components to the orchestrator.

```bash
eigenius --endpoint http://localhost:50051 run \
    demo/summarize-program.json demo/input.json

eigenius --endpoint http://localhost:50051 run \
    demo/summarize.esl demo/input.json
```

Both program and input may be Eigon-JSON or ESL — auto-detected by extension.

## 4.4. The server command

### `serve [--port <N>] [--orchestrator <URL>] [--db <PATH>]`

Start the gRPC server.

```bash
# In-memory, no orchestrator (file ops + queries only)
eigenius serve

# In-memory + orchestrator dispatch
eigenius serve --orchestrator http://localhost:8080

# Persistent + orchestrator dispatch
eigenius serve --db /var/lib/eigenius --orchestrator http://localhost:8080

# Custom port
eigenius serve --port 9000
```

Default port: 50051. The orchestrator URL can also come from the `EIGENIUS_ORCHESTRATOR_ENDPOINT` env var; the database path from `EIGENIUS_DB`.

| Flag | Default | Env var |
|---|---|---|
| `--port` | 50051 | — |
| `--orchestrator` | none | `EIGENIUS_ORCHESTRATOR_ENDPOINT` |
| `--db` | in-memory | `EIGENIUS_DB` |

When `--db <path>` is provided, the kernel persists layers, traces, and WASM capabilities to RocksDB and survives restart. See [chapter 6](06-database-management.md).

## 4.5. Database commands

Operate directly on a RocksDB database directory; the kernel server should be **stopped** for `compact` and `export` (RocksDB's lock file blocks concurrent processes).

### `db stats <PATH>`

Print storage statistics for the database.

```bash
eigenius db stats /var/lib/eigenius
```

Reports per-column-family statistics: live data size, number of keys, level distribution, etc.

### `db compact <PATH>`

Trigger a manual full compaction. Useful after large deletes or to defragment after extensive trace generation.

```bash
eigenius db compact /var/lib/eigenius
```

### `db export <DB_PATH> <OUTPUT_PATH>`

Dump every resource in the database as Eigon-JSON files into a directory.

```bash
eigenius db export /var/lib/eigenius /tmp/eigenius-export
```

Useful for backup snapshots and for migrating between RocksDB versions. The output is round-trippable: `eigenius load` over the exported files reconstructs an equivalent layer set.

## 4.6. Capability commands

WASM components and institutions are managed through the `capability` subcommand. All require `--endpoint`.

### `capability list`

```bash
eigenius --endpoint http://localhost:50051 capability list
```

List every registered component and institution with kind and capability level.

### `capability inspect <IRI>`

```bash
eigenius --endpoint http://localhost:50051 capability inspect \
    urn:example:components:DocValidator
```

Print details for a registered capability: input/output types (components), declared morphism/query/comorphism types (institutions), capability level, fuel limits.

### `capability install <WASM_FILE> [...]`

Install a WASM binary into the running kernel.

**Quick mode** — pass the IRI and metadata as flags:

```bash
eigenius --endpoint http://localhost:50051 capability install \
    examples/wasm-doc-validator/target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm \
    --as-iri urn:example:components:DocValidator \
    --kind component \
    --capability pure \
    --input-type urn:example:doc:Document \
    --output-type urn:example:doc:ValidationResult
```

**Full mode** — provide a definition file (Eigon-JSON or ESL) declaring the capability resource; the CLI fills in `wasm_binary` and `implementation: "wasm"`:

```bash
eigenius --endpoint http://localhost:50051 capability install \
    my-component.wasm \
    --definition my-component-definition.esl
```

Flags:

| Flag | Default | Use |
|---|---|---|
| `--definition <FILE>` | — | Full mode — capability resource declaration |
| `--as-iri <IRI>` | — | Quick mode — capability IRI (mutually exclusive with `--definition`) |
| `--kind <KIND>` | `component` | Quick mode — `component` or `institution` |
| `--capability <LEVEL>` | `pure` | Quick mode — `pure`, `read`, or `io` |
| `--input-type <IRI>` | — | Quick mode — components only |
| `--output-type <IRI>` | — | Quick mode — components only |

### `capability test <IRI> --input <FILE> [--mode query|discover]`

Invoke a registered capability with test input.

```bash
eigenius --endpoint http://localhost:50051 capability test \
    urn:example:components:DocValidator \
    --input /tmp/doc.json
```

For institutions, `--mode query` (default) dispatches a fiber query; `--mode discover` dispatches `discover-morphisms`.

## 4.7. Task commands (require `--endpoint`)

Inspect and control persisted tasks (D21).

### `tasks list`

```bash
eigenius --endpoint http://localhost:50051 tasks list
```

List every task in the session with status (`Running`, `Completed`, `Failed`, `Cancelled`).

### `tasks status <TASK_ID>`

```bash
eigenius --endpoint http://localhost:50051 tasks status <uuid>
```

Detailed status: program IRI, input layer IDs, current checkpoint, elapsed time, last event.

### `tasks cancel <TASK_ID>`

```bash
eigenius --endpoint http://localhost:50051 tasks cancel <uuid>
```

Request cooperative cancellation. The task transitions to `Cancelled` at its next checkpoint.

## 4.8. Other commands

### `list-institutions` (requires `--endpoint`)

```bash
eigenius --endpoint http://localhost:50051 list-institutions
```

List registered institutions, their declared morphism types, query types, and IRIs.

### `get-schema <CLASS_IRI>` (requires `--endpoint`)

```bash
eigenius --endpoint http://localhost:50051 get-schema "urn:example:Document"
```

Generate JSON Schema for an ontology class. Used internally by the `CompleteJson` LLM component to constrain structured outputs.

### `reflect <FILE>`

```bash
eigenius reflect path/to/trace.json
```

Record a reasoning trace from a JSON or ESL file. Used during testing of the trace-recording machinery.

### `version`

```bash
eigenius version
```

Print the build version and metadata.

## 4.9. Output formatting

The global `--json` flag switches output from human-formatted prose to a machine-readable JSON envelope, suitable for piping into `jq` or scripting:

```bash
eigenius --json query 'MATCH ?x {} RETURN [] { x: ?x }' | jq '.results[0]'
```

Without `--json`, output is colourised plain text intended for terminal display.

## 4.10. Exit codes

The CLI uses standard exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Error (CLI-level, e.g. unknown subcommand) |
| 2 | Validation failure |
| 3 | Type-check failure |
| 4 | Runtime / dispatch failure |
| 5 | Connection failure (remote mode) |

In CI scripts, check the exit code to distinguish success from each failure mode.

---

Next: **[5. Running the platform locally →](05-running-locally.md)**
