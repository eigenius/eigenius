# D23 — Out-of-Core Layer Architecture

**Status:** Implemented (Phase 14; topology/content split, branches + CAS, GC, per-layer triple index)
**Phase:** 14
**Supersedes:** the in-memory layer chain established in Phase 0; the linear chain assumption baked into D13's persistent store
**Companion docs:** D13 (durable kernel state, the atomicity guarantees Phase 14 inherits), D20 (layer reconciliation, Phase 15, uses Phase 14's branching primitive), D42 (out-of-core query execution, Phase 16, builds on Phase 14's storage abstractions)

## 1. Summary

Phase 14 lifts the kernel's working-set bound from "graph size" to "cache size" by separating **layer topology** (small, cheap, always in memory) from **resource content** (potentially large, paged through a bounded cache from the persistent backend). On top of that split, the layer model generalises from a single chain to a **DAG with named branches**, supports **multi-session writes** (one writer per branch, any reader anywhere), and gains **lifecycle operations**: reachability-based **garbage collection** with trace pinning, and explicit **branch pruning**. The EigenQL evaluator's pattern-matching path becomes **index-driven** through the previously-stubbed SPO/POS/OPS indexes; result-set processing remains in memory (operator-level spill is D42).

This document specifies the data model, the storage layout, the public Rust traits and gRPC additions, and a one-time migration path from the Phase 9a layout. Phase 15 (merge) and Phase 16 (operator spill) are downstream and lean on the structures here without modifying them.

## 2. Motivation

Three independent pressures break the current model:

1. **Graph size > RAM.** The Phase 0 design loads the entire layer chain into `Arc<Layer>` at startup ([`storage/rocksdb/src/lib.rs`](../../storage/rocksdb/src/lib.rs) `load_chain_from`). Every resource in every layer of the active head sits in a per-layer `BTreeMap<Iri, Resource>`. For the life-science worked examples (ensemble docking poses, assay replicates, PK timecourses), a single project can produce millions of resources; the kernel's footprint scales linearly with history.

2. **Branches multiply pressure.** Once branches exist (Phase 15's prerequisite), having N active heads holds N chains in memory. The naive extension of the current design doesn't survive a few branches.

3. **History accumulates.** Long-lived databases keep producing layers. Resources at lower layers that are completely shadowed by higher-layer overrides can never be observed in a current-head query, but they still sit in memory and on disk forever. Without a collection mechanism the working set grows monotonically regardless of what the user actually queries.

The fix is structural: stop holding all resource content in memory; stop assuming the chain is linear; introduce the lifecycle operations that long-lived storage requires. Bundling these together avoids building single-chain caching machinery that we'd then have to retrofit for branches — every piece needs the others.

## 3. Goals and non-goals

**Goals:**

- Working set bounded by configured cache size, not graph size.
- Multiple branches per database, single writer per branch, multi-reader.
- Background reachability-based GC; explicit branch pruning.
- Indexed query reads via SPO/POS/OPS (currently `todo!()`).
- Atomic commit across topology + bloom + content + branch ref.
- Time-travel queries (`--at-layer L`) preserved with bounded reconstruction cost.
- Restart correctness preserved (Phase 9a's RESUME path still works on the new layout).
- Online migration from Phase 9a's layout, no DB rebuild required.

**Non-goals:**

- Branch merging / reconciliation. (D20 / Phase 15.)
- Operator-level spill in EigenQL — joins, sorts, group-by accumulators stay in RAM. (D42 / Phase 16.)
- Distributed storage or read replicas. (TiKV deployment is a parallel story, not impacted here.)
- Online schema migration via comorphism. (D20's `migrate`.)
- Compression of resource content beyond what RocksDB already provides.
- Replacing the gRPC surface; this phase additively extends it.

## 4. Background — the current model

Three things in particular shape the design:

**The Layer type holds only its own resources, not the chain's merged view.** [`kernel/src/layer/mod.rs`](../../kernel/src/layer/mod.rs) `struct Layer { id: LayerId, resources: BTreeMap<Iri, Resource>, parent: Option<Arc<Layer>> }`. Resolution walks the parent chain via `resolve(iri)`; merged views are constructed on demand via `all_resources()`. This is good news — the topology/content split is already half-done; we just need to make content loadable from the backend instead of being eagerly held.

**Storage uses string-prefixed keys, not RocksDB column families.** Per [`storage/rocksdb/src/lib.rs`](../../storage/rocksdb/src/lib.rs): `layer:<id_hex>:meta`, `layer:<id_hex>:res:<iri>`, `chain:<id_hex>`, `head`, `trace:<key_hex>`. Design decision (§6.1): we keep the prefix scheme and add new prefixes for Phase 14 surfaces, rather than switching to CFs. Rationale: CFs add operational complexity (per-CF compaction tuning) for a benefit (per-CF read isolation) we don't currently exploit; the prefix scheme is debuggable with `rocksdb_dump` and trivially extensible.

**Phase 14h shipped a per-layer triple index** (§5.9) in [`kernel/src/layer/index.rs`](../../kernel/src/layer/index.rs) (trait + in-memory impl + chain-walk helpers) and [`storage/rocksdb/src/triple_index.rs`](../../storage/rocksdb/src/triple_index.rs) (RocksDB-backed impl). The earlier `storage/indexing/` stub crate was superseded and removed.

The session model ([`kernel/src/task/mod.rs`](../../kernel/src/task/mod.rs)) hardwires a single `Session::hardwired()` returning `Uuid::nil()`. D21 explicitly noted multi-session as a Phase-14 surface expansion; this doc cashes in that promise.

## 5. Data model

The fundamental shape change: **a Layer is now a node in a DAG with named branch heads, holding metadata only**. Resource content is fetched from a cache that pages through the persistent backend.

```
                       ┌─────────────────────────────────┐
                       │      LayerTopology (in RAM)     │
                       │ ┌───────────────────────────┐   │
                       │ │ layers: Map<LayerId, Hand>│   │
                       │ │ branches: Map<Name, Head> │   │
                       │ └───────────────────────────┘   │
                       └────┬─────────────────┬──────────┘
                            │                 │
              ┌─────────────▼──┐   ┌──────────▼──────────────┐
              │ BloomCache     │   │ ResourceCache (bounded) │
              │ (per layer     │   │ key: (LayerId, Iri)     │
              │  blooms,       │   │ two pools: active /     │
              │  bounded)      │   │            historical   │
              └─────────┬──────┘   └────────┬────────────────┘
                        │                   │
                        └─────────┬─────────┘
                                  │  miss
                                  ▼
                       ┌─────────────────────────────────┐
                       │      PersistentBackend          │
                       │  (RocksStore, prefix-keyed)     │
                       └─────────────────────────────────┘
```

### 5.1 LayerHandle and LayerTopology

```rust
/// Metadata-only handle for a layer. Replaces the in-memory `Arc<Layer>` chain.
pub struct LayerHandle {
    pub id: LayerId,
    /// Multiple parents only after Phase 15 introduces merge layers.
    pub parents: Vec<LayerId>,
    pub created_at: Timestamp,
    pub author: Option<String>,
    pub label: Option<String>,
    /// Number of resources defined in *this* layer (not the chain).
    pub resource_count: u64,
}

pub struct LayerTopology {
    layers: BTreeMap<LayerId, LayerHandle>,
    branches: BTreeMap<BranchName, LayerId>,
}
```

The topology is bounded by the number of layers and branches, not by graph size. For a database with 100k layers and 50 branches, the topology is comfortably under 100MB even with generous metadata.

### 5.2 Per-layer shadowing bloom

The kernel needs a way to skip layers during `Layer::resolve(iri)` without a storage probe. The naïve walk — for each layer in the head→root chain, look up `iri` in the cache (and on miss, the backend) — is O(chain_depth) storage hits in the worst case (deep history, IRI not present anywhere). For long-lived databases this is the dominant cost.

**The structure: one bloom filter per layer.** A layer's bloom describes exactly the set of IRIs it *defines directly* — the same set as the layer's `defined_iris`, just compressed. `might_contain(iri)` answers "should I bother probing this layer for `iri`?" in a few hash operations over a small bit array.

**Why per-layer, not per-head.** Whether layer `L` defines IRI `X` is a property of `L` alone. The shadowing question — *which* layer in head H's chain defines X — varies per head, but the answer is computed by walking the chain and asking each layer's bloom in turn. Keeping the index per-layer gives us:

- An immutable per-layer artifact, computable from `defined_iris` at commit time. Same lifecycle as the layer itself.
- Zero per-head bookkeeping. The kernel stays a pure DAG: there is no "shadowing index for head H" to maintain on commit, branch, or merge.
- Free multi-parent merges (Phase 14e and 15): chain walks handle multiple parents naturally; nothing to reconcile in the index.
- Clean GC: drop a layer, drop its bloom. No surgical edits.

The cost is asymptotic — `O(chain_depth)` bloom checks instead of `O(1)` per-head index lookup — but bloom checks are in-memory hash ops measured in tens of nanoseconds. Real probes (cache → backend) only happen at layers the bloom flags as "maybe present", which is the matching layer plus a small false-positive count (1% FPR by default → ~1 false probe per 100-layer chain).

#### 5.2.1 The cache

Blooms live behind a bounded cache, paged from the persistent backend. Same shape as `ResourceCache`:

```rust
pub trait BloomCache: Send + Sync {
    fn get(&self, layer: &LayerId) -> Option<Arc<BloomFilter>>;
    fn put(&self, layer: LayerId, bloom: Arc<BloomFilter>);
    /// Called by GC to drop entries belonging to a swept layer.
    fn evict_layer(&self, layer: &LayerId);
    fn stats(&self) -> CacheStats;
}

/// Backend-side surface. May be implemented by the same type as
/// `PersistentBackend` (RocksStore is) or kept separate for tests.
pub trait BloomBackend: Send + Sync {
    fn load_bloom(&self, layer: &LayerId) -> Result<Option<BloomFilter>, StorageError>;
    fn store_bloom(&self, layer: &LayerId, bloom: &BloomFilter) -> Result<(), StorageError>;
}
```

`Layer` carries `Arc<dyn BloomCache>` alongside the existing `Arc<dyn ResourceCache>` and `Arc<dyn ResourceBackend>`; `build_chain` threads it through.

#### 5.2.2 The resolve algorithm

```rust
impl Layer {
    pub fn resolve(&self, iri: &Iri) -> Option<Arc<Resource>> {
        for layer in self.iter_head_to_root() {
            let bloom = layer.bloom_cache.get(&layer.id)
                .or_else(|| {
                    let b = layer.bloom_backend.load_bloom(&layer.id).ok().flatten()?;
                    let arc = Arc::new(b);
                    layer.bloom_cache.put(layer.id.clone(), Arc::clone(&arc));
                    Some(arc)
                })?;
            if !bloom.might_contain(iri) { continue; }
            if let Some(r) = layer.lookup_local(iri) { return Some(r); }
            // false positive — keep walking
        }
        None
    }
}
```

Hot path (head's recent layers, blooms cached): all checks are in-memory hash ops + one cache/backend probe at the matching layer. Cold path (first touch of an unwarmed branch): a cache miss per layer triggers one backend probe per layer to load blooms; subsequent resolves on the same chain are warm.

#### 5.2.3 Storage

`bloom:<layer_id_hex>` → CBOR-encoded `BloomFilter` (bit array + hash count + bit width + IRI count for diagnostics). Written at `PB::store_layer` time alongside `topo:` and `layer:...:res:...` entries — part of the same atomic `WriteBatch` (§6.3).

A `BloomFilter` for a typical layer is small:

| IRIs in layer | Bloom @ 1% FPR | Bloom @ 0.1% FPR |
|---|---|---|
| 1,000 | ~1.2 KB | ~1.8 KB |
| 10,000 | ~12 KB | ~18 KB |
| 100,000 | ~120 KB | ~180 KB |
| 1,000,000 | ~1.2 MB | ~1.8 MB |

Disk cost across the whole DB ≈ 1.2 bytes per defined IRI at 1% FPR — ~1 GB of bloom storage for 1B distinct IRIs across history. Negligible relative to resource bodies and easily compressed by RocksDB's Lz4.

#### 5.2.4 Memory budget

Cache budget is a Phase 14b config knob. As an indicative target, **64 MB** holds blooms for ~5,000 typical layers (assuming ~12 KB/bloom for a 10K-IRI layer at 1% FPR). That covers the active reach of most workloads. Heavy-branching workloads with many simultaneously-active heads scale linearly — but blooms are cheap; doubling the budget is a one-line change.

The bloom cache is sized independently of the resource cache (which is much larger because resource bodies dwarf bloom data). Both share the same `evict_layer(&LayerId)` interface so GC treats them uniformly.

#### 5.2.5 Build, eviction, and GC

- **Build**: at commit time, the kernel computes the bloom from `defined_iris` (already gathered for the `LayerHandle`) and hands it to `BloomBackend::store_bloom`. Linear in IRI count; microseconds for typical layers, low milliseconds for the largest.
- **Eviction**: bounded cache with the same policy class as `ResourceCache`. Evicted blooms reload from the backend on next access.
- **GC**: when layer `L` is swept (§5.7), the same `WriteBatch` that drops `topo:<L>` and the `layer:<L>:res:*` range also drops `bloom:<L>`, and the GC sweep calls `BloomCache::evict_layer(&L)`.

#### 5.2.6 False-positive budget and tuning

Bloom FPR is fixed per layer at commit time (the bit array's `m/n` ratio is baked in). The default is **1% FPR** at the layer's actual IRI count, sized to a power-of-two bit width for fast modular arithmetic. The kernel does not currently expose per-layer FPR tuning; a heavy-resolve workload could lower the default globally via config without changing the bloom format.

A 1% FPR over a 100-layer chain produces ~1 spurious probe per resolve — measurable in profiling but not architecturally significant. Below ~0.1% FPR the bloom-storage cost grows faster than the saved probes; the default sits in the sweet spot.

#### 5.2.7 What this design gives up

The per-head shadowing index originally proposed in this doc had `O(1)` resolve. Per-layer blooms are `O(chain_depth × bloom_check)`. This matters in two regimes:

1. **Pathologically deep chains (10⁴+ layers without consolidation).** Bloom-walk dominates resolve cost. Mitigation: those workloads need layer consolidation (Phase 14e merge) anyway. If real workloads ever stress this, an optional roll-up index — periodically materialising "as of layer L, here is `iri → defining_layer`" — can sit alongside the per-layer blooms without changing the kernel's resolution algorithm. Deferred until a workload demands it.
2. **Negative-resolve hot paths (most queries return "no such IRI").** Each negative resolve walks the entire chain (every bloom returns "no"). For workloads where this dominates, the same roll-up index helps; alternatively, a per-head presence sketch (cheap because IRIs not in any layer of the chain are also not in any sketch's union) can short-circuit. Also deferred.

Both mitigations are additive — they do not require changing the per-layer bloom design that 14b ships.

### 5.3 ResourceCache (two pools)

```rust
pub trait ResourceCache: Send + Sync {
    fn get(&self, key: ResourceKey) -> Option<Arc<Resource>>;
    fn put(&self, key: ResourceKey, resource: Arc<Resource>, tier: CacheTier);
    /// Called by GC to drop entries belonging to a swept layer.
    fn evict_layer(&self, layer: LayerId);
    fn stats(&self) -> CacheStats;
}

pub struct ResourceKey {
    pub layer: LayerId,
    pub iri: Iri,
}

pub enum CacheTier {
    /// Entry is top-of-stack for the active head — i.e., the first layer
    /// in head→root order whose bloom flags `iri` and that actually defines
    /// it. Determined by the same chain-walk resolve algorithm used for
    /// reads (§5.2.2).
    Active,
    /// Entry is shadowed by a higher layer in every active head; only
    /// reachable via time-travel reads or trace dereferences.
    Historical,
}

pub struct CacheStats {
    pub active_entries: u64,
    pub historical_entries: u64,
    pub active_bytes: u64,
    pub historical_bytes: u64,
    pub hits: u64,
    pub misses: u64,
    pub promotions: u64,  // historical -> active on access
    pub demotions: u64,   // active -> historical on shadow change
}
```

**Why two pools, not one:**

- The active pool serves the steady-state query workload — top-of-stack entries that will be hit again on the next query.
- The historical pool serves time-travel queries (`--at-layer L`) and trace dereferences. These have lower locality and shouldn't push hot entries out of the active pool.
- A single LRU would conflate the two and degrade hit rates on mixed workloads.

**Eviction algorithm inside each pool:** ARC (Adaptive Replacement Cache). ARC tracks both recency and frequency in adaptive proportions, handling our workload mix (steady queries + occasional rewinds) without manual tuning. Open question (§11): benchmark ARC vs. CLOCK-Pro vs. plain LRU before committing.

**Pool capacity defaults:**

- Active: 60% of total cache budget.
- Historical: 40%.
- Configurable via `--cache-budget-mb` (total) and `--cache-active-fraction` (split).

**Cross-pool transitions:**

- *Promotion historical → active*: on access, if the entry's layer is now top-of-stack for the current head (per the §5.2.2 chain walk), promote.
- *Demotion active → historical*: when a new layer commit shadows an entry's IRI, demote on next access (lazy; no eager scan).
- *Eviction*: out of historical first, then out of active.

### 5.4 The write model: commit_layer and update_branch

The kernel's write surface decomposes into two independent, stateless operations: **`commit_layer`** appends a layer to the immutable DAG; **`update_branch`** advances a branch ref via CAS. Pinning is just a `LayerId` parameter passed on these RPCs — the kernel does not own a "session" or "scratch chain" concept beyond what D21 already provides for tasks. Clients (CLI invocations, notebooks, SDK callers, the task runner) orchestrate the two operations to produce whatever workflow they need.

This section establishes the write contract; §5.5 covers the branch surface that builds on it; §5.6 covers the time-travel reads.

#### 5.4.1 The two primitives

```rust
/// Append an immutable layer to the DAG. Returns the new layer's id.
/// The kernel does not consult or update any branch ref — that's a
/// separate operation (see §5.4.2).
pub fn commit_layer(
    parent: LayerId,
    content: LayerContent,
) -> Result<LayerId>;

/// Atomically advance a branch ref via CAS.
pub fn update_branch(
    branch: &str,
    expected_old_head: Option<LayerId>,  // None ⇒ creating a new branch
    new_head: LayerId,
    on_conflict: ConflictPolicy,
) -> Result<UpdateOutcome>;

pub enum UpdateOutcome {
    /// expected_old_head was the branch's actual head; CAS succeeded.
    FastForward,
    /// branch's head had already advanced past expected_old_head, but
    /// the changes since the divergence touch disjoint sets of IRIs;
    /// the kernel produced a merge layer with both heads as parents
    /// and updated the branch to point at it. (See §5.4.3.)
    TrivialMerge { merge_layer: LayerId },
    /// Conflict that requires a Comorphism witness (Phase 15). The
    /// branch is unchanged; the caller's `new_head` chain still exists
    /// in the DAG but isn't pointed at by any branch ref.
    NeedsWitnessedMerge { current_head: LayerId, conflicting_iris: Vec<Iri> },
}

pub enum ConflictPolicy {
    /// Allow trivial merge if no IRIs conflict; otherwise return NeedsWitnessedMerge.
    AllowTrivial,
    /// Refuse anything but a fast-forward. Useful for "I really expect this
    /// to be a clean append; surface anything else."
    StrictFastForward,
}
```

That's the entire write surface. Everything else — promotion, divergence handling, scratch chain naming — is a workflow built by clients on top of these two operations.

#### 5.4.2 Pinning is a parameter

The "pin" is just whatever `LayerId` a client passes as `parent` to `commit_layer` (and as `expected_old_head` to `update_branch`). The kernel never remembers it across calls; the client is responsible for tracking what layer it's working against.

Where clients keep their pin:

| Client | Pin storage |
|---|---|
| CLI quick command (`load`, `run`) | RPC parameter, defaulting to the current head of the target branch (one-shot read at start) |
| Long-running task | `TaskRecord.layer_head` (already exists in D21; semantically already a pin) |
| Notebook | Notebook resource field (added by D22 follow-on); persists across reconnects |
| SDK client | Application-level state held by the embedding |

Snapshot isolation falls out of this naturally. A client that holds its pin and uses it as `parent` on every `commit_layer` call sees a stable view (their reads against `pin` plus whatever they themselves have committed on top). The pin doesn't silently advance because nothing in the kernel advances it.

This is what the user means by "you pick a starting point from where you do your work, and that anchors the layers you see." Pin is a client-held `LayerId`. End of story.

#### 5.4.3 Trivial merge in `update_branch`

When a client calls `update_branch(branch, expected_old=L_pin, new_head=L_new, on_conflict=AllowTrivial)` and the branch's head has moved to `L_other ≠ L_pin`, the kernel attempts a **trivial merge** before declaring divergence:

1. Walk the topology DAG to find the lowest common ancestor `L_anc` of `L_pin → L_new` and `L_pin → L_other`. (Since both chains are rooted at `L_pin`, `L_anc = L_pin` in the simplest case; for cases where branches were already merged, `L_anc` may be earlier.)
2. Compute the IRI sets `S_caller = IRIs defined in [L_anc, L_new]` and `S_other = IRIs defined in [L_anc, L_other]`.
3. If `S_caller ∩ S_other = ∅` (disjoint contributions), produce a merge layer with `parents = [L_new, L_other]` containing the union of contributions. CAS-update the branch ref to point at the merge layer. Return `TrivialMerge`.
4. If `S_caller ∩ S_other ≠ ∅` (real conflict), the branch is left unchanged. Return `NeedsWitnessedMerge { current_head: L_other, conflicting_iris: S_caller ∩ S_other }`. The caller's new-head chain still exists in the DAG; the caller can either name it as a sibling branch (`update_branch("auto-...", None, L_new, ...)`) or discard it (let it become GC-eligible).

**Implementation cost.** The IRI-set computation is a scan of the per-layer `defined_iris` set already carried on each `LayerHandle` (§5.1). Common-ancestor traversal is a graph walk over the topology DAG (small in-memory structure). Producing the merge layer is a `commit_layer` with multi-parent semantics — already supported by `LayerHandle.parents: Vec<LayerId>` (§5.1). Total: ~6–8 days of additional implementation in Phase 14e.

**Why this is the right place for trivial merge.** Without it, every concurrent activity (two notebooks, a long-running task while you load fresh data, etc.) produces a divergent branch even when the contributions don't actually conflict. The user-visible result is "lots of branches piling up that no one wants." Trivial merge handles the 80% case automatically; Phase 15's witnessed merge handles the residual 20% where contributions genuinely conflict.

**The Phase 15 dependency.** Trivial merge is the *only* automatic merge Phase 14 ships. Real conflicts (incompatible modifications to the same IRI) require a `Comorphism` witness — that's Phase 15. Pre-Phase-15, conflicts surface as `NeedsWitnessedMerge` outcomes that the caller has to resolve manually (typically: name the chain as a sibling branch and re-do the work against the new head). This is genuinely a usability constraint on Phase 14, not a deferrable enhancement (see §11).

#### 5.4.4 Naming chains and "promotion"

There is no `promote` primitive. The combination of operations a notebook calls "publish to main" is just:

```
1. commit_layer(parent=pin, content=this_cell_output) → L1
2. commit_layer(parent=L1, content=next_cell_output)  → L2
3. ...
4. update_branch("main", expected_old=pin, new_head=Ln, on_conflict=AllowTrivial)
   → FastForward | TrivialMerge { ... } | NeedsWitnessedMerge { ... }
```

If `update_branch` returns `NeedsWitnessedMerge`, the client decides:

- *Save the work as a sibling branch:* `update_branch("auto-foo-2026-04-26", expected_old=None, new_head=Ln, ...)` — succeeds (creating a new branch is unconditional). The chain is now reachable via a name.
- *Re-derive against the new head:* discard `Ln` (it'll be GC'd), set client pin to the new head, re-do the work.
- *Defer:* keep `Ln` in client memory; do nothing now; the layers stay in storage but aren't reachable via any branch (eligible for GC if the client forgets the LayerId).

What the doc previously called "promotion" is just step 4. What it called "divergence with auto-naming" is just step 4 returning `NeedsWitnessedMerge` and the client choosing the sibling-branch option.

#### 5.4.5 Reanchoring is a client-side workflow

The same logic applies to "rebase my session onto the current main." There's no kernel primitive; the client:

1. Calls `update_branch("auto-...", None, current_chain_tip, ...)` to save the old work as a sibling branch (if desired).
2. Reads the current head of the target branch to get a new pin.
3. Continues working from the new pin.

If the user wants the old work merged with the new pin's history, that's a separate `update_branch` against the saved sibling branch's name later — typically once Phase 15's witnessed merge is available.

#### 5.4.6 Concurrency under the stateless write model

Multiple concurrent clients writing against the same DB:

- Two clients call `commit_layer` simultaneously with different parents. Both succeed independently; neither sees the other's layer until they read it. Layers are content-addressed; two independent commits don't conflict.
- Two clients call `update_branch("main", expected_old=L_pin, new_head=...)` simultaneously, both expecting `L_pin`. Both attempt CAS:
  - First call: CAS succeeds, branch advances to its `new_head`. Returns `FastForward`.
  - Second call: CAS fails (branch.head ≠ L_pin now). Kernel attempts trivial merge against the new head. If successful, returns `TrivialMerge`; if not, returns `NeedsWitnessedMerge`.
- The CAS on the branch ref is an in-process critical section (one branch-mutex per branch name); RocksDB's `write_batch` provides the atomicity for the actual store update.

There is no deadlock surface because there's no cross-branch coordination — each branch ref's CAS is independent.

#### 5.4.7 D21 (TaskRecord) and notebooks under this model

The good news: nothing about `TaskRecord` needs to change structurally for Phase 14. D21's `layer_head` field is exactly the pin. The task runner is already a client of the write API:

- On task start: read current branch head (or use a passed-in pin), store as `task.layer_head`.
- On each task step that produces a layer: `commit_layer(parent=last_layer_id, content=...)`.
- On task completion: `update_branch(target_branch, expected_old=task.layer_head, new_head=last_committed, ...)`. Outcome is recorded somewhere on the task (probably as a status field, alongside the existing terminal-state enum).

A future schema delta to `TaskRecord` would add the final outcome of `update_branch` (so users can see whether a task fast-forwarded, trivially merged, or needs witnessed-merge resolution). That's a small additive field, not a structural rework. Done in the D21 revision that lands with Phase 14e.

Notebooks are clients of the same API, with the pin held in their resource representation (added by D22 follow-on). The notebook publishes by calling `commit_layer` per cell-publish gesture, accumulates LayerIds in the notebook's resource, and offers a "promote to branch" UI gesture that issues `update_branch`. Re-anchoring composes the same operations.

#### 5.4.8 Type-check-layer monotonicity

A program is type-checked against the layer that holds its referenced ontology classes (call it `L_typecheck`). Execution must run against a layer that preserves those class signatures. In Phase 14 (only trivial merge), preservation is automatic for fast-forwards (descendants can only add new resources or shadow existing ones with same-typed redefinitions per existing validation rules) and for trivial merges (disjoint-IRI contributions cannot affect the type-checked classes, since the classes are themselves IRIs that would have been in both contribution sets if both branches modified them).

Phase 15's witnessed merge will need to enforce this monotonicity explicitly: a merge that drops or breaks classes a previously-validated program references must either re-validate the program against the merged layer or be refused. The Comorphism witness is the natural place to record the "classes are preserved by this translation" evidence.

D23 records the type-check layer in the program resource (a small additive field on the program ontology, landed alongside 14e) so Phase 15's merge can reason about it without re-walking the entire program graph.

### 5.5 Branches and BranchManager

Branches are the user-facing surface for naming layers in the DAG. They are not a separate mechanism — `update_branch` (§5.4.1) creates and advances them; `BranchManager` is the read/listing/delete surface.

```rust
pub struct BranchRef {
    pub name: BranchName,
    pub head: LayerId,
    /// The layer at which this branch diverged from its parent branch
    /// (None for the initial "main" branch).
    pub diverged_at: Option<LayerId>,
    pub parent_branch: Option<BranchName>,
    pub created_at: Timestamp,
    pub last_commit_at: Timestamp,
}

pub trait BranchManager {
    fn list(&self) -> Vec<BranchRef>;
    fn get(&self, name: &str) -> Result<BranchRef>;
    /// Removes the branch ref. Layers stay until GC; pruning is in §5.8.
    fn delete(&mut self, name: &str) -> Result<()>;
}
```

Branch creation is just `update_branch(name, expected_old=None, new_head=L, ...)` — the `expected_old=None` case unconditionally creates a new branch ref. Branch advancement is `update_branch(name, expected_old=current_head, new_head=L, ...)` with the CAS semantics from §5.4.1. There is no separate `create` or `commit` method.

**Naming:** branch names are user-given strings, validated against `[a-zA-Z0-9_-]+`. The default branch is `main` (created on first DB initialisation if absent). Auto-generated sibling-branch names (created by clients to preserve work after a `NeedsWitnessedMerge` outcome) use the prefix `auto-` to keep them visually distinct from user-created branches; `db branch list` filters them out by default, with `db divergence list` as the surface for finding them.

**Branch identity:** by name only. Layers themselves are content-addressed by `LayerId` (SHA-256 of CBOR-canonical resources + parent IDs), so the same logical branch state across two databases has the same `LayerId`s.

**`delete` vs. `prune`:** `delete` removes the branch ref but leaves the layers in place (they remain reachable through other branches or as orphans pending GC). `prune` (§5.8) is the destructive operation that tells GC "anything reachable only via this branch is collectible."

### 5.6 Time-travel reads

`--at-layer L` queries (D21 §3.6) need an "as-of-L" view. With per-layer blooms (§5.2), this requires no special machinery: the resolve algorithm against L is the same chain walk used for resolve against the current head, just rooted at L instead. `PB::load_chain_from(L)` reconstructs the chain metadata; `Layer::resolve` then walks it with the same bloom-cache + resource-cache fall-through.

The historical pool of `ResourceCache` (§5.3) absorbs the working-set pressure that time-travel queries create — entries fetched on a `--at-layer` resolve land there rather than evicting steady-state hot entries from the active pool.

No `CheckpointStore`, no shadowing-snapshot serialisation, no checkpoint cadence to tune. The earlier draft of this doc carried such a structure to keep the per-head shadowing index time-travel-friendly; that machinery exists only to support a per-head index and disappears once shadowing is per-layer.

**Reconstruction cost:** O(chain_depth × bloom_check) per resolve, identical to the current-head case. Storage cost: zero additional bytes beyond the per-layer blooms already written.

### 5.7 GarbageCollector

GC operates in one of two modes per the §11.1 decision: **topology-DAG reachability** (default) preserves all layers transitively reachable through parent pointers; **content-tree reachability** (`db gc --keep-from <layer>`) drops everything outside the chosen retention window. Both share the sweep mechanism.

```rust
pub struct GCRoots {
    pub branch_heads: Vec<LayerId>,
    /// Layers any active session has pinned (pinned_layer + scratch_head per session).
    pub session_pins: Vec<LayerId>,
    /// (Layer, IRI) pairs referenced by reflection-ontology traces.
    pub trace_pins: Vec<ResourceKey>,
    /// (Layer, IRI) pairs referenced by verified-knowledge claims.
    pub verified_pins: Vec<ResourceKey>,
}

pub enum GCMode {
    /// Default: walk parents in the topology DAG from every root; everything
    /// transitively reachable is preserved.
    TopologyDAG,
    /// Aggressive compaction: walk only the content tree of `keep_from`;
    /// drop all layers older than it. Loses time-travel below `keep_from`.
    ContentTree { keep_from: LayerId },
}

pub struct GCConfig {
    pub mode: GCMode,                  // default GCMode::TopologyDAG
    pub min_idle_seconds: u64,         // default 600 — wait for activity to settle
    pub size_threshold_bytes: u64,     // default 1 GiB — don't run on small DBs
    pub max_runtime_seconds: u64,      // default 300 — bound a single pass
    pub respect_trace_pins: bool,      // default true
    pub respect_verified_pins: bool,   // default true
}

pub struct SweepStats {
    pub layers_dropped: u64,
    pub resources_swept: u64,
    pub bytes_freed: u64,
    pub elapsed: Duration,
    pub completed: bool,  // false if timed out mid-pass
}

pub trait GarbageCollector {
    fn collect(&self, roots: GCRoots, config: &GCConfig) -> Result<SweepStats>;
}
```

**Algorithm (mark-and-sweep, topology-DAG mode):**

1. **Mark phase:** from `roots.branch_heads`, walk parents in the topology DAG, building a `reachable_layers: HashSet<LayerId>`. Add `roots.session_pins` (each active session contributes both its `pinned_layer` and `scratch_head` if any). For each `ResourceKey` in `trace_pins` and `verified_pins`, walk back from the referenced layer.
2. **For each layer L not in `reachable_layers`:** the layer itself is collectible. Schedule layer drop. This includes scratch chains from sessions that have closed without promoting, and sibling branches that have been deleted but never pruned.
3. **For each (L, X) where L is reachable but X is shadowed in every reachable head:** the resource is collectible. Schedule resource drop.
4. **Sweep:** in a single batched RocksDB transaction, drop the swept layers' `topo:<id>`, `bloom:<id>`, and `layer:<id>:res:*` ranges. Notify both the bloom cache and the resource cache to evict via `BloomCache::evict_layer` and `ResourceCache::evict_layer`.
5. **Compaction:** RocksDB compaction reclaims tombstoned space asynchronously.

**Algorithm (content-tree mode, `--keep-from <layer>`):**

1. **Mark phase:** walk the content tree of `keep_from` — for each IRI defined or shadowed-up-to that layer, mark its defining layer as reachable. Also mark `keep_from`'s descendants (everything reachable through branches that have moved past `keep_from`). Trace pins and verified-knowledge pins are still respected (a pin can keep an older layer alive even in this mode).
2. Steps 2–5 as above.

The content-tree mode is invoked manually via the CLI; it is never triggered by the background scheduler. Users who run it accept the loss of time-travel queries to layers older than `keep_from`.

**Cancellation:** the GC pass is a Tokio task; cancellation drops the in-flight sweep transaction and leaves the DB in its pre-sweep state. The `completed: false` stat distinguishes timeout from full completion.

**Background scheduling:** GC is invoked by a long-running Tokio task with three triggers (any of which fires the next pass):

- Idle trigger: no commits for `min_idle_seconds`.
- Size trigger: estimated DB size exceeds `size_threshold_bytes` and last GC ran more than 24 hours ago.
- Manual trigger: `eigenius db gc` CLI command.

**Trace pinning depth:** traces reference resources by `ResourceKey`. Whether trace pins should *transitively* pin (i.e. resources referenced by pinned resources) is an open question (§11). Default for v1: shallow pin only. Traces themselves are collectible if their parent layer is unreachable; the trace-pin set is therefore implicitly pruned by the layer-drop pass.

### 5.8 BranchPruner

```rust
pub struct PruneStats {
    pub layers_removed: u64,
    pub resources_freed: u64,
    pub bytes_freed: u64,
}

pub trait BranchPruner {
    /// Removes the branch ref and triggers immediate GC on the resulting
    /// unreachable set. Refuses if any active session is pinned to a layer
    /// in this branch's chain unless `force` is true.
    fn prune(&mut self, branch: &str, force: bool) -> Result<PruneStats>;
}
```

Pruning is GC's policy front end: it removes a branch ref from the topology, immediately triggering GC against the current root set. The expected outcome is that any layer reachable *only* from the pruned branch becomes collectible. Branch deletion (`delete` on `BranchManager`) does not trigger GC — orphaned layers stay until the next scheduled pass. Pruning is the explicit "I want this gone now" operation.

### 5.9 Indexed query reads (14h)

The three EigenQL evaluator hot spots that pre-14h scan `layer.iter_all_resources()`:

| Site | File:line | Pre-14h | Post-14h |
|---|---|---|---|
| `resolve_name_to_class_iri` | [`evaluate.rs:389`](../../kernel/src/query/evaluate.rs#L389) | linear scan | scan over index-narrowed Class candidates |
| `collect_candidates` | [`evaluate.rs:491`](../../kernel/src/query/evaluate.rs#L491) | scan + class filter | POS index scan + chain dedup |
| Negation helper | [`evaluate.rs:468`](../../kernel/src/query/evaluate.rs#L468) | scan + non-match filter | shares `collect_candidates`'s indexed path |

#### Per-layer storage (matching §5.2)

The earlier draft of this section specified per-head materialisation (`idx_*:<head>:<s>:<p>:<o>`) on the same reasoning that motivated a per-head shadowing snapshot. §5.2 then made that argument *against* itself for blooms — every layer's defined-IRI set is a property of that layer alone, so per-head bookkeeping is unnecessary if the read path can walk the chain. The triple index gets the same treatment for the same reasons:

- Branch divergence is naturally represented (each chain walks its own ancestors).
- Multi-parent merges (Phase 14e) need no index reconciliation — `collect_ancestors` walks the topology.
- No replication on every commit; each layer writes only its own diff.

Read-path cost is `O(answer × shadow_check)` instead of `O(answer)`. The shadow check piggybacks on the per-layer blooms (§5.2): bloom probes are tens of nanoseconds, so the asymmetric cost is bounded by chain depth × small constant. For typical chains (10–50 layers, occasional Phase 14e merges) this is well under a millisecond.

#### Schema

Two physical orderings, both presence-only (empty values), keyed using length-prefixed IRI segments + a fixed 32-byte `LayerId`. Encoder lives in [`kernel/src/layer/index.rs::index_keys`](../../kernel/src/layer/index.rs).

| Prefix | Purpose | Why |
|---|---|---|
| `idx_pos:<predicate>:<object>:<subject>:<layer>` | Read path | One prefix scan answers `(p, o) → {(s, defining_layer)}` across the entire DAG; chain membership and shadow check are post-filters in process memory. |
| `idx_layer:<layer>:<predicate>:<object>:<subject>` | GC path | One prefix scan per layer drop yields every entry that layer contributed; deletes both orderings in a single atomic batch. |

The reverse `idx_layer:` index doubles storage but turns GC's `delete_layer` into a clean prefix delete. Tolerable trade for a presence-only index.

POS only in v1. SPO and OPS are deferred — the three hot sites all want "subjects with predicate p and object o". Add other orderings when a workload demands them.

#### Indexability rule

A `(subject, predicate, object)` triple is indexed iff `predicate`'s `Property.data_type` resolves to `urn:eigenius:core:resource` or `urn:eigenius:core:resource_array` at the layer being committed. Same rule decides query-time eligibility — write and read paths share the [`is_indexable_predicate`](../../kernel/src/layer/index.rs) helper, so the planner deterministically picks the indexed path when the predicate is IRI-typed and the scan path otherwise. `resource_array` values unpack to one entry per element. Literal-typed properties (string, integer, boolean, embedded) bypass the index and post-filter the index-narrowed candidate set in process memory.

Schema mutation (a property's `data_type` flipped post-commit) does not trigger reindexing — each layer's entries reflect the predicate def visible at that layer's commit time. Documented limitation; manual rebuild required if a property's data_type changes class.

#### Trait

```rust
pub trait TripleIndex: Send + Sync {
    /// Insert all triples for a layer. Idempotent by `(layer, p, o, s)`.
    fn extend_layer(&self, layer: &LayerId, triples: &[Triple<'_>]) -> Result<(), StorageError>;

    /// Drop both orderings' entries for a layer. Called by GC.
    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError>;

    /// Iterate `(subject, defining_layer)` pairs matching `(p, o)`,
    /// across the entire DAG. Caller filters by chain membership and
    /// shadow-checks via the per-layer bloom cache.
    fn scan_predicate_object<'a>(
        &'a self,
        p: &Iri,
        o: &Iri,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + 'a>;

    fn stats(&self) -> IndexStats;
}
```

#### Query algorithm

[`scan_chain(head, predicate, object)`](../../kernel/src/layer/index.rs) implements:

1. Build `head`'s ancestor set via [`collect_ancestors`](../../kernel/src/layer/index.rs) (BFS over `Layer.parents`).
2. Iterate the global `idx_pos:` scan for `(predicate, object)`.
3. Filter to entries whose `defining_layer ∈ ancestor_set`.
4. For each survivor, [`is_shadowed(head, defining, subject)`](../../kernel/src/layer/index.rs) bloom-walks `head`'s ancestors that descend from `defining` (skipping `defining` itself) — first confirmed redefinition drops the candidate. Same mechanic `Layer::resolve` already uses.
5. Return the deduplicated subject set.

`collect_candidates` calls `scan_chain` once per concrete class in `{class} ∪ subclass_closure(class)`; the closure walk uses `scan_chain` recursively against the `subclass_of` predicate.

#### Atomicity

`LayerBuilder::build` populates the triple index for the freshly-built layer in process memory (mirrors how it pre-populates the bloom cache). The persistent backend's `RocksTripleIndex` writes through to the same RocksDB the layer commit uses, so by the time `store_layer`'s atomic batch commits, the index is already populated. A crash between `build` and `store_layer` leaves orphan index entries pointing at a layer that doesn't exist — they're invisible to queries (the chain-membership filter drops them) and reclaimed when `delete_layer` is called against the orphan layer (or by a future "rebuild index for layer L" tool).

#### Cost-model awareness

Deferred. The three hot sites have one bound predicate-object pair each, so there's no choice between probe orderings. A v2 cost model becomes useful once SPO/OPS land and the planner has to pick the most selective probe — D42's cardinality estimates handle that case.

## 6. Storage layout

The Phase 9a layout uses prefix-keyed entries in a single RocksDB instance. Phase 14 keeps the prefix scheme and adds new prefixes. **No column-family migration is required.** This is a deliberate choice (§4): the prefix scheme is debuggable with `rocksdb_dump`, requires no operational tuning per CF, and adds Phase 14 surfaces additively.

### 6.1 Existing prefixes (Phase 9a — preserved)

| Prefix | Key tail | Value | Notes |
|---|---|---|---|
| `layer:<id>:meta` | — | layer metadata (JSON) | preserved |
| `layer:<id>:res:<iri>` | — | resource (CBOR) | preserved |
| `chain:<id>` | — | parent layer ID hex | preserved (single-parent, multi-parent moves to topology) |
| `head` | — | current head layer ID hex | superseded by `branch:main` (see migration §9) |
| `trace:<key>` | — | ComponentTrace (JSON) | preserved |

### 6.2 New prefixes (Phase 14)

| Prefix | Key tail | Value | Purpose |
|---|---|---|---|
| `topo:<layer_id>` | — | LayerHandle CBOR | Topology entry per layer |
| `branch:<name>` | — | BranchRef CBOR | Named branch heads |
| `bloom:<layer_id>` | — | serialised BloomFilter (CBOR) | Per-layer shadowing bloom (§5.2) |
| `idx_pos:<p>:<o>:<s>:<layer>` | — | (empty) | POS triple index, read path (§5.9) |
| `idx_layer:<layer>:<p>:<o>:<s>` | — | (empty) | Reverse triple index, GC path (§5.9) |
| `gc:state` | — | GCState CBOR | Last-run timestamp, in-progress flag |
| `gc:tombstone:<layer>` | — | swept-at timestamp | Layers awaiting compaction |

The `idx_*` segments are length-prefixed, not `:`-separated, because IRIs contain `:`. SPO and OPS orderings are deferred until a workload demands them.

### 6.3 Atomic commit

Every layer commit atomically writes:

1. `layer:<new_id>:meta` and all `layer:<new_id>:res:<iri>` entries.
2. `topo:<new_id>` with the LayerHandle.
3. `bloom:<new_id>` with the layer's per-IRI shadowing bloom (§5.2).
4. `idx_pos:` and `idx_layer:` entries for each indexable `(s, p, o)` triple in the new layer (§5.9). Population happens at `LayerBuilder::build` time through the shared `Arc<dyn TripleIndex>` in `LayerStorage`; the persistent backend's `RocksTripleIndex` writes to the same RocksDB so the entries are durable by the time the layer's other content lands.
5. `branch:<branch_name>` to point at `<new_id>` (separate `update_branch` CAS — see §5.4).

Steps 1–3 are one RocksDB `WriteBatch` inside `store_layer`; RocksDB guarantees atomicity across the batch, so partial commits of layer/topology/bloom are impossible. Step 4's index writes happen via a separate `WriteBatch` driven by `RocksTripleIndex::extend_layer`. A crash between step 4 and step 1 leaves orphan index entries — invisible to queries because the chain-membership filter drops them, harmless until the next `delete_layer` call against the orphan layer reclaims them. Step 5 (branch CAS) is sequenced after the layer is durable per §5.4.

## 7. Public API

### 7.1 Rust traits (kernel-internal)

New traits in `kernel/src/storage/`:

- `LayerTopologyStore` — append-only DAG (`commit_layer` lives here, §5.4.1)
- `BloomCache` — bounded per-layer shadowing-bloom cache (§5.2)
- `BloomBackend` — persistent read/write surface for `bloom:<layer_id>` (§5.2); typically implemented by the same type as `PersistentBackend`
- `ResourceCache` — bounded two-pool content cache (§5.3)
- `BranchManager` — branch ref read/list/delete surface (§5.5); `update_branch` lives here as the CAS primitive with the trivial-merge contract from §5.4.3
- `GarbageCollector` — reachability-based mark-and-sweep (§5.7)
- `BranchPruner` — explicit branch removal + GC trigger (§5.8)
- `TripleIndex` — per-layer triple index for indexed query reads (§5.9). Trait + in-memory impl in `kernel/src/layer/index.rs`; RocksDB-backed impl in `storage/rocksdb/src/triple_index.rs`.

The trait list deliberately *excludes* a `SessionRegistry` / `PromotionService` / `ReanchorService` shape — D23's write model (§5.4) is stateless on the kernel side. Pinning is a parameter; promotion / reanchor are workflows clients orchestrate via `commit_layer` + `update_branch`. Sessions, where they exist (D21's `TaskRecord`), already hold their pin in `layer_head` and do not need new kernel infrastructure.

The `Layer` struct in `kernel/src/layer/` stops holding `BTreeMap<Iri, Resource>` directly. It becomes a thin handle that fetches via `ResourceCache`:

```rust
pub struct Layer {
    handle: LayerHandle,
    cache: Arc<dyn ResourceCache>,
    bloom_cache: Arc<dyn BloomCache>,
    backend: Arc<dyn PersistentBackend>,  // also impls BloomBackend
}

impl Layer {
    pub fn id(&self) -> LayerId { self.handle.id }
    pub fn parents(&self) -> &[LayerId] { &self.handle.parents }
    pub fn resolve(&self, branch_head: LayerId, iri: &Iri) -> Result<Option<Arc<Resource>>>;
    /// Replaces `all_resources()`; returns a streaming iterator instead of a
    /// materialised map.
    pub fn iter_resources(&self, branch_head: LayerId) -> impl Iterator<Item = (Iri, Arc<Resource>)>;
}
```

This is a breaking change to in-process kernel code. The migration is contained to the kernel crate; CLI / orchestrator / SDK don't see it.

### 7.2 gRPC additions

Per [`proto/eigenius_kernel.proto`](../../proto/eigenius_kernel.proto), additions:

- `CommitLayer(CommitLayerRequest) → CommitLayerResponse` — the §5.4.1 `commit_layer` primitive. Caller passes parent LayerId and content; kernel returns the new LayerId. No branch involvement.
- `UpdateBranch(UpdateBranchRequest) → UpdateBranchResponse` — the §5.4.1 CAS primitive with FastForward / TrivialMerge / NeedsWitnessedMerge outcomes. Used both for advancing existing branches and creating new ones (`expected_old=null` case).
- `ListBranches(ListBranchesRequest) → ListBranchesResponse` — accepts an `include_auto_named: bool` flag (default false; `db divergence list` sets it true).
- `DeleteBranch(DeleteBranchRequest) → ()`
- `PruneBranch(PruneBranchRequest) → PruneStats`
- `RunGC(RunGCRequest) → SweepStats` (admin endpoint)
- `GetCacheStats(()) → CacheStats` (observability)

Existing `Inspect`, `Query`, `Load`, `Run` RPCs gain an optional `pin: LayerId` parameter. When set, reads dispatch against that pin; when unset, the RPC reads against the current head of the target branch. `Load` and `Run` use `CommitLayer` + `UpdateBranch` internally rather than touching branches directly.

### 7.3 CLI

New subcommands:

```
eigenius db branch list                    # user-named branches only
eigenius db branch create <name> [--from <branch>]   # convenience wrapper around UpdateBranch
eigenius db branch delete <name>
eigenius db divergence list                # auto-named branches from clients that hit NeedsWitnessedMerge
eigenius db prune <name> [--force]
eigenius db gc [--max-runtime <secs>]                 # topology-DAG mode (default per §11.1)
eigenius db gc --keep-from <layer-id>                  # content-tree mode: drop layers older than <layer-id>
eigenius db cache-stats
```

Modified subcommands:

```
eigenius serve --db <path> [--cache-budget-mb <N>]
eigenius load --branch <name> [--pin <layer-id>] ...
eigenius query --branch <name> [--pin <layer-id>] [--at-layer <id>] ...
eigenius inspect --branch <name> [--pin <layer-id>] [--at-layer <id>] ...
eigenius run --branch <name> [--pin <layer-id>] ...
```

`--branch` defaults to `main` everywhere. `--pin` overrides the default of "current head of `--branch`"; useful for re-deriving against a specific historical layer. `--at-layer` is read-only time-travel (per §5.6: same resolve algorithm rooted at the target layer, no checkpoint machinery required) and is mutually exclusive with `--pin`.

## 8. Operational behaviour

### 8.1 Memory budget

Default total budget if `--cache-budget-mb` is unset:

```
min(physical_RAM × 0.5, 4 GiB)
```

Split:

- Active pool: 60% of budget.
- Historical pool: 40% of budget.
- Bloom cache: separate from the resource-cache budget; default 64 MiB, holding ~5K typical layers' blooms (§5.2.4). Configurable via `--bloom-cache-mb`.
- Topology + bloom hot pages: bounded by RocksDB's own block cache (governed by RocksDB settings, not Phase 14 config).

### 8.2 GC scheduling

Default GC config:

```
min_idle_seconds: 600          # 10 minutes
size_threshold_bytes: 1 GiB
max_runtime_seconds: 300       # 5 minutes
respect_trace_pins: true
respect_verified_pins: true
```

Operators can set `--gc-disabled` to disable background GC entirely (useful for embedded / append-only deployments).

### 8.3 Single-process concurrency

- One `eigenius serve` per DB. RocksDB enforces this at the storage layer; attempting to start a second instance against the same `--db` path errors out at RocksDB's `Open`.
- Multiple gRPC clients connect to that one server. Each call carries the LayerId it expects to extend (per §5.4); no kernel-side session state mediates between them.
- Concurrent `commit_layer` calls from different clients are independent — both succeed and produce different LayerIds; neither blocks the other.
- Concurrent `update_branch` calls against the same branch race through an in-process per-branch CAS mutex. The first wins (FastForward); the second sees the moved head and either trivially merges (if contributions are disjoint) or returns NeedsWitnessedMerge.
- On `kill -9`: the process dies; any in-flight commit that hadn't reached `WriteBatch::write` is lost; everything written through `WriteBatch::write` is durable. Layers committed by clients that hadn't yet called `update_branch` are still in storage but unreachable from any branch ref — they become collectible on the next GC pass. No coordination state to recover.
- Multi-process / multi-host deployment is out of scope for Phase 14 (would require distributed coordination beyond RocksDB's per-process write lock).

### 8.4 Time-travel queries

`--at-layer <id>` performs:

1. `PB::load_chain_from(<id>)` returns the chain metadata rooted at the target layer.
2. `build_chain` constructs the `Arc<Layer>` chain against the same bloom cache and resource cache used by the current head; the resource cache's historical pool absorbs the working-set pressure (§5.6).
3. The query executes against that chain using the standard `Layer::resolve` path (§5.2.2).

Reconstruction cost: identical to the current-head case — `O(chain_depth × bloom_check) + O(matches)`. No checkpoint replay, no separate snapshot to materialise.

## 9. Migration from Phase 9a

There are no production deployments to preserve, so migration is a dev-time convenience rather than a guaranteed property of the release. The intended workflow is: rebuild dev DBs by re-loading their source files (`.esl` / `.json`); fall back to in-place migration only if rebuilding is inconvenient.

**In-place migration (best-effort, dev-only):**

1. On Phase-14 kernel startup, check for the presence of `topo:` keys. Absent → run migration.
2. Walk the existing chain from `head` backward via `chain:<id>` pointers; for each layer L, write `topo:<L>` with `parents = [chain[L]]` and `resource_count` from a key scan.
3. Create `branch:main` pointing at the current head.
4. For each layer L, compute its shadowing bloom from the per-layer defined-IRI set and write `bloom:<L>`.
5. Build the triple indexes by walking each layer's resources and emitting (s, p, o) entries.
6. Mark migration complete in metadata.

If migration fails or behaves oddly, the recovery is `rm -rf <db>` and re-load. We do not promise atomicity, partial-resume, or rollback for the migration pass — those become deliverables only when there's a production deployment to protect.

**Refusal semantics:** unchanged from D13. A pre-Phase-9a DB (no manifest) still gets drift-refusal.

## 10. Test plan

Per sub-milestone, with success criteria:

| Milestone | Test surface | Pass criterion |
|---|---|---|
| 14a | Unit tests for `LayerHandle`, `ResourceCache`, `Layer::resolve` | Resolve walks topology + cache; cache miss falls through to backend; correctness preserved against current behaviour |
| 14b | Unit tests for `BloomFilter` build/might_contain/FPR; `BloomCache` get/put/evict; `Layer::resolve` cold-cache path; integration test on commit path | Resolve probes the matching layer plus ≤ expected bloom false positives over the chain; commit atomically writes `bloom:<id>` alongside topology + content |
| 14c | Unit tests for `TwoPoolCache`; benchmark on a simulated workload | Hit rate within 5% of analytical optimum on a Zipfian access pattern; promotion/demotion behaves per spec |
| 14d | Integration tests for branch CRUD; end-to-end test of two-branch divergence | Create / list / delete work atomically; reads from sibling branch return that branch's view |
| 14e | Concurrent integration tests with two `serve` instances | Cross-branch writes refused with clear error; cross-branch reads succeed; lease handoff after kill -9 works within blackout window |
| 14f | GC unit tests with synthetic root sets; integration test with pinned trace | Mark-and-sweep produces correct unreachable set; trace-pinned resources survive GC; cancellation leaves DB intact |
| 14g | Branch-prune integration test | Pruning a branch with no other reachers frees expected resources; pruning with active session refused without `--force` |
| 14h | EigenQL regression suite + new index tests | All existing EigenQL tests pass against indexed reads; new tests confirm `EXPLAIN`-style index selection |

Cross-cutting tests:

- **Migration test:** ship a Phase-9a DB fixture; load with Phase-14 kernel; confirm migration completes and all queries return identical results.
- **Restart correctness:** kill -9 the kernel mid-commit; restart; confirm topology + indexes + branches all consistent.
- **Stress test:** 10 sessions on 5 branches under sustained read+write load for 1 hour; no corruption, GC runs to completion at least once.

## 11. Open questions and Phase 15 dependency

### 11.1 GC reachability model: topology-DAG default + explicit content-tree escape valve

**Decision (2026-04-27):** Phase 14 ships **topology-DAG reachability as the default** GC model. The content-tree alternative (Irmin/Tezos style) is exposed as an explicit `eigenius db gc --keep-from <layer>` operation for users who want to compact aggressively after experimentation.

| Approach | What's reachable | What's preserved | Cost |
|---|---|---|---|
| **Topology-DAG (default)** | Walk parents in the layer DAG from every branch head + session pin + trace pin. Every layer transitively reachable through `parents` is preserved. | Full time-travel anywhere along any branch's history, indefinitely. | Storage grows monotonically with history; long-running DBs accumulate "shadowed-but-still-pinned-via-parents" layers forever. Bounded only by trace pins / verified claims / branch refs going stale. |
| **Content-tree (explicit `--keep-from <layer>`)** | Walk only the *content tree* of the chosen layer. Parent references between commits are *not* followed. Layers older than `<layer>` are dropped wholesale. | Time-travel within the explicit retention window. | Aggressive compaction; loss of deep historical queries. |

**Why this is right for Eigenius:** the epistemic model frames derived results as replayable. A trace recorded "input was IRI X at L_pin" — for replay to work, X at L_pin has to remain reachable. Under topology-DAG that's automatic (the layer is preserved by virtue of being an ancestor of any reachable head). Under content-tree it would require explicit trace-pin roots, with all the bookkeeping fragility that implies. Topology-DAG aligns with our default semantics; content-tree is the user's deliberate choice when they want to reclaim disk after experimentation, accepting the loss of historical depth.

**Implementation impact (Phase 14f):** the GC mark phase walks the topology DAG from `GCRoots.branch_heads + session_pins`, plus follows resource references from `trace_pins` and `verified_pins`. The `--keep-from <layer>` mode invokes a different mark phase: walk only the chosen layer's content tree (resources defined or shadowed-up-to that layer), drop everything else. Both share the sweep mechanism.

### 11.2 Phase 15 dependency: witnessed merge unblocks the residual case

Phase 14 ships trivial merge (§5.4.3) — concurrent activity that touches disjoint IRI sets is reconciled automatically inside `update_branch`. This handles the majority of real-world divergence in dev workflows: two notebooks editing different parts of an ontology, a long-running task while a CLI loads fresh data, etc.

It does not handle conflicts where two branches modify the same IRI in incompatible ways. Pre-Phase-15, those surface as `NeedsWitnessedMerge` outcomes that the caller must resolve manually:

- Save the would-be-merged chain as a sibling branch (`update_branch("auto-...", expected_old=None, ...)`) and revisit later.
- Discard the chain and re-derive against the current head.
- Defer (keep the LayerId in client state; wait for Phase 15).

This is a usability constraint Phase 14 ships under, not a deferrable enhancement. Heavy concurrent ontology evolution (multiple authors editing the same classes; cross-cutting refactors) will accumulate `auto-*` branches that can only be pruned, not consolidated, until Phase 15's witnessed merge lands. Phase 15 is therefore a high-priority follow-on rather than a "future work" item.

### 11.3 Decisions deferred to implementation

Defaults are noted; benchmark or operational data is the natural trigger for revisiting:

1. **ARC vs. CLOCK-Pro vs. LRU.** ARC is the default; benchmark before committing.
2. **Per-layer bloom FPR.** 1% is the default at commit time (§5.2.6). Tune globally via config if a heavy-resolve workload demonstrates value; lower FPR trades disk for fewer spurious probes.
3. **Bloom cache budget.** 64 MiB is the starting point (§5.2.4). Workloads with deep chains across many simultaneously-active heads may want more.
4. **GC trigger composition.** Default supports idle + size + manual triggers (any-of). Decide whether to add a quota-style trigger ("auto-run once GC reclaim potential exceeds X bytes") based on observed behaviour.
5. **Trace pinning depth.** Default: shallow (resources directly referenced by traces). Decide whether to extend to transitive after observing trace-graph shapes.
6. **Roll-up index for pathological chains.** §5.2.7 notes that 10⁴+ deep chains or negative-resolve-heavy workloads may want a periodically-materialised "as of layer L, here is `iri → defining_layer`" index alongside the per-layer blooms. Defer until a workload demands it.
7. **Cache stats observability.** What to expose via `eigenius db cache-stats` and gRPC's `GetCacheStats`. Default: hit rate, byte counts per pool, promotions/demotions; add more on user request.
8. **Default branch name.** `main` (matches modern Git convention). Configurable via `EIGENIUS_DEFAULT_BRANCH`.
9. **Trivial-merge IRI-set computation.** The diff `[L_anc, L_new]` and `[L_anc, L_other]` is naively `O(layers × per-layer-IRIs)`. For deep chains (long sessions producing many layers) this could be expensive; possible optimisation is a per-chain summary index. Default: compute on demand; add the summary if profile data shows it matters.
10. **Auto-branch expiration.** Auto-named sibling branches from `NeedsWitnessedMerge` outcomes accumulate; should they auto-expire after N days idle? Default for v1: never; require explicit prune. Revisit if `db divergence list` becomes cluttered in practice.

## 12. Sub-milestone sequencing and dependencies

Implementation order (see [implementation-plan.md §Phase 14](implementation-plan.md#phase-14--out-of-core-layer-architecture) for estimates):

```
14a topology/content split  ──┐
                              ├─→ 14b per-layer bloom   ──┐
                              │                          ├─→ 14c two-pool cache
                              │                          │
                              │                          ├─→ 14d commit_layer + update_branch
                              │                          │       (CAS, FastForward outcome only)
                              │                          │       │
                              │                          │       └─→ 14e trivial merge in update_branch
                              │                          │               + branch read/list/delete surface
                              │                          │               + small D21 TaskRecord outcome field
                              │                          │
                              │                          ├─→ 14f GC ──→ 14g pruning
                              │                          │
                              │                          └─→ 14h indexed reads
                              │
                              └─→ migration (parallel; ships with first 14a release)
```

14a is the prerequisite for everything; 14b is the prerequisite for everything that does lookups against the cache. 14d implements `commit_layer` and `update_branch` with CAS (fast-forward only); 14e adds the trivial-merge logic to `update_branch` and the read-side branch surface. Splitting 14d/14e this way lets the simpler CAS-only path land first as a usable milestone before the trivial-merge work begins. 14c, 14f-g, and 14h are independent of 14d-e and can be parallelised.

## 13. Related work and external references

The combination of features in Phase 14 — branched immutable storage with shadowing, content-addressed layers, reachability-based GC, time-travel queries, two-pool caching, trivial merge — is unusual but the individual pieces are well-trodden. The references below are the directly-relevant design influences and engineering touchstones; they are not background reading, they are *systems whose decisions we should weigh against ours before locking in implementation choices*.

### 13.1 Closest single-system analog: Irmin

[**Irmin**](https://irmin.org/) (OCaml, used by [Mirage](https://mirageos.org/) and [Tezos](https://tezos.com/)) is the closest existing system to what we are building. Content-addressed, Git-like, branched, with composable per-type merge functions and reachability-based GC. The Tarides "[Introducing irmin-pack](https://tarides.com/blog/2020-09-01-introducing-irmin-pack/)" post is the most useful single document for our purposes — it lays out the three-component storage architecture and the index design that informs §5.2 and §6.

Mapping Irmin's components onto our design:

| Irmin component | Our equivalent | Note |
|---|---|---|
| **Pack file** — append-only blob storage; each entry = serialized data + length + hash; **internal references use offsets, not hashes** | Our `layer:<id>:res:<iri>` entries in RocksDB. We use `LayerId` (hash) for parent refs in the topology. | Irmin's offset-based parent refs avoid an index lookup per DAG traversal. Worth considering for `LayerHandle.parents` (see refinement §13.5). |
| **Dict** — bidirectional hash table mapping path strings to short integer IDs; persisted via append-only file; ~15 MB at Tezos scale | We don't currently have this. | Our IRIs are long URI strings; a dict would substantially compress storage and speed up lookups. See refinement §13.5. |
| **Index** — hash → `(offset, length)` map; **two-tier**: bounded `log` (recent bindings, in-memory + WAL) + larger sorted `data` (historical bindings, on-disk with interpolation search); background merge log → data when the log fills | Not adopted. Our §5.2 uses per-layer blooms + cache instead of a per-head global index, sidestepping the problem Irmin's two-tier design solves. | The Irmin approach is well-suited to a per-head shadowing index (the shape this doc originally proposed); per-layer blooms make it unnecessary. See §13.5 for why we declined to import it even when we had a per-head index. |
| **Merge module** — composable per-type 3-way merge functions (`Merge.option`, `Merge.idempotent`, `Merge.unique`, etc.) | Our `update_branch` trivial-merge logic (§5.4.3) is the disjoint-IRI generalization of these. | Phase 15's witnessed merge can borrow Irmin's primitive vocabulary; pending detailed read of the Merge API. |

**Headline number from production:** Tezos saw **250 GB → 25 GB** (~10×) compression switching to irmin-pack from earlier backends, with comparable query performance in memory-constrained environments. The dict and the offset-based pack format together account for most of this.

**What's notably missing from the public Irmin docs:** garbage collection. The "Introducing irmin-pack" post does not cover it; we need to dig further into Tarides' [Irmin tag](https://tarides.com/blog/tag/irmin/) to find the GC strategy. Captured as follow-up §13.6.

### 13.2 Adjacent "Git for data" systems

- [**Dolt**](https://github.com/dolthub/dolt) — Git for relational data. Branch / merge / diff over tabular records. Open-source Go implementation. Their merge-conflict surface is closer to what we need than git's because it operates on structured data with primary keys (analogous to our IRIs).
- [**TerminusDB**](https://terminusdb.com/) — Git for typed graph data. Closer in shape to Eigenius (typed graph, branch / merge, time-travel). Their [time-travel docs](https://terminusdb.com/docs/) cover the per-branch versioning case we hit in §5.6.
- [**Noms**](https://github.com/attic-labs/noms) (defunct) — content-addressed decentralized database with Prolly trees as the indexing structure. Several published papers; same DAG-of-immutable-snapshots model.

### 13.3 Layer shadowing precedent

The "stack of immutable layers, top-most wins" pattern is identical to **container image layers** and **union filesystems**:

- **OverlayFS** (Linux kernel) — refined "whiteout files" for explicit shadowing; the cleanest implementation of layered shadowing in production code.
- **Docker image layers** — same logical structure; `IMAGE LAYERS` are content-addressed, parent-pointed, immutable.
- **OSTree** / `rpm-ostree` — content-addressed file system layers with named refs; closest to our LayerId + branch-ref model.
- **NixOS / Guix store paths** — content-addressed, with profile generations forming a graph and `nix-store --gc-roots` defining reachability.

### 13.4 Cache architecture and time-travel queries

**Cache:**

- **ARC (Megiddo & Modha 2003)** — [the canonical paper](https://www.usenix.org/legacy/events/fast03/tech/full_papers/megiddo/megiddo.pdf). The default replacement algorithm for our two-pool design (§5.3).
- **Caffeine** — [Java cache library](https://github.com/ben-manes/caffeine) using W-TinyLFU. Best-in-class production cache; the [design notes and benchmark methodology](https://github.com/ben-manes/caffeine/wiki/Design) are pedagogical regardless of implementation language. Worth reading before committing to ARC over W-TinyLFU.
- **2Q (Johnson & Shasha 1994)** — original two-queue design; maps almost directly onto our active/historical pool split. Postgres' buffer manager is roughly 2Q-shaped if you want a production example.

**Time-travel:**

- **Datomic** — Rich Hickey's "[Datomic Architecture](https://www.youtube.com/watch?v=5GMGGjvSIqg)" talk and the [architecture page](https://docs.datomic.com/cloud/whatis/architecture.html). The most mature production example of "the database remembers everything" with `as-of` semantics. Their per-tx covering indexes solve the same problem our per-layer blooms (§5.2) do, with a different trade-off: O(1) lookup at the cost of a per-tx index. We accepted O(chain_depth × bloom_check) to keep shadowing as a per-layer immutable artifact rather than a per-head structure.
- **XTDB** (formerly Crux) — open-source bitemporal database with similar time-travel semantics.

### 13.5 Irmin mechanisms considered and not adopted

The mechanical optimisations Irmin uses are heavily shaped by their substrate: they sit directly on POSIX I/O, so they had to build their own LSM, their own dict, their own offset-based pack format. We sit on RocksDB, which is itself a tuned LSM, so most of those mechanisms either duplicate what RocksDB already gives us or solve problems we don't have. The conceptually-load-bearing pieces of the Irmin reading (merge composability, GC reachability semantics, branch/ref separation) are captured elsewhere in this section and in §11.1 / §13.6; the mechanism-level borrowings below were considered and dropped.

1. **IRI dictionary** — *deferred as possible future optimization.* RocksDB's LZ4 compression already handles the bulk of IRI repetition; an explicit dict adds maybe 1.5–2× on top at the cost of meaningful concurrency and atomicity complexity. No production data points at this as a bottleneck. Revisit only if storage size becomes a measured constraint; doing it later means rewriting resources, but in our dev-only mode the recovery is `rm -rf <db>` and re-load.

2. **Two-tier shadowing index** — *not applicable.* Irmin's `log + data + background merge` design solves the per-head global-index lookup problem. With per-layer blooms (§5.2) we don't have a per-head global index in the first place — each layer carries its own immutable bloom, paged through `BloomCache`. The two-tier design's complexity buys nothing in the per-layer model.

3. **Offset-based parent refs** — *dropped, premise does not apply.* The motivation was "avoid an index lookup per DAG walk," but our topology (§5.1) is a fully in-memory `BTreeMap<LayerId, LayerHandle>` — parent traversal is already an in-memory map probe with no RocksDB read involved. Irmin needs offsets because they don't keep the topology in memory; we do. Removed entirely rather than deferred.

### 13.6 Findings from the Irmin GC and Merge follow-ups

**Irmin's GC architecture** (sources: Tarides "[Towards Minimal Disk-Usage for Tezos Bakers](https://tarides.com/blog/2022-11-10-towards-minimal-disk-usage-for-tezos-bakers/)", "[Optimising Archive Node Storage for Tezos](https://tarides.com/blog/2023-05-05-optimising-archive-node-storage-for-tezos/)", and Nomadic Labs "[Pruning the context — and other seasonal activities](https://research-development.nomadic-labs.com/pruning-the-context-and-other-seasonal-activities.html)"):

1. **Mark-and-sweep variant** — confirms our §5.7 algorithm choice.
2. **Critical design decision: GC ignores commit→parent references when computing reachability.** The user picks a "GC-commit" root; the GC keeps only objects reachable from that commit's content tree, *not transitively all ancestors*. Layers older than the GC-commit are dropped wholesale. This is materially different from our current §5.7 spec and surfaces a real choice we need to make (see §11.3).
3. **Async, out-of-process GC with atomic switch.** A worker process builds new "prefix" + "suffix" file structures while the main node continues operating; atomic switch when done. Read-only nodes synchronise via a single control file. Avoids GC pauses. Our §5.7 currently spec's "Tokio background task" — the worker-process pattern is more refined and isolated.
4. **Prefix/suffix storage layout.** The "prefix" stores compacted reachable objects from the past; the "suffix" contains the GC-commit root and everything after. Virtual offsets (original positions) translate to real offsets (new positions) via a persistent mapping table. Doesn't map directly onto our RocksDB key-value layout, but the conceptual "compact recent reachable into a tight contiguous structure" idea informs our compaction triggers.
5. **Two operating modes.** *Rolling nodes* actually delete old data (window-based pruning). *Archive nodes* preserve all history by moving data to "lower volumes" with compaction instead of deleting. Roughly corresponds to "GC-enabled" vs "GC-disabled" in our model — but archive mode is genuinely different, since it compacts without losing history.
6. **Production numbers (Tezos):** 140 GB → 30-60 GB (2-4× reduction) on rolling nodes, on top of irmin-pack's already 10× compression. First-run GC on long-history nodes is slow; memory spikes during pruning; current implementation temporarily doubles disk footprint during the GC pass (working on reducing it).

**Irmin's Merge module API** (source: [`Irmin.Merge` docs](https://mirage.github.io/irmin/irmin/Irmin/Merge/index.html)):

Core type: `'a f = old:'a promise -> 'a -> 'a -> ('a, conflict) result`. Three-way merge: ancestor + v1 + v2 → result-or-conflict.

Primitive combinators:
- `default` — accepts changes in one branch only; conflict if both modified differently.
- `idempotent` — accepts identical changes from both branches; otherwise behaves like `default`.
- `option` — lifts a merge to optional types, treating `None` specially.
- `pair`, `triple` — compose merges for tuples by merging each component independently.
- `like` / `like_lwt` — transform between domains via a converting function (lets a merge for type A be reused for type B given an iso).
- `counter` — preserves increment/decrement semantics across concurrent modifications.
- `alist`, `Map` — merge maps by merging values at identical keys; track adds/removes.
- `seq` — try multiple merges in order, stopping at the first success.

**Mapping onto our design:**

- Our **trivial merge** (§5.4.3) corresponds to **`default + idempotent`** at the IRI level: same modification on both sides → accept; one-sided modification → accept; conflicting modifications → return `NeedsWitnessedMerge`. This is the simplest possible composition; we should adopt the same vocabulary internally.
- **Phase 15's witnessed merge** should follow Irmin's combinator pattern: per-class merge functions defined by institutions (since classes are domain-specific), generic combinators for sums/products/options derived from our type theory, the `Comorphism` witness as the per-class custom merge for institutional types.
- The `seq` combinator suggests a useful Phase 15 pattern: try increasingly specific witnesses (per-IRI overrides → per-class default → cross-class fallback), fall back to manual conflict surfacing if none match.

**Still on the follow-up list (lower priority):**

- **Datomic's index design** — specifically the per-tx covering index trees. The closest external precedent for an O(1) per-head shadowing structure; worth understanding if §5.2's per-layer blooms ever prove inadequate and we revisit a per-head materialised index (the §5.2.7 roll-up option).
- **Nix store GC** — [`nix-collect-garbage` documentation](https://nixos.org/manual/nix/stable/package-management/garbage-collection.html). Same shape as our reachability-based GC (§5.7), with the "long-lived references pinning large subgraphs" case handled explicitly — directly relevant to our trace-pinning question.

## 14. References

- D13 — Durable Kernel State (the atomicity guarantees Phase 14 inherits via `WriteBatch`)
- D21 — Task Traces and Checkpointing (`TaskRecord.layer_head` is already the per-task pin Phase 14 builds on)
- D2 — EigenQL Specification (the operator surface that §5.9 preserves)
- D6b — Reasoning Trace Schema (the trace structure that §5.7's pinning rules respect)
- D20 (forthcoming, Phase 15) — Layer Reconciliation via Comorphisms (the witnessed-merge case Phase 14's trivial merge complements)
- D42 (forthcoming, Phase 16) — Out-of-Core Query Execution

Source code touchpoints (entering Phase 14):

- [`kernel/src/layer/mod.rs`](../../kernel/src/layer/mod.rs) — Layer struct (becomes a handle in 14a)
- [`kernel/src/context/mod.rs`](../../kernel/src/context/mod.rs) — ExecutionContext (read APIs gain explicit pin parameter in 14d)
- [`storage/rocksdb/src/lib.rs`](../../storage/rocksdb/src/lib.rs) — RocksStore (extended with new prefixes throughout)
- [`kernel/src/layer/index.rs`](../../kernel/src/layer/index.rs) — `TripleIndex` trait + `MemoryTripleIndex` + chain-walk helpers (Phase 14h)
- [`storage/rocksdb/src/triple_index.rs`](../../storage/rocksdb/src/triple_index.rs) — RocksDB-backed `TripleIndex` (Phase 14h)
- [`kernel/src/task/mod.rs`](../../kernel/src/task/mod.rs) — TaskRecord (gains a small outcome field in 14e)
- [`kernel/src/query/evaluate.rs`](../../kernel/src/query/evaluate.rs) — pattern-matching evaluator (rewritten in 14h)
- [`kernel/src/storage/mod.rs`](../../kernel/src/storage/mod.rs) — PersistentBackend trait (extended for the new prefixes)
