//! Class entity representing an ontological type definition.
//!
//! Part of the Eigenius Ontology Layer (§6). Classes form the core of the self-describing
//! Core Ontology, enabling representation of resources, properties, and relationships.
//! Each Class defines a schema with a URI, human-readable label, parent class hierarchy,
//! and associated properties.

use serde::{Serialize, Deserialize};

/// A Class represents an ontological type with schema information.
///
/// Corresponds to OWL/RDF Class concepts, enabling structured representation
/// of resource types in the Core Ontology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Class {
    /// Unique identifier for this class in URI format
    pub uri: String,

    /// Human-readable label for this class
    pub label: String,

    /// URIs of parent classes in the inheritance hierarchy
    pub parent_classes: Vec<String>,

    /// URIs of properties defined on this class
    pub properties: Vec<String>,
}

impl Class {
    /// Creates a new Class definition.
    pub fn new(uri: String, label: String) -> Self {
        Self {
            uri,
            label,
            parent_classes: Vec::new(),
            properties: Vec::new(),
        }
    }
}
