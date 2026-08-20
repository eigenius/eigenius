# 6. Database management

By default, `eigenius serve` runs in-memory: every layer, trace, and registered capability lives in RAM and is lost when the kernel exits. For long-running deployments — or any setup where you want loaded ontologies and computed traces to survive across restarts — use `--db <path>` to enable RocksDB-backed persistence.

The full specification of the persistence machinery is in [D13 — Durable kernel state](../../design/d13-durable-kernel-state.md). This chapter is the operator's view.

## 6.1. Enabling persistence

```bash
eigenius serve --db /var/lib/eigenius --orchestrator http://localhost:8080
```

Or via env var:

```bash
EIGENIUS_DB=/var/lib/eigenius eigenius serve --orchestrator http://localhost:8080
```

The path can be anywhere the kernel process can write. On first start, the kernel creates the directory if it doesn't exist. Subsequent starts open the existing database.

## 6.2. What gets persisted

Phase 14 made the on-disk layout a stable contract. The kernel writes (and reads) several distinct prefixes inside a single RocksDB instance — see [D23 §6](../../design/d23-out-of-core-layer-architecture.md#6-storage-layout) for the full layout. At a high level:

Fifteen key prefixes on the **default** column family:

| Category | Prefix(es) | Purpose |
|---|---|---|
| **Layer content** | `layer:<id>:res:<iri>` | Every resource a layer defines, CBOR-encoded |
| **Topology** | `topo:<id>` | Per-layer `LayerHandle` (DAG metadata) |
| **Chain** | `chain:<id>` | Canonical parent edge for chain-walk reconstruction (empty for a root) |
| **Branches** | `branch:<name>` | Named mutable pointers into the layer DAG (D23 §5.5) |
| **Tags** | `tag:<name>` | Immutable named refs; GC roots alongside branches (D34 §G.2) |
| **Shadowing blooms** | `bloom:<id>` | Per-layer Bloom filters for `Layer::resolve` (D23 §5.2) |
| **Content dedup** | `content:<content_hash>:<position_hash>` | Every position sharing a content hash (D25 §11.0 / D33 §6) |
| **Anchored commits** | `anchored:<content>:<supporting_content>` | Commit-time cache so byte-identical regeneration reuses a layer id (D33 §6) |
| **Redirects** | `redirect:<source_layer>` | Resolve redirects installed by below-head consolidation (D25 §12.8) |
| **Triple index** | `idx_pos:<p>:<o>:<s>:<layer>`, `idx_layer:<layer>:<p>:<o>:<s>` | POS index for indexed query reads (D23 §5.9) |
| **Value index** | `vidx_pos:…`, `vidx_layer:…` | Exact-value index (D65) |
| **Traces** | `trace:<key>` | Completed program execution traces (D6b, D21) |
| **Tasks** | `meta:session:<id>:task:<id>:*` | Task records + per-task IO traces (D21) |
| **Schema metadata** | `meta:schema_version`, `meta:last_writer_version`, `meta:schema_history` | On-disk schema versioning (D24) |
| **Manifest** | `meta:seed_manifest_v1` | Embedded-ontology hash, for drift refusal |

There is no `head` key — the pre-Phase-14 single-head pointer was replaced by branch refs — and no `layer:<id>:meta` key; per-layer metadata lives in the `topo:<id>` handle.

Three further keyspaces live on their own column families: `cf_text` (`text_term:`, `text_docs:`, `text_stats:`, `text_terms_layer:`), `cf_vec` (`vec_seg:`, `vec_layer:`), and `cf_embed_cache`, which is opened but not yet written. The live list is the module header of [`storage/rocksdb/src/lib.rs`](../../../storage/rocksdb/src/lib.rs).

Institutions and components are persisted as ordinary resources inside layers — they live under `layer:<id>:res:<iri>` like any other resource and are re-registered from a chain scan on kernel restart.

Direct manipulation of these prefixes outside the kernel API is unsupported and likely to cause corruption — go through `eigenius` or the gRPC surface.

## 6.3. Boot-time refusal: schema version + manifest

`bootstrap_persistent` runs two independent checks on every restart against a populated database. The kernel refuses to start if either fails.

### Schema-version check (D24)

The first time `serve --db <path>` runs against a fresh DB, the kernel stamps `meta:schema_version` with its compiled-in `SCHEMA_VERSION` (currently `1` — the cumulative Phase 14 layout). On every restart, it reads that value and compares to its expectation:

| Stored | Action |
|---|---|
| Missing on a non-empty DB | Refuse — `SchemaVersionAbsent`. The DB was written by a pre-marker kernel; re-seed against a fresh `--db` path. |
| Equal to current | Resume normally. |
| Lower than current | Run registered migrations in order. Phase 14 is `v1` and ships no migrations; the first migration arrives with whichever future PR bumps to `v2`. |
| Higher than current | Refuse — `SchemaTooNew`. Older kernel against a newer DB; upgrade the kernel binary. |

See [D24 — Schema Versioning Policy](../../design/d24-schema-versioning.md) and the [schema changelog](../../design/schema-changelog.md) for the full contract and the per-version history.

The schema-version check fires *first* — there's no point validating ontology fingerprints against a DB whose shape we can't safely walk.

### Manifest drift check

The kernel also records a SHA-256 manifest of the embedded ontologies on first start — all twenty of them, in `BOOTSTRAP_CHAIN` order, as `<name>:<sha256_hex>` lines over the raw source bytes. On every subsequent restart, it re-hashes the embedded sources and compares to the stored manifest. The same array drives the chain build, so no layer can be hashed and not booted, or booted and not hashed.

If the hashes differ — typically because you upgraded `eigenius` to a version that ships different bootstrap ontology JSON — the kernel refuses to start with `ManifestDrift`. The error names the differing file(s).

This is intentional: an ontology change can invalidate persisted resources whose validation depended on the prior shape. The recovery path:

1. **Export** the current database with `eigenius db export <db-path> /tmp/export`.
2. **Inspect** the export to identify resources that may need to be migrated.
3. **Delete** the database directory (or move it aside) and re-create it with the new kernel: `eigenius serve --db <path>` re-seeds the manifest.
4. **Re-load** the exported resources with `eigenius load`.

For routine kernel upgrades that don't touch the bootstrap ontologies (the common case), no migration is needed and no drift is detected.

The two checks are independent: a kernel upgrade that changes storage shape (schema-version bump) but keeps the same ontologies passes the manifest check; a kernel that bundles a new ontology JSON without changing storage shape passes the version check. In practice most upgrades pass both.

## 6.4. `db stats` — what's in there

Stop the server first (RocksDB takes a directory lock; `db stats` opens the directory cleanly when the server is down).

```bash
eigenius db stats /var/lib/eigenius
```

Output includes:

- The database path and the layer count read from the persisted topology.
- One line per layer with its resource count, then the total resource count.
- One line per branch ref with its current head (Phase 14g) — useful for spotting stale feature branches that haven't been pruned.

It reports no byte sizes and no key counts; for on-disk size use `du -sh` on the directory. Use this to audit layers and branches before running cleanup. For a live kernel, `eigenius --endpoint ... branch list` gives the branch view without taking the database offline.

## 6.5. `db compact` — defragmenting

```bash
eigenius db compact /var/lib/eigenius
```

Triggers a manual full compaction on every column family. Compaction is the process by which RocksDB rewrites SSTables to remove tombstones and merge level-N files into level-(N+1).

When to run:

- After a large delete operation (compaction reclaims tombstoned space).
- After a long period of trace generation (traces accumulate and compact opportunistically; manual compaction can free disk faster).
- Before backing up — produces a smaller, more contiguous on-disk image.

Compaction is I/O-intensive. Run during a maintenance window if the database is large.

## 6.6. `db export` — dumping to JSON

```bash
eigenius db export /var/lib/eigenius /tmp/eigenius-export
```

Walks every layer in the database and emits Eigon-JSON files into the output directory. The export is round-trippable: `eigenius load` over the resulting files reconstructs an equivalent layer set on a fresh database.

Use cases:

- **Backup snapshots** — periodic full exports.
- **Migration** — across kernel versions that changed bootstrap ontologies.
- **Debugging** — JSON is easier to grep than binary RocksDB files.
- **Cross-environment transfer** — export from production, load into a dev kernel for repro.

The exported file format is the standard Eigon-JSON ([D1](../../design/d1-eigon-serialization-format.md)) — readable in any editor, compact via `gzip`.

## 6.7. Branches

Phase 14g made branches the only head-pointer surface — every commit lands on a branch, and `main` is the default. Day-to-day operators interact with branches through the CLI:

```bash
# List every branch and its current head
eigenius --endpoint http://localhost:50051 branch list

# Show one branch
eigenius --endpoint http://localhost:50051 branch show main

# Create a feature branch off main
MAIN_HEAD=$(eigenius --endpoint http://localhost:50051 branch show main --json | jq -r .head_layer)
eigenius --endpoint http://localhost:50051 branch create feature-x --from "$MAIN_HEAD"

# Commit onto it
eigenius --endpoint http://localhost:50051 load demo/document.esl --branch feature-x

# Prune when done — refuses by default if a task pin matches the head
eigenius --endpoint http://localhost:50051 branch delete feature-x
eigenius --endpoint http://localhost:50051 branch delete feature-x --force   # skip the pin check
```

See [§4.6](04-cli-reference.md#46-branch-commands-require---endpoint) for the full command reference. The full design is in [D23 §5.5 (branch refs)](../../design/d23-out-of-core-layer-architecture.md#55-branch-refs-14d) and [§5.4 (`update_branch` CAS)](../../design/d23-out-of-core-layer-architecture.md#54-write-model).

When a branch is deleted, layers reachable only through it become candidates for garbage collection on the next sweep. Today GC is a library-level API only — no CLI command — so deleted-branch storage stays on disk until the dev DB is wiped (`docker compose down -v` or equivalent). [Issue #37](https://github.com/eigenius/eigenius/issues/37) tracks the operator-facing GC surface for Phase 14f-ii.

## 6.7a. Chain consolidation and merge

Two `db` subcommands operate on the chain rather than the files, and both **require** `--endpoint` because they serialise against the running kernel's branch lock. Full shapes in [§4.5](04-cli-reference.md#45-database-commands).

**`db consolidate <FROM..TO>`** collapses an inclusive layer range on a branch into one resolve-equivalent layer ([D25](../../design/d25-chain-consolidation.md)). This is the answer to a chain that has grown deep enough for resolve walks to dominate query latency; re-loading into a fresh database is no longer the workaround.

```bash
# Estimate first: cost, and the layer id the consolidation would produce
eigenius --endpoint http://localhost:50051 db consolidate <from-hex>..<to-hex> --dry-run

# Then commit it
eigenius --endpoint http://localhost:50051 db consolidate <from-hex>..<to-hex> --branch main
```

`--max-walk-entries` overrides the cost cap (kernel default `5_000_000`). `--preserve-history` applies to below-head consolidation only and keeps the source range readable, at the cost of GC not reclaiming it.

Consolidating *at* the branch head advances the branch ref to the new layer. Consolidating strictly below the head installs a resolve redirect at `to` and leaves the branch ref alone — the response reports `head_advanced: false`, which is a success, not a failure.

**`db merge preview` / `db merge resolve`** reconcile a diverged head with a branch ([D20](../../design/d20-layer-reconciliation.md)); see [chapter 16](16-merge-resolution.md) for the strategies and the resolution-file format.

## 6.8. Backup strategy

Three options, ordered by overhead:

1. **Filesystem snapshot of the RocksDB directory** — fastest. Stop the kernel, copy the directory, restart. Works because RocksDB's on-disk format is self-contained; copying yields a usable database. Suitable for short maintenance windows.

2. **`db export` snapshot** — slower (full walk + JSON serialization), but produces a portable, version-independent artifact. Suitable for archival and for migrations.

3. **Live snapshot** (RocksDB checkpoint) — not currently exposed via CLI; would require a small Rust shim. Filed under future work.

**None of the three works against a running kernel.** `db export` opens RocksDB directly and the kernel holds the exclusive directory lock while it is up, so (1) and (2) both require stopping the server; (3) does not exist. There is no online-backup path today.

## 6.9. RocksDB layout

The RocksDB directory contains the standard RocksDB on-disk files:

- `OPTIONS-*` — RocksDB configuration files
- `MANIFEST-*` — RocksDB's internal manifest
- `CURRENT` — pointer to the current MANIFEST
- `*.sst` — SSTable files holding the actual data
- `*.log` — write-ahead logs for recent writes
- `LOCK` — process lock (the reason `serve` and `db compact` can't run concurrently)
- `LOG`, `LOG.old.*` — RocksDB internal info logs

Eigenius keeps everything structural on the **default column family** with prefix-keyed entries — see §6.2 for the full prefix list. This is a deliberate choice (D23 §4): the prefix scheme is debuggable with `rocksdb_dump`, requires no per-CF tuning, and extends additively as new surfaces land. Phase 14 added `topo:`, `bloom:`, `branch:`, `idx_pos:` and `idx_layer:`; `tag:`, `content:`, `anchored:`, `redirect:` and the `vidx_*` pair came later. The three D43 column families (`cf_text`, `cf_vec`, `cf_embed_cache`) are the only departure, isolating text and vector segment churn from layer and topology writes.

Direct manipulation of the RocksDB files outside the `eigenius db` and `eigenius branch` commands is unsupported and likely to cause corruption.

## 6.10. Restart re-registration

When the kernel restarts on a populated database, `start_server` pre-registers the in-process institutions the binary links, then calls `rebuild_institution_index` against the rehydrated branch head — a chain scan that picks up every `Institution` declaration in the persisted layers, so `AutoOnLoad` QueryClasses fire and institution IRIs dispatch again with no manual step. Components are resolved from the chain at dispatch time. See [D13 §4](../../design/d13-durable-kernel-state.md) for the protocol.

There is no WASM runtime involved. WASM extensibility was removed on 2026-07-08 and `wasmtime` is not a workspace dependency; nothing is loaded into a sandbox at restart. Executable extensions live either in-process (Rust institutions, compiled into the kernel) or in substrate worker containers spawned per dispatch ([chapter 11](11-runtime-substrate.md)) — neither is persisted in the database.

## 6.11. Sizing and growth

Rough numbers from the demo:

- A single small ontology (< 50 resources, < 10 KB JSON) → ~50 KB on disk after compaction.
- A program execution trace (10–20 expressions, no LLM calls) → ~5 KB.
For production-sized deployments, the dominant growth factor is usually trace storage. There is no trace retention policy and no CLI surface for garbage collection: traces accumulate until the database is dropped and re-loaded.

## 6.12. The TiKV backend (placeholder)

[`storage/tikv/`](../../../storage/tikv/) exists as a placeholder for a future distributed-storage backend. The kernel has the abstraction in place ([`kernel/src/storage/`](../../../kernel/src/storage/) traits) but the TiKV implementation is not production-ready and is not covered by this guide.

For multi-node deployments today, the recommended pattern is per-node RocksDB with the kernel as a single-tenant service — see [chapter 12](12-deployment.md) for the deployment models we support.

---

Next: **[7. The orchestrator →](07-orchestrator.md)**
