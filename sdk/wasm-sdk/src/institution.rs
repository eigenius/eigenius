//! Institution helpers for WASM fiber reasoners.
//!
//! Provides types and builders that match the kernel's
//! `WasmFiberReasoner::parse_declaration` expectations. A guest component
//! implements `fiber-declaration` by constructing a `FiberDeclaration` and
//! serializing it via `into_resource().to_cbor()`.

use crate::{Resource, Value};

/// Declaration of a fiber reasoner's structure, matching the kernel's
/// `FiberDeclaration` struct. Serializes into a Resource with the
/// institution property names the kernel expects.
pub struct FiberDeclaration {
    pub institution_iri: String,
    pub name: String,
    pub morphism_types: Vec<Resource>,
    pub query_types: Vec<Resource>,
    pub structural_properties: Vec<Resource>,
}

impl FiberDeclaration {
    /// Convert this declaration into a CBOR-serializable Resource.
    pub fn into_resource(self) -> Resource {
        let mut r = Resource::new();
        r.set(
            "urn:eigenius:institution:institution_iri",
            Value::String(self.institution_iri),
        );
        r.set(
            "urn:eigenius:institution:institution_name",
            Value::String(self.name),
        );
        if !self.morphism_types.is_empty() {
            r.set(
                "urn:eigenius:institution:morphism_types",
                Value::Array(
                    self.morphism_types
                        .into_iter()
                        .map(|r| Value::Embedded(Box::new(r)))
                        .collect(),
                ),
            );
        }
        if !self.query_types.is_empty() {
            r.set(
                "urn:eigenius:institution:query_types",
                Value::Array(
                    self.query_types
                        .into_iter()
                        .map(|r| Value::Embedded(Box::new(r)))
                        .collect(),
                ),
            );
        }
        if !self.structural_properties.is_empty() {
            r.set(
                "urn:eigenius:institution:structural_properties",
                Value::Array(
                    self.structural_properties
                        .into_iter()
                        .map(|r| Value::Embedded(Box::new(r)))
                        .collect(),
                ),
            );
        }
        r
    }
}

/// Result of morphism validation. Matches the kernel's `MorphismValidation`
/// enum and the `validation-result` WIT enum.
#[derive(Debug, Clone, PartialEq)]
pub enum MorphismValidation {
    Valid,
    Invalid(String),
    Undecidable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fiber_declaration_roundtrips() {
        let decl = FiberDeclaration {
            institution_iri: "urn:example:inst".into(),
            name: "Example".into(),
            morphism_types: vec![],
            query_types: vec![],
            structural_properties: vec![],
        };
        let r = decl.into_resource();
        assert_eq!(
            r.get_string("urn:eigenius:institution:institution_iri"),
            Some("urn:example:inst")
        );
        assert_eq!(
            r.get_string("urn:eigenius:institution:institution_name"),
            Some("Example")
        );
    }

    #[test]
    fn fiber_declaration_with_morphism_types() {
        let mut morphism = Resource::with_id("urn:example:Refinement");
        morphism.set_is_a(["urn:eigenius:core:Class"]);
        morphism.set(
            "urn:eigenius:core:short_name",
            Value::String("Refinement".into()),
        );

        let decl = FiberDeclaration {
            institution_iri: "urn:example:inst".into(),
            name: "Example".into(),
            morphism_types: vec![morphism],
            query_types: vec![],
            structural_properties: vec![],
        };

        let r = decl.into_resource();
        let morphisms = r
            .get("urn:eigenius:institution:morphism_types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(morphisms.len(), 1);
    }
}
