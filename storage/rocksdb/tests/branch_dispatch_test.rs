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

//! Phase 14g per-request branch dispatch.
//!
//! Verifies that the server routes Load/Inspect/Query/Run to the
//! request's named branch:
//!  - Two branches advance independently when targeted.
//!  - Inspect with `branch=foo` sees data committed via `branch=foo`
//!    but not data committed via `branch=main`, and vice versa.
//!  - The default empty branch resolves to "main".
//!  - Inspect with both `at_layer` and `branch` set is rejected.

use std::sync::Arc;

use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::{
    CreateBranchRequest, GetBranchRequest, InspectRequest, LoadRequest,
};
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::{Code, Request};

const ALPHA_JSON: &str = r#"[
    {
        "@id": "urn:eigenius:test:Alpha",
        "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
        "urn:eigenius:core:short_name": "Alpha",
        "urn:eigenius:core:description": "alpha"
    }
]"#;

const BETA_JSON: &str = r#"[
    {
        "@id": "urn:eigenius:test:Beta",
        "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
        "urn:eigenius:core:short_name": "Beta",
        "urn:eigenius:core:description": "beta"
    }
]"#;

fn build_service() -> (TempDir, EigeniusService) {
    let tmp = TempDir::new().expect("tempdir");
    let store = Arc::new(RocksStore::open(tmp.path()).expect("open rocks"));
    let backend: Arc<dyn PersistentBackend> = store;
    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        backend,
    )
    .expect("build service");
    (tmp, service)
}

async fn current_main_head(service: &EigeniusService) -> String {
    service
        .get_branch(Request::new(GetBranchRequest {
            name: "main".into(),
        }))
        .await
        .expect("get_branch main")
        .into_inner()
        .head_layer
}

async fn create_branch_off_main(service: &EigeniusService, name: &str) {
    let head = current_main_head(service).await;
    let create = service
        .create_branch(Request::new(CreateBranchRequest {
            name: name.into(),
            from_layer: head,
        }))
        .await
        .expect("create_branch")
        .into_inner();
    assert!(create.success, "{}", create.error);
}

async fn load_into(service: &EigeniusService, branch: &str, payload: &str) -> String {
    let resp = service
        .load(Request::new(LoadRequest {
            resources: payload.as_bytes().to_vec(),
            content_type: "application/eigon+json".into(),
            auto_commit: true,
            branch: branch.into(),
            policy: None,
            explicit_tombstones: Vec::new(),
        }))
        .await
        .expect("load")
        .into_inner();
    assert!(resp.success, "{:?}", resp.errors);
    let expected_echo = if branch.is_empty() { "main" } else { branch };
    assert_eq!(resp.branch, expected_echo);
    resp.layer_id
}

#[tokio::test(flavor = "multi_thread")]
async fn two_branches_advance_independently() {
    let (_tmp, service) = build_service();
    create_branch_off_main(&service, "feature-x").await;

    let main_before = current_main_head(&service).await;

    // Load Alpha into feature-x; main should be unchanged.
    let alpha_layer = load_into(&service, "feature-x", ALPHA_JSON).await;
    assert!(!alpha_layer.is_empty());

    let main_after = current_main_head(&service).await;
    assert_eq!(
        main_before, main_after,
        "main should not advance when committing to feature-x",
    );

    // feature-x's head should now be the new layer.
    let feature_head = service
        .get_branch(Request::new(GetBranchRequest {
            name: "feature-x".into(),
        }))
        .await
        .expect("get feature-x")
        .into_inner()
        .head_layer;
    assert_eq!(feature_head, alpha_layer);

    // Load Beta into main; feature-x should be unchanged.
    let beta_layer = load_into(&service, "main", BETA_JSON).await;
    let feature_after_beta = service
        .get_branch(Request::new(GetBranchRequest {
            name: "feature-x".into(),
        }))
        .await
        .expect("get feature-x")
        .into_inner()
        .head_layer;
    assert_eq!(feature_after_beta, alpha_layer);
    assert_ne!(feature_after_beta, beta_layer);
}

#[tokio::test(flavor = "multi_thread")]
async fn inspect_routes_by_branch() {
    let (_tmp, service) = build_service();
    create_branch_off_main(&service, "feature-x").await;

    // Alpha goes to feature-x; Beta goes to main.
    load_into(&service, "feature-x", ALPHA_JSON).await;
    load_into(&service, "main", BETA_JSON).await;

    // Alpha visible on feature-x.
    let alpha_on_feature = service
        .inspect(Request::new(InspectRequest {
            iri: "urn:eigenius:test:Alpha".into(),
            at_layer: String::new(),
            branch: "feature-x".into(),
        }))
        .await
        .expect("inspect alpha on feature-x")
        .into_inner();
    assert!(alpha_on_feature.found);

    // Alpha NOT visible on main.
    let alpha_on_main = service
        .inspect(Request::new(InspectRequest {
            iri: "urn:eigenius:test:Alpha".into(),
            at_layer: String::new(),
            branch: "main".into(),
        }))
        .await
        .expect("inspect alpha on main")
        .into_inner();
    assert!(!alpha_on_main.found);

    // Beta visible on main.
    let beta_on_main = service
        .inspect(Request::new(InspectRequest {
            iri: "urn:eigenius:test:Beta".into(),
            at_layer: String::new(),
            branch: "main".into(),
        }))
        .await
        .expect("inspect beta on main")
        .into_inner();
    assert!(beta_on_main.found);

    // Beta NOT visible on feature-x.
    let beta_on_feature = service
        .inspect(Request::new(InspectRequest {
            iri: "urn:eigenius:test:Beta".into(),
            at_layer: String::new(),
            branch: "feature-x".into(),
        }))
        .await
        .expect("inspect beta on feature-x")
        .into_inner();
    assert!(!beta_on_feature.found);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_branch_defaults_to_main() {
    let (_tmp, service) = build_service();
    let layer = load_into(&service, "", BETA_JSON).await;
    let main_head = current_main_head(&service).await;
    assert_eq!(layer, main_head);
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_branch_returns_not_found() {
    let (_tmp, service) = build_service();
    let err = service
        .load(Request::new(LoadRequest {
            resources: ALPHA_JSON.as_bytes().to_vec(),
            content_type: "application/eigon+json".into(),
            auto_commit: true,
            branch: "ghost-branch".into(),
            policy: None,
            explicit_tombstones: Vec::new(),
        }))
        .await
        .expect_err("ghost branch should fail");
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn at_layer_and_branch_mutually_exclusive() {
    let (_tmp, service) = build_service();
    let err = service
        .inspect(Request::new(InspectRequest {
            iri: "urn:eigenius:core:Class".into(),
            at_layer: "00".repeat(32),
            branch: "main".into(),
        }))
        .await
        .expect_err("at_layer + branch should be rejected");
    assert_eq!(err.code(), Code::InvalidArgument);
}
