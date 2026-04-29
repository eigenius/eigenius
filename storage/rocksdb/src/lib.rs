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

//! RocksDB storage backend for Eigenius.
//!
//! Implements `LayerStore` and `ResourceStore` using RocksDB as the
//! persistent ordered key-value store. Key encoding follows D4.
//!
//! Key scheme:
//!   layer:<layer_id_hex>:res:<iri>    → Resource (CBOR)
//!   chain:<layer_id_hex>              → Parent layer ID hex (or empty)
//!   head                              → Current head layer ID hex
//!   topo:<layer_id_hex>               → LayerHandle (CBOR, Phase 14a-ii)
//!   trace:<key_hex>                   → ComponentTrace (CBOR)
//!   meta:<key>                        → Generic metadata KV

use async_trait::async_trait;
#[cfg(test)]
use eigenius_kernel::layer::LayerBuilder;
use eigenius_kernel::layer::{BloomFilter, Layer, LayerHandle, LayerId, LayerTopology};
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::storage::{LayerStore, ResourceBackend, ResourceStore, StorageError};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const TOPO_PREFIX: &str = "topo:";
const BLOOM_PREFIX: &str = "bloom:";
const BRANCH_PREFIX: &str = "branch:";

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// RocksDB-backed storage.
pub struct RocksStore {
    db: rocksdb::DB,
}

impl RocksStore {
    /// Open or create a RocksDB database at the given path.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

        let db = rocksdb::DB::open(&opts, path)
            .map_err(|e| StorageError::Internal(format!("failed to open RocksDB: {e}")))?;

        Ok(Self { db })
    }

    /// Trigger manual compaction on the entire database.
    pub fn compact(&self) {
        self.db.compact_range::<&[u8], &[u8]>(None, None);
    }

    /// Store a layer's parent chain pointer.
    fn set_chain(
        &self,
        layer_id: &LayerId,
        parent_id: Option<&LayerId>,
    ) -> Result<(), StorageError> {
        let key = format!("chain:{}", hex::encode(layer_id.0));
        let value = match parent_id {
            Some(pid) => hex::encode(pid.0),
            None => String::new(),
        };
        self.db
            .put(key.as_bytes(), value.as_bytes())
            .map_err(|e| StorageError::Internal(format!("failed to set chain: {e}")))
    }

    /// Get a layer's parent ID from the chain.
    fn get_chain(&self, layer_id: &LayerId) -> Result<Option<LayerId>, StorageError> {
        let key = format!("chain:{}", hex::encode(layer_id.0));
        match self.db.get(key.as_bytes()) {
            Ok(Some(bytes)) => {
                let hex_str = String::from_utf8(bytes)
                    .map_err(|e| StorageError::Internal(format!("invalid chain value: {e}")))?;
                if hex_str.is_empty() {
                    Ok(None) // Root layer
                } else {
                    Ok(Some(hex_to_layer_id(&hex_str)?))
                }
            }
            Ok(None) => Err(StorageError::NotFound(format!(
                "chain entry for layer {}",
                hex::encode(layer_id.0)
            ))),
            Err(e) => Err(StorageError::Internal(format!("failed to get chain: {e}"))),
        }
    }

    /// Build a `ChainInfo` describing the chain from root → `head`. Phase 14a-iii:
    /// returns metadata only, no resource bodies; the caller turns this into
    /// an `Arc<Layer>` chain via `crate::layer::build_chain`.
    pub fn build_chain_info(
        &self,
        head: &LayerId,
    ) -> Result<Option<eigenius_kernel::storage::ChainInfo>, StorageError> {
        // Walk parent pointers head → root, then reverse for build order.
        let mut chain_ids = vec![head.clone()];
        let mut current = head.clone();
        while let Some(parent_id) = self.get_chain(&current)? {
            chain_ids.push(parent_id.clone());
            current = parent_id;
        }
        chain_ids.reverse();

        let mut handles = Vec::with_capacity(chain_ids.len());
        let mut defined_iris_per_layer = std::collections::BTreeMap::new();
        for id in &chain_ids {
            let topo_key = format!("{TOPO_PREFIX}{}", hex::encode(id.0));
            let bytes = self
                .db
                .get(topo_key.as_bytes())
                .map_err(|e| StorageError::Internal(format!("get topo entry: {e}")))?
                .ok_or_else(|| {
                    StorageError::NotFound(format!("topo entry for layer {}", hex::encode(id.0)))
                })?;
            let handle: LayerHandle = ciborium::from_reader(bytes.as_slice())
                .map_err(|e| StorageError::Internal(format!("decode LayerHandle: {e}")))?;
            let iris = ResourceBackend::list_layer_iris(self, id)?;
            handles.push(handle);
            defined_iris_per_layer.insert(id.clone(), iris);
        }

        Ok(Some(eigenius_kernel::storage::ChainInfo {
            head: head.clone(),
            handles,
            defined_iris_per_layer,
        }))
    }

    /// Load layer metadata (name + first parent) for a known layer.
    ///
    /// Reads the canonical CBOR `topo:<id>` entry. There is no legacy
    /// fallback — pre-Phase-14 DBs are not supported; recovery is to drop
    /// the DB and re-load from source files.
    fn load_layer_meta(
        &self,
        layer_id: &LayerId,
    ) -> Result<(String, Option<LayerId>), StorageError> {
        let topo_key = format!("{TOPO_PREFIX}{}", hex::encode(layer_id.0));
        let bytes = self
            .db
            .get(topo_key.as_bytes())
            .map_err(|e| StorageError::Internal(format!("failed to load topo entry: {e}")))?
            .ok_or_else(|| StorageError::NotFound(format!("layer {}", hex::encode(layer_id.0))))?;
        let handle: LayerHandle = ciborium::from_reader(bytes.as_slice())
            .map_err(|e| StorageError::Internal(format!("decode LayerHandle: {e}")))?;
        Ok((handle.name, handle.parents.into_iter().next()))
    }

    /// Write a `topo:<id>` entry containing the LayerHandle. Phase 14a-ii.
    fn put_topology_entry(&self, handle: &LayerHandle) -> Result<(), StorageError> {
        let key = format!("{TOPO_PREFIX}{}", hex::encode(handle.id.0));
        let mut bytes = Vec::new();
        ciborium::into_writer(handle, &mut bytes)
            .map_err(|e| StorageError::Internal(format!("encode LayerHandle: {e}")))?;
        self.db
            .put(key.as_bytes(), bytes)
            .map_err(|e| StorageError::Internal(format!("put topology entry: {e}")))
    }

    /// Read all `topo:<id>` entries into an in-memory `LayerTopology`. Returns
    /// an empty topology if no entries exist (caller decides whether to
    /// migrate from the legacy `chain:` layout).
    fn read_topology_entries(&self) -> Result<LayerTopology, StorageError> {
        let mut topology = LayerTopology::new();
        let iter = self.db.prefix_iterator(TOPO_PREFIX.as_bytes());
        for item in iter {
            let (key, value) =
                item.map_err(|e| StorageError::Internal(format!("topology iter: {e}")))?;
            let key_str = std::str::from_utf8(&key)
                .map_err(|e| StorageError::Internal(format!("non-utf8 topo key: {e}")))?;
            // Prefix iterator may overshoot — trim.
            if !key_str.starts_with(TOPO_PREFIX) {
                break;
            }
            let handle: LayerHandle = ciborium::from_reader(value.as_ref())
                .map_err(|e| StorageError::Internal(format!("decode LayerHandle: {e}")))?;
            topology.insert_layer(handle);
        }
        Ok(topology)
    }
}

// --- Trace Store ---

use eigenius_kernel::program::trace::{ComponentMetrics, ComponentTrace, TraceStore};

impl TraceStore for RocksStore {
    fn get_component_trace(&self, key: &[u8; 32]) -> Option<ComponentTrace> {
        let db_key = format!("trace:{}", hex::encode(key));
        match self.db.get(db_key.as_bytes()) {
            Ok(Some(bytes)) => deserialize_component_trace(&bytes).ok(),
            _ => None,
        }
    }

    fn put_component_trace(&self, key: [u8; 32], trace: ComponentTrace) {
        let db_key = format!("trace:{}", hex::encode(key));
        if let Ok(bytes) = serialize_component_trace(&trace) {
            let _ = self.db.put(db_key.as_bytes(), bytes);
        }
    }
}

/// CBOR-serializable wrapper for ComponentTrace storage. The `output` is
/// pre-encoded as canonical CBOR bytes using `eigon_cbor::serialize_resource`
/// (the same encoding used for `layer:<id>:res:<iri>` entries) so we don't
/// need a generic serde impl on `Resource`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredTrace {
    component: String,
    input_hash: [u8; 32],
    argument_hash: Option<[u8; 32]>,
    output_cbor: Vec<u8>,
    metrics: Option<StoredMetrics>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredMetrics {
    provider: String,
    model: String,
    prompt_tokens: i64,
    completion_tokens: i64,
    latency_ms: i64,
}

/// Serialize a ComponentTrace to CBOR bytes for storage.
fn serialize_component_trace(trace: &ComponentTrace) -> Result<Vec<u8>, StorageError> {
    let stored = StoredTrace {
        component: trace.component.clone(),
        input_hash: trace.input_hash,
        argument_hash: trace.argument_hash,
        output_cbor: eigon_cbor::serialize_resource(&trace.output),
        metrics: trace.metrics.as_ref().map(|m| StoredMetrics {
            provider: m.provider.clone(),
            model: m.model.clone(),
            prompt_tokens: m.prompt_tokens,
            completion_tokens: m.completion_tokens,
            latency_ms: m.latency_ms,
        }),
    };
    let mut bytes = Vec::new();
    ciborium::into_writer(&stored, &mut bytes)
        .map_err(|e| StorageError::Internal(format!("serialize trace: {e}")))?;
    Ok(bytes)
}

/// Deserialize a ComponentTrace from CBOR bytes.
fn deserialize_component_trace(bytes: &[u8]) -> Result<ComponentTrace, StorageError> {
    let stored: StoredTrace = ciborium::from_reader(bytes)
        .map_err(|e| StorageError::Internal(format!("deserialize trace: {e}")))?;
    let output = eigon_cbor::parse_resource(&stored.output_cbor)
        .map_err(|e| StorageError::Internal(format!("parse trace output: {e}")))?;
    let metrics = stored.metrics.map(|m| ComponentMetrics {
        provider: m.provider,
        model: m.model,
        prompt_tokens: m.prompt_tokens,
        completion_tokens: m.completion_tokens,
        latency_ms: m.latency_ms,
    });
    Ok(ComponentTrace {
        component: stored.component,
        input_hash: stored.input_hash,
        argument_hash: stored.argument_hash,
        output,
        cached: false, // When loaded from storage, it will be marked cached by the caller
        metrics,
    })
}

fn hex_to_layer_id(hex_str: &str) -> Result<LayerId, StorageError> {
    let bytes =
        hex::decode(hex_str).map_err(|e| StorageError::Internal(format!("invalid hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(StorageError::Internal(format!(
            "layer ID must be 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes);
    Ok(LayerId(id))
}

#[async_trait]
impl LayerStore for RocksStore {
    async fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError> {
        let id = layer.id().clone();
        // 14e: persist all topological parents in the LayerHandle so
        // multi-parent merge layers round-trip correctly. The legacy
        // `chain:<id>` map below stores `parents.first()` as the
        // canonical parent for chain-walk reconstruction — consistent
        // with `Layer::parent()` semantics.
        let all_parents: Vec<LayerId> = layer.parents().iter().map(|p| p.id().clone()).collect();
        let canonical_parent = all_parents.first().cloned();

        let handle = LayerHandle {
            id: id.clone(),
            parents: all_parents,
            name: layer.name().to_string(),
            resource_count: layer.defined_iris().len() as u64,
            created_at: now_millis(),
        };
        self.put_topology_entry(&handle)?;

        // Store each resource as CBOR
        for (iri, resource) in layer.iter_resources() {
            let key = format!("layer:{}:res:{}", hex::encode(id.0), iri.as_str());
            let value = eigon_cbor::serialize_resource(&resource);
            self.db
                .put(key.as_bytes(), value)
                .map_err(|e| StorageError::Internal(format!("failed to store resource: {e}")))?;
        }

        // Store chain pointer (canonical parent only — full multi-parent
        // record is in the topology entry above).
        self.set_chain(&id, canonical_parent.as_ref())?;

        Ok(id)
    }

    async fn load_layer(&self, id: &LayerId) -> Result<Layer, StorageError> {
        // Phase 14a-iii: rebuild as a parent-less Layer with self as both
        // cache (fresh) and backend. Used by tests that exercise the older
        // `LayerStore` API in isolation; production code uses
        // `PersistentBackend::load_chain` + `build_chain` instead.
        let (name, _parent_id) = self.load_layer_meta(id)?;
        let defined_iris = ResourceBackend::list_layer_iris(self, id)?;
        // Construct an in-memory storage bundle and warm both caches from
        // RocksDB so reads via the returned Layer succeed without going
        // back to disk. (Production callers use `PersistentBackend::load_chain`
        // + `build_chain` instead — this path exists for the older async
        // `LayerStore` API tests.)
        let storage = eigenius_kernel::layer::LayerStorage::in_memory();
        let handle = LayerHandle {
            id: id.clone(),
            parents: Vec::new(),
            name,
            resource_count: defined_iris.len() as u64,
            created_at: 0,
        };
        for iri in &defined_iris {
            if let Some(resource) = ResourceBackend::load_resource(self, id, iri) {
                storage.cache.put(
                    eigenius_kernel::layer::ResourceKey::new(id.clone(), iri.clone()),
                    Arc::new(resource),
                    eigenius_kernel::layer::CacheTier::Active,
                );
            }
        }
        if let Ok(Some(bloom)) = eigenius_kernel::storage::PersistentBackend::load_bloom(self, id) {
            storage.bloom_cache.put(id.clone(), Arc::new(bloom));
        }
        Ok(Layer::from_handle(handle, None, defined_iris, storage))
    }

    async fn list_layers(&self) -> Result<Vec<LayerId>, StorageError> {
        let prefix = b"layer:";
        let mut ids = std::collections::BTreeSet::new();

        let iter = self.db.prefix_iterator(prefix);
        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::Internal(format!("iteration error: {e}")))?;

            let key_str = String::from_utf8_lossy(&key);
            if !key_str.starts_with("layer:") {
                break;
            }

            // Extract layer ID from key: "layer:<hex>:..."
            if let Some(rest) = key_str.strip_prefix("layer:") {
                if let Some(hex_part) = rest.split(':').next() {
                    if let Ok(id) = hex_to_layer_id(hex_part) {
                        ids.insert(id);
                    }
                }
            }
        }

        Ok(ids.into_iter().collect())
    }
}

#[async_trait]
impl ResourceStore for RocksStore {
    async fn store_resource(
        &self,
        layer_id: &LayerId,
        resource: &Resource,
    ) -> Result<(), StorageError> {
        let iri = resource
            .id()
            .ok_or_else(|| StorageError::Internal("resource has no @id".to_string()))?;
        let key = format!("layer:{}:res:{}", hex::encode(layer_id.0), iri.as_str());
        let value = eigon_cbor::serialize_resource(resource);
        self.db
            .put(key.as_bytes(), value)
            .map_err(|e| StorageError::Internal(format!("failed to store resource: {e}")))
    }

    async fn load_resource(
        &self,
        layer_id: &LayerId,
        iri: &Iri,
    ) -> Result<Option<Resource>, StorageError> {
        let key = format!("layer:{}:res:{}", hex::encode(layer_id.0), iri.as_str());
        match self.db.get(key.as_bytes()) {
            Ok(Some(bytes)) => {
                let resource = eigon_cbor::parse_resource(&bytes)
                    .map_err(|e| StorageError::Internal(format!("CBOR parse error: {e}")))?;
                Ok(Some(resource))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(StorageError::Internal(format!(
                "failed to load resource: {e}"
            ))),
        }
    }

    async fn list_resources(&self, layer_id: &LayerId) -> Result<Vec<Iri>, StorageError> {
        let prefix = format!("layer:{}:res:", hex::encode(layer_id.0));
        let mut iris = Vec::new();

        let iter = self.db.prefix_iterator(prefix.as_bytes());
        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::Internal(format!("iteration error: {e}")))?;

            let key_str = String::from_utf8_lossy(&key);
            if !key_str.starts_with(&prefix) {
                break;
            }

            if let Some(iri_str) = key_str.strip_prefix(&prefix) {
                if let Ok(iri) = Iri::parse(iri_str) {
                    iris.push(iri);
                }
            }
        }

        Ok(iris)
    }
}

// --- ResourceBackend (Phase 14a-iii: sync single-resource lookup) ---

impl ResourceBackend for RocksStore {
    fn load_resource(&self, layer_id: &LayerId, iri: &Iri) -> Option<Resource> {
        // Panic on storage error: matches the kernel's broken-disk failure
        // model. Use try_load_resource for fallible callers.
        match self.try_load_resource(layer_id, iri) {
            Ok(opt) => opt,
            Err(e) => panic!("RocksStore::load_resource failed: {e}"),
        }
    }

    fn try_load_resource(
        &self,
        layer_id: &LayerId,
        iri: &Iri,
    ) -> Result<Option<Resource>, StorageError> {
        let key = format!("layer:{}:res:{}", hex::encode(layer_id.0), iri.as_str());
        match self.db.get(key.as_bytes()) {
            Ok(Some(bytes)) => {
                let resource = eigon_cbor::parse_resource(&bytes)
                    .map_err(|e| StorageError::Internal(format!("CBOR parse error: {e}")))?;
                Ok(Some(resource))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(StorageError::Internal(format!(
                "failed to load resource: {e}"
            ))),
        }
    }

    fn list_layer_iris(
        &self,
        layer_id: &LayerId,
    ) -> Result<std::collections::BTreeSet<Iri>, StorageError> {
        let prefix = format!("layer:{}:res:", hex::encode(layer_id.0));
        let mut iris = std::collections::BTreeSet::new();
        let iter = self.db.prefix_iterator(prefix.as_bytes());
        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::Internal(format!("list_layer_iris iter: {e}")))?;
            let key_str = String::from_utf8_lossy(&key);
            if !key_str.starts_with(&prefix) {
                break;
            }
            if let Some(iri_str) = key_str.strip_prefix(&prefix) {
                if let Ok(iri) = Iri::parse(iri_str) {
                    iris.insert(iri);
                }
            }
        }
        Ok(iris)
    }
}

// --- PersistentBackend (D13) ---

impl eigenius_kernel::storage::PersistentBackend for RocksStore {
    fn load_chain_from(
        &self,
        head_id: &LayerId,
    ) -> Result<Option<eigenius_kernel::storage::ChainInfo>, StorageError> {
        self.build_chain_info(head_id)
    }

    fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError> {
        // Per D23 §6.3, a layer commit must atomically write the topology
        // entry, the per-layer bloom (Phase 14b), every `layer:<id>:res:`
        // entry, and the chain pointer. We bundle them into one
        // `WriteBatch`; RocksDB guarantees atomicity across the batch so a
        // partial commit is impossible. (The pre-14b code used individual
        // `put` calls and relied on commit ordering — fine in practice but
        // not what the spec promises.)
        let id = layer.id().clone();
        // 14e: persist all topological parents in the LayerHandle so
        // multi-parent merge layers round-trip correctly. The
        // `chain:<id>` key below stores `parents.first()` as the
        // canonical parent for chain-walk reconstruction — consistent
        // with `Layer::parent()` semantics.
        let all_parents: Vec<LayerId> = layer.parents().iter().map(|p| p.id().clone()).collect();
        let canonical_parent = all_parents.first().cloned();

        let handle = LayerHandle {
            id: id.clone(),
            parents: all_parents,
            name: layer.name().to_string(),
            resource_count: layer.defined_iris().len() as u64,
            created_at: now_millis(),
        };
        let bloom = BloomFilter::for_iris(layer.defined_iris());

        // Encode CBOR payloads outside the batch — encoding is CPU work
        // and can fail; no point holding the batch while computing.
        let mut handle_bytes = Vec::new();
        ciborium::into_writer(&handle, &mut handle_bytes)
            .map_err(|e| StorageError::Internal(format!("encode LayerHandle: {e}")))?;
        let mut bloom_bytes = Vec::new();
        ciborium::into_writer(&bloom, &mut bloom_bytes)
            .map_err(|e| StorageError::Internal(format!("encode BloomFilter: {e}")))?;

        let mut batch = rocksdb::WriteBatch::default();

        let topo_key = format!("{TOPO_PREFIX}{}", hex::encode(id.0));
        batch.put(topo_key.as_bytes(), &handle_bytes);

        let bloom_key = format!("{BLOOM_PREFIX}{}", hex::encode(id.0));
        batch.put(bloom_key.as_bytes(), &bloom_bytes);

        for (iri, resource) in layer.iter_resources() {
            let key = format!("layer:{}:res:{}", hex::encode(id.0), iri.as_str());
            let value = eigon_cbor::serialize_resource(&resource);
            batch.put(key.as_bytes(), value);
        }

        let chain_key = format!("chain:{}", hex::encode(id.0));
        let chain_value = match canonical_parent.as_ref() {
            Some(pid) => hex::encode(pid.0),
            None => String::new(),
        };
        batch.put(chain_key.as_bytes(), chain_value.as_bytes());

        self.db
            .write(batch)
            .map_err(|e| StorageError::Internal(format!("store_layer batch: {e}")))?;
        Ok(id)
    }

    fn load_topology(&self) -> Result<LayerTopology, StorageError> {
        self.read_topology_entries()
    }

    fn get_meta(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let db_key = format!("meta:{key}");
        self.db
            .get(db_key.as_bytes())
            .map_err(|e| StorageError::Internal(format!("meta get: {e}")))
    }

    fn put_meta(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        let db_key = format!("meta:{key}");
        self.db
            .put(db_key.as_bytes(), value)
            .map_err(|e| StorageError::Internal(format!("meta put: {e}")))
    }

    fn delete_meta(&self, key: &str) -> Result<(), StorageError> {
        let db_key = format!("meta:{key}");
        self.db
            .delete(db_key.as_bytes())
            .map_err(|e| StorageError::Internal(format!("meta delete: {e}")))
    }

    fn write_batch(&self, ops: &[eigenius_kernel::storage::BatchOp]) -> Result<(), StorageError> {
        use eigenius_kernel::storage::BatchOp;
        let mut batch = rocksdb::WriteBatch::default();
        for op in ops {
            match op {
                BatchOp::PutMeta { key, value } => {
                    let db_key = format!("meta:{key}");
                    batch.put(db_key.as_bytes(), value);
                }
                BatchOp::DeleteMeta { key } => {
                    let db_key = format!("meta:{key}");
                    batch.delete(db_key.as_bytes());
                }
            }
        }
        self.db
            .write(batch)
            .map_err(|e| StorageError::Internal(format!("write_batch: {e}")))
    }

    fn list_meta_prefix(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let db_prefix = format!("meta:{prefix}");
        let mut out = Vec::new();
        let iter = self.db.prefix_iterator(db_prefix.as_bytes());
        for item in iter {
            let (k, _v) =
                item.map_err(|e| StorageError::Internal(format!("list_meta_prefix: {e}")))?;
            let key_str = std::str::from_utf8(&k)
                .map_err(|e| StorageError::Internal(format!("non-utf8 meta key: {e}")))?;
            // Prefix iterator may overshoot — trim.
            if !key_str.starts_with(&db_prefix) {
                break;
            }
            out.push(key_str["meta:".len()..].to_string());
        }
        Ok(out)
    }

    fn as_trace_store(&self) -> &(dyn eigenius_kernel::program::trace::TraceStore + Send + Sync) {
        self
    }

    fn load_bloom(&self, layer: &LayerId) -> Result<Option<BloomFilter>, StorageError> {
        let key = format!("{BLOOM_PREFIX}{}", hex::encode(layer.0));
        match self.db.get(key.as_bytes()) {
            Ok(Some(bytes)) => {
                let bloom: BloomFilter = ciborium::from_reader(bytes.as_slice())
                    .map_err(|e| StorageError::Internal(format!("decode BloomFilter: {e}")))?;
                Ok(Some(bloom))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(StorageError::Internal(format!("load_bloom: {e}"))),
        }
    }

    fn store_bloom(&self, layer: &LayerId, bloom: &BloomFilter) -> Result<(), StorageError> {
        let key = format!("{BLOOM_PREFIX}{}", hex::encode(layer.0));
        let mut bytes = Vec::new();
        ciborium::into_writer(bloom, &mut bytes)
            .map_err(|e| StorageError::Internal(format!("encode BloomFilter: {e}")))?;
        self.db
            .put(key.as_bytes(), bytes)
            .map_err(|e| StorageError::Internal(format!("store_bloom: {e}")))
    }

    fn get_branch(&self, name: &str) -> Result<Option<LayerId>, StorageError> {
        let key = format!("{BRANCH_PREFIX}{name}");
        match self.db.get(key.as_bytes()) {
            Ok(Some(bytes)) => {
                let hex_str = String::from_utf8(bytes).map_err(|e| {
                    StorageError::Internal(format!("invalid branch ref value: {e}"))
                })?;
                Ok(Some(hex_to_layer_id(&hex_str)?))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(StorageError::Internal(format!("get_branch: {e}"))),
        }
    }

    fn put_branch(&self, name: &str, id: &LayerId) -> Result<(), StorageError> {
        let key = format!("{BRANCH_PREFIX}{name}");
        self.db
            .put(key.as_bytes(), hex::encode(id.0))
            .map_err(|e| StorageError::Internal(format!("put_branch: {e}")))
    }

    fn delete_branch(&self, name: &str) -> Result<(), StorageError> {
        let key = format!("{BRANCH_PREFIX}{name}");
        self.db
            .delete(key.as_bytes())
            .map_err(|e| StorageError::Internal(format!("delete_branch: {e}")))
    }

    fn delete_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        let id_hex = hex::encode(layer.0);
        // Per D23 §6.3, layer-shape mutations land via one WriteBatch.
        // Atomic across the topology entry, bloom, chain pointer, and
        // every resource entry — no partial state visible after a
        // crash mid-delete.
        let mut batch = rocksdb::WriteBatch::default();

        let topo_key = format!("{TOPO_PREFIX}{id_hex}");
        batch.delete(topo_key.as_bytes());

        let bloom_key = format!("{BLOOM_PREFIX}{id_hex}");
        batch.delete(bloom_key.as_bytes());

        let chain_key = format!("chain:{id_hex}");
        batch.delete(chain_key.as_bytes());

        // Resource entries: prefix-scan + per-key delete inside the
        // batch. RocksDB's `delete_range` is faster but has subtle
        // interactions with snapshot iterators we don't want to pull
        // in for v1; per-key delete is correct and fast enough for
        // typical layer sizes.
        let res_prefix = format!("layer:{id_hex}:res:");
        let iter = self.db.prefix_iterator(res_prefix.as_bytes());
        for item in iter {
            let (k, _v) =
                item.map_err(|e| StorageError::Internal(format!("delete_layer iter: {e}")))?;
            if !k.starts_with(res_prefix.as_bytes()) {
                break;
            }
            batch.delete(&k);
        }

        self.db
            .write(batch)
            .map_err(|e| StorageError::Internal(format!("delete_layer batch: {e}")))?;
        Ok(())
    }

    fn list_branches(&self) -> Result<Vec<(String, LayerId)>, StorageError> {
        let mut out = Vec::new();
        let iter = self.db.prefix_iterator(BRANCH_PREFIX.as_bytes());
        for item in iter {
            let (k, v) =
                item.map_err(|e| StorageError::Internal(format!("list_branches iter: {e}")))?;
            let key_str = std::str::from_utf8(&k)
                .map_err(|e| StorageError::Internal(format!("non-utf8 branch key: {e}")))?;
            // Prefix iterator may overshoot.
            if !key_str.starts_with(BRANCH_PREFIX) {
                break;
            }
            let name = key_str[BRANCH_PREFIX.len()..].to_string();
            let hex_str = std::str::from_utf8(&v)
                .map_err(|e| StorageError::Internal(format!("non-utf8 branch value: {e}")))?;
            let id = hex_to_layer_id(hex_str)?;
            out.push((name, id));
        }
        // BTreeMap-style sort for deterministic ordering even though
        // prefix-scan already yields sorted keys; defensive against
        // future column-family layout changes.
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::ontology::eigon_json;
    use eigenius_kernel::ontology::resource::Value;
    use tempfile::TempDir;

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

    fn open_temp_store() -> (RocksStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = RocksStore::open(dir.path()).unwrap();
        (store, dir)
    }

    #[tokio::test]
    async fn store_and_load_layer() {
        let (store, _dir) = open_temp_store();

        let mut builder = LayerBuilder::new("test", None);
        builder
            .add_resource(make_resource(
                "urn:eigenius:core:test",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("hello".into()),
                )],
            ))
            .unwrap();
        let layer = builder.build(eigenius_kernel::layer::LayerStorage::in_memory());
        let id = layer.id().clone();

        store.store_layer(&layer).await.unwrap();
        let loaded = store.load_layer(&id).await.unwrap();
        assert_eq!(loaded.name(), "test");
        assert_eq!(loaded.iter_resources().count(), 1);
    }

    #[tokio::test]
    async fn load_nonexistent_layer() {
        let (store, _dir) = open_temp_store();
        let fake_id = LayerId([0u8; 32]);
        assert!(matches!(
            store.load_layer(&fake_id).await,
            Err(StorageError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn store_and_load_resource() {
        let (store, _dir) = open_temp_store();
        let layer_id = LayerId([1u8; 32]);
        let resource = make_resource(
            "urn:eigenius:test:foo",
            vec![("urn:eigenius:core:description", Value::String("bar".into()))],
        );

        store.store_resource(&layer_id, &resource).await.unwrap();
        let loaded = ResourceStore::load_resource(&store, &layer_id, &iri("urn:eigenius:test:foo"))
            .await
            .unwrap();
        assert!(loaded.is_some());
        assert_eq!(
            loaded
                .unwrap()
                .get(&iri("urn:eigenius:core:description"))
                .unwrap()
                .as_str(),
            Some("bar")
        );
    }

    #[tokio::test]
    async fn list_resources() {
        let (store, _dir) = open_temp_store();
        let layer_id = LayerId([2u8; 32]);
        store
            .store_resource(&layer_id, &make_resource("urn:eigenius:test:a", vec![]))
            .await
            .unwrap();
        store
            .store_resource(&layer_id, &make_resource("urn:eigenius:test:b", vec![]))
            .await
            .unwrap();

        let iris = store.list_resources(&layer_id).await.unwrap();
        assert_eq!(iris.len(), 2);
    }

    #[tokio::test]
    async fn list_layers() {
        let (store, _dir) = open_temp_store();

        let mut b1 = LayerBuilder::new("a", None);
        b1.add_resource(make_resource("urn:eigenius:core:x", vec![]))
            .unwrap();
        let l1 = b1.build(eigenius_kernel::layer::LayerStorage::in_memory());

        let mut b2 = LayerBuilder::new("b", None);
        b2.add_resource(make_resource("urn:eigenius:core:y", vec![]))
            .unwrap();
        let l2 = b2.build(eigenius_kernel::layer::LayerStorage::in_memory());

        store.store_layer(&l1).await.unwrap();
        store.store_layer(&l2).await.unwrap();

        let ids = store.list_layers().await.unwrap();
        assert_eq!(ids.len(), 2);
    }

    // Phase 14g: the legacy `head_pointer` test was removed. The
    // pre-Phase-14 single-head pointer (`set_head`/`get_head`) is gone;
    // branches via `put_branch`/`get_branch` are the only head-pointer
    // surface. Branch-ref round-trip is exercised by
    // `cbor_coverage_tests::branch_refs_round_trip` below.

    #[tokio::test]
    async fn persistence_across_reopen() {
        let dir = TempDir::new().unwrap();

        // Write data
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let mut builder = LayerBuilder::new("persisted", None);
            builder
                .add_resource(make_resource(
                    "urn:eigenius:core:persistent",
                    vec![(
                        "urn:eigenius:core:description",
                        Value::String("survives restart".into()),
                    )],
                ))
                .unwrap();
            let layer = builder.build(eigenius_kernel::layer::LayerStorage::in_memory());
            let id = layer.id().clone();
            store.store_layer(&layer).await.unwrap();
            // Phase 14g: track the head via `branch:main` instead of
            // the removed `set_head`.
            eigenius_kernel::storage::PersistentBackend::put_branch(&store, "main", &id).unwrap();
        }

        // Reopen and verify
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let head = eigenius_kernel::storage::PersistentBackend::get_branch(&store, "main")
                .unwrap()
                .expect("branch:main survives reopen");

            let layer = store.load_layer(&head).await.unwrap();
            assert_eq!(layer.name(), "persisted");
            assert!(layer
                .get_resource(&iri("urn:eigenius:core:persistent"))
                .is_some());
        }
    }

    #[tokio::test]
    async fn chain_reconstruction() {
        let (store, _dir) = open_temp_store();

        // Build and store root layer
        let mut root_builder = LayerBuilder::new("core", None);
        root_builder
            .add_resource(make_resource("urn:eigenius:core:Class", vec![]))
            .unwrap();
        let root = Arc::new(root_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
        store.store_layer(&root).await.unwrap();

        // Build and store child layer
        let mut child_builder = LayerBuilder::new("domain", Some(Arc::clone(&root)));
        child_builder
            .add_resource(make_resource(
                "urn:eigenius:example:Dog",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("A dog".into()),
                )],
            ))
            .unwrap();
        let child = child_builder.build(eigenius_kernel::layer::LayerStorage::in_memory());
        let child_id = child.id().clone();
        store.store_layer(&child).await.unwrap();
        // Phase 14g: track head via `branch:main`; load chain via
        // `load_chain_from(branch_head)` rather than the removed
        // no-arg `load_chain()`.
        eigenius_kernel::storage::PersistentBackend::put_branch(&store, "main", &child_id).unwrap();

        let main_head = eigenius_kernel::storage::PersistentBackend::get_branch(&store, "main")
            .unwrap()
            .expect("branch:main present");
        let info = eigenius_kernel::storage::PersistentBackend::load_chain_from(&store, &main_head)
            .unwrap()
            .expect("chain present");
        let storage = eigenius_kernel::layer::LayerStorage::in_memory();
        // Pre-warm the caches from the persistent store so resolve hits succeed.
        for handle in &info.handles {
            if let Some(iris) = info.defined_iris_per_layer.get(&handle.id) {
                for iri_h in iris {
                    if let Some(r) = ResourceBackend::load_resource(&store, &handle.id, iri_h) {
                        storage.cache.put(
                            eigenius_kernel::layer::ResourceKey::new(
                                handle.id.clone(),
                                iri_h.clone(),
                            ),
                            Arc::new(r),
                            eigenius_kernel::layer::CacheTier::Active,
                        );
                    }
                }
            }
            if let Ok(Some(bloom)) =
                eigenius_kernel::storage::PersistentBackend::load_bloom(&store, &handle.id)
            {
                storage.bloom_cache.put(handle.id.clone(), Arc::new(bloom));
            }
        }
        let head = eigenius_kernel::layer::build_chain(info, storage);
        assert!(!head.is_root());
        // Should resolve resources from both layers
        assert!(head.resolve(&iri("urn:eigenius:core:Class")).is_some());
        assert!(head.resolve(&iri("urn:eigenius:example:Dog")).is_some());
    }

    // Replaced by `cbor_coverage_tests::core_ontology_field_level_equality`,
    // which checks every property survives the round-trip rather than just
    // resource count.

    #[tokio::test]
    async fn trace_store_round_trip() {
        let (store, _dir) = open_temp_store();

        let key = [42u8; 32];
        assert!(store.get_component_trace(&key).is_none());

        let trace = ComponentTrace {
            component: "urn:eigenius:program:components:CompleteText".to_string(),
            input_hash: key,
            argument_hash: None,
            output: make_resource(
                "urn:eigenius:test:output",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("LLM output".into()),
                )],
            ),
            cached: false,
            metrics: Some(ComponentMetrics {
                provider: "anthropic".to_string(),
                model: "claude-sonnet".to_string(),
                prompt_tokens: 100,
                completion_tokens: 50,
                latency_ms: 500,
            }),
        };

        store.put_component_trace(key, trace);
        let loaded = store.get_component_trace(&key).unwrap();

        assert_eq!(
            loaded.component,
            "urn:eigenius:program:components:CompleteText"
        );
        assert_eq!(loaded.input_hash, key);
        assert!(loaded.metrics.is_some());
        let m = loaded.metrics.unwrap();
        assert_eq!(m.provider, "anthropic");
        assert_eq!(m.prompt_tokens, 100);
        assert_eq!(m.completion_tokens, 50);
        assert_eq!(m.latency_ms, 500);
        assert_eq!(
            loaded
                .output
                .get(&iri("urn:eigenius:core:description"))
                .unwrap()
                .as_str(),
            Some("LLM output")
        );
    }

    // --- Phase 14a-ii: topology storage tests ---
    //
    // Wrapped in a sub-module so the `use PersistentBackend` import doesn't
    // leak into the parent test module — both `LayerStore` and
    // `PersistentBackend` define `store_layer`, and bringing both into the
    // same scope creates method-resolution ambiguity on the older async tests.
    mod topology_tests {
        use super::*;
        use eigenius_kernel::storage::PersistentBackend as PB;

        #[test]
        fn topology_round_trip_via_store_layer() {
            // PB::store_layer must populate `topo:<id>` so load_topology returns
            // the layer's handle.
            let (store, _dir) = open_temp_store();

            let mut builder = LayerBuilder::new("root", None);
            builder
                .add_resource(make_resource("urn:eigenius:core:A", vec![]))
                .unwrap();
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let id = layer.id().clone();

            PB::store_layer(&store, &layer).unwrap();

            let topology = PB::load_topology(&store).unwrap();
            assert_eq!(topology.layer_count(), 1);
            let handle = topology.get_layer(&id).expect("handle present");
            assert_eq!(handle.name, "root");
            assert!(handle.is_root());
            assert_eq!(handle.resource_count, 1);
            // created_at was populated via now_millis() on commit (non-sentinel).
            assert!(handle.created_at > 0);
        }

        #[test]
        fn topology_walk_chain_after_multiple_commits() {
            let (store, _dir) = open_temp_store();

            let mut root_builder = LayerBuilder::new("root", None);
            root_builder
                .add_resource(make_resource("urn:eigenius:core:A", vec![]))
                .unwrap();
            let root =
                Arc::new(root_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let root_id = root.id().clone();

            let mut child_builder = LayerBuilder::new("child", Some(Arc::clone(&root)));
            child_builder
                .add_resource(make_resource("urn:eigenius:example:B", vec![]))
                .unwrap();
            let child =
                Arc::new(child_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let child_id = child.id().clone();

            PB::store_layer(&store, &root).unwrap();
            PB::store_layer(&store, &child).unwrap();

            let topology = PB::load_topology(&store).unwrap();
            assert_eq!(topology.layer_count(), 2);

            // Walk from child should yield [child, root].
            let walked: Vec<&str> = topology
                .walk_chain(&child_id)
                .map(|h| h.name.as_str())
                .collect();
            assert_eq!(walked, vec!["child", "root"]);

            // Walk from root yields just [root].
            let walked_root: Vec<&str> = topology
                .walk_chain(&root_id)
                .map(|h| h.name.as_str())
                .collect();
            assert_eq!(walked_root, vec!["root"]);
        }

        #[test]
        fn topology_persists_across_reopen() {
            let dir = TempDir::new().unwrap();
            let layer_id;

            // Write via PersistentBackend; close.
            {
                let store = RocksStore::open(dir.path()).unwrap();
                let mut builder = LayerBuilder::new("persisted", None);
                builder
                    .add_resource(make_resource("urn:eigenius:core:X", vec![]))
                    .unwrap();
                let layer =
                    Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
                layer_id = layer.id().clone();
                PB::store_layer(&store, &layer).unwrap();
            }

            // Reopen; topology entry must be there without re-storing.
            {
                let store = RocksStore::open(dir.path()).unwrap();
                let topology = PB::load_topology(&store).unwrap();
                assert_eq!(topology.layer_count(), 1);
                assert!(topology.get_layer(&layer_id).is_some());
            }
        }

        #[test]
        fn topology_load_from_empty_db_is_empty() {
            let (store, _dir) = open_temp_store();
            let topology = PB::load_topology(&store).unwrap();
            assert_eq!(topology.layer_count(), 0);
        }
    } // mod topology_tests

    // --- CBOR-coverage tests for the persistent backend ---
    //
    // Wrapped in a sub-module so the `use PersistentBackend` import doesn't
    // collide with the older `LayerStore::store_layer` async tests above.
    mod cbor_coverage_tests {
        use super::*;
        use eigenius_kernel::storage::PersistentBackend as PB;
        use eigenius_kernel::storage::{BatchOp, ChainInfo};

        /// All wire-typed `Value` variants survive `store_layer` →
        /// `load_resource` through CBOR with structural equality. Variants
        /// excluded here (`ResourceRef`, `Json`) are in-memory convenience
        /// shapes that normalize to the wire-typed form on round-trip; their
        /// behavior is pinned by `value_variants_round_trip_normalizations`
        /// below.
        #[test]
        fn value_variants_round_trip() {
            let (store, _dir) = open_temp_store();

            let mut inner = Resource::new_embedded();
            inner.set(
                iri("urn:eigenius:test:city"),
                Value::String("Berlin".into()),
            );

            let mut r = Resource::new(iri("urn:eigenius:test:variants"));
            r.set(iri("urn:eigenius:test:s"), Value::String("hello".into()));
            r.set(iri("urn:eigenius:test:i"), Value::Integer(-12345));
            r.set(iri("urn:eigenius:test:f"), Value::Float(1.234567890123));
            r.set(iri("urn:eigenius:test:b"), Value::Boolean(true));
            r.set(
                iri("urn:eigenius:test:emb"),
                Value::Embedded(Box::new(inner)),
            );
            r.set(
                iri("urn:eigenius:test:arr"),
                Value::Array(vec![
                    Value::Integer(1),
                    Value::String("two".into()),
                    Value::Boolean(false),
                ]),
            );
            r.set(
                iri("urn:eigenius:test:nested_arr"),
                Value::Array(vec![Value::Array(vec![Value::Integer(42)])]),
            );

            let original = r.clone();
            let mut builder = LayerBuilder::new("variants", None);
            builder.add_resource(r).unwrap();
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let layer_id = layer.id().clone();

            PB::store_layer(&store, &layer).unwrap();

            // Read directly via the ResourceBackend surface (not load_layer,
            // which warms a cache — we want the on-disk CBOR decode path).
            let loaded = ResourceBackend::load_resource(
                &store,
                &layer_id,
                &iri("urn:eigenius:test:variants"),
            )
            .expect("resource present");

            // Resource derives PartialEq: full structural equality.
            assert_eq!(loaded, original);
        }

        /// Pins the intentional CBOR normalizations: `ResourceRef` and `Json`
        /// are in-memory convenience variants that the wire layer collapses
        /// into wire-typed forms (`String` / `Integer` / `Bool` / etc.). The
        /// String-vs-ResourceRef discrimination happens at validation time
        /// based on the property's declared `data_type`. If this test starts
        /// failing, the CBOR layer has changed its typing contract and that
        /// needs a deliberate decision (and content-addressing implications),
        /// not a silent drift.
        #[test]
        fn value_variants_round_trip_normalizations() {
            let (store, _dir) = open_temp_store();

            let mut r = Resource::new(iri("urn:eigenius:test:lossy"));
            r.set(
                iri("urn:eigenius:test:ref"),
                Value::ResourceRef(iri("urn:eigenius:test:other")),
            );
            r.set(
                iri("urn:eigenius:test:json_str"),
                Value::Json(serde_json::Value::String("hi".into())),
            );
            r.set(
                iri("urn:eigenius:test:json_num"),
                Value::Json(serde_json::Value::Number(7i64.into())),
            );

            let mut builder = LayerBuilder::new("lossy", None);
            builder.add_resource(r).unwrap();
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let layer_id = layer.id().clone();
            PB::store_layer(&store, &layer).unwrap();

            let loaded =
                ResourceBackend::load_resource(&store, &layer_id, &iri("urn:eigenius:test:lossy"))
                    .expect("resource present");

            // ResourceRef → String (same wire bytes; discrimination at
            // validation time using the property's data_type).
            assert_eq!(
                loaded.get(&iri("urn:eigenius:test:ref")),
                Some(&Value::String("urn:eigenius:test:other".into()))
            );
            // Json(String) → String, Json(Number) → Integer.
            assert_eq!(
                loaded.get(&iri("urn:eigenius:test:json_str")),
                Some(&Value::String("hi".into()))
            );
            assert_eq!(
                loaded.get(&iri("urn:eigenius:test:json_num")),
                Some(&Value::Integer(7))
            );
        }

        /// Every resource in the core ontology must round-trip with full
        /// structural equality, not just preserved count. Catches any
        /// encoder/decoder regression that drops or mangles fields.
        #[test]
        fn core_ontology_field_level_equality() {
            let (store, _dir) = open_temp_store();
            let core_json = include_str!("../../../ontologies/core/core-ontology.json");
            let resources = eigon_json::parse_document(core_json).unwrap();

            let mut originals: std::collections::BTreeMap<Iri, Resource> =
                std::collections::BTreeMap::new();
            for r in &resources {
                originals.insert(r.id().expect("core resource has @id").clone(), r.clone());
            }

            let mut builder = LayerBuilder::new("core", None);
            for r in resources {
                builder.add_resource(r).unwrap();
            }
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let id = layer.id().clone();

            PB::store_layer(&store, &layer).unwrap();

            // Read each one back through the backend and compare.
            for (iri, original) in &originals {
                let loaded = ResourceBackend::load_resource(&store, &id, iri)
                    .unwrap_or_else(|| panic!("missing core resource {iri}"));
                assert_eq!(&loaded, original, "round-trip mismatch for {iri}");
            }

            // And nothing extra appeared.
            let loaded_iris = ResourceBackend::list_layer_iris(&store, &id).unwrap();
            assert_eq!(
                loaded_iris,
                originals
                    .keys()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>()
            );
        }

        /// `build_chain` against the live `RocksStore` backend with a fresh
        /// cache: every `resolve` must hit the backend's CBOR-decode path,
        /// since the cache starts empty. This is the path that production
        /// uses but no existing test exercises end-to-end.
        #[test]
        fn chain_resolve_with_cold_cache() {
            let (store, _dir) = open_temp_store();
            let store_arc: Arc<RocksStore> = Arc::new(store);

            // Build root with one resource.
            let mut root_builder = LayerBuilder::new("root", None);
            root_builder
                .add_resource(make_resource(
                    "urn:eigenius:core:Class",
                    vec![(
                        "urn:eigenius:core:description",
                        Value::String("class".into()),
                    )],
                ))
                .unwrap();
            let root =
                Arc::new(root_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));

            // Build child with another resource.
            let mut child_builder = LayerBuilder::new("domain", Some(Arc::clone(&root)));
            child_builder
                .add_resource(make_resource(
                    "urn:eigenius:example:Dog",
                    vec![("urn:eigenius:core:description", Value::String("dog".into()))],
                ))
                .unwrap();
            let child =
                Arc::new(child_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let child_id = child.id().clone();

            PB::store_layer(&*store_arc, &root).unwrap();
            PB::store_layer(&*store_arc, &child).unwrap();
            // Phase 14g: head pointer via `branch:main`.
            PB::put_branch(&*store_arc, "main", &child_id).unwrap();

            // Drop the original layer Arcs so their throwaway caches go away.
            drop(root);
            drop(child);

            // Reconstruct the chain pointing at the live RocksStore — fresh
            // cache, real backend.
            let main_head = PB::get_branch(&*store_arc, "main").unwrap().unwrap();
            let info = PB::load_chain_from(&*store_arc, &main_head)
                .unwrap()
                .expect("chain present");
            // Storage backed by the live RocksStore — fresh resource cache
            // (cold), bloom cache backed by the same store so cold-resolve
            // exercises both backend probes.
            let pb_arc: Arc<dyn eigenius_kernel::storage::PersistentBackend> =
                Arc::clone(&store_arc) as _;
            let storage = eigenius_kernel::layer::LayerStorage::with_persistent(pb_arc);
            let head = eigenius_kernel::layer::build_chain(info, storage.clone());

            // Cache is empty: this resolve must traverse the parent chain and
            // decode CBOR from RocksDB.
            let class = head
                .resolve(&iri("urn:eigenius:core:Class"))
                .expect("Class resolves through cold cache");
            assert_eq!(
                class
                    .get(&iri("urn:eigenius:core:description"))
                    .and_then(|v| v.as_str()),
                Some("class")
            );

            let dog = head
                .resolve(&iri("urn:eigenius:example:Dog"))
                .expect("Dog resolves through cold cache");
            assert_eq!(
                dog.get(&iri("urn:eigenius:core:description"))
                    .and_then(|v| v.as_str()),
                Some("dog")
            );

            // Cache should now have populated entries (proving misses fell
            // through to the backend rather than silently failing).
            assert!(storage.cache.stats().entries >= 2);
        }

        /// `meta:` key/value surface — `put_meta`/`get_meta`/`delete_meta`/
        /// `list_meta_prefix`. This is the substrate D21 task storage runs on,
        /// previously untested at the `PersistentBackend` level.
        #[test]
        fn meta_kv_round_trip() {
            let (store, _dir) = open_temp_store();

            assert!(PB::get_meta(&store, "absent").unwrap().is_none());

            PB::put_meta(&store, "session:abc", b"value-abc").unwrap();
            PB::put_meta(&store, "session:def", b"value-def").unwrap();
            PB::put_meta(&store, "other:xyz", b"value-xyz").unwrap();

            assert_eq!(
                PB::get_meta(&store, "session:abc").unwrap().as_deref(),
                Some(b"value-abc".as_ref())
            );
            assert_eq!(
                PB::get_meta(&store, "session:def").unwrap().as_deref(),
                Some(b"value-def".as_ref())
            );

            // list_meta_prefix scopes correctly.
            let session_keys = PB::list_meta_prefix(&store, "session:").unwrap();
            let mut session_sorted = session_keys.clone();
            session_sorted.sort();
            assert_eq!(session_sorted, vec!["session:abc", "session:def"]);

            // delete_meta on present key removes it.
            PB::delete_meta(&store, "session:abc").unwrap();
            assert!(PB::get_meta(&store, "session:abc").unwrap().is_none());

            // delete_meta on absent key is a no-op (per trait contract).
            PB::delete_meta(&store, "session:never_existed").unwrap();

            // Other prefix unaffected.
            assert_eq!(
                PB::get_meta(&store, "other:xyz").unwrap().as_deref(),
                Some(b"value-xyz".as_ref())
            );
        }

        /// `write_batch` must apply every operation. Per D21 §8 step
        /// atomicity, this is the single-commit primitive task steps use;
        /// correctness here is structural.
        #[test]
        fn write_batch_applies_all_ops() {
            let (store, _dir) = open_temp_store();

            // Pre-populate one key so we can verify a delete inside the batch.
            PB::put_meta(&store, "to_delete", b"old").unwrap();

            let ops = vec![
                BatchOp::PutMeta {
                    key: "k1".into(),
                    value: b"v1".to_vec(),
                },
                BatchOp::PutMeta {
                    key: "k2".into(),
                    value: b"v2".to_vec(),
                },
                BatchOp::DeleteMeta {
                    key: "to_delete".into(),
                },
                BatchOp::PutMeta {
                    key: "k3".into(),
                    value: b"v3".to_vec(),
                },
            ];
            PB::write_batch(&store, &ops).unwrap();

            assert_eq!(
                PB::get_meta(&store, "k1").unwrap().as_deref(),
                Some(b"v1".as_ref())
            );
            assert_eq!(
                PB::get_meta(&store, "k2").unwrap().as_deref(),
                Some(b"v2".as_ref())
            );
            assert_eq!(
                PB::get_meta(&store, "k3").unwrap().as_deref(),
                Some(b"v3".as_ref())
            );
            assert!(PB::get_meta(&store, "to_delete").unwrap().is_none());
        }

        /// `load_chain_from(head_id)` walks from an arbitrary layer, not
        /// just the persisted head. Critical for `at_layer` reads and task
        /// resume that pin specific heads. Multi-head test: two children
        /// off one parent must each rebuild the correct chain.
        #[test]
        fn load_chain_from_specific_head() {
            let (store, _dir) = open_temp_store();

            let mut root_b = LayerBuilder::new("root", None);
            root_b
                .add_resource(make_resource("urn:eigenius:core:R", vec![]))
                .unwrap();
            let root = Arc::new(root_b.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let root_id = root.id().clone();

            // Two distinct children off the same root — distinct because
            // they define different IRIs.
            let mut a_b = LayerBuilder::new("child_a", Some(Arc::clone(&root)));
            a_b.add_resource(make_resource("urn:eigenius:example:A", vec![]))
                .unwrap();
            let child_a = Arc::new(a_b.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let a_id = child_a.id().clone();

            let mut b_b = LayerBuilder::new("child_b", Some(Arc::clone(&root)));
            b_b.add_resource(make_resource("urn:eigenius:example:B", vec![]))
                .unwrap();
            let child_b = Arc::new(b_b.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let b_id = child_b.id().clone();

            PB::store_layer(&store, &root).unwrap();
            PB::store_layer(&store, &child_a).unwrap();
            PB::store_layer(&store, &child_b).unwrap();
            // Note: no `set_head` — load_chain_from must not depend on it.

            let info_a: ChainInfo = PB::load_chain_from(&store, &a_id)
                .unwrap()
                .expect("chain for a");
            assert_eq!(info_a.head, a_id);
            let names_a: Vec<&str> = info_a.handles.iter().map(|h| h.name.as_str()).collect();
            assert_eq!(names_a, vec!["root", "child_a"]);
            assert!(info_a.defined_iris_per_layer.contains_key(&root_id));
            assert!(info_a.defined_iris_per_layer.contains_key(&a_id));

            let info_b: ChainInfo = PB::load_chain_from(&store, &b_id)
                .unwrap()
                .expect("chain for b");
            assert_eq!(info_b.head, b_id);
            let names_b: Vec<&str> = info_b.handles.iter().map(|h| h.name.as_str()).collect();
            assert_eq!(names_b, vec!["root", "child_b"]);
            assert!(info_b.defined_iris_per_layer.contains_key(&b_id));

            // Asking for the root alone yields a one-element chain.
            let info_root: ChainInfo = PB::load_chain_from(&store, &root_id)
                .unwrap()
                .expect("chain for root");
            assert_eq!(info_root.head, root_id);
            let names_root: Vec<&str> = info_root.handles.iter().map(|h| h.name.as_str()).collect();
            assert_eq!(names_root, vec!["root"]);
        }

        /// Phase 14b: `store_layer` writes a `bloom:<id>` entry and
        /// `load_bloom` reads it back. Round-trips through CBOR via
        /// `ciborium`. Verified by reconstructing the same bloom from the
        /// original IRI set and asserting structural equality, plus
        /// confirming `might_contain` agrees on every inserted IRI.
        #[test]
        fn bloom_round_trip_via_store_layer() {
            use eigenius_kernel::layer::BloomFilter;

            let (store, _dir) = open_temp_store();

            let mut builder = LayerBuilder::new("bloom_layer", None);
            for i in 0..200 {
                builder
                    .add_resource(make_resource(&format!("urn:eigenius:test:r{i}"), vec![]))
                    .unwrap();
            }
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let id = layer.id().clone();
            let original_iris = layer.defined_iris().clone();

            PB::store_layer(&store, &layer).unwrap();

            let loaded = PB::load_bloom(&store, &id).unwrap().expect("bloom present");
            let expected = BloomFilter::for_iris(&original_iris);
            assert_eq!(
                loaded, expected,
                "bloom must survive CBOR round-trip intact"
            );
            for iri_h in &original_iris {
                assert!(loaded.might_contain(iri_h));
            }
        }

        /// Bloom + topology + content + chain must all be visible after
        /// `store_layer`. This validates the D23 §6.3 atomic-commit
        /// contract — the new `WriteBatch` shape applies them as one
        /// commit; nothing should land partially.
        #[test]
        fn store_layer_writes_all_keys_atomically() {
            let (store, _dir) = open_temp_store();

            let mut builder = LayerBuilder::new("atomic", None);
            builder
                .add_resource(make_resource("urn:eigenius:test:a", vec![]))
                .unwrap();
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let id = layer.id().clone();

            PB::store_layer(&store, &layer).unwrap();

            // Topology entry present.
            let topology = PB::load_topology(&store).unwrap();
            assert!(topology.get_layer(&id).is_some());
            // Bloom present.
            assert!(PB::load_bloom(&store, &id).unwrap().is_some());
            // Resource present.
            assert!(
                ResourceBackend::load_resource(&store, &id, &iri("urn:eigenius:test:a")).is_some()
            );
            // Chain entry present (root layer — empty parent).
            let info = PB::load_chain_from(&store, &id).unwrap().expect("chain");
            assert_eq!(info.handles.len(), 1);
            assert!(info.handles[0].is_root());
        }

        /// `store_bloom` standalone path (separate from `store_layer`'s
        /// commit batch). Useful for migrations and tests.
        #[test]
        fn store_bloom_standalone_round_trip() {
            use eigenius_kernel::layer::BloomFilter;
            use std::collections::BTreeSet;

            let (store, _dir) = open_temp_store();
            let layer_id = LayerId([13u8; 32]);

            // No bloom yet.
            assert!(PB::load_bloom(&store, &layer_id).unwrap().is_none());

            let iris: BTreeSet<_> = (0..50)
                .map(|i| iri(&format!("urn:eigenius:test:s{i}")))
                .collect();
            let bloom = BloomFilter::for_iris(&iris);
            PB::store_bloom(&store, &layer_id, &bloom).unwrap();

            let loaded = PB::load_bloom(&store, &layer_id).unwrap().expect("present");
            assert_eq!(loaded, bloom);
        }

        /// Phase 14d: branch ref round-trip through RocksDB. Validates
        /// `branch:<name>` key encoding, multi-branch enumeration order,
        /// and persistence across reopen (key is plain bytes, no CBOR
        /// surface to drift).
        #[test]
        fn branch_refs_round_trip() {
            let dir = TempDir::new().unwrap();
            let id_a = LayerId([7u8; 32]);
            let id_b = LayerId([8u8; 32]);

            // Write + close.
            {
                let store = RocksStore::open(dir.path()).unwrap();
                assert!(PB::get_branch(&store, "main").unwrap().is_none());
                assert!(PB::list_branches(&store).unwrap().is_empty());

                PB::put_branch(&store, "main", &id_a).unwrap();
                PB::put_branch(&store, "auto-divergent-1", &id_b).unwrap();

                let listed = PB::list_branches(&store).unwrap();
                assert_eq!(listed.len(), 2);
                // Sorted by name.
                assert_eq!(listed[0], ("auto-divergent-1".into(), id_b.clone()));
                assert_eq!(listed[1], ("main".into(), id_a.clone()));
            }

            // Reopen — branch refs survive.
            {
                let store = RocksStore::open(dir.path()).unwrap();
                assert_eq!(PB::get_branch(&store, "main").unwrap(), Some(id_a.clone()));
                assert_eq!(
                    PB::get_branch(&store, "auto-divergent-1").unwrap(),
                    Some(id_b.clone())
                );

                // Delete + verify.
                PB::delete_branch(&store, "main").unwrap();
                assert!(PB::get_branch(&store, "main").unwrap().is_none());
                let remaining = PB::list_branches(&store).unwrap();
                assert_eq!(remaining.len(), 1);
                assert_eq!(remaining[0].0, "auto-divergent-1");

                // Delete on absent is a no-op.
                PB::delete_branch(&store, "main").unwrap();
            }
        }
    } // mod cbor_coverage_tests

    #[tokio::test]
    async fn trace_store_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let key = [99u8; 32];

        // Write trace
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let trace = ComponentTrace {
                component: "urn:test:comp".to_string(),
                input_hash: key,
                argument_hash: None,
                output: Resource::new(iri("urn:test:out")),
                cached: false,
                metrics: None,
            };
            store.put_component_trace(key, trace);
        }

        // Reopen and verify
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let loaded = store.get_component_trace(&key);
            assert!(loaded.is_some());
            assert_eq!(loaded.unwrap().component, "urn:test:comp");
        }
    }
}
