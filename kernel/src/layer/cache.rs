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

use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use std::collections::BTreeMap;
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

/// Read/write cache for resources, keyed by `(LayerId, Iri)`.
///
/// Implementations may evict at any time; the cache is a hint, not a source
/// of truth. Misses fall through to the persistent backend (§5.4.2). Phase
/// 14c replaces the naïve implementation with a bounded two-pool ARC cache;
/// the trait surface is shaped to accommodate both.
pub trait ResourceCache: Send + Sync {
    /// Look up a resource. Returns `None` on miss; the caller falls through
    /// to the persistent backend.
    fn get(&self, key: &ResourceKey) -> Option<Arc<Resource>>;

    /// Insert or replace a resource. Implementations may evict other entries
    /// to make room.
    fn put(&self, key: ResourceKey, resource: Arc<Resource>);

    /// Drop all entries for a given layer. Called by GC (§5.7) when a layer
    /// is swept; also by branch pruning (§5.8).
    fn evict_layer(&self, layer: &LayerId);

    /// Snapshot of basic counters. Implementations may report zeros for
    /// counters they don't track. Real metrics ship with 14c's two-pool
    /// implementation.
    fn stats(&self) -> CacheStats;
}

/// Coarse counters reported by `ResourceCache::stats`. Implementations that
/// don't track a particular field may report 0.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Number of entries currently held.
    pub entries: u64,
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

    fn put(&self, key: ResourceKey, resource: Arc<Resource>) {
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
            hits: state.hits,
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
        cache.put(key.clone(), make_resource("urn:eigenius:example:A"));
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
        cache.put(key.clone(), make_resource("urn:eigenius:example:A"));
        cache.put(key.clone(), make_resource("urn:eigenius:example:A"));
        assert_eq!(cache.stats().entries, 1);
    }

    #[test]
    fn distinct_layers_are_distinct_keys() {
        let cache = MemoryResourceCache::new();
        let key_a = ResourceKey::new(lid(1), iri("urn:eigenius:example:X"));
        let key_b = ResourceKey::new(lid(2), iri("urn:eigenius:example:X"));
        cache.put(key_a, make_resource("urn:eigenius:example:X"));
        cache.put(key_b, make_resource("urn:eigenius:example:X"));
        assert_eq!(cache.stats().entries, 2);
    }

    #[test]
    fn evict_layer_drops_only_that_layer() {
        let cache = MemoryResourceCache::new();
        let l1_a = ResourceKey::new(lid(1), iri("urn:eigenius:example:A"));
        let l1_b = ResourceKey::new(lid(1), iri("urn:eigenius:example:B"));
        let l2_a = ResourceKey::new(lid(2), iri("urn:eigenius:example:A"));

        cache.put(l1_a.clone(), make_resource("urn:eigenius:example:A"));
        cache.put(l1_b.clone(), make_resource("urn:eigenius:example:B"));
        cache.put(l2_a.clone(), make_resource("urn:eigenius:example:A"));
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
        cache.put(key.clone(), Arc::clone(&resource));

        // Strong count: original + cache-held = 2.
        assert_eq!(Arc::strong_count(&resource), 2);

        // Each get bumps the count while the returned Arc is alive.
        let got = cache.get(&key).expect("expected hit");
        assert_eq!(Arc::strong_count(&resource), 3);
        drop(got);
        assert_eq!(Arc::strong_count(&resource), 2);
    }
}
