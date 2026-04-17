//! Program execution via NbE with IO capability mode.
//!
//! Replaces the hand-written executor (execute.rs) with NbE evaluation
//! in IO mode. Programs are parsed to Mini-TT terms, then evaluated
//! with component dispatch, trace memoization, and resource conversion.

use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::eval::{eval_ctx, EvalCtx};
use crate::nbe::term::Patt;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::program::execute::{ComponentRegistry, ProgramError};
use crate::program::trace::TraceStore;
use std::sync::Arc;

/// Execute a program resource via NbE in IO mode.
pub fn execute_program_nbe(
    program: &Resource,
    input: &Resource,
    layer: Arc<Layer>,
    registry: Arc<ComponentRegistry>,
    trace_store: Option<Arc<dyn TraceStore>>,
) -> Result<Resource, ProgramError> {
    // Extract the program body expression
    let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
    let body = match program.get(&body_prop) {
        Some(Value::Embedded(r)) => r,
        _ => return Err(ProgramError::Parse("program has no 'body'".to_string())),
    };

    // Parse the body to a Mini-TT expression
    let body_exp =
        crate::program::expr::parse_expression(body, &layer).map_err(ProgramError::Parse)?;

    // Build the IO evaluation context
    let ctx = EvalCtx::IO {
        layer,
        registry,
        trace_store,
    };

    // Bind input as a Val::ResourceVal in the environment
    let rho = Rho::Nil.extend(
        Patt::Var("input".to_string()),
        Val::ResourceVal(Box::new(input.clone())),
    );

    // Evaluate the body expression in IO mode
    let result = eval_ctx(&body_exp, &rho, &ctx);

    // Convert the result Val back to a Resource
    val_to_resource(&result)
}

/// Convert a Val result to a Resource.
fn val_to_resource(val: &Val) -> Result<Resource, ProgramError> {
    match val {
        Val::ResourceVal(r) => Ok(r.as_ref().clone()),
        Val::Unit => Ok(Resource::new_embedded()),
        Val::Pair(a, b) => {
            let mut r = Resource::new_embedded();
            if let Val::ResourceVal(ra) = a.as_ref() {
                for (k, v) in ra.properties() {
                    r.set(k.clone(), v.clone());
                }
            }
            if let Val::ResourceVal(rb) = b.as_ref() {
                for (k, v) in rb.properties() {
                    r.set(k.clone(), v.clone());
                }
            }
            Ok(r)
        }
        _ => Ok(Resource::new_embedded()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::eigon_json;
    use crate::program::execute::ComponentRegistry;
    use crate::program::trace::InMemoryTraceStore;

    #[test]
    fn execute_identity_via_nbe() {
        let json = r#"{
            "@id": "urn:eigenius:test:prog",
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
            "urn:eigenius:program:body": {
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                "urn:eigenius:program:name": "input"
            }
        }"#;
        let program = eigon_json::parse_document(json).unwrap().remove(0);

        let mut input = Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:example:name").unwrap(),
            Value::String("Rex".into()),
        );

        let layer = Arc::new(crate::layer::LayerBuilder::new("empty", None).build());
        let registry = Arc::new(ComponentRegistry::default());

        let output = execute_program_nbe(&program, &input, layer, registry, None).unwrap();

        assert_eq!(
            output
                .get(&Iri::parse("urn:eigenius:example:name").unwrap())
                .unwrap()
                .as_str(),
            Some("Rex")
        );
    }

    #[test]
    fn execute_identity_component_via_nbe() {
        let json = r#"{
            "@id": "urn:eigenius:test:prog",
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
            "urn:eigenius:program:body": {
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
                "urn:eigenius:program:function": "urn:eigenius:components:Identity",
                "urn:eigenius:program:argument": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                    "urn:eigenius:program:name": "input"
                }
            }
        }"#;
        let program = eigon_json::parse_document(json).unwrap().remove(0);

        let mut input = Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:example:name").unwrap(),
            Value::String("Rex".into()),
        );

        let layer = Arc::new(crate::layer::LayerBuilder::new("empty", None).build());
        let registry = Arc::new(ComponentRegistry::default());
        let trace_store: Arc<dyn TraceStore> = Arc::new(InMemoryTraceStore::new());

        let output =
            execute_program_nbe(&program, &input, layer, registry, Some(trace_store)).unwrap();

        assert_eq!(
            output
                .get(&Iri::parse("urn:eigenius:example:name").unwrap())
                .unwrap()
                .as_str(),
            Some("Rex")
        );
    }
}
