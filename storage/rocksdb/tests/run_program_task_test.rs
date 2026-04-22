//! Phase 9b-iii.3a integration test: RunProgram allocates a task and
//! persists its final record.
//!
//! Wires the full `EigeniusService`-with-persistent-backend path:
//! `RunProgram` should return a non-empty `task_id` in the response
//! and leave a `Completed` `TaskRecord` in the task store that
//! clients can later look up via `GetTaskStatus`.
//!
//! The 9b-iii.3c task RPCs will add a proper `GetTaskStatus`; here we
//! reach into the task store directly to verify the record.

use std::sync::Arc;

use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::RunProgramRequest;
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_kernel::task::{BackendTaskStore, TaskStatus, TaskStore};
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::Request;
use uuid::Uuid;

/// An identity program: `input -> input`. Minimal program that
/// returns its input unchanged. Exercises RunProgram without
/// pulling in any component dispatch.
fn identity_program_json() -> String {
    let program = serde_json::json!({
        "@id": "urn:eigenius:test:program:identity",
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
        "urn:eigenius:program:input_type": "urn:eigenius:example:Thing",
        "urn:eigenius:program:output_type": "urn:eigenius:example:Thing",
        "urn:eigenius:program:body": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
            "urn:eigenius:program:name": "input",
        }
    });
    program.to_string()
}

fn class_and_input_json() -> String {
    // An `ex:Thing` class for the program's I/O, plus one instance
    // that will be the input resource. Loaded together before
    // RunProgram so parse_program can resolve the types.
    serde_json::json!([
        {
            "@id": "urn:eigenius:example:Thing",
            "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
            "urn:eigenius:core:description": "test type",
            "urn:eigenius:core:short_name": "Thing"
        },
        {
            "@id": "urn:eigenius:test:input:payload",
            "urn:eigenius:core:is_a": ["urn:eigenius:example:Thing"]
        }
    ])
    .to_string()
}

#[tokio::test]
async fn run_program_persists_task_record() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;

    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    // Step 1: load the class + input resource so they're in the layer.
    let load_resp = service
        .load(Request::new(eigenius_kernel::server::proto::LoadRequest {
            resources: class_and_input_json().into_bytes(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
        }))
        .await
        .expect("load")
        .into_inner();
    assert!(load_resp.success, "load failed: {:?}", load_resp.errors);

    // Step 2: run the program.
    // Input in JSON form (matches content_type).
    let input_bytes = serde_json::json!({
        "@id": "urn:eigenius:test:input:payload",
        "urn:eigenius:core:is_a": ["urn:eigenius:example:Thing"]
    })
    .to_string()
    .into_bytes();

    let run_resp = service
        .run_program(Request::new(RunProgramRequest {
            program: identity_program_json().into_bytes(),
            input: input_bytes,
            content_type: "application/eigon+json".to_string(),
        }))
        .await
        .expect("run_program")
        .into_inner();

    assert!(run_resp.success, "run failed: {:?}", run_resp.errors);

    // Step 3: verify task_id is populated and refers to a persisted
    // TaskRecord with status=Completed.
    assert!(
        !run_resp.task_id.is_empty(),
        "task_id should be populated when a backend is attached"
    );
    let task_id = Uuid::parse_str(&run_resp.task_id).expect("valid task_id UUID");

    let tasks = BackendTaskStore::new(Arc::clone(&backend));
    let record = tasks
        .get_task(&Uuid::nil(), &task_id)
        .expect("get_task")
        .expect("record exists");

    assert_eq!(record.task_id, task_id);
    assert_eq!(record.session_id, Uuid::nil());
    assert_eq!(record.status, TaskStatus::Completed);
    assert_eq!(record.program_iri, "urn:eigenius:test:program:identity");
    // `result_layer_head` is set to the trace layer when one commits.
    // An identity program has no dispatched ComponentTraces, so the
    // trace commit may be a no-op (nothing but the `ProgramTrace`
    // resource itself). Don't over-assert; the field is nullable on
    // the wire.
    assert!(record.created_at > 0);
    assert!(record.updated_at >= record.created_at);
}

#[tokio::test]
async fn list_tasks_and_get_task_status() {
    // Spin up a service, run the identity program, then exercise
    // ListTasks + GetTaskStatus on its returned task_id.
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;

    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    let _ = service
        .load(Request::new(eigenius_kernel::server::proto::LoadRequest {
            resources: class_and_input_json().into_bytes(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
        }))
        .await
        .expect("load");

    let input_bytes = serde_json::json!({
        "@id": "urn:eigenius:test:input:payload",
        "urn:eigenius:core:is_a": ["urn:eigenius:example:Thing"]
    })
    .to_string()
    .into_bytes();

    let run_resp = service
        .run_program(Request::new(RunProgramRequest {
            program: identity_program_json().into_bytes(),
            input: input_bytes,
            content_type: "application/eigon+json".to_string(),
        }))
        .await
        .expect("run_program")
        .into_inner();
    let task_id_str = run_resp.task_id.clone();
    assert!(!task_id_str.is_empty());

    // ListTasks
    let list = service
        .list_tasks(Request::new(
            eigenius_kernel::server::proto::ListTasksRequest {},
        ))
        .await
        .expect("list_tasks")
        .into_inner();
    assert_eq!(list.tasks.len(), 1);
    let info = &list.tasks[0];
    assert_eq!(info.task_id, task_id_str);
    assert_eq!(info.status, "Completed");
    assert_eq!(info.program_iri, "urn:eigenius:test:program:identity");
    assert_eq!(info.session_id, Uuid::nil().to_string());
    assert!(!info.layer_head.is_empty());

    // GetTaskStatus (found)
    let get = service
        .get_task_status(Request::new(
            eigenius_kernel::server::proto::GetTaskStatusRequest {
                task_id: task_id_str.clone(),
            },
        ))
        .await
        .expect("get_task_status")
        .into_inner();
    assert!(get.found);
    assert_eq!(get.task.as_ref().unwrap().status, "Completed");

    // GetTaskStatus (not found)
    let get_missing = service
        .get_task_status(Request::new(
            eigenius_kernel::server::proto::GetTaskStatusRequest {
                task_id: Uuid::from_u128(0xdeadbeef).to_string(),
            },
        ))
        .await
        .expect("get_task_status missing")
        .into_inner();
    assert!(!get_missing.found);
}

#[tokio::test]
async fn cancel_task_marks_running_as_cancelling_and_terminal_is_noop() {
    // Since RunProgram is synchronous in 9b-iii.3, the task is
    // always Completed by the time CancelTask runs. For 9b-iii.3c
    // we verify: cancelling a completed task is a no-op that echoes
    // the existing status; cancelling a manually-injected Running
    // record flips it to Cancelling.
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;

    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    let _ = service
        .load(Request::new(eigenius_kernel::server::proto::LoadRequest {
            resources: class_and_input_json().into_bytes(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
        }))
        .await
        .expect("load");

    // Inject a Running task directly via the store.
    let tasks = BackendTaskStore::new(Arc::clone(&backend));
    let running_id = Uuid::from_u128(0xa111);
    let running = eigenius_kernel::task::TaskRecord::new_running(
        Uuid::nil(),
        running_id,
        "urn:test:p".to_string(),
        "urn:test:i".to_string(),
        eigenius_kernel::layer::LayerId([0; 32]),
        0,
    );
    tasks.put_task(&running).unwrap();

    // Cancel the Running task — flips to Cancelling.
    let resp = service
        .cancel_task(Request::new(
            eigenius_kernel::server::proto::CancelTaskRequest {
                task_id: running_id.to_string(),
            },
        ))
        .await
        .expect("cancel")
        .into_inner();
    assert!(resp.success);
    assert_eq!(resp.status, "Cancelling");
    let back = tasks.get_task(&Uuid::nil(), &running_id).unwrap().unwrap();
    assert_eq!(back.status, eigenius_kernel::task::TaskStatus::Cancelling);

    // Cancel a Completed task (via RunProgram) — no-op.
    let input_bytes = serde_json::json!({
        "@id": "urn:eigenius:test:input:payload",
        "urn:eigenius:core:is_a": ["urn:eigenius:example:Thing"]
    })
    .to_string()
    .into_bytes();

    let run_resp = service
        .run_program(Request::new(RunProgramRequest {
            program: identity_program_json().into_bytes(),
            input: input_bytes,
            content_type: "application/eigon+json".to_string(),
        }))
        .await
        .expect("run_program")
        .into_inner();
    let completed_id = run_resp.task_id.clone();

    let resp = service
        .cancel_task(Request::new(
            eigenius_kernel::server::proto::CancelTaskRequest {
                task_id: completed_id,
            },
        ))
        .await
        .expect("cancel completed")
        .into_inner();
    assert!(resp.success);
    assert_eq!(resp.status, "Completed");
}

#[tokio::test]
async fn run_program_without_backend_has_empty_task_id() {
    // No persistent backend → no task store → task_id stays empty,
    // preserving the pre-Phase-9b-iii behaviour for ephemeral
    // kernels (no regressions for existing synchronous clients).
    let service = EigeniusService::new().expect("service");

    // Same load+run as above, minus persistence.
    let load_resp = service
        .load(Request::new(eigenius_kernel::server::proto::LoadRequest {
            resources: class_and_input_json().into_bytes(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
        }))
        .await
        .expect("load")
        .into_inner();
    assert!(load_resp.success);

    // Input in JSON form (matches content_type).
    let input_bytes = serde_json::json!({
        "@id": "urn:eigenius:test:input:payload",
        "urn:eigenius:core:is_a": ["urn:eigenius:example:Thing"]
    })
    .to_string()
    .into_bytes();

    let run_resp = service
        .run_program(Request::new(RunProgramRequest {
            program: identity_program_json().into_bytes(),
            input: input_bytes,
            content_type: "application/eigon+json".to_string(),
        }))
        .await
        .expect("run_program")
        .into_inner();

    assert!(run_resp.success);
    assert!(
        run_resp.task_id.is_empty(),
        "task_id must be empty when no backend is attached"
    );
}
