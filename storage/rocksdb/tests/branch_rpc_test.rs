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

//! Phase 14g integration test: branch RPCs.
//!
//! Drives `ListBranches` / `GetBranch` / `CreateBranch` / `DeleteBranch`
//! against an `EigeniusService` backed by a real RocksStore, verifying the
//! happy paths and the validation/safety branches.

use std::sync::Arc;

use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::{
    CreateBranchRequest, DeleteBranchRequest, GetBranchRequest, ListBranchesRequest,
};
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::{Code, Request};

fn build_service() -> (TempDir, EigeniusService, Arc<dyn PersistentBackend>) {
    let tmp = TempDir::new().expect("tempdir");
    let store = Arc::new(RocksStore::open(tmp.path()).expect("open rocks"));
    let backend: Arc<dyn PersistentBackend> = store;
    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("build service");
    (tmp, service, backend)
}

#[tokio::test]
async fn list_branches_returns_seeded_main() {
    let (_tmp, service, _backend) = build_service();
    let resp = service
        .list_branches(Request::new(ListBranchesRequest {}))
        .await
        .expect("list_branches")
        .into_inner();
    assert_eq!(resp.branches.len(), 1, "bootstrap should seed branch:main");
    assert_eq!(resp.branches[0].name, "main");
    assert!(!resp.branches[0].head_layer.is_empty());
}

#[tokio::test]
async fn get_branch_resolves_main_and_returns_not_found() {
    let (_tmp, service, _backend) = build_service();
    let main = service
        .get_branch(Request::new(GetBranchRequest {
            name: "main".into(),
        }))
        .await
        .expect("get_branch main")
        .into_inner();
    assert!(main.found);
    assert_eq!(main.head_layer.len(), 64); // 32-byte hex

    let missing = service
        .get_branch(Request::new(GetBranchRequest {
            name: "does-not-exist".into(),
        }))
        .await
        .expect("get_branch missing")
        .into_inner();
    assert!(!missing.found);
    assert!(missing.head_layer.is_empty());
}

#[tokio::test]
async fn create_branch_off_main_then_delete() {
    let (_tmp, service, _backend) = build_service();

    // Resolve main's head to use as the branch base.
    let main_head = service
        .get_branch(Request::new(GetBranchRequest {
            name: "main".into(),
        }))
        .await
        .expect("get_branch main")
        .into_inner()
        .head_layer;
    assert!(!main_head.is_empty());

    // Create.
    let create = service
        .create_branch(Request::new(CreateBranchRequest {
            name: "feature-x".into(),
            from_layer: main_head.clone(),
        }))
        .await
        .expect("create_branch")
        .into_inner();
    assert!(create.success, "{}", create.error);
    assert_eq!(create.head_layer, main_head);

    // List shows both.
    let listed = service
        .list_branches(Request::new(ListBranchesRequest {}))
        .await
        .expect("list_branches")
        .into_inner();
    let names: Vec<String> = listed.branches.iter().map(|b| b.name.clone()).collect();
    assert!(names.contains(&"main".to_string()));
    assert!(names.contains(&"feature-x".to_string()));

    // Delete.
    let deleted = service
        .delete_branch(Request::new(DeleteBranchRequest {
            name: "feature-x".into(),
            force: false,
        }))
        .await
        .expect("delete_branch")
        .into_inner();
    assert!(deleted.success);
    assert!(deleted.deleted);
    assert_eq!(deleted.previous_head, main_head);

    // Gone.
    let gone = service
        .get_branch(Request::new(GetBranchRequest {
            name: "feature-x".into(),
        }))
        .await
        .expect("get_branch feature-x")
        .into_inner();
    assert!(!gone.found);
}

#[tokio::test]
async fn create_branch_rejects_duplicate_name() {
    let (_tmp, service, _backend) = build_service();
    let head = service
        .get_branch(Request::new(GetBranchRequest {
            name: "main".into(),
        }))
        .await
        .expect("get_branch main")
        .into_inner()
        .head_layer;

    let dup = service
        .create_branch(Request::new(CreateBranchRequest {
            name: "main".into(),
            from_layer: head,
        }))
        .await
        .expect("create_branch (duplicate)")
        .into_inner();
    assert!(!dup.success);
    assert!(dup.error.contains("already exists"), "{}", dup.error);
}

#[tokio::test]
async fn create_branch_rejects_invalid_name() {
    let (_tmp, service, _backend) = build_service();
    let head = service
        .get_branch(Request::new(GetBranchRequest {
            name: "main".into(),
        }))
        .await
        .expect("get_branch main")
        .into_inner()
        .head_layer;

    let err = service
        .create_branch(Request::new(CreateBranchRequest {
            name: "bad name with spaces".into(),
            from_layer: head,
        }))
        .await
        .expect_err("invalid name should be rejected");
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn create_branch_rejects_unknown_layer() {
    let (_tmp, service, _backend) = build_service();
    let bogus = "ff".repeat(32);

    let err = service
        .create_branch(Request::new(CreateBranchRequest {
            name: "ghost".into(),
            from_layer: bogus,
        }))
        .await
        .expect_err("unknown layer should be rejected");
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test]
async fn delete_branch_missing_is_success_not_deleted() {
    let (_tmp, service, _backend) = build_service();
    let resp = service
        .delete_branch(Request::new(DeleteBranchRequest {
            name: "never-existed".into(),
            force: false,
        }))
        .await
        .expect("delete_branch missing")
        .into_inner();
    assert!(resp.success);
    assert!(!resp.deleted);
    assert!(resp.previous_head.is_empty());
}
