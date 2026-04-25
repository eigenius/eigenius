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

Three categories:

| Category | Source of truth |
|---|---|
| **Layers** — every layer loaded after the bootstrap chain | [`storage/rocksdb/`](../../../storage/rocksdb/) layer column families |
| **Traces** — completed program execution traces (D6b, D21) | trace column family |
| **WASM capabilities** — installed component and institution binaries plus their declarations | capability column family |

Plus one important piece of metadata:

| Metadata | Purpose |
|---|---|
| **Embedded-ontology manifest** — SHA-256 of the four bootstrap ontology files at first start | Drift refusal on subsequent restarts |

## 6.3. Drift refusal

The first time you run `serve --db <path>` on a fresh database, the kernel records a SHA-256 manifest of the four embedded ontologies (`core`, `program`, `reflection`, `institution`) into the database. On every subsequent restart, it re-hashes the embedded ontologies and compares to the stored manifest.

If the hashes differ — typically because you upgraded `eigenius` to a version that ships a different bootstrap ontology — **the kernel refuses to start**. The error message names the differing file(s).

This is intentional: an ontology change can invalidate persisted resources whose validation depended on the prior shape. The recovery path:

1. **Export** the current database with `eigenius db export <db-path> /tmp/export`.
2. **Inspect** the export to identify resources that may need to be migrated.
3. **Delete** the database directory (or move it aside) and re-create it with the new kernel: `eigenius serve --db <path>` re-seeds the manifest.
4. **Re-load** the exported resources with `eigenius load`.

For routine kernel upgrades that don't touch the bootstrap ontologies (the common case), no migration is needed and no drift is detected.

## 6.4. `db stats` — what's in there

Stop the server first (RocksDB takes a directory lock; `db stats` opens the directory read-only-ish but cleanly).

```bash
eigenius db stats /var/lib/eigenius
```

Output includes per-column-family statistics:

- Live data size (compressed bytes on disk)
- Number of keys
- Level distribution (RocksDB's LSM tree levels)

Use this to spot unexpected size growth or to confirm a compaction took effect.

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

## 6.7. Backup strategy

Three options, ordered by overhead:

1. **Filesystem snapshot of the RocksDB directory** — fastest. Stop the kernel, copy the directory, restart. Works because RocksDB's on-disk format is self-contained; copying yields a usable database. Suitable for short maintenance windows.

2. **`db export` snapshot** — slower (full walk + JSON serialization), but produces a portable, version-independent artifact. Suitable for archival and for migrations.

3. **Live snapshot** (RocksDB checkpoint) — not currently exposed via CLI; would require a small Rust shim. Filed under future work.

For point-in-time backups during operation, prefer (2) — it doesn't require stopping the server.

## 6.8. RocksDB layout

The RocksDB directory contains:

- `OPTIONS-*` — RocksDB configuration files
- `MANIFEST-*` — RocksDB's internal manifest
- `*.sst` — SSTable files holding the actual data
- `LOCK` — process lock (the reason `serve` and `db compact` can't run concurrently)
- `LOG`, `LOG.old.*` — RocksDB internal logs

Eigenius uses several **column families** (separate keyspaces within one database):

| Column family | Holds |
|---|---|
| `layers` | Layer metadata + layer-id chains |
| `resources` | Resources keyed by (LayerId, IRI) |
| `traces` | Completed reasoning traces |
| `capabilities` | Installed WASM binaries and capability declarations |
| `manifest` | Embedded-ontology hashes (drift detection) |

Direct manipulation of the RocksDB files outside the `eigenius db` commands is unsupported and likely to cause corruption.

## 6.9. Restart re-registration

When the kernel restarts on a populated database, persisted WASM capabilities are **re-registered** with the runtime. This means:

- The component / institution registry is rebuilt from disk.
- Each WASM binary is re-loaded into the `wasmtime` runtime.
- IRIs become reachable again immediately — no manual `capability install` needed.

Restart re-registration is what makes `serve --db` truly persistent: not just data but the *executable extensions* survive. See [D13 §4](../../design/d13-durable-kernel-state.md) for the protocol.

## 6.10. Sizing and growth

Rough numbers from the demo:

- A single small ontology (< 50 resources, < 10 KB JSON) → ~50 KB on disk after compaction.
- A program execution trace (10–20 expressions, no LLM calls) → ~5 KB.
- A WASM component binary → varies; the example components are ~50–200 KB after `cargo component build --release`.

For production-sized deployments, the dominant growth factor is usually trace storage. If trace volume becomes a concern, the trace store has a configurable retention policy (planned for Phase 14) and can be size-bounded.

## 6.11. The TiKV backend (placeholder)

[`storage/tikv/`](../../../storage/tikv/) exists as a placeholder for a future distributed-storage backend. The kernel has the abstraction in place ([`kernel/src/storage/`](../../../kernel/src/storage/) traits) but the TiKV implementation is not production-ready and is not covered by this guide.

For multi-node deployments today, the recommended pattern is per-node RocksDB with the kernel as a single-tenant service — see [chapter 11](11-deployment.md) for the deployment models we support.

---

Next: **[7. The orchestrator →](07-orchestrator.md)**
