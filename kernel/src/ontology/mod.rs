//! Core Ontology — Eigon data model
//!
//! Everything in Eigenius is a Resource. Classes, properties, data types,
//! formats, and instance data are all represented as Resources with IRI
//! identity and typed property values.
//!
//! The core ontology is loaded from `ontologies/core/core-ontology.json`
//! and forms the root layer of the layer chain.

pub mod eigon_json;
pub mod iri;
pub mod resource;
pub mod well_known;

pub use iri::Iri;
pub use resource::{Resource, Value};
