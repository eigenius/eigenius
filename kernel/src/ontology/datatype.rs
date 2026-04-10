//! Datatype enum representing primitive and complex value types.
//!
//! Part of the Eigenius Ontology Layer (§6). Datatypes define the range
//! of acceptable values for properties, from primitives (String, Integer)
//! to complex types (Resource, Blob).

use serde::{Serialize, Deserialize};

/// Datatype enumeration for property values.
///
/// Represents the core set of value types recognized by the ontology system,
/// supporting both primitive types and references to structured data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Datatype {
    /// UTF-8 encoded text value
    String,

    /// 64-bit signed integer
    Integer,

    /// 64-bit IEEE 754 floating-point number
    Float,

    /// Boolean true/false value
    Boolean,

    /// ISO 8601 datetime with timezone
    DateTime,

    /// Reference to another Resource via URI
    Resource,

    /// Opaque binary large object (BLOB)
    Blob,
}

impl Datatype {
    /// Returns the URI identifier for this datatype.
    pub fn uri(&self) -> &'static str {
        match self {
            Datatype::String => "eigenius:String",
            Datatype::Integer => "eigenius:Integer",
            Datatype::Float => "eigenius:Float",
            Datatype::Boolean => "eigenius:Boolean",
            Datatype::DateTime => "eigenius:DateTime",
            Datatype::Resource => "eigenius:Resource",
            Datatype::Blob => "eigenius:Blob",
        }
    }
}
