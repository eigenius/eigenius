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

//! Phase 19 — program-output chain reinsertion (D14 §9.3 step 4).
//!
//! Programs that emit a typed Resource output should land that output
//! in the regular chain at a deterministic content-hash IRI, so it
//! becomes addressable, queryable, and dedupes across re-runs. This
//! test drives the full server boundary: load an ontology + a Construct
//! program, run it over a top-level input, then resolve the returned
//! `output_resource_iris` through `inspect`.

use std::sync::Arc;

use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::{InspectRequest, LoadRequest, RunProgramRequest};
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::Request;

const ESL_SOURCE: &str = r#"
namespace core = "urn:eigenius:core";
namespace ex = "urn:eigenius:example";

class ex:Thing {
    description = "test type";
    requires ex:label;
}

property ex:label : core:string {
    description = "the label";
}

program ex:reproject : ex:Thing -> ex:Thing {
    Construct ex:Thing { ex:label = input.ex:label }
}
"#;

/// Compile the ESL source and split out the program resource from
/// the ontology declarations. Both come back as `serde_json::Value`
/// blobs ready for the gRPC `Load` / `RunProgram` calls.
fn compile_test_artifacts() -> (serde_json::Value, serde_json::Value) {
    let resources = eigenius_kernel::esl::compile(ESL_SOURCE).expect("ESL compile");
    let program_iri = "urn:eigenius:example:reproject";
    let mut program: Option<serde_json::Value> = None;
    let mut others: Vec<serde_json::Value> = Vec::new();
    for r in &resources {
        let json = eigon_json::serialize_resource(r);
        if r.id().map(|i| i.as_str()) == Some(program_iri) {
            program = Some(json);
        } else {
            others.push(json);
        }
    }
    let program = program.expect("program resource compiled");
    let ontology = serde_json::Value::Array(others);
    (program, ontology)
}

#[tokio::test(flavor = "multi_thread")]
async fn run_program_commits_construct_output_to_chain_at_deterministic_iri() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;

    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    let (program_json, ontology_json) = compile_test_artifacts();

    // Load the ontology so the program's input/output type and
    // `ex:label` property resolve at parse and validate time.
    let load_resp = service
        .load(Request::new(LoadRequest {
            resources: ontology_json.to_string().into_bytes(),
            content_type: "application/eigon+json".to_string(),
            auto_commit: true,
            branch: String::new(),
            policy: None,
            explicit_tombstones: Vec::new(),
        }))
        .await
        .expect("load ontology")
        .into_inner();
    assert!(load_resp.success, "load failed: {:?}", load_resp.errors);

    let input_run1 = serde_json::json!({
        "@id": "urn:eigenius:test:input:run1",
        "urn:eigenius:core:is_a": ["urn:eigenius:example:Thing"],
        "urn:eigenius:example:label": "first-run"
    })
    .to_string()
    .into_bytes();

    let run_resp = service
        .run_program(Request::new(RunProgramRequest {
            program: program_json.to_string().into_bytes(),
            input: input_run1.clone(),
            content_type: "application/eigon+json".to_string(),
            branch: String::new(),
        }))
        .await
        .expect("run_program")
        .into_inner();
    assert!(run_resp.success, "run failed: {:?}", run_resp.errors);

    // The Construct program returns a fresh embedded Resource; the
    // run-boundary mints a deterministic
    // `urn:eigenius:program-output:reproject:<hex>` IRI for it and
    // commits it to the chain. Response carries the IRI back so
    // clients can resolve it.
    assert_eq!(
        run_resp.output_resource_iris.len(),
        1,
        "expected exactly one elevated program output IRI, got {:?}",
        run_resp.output_resource_iris
    );
    let output_iri = run_resp.output_resource_iris[0].clone();
    assert!(
        output_iri.starts_with("urn:eigenius:program-output:reproject:"),
        "expected `urn:eigenius:program-output:reproject:<hex>` IRI, got {output_iri}"
    );

    // The resource resolves through the chain — downstream EigenQL,
    // MATCH, or `resolve` calls can find it.
    let inspect = service
        .inspect(Request::new(InspectRequest {
            iri: output_iri.clone(),
            at_layer: String::new(),
            branch: String::new(),
        }))
        .await
        .expect("inspect")
        .into_inner();
    assert!(
        inspect.found,
        "elevated program output {output_iri} should be resolvable in the chain"
    );

    // Determinism: re-running with identical input mints the same
    // IRI (chain-dedup property — two paths arriving at the same
    // sentence land at the same resource).
    let run_resp2 = service
        .run_program(Request::new(RunProgramRequest {
            program: program_json.to_string().into_bytes(),
            input: input_run1,
            content_type: "application/eigon+json".to_string(),
            branch: String::new(),
        }))
        .await
        .expect("run_program 2")
        .into_inner();
    assert!(run_resp2.success);
    assert_eq!(
        run_resp2.output_resource_iris, run_resp.output_resource_iris,
        "identical input must mint identical content-hash IRI on re-run"
    );

    // Distinct input → distinct IRI.
    let input_run3 = serde_json::json!({
        "@id": "urn:eigenius:test:input:run3",
        "urn:eigenius:core:is_a": ["urn:eigenius:example:Thing"],
        "urn:eigenius:example:label": "different-run"
    })
    .to_string()
    .into_bytes();
    let run_resp3 = service
        .run_program(Request::new(RunProgramRequest {
            program: program_json.to_string().into_bytes(),
            input: input_run3,
            content_type: "application/eigon+json".to_string(),
            branch: String::new(),
        }))
        .await
        .expect("run_program 3")
        .into_inner();
    assert!(run_resp3.success);
    assert_ne!(
        run_resp3.output_resource_iris, run_resp.output_resource_iris,
        "distinct input must mint distinct content-hash IRI"
    );
}
