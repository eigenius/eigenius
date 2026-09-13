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
//!
//! Only `ComponentTrace`s participate in memoization, but not for the
//! reason D6b §8 gives. Cache routing is determinism-gated (D21 §3.3),
//! and the split is the opposite of "only IO needs caching": an IO
//! component uses the positional per-task replay slot in `TaskContext`
//! and never consults this content-addressed store, while a
//! deterministic (non-IO) component is the only kind that reads and
//! writes it. See `institution::eval_hooks::dispatch_component`.

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
/// Only `ComponentTrace`s are stored. They are *not* the IO boundary:
/// `dispatch_component` routes IO components to the positional per-task
/// replay slot and only deterministic components to this store (D21
/// §3.3). Nothing else is memoized here.
///
/// The key is SHA-256 over all three factors — component IRI, input and argument, each
/// length-prefixed, with a presence tag on the argument; see [`compute_trace_key`] for
/// why the framing rather than bare concatenation. It implemented only the first two
/// until `2026-09-12`, and both construction sites set
/// `argument_hash: None`, so two calls to one component with the same
/// input and different arguments collided; since the cache is consulted
/// before execution, the second call was served the first one's output
/// and never ran (eigenius#146).
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
/// as `program:traces:EmptyTrace` so trace-child properties can be
/// constrained to `program:traces:Trace` without admitting untyped
/// embedded resources.
fn empty_trace_resource() -> Resource {
    let mut r = Resource::new_embedded();
    set_is_a(&mut r, "urn:eigenius:program:traces:EmptyTrace");
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
            set_is_a(&mut r, "urn:eigenius:program:traces:LetTrace");
            r.set(
                Iri::parse("urn:eigenius:program:traces:name").unwrap(),
                Value::String(name.clone()),
            );
            if let Some(vt) = value_trace {
                r.set(
                    Iri::parse("urn:eigenius:program:traces:value_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(vt))),
                );
            }
            if let Some(bt) = body_trace {
                r.set(
                    Iri::parse("urn:eigenius:program:traces:body_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(bt))),
                );
            }
            r
        }
        Trace::Component(ct) => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:program:traces:ComponentTrace");
            r.set(
                Iri::parse("urn:eigenius:program:traces:component").unwrap(),
                Value::String(ct.component.clone()),
            );
            r.set(
                Iri::parse("urn:eigenius:program:traces:input_hash").unwrap(),
                Value::String(hex::encode(ct.input_hash)),
            );
            if let Some(ah) = &ct.argument_hash {
                r.set(
                    Iri::parse("urn:eigenius:program:traces:argument_hash").unwrap(),
                    Value::String(hex::encode(ah)),
                );
            }
            r.set(
                Iri::parse("urn:eigenius:program:traces:output").unwrap(),
                Value::Embedded(Box::new(ct.output.clone())),
            );
            r.set(
                Iri::parse("urn:eigenius:program:traces:cached").unwrap(),
                Value::Boolean(ct.cached),
            );
            if let Some(m) = &ct.metrics {
                r.set(
                    Iri::parse("urn:eigenius:program:traces:provider").unwrap(),
                    Value::String(m.provider.clone()),
                );
                r.set(
                    Iri::parse("urn:eigenius:program:traces:model").unwrap(),
                    Value::String(m.model.clone()),
                );
                r.set(
                    Iri::parse("urn:eigenius:program:traces:prompt_tokens").unwrap(),
                    Value::Integer(m.prompt_tokens),
                );
                r.set(
                    Iri::parse("urn:eigenius:program:traces:completion_tokens").unwrap(),
                    Value::Integer(m.completion_tokens),
                );
                r.set(
                    Iri::parse("urn:eigenius:program:traces:latency_ms").unwrap(),
                    Value::Integer(m.latency_ms),
                );
            }
            r
        }
        Trace::Pure { component, output } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:program:traces:PureTrace");
            r.set(
                Iri::parse("urn:eigenius:program:traces:component").unwrap(),
                Value::String(component.clone()),
            );
            r.set(
                Iri::parse("urn:eigenius:program:traces:output").unwrap(),
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
            set_is_a(&mut r, "urn:eigenius:program:traces:ComorphismTrace");
            r.set(
                Iri::parse("urn:eigenius:program:traces:comorphism").unwrap(),
                Value::String(comorphism_iri.clone()),
            );
            r.set(
                Iri::parse("urn:eigenius:program:traces:target_iri").unwrap(),
                Value::String(target_iri.clone()),
            );
            r.set(
                Iri::parse("urn:eigenius:program:traces:target_class").unwrap(),
                Value::String(target_class.clone()),
            );
            if let Some(st) = source_trace {
                r.set(
                    Iri::parse("urn:eigenius:program:traces:source_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(st))),
                );
            }
            r
        }
        Trace::Map { element_traces } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:program:traces:MapTrace");
            let traces: Vec<Value> = element_traces
                .iter()
                .map(|t| match t {
                    Some(t) => Value::Embedded(Box::new(trace_to_resource(t))),
                    None => Value::Embedded(Box::new(empty_trace_resource())),
                })
                .collect();
            r.set(
                Iri::parse("urn:eigenius:program:traces:element_traces").unwrap(),
                Value::Array(traces),
            );
            r
        }
        Trace::Reduce { step_traces } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:program:traces:ReduceTrace");
            let traces: Vec<Value> = step_traces
                .iter()
                .map(|t| match t {
                    Some(t) => Value::Embedded(Box::new(trace_to_resource(t))),
                    None => Value::Embedded(Box::new(empty_trace_resource())),
                })
                .collect();
            r.set(
                Iri::parse("urn:eigenius:program:traces:step_traces").unwrap(),
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
            set_is_a(&mut r, "urn:eigenius:program:traces:CaseTrace");
            if let Some(st) = scrutinee_trace {
                r.set(
                    Iri::parse("urn:eigenius:program:traces:scrutinee_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(st))),
                );
            }
            r.set(
                Iri::parse("urn:eigenius:program:traces:branch_taken").unwrap(),
                Value::String(branch_taken.clone()),
            );
            if let Some(bt) = branch_trace {
                r.set(
                    Iri::parse("urn:eigenius:program:traces:branch_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(bt))),
                );
            }
            r
        }
        Trace::Construct { field_traces } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:program:traces:ConstructTrace");
            // One typed FieldTrace entry per constructed property. (An
            // earlier encoding abused an untyped embedded resource as an
            // IRI-keyed map, which recursive validation rightly rejects:
            // the keys are other classes' property IRIs.)
            let entries: Vec<Value> = field_traces
                .iter()
                .map(|(iri, t)| {
                    let mut entry = Resource::new_embedded();
                    set_is_a(&mut entry, "urn:eigenius:program:traces:FieldTrace");
                    entry.set(
                        Iri::parse("urn:eigenius:program:traces:property").unwrap(),
                        Value::iri(iri),
                    );
                    let trace_node = match t {
                        Some(t) => trace_to_resource(t),
                        None => empty_trace_resource(),
                    };
                    entry.set(
                        Iri::parse("urn:eigenius:program:traces:trace").unwrap(),
                        Value::Embedded(Box::new(trace_node)),
                    );
                    Value::Embedded(Box::new(entry))
                })
                .collect();
            r.set(
                Iri::parse("urn:eigenius:program:traces:field_traces").unwrap(),
                Value::Array(entries),
            );
            r
        }
        Trace::Project {
            source_trace,
            property,
        } => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:program:traces:ProjectTrace");
            if let Some(st) = source_trace {
                r.set(
                    Iri::parse("urn:eigenius:program:traces:source_trace").unwrap(),
                    Value::Embedded(Box::new(trace_to_resource(st))),
                );
            }
            r.set(
                Iri::parse("urn:eigenius:program:traces:property").unwrap(),
                Value::iri(property),
            );
            r
        }
        Trace::Seq(children) => {
            let mut r = Resource::new_embedded();
            set_is_a(&mut r, "urn:eigenius:program:traces:SeqTrace");
            let traces: Vec<Value> = children
                .iter()
                .map(|t| Value::Embedded(Box::new(trace_to_resource(t))))
                .collect();
            r.set(
                Iri::parse("urn:eigenius:program:traces:child_traces").unwrap(),
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

/// Content hash of one resource under CBOR canonicalisation.
///
/// This is what `program:traces:input_hash` and `program:traces:argument_hash`
/// are declared to carry: *"Content hash of the component input"*, and the same
/// for the argument. The trace used to put the whole composite cache key in the
/// `input_hash` slot, which is a different value under a name that says otherwise.
pub fn hash_resource(r: &Resource) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(crate::ontology::eigon_cbor::canonicalize(r));
    hasher.finalize().into()
}

/// Compute the content-addressed key for a ComponentTrace cache lookup.
///
/// Key = SHA-256(component_iri ‖ CBOR(input) ‖ CBOR(argument)), the three factors
/// [`TraceStore`] specifies. The argument used to be missing, and because the
/// cache is consulted BEFORE execution, a second call with the same input and a
/// different argument was served the first call's output and never ran
/// (eigenius#146).
///
/// Every field is length-prefixed and the argument carries a presence tag, so no
/// two distinct triples can produce the same byte sequence. Without the tag a
/// component called with no argument and one called with an argument that
/// canonicalises to nothing would key alike; without the lengths, a longer IRI
/// against a shorter input could.
pub fn compute_trace_key(
    component: &str,
    input: &Resource,
    argument: Option<&Resource>,
) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let field = |bytes: &[u8], h: &mut Sha256| {
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    };
    field(component.as_bytes(), &mut hasher);
    field(
        &crate::ontology::eigon_cbor::canonicalize(input),
        &mut hasher,
    );
    match argument {
        None => hasher.update([0u8]),
        Some(a) => {
            hasher.update([1u8]);
            field(&crate::ontology::eigon_cbor::canonicalize(a), &mut hasher);
        }
    }
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
        assert_eq!(is_a[0].as_str(), "urn:eigenius:program:traces:LetTrace");
        let name = r
            .get(&Iri::parse("urn:eigenius:program:traces:name").unwrap())
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
        assert_eq!(
            is_a[0].as_str(),
            "urn:eigenius:program:traces:ComponentTrace"
        );
    }

    #[test]
    fn compute_trace_key_deterministic() {
        let input = Resource::new_embedded();
        let k1 = compute_trace_key("urn:test:comp", &input, None);
        let k2 = compute_trace_key("urn:test:comp", &input, None);
        assert_eq!(k1, k2);

        // Different component → different key
        let k3 = compute_trace_key("urn:test:other", &input, None);
        assert_ne!(k1, k3);
    }

    /// **eigenius#146.** The argument is part of the key.
    ///
    /// It was not, and the cache is consulted before execution, so one component
    /// called twice with the same input and different arguments was served the
    /// first call's output and never ran. The three cases below are the ones that
    /// collided: two arguments, and an argument against none.
    #[test]
    fn the_argument_is_part_of_the_memo_key() {
        fn arg(v: i64) -> Resource {
            let mut r = Resource::new_embedded();
            r.set(Iri::parse("urn:test:n").unwrap(), Value::Integer(v));
            r
        }
        let input = Resource::new_embedded();
        let none = compute_trace_key("urn:test:comp", &input, None);
        let a = compute_trace_key("urn:test:comp", &input, Some(&arg(1)));
        let b = compute_trace_key("urn:test:comp", &input, Some(&arg(2)));

        assert_ne!(a, b, "same input, different argument, must not share a key");
        assert_ne!(none, a, "an argument must not key like no argument");
        assert_eq!(
            a,
            compute_trace_key("urn:test:comp", &input, Some(&arg(1))),
            "and the key is still deterministic"
        );
    }

    /// The two hash slots carry what the ontology says they carry.
    ///
    /// `program:traces:input_hash` is declared "Content hash of the component
    /// input"; the trace used to put the whole composite cache key there, which is
    /// a different value under a name that says otherwise.
    #[test]
    fn the_trace_hash_slots_hold_the_hashes_they_are_named_for() {
        let mut input = Resource::new_embedded();
        input.set(Iri::parse("urn:test:x").unwrap(), Value::Integer(7));

        assert_eq!(hash_resource(&input), hash_resource(&input.clone()));
        assert_ne!(
            hash_resource(&input),
            compute_trace_key("urn:test:comp", &input, None),
            "the input hash is not the cache key"
        );
    }
}
