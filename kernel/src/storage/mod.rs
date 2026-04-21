//! Storage interface traits for persisting layers and resources.
//!
//! Storage backends implement these traits. Phase 0 uses the in-memory
//! backend; SQLite and TiKV come in later phases.

use crate::layer::{Layer, LayerId};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use async_trait::async_trait;
use std::fmt;
use std::sync::Arc;

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

/// A persistent backend usable by the kernel server.
///
/// Combines layer storage, metadata storage (for the seed manifest from
/// D13 §4.2), and trace-store access into a single trait object the
/// kernel can carry without depending on any particular storage crate.
/// The sync-flavored head/chain methods are used at boot, so we keep
/// them synchronous rather than going async-within-async.
pub trait PersistentBackend: Send + Sync + 'static {
    /// Read the current head layer ID, if any.
    fn get_head(&self) -> Result<Option<LayerId>, StorageError>;

    /// Write the current head layer ID atomically.
    fn set_head(&self, id: &LayerId) -> Result<(), StorageError>;

    /// Reconstruct the full layer chain from the persisted head.
    fn load_chain(&self) -> Result<Option<Arc<Layer>>, StorageError>;

    /// Store a layer (metadata + resources + chain pointer). Idempotent
    /// by layer id (content-addressed).
    fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError>;

    /// Generic metadata key-value store. Used for the seed manifest
    /// (D13 §4.2) and for future configuration that shouldn't live in
    /// an Eigon resource.
    fn get_meta(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError>;

    /// Store a metadata value at `key`.
    fn put_meta(&self, key: &str, value: &[u8]) -> Result<(), StorageError>;

    /// Borrow the trace store view of this backend. Lets the server
    /// route `ComponentTrace` reads/writes through the same storage.
    fn as_trace_store(&self) -> &(dyn crate::program::trace::TraceStore + Send + Sync);
}
