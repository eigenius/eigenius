//! Resource entity representing concrete instances in the ontology.
//!
//! Part of the Eigenius Ontology Layer (§6). Resources are concrete instances
//! of Classes, with a URI identity, class membership, and property values
//! stored in a flexible map structure.

use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// A Resource represents a concrete instance of a Class.
///
/// Resources embody the actual data objects in the system, each belonging
/// to a Class and carrying property values conforming to that Class's schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    /// Unique identifier for this resource in URI format
    pub uri: String,

    /// URI of the Class that this resource instantiates
    pub class_uri: String,

    /// Property values indexed by property URI
    pub properties: HashMap<String, serde_json::Value>,
}

impl Resource {
    /// Creates a new Resource instance.
    pub fn new(uri: String, class_uri: String) -> Self {
        Self {
            uri,
            class_uri,
            properties: HashMap::new(),
        }
    }

    /// Sets a property value on this resource.
    pub fn set_property(&mut self, property_uri: String, value: serde_json::Value) {
        self.properties.insert(property_uri, value);
    }

    /// Retrieves a property value from this resource.
    pub fn get_property(&self, property_uri: &str) -> Option<&serde_json::Value> {
        self.properties.get(property_uri)
    }
}
