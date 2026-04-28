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
mod handle;
mod storage;

pub use bloom::{BloomFilter, DEFAULT_FPR};
pub use cache::{
    BloomCache, BoundedResourceCache, CacheStats, CacheTier, MemoryBloomCache,
    MemoryResourceBackend, MemoryResourceCache, ResourceCache, ResourceKey,
};
pub use handle::{ChainIter, LayerHandle, LayerTopology};
pub use storage::LayerStorage;

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
use crate::ontology::resource::Resource;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

/// Content-addressed layer identifier (SHA-256 hash).
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

/// An immutable layer in the chain.
///
/// **Phase 14a-iii / 14b structure**: holds metadata (id, name, parent pointer,
/// the set of IRIs defined in this specific layer) and a `LayerStorage`
/// bundle (resource cache + backend + per-layer shadowing bloom cache). It
/// does NOT hold full resource content; resource bodies are fetched lazily
/// via the cache, falling through to the backend on miss.
///
/// The `parent: Option<Arc<Layer>>` chain is preserved as a transitional
/// shape — chain walking still happens via `resolve` iterating through
/// parent pointers. The bloom filter (Phase 14b) lets `resolve` skip layers
/// that cannot define `iri` without consulting the cache or backend.
///
/// Cloning a Layer is cheap: it clones a `LayerStorage` (a few atomic
/// increments on the bundled Arcs) and shallow-copies the metadata. The
/// parent Arc is shared.
#[derive(Clone)]
pub struct Layer {
    id: LayerId,
    name: String,
    parent: Option<Arc<Layer>>,
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
            .field("parent_id", &self.parent.as_ref().map(|p| &p.id))
            .field("defined_iri_count", &self.defined_iris.len())
            .finish()
    }
}

impl Layer {
    /// Construct a Layer from already-stored content. Used by storage
    /// backends when reconstructing a chain — caller passes the metadata
    /// (handle), parent pointer, the set of IRIs this layer defines (typically
    /// gathered via a `layer:<id>:res:` prefix scan), and the storage bundle
    /// for lazy reads. No resource content is loaded eagerly; the bloom is
    /// loaded on first `resolve` through this layer (or never, if the layer
    /// is skipped by an ancestor's bloom).
    pub fn from_handle(
        handle: LayerHandle,
        parent: Option<Arc<Layer>>,
        defined_iris: BTreeSet<Iri>,
        storage: LayerStorage,
    ) -> Self {
        Self {
            id: handle.id,
            name: handle.name,
            parent,
            defined_iris,
            storage,
        }
    }

    /// Returns the content-addressed identifier of this layer.
    pub fn id(&self) -> &LayerId {
        &self.id
    }

    /// Returns the human-readable name of this layer.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the parent layer, if any.
    pub fn parent(&self) -> Option<&Arc<Layer>> {
        self.parent.as_ref()
    }

    /// Returns true if this is the root layer (no parent).
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
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
            current = layer.parent.as_deref();
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
        if let Some(parent) = &self.parent {
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
            current = layer.parent.as_deref();
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
/// Accumulates resources, then `build()` computes the content-addressed
/// `LayerId` and produces an immutable `Layer`.
pub struct LayerBuilder {
    name: String,
    resources: BTreeMap<Iri, Resource>,
    parent: Option<Arc<Layer>>,
}

impl LayerBuilder {
    /// Create a new builder.
    ///
    /// If `parent` is `None`, this builds a root layer (core ontology).
    pub fn new(name: &str, parent: Option<Arc<Layer>>) -> Self {
        Self {
            name: name.to_string(),
            resources: BTreeMap::new(),
            parent,
        }
    }

    /// Add a resource to the layer being built.
    ///
    /// Fails if:
    /// - The resource has no `@id`
    /// - The resource's IRI is in the core namespace but this is not a root layer
    pub fn add_resource(&mut self, resource: Resource) -> Result<(), LayerError> {
        let iri = resource.id().ok_or(LayerError::MissingId)?.clone();

        if iri.is_core() && self.parent.is_some() {
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
    pub fn build(self, storage: LayerStorage) -> Layer {
        let id = self.compute_layer_id();
        let defined_iris: BTreeSet<Iri> = self.resources.keys().cloned().collect();
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
        Layer {
            id,
            name: self.name,
            parent: self.parent,
            defined_iris,
            storage,
        }
    }

    fn compute_layer_id(&self) -> LayerId {
        let mut hasher = Sha256::new();

        // Hash each resource's CBOR deterministic encoding in IRI order
        // (BTreeMap guarantees sorted iteration)
        for (iri, resource) in &self.resources {
            hasher.update(iri.as_str().as_bytes());
            hasher.update(crate::ontology::eigon_cbor::canonicalize(resource));
        }

        let hash = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&hash);
        LayerId(id)
    }
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
}
