//! RocksDB storage backend for Eigenius.
//!
//! Implements `LayerStore` and `ResourceStore` using RocksDB as the
//! persistent ordered key-value store. Key encoding follows D4.
//!
//! Key scheme:
//!   layer:<layer_id_hex>:meta         → Layer metadata (CBOR)
//!   layer:<layer_id_hex>:res:<iri>    → Resource (CBOR)
//!   chain:<layer_id_hex>              → Parent layer ID hex (or empty)
//!   head                              → Current head layer ID hex

use async_trait::async_trait;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerId};
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::storage::{LayerStore, ResourceStore, StorageError};
use std::path::Path;
use std::sync::Arc;

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

    /// Reconstruct the full layer chain from storage, starting from the head.
    pub fn load_chain(&self) -> Result<Option<Arc<Layer>>, StorageError> {
        let head_id = match self.get_head()? {
            Some(id) => id,
            None => return Ok(None),
        };

        self.load_chain_from(&head_id)
    }

    /// Reconstruct a layer chain from a specific layer ID.
    fn load_chain_from(&self, layer_id: &LayerId) -> Result<Option<Arc<Layer>>, StorageError> {
        // Walk the chain to collect IDs from head to root
        let mut chain_ids = vec![layer_id.clone()];
        let mut current = layer_id.clone();
        while let Some(parent_id) = self.get_chain(&current)? {
            chain_ids.push(parent_id.clone());
            current = parent_id;
        }

        // Build layers from root to head
        chain_ids.reverse();
        let mut parent: Option<Arc<Layer>> = None;

        for id in &chain_ids {
            let (name, _parent_id) = self.load_layer_meta(id)?;
            let resources = self.load_all_resources(id)?;

            let mut builder = LayerBuilder::new(&name, parent.clone());
            for resource in resources {
                builder
                    .add_resource(resource)
                    .map_err(|e| StorageError::Internal(format!("rebuild error: {e}")))?;
            }
            parent = Some(Arc::new(builder.build()));
        }

        Ok(parent)
    }

    /// Load all resources for a given layer.
    fn load_all_resources(&self, layer_id: &LayerId) -> Result<Vec<Resource>, StorageError> {
        let prefix = format!("layer:{}:res:", hex::encode(layer_id.0));
        let mut resources = Vec::new();

        let iter = self.db.prefix_iterator(prefix.as_bytes());
        for item in iter {
            let (key, value) =
                item.map_err(|e| StorageError::Internal(format!("iteration error: {e}")))?;

            let key_str = String::from_utf8_lossy(&key);
            if !key_str.starts_with(&prefix) {
                break; // Past the prefix
            }

            let resource = eigon_cbor::parse_resource(&value)
                .map_err(|e| StorageError::Internal(format!("CBOR parse error: {e}")))?;
            resources.push(resource);
        }

        Ok(resources)
    }

    /// Store layer metadata.
    fn store_layer_meta(
        &self,
        layer_id: &LayerId,
        name: &str,
        parent_id: Option<&LayerId>,
    ) -> Result<(), StorageError> {
        let key = format!("layer:{}:meta", hex::encode(layer_id.0));
        let meta = serde_json::json!({
            "name": name,
            "parent_id": parent_id.map(|p| hex::encode(p.0)),
        });
        let value = serde_json::to_vec(&meta)
            .map_err(|e| StorageError::Internal(format!("meta serialize error: {e}")))?;
        self.db
            .put(key.as_bytes(), value)
            .map_err(|e| StorageError::Internal(format!("failed to store meta: {e}")))
    }

    /// Load layer metadata.
    fn load_layer_meta(
        &self,
        layer_id: &LayerId,
    ) -> Result<(String, Option<LayerId>), StorageError> {
        let key = format!("layer:{}:meta", hex::encode(layer_id.0));
        let bytes = self
            .db
            .get(key.as_bytes())
            .map_err(|e| StorageError::Internal(format!("failed to load meta: {e}")))?
            .ok_or_else(|| StorageError::NotFound(format!("layer {}", hex::encode(layer_id.0))))?;

        let meta: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| StorageError::Internal(format!("meta parse error: {e}")))?;

        let name = meta["name"].as_str().unwrap_or("unknown").to_string();
        let parent_id = meta["parent_id"]
            .as_str()
            .and_then(|s| hex_to_layer_id(s).ok());

        Ok((name, parent_id))
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

/// Serialize a ComponentTrace to JSON bytes for storage.
fn serialize_component_trace(trace: &ComponentTrace) -> Result<Vec<u8>, StorageError> {
    let output_json = eigenius_kernel::ontology::eigon_json::serialize_resource(&trace.output);
    let mut obj = serde_json::Map::new();
    obj.insert(
        "component".into(),
        serde_json::Value::String(trace.component.clone()),
    );
    obj.insert(
        "input_hash".into(),
        serde_json::Value::String(hex::encode(trace.input_hash)),
    );
    if let Some(ah) = &trace.argument_hash {
        obj.insert(
            "argument_hash".into(),
            serde_json::Value::String(hex::encode(ah)),
        );
    }
    obj.insert("output".into(), output_json);
    obj.insert("cached".into(), serde_json::Value::Bool(trace.cached));
    if let Some(m) = &trace.metrics {
        let mut metrics = serde_json::Map::new();
        metrics.insert(
            "provider".into(),
            serde_json::Value::String(m.provider.clone()),
        );
        metrics.insert("model".into(), serde_json::Value::String(m.model.clone()));
        metrics.insert(
            "prompt_tokens".into(),
            serde_json::Value::Number(m.prompt_tokens.into()),
        );
        metrics.insert(
            "completion_tokens".into(),
            serde_json::Value::Number(m.completion_tokens.into()),
        );
        metrics.insert(
            "latency_ms".into(),
            serde_json::Value::Number(m.latency_ms.into()),
        );
        obj.insert("metrics".into(), serde_json::Value::Object(metrics));
    }
    serde_json::to_vec(&serde_json::Value::Object(obj))
        .map_err(|e| StorageError::Internal(format!("serialize trace: {e}")))
}

/// Deserialize a ComponentTrace from JSON bytes.
fn deserialize_component_trace(bytes: &[u8]) -> Result<ComponentTrace, StorageError> {
    let obj: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| StorageError::Internal(format!("deserialize trace: {e}")))?;

    let component = obj["component"]
        .as_str()
        .ok_or_else(|| StorageError::Internal("missing component".into()))?
        .to_string();

    let input_hash_hex = obj["input_hash"]
        .as_str()
        .ok_or_else(|| StorageError::Internal("missing input_hash".into()))?;
    let input_hash_bytes = hex::decode(input_hash_hex)
        .map_err(|e| StorageError::Internal(format!("invalid input_hash: {e}")))?;
    let mut input_hash = [0u8; 32];
    if input_hash_bytes.len() == 32 {
        input_hash.copy_from_slice(&input_hash_bytes);
    }

    let argument_hash = obj
        .get("argument_hash")
        .and_then(|v| v.as_str())
        .and_then(|s| hex::decode(s).ok())
        .and_then(|b| {
            if b.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&b);
                Some(arr)
            } else {
                None
            }
        });

    let output_json = obj["output"].to_string();
    let output = eigenius_kernel::ontology::eigon_json::parse_embedded(&output_json)
        .or_else(|_| {
            eigenius_kernel::ontology::eigon_json::parse_document(&output_json)
                .map(|mut v| v.pop().unwrap_or_else(Resource::new_embedded))
        })
        .map_err(|e| StorageError::Internal(format!("parse trace output: {e}")))?;

    let metrics = obj.get("metrics").and_then(|m| {
        Some(ComponentMetrics {
            provider: m["provider"].as_str()?.to_string(),
            model: m["model"].as_str()?.to_string(),
            prompt_tokens: m["prompt_tokens"].as_i64()?,
            completion_tokens: m["completion_tokens"].as_i64()?,
            latency_ms: m["latency_ms"].as_i64()?,
        })
    });

    Ok(ComponentTrace {
        component,
        input_hash,
        argument_hash,
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

        // Store metadata
        self.store_layer_meta(&id, layer.name(), parent_id.as_ref())?;

        // Store each resource as CBOR
        for (iri, resource) in layer.resources() {
            let key = format!("layer:{}:res:{}", hex::encode(id.0), iri.as_str());
            let value = eigon_cbor::serialize_resource(resource);
            self.db
                .put(key.as_bytes(), value)
                .map_err(|e| StorageError::Internal(format!("failed to store resource: {e}")))?;
        }

        // Store chain pointer
        self.set_chain(&id, parent_id.as_ref())?;

        Ok(id)
    }

    async fn load_layer(&self, id: &LayerId) -> Result<Layer, StorageError> {
        let (name, _parent_id) = self.load_layer_meta(id)?;
        let resources = self.load_all_resources(id)?;

        // Rebuild layer (without parent pointer — caller reconstructs chain)
        let mut builder = LayerBuilder::new(&name, None);
        for resource in resources {
            builder
                .add_resource(resource)
                .map_err(|e| StorageError::Internal(format!("rebuild error: {e}")))?;
        }

        Ok(builder.build())
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

// --- PersistentBackend (D13) ---

impl eigenius_kernel::storage::PersistentBackend for RocksStore {
    fn get_head(&self) -> Result<Option<LayerId>, StorageError> {
        RocksStore::get_head(self)
    }

    fn set_head(&self, id: &LayerId) -> Result<(), StorageError> {
        RocksStore::set_head(self, id)
    }

    fn load_chain(&self) -> Result<Option<Arc<Layer>>, StorageError> {
        RocksStore::load_chain(self)
    }

    fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError> {
        // The LayerStore impl is async but its body is purely synchronous
        // RocksDB work; re-implement synchronously here to avoid
        // blocking-in-async cases. Matches D13 §5 "commit-through" design.
        let id = layer.id().clone();
        let parent_id = layer.parent().map(|p| p.id().clone());

        self.store_layer_meta(&id, layer.name(), parent_id.as_ref())?;
        for (iri, resource) in layer.resources() {
            let key = format!("layer:{}:res:{}", hex::encode(id.0), iri.as_str());
            let value = eigon_cbor::serialize_resource(resource);
            self.db
                .put(key.as_bytes(), value)
                .map_err(|e| StorageError::Internal(format!("failed to store resource: {e}")))?;
        }
        self.set_chain(&id, parent_id.as_ref())?;
        Ok(id)
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
        let layer = builder.build();
        let id = layer.id().clone();

        store.store_layer(&layer).await.unwrap();
        let loaded = store.load_layer(&id).await.unwrap();
        assert_eq!(loaded.name(), "test");
        assert_eq!(loaded.resources().len(), 1);
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
        let loaded = store
            .load_resource(&layer_id, &iri("urn:eigenius:test:foo"))
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
        let l1 = b1.build();

        let mut b2 = LayerBuilder::new("b", None);
        b2.add_resource(make_resource("urn:eigenius:core:y", vec![]))
            .unwrap();
        let l2 = b2.build();

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
        let layer = builder.build();
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
            let layer = builder.build();
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
        let root = Arc::new(root_builder.build());
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
        let child = child_builder.build();
        let child_id = child.id().clone();
        store.store_layer(&child).await.unwrap();
        store.set_head(&child_id).unwrap();

        // Reconstruct chain
        let reconstructed = store.load_chain().unwrap();
        assert!(reconstructed.is_some());

        let head = reconstructed.unwrap();
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
        let layer = builder.build();
        let id = layer.id().clone();

        store.store_layer(&layer).await.unwrap();
        let loaded = store.load_layer(&id).await.unwrap();

        assert_eq!(loaded.resources().len(), count);
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
