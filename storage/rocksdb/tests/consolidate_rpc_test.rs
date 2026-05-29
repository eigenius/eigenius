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

//! Phase 17e integration test: chain-consolidation RPCs.
//!
//! Drives `ConsolidateChain` and `EstimateConsolidation` against an
//! `EigeniusService` backed by a real `RocksStore`, verifying the
//! happy paths and the typed-error variants surface end-to-end on
//! the wire.

use std::sync::Arc;

use eigenius_kernel::lattice::{commit_layer_default, update_branch, ConflictPolicy};
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::{
    ConsolidateChainRequest, ConsolidateErrorKind, EstimateConsolidationRequest, GetBranchRequest,
};
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::Request;

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

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// Commit `n` chain layers above `main`'s current head. Each layer
/// defines a single `urn:eigenius:demo:layer_{i}` resource and
/// advances `main` to point at it. Returns the layer ids in commit
/// order (oldest first).
fn append_chain(n: usize, backend: &Arc<dyn PersistentBackend>) -> Vec<String> {
    let storage = LayerStorage::with_persistent(Arc::clone(backend));
    let mut head_hex = backend
        .get_branch("main")
        .unwrap()
        .expect("seeded main present")
        .clone();
    let mut layer_hex = Vec::new();
    for i in 0..n {
        // Reconstruct the parent Arc<Layer> via load_chain_from so the
        // builder has a real parent pointer.
        let info = backend
            .load_chain_from(&head_hex)
            .unwrap()
            .expect("chain present");
        let parent = eigenius_kernel::layer::build_chain(info, storage.clone());

        let mut builder = LayerBuilder::new(&format!("L{i}"), Some(parent));
        let mut r = Resource::new(iri(&format!("urn:eigenius:demo:layer_{i}")));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
        );
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String(format!("layer_{i}")),
        );
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String(format!("v{i}")),
        );
        builder.add_resource(r).unwrap();
        let layer = commit_layer_default(builder, storage.clone(), backend.as_ref()).unwrap();

        update_branch(
            "main",
            Some(head_hex.clone()),
            layer.id().clone(),
            ConflictPolicy::StrictFastForward,
            storage.clone(),
            backend.as_ref(),
        )
        .unwrap();

        layer_hex.push(hex::encode(layer.id().0));
        head_hex = layer.id().clone();
    }
    layer_hex
}

/// Resolve `main`'s current head as a hex string.
async fn main_head_hex(service: &EigeniusService) -> String {
    service
        .get_branch(Request::new(GetBranchRequest {
            name: "main".into(),
        }))
        .await
        .expect("get_branch main")
        .into_inner()
        .head_layer
}

#[tokio::test]
async fn estimate_then_consolidate_round_trips() {
    let (_tmp, service, backend) = build_service();
    let layers = append_chain(3, &backend);
    let head = main_head_hex(&service).await;
    assert_eq!(
        head, layers[2],
        "main should point at the last committed layer"
    );

    // Estimate the [layers[0]..head] consolidation. No commit yet.
    let estimate = service
        .estimate_consolidation(Request::new(EstimateConsolidationRequest {
            branch: String::new(), // defaults to main
            from_layer: layers[0].clone(),
            to_layer: head.clone(),
            max_walk_entries: 0, // use kernel default
            trace_pin_policy: String::new(),
            preserve_history: false,
        }))
        .await
        .expect("estimate rpc")
        .into_inner();
    assert!(estimate.success, "estimate failed: {}", estimate.error);
    assert_eq!(estimate.collapsed_layer_count, 3);
    // Each layer defines exactly one IRI → predicted and actual match.
    assert_eq!(estimate.predicted_walk_entries, 3);
    assert_eq!(estimate.actual_walk_entries, 3);
    assert_eq!(estimate.predicted_consolidated_layer.len(), 64);
    let predicted = estimate.predicted_consolidated_layer.clone();

    // Branch unchanged after the dry-run.
    assert_eq!(main_head_hex(&service).await, head);

    // Commit the same range. Predicted LayerId must equal the actual.
    let consolidate = service
        .consolidate_chain(Request::new(ConsolidateChainRequest {
            branch: "main".into(),
            from_layer: layers[0].clone(),
            to_layer: head.clone(),
            max_walk_entries: 0,
            trace_pin_policy: String::new(),
            preserve_history: false,
        }))
        .await
        .expect("consolidate rpc")
        .into_inner();
    assert!(
        consolidate.success,
        "consolidate failed: {}",
        consolidate.error
    );
    assert_eq!(consolidate.collapsed_layer_count, 3);
    assert!(consolidate.head_advanced);
    assert_eq!(
        consolidate.consolidated_layer, predicted,
        "estimate's predicted LayerId must equal the real consolidated id"
    );

    // Branch advances to the consolidated layer.
    assert_eq!(
        main_head_hex(&service).await,
        consolidate.consolidated_layer
    );
}

#[tokio::test]
async fn consolidate_surfaces_range_not_ancestral_as_typed_error() {
    let (_tmp, service, backend) = build_service();
    let _layers = append_chain(2, &backend);
    let head = main_head_hex(&service).await;

    // Bogus `from` that's not in the chain.
    let bogus_from = "ff".repeat(32);
    let resp = service
        .consolidate_chain(Request::new(ConsolidateChainRequest {
            branch: "main".into(),
            from_layer: bogus_from.clone(),
            to_layer: head,
            max_walk_entries: 0,
            trace_pin_policy: String::new(),
            preserve_history: false,
        }))
        .await
        .expect("consolidate rpc")
        .into_inner();
    assert!(!resp.success);
    assert_eq!(
        resp.error_kind,
        ConsolidateErrorKind::RangeNotAncestral as i32
    );
    assert_eq!(resp.error_layer, bogus_from);
    assert_eq!(resp.collapsed_layer_count, 0);
}

/// Phase 17f-E: end-to-end RPC test for below-head consolidation.
/// Commits a 5-layer chain, then consolidates an interior 3-layer
/// window. The response reports `head_advanced = false`, a redirect
/// is installed, and the branch ref stays at the original head.
#[tokio::test]
async fn consolidate_chain_rpc_supports_below_head_with_redirect() {
    let (_tmp, service, backend) = build_service();
    let layers = append_chain(5, &backend);
    let head = main_head_hex(&service).await;

    // Consolidate the middle slice [layers[1]..layers[3]].
    let resp = service
        .consolidate_chain(Request::new(ConsolidateChainRequest {
            branch: "main".into(),
            from_layer: layers[1].clone(),
            to_layer: layers[3].clone(),
            max_walk_entries: 0,
            trace_pin_policy: String::new(),
            preserve_history: false,
        }))
        .await
        .expect("consolidate rpc")
        .into_inner();
    assert!(resp.success, "consolidate failed: {}", resp.error);
    assert_eq!(resp.collapsed_layer_count, 3);
    assert!(
        !resp.head_advanced,
        "below-head consolidation must not advance the branch ref"
    );

    // Branch is still at the original head.
    assert_eq!(main_head_hex(&service).await, head);

    // A redirect has been installed at layers[3] → resp.consolidated_layer.
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hex::decode(&layers[3]).unwrap());
    let source = eigenius_kernel::layer::LayerId(bytes);
    let entry = backend
        .lookup_redirect(&source)
        .unwrap()
        .expect("redirect installed");
    assert_eq!(hex::encode(entry.target.0), resp.consolidated_layer);
    assert!(!entry.preserve_history);
}

/// Phase 17f-E: typed `ToNotReachableFromHead` surfaces over the wire
/// when `to` doesn't appear in the branch's chain.
#[tokio::test]
async fn consolidate_chain_rpc_surfaces_to_not_reachable() {
    let (_tmp, service, backend) = build_service();
    let _layers = append_chain(2, &backend);

    let bogus_to = "aa".repeat(32);
    let resp = service
        .consolidate_chain(Request::new(ConsolidateChainRequest {
            branch: "main".into(),
            from_layer: bogus_to.clone(),
            to_layer: bogus_to.clone(),
            max_walk_entries: 0,
            trace_pin_policy: String::new(),
            preserve_history: false,
        }))
        .await
        .expect("consolidate rpc")
        .into_inner();
    assert!(!resp.success);
    // The kernel may surface either `ToNotReachableFromHead` (if the
    // reachability check fires first) or `RangeNotAncestral` (if the
    // chain load happens first). Either is a meaningful operator
    // signal — the test is permissive on which one wins.
    let kind = resp.error_kind;
    assert!(
        kind == ConsolidateErrorKind::ToNotReachableFromHead as i32
            || kind == ConsolidateErrorKind::RangeNotAncestral as i32,
        "expected typed error for unreachable `to`, got error_kind={kind}"
    );
}

#[tokio::test]
async fn estimate_surfaces_cost_cap_with_predicted_entries_in_error_count() {
    let (_tmp, service, backend) = build_service();
    let layers = append_chain(4, &backend);
    let head = main_head_hex(&service).await;

    // Cap at 2, but the range has 4 layers each defining one IRI →
    // predicted entries = 4. Estimate surfaces the typed error so the
    // CLI can show it without committing.
    let resp = service
        .estimate_consolidation(Request::new(EstimateConsolidationRequest {
            branch: "main".into(),
            from_layer: layers[0].clone(),
            to_layer: head,
            max_walk_entries: 2,
            trace_pin_policy: String::new(),
            preserve_history: false,
        }))
        .await
        .expect("estimate rpc")
        .into_inner();
    assert!(!resp.success);
    assert_eq!(resp.error_kind, ConsolidateErrorKind::CostExceedsCap as i32);
    // `error_count` carries the predicted entry count for this kind
    // (D25 §6's `CostExceedsCap` invariant). Useful for the CLI to
    // suggest a `--max-walk-entries` to retry with.
    assert_eq!(resp.error_count, 4);
}
