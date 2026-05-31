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

//! D43 §5.9 / M5.6 — vector SegmentCache.
//!
//! Bounded LRU on deserialised per-`(index_iri, layer_id)`
//! [`crate::layer::SegmentView`] snapshots. Without this, every
//! `VECTOR_NEAR` / `VECTOR_SIM` probe re-fetches and (in the
//! RocksDB-backed path) re-deserialises the segment from CBOR; with
//! it, the second probe against the same `(index, layer)` is a
//! `BTreeMap` lookup plus an `Arc` clone.
//!
//! Sized analogously to [`crate::query::text::cache::DocsCache`]:
//! moka-backed for lock-free reads on the hot path, entry-count
//! budget for simplicity. Per-entry cost is dominated by the
//! `vectors: Vec<f32>` payload — `count × dim × 4 bytes` — plus
//! the subject IRI list. A 1024-entry default fits a chain of
//! ~1000 layers with modest per-layer vector counts; deployments
//! whose segment sizes push the byte budget should configure a
//! smaller entry budget rather than reasoning about absolute MiB
//! caps (the §5.9 design's 256 MiB language is the size-weighted
//! target the RocksDB-backed SegmentCache will eventually honour;
//! v1 ships entry-count, switchable without surface change).
//!
//! **Invalidation.** `delete_layer(L)` invalidates every entry
//! under that layer's id; D43 §2.8 consolidation invalidates the
//! collapsed range and admits the consolidated segment.
//! [`SegmentCache::invalidate_all`] is the bulk path used by
//! consolidation / reindex sweeps.

use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::query::vector::segment::SegmentView;
use moka::sync::Cache;
use std::sync::Arc;

/// Bounded LRU on deserialised per-`(index, layer)` `SegmentView`
/// snapshots.
///
/// Entries hold `Arc<SegmentView>` so multiple consumers
/// (concurrent `VECTOR_NEAR` probes; the planner's hybrid scoring
/// in M7) share the same allocation without cloning the per-vector
/// `f32` payload.
pub struct SegmentCache {
    inner: Cache<(Iri, LayerId), Arc<SegmentView>>,
}

impl SegmentCache {
    /// Create a cache with the given maximum-entry budget. A
    /// budget of `0` produces a cache that never holds anything.
    pub fn new(max_entries: u64) -> Self {
        Self {
            inner: Cache::new(max_entries),
        }
    }

    /// Probe the cache. Returns `Some(Arc<SegmentView>)` on hit.
    pub fn get(&self, index: &Iri, layer: &LayerId) -> Option<Arc<SegmentView>> {
        self.inner.get(&(index.clone(), layer.clone()))
    }

    /// Admit a fresh entry. If the cache is at capacity the
    /// least-recently-used entry is evicted.
    pub fn insert(&self, index: Iri, layer: LayerId, segment: Arc<SegmentView>) {
        self.inner.insert((index, layer), segment);
    }

    /// Invalidate a single entry. Called by the M2.7 `delete_layer`
    /// path so a layer's segment doesn't survive its GC.
    pub fn invalidate(&self, index: &Iri, layer: &LayerId) {
        self.inner.invalidate(&(index.clone(), layer.clone()));
    }

    /// Invalidate every cached entry — used after consolidation
    /// (D43 §2.8) and atomic reindex (§5.7) when many `(index,
    /// layer)` pairs are replaced at once and per-entry
    /// invalidation would be expensive.
    pub fn invalidate_all(&self) {
        self.inner.invalidate_all();
    }

    /// Current entry count (approximate — moka maintains it
    /// eventually-consistently).
    pub fn approximate_count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Test-only handle to flush moka's pending eviction tasks
    /// synchronously. Without this, `approximate_count` and
    /// post-invalidate `get` checks race the background
    /// maintenance thread.
    #[cfg(test)]
    pub(crate) fn run_pending_tasks(&self) {
        self.inner.run_pending_tasks();
    }
}

impl Default for SegmentCache {
    /// 1024-entry default — same shape as `DocsCache`'s default.
    /// Configurable per deployment.
    fn default() -> Self {
        Self::new(1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn layer_id(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }

    fn dummy_segment(n: usize, dim: u32) -> Arc<SegmentView> {
        Arc::new(SegmentView::from_segment(crate::layer::VectorSegment {
            model_iri: iri("urn:eigenius:embed:test:m1"),
            dim,
            distance: "cosine".into(),
            subjects: (0..n)
                .map(|i| iri(&format!("urn:eigenius:test:s{i}")))
                .collect(),
            vectors: vec![0.5f32; n * dim as usize],
            hnsw_graph_bytes: None,
        }))
    }

    #[test]
    fn insert_then_get_round_trips() {
        let cache = SegmentCache::new(16);
        let i1 = iri("urn:eigenius:test:vi");
        let l1 = layer_id(1);
        let seg = dummy_segment(3, 4);
        cache.insert(i1.clone(), l1.clone(), Arc::clone(&seg));

        let got = cache.get(&i1, &l1).expect("cached");
        assert_eq!(got.count(), 3);
        assert!(Arc::ptr_eq(&seg, &got), "same allocation");
    }

    #[test]
    fn miss_returns_none() {
        let cache = SegmentCache::new(16);
        let i1 = iri("urn:eigenius:test:vi");
        assert!(cache.get(&i1, &layer_id(0)).is_none());
    }

    #[test]
    fn distinct_keys_independent() {
        let cache = SegmentCache::new(16);
        let i1 = iri("urn:eigenius:test:vi_a");
        let i2 = iri("urn:eigenius:test:vi_b");
        cache.insert(i1.clone(), layer_id(1), dummy_segment(1, 4));
        cache.insert(i2.clone(), layer_id(1), dummy_segment(2, 4));
        cache.insert(i1.clone(), layer_id(2), dummy_segment(3, 4));

        assert_eq!(cache.get(&i1, &layer_id(1)).unwrap().count(), 1);
        assert_eq!(cache.get(&i2, &layer_id(1)).unwrap().count(), 2);
        assert_eq!(cache.get(&i1, &layer_id(2)).unwrap().count(), 3);
    }

    #[test]
    fn invalidate_one_entry() {
        let cache = SegmentCache::new(16);
        let i1 = iri("urn:eigenius:test:vi");
        cache.insert(i1.clone(), layer_id(1), dummy_segment(1, 4));
        cache.insert(i1.clone(), layer_id(2), dummy_segment(2, 4));

        cache.invalidate(&i1, &layer_id(1));
        cache.run_pending_tasks();

        assert!(cache.get(&i1, &layer_id(1)).is_none());
        assert!(cache.get(&i1, &layer_id(2)).is_some());
    }

    #[test]
    fn invalidate_all_clears_cache() {
        let cache = SegmentCache::new(16);
        let i1 = iri("urn:eigenius:test:vi");
        for byte in 0u8..5 {
            cache.insert(
                i1.clone(),
                layer_id(byte),
                dummy_segment(byte as usize + 1, 4),
            );
        }
        cache.invalidate_all();
        cache.run_pending_tasks();
        for byte in 0u8..5 {
            assert!(cache.get(&i1, &layer_id(byte)).is_none());
        }
    }

    #[test]
    fn zero_budget_never_caches() {
        let cache = SegmentCache::new(0);
        let i1 = iri("urn:eigenius:test:vi");
        cache.insert(i1.clone(), layer_id(0), dummy_segment(1, 4));
        cache.run_pending_tasks();
        assert!(cache.get(&i1, &layer_id(0)).is_none());
    }

    #[test]
    fn default_cache_starts_empty() {
        let cache = SegmentCache::default();
        assert!(cache
            .get(&iri("urn:eigenius:test:vi"), &layer_id(0))
            .is_none());
    }
}
