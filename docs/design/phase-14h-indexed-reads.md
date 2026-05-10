# Phase 14h: POS Index for EigenQL Reads

## Goal

Replace the three `kernel/src/query/evaluate.rs` hot-site scans with a per-layer triple index. POS only in v1, IRI-valued objects only, gated by `Property.data_type ∈ {resource, resource_array}`.

## Why per-layer (not per-head)

D23 §5.9 originally specified per-head storage. D23 §5.2 (per-layer blooms) argued forcefully against per-head bookkeeping for the same data: "Whether layer L defines IRI X is a property of L alone… Zero per-head bookkeeping. Free multi-parent merges." That argument applies equally to triple indexes.

Per-layer keeps:
- Branch divergence handled correctly by chain walk (each chain sees its own ancestors).
- No replication of the resolved triple set on every commit.
- Multi-parent merges work without index reconciliation.

Cost asymmetry: per-head is `O(answer)` reads, `O(parent_index + diff)` writes. Per-layer is `O(answer × chain_depth × bloom_check)` reads, `O(diff)` writes. Bloom checks are ns-scale; chain depth is small in practice. The user-side trick that makes per-layer competitive is putting `<layer_id>` at the *end* of the index key, so reads do one global prefix scan and use shadow checks (via existing per-layer blooms) to dedupe — see Schema below.

D23 §5.9 will be rewritten to per-layer as commit 4.

## Schema

Two RocksDB prefixes, both with empty values:

```
idx_pos:<predicate_iri>:<object_iri>:<subject_iri>:<layer_id>
idx_layer:<layer_id>:<predicate_iri>:<object_iri>:<subject_iri>
```

Key composition uses 4-byte big-endian length prefixes per IRI segment; `layer_id` is fixed 32 bytes. Helper module `kernel/src/layer/index_keys.rs` hides the encoding.

The reverse `idx_layer:` index makes GC's `delete_layer` a clean prefix delete; the `idx_pos:` entries it discovers are deleted in the same atomic batch. Storage cost ~2× — tolerable for presence-only entries.

## Indexability rule

A `(subject, predicate, object)` triple is indexed iff `predicate`'s `Property.data_type` resolves to `urn:eigenius:core:resource` or `urn:eigenius:core:resource_array` at the layer being committed. The same rule decides query-time eligibility — write and read paths share the helper:

```rust
fn is_indexable(layer: &Layer, predicate: &Iri) -> bool {
    layer.resolve(predicate)
        .and_then(|prop| prop.get(&DATA_TYPE).and_then(|v| v.as_str().map(str::to_string)))
        .map(|t| t == wk::RESOURCE || t == wk::RESOURCE_ARRAY)
        .unwrap_or(false)
}
```

Resource_array unpacks to one entry per element. Schema mutation (a property's `data_type` flipped post-commit) doesn't trigger reindexing — documented limitation.

The same rule decides query-planner eligibility: a MATCH pattern uses the index iff its predicate's `data_type` is `resource` / `resource_array`. Literal-typed predicates (string, integer, boolean, embedded) post-filter the index-narrowed candidate set. Unbound predicates fall back to scan.

## Trait

```rust
pub trait TripleIndex: Send + Sync {
    fn extend_layer(
        &self,
        batch: &mut dyn IndexBatch,
        layer: &LayerId,
        triples: &[Triple<'_>],
    ) -> Result<(), StorageError>;

    fn drop_layer(
        &self,
        batch: &mut dyn IndexBatch,
        layer: &LayerId,
    ) -> Result<(), StorageError>;

    fn scan_predicate_object(
        &self,
        p: &Iri,
        o: &Iri,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + '_>;

    fn stats(&self) -> IndexStats;
}

pub struct Triple<'a> {
    pub subject: &'a Iri,
    pub predicate: &'a Iri,
    pub object: &'a Iri,
}
```

`IndexBatch` is a thin wrapper around the backend's batching primitive — RocksDB's `WriteBatch`, the in-memory backend's `Vec<BatchOp>`. The index participates in `store_layer`'s existing atomic batch; no new commit transaction.

`PersistentBackend` gains `as_triple_index(&self) -> &dyn TripleIndex`, mirroring `as_trace_store`.

## Query algorithm

```rust
fn scan_chain(head: &Layer, p: &Iri, o: &Iri) -> Vec<Iri> {
    let chain = collect_ancestors(head);  // BTreeSet<LayerId>
    let mut results = BTreeSet::new();
    for entry in head.storage.triple_index.scan_predicate_object(p, o) {
        let (s, defining) = entry?;
        if !chain.contains(&defining) { continue; }
        if !is_shadowed(head, &defining, &s) {
            results.insert(s);
        }
    }
    results.into_iter().collect()
}
```

`is_shadowed(head, defining_layer, s)` does a constrained BFS over `head`'s ancestors that descend from `defining_layer` (excluding `defining_layer` itself), bloom-probing each for `s`. First confirmed hit → shadowed. Same mechanic as `Layer::resolve`'s bloom-walk; just generalized to multi-parent topology.

No numeric layer depth needed — the shadow check handles dedup correctly via topology walk alone.

## Three call-site rewrites

1. **`resolve_name_to_class_iri(layer, "Animal")`** — wants Class instances whose `short_name` equals the string. `short_name` is a string property (not indexable). The site stays a scan but operates on the index-narrowed Class candidate set instead of the full chain. Small refactor: collect `(is_a, Class)` candidates first, then filter by `short_name`.

2. **`collect_candidates(class_iri)`** — `scan_chain(layer, is_a, class_iri)` for the bound class, plus class-hierarchy expansion via the existing `subclass_of` walk (one index probe per concrete class in the closure).

3. **Negation helper** — same shape as collect_candidates, complement against the unfiltered universe. Falls back to scan when the predicate is unbound.

Query planner branch (per pattern):

```
if pattern.predicate is bound AND is_indexable(predicate):
    use scan_chain
else:
    use the existing scan path
```

## Merge layers (Phase 14e interaction)

Trivial merge layers define no resources (`defined_iris` empty), so the indexing logic writes zero entries — falls out of the same code path as a normal commit. Queries at a merge head walk through it to its multiple parents via `Layer.parents`; the chain set includes both branches; results from both branches are visible. No special-case logic for merges.

Future witnessed merges (Phase 15) that resolve conflicts by writing new resources land in `defined_iris` and get indexed normally.

## No backward compat

No legacy DBs exist. Bootstrap doesn't verify index/layer consistency; the atomic-batch guarantee makes drift impossible at commit time. Any DB the kernel opens was built post-14h and therefore has index entries. No migration code, no rebuild path.

## Steps (4 commits — as landed)

**Commit 1 — Trait + in-memory impl + LayerStorage slot**

- `kernel/src/layer/index.rs`: `TripleIndex` trait, `Triple`, `IndexStats`, `MemoryTripleIndex`, plus the `index_keys` submodule for length-prefixed forward/reverse keys.
- `LayerStorage` gains `triple_index: Arc<dyn TripleIndex>`; all three constructors wired (`in_memory()`, `with_persistent()`, `with_persistent_bounded()`).
- `PersistentBackend::triple_index_arc()` mirrors `as_trace_store`; `MemoryPersistentBackend` returns its index; `RocksStore` carries a placeholder `Arc<MemoryTripleIndex>` that commit 2 swaps out for the real impl.
- The pre-14h `storage/indexing/` stub crate is left in place for commit 1 (zero callers) and removed in commit 4 alongside the doc cleanup.

**Commit 2 — RocksDB impl + commit-time population + GC drop**

- `storage/rocksdb/src/triple_index.rs`: `RocksTripleIndex` over `Arc<rocksdb::DB>` with both `idx_pos:` and `idx_layer:` prefixes; standalone `extend_layer`/`drop_layer` create their own `WriteBatch`; `extend_into_batch`/`drop_into_batch` append to a caller-supplied batch.
- `RocksStore.db: Arc<rocksdb::DB>` so the index can share the handle. `RocksStore.triple_index: Arc<RocksTripleIndex>` replaces the placeholder.
- `RocksStore::store_layer` initially populated the index in its `WriteBatch`; commit 3 moved population to `LayerBuilder::build` so the in-memory and persistent paths are symmetric. `RocksStore::delete_layer` drops index entries via the reverse-index walk in its `WriteBatch`.
- Tests: forward/reverse symmetry, restart consistency, GC drop, branch-divergence (define `rex` differently on `main` and `feature`; verify per-chain queries see the right thing).

**Commit 3 — Wire query evaluator**

- `kernel/src/layer/index.rs` adds `collect_ancestors`, `is_shadowed`, `scan_chain` — bloom-walk over the multi-parent ancestor topology, skipping the defining layer and its parents.
- `LayerBuilder::build` pre-populates the triple index for the freshly-built layer (mirrors how it pre-populates the bloom). This is what makes the in-memory bootstrap path index-aware without going through `store_layer`. The duplicate population in `RocksStore::store_layer` is removed.
- `kernel/src/query/evaluate.rs::collect_candidates` uses `scan_chain` when the pattern's class is bound and `is_a` is indexable; falls back to `collect_candidates_via_scan` for untyped patterns or non-indexable predicates. `class_with_subclass_closure` walks `subclass_of` recursively via `scan_chain`.
- Test helpers updated to share `LayerStorage` across builders (matches production bootstrap semantics).

**Commit 4 — Doc cleanup + remove superseded crate**

- D23 §5.9 rewritten to per-layer storage matching §5.2; §6.2 prefix table updated to `idx_pos:` / `idx_layer:`; §6.3 atomic-commit list reflects build-time population.
- D23 §3, §7.1, §10 references updated; appendix and implementation plan mirror the same.
- `storage/indexing/` crate deleted (superseded; stubs were never called).
- `Cargo.toml` workspace member list updated.

## Files changed

| File | Change |
|------|--------|
| `kernel/src/layer/index.rs` | New: `TripleIndex` trait + `MemoryTripleIndex` + `index_keys` + `extract_indexable_triples` + `is_indexable_predicate` + `collect_ancestors` + `is_shadowed` + `scan_chain` |
| `kernel/src/layer/storage.rs` | `LayerStorage.triple_index` |
| `kernel/src/layer/mod.rs` | Re-exports + `LayerBuilder::build` pre-populates the index |
| `kernel/src/storage/mod.rs` | `PersistentBackend::triple_index_arc` |
| `kernel/src/storage/memory.rs` | `MemoryPersistentBackend.triple_index` |
| `storage/rocksdb/src/triple_index.rs` | New: `RocksTripleIndex` |
| `storage/rocksdb/src/lib.rs` | `RocksStore.db: Arc<DB>`, real index plumbing, `delete_layer` drops index entries atomically |
| `kernel/src/query/evaluate.rs` | `collect_candidates` indexed path + `class_with_subclass_closure` + scan fallback |
| `storage/rocksdb/tests/triple_index_test.rs` | New: 7 integration tests |
| `docs/design/d23-out-of-core-layer-architecture.md` | §5.9 / §6.2 / §6.3 rewrite |
| `docs/design/implementation-plan.md` + `docs/guides/platform/16-appendix.md` | References updated |
| `Cargo.toml` + `storage/indexing/` | Workspace member removed; superseded crate deleted |

## Risk areas

- **Equivalence regression**: keep the indexed/scan equivalence test in the suite permanently as a regression guard, even after the scan path is removed.
- **Indexability rule edge cases**: properties without a `data_type` field at all (malformed defs) → not indexed, queries fall back to scan. Properties whose `data_type` resolves through subclass chains → walked normally by `Layer::resolve`. No new failure modes.
- **Multi-parent shadow-check correctness**: the BFS over ancestor topology is the most subtle piece of new logic. Cover with a dedicated test using a Phase 14e merge layer.
- **Index size**: a fully-loaded ontology with 10k Class instances writes ~10k POS entries (× 2 with reverse = 20k). RocksDB compaction handles this fine. Monitor via `IndexStats` if it ever becomes a question.

## Verification

- `cargo test --workspace` — all existing tests + new ones pass.
- `cargo clippy --workspace --all-targets`, `cargo fmt --all` clean.
- Equivalence test confirms every existing EigenQL query returns identical results indexed vs. scanned.
- Microbenchmark (optional, in commit 4): 1k / 10k / 100k Class instances; indexed lookup is `O(answer)` not `O(N)`.

## Resolved design questions

- **Q1**: Indexability rule = `Property.data_type ∈ {resource, resource_array}`.
- **Q2**: No diagnostic needed for unindexable patterns — same `data_type` rule decides write and read paths; planner picks the right path mechanically.
- **Q3**: Numeric layer depth not needed — shadow check via topology walk handles dedup correctly.
- **Q4**: Trivial merge layers add no index entries (their `defined_iris` is empty); no special handling.
- **Q5/Q6**: No backward compat. No legacy DBs exist; no migration code, no startup consistency check.
