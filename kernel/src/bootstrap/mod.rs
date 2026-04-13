//! Bootstrap sequence for kernel initialization.
//!
//! Loads the core ontology from `ontologies/core/core-ontology.json`,
//! creates the root layer, validates it against itself, and returns
//! a working execution context.

use crate::context::{ExecutionContext, ExecutionMode};
use crate::layer::{Layer, LayerBuilder};
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

/// Load, build, and validate a layer from embedded JSON.
fn load_layer(
    name: &str,
    json: &str,
    parent: Option<Arc<Layer>>,
) -> Result<Arc<Layer>, BootstrapError> {
    let resources = eigon_json::parse_document(json).map_err(BootstrapError::Parse)?;

    let mut builder = LayerBuilder::new(name, parent);
    for resource in resources {
        builder
            .add_resource(resource)
            .map_err(BootstrapError::Layer)?;
    }
    let layer = Arc::new(builder.build());

    let validator = Validator::new(&layer);
    let errors = validator.validate();
    if !errors.is_empty() {
        return Err(BootstrapError::CoreOntologyInvalid(errors));
    }

    Ok(layer)
}

/// Bootstrap the Eigenius kernel.
///
/// Loads three ontology layers: core → program → reflection.
/// All are validated. Returns an `ExecutionContext` with the
/// reflection layer as head.
pub fn bootstrap() -> Result<ExecutionContext, BootstrapError> {
    let core = load_layer(
        "core",
        include_str!("../../../ontologies/core/core-ontology.json"),
        None,
    )?;

    let program = load_layer(
        "program",
        include_str!("../../../ontologies/program/program-ontology.json"),
        Some(core),
    )?;

    let reflection = load_layer(
        "reflection",
        include_str!("../../../ontologies/reflection/reflection-ontology.json"),
        Some(program),
    )?;

    Ok(ExecutionContext::new(
        reflection,
        "working",
        ExecutionMode::ReadWrite,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::iri::Iri;

    #[test]
    fn bootstrap_succeeds() {
        let ctx = bootstrap().unwrap();
        // Head is the reflection layer (on top of program, on top of core)
        assert!(!ctx.head().is_root());
        // Program layer (parent of reflection)
        let program = ctx.head().parent().unwrap();
        assert!(!program.is_root());
        // Core layer (parent of program) should be root
        assert!(program.parent().unwrap().is_root());
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

    #[test]
    fn can_resolve_program_classes() {
        let ctx = bootstrap().unwrap();
        for class in [
            "Program",
            "Let",
            "Apply",
            "Var",
            "Lambda",
            "Case",
            "Branch",
            "Pair",
            "Construct",
            "Project",
            "Map",
            "Reduce",
            "Literal",
            "Component",
            "CapabilityLevel",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:program:{class}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve program class {class}"
            );
        }
    }

    #[test]
    fn can_resolve_builtin_components() {
        let ctx = bootstrap().unwrap();
        for comp in [
            "Identity",
            "CompleteText",
            "CompleteJson",
            "Combine",
            "Extract",
            "Transform",
            "HttpRequest",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:program:components:{comp}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve component {comp}"
            );
        }
    }

    #[test]
    fn can_resolve_reflection_classes() {
        let ctx = bootstrap().unwrap();
        for class in [
            "DeclaredResource",
            "ObservedResource",
            "DerivedResource",
            "VerifiedResource",
            "ComponentTrace",
            "ProgramTrace",
            "DeclarationTrace",
            "ObservationTrace",
            "VerificationTrace",
            "LetTrace",
            "MapTrace",
            "CaseTrace",
            "ConstructTrace",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:reflection:{class}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve reflection class {class}"
            );
        }
    }

    #[test]
    fn can_resolve_epistemic_statuses() {
        let ctx = bootstrap().unwrap();
        for status in ["declared", "observed", "derived", "verified"] {
            let iri = Iri::parse(&format!("urn:eigenius:reflection:epistemic:{status}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve epistemic status {status}"
            );
        }
    }
}
