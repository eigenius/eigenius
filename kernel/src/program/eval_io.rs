// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Program execution via NbE with IO capability mode.
//!
//! Replaces the hand-written executor (execute.rs) with NbE evaluation
//! in IO mode. Programs are parsed to Mini-TT terms, then evaluated
//! with component dispatch, trace memoization, and resource conversion.

use crate::institution::InstitutionRegistry;
use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::eval::{eval_traced, EvalCtx};
use crate::nbe::term::Patt;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::program::component::{ComponentRegistry, ProgramError};
use crate::program::trace::{ComponentTrace, Trace, TraceStore};
use std::sync::{Arc, Mutex};

/// Result of NbE program execution: output resource + dispatched IO traces.
pub struct NbeExecutionResult {
    pub output: Resource,
    /// ComponentTraces produced during execution (for trace layer commits).
    pub dispatched_traces: Vec<ComponentTrace>,
    /// Tree-structured trace from `eval_traced` (D6b §2).
    pub root_trace: Option<Trace>,
}

/// Execute a program resource via NbE in IO mode.
/// Returns the output resource and all IO ComponentTraces dispatched.
pub fn execute_program_nbe(
    program: &Resource,
    input: &Resource,
    layer: Arc<Layer>,
    registry: Arc<ComponentRegistry>,
    trace_store: Option<Arc<dyn TraceStore>>,
) -> Result<NbeExecutionResult, ProgramError> {
    execute_program_nbe_with_institutions(
        program,
        input,
        layer,
        registry,
        Arc::new(InstitutionRegistry::new()),
        trace_store,
        None,
    )
}

/// Execute a program resource via NbE in IO mode with institution support.
///
/// `task_context`, when present, routes IO dispatches through
/// per-task positional trace keys so the task can be resumed after a
/// crash (D21 §3.2). When `None`, the evaluator runs without task
/// tracking (type-checker, ad-hoc eval, pre-task callers).
pub fn execute_program_nbe_with_institutions(
    program: &Resource,
    input: &Resource,
    layer: Arc<Layer>,
    registry: Arc<ComponentRegistry>,
    institutions: Arc<InstitutionRegistry>,
    trace_store: Option<Arc<dyn TraceStore>>,
    task_context: Option<Arc<crate::task::TaskContext>>,
) -> Result<NbeExecutionResult, ProgramError> {
    // Extract the program body expression
    let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
    let body = match program.get(&body_prop) {
        Some(Value::Embedded(r)) => r,
        _ => return Err(ProgramError::Parse("program has no 'body'".to_string())),
    };

    // Parse the body to a Mini-TT expression
    let body_exp =
        crate::program::expr::parse_expression(body, &layer).map_err(ProgramError::Parse)?;

    // Build the IO evaluation context with trace collection
    let dispatched_traces = Arc::new(Mutex::new(Vec::new()));
    let ctx = EvalCtx::IO {
        layer,
        registry,
        institutions,
        trace_store,
        dispatched_traces: Arc::clone(&dispatched_traces),
        task_context,
    };

    // Bind input as a Val::ResourceVal in the environment
    let rho = Rho::Nil.extend(
        Patt::Var("input".to_string()),
        Val::ResourceVal(Box::new(input.clone())),
    );

    // Evaluate the body expression in IO mode with tracing.
    // Wrap in catch_unwind so remaining panics in the evaluator
    // become ProgramError::Execution instead of crashing the server
    // (Phase 10c, defence-in-depth layer 2).
    let eval_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eval_traced(&body_exp, &rho, &ctx)
    }));
    let (result, root_trace) = match eval_result {
        Ok(Ok(r)) => r,
        Ok(Err(eval_err)) => {
            return Err(ProgramError::Execution(eval_err.to_string()));
        }
        Err(e) => {
            // Defence-in-depth: should not fire with Result propagation
            let msg = if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "unknown panic during evaluation".to_string()
            };
            return Err(ProgramError::Execution(msg));
        }
    };

    // Convert the result Val back to a Resource
    let output = val_to_resource(&result)?;

    // Extract collected traces
    let traces = match Arc::try_unwrap(dispatched_traces) {
        Ok(mutex) => mutex.into_inner().unwrap_or_default(),
        Err(arc) => arc.lock().unwrap().clone(),
    };

    Ok(NbeExecutionResult {
        output,
        dispatched_traces: traces,
        root_trace,
    })
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
        _ => {
            // Lossy conversion — fire in debug builds so tests surface
            // unexpected Val types reaching the execution boundary
            // (Phase 10c, defence-in-depth layer 3).
            debug_assert!(
                false,
                "val_to_resource: lossy conversion of {:?} to empty resource",
                val
            );
            Ok(Resource::new_embedded())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::eigon_json;
    use crate::program::component::{ComponentRegistry, ProgramError};
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

        let layer = Arc::new(
            crate::layer::LayerBuilder::new("empty", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let registry = Arc::new(ComponentRegistry::default());

        let result = execute_program_nbe(&program, &input, layer, registry, None).unwrap();

        assert_eq!(
            result
                .output
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
                "urn:eigenius:program:function": "urn:eigenius:program:components:Identity",
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

        let layer = Arc::new(
            crate::layer::LayerBuilder::new("empty", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let registry = Arc::new(ComponentRegistry::default());
        let trace_store: Arc<dyn TraceStore> = Arc::new(InMemoryTraceStore::new());

        let result =
            execute_program_nbe(&program, &input, layer, registry, Some(trace_store)).unwrap();

        assert_eq!(
            result
                .output
                .get(&Iri::parse("urn:eigenius:example:name").unwrap())
                .unwrap()
                .as_str(),
            Some("Rex")
        );
    }

    #[test]
    fn execute_identity_produces_root_trace_none() {
        // A simple Var expression produces no trace (pure leaf)
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
            Iri::parse("urn:eigenius:example:x").unwrap(),
            Value::String("val".into()),
        );

        let layer = Arc::new(
            crate::layer::LayerBuilder::new("empty", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let registry = Arc::new(ComponentRegistry::default());

        let result = execute_program_nbe(&program, &input, layer, registry, None).unwrap();
        assert!(
            result.root_trace.is_none(),
            "identity (Var) should have no trace"
        );
    }

    #[test]
    fn execute_component_produces_root_trace() {
        // An Identity component dispatch should produce a Component trace
        let json = r#"{
            "@id": "urn:eigenius:test:prog",
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
            "urn:eigenius:program:body": {
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
                "urn:eigenius:program:function": "urn:eigenius:program:components:Identity",
                "urn:eigenius:program:argument": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                    "urn:eigenius:program:name": "input"
                }
            }
        }"#;
        let program = eigon_json::parse_document(json).unwrap().remove(0);

        let mut input = Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:example:x").unwrap(),
            Value::String("val".into()),
        );

        let layer = Arc::new(
            crate::layer::LayerBuilder::new("empty", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let registry = Arc::new(ComponentRegistry::default());

        let result = execute_program_nbe(&program, &input, layer, registry, None).unwrap();
        assert!(
            result.root_trace.is_some(),
            "Identity component dispatch should produce a root trace"
        );
        match result.root_trace.unwrap() {
            Trace::Component(ct) => {
                assert_eq!(ct.component, "urn:eigenius:program:components:Identity");
            }
            other => panic!("expected Trace::Component, got {:?}", other),
        }
    }

    #[test]
    fn catch_unwind_converts_panic_to_execution_error() {
        // Phase 10c: A program that triggers a remaining panic (applying a
        // non-function) should be caught by catch_unwind and returned as
        // ProgramError::Execution instead of crashing the process.
        //
        // Body: Apply(Pair(input, input), input)
        // This evaluates the function position to Val::Pair, then
        // app_ctx_traced panics with "not a function".
        let json = r#"{
            "@id": "urn:eigenius:test:prog",
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
            "urn:eigenius:program:body": {
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
                "urn:eigenius:program:function": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Pair"],
                    "urn:eigenius:program:first": {
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                        "urn:eigenius:program:name": "input"
                    },
                    "urn:eigenius:program:second": {
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                        "urn:eigenius:program:name": "input"
                    }
                },
                "urn:eigenius:program:argument": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                    "urn:eigenius:program:name": "input"
                }
            }
        }"#;
        let program = eigon_json::parse_document(json).unwrap().remove(0);

        let mut input = Resource::new_embedded();
        input.set(
            Iri::parse("urn:eigenius:example:x").unwrap(),
            Value::String("val".into()),
        );

        let layer = Arc::new(
            crate::layer::LayerBuilder::new("empty", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let registry = Arc::new(ComponentRegistry::default());

        let result = execute_program_nbe(&program, &input, layer, registry, None);
        match result {
            Err(ProgramError::Execution(msg)) => {
                assert!(
                    msg.contains("not a function"),
                    "expected 'not a function' in error, got: {msg}"
                );
            }
            Ok(_) => panic!("expected ProgramError::Execution, got Ok"),
            Err(e) => panic!("expected ProgramError::Execution, got {:?}", e),
        }
    }
}
