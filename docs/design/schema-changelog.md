# Schema Changelog

Append-only log of every on-disk schema bump. See [D24 — Schema Versioning Policy](d24-schema-versioning.md) for the contract this changelog tracks.

Versions are monotonic `u32`. No gaps; no number ever reused. Each entry records the kernel version that introduced the bump, the migration file (if any), and a short rationale.

## v1 — Phase 14 cumulative on-disk shape

**Kernel version:** 0.1.0 (Phase 14 close-out)
**Migration file:** none — v1 is the initial schema; pre-v1 DBs are not supported (refuse-to-boot per D24 §2).
**Affected prefixes:** all of them. Phase 14 is what made the on-disk shape a stable contract for the first time.

| Prefix | Phase | Source |
|---|---|---|
| `layer:<id>:meta` | pre-14 | D4 |
| `layer:<id>:res:<iri>` | pre-14 | D4 |
| `chain:<id>` | pre-14 | D4 |
| `trace:<key>` | pre-14 | D6b / D21 |
| `meta:<key>` | pre-14 | D13 |
| `topo:<id>` | 14a-ii | D23 §5.1 |
| `bloom:<id>` | 14b | D23 §5.2 |
| `branch:<name>` | 14g | D23 §5.5 |
| `idx_pos:<p>:<o>:<s>:<layer>` | 14h | D23 §5.9 |
| `idx_layer:<layer>:<p>:<o>:<s>` | 14h | D23 §5.9 |
| `meta:schema_version` | this commit | D24 §3.1 |
| `meta:last_writer_version` | this commit | D24 §3.1 |
| `meta:schema_history` | this commit | D24 §3.1 |
| `meta:seed_manifest_v1` | 9a | D13 §8 |

Pre-14 DBs (single-head, no topology, no blooms, no branches, no triple index) are not supported. Operators upgrading from a pre-14 kernel run a fresh `eigenius serve --db <new-path>` to seed at v1; data in the pre-14 DB is reproduced from source.

## Prefixes added after v1, with no version bump

`SCHEMA_VERSION` is still `1`, and the following key prefixes and column families have entered the on-disk shape since the v1 table above was written. None of them appears in a changelog entry, so this table is a record of the drift rather than a bump:

| Prefix / CF | Purpose | Design |
|---|---|---|
| `tag:<name>` | Immutable named refs; GC roots | D34 §G.2 |
| `content:<content_hash>:<position_hash>` | Content-hash dedup index | D25 §11.0 / D33 §6 |
| `anchored:<content>:<supporting_content>` | Anchored-commit cache | D33 §6 |
| `redirect:<source_layer>` | Below-head consolidation redirects | D25 §12.8 |
| `vidx_pos:`, `vidx_layer:` | Exact value index | D65 |
| `cf_text` (`text_term:`, `text_docs:`, `text_stats:`, `text_terms_layer:`) | Text index | D43 §2.3 |
| `cf_vec` (`vec_seg:`, `vec_layer:`) | Vector segments | D43 §2.4 |
| `cf_embed_cache` | Embedding cache — column family opened, never written | D43 §5.3 |

Also gone since v1: the `head` key, and `layer:<id>:meta`. Per [D24's "When to bump"](d24-schema-versioning.md#when-to-bump), a new persistent prefix the kernel reads on the hot path is a bump; several of the above qualify. **Whether to bump and what the migration would be is an open decision, not a scheduled one.** Until it is made, the live keyspace list is the module header of `storage/rocksdb/src/lib.rs`, not this file.
