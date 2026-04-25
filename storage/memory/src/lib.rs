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

//! In-memory storage backend for testing and development.
//!
//! Implements `LayerStore` and `ResourceStore` using BTreeMaps
//! behind `Arc<RwLock<...>>` for thread-safe concurrent access.

use async_trait::async_trait;
use eigenius_kernel::layer::{Layer, LayerId};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::storage::{LayerStore, ResourceStore, StorageError};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory storage backend.
///
/// Suitable for testing, development, and small deployments.
/// Data does not survive process restarts.
#[derive(Clone)]
pub struct InMemoryStore {
    layers: Arc<RwLock<BTreeMap<LayerId, Layer>>>,
    resources: Arc<RwLock<BTreeMap<(LayerId, Iri), Resource>>>,
}

impl InMemoryStore {
    /// Create a new empty in-memory store.
    pub fn new() -> Self {
        Self {
            layers: Arc::new(RwLock::new(BTreeMap::new())),
            resources: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LayerStore for InMemoryStore {
    async fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError> {
        let id = layer.id().clone();
        let mut layers = self.layers.write().await;
        layers.insert(id.clone(), layer.clone());
        Ok(id)
    }

    async fn load_layer(&self, id: &LayerId) -> Result<Layer, StorageError> {
        let layers = self.layers.read().await;
        layers
            .get(id)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(format!("layer {id}")))
    }

    async fn list_layers(&self) -> Result<Vec<LayerId>, StorageError> {
        let layers = self.layers.read().await;
        Ok(layers.keys().cloned().collect())
    }
}

#[async_trait]
impl ResourceStore for InMemoryStore {
    async fn store_resource(
        &self,
        layer_id: &LayerId,
        resource: &Resource,
    ) -> Result<(), StorageError> {
        let iri = resource
            .id()
            .ok_or_else(|| StorageError::Internal("resource has no @id".to_string()))?
            .clone();
        let mut resources = self.resources.write().await;
        resources.insert((layer_id.clone(), iri), resource.clone());
        Ok(())
    }

    async fn load_resource(
        &self,
        layer_id: &LayerId,
        iri: &Iri,
    ) -> Result<Option<Resource>, StorageError> {
        let resources = self.resources.read().await;
        Ok(resources.get(&(layer_id.clone(), iri.clone())).cloned())
    }

    async fn list_resources(&self, layer_id: &LayerId) -> Result<Vec<Iri>, StorageError> {
        let resources = self.resources.read().await;
        let iris: Vec<Iri> = resources
            .keys()
            .filter(|(lid, _)| lid == layer_id)
            .map(|(_, iri)| iri.clone())
            .collect();
        Ok(iris)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::layer::LayerBuilder;
    use eigenius_kernel::ontology::resource::Value;

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

    #[tokio::test]
    async fn store_and_load_layer() {
        let store = InMemoryStore::new();
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
        assert_eq!(loaded.id(), &id);
    }

    #[tokio::test]
    async fn load_nonexistent_layer() {
        let store = InMemoryStore::new();
        let fake_id = LayerId([0u8; 32]);
        assert!(matches!(
            store.load_layer(&fake_id).await,
            Err(StorageError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn store_and_load_resource() {
        let store = InMemoryStore::new();
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
    }

    #[tokio::test]
    async fn list_resources() {
        let store = InMemoryStore::new();
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
        let store = InMemoryStore::new();

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
}
