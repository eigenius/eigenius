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

//! In-memory `PersistentBackend` — the reference implementation.
//!
//! Stores every backend surface (topology, blooms, branches, tags,
//! resources, meta, redirects, anchored-commit cache, content-hash
//! index, triple index, traces) in `BTreeMap`s. Used by kernel tests
//! to exercise the chain machinery without spinning up RocksDB, and
//! by the storage-backend cross-validation harness as the *reference*
//! that the RocksDB-backed `RocksStore` is checked against. Behavior
//! is exact for the trait contract; CBOR-encoding correctness is out
//! of scope (no encoding happens in-memory).
//!
//! **Trade-offs vs. `RocksStore`.** No persistence (lost on process
//! exit). No fsync. No compaction overhead. Single-process only.
//! Production deployments use `RocksStore`; development, tests, and
//! reference-implementation comparisons use this.

use crate::layer::{
    BloomFilter, ContentHash, Layer, LayerHandle, LayerId, LayerTopology, MemoryTextIndex,
    MemoryTripleIndex, MemoryValueIndex, MemoryVectorIndex, RedirectEntry, TextIndex, TripleIndex,
    ValueIndex, VectorIndex,
};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::program::trace::{InMemoryTraceStore, TraceStore};
use crate::storage::{BatchOp, ChainInfo, PersistentBackend, ResourceBackend, StorageError};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

/// In-memory `PersistentBackend`. See module docs.
pub struct MemoryPersistentBackend {
    inner: RwLock<MemoryState>,
    traces: InMemoryTraceStore,
    triple_index: Arc<MemoryTripleIndex>,
    /// D43 §2.3 text index (M2.3). In-memory backend uses the
    /// `MemoryTextIndex` impl from `kernel/src/layer/text_index.rs`.
    text_index: Arc<MemoryTextIndex>,
    /// D43 §2.4 vector index (M2.3). In-memory backend uses the
    /// `MemoryVectorIndex` impl from `kernel/src/layer/vector_index.rs`.
    vector_index: Arc<MemoryVectorIndex>,
    /// D65 exact value index. In-memory backend uses the `MemoryValueIndex`
    /// impl from `kernel/src/layer/value_index.rs`.
    value_index: Arc<MemoryValueIndex>,
}

struct MemoryState {
    /// `(LayerId, Iri) → Resource` — flat resource store.
    resources: BTreeMap<(LayerId, Iri), Resource>,
    /// `LayerId → LayerHandle` — topology entries.
    topology: BTreeMap<LayerId, LayerHandle>,
    /// `LayerId → parent_id` — single-parent (canonical) chain edges
    /// for `load_chain_from`. The full multi-parent record lives on
    /// `LayerHandle.parents` in `topology`.
    chain: BTreeMap<LayerId, Option<LayerId>>,
    /// Generic key/value metadata (D21 task storage substrate).
    meta: BTreeMap<String, Vec<u8>>,
    /// `LayerId → BloomFilter` — D23 §5.2 per-layer shadowing blooms.
    /// `store_layer` builds these from the layer's `defined_iris` and
    /// inserts here; `load_bloom` reads back.
    blooms: BTreeMap<LayerId, BloomFilter>,
    /// Branch refs (D23 §5.5 / Phase 14d). Phase 14g made these the
    /// only head-pointer surface — the legacy single-`head` field is
    /// gone.
    branches: BTreeMap<String, LayerId>,
    /// Tag refs (D34 §G.2 / §8). Immutable named pointers; once a
    /// `(name, layer_id)` lands here, `name` cannot be retargeted —
    /// only `delete_tag` removes it. Tags are GC roots, so the
    /// reachability walk gathers them alongside branch heads.
    tags: BTreeMap<String, LayerId>,
    /// `ContentHash → set of position hashes` — the content-hash dedup
    /// index introduced by D25 §11.0 / D33 §6. Many position hashes can
    /// share a content hash (same notebook cell committed against
    /// different parent chains); the index makes
    /// `lookup_by_content_hash` O(log n).
    content_index: BTreeMap<ContentHash, BTreeSet<LayerId>>,
    /// Installed resolve redirects (D25 §12.8 / Phase 17f). Keyed by
    /// the redirect *source* layer id. One entry per consolidation
    /// where `to` was below the branch head.
    redirects: BTreeMap<LayerId, RedirectEntry>,
    /// Anchored-commit cache (D33 §6 / Phase 20c). Keyed by
    /// `(content_hash, supporting_content_hash)` → cached layer id.
    /// Memoizes `commit(content, supporting_layer) → LayerId`, so
    /// any deterministic content generator anchored to a supporting
    /// layer (notebook cells, institution ontology reload, mirror
    /// regeneration) can reuse the existing layer without
    /// re-committing.
    anchored_commits: BTreeMap<(ContentHash, ContentHash), LayerId>,
}

impl Default for MemoryPersistentBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryPersistentBackend {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(MemoryState {
                resources: BTreeMap::new(),
                topology: BTreeMap::new(),
                chain: BTreeMap::new(),
                meta: BTreeMap::new(),
                blooms: BTreeMap::new(),
                branches: BTreeMap::new(),
                tags: BTreeMap::new(),
                content_index: BTreeMap::new(),
                redirects: BTreeMap::new(),
                anchored_commits: BTreeMap::new(),
            }),
            traces: InMemoryTraceStore::new(),
            triple_index: Arc::new(MemoryTripleIndex::new()),
            text_index: Arc::new(MemoryTextIndex::new()),
            vector_index: Arc::new(MemoryVectorIndex::new()),
            value_index: Arc::new(MemoryValueIndex::new()),
        }
    }
}

// `now_millis` removed — `LayerHandle.created_at` is now sourced from
// `Layer.created_at()` (stamped at `LayerBuilder::build` time), so the
// backend no longer generates its own timestamp.

impl ResourceBackend for MemoryPersistentBackend {
    fn load_resource(&self, layer_id: &LayerId, iri: &Iri) -> Option<Resource> {
        let state = self.inner.read().expect("MemoryPersistentBackend poisoned");
        state
            .resources
            .get(&(layer_id.clone(), iri.clone()))
            .cloned()
    }

    fn try_load_resource(
        &self,
        layer_id: &LayerId,
        iri: &Iri,
    ) -> Result<Option<Resource>, StorageError> {
        Ok(self.load_resource(layer_id, iri))
    }

    fn list_layer_iris(&self, layer_id: &LayerId) -> Result<BTreeSet<Iri>, StorageError> {
        let state = self.inner.read().expect("MemoryPersistentBackend poisoned");
        Ok(state
            .resources
            .keys()
            .filter(|(lid, _)| lid == layer_id)
            .map(|(_, iri)| iri.clone())
            .collect())
    }
}

impl PersistentBackend for MemoryPersistentBackend {
    fn load_chain_from(&self, head_id: &LayerId) -> Result<Option<ChainInfo>, StorageError> {
        let state = self.inner.read().expect("poisoned");
        if !state.topology.contains_key(head_id) {
            return Ok(None);
        }

        // Walk parents head → root, redirect-aware. When the walk
        // reaches a layer that's a redirect source, switch to walking
        // the target's chain instead of continuing through the
        // (potentially reclaimed) original parent. v1's refuse-chaining
        // policy guarantees a single hop is enough — no cycles.
        let mut chain_ids = vec![head_id.clone()];
        let mut current = head_id.clone();
        loop {
            if let Some(entry) = state.redirects.get(&current) {
                chain_ids.push(entry.target.clone());
                current = entry.target.clone();
                continue;
            }
            match state.chain.get(&current).cloned() {
                Some(Some(parent)) => {
                    chain_ids.push(parent.clone());
                    current = parent;
                }
                _ => break,
            }
        }
        chain_ids.reverse();

        let mut handles = Vec::with_capacity(chain_ids.len());
        let mut defined_iris_per_layer = BTreeMap::new();
        for id in &chain_ids {
            let handle = state
                .topology
                .get(id)
                .cloned()
                .ok_or_else(|| StorageError::NotFound(format!("topo entry for {id}")))?;
            let iris: BTreeSet<Iri> = state
                .resources
                .keys()
                .filter(|(lid, _)| lid == id)
                .map(|(_, iri)| iri.clone())
                .collect();
            handles.push(handle);
            defined_iris_per_layer.insert(id.clone(), iris);
        }

        Ok(Some(ChainInfo {
            head: head_id.clone(),
            handles,
            defined_iris_per_layer,
        }))
    }

    fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError> {
        // D65 index lifecycle: materialise the layer's derived indexes into this
        // backend's indexes at the persist step (mirrors `RocksStore::store_layer`),
        // so index population happens post-validation and seeded/committed layers
        // are indexed durably. Writes through `layer.storage()` = this backend.
        crate::layer::populate_layer_indexes(layer);
        let id = layer.id().clone();
        // 14e: persist all topological parents in the LayerHandle so
        // multi-parent merge layers round-trip correctly. The legacy
        // single-parent `chain` map below stores `parents.first()` as
        // the canonical parent for chain-walk reconstruction —
        // consistent with `Layer::parent()` semantics.
        let all_parents: Vec<LayerId> = layer.parents().iter().map(|p| p.id().clone()).collect();
        let canonical_parent = all_parents.first().cloned();
        // Match the persistent backend's `byte_size` accounting so
        // GC estimate tests against this fixture produce the same
        // numbers a real backend would.
        // One walk, two stamps: `byte_size` and the D66 witness-scan skip hint.
        let mut has_witness_candidates = false;
        let byte_size: u64 = layer
            .iter_resources()
            .map(|(_, r)| {
                has_witness_candidates |= crate::layer::is_witness_candidate(&r);
                crate::ontology::eigon_cbor::serialize_resource(&r).len() as u64
            })
            .sum();
        let handle = LayerHandle {
            id: id.clone(),
            content_hash: layer.content_hash().clone(),
            supporting_layer: layer.supporting_layer().cloned(),
            parents: all_parents,
            name: layer.name().to_string(),
            resource_count: layer.defined_iris().len() as u64,
            has_witness_candidates,
            // Copy the build-time stamp instead of taking `now_millis()`
            // here — keeps the in-memory Layer and persisted handle
            // consistent on `created_at` (single source of truth in
            // `LayerBuilder::build`).
            created_at: layer.created_at(),
            byte_size,
            is_redirect_source: false,
            // 15g step 3: persist tombstones onto the handle so
            // `load_chain_from` → `build_chain` → `Layer::from_handle`
            // round-trips the suppression set. Cheap (a small
            // BTreeSet of IRI strings); zero overhead for layers
            // without any.
            tombstoned_iris: layer.tombstoned_iris().clone(),
        };
        // Build the bloom outside the lock (it's a hash-heavy loop) and
        // insert it together with the rest of the layer's state.
        // Bloom covers `defined ∪ tombstoned` so chain-walkers (e.g.
        // `Layer::resolve`, `is_shadowed`) can use it as the master
        // "consult this layer" gate (D23 §5.2).
        let bloom = BloomFilter::for_layer(layer.defined_iris(), layer.tombstoned_iris());

        let content_hash = layer.content_hash().clone();

        let mut state = self.inner.write().expect("poisoned");
        state.topology.insert(id.clone(), handle);
        state.chain.insert(id.clone(), canonical_parent);
        for (iri, resource) in layer.iter_resources() {
            state
                .resources
                .insert((id.clone(), iri), (*resource).clone());
        }
        state.blooms.insert(id.clone(), bloom);
        state
            .content_index
            .entry(content_hash)
            .or_default()
            .insert(id.clone());
        drop(state);
        // Resources are now in the backend — drain the layer's `pending` stage (D23
        // write path) so its in-memory copy is released; later reads page through the
        // cache/backend. Only when the layer's storage is backed by a persistent backend
        // (which can page the resources back): backend-less `in_memory()` storage keeps
        // the stage as its only read home.
        if layer.storage().persistent_backend.is_some() {
            layer
                .storage()
                .pending
                .write()
                .expect("pending stage poisoned")
                .remove(layer.id());
        }
        Ok(id)
    }

    fn load_topology(&self) -> Result<LayerTopology, StorageError> {
        let state = self.inner.read().expect("poisoned");
        let mut topology = LayerTopology::new();
        for handle in state.topology.values() {
            topology.insert_layer(handle.clone());
        }
        // D25 §12.8.1(d): manufacture synthetic tombstones for every
        // redirect source whose original handle has been reclaimed.
        // The redirects map is cheap to iterate (one entry per
        // consolidation, not per layer).
        let entries: Vec<RedirectEntry> = state.redirects.values().cloned().collect();
        crate::layer::augment_topology_with_redirects(&mut topology, &entries);
        Ok(topology)
    }

    fn load_handle(&self, layer_id: &LayerId) -> Result<Option<LayerHandle>, StorageError> {
        let state = self.inner.read().expect("poisoned");
        // Real handle, if present on disk.
        if let Some(handle) = state.topology.get(layer_id) {
            return Ok(Some(handle.clone()));
        }
        // Synthetic tombstone, if a redirect references this layer
        // (D25 §12.8.1(d)). Matches `load_topology`'s view.
        if let Some(entry) = state.redirects.get(layer_id) {
            return Ok(Some(crate::layer::manufacture_tombstone(entry)));
        }
        Ok(None)
    }

    fn get_meta(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self.inner.read().expect("poisoned").meta.get(key).cloned())
    }

    fn put_meta(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        self.inner
            .write()
            .expect("poisoned")
            .meta
            .insert(key.to_string(), value.to_vec());
        Ok(())
    }

    fn delete_meta(&self, key: &str) -> Result<(), StorageError> {
        self.inner.write().expect("poisoned").meta.remove(key);
        Ok(())
    }

    fn write_batch(&self, ops: &[BatchOp]) -> Result<(), StorageError> {
        // Apply ops sequentially under the write lock — trivially atomic
        // because nothing else observes the store during the batch.
        let mut state = self.inner.write().expect("poisoned");
        for op in ops {
            match op {
                BatchOp::PutMeta { key, value } => {
                    state.meta.insert(key.clone(), value.clone());
                }
                BatchOp::DeleteMeta { key } => {
                    state.meta.remove(key);
                }
            }
        }
        Ok(())
    }

    fn list_meta_prefix(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let state = self.inner.read().expect("poisoned");
        Ok(state
            .meta
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }

    fn as_trace_store(&self) -> &(dyn TraceStore + Send + Sync) {
        &self.traces
    }

    fn triple_index_arc(&self) -> Arc<dyn TripleIndex> {
        Arc::clone(&self.triple_index) as Arc<dyn TripleIndex>
    }

    fn text_index_arc(&self) -> Arc<dyn TextIndex> {
        Arc::clone(&self.text_index) as Arc<dyn TextIndex>
    }

    fn vector_index_arc(&self) -> Arc<dyn VectorIndex> {
        Arc::clone(&self.vector_index) as Arc<dyn VectorIndex>
    }

    fn value_index_arc(&self) -> Arc<dyn ValueIndex> {
        Arc::clone(&self.value_index) as Arc<dyn ValueIndex>
    }

    fn load_bloom(&self, layer: &LayerId) -> Result<Option<BloomFilter>, StorageError> {
        Ok(self
            .inner
            .read()
            .expect("poisoned")
            .blooms
            .get(layer)
            .cloned())
    }

    fn store_bloom(&self, layer: &LayerId, bloom: &BloomFilter) -> Result<(), StorageError> {
        self.inner
            .write()
            .expect("poisoned")
            .blooms
            .insert(layer.clone(), bloom.clone());
        Ok(())
    }

    fn get_branch(&self, name: &str) -> Result<Option<LayerId>, StorageError> {
        Ok(self
            .inner
            .read()
            .expect("poisoned")
            .branches
            .get(name)
            .cloned())
    }

    fn put_branch(&self, name: &str, id: &LayerId) -> Result<(), StorageError> {
        self.inner
            .write()
            .expect("poisoned")
            .branches
            .insert(name.to_string(), id.clone());
        Ok(())
    }

    fn delete_branch(&self, name: &str) -> Result<(), StorageError> {
        self.inner.write().expect("poisoned").branches.remove(name);
        Ok(())
    }

    fn list_branches(&self) -> Result<Vec<(String, LayerId)>, StorageError> {
        Ok(self
            .inner
            .read()
            .expect("poisoned")
            .branches
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn create_tag(&self, name: &str, id: &LayerId) -> Result<bool, StorageError> {
        let mut state = self.inner.write().expect("poisoned");
        if state.tags.contains_key(name) {
            return Ok(false);
        }
        state.tags.insert(name.to_string(), id.clone());
        Ok(true)
    }

    fn get_tag(&self, name: &str) -> Result<Option<LayerId>, StorageError> {
        Ok(self.inner.read().expect("poisoned").tags.get(name).cloned())
    }

    fn delete_tag(&self, name: &str) -> Result<bool, StorageError> {
        let mut state = self.inner.write().expect("poisoned");
        Ok(state.tags.remove(name).is_some())
    }

    fn list_tags(&self) -> Result<Vec<(String, LayerId)>, StorageError> {
        Ok(self
            .inner
            .read()
            .expect("poisoned")
            .tags
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn delete_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        let mut state = self.inner.write().expect("poisoned");
        // Pull the content hash off the topology entry before we remove
        // it so we can clean the content-hash index in the same write.
        let content_hash = state.topology.get(layer).map(|h| h.content_hash.clone());
        state.topology.remove(layer);
        state.chain.remove(layer);
        state.blooms.remove(layer);
        state.resources.retain(|(lid, _), _| lid != layer);
        if let Some(ch) = content_hash {
            if let Some(set) = state.content_index.get_mut(&ch) {
                set.remove(layer);
                if set.is_empty() {
                    state.content_index.remove(&ch);
                }
            }
        }
        // Don't touch branches — branches pointing at deleted layers
        // would be a caller bug and we don't masquerade by silently
        // unsetting them. GC ensures branch-pointed layers stay in
        // the reachable set.
        Ok(())
    }

    fn lookup_by_content_hash(
        &self,
        content_hash: &ContentHash,
    ) -> Result<Vec<LayerId>, StorageError> {
        let state = self.inner.read().expect("poisoned");
        Ok(state
            .content_index
            .get(content_hash)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default())
    }

    fn put_redirect(&self, entry: &RedirectEntry) -> Result<(), StorageError> {
        let mut state = self.inner.write().expect("poisoned");
        state
            .redirects
            .insert(entry.source().clone(), entry.clone());
        Ok(())
    }

    fn lookup_redirect(&self, source: &LayerId) -> Result<Option<RedirectEntry>, StorageError> {
        let state = self.inner.read().expect("poisoned");
        Ok(state.redirects.get(source).cloned())
    }

    fn delete_redirect(&self, source: &LayerId) -> Result<(), StorageError> {
        let mut state = self.inner.write().expect("poisoned");
        state.redirects.remove(source);
        Ok(())
    }

    fn list_redirects(&self) -> Result<Vec<RedirectEntry>, StorageError> {
        let state = self.inner.read().expect("poisoned");
        Ok(state.redirects.values().cloned().collect())
    }

    fn lookup_anchored_commit(
        &self,
        content_hash: &ContentHash,
        supporting_content_hash: &ContentHash,
    ) -> Result<Option<LayerId>, StorageError> {
        let state = self.inner.read().expect("poisoned");
        Ok(state
            .anchored_commits
            .get(&(content_hash.clone(), supporting_content_hash.clone()))
            .cloned())
    }

    fn put_anchored_commit(
        &self,
        content_hash: &ContentHash,
        supporting_content_hash: &ContentHash,
        layer_id: &LayerId,
    ) -> Result<(), StorageError> {
        let mut state = self.inner.write().expect("poisoned");
        state.anchored_commits.insert(
            (content_hash.clone(), supporting_content_hash.clone()),
            layer_id.clone(),
        );
        Ok(())
    }

    fn delete_anchored_commit(
        &self,
        content_hash: &ContentHash,
        supporting_content_hash: &ContentHash,
    ) -> Result<(), StorageError> {
        let mut state = self.inner.write().expect("poisoned");
        state
            .anchored_commits
            .remove(&(content_hash.clone(), supporting_content_hash.clone()));
        Ok(())
    }

    fn list_anchored_commits(
        &self,
    ) -> Result<Vec<crate::storage::AnchoredCommitEntry>, StorageError> {
        let state = self.inner.read().expect("poisoned");
        Ok(state
            .anchored_commits
            .iter()
            .map(
                |((content, supporting), id)| crate::storage::AnchoredCommitEntry {
                    content_hash: content.clone(),
                    supporting_content_hash: supporting.clone(),
                    layer_id: id.clone(),
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::Value;
    use std::sync::Arc;

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

    /// Construct a simple layer with one resource against a fresh
    /// `MemoryPersistentBackend`. Smoke test that round-trip works.
    #[test]
    fn store_layer_round_trip() {
        let backend = MemoryPersistentBackend::new();

        let storage = crate::layer::LayerStorage::in_memory();

        let mut builder = LayerBuilder::new("test", None);
        builder
            .add_resource(make_resource(
                "urn:eigenius:core:x",
                vec![("urn:eigenius:core:description", Value::String("hi".into()))],
            ))
            .unwrap();
        let layer = builder.build(storage);
        let id = layer.id().clone();

        backend.store_layer(&layer).unwrap();

        let loaded = backend
            .load_resource(&id, &iri("urn:eigenius:core:x"))
            .expect("present");
        assert_eq!(
            loaded
                .get(&iri("urn:eigenius:core:description"))
                .and_then(|v| v.as_str()),
            Some("hi")
        );

        let topology = backend.load_topology().unwrap();
        assert_eq!(topology.layer_count(), 1);
    }

    /// Two layers with the same single resource but different parents
    /// share a `ContentHash` (the resource set is identical) but get
    /// distinct `PositionHash`es (parents differ). `lookup_by_content_hash`
    /// must return both positions; deleting one cleans only its entry.
    #[test]
    fn content_hash_index_dedup_and_cleanup() {
        let backend = MemoryPersistentBackend::new();
        let storage = crate::layer::LayerStorage::in_memory();

        // Two distinct root layers (different content) so each presents
        // a different parent to the child layers below.
        let root_a = {
            let mut b = LayerBuilder::new("root_a", None);
            b.add_resource(make_resource(
                "urn:eigenius:core:root_a_marker",
                vec![("urn:eigenius:core:description", Value::String("a".into()))],
            ))
            .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        let root_b = {
            let mut b = LayerBuilder::new("root_b", None);
            b.add_resource(make_resource(
                "urn:eigenius:core:root_b_marker",
                vec![("urn:eigenius:core:description", Value::String("b".into()))],
            ))
            .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        backend.store_layer(&root_a).unwrap();
        backend.store_layer(&root_b).unwrap();

        // Two child layers carrying byte-identical resources but rooted
        // at different parents. Same content hash; different position.
        let make_child = |parent: Arc<crate::layer::Layer>| -> crate::layer::Layer {
            let mut b = LayerBuilder::new("child", Some(parent));
            b.add_resource(make_resource(
                "urn:eigenius:demo:shared",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("shared".into()),
                )],
            ))
            .unwrap();
            b.build(storage.clone())
        };
        let child_a = make_child(Arc::clone(&root_a));
        let child_b = make_child(Arc::clone(&root_b));

        assert_eq!(
            child_a.content_hash(),
            child_b.content_hash(),
            "identical resource sets must share a content hash"
        );
        assert_ne!(
            child_a.id(),
            child_b.id(),
            "different parents must yield distinct position hashes"
        );

        backend.store_layer(&child_a).unwrap();
        backend.store_layer(&child_b).unwrap();

        let mut hits = backend
            .lookup_by_content_hash(child_a.content_hash())
            .unwrap();
        hits.sort();
        let mut expected = vec![child_a.id().clone(), child_b.id().clone()];
        expected.sort();
        assert_eq!(hits, expected);

        // Delete one position; the other remains.
        backend.delete_layer(child_a.id()).unwrap();
        let remaining = backend
            .lookup_by_content_hash(child_a.content_hash())
            .unwrap();
        assert_eq!(remaining, vec![child_b.id().clone()]);

        // Delete the second; index is empty (not just emptied — the
        // memory backend prunes the now-empty bucket).
        backend.delete_layer(child_b.id()).unwrap();
        let empty = backend
            .lookup_by_content_hash(child_a.content_hash())
            .unwrap();
        assert!(empty.is_empty());
    }

    /// Phase 17f-A: redirects round-trip through the storage layer and
    /// `load_topology` manufactures a synthetic tombstone for a redirect
    /// whose source has been reclaimed from the topology.
    #[test]
    fn redirect_round_trip_and_synthetic_tombstone() {
        let backend = MemoryPersistentBackend::new();

        // Build a root + child layer; the child will become the
        // "source" of a redirect (the to-be-consolidated layer).
        let storage = crate::layer::LayerStorage::in_memory();
        let mut rb = LayerBuilder::new("root", None);
        rb.add_resource(make_resource("urn:eigenius:core:R", vec![]))
            .unwrap();
        let root = Arc::new(rb.build(storage.clone()));
        backend.store_layer(&root).unwrap();

        let mut sb = LayerBuilder::new("source", Some(Arc::clone(&root)));
        sb.add_resource(make_resource("urn:eigenius:demo:original", vec![]))
            .unwrap();
        let source = Arc::new(sb.build(storage.clone()));
        backend.store_layer(&source).unwrap();

        // Hypothetical L_c — we don't actually build the chain, just
        // need a target id for the redirect to point at.
        let target_id = crate::layer::LayerId([0xab; 32]);

        // Pre-condition: no redirects, no tombstones.
        assert!(backend.list_redirects().unwrap().is_empty());
        let topo_before = backend.load_topology().unwrap();
        assert!(
            !topo_before
                .get_layer(source.id())
                .unwrap()
                .is_redirect_source
        );

        // Install the redirect. The entry carries the source's full
        // LayerHandle so `load_topology` can manufacture a tombstone
        // after `delete_layer` reclaims the original.
        let source_handle = topo_before.get_layer(source.id()).unwrap().clone();
        let redirect = crate::layer::RedirectEntry {
            target: target_id.clone(),
            source_handle,
            preserve_history: false,
        };
        backend.put_redirect(&redirect).unwrap();

        // Round-trip: lookup returns the entry; list returns it once.
        let looked_up = backend
            .lookup_redirect(source.id())
            .unwrap()
            .expect("redirect present");
        assert_eq!(looked_up.target, target_id);
        assert_eq!(looked_up.source(), source.id());
        assert_eq!(backend.list_redirects().unwrap().len(), 1);

        // Reclaim `source` from the topology. `load_topology` should
        // now manufacture a tombstone with `is_redirect_source = true`
        // and the original handle's metadata.
        backend.delete_layer(source.id()).unwrap();
        let topo_after = backend.load_topology().unwrap();
        let tombstone = topo_after
            .get_layer(source.id())
            .expect("tombstone present");
        assert!(tombstone.is_redirect_source);
        assert_eq!(tombstone.id, *source.id());
        assert_eq!(tombstone.name, "source"); // preserved from original handle

        // Delete the redirect. The tombstone goes away with it.
        backend.delete_redirect(source.id()).unwrap();
        let topo_final = backend.load_topology().unwrap();
        assert!(topo_final.get_layer(source.id()).is_none());
    }

    /// Phase 20c: anchored-commit cache round-trip through the four
    /// trait methods. Hit on byte-equal key; miss on either-key
    /// change; list reports all entries; delete removes a single entry.
    #[test]
    fn anchored_commit_cache_round_trip() {
        let backend = MemoryPersistentBackend::new();

        let content_a = ContentHash([1u8; 32]);
        let content_b = ContentHash([2u8; 32]);
        let support_x = ContentHash([3u8; 32]);
        let support_y = ContentHash([4u8; 32]);
        let layer_one = LayerId([0x10; 32]);
        let layer_two = LayerId([0x20; 32]);

        // Pre-condition: empty.
        assert!(backend
            .lookup_anchored_commit(&content_a, &support_x)
            .unwrap()
            .is_none());
        assert!(backend.list_anchored_commits().unwrap().is_empty());

        // Insert one entry; round-trip via lookup.
        backend
            .put_anchored_commit(&content_a, &support_x, &layer_one)
            .unwrap();
        let hit = backend
            .lookup_anchored_commit(&content_a, &support_x)
            .unwrap()
            .expect("cache hit on byte-equal key");
        assert_eq!(hit, layer_one);

        // Different content → miss. Different supporting → miss.
        assert!(backend
            .lookup_anchored_commit(&content_b, &support_x)
            .unwrap()
            .is_none());
        assert!(backend
            .lookup_anchored_commit(&content_a, &support_y)
            .unwrap()
            .is_none());

        // Insert a second entry; list reports both.
        backend
            .put_anchored_commit(&content_b, &support_y, &layer_two)
            .unwrap();
        let mut entries = backend.list_anchored_commits().unwrap();
        entries.sort_by_key(|e| e.content_hash.0);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content_hash, content_a);
        assert_eq!(entries[0].supporting_content_hash, support_x);
        assert_eq!(entries[0].layer_id, layer_one);
        assert_eq!(entries[1].content_hash, content_b);
        assert_eq!(entries[1].supporting_content_hash, support_y);
        assert_eq!(entries[1].layer_id, layer_two);

        // Overwriting an existing entry replaces the layer.
        let layer_one_v2 = LayerId([0x11; 32]);
        backend
            .put_anchored_commit(&content_a, &support_x, &layer_one_v2)
            .unwrap();
        let hit2 = backend
            .lookup_anchored_commit(&content_a, &support_x)
            .unwrap()
            .expect("cache hit after overwrite");
        assert_eq!(hit2, layer_one_v2);
        assert_eq!(backend.list_anchored_commits().unwrap().len(), 2);

        // Delete one entry; the other remains.
        backend
            .delete_anchored_commit(&content_a, &support_x)
            .unwrap();
        assert!(backend
            .lookup_anchored_commit(&content_a, &support_x)
            .unwrap()
            .is_none());
        assert_eq!(backend.list_anchored_commits().unwrap().len(), 1);
    }

    #[test]
    fn meta_kv_round_trip() {
        let backend = MemoryPersistentBackend::new();
        assert!(backend.get_meta("absent").unwrap().is_none());

        backend.put_meta("k", b"v").unwrap();
        assert_eq!(
            backend.get_meta("k").unwrap().as_deref(),
            Some(b"v".as_ref())
        );

        backend.delete_meta("k").unwrap();
        assert!(backend.get_meta("k").unwrap().is_none());

        backend.put_meta("session:a", b"1").unwrap();
        backend.put_meta("session:b", b"2").unwrap();
        backend.put_meta("other:c", b"3").unwrap();
        let mut keys = backend.list_meta_prefix("session:").unwrap();
        keys.sort();
        assert_eq!(keys, vec!["session:a", "session:b"]);
    }

    #[test]
    fn write_batch_atomic() {
        let backend = MemoryPersistentBackend::new();
        backend.put_meta("to_delete", b"old").unwrap();

        backend
            .write_batch(&[
                BatchOp::PutMeta {
                    key: "k1".into(),
                    value: b"v1".to_vec(),
                },
                BatchOp::DeleteMeta {
                    key: "to_delete".into(),
                },
            ])
            .unwrap();

        assert_eq!(
            backend.get_meta("k1").unwrap().as_deref(),
            Some(b"v1".as_ref())
        );
        assert!(backend.get_meta("to_delete").unwrap().is_none());
    }

    #[test]
    fn branch_refs_round_trip() {
        let backend = MemoryPersistentBackend::new();
        assert!(backend.get_branch("main").unwrap().is_none());
        assert!(backend.list_branches().unwrap().is_empty());

        let id_a = LayerId([1u8; 32]);
        let id_b = LayerId([2u8; 32]);
        backend.put_branch("main", &id_a).unwrap();
        backend.put_branch("auto-divergent", &id_b).unwrap();

        assert_eq!(backend.get_branch("main").unwrap(), Some(id_a.clone()));
        assert_eq!(
            backend.get_branch("auto-divergent").unwrap(),
            Some(id_b.clone())
        );

        // list_branches returns sorted by name.
        let listed = backend.list_branches().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, "auto-divergent");
        assert_eq!(listed[1].0, "main");

        // Overwrite + delete.
        backend.put_branch("main", &id_b).unwrap();
        assert_eq!(backend.get_branch("main").unwrap(), Some(id_b.clone()));
        backend.delete_branch("main").unwrap();
        assert!(backend.get_branch("main").unwrap().is_none());

        // Delete on absent key is a no-op.
        backend.delete_branch("nonexistent").unwrap();
    }

    #[test]
    fn tag_refs_round_trip() {
        // Verifies the structural invariants of the tag primitive:
        // immutable (re-create returns false; existing target stays),
        // idempotent delete (false on absent), and stable sort order.
        let backend = MemoryPersistentBackend::new();
        assert!(backend.get_tag("release-v1").unwrap().is_none());
        assert!(backend.list_tags().unwrap().is_empty());

        let id_a = LayerId([1u8; 32]);
        let id_b = LayerId([2u8; 32]);
        assert!(backend.create_tag("release-v1", &id_a).unwrap());
        assert!(backend.create_tag("baseline", &id_b).unwrap());

        assert_eq!(backend.get_tag("release-v1").unwrap(), Some(id_a.clone()));
        assert_eq!(backend.get_tag("baseline").unwrap(), Some(id_b.clone()));

        // Tag immutability: re-creating an existing name returns
        // false and the original target survives.
        assert!(!backend.create_tag("release-v1", &id_b).unwrap());
        assert_eq!(
            backend.get_tag("release-v1").unwrap(),
            Some(id_a.clone()),
            "create_tag must NOT retarget an existing name"
        );

        // list_tags returns sorted by name.
        let listed = backend.list_tags().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, "baseline");
        assert_eq!(listed[1].0, "release-v1");

        // Delete reports whether the tag existed.
        assert!(backend.delete_tag("release-v1").unwrap());
        assert!(backend.get_tag("release-v1").unwrap().is_none());
        assert!(
            !backend.delete_tag("release-v1").unwrap(),
            "delete on absent name is idempotent — returns false, not an error"
        );
    }

    #[test]
    fn load_chain_from_walks_parents() {
        let backend = MemoryPersistentBackend::new();
        let storage = crate::layer::LayerStorage::in_memory();

        let mut root_b = LayerBuilder::new("root", None);
        root_b
            .add_resource(make_resource("urn:eigenius:core:r", vec![]))
            .unwrap();
        let root = Arc::new(root_b.build(storage.clone()));

        let mut child_b = LayerBuilder::new("child", Some(Arc::clone(&root)));
        child_b
            .add_resource(make_resource("urn:eigenius:example:c", vec![]))
            .unwrap();
        let child = Arc::new(child_b.build(storage));
        let child_id = child.id().clone();

        backend.store_layer(&root).unwrap();
        backend.store_layer(&child).unwrap();

        let info = backend
            .load_chain_from(&child_id)
            .unwrap()
            .expect("chain present");
        let names: Vec<&str> = info.handles.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["root", "child"]);
    }

    /// D43 M2.3 — `PersistentBackend::text_index_arc` returns an
    /// `Arc<dyn TextIndex>` that is shareable and Arc-cloned across
    /// calls (so `LayerStorage` instances built from the same
    /// backend see the same physical index).
    #[test]
    fn text_index_arc_returns_shared_handle() {
        use crate::layer::TextDoc;

        let backend = MemoryPersistentBackend::new();
        let ti_a = backend.text_index_arc();
        let ti_b = backend.text_index_arc();

        // Write through one handle, read through the other —
        // shared state confirms both Arcs point at the same
        // underlying MemoryTextIndex.
        let index_iri = Iri::parse("urn:eigenius:test:ti").unwrap();
        let layer = LayerId([7u8; 32]);
        let subject = Iri::parse("urn:eigenius:test:s").unwrap();
        let tokens = vec!["alpha".to_string(), "beta".to_string()];
        let docs = [TextDoc {
            subject: &subject,
            tokens: &tokens,
        }];
        ti_a.extend_layer(&index_iri, &layer, "en-stem-v1", &docs)
            .unwrap();

        let hits: Vec<_> = ti_b
            .scan_term(&index_iri, "alpha")
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits.len(), 1, "second Arc handle sees writes via first");
        assert_eq!(hits[0].df, 1);
    }

    /// D43 M2.3 — same shape for the vector index Arc.
    #[test]
    fn vector_index_arc_returns_shared_handle() {
        use crate::layer::VectorDoc;

        let backend = MemoryPersistentBackend::new();
        let vi_a = backend.vector_index_arc();
        let vi_b = backend.vector_index_arc();

        let index_iri = Iri::parse("urn:eigenius:test:vi").unwrap();
        let layer = LayerId([11u8; 32]);
        let model_iri = Iri::parse("urn:eigenius:test:embedder").unwrap();
        let subject = Iri::parse("urn:eigenius:test:s").unwrap();
        let vec_data = [1.0f32, 0.5, 0.0];
        let docs = [VectorDoc {
            subject: &subject,
            vector: &vec_data,
        }];
        vi_a.extend_layer(&index_iri, &layer, &model_iri, 3, "cosine", &docs, None)
            .unwrap();

        let seg = vi_b
            .get_segment(&index_iri, &layer)
            .unwrap()
            .expect("second Arc handle sees writes via first");
        assert_eq!(seg.count(), 1);
        assert_eq!(seg.vector_at(0), &vec_data);
    }
}
