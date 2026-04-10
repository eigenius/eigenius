//! Core Ontology bootstrap and schema definitions.
//!
//! Phase 0: bootstrap the self-describing Core Ontology (§6).
//!
//! The Core Ontology is the foundational schema layer that describes itself.
//! It defines the Classes (Class, Property, Resource) and relationships that
//! form the basis for all higher-level ontology layers. This module provides
//! the hardcoded bootstrap definitions.

use crate::ontology::{Class, Property};

/// The bootstrapped Core Ontology providing self-describing schema.
#[derive(Debug, Clone)]
pub struct CoreOntology {
    /// Core Class definitions
    pub classes: Vec<Class>,

    /// Core Property definitions
    pub properties: Vec<Property>,
}

impl CoreOntology {
    /// Initializes the hardcoded Core Ontology bootstrap.
    ///
    /// Returns the self-describing ontological foundation with Class, Property,
    /// and Resource metaclasses and their essential properties.
    pub fn init() -> Self {
        // Placeholder: minimal bootstrap structure
        let class_class = Class::new(
            "eigenius:Class".to_string(),
            "Class".to_string(),
        );

        let property_class = Class::new(
            "eigenius:Property".to_string(),
            "Property".to_string(),
        );

        let resource_class = Class::new(
            "eigenius:Resource".to_string(),
            "Resource".to_string(),
        );

        let classes = vec![class_class, property_class, resource_class];
        let properties = Vec::new();

        CoreOntology { classes, properties }
    }

    /// Returns the Class definition matching the given URI.
    pub fn get_class(&self, uri: &str) -> Option<&Class> {
        self.classes.iter().find(|c| c.uri == uri)
    }
}
