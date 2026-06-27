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
    BloomCache, BoundedResourceCache, LayerId, MemoryBloomCache, MemoryResourceBackend,
    MemoryResourceCache, MemoryTextIndex, MemoryTripleIndex, MemoryValueIndex, MemoryVectorIndex,
    NoRedirects, RedirectMap, ResourceCache, TextIndex, TripleIndex, ValueIndex, VectorIndex,
};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::storage::{PersistentBackend, ResourceBackend};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock, RwLock};

/// Staging for a freshly-**built** layer's resources, held until `store_layer` persists
/// them to the backend (D23 write path). A built `Layer` is metadata-only — its
/// resources have no durable home until the persist step — so they cannot live solely
/// in the (bounded) resource cache: a layer larger than the cache budget would have its
/// own resources evicted before `store_layer` reads them, silently losing data. The
/// `PendingStage` is that durable-until-persist home: `LayerBuilder::build` inserts the
/// layer's resources here, `Layer::get_resource` consults it first, and `store_layer`
/// **drains** the entry once the resources are on the backend. So at any moment it holds
/// only the in-flight (built-but-unpersisted) layers — typically one — while committed
/// layers live on the backend and page through the bounded cache. Keyed by `LayerId`.
pub type PendingStage = Arc<RwLock<HashMap<LayerId, BTreeMap<Iri, Arc<Resource>>>>>;

/// A fresh, empty [`PendingStage`].
fn new_pending() -> PendingStage {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Default bounded resource-cache budget (entry count) for a persistent-backed
/// [`LayerStorage`] when the process hasn't configured one (D23 §5.3). ~250k resource
/// entries — at a ~1 KiB mean, a few hundred MB resident — with cold reads paging from
/// the backend. Bounding is essential: over a durable backend an *unbounded* cache holds
/// every resource ever touched, so a bulk load (e.g. a domain-lexicon import) grows the
/// process without limit. Override per-process via [`set_cache_budget`] (the
/// `serve --cache-budget` flag) or per-call via [`LayerStorage::with_persistent_bounded`].
pub const DEFAULT_CACHE_BUDGET_ENTRIES: u64 = 250_000;

static CACHE_BUDGET: OnceLock<u64> = OnceLock::new();

/// Set the process-wide resource-cache budget (entry count) that
/// [`LayerStorage::with_persistent`] sizes its bounded cache to. Call once at startup
/// (e.g. from `serve`), before any persistent storage is constructed; set-once, so later
/// calls are ignored. Unset ⇒ [`DEFAULT_CACHE_BUDGET_ENTRIES`].
pub fn set_cache_budget(total_entries: u64) {
    let _ = CACHE_BUDGET.set(total_entries);
}

/// The configured process-wide resource-cache budget, or [`DEFAULT_CACHE_BUDGET_ENTRIES`].
pub fn cache_budget() -> u64 {
    CACHE_BUDGET
        .get()
        .copied()
        .unwrap_or(DEFAULT_CACHE_BUDGET_ENTRIES)
}

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
    /// Resources of freshly-built, not-yet-persisted layers (see [`PendingStage`]).
    /// Shared across all layers built on this storage; drained by `store_layer`.
    pub pending: PendingStage,
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
            pending: new_pending(),
        }
    }

    /// Storage bound to a `PersistentBackend` (typically `RocksStore`) with a
    /// **bounded** two-pool resource cache sized to the process-wide [`cache_budget`]
    /// ([`DEFAULT_CACHE_BUDGET_ENTRIES`] unless `serve --cache-budget` set otherwise).
    /// This is the production constructor: over a durable backend the cache is a
    /// bounded *hint* (misses page from the backend), never the source of truth, so
    /// bounding it caps resident memory regardless of how much is loaded. Use
    /// [`with_persistent_bounded`](Self::with_persistent_bounded) to pin an explicit
    /// per-call budget; [`in_memory`](Self::in_memory) for the backend-less path (whose
    /// cache *is* the source of truth and so stays unbounded).
    pub fn with_persistent(pb: Arc<dyn PersistentBackend>) -> Self {
        Self::with_persistent_bounded(pb, cache_budget())
    }

    /// Like [`with_persistent`](Self::with_persistent) but with an explicitly unbounded
    /// resource cache — every resolved resource is retained until its layer is evicted.
    /// Only for short-lived processes / small workloads where holding everything is fine;
    /// production should prefer the bounded `with_persistent`.
    pub fn with_persistent_unbounded(pb: Arc<dyn PersistentBackend>) -> Self {
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
            pending: new_pending(),
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
            pending: new_pending(),
        }
    }
}
