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

//! End-to-end commit-time validation against bootstrapped chains.
//!
//! Combines four scopes that the focused-unit tests around the kernel
//! don't exercise together:
//!
//! 1. **Tombstones through real storage** — committed layers carrying
//!    tombstones round-trip through `RocksStore` and survive a
//!    process-restart-style reopen.
//! 2. **Retroactive validation against a real bootstrapped chain** —
//!    the per-commit retroactive pass fires against the real
//!    `bootstrap_persistent` core ontology + a domain layer, not the
//!    synthetic test fixture the kernel unit tests use.
//! 3. **Cascade end-to-end** — `CommitPolicy::CascadeTombstone`
//!    drives a Property/Class redef commit through the cascade loop;
//!    the resulting layer's `tombstoned_iris` are persisted, and a
//!    reopen-from-disk verifies they survive.
//! 4. **Memory-vs-RocksDB parity under the new policies** — the same
//!    scenario run against both backends produces byte-identical
//!    layer ids (because content_hash + position_hash are fully
//!    deterministic and both backends share the same canonicalisation
//!    + tombstone-in-content-hash semantics).
//!
//! If any of these regress — a CBOR encoding drift, a missing index
//! write, a forgotten tombstone-in-content-hash entry, a divergence
//! between in-memory and RocksDB cascade outcomes — this file fires.

use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::lattice::{commit_layer, commit_layer_default, CommitError, CommitPolicy};
use eigenius_kernel::layer::{build_chain, Layer, LayerBuilder, LayerId, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::storage::memory::MemoryPersistentBackend;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_kernel::validation::CommitWorkingSet;
use eigenius_storage_rocksdb::RocksStore;
use std::collections::BTreeSet;
use std::sync::Arc;
use tempfile::TempDir;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

// ─── Scenario fixtures ──────────────────────────────────────────────────

/// Bootstrap a chain on the given backend and return the (head Arc,
/// LayerStorage) pair for further commits.
fn open_bootstrapped(backend: Arc<dyn PersistentBackend>) -> (Arc<Layer>, LayerStorage) {
    bootstrap_persistent(Arc::clone(&backend)).expect("bootstrap");
    let storage = LayerStorage::with_persistent(Arc::clone(&backend));
    let head_id = backend
        .get_branch("main")
        .unwrap()
        .expect("bootstrap leaves branch:main pointing at the institution layer");
    let info = backend
        .load_chain_from(&head_id)
        .unwrap()
        .expect("chain reconstructs from bootstrap head");
    let head = build_chain(info, storage.clone());
    (head, storage)
}

/// Commit a domain layer on top of `parent` that defines `demo:Animal`
/// (a Class with the meta-ontology baseline requires) and one
/// instance `demo:rex`. Returns the committed layer and advances
/// `branch:main` to it.
fn commit_domain_layer(
    parent: Arc<Layer>,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Arc<Layer> {
    let mut b = LayerBuilder::new("demo_domain", Some(parent));

    let mut animal = Resource::new(iri("urn:eigenius:demo:Animal"));
    animal.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
    );
    animal.set(iri(wk::DESCRIPTION), Value::String("Animal class".into()));
    animal.set(iri(wk::SHORT_NAME), Value::String("Animal".into()));
    b.add_resource(animal).unwrap();

    let mut rex = Resource::new(iri("urn:eigenius:demo:rex"));
    rex.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:demo:Animal"))]),
    );
    rex.set(iri(wk::DESCRIPTION), Value::String("Rex the dog".into()));
    rex.set(iri(wk::SHORT_NAME), Value::String("rex".into()));
    b.add_resource(rex).unwrap();

    let layer =
        commit_layer_default(b, storage, backend).expect("domain commit validates and stores");
    backend.put_branch("main", layer.id()).unwrap();
    layer
}

/// Build a redef layer that adds `demo:species` (a new Property) and
/// redefines `demo:Animal` to require it. Caller commits with a
/// chosen policy.
fn build_redef_builder(parent: Arc<Layer>) -> LayerBuilder {
    let mut b = LayerBuilder::new("demo_redef", Some(parent));

    let mut species = Resource::new(iri("urn:eigenius:demo:species"));
    species.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
    );
    species.set(iri(wk::DESCRIPTION), Value::String("species".into()));
    species.set(iri(wk::SHORT_NAME), Value::String("species".into()));
    species.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
    b.add_resource(species).unwrap();

    let mut animal = Resource::new(iri("urn:eigenius:demo:Animal"));
    animal.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
    );
    animal.set(
        iri(wk::DESCRIPTION),
        Value::String("Animal class (with species)".into()),
    );
    animal.set(iri(wk::SHORT_NAME), Value::String("Animal".into()));
    animal.set(
        iri(wk::REQUIRES),
        Value::Array(vec![
            Value::ResourceRef(iri(wk::IS_A)),
            Value::ResourceRef(iri(wk::DESCRIPTION)),
            Value::ResourceRef(iri(wk::SHORT_NAME)),
            Value::ResourceRef(iri("urn:eigenius:demo:species")),
        ]),
    );
    b.add_resource(animal).unwrap();

    b
}

// ─── Scenario runner ────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
struct CascadeScenarioState {
    /// Position hash of the committed layer.
    layer_id: LayerId,
    /// Cascade-added tombstones (excludes whatever the caller's
    /// builder already had).
    cascade_tombstones: BTreeSet<Iri>,
    /// Number of cascade fixpoint iterations.
    cascade_iterations: u32,
    /// Does `demo:rex` resolve from the committed layer? Must be
    /// false — cascade should have tombstoned it.
    rex_resolves_at_head: bool,
    /// Does `demo:Animal` resolve from the committed layer with the
    /// new `requires` slot?
    animal_required_count: usize,
    /// Branch state at end of scenario.
    branch_main: Option<LayerId>,
}

/// Run the cascade scenario against `backend` and capture observable
/// state. Used by the parity test to compare in-memory vs. RocksDB.
fn run_cascade_scenario(backend: Arc<dyn PersistentBackend>) -> CascadeScenarioState {
    let (head, storage) = open_bootstrapped(Arc::clone(&backend));
    let domain = commit_domain_layer(head, storage.clone(), backend.as_ref());

    let redef_b = build_redef_builder(Arc::clone(&domain));
    let mut ws = CommitWorkingSet::in_memory();
    let outcome = commit_layer(
        redef_b,
        storage,
        backend.as_ref(),
        CommitPolicy::CascadeTombstone,
        &mut ws,
    )
    .expect("cascade commit should succeed by tombstoning demo:rex");

    backend.put_branch("main", outcome.layer.id()).unwrap();

    let rex_resolves_at_head = outcome
        .layer
        .resolve(&iri("urn:eigenius:demo:rex"))
        .is_some();
    let animal_required_count = outcome
        .layer
        .resolve(&iri("urn:eigenius:demo:Animal"))
        .and_then(|r| r.get(&iri(wk::REQUIRES)).cloned())
        .map(|v| v.as_iri_array().len())
        .unwrap_or(0);

    CascadeScenarioState {
        layer_id: outcome.layer.id().clone(),
        cascade_tombstones: outcome.cascade_tombstones,
        cascade_iterations: outcome.cascade_iterations,
        rex_resolves_at_head,
        animal_required_count,
        branch_main: backend.get_branch("main").unwrap(),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

/// (1) Tombstone round-trip through RocksDB + reopen.
///
/// Caller explicitly tombstones an IRI in a commit. After persisting
/// + closing + reopening the store, the tombstone must survive: the
/// IRI resolves to `None` from the reopened head.
#[tokio::test(flavor = "multi_thread")]
async fn tombstone_round_trips_through_rocksdb_reopen() {
    let dir = TempDir::new().unwrap();

    let domain_id: LayerId;
    let tombstoning_id: LayerId;

    // Phase A: bootstrap, commit domain, commit a tombstoning layer.
    {
        let backend: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(dir.path()).unwrap());
        let (head, storage) = open_bootstrapped(Arc::clone(&backend));
        let domain = commit_domain_layer(head, storage.clone(), backend.as_ref());
        domain_id = domain.id().clone();

        // Caller-driven tombstone: hide demo:rex without redef.
        let mut tomb_b = LayerBuilder::new("hide_rex", Some(Arc::clone(&domain)));
        tomb_b.tombstone(iri("urn:eigenius:demo:rex")).unwrap();
        let tomb_layer =
            commit_layer_default(tomb_b, storage, backend.as_ref()).expect("tombstone commit");
        tombstoning_id = tomb_layer.id().clone();
        backend.put_branch("main", &tombstoning_id).unwrap();

        // Pre-reopen sanity.
        assert!(tomb_layer.resolve(&iri("urn:eigenius:demo:rex")).is_none());
        assert!(tomb_layer
            .resolve(&iri("urn:eigenius:demo:Animal"))
            .is_some());
    }

    // Phase B: reopen, rebuild chain, verify tombstone survives.
    {
        let backend: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(dir.path()).unwrap());
        let head_id = backend
            .get_branch("main")
            .unwrap()
            .expect("branch:main survives reopen");
        assert_eq!(head_id, tombstoning_id);

        let info = backend.load_chain_from(&head_id).unwrap().unwrap();
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));
        let head = build_chain(info, storage);

        assert!(
            head.resolve(&iri("urn:eigenius:demo:rex")).is_none(),
            "tombstone must survive process restart"
        );
        assert!(
            head.resolve(&iri("urn:eigenius:demo:Animal")).is_some(),
            "other domain resources still resolve"
        );
        // The domain layer is still in the topology (we kept it as
        // the tombstoning layer's parent).
        let topo = backend.load_topology().unwrap();
        assert!(topo.get_layer(&domain_id).is_some());
        assert!(topo.get_layer(&tombstoning_id).is_some());
    }
}

/// (2) Retroactive `Reject` against a bootstrapped chain.
///
/// A Class redef that adds a required property must surface a
/// retroactive violation on every lower-layer instance that doesn't
/// carry the new requirement. The commit must fail with
/// `CommitError::Validation` naming `demo:rex` as the violating IRI.
#[tokio::test(flavor = "multi_thread")]
async fn retroactive_reject_against_bootstrapped_rocksdb_chain() {
    let dir = TempDir::new().unwrap();
    let backend: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(dir.path()).unwrap());
    let (head, storage) = open_bootstrapped(Arc::clone(&backend));
    let domain = commit_domain_layer(head, storage.clone(), backend.as_ref());

    let redef_b = build_redef_builder(Arc::clone(&domain));
    let mut ws = CommitWorkingSet::in_memory();
    let result = commit_layer(
        redef_b,
        storage,
        backend.as_ref(),
        CommitPolicy::Reject {
            max_violations: 100,
        },
        &mut ws,
    );

    match result {
        Err(CommitError::Validation {
            errors,
            total_violations,
        }) => {
            let violating_ids: BTreeSet<String> = errors
                .iter()
                .filter_map(|e| e.resource_id.as_ref().map(|i| i.as_str().to_string()))
                .collect();
            assert!(
                violating_ids.contains("urn:eigenius:demo:rex"),
                "expected demo:rex in violations; got {violating_ids:?}"
            );
            assert!(
                total_violations >= 1,
                "expected at least one violation, got {total_violations}"
            );
        }
        other => panic!("expected CommitError::Validation, got {other:?}"),
    }

    // The branch ref must still point at the domain layer — the
    // failed commit didn't advance it.
    assert_eq!(
        backend.get_branch("main").unwrap(),
        Some(domain.id().clone())
    );
}

/// (3) Cascade end-to-end through RocksDB with reopen.
///
/// Same Class redef, but `CommitPolicy::CascadeTombstone`. The
/// cascade tombstones `demo:rex`, fixpoint at iteration 2, persists
/// the layer with the cascade-added tombstone, advances the branch,
/// and a reopen confirms the persisted state matches.
#[tokio::test(flavor = "multi_thread")]
async fn cascade_end_to_end_through_rocksdb_with_reopen() {
    let dir = TempDir::new().unwrap();
    let layer_id: LayerId;

    // Phase A: bootstrap, domain, cascade commit, branch advance.
    {
        let backend: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(dir.path()).unwrap());
        let state = run_cascade_scenario(Arc::clone(&backend));
        assert!(
            state
                .cascade_tombstones
                .contains(&iri("urn:eigenius:demo:rex")),
            "cascade must tombstone demo:rex"
        );
        assert_eq!(state.cascade_iterations, 2);
        assert!(!state.rex_resolves_at_head);
        // Animal's `requires` set is now 4 (the three baselines plus
        // demo:species).
        assert_eq!(state.animal_required_count, 4);
        layer_id = state.layer_id.clone();
        assert_eq!(state.branch_main, Some(layer_id.clone()));
    }

    // Phase B: reopen, verify persisted state matches.
    {
        let backend: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(dir.path()).unwrap());
        let head_id = backend
            .get_branch("main")
            .unwrap()
            .expect("branch:main survives reopen");
        assert_eq!(head_id, layer_id);

        // The committed layer's tombstoned_iris contains demo:rex.
        let handle = backend.load_handle(&head_id).unwrap().expect("handle");
        assert!(
            handle
                .tombstoned_iris
                .contains(&iri("urn:eigenius:demo:rex")),
            "persisted handle must carry the cascade tombstone"
        );

        // Rebuild the chain and verify resolve semantics.
        let info = backend.load_chain_from(&head_id).unwrap().unwrap();
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));
        let head = build_chain(info, storage);

        assert!(
            head.resolve(&iri("urn:eigenius:demo:rex")).is_none(),
            "cascade tombstone survives reopen"
        );
        assert!(
            head.resolve(&iri("urn:eigenius:demo:Animal")).is_some(),
            "Animal class still resolves"
        );
        // The redefined Animal carries the new requires.
        let animal = head
            .resolve(&iri("urn:eigenius:demo:Animal"))
            .expect("Animal");
        let requires = animal
            .get(&iri(wk::REQUIRES))
            .expect("requires set")
            .as_iri_array();
        assert!(
            requires.contains(&iri("urn:eigenius:demo:species")),
            "redef'd Animal must require demo:species; got {requires:?}"
        );
    }
}

/// (4) Parity: `MemoryPersistentBackend` and `RocksStore` must
/// produce byte-identical state for the cascade scenario.
///
/// Content_hash + position_hash are deterministic, both backends
/// share the same canonicalisation, and tombstones participate in
/// content_hash. So if the cascade produces the same tombstone set
/// (it must, since the input is identical), the resulting layer ids
/// must match across backends.
#[tokio::test(flavor = "multi_thread")]
async fn parity_cascade_rocksdb_vs_memory_backend() {
    let mem_state = run_cascade_scenario(Arc::new(MemoryPersistentBackend::new()));

    let dir = TempDir::new().unwrap();
    let rocks_state =
        run_cascade_scenario(Arc::new(RocksStore::open(dir.path()).unwrap()) as Arc<_>);

    // Layer ids must match — same content, same parents, same
    // tombstones, both backends produce the same position hash.
    assert_eq!(
        mem_state.layer_id, rocks_state.layer_id,
        "layer ids diverged across backends"
    );
    assert_eq!(mem_state.cascade_tombstones, rocks_state.cascade_tombstones);
    assert_eq!(mem_state.cascade_iterations, rocks_state.cascade_iterations);
    assert_eq!(
        mem_state.rex_resolves_at_head,
        rocks_state.rex_resolves_at_head
    );
    assert_eq!(
        mem_state.animal_required_count,
        rocks_state.animal_required_count
    );
    assert_eq!(mem_state.branch_main, rocks_state.branch_main);
}
