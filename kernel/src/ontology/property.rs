//! Property entity representing attribute definitions on Classes.
//!
//! Part of the Eigenius Ontology Layer (§6). Properties define the attributes
//! that instances of a Class may or must possess, with type constraints (domain/range)
//! and cardinality constraints (required/multiple).

use serde::{Serialize, Deserialize};

/// A Property represents an attribute or relation defined on a Class.
///
/// Properties capture the schema constraints for instance attributes,
/// including domain (owning class), range (value type), and cardinality.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Property {
    /// Unique identifier for this property in URI format
    pub uri: String,

    /// Human-readable label for this property
    pub label: String,

    /// URI of the class that defines this property
    pub domain: String,

    /// URI or datatype identifier for values of this property
    pub range: String,

    /// Whether instances must provide a value for this property
    pub required: bool,

    /// Whether this property can hold multiple values (array-like)
    pub multiple: bool,
}

impl Property {
    /// Creates a new Property definition.
    pub fn new(uri: String, label: String, domain: String, range: String) -> Self {
        Self {
            uri,
            label,
            domain,
            range,
            required: false,
            multiple: false,
        }
    }
}
