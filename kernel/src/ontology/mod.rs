//! Core Ontology — Eigon structural type system
//!
//! Implements the three primitive kinds (Class, Property, Datatype),
//! resource representation, URI identity, and structural validation.
//! The Core Ontology is self-describing: Class is an instance of Class.
//!
//! Architecture reference: §3 (Core Ontology)

mod class;
mod property;
mod datatype;
mod resource;
mod uri;
mod core_ontology;

pub use class::Class;
pub use property::Property;
pub use datatype::Datatype;
pub use resource::Resource;
pub use uri::Uri;
pub use core_ontology::CoreOntology;
