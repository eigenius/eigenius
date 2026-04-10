//! In-memory storage backend for testing and development.
//!
//! Architecture §10.7

use std::collections::HashMap;
use std::sync::Arc;

/// In-memory key-value store backed by a HashMap.
///
/// Suitable for testing, development, and small deployments.
#[derive(Clone)]
#[allow(dead_code)]
pub struct InMemoryStore {
    data: Arc<tokio::sync::RwLock<HashMap<String, Vec<u8>>>>,
}

impl InMemoryStore {
    /// Create a new empty in-memory store.
    pub fn new() -> Self {
        Self {
            data: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Get a value by key.
    pub async fn get(&self, _key: &str) -> Option<Vec<u8>> {
        todo!()
    }

    /// Set a key-value pair.
    pub async fn set(&self, _key: String, _value: Vec<u8>) -> Result<(), String> {
        todo!()
    }

    /// Delete a key.
    pub async fn delete(&self, _key: &str) -> Result<(), String> {
        todo!()
    }

    /// Scan all keys with a prefix.
    pub async fn scan_prefix(&self, _prefix: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
        todo!()
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}
