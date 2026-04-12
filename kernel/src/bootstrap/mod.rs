//! Bootstrap sequence for kernel initialization.
//!
//! Loads the core ontology from `ontologies/core/core-ontology.json`,
//! creates the root layer, validates it against itself, and returns
//! a working execution context.

use crate::context::{ExecutionContext, ExecutionMode};
use crate::layer::LayerBuilder;
use crate::ontology::eigon_json;
use crate::validation::Validator;
use std::fmt;
use std::sync::Arc;

/// Errors during bootstrap.
#[derive(Debug)]
pub enum BootstrapError {
    Parse(eigon_json::ParseError),
    Layer(crate::layer::LayerError),
    CoreOntologyInvalid(Vec<crate::validation::ValidationError>),
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootstrapError::Parse(e) => write!(f, "failed to parse core ontology: {e}"),
            BootstrapError::Layer(e) => write!(f, "failed to build core layer: {e}"),
            BootstrapError::CoreOntologyInvalid(errors) => {
                writeln!(
                    f,
                    "core ontology validation failed with {} error(s):",
                    errors.len()
                )?;
                for e in errors {
                    writeln!(f, "  {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for BootstrapError {}

/// Bootstrap the Eigenius kernel.
///
/// 1. Parse the core ontology from the embedded JSON
/// 2. Build the root layer (no parent)
/// 3. Validate the core ontology against itself
/// 4. Return an `ExecutionContext` with the core layer as head
pub fn bootstrap() -> Result<ExecutionContext, BootstrapError> {
    // 1. Parse core ontology from embedded JSON
    let core_json = include_str!("../../../ontologies/core/core-ontology.json");
    let resources = eigon_json::parse_document(core_json).map_err(BootstrapError::Parse)?;

    // 2. Build the core layer (root — no parent)
    let mut builder = LayerBuilder::new("core", None);
    for resource in resources {
        builder
            .add_resource(resource)
            .map_err(BootstrapError::Layer)?;
    }
    let core_layer = Arc::new(builder.build());

    // 3. Validate the core layer against itself
    let validator = Validator::new(&core_layer);
    let errors = validator.validate();
    if !errors.is_empty() {
        return Err(BootstrapError::CoreOntologyInvalid(errors));
    }

    // 4. Create a working context with the core layer as head
    let ctx = ExecutionContext::new(core_layer, "working", ExecutionMode::ReadWrite);

    Ok(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::iri::Iri;

    #[test]
    fn bootstrap_succeeds() {
        let ctx = bootstrap().unwrap();
        // Core layer should be root
        assert!(ctx.head().is_root());
    }

    #[test]
    fn can_resolve_core_resources() {
        let ctx = bootstrap().unwrap();
        let class_iri = Iri::parse("urn:eigenius:core:Class").unwrap();
        let resolved = ctx.resolve(&class_iri);
        assert!(
            resolved.is_some(),
            "should resolve Class from core ontology"
        );
    }

    #[test]
    fn can_resolve_all_core_classes() {
        let ctx = bootstrap().unwrap();
        for class_name in [
            "Class",
            "Property",
            "DataType",
            "Format",
            "Encoding",
            "ConditionalRequirement",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{class_name}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve core class {class_name}"
            );
        }
    }

    #[test]
    fn can_resolve_core_properties() {
        let ctx = bootstrap().unwrap();
        for prop in [
            "is_a",
            "description",
            "short_name",
            "data_type",
            "requires",
            "recommends",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{prop}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve core property {prop}"
            );
        }
    }

    #[test]
    fn can_resolve_data_types() {
        let ctx = bootstrap().unwrap();
        for dt in [
            "string",
            "integer",
            "float",
            "boolean",
            "resource",
            "resource_array",
            "value_array",
            "json",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{dt}")).unwrap();
            assert!(ctx.resolve(&iri).is_some(), "should resolve data type {dt}");
        }
    }

    #[test]
    fn can_resolve_formats() {
        let ctx = bootstrap().unwrap();
        for fmt in ["date", "datetime", "time", "iri", "uuid", "regex"] {
            let iri = Iri::parse(&format!("urn:eigenius:core:formats:{fmt}")).unwrap();
            assert!(ctx.resolve(&iri).is_some(), "should resolve format {fmt}");
        }
    }
}
