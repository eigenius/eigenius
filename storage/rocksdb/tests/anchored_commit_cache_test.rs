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

//! D33 §6 / Phase 20c end-to-end test for the anchored-commit cache
//! wired into the kernel's `persist_layer_if_backend` path.
//!
//! Drives the gRPC `Load` RPC against an `EigeniusService` backed by a
//! real `RocksStore`. Loads the same content twice and verifies:
//!
//! 1. First load: commits a fresh layer; the cache picks up one
//!    entry; the branch advances.
//! 2. Second load (byte-identical input): hits the cache, returns
//!    the same canonical `LayerId`, the cache still has exactly one
//!    entry, and the branch ref didn't move further.
//!
//! This exercises the "cache substitution as a transparent
//! optimization" semantic — the simplest cache flow, where `Load`'s
//! response surfaces the canonical id in both calls.

use std::sync::Arc;

use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::{GetBranchRequest, LoadRequest};
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::Request;

/// Minimal ontology — one class declaration. Loading it twice
/// exercises the cache.
const ONTOLOGY_JSON: &str = r#"[
  {
    "@id": "urn:eigenius:demo:Widget",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:short_name": "Widget",
    "urn:eigenius:core:description": "A demonstration class for the anchored-commit cache test."
  }
]"#;

// D41 Phase E: the kernel's commit orchestrator runs `didPersist` /
// `didDrain` hooks via `tokio::task::block_in_place`, requiring the
// multi-threaded runtime. Also matches the existing pattern for
// rocksdb e2e tests that exercise the persistent backend through
// commit-shaped RPCs.
#[tokio::test(flavor = "multi_thread")]
async fn load_twice_with_identical_content_hits_anchored_commit_cache() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;
    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    // Pre-condition: empty anchored-commit cache.
    assert!(
        backend.list_anchored_commits().unwrap().is_empty(),
        "fresh DB should start with an empty cache"
    );

    // Capture the branch head before any commit so we can verify it
    // moves on the first load and stays put on the second.
    let pre_head = service
        .get_branch(Request::new(GetBranchRequest {
            name: "main".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .head_layer;
    assert!(!pre_head.is_empty(), "bootstrap should seed branch:main");

    let load_request = || LoadRequest {
        resources: ONTOLOGY_JSON.as_bytes().to_vec(),
        content_type: "application/eigon+json".to_string(),
        auto_commit: true,
        branch: String::new(),
        policy: None,
        explicit_tombstones: Vec::new(),
    };

    // First Load: cache miss → commits a fresh layer; branch advances.
    let first = service
        .load(Request::new(load_request()))
        .await
        .expect("first load rpc")
        .into_inner();
    assert!(first.success, "first load failed: {:?}", first.errors);
    assert!(!first.layer_id.is_empty());
    assert!(
        first.branch_advanced,
        "cache miss must report branch_advanced = true"
    );
    // Cache-miss path runs a fresh CAS against an unchallenged branch
    // → FAST_FORWARD. The conflicting/merge-layer fields are empty.
    let first_merge = first.merge.as_ref().expect("merge info present");
    assert_eq!(
        first_merge.outcome,
        eigenius_kernel::server::proto::MergeOutcome::FastForward as i32,
        "cache miss must report MergeOutcome::FastForward"
    );
    assert!(first_merge.merge_layer_id.is_empty());
    assert!(first_merge.conflicting_iris.is_empty());
    let first_layer_id = first.layer_id.clone();

    let head_after_first = service
        .get_branch(Request::new(GetBranchRequest {
            name: "main".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .head_layer;
    assert_eq!(
        head_after_first, first_layer_id,
        "branch should advance to the freshly-committed layer"
    );

    // The cache should now hold exactly one entry — the one we just
    // wrote.
    let entries_after_first = backend.list_anchored_commits().unwrap();
    assert_eq!(
        entries_after_first.len(),
        1,
        "cache miss should insert exactly one entry"
    );
    assert_eq!(
        hex::encode(entries_after_first[0].layer_id.0),
        first_layer_id
    );

    // Second Load: byte-identical content. The fresh layer is built
    // on top of the just-committed L1 (parent = L1), so its
    // position-hash differs from the cached entry (which has parent =
    // bootstrap). The anchored-commit cache keys on
    // `(content_hash, supporting_content_hash)` — both unchanged —
    // so it hits and returns the cached id L1. This is a
    // **different-position** hit: store_layer and update_branch are
    // both skipped; the branch ref stays where the first load put it.
    let second = service
        .load(Request::new(load_request()))
        .await
        .expect("second load rpc")
        .into_inner();
    assert!(second.success, "second load failed: {:?}", second.errors);
    assert_eq!(
        second.layer_id, first_layer_id,
        "second load must return the canonical (cached) layer id"
    );
    assert!(
        !second.branch_advanced,
        "different-position cache hit must report branch_advanced = false"
    );
    // No CAS was attempted (the persist short-circuited), but the
    // anchored-commit cache was hit at a different chain position —
    // surface `CACHED_DIFFERENT_POSITION` with the cached layer's id
    // in `merge_layer_id` so consumers can distinguish a cache hit
    // from the no-backend / no-commit `UNSPECIFIED` shape.
    let second_merge = second.merge.as_ref().expect("merge info present");
    assert_eq!(
        second_merge.outcome,
        eigenius_kernel::server::proto::MergeOutcome::CachedDifferentPosition as i32,
        "different-position cache hit must report MergeOutcome::CachedDifferentPosition"
    );
    assert_eq!(
        second_merge.merge_layer_id, first_layer_id,
        "cache-hit merge_layer_id must be the canonical (cached) layer id"
    );

    // Branch is still at the same head — no second advance happened.
    let head_after_second = service
        .get_branch(Request::new(GetBranchRequest {
            name: "main".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .head_layer;
    assert_eq!(
        head_after_second, first_layer_id,
        "branch head must still match the first load's layer id"
    );

    // Cache still holds exactly one entry (no duplicate insert; the
    // same `(content_hash, supporting_content_hash) → layer_id`
    // tuple is overwritten idempotently).
    let entries_after_second = backend.list_anchored_commits().unwrap();
    assert_eq!(
        entries_after_second.len(),
        1,
        "cache hit must not add a new entry"
    );
}
