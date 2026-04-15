//! Program executor: evaluate a typed program against input data.
//!
//! Each expression evaluation returns `(Resource, Option<Trace>)` where
//! the trace mirrors the expression tree (D6b §2.1). IO component calls
//! check the trace store before dispatching (memoization).

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::program::trace::{
    compute_trace_key, ComponentMetrics, ComponentTrace, Trace, TraceStore,
};
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

/// Result of executing a component: output resource plus optional metrics.
pub struct ComponentResult {
    pub output: Resource,
    pub metrics: Option<ComponentMetrics>,
}

/// A built-in component implementation.
pub trait BuiltinComponent: Send + Sync {
    /// Whether this component performs IO (non-deterministic, cacheable).
    fn is_io(&self) -> bool {
        false
    }

    /// Execute the component.
    ///
    /// - `input`: the evaluated argument expression (data flowing through the program)
    /// - `argument`: static component configuration (e.g., prompt template, model params).
    ///   Comes from `component_argument` on the Apply node. `None` if not provided.
    /// - `layer`: the current layer chain for resolution
    fn execute(
        &self,
        input: &Resource,
        argument: Option<&Resource>,
        layer: &Layer,
    ) -> Result<ComponentResult, String>;
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

/// Execute a program resource against input data, producing output and trace.
pub fn execute_program(
    program: &Resource,
    input: &Resource,
    layer: &Layer,
    registry: &ComponentRegistry,
) -> Result<Resource, ProgramError> {
    let (result, _trace) = execute_program_traced(program, input, layer, registry, None)?;
    Ok(result)
}

/// Execute a program resource with full trace recording.
///
/// Returns `(output, trace_tree, program_metrics)`.
pub fn execute_program_traced(
    program: &Resource,
    input: &Resource,
    layer: &Layer,
    registry: &ComponentRegistry,
    trace_store: Option<&dyn TraceStore>,
) -> Result<(Resource, Option<Trace>), ProgramError> {
    let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
    let body = match program.get(&body_prop) {
        Some(Value::Embedded(r)) => r,
        _ => return Err(ProgramError::Parse("program has no 'body'".to_string())),
    };

    let mut context = BTreeMap::new();
    context.insert("input".to_string(), input.clone());

    execute_expression(body, &context, layer, registry, trace_store)
}

/// Execute an expression against a variable context, returning result and trace.
fn execute_expression(
    expr: &Resource,
    context: &BTreeMap<String, Resource>,
    layer: &Layer,
    registry: &ComponentRegistry,
    trace_store: Option<&dyn TraceStore>,
) -> Result<(Resource, Option<Trace>), ProgramError> {
    let is_a = expr.is_a();
    let class_str = is_a.first().map(|i| i.as_str()).unwrap_or("");

    match class_str {
        "urn:eigenius:program:Let" => {
            let name = get_str(expr, "urn:eigenius:program:name")?;
            let value_resource = get_emb(expr, "urn:eigenius:program:value")?;
            let (value, value_trace) =
                execute_expression(&value_resource, context, layer, registry, trace_store)?;

            let mut new_context = context.clone();
            new_context.insert(name.clone(), value);

            let body_resource = get_emb(expr, "urn:eigenius:program:body")?;
            let (result, body_trace) =
                execute_expression(&body_resource, &new_context, layer, registry, trace_store)?;

            let trace = Trace::Let {
                name,
                value_trace: value_trace.map(Box::new),
                body_trace: body_trace.map(Box::new),
            };
            Ok((result, Some(trace)))
        }

        "urn:eigenius:program:Apply" => {
            let func_prop = Iri::parse("urn:eigenius:program:function").unwrap();
            let func_name = match expr.get(&func_prop) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Embedded(r)) => {
                    let _func_val = execute_expression(r, context, layer, registry, trace_store)?;
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

            // Evaluate the argument expression (data input to the component)
            let arg_prop = Iri::parse("urn:eigenius:program:argument").unwrap();
            let input = match expr.get(&arg_prop) {
                Some(Value::Embedded(r)) => {
                    execute_expression(r, context, layer, registry, trace_store)?.0
                }
                Some(Value::String(s)) => {
                    if let Some(r) = context.get(s) {
                        r.clone()
                    } else {
                        Resource::new_embedded()
                    }
                }
                _ => Resource::new_embedded(),
            };

            // Extract static component_argument (not evaluated — passed directly)
            let comp_arg_prop = Iri::parse("urn:eigenius:program:component_argument").unwrap();
            let component_argument = match expr.get(&comp_arg_prop) {
                Some(Value::Embedded(r)) => Some(r.as_ref().clone()),
                _ => None,
            };

            match registry.get(&func_name) {
                Some(component) => {
                    if component.is_io() {
                        // IO component — check trace cache first
                        let cache_key = compute_trace_key(&func_name, &input);

                        if let Some(store) = trace_store {
                            if let Some(cached) = store.get_component_trace(&cache_key) {
                                let result = cached.output.clone();
                                let trace = Trace::Component(ComponentTrace {
                                    cached: true,
                                    ..cached
                                });
                                return Ok((result, Some(trace)));
                            }
                        }

                        let comp_result = component
                            .execute(&input, component_argument.as_ref(), layer)
                            .map_err(ProgramError::Execution)?;

                        let ct = ComponentTrace {
                            component: func_name,
                            input_hash: cache_key,
                            argument_hash: None,
                            output: comp_result.output.clone(),
                            cached: false,
                            metrics: comp_result.metrics,
                        };

                        if let Some(store) = trace_store {
                            store.put_component_trace(cache_key, ct.clone());
                        }

                        Ok((comp_result.output, Some(Trace::Component(ct))))
                    } else {
                        // Pure component — no caching
                        let comp_result = component
                            .execute(&input, component_argument.as_ref(), layer)
                            .map_err(ProgramError::Execution)?;

                        let trace = Trace::Pure {
                            component: func_name,
                            output: comp_result.output.clone(),
                        };
                        Ok((comp_result.output, Some(trace)))
                    }
                }
                None => {
                    // Unknown component — return input (identity fallback)
                    Ok((input, None))
                }
            }
        }

        "urn:eigenius:program:Var" => {
            let name = get_str(expr, "urn:eigenius:program:name")?;
            let result = context
                .get(&name)
                .cloned()
                .ok_or_else(|| ProgramError::Execution(format!("unbound variable: {name}")))?;
            Ok((result, None)) // Var produces no trace
        }

        "urn:eigenius:program:Project" => {
            let inner_resource = get_emb(expr, "urn:eigenius:program:expression")?;
            let (inner, inner_trace) =
                execute_expression(&inner_resource, context, layer, registry, trace_store)?;

            let prop_iri = get_iri(expr, "urn:eigenius:program:property")?;
            match inner.get(&prop_iri) {
                Some(val) => {
                    let mut result = Resource::new_embedded();
                    result.set(prop_iri.clone(), val.clone());
                    let trace = Trace::Project {
                        source_trace: inner_trace.map(Box::new),
                        property: prop_iri,
                    };
                    Ok((result, Some(trace)))
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

            // Set is_a from the class property
            let class_prop = Iri::parse("urn:eigenius:program:class").unwrap();
            if let Some(Value::String(class_iri)) = expr.get(&class_prop) {
                result.set(
                    Iri::parse("urn:eigenius:core:is_a").unwrap(),
                    Value::Array(vec![Value::String(class_iri.clone())]),
                );
            }

            let mut field_traces = BTreeMap::new();

            for (prop_iri, val) in fields.properties() {
                let (field_val, field_trace) = match val {
                    Value::Embedded(r) => {
                        let (field_resource, ft) =
                            execute_expression(r, context, layer, registry, trace_store)?;
                        // If the expression produced a single-property resource
                        // (e.g. from Project), extract the value. Otherwise
                        // look for a urn:eigenius:program:value wrapper, or
                        // embed the whole resource.
                        let props: Vec<_> = field_resource
                            .properties()
                            .iter()
                            .filter(|(k, _)| k.as_str() != "urn:eigenius:core:is_a")
                            .collect();
                        let v = if props.len() == 1 {
                            props[0].1.clone()
                        } else {
                            Value::Embedded(Box::new(field_resource))
                        };
                        (v, ft)
                    }
                    Value::String(s) => {
                        if let Some(r) = context.get(s) {
                            (Value::Embedded(Box::new(r.clone())), None)
                        } else {
                            (val.clone(), None)
                        }
                    }
                    _ => (val.clone(), None),
                };
                result.set(prop_iri.clone(), field_val);
                field_traces.insert(prop_iri.clone(), field_trace);
            }

            let trace = Trace::Construct { field_traces };
            Ok((result, Some(trace)))
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
            Ok((result, None)) // Literal produces no trace
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
    fn execute(
        &self,
        input: &Resource,
        _argument: Option<&Resource>,
        _layer: &Layer,
    ) -> Result<ComponentResult, String> {
        Ok(ComponentResult {
            output: input.clone(),
            metrics: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::eigon_json;
    use crate::program::trace::{InMemoryTraceStore, ProgramMetrics};

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
    fn execute_identity_produces_trace() {
        let program = make_simple_program();
        let input = make_input();
        let layer = crate::layer::LayerBuilder::new("empty", None).build();
        let registry = ComponentRegistry::default();

        let (output, trace) =
            execute_program_traced(&program, &input, &layer, &registry, None).unwrap();
        let name_iri = Iri::parse("urn:eigenius:example:name").unwrap();
        assert_eq!(output.get(&name_iri).unwrap().as_str(), Some("Rex"));

        // Identity is a pure component → PureTrace
        assert!(trace.is_some());
        assert!(matches!(trace.unwrap(), Trace::Pure { .. }));
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

    #[test]
    fn execute_let_produces_let_trace() {
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

        let (_output, trace) =
            execute_program_traced(&program, &input, &layer, &registry, None).unwrap();

        // Should be LetTrace with value_trace = PureTrace and body_trace = None (Var)
        match trace.unwrap() {
            Trace::Let {
                name,
                value_trace,
                body_trace,
            } => {
                assert_eq!(name, "result");
                assert!(matches!(*value_trace.unwrap(), Trace::Pure { .. }));
                assert!(body_trace.is_none()); // Var produces no trace
            }
            other => panic!("expected LetTrace, got {other:?}"),
        }
    }

    #[test]
    fn io_component_uses_trace_cache() {
        // Create an IO component that counts invocations
        use std::sync::atomic::{AtomicU32, Ordering};

        struct CountingIoComponent {
            count: AtomicU32,
        }

        impl BuiltinComponent for CountingIoComponent {
            fn is_io(&self) -> bool {
                true
            }

            fn execute(
                &self,
                input: &Resource,
                _argument: Option<&Resource>,
                _layer: &Layer,
            ) -> Result<ComponentResult, String> {
                self.count.fetch_add(1, Ordering::SeqCst);
                Ok(ComponentResult {
                    output: input.clone(),
                    metrics: Some(ComponentMetrics {
                        provider: "test".to_string(),
                        model: "test-model".to_string(),
                        prompt_tokens: 10,
                        completion_tokens: 5,
                        latency_ms: 100,
                    }),
                })
            }
        }

        let counter = std::sync::Arc::new(CountingIoComponent {
            count: AtomicU32::new(0),
        });

        let mut registry = ComponentRegistry::new();
        registry.register(
            "urn:eigenius:test:io-comp".to_string(),
            Box::new(CountingIoComponent {
                count: AtomicU32::new(0),
            }),
        );

        // We need to use the same component instance to track count
        // So let's just verify the cache behavior via the trace store
        let store = InMemoryTraceStore::new();

        let json = r#"{
            "@id": "urn:eigenius:test:prog",
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
            "urn:eigenius:program:body": {
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
                "urn:eigenius:program:function": "urn:eigenius:test:io-comp",
                "urn:eigenius:program:argument": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                    "urn:eigenius:program:name": "input"
                }
            }
        }"#;
        let program = eigon_json::parse_document(json).unwrap().remove(0);
        let input = make_input();
        let layer = crate::layer::LayerBuilder::new("empty", None).build();

        // First execution — dispatches to component
        let (_out1, trace1) =
            execute_program_traced(&program, &input, &layer, &registry, Some(&store)).unwrap();
        match &trace1 {
            Some(Trace::Component(ct)) => {
                assert!(!ct.cached);
                assert!(ct.metrics.is_some());
            }
            _ => panic!("expected ComponentTrace"),
        }

        // Second execution — same input, should hit cache
        let (_out2, trace2) =
            execute_program_traced(&program, &input, &layer, &registry, Some(&store)).unwrap();
        match &trace2 {
            Some(Trace::Component(ct)) => {
                assert!(ct.cached, "second execution should be cached");
            }
            _ => panic!("expected ComponentTrace"),
        }

        // Verify metrics
        let metrics = ProgramMetrics::from_trace(&trace1);
        assert_eq!(metrics.executed_steps, 1);
        assert_eq!(metrics.total_tokens, 15);

        let metrics2 = ProgramMetrics::from_trace(&trace2);
        assert_eq!(metrics2.cached_steps, 1);
        assert_eq!(metrics2.executed_steps, 0);

        drop(counter); // suppress unused warning
    }
}
