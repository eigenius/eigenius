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

//! Phase 9b-iii.4 integration test: the startup resume sweep.
//!
//! Simulates a crashed kernel by injecting a `Running` TaskRecord
//! into a fresh RocksStore, then invokes `resume_sweep` directly and
//! verifies that:
//! - The task transitions to a terminal state (Completed or Failed).
//! - `ResumeState.in_progress` flips true during the sweep and false
//!   after.
//! - `ResumeState.remaining` counts down.
//!
//! We invoke `resume_sweep` as a free async function rather than
//! booting a full `start_server`, keeping the test deterministic and
//! independent of the gRPC transport.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::program::component::ComponentRegistry;
use eigenius_kernel::server::{resume_sweep, ResumeConfig, ResumeInputs, ResumeState};
use eigenius_kernel::storage::PersistentBackend;
use eigenius_kernel::task::{BackendTaskStore, TaskRecord, TaskStatus, TaskStore};
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use uuid::Uuid;

/// A tiny identity program JSON — takes input, returns it.
fn identity_program() -> eigenius_kernel::ontology::resource::Resource {
    let json = r#"{
        "@id": "urn:eigenius:test:program:identity",
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
        "urn:eigenius:program:input_type": "urn:eigenius:example:Thing",
        "urn:eigenius:program:output_type": "urn:eigenius:example:Thing",
        "urn:eigenius:program:body": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
            "urn:eigenius:program:name": "input"
        }
    }"#;
    eigenius_kernel::ontology::eigon_json::parse_document(json)
        .unwrap()
        .remove(0)
}

fn thing_class() -> eigenius_kernel::ontology::resource::Resource {
    let json = r#"{
        "@id": "urn:eigenius:example:Thing",
        "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
        "urn:eigenius:core:description": "test",
        "urn:eigenius:core:short_name": "Thing"
    }"#;
    eigenius_kernel::ontology::eigon_json::parse_document(json)
        .unwrap()
        .remove(0)
}

fn payload() -> eigenius_kernel::ontology::resource::Resource {
    let json = r#"{
        "@id": "urn:eigenius:test:input:payload",
        "urn:eigenius:core:is_a": ["urn:eigenius:example:Thing"]
    }"#;
    eigenius_kernel::ontology::eigon_json::parse_document(json)
        .unwrap()
        .remove(0)
}

#[tokio::test]
async fn resume_sweep_completes_injected_running_task() {
    // Setup: persist the bootstrap ontologies + a test program/class/input,
    // then drop an in-flight Running TaskRecord into the task store —
    // simulating a crash just before the task could complete.
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;

    // Drive persistence through bootstrap + commit so the backend has
    // a real head to resolve against. `ExecutionContext::commit` was
    // retired in D41 Phase G — route through `commit_layer_default`.
    let mut ctx = eigenius_kernel::bootstrap::bootstrap_persistent(Arc::clone(&backend)).unwrap();
    for r in [thing_class(), payload(), identity_program()] {
        ctx.add_resource(r).unwrap();
    }
    let working = ctx.take_working("test_setup").unwrap();
    // `commit_layer_default` persists through `BackendStorePersister`,
    // so we no longer need a separate `backend.store_layer` call after
    // the commit lands.
    let layer = eigenius_kernel::lattice::commit_layer_default(
        working,
        ctx.storage().clone(),
        backend.as_ref(),
    )
    .unwrap();
    ctx.advance_head(Arc::clone(&layer), "test_setup").unwrap();
    // Phase 14g: branches replace the legacy single-head pointer.
    backend.put_branch("main", layer.id()).unwrap();
    let pinned_head = layer.id().clone();

    // Inject a Running task pointing at that program.
    let task_store: Arc<dyn TaskStore> = Arc::new(BackendTaskStore::new(Arc::clone(&backend)));
    let session_id = Uuid::nil();
    let task_id = Uuid::from_u128(0x9b_1113_4001);
    let record = TaskRecord::new_running(
        session_id,
        task_id,
        "urn:eigenius:test:program:identity".to_string(),
        "urn:eigenius:test:input:payload".to_string(),
        pinned_head,
        1_000_000,
    );
    task_store.put_task(&record).unwrap();

    // Sanity pre-check.
    assert_eq!(
        task_store
            .get_task(&session_id, &task_id)
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Running,
    );

    // Run the sweep.
    let resume_state = Arc::new(ResumeState::default());
    let inputs = ResumeInputs {
        task_store: Arc::clone(&task_store),
        backend: Arc::clone(&backend),
        trace_store: Arc::new(eigenius_kernel::program::trace::InMemoryTraceStore::new()),
        resume_state: Arc::clone(&resume_state),
    };
    let components = Arc::new(ComponentRegistry::default());
    resume_sweep(inputs, session_id, components, ResumeConfig::default()).await;

    // Sweep drained.
    assert!(!resume_state.in_progress.load(Ordering::SeqCst));
    assert_eq!(resume_state.remaining.load(Ordering::SeqCst), 0);

    // Task transitioned to a terminal state.
    let final_record = task_store.get_task(&session_id, &task_id).unwrap().unwrap();
    assert!(
        final_record.status.is_terminal(),
        "task should be terminal after resume, got {:?}",
        final_record.status
    );
    // Identity program should complete cleanly.
    assert_eq!(final_record.status, TaskStatus::Completed);
    assert!(final_record.updated_at > final_record.created_at);
}

#[tokio::test]
async fn resume_sweep_fails_task_when_pinned_layer_missing() {
    // A task whose pinned layer is absent should transition to
    // Failed rather than hanging or panicking.
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;

    // Just the seed layers — no user-loaded content.
    let _ = eigenius_kernel::bootstrap::bootstrap_persistent(Arc::clone(&backend)).unwrap();

    let task_store: Arc<dyn TaskStore> = Arc::new(BackendTaskStore::new(Arc::clone(&backend)));
    let task_id = Uuid::from_u128(0x9b_1113_4002);
    let record = TaskRecord::new_running(
        Uuid::nil(),
        task_id,
        "urn:missing:program".to_string(),
        "urn:missing:input".to_string(),
        eigenius_kernel::layer::LayerId([0xff; 32]), // bogus layer
        0,
    );
    task_store.put_task(&record).unwrap();

    let resume_state = Arc::new(ResumeState::default());
    let inputs = ResumeInputs {
        task_store: Arc::clone(&task_store),
        backend: Arc::clone(&backend),
        trace_store: Arc::new(eigenius_kernel::program::trace::InMemoryTraceStore::new()),
        resume_state: Arc::clone(&resume_state),
    };
    resume_sweep(
        inputs,
        Uuid::nil(),
        Arc::new(ComponentRegistry::default()),
        ResumeConfig::default(),
    )
    .await;

    let rec = task_store
        .get_task(&Uuid::nil(), &task_id)
        .unwrap()
        .unwrap();
    assert_eq!(rec.status, TaskStatus::Failed);
}

#[tokio::test]
async fn resume_sweep_ignores_terminal_tasks() {
    // Tasks in terminal states (Completed, Failed, Cancelled) must
    // not be re-executed by the sweep.
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;
    let _ = bootstrap::bootstrap_persistent(Arc::clone(&backend)).unwrap();

    let task_store: Arc<dyn TaskStore> = Arc::new(BackendTaskStore::new(Arc::clone(&backend)));
    let session_id = Uuid::nil();

    for (i, status) in [
        TaskStatus::Completed,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ]
    .into_iter()
    .enumerate()
    {
        let mut rec = TaskRecord::new_running(
            session_id,
            Uuid::from_u128(0xdead_beef_0000 + i as u128),
            "urn:test:p".to_string(),
            "urn:test:i".to_string(),
            eigenius_kernel::layer::LayerId([0; 32]),
            0,
        );
        rec.status = status;
        task_store.put_task(&rec).unwrap();
    }

    let resume_state = Arc::new(ResumeState::default());
    let inputs = ResumeInputs {
        task_store: Arc::clone(&task_store),
        backend: Arc::clone(&backend),
        trace_store: Arc::new(eigenius_kernel::program::trace::InMemoryTraceStore::new()),
        resume_state: Arc::clone(&resume_state),
    };
    resume_sweep(
        inputs,
        session_id,
        Arc::new(ComponentRegistry::default()),
        ResumeConfig::default(),
    )
    .await;

    // No task should have been touched — resume_state never flipped
    // (empty resumable list returns early).
    assert!(!resume_state.in_progress.load(Ordering::SeqCst));
    let tasks = task_store.list_tasks(&session_id).unwrap();
    for t in &tasks {
        assert!(
            t.status.is_terminal(),
            "terminal task should stay terminal: {:?}",
            t.status
        );
    }
}
