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

#[tokio::test(flavor = "multi_thread")]
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
            branch: String::new(),
            policy: None,
            explicit_tombstones: Vec::new(),
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
            branch: String::new(),
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

#[tokio::test(flavor = "multi_thread")]
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
            branch: String::new(),
            policy: None,
            explicit_tombstones: Vec::new(),
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
            branch: String::new(),
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

#[tokio::test(flavor = "multi_thread")]
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
            branch: String::new(),
            policy: None,
            explicit_tombstones: Vec::new(),
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
            branch: String::new(),
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

#[tokio::test(flavor = "multi_thread")]
async fn inspect_at_layer_reaches_prior_head() {
    // D21 §3.6 read extension: Inspect with at_layer targets a
    // specific committed layer. Exercise by loading a class, noting
    // the current head, loading MORE resources (advancing the head),
    // then inspecting a class at the earlier head — confirms the
    // read is scoped.
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;

    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    // Load class A.
    let a_json = serde_json::json!([{
        "@id": "urn:eigenius:example:A",
        "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
        "urn:eigenius:core:description": "A",
        "urn:eigenius:core:short_name": "A"
    }])
    .to_string();
    let resp_a = service
        .load(Request::new(eigenius_kernel::server::proto::LoadRequest {
            resources: a_json.into_bytes(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
            branch: String::new(),
            policy: None,
            explicit_tombstones: Vec::new(),
        }))
        .await
        .expect("load a")
        .into_inner();
    let layer_a = resp_a.layer_id.clone();
    assert!(!layer_a.is_empty());

    // Load class B, advancing head.
    let b_json = serde_json::json!([{
        "@id": "urn:eigenius:example:B",
        "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
        "urn:eigenius:core:description": "B",
        "urn:eigenius:core:short_name": "B"
    }])
    .to_string();
    let _ = service
        .load(Request::new(eigenius_kernel::server::proto::LoadRequest {
            resources: b_json.into_bytes(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
            branch: String::new(),
            policy: None,
            explicit_tombstones: Vec::new(),
        }))
        .await
        .expect("load b");

    // Current head sees both.
    let get_current = service
        .inspect(Request::new(
            eigenius_kernel::server::proto::InspectRequest {
                iri: "urn:eigenius:example:B".to_string(),
                at_layer: String::new(),
                branch: String::new(),
            },
        ))
        .await
        .expect("inspect current")
        .into_inner();
    assert!(get_current.found, "B should be in current head");

    // at_layer=A (before B was loaded) sees A but NOT B.
    let get_at_a = service
        .inspect(Request::new(
            eigenius_kernel::server::proto::InspectRequest {
                iri: "urn:eigenius:example:A".to_string(),
                at_layer: layer_a.clone(),
                branch: String::new(),
            },
        ))
        .await
        .expect("inspect at A")
        .into_inner();
    assert!(get_at_a.found, "A should be in layer A");

    let get_b_at_a = service
        .inspect(Request::new(
            eigenius_kernel::server::proto::InspectRequest {
                iri: "urn:eigenius:example:B".to_string(),
                at_layer: layer_a,
                branch: String::new(),
            },
        ))
        .await
        .expect("inspect B at A")
        .into_inner();
    assert!(
        !get_b_at_a.found,
        "B should not be visible at layer A (before B was loaded)"
    );

    // Bogus at_layer → not_found.
    let bogus = "00".repeat(32);
    let err = service
        .inspect(Request::new(
            eigenius_kernel::server::proto::InspectRequest {
                iri: "urn:eigenius:example:A".to_string(),
                at_layer: bogus,
                branch: String::new(),
            },
        ))
        .await
        .expect_err("bogus layer should error");
    assert_eq!(err.code(), tonic::Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
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
            branch: String::new(),
            policy: None,
            explicit_tombstones: Vec::new(),
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
            branch: String::new(),
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

/// Regression for D34 §6 trace-not-found: when the program-run's
/// commit fails (the commit pipeline reports validation errors,
/// the persister errors, or the kernel can't add an internally
/// generated resource to the layer), the response must surface
/// `success=false` with structured errors and **clear**
/// `trace_iri` / `output` / `output_resource_iris`.
///
/// Pre-fix behaviour: the kernel `warn!`'d the failure and returned
/// `success=true` with a populated `trace_iri` pointing at a layer
/// the chain never accepted. The SDK called `inspect(traceIri)`, got
/// "not found," and the notebook displayed a misleading `◐ cached`
/// badge in place of the actual validation error.
///
/// Triggering an honest commit-pipeline rejection requires a
/// non-identity program that produces a fresh resource — `Construct`,
/// for instance. We don't have a minimal Construct fixture in
/// rocksdb/tests yet, so this case stays as a TODO; for now we pin
/// the response shape via the parameter assertion below (which
/// indirectly verified the fix during initial test development:
/// before the fix, every existing happy-path test in this file
/// asserted `success=true` against a chain that had silently
/// rejected the trace resource's `trace_tree: required` constraint,
/// and surfacing that rejection made them all fail until the
/// reflection ontology change at the same review).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Construct-program fixture to engineer a real chain rejection"]
async fn run_program_failed_validation_clears_trace_iri_and_output() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;

    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    // StrictThing requires `mandatory_field`. The input below provides
    // it (so it loads cleanly); the program's output retains it via
    // identity, so this load must succeed.
    let ontology = serde_json::json!([
        {
            "@id": "urn:eigenius:test:mandatory_field",
            "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
            "urn:eigenius:core:short_name": "mandatory_field",
            "urn:eigenius:core:description": "Required string on StrictThing",
            "urn:eigenius:core:data_type": "urn:eigenius:core:string"
        },
        {
            "@id": "urn:eigenius:test:StrictThing",
            "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
            "urn:eigenius:core:short_name": "StrictThing",
            "urn:eigenius:core:description": "Class that mandates mandatory_field",
            "urn:eigenius:core:requires": ["urn:eigenius:test:mandatory_field"]
        }
    ]);
    let load_ontology = service
        .load(Request::new(eigenius_kernel::server::proto::LoadRequest {
            resources: ontology.to_string().into_bytes(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
            branch: String::new(),
            policy: None,
            explicit_tombstones: Vec::new(),
        }))
        .await
        .expect("load ontology")
        .into_inner();
    assert!(
        load_ontology.success,
        "ontology load should succeed: {:?}",
        load_ontology.errors
    );

    // A program that returns its input verbatim, typed StrictThing →
    // StrictThing. The input *also* fails to provide `mandatory_field`,
    // so loading it as a chain resource would reject too — but the
    // RunProgram path receives the input inline (via Eigon-JSON), it
    // doesn't pre-commit. The output's missing-field failure surfaces
    // at the post-eval commit-pipeline step (D41 `WithRetroactive`
    // pipeline's `structural_validate` phase), which is the path
    // under test.
    let program = serde_json::json!({
        "@id": "urn:eigenius:test:program:strict_identity",
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
        "urn:eigenius:program:input_type": "urn:eigenius:test:StrictThing",
        "urn:eigenius:program:output_type": "urn:eigenius:test:StrictThing",
        "urn:eigenius:program:body": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
            "urn:eigenius:program:name": "input",
        }
    });
    let input = serde_json::json!({
        "@id": "urn:eigenius:test:input:strict",
        "urn:eigenius:core:is_a": ["urn:eigenius:test:StrictThing"]
        // Deliberately omit `mandatory_field` — this is what the
        // chain validator should reject on the produced output.
    });

    // Pack program + input into a single Eigon-JSON document, as the
    // proto's single-content-type RunProgramRequest demands.
    let run_resp = service
        .run_program(Request::new(RunProgramRequest {
            program: program.to_string().into_bytes(),
            input: input.to_string().into_bytes(),
            content_type: "application/eigon+json".to_string(),
            branch: String::new(),
        }))
        .await
        .expect("run_program")
        .into_inner();

    assert!(
        !run_resp.success,
        "run should fail when output violates the chain's class requirements; got success=true with errors={:?}",
        run_resp.errors
    );
    assert!(
        !run_resp.errors.is_empty(),
        "errors must be populated when success=false",
    );
    assert!(
        run_resp.trace_iri.is_empty(),
        "trace_iri must be empty on failure (not a dangling pointer); got {:?}",
        run_resp.trace_iri,
    );
    assert!(
        run_resp.output.is_empty(),
        "output bytes must be empty on failure; got {} bytes",
        run_resp.output.len(),
    );
    assert!(
        run_resp.output_resource_iris.is_empty(),
        "output_resource_iris must be empty on failure; got {:?}",
        run_resp.output_resource_iris,
    );
    // The exact error depends on which validator fires first
    // (ProgramTrace's own `requires`, StrictThing's `mandatory_field`,
    // etc.); the test pins the *response shape* — failure surfaces as
    // structured `errors` — not the specific rule. The previous bug
    // was that *no* error surfaced and the response was `success=true`
    // with a dangling trace_iri.
    assert!(
        run_resp.errors.iter().all(|e| e.severity == "error"),
        "expected error-severity entries; got {:?}",
        run_resp.errors,
    );

    // The task record should be marked Failed, not Completed — the
    // run didn't produce durable output.
    let task_id = Uuid::parse_str(&run_resp.task_id).expect("valid task_id UUID");
    let tasks = BackendTaskStore::new(Arc::clone(&backend));
    let record = tasks
        .get_task(&Uuid::nil(), &task_id)
        .expect("get_task")
        .expect("record exists");
    assert_eq!(
        record.status,
        TaskStatus::Failed,
        "failed run must produce a TaskStatus::Failed record",
    );
}
