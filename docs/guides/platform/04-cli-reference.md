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

Surface-language transformation — no validation, and nothing is committed. It does bootstrap a layer first, so that constructor short names resolve through the chain's ctor table (`collect_ctors_from_layer`): a file citing `reasoning:JustifiedBy`'s constructors compiles here rather than only inside a running server. Seeding the bootstrap layer only *adds* resolvable names, so it cannot make a previously-compiling file fail.

<a id="decompile-file---verify---pretty"></a>
### `decompile <FILE> [--verify] [--pretty]`

Print an Eigon-JSON document back as ESL source — the inverse of `compile`. Every D47 term value (`reasoning:proposition`, `reasoning:certificate`, `eigentt:axiom_statement`, `reflection:canonical_proposition`, …) is rendered in the [`type_expr(...)`](../esl/05-expressions.md#5-14a-type_expr-eigentt-type-expressions) sublanguage.

```bash
eigenius decompile chain/sentence.json
eigenius decompile chain/sentence.json --verify --pretty
```

- `--verify` re-compiles the printed source and checks that every term is alpha-equal to the one in the input, under the same canonicalisation the witness index uses. A mismatch prints the offending `@id :: property` pairs and exits non-zero, rather than emitting source that would commit a different object. Like `compile`, verification runs against a bootstrapped layer so ctor short names resolve.
- `--pretty` indents expression trees across lines; the default emits each term on one line. Layout is the only difference — the terms are identical either way.

This is what keeps chain content inside the reach of the source language: a resource the kernel or an institution *generated* can be read back as ESL, and machine-minted IRIs must therefore have local names that are legal ESL identifiers (`…:assertion_trace`, not `…:assertion-trace`). A ctor with no ESL surface is refused rather than printed approximately.

### `inspect <IRI> [--at-layer <LAYER_ID>] [--branch <NAME>]`

Print a resource by IRI. Resolves through the in-process layer chain (or through a remote kernel's chain when combined with `--endpoint`).

```bash
eigenius inspect "urn:eigenius:core:Class"
eigenius --endpoint http://localhost:50051 inspect "urn:example:Dog"

# Pin to a feature branch's current head
eigenius --endpoint http://localhost:50051 inspect "urn:example:Dog" --branch feature-x
```

`--at-layer` (remote mode only) resolves at a specific historical layer rather than the current top — useful for reaching a forked task result layer (D21 §3.6).

`--branch` (remote mode only) pins reads to the named branch's current head. Mutually exclusive with `--at-layer`. Empty / omitted defaults to `main`.

## 4.2. Knowledge-graph commands

Read or modify the layer chain. In-process operations get a fresh in-memory chain each invocation; remote operations work against the running kernel's persistent state.

### `load <FILE> [--branch <NAME>] [--commit-policy <reject|cascade>] [--max-violations <N>] [--explicit-tombstone <IRI>]...`

Load an Eigon-JSON or ESL file as a new layer on top of the current chain. Validates first; rejects on validation failure unless cascade is requested.

```bash
eigenius --endpoint http://localhost:50051 load demo/document.json
eigenius --endpoint http://localhost:50051 load demo/document.esl

# Commit to a named branch instead of main
eigenius --endpoint http://localhost:50051 load demo/document.esl --branch feature-x

# Cascade-tombstone lower-layer resources that the new layer's
# class redefinitions retroactively invalidate (D41 §3.3)
eigenius --endpoint http://localhost:50051 load demo/redef.json --commit-policy cascade

# Tombstone specific IRIs alongside the commit (D41 §10.1)
eigenius --endpoint http://localhost:50051 load demo/marker.json \
    --explicit-tombstone urn:eigenius:demo:to-suppress \
    --explicit-tombstone urn:eigenius:demo:also-suppress
```

In-process `load` is mostly useful with `--json` for scripting; the new layer is in-memory and discarded when the command exits.

`--branch` (remote mode only) commits the new layer to the named branch. Empty / omitted defaults to `main`. The branch must already exist — create it with `eigenius branch create` first.

`--commit-policy reject` (default) fails the commit on any retroactive validation violation; up to `--max-violations` (default 100) errors are surfaced, with the full count in the JSON response's `total_violations` field. `--commit-policy cascade` tombstones violating lower-layer IRIs iteratively to fixpoint; the cascade aborts if it would have to tombstone an IRI the new layer itself defines.

`--explicit-tombstone <IRI>` (repeatable) tombstones the given IRI as part of the same commit. Applied to the user-layer builder before retroactive validation; under `--commit-policy cascade` they combine with cascade-inferred tombstones. See D41 §10.1.

JSON output (`--json`) carries `success`, `layer_id`, `resource_count`, `branch_advanced`, `cascade_tombstones` (count), `cascade_iterations`, and `total_violations`.

### `query <EIGENQL> [--file <PATH>] [--at-layer <LAYER_ID>] [--branch <NAME>]`

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

# Query against a feature branch's current head
eigenius --endpoint http://localhost:50051 query \
    'MATCH "urn:eigenius:core:Class"(?c) { short_name: ?n } RETURN [] { name: ?n }' \
    --branch feature-x
```

`--at-layer` (remote mode only) targets a specific historical layer. `--branch` (remote mode only) pins the read to the named branch's current head; mutually exclusive with `--at-layer`. Empty / omitted defaults to `main`.

When `--file` is supplied, the load step accepts the same `--commit-policy`, `--max-violations`, and `--explicit-tombstone` flags as `load`. They're ignored when `--file` is omitted.

EigenQL syntax: see the [EigenQL guide](../eigenql/README.md).

## 4.3. Program commands

### `program-validate <PROGRAM_FILE> [--ontology <FILE>]` (in-process)

Type-check a program. The optional `--ontology` loads supporting class/property declarations before checking.

```bash
eigenius program-validate ontologies/examples/simple-program.json \
    --ontology ontologies/examples/animals.json
```

### `run <PROGRAM_FILE> <INPUT_FILE> [--branch <NAME>]` (requires `--endpoint`)

Execute a program against an input. Requires a running kernel because programs may dispatch IO components to the orchestrator.

```bash
eigenius --endpoint http://localhost:50051 run \
    demo/summarize-program.json demo/input.json

eigenius --endpoint http://localhost:50051 run \
    demo/summarize.esl demo/input.json

# Commit the trace layer to a feature branch
eigenius --endpoint http://localhost:50051 run \
    demo/summarize.esl demo/input.json --branch feature-x
```

Both program and input may be Eigon-JSON or ESL — auto-detected by extension.

`--branch` chooses the branch the trace layer commits into. Empty / omitted defaults to `main`.

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

When `--db <path>` is provided, the kernel persists layers, traces, and institution registrations to RocksDB and survives restart. See [chapter 6](06-database-management.md).

## 4.5. Database commands

Operate directly on a RocksDB database directory; the kernel server should be **stopped** for `compact` and `export` (RocksDB's lock file blocks concurrent processes).

### `db stats <PATH>`

Print storage statistics for the database.

```bash
eigenius db stats /var/lib/eigenius
```

Reports storage statistics: total keys, total bytes, plus a list of every branch ref with its current head.

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

## 4.6. Branch commands (require `--endpoint`)

Branches are named pointers into the layer DAG (D23 §5.5). Every commit lands on a branch — `main` is the default for any `load` / `run` / `reflect` that omits `--branch`. Feature branches let you stage divergent work without touching `main`; trivial-merge auto-reconciles disjoint changes (D23 §5.4).

All branch commands require `--endpoint` — branches require a persistent backend, which only the running kernel exposes.

### `branch list`

```bash
eigenius --endpoint http://localhost:50051 branch list
```

Print every branch ref with its current head:

```
NAME                              HEAD
feature-x                         abe85ea7d9b7f2bc4a32...
main                              5b2d014a3c8e9f1d2b88...
```

### `branch show <NAME>`

```bash
eigenius --endpoint http://localhost:50051 branch show main
```

Show a single branch's current head. Exits non-zero if the branch doesn't exist.

### `branch create <NAME> --from <LAYER_ID>`

Create a new branch pointing at an existing layer. Branch names match `[A-Za-z0-9_-]+` (max 256 chars). Fails if a branch with the same name already exists or if the `from_layer` is unknown.

```bash
# Branch off main's current head
MAIN_HEAD=$(eigenius --endpoint http://localhost:50051 branch show main --json | jq -r .head_layer)
eigenius --endpoint http://localhost:50051 branch create feature-x --from "$MAIN_HEAD"
```

After creation, `eigenius load --branch feature-x ...` commits onto the new branch.

### `branch delete <NAME> [--force]`

Remove a branch ref. Layers reachable only through this branch are reclaimed by the next GC pass; the ref itself is gone immediately.

```bash
eigenius --endpoint http://localhost:50051 branch delete feature-x
```

By default, the kernel refuses to prune a branch whose head matches an active task pin (a running task pinned its `layer_head` here). Pass `--force` to delete unconditionally — task pins outlive the branch ref via the GC root system, so data isn't lost; only the branch label disappears.

```bash
# Force-delete even if a task is pinned to this branch's head
eigenius --endpoint http://localhost:50051 branch delete feature-x --force
```

## 4.7. Mirror commands (require `--endpoint`)

The `mirror` subcommand group operates on `RuntimePackageMirror` resources — auto-generated, language-specific source code that mirrors a slice of the chain into typed structs that a substrate-hosted worker can decode and dispatch on. Used as the first step of the substrate-institution install flow ([chapter 11](11-runtime-substrate.md)).

### `mirror create [--filter <EIGENQL> | --filter-file <FILE>] [--institution-file <FILE>] --layer <IRI> --language <LANG> --output <DIR>`

Generate a mirror against a layer; commit a `RuntimePackageMirror` resource and write the source files locally.

```bash
eigenius --endpoint http://localhost:50051 mirror create \
    --layer "$(eigenius branch show main | awk '{print "urn:eigenius:layer:"$2}')" \
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) {
                "urn:eigenius:core:short_name": ?name
              }
              WHERE ?name IN ["BoundedBy", "BoundsRequest", "IntervalFunction"]
              RETURN [] { iri: ?iri }' \
    --institution-file julia/institutions/intervals/declarations/intervals-institution.eigon.json \
    --language julia \
    --output /tmp/intervals-mirror \
    --json
```

| Flag | Use |
|---|---|
| `--layer <IRI>` | Layer the mirror anchors to (committed under `runtime:source_layer`). Pin to the head of the branch you'll install against. |
| `--filter <EIGENQL>` | Inline EigenQL query selecting seed class IRIs. Mutually exclusive with `--filter-file`. The query must `RETURN [] { iri: ?iri }`. |
| `--filter-file <FILE>` | Path to a file containing the filter query. |
| `--institution-file <FILE>` | Optional path to the institution declaration file (the same file passed to `institution install`). When set, the seed is augmented with every class referenced by the file's `RuntimeMethodSignature.input_types` / `output_type` — closes the gap the closure walker can't reach (cross-institution return classes). The flag reads the file rather than querying the chain because the institution declaration commits *after* `mirror create` in the canonical install order. |
| `--language <LANG>` | Target language. v1 supports `julia`; others tracked in [issue #41](https://github.com/eigenius/eigenius/issues/41). |
| `--output <DIR>` | Directory the source files are written to (commits to the chain regardless). |
| `--json` | JSON-formatted output (mirror IRI, file count, output dir). |

**The closure walker.** From the seed classes, the mirror generator follows every `requires`, `class_types`, and inductive-type-ctor reference, recursively. `--institution-file` augments that closure with classes mentioned in the institution's typed method contracts — needed because cross-institution return classes (e.g. an `OptimisationProblem` returned from a Symbolics handler) aren't reachable by class-property walking from a Symbolics-rooted seed.

### `mirror get --iri <MIRROR_IRI> --output <DIR>`

Fetch a previously-committed mirror's source files. No commit.

```bash
eigenius --endpoint http://localhost:50051 mirror get \
    --iri urn:eigenius:runtime:mirror:julia:6b15cd5c3e289a8c \
    --output /tmp/mirror-extract
```

### `mirror list [--language <LANG>]`

List committed mirrors.

```bash
eigenius --endpoint http://localhost:50051 mirror list --language julia
```

### `mirror inspect <MIRROR_IRI>`

Inspect a mirror's metadata (source layer, seed classes, file count, language, source hash).

```bash
eigenius --endpoint http://localhost:50051 mirror inspect \
    urn:eigenius:runtime:mirror:julia:6b15cd5c3e289a8c
```

## 4.8. Env commands (require `--endpoint`)

The `env` subcommand group manages `RuntimeEnvironment` resources — pinned worker-image identities (image digest + runtime version + lockfile + lifecycle). Used as the second step of the substrate-institution install flow.

### `env build --language <LANG> --mirror <MIRROR_IRI> [--package-path <DIR>] [--base-image <REF>] [--worker-source-dir <DIR>] [--depot <DIR>]`

Build a worker container image from a handler package + a previously-committed mirror. Runs `buildah` on the host, then `docker load`s the result so the orchestrator's daemon can run it. Prints the resulting image digest and the runtime version captured from the built image. Does **not** commit a chain resource — pass the printed digest to `env create` for that.

```bash
eigenius --endpoint http://localhost:50051 env build \
    --language julia \
    --package-path julia/institutions/intervals/EigeniusIntervals \
    --mirror urn:eigenius:runtime:mirror:julia:6b15cd5c3e289a8c \
    --base-image docker.io/library/julia:1.12-bookworm \
    --json
```

| Flag | Default | Use |
|---|---|---|
| `--language <LANG>` | `julia` | Target language. |
| `--package-path <DIR>` | cwd | Handler package directory (must contain `Project.toml` and `src/`). |
| `--mirror <MIRROR_IRI>` | — | A previously-committed `RuntimePackageMirror` to bake in. |
| `--base-image <REF>` | `julia:1.12-bookworm` | Override the language's default base image. Pin by digest in production. |
| `--worker-source-dir <DIR>` | `julia/runtime-worker/` resolved against `$EIGENIUS_HOME` | Path to the language-runtime worker source. |
| `--depot <DIR>` | fresh temp dir | Build context / depot path the buildah build reads from. |
| `--json` | — | JSON output: `{image_digest, runtime_version, package_name, mirror_iri}`. |

Cold builds take 30–90 seconds (most of it `Pkg.precompile`); subsequent rebuilds without input changes hit buildah's layer cache.

**`--language oci` (D60, generic tool runtime).** Bakes a pinned Eigenius worker
binary (no mirror, no handler package) into an image over `--base-image`, and emits a
kernel-tracked `runtime:BuildRecipe` alongside the digest:

```bash
eigenius --endpoint http://localhost:50051 env build --language oci \
    --worker-source-dir target/release/eigenius-schemaorg-worker \
    --base-image debian:bookworm-slim
# → Digest: sha256:…  +  BuildRecipe (Eigon-JSON: base_image, artifact_hashes,
#   dockerfile, build_command, builder_version) — commit it with `env create`.
```

The worker binary must be the *same* one the orchestrator stages
(`EIGENIUS_OCI_WORKER_BINARY_PATH`), or the boot cross-check (D26 §9.3) fails. See
[chapter 11 §11.7](11-runtime-substrate.md) and
[D60](../../design/d60-native-runtime-and-tracked-env-build.md).

### `env create --language <LANG> --handler-package <DIR> --mirror <MIRROR_IRI> --as-iri <ENV_IRI> --image-digest <DIGEST> --runtime-version <VERSION> [--include-package <DIR> ...] [--base-image <REF>]`

Commit a `RuntimeEnvironment` resource pinning the env image identity. Pass the digest and runtime version that `env build` printed.

```bash
eigenius --endpoint http://localhost:50051 env create \
    --language julia \
    --handler-package julia/institutions/intervals/EigeniusIntervals \
    --mirror urn:eigenius:runtime:mirror:julia:6b15cd5c3e289a8c \
    --as-iri urn:eigenius:intervals:env:v1 \
    --image-digest sha256:1234... \
    --runtime-version 1.12.6
```

| Flag | Use |
|---|---|
| `--as-iri <ENV_IRI>` | IRI to commit the `RuntimeEnvironment` under. |
| `--image-digest <DIGEST>` | `sha256:` prefix; the digest `env build` printed. |
| `--runtime-version <VERSION>` | Exact runtime version (e.g. `1.12.6`). Required by the chain ontology. |
| `--include-package <DIR>` | Repeatable. Extra package directories to bake in as path-deps. |
| `--base-image <REF>` | Override the language's default base image. |

### `env list [--language <LANG>]`

```bash
eigenius --endpoint http://localhost:50051 env list --language julia
```

### `env inspect <ENV_IRI>`

```bash
eigenius --endpoint http://localhost:50051 env inspect urn:eigenius:intervals:env:v1
```

## 4.9. Institution commands (require `--endpoint`)

Install and inspect institution declarations. Used as the third step of the substrate-institution install flow.

Use `institution install` for substrate-hosted institutions whose declaration includes `Institution { runtime: external, requires_environment: ... }`. In-process institutions (`runtime: in_process`, e.g. Reasoning / Lean / Statistics) are linked into the kernel binary and register at startup — no install step.

### `institution install --definition <FILE>`

Submit an institution definition (Eigon-JSON or ESL) to the chain via `Load`. The file typically commits 5–10 resources in one shot — `Institution` + `RuntimeMethodSignature × N` + `QueryClass × N` + `ExportFormat` / `ImportFormat`.

```bash
eigenius --endpoint http://localhost:50051 institution install \
    --definition julia/institutions/intervals/declarations/intervals-institution.eigon.json
```

### `institution list`

```bash
eigenius --endpoint http://localhost:50051 institution list
```

### `institution inspect <IRI>`

Print an installed institution's full surface — `Institution` resource plus the QueryClasses, ExportFormats, ImportFormats, and signatures anchored on it.

```bash
eigenius --endpoint http://localhost:50051 institution inspect \
    urn:eigenius:institutions:intervals
```

## 4.10. Capability commands

Registered components and institutions are inspected through the
`capability` subcommand. All require `--endpoint`.

> **Note (2026-07-08):** `capability install` was removed with WASM
> extensibility. Components and institutions are now declared as ontology
> resources loaded via `load` (external / in-process backends), not
> installed as WASM binaries.

### `capability list`

```bash
eigenius --endpoint http://localhost:50051 capability list
```

List every registered component and institution with kind and capability level.

### `capability inspect <IRI>`

```bash
eigenius --endpoint http://localhost:50051 capability inspect \
    urn:example:institutions:Reasoning
```

Print details for a registered capability: input/output types (components), declared morphism/query/comorphism types (institutions), capability level.

### `capability test <IRI> --input <FILE> [--mode query|discover]`

Invoke a registered capability with test input.

```bash
eigenius --endpoint http://localhost:50051 capability test \
    urn:example:components:DocValidator \
    --input /tmp/doc.json
```

For institutions, `--mode query` (default) dispatches a fiber query; `--mode discover` dispatches `discover-morphisms`.

## 4.11. Task commands (require `--endpoint`)

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

## 4.12. Data commands (require `--endpoint`)

The `data` subcommand group manages **external data files** — large dataset files (CRISPR dependency matrices, expression tables, GMT gene-set files, `.rds` blobs) that are too big, too binary, or too provenance-sensitive to inline into the chain. Each file is attached as a content-addressed `ingest:PinnedExternalFile` node ([D53](../../design/d53-large-data-tracking.md)): the **bytes stay off-chain**; only the durable `reference` (locator), the `content_hash` (sha256), and the `media_type` travel on the chain. The IRI is derived from the content hash, so byte-identical files converge to one node (idempotent attach).

All `data` commands talk to a running kernel and require `--endpoint`.

**Reference schemes.** A file is identified by a *reference* — the durable locator the substrate fetches from later:

| Scheme | Example | Notes |
|---|---|---|
| local path | `data/depmap/crispr.parquet` | Canonicalised to a `file://` absolute path on attach. |
| `file://` | `file:///var/lib/eigenius/depot/crispr.parquet` | A path on a volume the kernel can read directly — no provisioning needed. |
| `oxen://` | `oxen://ml-datasets/depmap@main/crispr.parquet` | Versioned, content-addressed Oxen remote. Grammar: `oxen://[<host>/]<namespace>/<repo>@<revision>/<path>`. `<host>` is optional and defaults to `hub.oxen.ai`; `<revision>` is a branch name or commit id. The CLI fetches once (via the prebuilt `oxen` client) to compute the hash. |

**Oxen auth.** Oxen access uses a per-host bearer token in `auth_config.toml` under the Oxen config dir (`$OXEN_CONFIG_DIR`). The token is a deployment secret held substrate-side; it never enters a worker image. Override the client binary with `EIGENIUS_OXEN_BIN` and the URL scheme with `EIGENIUS_OXEN_SCHEME` if needed.

### `data attach <FILE_OR_REF> [--reference <REF>] [--media-type <MT>] [--name <NAME>]`

Hash the bytes, mint the content-addressed IRI, and commit the `PinnedExternalFile` node. For an `oxen://` reference the CLI downloads once to compute the hash, then discards the temp copy.

```bash
# Local file — reference defaults to a file:// URL of the absolute path
eigenius --endpoint http://localhost:50051 data attach \
    data/depmap/crispr-gene-effect.csv

# Oxen-backed — the oxen:// reference is stored verbatim as the locator
eigenius --endpoint http://localhost:50051 data attach \
    oxen://ml-datasets/depmap@main/crispr-gene-effect.parquet

# Override the durable locator (e.g. attach from a local copy but record the
# shared-volume path the kernel will read at recompute time)
eigenius --endpoint http://localhost:50051 data attach /tmp/crispr.parquet \
    --reference file:///var/lib/eigenius/depot/crispr.parquet
```

| Flag | Default | Use |
|---|---|---|
| `--reference <REF>` | `file://` of the abs path (local), or the `oxen://` ref verbatim | The durable backend locator stored on the node — what the substrate fetches from later. Override when the bytes you're hashing live somewhere other than where the kernel will read them. |
| `--media-type <MT>` | inferred from extension | Override the IANA media type (e.g. `text/csv`). Inference strips a trailing `.gz` and maps `.parquet`, `.arrow`, `.csv`, `.tsv`/`.gmt`, `.json`, `.xlsx`, `.h5`/`.hdf5`, `.rds`; anything else is `application/octet-stream`. |
| `--name <NAME>` | the file name | Override the `short_name`. |

JSON output (`--json`) carries `success`, `iri`, `content_hash`, `reference`, `media_type`.

### `data list [--media-type <MT>]`

List every attached `PinnedExternalFile` with its media type and reference.

```bash
eigenius --endpoint http://localhost:50051 data list
eigenius --endpoint http://localhost:50051 data list --media-type text/csv
```

### `data inspect <DATA_IRI>`

Print one pinned file's metadata — reference, content hash, media type, bound schema, source.

```bash
eigenius --endpoint http://localhost:50051 data inspect \
    urn:eigenius:ingest:file:9b1c...
```

### `data verify <DATA_IRI>`

Re-fetch the bytes by the node's `reference`, recompute the hash, and check it against the pinned `content_hash` (fail closed — D53 §5). Proves the off-chain bytes still match what was attached. Exits non-zero on a mismatch.

```bash
eigenius --endpoint http://localhost:50051 data verify \
    urn:eigenius:ingest:file:9b1c...
```

### `data validate <DATA_IRI>`

The D53 §4.1 checkable layout gate. Materializes the file (which also re-verifies the content hash), reads its header, and checks each bound `DatasetSchema`'s declared layout against the actual columns. Delimited text (CSV/TSV) is header-checked in process; columnar formats (Parquet/Arrow) and compressed files carry their schema in-file and defer to the worker. A file with no bound schema is reported valid (opaque file — nothing to check).

```bash
eigenius --endpoint http://localhost:50051 data validate \
    urn:eigenius:ingest:file:9b1c...
```

### `data provision <DATA_IRI> [--cache-root <DIR>]`

Materialize a pinned file into the local content-addressed cache (`<cache>/<sha256-hex>/<name>`) that the kernel reads for native file-backed `SampleSet` recompute (D53 §6.1 / §7). Fetches and content-verifies via the §5 resolver. Run this on the host whose depot the kernel reads.

```bash
eigenius --endpoint http://localhost:50051 data provision \
    urn:eigenius:ingest:file:9b1c... \
    --cache-root /var/lib/eigenius/substrate-depot/extfile-cache
```

| Flag | Default | Use |
|---|---|---|
| `--cache-root <DIR>` | `$EIGENIUS_EXTFILE_CACHE_DIR` | The depot's extfile-cache directory the kernel reads. Required either via this flag or the env var. |

A `file://` reference on a volume the kernel already reads needs no provisioning — the kernel reads it in place. Provision is for `oxen://` (and any reference you want warmed into the cache ahead of a recompute).

## 4.13. Other commands

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

## 4.14. Output formatting

The global `--json` flag switches output from human-formatted prose to a machine-readable JSON envelope, suitable for piping into `jq` or scripting:

```bash
eigenius --json query 'MATCH ?x {} RETURN [] { x: ?x }' | jq '.results[0]'
```

Without `--json`, output is colourised plain text intended for terminal display.

## 4.15. Exit codes

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
