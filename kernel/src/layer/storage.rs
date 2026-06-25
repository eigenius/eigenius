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

//! `LayerStorage` — the bundle of caches and backends a `Layer` needs.
//!
//! Every `Layer` carries a resource cache, a resource backend, and a
//! per-layer shadowing bloom cache. Phase 14 keeps adding such handles
//! (a triple-pattern index in 14h, a GC-roots tracker in 14f, possibly an
//! IRI dictionary later). Threading each as an independent argument
//! through `LayerBuilder::build`, `Layer::from_handle`, `build_chain`,
//! and `ExecutionContext::new` produces 50+ call sites that all need
//! coordinated updates whenever a component is added.
//!
//! `LayerStorage` is the parameter object: one struct, cloned cheaply
//! (each field is an `Arc`). New components become a new field plus an
//! update to the constructors below; call sites stay unchanged.

use crate::layer::{
    BloomCache, BoundedResourceCache, MemoryBloomCache, MemoryResourceBackend, MemoryResourceCache,
    MemoryTextIndex, MemoryTripleIndex, MemoryValueIndex, MemoryVectorIndex, NoRedirects,
    RedirectMap, ResourceCache, TextIndex, TripleIndex, ValueIndex, VectorIndex,
};
use crate::storage::{PersistentBackend, ResourceBackend};
use std::sync::Arc;

/// Bundle of storage handles a `Layer` consults to read its content,
/// resolve through its parent chain, and produce committed children.
///
/// All fields are `Arc`s, so cloning a `LayerStorage` is three (or however
/// many components are present) atomic increments. Layers, contexts, and
/// chain builders share copies freely.
#[derive(Clone)]
pub struct LayerStorage {
    /// Resource content cache (`(LayerId, Iri) → Arc<Resource>`). Misses
    /// fall through to `backend`.
    pub cache: Arc<dyn ResourceCache>,
    /// Persistent resource read surface. In production this is typically
    /// the same Arc as the bloom cache's fall-through `PersistentBackend`,
    /// upcast to `ResourceBackend`.
    pub backend: Arc<dyn ResourceBackend>,
    /// Per-layer shadowing bloom cache (D23 §5.2). On miss falls through
    /// to its own `PersistentBackend` Arc (set when the cache was built);
    /// `Layer::resolve` consults it before probing the resource cache.
    pub bloom_cache: Arc<dyn BloomCache>,
    /// Per-layer triple index (D23 §5.9 / Phase 14h). Populated at
    /// commit time inside `store_layer`'s atomic batch; consulted by
    /// the EigenQL evaluator's `scan_chain` helper. In-memory layers
    /// share a fresh `MemoryTripleIndex`; persistent layers share the
    /// backend's `as_triple_index()` view.
    pub triple_index: Arc<dyn TripleIndex>,
    /// Per-`(TextIndex Resource, layer)` inverted index (D43 §2.3).
    /// Populated by `LayerBuilder::build` (M2.6) — discovers active
    /// `core:TextIndex` Resources at the commit head and indexes
    /// each indexed property's tokens. Consulted by the EigenQL
    /// text retrieval path (M3).
    pub text_index: Arc<dyn TextIndex>,
    /// Per-`(VectorIndex Resource, layer)` vector segment store
    /// (D43 §2.4). Populated by the M5 post-Load embedding sweep;
    /// consulted by the EigenQL vector retrieval path (M5+ for the
    /// flat path; M6 for HNSW).
    pub vector_index: Arc<dyn VectorIndex>,
    /// Per-`(ValueIndex Resource, layer)` exact value index (D65).
    /// Pre-populated by `LayerBuilder::build` (like the triple index) —
    /// discovers active `core:ValueIndex` Resources at the head and keys
    /// each target property's normalized value to its subjects. Consulted
    /// by the lazy lexicon lookup (and exact literal-property queries).
    pub value_index: Arc<dyn ValueIndex>,
    /// In-memory cache of installed resolve redirects (D25 §12.8 /
    /// Phase 17f). Populated at `with_persistent` time from the
    /// backend's `list_redirects()`; consulted by `build_chain` to
    /// populate `Layer::redirect_target` per layer. `in_memory()`
    /// uses a `NoRedirects` no-op shim.
    pub redirect_map: Arc<dyn RedirectMap>,
    /// Optional persistent-backend handle for redirect resolution
    /// during `build_chain`. When a layer is a redirect source,
    /// `build_chain` calls `load_chain_from` on this backend to
    /// fetch the target's chain. `None` for in-memory storage —
    /// redirects can't be resolved there, but `redirect_map` is also
    /// empty so the case never arises.
    pub persistent_backend: Option<Arc<dyn PersistentBackend>>,
}

impl LayerStorage {
    /// In-memory storage for tests and the non-persistent bootstrap path.
    /// Resource backend is empty (`MemoryResourceBackend` with no inserts);
    /// bloom cache is cache-only with no backend fall-through. Built
    /// layers populate both eagerly via `LayerBuilder::build`.
    pub fn in_memory() -> Self {
        Self {
            cache: Arc::new(MemoryResourceCache::new()),
            backend: Arc::new(MemoryResourceBackend::new()),
            bloom_cache: Arc::new(MemoryBloomCache::cache_only()),
            triple_index: Arc::new(MemoryTripleIndex::new()),
            text_index: Arc::new(MemoryTextIndex::new()),
            vector_index: Arc::new(MemoryVectorIndex::new()),
            value_index: Arc::new(MemoryValueIndex::new()),
            redirect_map: Arc::new(NoRedirects),
            persistent_backend: None,
        }
    }

    /// Storage bound to a `PersistentBackend` (typically `RocksStore`)
    /// with an unbounded in-memory resource cache. Suitable for
    /// short-lived processes, tests, and small workloads where the
    /// memory pressure of holding every resolved resource is fine.
    /// For long-running production workloads, use
    /// `with_persistent_bounded`.
    pub fn with_persistent(pb: Arc<dyn PersistentBackend>) -> Self {
        let triple_index = pb.triple_index_arc();
        let text_index = pb.text_index_arc();
        let vector_index = pb.vector_index_arc();
        let value_index = pb.value_index_arc();
        let redirect_map = crate::layer::redirect::redirect_map_from_backend(pb.as_ref());
        Self {
            cache: Arc::new(MemoryResourceCache::new()),
            backend: Arc::clone(&pb) as Arc<dyn ResourceBackend>,
            bloom_cache: Arc::new(MemoryBloomCache::new(Arc::clone(&pb))),
            triple_index,
            text_index,
            vector_index,
            value_index,
            redirect_map,
            persistent_backend: Some(pb),
        }
    }

    /// Storage bound to a `PersistentBackend` with a **bounded**
    /// two-pool resource cache (D23 §5.3 / Phase 14c). `total_entries`
    /// is the combined entry budget across both pools; the active pool
    /// gets 60% by default and the historical pool 40%. Cold-cache
    /// reads hit the backend on demand; evicted entries reload on next
    /// access.
    ///
    /// `total_entries` is an entry count (not byte budget). Pick a value
    /// such that worst-case total memory — entries × average resource
    /// size — fits the deployment's RAM target. A common starting point
    /// for ~1 KiB-mean resources is 1M entries (~1 GiB). Phase 12
    /// workload data informs the production default.
    pub fn with_persistent_bounded(pb: Arc<dyn PersistentBackend>, total_entries: u64) -> Self {
        let triple_index = pb.triple_index_arc();
        let text_index = pb.text_index_arc();
        let vector_index = pb.vector_index_arc();
        let value_index = pb.value_index_arc();
        let redirect_map = crate::layer::redirect::redirect_map_from_backend(pb.as_ref());
        Self {
            cache: Arc::new(BoundedResourceCache::new(total_entries)),
            backend: Arc::clone(&pb) as Arc<dyn ResourceBackend>,
            bloom_cache: Arc::new(MemoryBloomCache::new(Arc::clone(&pb))),
            triple_index,
            text_index,
            vector_index,
            value_index,
            redirect_map,
            persistent_backend: Some(pb),
        }
    }
}
