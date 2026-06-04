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

//! D43 §5.3 / M4 — content-addressed embedding cache.
//!
//! Embedder calls are IO-heavy: hosted-API embedders can cost
//! milliseconds-to-seconds per call and may have rate limits. The
//! same `(content, model_iri)` pair always embeds to the same vector
//! (modulo the embedder's own non-determinism — which the cache
//! masks for the lifetime of the entry), so a content-addressed
//! cache turns repeated embeds into local lookups.
//!
//! **Key**: `(sha256(content), model_iri)`. Per D43 §5.3 the spec
//! settles on `blake3(content)` for production; v1 uses SHA-256
//! because `sha2` is already a workspace dependency and the cache
//! lives in-memory only — entries don't outlive the kernel process
//! and the switch to blake3 (when the RocksDB-backed
//! `cf_embed_cache` column family lands) is a one-time
//! invalidation, not a cross-version compatibility concern.
//!
//! **Value**: `Arc<Vec<f32>>` so multiple consumers (the inline
//! `EMBED` evaluator + a future sweep that bulk-embeds at
//! indexing time) share the same allocation.
//!
//! **Storage**: bounded LRU via `moka::sync::Cache` — identical
//! shape to `crate::query::text::cache::DocsCache`. Eviction is
//! triggered by entry count; per-entry memory cost is roughly
//! `dim * 4 bytes` plus map overhead. A 100 000-entry cache at
//! 768-dim costs ~300 MiB; deployments tune via
//! [`EmbeddingCache::new`].
//!
//! **Trace IRI** is not carried in v1. The design (§5.3) records
//! the producing trace IRI alongside the vector so cache hits can
//! audit back to the original Embedder Component invocation; that
//! lands with the broader trace-recording work and slots into this
//! type's value position without disturbing the key.

use crate::ontology::iri::Iri;
use moka::sync::Cache;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Hash of an embedder's input content, treated as opaque bytes by
/// the cache. Wrapping the digest in a named type keeps the key
/// shape self-documenting at the trait surface.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    /// Compute the cache content hash of `text`. v1 uses SHA-256;
    /// the production design (§5.3) specifies blake3 — see
    /// module-level docs for the rationale on the v1 choice.
    pub fn of(text: &str) -> Self {
        let mut h = Sha256::new();
        h.update(text.as_bytes());
        let out = h.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&out);
        ContentHash(bytes)
    }
}

/// Cache key — both halves form the identity, so neither model
/// upgrades nor content changes can produce a stale hit.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct CacheKey {
    pub content_hash: ContentHash,
    pub model_iri: Iri,
}

/// Bounded LRU over `(content_hash, model_iri) → Arc<Vec<f32>>`.
///
/// `Send + Sync` so a single cache can live inside an
/// `ExecutionContext` and serve concurrent query handlers. The moka
/// backing provides lock-free reads on the hot path.
pub struct EmbeddingCache {
    inner: Cache<CacheKey, Arc<Vec<f32>>>,
}

impl EmbeddingCache {
    /// Create a cache with the given maximum-entry budget. A budget
    /// of `0` produces a cache that never holds anything — useful
    /// for tests that want to disable caching, and for memory-
    /// constrained deployments where even one entry is too much
    /// (high-dim models against very large content).
    pub fn new(max_entries: u64) -> Self {
        Self {
            inner: Cache::new(max_entries),
        }
    }

    /// Probe the cache. Returns `Some(Arc<Vec<f32>>)` on hit.
    pub fn get(&self, content: &str, model_iri: &Iri) -> Option<Arc<Vec<f32>>> {
        let key = CacheKey {
            content_hash: ContentHash::of(content),
            model_iri: model_iri.clone(),
        };
        self.inner.get(&key)
    }

    /// Admit a fresh entry. If the cache is at capacity the
    /// least-recently-used entry is evicted.
    pub fn insert(&self, content: &str, model_iri: &Iri, vector: Arc<Vec<f32>>) {
        let key = CacheKey {
            content_hash: ContentHash::of(content),
            model_iri: model_iri.clone(),
        };
        self.inner.insert(key, vector);
    }

    /// Approximate entry count — moka maintains it eventually-
    /// consistently, so this is a "loose" measure useful for
    /// telemetry and for tests that need to verify cache behaviour
    /// after `run_pending_tasks`.
    pub fn approximate_count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Test-only handle that runs moka's pending eviction tasks
    /// synchronously. Without this, `approximate_count` and
    /// post-eviction `get` checks race the background maintenance
    /// thread.
    #[cfg(test)]
    pub(crate) fn run_pending_tasks(&self) {
        self.inner.run_pending_tasks();
    }
}

impl Default for EmbeddingCache {
    /// 100 000-entry default — matches the §5.9 "configurable budget"
    /// language; tunable per deployment when this proves over- or
    /// under-budgeted.
    fn default() -> Self {
        Self::new(100_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    #[test]
    fn content_hash_is_deterministic() {
        let a = ContentHash::of("hello world");
        let b = ContentHash::of("hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn content_hash_differs_on_different_inputs() {
        let a = ContentHash::of("foo");
        let b = ContentHash::of("bar");
        assert_ne!(a, b);
    }

    #[test]
    fn insert_then_get_round_trips() {
        let cache = EmbeddingCache::new(16);
        let model = iri("urn:eigenius:embed:m1");
        cache.insert("hello", &model, Arc::new(vec![0.1f32, 0.2, 0.3]));
        let got = cache.get("hello", &model).expect("cached");
        assert_eq!(got.as_slice(), &[0.1f32, 0.2, 0.3]);
    }

    #[test]
    fn miss_returns_none() {
        let cache = EmbeddingCache::new(16);
        let model = iri("urn:eigenius:embed:m1");
        assert!(cache.get("never inserted", &model).is_none());
    }

    #[test]
    fn distinct_models_dont_collide() {
        let cache = EmbeddingCache::new(16);
        let m1 = iri("urn:eigenius:embed:m1");
        let m2 = iri("urn:eigenius:embed:m2");
        cache.insert("same text", &m1, Arc::new(vec![1.0]));
        cache.insert("same text", &m2, Arc::new(vec![2.0]));
        assert_eq!(cache.get("same text", &m1).unwrap().as_slice(), &[1.0]);
        assert_eq!(cache.get("same text", &m2).unwrap().as_slice(), &[2.0]);
    }

    #[test]
    fn distinct_content_doesnt_collide() {
        let cache = EmbeddingCache::new(16);
        let m1 = iri("urn:eigenius:embed:m1");
        cache.insert("alpha", &m1, Arc::new(vec![1.0]));
        cache.insert("beta", &m1, Arc::new(vec![2.0]));
        assert_eq!(cache.get("alpha", &m1).unwrap().as_slice(), &[1.0]);
        assert_eq!(cache.get("beta", &m1).unwrap().as_slice(), &[2.0]);
    }

    #[test]
    fn zero_budget_never_caches() {
        let cache = EmbeddingCache::new(0);
        let model = iri("urn:eigenius:embed:m1");
        cache.insert("hello", &model, Arc::new(vec![0.1]));
        cache.run_pending_tasks();
        assert!(cache.get("hello", &model).is_none());
    }

    #[test]
    fn arc_sharing_avoids_clones() {
        let cache = EmbeddingCache::new(16);
        let model = iri("urn:eigenius:embed:m1");
        let original = Arc::new(vec![1.0f32, 2.0, 3.0]);
        cache.insert("hello", &model, Arc::clone(&original));
        let got = cache.get("hello", &model).unwrap();
        assert!(Arc::ptr_eq(&original, &got), "should be the same Arc");
    }

    #[test]
    fn default_cache_starts_empty() {
        let cache = EmbeddingCache::default();
        let model = iri("urn:eigenius:embed:m1");
        assert!(cache.get("anything", &model).is_none());
    }
}
