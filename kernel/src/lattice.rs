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

use crate::layer::{Layer, LayerBuilder, LayerError, LayerId, LayerStorage, LayerTopology};
use crate::ontology::iri::Iri;
use crate::storage::{PersistentBackend, StorageError};
use crate::validation::{ValidationError, Validator};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};

/// Ref-name validation: matches `[A-Za-z0-9_-]+`, max 256 chars.
///
/// Shared by branches (D23 §5.5) and tags (D34 §G.2 / §8) — same
/// lexical rules across both ref kinds so the picker, URL routing,
/// and validation messages don't have to diverge.
pub(crate) fn is_valid_ref_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Branch-name validation: matches `[A-Za-z0-9_-]+` per D23 §5.5.
fn is_valid_branch_name(name: &str) -> bool {
    is_valid_ref_name(name)
}

/// Errors from `commit_layer`.
#[derive(Debug)]
pub enum CommitError {
    /// One or more resources failed validation. `errors` is truncated to
    /// the policy's `max_violations` cap; `total_violations` reports the
    /// true count so callers can surface "showing X of Y."
    Validation {
        errors: Vec<ValidationError>,
        total_violations: usize,
    },
    /// Storage backend reported an error during the commit write.
    Storage(StorageError),
    /// The commit pipeline's [`crate::commit::LayerPersister`] returned
    /// an error. The new pipeline's persist seam (D41 §7) returns
    /// [`ValidationError`] rather than [`StorageError`] because Phase
    /// C will widen its impl to cover anchored-commit cache + branch
    /// CAS conditions that don't reduce cleanly to storage I/O. Phase
    /// B routes the lattice wrapper's [`crate::commit::BackendStorePersister`]
    /// errors through this variant.
    ///
    /// D41 Phase B.
    Persist(ValidationError),
    /// The builder rejected a resource (e.g., core-namespace violation
    /// on a non-root layer). Surfaced from `LayerBuilder::add_resource`
    /// callers; the lattice doesn't generate these itself but propagates
    /// them when the builder is constructed inline.
    Layer(LayerError),
    /// A working-set collection hit its capacity cap mid-commit. The
    /// commit was abandoned; nothing was written. Caller can either
    /// raise the cap (build a larger `CommitWorkingSet`) or back off.
    WorkingSetExhausted(crate::validation::WorkingSetExhausted),
    /// `CommitPolicy::CascadeTombstone` reached an iteration where it
    /// would have to tombstone an IRI the new layer itself defines, or
    /// where the cascade tombstones invalidated one of the new layer's
    /// own resources. Cascade can only suppress *lower-layer* IRIs;
    /// the caller is asking us to *add* their content, not remove it.
    /// The commit was abandoned; nothing was written.
    ///
    /// `errors` are the new-layer violations that triggered the abort.
    /// `cascade_tombstones` is the set the cascade had already
    /// accumulated when it hit the breakage — useful for the caller
    /// to see "you'd have to also explicitly tombstone these N IRIs
    /// to make this commit work."
    CascadeAbort {
        iterations: u32,
        cascade_tombstones: std::collections::BTreeSet<Iri>,
        errors: Vec<ValidationError>,
        total_violations: usize,
    },
    /// The orchestrator's FIFO drain queued an emission whose depth
    /// exceeded [`crate::commit::MAX_EMISSION_DEPTH`]. Catches a
    /// runaway phase or hook that emits unboundedly. Today's depth is
    /// at most 1 on every code path; hitting the cap is a programming
    /// error and the offending emission's `name` is surfaced for
    /// debuggability. D41 §6.3 / Phase D.
    EmissionDepthExceeded {
        depth: u32,
        layer_name: &'static str,
    },
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitError::Validation {
                errors,
                total_violations,
            } => {
                writeln!(
                    f,
                    "validation failed with {total_violations} error(s){}:",
                    if *total_violations > errors.len() {
                        format!(" (showing first {})", errors.len())
                    } else {
                        String::new()
                    }
                )?;
                for e in errors {
                    writeln!(f, "  {e}")?;
                }
                Ok(())
            }
            CommitError::Storage(e) => write!(f, "storage error during commit: {e}"),
            CommitError::Persist(e) => write!(f, "persist error during commit: {e}"),
            CommitError::Layer(e) => write!(f, "layer build error: {e}"),
            CommitError::WorkingSetExhausted(e) => {
                write!(f, "commit aborted: {e}")
            }
            CommitError::CascadeAbort {
                iterations,
                cascade_tombstones,
                errors,
                total_violations,
            } => {
                writeln!(
                    f,
                    "cascade aborted after {iterations} iteration(s); \
                     {total_violations} new-layer violation(s){}; \
                     cascade had accumulated {} tombstone(s) before abort:",
                    if *total_violations > errors.len() {
                        format!(" (showing first {})", errors.len())
                    } else {
                        String::new()
                    },
                    cascade_tombstones.len()
                )?;
                for e in errors {
                    writeln!(f, "  {e}")?;
                }
                Ok(())
            }
            CommitError::EmissionDepthExceeded { depth, layer_name } => {
                write!(
                    f,
                    "commit orchestrator emission depth cap exceeded at depth {depth} \
                     (layer `{layer_name}`); MAX_EMISSION_DEPTH={}",
                    crate::commit::MAX_EMISSION_DEPTH
                )
            }
        }
    }
}

impl std::error::Error for CommitError {}

/// Policy that controls how `commit_layer` handles violations
/// discovered during the retroactive validation pass (i.e., when the
/// new layer's effect on lower-layer resources causes them to fail
/// validation).
///
/// The retroactive pass itself lands in Phase 2; this enum defines
/// the contract that pass will obey.
#[derive(Debug, Clone)]
pub enum CommitPolicy {
    /// Reject the commit if any retroactive violation is found. Up to
    /// `max_violations` errors are surfaced; the rest are counted but
    /// dropped (see [`CommitError::Validation::total_violations`]).
    Reject { max_violations: usize },
    /// Tombstone violating lower-layer resources iteratively until no
    /// further resources are invalid. Each tombstone may cascade — a
    /// resource that referenced the tombstoned IRI now also violates
    /// and joins the tombstone set. The cascade aborts (rejects the
    /// commit) if it would tombstone an IRI that the new layer
    /// itself defines, since the caller is asking us to *add* that
    /// IRI, not remove it.
    CascadeTombstone,
}

impl Default for CommitPolicy {
    fn default() -> Self {
        Self::Reject {
            max_violations: 100,
        }
    }
}

/// Result of a successful `commit_layer`.
#[derive(Debug)]
pub struct CommitOutcome {
    /// The newly-committed layer. Either freshly created from the
    /// builder (most cases) or the same layer plus a cascade-added
    /// tombstone set (under [`CommitPolicy::CascadeTombstone`]).
    pub layer: Arc<Layer>,
    /// IRIs the cascade tombstoned in addition to whatever the
    /// caller's builder already tombstoned. Always empty under
    /// `CommitPolicy::Reject`. Always empty in Phase 1 (the
    /// retroactive pass + cascade arrives in Phase 2/3).
    pub cascade_tombstones: std::collections::BTreeSet<Iri>,
    /// Number of fixpoint iterations the cascade needed. `0` means
    /// no cascade ran (either because the policy was `Reject` or
    /// because no retroactive violations were found).
    pub cascade_iterations: u32,
}

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
    ///
    /// `orphan_head` is the `new_head` the caller passed in — the layer
    /// they built that didn't make it onto the branch. The notebook's
    /// witnessed-merge recovery dialog (D34 §6.2) uses it to offer
    /// "Save my work as a sibling branch": `CreateBranch(name,
    /// orphan_head)` keeps the layer reachable until the user decides
    /// what to do with it. Without this, the orphan would only be
    /// findable by hash and would become GC-eligible the moment
    /// nothing referenced it.
    NeedsWitnessedMerge {
        current_head: LayerId,
        conflicting_iris: Vec<Iri>,
        orphan_head: LayerId,
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

/// Process-wide *snapshot* gate. Held in:
///
/// - **read mode** by per-branch CAS operations (`update_branch`,
///   `prune_branch`, `merge_branch_tips`). Multiple operations on
///   *different* branches proceed concurrently — read locks are
///   shared. Two operations on the same branch then serialize on
///   that branch's per-branch lock (see [`branch_slot`]).
/// - **write mode** by snapshot operations ([`with_branch_lock`]).
///   Blocks all in-flight per-branch CAS until released. GC's mark
///   phase uses this to read every branch ref atomically.
///
/// The gate gives `with_branch_lock` a coherent view *without*
/// requiring it to acquire every per-branch lock individually —
/// cheaper and immune to "new branch created mid-snapshot" races.
fn snapshot_gate() -> &'static RwLock<()> {
    use std::sync::OnceLock;
    static GATE: OnceLock<RwLock<()>> = OnceLock::new();
    GATE.get_or_init(|| RwLock::new(()))
}

/// Per-branch CAS slot. Lazily created on first reference; subsequent
/// lookups hit the same `Arc<Mutex<()>>` so concurrent operations on
/// the same branch serialize. Operations on different branches get
/// different slots and run in parallel.
///
/// The outer `Mutex<HashMap>` is contended only during slot creation
/// and lookup (microseconds); per-branch CAS holds the inner slot's
/// `Mutex<()>` for the duration of its sync work (~10ms RocksDB
/// fsync).
fn branch_slot(name: &str) -> Arc<Mutex<()>> {
    use std::sync::OnceLock;
    static SLOTS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let slots = SLOTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = slots.lock().expect("branch slot map poisoned");
    Arc::clone(
        guard
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}

/// Outcome of `prune_branch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneOutcome {
    /// Branch was deleted; previous head is returned so the caller
    /// can pass it to `gc::collect` or otherwise decide what to do
    /// (e.g., display "branch X pointed at L; layers reachable only
    /// via X will be reclaimed on the next GC pass").
    Pruned { previous_head: LayerId },
    /// Branch didn't exist; nothing was done.
    NotFound,
}

/// Safety policy for `prune_branch`.
pub enum PruneSafety<'a> {
    /// Reject if any pin in `active_pins` equals the branch's current
    /// head. The caller is expected to populate this from the task
    /// store (running tasks' `TaskRecord.layer_head` pins). The check
    /// is conservative — a task pinned at the branch head suggests
    /// "someone is actively working off this branch as their starting
    /// point." Note that even with `Force`, task-pinned layers
    /// survive subsequent GC because the pin is itself a GC root;
    /// the safety check is about preserving the branch *label*, not
    /// preventing data loss.
    CheckPins(&'a [LayerId]),
    /// Skip the safety check; just delete the branch ref. The caller
    /// has decided that any active sessions are on their own.
    Force,
}

/// Errors from `prune_branch`.
#[derive(Debug)]
pub enum PruneError {
    /// Branch name fails the regex `[A-Za-z0-9_-]+` (or is too long).
    InvalidBranchName(String),
    /// Storage backend reported an error during read or write.
    Storage(StorageError),
    /// `CheckPins` policy and the branch's current head matches an
    /// active task pin. The branch is unchanged.
    InUse { branch: String, head: LayerId },
}

impl std::fmt::Display for PruneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PruneError::InvalidBranchName(n) => write!(f, "invalid branch name: {n:?}"),
            PruneError::Storage(e) => write!(f, "storage error: {e}"),
            PruneError::InUse { branch, head } => {
                write!(
                    f,
                    "branch {branch:?} is in use (head {head} matches an active task pin)"
                )
            }
        }
    }
}

impl std::error::Error for PruneError {}

/// Remove a branch ref. The layers it pointed at remain in the DAG
/// until the next `gc::collect` reclaims layers reachable only through
/// this branch.
///
/// **Phase 14g.** Sister operation to `update_branch`; same name
/// validation, same branch lock for serialization. The actual layer
/// reclamation is GC's job — pruning a branch just removes the label.
///
/// `safety` controls whether the kernel rejects pruning a branch that
/// matches an active task pin. See [`PruneSafety`] for the policies.
pub fn prune_branch(
    name: &str,
    safety: PruneSafety<'_>,
    backend: &dyn PersistentBackend,
) -> Result<PruneOutcome, PruneError> {
    if !is_valid_branch_name(name) {
        return Err(PruneError::InvalidBranchName(name.to_string()));
    }

    // Per-branch CAS pattern: snapshot gate in read mode (lets
    // concurrent prunes/updates on *other* branches proceed) +
    // exclusive per-branch lock for this branch.
    let _snapshot = snapshot_gate()
        .read()
        .expect("snapshot gate poisoned (read)");
    let slot = branch_slot(name);
    let _guard = slot.lock().expect("branch slot poisoned");

    let head = match backend.get_branch(name).map_err(PruneError::Storage)? {
        Some(h) => h,
        None => return Ok(PruneOutcome::NotFound),
    };

    if let PruneSafety::CheckPins(pins) = safety {
        if pins.contains(&head) {
            return Err(PruneError::InUse {
                branch: name.to_string(),
                head,
            });
        }
    }

    backend.delete_branch(name).map_err(PruneError::Storage)?;

    Ok(PruneOutcome::Pruned {
        previous_head: head,
    })
}

/// Run `f` while holding the snapshot gate in exclusive (write) mode.
/// Phase 14f's GC uses this to take a consistent snapshot of branch
/// refs at the start of its mark phase — no `update_branch`,
/// `prune_branch`, or `merge_branch_tips` can be in flight while `f`
/// runs, so the snapshot is coherent across all branches. The gate is
/// released when `f` returns; GC's mark + sweep work happens outside
/// the gate (concurrent commits and updates are safe per the min-age
/// contract).
///
/// `f` should be brief (read branch refs into memory, return). Long
/// work inside the closure blocks all per-branch CAS operations.
///
/// **Why a write-mode lock instead of acquiring every per-branch
/// lock.** A snapshot needs an instant where *no* per-branch CAS is
/// partway through. The write-mode acquisition of the snapshot gate
/// gives that for free: in-flight CAS operations hold the gate in
/// read mode, so they all drain before the write-mode acquisition
/// completes, and new CAS attempts block on the pending writer until
/// `f` returns. Iterating per-branch locks instead would race against
/// "branch created mid-snapshot" — a new branch's lock didn't exist
/// at iteration start, so the snapshot would miss its CAS.
pub fn with_branch_lock<R>(f: impl FnOnce() -> R) -> R {
    let _guard = snapshot_gate()
        .write()
        .expect("snapshot gate poisoned (write)");
    f()
}

/// Append a validated layer to the DAG.
///
/// Thin wrapper around [`crate::commit::CommitPipeline::with_retroactive`]
/// (D41 Phase B). Builds the `Layer` from `builder`, runs structural
/// validation, runs the retroactive validation pass against lower-layer
/// resources affected by the new content per `policy`, and persists via
/// `backend.store_layer` through the
/// [`crate::commit::BackendStorePersister`] adapter. Does **not** touch
/// any branch ref — call `update_branch` separately.
///
/// `working_set` holds the scratch collections the retroactive pass
/// uses. Callers driving many commits should reuse one via
/// [`crate::validation::CommitWorkingSetPool`]; single-shot callers
/// can pass a fresh `CommitWorkingSet::in_memory()`.
///
/// **Return shape preserved.** The pipeline produces a
/// [`crate::commit::LayerCommitOutcome`]; this wrapper translates back
/// to the legacy [`CommitOutcome`] so all existing callers (lattice
/// tests, gc.rs, bootstrap, storage E2E tests) compile unchanged.
/// D41 §11 retires this wrapper in a later phase.
///
/// D41 §11 / Phase B.
pub fn commit_layer(
    builder: LayerBuilder,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
    policy: CommitPolicy,
    working_set: &mut crate::validation::CommitWorkingSet,
) -> Result<CommitOutcome, CommitError> {
    // D41 §11.1: the lattice wrapper delegates to the new pipeline via
    // a minimal `LayerPersister` adapter that just calls
    // `backend.store_layer`. No CAS, no cache — the lattice path has
    // never owned those.
    let persister = crate::commit::BackendStorePersister { backend };
    // D41 Phase D: the lattice path doesn't run `with_institutions` and
    // doesn't trigger any `didPersist` hook, but `PipelineConfig` still
    // requires a `host`. NoopHost satisfies the trait without pulling
    // in service state.
    let host = crate::commit::hooks::NoopHost;
    let cfg = crate::commit::PipelineConfig {
        persister: &persister,
        host: &host,
        // Lattice path is branch-agnostic. `"main"` is a placeholder;
        // `BackendStorePersister` ignores it.
        branch: "main",
        policy,
        institutions: None,
        storage,
    };
    // Stable diagnostic name for the lattice wrapper's single-shot run;
    // the lattice path is not driven by an orchestrator, so this name
    // never reaches a `MultiLayerOutcome` consumer — it only flows into
    // the `LayerCommitOutcome` we immediately decompose below.
    let outcome = crate::commit::CommitPipeline::with_retroactive()
        .run(
            "lattice_commit",
            crate::commit::LayerRole::User,
            builder,
            cfg,
            working_set,
        )
        // D41 Phase D: the lattice wrapper doesn't run an orchestrator,
        // so any rescued Sibling emissions in `PipelineRunErr` are dead
        // — drop them and surface only the inner `CommitError`.
        .map_err(|e| e.error)?;
    Ok(CommitOutcome {
        layer: outcome.layer,
        cascade_tombstones: outcome.cascade_tombstones,
        cascade_iterations: outcome.cascade_iterations,
    })
}

/// Convenience wrapper: commits with [`CommitPolicy::default()`] and a
/// freshly-allocated in-memory working set, returning just the
/// `Arc<Layer>` for callers that don't need the full outcome.
///
/// Equivalent to:
///
/// ```text
/// let mut ws = crate::validation::CommitWorkingSet::in_memory();
/// commit_layer(builder, storage, backend, CommitPolicy::default(), &mut ws)
///     .map(|outcome| outcome.layer)
/// ```
///
/// Suitable for tests, bootstrap, CLI commands, and any caller that
/// doesn't care about policy or cascade results. The gRPC server and
/// other long-running callers should use [`commit_layer`] directly
/// with a pooled working set.
pub fn commit_layer_default(
    builder: LayerBuilder,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<Arc<Layer>, CommitError> {
    let mut ws = crate::validation::CommitWorkingSet::in_memory();
    commit_layer(builder, storage, backend, CommitPolicy::default(), &mut ws)
        .map(|outcome| outcome.layer)
}

/// Outcome of [`commit_layer_with_cache`] (D33 §6 / Phase 20c).
///
/// `Hit` means an existing layer with byte-equal content and
/// byte-equal supporting-layer content was already in storage; no new
/// commit was performed. `Miss` means the standard commit path ran
/// and the cache was updated.
#[derive(Debug)]
pub enum AnchoredCommitOutcome {
    /// A previously-committed layer with byte-equal content and
    /// supporting-context content was found in the anchored-commit
    /// cache. The cached `LayerId` is returned; the caller can
    /// reconstruct an `Arc<Layer>` via
    /// [`PersistentBackend::load_chain_from`] +
    /// [`crate::layer::build_chain`] if they need one.
    Hit {
        cached_layer_id: crate::layer::LayerId,
    },
    /// No cache hit. The layer was built, validated, stored, and the
    /// cache was updated. Returns the freshly-committed `Arc<Layer>`.
    Miss { layer: Arc<Layer> },
}

/// Commit a layer with anchored-commit-cache lookup (D33 §6 / Phase 20c).
///
/// The notebook-style commit path: before persisting, probe the
/// cache keyed on `(content_hash, supporting_layer_content_hash)`. A
/// hit returns the cached `LayerId` without committing — the cell's
/// output is structurally identical to a previous run against an
/// equivalent supporting context, so the existing layer is the
/// canonical record.
///
/// **Why validation is skipped on cache hits.** The cache key
/// covers both the content and the supporting context. Two layers
/// with byte-equal content and supporting layers whose content
/// hashes match resolve every reference through the same ancestor
/// closure: if the cached commit passed validation, so does this
/// one. Skipping the validator on hit saves the work entirely.
///
/// **Layers with no supporting layer** (pure root-content commits,
/// fully self-referential layers) bypass the cache: there's no
/// supporting context to key on. Such commits go through the
/// standard commit path and return `Miss`.
///
/// The wrapper does not advance any branch ref — the caller invokes
/// [`update_branch`] separately, with the cached or fresh
/// `LayerId`, exactly as they would for `commit_layer`.
pub fn commit_layer_with_cache(
    builder: LayerBuilder,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<AnchoredCommitOutcome, CommitError> {
    let layer = Arc::new(builder.build(storage));

    // No supporting layer → no cache check (cache key requires both
    // the content and the supporting context).
    let Some(supporting_id) = layer.supporting_layer().cloned() else {
        return commit_without_cache(layer, backend);
    };

    // Resolve the supporting layer's content_hash via the backend's
    // single-handle lookup. Cheap — one O(1) probe, no full-topology
    // load.
    let supporting_handle = backend
        .load_handle(&supporting_id)
        .map_err(CommitError::Storage)?
        .ok_or_else(|| {
            CommitError::Storage(crate::storage::StorageError::Internal(format!(
                "supporting layer {supporting_id} absent from topology at commit time"
            )))
        })?;
    let supporting_content = supporting_handle.content_hash;

    // Cache probe.
    if let Some(cached_id) = backend
        .lookup_anchored_commit(layer.content_hash(), &supporting_content)
        .map_err(CommitError::Storage)?
    {
        return Ok(AnchoredCommitOutcome::Hit {
            cached_layer_id: cached_id,
        });
    }

    // Cache miss: standard validate + store, then insert.
    let validator = Validator::new(Arc::clone(&layer));
    let errors = validator.validate();
    if !errors.is_empty() {
        let total = errors.len();
        return Err(CommitError::Validation {
            errors,
            total_violations: total,
        });
    }
    backend.store_layer(&layer).map_err(CommitError::Storage)?;
    backend
        .put_anchored_commit(layer.content_hash(), &supporting_content, layer.id())
        .map_err(CommitError::Storage)?;
    Ok(AnchoredCommitOutcome::Miss { layer })
}

/// Helper for the "no supporting layer" branch of
/// `commit_layer_with_cache`. Mirrors `commit_layer` but returns
/// the wrapped `AnchoredCommitOutcome::Miss`.
fn commit_without_cache(
    layer: Arc<Layer>,
    backend: &dyn PersistentBackend,
) -> Result<AnchoredCommitOutcome, CommitError> {
    let validator = Validator::new(Arc::clone(&layer));
    let errors = validator.validate();
    if !errors.is_empty() {
        let total = errors.len();
        return Err(CommitError::Validation {
            errors,
            total_violations: total,
        });
    }
    backend.store_layer(&layer).map_err(CommitError::Storage)?;
    Ok(AnchoredCommitOutcome::Miss { layer })
}

/// Advance `branch` from `expected_old_head` to `new_head` via CAS.
///
/// `expected_old_head = None` creates a new branch (fails if one
/// already exists with that name). Returns the outcome describing what
/// happened: `FastForward` on a clean CAS, `TrivialMerge` if the
/// branch's actual head and `new_head` modify disjoint IRIs since their
/// LCA (Phase 14e), or `NeedsWitnessedMerge` on genuine conflict.
///
/// **Storage parameter.** The trivial-merge path needs a `LayerStorage`
/// to construct the merge layer (cache + bloom + backend bundle). The
/// FastForward and StrictFastForward paths don't use it; pass any
/// valid storage (typically `LayerStorage::with_persistent(backend)`).
///
/// **Concurrency.** Two-level locking: takes the snapshot gate in
/// read mode (shared with other branches' updates; only blocks during
/// a GC snapshot) and the per-branch CAS slot in exclusive mode.
/// Concurrent `update_branch` calls on *different* branches proceed
/// in parallel; concurrent calls on the *same* branch serialize.
/// The function is sync because the locks are sync; gRPC handlers
/// wrap the call in `tokio::task::spawn_blocking` to keep tokio
/// worker threads off the disk-I/O critical path.
pub fn update_branch(
    name: &str,
    expected_old_head: Option<LayerId>,
    new_head: LayerId,
    policy: ConflictPolicy,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<UpdateOutcome, BranchUpdateError> {
    if !is_valid_branch_name(name) {
        return Err(BranchUpdateError::InvalidBranchName(name.to_string()));
    }

    // Snapshot gate (shared) + per-branch CAS slot (exclusive).
    let _snapshot = snapshot_gate()
        .read()
        .expect("snapshot gate poisoned (read)");
    let slot = branch_slot(name);
    let _guard = slot.lock().expect("branch slot poisoned");

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

    // Divergence — actual ≠ expected. The branch is somewhere other
    // than where the caller started from.
    match policy {
        ConflictPolicy::StrictFastForward => Err(BranchUpdateError::StrictFastForwardViolation {
            branch: name.to_string(),
            expected: expected_old_head,
            actual,
        }),
        ConflictPolicy::AllowTrivial => {
            // Stash a copy of the caller's `new_head` so we can
            // surface it as `orphan_head` if the merge attempt
            // doesn't put it on a branch. `merge_independent_heads`
            // consumes its input vec, so this clone has to happen
            // before we hand it over.
            let orphan_head = new_head.clone();
            // If the branch was deleted (actual is None), trivial
            // merge has nothing to merge against — surface as a
            // witnessed-merge requirement.
            let actual_head = match actual {
                Some(h) => h,
                None => {
                    return Ok(UpdateOutcome::NeedsWitnessedMerge {
                        current_head: LayerId([0u8; 32]),
                        conflicting_iris: Vec::new(),
                        orphan_head,
                    });
                }
            };
            // Attempt N=2 trivial merge between branch's actual head
            // and the caller's new_head.
            match merge_independent_heads(vec![actual_head.clone(), new_head], storage, backend) {
                Ok(MergeOutcome::Merged { merge_layer }) => {
                    // CAS the branch to the merge layer.
                    backend
                        .put_branch(name, merge_layer.id())
                        .map_err(BranchUpdateError::Storage)?;
                    Ok(UpdateOutcome::TrivialMerge {
                        merge_layer: merge_layer.id().clone(),
                    })
                }
                Ok(MergeOutcome::Conflict { conflicting_iris }) => {
                    Ok(UpdateOutcome::NeedsWitnessedMerge {
                        current_head: actual_head,
                        conflicting_iris,
                        orphan_head,
                    })
                }
                Err(MergeError::Storage(e)) => Err(BranchUpdateError::Storage(e)),
                Err(MergeError::InvalidHeads(msg)) => Err(BranchUpdateError::Storage(
                    StorageError::Internal(format!("merge during update_branch: {msg}")),
                )),
                Err(MergeError::Validation(v)) => {
                    Err(BranchUpdateError::Storage(StorageError::Internal(format!(
                        "merge layer failed validation ({} errors)",
                        v.len()
                    ))))
                }
            }
        }
    }
}

/// Fold `source_tip` into the branch named `target`.
///
/// Unlike [`update_branch`], this does **not** assume `source_tip` is
/// built on top of the target's current tip. It computes the LCA of
/// the two tips and routes between three outcomes:
///
/// - `LCA == target_tip` → `source_tip` descends from the target —
///   advance the branch to `source_tip` (true fast-forward).
/// - `LCA == source_tip` → the target already includes `source_tip` —
///   no-op fast-forward.
/// - Otherwise → genuinely divergent: dispatch to
///   [`merge_independent_heads`]. On disjoint contributions, build a
///   multi-parent merge layer and CAS the branch; on overlap, return
///   `NeedsWitnessedMerge` with `source_tip` as `orphan_head` (the
///   D36 resolution flow feeds this in as `candidate_head`).
///
/// `update_branch`'s CAS shortcut is correct for cell-commit callers
/// (the cell pipeline guarantees `new_head` descends from
/// `expected_old_head`) but destructive for cross-branch merges where
/// `source_tip` can be a sibling head touching the same IRIs as the
/// target. Use this function for the cross-branch case.
///
/// Takes the same two-level lock as [`update_branch`] (snapshot gate
/// read + per-branch CAS slot on `target`). Concurrent
/// `merge_branch_tips` calls into different target branches proceed
/// in parallel; into the same target branch they serialize. GC's
/// `with_branch_lock` blocks both.
pub fn merge_branch_tips(
    target: &str,
    source_tip: LayerId,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<UpdateOutcome, BranchUpdateError> {
    if !is_valid_branch_name(target) {
        return Err(BranchUpdateError::InvalidBranchName(target.to_string()));
    }

    let _snapshot = snapshot_gate()
        .read()
        .expect("snapshot gate poisoned (read)");
    let slot = branch_slot(target);
    let _guard = slot.lock().expect("branch slot poisoned");

    let target_tip = backend
        .get_branch(target)
        .map_err(BranchUpdateError::Storage)?
        .ok_or_else(|| {
            BranchUpdateError::Storage(StorageError::Internal(format!(
                "merge_branch_tips: target branch {target:?} not found"
            )))
        })?;

    if target_tip == source_tip {
        return Ok(UpdateOutcome::FastForward);
    }

    let topology = backend
        .load_topology()
        .map_err(BranchUpdateError::Storage)?;
    let lca = find_lca(&[target_tip.clone(), source_tip.clone()], &topology).ok_or_else(|| {
        BranchUpdateError::Storage(StorageError::Internal(
            "merge_branch_tips: target and source share no common ancestor".into(),
        ))
    })?;

    if lca == target_tip {
        backend
            .put_branch(target, &source_tip)
            .map_err(BranchUpdateError::Storage)?;
        return Ok(UpdateOutcome::FastForward);
    }
    if lca == source_tip {
        return Ok(UpdateOutcome::FastForward);
    }

    match merge_independent_heads(
        vec![target_tip.clone(), source_tip.clone()],
        storage,
        backend,
    ) {
        Ok(MergeOutcome::Merged { merge_layer }) => {
            backend
                .put_branch(target, merge_layer.id())
                .map_err(BranchUpdateError::Storage)?;
            Ok(UpdateOutcome::TrivialMerge {
                merge_layer: merge_layer.id().clone(),
            })
        }
        Ok(MergeOutcome::Conflict { conflicting_iris }) => Ok(UpdateOutcome::NeedsWitnessedMerge {
            current_head: target_tip,
            conflicting_iris,
            orphan_head: source_tip,
        }),
        Err(MergeError::Storage(e)) => Err(BranchUpdateError::Storage(e)),
        Err(MergeError::InvalidHeads(msg)) => Err(BranchUpdateError::Storage(
            StorageError::Internal(format!("merge during merge_branch_tips: {msg}")),
        )),
        Err(MergeError::Validation(v)) => Err(BranchUpdateError::Storage(StorageError::Internal(
            format!("merge layer failed validation ({} errors)", v.len()),
        ))),
    }
}

// --- Topology analysis (Phase 14e-ii): LCA + change-set computation ---

/// Lowest common ancestor of N heads in the layer DAG.
///
/// Returns `None` only if `heads` is empty or if any head is unknown to
/// the topology. Otherwise returns the deepest layer that is an
/// ancestor of every head. Because every layer in an Eigenius DB
/// descends from the bootstrap chain (core → program → reflection →
/// institution → notebook), the LCA always exists for any pair of
/// known layers — there's no "disjoint DAGs" edge case to worry about.
///
/// **Algorithm.** Standard multi-source BFS over `LayerHandle.parents`.
/// For each head, BFS from it collecting its ancestor set. The LCA is
/// the layer that:
/// 1. appears in every head's ancestor set (common ancestor), and
/// 2. has no descendant in that intersection (lowest).
///
/// Operates over the in-memory `LayerTopology` snapshot rather than
/// streaming through `PersistentBackend` calls, which is fine because
/// the topology is bounded by layer count (typically 10²–10⁴) and lives
/// entirely in RAM per Phase 14a.
///
/// `heads` may include the same id multiple times (e.g., a head merged
/// with itself); duplicates are deduplicated. A single-head input
/// returns that head unchanged.
pub fn find_lca(heads: &[LayerId], topology: &LayerTopology) -> Option<LayerId> {
    let unique: BTreeSet<&LayerId> = heads.iter().collect();
    if unique.is_empty() {
        return None;
    }
    if unique.len() == 1 {
        // LCA of a single head is the head itself, but only if it's in
        // the topology — otherwise None per the trait contract.
        let only = *unique.iter().next().unwrap();
        return topology.get_layer(only).map(|_| only.clone());
    }

    // Compute each head's ancestor set (including the head itself).
    let mut ancestor_sets: Vec<BTreeSet<LayerId>> = Vec::with_capacity(unique.len());
    for head in &unique {
        let mut visited: BTreeSet<LayerId> = BTreeSet::new();
        let mut queue: VecDeque<LayerId> = VecDeque::new();
        queue.push_back((*head).clone());
        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }
            // Walk parents via topology. Unknown layers terminate that path.
            if let Some(handle) = topology.get_layer(&id) {
                for parent in &handle.parents {
                    queue.push_back(parent.clone());
                }
            }
        }
        // If a head wasn't in the topology its ancestor set is empty.
        if visited.is_empty() {
            return None;
        }
        ancestor_sets.push(visited);
    }

    // Common ancestors = intersection of all heads' ancestor sets.
    let mut common = ancestor_sets.swap_remove(0);
    for other in &ancestor_sets {
        common = common.intersection(other).cloned().collect();
    }
    if common.is_empty() {
        // Per the bootstrap-chain invariant this shouldn't happen for
        // layers persisted via the kernel's normal bootstrap path.
        // Defensive: return None rather than picking a non-LCA.
        return None;
    }

    // Lowest = the deepest common ancestor = the one that descends from
    // every other common ancestor. Equivalently: the candidate whose
    // own ancestor set (walking parents up to the root) contains every
    // other common ancestor.
    //
    // For a chain root → a → b: commons are {b, a, root}; b descends
    // from a and root, so b's ancestor set is {b, a, root} which
    // contains all commons → b is the LCA.
    for candidate in &common {
        let mut visited: BTreeSet<LayerId> = BTreeSet::new();
        let mut queue: VecDeque<LayerId> = VecDeque::new();
        queue.push_back(candidate.clone());
        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }
            if let Some(handle) = topology.get_layer(&id) {
                for parent in &handle.parents {
                    queue.push_back(parent.clone());
                }
            }
        }
        // `candidate` is the LCA iff every other common ancestor is in
        // its ancestor set (i.e., candidate is a descendant of each).
        if common.iter().all(|c| visited.contains(c)) {
            return Some(candidate.clone());
        }
    }
    None
}

/// For each IRI touched between `head` and `ancestor` (exclusive of
/// `ancestor`), the topmost layer in `[head, ancestor)` that defines
/// it.
///
/// This is the data structure trivial-merge needs: the keys give the
/// touched-IRI set (used for the pairwise-disjoint check), and the
/// values give the source layer to load the top-of-stack resource
/// from when constructing the merge content.
///
/// **Algorithm.** Top-down BFS from `head`, walking `topology.parents`,
/// terminating each path when it reaches `ancestor`. At each visited
/// layer, fetch its `defined_iris` via `list_layer_iris`; for each
/// IRI not yet seen, record the current layer as its source. Because
/// the BFS is roughly head→root order (top-down), the first layer to
/// define an IRI is the topmost one in `[head, ancestor)`, which is
/// exactly the resolve-equivalent value at the head.
///
/// Multi-parent merge layers (Phase 14e) along the walk contribute all
/// their parents into the BFS frontier — this correctly captures every
/// IRI touched in the divergence region regardless of merge structure.
///
/// Returns `Ok(empty_map)` if `head == ancestor`. Returns
/// `Err(StorageError)` if listing IRIs from the backend fails.
///
/// **Tombstones are not collected here.** A tombstone is a
/// visibility-modifying change since the LCA but not a definition with
/// a body. Callers that need both — e.g., merge conflict detection —
/// pair this with [`iri_tombstones_since`].
pub fn iri_sources_since(
    head: &LayerId,
    ancestor: &LayerId,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<std::collections::BTreeMap<Iri, LayerId>, StorageError> {
    use crate::storage::ResourceBackend;

    let mut sources: std::collections::BTreeMap<Iri, LayerId> = std::collections::BTreeMap::new();
    if head == ancestor {
        return Ok(sources);
    }

    // BFS in roughly head→root order. BFS doesn't strictly preserve
    // depth ordering when multi-parent merges are present, so we tag
    // each enqueued layer with its discovery depth and let the first
    // (lowest-depth) sighting of an IRI win.
    let mut visited: BTreeSet<LayerId> = BTreeSet::new();
    let mut queue: VecDeque<(LayerId, u32)> = VecDeque::new();
    queue.push_back((head.clone(), 0));

    // Track each IRI's first-seen depth so deeper sightings don't
    // overwrite shallower (topmost) ones.
    let mut iri_depth: std::collections::BTreeMap<Iri, u32> = std::collections::BTreeMap::new();

    while let Some((id, depth)) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if id == *ancestor {
            // Stop at the ancestor — its IRIs are not "since" itself.
            continue;
        }
        let layer_iris = ResourceBackend::list_layer_iris(backend, &id)?;
        for iri in layer_iris {
            // Only insert if we haven't seen this IRI at a shallower
            // (i.e., topologically more recent) depth.
            let existing = iri_depth.get(&iri).copied();
            if existing.is_none_or(|d| depth < d) {
                iri_depth.insert(iri.clone(), depth);
                sources.insert(iri, id.clone());
            }
        }
        if let Some(handle) = topology.get_layer(&id) {
            for parent in &handle.parents {
                if !visited.contains(parent) {
                    queue.push_back((parent.clone(), depth + 1));
                }
            }
        }
    }

    Ok(sources)
}

/// For each IRI tombstoned between `head` and `ancestor` (exclusive of
/// `ancestor`), the result set contains that IRI if the tombstone is
/// the *top-of-stack* modification — i.e., no layer between the
/// tombstoning layer and `head` redefines the IRI.
///
/// This is the tombstone-side companion to [`iri_sources_since`]: the
/// two together describe every modification a branch makes to chain
/// visibility since the LCA. Merge conflict detection needs both — a
/// definition on one side and a tombstone on the other is a real
/// conflict that trivial merge can't reconcile.
///
/// **Algorithm.** Head→root BFS over `LayerHandle.parents`, walking
/// each visited layer's `tombstoned_iris` and accumulating the union.
/// Definition-vs-tombstone shadowing (an in-branch redefinition
/// hiding a deeper tombstone) is deferred to the caller, which
/// already has the branch's defines map from `iri_sources_since`.
/// This function returns the *raw* tombstone union; the merge driver
/// removes tombstones whose IRI also appears in the defines map.
///
/// Returns the deduplicated set of IRIs tombstoned anywhere in
/// `[head, ancestor)`. Returns an empty set if `head == ancestor`.
pub fn iri_tombstones_since(
    head: &LayerId,
    ancestor: &LayerId,
    topology: &LayerTopology,
) -> BTreeSet<Iri> {
    let mut tombstones: BTreeSet<Iri> = BTreeSet::new();
    if head == ancestor {
        return tombstones;
    }

    let mut visited: BTreeSet<LayerId> = BTreeSet::new();
    let mut queue: VecDeque<LayerId> = VecDeque::new();
    queue.push_back(head.clone());

    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if id == *ancestor {
            continue;
        }
        if let Some(handle) = topology.get_layer(&id) {
            for tomb in &handle.tombstoned_iris {
                tombstones.insert(tomb.clone());
            }
            for parent in &handle.parents {
                if !visited.contains(parent) {
                    queue.push_back(parent.clone());
                }
            }
        }
    }

    tombstones
}

// --- Trivial merge (Phase 14e-iii) ---

/// Outcome of `merge_independent_heads`.
#[derive(Debug)]
pub enum MergeOutcome {
    /// All heads' contributions since their LCA are pairwise disjoint;
    /// the kernel built and persisted a multi-parent layer with
    /// `parents = heads` (sorted) and content = union of contributions.
    Merged { merge_layer: Arc<Layer> },
    /// Two or more heads modify the same IRI(s) since their LCA;
    /// reconciliation requires Phase 15 witnessed merge.
    Conflict { conflicting_iris: Vec<Iri> },
}

/// Side-effect-free verdict of a merge attempt. Returned by
/// [`preview_merge_independent_heads`]; mirrors the [`MergeOutcome`]
/// shape but doesn't carry the materialised merge layer (none is
/// built). The notebook's explicit Merge dialog (D34 §6.3) renders
/// this as the "Estimated outcome" line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergePreview {
    /// Heads' contributions since their LCA are pairwise disjoint —
    /// a real merge would produce a [`MergeOutcome::Merged`].
    /// `iri_count` is the total number of resources the merge layer
    /// would contain (union of per-head contributions), useful for
    /// the preview's "12 layers ahead, 4 behind" / "0 IRIs overlap"
    /// summary line.
    Disjoint { iri_count: usize },
    /// Heads conflict on `conflicting_iris`. A real merge would
    /// return [`MergeOutcome::Conflict`] with the same IRIs.
    Conflict { conflicting_iris: Vec<Iri> },
    /// One head is an ancestor of the other (or they're equal) — no
    /// merge is needed at all. The CAS would short-circuit as
    /// `FastForward`. Reported separately so the dialog can say so
    /// honestly instead of pretending a trivial merge would happen.
    FastForward,
}

/// Errors from `merge_independent_heads`.
#[derive(Debug)]
pub enum MergeError {
    /// `heads` was empty, contained an unknown LayerId, or has no LCA.
    InvalidHeads(String),
    /// Backend reported an error during the topology load, IRI listing,
    /// resource fetch, or merge-layer commit.
    Storage(StorageError),
    /// The merge layer was constructed but failed validation against
    /// its parent chain. Should not happen for trivial merges over
    /// already-validated heads, but surfaces if a corrupted source
    /// layer slips through.
    Validation(Vec<ValidationError>),
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::InvalidHeads(msg) => write!(f, "invalid heads: {msg}"),
            MergeError::Storage(e) => write!(f, "storage error: {e}"),
            MergeError::Validation(errs) => {
                writeln!(f, "merge layer failed validation ({} errors):", errs.len())?;
                for e in errs {
                    writeln!(f, "  {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for MergeError {}

/// N-way trivial merge of independent heads.
///
/// For each head, computes the IRIs it touched since the common LCA
/// and the layer that defines each IRI's top-of-stack value. If the
/// touched-IRI sets are pairwise disjoint, builds a multi-parent merge
/// layer whose:
///
/// - `parents` are the input `heads` sorted by `LayerId` (canonical
///   order — the order is part of the layer's identity per
///   `LayerBuilder::compute_layer_id`)
/// - `defined_iris` is the union of contributions from all heads
/// - per-IRI value comes from the topmost layer in the contributing
///   head's chain that defines it
///
/// On non-disjoint contributions, returns `Conflict { conflicting_iris }`
/// with the union of all IRI conflicts; the caller resolves via
/// Phase 15 witnessed merge.
///
/// **Single-head input** is a no-op: returns `Merged` wrapping the
/// single head loaded as a `Layer` (no new commit).
///
/// **Layer construction.** Uses `commit_layer` internally so the merge
/// layer is validated and persisted through the same atomic write
/// batch as any other commit.
/// Materials produced by [`compute_merge_check`] when the heads
/// merge cleanly. Carries through what the build step needs (sorted
/// heads, per-head IRI source maps, per-head top-of-stack
/// tombstone sets) so it doesn't recompute.
struct MergeCompute {
    heads: Vec<LayerId>,
    per_head_sources: Vec<std::collections::BTreeMap<Iri, LayerId>>,
    /// Per-head top-of-stack tombstones since LCA. Tombstones with an
    /// in-branch redefinition are already filtered out — only the
    /// tombstones that still hide a parent's body at the head appear
    /// here. The merge driver propagates these into the merge layer
    /// so the merge continues to hide what each branch hid.
    per_head_tombstones: Vec<BTreeSet<Iri>>,
}

/// Result of the side-effect-free portion of a merge attempt.
/// Used internally by both [`merge_independent_heads`] (which
/// proceeds to build + commit when this returns `Disjoint`) and
/// [`preview_merge_independent_heads`] (which stops here).
enum MergeCheck {
    /// One-head input. The caller can reuse the existing head as
    /// the "merge" with no new commit.
    SingleHead { head: LayerId },
    /// Heads diverge on the same IRIs. A real merge would fail.
    Conflict { conflicting_iris: Vec<Iri> },
    /// Heads can be merged without conflict. The caller has the
    /// materials to build the merge layer.
    Disjoint(MergeCompute),
}

/// Run the side-effect-free disjointness check. No writes touch the
/// backend; on a conflict the function returns immediately without
/// building anything. On the disjoint path it returns the precomputed
/// `per_head_sources` so [`merge_independent_heads`] doesn't redo
/// the LCA + ancestry walk.
fn compute_merge_check(
    heads: Vec<LayerId>,
    backend: &dyn PersistentBackend,
) -> Result<MergeCheck, MergeError> {
    if heads.is_empty() {
        return Err(MergeError::InvalidHeads("heads cannot be empty".into()));
    }

    // Canonical head ordering — affects merge LayerId.
    let mut heads = heads;
    heads.sort();
    heads.dedup();

    let topology = backend.load_topology().map_err(MergeError::Storage)?;

    // Validate all heads exist in the topology.
    for h in &heads {
        if topology.get_layer(h).is_none() {
            return Err(MergeError::InvalidHeads(format!(
                "head not found in topology: {h}"
            )));
        }
    }

    // Single-head: nothing to merge.
    if heads.len() == 1 {
        return Ok(MergeCheck::SingleHead {
            head: heads.into_iter().next().expect("len == 1"),
        });
    }

    // Compute LCA. Per the bootstrap-chain invariant, this should always
    // exist for layers persisted via the kernel.
    let lca = find_lca(&heads, &topology).ok_or_else(|| {
        MergeError::InvalidHeads("no common ancestor found — heads belong to disjoint DAGs".into())
    })?;

    // For each head, compute its IRI → source-layer map (definitions
    // since LCA) and its top-of-stack tombstone set (removals since
    // LCA, filtered by in-branch redefinitions).
    let mut per_head_sources: Vec<std::collections::BTreeMap<Iri, LayerId>> =
        Vec::with_capacity(heads.len());
    let mut per_head_tombstones: Vec<BTreeSet<Iri>> = Vec::with_capacity(heads.len());
    for head in &heads {
        let sources =
            iri_sources_since(head, &lca, &topology, backend).map_err(MergeError::Storage)?;
        // A tombstone is shadowed within the branch if the branch
        // also redefines the IRI above the tombstone. The defines
        // map's keys identify every IRI the branch defines at
        // top-of-stack since LCA; we filter the raw tombstone set
        // against it so only the surviving tombstones (those still
        // hiding a parent's body at the branch head) remain.
        let raw_tombstones = iri_tombstones_since(head, &lca, &topology);
        let tombstones: BTreeSet<Iri> = raw_tombstones
            .into_iter()
            .filter(|iri| !sources.contains_key(iri))
            .collect();
        per_head_sources.push(sources);
        per_head_tombstones.push(tombstones);
    }

    // Conflict detection. An IRI conflicts if more than one branch
    // modifies it (define or tombstone), unless all the modifications
    // agree (idempotent identical-tombstone case below). For trivial
    // merge v1 we treat any cross-branch overlap on an IRI other than
    // "all-tombstone" as a conflict — definition-vs-definition,
    // definition-vs-tombstone, and any mixed shape all surface as
    // `NeedsWitnessedMerge`. Witnessed merge (Phase 15) is the place
    // that resolves them; trivial merge is intentionally narrow.
    let mut conflicts: BTreeSet<Iri> = BTreeSet::new();
    // Per-IRI counts of (defines, tombstones) across branches.
    let mut iri_define_count: std::collections::BTreeMap<&Iri, u32> =
        std::collections::BTreeMap::new();
    let mut iri_tombstone_count: std::collections::BTreeMap<&Iri, u32> =
        std::collections::BTreeMap::new();
    for sources in &per_head_sources {
        for iri in sources.keys() {
            *iri_define_count.entry(iri).or_insert(0) += 1;
        }
    }
    for tombs in &per_head_tombstones {
        for iri in tombs {
            *iri_tombstone_count.entry(iri).or_insert(0) += 1;
        }
    }
    // Two-or-more branches defining the same IRI → conflict.
    for (iri, count) in &iri_define_count {
        if *count >= 2 {
            conflicts.insert((*iri).clone());
        }
    }
    // Any IRI tombstoned in one branch AND defined or tombstoned in
    // another → conflict. (Two branches identically tombstoning the
    // same IRI without any branch redefining it is the *only*
    // multi-touch case trivial merge accepts; it's idempotent so it
    // falls through without contributing to `conflicts`. Even there,
    // the merge layer still tombstones the IRI — handled in the
    // build step via `per_head_tombstones`.)
    for (iri, tomb_count) in &iri_tombstone_count {
        let def_count = iri_define_count.get(*iri).copied().unwrap_or(0);
        if def_count > 0 {
            conflicts.insert((*iri).clone());
        } else if *tomb_count >= 2 {
            // All tombstones, no defines — idempotent. Skip.
            continue;
        }
    }

    if !conflicts.is_empty() {
        let conflicting_iris: Vec<Iri> = conflicts.into_iter().collect();
        return Ok(MergeCheck::Conflict { conflicting_iris });
    }

    Ok(MergeCheck::Disjoint(MergeCompute {
        heads,
        per_head_sources,
        per_head_tombstones,
    }))
}

/// Dry-run [`merge_independent_heads`]: same LCA + IRI-disjointness
/// computation, no merge layer built, no branch ref moved. Powers
/// the notebook's explicit Merge dialog "preview" (D34 §6.3 — the
/// user sees the predicted outcome before committing).
///
/// Single-head inputs (or heads where one is an ancestor of the
/// others) report [`MergePreview::FastForward`] — there's nothing to
/// merge.
pub fn preview_merge_independent_heads(
    heads: Vec<LayerId>,
    backend: &dyn PersistentBackend,
) -> Result<MergePreview, MergeError> {
    match compute_merge_check(heads, backend)? {
        MergeCheck::SingleHead { .. } => Ok(MergePreview::FastForward),
        MergeCheck::Conflict { conflicting_iris } => {
            Ok(MergePreview::Conflict { conflicting_iris })
        }
        MergeCheck::Disjoint(MergeCompute {
            per_head_sources,
            per_head_tombstones,
            ..
        }) => {
            // Count is the merge layer's `defined_iris` size — the
            // union of per-head contributions. Since the disjoint
            // check has run, each IRI appears in exactly one head's
            // define map (no cross-branch overlaps survive). Tombstones
            // are union'd separately: identical tombstones across
            // branches collapse to one in the merge layer's
            // `tombstoned_iris`. The preview reports the merge
            // layer's `defined_iris` count to match what the user
            // would see post-commit; tombstones contribute to the
            // merge layer's visibility footprint but not to its
            // resource body count.
            let iri_count = per_head_sources.iter().map(|s| s.len()).sum();
            let _ = per_head_tombstones; // available for future preview expansion
            Ok(MergePreview::Disjoint { iri_count })
        }
    }
}

pub fn merge_independent_heads(
    heads: Vec<LayerId>,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<MergeOutcome, MergeError> {
    use crate::storage::ResourceBackend;

    // Run the compute half. On conflict, return immediately — no
    // merge layer is built. On the disjoint path we get back the
    // sorted heads and per-head source maps and proceed to build.
    let MergeCompute {
        heads,
        per_head_sources,
        per_head_tombstones,
    } = match compute_merge_check(heads, backend)? {
        MergeCheck::Conflict { conflicting_iris } => {
            return Ok(MergeOutcome::Conflict { conflicting_iris });
        }
        MergeCheck::SingleHead { head } => {
            // Single-head input. Reconstruct the Layer so callers
            // get a uniform `Merged { merge_layer }` outcome.
            let info = backend
                .load_chain_from(&head)
                .map_err(MergeError::Storage)?
                .ok_or_else(|| {
                    MergeError::InvalidHeads(format!("could not load chain for head {head}"))
                })?;
            let layer = crate::layer::build_chain(info, storage);
            return Ok(MergeOutcome::Merged { merge_layer: layer });
        }
        MergeCheck::Disjoint(c) => c,
    };

    // Build the merge layer. Parents = sorted heads (already sorted).
    // The parent Arcs need to be loaded as Layers; reuse LayerStorage's
    // resolve path via build_chain on each head, then assemble the
    // merge with `with_parents`.
    let mut parent_layers: Vec<Arc<Layer>> = Vec::with_capacity(heads.len());
    for head in &heads {
        let info = backend
            .load_chain_from(head)
            .map_err(MergeError::Storage)?
            .ok_or_else(|| {
                MergeError::InvalidHeads(format!("could not load chain for head {head}"))
            })?;
        let layer = crate::layer::build_chain(info, storage.clone());
        parent_layers.push(layer);
    }

    let mut builder = LayerBuilder::with_parents("merge", parent_layers);
    for sources in &per_head_sources {
        for (iri, source_layer_id) in sources {
            let resource = ResourceBackend::load_resource(backend, source_layer_id, iri)
                .ok_or_else(|| {
                    MergeError::Storage(StorageError::NotFound(format!(
                        "resource {iri} expected at layer {source_layer_id} during merge"
                    )))
                })?;
            // CoreNamespaceViolation cannot trigger here: merges have
            // parents (non-root), and core IRIs would have been
            // rejected when the source layer was originally committed.
            builder
                .add_resource(resource)
                .expect("builder accepts resource (non-root merge layer)");
        }
    }
    // Propagate top-of-stack tombstones from each branch into the
    // merge layer. The conflict check above guarantees no IRI is both
    // defined and tombstoned across branches, so the order of
    // `add_resource` then `tombstone` here can't trigger a collision.
    // The union over branches collapses identical tombstones to one
    // entry in the merge layer's `tombstoned_iris`.
    let mut union_tombstones: BTreeSet<Iri> = BTreeSet::new();
    for tombs in &per_head_tombstones {
        for iri in tombs {
            union_tombstones.insert(iri.clone());
        }
    }
    for iri in union_tombstones {
        builder
            .tombstone(iri)
            .expect("builder accepts tombstone (post-conflict check)");
    }

    let merge_layer = commit_layer_default(builder, storage, backend).map_err(|e| match e {
        CommitError::Validation { errors, .. } => MergeError::Validation(errors),
        CommitError::Storage(s) => MergeError::Storage(s),
        CommitError::Persist(ve) => {
            // D41 Phase B: the lattice path's `BackendStorePersister`
            // surfaces `backend.store_layer` failures via
            // `CommitError::Persist(ValidationError)`. The merge call
            // site treats them the same as `CommitError::Storage`.
            MergeError::Storage(StorageError::Internal(format!(
                "merge commit persist: {ve}"
            )))
        }
        CommitError::Layer(_) => unreachable!("builder errors handled above"),
        CommitError::WorkingSetExhausted(e) => {
            MergeError::Storage(StorageError::Internal(format!("merge commit: {e}")))
        }
        CommitError::CascadeAbort { .. } => {
            // commit_layer_default uses CommitPolicy::Reject — cascade
            // path isn't reachable here. Treat as Storage::Internal in
            // case the default ever changes.
            MergeError::Storage(StorageError::Internal(
                "merge commit produced unexpected CascadeAbort".into(),
            ))
        }
        CommitError::EmissionDepthExceeded { .. } => {
            // commit_layer_default goes through `BackendStorePersister`,
            // bypassing the orchestrator entirely. Surface as
            // Storage::Internal in case the wrapping ever changes.
            MergeError::Storage(StorageError::Internal(
                "merge commit produced unexpected EmissionDepthExceeded".into(),
            ))
        }
    })?;

    Ok(MergeOutcome::Merged { merge_layer })
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

    /// Stamp the validator-required `is_a` field on a test resource.
    /// Use `urn:eigenius:core:Class` as a generic placeholder; the
    /// lattice tests don't exercise class-typing semantics so the
    /// specific target doesn't matter. Call after `Resource::new` for
    /// any test fixture that the validator will see.
    fn set_default_is_a(r: &mut Resource) {
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
        );
        // Real `core:Class` requires `short_name`; stamp it so any fixture typed against
        // it validates against real core. (Idempotent with callers that also set it.)
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("test_fixture".into()),
        );
    }

    fn make_resource(id: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        set_default_is_a(&mut r);
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String("v".into()),
        );
        // Real `core:Class` requires `short_name` (the old coreless fixtures typed
        // against a fake Class with no requires); supply it so these instances validate.
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("test_fixture".into()),
        );
        r
    }

    /// A string-typed `core:Property` fixture, for IRIs used as property KEYS (which
    /// must resolve to a declared Property under reference integrity, Rule 22 §(c)).
    fn make_property(id: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Property".into())]),
        );
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("test_marker".into()),
        );
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String("test marker property".into()),
        );
        r.set(
            iri("urn:eigenius:core:data_type"),
            Value::String("urn:eigenius:core:string".into()),
        );
        r
    }

    /// Commit the real core ontology as the parent=None base layer, persisted to
    /// `backend`. Test fragments build on this so their property KEYS (`is_a`,
    /// `description`, …) resolve to declared `core:Property` resources (reference
    /// integrity, Rule 22 §(c)) — the same chain shape production always has. Replaces
    /// the old hand-rolled coreless `class_def` root.
    fn core_base(storage: &LayerStorage, backend: &dyn PersistentBackend) -> Arc<Layer> {
        let core_json = include_str!("../../ontologies/core/core-ontology.json");
        let resources = crate::ontology::eigon_json::parse_document(core_json).unwrap();
        let mut b = LayerBuilder::new("core", None);
        for r in resources {
            b.add_resource(r).unwrap();
        }
        commit_layer_default(b, storage.clone(), backend).unwrap()
    }

    /// A root `LayerBuilder` parented on the committed core base `core`, so its fixtures
    /// validate against real core vocabulary. Use instead of `LayerBuilder::new(name,
    /// None)` for test root layers.
    fn root_layer(name: &str, core: &Arc<Layer>) -> LayerBuilder {
        LayerBuilder::new(name, Some(Arc::clone(core)))
    }

    /// Build a small root layer (child of a freshly-committed core base) via the lattice
    /// commit primitive.
    fn commit_root(
        backend: &dyn PersistentBackend,
        name: &str,
        storage: &LayerStorage,
    ) -> Arc<Layer> {
        let core = core_base(storage, backend);
        let mut b = root_layer(name, &core);
        b.add_resource(make_resource("urn:eigenius:test:r"))
            .unwrap();
        commit_layer_default(b, storage.clone(), backend).unwrap()
    }

    #[test]
    fn commit_layer_persists_via_store_layer() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let layer = commit_root(&backend, "root", &storage);

        // Layer is in the topology + bloom + resources.
        let topo = backend.load_topology().unwrap();
        assert!(topo.get_layer(layer.id()).is_some());
        assert!(backend.load_bloom(layer.id()).unwrap().is_some());
        assert!(backend
            .load_resource(layer.id(), &iri("urn:eigenius:test:r"))
            .is_some());
    }

    #[test]
    fn commit_layer_does_not_touch_branches() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let _layer = commit_root(&backend, "root", &storage);

        // No branch was advanced by `commit_layer`. Branches are an
        // orthogonal surface.
        assert!(backend.list_branches().unwrap().is_empty());
    }

    #[test]
    fn update_branch_creates_new_branch() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let layer = commit_root(&backend, "root", &storage);

        // Creating a new branch: expected_old_head = None.
        let outcome = update_branch(
            "main",
            None,
            layer.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
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
        let root = commit_root(&backend, "root", &storage);

        // Initial branch creation.
        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Commit a child and fast-forward.
        let mut child_b = LayerBuilder::new("child", Some(Arc::clone(&root)));
        child_b
            .add_resource(make_resource("urn:eigenius:example:c"))
            .unwrap();
        let child = commit_layer_default(child_b, storage.clone(), &backend).unwrap();

        let outcome = update_branch(
            "main",
            Some(root.id().clone()),
            child.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();
        assert_eq!(outcome, UpdateOutcome::FastForward);
        assert_eq!(
            backend.get_branch("main").unwrap(),
            Some(child.id().clone())
        );
    }

    /// 14e: when divergent heads touch the SAME IRI with different
    /// values, trivial merge can't reconcile and `update_branch` returns
    /// `NeedsWitnessedMerge` with `conflicting_iris` populated.
    #[test]
    fn update_branch_conflict_returns_needs_witnessed_merge() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        // Branch starts at root.
        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Two diverging children both touching the SAME IRI with
        // different values — this is the conflict case.
        let conflict_iri = "urn:eigenius:example:contested";
        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        let mut r_a = Resource::new(iri(conflict_iri));
        set_default_is_a(&mut r_a);
        r_a.set(
            iri("urn:eigenius:core:description"),
            Value::String("from a".into()),
        );
        a_b.add_resource(r_a).unwrap();
        let a = commit_layer_default(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        let mut r_b = Resource::new(iri(conflict_iri));
        set_default_is_a(&mut r_b);
        r_b.set(
            iri("urn:eigenius:core:description"),
            Value::String("from b".into()),
        );
        b_b.add_resource(r_b).unwrap();
        let b = commit_layer_default(b_b, storage.clone(), &backend).unwrap();

        // Advance branch to `a`.
        update_branch(
            "main",
            Some(root.id().clone()),
            a.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Now try to advance to `b` claiming root was the parent —
        // branch moved to `a` and they conflict on the same IRI.
        let outcome = update_branch(
            "main",
            Some(root.id().clone()),
            b.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();
        match outcome {
            UpdateOutcome::NeedsWitnessedMerge {
                current_head,
                conflicting_iris,
                orphan_head: _,
            } => {
                assert_eq!(current_head, *a.id());
                // 14e populates the conflicts.
                assert_eq!(conflicting_iris, vec![iri(conflict_iri)]);
            }
            other => panic!("expected NeedsWitnessedMerge, got {other:?}"),
        }

        // Branch unchanged.
        assert_eq!(backend.get_branch("main").unwrap(), Some(a.id().clone()));
    }

    /// 14e: when divergent heads touch DISJOINT IRIs, trivial merge
    /// auto-resolves and `update_branch` returns `TrivialMerge` with
    /// the merge layer's id; the branch advances to the merge.
    #[test]
    fn update_branch_disjoint_divergence_trivial_merges() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Two diverging children touching disjoint IRIs.
        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        a_b.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer_default(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer_default(b_b, storage.clone(), &backend).unwrap();

        update_branch(
            "main",
            Some(root.id().clone()),
            a.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        let outcome = update_branch(
            "main",
            Some(root.id().clone()),
            b.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();
        let merge_id = match outcome {
            UpdateOutcome::TrivialMerge { merge_layer } => merge_layer,
            other => panic!("expected TrivialMerge, got {other:?}"),
        };

        // Branch points at the merge layer (not a or b).
        assert_eq!(backend.get_branch("main").unwrap(), Some(merge_id.clone()));
        assert!(merge_id != *a.id() && merge_id != *b.id());

        // Topology records the merge as having both heads as parents.
        let topo = backend.load_topology().unwrap();
        let merge_handle = topo.get_layer(&merge_id).expect("merge in topology");
        assert_eq!(merge_handle.parents.len(), 2);
        assert!(merge_handle.parents.contains(a.id()));
        assert!(merge_handle.parents.contains(b.id()));
    }

    /// Branch A tombstones an IRI defined at LCA; branch B doesn't
    /// touch that IRI. Trivial merge accepts A's tombstone as a
    /// one-sided change — the merge layer tombstones the IRI so
    /// `resolve` at the merge head agrees with branch A's view.
    /// Symmetric to the "one-sided definition" trivial-merge case.
    #[test]
    fn trivial_merge_propagates_one_sided_tombstone() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();

        // Root defines demo:X — the IRI A will tombstone.
        let core = core_base(&storage, &backend);
        let mut root_b = root_layer("root", &core);
        let mut root_resource = Resource::new(iri("urn:eigenius:demo:X"));
        // Validator requires non-empty `is_a`; the trivial-merge tests
        // don't exercise class-typing semantics so any value satisfies
        // the rule.
        root_resource.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
        );
        root_resource.set(
            iri("urn:eigenius:core:description"),
            Value::String("v_root".into()),
        );
        root_resource.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("x".into()),
        );
        root_b.add_resource(root_resource).unwrap();
        let root = commit_layer_default(root_b, storage.clone(), &backend).unwrap();

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Branch A: tombstones demo:X.
        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        a_b.tombstone(iri("urn:eigenius:demo:X")).unwrap();
        let a = commit_layer_default(a_b, storage.clone(), &backend).unwrap();

        // Branch B: adds an unrelated IRI, leaves demo:X alone.
        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:demo:Y"))
            .unwrap();
        let b = commit_layer_default(b_b, storage.clone(), &backend).unwrap();

        // Advance branch to A.
        update_branch(
            "main",
            Some(root.id().clone()),
            a.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Try to advance to B claiming root as parent. A's tombstone
        // and B's unrelated define are disjoint — trivial merge.
        let outcome = update_branch(
            "main",
            Some(root.id().clone()),
            b.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();
        let merge_id = match outcome {
            UpdateOutcome::TrivialMerge { merge_layer } => merge_layer,
            other => panic!("expected TrivialMerge, got {other:?}"),
        };

        // Resolve at the merge head: demo:X must be hidden (A's
        // tombstone propagated), demo:Y must be visible (B's add).
        let info = backend.load_chain_from(&merge_id).unwrap().unwrap();
        let merge_layer = crate::layer::build_chain(info, storage);
        assert!(
            merge_layer.resolve(&iri("urn:eigenius:demo:X")).is_none(),
            "merge layer must continue to hide demo:X via A's propagated tombstone"
        );
        assert!(
            merge_layer.resolve(&iri("urn:eigenius:demo:Y")).is_some(),
            "merge layer must preserve B's demo:Y"
        );
    }

    /// One branch defines an IRI, the other tombstones the same IRI
    /// (or a parent-side definition of it). Conflicting actions —
    /// trivial merge must surface as `NeedsWitnessedMerge` so the
    /// caller picks a witness or rebases.
    #[test]
    fn trivial_merge_define_vs_tombstone_is_conflict() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();

        // Root defines demo:X.
        let core = core_base(&storage, &backend);
        let mut root_b = root_layer("root", &core);
        let mut root_resource = Resource::new(iri("urn:eigenius:demo:X"));
        // Validator requires non-empty `is_a`; the trivial-merge tests
        // don't exercise class-typing semantics so any value satisfies
        // the rule.
        root_resource.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
        );
        root_resource.set(
            iri("urn:eigenius:core:description"),
            Value::String("v_root".into()),
        );
        root_resource.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("x".into()),
        );
        root_b.add_resource(root_resource).unwrap();
        let root = commit_layer_default(root_b, storage.clone(), &backend).unwrap();

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Branch A: tombstones demo:X.
        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        a_b.tombstone(iri("urn:eigenius:demo:X")).unwrap();
        let a = commit_layer_default(a_b, storage.clone(), &backend).unwrap();

        // Branch B: redefines demo:X with a different body.
        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        let mut x_b = Resource::new(iri("urn:eigenius:demo:X"));
        x_b.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
        );
        x_b.set(
            iri("urn:eigenius:core:description"),
            Value::String("from b".into()),
        );
        x_b.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("x".into()),
        );
        b_b.add_resource(x_b).unwrap();
        let b = commit_layer_default(b_b, storage.clone(), &backend).unwrap();

        update_branch(
            "main",
            Some(root.id().clone()),
            a.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        let outcome = update_branch(
            "main",
            Some(root.id().clone()),
            b.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();
        match outcome {
            UpdateOutcome::NeedsWitnessedMerge {
                conflicting_iris, ..
            } => {
                assert_eq!(conflicting_iris, vec![iri("urn:eigenius:demo:X")]);
            }
            other => panic!("expected NeedsWitnessedMerge, got {other:?}"),
        }
    }

    // --- merge_branch_tips: cross-branch merge ---

    /// Regression: two sibling branches both touching the same IRI
    /// must produce `NeedsWitnessedMerge` from the cross-branch merge
    /// surface — never a silent fast-forward overwrite. The old
    /// `merge_branches` server handler routed through `update_branch`,
    /// which took the CAS-shortcut and unconditionally advanced the
    /// target to the source tip when the caller's `expected_old_head`
    /// matched. That destroyed the target's history.
    #[test]
    fn merge_branch_tips_conflict_returns_needs_witnessed_merge() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        let conflict_iri = "urn:eigenius:example:contested";
        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        let mut r_a = Resource::new(iri(conflict_iri));
        set_default_is_a(&mut r_a);
        r_a.set(
            iri("urn:eigenius:core:description"),
            Value::String("from a".into()),
        );
        a_b.add_resource(r_a).unwrap();
        let a = commit_layer_default(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        let mut r_b = Resource::new(iri(conflict_iri));
        set_default_is_a(&mut r_b);
        r_b.set(
            iri("urn:eigenius:core:description"),
            Value::String("from b".into()),
        );
        b_b.add_resource(r_b).unwrap();
        let b = commit_layer_default(b_b, storage.clone(), &backend).unwrap();

        // Advance `main` to `a`. Both `a` and `b` are siblings off `root`,
        // both touching `conflict_iri` with different values.
        backend.put_branch("main", a.id()).unwrap();

        let outcome = merge_branch_tips("main", b.id().clone(), storage.clone(), &backend).unwrap();
        match outcome {
            UpdateOutcome::NeedsWitnessedMerge {
                current_head,
                conflicting_iris,
                orphan_head,
            } => {
                assert_eq!(current_head, *a.id());
                assert_eq!(orphan_head, *b.id());
                assert_eq!(conflicting_iris, vec![iri(conflict_iri)]);
            }
            other => panic!("expected NeedsWitnessedMerge, got {other:?}"),
        }

        // Target branch unchanged — this is the load-bearing assertion:
        // the old code would have overwritten `main` to point at `b`.
        assert_eq!(backend.get_branch("main").unwrap(), Some(a.id().clone()));
    }

    /// Source tip is a descendant of the target tip — the merge surface
    /// fast-forwards the target.
    #[test]
    fn merge_branch_tips_fast_forwards_when_source_descends_from_target() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut child_b = LayerBuilder::new("child", Some(Arc::clone(&root)));
        child_b
            .add_resource(make_resource("urn:eigenius:example:c"))
            .unwrap();
        let child = commit_layer_default(child_b, storage.clone(), &backend).unwrap();

        backend.put_branch("main", root.id()).unwrap();

        let outcome =
            merge_branch_tips("main", child.id().clone(), storage.clone(), &backend).unwrap();
        assert_eq!(outcome, UpdateOutcome::FastForward);
        assert_eq!(
            backend.get_branch("main").unwrap(),
            Some(child.id().clone())
        );
    }

    /// Target already includes the source tip in its history — no-op
    /// fast-forward, branch ref unchanged.
    #[test]
    fn merge_branch_tips_noop_when_target_already_includes_source() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut child_b = LayerBuilder::new("child", Some(Arc::clone(&root)));
        child_b
            .add_resource(make_resource("urn:eigenius:example:c"))
            .unwrap();
        let child = commit_layer_default(child_b, storage.clone(), &backend).unwrap();

        backend.put_branch("main", child.id()).unwrap();

        let outcome =
            merge_branch_tips("main", root.id().clone(), storage.clone(), &backend).unwrap();
        assert_eq!(outcome, UpdateOutcome::FastForward);
        // Branch unchanged — target was already ahead of source.
        assert_eq!(
            backend.get_branch("main").unwrap(),
            Some(child.id().clone())
        );
    }

    /// Disjoint sibling contributions — both branches advanced by
    /// non-overlapping IRIs — produce a multi-parent merge layer and
    /// the target branch points at it.
    #[test]
    fn merge_branch_tips_disjoint_produces_trivial_merge() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        a_b.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer_default(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer_default(b_b, storage.clone(), &backend).unwrap();

        backend.put_branch("main", a.id()).unwrap();

        let outcome = merge_branch_tips("main", b.id().clone(), storage.clone(), &backend).unwrap();
        let merge_id = match outcome {
            UpdateOutcome::TrivialMerge { merge_layer } => merge_layer,
            other => panic!("expected TrivialMerge, got {other:?}"),
        };
        assert_eq!(backend.get_branch("main").unwrap(), Some(merge_id.clone()));
        let topo = backend.load_topology().unwrap();
        let handle = topo.get_layer(&merge_id).expect("merge in topology");
        assert_eq!(handle.parents.len(), 2);
        assert!(handle.parents.contains(a.id()));
        assert!(handle.parents.contains(b.id()));
    }

    // --- 14e-ii: find_lca + iri_sources_since primitives ---

    #[test]
    fn find_lca_single_parent_chain() {
        // root → a → b → c. LCA(b, c) = b. LCA(a, c) = a. LCA(c, c) = c.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut ab = LayerBuilder::new("a", Some(Arc::clone(&root)));
        ab.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer_default(ab, storage.clone(), &backend).unwrap();

        let mut bb = LayerBuilder::new("b", Some(Arc::clone(&a)));
        bb.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer_default(bb, storage.clone(), &backend).unwrap();

        let mut cb = LayerBuilder::new("c", Some(Arc::clone(&b)));
        cb.add_resource(make_resource("urn:eigenius:example:c"))
            .unwrap();
        let c = commit_layer_default(cb, storage.clone(), &backend).unwrap();

        let topo = backend.load_topology().unwrap();
        assert_eq!(
            find_lca(&[b.id().clone(), c.id().clone()], &topo),
            Some(b.id().clone())
        );
        assert_eq!(
            find_lca(&[a.id().clone(), c.id().clone()], &topo),
            Some(a.id().clone())
        );
        assert_eq!(find_lca(&[c.id().clone()], &topo), Some(c.id().clone()));
    }

    #[test]
    fn find_lca_diverging_branches() {
        // root → a, root → b. LCA(a, b) = root.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut ab = LayerBuilder::new("a", Some(Arc::clone(&root)));
        ab.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer_default(ab, storage.clone(), &backend).unwrap();

        let mut bb = LayerBuilder::new("b", Some(Arc::clone(&root)));
        bb.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer_default(bb, storage.clone(), &backend).unwrap();

        let topo = backend.load_topology().unwrap();
        assert_eq!(
            find_lca(&[a.id().clone(), b.id().clone()], &topo),
            Some(root.id().clone())
        );
    }

    #[test]
    fn find_lca_n_way_returns_deepest_common() {
        // root → a → x; root → a → y; root → a → z. LCA = a.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut ab = LayerBuilder::new("a", Some(Arc::clone(&root)));
        ab.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer_default(ab, storage.clone(), &backend).unwrap();

        let mut leaves = Vec::new();
        for tag in ["x", "y", "z"] {
            let mut lb = LayerBuilder::new(tag, Some(Arc::clone(&a)));
            lb.add_resource(make_resource(&format!("urn:eigenius:example:{tag}")))
                .unwrap();
            leaves.push(commit_layer_default(lb, storage.clone(), &backend).unwrap());
        }

        let heads: Vec<LayerId> = leaves.iter().map(|l| l.id().clone()).collect();
        let topo = backend.load_topology().unwrap();
        assert_eq!(find_lca(&heads, &topo), Some(a.id().clone()));
    }

    #[test]
    fn iri_sources_since_walks_diverged_chain() {
        // root → mid (defines :m) → tip (defines :t). LCA = root, head = tip.
        // Sources should map :m → mid and :t → tip.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut mid_b = LayerBuilder::new("mid", Some(Arc::clone(&root)));
        mid_b
            .add_resource(make_resource("urn:eigenius:example:m"))
            .unwrap();
        let mid = commit_layer_default(mid_b, storage.clone(), &backend).unwrap();

        let mut tip_b = LayerBuilder::new("tip", Some(Arc::clone(&mid)));
        tip_b
            .add_resource(make_resource("urn:eigenius:example:t"))
            .unwrap();
        let tip = commit_layer_default(tip_b, storage.clone(), &backend).unwrap();

        let topo = backend.load_topology().unwrap();
        let sources = iri_sources_since(tip.id(), root.id(), &topo, &backend).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources.get(&iri("urn:eigenius:example:m")), Some(mid.id()));
        assert_eq!(sources.get(&iri("urn:eigenius:example:t")), Some(tip.id()));
    }

    #[test]
    fn iri_sources_since_topmost_wins_on_redefinition() {
        // mid defines :x. tip redefines :x. Source for :x = tip.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut mid_b = LayerBuilder::new("mid", Some(Arc::clone(&root)));
        let mut r = Resource::new(iri("urn:eigenius:example:x"));
        set_default_is_a(&mut r);
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String("v1".into()),
        );
        mid_b.add_resource(r).unwrap();
        let mid = commit_layer_default(mid_b, storage.clone(), &backend).unwrap();

        let mut tip_b = LayerBuilder::new("tip", Some(Arc::clone(&mid)));
        let mut r2 = Resource::new(iri("urn:eigenius:example:x"));
        set_default_is_a(&mut r2);
        r2.set(
            iri("urn:eigenius:core:description"),
            Value::String("v2".into()),
        );
        tip_b.add_resource(r2).unwrap();
        let tip = commit_layer_default(tip_b, storage.clone(), &backend).unwrap();

        let topo = backend.load_topology().unwrap();
        let sources = iri_sources_since(tip.id(), root.id(), &topo, &backend).unwrap();
        assert_eq!(sources.get(&iri("urn:eigenius:example:x")), Some(tip.id()));
    }

    // --- 14e-iii: merge_independent_heads ---

    #[test]
    fn merge_independent_heads_three_way_disjoint() {
        // The user's case: three task results, each touching a distinct
        // IRI, consolidated into a single layer with three parents.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut heads = Vec::new();
        for tag in ["task1", "task2", "task3"] {
            let mut b = LayerBuilder::new(tag, Some(Arc::clone(&root)));
            b.add_resource(make_resource(&format!("urn:eigenius:result:{tag}")))
                .unwrap();
            heads.push(commit_layer_default(b, storage.clone(), &backend).unwrap());
        }
        let head_ids: Vec<LayerId> = heads.iter().map(|h| h.id().clone()).collect();

        let outcome = merge_independent_heads(head_ids.clone(), storage.clone(), &backend).unwrap();
        let merge = match outcome {
            MergeOutcome::Merged { merge_layer } => merge_layer,
            MergeOutcome::Conflict { conflicting_iris } => {
                panic!("expected Merged, got Conflict({conflicting_iris:?})")
            }
        };

        // Merge has 3 parents (sorted), 3 IRIs of contributions.
        assert_eq!(merge.parents().len(), 3);
        let merge_handle = backend
            .load_topology()
            .unwrap()
            .get_layer(merge.id())
            .cloned()
            .unwrap();
        for h in &head_ids {
            assert!(merge_handle.parents.contains(h));
        }
        for tag in ["task1", "task2", "task3"] {
            assert!(merge
                .defined_iris()
                .contains(&iri(&format!("urn:eigenius:result:{tag}"))));
        }
    }

    #[test]
    fn merge_independent_heads_conflict_reports_iris() {
        // Two heads both touching the same IRI with different values →
        // Conflict, not Merged.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let conflict_iri = "urn:eigenius:example:contested";
        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        let mut r_a = Resource::new(iri(conflict_iri));
        set_default_is_a(&mut r_a);
        r_a.set(
            iri("urn:eigenius:core:description"),
            Value::String("a".into()),
        );
        a_b.add_resource(r_a).unwrap();
        let a = commit_layer_default(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        let mut r_b = Resource::new(iri(conflict_iri));
        set_default_is_a(&mut r_b);
        r_b.set(
            iri("urn:eigenius:core:description"),
            Value::String("b".into()),
        );
        b_b.add_resource(r_b).unwrap();
        let b = commit_layer_default(b_b, storage.clone(), &backend).unwrap();

        let outcome = merge_independent_heads(
            vec![a.id().clone(), b.id().clone()],
            storage.clone(),
            &backend,
        )
        .unwrap();
        match outcome {
            MergeOutcome::Conflict { conflicting_iris } => {
                assert_eq!(conflicting_iris, vec![iri(conflict_iri)]);
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn merge_independent_heads_single_head_is_noop() {
        // Single head should return Merged wrapping that head — no
        // new commit.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let outcome =
            merge_independent_heads(vec![root.id().clone()], storage.clone(), &backend).unwrap();
        match outcome {
            MergeOutcome::Merged { merge_layer } => {
                assert_eq!(merge_layer.id(), root.id());
            }
            other => panic!("expected Merged, got {other:?}"),
        }
    }

    #[test]
    fn merge_independent_heads_resolves_through_merge() {
        // After the merge, resolve() at the merge layer returns the
        // values each head contributed. End-to-end correctness check.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        let mut r_a = Resource::new(iri("urn:eigenius:example:a"));
        set_default_is_a(&mut r_a);
        r_a.set(
            iri("urn:eigenius:core:description"),
            Value::String("from a".into()),
        );
        a_b.add_resource(r_a).unwrap();
        let a = commit_layer_default(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        let mut r_b = Resource::new(iri("urn:eigenius:example:b"));
        set_default_is_a(&mut r_b);
        r_b.set(
            iri("urn:eigenius:core:description"),
            Value::String("from b".into()),
        );
        b_b.add_resource(r_b).unwrap();
        let b = commit_layer_default(b_b, storage.clone(), &backend).unwrap();

        let outcome = merge_independent_heads(
            vec![a.id().clone(), b.id().clone()],
            storage.clone(),
            &backend,
        )
        .unwrap();
        let merge = match outcome {
            MergeOutcome::Merged { merge_layer } => merge_layer,
            other => panic!("expected Merged, got {other:?}"),
        };

        let res_a = merge.resolve(&iri("urn:eigenius:example:a")).unwrap();
        assert_eq!(
            res_a
                .get(&iri("urn:eigenius:core:description"))
                .and_then(|v| v.as_str()),
            Some("from a")
        );
        let res_b = merge.resolve(&iri("urn:eigenius:example:b")).unwrap();
        assert_eq!(
            res_b
                .get(&iri("urn:eigenius:core:description"))
                .and_then(|v| v.as_str()),
            Some("from b")
        );
    }

    #[test]
    fn update_branch_strict_fast_forward_rejects_divergence() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        a_b.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer_default(a_b, storage.clone(), &backend).unwrap();
        update_branch(
            "main",
            Some(root.id().clone()),
            a.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Stale-expected against StrictFastForward → error, not outcome.
        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer_default(b_b, storage.clone(), &backend).unwrap();
        let err = update_branch(
            "main",
            Some(root.id().clone()),
            b.id().clone(),
            ConflictPolicy::StrictFastForward,
            storage.clone(),
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
        let storage = LayerStorage::in_memory();
        // Use a real layer id so the trivial-merge path inside
        // update_branch (when the branch already exists) doesn't trip
        // on an unknown-id lookup. For these tests we mostly care about
        // the name-validation gate, which fires before any storage
        // touch, so a synthetic id is fine for the bad-name cases.
        let layer = commit_root(&backend, "root", &storage);
        let id = layer.id().clone();

        for bad in ["", "has space", "has/slash", "has.dot", &"x".repeat(257)] {
            let err = update_branch(
                bad,
                None,
                id.clone(),
                ConflictPolicy::AllowTrivial,
                storage.clone(),
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
            let outcome = update_branch(
                ok,
                None,
                id.clone(),
                ConflictPolicy::AllowTrivial,
                storage.clone(),
                &backend,
            );
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
        let root = commit_root(backend.as_ref(), "root", &storage);

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            backend.as_ref(),
        )
        .unwrap();

        let mut a_b = LayerBuilder::new("a", Some(Arc::clone(&root)));
        a_b.add_resource(make_resource("urn:eigenius:example:a"))
            .unwrap();
        let a = commit_layer_default(a_b, storage.clone(), backend.as_ref()).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer_default(b_b, storage.clone(), backend.as_ref()).unwrap();

        let backend_a = Arc::clone(&backend);
        let storage_a = storage.clone();
        let root_id_a = root.id().clone();
        let a_id = a.id().clone();
        let t_a = thread::spawn(move || {
            update_branch(
                "main",
                Some(root_id_a),
                a_id,
                ConflictPolicy::AllowTrivial,
                storage_a,
                backend_a.as_ref(),
            )
            .unwrap()
        });

        let backend_b = Arc::clone(&backend);
        let storage_b = storage.clone();
        let root_id_b = root.id().clone();
        let b_id = b.id().clone();
        let t_b = thread::spawn(move || {
            update_branch(
                "main",
                Some(root_id_b),
                b_id,
                ConflictPolicy::AllowTrivial,
                storage_b,
                backend_b.as_ref(),
            )
            .unwrap()
        });

        let r_a = t_a.join().unwrap();
        let r_b = t_b.join().unwrap();

        // 14e behavior: a and b touch disjoint IRIs (one urn:...:a, the
        // other urn:...:b), so the loser of the CAS race trivially
        // merges. Exactly one FastForward + exactly one TrivialMerge.
        let ff_count = [&r_a, &r_b]
            .iter()
            .filter(|o| matches!(o, UpdateOutcome::FastForward))
            .count();
        let trivial_count = [&r_a, &r_b]
            .iter()
            .filter(|o| matches!(o, UpdateOutcome::TrivialMerge { .. }))
            .count();
        assert_eq!(
            ff_count, 1,
            "exactly one CAS must fast-forward (got {ff_count} FF, {trivial_count} trivial)"
        );
        assert_eq!(trivial_count, 1);

        // After the trivial merge, the branch points at the merge layer
        // whose parents are sorted [a, b]. We can verify by reading the
        // final head and asserting it's neither a nor b directly.
        let final_head = backend.get_branch("main").unwrap().unwrap();
        assert!(
            final_head != *a.id() && final_head != *b.id(),
            "branch should point at the merge layer, not a or b"
        );
    }

    /// Two per-branch CAS critical sections on **different** branches
    /// must execute concurrently — they hold the snapshot gate in
    /// shared (read) mode and disjoint per-branch slots, so neither
    /// blocks the other. Pre-fix (single global mutex) would
    /// serialize them; this test asserts the time windows overlap.
    #[test]
    fn per_branch_locks_run_concurrently_on_distinct_branches() {
        use std::sync::Barrier;
        use std::thread;
        use std::time::{Duration, Instant};

        let barrier = Arc::new(Barrier::new(2));
        let entered_main: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let exited_main: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let entered_feat: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let exited_feat: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

        let spawn_for = |branch: &'static str,
                         entered: Arc<Mutex<Option<Instant>>>,
                         exited: Arc<Mutex<Option<Instant>>>,
                         barrier: Arc<Barrier>| {
            thread::spawn(move || {
                barrier.wait();
                let snapshot = snapshot_gate().read().expect("gate poisoned");
                let slot = branch_slot(branch);
                let _guard = slot.lock().expect("slot poisoned");
                *entered.lock().unwrap() = Some(Instant::now());
                thread::sleep(Duration::from_millis(60));
                *exited.lock().unwrap() = Some(Instant::now());
                drop(snapshot);
            })
        };

        let t_main = spawn_for(
            "main",
            Arc::clone(&entered_main),
            Arc::clone(&exited_main),
            Arc::clone(&barrier),
        );
        let t_feat = spawn_for(
            "feature",
            Arc::clone(&entered_feat),
            Arc::clone(&exited_feat),
            Arc::clone(&barrier),
        );

        t_main.join().unwrap();
        t_feat.join().unwrap();

        let em = entered_main.lock().unwrap().unwrap();
        let xm = exited_main.lock().unwrap().unwrap();
        let ef = entered_feat.lock().unwrap().unwrap();
        let xf = exited_feat.lock().unwrap().unwrap();

        // Windows overlap iff one entered before the other exited.
        let overlap = em <= xf && ef <= xm;
        assert!(
            overlap,
            "distinct-branch CAS slots must run in parallel \
             (entered_main={em:?}, exited_main={xm:?}, \
              entered_feature={ef:?}, exited_feature={xf:?})"
        );
    }

    /// `with_branch_lock` (snapshot gate in write mode) blocks all
    /// in-flight per-branch CAS until it returns. Tested by spawning
    /// an `update_branch` thread that pauses inside its critical
    /// section, then asserting `with_branch_lock` only enters after
    /// the CAS thread releases.
    #[test]
    fn with_branch_lock_blocks_per_branch_updates() {
        use std::sync::Barrier;
        use std::thread;
        use std::time::{Duration, Instant};

        let barrier = Arc::new(Barrier::new(2));
        let update_entered: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let update_released: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

        let b1 = Arc::clone(&barrier);
        let ue = Arc::clone(&update_entered);
        let ur = Arc::clone(&update_released);
        let updater = thread::spawn(move || {
            b1.wait();
            let snapshot = snapshot_gate().read().expect("gate poisoned");
            let slot = branch_slot("main");
            let _guard = slot.lock().expect("slot poisoned");
            *ue.lock().unwrap() = Some(Instant::now());
            // Hold the slot for a measurable window.
            thread::sleep(Duration::from_millis(100));
            *ur.lock().unwrap() = Some(Instant::now());
            drop(snapshot);
        });

        // Give the updater a moment to acquire its locks.
        thread::sleep(Duration::from_millis(20));
        barrier.wait();
        // Updater is now in flight.
        thread::sleep(Duration::from_millis(20));

        let snapshot_taken = Instant::now();
        let entered_snapshot = with_branch_lock(Instant::now);

        updater.join().unwrap();

        let released = update_released.lock().unwrap().unwrap();

        assert!(
            entered_snapshot >= released,
            "snapshot must enter after the updater releases (entered={entered_snapshot:?}, \
             released={released:?})"
        );
        // Sanity: the wait was non-trivial (we actually blocked).
        let wait = entered_snapshot.duration_since(snapshot_taken);
        assert!(
            wait >= Duration::from_millis(20),
            "expected to block ≥20ms waiting for the updater; got {wait:?}"
        );
    }

    // --- 14g-iii: prune_branch ---

    #[test]
    fn prune_branch_removes_existing_branch() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        let outcome = prune_branch("main", PruneSafety::Force, &backend).unwrap();
        match outcome {
            PruneOutcome::Pruned { previous_head } => {
                assert_eq!(previous_head, *root.id());
            }
            PruneOutcome::NotFound => panic!("expected Pruned, got NotFound"),
        }

        // Branch is gone.
        assert!(backend.get_branch("main").unwrap().is_none());
    }

    #[test]
    fn prune_branch_returns_not_found_for_unknown_branch() {
        let backend = MemoryPersistentBackend::new();
        let outcome = prune_branch("never-existed", PruneSafety::Force, &backend).unwrap();
        assert_eq!(outcome, PruneOutcome::NotFound);
    }

    #[test]
    fn prune_branch_check_pins_rejects_in_use() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Pretend a task is pinned at root (the branch's current head).
        let pins = vec![root.id().clone()];
        let err = prune_branch("main", PruneSafety::CheckPins(&pins), &backend).unwrap_err();
        match err {
            PruneError::InUse { branch, head } => {
                assert_eq!(branch, "main");
                assert_eq!(head, *root.id());
            }
            other => panic!("expected InUse, got {other:?}"),
        }

        // Branch unchanged.
        assert_eq!(backend.get_branch("main").unwrap(), Some(root.id().clone()));
    }

    #[test]
    fn prune_branch_check_pins_allows_when_pin_doesnt_match() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);
        let other = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "other",
            "urn:eigenius:test:o",
        );

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Task pinned at `other`, branch points at `root` — no conflict.
        let pins = vec![other.id().clone()];
        let outcome = prune_branch("main", PruneSafety::CheckPins(&pins), &backend).unwrap();
        assert!(matches!(outcome, PruneOutcome::Pruned { .. }));
    }

    #[test]
    fn prune_branch_force_overrides_pin_check() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);

        update_branch(
            "main",
            None,
            root.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Force ignores task pins.
        let outcome = prune_branch("main", PruneSafety::Force, &backend).unwrap();
        assert!(matches!(outcome, PruneOutcome::Pruned { .. }));
    }

    #[test]
    fn prune_branch_followed_by_gc_reclaims_orphaned_layers() {
        // End-to-end: prune the only branch pointing at a layer chain;
        // a subsequent gc::collect reclaims those layers.
        use crate::gc::{collect, GcConfig, GcRoots};

        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root(&backend, "root", &storage);
        let tip = commit_child(
            &backend,
            &storage,
            Arc::clone(&root),
            "tip",
            "urn:eigenius:test:t",
        );

        update_branch(
            "main",
            None,
            tip.id().clone(),
            ConflictPolicy::AllowTrivial,
            storage.clone(),
            &backend,
        )
        .unwrap();

        // Verify reachable before prune (core base + root + tip).
        assert_eq!(backend.load_topology().unwrap().layer_count(), 3);

        prune_branch("main", PruneSafety::Force, &backend).unwrap();

        // GC with min_age = 0 to skip the recent-commit protection.
        let stats = collect(
            GcRoots::from_branches(&backend).unwrap(),
            &GcConfig {
                min_age: std::time::Duration::from_secs(0),
            },
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            &backend,
        )
        .unwrap();
        assert_eq!(
            stats.layers_swept, 3,
            "core base + root + tip all reclaimed once the only branch is pruned"
        );
        assert_eq!(backend.load_topology().unwrap().layer_count(), 0);
    }

    /// Helper: commit a child layer above `parent`. Local to the
    /// prune-branch tests; not extracted because lattice.rs's other
    /// tests don't need it.
    fn commit_child(
        backend: &dyn PersistentBackend,
        storage: &LayerStorage,
        parent: Arc<Layer>,
        name: &str,
        iri_str: &str,
    ) -> Arc<Layer> {
        let mut b = LayerBuilder::new(name, Some(parent));
        b.add_resource(make_resource(iri_str)).unwrap();
        commit_layer_default(b, storage.clone(), backend).unwrap()
    }

    // ─── 20c anchored-commit cache wrapper ─────────────────────────────────

    /// Commit a root layer that declares `urn:eigenius:core:description`
    /// so cell layers (which use that property) have a real supporting
    /// layer in their chain — without this, `compute_supporting_layer`
    /// returns `None` and the cache is bypassed.
    fn commit_root_with_description(
        backend: &dyn PersistentBackend,
        name: &str,
        storage: &LayerStorage,
    ) -> Arc<Layer> {
        let core = core_base(storage, backend);
        let mut b = root_layer(name, &core);
        b.add_resource(make_resource("urn:eigenius:test:desc_marker"))
            .unwrap();
        commit_layer_default(b, storage.clone(), backend).unwrap()
    }

    /// Build a cell-style layer (parent + one demo resource that
    /// references a property defined in the parent's chain → forces
    /// the layer to have a non-None supporting_layer).
    fn build_test_child_layer(
        parent: Arc<Layer>,
        cell_iri: &str,
        cell_value: &str,
    ) -> LayerBuilder {
        let mut b = LayerBuilder::new("cell", Some(parent));
        let mut r = Resource::new(iri(cell_iri));
        set_default_is_a(&mut r);
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String(cell_value.into()),
        );
        b.add_resource(r).unwrap();
        b
    }

    /// Phase 20c: a re-run of a cell with byte-identical content
    /// against an identical supporting context returns
    /// `AnchoredCommitOutcome::Hit` with the previously-committed layer's
    /// id. The second commit doesn't persist a new layer; the cache
    /// holds exactly one entry.
    #[test]
    fn anchored_commit_hit_on_identical_run() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root_with_description(&backend, "root", &storage);

        // First commit: cache miss. A new layer is stored.
        let first = commit_layer_with_cache(
            build_test_child_layer(Arc::clone(&root), "urn:eigenius:demo:cell", "v1"),
            storage.clone(),
            &backend,
        )
        .unwrap();
        let first_id = match first {
            AnchoredCommitOutcome::Miss { layer } => layer.id().clone(),
            AnchoredCommitOutcome::Hit { .. } => panic!("first commit must miss"),
        };
        assert_eq!(backend.list_anchored_commits().unwrap().len(), 1);

        // Second commit: byte-identical content + same supporting
        // context → cache hit; no new layer stored.
        let second = commit_layer_with_cache(
            build_test_child_layer(Arc::clone(&root), "urn:eigenius:demo:cell", "v1"),
            storage,
            &backend,
        )
        .unwrap();
        match second {
            AnchoredCommitOutcome::Hit { cached_layer_id } => {
                assert_eq!(cached_layer_id, first_id);
            }
            AnchoredCommitOutcome::Miss { .. } => panic!("second commit must hit"),
        }
        // Cache still holds exactly one entry — the hit didn't add a
        // new row.
        assert_eq!(backend.list_anchored_commits().unwrap().len(), 1);
    }

    /// Phase 20c: changing the cell's content (different value)
    /// misses the cache. A new layer is stored; the cache grows by
    /// one entry.
    #[test]
    fn anchored_commit_miss_on_content_change() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let root = commit_root_with_description(&backend, "root", &storage);

        commit_layer_with_cache(
            build_test_child_layer(Arc::clone(&root), "urn:eigenius:demo:cell", "v1"),
            storage.clone(),
            &backend,
        )
        .unwrap();
        let second = commit_layer_with_cache(
            build_test_child_layer(Arc::clone(&root), "urn:eigenius:demo:cell", "v2"),
            storage,
            &backend,
        )
        .unwrap();
        match second {
            AnchoredCommitOutcome::Miss { .. } => {}
            AnchoredCommitOutcome::Hit { .. } => panic!("different content must miss"),
        }
        assert_eq!(backend.list_anchored_commits().unwrap().len(), 2);
    }

    /// Phase 20c (the load-bearing property): two structurally-
    /// equivalent supporting contexts hash to the same cache key.
    /// Build two parallel chains whose head layers have byte-equal
    /// content (so their `content_hash`es match) but different
    /// position hashes (different deeper ancestors). A cell layer
    /// committed against the first chain's head can be re-derived
    /// from the second chain's head and the cache hits.
    #[test]
    fn anchored_commit_hit_on_supporting_equivalent_context() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();

        // Build two structurally-equivalent supporting layers `s1`
        // and `s2`. They share content (the same resource declaring
        // `demo:Marker`) but live above different distinct roots,
        // so their position hashes differ while their content hashes
        // match. The roots themselves need distinct content
        // (otherwise they'd collapse to one layer per content
        // addressing), so we give each a unique marker resource in
        // addition to `core:description`.
        let core = core_base(&storage, &backend);
        let mut rb_a = root_layer("root_a", &core);
        rb_a.add_resource(make_resource("urn:eigenius:demo:a_marker"))
            .unwrap();
        let root_a = commit_layer_default(rb_a, storage.clone(), &backend).unwrap();

        let mut rb_b = root_layer("root_b", &core);
        rb_b.add_resource(make_resource("urn:eigenius:demo:b_marker"))
            .unwrap();
        let root_b = commit_layer_default(rb_b, storage.clone(), &backend).unwrap();
        assert_ne!(root_a.id(), root_b.id());

        let mut sb_a = LayerBuilder::new("support", Some(Arc::clone(&root_a)));
        sb_a.add_resource(make_property("urn:eigenius:demo:Marker"))
            .unwrap();
        let support_a = commit_layer_default(sb_a, storage.clone(), &backend).unwrap();

        let mut sb_b = LayerBuilder::new("support", Some(Arc::clone(&root_b)));
        sb_b.add_resource(make_property("urn:eigenius:demo:Marker"))
            .unwrap();
        let support_b = commit_layer_default(sb_b, storage.clone(), &backend).unwrap();

        // Pre-condition: same content_hash, different position
        // (different parent chains).
        assert_eq!(support_a.content_hash(), support_b.content_hash());
        assert_ne!(support_a.id(), support_b.id());

        // Commit the same cell content against each supporting layer.
        // The cell references `demo:Marker` as a property key —
        // since the support layer is the youngest layer that
        // defines `demo:Marker`, the cell's supporting layer
        // resolves there. Both supports have the same content hash,
        // so both cell commits should produce the same cache key.
        let build_marker_cell = |parent: Arc<Layer>| {
            let mut b = LayerBuilder::new("cell", Some(parent));
            let mut r = Resource::new(iri("urn:eigenius:demo:cell"));
            set_default_is_a(&mut r);
            r.set(
                iri("urn:eigenius:demo:Marker"),
                Value::String("attached".into()),
            );
            r.set(
                iri("urn:eigenius:core:description"),
                Value::String("v1".into()),
            );
            b.add_resource(r).unwrap();
            b
        };

        let first = commit_layer_with_cache(
            build_marker_cell(Arc::clone(&support_a)),
            storage.clone(),
            &backend,
        )
        .unwrap();
        let first_id = match first {
            AnchoredCommitOutcome::Miss { layer } => {
                // Confirm the supporting layer is support_a, not root_a
                // — that's the property we're testing.
                assert_eq!(layer.supporting_layer(), Some(support_a.id()));
                layer.id().clone()
            }
            AnchoredCommitOutcome::Hit { .. } => panic!("first commit must miss"),
        };

        let second =
            commit_layer_with_cache(build_marker_cell(Arc::clone(&support_b)), storage, &backend)
                .unwrap();
        match second {
            AnchoredCommitOutcome::Hit { cached_layer_id } => {
                assert_eq!(
                    cached_layer_id, first_id,
                    "supporting-equivalent context must hit the same cache entry"
                );
            }
            AnchoredCommitOutcome::Miss { .. } => {
                panic!("supporting-equivalent context must hit the cache (D33 §6)")
            }
        }
        // Cache still has exactly one entry — both runs share it.
        assert_eq!(backend.list_anchored_commits().unwrap().len(), 1);
    }

    /// Phase 20c: a layer with no supporting layer (its only IRI
    /// references are to itself) bypasses the cache entirely. The
    /// commit goes through the standard path; no cache entry is
    /// produced.
    #[test]
    fn anchored_commit_bypassed_when_no_supporting_layer() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();

        // A root layer is self-contained — it declares its own vocabulary (the core
        // ontology IS the root, parent=None) so its references resolve within itself,
        // leaving no supporting layer below. This is the genuine "no supporting layer"
        // shape under reference integrity (Rule 22 §(c)): a self-contained layer, not a
        // coreless fragment.
        let core_json = include_str!("../../ontologies/core/core-ontology.json");
        let mut rb = LayerBuilder::new("self-contained", None);
        for r in crate::ontology::eigon_json::parse_document(core_json).unwrap() {
            rb.add_resource(r).unwrap();
        }
        rb.add_resource(make_resource("urn:eigenius:test:r"))
            .unwrap();
        let outcome = commit_layer_with_cache(rb, storage, &backend).unwrap();
        match outcome {
            AnchoredCommitOutcome::Miss { layer } => {
                assert!(layer.supporting_layer().is_none());
            }
            AnchoredCommitOutcome::Hit { .. } => panic!("no supporting → must not cache hit"),
        }
        // No cache entry was written.
        assert_eq!(backend.list_anchored_commits().unwrap().len(), 0);
    }
}
