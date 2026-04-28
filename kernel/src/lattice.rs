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
use std::collections::{BTreeSet, VecDeque};
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
/// happened: `FastForward` on a clean CAS, `TrivialMerge` if the
/// branch's actual head and `new_head` modify disjoint IRIs since their
/// LCA (Phase 14e), or `NeedsWitnessedMerge` on genuine conflict.
///
/// **Storage parameter.** The trivial-merge path needs a `LayerStorage`
/// to construct the merge layer (cache + bloom + backend bundle). The
/// FastForward and StrictFastForward paths don't use it; pass any
/// valid storage (typically `LayerStorage::with_persistent(backend)`).
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
    storage: LayerStorage,
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

    // Divergence — actual ≠ expected. The branch is somewhere other
    // than where the caller started from.
    match policy {
        ConflictPolicy::StrictFastForward => Err(BranchUpdateError::StrictFastForwardViolation {
            branch: name.to_string(),
            expected: expected_old_head,
            actual,
        }),
        ConflictPolicy::AllowTrivial => {
            // If the branch was deleted (actual is None), trivial
            // merge has nothing to merge against — surface as a
            // witnessed-merge requirement.
            let actual_head = match actual {
                Some(h) => h,
                None => {
                    return Ok(UpdateOutcome::NeedsWitnessedMerge {
                        current_head: LayerId([0u8; 32]),
                        conflicting_iris: Vec::new(),
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
pub fn merge_independent_heads(
    heads: Vec<LayerId>,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<MergeOutcome, MergeError> {
    use crate::storage::ResourceBackend;

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

    // Single-head: nothing to merge. Reconstruct the Layer and return
    // it so callers get a uniform `Merged { merge_layer }` outcome.
    if heads.len() == 1 {
        let head_id = &heads[0];
        let info = backend
            .load_chain_from(head_id)
            .map_err(MergeError::Storage)?
            .ok_or_else(|| {
                MergeError::InvalidHeads(format!("could not load chain for head {head_id}"))
            })?;
        let layer = crate::layer::build_chain(info, storage);
        return Ok(MergeOutcome::Merged { merge_layer: layer });
    }

    // Compute LCA. Per the bootstrap-chain invariant, this should always
    // exist for layers persisted via the kernel.
    let lca = find_lca(&heads, &topology).ok_or_else(|| {
        MergeError::InvalidHeads("no common ancestor found — heads belong to disjoint DAGs".into())
    })?;

    // For each head, compute its IRI → source-layer map.
    let mut per_head_sources: Vec<std::collections::BTreeMap<Iri, LayerId>> =
        Vec::with_capacity(heads.len());
    for head in &heads {
        let sources =
            iri_sources_since(head, &lca, &topology, backend).map_err(MergeError::Storage)?;
        per_head_sources.push(sources);
    }

    // Pairwise-disjoint check. Conflicts = IRIs touched by ≥ 2 heads.
    let mut iri_to_head_count: std::collections::BTreeMap<&Iri, u32> =
        std::collections::BTreeMap::new();
    for sources in &per_head_sources {
        for iri in sources.keys() {
            *iri_to_head_count.entry(iri).or_insert(0) += 1;
        }
    }
    let conflicts: Vec<Iri> = iri_to_head_count
        .iter()
        .filter_map(|(iri, count)| {
            if *count >= 2 {
                Some((*iri).clone())
            } else {
                None
            }
        })
        .collect();
    if !conflicts.is_empty() {
        return Ok(MergeOutcome::Conflict {
            conflicting_iris: conflicts,
        });
    }

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

    let merge_layer = commit_layer(builder, storage, backend).map_err(|e| match e {
        CommitError::Validation(v) => MergeError::Validation(v),
        CommitError::Storage(s) => MergeError::Storage(s),
        CommitError::Layer(_) => unreachable!("builder errors handled above"),
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
        storage: &LayerStorage,
    ) -> Arc<Layer> {
        let mut b = LayerBuilder::new(name, None);
        b.add_resource(make_resource("urn:eigenius:core:r"))
            .unwrap();
        commit_layer(b, storage.clone(), backend).unwrap()
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
            .load_resource(layer.id(), &iri("urn:eigenius:core:r"))
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
        let child = commit_layer(child_b, storage.clone(), &backend).unwrap();

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
        r_a.set(
            iri("urn:eigenius:core:description"),
            Value::String("from a".into()),
        );
        a_b.add_resource(r_a).unwrap();
        let a = commit_layer(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        let mut r_b = Resource::new(iri(conflict_iri));
        r_b.set(
            iri("urn:eigenius:core:description"),
            Value::String("from b".into()),
        );
        b_b.add_resource(r_b).unwrap();
        let b = commit_layer(b_b, storage.clone(), &backend).unwrap();

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
        let a = commit_layer(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer(b_b, storage.clone(), &backend).unwrap();

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
        let a = commit_layer(ab, storage.clone(), &backend).unwrap();

        let mut bb = LayerBuilder::new("b", Some(Arc::clone(&a)));
        bb.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer(bb, storage.clone(), &backend).unwrap();

        let mut cb = LayerBuilder::new("c", Some(Arc::clone(&b)));
        cb.add_resource(make_resource("urn:eigenius:example:c"))
            .unwrap();
        let c = commit_layer(cb, storage.clone(), &backend).unwrap();

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
        let a = commit_layer(ab, storage.clone(), &backend).unwrap();

        let mut bb = LayerBuilder::new("b", Some(Arc::clone(&root)));
        bb.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer(bb, storage.clone(), &backend).unwrap();

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
        let a = commit_layer(ab, storage.clone(), &backend).unwrap();

        let mut leaves = Vec::new();
        for tag in ["x", "y", "z"] {
            let mut lb = LayerBuilder::new(tag, Some(Arc::clone(&a)));
            lb.add_resource(make_resource(&format!("urn:eigenius:example:{tag}")))
                .unwrap();
            leaves.push(commit_layer(lb, storage.clone(), &backend).unwrap());
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
        let mid = commit_layer(mid_b, storage.clone(), &backend).unwrap();

        let mut tip_b = LayerBuilder::new("tip", Some(Arc::clone(&mid)));
        tip_b
            .add_resource(make_resource("urn:eigenius:example:t"))
            .unwrap();
        let tip = commit_layer(tip_b, storage.clone(), &backend).unwrap();

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
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String("v1".into()),
        );
        mid_b.add_resource(r).unwrap();
        let mid = commit_layer(mid_b, storage.clone(), &backend).unwrap();

        let mut tip_b = LayerBuilder::new("tip", Some(Arc::clone(&mid)));
        let mut r2 = Resource::new(iri("urn:eigenius:example:x"));
        r2.set(
            iri("urn:eigenius:core:description"),
            Value::String("v2".into()),
        );
        tip_b.add_resource(r2).unwrap();
        let tip = commit_layer(tip_b, storage.clone(), &backend).unwrap();

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
            heads.push(commit_layer(b, storage.clone(), &backend).unwrap());
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
        r_a.set(
            iri("urn:eigenius:core:description"),
            Value::String("a".into()),
        );
        a_b.add_resource(r_a).unwrap();
        let a = commit_layer(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        let mut r_b = Resource::new(iri(conflict_iri));
        r_b.set(
            iri("urn:eigenius:core:description"),
            Value::String("b".into()),
        );
        b_b.add_resource(r_b).unwrap();
        let b = commit_layer(b_b, storage.clone(), &backend).unwrap();

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
        r_a.set(
            iri("urn:eigenius:core:description"),
            Value::String("from a".into()),
        );
        a_b.add_resource(r_a).unwrap();
        let a = commit_layer(a_b, storage.clone(), &backend).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        let mut r_b = Resource::new(iri("urn:eigenius:example:b"));
        r_b.set(
            iri("urn:eigenius:core:description"),
            Value::String("from b".into()),
        );
        b_b.add_resource(r_b).unwrap();
        let b = commit_layer(b_b, storage.clone(), &backend).unwrap();

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
        let a = commit_layer(a_b, storage.clone(), &backend).unwrap();
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
        let b = commit_layer(b_b, storage.clone(), &backend).unwrap();
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
        let a = commit_layer(a_b, storage.clone(), backend.as_ref()).unwrap();

        let mut b_b = LayerBuilder::new("b", Some(Arc::clone(&root)));
        b_b.add_resource(make_resource("urn:eigenius:example:b"))
            .unwrap();
        let b = commit_layer(b_b, storage.clone(), backend.as_ref()).unwrap();

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
}
