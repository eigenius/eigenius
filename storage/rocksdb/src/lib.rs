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
use eigenius_kernel::layer::{Layer, LayerHandle, LayerId, LayerTopology};
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::storage::{LayerStore, ResourceBackend, ResourceStore, StorageError};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const TOPO_PREFIX: &str = "topo:";

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

    /// Store the head layer ID.
    pub fn set_head(&self, layer_id: &LayerId) -> Result<(), StorageError> {
        self.db
            .put(b"head", hex::encode(layer_id.0))
            .map_err(|e| StorageError::Internal(format!("failed to set head: {e}")))
    }

    /// Get the current head layer ID.
    pub fn get_head(&self) -> Result<Option<LayerId>, StorageError> {
        match self.db.get(b"head") {
            Ok(Some(bytes)) => {
                let hex_str = String::from_utf8(bytes)
                    .map_err(|e| StorageError::Internal(format!("invalid head value: {e}")))?;
                let id = hex_to_layer_id(&hex_str)?;
                Ok(Some(id))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(StorageError::Internal(format!("failed to get head: {e}"))),
        }
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
        let parent_id = layer.parent().map(|p| p.id().clone());

        // Phase 14a-ii: write the canonical CBOR topology entry. Carries
        // name, parents, resource_count, created_at — supersedes the
        // pre-Phase-14 JSON `layer:<id>:meta` write.
        let handle = LayerHandle {
            id: id.clone(),
            parents: parent_id.clone().into_iter().collect(),
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

        // Store chain pointer
        self.set_chain(&id, parent_id.as_ref())?;

        Ok(id)
    }

    async fn load_layer(&self, id: &LayerId) -> Result<Layer, StorageError> {
        // Phase 14a-iii: rebuild as a parent-less Layer with self as both
        // cache (fresh) and backend. Used by tests that exercise the older
        // `LayerStore` API in isolation; production code uses
        // `PersistentBackend::load_chain` + `build_chain` instead.
        let (name, _parent_id) = self.load_layer_meta(id)?;
        let defined_iris = ResourceBackend::list_layer_iris(self, id)?;
        let cache: Arc<dyn eigenius_kernel::layer::ResourceCache> =
            Arc::new(eigenius_kernel::layer::MemoryResourceCache::new());
        // Use a no-op self-cloned backend reference: to avoid the trait
        // upcast from RocksStore Arc, we wrap a fresh MemoryResourceBackend
        // (the loaded layer's lookups will hit the cache populated below).
        let backend: Arc<dyn ResourceBackend> =
            Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new());
        let handle = LayerHandle {
            id: id.clone(),
            parents: Vec::new(),
            name,
            resource_count: defined_iris.len() as u64,
            created_at: 0,
        };
        // Pre-populate the temporary cache from RocksDB so reads via the
        // returned Layer succeed.
        for iri in &defined_iris {
            if let Some(resource) = ResourceBackend::load_resource(self, id, iri) {
                cache.put(
                    eigenius_kernel::layer::ResourceKey::new(id.clone(), iri.clone()),
                    Arc::new(resource),
                );
            }
        }
        Ok(Layer::from_handle(
            handle,
            None,
            defined_iris,
            cache,
            backend,
        ))
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
    fn get_head(&self) -> Result<Option<LayerId>, StorageError> {
        RocksStore::get_head(self)
    }

    fn set_head(&self, id: &LayerId) -> Result<(), StorageError> {
        RocksStore::set_head(self, id)
    }

    fn load_chain(&self) -> Result<Option<eigenius_kernel::storage::ChainInfo>, StorageError> {
        match self.get_head()? {
            Some(head) => self.build_chain_info(&head),
            None => Ok(None),
        }
    }

    fn load_chain_from(
        &self,
        head_id: &LayerId,
    ) -> Result<Option<eigenius_kernel::storage::ChainInfo>, StorageError> {
        self.build_chain_info(head_id)
    }

    fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError> {
        // The LayerStore impl is async but its body is purely synchronous
        // RocksDB work; re-implement synchronously here to avoid
        // blocking-in-async cases. Matches D13 §5 "commit-through" design.
        let id = layer.id().clone();
        let parent_id = layer.parent().map(|p| p.id().clone());

        // Phase 14a-ii: canonical CBOR topology entry replaces the legacy
        // JSON `layer:<id>:meta` write. Carries name, parents,
        // resource_count, created_at.
        let handle = LayerHandle {
            id: id.clone(),
            parents: parent_id.clone().into_iter().collect(),
            name: layer.name().to_string(),
            resource_count: layer.defined_iris().len() as u64,
            created_at: now_millis(),
        };
        self.put_topology_entry(&handle)?;

        for (iri, resource) in layer.iter_resources() {
            let key = format!("layer:{}:res:{}", hex::encode(id.0), iri.as_str());
            let value = eigon_cbor::serialize_resource(&resource);
            self.db
                .put(key.as_bytes(), value)
                .map_err(|e| StorageError::Internal(format!("failed to store resource: {e}")))?;
        }
        self.set_chain(&id, parent_id.as_ref())?;
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
        let layer = builder.build(
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
        );
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
        let l1 = b1.build(
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
        );

        let mut b2 = LayerBuilder::new("b", None);
        b2.add_resource(make_resource("urn:eigenius:core:y", vec![]))
            .unwrap();
        let l2 = b2.build(
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
        );

        store.store_layer(&l1).await.unwrap();
        store.store_layer(&l2).await.unwrap();

        let ids = store.list_layers().await.unwrap();
        assert_eq!(ids.len(), 2);
    }

    #[tokio::test]
    async fn head_pointer() {
        let (store, _dir) = open_temp_store();

        assert!(store.get_head().unwrap().is_none());

        let mut builder = LayerBuilder::new("test", None);
        builder
            .add_resource(make_resource("urn:eigenius:core:x", vec![]))
            .unwrap();
        let layer = builder.build(
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
        );
        let id = layer.id().clone();

        store.store_layer(&layer).await.unwrap();
        store.set_head(&id).unwrap();

        assert_eq!(store.get_head().unwrap().unwrap(), id);
    }

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
            let layer = builder.build(
                std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
                std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
            );
            let id = layer.id().clone();
            store.store_layer(&layer).await.unwrap();
            store.set_head(&id).unwrap();
        }

        // Reopen and verify
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let head = store.get_head().unwrap();
            assert!(head.is_some());

            let layer = store.load_layer(&head.unwrap()).await.unwrap();
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
        let root = Arc::new(root_builder.build(
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
        ));
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
        let child = child_builder.build(
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
        );
        let child_id = child.id().clone();
        store.store_layer(&child).await.unwrap();
        store.set_head(&child_id).unwrap();

        // Reconstruct chain via the new ChainInfo + build_chain pattern.
        let info = eigenius_kernel::storage::PersistentBackend::load_chain(&store)
            .unwrap()
            .expect("chain present");
        let cache: Arc<dyn eigenius_kernel::layer::ResourceCache> =
            Arc::new(eigenius_kernel::layer::MemoryResourceCache::new());
        let backend: Arc<dyn ResourceBackend> =
            Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new());
        // Pre-warm the cache from the persistent store so resolve hits succeed.
        for handle in &info.handles {
            if let Some(iris) = info.defined_iris_per_layer.get(&handle.id) {
                for iri_h in iris {
                    if let Some(r) = ResourceBackend::load_resource(&store, &handle.id, iri_h) {
                        cache.put(
                            eigenius_kernel::layer::ResourceKey::new(
                                handle.id.clone(),
                                iri_h.clone(),
                            ),
                            Arc::new(r),
                        );
                    }
                }
            }
        }
        let head = eigenius_kernel::layer::build_chain(info, cache, backend);
        assert!(!head.is_root());
        // Should resolve resources from both layers
        assert!(head.resolve(&iri("urn:eigenius:core:Class")).is_some());
        assert!(head.resolve(&iri("urn:eigenius:example:Dog")).is_some());
    }

    #[tokio::test]
    async fn core_ontology_round_trip() {
        let (store, _dir) = open_temp_store();

        // Load core ontology, store as a layer, reload, verify
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let resources = eigon_json::parse_document(core_json).unwrap();
        let count = resources.len();

        let mut builder = LayerBuilder::new("core", None);
        for r in resources {
            builder.add_resource(r).unwrap();
        }
        let layer = builder.build(
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
            std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
        );
        let id = layer.id().clone();

        store.store_layer(&layer).await.unwrap();
        let loaded = store.load_layer(&id).await.unwrap();

        assert_eq!(loaded.iter_resources().count(), count);
    }

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
            let layer = Arc::new(builder.build(
                std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
                std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
            ));
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
            let root = Arc::new(root_builder.build(
                std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
                std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
            ));
            let root_id = root.id().clone();

            let mut child_builder = LayerBuilder::new("child", Some(Arc::clone(&root)));
            child_builder
                .add_resource(make_resource("urn:eigenius:example:B", vec![]))
                .unwrap();
            let child = Arc::new(child_builder.build(
                std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
                std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
            ));
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
                let layer = Arc::new(builder.build(
                    std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceCache::new()),
                    std::sync::Arc::new(eigenius_kernel::layer::MemoryResourceBackend::new()),
                ));
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
