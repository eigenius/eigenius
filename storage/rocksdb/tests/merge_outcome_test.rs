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

//! D34 §G.1 / Phase 1a end-to-end regression test for the
//! `NeedsWitnessedMerge` silent-success bug.
//!
//! Before §G.1, [`EigeniusService::advance_branch_for_layer`] swallowed
//! every `Ok(_)` variant of [`UpdateOutcome`] as `Ok(())`, masking
//! `NeedsWitnessedMerge` (which leaves the branch ref unchanged) as
//! success. The freshly-built layer would be on disk but unreachable
//! from any branch; client responses would (wrongly) report
//! `branch_advanced = true`.
//!
//! This test pins the post-fix behaviour:
//!
//! 1. Set up a "concurrent commit" against branch `main` by using
//!    [`lattice::commit_layer`] directly to write a layer `L_A` that
//!    modifies `urn:eigenius:demo:Widget`, then side-channel the
//!    branch ref to point at `L_A` via [`PersistentBackend::put_branch`].
//!    The service's cached `ExecutionContext` for `main` still
//!    believes the branch tip is the bootstrap layer.
//! 2. Drive a `Load` RPC that also modifies `urn:eigenius:demo:Widget`.
//!    The service commits `L_B` on top of bootstrap (its stale view),
//!    and the CAS races against `L_A`; since both contributions touch
//!    the same IRI, the kernel returns `NeedsWitnessedMerge`.
//! 3. Assert the wire response carries:
//!    - `success = true` (the Load did not error)
//!    - `branch_advanced = false` (the fix — `NeedsWitnessedMerge`
//!      means the branch ref did NOT move)
//!    - `merge.outcome = NEEDS_WITNESSED_MERGE`
//!    - `merge.current_head = hex(L_A.id())`
//!    - `merge.conflicting_iris` contains the contested IRI
//! 4. Assert `backend.get_branch("main")` still returns `L_A` — the
//!    branch ref was not touched by the second commit.

use std::sync::Arc;

use eigenius_kernel::lattice::commit_layer_default;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::server::proto::eigenius_kernel_server::EigeniusKernel;
use eigenius_kernel::server::proto::{LoadRequest, MergeOutcome};
use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use tonic::Request;

const CONTESTED_IRI: &str = "urn:eigenius:demo:Widget";

/// Build a Widget resource with the given description. Both sides of
/// the race define this same IRI with different descriptions; that's
/// the disjointness-violating overlap that forces a witnessed merge.
fn widget(description: &str) -> Resource {
    let mut r = Resource::new(Iri::parse(CONTESTED_IRI).unwrap());
    r.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String("urn:eigenius:core:Class".to_string())]),
    );
    r.set(
        Iri::parse("urn:eigenius:core:short_name").unwrap(),
        Value::String("Widget".to_string()),
    );
    r.set(
        Iri::parse("urn:eigenius:core:description").unwrap(),
        Value::String(description.to_string()),
    );
    r
}

#[tokio::test(flavor = "multi_thread")]
async fn load_after_concurrent_conflicting_commit_reports_needs_witnessed_merge() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = Arc::clone(&store) as Arc<dyn PersistentBackend>;

    // Bootstrap the service. The service constructs its main-branch
    // `ExecutionContext` here, pinned to whatever bootstrap head the
    // RocksStore wrote — this is the head the second Load will build
    // against, regardless of what side-channel writes do to the
    // branch ref afterwards.
    let service = EigeniusService::with_persistent_backend(
        eigenius_kernel::program::component::ComponentRegistry::default(),
        Arc::clone(&backend),
    )
    .expect("service");

    let bootstrap_id = backend
        .get_branch("main")
        .unwrap()
        .expect("bootstrap seeds branch:main");
    let bootstrap_info = backend
        .load_chain_from(&bootstrap_id)
        .unwrap()
        .expect("bootstrap chain loads");

    // --- 1. Side-channel commit of L_A: a layer modifying Widget,
    //    parented on bootstrap, written via the lattice's
    //    `commit_layer` (the same primitive `persist_layer_if_backend`
    //    uses internally). Bypasses the service's ctx cache entirely.
    let storage = LayerStorage::with_persistent(Arc::clone(&backend));
    let bootstrap_arc: Arc<Layer> =
        eigenius_kernel::layer::build_chain(bootstrap_info, storage.clone());
    let mut a_builder = LayerBuilder::new("concurrent-a", Some(Arc::clone(&bootstrap_arc)));
    a_builder
        .add_resource(widget("from concurrent client A"))
        .unwrap();
    let layer_a = commit_layer_default(a_builder, storage.clone(), backend.as_ref())
        .expect("commit_layer A succeeds");

    // Advance the branch ref to L_A out-of-band. The service's cached
    // ctx for `main` still believes the tip is bootstrap — exactly
    // the stale-view condition a concurrent client would produce.
    backend
        .put_branch("main", layer_a.id())
        .expect("side-channel put_branch");

    // --- 2. Drive a Load that also modifies Widget. Goes through the
    //    full gRPC path so we exercise the bug-fix in
    //    `persist_layer_if_backend` end-to-end.
    let load_request = LoadRequest {
        resources: format!(
            r#"[{{
                "@id": "{CONTESTED_IRI}",
                "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
                "urn:eigenius:core:short_name": "Widget",
                "urn:eigenius:core:description": "from Load (would conflict with A)"
            }}]"#,
        )
        .into_bytes(),
        content_type: "application/eigon+json".to_string(),
        auto_commit: true,
        branch: String::new(),
        policy: None,
        explicit_tombstones: Vec::new(),
    };
    let response = service
        .load(Request::new(load_request))
        .await
        .expect("load rpc")
        .into_inner();

    // --- 3. Wire-format assertions.
    assert!(
        response.success,
        "Load itself doesn't error on NeedsWitnessedMerge — it surfaces \
         the outcome via the response, not via a tonic Status. errors: {:?}",
        response.errors
    );
    assert!(
        !response.branch_advanced,
        "NeedsWitnessedMerge means the branch ref did NOT move. \
         Pre-D34-§G.1 this lied as `true`; the bug fix lives in \
         persist_layer_if_backend and this assertion is the regression \
         guard."
    );
    let merge = response.merge.as_ref().expect("merge info present");
    assert_eq!(
        merge.outcome,
        MergeOutcome::NeedsWitnessedMerge as i32,
        "expected NEEDS_WITNESSED_MERGE outcome, got {:?}",
        merge.outcome
    );
    assert_eq!(
        merge.current_head,
        hex::encode(layer_a.id().0),
        "current_head should be the side-channel layer's id — what the \
         caller's CAS lost to"
    );
    assert!(
        merge.conflicting_iris.iter().any(|i| i == CONTESTED_IRI),
        "conflicting_iris should list {CONTESTED_IRI}; got {:?}",
        merge.conflicting_iris
    );
    assert!(
        merge.merge_layer_id.is_empty(),
        "merge_layer_id is set only on TrivialMerge outcomes"
    );

    // --- 4. Branch ref didn't move.
    let post_head = backend.get_branch("main").unwrap().expect("branch exists");
    assert_eq!(
        post_head,
        *layer_a.id(),
        "branch should remain at the side-channel layer; the Load's \
         layer is on disk but unreachable from any branch ref"
    );
}
