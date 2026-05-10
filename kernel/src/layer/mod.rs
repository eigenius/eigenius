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

//! Layer system for stratified ontology composition.
//!
//! Layers hold resources and form a chain via parent pointers.
//! Each layer sees everything below it as a read-only view.
//! The root layer (no parent) is the core ontology layer.
//!
//! A `Layer` is immutable once built. Use `LayerBuilder` to accumulate
//! resources and produce an immutable `Layer` via `build()`.
//!
//! Phase 14 (D23) introduces a topology / content split: `LayerHandle` and
//! `LayerTopology` (see [`handle`]) describe the DAG without holding any
//! resources, while resource content goes through a `ResourceCache` (see
//! [`cache`]). 14a-i ships those types as pure additions; integration with
//! the legacy `Layer` chain lands in 14a-ii / 14a-iii.

mod bloom;
mod cache;
mod consolidate;
mod handle;
mod index;
mod storage;
mod supporting;

pub use bloom::{BloomFilter, DEFAULT_FPR};
pub use cache::{
    BloomCache, BoundedResourceCache, CacheStats, CacheTier, MemoryBloomCache,
    MemoryResourceBackend, MemoryResourceCache, ResourceCache, ResourceKey,
};
pub use consolidate::{
    consolidate_chain, ConsolidateError, ConsolidateOpts, ConsolidationOutcome, TracePinPolicy,
};
pub use handle::{ChainIter, LayerHandle, LayerTopology};
pub use index::{
    collect_ancestors, extract_indexable_triples, index_keys, is_indexable_predicate, is_shadowed,
    scan_chain, IndexStats, MemoryTripleIndex, OwnedTriple, Triple, TripleIndex,
};
pub use storage::LayerStorage;
pub use supporting::compute_supporting_layer;

/// Construct an `Arc<Layer>` chain from chain metadata.
///
/// Wires each `LayerHandle` from `info.handles` (root → head) into a
/// `Layer` via `Layer::from_handle`, threading parent pointers, the
/// per-layer `defined_iris` set, and the shared `LayerStorage` bundle.
///
/// Returns the head `Arc<Layer>`. The caller normally obtained `info`
/// from `PersistentBackend::load_chain` or `load_chain_from`.
pub fn build_chain(
    info: crate::storage::ChainInfo,
    storage: LayerStorage,
) -> std::sync::Arc<Layer> {
    let mut parent: Option<std::sync::Arc<Layer>> = None;
    for handle in info.handles {
        let id = handle.id.clone();
        let defined = info
            .defined_iris_per_layer
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let layer = Layer::from_handle(handle, parent.clone(), defined, storage.clone());
        parent = Some(std::sync::Arc::new(layer));
    }
    parent.expect("ChainInfo must have at least one handle")
}

use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

/// Position-addressed layer identifier (SHA-256 hash).
///
/// Encodes a layer's *position* in the DAG: the hash covers the layer's
/// content hash and the (sorted) ids of its topological parents. Two
/// layers with identical content but different parent sets get
/// different `LayerId`s — the structural payoff of the position/content
/// split per [D25 §11.0](../../docs/design/d25-chain-consolidation.md)
/// and [D33 §5.1](../../docs/design/d33-partial-order-chains.md).
///
/// [`PositionHash`] is the precision-preferred alias used by code that
/// is making the position-vs-content distinction explicit; `LayerId` is
/// the historical name retained for call-site stability. They are the
/// same type.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct LayerId(pub [u8; 32]);

impl fmt::Debug for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LayerId({})", hex::encode(self.0))
    }
}

impl fmt::Display for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

/// Position-addressed layer identifier — alias for [`LayerId`].
///
/// Prefer this name in new code where the position-vs-content
/// distinction matters (chain walks, parent pointers, branch refs).
/// `LayerId` remains as the historical name; both resolve to the same
/// type.
pub type PositionHash = LayerId;

/// Content-only hash of a layer (SHA-256 over its resources).
///
/// Distinct from [`PositionHash`]: two layers committed at different
/// positions in the DAG with the same resources share a `ContentHash`
/// but have different `PositionHash`es. Used by content-hash dedup
/// (D25 §11.0), tag targets (D25 §12.1), cell-output cache keys
/// (D33 §6), and commutativity-equivalence checks (D33 §5.2) — all of
/// which need a stable identity for chain *content* that's independent
/// of where in the DAG the content was first committed.
///
/// `ContentHash` is a distinct type from `PositionHash` so the type
/// checker catches accidental mixing: passing a content hash where a
/// position hash is expected (or vice versa) is a compile error.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ContentHash(pub [u8; 32]);

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", hex::encode(self.0))
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

/// An immutable layer in the chain.
///
/// **Phase 14a-iii / 14b / 14e structure**: holds metadata (id, name,
/// topological parents, the set of IRIs defined in this specific layer)
/// and a `LayerStorage` bundle (resource cache + backend + per-layer
/// shadowing bloom cache). It does NOT hold full resource content;
/// resource bodies are fetched lazily via the cache, falling through to
/// the backend on miss.
///
/// **Multi-parent topology (Phase 14e).** `parents: Vec<Arc<Layer>>`
/// supports trivial-merge layers with N parents (each parent being a
/// merged head). The chain-walk parent (used by `resolve`'s recursion)
/// is `parents.first()`; non-first parents stay reachable via the
/// topology DAG (`LayerHandle.parents`) for GC, traceability, and
/// time-travel. The merge layer's content is the union of contributions
/// since the LCA, so resolve never needs N-way recursion — anything
/// touched on any merged side is in the merge layer's own
/// `defined_iris`; anything older than LCA is reachable via `parents[0]`.
///
/// **Why `parents.first()` is sufficient for chain walking.** A trivial
/// merge layer's `defined_iris` is the union of all merged sides'
/// post-LCA contributions. Resolve at the merge: if `iri` is in the
/// merge's bloom, hit the merge's content; otherwise recurse to
/// `parents[0]`, which reaches LCA → root. Any IRI older than LCA is
/// found there, regardless of which parent we picked.
///
/// Cloning a Layer is cheap: it clones a `LayerStorage` (a few atomic
/// increments on the bundled Arcs) and shallow-copies the metadata. The
/// parent Arcs are shared.
#[derive(Clone)]
pub struct Layer {
    id: LayerId,
    /// Content-only hash of this layer's resources, independent of position.
    /// Two layers with identical resources at different DAG positions share
    /// this hash. Used by content-hash dedup (D25 §11.0), cell-output cache
    /// keys (D33 §6), and tag targets (D25 §12.1).
    content_hash: ContentHash,
    /// Supporting layer per D33 §4.3 — the youngest ancestor this
    /// layer explicitly depends on. `None` for the root layer, for
    /// layers with no external references, and (transiently) for
    /// pre-PR-0 layers whose supporting layer hasn't been back-filled
    /// yet. Computed by `compute_supporting_layer` and cached here
    /// from `LayerBuilder::build`; PR 0 step 4 adds the persistent
    /// supporting-layer index that back-fills handles loaded from
    /// older storage.
    supporting_layer: Option<LayerId>,
    name: String,
    /// Topological parents. Empty for the root layer; one entry for
    /// every Phase-14d-and-prior layer; multiple entries for
    /// Phase-14e trivial-merge layers. `parent()` returns
    /// `parents.first()` for chain-walk recursion.
    parents: Vec<Arc<Layer>>,
    /// IRIs defined in this layer specifically (not transitively from parents).
    /// Bounded by per-layer resource count. Used by `iter_resources`,
    /// validation, and similar paths that need an exact answer for "what
    /// does this layer define." `Layer::resolve` uses the bloom inside
    /// `storage` instead — for deep chains, the bloom is the right data
    /// structure for the skip-or-probe decision (D23 §5.2).
    defined_iris: BTreeSet<Iri>,
    /// Bundled storage handles. Adding a new handle (e.g., a triple-pattern
    /// index in 14h) is a one-line change to `LayerStorage`; no call-site
    /// churn.
    storage: LayerStorage,
}

impl fmt::Debug for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Layer")
            .field("id", &self.id)
            .field("name", &self.name)
            .field(
                "parent_ids",
                &self.parents.iter().map(|p| &p.id).collect::<Vec<_>>(),
            )
            .field("defined_iri_count", &self.defined_iris.len())
            .finish()
    }
}

impl Layer {
    /// Construct a Layer from already-stored content. Used by storage
    /// backends when reconstructing a chain — caller passes the metadata
    /// (handle), parent pointer (single-parent path; for multi-parent
    /// merge layers, see `from_handle_multi`), the set of IRIs this
    /// layer defines, and the storage bundle for lazy reads.
    ///
    /// The handle carries both the position hash (`handle.id`) and the
    /// content hash (`handle.content_hash`); both are written into the
    /// reconstructed `Layer` unchanged.
    pub fn from_handle(
        handle: LayerHandle,
        parent: Option<Arc<Layer>>,
        defined_iris: BTreeSet<Iri>,
        storage: LayerStorage,
    ) -> Self {
        Self {
            id: handle.id,
            content_hash: handle.content_hash,
            supporting_layer: handle.supporting_layer,
            name: handle.name,
            parents: parent.into_iter().collect(),
            defined_iris,
            storage,
        }
    }

    /// Construct a Layer with `N` topological parents (Phase 14e
    /// trivial-merge case). The parents must be supplied in the
    /// canonical order they appear in `LayerHandle.parents` — that
    /// order is part of the layer's identity (see
    /// `LayerBuilder::compute_position_hash`).
    pub fn from_handle_multi(
        handle: LayerHandle,
        parents: Vec<Arc<Layer>>,
        defined_iris: BTreeSet<Iri>,
        storage: LayerStorage,
    ) -> Self {
        Self {
            id: handle.id,
            content_hash: handle.content_hash,
            supporting_layer: handle.supporting_layer,
            name: handle.name,
            parents,
            defined_iris,
            storage,
        }
    }

    /// Returns the position-addressed identifier of this layer.
    pub fn id(&self) -> &LayerId {
        &self.id
    }

    /// Returns the content-only hash of this layer (independent of position).
    /// See [`ContentHash`] for the position-vs-content distinction.
    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }

    /// Returns the cached supporting layer per D33 §4.3 — the youngest
    /// ancestor this layer explicitly depends on.
    ///
    /// `None` means one of:
    /// - This is the root layer (no ancestors).
    /// - The layer has no external references (pure top-level
    ///   definitions only).
    /// - The layer was committed by a pre-PR-0 kernel and the
    ///   supporting layer hasn't been back-filled yet (PR 0 step 4
    ///   ships the lazy back-fill path).
    ///
    /// To compute the supporting layer for a not-yet-committed
    /// resource set, use [`compute_supporting_layer`] directly.
    pub fn supporting_layer(&self) -> Option<&LayerId> {
        self.supporting_layer.as_ref()
    }

    /// Returns the human-readable name of this layer.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the chain-walk parent layer (`parents.first()`), if any.
    /// For multi-parent merge layers (Phase 14e), this is the first
    /// merged head — the merge layer's content is self-sufficient for
    /// post-LCA IRIs, so resolve recursion via the first parent
    /// reaches everything older. For pre-merge layers (single-parent),
    /// this is the only parent.
    pub fn parent(&self) -> Option<&Arc<Layer>> {
        self.parents.first()
    }

    /// Returns all topological parents. Empty for the root layer; one
    /// entry for single-parent layers; multiple entries for trivial-merge
    /// layers (Phase 14e). Multi-aware callers (GC reachability,
    /// `db log --all`, merge-history inspection) should use this; the
    /// chain-walk path uses `parent()`.
    pub fn parents(&self) -> &[Arc<Layer>] {
        &self.parents
    }

    /// Returns true if this is the root layer (no parents).
    pub fn is_root(&self) -> bool {
        self.parents.is_empty()
    }

    /// Returns the set of IRIs defined directly in this layer (not parents).
    pub fn defined_iris(&self) -> &BTreeSet<Iri> {
        &self.defined_iris
    }

    /// Returns the bundled `LayerStorage` (cache + backend + bloom cache).
    /// Use this when constructing a child layer or `ExecutionContext` that
    /// should share the same handles.
    pub fn storage(&self) -> &LayerStorage {
        &self.storage
    }

    /// Returns the shared resource cache this layer was built/loaded with.
    pub fn cache(&self) -> &Arc<dyn ResourceCache> {
        &self.storage.cache
    }

    /// Returns the shared backend this layer was built/loaded with.
    pub fn backend(&self) -> &Arc<dyn crate::storage::ResourceBackend> {
        &self.storage.backend
    }

    /// Returns the shared bloom cache this layer was built/loaded with.
    pub fn bloom_cache(&self) -> &Arc<dyn BloomCache> {
        &self.storage.bloom_cache
    }

    /// Look up a resource defined in this layer only (does not walk parents).
    /// Cache → backend fallback. Returns `None` if `iri` is not defined here.
    pub fn get_resource(&self, iri: &Iri) -> Option<Arc<Resource>> {
        if !self.defined_iris.contains(iri) {
            return None;
        }
        let key = ResourceKey::new(self.id.clone(), iri.clone());
        if let Some(r) = self.storage.cache.get(&key) {
            return Some(r);
        }
        let resource = self.storage.backend.load_resource(&self.id, iri)?;
        let arc = Arc::new(resource);
        // Default tier: Active. Without a per-head shadowing index we
        // can't tell whether this layer is top-of-stack for `iri` from
        // here; treating fresh fetches as Active matches the typical
        // case (resolve walks head→root and stops at first hit) and
        // accepts the cost of lazy-demoting outdated entries on later
        // reads (14c-ii).
        self.storage
            .cache
            .put(key, Arc::clone(&arc), CacheTier::Active);
        Some(arc)
    }

    /// Resolve a resource by IRI, walking the parent chain head→root.
    ///
    /// Phase 14b: at each layer, consults the cached per-layer shadowing
    /// bloom (D23 §5.2). If `bloom.might_contain(iri)` returns false, the
    /// layer is skipped without consulting the cache, backend, or
    /// `defined_iris` set — this is the optimization that lets resolve
    /// stay fast as chain depth grows. On a positive (or absent) bloom,
    /// falls through to `get_resource`. Returns the first match
    /// (topmost layer wins).
    ///
    /// Iterative rather than recursive so the bloom-cache `Result` doesn't
    /// have to thread through a recursive call chain. If the bloom cache
    /// returns an error or no entry, treats the layer as "maybe present"
    /// (defensive — better one extra probe than skipping a defining
    /// layer).
    pub fn resolve(&self, iri: &Iri) -> Option<Arc<Resource>> {
        let mut current: Option<&Layer> = Some(self);
        while let Some(layer) = current {
            let maybe_present = match layer.storage.bloom_cache.get_or_load(&layer.id) {
                Ok(Some(bloom)) => bloom.might_contain(iri),
                _ => true,
            };
            if maybe_present {
                if let Some(r) = layer.get_resource(iri) {
                    return Some(r);
                }
            }
            current = layer.parents.first().map(|p| p.as_ref());
        }
        None
    }

    /// Collect all resources at this IRI across the entire chain (top to
    /// bottom). Top layer's value comes first.
    pub fn resolve_all(&self, iri: &Iri) -> Vec<Arc<Resource>> {
        let mut results = Vec::new();
        if let Some(r) = self.get_resource(iri) {
            results.push(r);
        }
        if let Some(parent) = self.parents.first() {
            results.extend(parent.resolve_all(iri));
        }
        results
    }

    /// Iterate over resources defined directly in this layer.
    /// Yields owned `(Iri, Arc<Resource>)` pairs in IRI order.
    pub fn iter_resources(&self) -> impl Iterator<Item = (Iri, Arc<Resource>)> + '_ {
        self.defined_iris
            .iter()
            .filter_map(move |iri| self.get_resource(iri).map(|r| (iri.clone(), r)))
    }

    /// Iterate over the merged view across the entire chain (top layer wins
    /// for duplicate IRIs). Materialises the merged set eagerly for
    /// determinism; callers who need lazy iteration over very large chains
    /// should call `iter_resources` per layer manually.
    pub fn iter_all_resources(&self) -> impl Iterator<Item = (Iri, Arc<Resource>)> {
        let mut seen = BTreeSet::<Iri>::new();
        let mut buf: BTreeMap<Iri, Arc<Resource>> = BTreeMap::new();
        let mut current: Option<&Layer> = Some(self);
        while let Some(layer) = current {
            for iri in &layer.defined_iris {
                if seen.insert(iri.clone()) {
                    if let Some(res) = layer.get_resource(iri) {
                        buf.insert(iri.clone(), res);
                    }
                }
            }
            current = layer.parents.first().map(|p| p.as_ref());
        }
        buf.into_iter()
    }
}

/// Error that can occur when building a layer.
#[derive(Debug, Clone)]
pub enum LayerError {
    /// Cannot add a core namespace resource to a non-root layer.
    CoreNamespaceViolation { iri: Iri },
    /// Resource has no @id (only top-level resources can be added to layers).
    MissingId,
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayerError::CoreNamespaceViolation { iri } => {
                write!(
                    f,
                    "cannot add core namespace resource '{iri}' to non-root layer"
                )
            }
            LayerError::MissingId => write!(f, "resource must have an @id to be added to a layer"),
        }
    }
}

impl std::error::Error for LayerError {}

/// Builder for constructing an immutable `Layer`.
///
/// Accumulates resources, then `build()` computes the content-and-parent-
/// addressed `LayerId` and produces an immutable `Layer`. Phase 14e:
/// supports N parents for trivial-merge layers via
/// `LayerBuilder::with_parents`.
pub struct LayerBuilder {
    name: String,
    resources: BTreeMap<Iri, Resource>,
    parents: Vec<Arc<Layer>>,
}

impl LayerBuilder {
    /// Create a new builder.
    ///
    /// If `parent` is `None`, this builds a root layer (core ontology).
    /// For multi-parent merge layers, use `with_parents` instead.
    pub fn new(name: &str, parent: Option<Arc<Layer>>) -> Self {
        Self {
            name: name.to_string(),
            resources: BTreeMap::new(),
            parents: parent.into_iter().collect(),
        }
    }

    /// Create a builder for a trivial-merge layer with N parents
    /// (Phase 14e). The order of `parents` is part of the layer's
    /// identity — it's hashed into the `LayerId`. Callers should sort
    /// by `LayerId` for canonical ordering when no other order is
    /// natural; `merge_independent_heads` does this automatically.
    pub fn with_parents(name: &str, parents: Vec<Arc<Layer>>) -> Self {
        Self {
            name: name.to_string(),
            resources: BTreeMap::new(),
            parents,
        }
    }

    /// Add a resource to the layer being built.
    ///
    /// Fails if:
    /// - The resource has no `@id`
    /// - The resource's IRI is in the core namespace but this is not a root layer
    pub fn add_resource(&mut self, resource: Resource) -> Result<(), LayerError> {
        let iri = resource.id().ok_or(LayerError::MissingId)?.clone();

        if iri.is_core() && !self.parents.is_empty() {
            return Err(LayerError::CoreNamespaceViolation { iri });
        }

        self.resources.insert(iri, resource);
        Ok(())
    }

    /// Returns true if the builder has a resource with the given IRI.
    pub fn has_resource(&self, iri: &Iri) -> bool {
        self.resources.contains_key(iri)
    }

    /// Get a resource from the builder by IRI.
    pub fn get_resource(&self, iri: &Iri) -> Option<&Resource> {
        self.resources.get(iri)
    }

    /// Returns the resources accumulated so far.
    pub fn resources(&self) -> &BTreeMap<Iri, Resource> {
        &self.resources
    }

    /// Build the immutable `Layer`.
    ///
    /// Computes the `LayerId` as the SHA-256 hash of the canonical CBOR
    /// encoding of resources (in IRI order). Populates `cache` with one
    /// `(LayerId, Iri) → Arc<Resource>` entry per built resource and
    /// populates `bloom_cache` with this layer's shadowing bloom — both so
    /// subsequent lookups via the returned layer hit the in-memory caches
    /// without going to the backend.
    ///
    /// Note: this does NOT write to the backend. Durable persistence is the
    /// caller's responsibility (typically `PersistentBackend::store_layer`,
    /// which writes the same bloom value to `bloom:<id>`). If the cache
    /// evicts a freshly-built resource before commit, the resource is lost;
    /// commit promptly. The bounded cache (14c) will need coordination with
    /// this lifecycle.
    pub fn build(mut self, storage: LayerStorage) -> Layer {
        // Canonicalise property values BEFORE computing the layer id /
        // populating caches: every Eigon-JSON-parsed `Value::String`
        // that names a `data_type: resource` (or `resource_array`)
        // property gets upgraded to `Value::ResourceRef` so the
        // committed shape is uniform regardless of which codec
        // produced the resource. Downstream readers (validator,
        // triple index, query evaluator) can then assume one shape
        // per data_type. Bootstrap is unaffected: the lookup
        // consults `self.resources` first, so properties defined in
        // the layer being built (the core ontology's own property
        // declarations, for instance) are visible to their own
        // canonicalisation pass.
        canonicalise_resource_refs(&mut self.resources, &self.parents);

        let content_hash = compute_content_hash(&self.resources);
        let id = compute_position_hash(&content_hash, &self.parents);
        let defined_iris: BTreeSet<Iri> = self.resources.keys().cloned().collect();
        // Supporting layer (D33 §4.3). Computed against the builder's
        // owned resources before they move into the cache; the result
        // is cached on the `Layer` and (PR 0 step 4) persisted to the
        // supporting-layer index. Multi-parent merge layers use
        // `parents.first()` — matches `Layer::parent()`'s chain-walk
        // contract; trivial-merge's union-of-defined-iris discipline
        // means everything below LCA is reachable via the first parent.
        let supporting_layer =
            compute_supporting_layer(&self.resources, &defined_iris, self.parents.first());
        for (iri, resource) in self.resources {
            let key = ResourceKey::new(id.clone(), iri);
            // Freshly-built layers are top-of-stack by definition.
            storage
                .cache
                .put(key, Arc::new(resource), CacheTier::Active);
        }
        // Pre-populate the bloom cache. Same bloom value the persistent
        // backend will write on commit (deterministic from `defined_iris`),
        // so if the layer is later loaded from disk the cached bloom and
        // on-disk bloom match.
        let bloom = BloomFilter::for_iris(&defined_iris);
        storage.bloom_cache.put(id.clone(), Arc::new(bloom));
        let layer = Layer {
            id,
            content_hash,
            supporting_layer,
            name: self.name,
            parents: self.parents,
            defined_iris,
            storage,
        };
        // Phase 14h: pre-populate the triple index from the layer's
        // indexable triples. `extract_indexable_triples` consults each
        // predicate's `Property.data_type` via `Layer::resolve`, which
        // walks the cache (just populated above) and parents — so this
        // call is self-contained and doesn't touch the backend.
        // Mirrors the bloom precomputation: same entries the persistent
        // backend would write at commit, populated up front so reads
        // against a freshly-built (but not-yet-persisted) layer work
        // identically to reads after restart.
        let owned = crate::layer::index::extract_indexable_triples(&layer);
        if !owned.is_empty() {
            let borrowed: Vec<crate::layer::index::Triple> =
                owned.iter().map(|t| t.as_borrowed()).collect();
            // `extend_layer` is idempotent by `(layer, p, o, s)` — if
            // the persistent backend's `store_layer` later replays the
            // same writes inside its WriteBatch, the second write is a
            // no-op at the index's logical level (RocksDB will overwrite
            // the same key with the same empty value).
            let _ = layer
                .storage
                .triple_index
                .extend_layer(layer.id(), &borrowed);
        }
        layer
    }
}

/// Compute the content-only hash of a resource set.
///
/// Hash domain: `b"content:v1:"` (domain separator) ‖ for each
/// `(iri, resource)` pair in IRI-sorted order, the IRI bytes followed
/// by `canonical_eigon_cbor(resource)`. The result identifies the
/// chain *content* independent of where in the DAG the content was
/// committed: see [`ContentHash`].
///
/// This pairs with [`compute_position_hash`] which folds the content
/// hash together with the sorted parent ids to produce the position
/// hash that addresses the layer's slot in the DAG.
pub fn compute_content_hash(resources: &BTreeMap<Iri, Resource>) -> ContentHash {
    let mut hasher = Sha256::new();
    hasher.update(b"content:v1:");
    // Resource content in IRI-sorted order (BTreeMap iteration).
    for (iri, resource) in resources {
        hasher.update(iri.as_str().as_bytes());
        hasher.update(crate::ontology::eigon_cbor::canonicalize(resource));
    }
    let hash = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hash);
    ContentHash(bytes)
}

/// Compute the position-addressed `PositionHash` (aka `LayerId`) of a
/// layer.
///
/// Hash domain: `b"position:v1:"` (domain separator) ‖ content hash ‖
/// parent count (little-endian u64) ‖ sorted parent position-hash
/// bytes. Sorting the parent ids makes the hash commutative over
/// parent order — a merge that combines (A, B) is the same layer as
/// one that combines (B, A), matching trivial-merge semantics
/// (D33 §4.5).
///
/// **Wire-format note.** This is a deliberate break from Phase 14e's
/// `b"layer:v2:"` hash domain. The new content/position split makes
/// existing persistent DBs unreadable; recovery is `rm -rf <db>` +
/// reload. The break is justified per
/// [D25 §11.0](../../docs/design/d25-chain-consolidation.md) and
/// [D33 §5.1](../../docs/design/d33-partial-order-chains.md).
pub fn compute_position_hash(content_hash: &ContentHash, parents: &[Arc<Layer>]) -> PositionHash {
    let mut hasher = Sha256::new();
    hasher.update(b"position:v1:");
    hasher.update(content_hash.0);
    hasher.update((parents.len() as u64).to_le_bytes());

    // Sort parent ids so merge layers hash commutatively over parent
    // order. Single-parent and root layers are unaffected; the sort is
    // a no-op for those cases.
    let mut sorted_parent_ids: Vec<&[u8; 32]> = parents.iter().map(|p| &p.id().0).collect();
    sorted_parent_ids.sort();
    for pid in sorted_parent_ids {
        hasher.update(pid);
    }

    let hash = hasher.finalize();
    let mut id = [0u8; 32];
    id.copy_from_slice(&hash);
    LayerId(id)
}

// ─── Canonical resource-reference shape ────────────────────────────────
//
// The Eigon-JSON parser is intentionally schema-agnostic: every
// `"prop": "urn:..."` entry parses as `Value::String` because at parse
// time the parser doesn't know the property's `data_type`. The
// Eigon-CBOR codec and substrate-side resource builders use
// `Value::ResourceRef` for the same role. The chain itself should
// carry one canonical shape so downstream readers (validator,
// triple index, query evaluator) don't need a tolerant
// `String|ResourceRef` accept set everywhere.
//
// `canonicalise_resource_refs` runs once per `LayerBuilder::build`,
// before the layer id is computed and the resources are pushed into
// the cache. For every property whose declared `data_type` is
// `resource` or `resource_array`, it rewrites `Value::String` IRIs
// to `Value::ResourceRef`. Property definitions are looked up first
// in the layer being built (so a layer that introduces both a
// property and an instance of it canonicalises consistently — the
// core ontology's `is_a` is the canonical example) and then in the
// parent chain. Properties without a known `data_type` (custom
// extensions, malformed declarations) are left untouched; the
// validator surfaces the malformed shape via its standard rules.

const PROP_DATA_TYPE: &str = "urn:eigenius:core:data_type";

fn canonicalise_resource_refs(resources: &mut BTreeMap<Iri, Resource>, parents: &[Arc<Layer>]) {
    let data_type_iri =
        Iri::parse(PROP_DATA_TYPE).expect("static IRI urn:eigenius:core:data_type must parse");
    let resource_dt = wk::RESOURCE.to_string();
    let resource_array_dt = wk::RESOURCE_ARRAY.to_string();

    // Snapshot the `(prop_iri, data_type_iri)` pairs we'll need.
    // Computing them up front lets us rewrite `resources` without
    // borrowing it twice (once to look up, once to mutate).
    let mut prop_data_types: BTreeMap<Iri, String> = BTreeMap::new();
    let mut all_prop_iris: BTreeSet<Iri> = BTreeSet::new();
    for resource in resources.values() {
        for prop_iri in resource.property_iris() {
            all_prop_iris.insert(prop_iri.clone());
        }
    }
    for prop_iri in &all_prop_iris {
        if let Some(dt) = lookup_property_data_type(prop_iri, &data_type_iri, resources, parents) {
            prop_data_types.insert(prop_iri.clone(), dt);
        }
    }

    for resource in resources.values_mut() {
        let prop_iris: Vec<Iri> = resource.property_iris().cloned().collect();
        for prop_iri in prop_iris {
            let Some(dt) = prop_data_types.get(&prop_iri) else {
                continue;
            };
            if dt == &resource_dt {
                if let Some(value) = resource.get(&prop_iri).cloned() {
                    if let Some(canon) = canonicalise_single_value(value) {
                        resource.set(prop_iri, canon);
                    }
                }
            } else if dt == &resource_array_dt {
                if let Some(Value::Array(items)) = resource.get(&prop_iri).cloned() {
                    let canon = items
                        .into_iter()
                        .map(|v| canonicalise_single_value(v.clone()).unwrap_or(v))
                        .collect();
                    resource.set(prop_iri, Value::Array(canon));
                }
            }
        }
    }
}

/// Upgrade a single `Value::String` IRI to `Value::ResourceRef`.
/// Returns `None` when the value is already canonical (or isn't a
/// string at all) so callers can skip the `set` round-trip.
fn canonicalise_single_value(value: Value) -> Option<Value> {
    match value {
        Value::String(s) => Iri::parse(&s).ok().map(Value::ResourceRef),
        // ResourceRef and Embedded are already canonical for resource
        // references; other shapes are left for the validator to
        // flag as type mismatches.
        _ => None,
    }
}

/// Resolve a property IRI to its declared `data_type`, looking up
/// (in order):
/// 1. The layer being built (so a layer that introduces both a
///    property and a resource using it canonicalises consistently).
/// 2. The parent chain, via `Layer::resolve` (which walks ancestors).
///
/// Returns the data_type IRI as a `String` so callers can match
/// against `wk::RESOURCE` / `wk::RESOURCE_ARRAY` without an extra
/// `Iri::parse` per resource property.
fn lookup_property_data_type(
    prop_iri: &Iri,
    data_type_iri: &Iri,
    working: &BTreeMap<Iri, Resource>,
    parents: &[Arc<Layer>],
) -> Option<String> {
    let extract = |r: &Resource| -> Option<String> {
        match r.get(data_type_iri)? {
            Value::String(s) => Some(s.clone()),
            Value::ResourceRef(i) => Some(i.as_str().to_string()),
            _ => None,
        }
    };
    if let Some(r) = working.get(prop_iri) {
        if let Some(dt) = extract(r) {
            return Some(dt);
        }
    }
    for parent in parents {
        if let Some(r) = parent.resolve(prop_iri) {
            if let Some(dt) = extract(r.as_ref()) {
                return Some(dt);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::resource::Value;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_resource(id: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        for (k, v) in props {
            r.set(iri(k), v);
        }
        r
    }

    /// Test helper: build a fresh in-memory (cache, backend, bloom_cache) triple.
    fn test_storage() -> LayerStorage {
        LayerStorage::in_memory()
    }

    #[test]
    fn build_root_layer() {
        let storage = test_storage();
        let mut builder = LayerBuilder::new("core", None);
        builder
            .add_resource(make_resource("urn:eigenius:core:Class", vec![]))
            .unwrap();
        let layer = builder.build(storage);
        assert!(layer.is_root());
        assert!(layer
            .get_resource(&iri("urn:eigenius:core:Class"))
            .is_some());
    }

    #[test]
    fn core_namespace_protection() {
        let storage = test_storage();
        let root = Arc::new(LayerBuilder::new("core", None).build(storage));
        let mut builder = LayerBuilder::new("domain", Some(root));
        let result = builder.add_resource(make_resource("urn:eigenius:core:Foo", vec![]));
        assert!(matches!(
            result,
            Err(LayerError::CoreNamespaceViolation { .. })
        ));
    }

    #[test]
    fn core_namespace_allowed_on_root() {
        let mut builder = LayerBuilder::new("core", None);
        let result = builder.add_resource(make_resource("urn:eigenius:core:Foo", vec![]));
        assert!(result.is_ok());
    }

    #[test]
    fn embedded_resource_rejected() {
        let mut builder = LayerBuilder::new("test", None);
        let embedded = Resource::new_embedded();
        assert!(matches!(
            builder.add_resource(embedded),
            Err(LayerError::MissingId)
        ));
    }

    #[test]
    fn resolve_walks_parent_chain() {
        let storage = test_storage();

        // Build root with resource A
        let mut root_builder = LayerBuilder::new("root", None);
        root_builder
            .add_resource(make_resource(
                "urn:eigenius:core:A",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("from root".into()),
                )],
            ))
            .unwrap();
        let root = Arc::new(root_builder.build(storage.clone()));

        // Build child with resource B
        let mut child_builder = LayerBuilder::new("child", Some(root));
        child_builder
            .add_resource(make_resource(
                "urn:eigenius:example:B",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("from child".into()),
                )],
            ))
            .unwrap();
        let child = child_builder.build(storage);

        // Child can resolve both A (from root) and B (from self)
        assert!(child.resolve(&iri("urn:eigenius:core:A")).is_some());
        assert!(child.resolve(&iri("urn:eigenius:example:B")).is_some());
        // Non-existent returns None
        assert!(child.resolve(&iri("urn:eigenius:example:C")).is_none());
    }

    #[test]
    fn top_layer_shadows_parent() {
        let storage = test_storage();
        let mut root_builder = LayerBuilder::new("root", None);
        root_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![("urn:eigenius:core:description", Value::String("v1".into()))],
            ))
            .unwrap();
        let root = Arc::new(root_builder.build(storage.clone()));

        let mut child_builder = LayerBuilder::new("child", Some(root));
        child_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![("urn:eigenius:core:description", Value::String("v2".into()))],
            ))
            .unwrap();
        let child = child_builder.build(storage);

        let resolved = child.resolve(&iri("urn:eigenius:example:X")).unwrap();
        let desc = resolved.get(&iri("urn:eigenius:core:description")).unwrap();
        assert_eq!(desc.as_str(), Some("v2")); // child wins
    }

    #[test]
    fn deterministic_layer_id() {
        let build = || {
            let storage = test_storage();
            let mut builder = LayerBuilder::new("test", None);
            builder
                .add_resource(make_resource(
                    "urn:eigenius:core:A",
                    vec![(
                        "urn:eigenius:core:description",
                        Value::String("hello".into()),
                    )],
                ))
                .unwrap();
            builder.build(storage)
        };

        let layer1 = build();
        let layer2 = build();
        assert_eq!(layer1.id(), layer2.id());
    }

    #[test]
    fn different_content_different_id() {
        let storage = test_storage();
        let mut b1 = LayerBuilder::new("test", None);
        b1.add_resource(make_resource("urn:eigenius:core:A", vec![]))
            .unwrap();
        let l1 = b1.build(storage.clone());

        let mut b2 = LayerBuilder::new("test", None);
        b2.add_resource(make_resource("urn:eigenius:core:B", vec![]))
            .unwrap();
        let l2 = b2.build(storage);

        assert_ne!(l1.id(), l2.id());
    }

    #[test]
    fn iter_all_resources_merged() {
        let storage = test_storage();
        let mut root_builder = LayerBuilder::new("root", None);
        root_builder
            .add_resource(make_resource("urn:eigenius:core:A", vec![]))
            .unwrap();
        root_builder
            .add_resource(make_resource("urn:eigenius:core:B", vec![]))
            .unwrap();
        let root = Arc::new(root_builder.build(storage.clone()));

        let mut child_builder = LayerBuilder::new("child", Some(root));
        child_builder
            .add_resource(make_resource("urn:eigenius:example:C", vec![]))
            .unwrap();
        let child = child_builder.build(storage);

        let all: Vec<_> = child.iter_all_resources().collect();
        assert_eq!(all.len(), 3); // A, B from root + C from child
    }

    #[test]
    fn resolve_all_returns_both_layers() {
        let storage = test_storage();
        let mut root_builder = LayerBuilder::new("root", None);
        root_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![("urn:eigenius:core:description", Value::String("v1".into()))],
            ))
            .unwrap();
        let root = Arc::new(root_builder.build(storage.clone()));

        let mut child_builder = LayerBuilder::new("child", Some(root));
        child_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![("urn:eigenius:core:description", Value::String("v2".into()))],
            ))
            .unwrap();
        let child = child_builder.build(storage);

        let all = child.resolve_all(&iri("urn:eigenius:example:X"));
        assert_eq!(all.len(), 2);
    }

    /// Phase 14b: when a layer's bloom reports `false` for an IRI,
    /// `Layer::resolve` must skip that layer and continue walking the
    /// parent chain — even if the layer's `defined_iris` actually
    /// contains the IRI. This is what makes the bloom optimization
    /// load-bearing rather than a redundant pre-check.
    ///
    /// Construction: build root and child both defining the same IRI,
    /// with different values. Then overwrite the child's bloom in the
    /// shared cache with one built from an empty IRI set — i.e., a bloom
    /// that always says "no". Resolve must then return root's value,
    /// proving the child was skipped via its (lying) bloom and the walk
    /// continued past it.
    #[test]
    fn resolve_skips_layer_when_bloom_says_no() {
        let storage = test_storage();
        let mut root_builder = LayerBuilder::new("root", None);
        root_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("from_root".into()),
                )],
            ))
            .unwrap();
        let root = Arc::new(root_builder.build(storage.clone()));

        let mut child_builder = LayerBuilder::new("child", Some(root));
        child_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("from_child".into()),
                )],
            ))
            .unwrap();
        let child = child_builder.build(storage.clone());
        let child_id = child.id().clone();

        // Sanity: with the real bloom, child's value wins (top-of-stack).
        let real = child.resolve(&iri("urn:eigenius:example:X")).unwrap();
        assert_eq!(
            real.get(&iri("urn:eigenius:core:description"))
                .and_then(|v| v.as_str()),
            Some("from_child"),
        );

        // Replace child's bloom with one built from an empty IRI set.
        // `BloomFilter::for_iris` over an empty `BTreeSet` produces a
        // bloom whose `might_contain` always returns false.
        let empty_iris: BTreeSet<Iri> = BTreeSet::new();
        let lying_bloom = BloomFilter::for_iris(&empty_iris);
        storage.bloom_cache.put(child_id, Arc::new(lying_bloom));

        // Resolve again. The lying bloom skips child entirely; the walk
        // continues to root, returning root's value. If `Layer::resolve`
        // were ignoring the bloom and falling back to `defined_iris`,
        // it would still return "from_child" — this assertion catches
        // any regression that breaks the skip optimization.
        let after = child.resolve(&iri("urn:eigenius:example:X")).unwrap();
        assert_eq!(
            after
                .get(&iri("urn:eigenius:core:description"))
                .and_then(|v| v.as_str()),
            Some("from_root"),
        );
    }

    // ─── PR 0 cross-cutting: two-hash identity + supporting layer ──────────
    //
    // Each test below pins one structural invariant of the two-hash split
    // (D25 §11.0 / D33 §5.1) or the supporting-layer computation
    // (D33 §4.3). They share the convention that `position_hash` is
    // `Layer::id` and `content_hash` is `Layer::content_hash` — the
    // type-system layer pinning is in the `LayerId` / `ContentHash`
    // definitions, here we pin the *runtime* invariants.

    /// Same resources + same parents must yield the same content hash
    /// AND the same position hash. Strengthens `deterministic_layer_id`
    /// (which only pins the position hash) and is the minimum
    /// reproducibility property for cell-output cache hits (D33 §6).
    #[test]
    fn deterministic_two_hashes() {
        let build = || {
            let storage = test_storage();
            let mut builder = LayerBuilder::new("test", None);
            builder
                .add_resource(make_resource(
                    "urn:eigenius:core:A",
                    vec![(
                        "urn:eigenius:core:description",
                        Value::String("hello".into()),
                    )],
                ))
                .unwrap();
            builder.build(storage)
        };
        let l1 = build();
        let l2 = build();
        assert_eq!(
            l1.content_hash(),
            l2.content_hash(),
            "same resources must produce identical content hashes"
        );
        assert_eq!(
            l1.id(),
            l2.id(),
            "same resources + same parents must produce identical position hashes"
        );
    }

    /// The structural payoff of the position/content split:
    /// same resources committed against different parents share a
    /// `ContentHash` but get distinct `PositionHash`es. Without the
    /// split there would be only one hash and the chain couldn't tell
    /// a content-equivalent commit from a structurally-distinct one.
    #[test]
    fn content_hash_invariant_under_parent_change() {
        let storage = test_storage();

        // Two distinct root layers — they differ in their own content
        // (and therefore their position hashes), but neither is referenced
        // by the child built below.
        let root_a = {
            let mut b = LayerBuilder::new("root_a", None);
            b.add_resource(make_resource("urn:eigenius:core:RootA", vec![]))
                .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        let root_b = {
            let mut b = LayerBuilder::new("root_b", None);
            b.add_resource(make_resource("urn:eigenius:core:RootB", vec![]))
                .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        assert_ne!(root_a.id(), root_b.id());
        assert_ne!(root_a.content_hash(), root_b.content_hash());

        // Two child layers with byte-identical resource content,
        // attached to different parents.
        let make_child = |parent: Arc<Layer>| -> Layer {
            let mut b = LayerBuilder::new("child", Some(parent));
            b.add_resource(make_resource(
                "urn:eigenius:demo:shared",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("identical".into()),
                )],
            ))
            .unwrap();
            b.build(storage.clone())
        };
        let c_a = make_child(Arc::clone(&root_a));
        let c_b = make_child(Arc::clone(&root_b));

        assert_eq!(
            c_a.content_hash(),
            c_b.content_hash(),
            "identical resource sets must share a content hash regardless of parent"
        );
        assert_ne!(
            c_a.id(),
            c_b.id(),
            "different parents must produce distinct position hashes"
        );
    }

    /// Multi-parent (merge) position hash is commutative over parent
    /// order: merging `[A, B]` produces the same layer as merging
    /// `[B, A]`. Pinned by `compute_position_hash` sorting parent ids
    /// before hashing (D33 §5.1).
    #[test]
    fn position_hash_commutes_over_parent_order() {
        let storage = test_storage();

        let parent_a = {
            let mut b = LayerBuilder::new("a", None);
            b.add_resource(make_resource("urn:eigenius:core:A", vec![]))
                .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        let parent_b = {
            let mut b = LayerBuilder::new("b", None);
            b.add_resource(make_resource("urn:eigenius:core:B", vec![]))
                .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        assert_ne!(parent_a.id(), parent_b.id());

        // Two merge layers with identical content but reversed parent
        // ordering at construction time.
        let merge_ab = {
            let b = LayerBuilder::with_parents(
                "merge",
                vec![Arc::clone(&parent_a), Arc::clone(&parent_b)],
            );
            b.build(storage.clone())
        };
        let merge_ba = {
            let b = LayerBuilder::with_parents(
                "merge",
                vec![Arc::clone(&parent_b), Arc::clone(&parent_a)],
            );
            b.build(storage.clone())
        };
        assert_eq!(
            merge_ab.content_hash(),
            merge_ba.content_hash(),
            "merge layers with no resources have identical content hashes"
        );
        assert_eq!(
            merge_ab.id(),
            merge_ba.id(),
            "position hash must commute over parent order for trivial-merge semantics"
        );
    }

    /// Layers committed against different parents have different
    /// position hashes even when content is identical — confirms the
    /// position hash is genuinely parent-dependent.
    #[test]
    fn position_hash_distinguishes_distinct_parents() {
        let storage = test_storage();

        let p1 = {
            let mut b = LayerBuilder::new("p1", None);
            b.add_resource(make_resource("urn:eigenius:core:P1", vec![]))
                .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        let p2 = {
            let mut b = LayerBuilder::new("p2", None);
            b.add_resource(make_resource("urn:eigenius:core:P2", vec![]))
                .unwrap();
            Arc::new(b.build(storage.clone()))
        };

        // Same trivial child content against each parent.
        let c1 = {
            let mut b = LayerBuilder::new("child", Some(Arc::clone(&p1)));
            b.add_resource(make_resource("urn:eigenius:demo:X", vec![]))
                .unwrap();
            b.build(storage.clone())
        };
        let c2 = {
            let mut b = LayerBuilder::new("child", Some(Arc::clone(&p2)));
            b.add_resource(make_resource("urn:eigenius:demo:X", vec![]))
                .unwrap();
            b.build(storage.clone())
        };

        assert_eq!(c1.content_hash(), c2.content_hash());
        assert_ne!(c1.id(), c2.id());
    }

    /// Layer name is metadata-only: two layers with the same resources
    /// + parents but different names must share both hashes. Pins that
    /// the `name` field is *not* in the content hash (cell-output
    /// cache must hit across cosmetic renames).
    #[test]
    fn name_is_not_in_content_or_position_hash() {
        let storage = test_storage();
        let make = |name: &str| {
            let mut b = LayerBuilder::new(name, None);
            b.add_resource(make_resource(
                "urn:eigenius:core:A",
                vec![("urn:eigenius:core:description", Value::String("x".into()))],
            ))
            .unwrap();
            b.build(storage.clone())
        };
        let l_alpha = make("alpha");
        let l_beta = make("beta");
        assert_eq!(l_alpha.content_hash(), l_beta.content_hash());
        assert_eq!(l_alpha.id(), l_beta.id());
        assert_ne!(l_alpha.name(), l_beta.name());
    }

    /// Supporting-layer computation is deterministic: rebuilding the
    /// same layer against the same parent chain produces the same
    /// supporting-layer answer. This matches the back-fill-free
    /// migration story (no need to coordinate concurrent recomputes —
    /// they always agree).
    #[test]
    fn supporting_layer_is_deterministic() {
        let storage = test_storage();

        let root = {
            let mut b = LayerBuilder::new("root", None);
            b.add_resource(make_resource("urn:eigenius:core:ClassA", vec![]))
                .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        let make_child = || {
            let mut b = LayerBuilder::new("child", Some(Arc::clone(&root)));
            let mut r = Resource::new(iri("urn:eigenius:demo:X"));
            r.set(
                iri("urn:eigenius:core:is_a"),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:core:ClassA"))]),
            );
            b.add_resource(r).unwrap();
            b.build(storage.clone())
        };
        let c1 = make_child();
        let c2 = make_child();
        assert_eq!(c1.id(), c2.id());
        assert_eq!(c1.supporting_layer(), c2.supporting_layer());
        assert_eq!(c1.supporting_layer(), Some(root.id()));
    }

    /// Position hash distinguishes a single-parent layer from a
    /// no-parent layer with the same resources. Pins that the
    /// parent count is folded into the position hash; a root layer
    /// can never collide with a non-root layer. Uses a non-core
    /// namespace for the shared resource so the core-namespace
    /// restriction (only-on-root) doesn't reject the descendant build.
    #[test]
    fn parent_count_distinguishes_root_from_descendant() {
        let storage = test_storage();

        let parent = {
            let mut b = LayerBuilder::new("p", None);
            b.add_resource(make_resource("urn:eigenius:core:P", vec![]))
                .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        // Two layers with the same resources: one as a root, one as a
        // child of `parent`. Content hash is the same; position hash
        // is different (different parent count + parent set).
        let mut r_root = LayerBuilder::new("rooted", None);
        r_root
            .add_resource(make_resource("urn:eigenius:demo:X", vec![]))
            .unwrap();
        let l_root = r_root.build(storage.clone());

        let mut r_child = LayerBuilder::new("rooted", Some(Arc::clone(&parent)));
        r_child
            .add_resource(make_resource("urn:eigenius:demo:X", vec![]))
            .unwrap();
        let l_child = r_child.build(storage.clone());

        assert_eq!(
            l_root.content_hash(),
            l_child.content_hash(),
            "same resources → same content hash"
        );
        assert_ne!(
            l_root.id(),
            l_child.id(),
            "root vs. descendant must produce distinct position hashes"
        );
    }
}
