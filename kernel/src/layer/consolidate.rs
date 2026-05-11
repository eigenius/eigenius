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

//! Chain consolidation (D25 — Phase 17).
//!
//! Collapses a contiguous ancestral range `[from..to]` of layers on a
//! branch into a single consolidated layer `L_c` whose parent is
//! `parent(from)`. The consolidated layer is *resolve-equivalent* to
//! the original range under head substitution: for any IRI, the value
//! head-rooted reads return is unchanged before and after.
//!
//! See [D25 §4](../../../docs/design/d25-chain-consolidation.md) for
//! the resolve-equivalence invariant and [§6](../../../docs/design/d25-chain-consolidation.md)
//! for the top-of-stack walk algorithm.
//!
//! **Milestone status (D25 §11.1):**
//! - 17a — top-of-stack algorithm + branch CAS for `to = head`. ✅
//! - 17b — range validation: ancestral / merge-free / pin-free. ✅
//! - 17c — bloom-cache eviction for collapsed layers. ✅
//! - 17d — cost estimation gate + `estimate_consolidation` dry-run. ✅
//! - 17e — CLI (`db consolidate`) + gRPC
//!   (`ConsolidateChain` / `EstimateConsolidation`) surfaces. ✅
//!
//! Deferred from Phase 17: `db consolidate-summary` (the diagnostic
//! enumeration of past consolidations). It needs a separate
//! consolidation-record storage shape — D25 §6 sketches an embedded
//! property, but that would carry a timestamp into the content hash
//! and break the determinism property 17a / 17d tests pin. A
//! dedicated CF keyed by the consolidated layer id is the natural
//! resolution; tracked as a follow-up rather than blocking 17e.
//!
//! The `ConsolidateError` enum ships with every final variant so
//! downstream code can match exhaustively even before later milestones
//! land their corresponding validations.

use crate::layer::{Layer, LayerBuilder, LayerId, LayerStorage};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::storage::{PersistentBackend, StorageError};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Options governing a `consolidate_chain` call.
#[derive(Debug, Clone)]
pub struct ConsolidateOpts {
    /// Cost-estimation cap: if the predicted top-of-stack walk would
    /// exceed this many resource entries, return `CostExceedsCap`
    /// before computing. Default: `5_000_000`; deployment-tunable via
    /// `EIGENIUS_CONSOLIDATE_MAX_WALK_ENTRIES`.
    ///
    /// Predicted walk size is the upper bound
    /// `sum(handle.resource_count for handle in range)` — counted
    /// before the dedup pass, so ranges with heavy rewrites can trip
    /// the cap even when the actual dedup'd walk would be modest
    /// (D25 §12.5).
    pub max_walk_entries: u64,
    /// Trace-pin handling. v1 ships `Refuse` — the only supported
    /// policy. The variant exists on the API for forward compatibility
    /// with v2 re-pointing / invalidation policies (D25 §7.2).
    pub trace_pin_policy: TracePinPolicy,
    /// Layers pinned by external state — typically `TaskRecord.layer_head`
    /// values across active sessions (D21). Caller-supplied because the
    /// kernel doesn't enumerate sessions; the same pattern GC uses via
    /// `GcRoots.task_pins`.
    ///
    /// Map value is the pin count for that layer. v1 surfaces this in
    /// the typed error so the operator can tell whether a single stale
    /// task is blocking consolidation versus a busy workload genuinely
    /// using the range.
    ///
    /// Empty (the default) means "no pins known to the caller" —
    /// equivalent to skipping the pin check. Production callers should
    /// populate this from the task store before invoking; the CLI / gRPC
    /// surfaces in 17e make this a first-class concern.
    pub pinned_layers: BTreeMap<LayerId, u64>,
}

impl Default for ConsolidateOpts {
    fn default() -> Self {
        Self {
            max_walk_entries: 5_000_000,
            trace_pin_policy: TracePinPolicy::Refuse,
            pinned_layers: BTreeMap::new(),
        }
    }
}

/// Policy for handling trace pins inside the consolidation range.
///
/// v1 only implements `Refuse`. The non-`Refuse` variants are
/// reserved for v2 (D25 §7.2) and currently unhandled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracePinPolicy {
    /// v1 default: refuse if the range contains trace-pinned layers.
    Refuse,
    /// v2 (not implemented): re-point trace pins to the consolidated
    /// layer.
    RepointOnConsolidate,
    /// v2 (not implemented): mark pins stale; trace becomes
    /// uninspectable past the consolidation point.
    Invalidate,
}

/// Successful outcome of `consolidate_chain`.
#[derive(Debug, Clone)]
pub struct ConsolidationOutcome {
    /// The position hash of the freshly-committed consolidated layer.
    pub consolidated_layer: LayerId,
    /// Number of layers in the original `[from..to]` range. Equals
    /// the number of layers the chain shortens by (minus one, since
    /// `L_c` replaces them).
    pub collapsed_layer_count: u64,
    /// Crude upper bound on the bytes that the next GC pass will be
    /// able to reclaim. v1 reports `0` — operators using
    /// `db consolidate-summary` (17e) can read the pre-/post-
    /// consolidation chain size for the same effect at lower wire
    /// cost. Accurate per-call sizing is a v2 nice-to-have.
    pub reclaimable_bytes_estimate: u64,
    /// `true` if the branch's head moved as part of the operation.
    /// Always `true` in the current build (the only supported case);
    /// future no-op consolidations may return `false`.
    pub head_advanced: bool,
}

/// Typed errors returned by `consolidate_chain`.
///
/// Not `Clone` because `StorageError` isn't (matches the
/// `BranchUpdateError` precedent in [`crate::lattice`]); callers
/// pattern-match once and either bubble up or log.
#[derive(Debug)]
pub enum ConsolidateError {
    /// `from` is not an ancestor of `to`, or `to` is not the branch's
    /// current head.
    RangeNotAncestral { from: LayerId, to: LayerId },
    /// The branch ref didn't match the expected head: either the
    /// branch doesn't exist or its head moved since the caller
    /// captured `to`.
    BranchAdvancedConcurrently {
        observed_head: Option<LayerId>,
        expected_head: LayerId,
    },
    /// The range contains a multi-parent merge layer. v1 refuses;
    /// v2 multi-parent consolidation is the §8.2 sketch. Surfaced
    /// in 17b; the variant ships now for stable matching.
    RangeContainsMergeNode { merge_layer: LayerId },
    /// The range contains a layer with active trace pins. v1 refuses
    /// per `TracePinPolicy::Refuse`. Surfaced in 17b.
    RangeContainsTracePin {
        pinned_layer: LayerId,
        trace_count: u64,
    },
    /// Predicted walk exceeds `opts.max_walk_entries`. Surfaced in 17d.
    CostExceedsCap { predicted_entries: u64 },
    /// Underlying storage write failure.
    WriteFailed(StorageError),
    /// A referenced layer or resource was absent from storage. Usually
    /// indicates DB corruption or a programming bug — the caller
    /// already validated the range via `from`/`to` so storage misses
    /// here are unexpected.
    Internal(String),
}

impl std::fmt::Display for ConsolidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsolidateError::RangeNotAncestral { from, to } => write!(
                f,
                "consolidation range invalid: {from} is not an ancestor of {to}"
            ),
            ConsolidateError::BranchAdvancedConcurrently {
                observed_head,
                expected_head,
            } => write!(
                f,
                "branch advanced concurrently: expected head {expected_head}, observed {observed_head:?}"
            ),
            ConsolidateError::RangeContainsMergeNode { merge_layer } => write!(
                f,
                "consolidation range contains merge node {merge_layer}"
            ),
            ConsolidateError::RangeContainsTracePin {
                pinned_layer,
                trace_count,
            } => write!(
                f,
                "consolidation range contains layer {pinned_layer} pinned by {trace_count} trace(s)"
            ),
            ConsolidateError::CostExceedsCap { predicted_entries } => write!(
                f,
                "consolidation walk would exceed cost cap: {predicted_entries} predicted entries"
            ),
            ConsolidateError::WriteFailed(e) => write!(f, "consolidation write failed: {e}"),
            ConsolidateError::Internal(msg) => write!(f, "consolidation internal error: {msg}"),
        }
    }
}

impl std::error::Error for ConsolidateError {}

/// Consolidate the range `[from..to]` on `branch` into a single
/// resolve-equivalent layer.
///
/// `to` must equal the branch's current head; `from` must be an
/// ancestor of `to`. The consolidated layer's parent is `parent(from)`
/// (which is `None` if `from` is the chain root — the consolidated
/// layer becomes the new root).
///
/// The algorithm (D25 §6):
///
/// 1. Look up the branch's current head; verify it equals `to`.
/// 2. Walk the chain from `to` head→root, capturing layers until
///    `from` is reached. This is the consolidation range.
/// 3. For each IRI in the range's defined-iri union, record the
///    value from the *topmost* defining layer (first encountered in
///    the head→root walk). This is the top-of-stack value.
/// 4. Build a new `Layer` with `parent = parent(from)` and the
///    collected `(iri → resource)` pairs.
/// 5. Persist the new layer via `PersistentBackend::store_layer`
///    (atomic WriteBatch per D23 §6.3).
/// 6. CAS the branch ref to the new layer under the process-wide
///    branch lock (consistent with `lattice::update_branch`).
///
/// **17a limitations.**
/// - Range validation against merge nodes and trace pins is deferred
///   to 17b. 17a will consolidate across a merge node if you feed it
///   one — the produced layer is still resolve-equivalent for
///   head-rooted reads but loses the merge's resolution decisions.
///   This is safe for 17a's hand-constructed test ranges (no merges
///   in them) but not safe to expose to operators yet.
/// - Bloom cache eviction lands in 17c.
/// - Cost cap lands in 17d.
/// - The audit `consolidation_record` property on the consolidated
///   layer (D25 §6 last paragraph) is deliberately omitted: it would
///   embed a non-deterministic timestamp and break the determinism
///   property the milestone explicitly tests. It lands when 17e adds
///   the `db consolidate-summary` surface.
pub fn consolidate_chain(
    branch: &str,
    from: LayerId,
    to: LayerId,
    opts: ConsolidateOpts,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<ConsolidationOutcome, ConsolidateError> {
    // Serialize with `update_branch` and other branch-mutating
    // operations via the process-wide branch lock. Holding the lock
    // across the read-walk + store + CAS sequence makes the operation
    // logically atomic against concurrent branch updates (D23 §6.3's
    // "single WriteBatch" language is per-layer; the layer + branch
    // CAS pair stays consistent via the lock, the same pattern
    // `update_branch` uses today).
    crate::lattice::with_branch_lock(|| {
        consolidate_chain_locked(branch, from, to, opts, storage, backend)
    })
}

fn consolidate_chain_locked(
    branch: &str,
    from: LayerId,
    to: LayerId,
    opts: ConsolidateOpts,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<ConsolidationOutcome, ConsolidateError> {
    // Capture an Arc to the bloom cache before `storage` is consumed
    // by the prep helper; we evict the collapsed layers' bloom
    // entries at the tail of the operation (D25 §9). Cloning is
    // cheap — the bundle holds Arcs internally.
    let bloom_cache = Arc::clone(&storage.bloom_cache);

    let prep = prepare_consolidation(branch, from, to, &opts, storage, backend)?;
    let Prepared {
        consolidated_layer,
        range_layers,
        collapsed_layer_count,
        ..
    } = prep;

    // Persist the consolidated layer. `store_layer` writes the topo
    // entry, bloom, content-hash index, resources, and chain pointer
    // in one atomic WriteBatch per D23 §6.3. The fresh bloom for
    // `consolidated_layer` is pre-populated in the cache by
    // `LayerBuilder::build` — no separate insert needed.
    backend
        .store_layer(&consolidated_layer)
        .map_err(ConsolidateError::WriteFailed)?;

    // Advance the branch ref. Inside the branch lock so a concurrent
    // `update_branch` can't interleave between the head check above
    // and the put here.
    backend
        .put_branch(branch, consolidated_layer.id())
        .map_err(ConsolidateError::WriteFailed)?;

    // Bloom-cache eviction for the collapsed range (D25 §9). After
    // the branch CAS, head-rooted resolves no longer reach these
    // layers; their bloom entries are dead weight in the cache.
    // GC reuses the same `evict_layer` hook when it actually deletes
    // the layers; consolidation is an early trigger for bloom-side
    // eviction. (The resource cache and triple index entries stay
    // until GC actually removes the layers — they're keyed by
    // `LayerId` and won't be queried after the branch advances, so
    // the cost is bounded.)
    for layer in &range_layers {
        bloom_cache.evict_layer(layer.id());
    }

    Ok(ConsolidationOutcome {
        consolidated_layer: consolidated_layer.id().clone(),
        collapsed_layer_count,
        reclaimable_bytes_estimate: 0,
        head_advanced: true,
    })
}

/// Non-mutating cost preview for a `consolidate_chain` call.
///
/// Runs the same validation, range walk, and top-of-stack build that
/// `consolidate_chain` runs — and returns the predicted
/// [`LayerId`] of the would-be consolidated layer — but does *not*
/// persist the layer or advance the branch ref. The same typed errors
/// (`RangeNotAncestral`, `RangeContainsMergeNode`,
/// `RangeContainsTracePin`, `CostExceedsCap`, …) surface here too.
///
/// Backs the [`ConsolidateChain` `--dry-run`](D25 §5.3) CLI flag and
/// the `EstimateConsolidation` gRPC (D25 §5.2). The operator pipes the
/// estimate's `predicted_consolidated_layer` into a follow-up real
/// `consolidate_chain` call to confirm the operation is doing what
/// they expect.
///
/// **Cache footprint.** The estimate path builds the consolidated
/// layer through `LayerBuilder::build`, which pre-populates the local
/// storage bundle's bloom and resource caches. The layer is *not*
/// persisted, so a subsequent `consolidate_chain` against the same
/// range will produce the same `LayerId` and write it idempotently;
/// the cached state survives and is reused.
pub fn estimate_consolidation(
    branch: &str,
    from: LayerId,
    to: LayerId,
    opts: ConsolidateOpts,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<ConsolidationEstimate, ConsolidateError> {
    crate::lattice::with_branch_lock(|| {
        let prep = prepare_consolidation(branch, from, to, &opts, storage, backend)?;
        Ok(ConsolidationEstimate {
            predicted_consolidated_layer: prep.consolidated_layer.id().clone(),
            collapsed_layer_count: prep.collapsed_layer_count,
            predicted_walk_entries: prep.predicted_walk_entries,
            actual_walk_entries: prep.actual_walk_entries,
        })
    })
}

/// Cost preview returned by [`estimate_consolidation`].
#[derive(Debug, Clone)]
pub struct ConsolidationEstimate {
    /// The `LayerId` the consolidated layer would have if
    /// `consolidate_chain` were invoked with the same inputs.
    /// Content-addressed: the same range against the same parent
    /// produces the same id across runs.
    pub predicted_consolidated_layer: LayerId,
    /// Number of layers in `[from..to]` that would be collapsed.
    pub collapsed_layer_count: u64,
    /// Upper-bound prediction of the top-of-stack walk size. Computed
    /// as `sum(handle.resource_count for handle in range)`. This is
    /// the value the cost cap (`ConsolidateOpts.max_walk_entries`) is
    /// checked against — it's an upper bound because the walk skips
    /// IRIs already seen in topper layers (D25 §12.5).
    pub predicted_walk_entries: u64,
    /// Actual deduplicated walk size after top-of-stack. Equals the
    /// number of distinct IRIs across the range. Always
    /// `≤ predicted_walk_entries`; the gap is the dedup savings (large
    /// when the range contains heavy rewrites of the same IRI).
    pub actual_walk_entries: u64,
}

/// Internal state that `consolidate_chain` and `estimate_consolidation`
/// both produce — the result of validation + range walk + top-of-stack
/// build. The persist + branch CAS + bloom-evict steps live only on
/// the mutating path.
struct Prepared {
    consolidated_layer: Arc<Layer>,
    /// Layers in `[from..to]` in head→root order. Carried out for
    /// bloom-cache eviction on the mutating path; unused by the
    /// estimate path.
    range_layers: Vec<Arc<Layer>>,
    collapsed_layer_count: u64,
    predicted_walk_entries: u64,
    actual_walk_entries: u64,
}

/// Shared prep: verify the branch head, validate the range against
/// the typed checks, evaluate the cost cap, run the top-of-stack
/// walk, and build the consolidated layer. Both `consolidate_chain`
/// and `estimate_consolidation` lower to this function; the persist +
/// CAS steps are layered on top by the former.
///
/// Must be called from inside the branch lock.
fn prepare_consolidation(
    branch: &str,
    from: LayerId,
    to: LayerId,
    opts: &ConsolidateOpts,
    storage: LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<Prepared, ConsolidateError> {
    // Verify the branch's current head matches `to`. v1 collapses
    // onto the tip only: any layers above `to` would still encode
    // `to` (or its ancestors) in their position hashes, so changing
    // their parent to `L_c` would cascade re-ids through head.
    //
    // **v2 relaxation (forward pointers).** The cascade can be
    // avoided by introducing a *resolve redirect* on `to` —
    // metadata that says "walk through `L_c` instead of my content"
    // — without touching the topology hashes. Layers above `to`
    // keep their parent pointers; only the resolve walk
    // short-circuits. The redirect lives next to the topo entry,
    // not inside the hash domain. Captured in D25 §12.8.
    //
    // For v1 the `to = head` restriction is enough for the
    // "notebook session squash" pattern, which is the primary
    // workload. The forward-pointer machinery lands when an
    // operator-facing requirement justifies it.
    let observed_head = backend
        .get_branch(branch)
        .map_err(ConsolidateError::WriteFailed)?;
    if observed_head.as_ref() != Some(&to) {
        return Err(ConsolidateError::BranchAdvancedConcurrently {
            observed_head,
            expected_head: to,
        });
    }

    // Load the chain from `to`. The handles retain authoritative
    // multi-parent topology (`build_chain` would collapse it to
    // `parents.first()` for single-parent Layer reconstruction), so
    // we validate against handles before the chain is rebuilt.
    let info = backend
        .load_chain_from(&to)
        .map_err(ConsolidateError::WriteFailed)?
        .ok_or_else(|| ConsolidateError::Internal(format!("chain absent for head {to}")))?;

    // Validate the range against the on-disk handles. `info.handles`
    // is in root→head order; we walk it reversed (head→root) for the
    // validation and bail on the first reject. The cost-cap predicate
    // is computed during the same walk from `handle.resource_count`
    // (the recorded `defined_iris.len()` per layer) — predicted
    // walk-entry count is an upper bound on the actual top-of-stack
    // pass (D25 §12.5).
    let mut range_ids: Vec<LayerId> = Vec::new();
    let mut predicted_walk_entries: u64 = 0;
    let mut found_from = false;
    for handle in info.handles.iter().rev() {
        if handle.parents.len() > 1 {
            return Err(ConsolidateError::RangeContainsMergeNode {
                merge_layer: handle.id.clone(),
            });
        }
        if opts.trace_pin_policy == TracePinPolicy::Refuse {
            if let Some(&trace_count) = opts.pinned_layers.get(&handle.id) {
                if trace_count > 0 {
                    return Err(ConsolidateError::RangeContainsTracePin {
                        pinned_layer: handle.id.clone(),
                        trace_count,
                    });
                }
            }
        }
        predicted_walk_entries = predicted_walk_entries.saturating_add(handle.resource_count);
        range_ids.push(handle.id.clone());
        if handle.id == from {
            found_from = true;
            break;
        }
    }
    if !found_from {
        return Err(ConsolidateError::RangeNotAncestral { from, to });
    }

    // Cost-cap gate (D25 §6). The cap is checked *before* the
    // expensive top-of-stack walk so we fail fast on pathological
    // ranges. The bound is conservative — `predicted_walk_entries`
    // counts every (layer, defined_iri) pair before dedup, so a
    // range that *would* dedup heavily under the actual walk can
    // still trip the cap. v1 accepts this; v2 may invest in a tighter
    // estimate (§12.5).
    if predicted_walk_entries > opts.max_walk_entries {
        return Err(ConsolidateError::CostExceedsCap {
            predicted_entries: predicted_walk_entries,
        });
    }

    // Validation passed — reconstruct the chain so the top-of-stack
    // walk can call `get_resource`. The merge case is already
    // rejected above, so every layer we visit here is single-parent.
    let head = crate::layer::build_chain(info, storage.clone());
    let range_id_set: BTreeSet<&LayerId> = range_ids.iter().collect();
    let mut range_layers: Vec<Arc<Layer>> = Vec::new();
    let mut parent_of_from: Option<Arc<Layer>> = None;
    let mut current: Option<Arc<Layer>> = Some(head);
    while let Some(layer) = current {
        let is_from = layer.id() == &from;
        let next = layer.parent().cloned();
        if range_id_set.contains(layer.id()) {
            range_layers.push(Arc::clone(&layer));
        }
        if is_from {
            parent_of_from = next.clone();
            break;
        }
        current = next;
    }

    // Top-of-stack: walk the range head→root (already the walk order
    // above) and record the first-seen value for each IRI.
    let mut seen_iris: BTreeSet<Iri> = BTreeSet::new();
    let mut consolidated_resources: Vec<Resource> = Vec::new();
    for layer in &range_layers {
        for iri in layer.defined_iris() {
            if !seen_iris.insert(iri.clone()) {
                continue;
            }
            let resource = layer.get_resource(iri).ok_or_else(|| {
                ConsolidateError::Internal(format!(
                    "layer {} claims to define {iri} but get_resource returned None",
                    layer.id()
                ))
            })?;
            consolidated_resources.push((*resource).clone());
        }
    }

    let collapsed_layer_count = range_layers.len() as u64;
    let actual_walk_entries = seen_iris.len() as u64;

    // Build the consolidated layer. Name carries the range as a
    // diagnostic hint (it's metadata-only, not in any hash) so log
    // output and inspect surfaces can attribute the layer back to
    // its origin.
    let from_short = &format!("{from}")[..8.min(format!("{from}").len())].to_string();
    let to_short = &format!("{to}")[..8.min(format!("{to}").len())].to_string();
    let name = format!("consolidated:{from_short}..{to_short}");
    let mut builder = match parent_of_from.clone() {
        Some(parent) => LayerBuilder::new(&name, Some(parent)),
        None => LayerBuilder::new(&name, None),
    };
    for resource in consolidated_resources {
        builder.add_resource(resource).map_err(|e| {
            ConsolidateError::Internal(format!("consolidated layer rejected resource: {e}"))
        })?;
    }
    let consolidated_layer = Arc::new(builder.build(storage));

    Ok(Prepared {
        consolidated_layer,
        range_layers,
        collapsed_layer_count,
        predicted_walk_entries,
        actual_walk_entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::resource::Value;
    use crate::storage::memory::MemoryPersistentBackend;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_resource(id: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        for (k, v) in props {
            r.set(iri(k), v);
        }
        r
    }

    /// Build a chain of `n` layers on top of `root`. Each layer
    /// defines a single resource `urn:eigenius:demo:layer_{i}` with
    /// a `description` of `"v{i}"`. Returns the head layer.
    ///
    /// Storage backed by the supplied `backend` so the layers are
    /// persistent for `consolidate_chain` to find.
    fn build_chain_of(
        n: usize,
        backend: &dyn PersistentBackend,
    ) -> (Arc<Layer>, Vec<Arc<Layer>>, LayerStorage) {
        // In-memory storage for the per-layer build pipeline; the
        // resources also land in the persistent backend below via
        // `store_layer`, which is what `consolidate_chain` reads
        // through during the top-of-stack walk.
        let storage = LayerStorage::in_memory();

        // Root layer defines a couple of core resources the chain
        // references.
        let mut rb = LayerBuilder::new("root", None);
        rb.add_resource(make_resource("urn:eigenius:core:Class", vec![]))
            .unwrap();
        rb.add_resource(make_resource("urn:eigenius:core:description", vec![]))
            .unwrap();
        let root = Arc::new(rb.build(storage.clone()));
        backend.store_layer(&root).unwrap();

        let mut all = vec![Arc::clone(&root)];
        let mut current = Arc::clone(&root);
        for i in 0..n {
            let mut b = LayerBuilder::new(&format!("L{i}"), Some(Arc::clone(&current)));
            b.add_resource(make_resource(
                &format!("urn:eigenius:demo:layer_{i}"),
                vec![(
                    "urn:eigenius:core:description",
                    Value::String(format!("v{i}")),
                )],
            ))
            .unwrap();
            let layer = Arc::new(b.build(storage.clone()));
            backend.store_layer(&layer).unwrap();
            all.push(Arc::clone(&layer));
            current = layer;
        }
        (current, all, storage)
    }

    /// Snapshot every (IRI → value) pair reachable head→root from a
    /// chain head. Used by the resolve-equivalence regression: the
    /// snapshot before consolidation must equal the snapshot after.
    fn snapshot_chain(head: &Arc<Layer>) -> Vec<(Iri, String)> {
        let mut out = Vec::new();
        for (iri, resource) in head.iter_all_resources() {
            let desc = resource
                .get(&Iri::parse("urn:eigenius:core:description").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            out.push((iri.clone(), desc));
        }
        out.sort();
        out
    }

    /// Smallest interesting case: 10-layer chain (root + 9 commits),
    /// consolidate the middle 5. Confirms the consolidated layer is
    /// stored, the branch head advances, and resolve-equivalence
    /// holds for every IRI in the chain.
    #[test]
    fn consolidates_ten_layer_chain_preserving_resolves() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(9, &backend);
        backend.put_branch("main", head.id()).unwrap();

        // Snapshot the chain before consolidation.
        let before = snapshot_chain(&head);

        // Consolidate L2..L6 (a 5-layer middle window). Indices into
        // `layers`: layers[0] is root; layers[1] is L0; layers[3] is
        // L2; layers[7] is L6; layers[9] is L8 (the head).
        let from = layers[3].id().clone(); // L2
        let to = head.id().clone(); // L8 (also the head)
        let outcome = consolidate_chain(
            "main",
            from.clone(),
            to.clone(),
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("consolidation succeeds");

        assert_eq!(outcome.collapsed_layer_count, 7); // L2..L8 inclusive
        assert!(outcome.head_advanced);

        // The branch head now points at the consolidated layer.
        let new_head = backend.get_branch("main").unwrap().unwrap();
        assert_eq!(new_head, outcome.consolidated_layer);

        // Rebuild the new chain and verify resolve-equivalence.
        let info = backend.load_chain_from(&new_head).unwrap().unwrap();
        let new_head_layer = crate::layer::build_chain(info, storage);
        let after = snapshot_chain(&new_head_layer);
        assert_eq!(
            before, after,
            "consolidation must preserve head-rooted resolves for every IRI"
        );
    }

    /// 100-layer stress test. Same shape as the 10-layer case, just
    /// bigger. Confirms the walk + store + CAS scales linearly without
    /// pathology and the resolve-equivalence invariant still holds.
    #[test]
    fn consolidates_hundred_layer_chain_preserving_resolves() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(99, &backend);
        backend.put_branch("main", head.id()).unwrap();

        let before = snapshot_chain(&head);

        // Consolidate from L0 (layers[1]) to the head — squashes the
        // entire non-root span into one consolidated layer.
        let from = layers[1].id().clone();
        let to = head.id().clone();
        let outcome = consolidate_chain(
            "main",
            from,
            to,
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("consolidation succeeds");
        assert_eq!(outcome.collapsed_layer_count, 99);

        let new_head = backend.get_branch("main").unwrap().unwrap();
        assert_eq!(new_head, outcome.consolidated_layer);

        let info = backend.load_chain_from(&new_head).unwrap().unwrap();
        let new_head_layer = crate::layer::build_chain(info, storage);
        let after = snapshot_chain(&new_head_layer);
        assert_eq!(before, after);
    }

    /// Consolidating the same range twice produces the same
    /// `LayerId` — the content-addressed identity guarantees
    /// determinism. Pins the milestone criterion that two operators
    /// (or two retries) against the same range produce a single
    /// canonical consolidated layer.
    #[test]
    fn consolidated_layer_id_is_deterministic_across_runs() {
        let backend_a = MemoryPersistentBackend::new();
        let (head_a, layers_a, storage_a) = build_chain_of(20, &backend_a);
        backend_a.put_branch("main", head_a.id()).unwrap();
        let from = layers_a[5].id().clone();
        let to = head_a.id().clone();
        let outcome_a = consolidate_chain(
            "main",
            from.clone(),
            to.clone(),
            ConsolidateOpts::default(),
            storage_a,
            &backend_a,
        )
        .unwrap();

        // Build an independent chain on a fresh backend with the
        // same shape. Because each layer is content-addressed and
        // the resources are byte-identical between runs, every
        // LayerId in the second chain matches the first.
        let backend_b = MemoryPersistentBackend::new();
        let (head_b, layers_b, storage_b) = build_chain_of(20, &backend_b);
        assert_eq!(head_a.id(), head_b.id());
        backend_b.put_branch("main", head_b.id()).unwrap();
        let outcome_b = consolidate_chain(
            "main",
            layers_b[5].id().clone(),
            head_b.id().clone(),
            ConsolidateOpts::default(),
            storage_b,
            &backend_b,
        )
        .unwrap();

        assert_eq!(
            outcome_a.consolidated_layer, outcome_b.consolidated_layer,
            "two independent consolidations of the same range against the same parent \
             must produce the same content-addressed LayerId"
        );
    }

    /// Consolidating against a stale `to` (one that's not the
    /// branch's current head) returns the typed
    /// `BranchAdvancedConcurrently` error.
    #[test]
    fn refuses_consolidation_when_to_is_not_branch_head() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(5, &backend);
        backend.put_branch("main", head.id()).unwrap();

        // Aim at L2 (an interior layer), not the head L4.
        let to_interior = layers[3].id().clone();
        let from = layers[1].id().clone();
        let err = consolidate_chain(
            "main",
            from,
            to_interior.clone(),
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .unwrap_err();
        match err {
            ConsolidateError::BranchAdvancedConcurrently {
                observed_head,
                expected_head,
            } => {
                assert_eq!(observed_head.as_ref(), Some(head.id()));
                assert_eq!(expected_head, to_interior);
            }
            other => panic!("expected BranchAdvancedConcurrently, got {other:?}"),
        }
    }

    /// Consolidating with a `from` that's not in the chain returns
    /// `RangeNotAncestral`. A common operator mistake (pasted the
    /// wrong hex) should produce a clear error, not corruption.
    #[test]
    fn refuses_consolidation_when_from_is_not_an_ancestor() {
        let backend = MemoryPersistentBackend::new();
        let (head, _layers, storage) = build_chain_of(5, &backend);
        backend.put_branch("main", head.id()).unwrap();

        let bogus_from = LayerId([0xff; 32]);
        let err = consolidate_chain(
            "main",
            bogus_from.clone(),
            head.id().clone(),
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .unwrap_err();
        match err {
            ConsolidateError::RangeNotAncestral { from, to } => {
                assert_eq!(from, bogus_from);
                assert_eq!(&to, head.id());
            }
            other => panic!("expected RangeNotAncestral, got {other:?}"),
        }
    }

    // ─── 17b range validation ──────────────────────────────────────────

    /// Range crossing a merge node is refused per D25 §8.1. Build a
    /// fork at A with two children B1, B2; combine into merge M with
    /// `parents = [B1, B2]`; commit C on top of M. Asking to
    /// consolidate everything down to A trips the merge check and
    /// returns `RangeContainsMergeNode { merge_layer: M }` — the
    /// resolution decisions M encodes can't survive collapse in v1.
    #[test]
    fn refuses_consolidation_when_range_crosses_merge_node() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();

        // Root carries the core declarations every descendant references.
        let mut rb = LayerBuilder::new("root", None);
        rb.add_resource(make_resource("urn:eigenius:core:Class", vec![]))
            .unwrap();
        rb.add_resource(make_resource("urn:eigenius:core:description", vec![]))
            .unwrap();
        let root = Arc::new(rb.build(storage.clone()));
        backend.store_layer(&root).unwrap();

        // A — single shared ancestor of the fork.
        let mut ab = LayerBuilder::new("A", Some(Arc::clone(&root)));
        ab.add_resource(make_resource("urn:eigenius:demo:A_marker", vec![]))
            .unwrap();
        let a = Arc::new(ab.build(storage.clone()));
        backend.store_layer(&a).unwrap();

        // Two children of A with disjoint IRIs.
        let mut b1b = LayerBuilder::new("B1", Some(Arc::clone(&a)));
        b1b.add_resource(make_resource("urn:eigenius:demo:B1_marker", vec![]))
            .unwrap();
        let b1 = Arc::new(b1b.build(storage.clone()));
        backend.store_layer(&b1).unwrap();

        let mut b2b = LayerBuilder::new("B2", Some(Arc::clone(&a)));
        b2b.add_resource(make_resource("urn:eigenius:demo:B2_marker", vec![]))
            .unwrap();
        let b2 = Arc::new(b2b.build(storage.clone()));
        backend.store_layer(&b2).unwrap();

        // Trivial merge layer M with parents [B1, B2]. Empty content;
        // its load-bearing trait is `parents().len() == 2`.
        let mb = LayerBuilder::with_parents("M", vec![Arc::clone(&b1), Arc::clone(&b2)]);
        let m = Arc::new(mb.build(storage.clone()));
        assert_eq!(m.parents().len(), 2);
        backend.store_layer(&m).unwrap();

        // C — child of M; becomes the branch head we'll point at.
        let mut cb = LayerBuilder::new("C", Some(Arc::clone(&m)));
        cb.add_resource(make_resource("urn:eigenius:demo:C_marker", vec![]))
            .unwrap();
        let c = Arc::new(cb.build(storage.clone()));
        backend.store_layer(&c).unwrap();

        backend.put_branch("main", c.id()).unwrap();

        // Attempt to consolidate [A..C]. The walk hits C (single-parent),
        // then M (two parents) — the check fires there.
        let err = consolidate_chain(
            "main",
            a.id().clone(),
            c.id().clone(),
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .unwrap_err();
        match err {
            ConsolidateError::RangeContainsMergeNode { merge_layer } => {
                assert_eq!(&merge_layer, m.id());
            }
            other => panic!("expected RangeContainsMergeNode, got {other:?}"),
        }
    }

    /// Range containing a layer the caller has flagged as pinned is
    /// refused per `TracePinPolicy::Refuse`. The error carries the
    /// pin count so the operator can tell whether one stale task is
    /// blocking or whether the layer is genuinely busy.
    #[test]
    fn refuses_consolidation_when_range_layer_is_pinned() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(5, &backend);
        backend.put_branch("main", head.id()).unwrap();

        // Pin one layer in the middle of the chain.
        let pinned = layers[3].id().clone();
        let mut opts = ConsolidateOpts::default();
        opts.pinned_layers.insert(pinned.clone(), 3);

        let err = consolidate_chain(
            "main",
            layers[1].id().clone(),
            head.id().clone(),
            opts,
            storage,
            &backend,
        )
        .unwrap_err();
        match err {
            ConsolidateError::RangeContainsTracePin {
                pinned_layer,
                trace_count,
            } => {
                assert_eq!(pinned_layer, pinned);
                assert_eq!(trace_count, 3);
            }
            other => panic!("expected RangeContainsTracePin, got {other:?}"),
        }
    }

    /// 17c: after consolidation, the bloom cache no longer holds
    /// entries for collapsed layers, and *does* hold an entry for
    /// the consolidated layer. Subsequent resolves get the shallow
    /// path immediately, without probing dead bloom entries.
    #[test]
    fn bloom_cache_drops_collapsed_layers_and_caches_consolidated_layer() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(5, &backend);
        backend.put_branch("main", head.id()).unwrap();

        // Pre-condition: every range layer's bloom is in the cache
        // (LayerBuilder::build inserted it at construction time).
        let range_ids: Vec<LayerId> = layers
            .iter()
            .skip(1) // layers[0] is root; consolidation range is layers[1..]
            .map(|l| l.id().clone())
            .collect();
        for id in &range_ids {
            assert!(
                storage.bloom_cache.get_or_load(id).unwrap().is_some(),
                "pre-condition: layer {id} should be in the bloom cache"
            );
        }

        let outcome = consolidate_chain(
            "main",
            layers[1].id().clone(),
            head.id().clone(),
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("consolidation succeeds");

        // Post-condition (collapsed layers): no longer in the cache.
        // The in-memory bloom cache has no backend fall-through here
        // (the chain's storage bundle was built via `in_memory()`), so
        // a `None` return means truly evicted, not just-not-loaded.
        for id in &range_ids {
            assert!(
                storage.bloom_cache.get_or_load(id).unwrap().is_none(),
                "post-condition: collapsed layer {id} should be evicted from the bloom cache"
            );
        }

        // Post-condition (consolidated layer): its fresh bloom IS in
        // the cache, populated by `LayerBuilder::build` during
        // `consolidate_chain`. Subsequent resolves through the new
        // head hit this entry on the first probe.
        assert!(
            storage
                .bloom_cache
                .get_or_load(&outcome.consolidated_layer)
                .unwrap()
                .is_some(),
            "the consolidated layer's bloom must be cached after consolidation"
        );
    }

    /// Pins on layers *outside* the consolidation range do not block
    /// the operation. Pins below `from` (older history that survives
    /// consolidation unchanged) and pins recorded with a zero count
    /// (a stale entry) should both be ignored.
    #[test]
    fn pins_outside_range_do_not_block_consolidation() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(5, &backend);
        backend.put_branch("main", head.id()).unwrap();

        let from = layers[3].id().clone();
        let outside_below = layers[1].id().clone();
        assert_ne!(outside_below, from);

        let mut opts = ConsolidateOpts::default();
        // A pin on a layer below `from` (outside the range) — must be ignored.
        opts.pinned_layers.insert(outside_below, 5);
        // A zero-count entry on a layer inside the range — must be ignored
        // (the entry exists but the pin's been drained).
        opts.pinned_layers.insert(from.clone(), 0);

        let outcome = consolidate_chain("main", from, head.id().clone(), opts, storage, &backend)
            .expect("consolidation succeeds when no pins inside the range have nonzero counts");
        assert!(outcome.head_advanced);
    }

    // ─── 17d cost estimation + dry-run ───────────────────────────────────

    /// Cost cap fires before the top-of-stack walk runs. Each chain
    /// layer in `build_chain_of` defines a single resource, so a
    /// 10-layer range carries `predicted_walk_entries = 10`. Setting
    /// the cap to a value below that should return `CostExceedsCap`
    /// with the predicted count surfaced for the operator.
    #[test]
    fn cost_cap_rejects_oversized_range() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(10, &backend);
        backend.put_branch("main", head.id()).unwrap();

        let opts = ConsolidateOpts {
            max_walk_entries: 5,
            ..ConsolidateOpts::default()
        };
        let err = consolidate_chain(
            "main",
            layers[1].id().clone(),
            head.id().clone(),
            opts,
            storage,
            &backend,
        )
        .unwrap_err();
        match err {
            ConsolidateError::CostExceedsCap { predicted_entries } => {
                // 10 single-resource layers in the range → predicted = 10.
                assert_eq!(
                    predicted_entries, 10,
                    "predicted count must equal sum of handle.resource_count over the range"
                );
            }
            other => panic!("expected CostExceedsCap, got {other:?}"),
        }
    }

    /// `estimate_consolidation` returns the predicted `LayerId` and
    /// cost without persisting or advancing the branch. Subsequent
    /// real consolidation produces the *same* `LayerId` — that's the
    /// content-addressed identity guarantee, surfaced through the
    /// dry-run flow.
    #[test]
    fn estimate_predicts_actual_consolidated_layer_id() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(8, &backend);
        backend.put_branch("main", head.id()).unwrap();
        let head_before_estimate = backend.get_branch("main").unwrap();

        let from = layers[2].id().clone();
        let estimate = estimate_consolidation(
            "main",
            from.clone(),
            head.id().clone(),
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("estimate succeeds");

        // No persistence: the branch head is unchanged and the
        // predicted layer hasn't been committed as a topology entry.
        assert_eq!(
            backend.get_branch("main").unwrap(),
            head_before_estimate,
            "estimate must not advance the branch ref"
        );
        assert!(
            backend
                .load_chain_from(&estimate.predicted_consolidated_layer)
                .unwrap()
                .is_none(),
            "estimate must not persist the predicted layer to the backend"
        );

        // Counts: predicted is the upper-bound sum; actual is the
        // dedup'd top-of-stack. Each chain layer defines exactly one
        // distinct IRI, so the two are equal here.
        assert_eq!(estimate.collapsed_layer_count, 7);
        assert_eq!(estimate.predicted_walk_entries, 7);
        assert_eq!(estimate.actual_walk_entries, 7);

        // Real consolidation against the same range produces the same id.
        let outcome = consolidate_chain(
            "main",
            from,
            head.id().clone(),
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .expect("real consolidation succeeds");
        assert_eq!(
            outcome.consolidated_layer, estimate.predicted_consolidated_layer,
            "the estimate's predicted LayerId must equal what consolidate_chain produces"
        );
    }

    /// Estimate surfaces the same typed validation errors as
    /// `consolidate_chain` — a bad range is rejected at the estimate
    /// stage, no need to wait for the real operation.
    #[test]
    fn estimate_surfaces_validation_errors() {
        let backend = MemoryPersistentBackend::new();
        let (head, _layers, storage) = build_chain_of(5, &backend);
        backend.put_branch("main", head.id()).unwrap();

        let bogus_from = LayerId([0xff; 32]);
        let err = estimate_consolidation(
            "main",
            bogus_from.clone(),
            head.id().clone(),
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .unwrap_err();
        match err {
            ConsolidateError::RangeNotAncestral { from, .. } => assert_eq!(from, bogus_from),
            other => panic!("expected RangeNotAncestral from estimate, got {other:?}"),
        }
    }

    /// Dedup savings show up as `actual_walk_entries <
    /// predicted_walk_entries` when the range contains layers that
    /// redefine the same IRI. The upper-bound prediction is the sum
    /// over `resource_count`; the actual walk counts distinct IRIs
    /// (D25 §12.5). Same-IRI redefinitions are the canonical
    /// notebook-cell-edit pattern.
    #[test]
    fn estimate_reports_dedup_savings_for_rewrite_ranges() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();

        let mut rb = LayerBuilder::new("root", None);
        rb.add_resource(make_resource("urn:eigenius:core:Class", vec![]))
            .unwrap();
        rb.add_resource(make_resource("urn:eigenius:core:description", vec![]))
            .unwrap();
        let root = Arc::new(rb.build(storage.clone()));
        backend.store_layer(&root).unwrap();

        // Three layers, each redefining the *same* demo:X resource.
        // Predicted walk = 3 (one per handle); actual = 1 (one distinct
        // IRI after dedup).
        let mut current = Arc::clone(&root);
        let mut layers = Vec::new();
        for i in 0..3 {
            let mut b = LayerBuilder::new(&format!("L{i}"), Some(Arc::clone(&current)));
            b.add_resource(make_resource(
                "urn:eigenius:demo:X",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String(format!("v{i}")),
                )],
            ))
            .unwrap();
            let layer = Arc::new(b.build(storage.clone()));
            backend.store_layer(&layer).unwrap();
            layers.push(Arc::clone(&layer));
            current = layer;
        }
        let head = Arc::clone(&current);
        backend.put_branch("main", head.id()).unwrap();

        let estimate = estimate_consolidation(
            "main",
            layers[0].id().clone(),
            head.id().clone(),
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .expect("estimate succeeds");

        assert_eq!(estimate.predicted_walk_entries, 3);
        assert_eq!(
            estimate.actual_walk_entries, 1,
            "three layers redefining the same IRI dedup to one distinct entry"
        );
    }
}
