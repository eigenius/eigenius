//! Program executor: evaluate a typed program against input data.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use std::collections::BTreeMap;
use std::fmt;

/// Errors during program execution.
#[derive(Debug)]
pub enum ProgramError {
    Parse(String),
    TypeCheck(String),
    Execution(String),
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgramError::Parse(msg) => write!(f, "parse error: {msg}"),
            ProgramError::TypeCheck(msg) => write!(f, "type error: {msg}"),
            ProgramError::Execution(msg) => write!(f, "execution error: {msg}"),
        }
    }
}

impl std::error::Error for ProgramError {}

/// A built-in component implementation.
pub trait BuiltinComponent: Send + Sync {
    fn execute(&self, input: &Resource, layer: &Layer) -> Result<Resource, String>;
}

/// Registry of built-in components.
pub struct ComponentRegistry {
    components: BTreeMap<String, Box<dyn BuiltinComponent>>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            components: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, name: String, component: Box<dyn BuiltinComponent>) {
        self.components.insert(name, component);
    }

    pub fn get(&self, name: &str) -> Option<&dyn BuiltinComponent> {
        self.components.get(name).map(|b| b.as_ref())
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(
            "urn:eigenius:components:Identity".to_string(),
            Box::new(IdentityComponent),
        );
        registry
    }
}

/// Execute a program resource against input data.
pub fn execute_program(
    program: &Resource,
    input: &Resource,
    layer: &Layer,
    registry: &ComponentRegistry,
) -> Result<Resource, ProgramError> {
    // Parse the program body
    let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
    let body = match program.get(&body_prop) {
        Some(Value::Embedded(r)) => r,
        _ => return Err(ProgramError::Parse("program has no 'body'".to_string())),
    };

    // Execute with input bound in context
    let mut context = BTreeMap::new();
    context.insert("input".to_string(), input.clone());

    execute_expression(body, &context, layer, registry)
}

/// Execute an expression against a variable context.
fn execute_expression(
    expr: &Resource,
    context: &BTreeMap<String, Resource>,
    layer: &Layer,
    registry: &ComponentRegistry,
) -> Result<Resource, ProgramError> {
    let is_a = expr.is_a();
    let class_str = is_a.first().map(|i| i.as_str()).unwrap_or("");

    match class_str {
        "urn:eigenius:program:Let" => {
            let name = get_str(expr, "urn:eigenius:program:name")?;
            let value_resource = get_emb(expr, "urn:eigenius:program:value")?;
            let value = execute_expression(&value_resource, context, layer, registry)?;

            let mut new_context = context.clone();
            new_context.insert(name, value);

            let body_resource = get_emb(expr, "urn:eigenius:program:body")?;
            execute_expression(&body_resource, &new_context, layer, registry)
        }

        "urn:eigenius:program:Apply" => {
            let func_prop = Iri::parse("urn:eigenius:program:function").unwrap();
            let func_name = match expr.get(&func_prop) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Embedded(r)) => {
                    // Function is an expression — execute it
                    let _func_val = execute_expression(r, context, layer, registry)?;
                    return Err(ProgramError::Execution(
                        "higher-order function execution not yet supported".to_string(),
                    ));
                }
                _ => {
                    return Err(ProgramError::Execution(
                        "Apply: missing function".to_string(),
                    ))
                }
            };

            // Build input from argument
            let arg_prop = Iri::parse("urn:eigenius:program:argument").unwrap();
            let input = match expr.get(&arg_prop) {
                Some(Value::Embedded(r)) => execute_expression(r, context, layer, registry)?,
                Some(Value::String(s)) => {
                    // Variable reference or resource IRI
                    if let Some(r) = context.get(s) {
                        r.clone()
                    } else {
                        Resource::new_embedded() // Empty input
                    }
                }
                _ => Resource::new_embedded(),
            };

            // Dispatch to component
            match registry.get(&func_name) {
                Some(component) => component
                    .execute(&input, layer)
                    .map_err(ProgramError::Execution),
                None => {
                    // Unknown component — return input (identity fallback)
                    Ok(input)
                }
            }
        }

        "urn:eigenius:program:Var" => {
            let name = get_str(expr, "urn:eigenius:program:name")?;
            context
                .get(&name)
                .cloned()
                .ok_or_else(|| ProgramError::Execution(format!("unbound variable: {name}")))
        }

        "urn:eigenius:program:Project" => {
            let inner_resource = get_emb(expr, "urn:eigenius:program:expression")?;
            let inner = execute_expression(&inner_resource, context, layer, registry)?;

            let prop_iri = get_iri(expr, "urn:eigenius:program:property")?;
            match inner.get(&prop_iri) {
                Some(val) => {
                    let mut result = Resource::new_embedded();
                    result.set(prop_iri, val.clone());
                    Ok(result)
                }
                None => Err(ProgramError::Execution(format!(
                    "property {} not found",
                    prop_iri
                ))),
            }
        }

        "urn:eigenius:program:Construct" => {
            let fields_prop = Iri::parse("urn:eigenius:program:fields").unwrap();
            let fields = match expr.get(&fields_prop) {
                Some(Value::Embedded(r)) => r,
                _ => {
                    return Err(ProgramError::Execution(
                        "Construct: missing fields".to_string(),
                    ))
                }
            };

            let mut result = Resource::new_embedded();
            for (prop_iri, val) in fields.properties() {
                let field_val = match val {
                    Value::Embedded(r) => {
                        let field_resource = execute_expression(r, context, layer, registry)?;
                        // Extract the value from the field resource
                        if let Some((_, v)) = field_resource.properties().iter().next() {
                            v.clone()
                        } else {
                            Value::Embedded(Box::new(field_resource))
                        }
                    }
                    Value::String(s) => {
                        if let Some(r) = context.get(s) {
                            Value::Embedded(Box::new(r.clone()))
                        } else {
                            val.clone()
                        }
                    }
                    _ => val.clone(),
                };
                result.set(prop_iri.clone(), field_val);
            }
            Ok(result)
        }

        "urn:eigenius:program:Literal" => {
            let val_prop = Iri::parse("urn:eigenius:program:value").unwrap();
            let mut result = Resource::new_embedded();
            if let Some(val) = expr.get(&val_prop) {
                result.set(
                    Iri::parse("urn:eigenius:program:value").unwrap(),
                    val.clone(),
                );
            }
            Ok(result)
        }

        _ => Err(ProgramError::Execution(format!(
            "unknown expression class: '{class_str}'"
        ))),
    }
}

fn get_str(resource: &Resource, prop: &str) -> Result<String, ProgramError> {
    let prop_iri = Iri::parse(prop).unwrap();
    match resource.get(&prop_iri) {
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(ProgramError::Parse(format!("missing '{prop}'"))),
    }
}

fn get_emb(resource: &Resource, prop: &str) -> Result<Resource, ProgramError> {
    let prop_iri = Iri::parse(prop).unwrap();
    match resource.get(&prop_iri) {
        Some(Value::Embedded(r)) => Ok(r.as_ref().clone()),
        _ => Err(ProgramError::Parse(format!("missing embedded at '{prop}'"))),
    }
}

fn get_iri(resource: &Resource, prop: &str) -> Result<Iri, ProgramError> {
    let s = get_str(resource, prop)?;
    Iri::parse(&s).map_err(|e| ProgramError::Parse(format!("invalid IRI: {e}")))
}

// --- Built-in components ---

struct IdentityComponent;

impl BuiltinComponent for IdentityComponent {
    fn execute(&self, input: &Resource, _layer: &Layer) -> Result<Resource, String> {
        Ok(input.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::eigon_json;

    fn make_simple_program() -> Resource {
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
        eigon_json::parse_document(json).unwrap().remove(0)
    }

    fn make_input() -> Resource {
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:example:name").unwrap(),
            Value::String("Rex".into()),
        );
        r
    }

    #[test]
    fn execute_identity_program() {
        let program = make_simple_program();
        let input = make_input();
        let layer = crate::layer::LayerBuilder::new("empty", None).build();
        let registry = ComponentRegistry::default();

        let output = execute_program(&program, &input, &layer, &registry).unwrap();
        let name_iri = Iri::parse("urn:eigenius:example:name").unwrap();
        assert_eq!(output.get(&name_iri).unwrap().as_str(), Some("Rex"));
    }

    #[test]
    fn execute_let_binding() {
        let json = r#"{
            "@id": "urn:eigenius:test:prog",
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
            "urn:eigenius:program:body": {
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Let"],
                "urn:eigenius:program:name": "result",
                "urn:eigenius:program:type": "urn:eigenius:core:string",
                "urn:eigenius:program:value": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
                    "urn:eigenius:program:function": "urn:eigenius:components:Identity",
                    "urn:eigenius:program:argument": {
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                        "urn:eigenius:program:name": "input"
                    }
                },
                "urn:eigenius:program:body": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                    "urn:eigenius:program:name": "result"
                }
            }
        }"#;
        let program = eigon_json::parse_document(json).unwrap().remove(0);
        let input = make_input();
        let layer = crate::layer::LayerBuilder::new("empty", None).build();
        let registry = ComponentRegistry::default();

        let output = execute_program(&program, &input, &layer, &registry).unwrap();
        let name_iri = Iri::parse("urn:eigenius:example:name").unwrap();
        assert_eq!(output.get(&name_iri).unwrap().as_str(), Some("Rex"));
    }
}
