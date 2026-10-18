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

//! Resolve redirects — forward pointers for consolidating below the
//! branch head (D25 §12.8).
//!
//! A redirect lives outside the layer-id hash domain. When `Layer::resolve`
//! walks head→root and reaches a layer that's a redirect source, the walk
//! short-circuits to the redirect target (the consolidated `L_c`) and
//! continues from there. The original layer's topology slot can be
//! reclaimed on disk because [`PersistentBackend::load_topology`]
//! manufactures a synthetic in-memory [`LayerHandle`] from each
//! `RedirectEntry` at startup — every topology-walk caller sees a
//! consistent DAG (D25 §12.8.1(d)).

use crate::layer::{LayerHandle, LayerId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Persistent record of one resolve redirect installed by
/// `consolidate_chain` when `to` is below the branch head.
///
/// Carries enough of the original `to` layer's metadata to manufacture
/// the in-memory synthetic tombstone in `load_topology` even when the
/// original `LayerHandle` has been reclaimed. The fields mirror
/// `LayerHandle` directly — the redirect entry is "a `LayerHandle` plus
/// the redirect target."
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RedirectEntry {
    /// Position hash of the consolidated layer (the redirect target).
    /// Resolves above this layer follow the redirect and continue
    /// walking from `target`.
    pub target: LayerId,
    /// Snapshot of `to`'s `LayerHandle` at install time. Used by
    /// `load_topology` to manufacture the in-memory tombstone so
    /// every parent-pointer walk in the kernel sees a consistent
    /// topology DAG (D25 §12.8.1(d)). The `id` field on this handle
    /// is the redirect source.
    pub source_handle: LayerHandle,
    /// Whether GC should keep the consolidated range alive
    /// (D25 §12.8.1(b)). `false` is the default — GC's mark phase
    /// follows the redirect target only, so the source-side chain
    /// becomes unreachable from head-rooted marks and is eligible
    /// for sweep. `true` (operator opt-in via
    /// `ConsolidateOpts.preserve_history`) makes GC also mark the
    /// source-side chain, preserving pre-consolidation history for
    /// time-travel reads against intermediate layers in the range.
    pub preserve_history: bool,
}

impl RedirectEntry {
    /// Convenience: the `LayerId` of the layer this redirect replaces.
    pub fn source(&self) -> &LayerId {
        &self.source_handle.id
    }
}

/// Manufacture the in-memory synthetic tombstone for this redirect.
///
/// The returned `LayerHandle` matches the original `to` layer's
/// structure (id, parents, content_hash, supporting_layer, name,
/// resource_count, created_at) and additionally has
/// `is_redirect_source = true` so diagnostic surfaces can render it
/// as "consolidated into <target>" rather than as an ordinary
/// (empty-looking) layer.
pub fn manufacture_tombstone(entry: &RedirectEntry) -> LayerHandle {
    LayerHandle {
        is_redirect_source: true,
        ..entry.source_handle.clone()
    }
}

/// Augment a `LayerTopology` with synthetic tombstones for every
/// redirect whose source isn't already present in the topology.
///
/// Called by `PersistentBackend::load_topology` after the topology
/// CF has been read. Idempotent: redirects whose source is still on
/// disk (preserve-history mode) leave the topology entry alone;
/// reclaimed sources get a synthetic entry inserted.
pub fn augment_topology_with_redirects(
    topology: &mut crate::layer::LayerTopology,
    redirects: &[RedirectEntry],
) {
    for entry in redirects {
        if topology.get_layer(entry.source()).is_none() {
            topology.insert_layer(manufacture_tombstone(entry));
        }
    }
    // Note: we deliberately do NOT touch entries that already exist.
    // Preserve-history mode leaves both the original handle and the
    // redirect in storage; the original handle wins. The redirect
    // still drives the resolve-walk short-circuit (handled in
    // `Layer::redirect_target`); only the topology slot is shared.
}

/// In-memory cache of installed resolve redirects (`source → target`).
///
/// Sits on the [`LayerStorage`](crate::layer::LayerStorage) bundle
/// alongside `bloom_cache`. Loaded from the persistent backend at
/// `LayerStorage::with_persistent` construction time and kept in sync
/// by the consolidation algorithm: every `put_redirect` call to the
/// backend is mirrored by a `put` here.
///
/// `Layer::resolve` does not consult this directly. Instead,
/// `build_chain` pre-resolves redirects per layer: when a layer's id
/// is a redirect source, the target's full chain is built and stored
/// inline as [`Layer::redirect_target`](crate::layer::Layer::redirect_target).
/// The resolve walk's hot path is then a single inline branch with no
/// map probe.
pub trait RedirectMap: Send + Sync {
    /// Returns the redirect target for `source`, or `None` if the
    /// layer isn't a redirect source.
    fn lookup(&self, source: &LayerId) -> Option<LayerId>;

    /// Insert or update a redirect entry.
    fn put(&self, source: LayerId, target: LayerId);

    /// Remove a redirect entry.
    fn remove(&self, source: &LayerId);

    /// Total number of installed redirects.
    fn len(&self) -> usize;

    /// True when no redirects are installed. Lets callers short-circuit
    /// before paying for a full `len()` probe in implementations that
    /// might otherwise walk an underlying store.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Unbounded in-memory `RedirectMap`. The expected entry count is
/// "one per consolidation operator-invoked over the chain's lifetime,"
/// which is small in absolute terms — no eviction policy needed.
#[derive(Default)]
pub struct MemoryRedirectMap {
    inner: RwLock<HashMap<LayerId, LayerId>>,
}

impl MemoryRedirectMap {
    /// Empty map. Used by `LayerStorage::in_memory` and by tests that
    /// don't exercise redirects.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-populate from a slice of `RedirectEntry` (typically the
    /// result of `PersistentBackend::list_redirects` at startup).
    pub fn from_entries(entries: &[RedirectEntry]) -> Self {
        let map = Self::new();
        for entry in entries {
            map.put(entry.source().clone(), entry.target.clone());
        }
        map
    }
}

impl RedirectMap for MemoryRedirectMap {
    fn lookup(&self, source: &LayerId) -> Option<LayerId> {
        self.inner
            .read()
            .expect("MemoryRedirectMap poisoned")
            .get(source)
            .cloned()
    }

    fn put(&self, source: LayerId, target: LayerId) {
        self.inner
            .write()
            .expect("MemoryRedirectMap poisoned")
            .insert(source, target);
    }

    fn remove(&self, source: &LayerId) {
        self.inner
            .write()
            .expect("MemoryRedirectMap poisoned")
            .remove(source);
    }

    fn len(&self) -> usize {
        self.inner.read().expect("MemoryRedirectMap poisoned").len()
    }
}

/// Empty `RedirectMap` used by `LayerStorage::in_memory` when no
/// redirects can be installed. Cheaper than `MemoryRedirectMap::new`
/// for the common no-redirect case because lookups never take a lock.
pub struct NoRedirects;

impl RedirectMap for NoRedirects {
    fn lookup(&self, _source: &LayerId) -> Option<LayerId> {
        None
    }

    fn put(&self, _source: LayerId, _target: LayerId) {
        // No-op. Callers that want a mutable redirect map use
        // `MemoryRedirectMap` explicitly via `LayerStorage::with_persistent`.
    }

    fn remove(&self, _source: &LayerId) {}

    fn len(&self) -> usize {
        0
    }
}

/// Convenience: an `Arc<dyn RedirectMap>` pre-populated from a backend.
pub(crate) fn redirect_map_from_backend(
    backend: &dyn crate::storage::PersistentBackend,
) -> Arc<dyn RedirectMap> {
    match backend.list_redirects() {
        Ok(entries) => Arc::new(MemoryRedirectMap::from_entries(&entries)),
        Err(_) => Arc::new(NoRedirects),
    }
}
