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

//! In-memory `PersistentBackend` for kernel-side tests.
//!
//! Kernel internal tests can't dev-dep on `eigenius-storage-rocksdb`
//! because that would create a Cargo dev-dep cycle (storage-rocksdb
//! depends on the kernel lib), producing two compilations of the kernel
//! and breaking trait-object upcasts. Real-backend coverage already
//! lives in `storage/rocksdb/tests/` and `storage/rocksdb/src/lib.rs`'s
//! `cbor_coverage_tests` module — kernel tests just need a faithful
//! in-memory `PersistentBackend` to exercise kernel logic.
//!
//! `MemoryPersistentBackend` is that fixture. It implements every method
//! on `PersistentBackend` (and the supertrait `ResourceBackend`) over
//! `BTreeMap`s. Behavior matches `RocksStore` to the extent the trait
//! contract specifies; CBOR-correctness behavior is out of scope here
//! (no encoding happens in-memory).

#![cfg(test)]

use crate::layer::{BloomFilter, Layer, LayerHandle, LayerId, LayerTopology};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::program::trace::{InMemoryTraceStore, TraceStore};
use crate::storage::{BatchOp, ChainInfo, PersistentBackend, ResourceBackend, StorageError};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// In-memory `PersistentBackend` for kernel-side tests.
///
/// Stores everything in `BTreeMap`s. Construction is `MemoryPersistentBackend::new()`;
/// no configuration. Tests build one and pass it as the backend Arc through
/// the same paths production passes `Arc<RocksStore>`.
pub(crate) struct MemoryPersistentBackend {
    inner: RwLock<MemoryState>,
    traces: InMemoryTraceStore,
}

struct MemoryState {
    /// `(LayerId, Iri) → Resource` — flat resource store.
    resources: BTreeMap<(LayerId, Iri), Resource>,
    /// `LayerId → LayerHandle` — topology entries.
    topology: BTreeMap<LayerId, LayerHandle>,
    /// `LayerId → parent_id` — single-parent chain edges. Multi-parent
    /// merges (Phase 14e+) will need to use `LayerHandle::parents` instead.
    chain: BTreeMap<LayerId, Option<LayerId>>,
    /// Persisted head pointer.
    head: Option<LayerId>,
    /// Generic key/value metadata (D21 task storage substrate).
    meta: BTreeMap<String, Vec<u8>>,
    /// `LayerId → BloomFilter` — D23 §5.2 per-layer shadowing blooms.
    /// `store_layer` builds these from the layer's `defined_iris` and
    /// inserts here; `load_bloom` reads back.
    blooms: BTreeMap<LayerId, BloomFilter>,
}

impl MemoryPersistentBackend {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(MemoryState {
                resources: BTreeMap::new(),
                topology: BTreeMap::new(),
                chain: BTreeMap::new(),
                head: None,
                meta: BTreeMap::new(),
                blooms: BTreeMap::new(),
            }),
            traces: InMemoryTraceStore::new(),
        }
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

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
    fn get_head(&self) -> Result<Option<LayerId>, StorageError> {
        Ok(self.inner.read().expect("poisoned").head.clone())
    }

    fn set_head(&self, id: &LayerId) -> Result<(), StorageError> {
        self.inner.write().expect("poisoned").head = Some(id.clone());
        Ok(())
    }

    fn load_chain(&self) -> Result<Option<ChainInfo>, StorageError> {
        let head = self.inner.read().expect("poisoned").head.clone();
        match head {
            Some(h) => self.load_chain_from(&h),
            None => Ok(None),
        }
    }

    fn load_chain_from(&self, head_id: &LayerId) -> Result<Option<ChainInfo>, StorageError> {
        let state = self.inner.read().expect("poisoned");
        if !state.topology.contains_key(head_id) {
            return Ok(None);
        }

        // Walk parents head → root, then reverse.
        let mut chain_ids = vec![head_id.clone()];
        let mut current = head_id.clone();
        while let Some(Some(parent)) = state.chain.get(&current).cloned() {
            chain_ids.push(parent.clone());
            current = parent;
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
        let id = layer.id().clone();
        let parent_id = layer.parent().map(|p| p.id().clone());
        let handle = LayerHandle {
            id: id.clone(),
            parents: parent_id.clone().into_iter().collect(),
            name: layer.name().to_string(),
            resource_count: layer.defined_iris().len() as u64,
            created_at: now_millis(),
        };
        // Build the bloom outside the lock (it's a hash-heavy loop) and
        // insert it together with the rest of the layer's state.
        let bloom = BloomFilter::for_iris(layer.defined_iris());

        let mut state = self.inner.write().expect("poisoned");
        state.topology.insert(id.clone(), handle);
        state.chain.insert(id.clone(), parent_id);
        for (iri, resource) in layer.iter_resources() {
            state
                .resources
                .insert((id.clone(), iri), (*resource).clone());
        }
        state.blooms.insert(id.clone(), bloom);
        Ok(id)
    }

    fn load_topology(&self) -> Result<LayerTopology, StorageError> {
        let state = self.inner.read().expect("poisoned");
        let mut topology = LayerTopology::new();
        for handle in state.topology.values() {
            topology.insert_layer(handle.clone());
        }
        Ok(topology)
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
}
