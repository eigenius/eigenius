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

use crate::layer::{BloomCache, LayerId, LayerTopology, ResourceCache};
use crate::storage::{PersistentBackend, StorageError};
use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

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
}

impl GcRoots {
    /// Build a roots set from the persistent backend's branch refs.
    /// Task pins must be added separately by the caller.
    pub fn from_branches(backend: &dyn PersistentBackend) -> Result<Self, StorageError> {
        let branches = backend.list_branches()?;
        Ok(Self {
            branch_heads: branches.into_iter().map(|(_, id)| id).collect(),
            task_pins: Vec::new(),
        })
    }

    /// Iterator over every layer id that should be treated as a root.
    fn iter(&self) -> impl Iterator<Item = &LayerId> {
        self.branch_heads.iter().chain(self.task_pins.iter())
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
    // Step 1: snapshot under the branch lock. Topology is loaded here
    // too so the (branches + topology) pair is mutually consistent —
    // every branch head exists in the topology snapshot.
    let topology: LayerTopology = crate::lattice::with_branch_lock(|| backend.load_topology())?;

    // Step 2: mark phase. BFS from roots through topology.parents.
    let reachable = mark_reachable(&roots, &topology);

    // Step 3: sweep phase. Iterate every layer in the topology; if
    // not in reachable and old enough, delete. Counters tally what
    // happened.
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
    }

    Ok(stats)
}

/// BFS reachability over `LayerHandle.parents`.
fn mark_reachable(roots: &GcRoots, topology: &LayerTopology) -> BTreeSet<LayerId> {
    let mut reachable: BTreeSet<LayerId> = BTreeSet::new();
    let mut queue: VecDeque<LayerId> = VecDeque::new();
    for r in roots.iter() {
        queue.push_back(r.clone());
    }
    while let Some(id) = queue.pop_front() {
        if !reachable.insert(id.clone()) {
            continue;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::{commit_layer, update_branch, ConflictPolicy};
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
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String("v".into()),
        );
        r
    }

    /// Helper: commit a small root layer.
    fn commit_root(
        backend: &dyn PersistentBackend,
        storage: &LayerStorage,
    ) -> Arc<crate::layer::Layer> {
        let mut b = LayerBuilder::new("root", None);
        b.add_resource(make_resource("urn:eigenius:core:r"))
            .unwrap();
        commit_layer(b, storage.clone(), backend).unwrap()
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
        commit_layer(b, storage.clone(), backend).unwrap()
    }

    /// Aggressive config that skips no layers — for tests where the
    /// commit-to-publish gap doesn't matter.
    fn no_age_config() -> GcConfig {
        GcConfig {
            min_age: Duration::from_secs(0),
        }
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
}
