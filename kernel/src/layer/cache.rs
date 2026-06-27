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

//! Resource cache trait and a naïve in-memory implementation.
//!
//! Phase 14a-i ships the trait shape and a `MemoryResourceCache` that holds
//! everything in a single map without eviction. The bounded two-pool ARC
//! cache from D23 §5.3 lands in 14c; until then the naïve impl is correct
//! but unbounded — fine for the in-memory `MemoryStore` backend (which
//! already holds everything in memory anyway) and for unit tests.
//!
//! Cache keys are `(LayerId, Iri)` per D23 §5.4.2: the same IRI defined
//! at multiple layers caches as distinct entries. This is what makes the
//! topology walk + cache fall-through correct without the cache having to
//! understand shadowing — that's the shadowing index's job (§5.2 / 14b).

use crate::layer::{BloomFilter, LayerId};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::storage::{PersistentBackend, ResourceBackend, StorageError};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

/// Cache key: a specific (layer, iri) pair.
///
/// Distinct from "the resolved value of `iri` at `layer`'s view" — the cache
/// stores per-layer entries; resolution against a branch head goes through
/// the topology walk + shadowing index (14b) on top.
///
/// Derives `Ord` so the cache can use `BTreeMap` storage (matches the rest
/// of the kernel's "BTreeMap everywhere for deterministic ordering" rule).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceKey {
    pub layer: LayerId,
    pub iri: Iri,
}

impl ResourceKey {
    pub fn new(layer: LayerId, iri: Iri) -> Self {
        Self { layer, iri }
    }
}

/// Pool selector for `ResourceCache` insertions (D23 §5.3 / Phase 14c).
///
/// Bounded `ResourceCache` implementations partition their budget into two
/// pools: `Active` for entries that are top-of-stack for the current head
/// (the steady-state hot working set) and `Historical` for entries
/// shadowed in every active head (only reachable via time-travel reads or
/// trace dereferences). Historical-tier entries evict first under memory
/// pressure because they have lower locality.
///
/// Naïve implementations (the `MemoryResourceCache` used in tests and the
/// in-memory bootstrap) ignore the tier — there's no eviction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTier {
    /// Top-of-stack for the active head. Steady-state hot working set.
    Active,
    /// Shadowed by a higher layer in every active head; only reachable via
    /// time-travel or trace dereferences. Lower-priority for retention.
    Historical,
}

/// Read/write cache for resources, keyed by `(LayerId, Iri)`.
///
/// Implementations may evict at any time; the cache is a hint, not a source
/// of truth. Misses fall through to the persistent backend (§5.4.2).
/// Phase 14c provides a bounded two-pool implementation
/// (`BoundedResourceCache`); naïve unbounded ones (`MemoryResourceCache`)
/// remain available for tests and the in-memory bootstrap path where
/// eviction gains nothing.
pub trait ResourceCache: Send + Sync {
    /// Look up a resource. Returns `None` on miss; the caller falls through
    /// to the persistent backend.
    fn get(&self, key: &ResourceKey) -> Option<Arc<Resource>>;

    /// Insert or replace a resource in the requested pool. Implementations
    /// may evict other entries within the same pool to make room.
    fn put(&self, key: ResourceKey, resource: Arc<Resource>, tier: CacheTier);

    /// Drop all entries for a given layer. Called by GC (§5.7) when a layer
    /// is swept; also by branch pruning (§5.8).
    fn evict_layer(&self, layer: &LayerId);

    /// Snapshot of basic counters. Implementations may report zeros for
    /// counters they don't track.
    fn stats(&self) -> CacheStats;
}

/// Counters reported by `ResourceCache::stats`. Implementations that don't
/// track a particular field may report 0. The bounded two-pool impl
/// populates the per-tier `*_active` / `*_historical` fields; naïve
/// unbounded impls populate only the totals.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Number of entries currently held (sum of both pools).
    pub entries: u64,
    /// Entries in the active pool. Reported as 0 by impls that don't
    /// distinguish pools.
    pub active_entries: u64,
    /// Entries in the historical pool. Reported as 0 by impls that don't
    /// distinguish pools.
    pub historical_entries: u64,
    /// Cumulative `get` calls that hit.
    pub hits: u64,
    /// Cumulative `get` calls that missed.
    pub misses: u64,
}

/// Naïve unbounded in-memory cache. Holds every `(layer, iri)` ever inserted
/// until `evict_layer` removes them.
///
/// Phase 14a uses this for both the in-memory backend (where bounded eviction
/// gains nothing — the backend itself is in memory) and unit tests. The real
/// bounded two-pool ARC cache lands in 14c.
pub struct MemoryResourceCache {
    inner: RwLock<MemoryCacheState>,
}

struct MemoryCacheState {
    entries: BTreeMap<ResourceKey, Arc<Resource>>,
    hits: u64,
    misses: u64,
}

impl MemoryResourceCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(MemoryCacheState {
                entries: BTreeMap::new(),
                hits: 0,
                misses: 0,
            }),
        }
    }
}

impl Default for MemoryResourceCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceCache for MemoryResourceCache {
    fn get(&self, key: &ResourceKey) -> Option<Arc<Resource>> {
        let mut state = self.inner.write().expect("MemoryResourceCache poisoned");
        match state.entries.get(key).cloned() {
            Some(r) => {
                state.hits = state.hits.saturating_add(1);
                Some(r)
            }
            None => {
                state.misses = state.misses.saturating_add(1);
                None
            }
        }
    }

    fn put(&self, key: ResourceKey, resource: Arc<Resource>, _tier: CacheTier) {
        // Naïve impl is unbounded; the pool selector is meaningless here.
        // The bounded `BoundedResourceCache` honors it.
        let mut state = self.inner.write().expect("MemoryResourceCache poisoned");
        state.entries.insert(key, resource);
    }

    fn evict_layer(&self, layer: &LayerId) {
        let mut state = self.inner.write().expect("MemoryResourceCache poisoned");
        state.entries.retain(|key, _| &key.layer != layer);
    }

    fn stats(&self) -> CacheStats {
        let state = self.inner.read().expect("MemoryResourceCache poisoned");
        CacheStats {
            entries: state.entries.len() as u64,
            // Naïve impl doesn't partition; per-pool counters stay 0.
            active_entries: 0,
            historical_entries: 0,
            hits: state.hits,
            misses: state.misses,
        }
    }
}

// --- Bounded two-pool resource cache (D23 §5.3 / Phase 14c) ---

/// Bounded resource cache with two independently-sized pools and
/// W-TinyLFU eviction (via `moka`). Active-pool entries are top-of-stack
/// for the current head; historical-pool entries are shadowed and only
/// reachable via time-travel reads.
///
/// **Why two pools, not one.** A single LRU/W-TinyLFU over the merged
/// set conflates steady-state queries (active reach) with low-locality
/// time-travel and trace dereferences (historical reach). Mixing them
/// degrades hit rate on both. With separate budgets — default 60% active
/// / 40% historical — historical traffic can't push hot active entries
/// out, and "historical evicts first under pressure" is realized by the
/// budget split: each pool evicts independently within its own budget.
///
/// **Why moka.** D23 §5.3 originally proposed ARC; W-TinyLFU (the
/// algorithm `moka` implements, ported from Caffeine) outperforms ARC
/// on skewed access patterns and is production-grade in Rust services.
/// Eviction is eventually consistent — the cache may briefly overshoot
/// its capacity under high concurrency while background eviction
/// catches up. This is documented behavior, not a bug; for a kernel
/// resource cache "bounded memory" is the load-bearing property and
/// instantaneous byte-exact accounting isn't needed.
///
/// **Capacity.** Pool sizes are *entry counts*, not byte counts. Bytes
/// per entry vary widely (a small `is_a` resource vs. a long bio
/// description), but entry-counted budgets are simpler and our
/// downstream pressure (D23 §11.3) is an open question pending Phase 12
/// workload data.
pub struct BoundedResourceCache {
    active: moka::sync::Cache<ResourceKey, Arc<Resource>>,
    historical: moka::sync::Cache<ResourceKey, Arc<Resource>>,
    /// Per-cache hit/miss counters. moka tracks per-pool counters
    /// internally but doesn't surface combined hit-rate, so we keep our
    /// own atomics for `CacheStats`.
    hits: std::sync::atomic::AtomicU64,
    misses: std::sync::atomic::AtomicU64,
}

impl BoundedResourceCache {
    /// Default fraction of total budget allocated to the active pool.
    /// D23 §5.3: Active 60%, Historical 40%.
    pub const DEFAULT_ACTIVE_FRACTION: f64 = 0.60;

    /// Construct with `total_entries` total capacity, default 60/40 split.
    pub fn new(total_entries: u64) -> Self {
        Self::with_split(total_entries, Self::DEFAULT_ACTIVE_FRACTION)
    }

    /// Construct with explicit pool split. `active_fraction` must be in
    /// `(0.0, 1.0)`; values outside the range are clamped.
    pub fn with_split(total_entries: u64, active_fraction: f64) -> Self {
        let frac = active_fraction.clamp(f64::EPSILON, 1.0 - f64::EPSILON);
        let active_cap = ((total_entries as f64) * frac).round() as u64;
        let historical_cap = total_entries.saturating_sub(active_cap);
        Self {
            active: moka::sync::Cache::builder()
                .max_capacity(active_cap.max(1))
                .build(),
            historical: moka::sync::Cache::builder()
                .max_capacity(historical_cap.max(1))
                .build(),
            hits: std::sync::atomic::AtomicU64::new(0),
            misses: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl ResourceCache for BoundedResourceCache {
    fn get(&self, key: &ResourceKey) -> Option<Arc<Resource>> {
        // Check the active pool first; it's the hot path. Fall through
        // to the historical pool on miss. (No promotion-on-historical-hit
        // yet — that's a 14c-ii feature.)
        if let Some(r) = self.active.get(key) {
            self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Some(r);
        }
        if let Some(r) = self.historical.get(key) {
            self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Some(r);
        }
        self.misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        None
    }

    fn put(&self, key: ResourceKey, resource: Arc<Resource>, tier: CacheTier) {
        match tier {
            CacheTier::Active => {
                // If the entry was previously historical, drop the stale
                // copy so the pool counters reflect the move.
                self.historical.invalidate(&key);
                self.active.insert(key, resource);
            }
            CacheTier::Historical => {
                self.active.invalidate(&key);
                self.historical.insert(key, resource);
            }
        }
    }

    fn evict_layer(&self, layer: &LayerId) {
        // moka's `invalidate_entries_if` queues removals to run during
        // background maintenance; per-key `invalidate` is synchronous.
        // GC and branch-prune callers expect "the entries are gone now,"
        // so we collect matching keys via iteration and invalidate
        // them directly. Iteration on a `moka::sync::Cache` is a
        // weakly-consistent snapshot (no lock held); concurrent inserts
        // race naturally and converge on the next pass — same
        // semantics our callers already accept from any cache.
        let mut to_drop: Vec<ResourceKey> = Vec::new();
        for entry in self.active.iter() {
            if entry.0.layer == *layer {
                to_drop.push(entry.0.as_ref().clone());
            }
        }
        for k in to_drop.drain(..) {
            self.active.invalidate(&k);
        }
        for entry in self.historical.iter() {
            if entry.0.layer == *layer {
                to_drop.push(entry.0.as_ref().clone());
            }
        }
        for k in to_drop {
            self.historical.invalidate(&k);
        }
    }

    fn stats(&self) -> CacheStats {
        // moka counters are eventually consistent; `run_pending_tasks`
        // forces processing before reading. Cheap and matches what
        // tests want to assert on.
        self.active.run_pending_tasks();
        self.historical.run_pending_tasks();
        let active_entries = self.active.entry_count();
        let historical_entries = self.historical.entry_count();
        CacheStats {
            entries: active_entries.saturating_add(historical_entries),
            active_entries,
            historical_entries,
            hits: self.hits.load(std::sync::atomic::Ordering::Relaxed),
            misses: self.misses.load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

/// In-memory `ResourceBackend` for tests and the kernel's bootstrap path.
///
/// Holds resources keyed by `(LayerId, Iri)` in a single map. Equivalent to
/// `RocksStore` from a `Layer`'s point of view but with no durability and
/// none of the persistent-backend surface. Used so the kernel can exercise
/// `Layer` without spinning up a temp RocksDB.
pub struct MemoryResourceBackend {
    inner: RwLock<BTreeMap<(LayerId, Iri), Arc<Resource>>>,
}

impl MemoryResourceBackend {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(BTreeMap::new()),
        }
    }

    /// Insert a resource. Used during layer build/test setup.
    pub fn insert(&self, layer: LayerId, iri: Iri, resource: Arc<Resource>) {
        let mut state = self.inner.write().expect("MemoryResourceBackend poisoned");
        state.insert((layer, iri), resource);
    }
}

impl Default for MemoryResourceBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceBackend for MemoryResourceBackend {
    fn load_resource(&self, layer_id: &LayerId, iri: &Iri) -> Option<Resource> {
        let state = self.inner.read().expect("MemoryResourceBackend poisoned");
        state
            .get(&(layer_id.clone(), iri.clone()))
            .map(|arc| (**arc).clone())
    }

    fn try_load_resource(
        &self,
        layer_id: &LayerId,
        iri: &Iri,
    ) -> Result<Option<Resource>, StorageError> {
        Ok(self.load_resource(layer_id, iri))
    }

    fn list_layer_iris(&self, layer_id: &LayerId) -> Result<BTreeSet<Iri>, StorageError> {
        let state = self.inner.read().expect("MemoryResourceBackend poisoned");
        Ok(state
            .keys()
            .filter(|(lid, _)| lid == layer_id)
            .map(|(_, iri)| iri.clone())
            .collect())
    }
}

// --- BloomCache (D23 §5.2) ---

/// Bounded cache of per-layer shadowing blooms (D23 §5.2). Mirrors
/// `ResourceCache`'s shape, with the difference that miss handling is
/// encapsulated: `get_or_load` falls through to the cache's backing
/// `PersistentBackend` and inserts the loaded bloom before returning. The
/// `Layer::resolve` path treats the cache as a single-call surface and
/// never sees the backend for bloom purposes.
///
/// Phase 14b ships an unbounded `MemoryBloomCache`. Bounded ARC-style
/// eviction follows the same lifecycle as `ResourceCache`'s 14c work.
pub trait BloomCache: Send + Sync {
    /// Get the bloom for `layer`, fetching from the backend on miss.
    /// Returns `None` only if the backend reports no bloom for the
    /// layer (e.g., a layer that predates Phase 14b — should not occur
    /// in fresh DBs).
    fn get_or_load(&self, layer: &LayerId) -> Result<Option<Arc<BloomFilter>>, StorageError>;

    /// Insert or replace a bloom for `layer`. Used by `LayerBuilder::build`
    /// to pre-populate the cache with the freshly-computed bloom (avoids
    /// a backend round-trip on the first resolve through a just-built
    /// layer). Implementations may evict other entries to make room.
    fn put(&self, layer: LayerId, bloom: Arc<BloomFilter>);

    /// Drop all entries for a layer. Called by GC when a layer is
    /// swept and by branch pruning.
    fn evict_layer(&self, layer: &LayerId);

    /// Cache counters. Implementations may report zeros for fields
    /// they don't track.
    fn stats(&self) -> CacheStats;
}

/// Naïve unbounded bloom cache. Holds every fetched (or directly-`put`)
/// bloom forever until `evict_layer` removes it. Bounded eviction lands
/// with 14c.
///
/// Optionally backed by a `PersistentBackend` for fall-through reads on
/// cache miss. The in-memory bootstrap path constructs a cache *without*
/// a backend (every layer is freshly built and the bloom is populated
/// at `build` time); the persistent path passes the `RocksStore` Arc so
/// reloads-after-eviction work.
pub struct MemoryBloomCache {
    inner: RwLock<MemoryBloomCacheState>,
    backend: Option<Arc<dyn PersistentBackend>>,
}

struct MemoryBloomCacheState {
    entries: BTreeMap<LayerId, Arc<BloomFilter>>,
    // No `hits` counter: the bloom is the master gate of every chain walk, so
    // `get_or_load` is among the hottest paths in the kernel (≈157M calls to
    // validate one large lexicon chunk). A hit counter forced a write lock on
    // every hit — pure contention for a diagnostic. We keep only `misses`
    // (tracked on the already-write-locked slow path, so it's free).
    misses: u64,
}

impl MemoryBloomCache {
    /// Create a cache that falls through to `backend` on miss.
    pub fn new(backend: Arc<dyn PersistentBackend>) -> Self {
        Self {
            inner: RwLock::new(MemoryBloomCacheState {
                entries: BTreeMap::new(),
                misses: 0,
            }),
            backend: Some(backend),
        }
    }

    /// Create a cache with no backend fall-through. Misses return
    /// `Ok(None)` and `Layer::resolve` treats the layer as "maybe
    /// present" (defensive — better one extra defined-IRI check than
    /// skipping a defining layer). Used by the in-memory bootstrap
    /// path where every layer's bloom is `put` at build time.
    pub fn cache_only() -> Self {
        Self {
            inner: RwLock::new(MemoryBloomCacheState {
                entries: BTreeMap::new(),
                misses: 0,
            }),
            backend: None,
        }
    }
}

impl BloomCache for MemoryBloomCache {
    fn get_or_load(&self, layer: &LayerId) -> Result<Option<Arc<BloomFilter>>, StorageError> {
        // Fast path: hit under the read lock only. No counter bump — see
        // `MemoryBloomCacheState`: this is a top-N hottest path and a hit
        // counter would force a write lock on every probe.
        {
            let state = self.inner.read().expect("MemoryBloomCache poisoned");
            if let Some(b) = state.entries.get(layer).cloned() {
                return Ok(Some(b));
            }
        }

        // Miss: fetch from the backend (if configured), insert, return.
        let backend = match self.backend.as_ref() {
            Some(b) => b,
            None => {
                let mut state = self.inner.write().expect("MemoryBloomCache poisoned");
                state.misses = state.misses.saturating_add(1);
                return Ok(None);
            }
        };
        let bloom = match backend.load_bloom(layer)? {
            Some(b) => b,
            None => {
                let mut state = self.inner.write().expect("MemoryBloomCache poisoned");
                state.misses = state.misses.saturating_add(1);
                return Ok(None);
            }
        };
        let arc = Arc::new(bloom);
        let mut state = self.inner.write().expect("MemoryBloomCache poisoned");
        // Concurrent miss may have already inserted; keep the existing
        // entry to maintain Arc identity for any other holders.
        let stored = state
            .entries
            .entry(layer.clone())
            .or_insert_with(|| Arc::clone(&arc))
            .clone();
        state.misses = state.misses.saturating_add(1);
        Ok(Some(stored))
    }

    fn put(&self, layer: LayerId, bloom: Arc<BloomFilter>) {
        let mut state = self.inner.write().expect("MemoryBloomCache poisoned");
        state.entries.insert(layer, bloom);
    }

    fn evict_layer(&self, layer: &LayerId) {
        let mut state = self.inner.write().expect("MemoryBloomCache poisoned");
        state.entries.remove(layer);
    }

    fn stats(&self) -> CacheStats {
        let state = self.inner.read().expect("MemoryBloomCache poisoned");
        CacheStats {
            entries: state.entries.len() as u64,
            // Bloom cache doesn't partition; per-pool counters stay 0.
            active_entries: 0,
            historical_entries: 0,
            // Hits are no longer tracked (the counter cost a write lock on the
            // hottest path); only misses are reported.
            hits: 0,
            misses: state.misses,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lid(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_resource(id: &str) -> Arc<Resource> {
        Arc::new(Resource::new(iri(id)))
    }

    #[test]
    fn miss_and_hit_counters() {
        let cache = MemoryResourceCache::new();
        let key = ResourceKey::new(lid(1), iri("urn:eigenius:example:A"));

        // Initial miss.
        assert!(cache.get(&key).is_none());
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 0);

        // Insert and hit.
        cache.put(
            key.clone(),
            make_resource("urn:eigenius:example:A"),
            CacheTier::Active,
        );
        let got = cache.get(&key).expect("expected hit");
        assert_eq!(got.id().unwrap().as_str(), "urn:eigenius:example:A");
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().entries, 1);
    }

    #[test]
    fn put_replaces_existing() {
        let cache = MemoryResourceCache::new();
        let key = ResourceKey::new(lid(1), iri("urn:eigenius:example:A"));
        cache.put(
            key.clone(),
            make_resource("urn:eigenius:example:A"),
            CacheTier::Active,
        );
        cache.put(
            key.clone(),
            make_resource("urn:eigenius:example:A"),
            CacheTier::Active,
        );
        assert_eq!(cache.stats().entries, 1);
    }

    #[test]
    fn distinct_layers_are_distinct_keys() {
        let cache = MemoryResourceCache::new();
        let key_a = ResourceKey::new(lid(1), iri("urn:eigenius:example:X"));
        let key_b = ResourceKey::new(lid(2), iri("urn:eigenius:example:X"));
        cache.put(
            key_a,
            make_resource("urn:eigenius:example:X"),
            CacheTier::Active,
        );
        cache.put(
            key_b,
            make_resource("urn:eigenius:example:X"),
            CacheTier::Active,
        );
        assert_eq!(cache.stats().entries, 2);
    }

    #[test]
    fn evict_layer_drops_only_that_layer() {
        let cache = MemoryResourceCache::new();
        let l1_a = ResourceKey::new(lid(1), iri("urn:eigenius:example:A"));
        let l1_b = ResourceKey::new(lid(1), iri("urn:eigenius:example:B"));
        let l2_a = ResourceKey::new(lid(2), iri("urn:eigenius:example:A"));

        cache.put(
            l1_a.clone(),
            make_resource("urn:eigenius:example:A"),
            CacheTier::Active,
        );
        cache.put(
            l1_b.clone(),
            make_resource("urn:eigenius:example:B"),
            CacheTier::Active,
        );
        cache.put(
            l2_a.clone(),
            make_resource("urn:eigenius:example:A"),
            CacheTier::Active,
        );
        assert_eq!(cache.stats().entries, 3);

        cache.evict_layer(&lid(1));
        assert_eq!(cache.stats().entries, 1);
        assert!(cache.get(&l1_a).is_none());
        assert!(cache.get(&l1_b).is_none());
        assert!(cache.get(&l2_a).is_some());
    }

    #[test]
    fn arc_sharing_does_not_clone_resource_payload() {
        let cache = MemoryResourceCache::new();
        let key = ResourceKey::new(lid(1), iri("urn:eigenius:example:A"));
        let resource = make_resource("urn:eigenius:example:A");
        cache.put(key.clone(), Arc::clone(&resource), CacheTier::Active);

        // Strong count: original + cache-held = 2.
        assert_eq!(Arc::strong_count(&resource), 2);

        // Each get bumps the count while the returned Arc is alive.
        let got = cache.get(&key).expect("expected hit");
        assert_eq!(Arc::strong_count(&resource), 3);
        drop(got);
        assert_eq!(Arc::strong_count(&resource), 2);
    }

    // --- BloomCache tests ---

    use crate::layer::{BloomFilter, LayerBuilder, LayerStorage};
    use crate::ontology::resource::Value;
    use crate::storage::memory::MemoryPersistentBackend;

    fn small_layer(
        name: &str,
        parent: Option<Arc<crate::layer::Layer>>,
    ) -> Arc<crate::layer::Layer> {
        let mut builder = LayerBuilder::new(name, parent);
        let mut r = Resource::new(iri("urn:eigenius:test:r"));
        r.set(iri("urn:eigenius:test:p"), Value::String("v".into()));
        builder.add_resource(r).unwrap();
        Arc::new(builder.build(LayerStorage::in_memory()))
    }

    #[test]
    fn bloom_cache_get_or_load_hits_and_misses() {
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let layer = small_layer("test", None);
        backend.store_layer(&layer).unwrap();

        let cache = MemoryBloomCache::new(Arc::clone(&backend));

        // First get is a miss against the cache; loads from backend.
        let bloom1 = cache
            .get_or_load(layer.id())
            .unwrap()
            .expect("bloom present in backend");
        assert_eq!(cache.stats().entries, 1);
        assert_eq!(cache.stats().misses, 1);

        // Second get is a cache hit. Hits aren't counted (the counter cost a
        // write lock on the hottest path); the hit is proven by the unchanged
        // miss count + the same Arc being returned (no backend reload).
        let bloom2 = cache.get_or_load(layer.id()).unwrap().expect("hit");
        assert_eq!(cache.stats().misses, 1);
        assert!(Arc::ptr_eq(&bloom1, &bloom2));
    }

    #[test]
    fn bloom_cache_returns_none_when_backend_has_no_bloom() {
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let cache = MemoryBloomCache::new(backend);
        // No layer stored — load_bloom returns None.
        let bogus = LayerId([7u8; 32]);
        assert!(cache.get_or_load(&bogus).unwrap().is_none());
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().entries, 0);
    }

    #[test]
    fn bloom_cache_evict_drops_entry() {
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let layer = small_layer("test", None);
        backend.store_layer(&layer).unwrap();

        let cache = MemoryBloomCache::new(Arc::clone(&backend));
        let _ = cache.get_or_load(layer.id()).unwrap();
        assert_eq!(cache.stats().entries, 1);

        cache.evict_layer(layer.id());
        assert_eq!(cache.stats().entries, 0);

        // Next get reloads from backend (counts as miss).
        let _ = cache.get_or_load(layer.id()).unwrap();
        assert_eq!(cache.stats().misses, 2);
    }

    #[test]
    fn bloom_cache_uses_loaded_bloom_for_might_contain() {
        // End-to-end: store a layer with known IRIs, fetch the bloom via
        // the cache, query it. Verifies the bloom round-trips intact
        // through the backend (PB::store_bloom / PB::load_bloom path).
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let mut builder = LayerBuilder::new("test", None);
        for i in 0..50 {
            let mut r = Resource::new(iri(&format!("urn:eigenius:test:r{i}")));
            r.set(iri("urn:eigenius:test:p"), Value::Integer(i));
            builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(builder.build(LayerStorage::in_memory()));
        backend.store_layer(&layer).unwrap();

        let cache = MemoryBloomCache::new(Arc::clone(&backend));
        let bloom = cache.get_or_load(layer.id()).unwrap().expect("present");

        // No false negatives for the inserted IRIs.
        for i in 0..50 {
            assert!(bloom.might_contain(&iri(&format!("urn:eigenius:test:r{i}"))));
        }
        // And the loaded bloom matches what we'd build directly from
        // the layer's visibility state (defined ∪ tombstoned).
        let expected = BloomFilter::for_layer(layer.defined_iris(), layer.tombstoned_iris());
        assert_eq!(*bloom, expected);
    }

    // --- BoundedResourceCache tests ---

    /// Helper: run pending eviction tasks and return current entry count.
    /// moka eviction is eventually-consistent; tests assert on the
    /// post-`run_pending_tasks` snapshot for determinism.
    fn bounded_total(cache: &BoundedResourceCache) -> u64 {
        cache.stats().entries
    }

    #[test]
    fn bounded_cache_routes_by_tier() {
        // Entries inserted as Active land in the active pool; ditto
        // Historical. The per-pool counters in `CacheStats` reflect this.
        let cache = BoundedResourceCache::new(100);
        let key_a = ResourceKey::new(lid(1), iri("urn:eigenius:test:a"));
        let key_h = ResourceKey::new(lid(1), iri("urn:eigenius:test:h"));

        cache.put(
            key_a.clone(),
            make_resource("urn:eigenius:test:a"),
            CacheTier::Active,
        );
        cache.put(
            key_h.clone(),
            make_resource("urn:eigenius:test:h"),
            CacheTier::Historical,
        );

        let stats = cache.stats();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.active_entries, 1);
        assert_eq!(stats.historical_entries, 1);

        // Both reachable via `get` (no tier hint needed at read time).
        assert!(cache.get(&key_a).is_some());
        assert!(cache.get(&key_h).is_some());
    }

    #[test]
    fn bounded_cache_re_put_moves_between_pools() {
        // Putting an existing key with a different tier moves the entry
        // — the previous-pool copy is invalidated. Important so
        // promotion/demotion (lazy in 14c-i, automatic in a future
        // 14c-ii) doesn't leave duplicate entries.
        let cache = BoundedResourceCache::new(100);
        let key = ResourceKey::new(lid(1), iri("urn:eigenius:test:k"));

        cache.put(
            key.clone(),
            make_resource("urn:eigenius:test:k"),
            CacheTier::Historical,
        );
        assert_eq!(cache.stats().historical_entries, 1);
        assert_eq!(cache.stats().active_entries, 0);

        // Re-put as Active.
        cache.put(
            key.clone(),
            make_resource("urn:eigenius:test:k"),
            CacheTier::Active,
        );
        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.active_entries, 1);
        assert_eq!(stats.historical_entries, 0);
    }

    #[test]
    fn bounded_cache_honors_pool_capacities() {
        // 60/40 split of total=10 → active=6, historical=4. Stuff each
        // pool past its capacity and verify entries don't unbound. moka
        // eviction is eventually-consistent so we trigger pending tasks
        // via `run_pending_tasks` (called inside `stats()`).
        let cache = BoundedResourceCache::new(10);
        for i in 0..30 {
            let key = ResourceKey::new(lid(1), iri(&format!("urn:eigenius:test:a{i}")));
            cache.put(
                key,
                make_resource(&format!("urn:eigenius:test:a{i}")),
                CacheTier::Active,
            );
        }
        for i in 0..30 {
            let key = ResourceKey::new(lid(1), iri(&format!("urn:eigenius:test:h{i}")));
            cache.put(
                key,
                make_resource(&format!("urn:eigenius:test:h{i}")),
                CacheTier::Historical,
            );
        }
        let stats = cache.stats();
        // Per-pool bound: active≈6, historical≈4. Allow modest overshoot
        // since moka's bound is eventually-consistent under heavy churn.
        assert!(
            stats.active_entries <= 8,
            "active overshoot: {} (cap was 6)",
            stats.active_entries
        );
        assert!(
            stats.historical_entries <= 6,
            "historical overshoot: {} (cap was 4)",
            stats.historical_entries
        );
        assert!(stats.entries < 60, "total overshoot: {}", stats.entries);
    }

    #[test]
    fn bounded_cache_evict_layer_drops_only_that_layer() {
        let cache = BoundedResourceCache::new(100);
        let l1_a = ResourceKey::new(lid(1), iri("urn:eigenius:test:A"));
        let l1_b = ResourceKey::new(lid(1), iri("urn:eigenius:test:B"));
        let l2_a = ResourceKey::new(lid(2), iri("urn:eigenius:test:A"));

        cache.put(
            l1_a.clone(),
            make_resource("urn:eigenius:test:A"),
            CacheTier::Active,
        );
        cache.put(
            l1_b.clone(),
            make_resource("urn:eigenius:test:B"),
            CacheTier::Historical,
        );
        cache.put(
            l2_a.clone(),
            make_resource("urn:eigenius:test:A"),
            CacheTier::Active,
        );
        assert_eq!(bounded_total(&cache), 3);

        cache.evict_layer(&lid(1));
        // Both l1 entries (across both pools) gone; l2 survives.
        assert_eq!(bounded_total(&cache), 1);
        assert!(cache.get(&l1_a).is_none());
        assert!(cache.get(&l1_b).is_none());
        assert!(cache.get(&l2_a).is_some());
    }

    #[test]
    fn bounded_cache_hits_and_misses_counted() {
        let cache = BoundedResourceCache::new(10);
        let key = ResourceKey::new(lid(1), iri("urn:eigenius:test:k"));

        // Miss before insert.
        assert!(cache.get(&key).is_none());
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 0);

        cache.put(
            key.clone(),
            make_resource("urn:eigenius:test:k"),
            CacheTier::Active,
        );
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.stats().hits, 1);
    }
}
