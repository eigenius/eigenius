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

//! Lattice write surface — `commit_layer` + `update_branch` (D23 §5.4 /
//! Phase 14d).
//!
//! These two primitives are the *only* sanctioned way to advance the
//! layer DAG. Callers compose them to produce whatever workflow they
//! need (CLI commits, notebook saves, task runner output): `commit_layer`
//! appends an immutable layer to the DAG; `update_branch` advances a
//! branch ref via CAS. They are independent — committing a layer does
//! not touch any branch, and branch updates accept any committed
//! `LayerId`.
//!
//! **Why two primitives, not one.** Bundling commit-and-update would
//! force every commit to declare a branch upfront. That's wrong for
//! task output (the task may not own a branch), wrong for divergent
//! workflows (a notebook session that produces a chain to be reviewed
//! before pointing a branch at it), and wrong for time-travel (loading
//! a layer to inspect it shouldn't require a branch). Decoupled, the
//! surface fits all those cases.
//!
//! **Concurrency.** A single in-process branch mutex serialises
//! `update_branch` calls — the kernel runs as one process per DB
//! (RocksDB enforces this), so cross-process coordination doesn't
//! exist. Per-branch sub-locks would reduce contention if multiple
//! branches are advanced concurrently; v1 keeps a single mutex because
//! v1 workloads have one or two active branches at a time. Easy to
//! shard later if profiling demands it.

use crate::layer::{Layer, LayerBuilder, LayerError, LayerId, LayerStorage};
use crate::ontology::iri::Iri;
use crate::storage::{PersistentBackend, StorageError};
use crate::validation::{ValidationError, Validator};
use std::sync::{Arc, Mutex};

/// Branch-name validation: matches `[A-Za-z0-9_-]+` per D23 §5.5.
fn is_valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Errors from `commit_layer`.
#[derive(Debug)]
pub enum CommitError {
    /// The candidate layer failed validation against its parent chain.
    Validation(Vec<ValidationError>),
    /// Storage backend reported an error during the commit write.
    Storage(StorageError),
    /// The builder rejected a resource (e.g., core-namespace violation
    /// on a non-root layer). Surfaced from `LayerBuilder::add_resource`
    /// callers; the lattice doesn't generate these itself but propagates
    /// them when the builder is constructed inline.
    Layer(LayerError),
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitError::Validation(errs) => {
                writeln!(f, "validation failed with {} error(s):", errs.len())?;
                for e in errs {
                    writeln!(f, "  {e}")?;
                }
                Ok(())
            }
            CommitError::Storage(e) => write!(f, "storage error during commit: {e}"),
            CommitError::Layer(e) => write!(f, "layer build error: {e}"),
        }
    }
}

impl std::error::Error for CommitError {}

/// Outcome of an `update_branch` call.
///
/// 14d ships only `FastForward` and `NeedsWitnessedMerge` outcomes;
/// `TrivialMerge` is reserved for 14e (disjoint-IRI auto-reconciliation)
/// and `NeedsWitnessedMerge.conflicting_iris` is left empty in 14d
/// because populating it requires the same divergence-set computation
/// trivial merge introduces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// `expected_old_head` matched the branch's current head; the CAS
    /// succeeded and the branch now points at `new_head`.
    FastForward,
    /// 14e — the caller's chain and the branch's current head modify
    /// disjoint sets of IRIs since their lowest common ancestor; the
    /// kernel produced a merge layer with both heads as parents and
    /// updated the branch to point at it. Not produced by 14d.
    TrivialMerge { merge_layer: LayerId },
    /// Divergence: the branch's actual head is not `expected_old_head`,
    /// and the changes since divergence are (or might be) conflicting.
    /// The branch is unchanged; the caller's `new_head` chain still
    /// exists in the DAG but isn't pointed at by any branch ref.
    ///
    /// `conflicting_iris` is empty in 14d (see module docs) and
    /// populated in 14e once the divergence-set computation lands.
    NeedsWitnessedMerge {
        current_head: LayerId,
        conflicting_iris: Vec<Iri>,
    },
}

/// What `update_branch` should do when the CAS check finds a different
/// current head than the caller expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Allow trivial merge if no IRIs conflict; otherwise return
    /// `NeedsWitnessedMerge`. Trivial-merge resolution lands in 14e —
    /// in 14d this policy currently behaves identically to
    /// `StrictFastForward` modulo the error variant returned.
    AllowTrivial,
    /// Refuse anything but a fast-forward. Useful for "I really expect
    /// this to be a clean append; surface anything else as an error."
    StrictFastForward,
}

/// Errors from `update_branch`.
#[derive(Debug)]
pub enum BranchUpdateError {
    /// Branch name fails the regex `[A-Za-z0-9_-]+` (or is too long).
    InvalidBranchName(String),
    /// Storage backend reported an error during read or write.
    Storage(StorageError),
    /// `StrictFastForward` policy and the branch isn't at the expected
    /// head (no merge attempted).
    StrictFastForwardViolation {
        branch: String,
        expected: Option<LayerId>,
        actual: Option<LayerId>,
    },
}

impl std::fmt::Display for BranchUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BranchUpdateError::InvalidBranchName(n) => {
                write!(f, "invalid branch name: {n:?}")
            }
            BranchUpdateError::Storage(e) => write!(f, "storage error: {e}"),
            BranchUpdateError::StrictFastForwardViolation { branch, .. } => {
                write!(
                    f,
                    "strict fast-forward violation: branch {branch:?} is not at the expected head"
                )
            }
        }
    }
}

impl std::error::Error for BranchUpdateError {}

/// Process-wide branch CAS lock. v1 uses a single mutex — see module
/// docs for why. Lazily initialised on first use; `update_branch` is
/// the only caller, so contention is bounded by the surface area.
fn branch_lock() -> &'static Mutex<()> {
    use std::sync::OnceLock;
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Append a validated layer to the DAG.
///
/// Builds the `Layer` from `builder` (which already carries its parent
/// reference and accumulated resources), validates it against the
/// resolved chain, and persists it via the backend's atomic
/// `store_layer`. Returns the new `Arc<Layer>`. Does **not** touch any
/// branch ref — call `update_branch` separately to advance a branch
/// pointer.
///
/// `storage` flows into `LayerBuilder::build` and is the cache /
/// backend bundle the returned layer uses for resolves.
pub fn commit_layer(
    builder: LayerBuilder,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<Arc<Layer>, CommitError> {
    let layer = Arc::new(builder.build(storage));
    let validator = Validator::new(&layer);
    let errors = validator.validate();
    if !errors.is_empty() {
        return Err(CommitError::Validation(errors));
    }
    backend.store_layer(&layer).map_err(CommitError::Storage)?;
    Ok(layer)
}

/// Advance `branch` from `expected_old_head` to `new_head` via CAS.
///
/// `expected_old_head = None` creates a new branch (fails if one
/// already exists with that name). Returns the outcome describing what
/// happened: `FastForward` on a clean CAS, `NeedsWitnessedMerge` on
/// divergence (14d empties `conflicting_iris`; 14e populates it), or
/// `TrivialMerge` once 14e ships.
///
/// **Concurrency.** Acquires a process-wide branch mutex so concurrent
/// `update_branch` calls serialise. The caller's task runtime can be
/// async; this function is sync because the branch lock is sync and
/// branches are mutated rarely (commits, not reads).
pub fn update_branch(
    name: &str,
    expected_old_head: Option<LayerId>,
    new_head: LayerId,
    policy: ConflictPolicy,
    backend: &dyn PersistentBackend,
) -> Result<UpdateOutcome, BranchUpdateError> {
    if !is_valid_branch_name(name) {
        return Err(BranchUpdateError::InvalidBranchName(name.to_string()));
    }

    // Per-branch CAS via the global branch lock.
    let _guard = branch_lock().lock().expect("branch lock poisoned");

    let actual = backend
        .get_branch(name)
        .map_err(BranchUpdateError::Storage)?;

    if actual == expected_old_head {
        // CAS succeeded.
        backend
            .put_branch(name, &new_head)
            .map_err(BranchUpdateError::Storage)?;
        return Ok(UpdateOutcome::FastForward);
    }

    // Divergence — actual ≠ expected.
    match policy {
        ConflictPolicy::StrictFastForward => Err(BranchUpdateError::StrictFastForwardViolation {
            branch: name.to_string(),
            expected: expected_old_head,
            actual,
        }),
        ConflictPolicy::AllowTrivial => {
            // 14e fills in trivial-merge resolution and populates
            // `conflicting_iris`. For 14d we surface the divergence
            // unconditionally as NeedsWitnessedMerge; the caller can
            // see `current_head` differs from `expected_old_head` and
            // react.
            // No branch exists but expected was Some — caller's
            // expectation is stale (someone deleted the branch). Return
            // a sentinel zero LayerId; same shape as a resolved-vs-empty
            // case will land in 14e.
            let current_head = actual.unwrap_or(LayerId([0u8; 32]));
            Ok(UpdateOutcome::NeedsWitnessedMerge {
                current_head,
                conflicting_iris: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerStorage;
    use crate::ontology::resource::{Resource, Value};
    use crate::storage::memory::MemoryPersistentBackend;
    use crate::storage::ResourceBackend;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_resource(id: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String("v".into()),
        );
        r
    }

    /// Build a small root layer via the lattice commit primitive.
    fn commit_root(
        backend: &dyn PersistentBackend,
        name: &str,
        storage: LayerStorage,
    ) -> Arc<Layer> {
        let mut b = LayerBuilder::new(name, None);
        b.add_resource(make_resource("urn:eigenius:core:r"))
            .unwrap();
        commit_layer(b, storage, backend).unwrap()
    }

    #[test]
    fn commit_layer_persists_via_store_layer() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let layer = commit_root(&backend, "root", storage);

        // Layer is in the topology + bloom + resources.
        let topo = backend.load_topology().unwrap();
        assert!(topo.get_layer(layer.id()).is_some());
        assert!(backend.load_bloom(layer.id()).unwrap().is_some());
        assert!(backend
            .load_resource(layer.id(), &iri("urn:eigenius:core:r"))
            .is_some());
    }

    #[test]
    fn commit_layer_does_not_touch_branches() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let _layer = commit_root(&backend, "root", storage);

        // No branch was advanced by `commit_layer`. Branches are an
        // orthogonal surface.
        assert!(backend.list_branches().unwrap().is_empty());
    }

    #[test]
    fn update_branch_creates_new_branch() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let layer = commit_root(&backend, "root", storage);

        // Creating a new branch: expected_old_head = None.
        let outcome = update_branch(
            "main",
            None,
            layer.id().clone(),
            ConflictPolicy::AllowTrivial,
            &backend,
        )
        .unwrap();
        assert_eq!(outcome, UpdateOutcome::FastForward);

        assert_eq!(
            backend.get_branch("main").unwrap(),
            Some(layer.id().clone())
        );
    }

    #[test]
    fn update_branch_fast_forward() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", storage.clone());

        // Initial branch creation.
        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            &backend,
        )
        .unwrap();

        // Commit a child and fast-forward.
        let mut child_b = LayerBuilder::new("child", Some(Arc::clone(&root)));
        child_b
            .add_resource(make_resource("urn:eigenius:example:c"))
            .unwrap();
        let child = commit_layer(child_b, storage, &backend).unwrap();

        let outcome = update_branch(
            "main",
            Some(root.id().clone()),
            child.id().clone(),
            ConflictPolicy::AllowTrivial,
            &backend,
        )
        .unwrap();
        assert_eq!(outcome, UpdateOutcome::FastForward);
        assert_eq!(
            backend.get_branch("main").unwrap(),
            Some(child.id().clone())
        );
    }

    #[test]
    fn update_branch_divergence_returns_needs_witnessed_merge() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", storage.clone());

        // Branch starts at root.
        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            &backend,
        )
        .unwrap();

        // Two diverging children off root.
        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        a_b.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer(b_b, storage, &backend).unwrap();

        // Advance branch to `a`.
        update_branch(
            "main",
            Some(root.id().clone()),
            a.id().clone(),
            ConflictPolicy::AllowTrivial,
            &backend,
        )
        .unwrap();

        // Now try to advance to `b` claiming root was the parent — branch
        // moved to `a`, so this is divergence.
        let outcome = update_branch(
            "main",
            Some(root.id().clone()),
            b.id().clone(),
            ConflictPolicy::AllowTrivial,
            &backend,
        )
        .unwrap();
        match outcome {
            UpdateOutcome::NeedsWitnessedMerge {
                current_head,
                conflicting_iris,
            } => {
                assert_eq!(current_head, *a.id());
                // 14d leaves this empty; 14e fills it in.
                assert!(conflicting_iris.is_empty());
            }
            other => panic!("expected NeedsWitnessedMerge, got {other:?}"),
        }

        // Branch unchanged.
        assert_eq!(backend.get_branch("main").unwrap(), Some(a.id().clone()));
    }

    #[test]
    fn update_branch_strict_fast_forward_rejects_divergence() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", storage.clone());

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            &backend,
        )
        .unwrap();

        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        a_b.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer(a_b, storage.clone(), &backend).unwrap();
        update_branch(
            "main",
            Some(root.id().clone()),
            a.id().clone(),
            ConflictPolicy::AllowTrivial,
            &backend,
        )
        .unwrap();

        // Stale-expected against StrictFastForward → error, not outcome.
        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer(b_b, storage, &backend).unwrap();
        let err = update_branch(
            "main",
            Some(root.id().clone()),
            b.id().clone(),
            ConflictPolicy::StrictFastForward,
            &backend,
        )
        .unwrap_err();
        match err {
            BranchUpdateError::StrictFastForwardViolation {
                branch,
                expected,
                actual,
            } => {
                assert_eq!(branch, "main");
                assert_eq!(expected, Some(root.id().clone()));
                assert_eq!(actual, Some(a.id().clone()));
            }
            other => panic!("expected StrictFastForwardViolation, got {other:?}"),
        }
    }

    #[test]
    fn update_branch_rejects_invalid_names() {
        let backend = MemoryPersistentBackend::new();
        let id = LayerId([1u8; 32]);

        for bad in ["", "has space", "has/slash", "has.dot", &"x".repeat(257)] {
            let err = update_branch(
                bad,
                None,
                id.clone(),
                ConflictPolicy::AllowTrivial,
                &backend,
            )
            .unwrap_err();
            assert!(
                matches!(err, BranchUpdateError::InvalidBranchName(_)),
                "name {bad:?} should be rejected, got {err:?}"
            );
        }

        // Valid names (regex [A-Za-z0-9_-]+).
        for ok in ["main", "auto-divergent-1", "feature_x", "ABC123"] {
            let outcome =
                update_branch(ok, None, id.clone(), ConflictPolicy::AllowTrivial, &backend);
            assert!(outcome.is_ok(), "name {ok:?} should be accepted");
        }
    }

    #[test]
    fn update_branch_concurrent_cas_serialises() {
        // Two threads racing to update the same branch from the same
        // expected old; one wins (FastForward), the other sees
        // divergence (NeedsWitnessedMerge). The branch lock guarantees
        // exactly one CAS succeeds.
        use std::thread;

        let backend = Arc::new(MemoryPersistentBackend::new());
        let storage = LayerStorage::in_memory();
        let root = commit_root(backend.as_ref(), "root", storage.clone());

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            backend.as_ref(),
        )
        .unwrap();

        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        a_b.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer(a_b, storage.clone(), backend.as_ref()).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer(b_b, storage, backend.as_ref()).unwrap();

        let backend_a = Arc::clone(&backend);
        let root_id_a = root.id().clone();
        let a_id = a.id().clone();
        let t_a = thread::spawn(move || {
            update_branch(
                "main",
                Some(root_id_a),
                a_id,
                ConflictPolicy::AllowTrivial,
                backend_a.as_ref(),
            )
            .unwrap()
        });

        let backend_b = Arc::clone(&backend);
        let root_id_b = root.id().clone();
        let b_id = b.id().clone();
        let t_b = thread::spawn(move || {
            update_branch(
                "main",
                Some(root_id_b),
                b_id,
                ConflictPolicy::AllowTrivial,
                backend_b.as_ref(),
            )
            .unwrap()
        });

        let r_a = t_a.join().unwrap();
        let r_b = t_b.join().unwrap();

        // Exactly one fast-forward; the other is a divergence outcome.
        let ff_count = [&r_a, &r_b]
            .iter()
            .filter(|o| matches!(o, UpdateOutcome::FastForward))
            .count();
        let merge_count = [&r_a, &r_b]
            .iter()
            .filter(|o| matches!(o, UpdateOutcome::NeedsWitnessedMerge { .. }))
            .count();
        assert_eq!(
            ff_count, 1,
            "exactly one CAS must succeed (got {ff_count} FF, {merge_count} merge)"
        );
        assert_eq!(merge_count, 1);

        // The branch points at one of {a, b} — whichever won.
        let final_head = backend.get_branch("main").unwrap().unwrap();
        assert!(final_head == *a.id() || final_head == *b.id());
    }
}
