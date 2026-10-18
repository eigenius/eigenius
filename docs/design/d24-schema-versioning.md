# D24 — Schema Versioning Policy

## Status

Active. Introduced alongside the Phase 14 merge to `main` (see [`implementation-plan.md`](implementation-plan.md)). Schema version starts at **1** with the cumulative Phase 14 on-disk shape (`layer:`, `chain:`, `topo:`, `bloom:`, `branch:`, `idx_pos:`, `idx_layer:`, `meta:`, `trace:`).

## Motivation

Phase 14 makes the kernel's RocksDB layout a stable on-disk contract for the first time. Persistent state now spans branch refs, per-layer blooms, the topology DAG, and the triple index — and future phases (15 witnessed merges, 16 out-of-core query execution, the Lean/Verus institution work) will continue to evolve it. Without an explicit version stamp, any kernel that opens a DB written by a different version either silently misreads data or panics on a CBOR decode. Neither is acceptable for a system whose value proposition is provenance and verifiability.

This document defines the contract:

- **What the kernel does at boot** when the on-disk schema version disagrees with the kernel's compiled-in expectation.
- **When and how a contributor bumps the schema version** in a PR.
- **What infrastructure** the kernel ships to make migrations safe.
- **What we explicitly do not support** (downgrades, version skipping, forward compat).

The seed-manifest check from [D13 §8](d13-durable-kernel-state.md) — which catches *ontology-content* drift — is preserved unchanged. Schema versioning is the orthogonal axis: shape of the bytes, independent of which ontology resources occupy them.

## 1. Versioning scheme

Schema version is a `u32`. Monotonic. No semver, no major/minor split. Starts at 1 with the cumulative Phase 14 layout. No gaps; no number is ever reused.

The kernel carries its expected version as a compiled-in constant:

```rust
// kernel/src/storage/version.rs
pub const SCHEMA_VERSION: u32 = 1;
```

The DB carries its current version in a meta key:

```
meta:schema_version  →  4 big-endian bytes (u32)
```

These two values are the entire contract. The kernel's value never moves at runtime; the DB's value moves only when a registered `Migration` runs to completion and re-stamps it.

### Why not derive from `CARGO_PKG_VERSION`

Kernel version and schema version are different concepts. Most kernel releases (bug fixes, perf improvements, query rewrites, new gRPC fields) don't touch the on-disk shape; tying schema version to package version would force a refuse-to-boot on every kernel update against every existing DB. The constant is also self-documenting — `git blame SCHEMA_VERSION` shows the schema-affecting commits, separate from release-version churn.

The kernel version is recorded in the DB anyway (see §3.2, `meta:last_writer_version`) for diagnostics, but it's never consulted by the boot check.

## 2. Boot-time behavior

`bootstrap_persistent` reads `meta:schema_version` and dispatches:

| Stored | Action | Outcome |
|---|---|---|
| Absent (empty DB — no `branch:main`) | SEED path proceeds; stamp `SCHEMA_VERSION` after the bootstrap chain commits. | Resume normally. |
| Absent (non-empty DB) | Refuse to boot. `BootstrapError::SchemaVersionAbsent`. | Operator re-seeds the DB or runs a one-shot stamp tool (only if such a tool exists for the kernel version they're running). |
| Equal to `SCHEMA_VERSION` | Resume normally. | Resume normally. |
| Lower than `SCHEMA_VERSION` | Look up registered migrations from `stored → SCHEMA_VERSION`. If a contiguous chain exists, run them in order, re-stamping the DB after each successful step. If any link is missing, refuse with `BootstrapError::NoMigrationPath { from, to }`. | Resume normally on success; refuse with actionable detail otherwise. |
| Higher than `SCHEMA_VERSION` | Refuse to boot. `BootstrapError::SchemaTooNew { stored, kernel }`. | Operator upgrades the kernel binary. |

### 2.1 Seed path

`seed_backend` (after committing the bootstrap chain and creating `branch:main`) writes:

- `meta:schema_version` = current `SCHEMA_VERSION` as 4 BE bytes
- `meta:last_writer_version` = UTF-8 of `CARGO_PKG_VERSION`
- `meta:schema_history` = empty CBOR-encoded `Vec<MigrationRecord>` (allocated on seed so subsequent migrations can append without a "first migration is special" branch)
- `meta:seed_manifest_v1` (existing) = sha256-fingerprint of embedded ontologies

All four writes happen atomically with the bootstrap chain via the existing meta-write surface. A crash mid-seed leaves the DB unbootable (no `branch:main`), so partial state is invisible.

### 2.2 Resume path

`resume_from_backend` reads `meta:schema_version` first, then the seed manifest. The version check refuses earlier — there's no point validating ontology fingerprints against a DB whose shape we can't safely walk.

The two checks are independent: a DB can pass the version check and fail the manifest check (kernel built against different ontology JSON), or pass the manifest check and fail the version check (same ontology JSON, different storage shape — e.g., a Phase 16 kernel against a Phase 14 DB).

### 2.3 Migration application

Migrations form a contiguous chain `v_stored → v_stored+1 → ... → SCHEMA_VERSION`. The kernel:

1. Looks up `Migration` registered for `from_version() = stored`.
2. Calls `migration.apply(backend)`. If it returns `Err`, the boot fails (`BootstrapError::MigrationFailed { from, to, source }`); the DB is left at its previous version (migrations are required to be atomic — see §3.3).
3. On success, writes the new `meta:schema_version`, appends a `MigrationRecord` to `meta:schema_history`, and updates `meta:last_writer_version`.
4. Repeats with `stored = stored + 1` until `stored == SCHEMA_VERSION`.

Each step is its own logical commit. An interrupted upgrade resumes from whichever version it last completed.

## 3. Persistent surface

### 3.1 Meta keys (Phase 14 close-out additions)

| Key | Value | Written by | Read by |
|---|---|---|---|
| `meta:schema_version` | `u32` BE (4 bytes) | seed, every successful migration step | `bootstrap_persistent` (every resume) |
| `meta:last_writer_version` | UTF-8 of `CARGO_PKG_VERSION` | seed, every successful migration step | diagnostics only |
| `meta:schema_history` | CBOR `Vec<MigrationRecord>` | seed (empty), every successful migration step (append) | diagnostics, audit, `eigenius db info` |
| `meta:seed_manifest_v1` | (existing, see D13 §8) | seed | resume |

```rust
struct MigrationRecord {
    from: u32,
    to: u32,
    applied_at_ms: i64,
    kernel_version: String,
}
```

### 3.2 Migration trait (skeleton)

```rust
// kernel/src/storage/version.rs
pub trait Migration: Send + Sync {
    fn from_version(&self) -> u32;
    fn to_version(&self) -> u32;
    fn description(&self) -> &str;
    fn apply(&self, backend: &dyn PersistentBackend) -> Result<(), MigrationError>;
}

pub struct MigrationRegistry {
    migrations: BTreeMap<u32, Box<dyn Migration>>,
}
```

Phase 14 close-out ships an empty registry. The first registered migration arrives with whichever PR first bumps `SCHEMA_VERSION` to 2.

### 3.3 Migration contract

A `Migration` implementation MUST be:

- **Idempotent.** Re-running on an already-migrated DB is a no-op. A migration that's run halfway and crashed must be safe to re-run from scratch — typically achieved by structuring the migration as "if state X already exists, skip; otherwise apply X."
- **Atomic.** Either the migration completes and the DB is at the new version, or it leaves the DB at its previous version. Implementations that touch many keys MUST use `WriteBatch` (RocksDB) or the equivalent commit-or-abort primitive on other backends. The version stamp is the last thing written, after all data changes are durable.
- **Forward-only.** No `revert` method exists. Once at version N, the DB does not go back to N-1.
- **Self-contained.** A migration may not call into general kernel code that requires booting (which would be circular — bootstrap is what runs migrations). It uses only the `PersistentBackend` trait surface.

## 4. Bumping `SCHEMA_VERSION` — contributor checklist

A PR that changes the on-disk shape MUST:

1. **Bump `SCHEMA_VERSION`** by 1. Reviewers reject PRs that change persisted bytes without bumping; reviewers also reject PRs that bump without changing persisted bytes.
2. **Land a `Migration` impl** in `kernel/src/storage/migrations/v{N}_to_v{N+1}.rs` and register it in `MigrationRegistry::default()`. Empty migrations are valid (e.g., a new prefix the new kernel populates lazily) — register a no-op so the version chain has no gaps and the audit trail is uniform.
3. **Test the migration** with a DB seeded by the previous kernel version (or a fixture written by hand to mimic it), opened by the new kernel; assert post-migration state and idempotency on re-run.
4. **Append an entry to `docs/design/schema-changelog.md`** with the new version, affected prefixes / CBOR shapes, the migration file, the kernel version that introduced it, and a short rationale.
5. **Document the bump in the PR description** — quote the criterion that triggered it (which prefix / which CBOR field) so reviewers can challenge "is this really a schema break?"

### When to bump

A change is schema-affecting iff a kernel built before the change would fail or misread a DB written by a kernel built after the change. Concrete triggers:

- New persistent prefix the kernel reads on the hot path (e.g., 14h's `idx_pos:` would be a bump if it landed today).
- Changed CBOR shape of a persisted value (new required field on `LayerHandle`, renamed field).
- Renamed / reordered key prefix (`bloom:` → `b:`).
- Changed encoding (CBOR → length-prefixed bytes for some prefix).
- Changed key composition (adding a domain separator, changing length-prefix width).
- Removed prefix that older kernels expected to read.

Concrete non-triggers — do not bump:

- Bug fixes, perf improvements, refactors that don't change on-disk bytes.
- New gRPC fields / new RPCs / new CLI flags / new ESL syntax.
- Reflection / trace format additions that newer-kernel writes consume but older-kernel writes don't produce, *as long as the older kernel's reader handles missing fields gracefully.*
- Reorganising in-memory structures (caches, blooms — only the persisted bytes count).

The criterion is purely "would a kernel built before this PR fail to read a DB written by a kernel built after this PR?" If yes → bump.

## 5. What we explicitly do NOT support

- **Downgrades.** No `revert` method. Once migrated, no path back. Restore from a backup if you need to roll back a kernel upgrade.
- **Version skipping.** Migrations form a contiguous chain. v3 → v7 runs v3→v4, v4→v5, v5→v6, v6→v7 in sequence. Each step's idempotency contract makes interrupted upgrades safe to retry.
- **Reading without stamping.** A non-empty DB without `meta:schema_version` does not boot. Period.
- **Forward compatibility.** Newer DB → older kernel always refuses. There is no "compatibility mode" or "ignore unknown fields" toggle.
- **Implicit migrations.** A migration runs only if a registered `Migration` exists for the `(from, to)` step. No "auto-detect missing prefix and create it" heuristics.

## 6. Tracking surface

### 6.1 In code

- `SCHEMA_VERSION` — single `pub const` in `kernel/src/storage/version.rs`. Searchable, blame-able, and the source of truth.
- `kernel/src/storage/migrations/` — one file per `vN_to_vN+1.rs` migration. Each file owns its tests.
- `MigrationRegistry::default()` — registers all migrations the current kernel knows about. The default-impl produces a registry whose `migrations` set is exactly `{1→2, 2→3, ..., (SCHEMA_VERSION-1)→SCHEMA_VERSION}`. A debug-assert at startup verifies this invariant.

### 6.2 In docs

- `docs/design/schema-changelog.md` — append-only human-readable log. Every entry references its migration file and the PR.
- `docs/design/d24-schema-versioning.md` — this document. Updated when the policy itself changes.

### 6.3 Per-DB

- `meta:schema_history` — what versions has this specific DB been through, and when, and by which kernel? Useful for support and debugging. Surfaced via `eigenius db info` (when that command lands; it doesn't yet).

### 6.4 CI (future)

A future CI check fails any PR that:

- Adds a new `idx_*:` / `topo:` / `bloom:` / `branch:` prefix string literal in the storage backend without bumping `SCHEMA_VERSION`.
- Changes a `#[derive(Serialize, Deserialize)]` struct under `kernel/src/layer/`, `kernel/src/storage/`, or any `LayerHandle`/`MigrationRecord`-shaped type without bumping.

Until that check exists (it's worth building once the first non-trivial migration lands), the discipline is enforced by code review against this document.

## 7. Worked example — Phase 15 hypothesis

Suppose Phase 15 (witnessed merges) introduces a new prefix `merge_witness:<merge_layer_id>` carrying CBOR-encoded `WitnessRecord` per merge layer. The implementing PR:

1. Adds `WitnessRecord` to `kernel/src/lattice.rs` and the `merge_witness:` prefix to `RocksStore`.
2. Bumps `SCHEMA_VERSION` from 1 to 2 in `kernel/src/storage/version.rs`.
3. Adds `kernel/src/storage/migrations/v1_to_v2.rs`:
   - `from_version() = 1`, `to_version() = 2`, `description() = "Phase 15 witnessed merges add merge_witness: prefix"`.
   - `apply(&self, backend)` is empty — there's nothing to backfill, the new kernel populates `merge_witness:` lazily as merges happen.
   - Registered in `MigrationRegistry::default()`.
4. Tests: open a v1-stamped DB, boot a v2 kernel, observe `meta:schema_version` flips to 2 and `meta:schema_history` carries one record. Re-boot — boot is a no-op.
5. `docs/design/schema-changelog.md` entry: `## v2 (kernel v0.X.Y) — Phase 15 witnessed merges. Adds merge_witness: prefix. Migration: v1_to_v2 (no-op). PR #NN.`

If a later Phase 15 revision changes `WitnessRecord`'s CBOR shape (say, adds a required field), that's another bump — `SCHEMA_VERSION = 3`, with a real migration that walks every `merge_witness:*` entry and rewrites it.

## 8. References

- [D13 — Durable Kernel State](d13-durable-kernel-state.md), especially §8 (drift refusal) — the seed-manifest check, which is preserved as the orthogonal ontology-identity check.
- [D23 — Out-of-Core Layer Architecture](d23-out-of-core-layer-architecture.md), §6 (storage layout) — the cumulative Phase 14 prefix list that constitutes schema v1.
- [Implementation Plan](implementation-plan.md) — Phase 14 close-out introduces this stamp; future phases register migrations as needed.
- [Schema Changelog](schema-changelog.md) — append-only log of every schema bump.
