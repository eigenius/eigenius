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

//! Phase 9b-iii.2 integration test: positional trace lookup with a
//! TaskContext attached to the IO effect engine.
//!
//! Uses a counter-backed IO component to prove that:
//! 1. Without a TaskContext, repeated calls with the same input hit
//!    the cross-task content-address memo (legacy behavior preserved).
//! 2. With a TaskContext, each call consumes a distinct positional
//!    slot and re-dispatches — streams are not collapsed.
//! 3. Rewinding step_seq (simulating a replay after crash) makes the
//!    positional lookup hit the stored trace and return the cached
//!    output without re-dispatching.
//!
//! This is the core correctness claim behind D21 §3.2.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use eigenius_kernel::bootstrap;
use eigenius_kernel::layer::Layer;
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::{eval_ctx, EvalCtx};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::program::component::{BuiltinComponent, ComponentRegistry, ComponentResult};
use eigenius_kernel::program::trace::{InMemoryTraceStore, TraceStore};
use eigenius_kernel::storage::PersistentBackend;
use eigenius_kernel::task::{BackendTaskStore, TaskContext, TaskRecord, TaskStore};
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use uuid::Uuid;

const COUNTER_IRI: &str = "urn:eigenius:test:counter";
const COUNTER_OUTPUT_PROP: &str = "urn:eigenius:test:counter:value";

/// A test-only IO component that returns its monotonically-incremented
/// call count. Simulates a non-deterministic IO component (like
/// `dequeue` or `Now()`).
struct CounterComponent {
    calls: AtomicU64,
}

impl CounterComponent {
    fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
        }
    }

    fn call_count(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl BuiltinComponent for CounterComponent {
    fn is_io(&self) -> bool {
        true
    }

    fn execute(
        &self,
        _input: &Resource,
        _argument: Option<&Resource>,
        _layer: &Layer,
    ) -> Result<ComponentResult, String> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut out = Resource::new_embedded();
        out.set(
            Iri::parse(COUNTER_OUTPUT_PROP).unwrap(),
            Value::Integer(n as i64),
        );
        Ok(ComponentResult {
            output: out,
            metrics: None,
        })
    }
}

fn make_registry(counter: Arc<CounterComponent>) -> Arc<ComponentRegistry> {
    let mut registry = ComponentRegistry::new();
    // Wrap the Arc<CounterComponent> in a newtype so the registry's
    // `Box<dyn BuiltinComponent>` slot can hold it while the test
    // retains a handle to observe the counter.
    struct Wrap(Arc<CounterComponent>);
    impl BuiltinComponent for Wrap {
        fn is_io(&self) -> bool {
            self.0.is_io()
        }
        fn execute(
            &self,
            input: &Resource,
            argument: Option<&Resource>,
            layer: &Layer,
        ) -> Result<ComponentResult, String> {
            self.0.execute(input, argument, layer)
        }
    }
    registry.register(COUNTER_IRI.to_string(), Box::new(Wrap(counter)));
    Arc::new(registry)
}

fn dispatch_expr() -> Exp {
    // Applies the counter IRI to an empty Unit argument. In IO mode,
    // `Exp::Var(name)` where `name` is a registered component routes
    // through dispatch_component.
    Exp::App(
        Box::new(Exp::Var(COUNTER_IRI.to_string())),
        Box::new(Exp::Unit),
    )
}

fn counter_value(v: &Val) -> i64 {
    match v {
        Val::ResourceVal(r) => {
            let prop = Iri::parse(COUNTER_OUTPUT_PROP).unwrap();
            match r.get(&prop) {
                Some(Value::Integer(n)) => *n,
                other => panic!("expected Integer, got {other:?}"),
            }
        }
        other => panic!("expected ResourceVal, got {other:?}"),
    }
}

fn make_io_ctx(
    layer: Arc<Layer>,
    registry: Arc<ComponentRegistry>,
    trace_store: Option<Arc<dyn TraceStore>>,
    task_context: Option<Arc<TaskContext>>,
) -> EvalCtx {
    let engine = eigenius_kernel::institution::eval_hooks::InstitutionEngine::for_io(
        Arc::clone(&layer),
        registry,
        trace_store,
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
        task_context,
        None,
        None,
    );
    EvalCtx::effectful(Some(layer), Arc::new(engine))
}

#[test]
fn io_without_task_context_never_caches() -> Result<(), Box<dyn std::error::Error>> {
    // D21 §3.3: the content-address memo is unsafe for IO — two
    // calls with the same input may produce different results, so
    // caching them would silently collapse distinct observations.
    // Without a TaskContext, the evaluator must dispatch every call.
    let ctx_kernel = bootstrap::bootstrap()?;
    let layer = Arc::clone(ctx_kernel.head());

    let counter = Arc::new(CounterComponent::new());
    let registry = make_registry(Arc::clone(&counter));
    let trace_store: Arc<dyn TraceStore> = Arc::new(InMemoryTraceStore::new());

    let ctx = make_io_ctx(layer, registry, Some(trace_store), None);
    let exp = dispatch_expr();

    let v1 = eval_ctx(&exp, &Rho::Nil, &ctx)?;
    let v2 = eval_ctx(&exp, &Rho::Nil, &ctx)?;
    let v3 = eval_ctx(&exp, &Rho::Nil, &ctx)?;
    // Each call produces a fresh dispatch — no content-address memo
    // collapse.
    assert_eq!(counter_value(&v1), 0);
    assert_eq!(counter_value(&v2), 1);
    assert_eq!(counter_value(&v3), 2);
    assert_eq!(counter.call_count(), 3);
    Ok(())
}

#[test]
fn with_task_context_each_step_is_its_own_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    // D21 §3.2: positional keys mean each step consumes a distinct
    // slot; content-address collisions don't collapse them.
    let tmp = TempDir::new()?;
    let store = Arc::new(RocksStore::open(tmp.path())?);
    let backend: Arc<dyn PersistentBackend> = store;
    let task_store: Arc<dyn TaskStore> = Arc::new(BackendTaskStore::new(Arc::clone(&backend)));

    let session_id = Uuid::nil();
    let task_id = Uuid::from_u128(1);
    let tc = Arc::new(TaskContext::new(
        session_id,
        task_id,
        Arc::clone(&task_store),
    ));

    // Persist a fresh TaskRecord — dispatch_component's commit_step
    // needs to read + update it.
    let record = TaskRecord::new_running(
        session_id,
        task_id,
        "urn:test:program:foo".to_string(),
        "urn:test:input:bar".to_string(),
        eigenius_kernel::layer::LayerId([0; 32]),
        0,
    );
    task_store.put_task(&record)?;

    let ctx_kernel = bootstrap::bootstrap()?;
    let layer = Arc::clone(ctx_kernel.head());
    let counter = Arc::new(CounterComponent::new());
    let registry = make_registry(Arc::clone(&counter));

    let ctx = make_io_ctx(layer, registry, None, Some(Arc::clone(&tc)));
    let exp = dispatch_expr();

    // Three calls, same input — should produce three distinct
    // dispatches because each occupies its own step_seq.
    let v1 = eval_ctx(&exp, &Rho::Nil, &ctx)?;
    let v2 = eval_ctx(&exp, &Rho::Nil, &ctx)?;
    let v3 = eval_ctx(&exp, &Rho::Nil, &ctx)?;
    assert_eq!(counter_value(&v1), 0);
    assert_eq!(counter_value(&v2), 1);
    assert_eq!(counter_value(&v3), 2);
    assert_eq!(counter.call_count(), 3);
    assert_eq!(tc.current_step(), 3);

    // Record was updated via commit_step.
    let record_back = task_store.get_task(&session_id, &task_id)?.unwrap();
    assert_eq!(record_back.step_seq, 3);
    assert_eq!(record_back.latest_trace_seq, 2);
    Ok(())
}

#[test]
fn checkpoint_component_persists_checkpoint_and_updates_record(
) -> Result<(), Box<dyn std::error::Error>> {
    // D21 §4: calling `components:Checkpoint` during a task persists
    // the input resource as a Checkpoint and sets
    // TaskRecord.last_checkpoint to the current step_seq. Verifies
    // the commit_step atomic-write path end-to-end.
    let tmp = TempDir::new()?;
    let store = Arc::new(RocksStore::open(tmp.path())?);
    let backend: Arc<dyn PersistentBackend> = store;
    let task_store: Arc<dyn TaskStore> = Arc::new(BackendTaskStore::new(Arc::clone(&backend)));

    let session_id = Uuid::nil();
    let task_id = Uuid::from_u128(0xc4ec_0517_0000_0001);
    let tc = Arc::new(TaskContext::new(
        session_id,
        task_id,
        Arc::clone(&task_store),
    ));
    let record = TaskRecord::new_running(
        session_id,
        task_id,
        "urn:test:program:ckpt".to_string(),
        "urn:test:input:ckpt".to_string(),
        eigenius_kernel::layer::LayerId([0; 32]),
        0,
    );
    task_store.put_task(&record)?;

    let ctx_kernel = eigenius_kernel::bootstrap::bootstrap()?;
    let layer = Arc::clone(ctx_kernel.head());
    // Use the default registry — Checkpoint is a standard built-in.
    let registry = Arc::new(ComponentRegistry::default());

    let ctx = make_io_ctx(layer, registry, None, Some(Arc::clone(&tc)));
    // App(Var("urn:eigenius:program:components:Checkpoint"), Unit)
    let expr = eigenius_kernel::nbe::term::Exp::App(
        Box::new(eigenius_kernel::nbe::term::Exp::Var(
            "urn:eigenius:program:components:Checkpoint".to_string(),
        )),
        Box::new(eigenius_kernel::nbe::term::Exp::Unit),
    );
    let _ = eval_ctx(&expr, &eigenius_kernel::nbe::env::Rho::Nil, &ctx)?;

    // Record updated: step_seq=1, last_checkpoint=Some(0).
    let back = task_store.get_task(&session_id, &task_id)?.unwrap();
    assert_eq!(back.step_seq, 1);
    assert_eq!(back.latest_trace_seq, 0);
    assert_eq!(back.last_checkpoint, Some(0));

    // Checkpoint value is readable by step.
    let ckpt = task_store
        .get_checkpoint(&session_id, &task_id, 0)?
        .expect("checkpoint persisted");
    assert_eq!(ckpt.step_seq, 0);
    assert!(!ckpt.state.is_empty(), "checkpoint state bytes empty");
    Ok(())
}

#[test]
fn replay_hits_stored_traces_without_redispatching() -> Result<(), Box<dyn std::error::Error>> {
    // Run three IO calls with a TaskContext, then simulate a "crash
    // and restart" by reopening the store with a fresh TaskContext
    // (step_seq=0). The evaluator should hit the stored traces for
    // steps 0, 1, 2 without re-dispatching.
    let tmp = TempDir::new()?;
    let session_id = Uuid::nil();
    let task_id = Uuid::from_u128(42);

    // --- Round 1: dispatch three times and persist ---
    {
        let store = Arc::new(RocksStore::open(tmp.path())?);
        let backend: Arc<dyn PersistentBackend> = store;
        let task_store: Arc<dyn TaskStore> = Arc::new(BackendTaskStore::new(Arc::clone(&backend)));
        let tc = Arc::new(TaskContext::new(
            session_id,
            task_id,
            Arc::clone(&task_store),
        ));
        let record = TaskRecord::new_running(
            session_id,
            task_id,
            "urn:test:program:foo".to_string(),
            "urn:test:input:bar".to_string(),
            eigenius_kernel::layer::LayerId([0; 32]),
            0,
        );
        task_store.put_task(&record)?;

        let ctx_kernel = bootstrap::bootstrap()?;
        let layer = Arc::clone(ctx_kernel.head());
        let counter = Arc::new(CounterComponent::new());
        let registry = make_registry(counter);
        let ctx = make_io_ctx(layer, registry, None, Some(Arc::clone(&tc)));
        let exp = dispatch_expr();

        let _ = eval_ctx(&exp, &Rho::Nil, &ctx)?;
        let _ = eval_ctx(&exp, &Rho::Nil, &ctx)?;
        let _ = eval_ctx(&exp, &Rho::Nil, &ctx)?;
    }

    // --- Round 2: fresh TaskContext, fresh counter, replay ---
    {
        let store = Arc::new(RocksStore::open(tmp.path())?);
        let backend: Arc<dyn PersistentBackend> = store;
        let task_store: Arc<dyn TaskStore> = Arc::new(BackendTaskStore::new(Arc::clone(&backend)));
        let tc = Arc::new(TaskContext::new(
            session_id,
            task_id,
            Arc::clone(&task_store),
        ));

        let ctx_kernel = bootstrap::bootstrap()?;
        let layer = Arc::clone(ctx_kernel.head());
        let counter = Arc::new(CounterComponent::new());
        let registry = make_registry(Arc::clone(&counter));
        let ctx = make_io_ctx(layer, registry, None, Some(Arc::clone(&tc)));
        let exp = dispatch_expr();

        let v1 = eval_ctx(&exp, &Rho::Nil, &ctx)?;
        let v2 = eval_ctx(&exp, &Rho::Nil, &ctx)?;
        let v3 = eval_ctx(&exp, &Rho::Nil, &ctx)?;

        // The counter belongs to the new (fresh) component — if
        // replay worked, it was never called.
        assert_eq!(counter.call_count(), 0, "replay should not re-dispatch");
        assert_eq!(counter_value(&v1), 0);
        assert_eq!(counter_value(&v2), 1);
        assert_eq!(counter_value(&v3), 2);
    }
    Ok(())
}
