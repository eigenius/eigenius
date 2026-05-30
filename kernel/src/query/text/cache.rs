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

//! D43 §2.3 / M3.4 — query-time caches for text retrieval.
//!
//! This module ships [`DocsCache`], a bounded LRU keyed by
//! `(index_iri, layer_id)` that holds deserialised
//! [`crate::layer::TextDocs`] structures. Hot layers are likely to
//! be touched by many consecutive queries (the same head, the same
//! active TextIndex, possibly different query strings); the
//! CBOR-decode of the per-`(index, layer)` docs blob then becomes
//! free.
//!
//! **Why no `TermCache`.** The trait surface already provides
//! [`crate::layer::TextIndex::intersect_layer`], which does the
//! AND of multiple posting lists in the backend's native bitmap
//! representation in a single call. Caching the per-term decoded
//! posting at the orchestrator layer would either duplicate the
//! backend's representation (memory-hungry, and the memory backend
//! already holds them decoded internally) or pay the format-
//! conversion cost on hit (which defeats the cache). A
//! backend-internal cache — RocksDB block cache for the
//! `text_term:` key family, plus the in-process Roaring bitmap's
//! own internal sharing — is the right layer to optimise this if
//! profiling shows it matters. Deferred until that signal exists.
//!
//! The cache is `Send + Sync` so it can be held inside an
//! `ExecutionContext` shared across concurrent query handlers; the
//! moka backing provides lock-free reads on the hot path.

use crate::layer::{LayerId, TextDocs};
use crate::ontology::iri::Iri;
use moka::sync::Cache;
use std::sync::Arc;

/// Bounded LRU on deserialised per-`(index, layer)` `TextDocs`
/// snapshots.
///
/// Entries hold `Arc<TextDocs>` so multiple consumers (concurrent
/// queries, the planner's selectivity estimator in M3.6) share the
/// same allocation without cloning the subject IRI list.
pub struct DocsCache {
    inner: Cache<(Iri, LayerId), Arc<TextDocs>>,
}

impl DocsCache {
    /// Create a fresh cache with the given entry budget. A budget
    /// of `0` produces a cache that never holds anything — useful
    /// for tests and for memory-constrained deployments where the
    /// docs blob is large enough that one cached entry is itself
    /// expensive.
    pub fn new(max_entries: u64) -> Self {
        Self {
            inner: Cache::new(max_entries),
        }
    }

    /// Probe the cache. Returns `Some(Arc<TextDocs>)` on hit.
    pub fn get(&self, index: &Iri, layer: &LayerId) -> Option<Arc<TextDocs>> {
        self.inner.get(&(index.clone(), layer.clone()))
    }

    /// Admit a fresh entry. If the cache is at capacity the
    /// least-recently-used entry is evicted.
    pub fn insert(&self, index: Iri, layer: LayerId, docs: Arc<TextDocs>) {
        self.inner.insert((index, layer), docs);
    }

    /// Invalidate a single entry. Called by the M2.7 `delete_layer`
    /// path so a layer's docs blob doesn't survive its GC.
    pub fn invalidate(&self, index: &Iri, layer: &LayerId) {
        self.inner.invalidate(&(index.clone(), layer.clone()));
    }

    /// Invalidate every cached entry — used after consolidation
    /// (D43 §2.8) when many `(index, layer)` pairs are replaced at
    /// once and per-entry invalidation would be expensive.
    pub fn invalidate_all(&self) {
        self.inner.invalidate_all();
    }

    /// Current entry count (approximate — moka maintains it
    /// eventually-consistently).
    pub fn approximate_count(&self) -> u64 {
        self.inner.entry_count()
    }
}

impl Default for DocsCache {
    /// 1024-entry default — fits a chain of ~1000 layers, each with
    /// modest doc counts under a single TextIndex. Configurable per
    /// deployment when this proves over- or under-budgeted.
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

    fn dummy_docs(n_subjects: usize) -> Arc<TextDocs> {
        let subjects = (0..n_subjects)
            .map(|i| iri(&format!("urn:eigenius:test:s{i}")))
            .collect();
        let doc_lengths = vec![1u32; n_subjects];
        Arc::new(TextDocs {
            subjects,
            doc_lengths,
        })
    }

    /// Round-trip: insert + get returns the same Arc.
    #[test]
    fn insert_then_get_round_trips() {
        let cache = DocsCache::new(16);
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let docs = dummy_docs(3);
        cache.insert(i1.clone(), l1.clone(), Arc::clone(&docs));

        let got = cache.get(&i1, &l1).expect("entry should be cached");
        assert_eq!(got.subjects.len(), 3);
        assert!(Arc::ptr_eq(&docs, &got), "should be the same allocation");
    }

    /// Miss returns None.
    #[test]
    fn miss_returns_none() {
        let cache = DocsCache::new(16);
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        assert!(cache.get(&i1, &l1).is_none());
    }

    /// Different `(index, layer)` pairs occupy independent
    /// entries.
    #[test]
    fn distinct_keys_are_independent() {
        let cache = DocsCache::new(16);
        let i1 = iri("urn:eigenius:test:i1");
        let i2 = iri("urn:eigenius:test:i2");
        let l1 = layer_id(1);
        let l2 = layer_id(2);

        cache.insert(i1.clone(), l1.clone(), dummy_docs(1));
        cache.insert(i2.clone(), l1.clone(), dummy_docs(2));
        cache.insert(i1.clone(), l2.clone(), dummy_docs(3));

        assert_eq!(cache.get(&i1, &l1).unwrap().subjects.len(), 1);
        assert_eq!(cache.get(&i2, &l1).unwrap().subjects.len(), 2);
        assert_eq!(cache.get(&i1, &l2).unwrap().subjects.len(), 3);
    }

    /// `invalidate` removes one entry without affecting others.
    #[test]
    fn invalidate_one_entry() {
        let cache = DocsCache::new(16);
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let l2 = layer_id(2);
        cache.insert(i1.clone(), l1.clone(), dummy_docs(1));
        cache.insert(i1.clone(), l2.clone(), dummy_docs(2));

        cache.invalidate(&i1, &l1);
        cache.inner.run_pending_tasks();

        assert!(cache.get(&i1, &l1).is_none());
        assert!(cache.get(&i1, &l2).is_some());
    }

    /// `invalidate_all` clears every entry.
    #[test]
    fn invalidate_all_clears_cache() {
        let cache = DocsCache::new(16);
        let i1 = iri("urn:eigenius:test:i1");
        for byte in 0u8..5 {
            cache.insert(i1.clone(), layer_id(byte), dummy_docs(byte as usize + 1));
        }
        cache.invalidate_all();
        cache.inner.run_pending_tasks();
        for byte in 0u8..5 {
            assert!(cache.get(&i1, &layer_id(byte)).is_none());
        }
    }

    /// Default cache is constructible and starts empty.
    #[test]
    fn default_cache_starts_empty() {
        let cache = DocsCache::default();
        let i1 = iri("urn:eigenius:test:i1");
        assert!(cache.get(&i1, &layer_id(0)).is_none());
    }
}
