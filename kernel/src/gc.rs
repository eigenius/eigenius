// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Reachability-based garbage collection (D23 §5.7 / Phase 14f).
//!
//! Mark-and-sweep over the layer DAG. Roots are the layers that the
//! caller declares "load-bearing right now" — branch refs and the
//! `TaskRecord.layer_head` pin of every live task. Anything not
//! transitively reachable from those roots is candidate for sweep.
//!
//! ## Concurrency contract
//!
//! GC runs concurrently with `commit_layer` / `update_branch` /
//! `merge_independent_heads` in the same kernel process. Two
//! mechanisms keep them coherent:
//!
//! 1. **Snapshot under the branch lock.** [`collect`] takes the
//!    branch lock briefly via [`crate::lattice::with_branch_lock`] to
//!    read all branch refs atomically — no `update_branch` is in
//!    flight while roots are being gathered. The lock is released
//!    before mark + sweep begin; concurrent commits are safe via (2).
//! 2. **Minimum age before sweep.** Layers younger than
//!    [`GcConfig::min_age_seconds`] (default 60) are skipped during
//!    sweep regardless of reachability. This protects the brief
//!    window between `commit_layer` returning and the caller invoking
//!    `update_branch` (or registering the layer in a `TaskRecord`).
//!
//! ## Caller contract
//!
//! Layers are protected from GC if they're reachable from a branch
//! ref or from a `TaskRecord.layer_head` pin. Layers committed
//! without such a reference within `gc_min_age_seconds` may be
//! reclaimed. Workflows that need long-lived unpublished layers
//! (manual-review, multi-step staging) should publish to an `auto-*`
//! branch immediately — that's a root pin and keeps the layer alive
//! indefinitely until the branch is pruned.
//!
//! ## Failure mode
//!
//! Visible, not silent. If a caller waits longer than
//! `min_age_seconds` between `commit_layer` and `update_branch`, the
//! layer may be reclaimed and `update_branch` will fail with a
//! storage error (parent not found in topology). The right caller
//! response is to retry against a fresh head.
//!
//! ## What's NOT in 14f-i
//!
//! - Background scheduling (idle-trigger, size-trigger). For 14f-i,
//!   GC is invoked explicitly via [`collect`]. Triggers land in 14f-ii.
//! - Trace-pin / verified-knowledge-pin roots. Tasks pin via their
//!   `TaskRecord.layer_head` (already a root); reflection traces and
//!   verified claims that reference specific (layer, iri) pairs need
//!   their own root surface, deferred to a follow-up.
//! - `ContentTree` mode (D23 §5.7's `--keep-from`). `TopologyDAG` is
//!   the default and only mode in 14f-i; aggressive compaction
//!   follows when there's a workload to justify it.

use crate::layer::{BloomCache, LayerId, LayerTopology, RedirectEntry, ResourceCache};
use crate::observability::{field, operation};
use crate::storage::{PersistentBackend, StorageError};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

/// Roots from which reachability is computed. Anything transitively
/// reachable through `LayerHandle.parents` from any layer in any of
/// these vectors is preserved.
#[derive(Debug, Clone, Default)]
pub struct GcRoots {
    /// Branch heads (typically populated via
    /// `PersistentBackend::list_branches`).
    pub branch_heads: Vec<LayerId>,
    /// Layers pinned by tasks' `TaskRecord.layer_head` field.
    /// Caller-supplied — the kernel doesn't enumerate sessions.
    /// A typical caller iterates known sessions, calls
    /// `TaskStore::list_tasks(session)`, and collects each
    /// record's `layer_head`.
    pub task_pins: Vec<LayerId>,
    /// Tag targets (D34 §G.2 / §8.3). Tags are GC roots — protecting
    /// their target (and its ancestors) for as long as the tag
    /// exists is what makes "tag this state so I can come back to
    /// it later" actually durable.
    pub tag_targets: Vec<LayerId>,
}

impl GcRoots {
    /// Build a roots set from the persistent backend's branch refs
    /// and tag refs. Task pins must be added separately by the
    /// caller.
    pub fn from_branches(backend: &dyn PersistentBackend) -> Result<Self, StorageError> {
        let branches = backend.list_branches()?;
        let tags = backend.list_tags()?;
        Ok(Self {
            branch_heads: branches.into_iter().map(|(_, id)| id).collect(),
            task_pins: Vec::new(),
            tag_targets: tags.into_iter().map(|(_, id)| id).collect(),
        })
    }

    /// Iterator over every layer id that should be treated as a root.
    fn iter(&self) -> impl Iterator<Item = &LayerId> {
        self.branch_heads
            .iter()
            .chain(self.task_pins.iter())
            .chain(self.tag_targets.iter())
    }
}

/// Tunables for a `collect` call.
#[derive(Debug, Clone)]
pub struct GcConfig {
    /// Layers younger than this are skipped during sweep regardless
    /// of reachability. Protects the `commit_layer` → `update_branch`
    /// window. Default 60 s.
    pub min_age: Duration,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            min_age: Duration::from_secs(60),
        }
    }
}

/// Counters returned from a `collect` call.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepStats {
    /// Number of layers walked during the mark phase (i.e., reachable
    /// from any root).
    pub layers_marked: u64,
    /// Number of layers identified as unreachable.
    pub layers_unreachable: u64,
    /// Number of unreachable layers actually deleted (i.e., excluding
    /// those skipped because they were younger than `min_age`).
    pub layers_swept: u64,
    /// Number of layers that were unreachable but skipped due to
    /// `min_age` protection.
    pub layers_protected_by_age: u64,
    /// Sum of `LayerHandle.byte_size` over the eligible set:
    /// - For `estimate`: bytes that *would* be reclaimed by a real
    ///   sweep right now (unreachable AND past `min_age`).
    /// - For `collect`: bytes actually reclaimed (same set after the
    ///   sweep).
    ///
    /// Approximation: counts encoded resource bytes per layer. The
    /// per-layer bloom, topology entry, chain pointer, content-hash
    /// index, and triple-index entries are bounded per-layer
    /// overhead and not included — see `LayerHandle::byte_size` for
    /// the rationale.
    pub bytes_reclaimable: u64,
}

/// Run a single mark-and-sweep pass over the layer DAG.
///
/// **Algorithm:**
///
/// 1. Snapshot branch refs and load topology under the branch lock —
///    no `update_branch` is in flight during this step.
/// 2. Mark phase: BFS from `roots` over `LayerHandle.parents`,
///    collecting the reachable set.
/// 3. Sweep phase: every layer in the topology not in the reachable
///    set is unreachable. Per-layer: if the layer's `created_at` is
///    older than `now - config.min_age`, atomically delete via
///    `PersistentBackend::delete_layer` and notify the caches via
///    `evict_layer`. Layers younger than `min_age` are skipped (see
///    module docs for the contract).
///
/// Returns counters describing what happened. Errors abort the pass —
/// any partial sweep that occurred before the error remains in the
/// store; a future `collect` call will pick up where this one left
/// off (mark phase is recomputed; idempotent).
pub fn collect(
    roots: GcRoots,
    config: &GcConfig,
    cache: &dyn ResourceCache,
    bloom_cache: &dyn BloomCache,
    backend: &dyn PersistentBackend,
) -> Result<SweepStats, StorageError> {
    // Step 1: snapshot under the branch lock. Topology + redirects
    // are loaded together so the (branches + topology + redirects)
    // triple is mutually consistent — every branch head exists in
    // the topology snapshot and every redirect's source layer is
    // present in (or manufacturable from) the topology.
    let load_started = Instant::now();
    let (topology, redirects): (LayerTopology, Vec<RedirectEntry>) =
        crate::lattice::with_branch_lock(|| {
            let topo = backend.load_topology()?;
            let redirects = backend.list_redirects()?;
            Ok::<_, StorageError>((topo, redirects))
        })?;
    emit_load_topology_metrics(&topology, load_started.elapsed());

    // Step 2: mark phase. BFS from roots through topology.parents
    // with redirect-following per D25 §12.8.1(d).
    let mark_started = Instant::now();
    let reachable = mark_reachable(&roots, &topology, &redirects);
    emit_mark_metrics(reachable.len() as u64, mark_started.elapsed());

    // Step 3: sweep phase. Iterate every layer in the topology; if
    // not in reachable and old enough, delete. Counters tally what
    // happened.
    let sweep_started = Instant::now();
    let now_ms = current_time_millis();
    let min_age_ms = config.min_age.as_millis() as i64;
    let mut stats = SweepStats {
        layers_marked: reachable.len() as u64,
        ..Default::default()
    };

    // Topology is a `BTreeMap` internally — no public iter API. We
    // walk via `walk_chain` from each root we know plus a manual
    // pass over every key. For 14f-i, simplest is: also collect "all
    // layer ids" by listing topology layers via the topology API.
    // We exposed `iter_layers` for this purpose below.
    for handle in topology.iter_layers() {
        if reachable.contains(&handle.id) {
            continue;
        }
        stats.layers_unreachable += 1;
        let age_ms = now_ms.saturating_sub(handle.created_at);
        if age_ms < min_age_ms {
            stats.layers_protected_by_age += 1;
            continue;
        }
        // Atomic delete + cache eviction. The delete is per-layer;
        // failure propagates so a partial pass is visible.
        backend.delete_layer(&handle.id)?;
        cache.evict_layer(&handle.id);
        bloom_cache.evict_layer(&handle.id);
        stats.layers_swept += 1;
        stats.bytes_reclaimable += handle.byte_size;
    }
    emit_sweep_metrics(&stats, sweep_started.elapsed());

    Ok(stats)
}

/// Read-only preview of a `collect` pass. Same root snapshot + mark
/// walk + age classification, *no deletes*. Powers the notebook's
/// GC panel Step 1 (D34 §9.4) — operators want to know how many
/// layers a `RunGc` will sweep before they commit to it.
///
/// `layers_swept` on the returned stats is always 0; `layers_marked`,
/// `layers_unreachable`, and `layers_protected_by_age` carry the
/// same meaning as the corresponding `collect` fields. The
/// "eligible_layers" the RPC surfaces is
/// `layers_unreachable - layers_protected_by_age`.
pub fn estimate(
    roots: GcRoots,
    config: &GcConfig,
    backend: &dyn PersistentBackend,
) -> Result<SweepStats, StorageError> {
    let load_started = Instant::now();
    let (topology, redirects): (LayerTopology, Vec<RedirectEntry>) =
        crate::lattice::with_branch_lock(|| {
            let topo = backend.load_topology()?;
            let redirects = backend.list_redirects()?;
            Ok::<_, StorageError>((topo, redirects))
        })?;
    emit_load_topology_metrics(&topology, load_started.elapsed());

    let mark_started = Instant::now();
    let reachable = mark_reachable(&roots, &topology, &redirects);
    emit_mark_metrics(reachable.len() as u64, mark_started.elapsed());

    let sweep_started = Instant::now();
    let now_ms = current_time_millis();
    let min_age_ms = config.min_age.as_millis() as i64;
    let mut stats = SweepStats {
        layers_marked: reachable.len() as u64,
        ..Default::default()
    };
    for handle in topology.iter_layers() {
        if reachable.contains(&handle.id) {
            continue;
        }
        stats.layers_unreachable += 1;
        let age_ms = now_ms.saturating_sub(handle.created_at);
        if age_ms < min_age_ms {
            stats.layers_protected_by_age += 1;
            continue;
        }
        // Eligible (unreachable AND past min_age) — accumulate the
        // bytes that a real sweep would reclaim.
        stats.bytes_reclaimable += handle.byte_size;
    }
    // `estimate` doesn't delete; the sweep phase here is just the
    // classification loop. We still emit the metric so dashboards see
    // the same per-phase shape as `collect` for trend comparison.
    emit_sweep_metrics(&stats, sweep_started.elapsed());
    Ok(stats)
}

/// BFS reachability over `LayerHandle.parents`, augmented for
/// D25 §12.8 forward-pointer consolidation.
///
/// When the walk encounters a layer that's a redirect *source*, the
/// behavior depends on the redirect's `preserve_history` flag:
///
/// - `false` (default reclaim mode): mark the redirect's target
///   (and follow ITS parents transitively) so `L_c`'s ancestor
///   closure stays alive, but *skip* the source's own
///   `LayerHandle.parents`. The consolidated source-side chain
///   becomes unreachable from head-rooted marks and is eligible
///   for sweep on this pass.
///
/// - `true` (preserve-history mode): mark the redirect's target
///   *and* walk the source's parents. Both the consolidated layer
///   and the original source chain stay alive — time-travel reads
///   against intermediate layers in the range continue to resolve.
///
/// Either way, the redirect entry itself persists in storage and the
/// in-memory topology continues to manufacture a synthetic tombstone
/// for the source (D25 §12.8.1(d)) so future load_topology calls see
/// the consistent DAG.
fn mark_reachable(
    roots: &GcRoots,
    topology: &LayerTopology,
    redirects: &[RedirectEntry],
) -> BTreeSet<LayerId> {
    // Index redirects by source for O(1) probe during the walk.
    let redirect_index: BTreeMap<LayerId, &RedirectEntry> = redirects
        .iter()
        .map(|entry| (entry.source().clone(), entry))
        .collect();

    let mut reachable: BTreeSet<LayerId> = BTreeSet::new();
    let mut queue: VecDeque<LayerId> = VecDeque::new();
    for r in roots.iter() {
        queue.push_back(r.clone());
    }
    while let Some(id) = queue.pop_front() {
        if !reachable.insert(id.clone()) {
            continue;
        }

        // Redirect-aware fork: if this layer is a redirect source,
        // mark the target chain unconditionally, and skip the source's
        // own parents unless preserve_history was set at install time.
        if let Some(entry) = redirect_index.get(&id) {
            if !reachable.contains(&entry.target) {
                queue.push_back(entry.target.clone());
            }
            if !entry.preserve_history {
                continue;
            }
        }

        if let Some(handle) = topology.get_layer(&id) {
            for parent in &handle.parents {
                if !reachable.contains(parent) {
                    queue.push_back(parent.clone());
                }
            }
        }
        // Unknown ids (in roots but not in topology) are silently
        // ignored. This can happen if a caller's task pin references
        // a layer that was already swept by a prior pass — defensive,
        // not a panic.
    }
    reachable
}

fn current_time_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Emit the per-pass topology metrics. `count` is the layer count;
/// `size_bytes` is the sum of every handle's `byte_size` (encoded
/// resource bytes across the topology — a proxy for "how much state
/// the chain has accumulated"). In-memory cost is proportional but
/// smaller per handle; if a dashboard needs exact RSS, infer it from
/// `count` (struct overhead is bounded per handle).
fn emit_load_topology_metrics(topology: &LayerTopology, elapsed: Duration) {
    let layer_count = topology.layer_count() as u64;
    let handle_bytes_total: u64 = topology.iter_layers().map(|h| h.byte_size).sum();
    tracing::info!(
        { field::OPERATION } = operation::GC_LOAD_TOPOLOGY,
        { field::COUNT } = layer_count,
        { field::SIZE_BYTES } = handle_bytes_total,
        { field::LATENCY_MS } = elapsed.as_millis() as u64,
        "GC topology loaded"
    );
}

fn emit_mark_metrics(reachable_count: u64, elapsed: Duration) {
    tracing::info!(
        { field::OPERATION } = operation::GC_MARK,
        { field::COUNT } = reachable_count,
        { field::LATENCY_MS } = elapsed.as_millis() as u64,
        "GC mark phase complete"
    );
}

fn emit_sweep_metrics(stats: &SweepStats, elapsed: Duration) {
    tracing::info!(
        { field::OPERATION } = operation::GC_SWEEP,
        { field::COUNT } = stats.layers_swept,
        { field::SIZE_BYTES } = stats.bytes_reclaimable,
        { field::LATENCY_MS } = elapsed.as_millis() as u64,
        layers_unreachable = stats.layers_unreachable,
        layers_protected_by_age = stats.layers_protected_by_age,
        "GC sweep phase complete"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::{commit_layer_default, update_branch, ConflictPolicy};
    use crate::layer::{LayerBuilder, LayerStorage};
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};
    use crate::storage::memory::MemoryPersistentBackend;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_resource(id: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        // Validator requires non-empty `is_a` (see
        // `Validator::validate_resource`). Use core:Class as a generic
        // placeholder; the GC tests don't exercise class-typing
        // semantics so the specific target doesn't matter.
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
        );
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String("v".into()),
        );
        // Real `core:Class` requires `short_name` — supply it so fixtures validate
        // against real core (the chain is rooted on the core base, below).
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("test_fixture".into()),
        );
        r
    }

    /// Helper: commit a small root layer. The root is **self-contained** — it carries
    /// the real core ontology (parent=None) plus the test resource, so fixtures'
    /// property KEYS resolve to declared `core:Property` resources within the layer
    /// (reference integrity, Rule 22 §(c)). Carrying core *in* the root rather than as a
    /// separate base keeps the GC tests' layer **counts** unchanged (the root is one
    /// layer, just larger), so their reachability/sweep assertions hold as written.
    fn commit_root(
        backend: &dyn PersistentBackend,
        storage: &LayerStorage,
    ) -> Arc<crate::layer::Layer> {
        let core_json = include_str!("../../ontologies/core/core-ontology.json");
        let mut b = LayerBuilder::new("root", None);
        for r in crate::ontology::eigon_json::parse_document(core_json).unwrap() {
            b.add_resource(r).unwrap();
        }
        b.add_resource(make_resource("urn:eigenius:test:r"))
            .unwrap();
        commit_layer_default(b, storage.clone(), backend).unwrap()
    }

    /// Helper: commit a child layer above `parent`.
    fn commit_child(
        backend: &dyn PersistentBackend,
        storage: &LayerStorage,
        parent: Arc<crate::layer::Layer>,
        name: &str,
        iri_str: &str,
    ) -> Arc<crate::layer::Layer> {
        let mut b = LayerBuilder::new(name, Some(parent));
        b.add_resource(make_resource(iri_str)).unwrap();
        commit_layer_default(b, storage.clone(), backend).unwrap()
    }

    /// Aggressive config that skips no layers — for tests where the
    /// commit-to-publish gap doesn't matter.
    fn no_age_config() -> GcConfig {
        GcConfig {
            min_age: Duration::from_secs(0),
        }
    }

    #[test]
    fn estimate_reports_eligible_layers_without_sweeping() {
        // The same chain that `unreachable_layer_swept_when_no_root_*`
        // exercises, but through `estimate` instead of `collect` —
        // the layers must NOT be deleted. This is the structural
        // invariant the GC panel's preview step relies on.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let _root = commit_root(&backend, &storage);
        let _orphan = commit_child(
            &backend,
            &storage,
            Arc::clone(&_root),
            "orphan",
            "urn:eigenius:test:o",
        );

        let stats = estimate(GcRoots::default(), &no_age_config(), &backend).unwrap();
        assert_eq!(stats.layers_marked, 0);
        assert_eq!(stats.layers_unreachable, 2);
        assert_eq!(
            stats.layers_swept, 0,
            "estimate must not sweep — that's the point of the preview"
        );
        // Topology untouched.
        assert_eq!(backend.load_topology().unwrap().layer_count(), 2);
    }

    #[test]
    fn estimate_sums_reclaimable_bytes_over_eligible_layers() {
        // Verifies the `bytes_reclaimable` accumulator only counts
        // layers that would actually be swept — unreachable AND past
        // `min_age`. Layers reachable from any root or protected by
        // age must NOT contribute, otherwise the operator's reclaim
        // estimate overpromises.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let _root = commit_root(&backend, &storage);
        let _orphan = commit_child(
            &backend,
            &storage,
            Arc::clone(&_root),
            "orphan",
            "urn:eigenius:test:o",
        );

        let topology = backend.load_topology().unwrap();
        let expected: u64 = topology.iter_layers().map(|h| h.byte_size).sum();
        assert!(
            expected > 0,
            "fixture must produce non-zero byte_size on stored handles"
        );

        // No roots: both layers are unreachable and (with no-age
        // config) eligible. Expected reclaim == sum of every handle.
        let stats = estimate(GcRoots::default(), &no_age_config(), &backend).unwrap();
        assert_eq!(stats.layers_unreachable, 2);
        assert_eq!(stats.layers_protected_by_age, 0);
        assert_eq!(
            stats.bytes_reclaimable, expected,
            "all eligible layers' bytes must accumulate"
        );

        // Same chain, default config (60s min_age): both layers are
        // unreachable but freshly-committed, so the protection window
        // shields them. Bytes reclaimable must be 0.
        let stats = estimate(GcRoots::default(), &GcConfig::default(), &backend).unwrap();
        assert_eq!(stats.layers_protected_by_age, 2);
        assert_eq!(
            stats.bytes_reclaimable, 0,
            "age-protected layers must NOT contribute to reclaim"
        );
    }

    #[test]
    fn estimate_classifies_age_protection() {
        // With the default min_age (60s), a just-committed unreachable
        // layer counts as `protected_by_age` rather than eligible —
        // matching what `collect` would skip. The GC panel surfaces
        // this so the operator sees why the eligible count isn't
        // higher.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let _orphan = commit_root(&backend, &storage);

        let stats = estimate(GcRoots::default(), &GcConfig::default(), &backend).unwrap();
        assert_eq!(stats.layers_unreachable, 1);
        assert_eq!(stats.layers_protected_by_age, 1);
        assert_eq!(stats.layers_swept, 0);
        // Layer still on disk.
        assert_eq!(backend.load_topology().unwrap().layer_count(), 1);
    }

    #[test]
    fn unreachable_layer_swept_when_no_root_references_it() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, &storage);
        let _orphan = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "orphan",
            "urn:eigenius:test:o",
        );

        // No branches, no task pins → only `root` is reachable if it's
        // a root, but here we declare empty roots, so EVERYTHING is
        // unreachable. Verify the orphan and the root both get swept.
        let stats = collect(
            GcRoots::default(),
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(stats.layers_marked, 0);
        assert_eq!(stats.layers_unreachable, 2);
        assert_eq!(stats.layers_swept, 2);
        assert_eq!(stats.layers_protected_by_age, 0);

        // Topology should be empty after sweep.
        assert_eq!(backend.load_topology().unwrap().layer_count(), 0);
    }

    #[test]
    fn reachable_chain_survives_via_branch_root() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, &storage);
        let middle = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "middle",
            "urn:eigenius:test:m",
        );
        let tip = commit_child(
            &backend,
            &storage,
            Arc::clone(&middle),
            "tip",
            "urn:eigenius:test:t",
        );

        // Branch points at tip; root + middle + tip all reachable.
        update_branch(
            "main",
            None,
            tip.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Also commit an unreferenced sibling that should be swept.
        let _orphan = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "orphan",
            "urn:eigenius:test:o",
        );

        let roots = GcRoots::from_branches(&backend).unwrap();
        let stats = collect(
            roots,
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();

        assert_eq!(stats.layers_marked, 3, "root + middle + tip");
        assert_eq!(stats.layers_unreachable, 1, "the orphan");
        assert_eq!(stats.layers_swept, 1);

        // Reachable layers still in topology.
        let topo = backend.load_topology().unwrap();
        assert!(topo.get_layer(root.id()).is_some());
        assert!(topo.get_layer(middle.id()).is_some());
        assert!(topo.get_layer(tip.id()).is_some());
    }

    #[test]
    fn task_pin_keeps_layer_alive() {
        // A layer not on any branch but held in a `task_pin` must
        // survive GC.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, &storage);
        let pinned = commit_child(&backend, &storage, root, "pinned", "urn:eigenius:test:p");

        // Empty branches; task pin holds it.
        let roots = GcRoots {
            branch_heads: Vec::new(),
            task_pins: vec![pinned.id().clone()],
            tag_targets: Vec::new(),
        };
        let stats = collect(
            roots,
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(stats.layers_marked, 2, "pinned + its parent root");
        assert_eq!(stats.layers_swept, 0);
        assert!(backend
            .load_topology()
            .unwrap()
            .get_layer(pinned.id())
            .is_some());
    }

    #[test]
    fn tag_keeps_target_and_ancestors_alive() {
        // D34 §8.3 invariant: a tag protects its target *and that
        // target's transitive ancestors* from GC for as long as the
        // tag exists. Verifies the `from_branches` constructor pulls
        // tag refs into the root set so the mark phase sees them.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, &storage);
        let tagged = commit_child(&backend, &storage, root, "tagged", "urn:eigenius:test:t");

        // No branches; only a tag holds the chain.
        backend.create_tag("release-v1", tagged.id()).unwrap();

        let roots = GcRoots::from_branches(&backend).unwrap();
        assert!(roots.branch_heads.is_empty(), "no branches in this test");
        assert_eq!(roots.tag_targets.len(), 1);

        let stats = collect(
            roots,
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(
            stats.layers_marked, 2,
            "tagged layer + its root ancestor must both be marked"
        );
        assert_eq!(stats.layers_swept, 0);
        assert!(backend
            .load_topology()
            .unwrap()
            .get_layer(tagged.id())
            .is_some());
    }

    #[test]
    fn deleting_tag_releases_protection() {
        // After deleting the tag, the previously-protected layer is
        // sweep-eligible (no other root reaches it). Pairs with the
        // "tag protects" test to confirm the protection is precisely
        // the tag, not an incidental side effect.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, &storage);
        let orphan = commit_child(&backend, &storage, root, "orphan", "urn:eigenius:test:o");

        backend.create_tag("temp", orphan.id()).unwrap();
        // While the tag exists the layer is protected — same shape as
        // the previous test.
        let stats_with_tag = collect(
            GcRoots::from_branches(&backend).unwrap(),
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(stats_with_tag.layers_swept, 0);

        // Delete the tag and re-run GC: the layer (and its root
        // ancestor) become unreachable.
        let deleted = backend.delete_tag("temp").unwrap();
        assert!(deleted, "delete_tag returns true when the tag existed");
        let stats_after = collect(
            GcRoots::from_branches(&backend).unwrap(),
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(stats_after.layers_swept, 2, "both layers reclaimed");
    }

    #[test]
    fn min_age_protects_recent_commits() {
        // Default config has min_age=60s. Just-committed layer is
        // unreachable but protected; sweep skips it.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let _orphan = commit_root(&backend, &storage);

        let stats = collect(
            GcRoots::default(),
            &GcConfig::default(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(stats.layers_unreachable, 1);
        assert_eq!(stats.layers_protected_by_age, 1);
        assert_eq!(stats.layers_swept, 0);
        // Layer survives.
        assert_eq!(backend.load_topology().unwrap().layer_count(), 1);
    }

    #[test]
    fn merge_layer_keeps_all_parents_alive() {
        // Trivial merge: branch points at merge layer; both merged
        // heads must survive (reachable as merge.parents).
        use crate::lattice::merge_independent_heads;
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, &storage);
        let a = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "a",
            "urn:eigenius:test:a",
        );
        let b = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "b",
            "urn:eigenius:test:b",
        );

        let merge = match merge_independent_heads(
            vec![a.id().clone(), b.id().clone()],
            storage.clone(),
            &backend,
        )
        .unwrap()
        {
            crate::lattice::MergeOutcome::Merged { merge_layer } => merge_layer,
            other => panic!("expected Merged, got {other:?}"),
        };

        update_branch(
            "main",
            None,
            merge.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        let stats = collect(
            GcRoots::from_branches(&backend).unwrap(),
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(stats.layers_marked, 4, "root + a + b + merge");
        assert_eq!(stats.layers_swept, 0);
        let topo = backend.load_topology().unwrap();
        assert!(topo.get_layer(a.id()).is_some());
        assert!(topo.get_layer(b.id()).is_some());
        assert!(topo.get_layer(merge.id()).is_some());
    }

    #[test]
    fn idempotent_repeat_runs() {
        // Running collect twice in a row leaves the same state.
        // Second call has nothing to sweep.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, &storage);
        let _orphan = commit_child(
            &backend,
            &storage,
            root.clone(),
            "orphan",
            "urn:eigenius:test:o",
        );
        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        let stats1 = collect(
            GcRoots::from_branches(&backend).unwrap(),
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(stats1.layers_swept, 1);

        let stats2 = collect(
            GcRoots::from_branches(&backend).unwrap(),
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(stats2.layers_swept, 0);
        assert_eq!(stats2.layers_unreachable, 0);
    }

    // ─── 17f-D redirect-aware mark phase ────────────────────────────────

    /// Build the standard redirect test scaffold:
    ///
    /// `root → mid → tip` (the branch's actual chain) plus a separate
    /// `target` layer also rooted at `root`. Returns
    /// `(root, mid, tip, target)`. The branch `main` is set to `tip`.
    fn build_redirect_scaffold(
        backend: &dyn PersistentBackend,
        storage: &LayerStorage,
    ) -> (
        Arc<crate::layer::Layer>,
        Arc<crate::layer::Layer>,
        Arc<crate::layer::Layer>,
        Arc<crate::layer::Layer>,
    ) {
        let root = commit_root(backend, storage);
        let mid = commit_child(
            backend,
            storage,
            Arc::clone(&root),
            "mid",
            "urn:eigenius:test:mid",
        );
        let tip = commit_child(
            backend,
            storage,
            Arc::clone(&mid),
            "tip",
            "urn:eigenius:test:tip",
        );
        let target = commit_child(
            backend,
            storage,
            Arc::clone(&root),
            "target",
            "urn:eigenius:test:target",
        );

        update_branch(
            "main",
            None,
            tip.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            backend,
        )
        .unwrap();

        (root, mid, tip, target)
    }

    /// Default (`preserve_history = false`) reclaim mode: with a
    /// redirect installed at `mid → target`, GC marks `tip` (branch
    /// head), `mid` (redirect source), `target` and its ancestors
    /// (followed through the redirect), and `root` (target's parent).
    /// `mid`'s own parent chain is NOT followed — but `mid → root`
    /// shares `root` with `target → root`, so `root` stays alive via
    /// the target side. The original mid-side intermediate (between
    /// mid and root, if any) would be reclaimed; here the chain is
    /// short so nothing distinct is reclaimable. Verify the mark set
    /// includes exactly the expected layers.
    #[test]
    fn reclaim_mode_marks_target_chain_skips_source_parents() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let (root, mid, tip, target) = build_redirect_scaffold(&backend, &storage);

        // Install a redirect mid → target (reclaim mode).
        let mid_handle = backend
            .load_topology()
            .unwrap()
            .get_layer(mid.id())
            .unwrap()
            .clone();
        backend
            .put_redirect(&crate::layer::RedirectEntry {
                target: target.id().clone(),
                source_handle: mid_handle,
                preserve_history: false,
            })
            .unwrap();

        let topology = backend.load_topology().unwrap();
        let redirects = backend.list_redirects().unwrap();
        let roots = GcRoots {
            branch_heads: vec![tip.id().clone()],
            task_pins: vec![],
            tag_targets: vec![],
        };
        let reachable = mark_reachable(&roots, &topology, &redirects);

        assert!(reachable.contains(tip.id()), "branch head must be marked");
        assert!(reachable.contains(mid.id()), "redirect source visited");
        assert!(
            reachable.contains(target.id()),
            "redirect target must be marked (followed through redirect)"
        );
        assert!(
            reachable.contains(root.id()),
            "root must be marked via target's parent chain"
        );
    }

    /// Preserve-history mode: same scaffold, but `preserve_history =
    /// true`. The mark phase walks BOTH `target → root` (via the
    /// redirect) AND `mid → root` (via the source's own parents).
    /// Indistinguishable from reclaim mode in this 3-layer scaffold
    /// since both walks share `root`; the test asserts the
    /// source-parent walk is enabled by checking an intermediate
    /// layer on the source side.
    #[test]
    fn preserve_history_marks_source_chain_too() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        // Build a longer source chain so we can distinguish:
        // root → mid_below → mid → tip,  and target → root.
        let root = commit_root(&backend, &storage);
        let mid_below = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "mid_below",
            "urn:eigenius:test:mid_below",
        );
        let mid = commit_child(
            &backend,
            &storage,
            Arc::clone(&mid_below),
            "mid",
            "urn:eigenius:test:mid",
        );
        let tip = commit_child(
            &backend,
            &storage,
            Arc::clone(&mid),
            "tip",
            "urn:eigenius:test:tip",
        );
        let target = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "target",
            "urn:eigenius:test:target",
        );

        let mid_handle = backend
            .load_topology()
            .unwrap()
            .get_layer(mid.id())
            .unwrap()
            .clone();

        // Preserve mode: walking from tip → mid → (redirect AND
        // mid.parents) → mid_below → root. So mid_below stays alive.
        backend
            .put_redirect(&crate::layer::RedirectEntry {
                target: target.id().clone(),
                source_handle: mid_handle,
                preserve_history: true,
            })
            .unwrap();

        let topology = backend.load_topology().unwrap();
        let redirects = backend.list_redirects().unwrap();
        let roots = GcRoots {
            branch_heads: vec![tip.id().clone()],
            task_pins: vec![],
            tag_targets: vec![],
        };
        let reachable = mark_reachable(&roots, &topology, &redirects);

        assert!(reachable.contains(tip.id()));
        assert!(reachable.contains(mid.id()));
        assert!(
            reachable.contains(mid_below.id()),
            "preserve_history must keep the source-side parent chain alive"
        );
        assert!(reachable.contains(target.id()));
        assert!(reachable.contains(root.id()));
    }

    /// Reclaim mode, this time with a distinguishable intermediate on
    /// the source side: `mid_below` is the source's parent and
    /// nothing else references it. Under `preserve_history = false`
    /// it becomes unreachable after the redirect is installed → GC
    /// sweeps it.
    #[test]
    fn reclaim_mode_sweeps_source_side_intermediate() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, &storage);
        let mid_below = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "mid_below",
            "urn:eigenius:test:mid_below",
        );
        let mid = commit_child(
            &backend,
            &storage,
            Arc::clone(&mid_below),
            "mid",
            "urn:eigenius:test:mid",
        );
        let tip = commit_child(
            &backend,
            &storage,
            Arc::clone(&mid),
            "tip",
            "urn:eigenius:test:tip",
        );
        let target = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "target",
            "urn:eigenius:test:target",
        );

        update_branch(
            "main",
            None,
            tip.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        let mid_handle = backend
            .load_topology()
            .unwrap()
            .get_layer(mid.id())
            .unwrap()
            .clone();
        backend
            .put_redirect(&crate::layer::RedirectEntry {
                target: target.id().clone(),
                source_handle: mid_handle,
                preserve_history: false,
            })
            .unwrap();

        let stats = collect(
            GcRoots::from_branches(&backend).unwrap(),
            &no_age_config(),
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();

        // Expected reachable: tip, mid (visited but parents skipped),
        // target, root. mid_below is unreachable → swept.
        assert_eq!(stats.layers_marked, 4);
        assert!(stats.layers_swept >= 1, "mid_below should be swept");

        // mid_below's handle is gone from the topology.
        assert!(backend
            .load_topology()
            .unwrap()
            .get_layer(mid_below.id())
            .is_none());
    }
}
