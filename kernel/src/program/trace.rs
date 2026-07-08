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

//! Trace types and TraceStore trait for program execution tracing.
//!
//! Each expression evaluation returns `(Resource, Option<Trace>)`.
//! The trace tree mirrors the expression tree (D6b §2).
//! Only ComponentTraces participate in memoization — they are the
//! atomic unit of IO computation with content-addressed keys.

use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use std::collections::BTreeMap;

/// A trace node produced during expression evaluation.
///
/// Mirrors the expression types from D3/D6b. Pure leaf expressions
/// (Var, Literal) produce no trace — they involve no computation.
#[derive(Debug, Clone)]
pub enum Trace {
    /// Trace of a Let binding: name, value trace, body trace.
    Let {
        name: String,
        value_trace: Option<Box<Trace>>,
        body_trace: Option<Box<Trace>>,
    },
    /// Trace of an IO component invocation (memoization cache unit).
    Component(ComponentTrace),
    /// Trace of a pure (non-IO) component invocation.
    Pure { component: String, output: Resource },
    /// Trace of a comorphism dispatch (D14 §9.3 four-step pipeline).
    ///
    /// Records the structural fact that the program ran a comorphism:
    /// which one (`comorphism_iri`), the trace of the source
    /// expression evaluation (`source_trace`), and the chain IRI the
    /// kernel committed the produced target-class resource at
    /// (`target_iri`, `target_class` — D14 §9.3 step 4 chain
    /// reinsertion). Substrate-side per-step provenance
    /// (extract/reify timestamps, image_digest, dispatched_to) lives
    /// in the chain-resident `RuntimeInvocation` (D31 §6.2),
    /// referenced from the audit chain via the produced resource's
    /// `derivation` link rather than carried inline here.
    Comorphism {
        comorphism_iri: String,
        source_trace: Option<Box<Trace>>,
        target_iri: String,
        target_class: String,
    },
    /// Trace of a Map over a collection.
    Map { element_traces: Vec<Option<Trace>> },
    /// Trace of a Reduce (fold).
    Reduce { step_traces: Vec<Option<Trace>> },
    /// Trace of a Case expression.
    Case {
        scrutinee_trace: Option<Box<Trace>>,
        branch_taken: String,
        branch_trace: Option<Box<Trace>>,
    },
    /// Trace of a Construct expression.
    Construct {
        field_traces: BTreeMap<Iri, Option<Trace>>,
    },
    /// Trace of a property projection.
    Project {
        source_trace: Option<Box<Trace>>,
        property: Iri,
    },
    /// Sequence of sibling traces from one structural expression whose
    /// children carried more than one effectful sub-computation (e.g.
    /// a `Pair` whose both components dispatched components, or the
    /// two curried applications of one `Reduce` step). Introduced for
    /// trace-tree completeness (F-5, NbE analysis §3.2) — before it,
    /// multi-child structural nodes silently dropped all but one
    /// child's trace.
    Seq(Vec<Trace>),
}

/// Trace of an IO component invocation — the memoization cache unit.
#[derive(Debug, Clone)]
pub struct ComponentTrace {
    /// Component IRI.
    pub component: String,
    /// SHA-256 of CBOR-canonicalized input.
    pub input_hash: [u8; 32],
    /// SHA-256 of CBOR-canonicalized argument (if any).
    pub argument_hash: Option<[u8; 32]>,
    /// The output resource.
    pub output: Resource,
    /// Whether this result was served from cache.
    pub cached: bool,
    /// LLM metrics (optional).
    pub metrics: Option<ComponentMetrics>,
}

/// LLM metrics recorded in a ComponentTrace.
#[derive(Debug, Clone)]
pub struct ComponentMetrics {
    pub provider: String,
    pub model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub latency_ms: i64,
}

/// Aggregate metrics for a ProgramTrace.
#[derive(Debug, Clone, Default)]
pub struct ProgramMetrics {
    pub total_tokens: i64,
    pub total_latency_ms: i64,
    pub cached_steps: i64,
    pub executed_steps: i64,
}

impl ProgramMetrics {
    /// Walk the trace tree and accumulate metrics.
    pub fn from_trace(trace: &Option<Trace>) -> Self {
        let mut metrics = ProgramMetrics::default();
        if let Some(t) = trace {
            metrics.accumulate(t);
        }
        metrics
    }

    fn accumulate(&mut self, trace: &Trace) {
        match trace {
            Trace::Component(ct) => {
                if ct.cached {
                    self.cached_steps += 1;
                } else {
                    self.executed_steps += 1;
                }
                if let Some(m) = &ct.metrics {
                    self.total_tokens += m.prompt_tokens + m.completion_tokens;
                    self.total_latency_ms += m.latency_ms;
                }
            }
            Trace::Pure { .. } => {
                self.executed_steps += 1;
            }
            Trace::Comorphism { source_trace, .. } => {
                self.executed_steps += 1;
                if let Some(t) = source_trace {
                    self.accumulate(t);
                }
            }
            Trace::Let {
                value_trace,
                body_trace,
                ..
            } => {
                if let Some(t) = value_trace {
                    self.accumulate(t);
                }
                if let Some(t) = body_trace {
                    self.accumulate(t);
                }
            }
            Trace::Map { element_traces } => {
                for t in element_traces.iter().flatten() {
                    self.accumulate(t);
                }
            }
            Trace::Reduce { step_traces } => {
                for t in step_traces.iter().flatten() {
                    self.accumulate(t);
                }
            }
            Trace::Case {
                scrutinee_trace,
                branch_trace,
                ..
            } => {
                if let Some(t) = scrutinee_trace {
                    self.accumulate(t);
                }
                if let Some(t) = branch_trace {
                    self.accumulate(t);
                }
            }
            Trace::Construct { field_traces } => {
                for t in field_traces.values().flatten() {
                    self.accumulate(t);
                }
            }
            Trace::Project { source_trace, .. } => {
                if let Some(t) = source_trace {
                    self.accumulate(t);
                }
            }
            Trace::Seq(children) => {
                for t in children {
                    self.accumulate(t);
                }
            }
        }
    }
}

/// Trait for trace memoization storage.
///
/// Only ComponentTraces are stored — they are the IO boundary.
/// The key is SHA-256(component_iri || CBOR(input) || CBOR(argument)).
pub trait TraceStore: Send + Sync {
    /// Look up a cached ComponentTrace by content-addressed key.
    fn get_component_trace(&self, key: &[u8; 32]) -> Option<ComponentTrace>;
    /// Store a ComponentTrace by content-addressed key.
    fn put_component_trace(&self, key: [u8; 32], trace: ComponentTrace);
}

/// In-memory trace store for testing.
pub struct InMemoryTraceStore {
    traces: std::sync::RwLock<BTreeMap<[u8; 32], ComponentTrace>>,
}

impl InMemoryTraceStore {
    pub fn new() -> Self {
        Self {
            traces: std::sync::RwLock::new(BTreeMap::new()),
        }
    }
}

impl Default for InMemoryTraceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceStore for InMemoryTraceStore {
    fn get_component_trace(&self, key: &[u8; 32]) -> Option<ComponentTrace> {
        self.traces.read().unwrap().get(key).cloned()
    }

    fn put_component_trace(&self, key: [u8; 32], trace: ComponentTrace) {
        self.traces.write().unwrap().insert(key, trace);
    }
}

/// Typed placeholder for a positional trace slot with no computation
/// (a pure Map element, Reduce step, or Construct field). Class-typed
/// as `reflection:EmptyTrace` so trace-child properties can be
/// constrained to `reflection:Trace` without admitting untyped
/// embedded resources.
fn empty_trace_resource() -> Resource {
    let mut r = Resource::new_embedded();
    set_is_a(&mut r, "urn:eigenius:reflection:EmptyTrace");
    r
}

/// Convert a Trace tree into an Eigon Resource (for storage/serialization).
pub fn trace_to_resource(trace: &Trace) -> Resource {
    match trace {
        Trace::Let {
            name,
            value_trace,
            body_trace,
        } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:LetTrace");
            r.set(
                Iri::parse("urn:eigenius:reflection:name").unwrap(),
                Value::String(name.clone()),
            );
            if let Some(vt) = value_trace {
                r.set(
                    Iri::parse("urn:eigenius:reflection:value_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(vt))),
                );
            }
            if let Some(bt) = body_trace {
                r.set(
                    Iri::parse("urn:eigenius:reflection:body_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(bt))),
                );
            }
            r
        }
        Trace::Component(ct) => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:ComponentTrace");
            r.set(
                Iri::parse("urn:eigenius:reflection:component").unwrap(),
                Value::String(ct.component.clone()),
            );
            r.set(
                Iri::parse("urn:eigenius:reflection:input_hash").unwrap(),
                Value::String(hex::encode(ct.input_hash)),
            );
            if let Some(ah) = &ct.argument_hash {
                r.set(
                    Iri::parse("urn:eigenius:reflection:argument_hash").unwrap(),
                    Value::String(hex::encode(ah)),
                );
            }
            r.set(
                Iri::parse("urn:eigenius:reflection:output").unwrap(),
                Value::Embedded(Box::new(ct.output.clone())),
            );
            r.set(
                Iri::parse("urn:eigenius:reflection:cached").unwrap(),
                Value::Boolean(ct.cached),
            );
            if let Some(m) = &ct.metrics {
                r.set(
                    Iri::parse("urn:eigenius:reflection:provider").unwrap(),
                    Value::String(m.provider.clone()),
                );
                r.set(
                    Iri::parse("urn:eigenius:reflection:model").unwrap(),
                    Value::String(m.model.clone()),
                );
                r.set(
                    Iri::parse("urn:eigenius:reflection:prompt_tokens").unwrap(),
                    Value::Integer(m.prompt_tokens),
                );
                r.set(
                    Iri::parse("urn:eigenius:reflection:completion_tokens").unwrap(),
                    Value::Integer(m.completion_tokens),
                );
                r.set(
                    Iri::parse("urn:eigenius:reflection:latency_ms").unwrap(),
                    Value::Integer(m.latency_ms),
                );
            }
            r
        }
        Trace::Pure { component, output } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:PureTrace");
            r.set(
                Iri::parse("urn:eigenius:reflection:component").unwrap(),
                Value::String(component.clone()),
            );
            r.set(
                Iri::parse("urn:eigenius:reflection:output").unwrap(),
                Value::Embedded(Box::new(output.clone())),
            );
            r
        }
        Trace::Comorphism {
            comorphism_iri,
            source_trace,
            target_iri,
            target_class,
        } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:ComorphismTrace");
            r.set(
                Iri::parse("urn:eigenius:reflection:comorphism").unwrap(),
                Value::String(comorphism_iri.clone()),
            );
            r.set(
                Iri::parse("urn:eigenius:reflection:target_iri").unwrap(),
                Value::String(target_iri.clone()),
            );
            r.set(
                Iri::parse("urn:eigenius:reflection:target_class").unwrap(),
                Value::String(target_class.clone()),
            );
            if let Some(st) = source_trace {
                r.set(
                    Iri::parse("urn:eigenius:reflection:source_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(st))),
                );
            }
            r
        }
        Trace::Map { element_traces } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:MapTrace");
            let traces: Vec<Value> = element_traces
                .iter()
                .map(|t| match t {
                    Some(t) => Value::Embedded(Box::new(trace_to_resource(t))),
                    None => Value::Embedded(Box::new(empty_trace_resource())),
                })
                .collect();
            r.set(
                Iri::parse("urn:eigenius:reflection:element_traces").unwrap(),
                Value::Array(traces),
            );
            r
        }
        Trace::Reduce { step_traces } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:ReduceTrace");
            let traces: Vec<Value> = step_traces
                .iter()
                .map(|t| match t {
                    Some(t) => Value::Embedded(Box::new(trace_to_resource(t))),
                    None => Value::Embedded(Box::new(empty_trace_resource())),
                })
                .collect();
            r.set(
                Iri::parse("urn:eigenius:reflection:step_traces").unwrap(),
                Value::Array(traces),
            );
            r
        }
        Trace::Case {
            scrutinee_trace,
            branch_taken,
            branch_trace,
        } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:CaseTrace");
            if let Some(st) = scrutinee_trace {
                r.set(
                    Iri::parse("urn:eigenius:reflection:scrutinee_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(st))),
                );
            }
            r.set(
                Iri::parse("urn:eigenius:reflection:branch_taken").unwrap(),
                Value::String(branch_taken.clone()),
            );
            if let Some(bt) = branch_trace {
                r.set(
                    Iri::parse("urn:eigenius:reflection:branch_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(bt))),
                );
            }
            r
        }
        Trace::Construct { field_traces } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:ConstructTrace");
            // One typed FieldTrace entry per constructed property. (An
            // earlier encoding abused an untyped embedded resource as an
            // IRI-keyed map, which recursive validation rightly rejects:
            // the keys are other classes' property IRIs.)
            let entries: Vec<Value> = field_traces
                .iter()
                .map(|(iri, t)| {
                    let mut entry = Resource::new_embedded();
                    set_is_a(&mut entry, "urn:eigenius:reflection:FieldTrace");
                    entry.set(
                        Iri::parse("urn:eigenius:reflection:property").unwrap(),
                        Value::String(iri.as_str().to_string()),
                    );
                    let trace_node = match t {
                        Some(t) => trace_to_resource(t),
                        None => empty_trace_resource(),
                    };
                    entry.set(
                        Iri::parse("urn:eigenius:reflection:trace").unwrap(),
                        Value::Embedded(Box::new(trace_node)),
                    );
                    Value::Embedded(Box::new(entry))
                })
                .collect();
            r.set(
                Iri::parse("urn:eigenius:reflection:field_traces").unwrap(),
                Value::Array(entries),
            );
            r
        }
        Trace::Project {
            source_trace,
            property,
        } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:ProjectTrace");
            if let Some(st) = source_trace {
                r.set(
                    Iri::parse("urn:eigenius:reflection:source_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(st))),
                );
            }
            r.set(
                Iri::parse("urn:eigenius:reflection:property").unwrap(),
                Value::String(property.as_str().to_string()),
            );
            r
        }
        Trace::Seq(children) => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:reflection:SeqTrace");
            let traces: Vec<Value> = children
                .iter()
                .map(|t| Value::Embedded(Box::new(trace_to_resource(t))))
                .collect();
            r.set(
                Iri::parse("urn:eigenius:reflection:child_traces").unwrap(),
                Value::Array(traces),
            );
            r
        }
    }
}

fn set_is_a(resource: &mut Resource, class_iri: &str) {
    resource.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(class_iri.to_string())]),
    );
}

/// Compute the content-addressed key for a ComponentTrace cache lookup.
///
/// Key = SHA-256(component_iri || CBOR(input)).
pub fn compute_trace_key(component: &str, input: &Resource) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let cbor = crate::ontology::eigon_cbor::canonicalize(input);
    let mut hasher = Sha256::new();
    hasher.update(component.as_bytes());
    hasher.update(&cbor);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_trace_store() {
        let store = InMemoryTraceStore::new();
        let key = [0u8; 32];
        assert!(store.get_component_trace(&key).is_none());

        let ct = ComponentTrace {
            component: "urn:eigenius:components:Identity".to_string(),
            input_hash: key,
            argument_hash: None,
            output: Resource::new_embedded(),
            cached: false,
            metrics: None,
        };
        store.put_component_trace(key, ct.clone());
        let retrieved = store.get_component_trace(&key).unwrap();
        assert_eq!(retrieved.component, ct.component);
        assert!(!retrieved.cached);
    }

    #[test]
    fn program_metrics_from_empty_trace() {
        let metrics = ProgramMetrics::from_trace(&None);
        assert_eq!(metrics.total_tokens, 0);
        assert_eq!(metrics.cached_steps, 0);
        assert_eq!(metrics.executed_steps, 0);
    }

    #[test]
    fn program_metrics_accumulates() {
        let trace = Trace::Let {
            name: "x".to_string(),
            value_trace: Some(Box::new(Trace::Component(ComponentTrace {
                component: "urn:test:comp".to_string(),
                input_hash: [0; 32],
                argument_hash: None,
                output: Resource::new_embedded(),
                cached: false,
                metrics: Some(ComponentMetrics {
                    provider: "anthropic".to_string(),
                    model: "claude-sonnet".to_string(),
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    latency_ms: 500,
                }),
            }))),
            body_trace: Some(Box::new(Trace::Pure {
                component: "urn:test:pure".to_string(),
                output: Resource::new_embedded(),
            })),
        };

        let metrics = ProgramMetrics::from_trace(&Some(trace));
        assert_eq!(metrics.total_tokens, 150);
        assert_eq!(metrics.total_latency_ms, 500);
        assert_eq!(metrics.executed_steps, 2); // 1 component + 1 pure
        assert_eq!(metrics.cached_steps, 0);
    }

    #[test]
    fn program_metrics_counts_cached() {
        let trace = Trace::Component(ComponentTrace {
            component: "urn:test:comp".to_string(),
            input_hash: [0; 32],
            argument_hash: None,
            output: Resource::new_embedded(),
            cached: true,
            metrics: Some(ComponentMetrics {
                provider: "anthropic".to_string(),
                model: "claude-sonnet".to_string(),
                prompt_tokens: 100,
                completion_tokens: 50,
                latency_ms: 0,
            }),
        });

        let metrics = ProgramMetrics::from_trace(&Some(trace));
        assert_eq!(metrics.cached_steps, 1);
        assert_eq!(metrics.executed_steps, 0);
    }

    #[test]
    fn trace_to_resource_let() {
        let trace = Trace::Let {
            name: "x".to_string(),
            value_trace: None,
            body_trace: None,
        };
        let r = trace_to_resource(&trace);
        let is_a = r.is_a();
        assert_eq!(is_a[0].as_str(), "urn:eigenius:reflection:LetTrace");
        let name = r
            .get(&Iri::parse("urn:eigenius:reflection:name").unwrap())
            .unwrap();
        assert_eq!(name.as_str(), Some("x"));
    }

    #[test]
    fn trace_to_resource_component() {
        let trace = Trace::Component(ComponentTrace {
            component: "urn:test:comp".to_string(),
            input_hash: [1; 32],
            argument_hash: None,
            output: Resource::new_embedded(),
            cached: false,
            metrics: None,
        });
        let r = trace_to_resource(&trace);
        let is_a = r.is_a();
        assert_eq!(is_a[0].as_str(), "urn:eigenius:reflection:ComponentTrace");
    }

    #[test]
    fn compute_trace_key_deterministic() {
        let input = Resource::new_embedded();
        let k1 = compute_trace_key("urn:test:comp", &input);
        let k2 = compute_trace_key("urn:test:comp", &input);
        assert_eq!(k1, k2);

        // Different component → different key
        let k3 = compute_trace_key("urn:test:other", &input);
        assert_ne!(k1, k3);
    }
}
