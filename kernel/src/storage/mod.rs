//! Storage layer abstractions for persistent ontology and capability data.
//!
//! Storage traits (§10.6) define async interfaces for persisting layers,
//! capabilities, and binary blobs. Implementations may target databases,
//! filesystems, or cloud storage backends.

use async_trait::async_trait;
use crate::layer::Layer;
use crate::ontology::Resource;

/// Trait for storing and retrieving ontological layers.
#[async_trait]
pub trait LayerStore: Send + Sync {
    /// Loads a layer by identifier.
    async fn load_layer(&self, id: &str) -> Result<Layer, String>;

    /// Stores or updates a layer.
    async fn store_layer(&self, layer: Layer) -> Result<(), String>;

    /// Deletes a layer by identifier.
    async fn delete_layer(&self, id: &str) -> Result<(), String>;

    /// Lists all available layer identifiers.
    async fn list_layers(&self) -> Result<Vec<String>, String>;
}

/// Trait for storing and retrieving capability metadata and code.
#[async_trait]
pub trait CapabilityStore: Send + Sync {
    /// Loads capability metadata and code by ID.
    async fn load_capability(&self, id: &str) -> Result<Vec<u8>, String>;

    /// Stores capability code.
    async fn store_capability(&self, id: String, code: Vec<u8>) -> Result<(), String>;

    /// Deletes a capability by ID.
    async fn delete_capability(&self, id: &str) -> Result<(), String>;

    /// Lists all registered capability IDs.
    async fn list_capabilities(&self) -> Result<Vec<String>, String>;
}

/// Trait for storing and retrieving opaque binary blobs.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Loads a blob by URI.
    async fn load_blob(&self, uri: &str) -> Result<Vec<u8>, String>;

    /// Stores a blob and returns its URI.
    async fn store_blob(&self, data: Vec<u8>) -> Result<String, String>;

    /// Deletes a blob by URI.
    async fn delete_blob(&self, uri: &str) -> Result<(), String>;

    /// Checks if a blob exists.
    async fn blob_exists(&self, uri: &str) -> Result<bool, String>;
}

/// Trait for querying and storing ontological resources.
#[async_trait]
pub trait ResourceStore: Send + Sync {
    /// Loads a resource by URI.
    async fn load_resource(&self, uri: &str) -> Result<Resource, String>;

    /// Stores or updates a resource.
    async fn store_resource(&self, resource: Resource) -> Result<(), String>;

    /// Queries resources by class.
    async fn query_by_class(&self, class_uri: &str) -> Result<Vec<Resource>, String>;
}
