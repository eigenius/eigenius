//! Storage interface traits for persisting layers and resources.
//!
//! Storage backends implement these traits. Phase 0 uses the in-memory
//! backend; SQLite and TiKV come in later phases.

use crate::layer::{Layer, LayerId};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use async_trait::async_trait;
use std::fmt;

/// Errors from storage operations.
#[derive(Debug)]
pub enum StorageError {
    NotFound(String),
    Internal(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::NotFound(msg) => write!(f, "not found: {msg}"),
            StorageError::Internal(msg) => write!(f, "storage error: {msg}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// Trait for storing and retrieving committed layers.
#[async_trait]
pub trait LayerStore: Send + Sync {
    /// Store a committed layer.
    async fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError>;

    /// Load a layer by its content-addressed ID.
    async fn load_layer(&self, id: &LayerId) -> Result<Layer, StorageError>;

    /// List all stored layer IDs.
    async fn list_layers(&self) -> Result<Vec<LayerId>, StorageError>;
}

/// Trait for storing and retrieving individual resources within a layer.
#[async_trait]
pub trait ResourceStore: Send + Sync {
    /// Store a resource associated with a layer.
    async fn store_resource(
        &self,
        layer_id: &LayerId,
        resource: &Resource,
    ) -> Result<(), StorageError>;

    /// Load a resource by IRI within a layer.
    async fn load_resource(
        &self,
        layer_id: &LayerId,
        iri: &Iri,
    ) -> Result<Option<Resource>, StorageError>;

    /// List all resource IRIs in a layer.
    async fn list_resources(&self, layer_id: &LayerId) -> Result<Vec<Iri>, StorageError>;
}
