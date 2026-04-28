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
    BloomCache, MemoryBloomCache, MemoryResourceBackend, MemoryResourceCache, ResourceCache,
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
        }
    }

    /// Storage bound to a `PersistentBackend` (typically `RocksStore`).
    /// The same Arc serves as the resource backend (via the
    /// `PersistentBackend: ResourceBackend` supertrait) and as the bloom
    /// cache's fall-through. Cold-cache reads hit the backend on demand.
    pub fn with_persistent(pb: Arc<dyn PersistentBackend>) -> Self {
        Self {
            cache: Arc::new(MemoryResourceCache::new()),
            backend: Arc::clone(&pb) as Arc<dyn ResourceBackend>,
            bloom_cache: Arc::new(MemoryBloomCache::new(pb)),
        }
    }
}
