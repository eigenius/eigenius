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
