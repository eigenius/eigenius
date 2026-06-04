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
//! **Milestone status (D25 §11.1 / §12.8):**
//! - 17a — top-of-stack algorithm + branch CAS for `to = head`. ✅
//! - 17b — range validation: ancestral / merge-free / pin-free. ✅
//! - 17c — bloom-cache eviction for collapsed layers. ✅
//! - 17d — cost estimation gate + `estimate_consolidation` dry-run. ✅
//! - 17e — CLI (`db consolidate`) + gRPC
//!   (`ConsolidateChain` / `EstimateConsolidation`) surfaces. ✅
//! - 17f-A — redirect storage primitive (D25 §12.8). ✅
//! - 17f-B — resolve walk follows installed redirects. ✅
//! - 17f-C — below-head consolidation installs redirects;
//!   chain-cross refusal; `preserve_history` option. ✅
//! - 17f-D — GC reachability through redirects. ✅
//! - 17f-E — RPC + CLI surfaces for below-head consolidation
//!   (`preserve_history` flag, typed `ToNotReachableFromHead` +
//!   `RangeCrossesExistingRedirect` error variants). ✅
//! - 17f-F — cross-cutting end-to-end tests
//!   (resolve-equivalence with redirect installed; preserve vs.
//!   reclaim through real GC; redirect-aware `load_chain_from`). ✅
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
    /// Preserve the pre-consolidation history of the source range
    /// (D25 §12.8.1(b)). Default `false`: GC reclaims the consolidated
    /// layers on its next pass (matches the at-head behavior). `true`:
    /// the redirect is installed with `preserve_history = true`, and
    /// GC's mark phase will keep the source-side chain alive so
    /// time-travel reads against intermediate layers continue to
    /// resolve. Only meaningful for below-head consolidations — at-head
    /// consolidations advance the branch and have no redirect to
    /// preserve through.
    pub preserve_history: bool,
}

impl Default for ConsolidateOpts {
    fn default() -> Self {
        Self {
            max_walk_entries: 5_000_000,
            trace_pin_policy: TracePinPolicy::Refuse,
            pinned_layers: BTreeMap::new(),
            preserve_history: false,
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
    /// `true` if the branch's head moved as part of the operation
    /// (at-head consolidation). `false` for below-head consolidations
    /// that installed a resolve redirect instead of advancing the
    /// branch (D25 §12.8 / Phase 17f).
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
    /// `to` exists in storage but isn't an ancestor of the branch's
    /// current head — head-rooted resolves wouldn't pass through the
    /// would-be redirect, so installing one is pointless. Surfaced in
    /// 17f's below-head consolidation path. (At-head consolidations
    /// surface `BranchAdvancedConcurrently` instead, since the head
    /// genuinely is wrong.)
    ToNotReachableFromHead { to: LayerId, observed_head: LayerId },
    /// The consolidation range touches an existing resolve redirect —
    /// either `to` is already a redirect source, or some layer in the
    /// range is. v1 refuses to compose redirects (D25 §12.8.1(a)); the
    /// future `RedirectChainPolicy::Replace` (issue #49) will lift this.
    RangeCrossesExistingRedirect { offending_layer: LayerId },
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
            ConsolidateError::ToNotReachableFromHead { to, observed_head } => write!(
                f,
                "to {to} is not reachable from branch head {observed_head}"
            ),
            ConsolidateError::RangeCrossesExistingRedirect { offending_layer } => write!(
                f,
                "consolidation range crosses existing redirect at {offending_layer}; v1 refuses (see issue #49)"
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
    // Capture an Arc to the bloom cache and the in-memory redirect
    // map before `storage` is consumed by the prep helper. Both are
    // used in the post-commit tail (bloom eviction + in-memory
    // redirect-map update for below-head consolidations).
    let bloom_cache = Arc::clone(&storage.bloom_cache);
    let redirect_map = Arc::clone(&storage.redirect_map);

    let prep = prepare_consolidation(branch, from, to.clone(), &opts, storage, backend)?;
    let Prepared {
        consolidated_layer,
        range_layers,
        collapsed_layer_count,
        to_handle,
        is_below_head,
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

    // D43 §2.8 / M8.2 — vector consolidation. Re-embedding is not
    // required (vectors are model-deterministic); concat surviving
    // subjects' vectors from the collapsed range and rebuild HNSW
    // when the active strategy demands it. Text consolidation
    // already ran inside `LayerBuilder::build` via the existing
    // `populate_text_indexes` path; vector indexing isn't part of
    // that path (it needs an Embedder for normal sweeps, which
    // consolidation doesn't), so it runs here against the
    // pre-existing collapsed-range segments.
    if let Err(e) = crate::query::vector::indexing::consolidate_layer_vectors(
        &consolidated_layer,
        &range_layers,
    ) {
        return Err(ConsolidateError::Internal(format!(
            "vector consolidation failed: {e}"
        )));
    }

    if is_below_head {
        // Below-head consolidation (D25 §12.8). Install a resolve
        // redirect; do *not* touch the branch ref. Head-rooted walks
        // pick up the redirect at `to` and short-circuit through
        // `L_c`'s ancestor closure. The in-memory redirect map is
        // updated alongside the persistent CF so the next
        // `build_chain` against this `LayerStorage` populates the
        // inline `Layer::redirect_target` (Phase B).
        let entry = crate::layer::RedirectEntry {
            target: consolidated_layer.id().clone(),
            source_handle: to_handle,
            preserve_history: opts.preserve_history,
        };
        backend
            .put_redirect(&entry)
            .map_err(ConsolidateError::WriteFailed)?;
        redirect_map.put(to.clone(), consolidated_layer.id().clone());
    } else {
        // At-head consolidation (the existing 17a path). Advance the
        // branch ref to the consolidated layer. Inside the branch
        // lock so a concurrent `update_branch` can't interleave
        // between the head check above and the put here.
        backend
            .put_branch(branch, consolidated_layer.id())
            .map_err(ConsolidateError::WriteFailed)?;
    }

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

    // Anchored-commit cache invalidation (D33 §3 — supporting-layer
    // property enables this). Cache entries that point at a layer in
    // the consolidated range are now misleading: head-rooted resolves
    // no longer reach those layers, so reporting "cached at L" for
    // L ∈ range surfaces an orphan id to the caller (cells get told
    // their work lives at a position that no branch reaches). After
    // re-running, the next commit re-populates the cache with the
    // new canonical position.
    //
    // O(|cache|) scan — there is no reverse index by `Layer Id` today.
    // Consolidation isn't a hot path, so the cost is acceptable; a
    // reverse index is the natural follow-up if profiling flags it.
    let range_id_set: BTreeSet<LayerId> = range_layers.iter().map(|l| l.id().clone()).collect();
    match backend.list_anchored_commits() {
        Ok(entries) => {
            for entry in entries {
                if !range_id_set.contains(&entry.layer_id) {
                    continue;
                }
                if let Err(e) = backend
                    .delete_anchored_commit(&entry.content_hash, &entry.supporting_content_hash)
                {
                    // Best-effort: the commit succeeded; leaving a
                    // stale cache entry behind is a UX issue, not a
                    // correctness one (the probe will surface a stale
                    // orphan id until GC reclaims it). Logging it is
                    // enough.
                    tracing::warn!(
                        layer_id = %entry.layer_id,
                        error = %e,
                        "failed to evict anchored-commit cache entry for consolidated layer"
                    );
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "failed to list anchored-commit cache for invalidation"
            );
        }
    }

    Ok(ConsolidationOutcome {
        consolidated_layer: consolidated_layer.id().clone(),
        collapsed_layer_count,
        reclaimable_bytes_estimate: 0,
        // At-head: the branch ref moved to `L_c`. Below-head: the
        // branch ref stays at the same head; the redirect routes
        // resolves through `L_c` without changing the chain tip.
        head_advanced: !is_below_head,
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
    /// Snapshot of `to`'s `LayerHandle` at validation time. Used by
    /// `consolidate_chain_locked` to populate the `RedirectEntry`
    /// when the consolidation is below-head (D25 §12.8).
    to_handle: crate::layer::LayerHandle,
    /// `true` iff `to` is strictly below the branch's current head.
    /// Drives the choice between advancing the branch ref (false) and
    /// installing a resolve redirect (true).
    is_below_head: bool,
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
    // Resolve the branch head + determine whether this is an at-head
    // or below-head consolidation. v1 supports both:
    //
    // - **At-head** (`to == head`): collapses the range and advances
    //   the branch ref to `L_c`. The chain shortens by `len(range)`
    //   layers; the old range becomes unreachable from head-rooted
    //   walks and is eligible for GC.
    //
    // - **Below-head** (D25 §12.8 / Phase 17f): installs a resolve
    //   redirect on `to` pointing at `L_c`. The branch ref is
    //   unchanged; head-rooted walks short-circuit through the
    //   redirect at `to` and pick up `L_c`'s ancestor closure. Layers
    //   above `to` keep their existing parent pointers and `LayerId`s
    //   — no cascade re-id.
    let observed_head_opt = backend
        .get_branch(branch)
        .map_err(ConsolidateError::WriteFailed)?;
    let observed_head =
        observed_head_opt.ok_or_else(|| ConsolidateError::BranchAdvancedConcurrently {
            observed_head: None,
            expected_head: to.clone(),
        })?;

    let is_below_head = observed_head != to;

    // For below-head consolidation, verify `to` is actually on
    // `head`'s parent chain. A stale `to` (one that belongs to a
    // different branch or to a sibling head) would install a
    // redirect that no head-rooted walk would ever reach.
    if is_below_head {
        let head_chain = backend
            .load_chain_from(&observed_head)
            .map_err(ConsolidateError::WriteFailed)?
            .ok_or_else(|| {
                ConsolidateError::Internal(format!("chain absent for head {observed_head}"))
            })?;
        let reachable = head_chain.handles.iter().any(|h| h.id == to);
        if !reachable {
            return Err(ConsolidateError::ToNotReachableFromHead { to, observed_head });
        }
    }

    // Load existing redirects for the chaining-refusal check
    // (D25 §12.8.1(a)). Refuse any consolidation whose range touches
    // a layer that's already a redirect source — compose semantics
    // are an opt-in v2 addition tracked in issue #49.
    let existing_redirect_sources: BTreeSet<LayerId> = backend
        .list_redirects()
        .map_err(ConsolidateError::WriteFailed)?
        .iter()
        .map(|e| e.source().clone())
        .collect();

    // Load the chain from `to`. The handles retain authoritative
    // multi-parent topology (`build_chain` would collapse it to
    // `parents.first()` for single-parent Layer reconstruction), so
    // we validate against handles before the chain is rebuilt.
    let info = backend
        .load_chain_from(&to)
        .map_err(ConsolidateError::WriteFailed)?
        .ok_or_else(|| ConsolidateError::Internal(format!("chain absent for to {to}")))?;

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
    let mut to_handle: Option<crate::layer::LayerHandle> = None;
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
        if existing_redirect_sources.contains(&handle.id) {
            return Err(ConsolidateError::RangeCrossesExistingRedirect {
                offending_layer: handle.id.clone(),
            });
        }
        predicted_walk_entries = predicted_walk_entries.saturating_add(handle.resource_count);
        if handle.id == to {
            to_handle = Some(handle.clone());
        }
        range_ids.push(handle.id.clone());
        if handle.id == from {
            found_from = true;
            break;
        }
    }
    if !found_from {
        return Err(ConsolidateError::RangeNotAncestral { from, to });
    }
    let to_handle = to_handle.ok_or_else(|| {
        ConsolidateError::Internal(
            "to_handle not captured during validation walk (storage inconsistency?)".to_string(),
        )
    })?;

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
    // above) and record the first-seen modification for each IRI.
    //
    // A "modification" is either a definition (`defined_iris`) or a
    // tombstone (`tombstoned_iris` — D20 §6.2 / §6.3, 15g step 3).
    // Tombstones must propagate into the consolidated layer so the
    // post-consolidation resolve walk continues to hide the same IRIs
    // from the consolidated layer's parent chain. Without this,
    // collapsing a range that contains a tombstone would resurface
    // the parent's body — a resolve-equivalence violation.
    //
    // The "first seen wins" rule applies uniformly: if a higher layer
    // in the range redefines an IRI that a lower layer tombstones,
    // the redefinition wins (it shadowed the tombstone in the original
    // chain). Iterating `defined_iris` before `tombstoned_iris` at
    // each layer is structurally safe because `LayerBuilder` enforces
    // the two sets are disjoint per-layer; the order matters only
    // across layers (handled by the head→root walk).
    let mut seen_iris: BTreeSet<Iri> = BTreeSet::new();
    let mut consolidated_resources: Vec<Resource> = Vec::new();
    let mut consolidated_tombstones: BTreeSet<Iri> = BTreeSet::new();
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
        for tomb in layer.tombstoned_iris() {
            if !seen_iris.insert(tomb.clone()) {
                continue;
            }
            consolidated_tombstones.insert(tomb.clone());
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
    for tomb in consolidated_tombstones {
        builder.tombstone(tomb).map_err(|e| {
            ConsolidateError::Internal(format!("consolidated layer rejected tombstone: {e}"))
        })?;
    }
    let consolidated_layer = Arc::new(builder.build(storage));

    Ok(Prepared {
        consolidated_layer,
        range_layers,
        collapsed_layer_count,
        predicted_walk_entries,
        actual_walk_entries,
        to_handle,
        is_below_head,
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
    /// Regression: a range that contains a tombstone must continue
    /// to hide the parent's body after consolidation. Without
    /// propagating tombstones into the consolidated layer, the
    /// tombstone is lost and the parent's body resurfaces — a direct
    /// resolve-equivalence violation.
    ///
    /// Scenario:
    /// - root defines `demo:X` with body `v_root`.
    /// - L1 (in range) tombstones `demo:X`.
    /// - L2 (in range, head) defines an unrelated `demo:Y`.
    /// - Consolidate L1..L2.
    /// - Before: head.resolve(X) = None (tombstone at L1 hides root).
    /// - After: head.resolve(X) must still be None.
    #[test]
    fn consolidation_propagates_tombstones() {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();

        // Root defines demo:X.
        let mut root_b = LayerBuilder::new("root", None);
        root_b
            .add_resource(make_resource(
                "urn:eigenius:demo:X",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("v_root".into()),
                )],
            ))
            .unwrap();
        let root = Arc::new(root_b.build(storage.clone()));
        backend.store_layer(&root).unwrap();

        // L1 tombstones demo:X.
        let mut l1_b = LayerBuilder::new("L1", Some(Arc::clone(&root)));
        l1_b.tombstone(iri("urn:eigenius:demo:X")).unwrap();
        let l1 = Arc::new(l1_b.build(storage.clone()));
        backend.store_layer(&l1).unwrap();

        // L2 defines an unrelated IRI to give the range some real content.
        let mut l2_b = LayerBuilder::new("L2", Some(Arc::clone(&l1)));
        l2_b.add_resource(make_resource(
            "urn:eigenius:demo:Y",
            vec![("urn:eigenius:core:description", Value::String("v_y".into()))],
        ))
        .unwrap();
        let l2 = Arc::new(l2_b.build(storage.clone()));
        backend.store_layer(&l2).unwrap();

        backend.put_branch("main", l2.id()).unwrap();

        // Pre-condition: head sees X as removed.
        assert!(
            l2.resolve(&iri("urn:eigenius:demo:X")).is_none(),
            "tombstone at L1 must hide root's body from L2"
        );

        // Consolidate L1..L2.
        let outcome = consolidate_chain(
            "main",
            l1.id().clone(),
            l2.id().clone(),
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("consolidation succeeds");

        // Rebuild the new chain through the branch head and verify
        // resolve-equivalence: demo:X must still be hidden, demo:Y
        // must still be visible.
        let new_head = backend.get_branch("main").unwrap().unwrap();
        assert_eq!(new_head, outcome.consolidated_layer);
        let info = backend.load_chain_from(&new_head).unwrap().unwrap();
        let new_head_layer = crate::layer::build_chain(info, storage);

        assert!(
            new_head_layer
                .resolve(&iri("urn:eigenius:demo:X"))
                .is_none(),
            "consolidated layer must continue to hide demo:X via its propagated tombstone"
        );
        assert!(
            new_head_layer
                .resolve(&iri("urn:eigenius:demo:Y"))
                .is_some(),
            "consolidated layer must preserve demo:Y from the range"
        );
    }

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

    /// Consolidation invalidates anchored-commit cache entries pointing
    /// at layers inside the range (D33 §3). Cache entries pointing at
    /// layers *outside* the range must survive untouched — otherwise
    /// a single consolidation evicts the world.
    ///
    /// Regression: previously the cache was untouched on consolidation,
    /// so re-running a cell whose content had been canonically
    /// committed inside the range would return the now-orphan id and
    /// the branch wouldn't advance. See D34 Phase 6 issue.
    #[test]
    fn at_head_consolidation_evicts_anchored_commit_cache_in_range() {
        use crate::layer::ContentHash;
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(9, &backend);
        backend.put_branch("main", head.id()).unwrap();

        // Pick a victim *inside* the range we'll consolidate and a
        // bystander *outside* it. `layers[0]` is the root (outside the
        // range), `layers[5]` is L4 (inside, deep enough to be in the
        // middle), and the range will be `from = layers[3]` (L2) →
        // `to = head`.
        let victim_in_range = layers[5].id().clone();
        let bystander_outside = layers[0].id().clone(); // root
        let from = layers[3].id().clone();
        let to = head.id().clone();

        // Synthetic cache keys — we don't need them to match any real
        // content/supporting hash, just to be distinct so the
        // invalidation logic can find them by `layer_id`.
        let key_victim = (ContentHash([1u8; 32]), ContentHash([2u8; 32]));
        let key_bystander = (ContentHash([3u8; 32]), ContentHash([4u8; 32]));

        backend
            .put_anchored_commit(&key_victim.0, &key_victim.1, &victim_in_range)
            .unwrap();
        backend
            .put_anchored_commit(&key_bystander.0, &key_bystander.1, &bystander_outside)
            .unwrap();

        // Sanity: both entries are live before consolidation.
        assert_eq!(
            backend
                .lookup_anchored_commit(&key_victim.0, &key_victim.1)
                .unwrap(),
            Some(victim_in_range.clone())
        );
        assert_eq!(
            backend
                .lookup_anchored_commit(&key_bystander.0, &key_bystander.1)
                .unwrap(),
            Some(bystander_outside.clone())
        );

        consolidate_chain(
            "main",
            from,
            to,
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .expect("consolidation succeeds");

        // The victim's cache entry is gone — re-running the same
        // content now misses the cache and lands a fresh commit on the
        // new tip.
        assert_eq!(
            backend
                .lookup_anchored_commit(&key_victim.0, &key_victim.1)
                .unwrap(),
            None,
            "cache entry pointing at a consolidated layer must be evicted"
        );
        // The bystander's entry survives — it points at a layer
        // outside the consolidated range, which is still reachable.
        assert_eq!(
            backend
                .lookup_anchored_commit(&key_bystander.0, &key_bystander.1)
                .unwrap(),
            Some(bystander_outside),
            "cache entry pointing at a layer outside the range must survive"
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

    /// 17f-C: consolidating against a `to` below the branch head
    /// installs a resolve redirect and leaves the branch ref
    /// unchanged. Replaces the pre-17f version of this test, which
    /// expected `BranchAdvancedConcurrently` — that's now reserved
    /// for the "branch doesn't exist" case, and below-head ranges
    /// are a first-class flow.
    #[test]
    fn below_head_consolidation_installs_redirect_and_leaves_branch_unchanged() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(5, &backend);
        backend.put_branch("main", head.id()).unwrap();

        // Aim at L2 (an interior layer), not the head L4.
        let to_interior = layers[3].id().clone();
        let from = layers[1].id().clone();
        let outcome = consolidate_chain(
            "main",
            from,
            to_interior.clone(),
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .expect("below-head consolidation succeeds");
        // Below-head: branch ref does NOT move.
        assert!(!outcome.head_advanced);
        assert_eq!(
            backend.get_branch("main").unwrap().as_ref(),
            Some(head.id()),
            "branch ref must stay at the original head after below-head consolidation"
        );
        // A redirect was installed pointing the interior `to` at the
        // freshly-built consolidated layer.
        let entry = backend
            .lookup_redirect(&to_interior)
            .unwrap()
            .expect("redirect installed at the interior `to`");
        assert_eq!(entry.target, outcome.consolidated_layer);
        assert!(!entry.preserve_history); // default
    }

    /// 17f-C: a second consolidation whose range crosses an
    /// existing redirect source is refused with
    /// `RangeCrossesExistingRedirect`. v1 doesn't support compose
    /// (issue #49); the refusal protects the redirect's invariants.
    #[test]
    fn refuses_consolidation_that_crosses_existing_redirect() {
        let backend = MemoryPersistentBackend::new();
        let (head, layers, storage) = build_chain_of(6, &backend);
        backend.put_branch("main", head.id()).unwrap();

        // First consolidation: collapse layers[1..=3] into a redirect
        // on layers[3]. Branch stays put.
        let first = consolidate_chain(
            "main",
            layers[1].id().clone(),
            layers[3].id().clone(),
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("first below-head consolidation succeeds");
        assert!(!first.head_advanced);

        // Second consolidation: range [layers[2]..head] would cross
        // the existing redirect installed on layers[3]. Refuse.
        let err = consolidate_chain(
            "main",
            layers[2].id().clone(),
            head.id().clone(),
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .unwrap_err();
        match err {
            ConsolidateError::RangeCrossesExistingRedirect { offending_layer } => {
                assert_eq!(&offending_layer, layers[3].id());
            }
            other => panic!("expected RangeCrossesExistingRedirect, got {other:?}"),
        }
    }

    /// 17f-C: passing a `to` that doesn't appear in the branch's
    /// chain returns `ToNotReachableFromHead` — the redirect would
    /// be unreachable, so the operation is rejected.
    #[test]
    fn refuses_below_head_when_to_is_not_on_the_branch_chain() {
        let backend = MemoryPersistentBackend::new();
        let (head, _layers, storage) = build_chain_of(5, &backend);
        backend.put_branch("main", head.id()).unwrap();

        // Bogus `to` that's not in the branch's chain at all.
        let stray_to = LayerId([0xaa; 32]);
        let err = consolidate_chain(
            "main",
            stray_to.clone(),
            stray_to.clone(),
            ConsolidateOpts::default(),
            storage,
            &backend,
        )
        .unwrap_err();
        match err {
            ConsolidateError::ToNotReachableFromHead { to, observed_head } => {
                assert_eq!(to, stray_to);
                assert_eq!(&observed_head, head.id());
            }
            ConsolidateError::RangeNotAncestral { .. } => {
                // Also acceptable: if the chain load happens before
                // the reachability check fires, we surface this
                // instead. Either way the operator gets a clear
                // typed error.
            }
            other => panic!("expected ToNotReachableFromHead, got {other:?}"),
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

    // ─── 17f-F cross-cutting end-to-end ────────────────────────────────────

    /// Build a chain of `n` layers on top of `root`, exactly like
    /// `build_chain_of`, but on a shared `Arc<dyn PersistentBackend>`
    /// so callers can pass the same Arc to both `consolidate_chain`
    /// (which takes `&dyn`) and `LayerStorage::with_persistent`
    /// (which takes `Arc<dyn>`). Returns the head layer plus the
    /// chain of intermediate layers (oldest first).
    fn build_chain_of_arc(
        n: usize,
        backend: Arc<dyn PersistentBackend>,
    ) -> (Arc<Layer>, Vec<Arc<Layer>>) {
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));
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
        (current, all)
    }

    /// Build a fresh `LayerStorage` against the backend, load the
    /// chain from `head`, and snapshot every IRI's value. The fresh
    /// storage forces `redirect_map` to be re-read from the backend's
    /// redirect CF — captures any redirects installed since the last
    /// snapshot.
    fn snapshot_via_fresh_storage(
        backend: Arc<dyn PersistentBackend>,
        head: &LayerId,
    ) -> Vec<(Iri, String)> {
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));
        let info = backend.load_chain_from(head).unwrap().unwrap();
        let head_layer = crate::layer::build_chain(info, storage);
        snapshot_chain(&head_layer)
    }

    /// Load-bearing 17f-F regression: a below-head consolidation must
    /// preserve head-rooted resolves for every IRI. Build a chain,
    /// snapshot before consolidate, consolidate an interior range
    /// below the head, snapshot after, assert equality.
    #[test]
    fn below_head_consolidate_preserves_head_rooted_resolves() {
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let (head, layers) = build_chain_of_arc(6, Arc::clone(&backend));
        backend.put_branch("main", head.id()).unwrap();

        // Snapshot before. layers[0] is root; layers[1..=6] are L0..L5;
        // head == layers[6].
        let before = snapshot_via_fresh_storage(Arc::clone(&backend), head.id());
        assert!(
            before.iter().any(|(_, v)| v == "v0"),
            "pre-snapshot must contain values from L0..L5"
        );

        // Consolidate [L1..L4] — strictly below the head L5.
        let from = layers[2].id().clone(); // L1
        let to = layers[5].id().clone(); // L4
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));
        let outcome = consolidate_chain(
            "main",
            from,
            to,
            ConsolidateOpts::default(),
            storage,
            backend.as_ref(),
        )
        .expect("below-head consolidation succeeds");
        assert!(
            !outcome.head_advanced,
            "below-head must not advance the branch"
        );
        assert_eq!(outcome.collapsed_layer_count, 4);

        // Snapshot after. Must be byte-equal to before.
        let after = snapshot_via_fresh_storage(Arc::clone(&backend), head.id());
        assert_eq!(
            before, after,
            "below-head consolidation must preserve head-rooted resolves for every IRI"
        );
    }

    /// Preserve-history end-to-end: consolidate below-head with
    /// `preserve_history = true`, then run GC. The source-side
    /// intermediate layer (interior of the consolidated range) stays
    /// alive — time-travel reads against it still resolve.
    #[test]
    fn preserve_history_keeps_source_interior_alive_through_gc() {
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let (head, layers) = build_chain_of_arc(5, Arc::clone(&backend));
        backend.put_branch("main", head.id()).unwrap();

        // Consolidate [L1..L3] below head with preserve_history=true.
        let interior_iri = crate::ontology::iri::Iri::parse("urn:eigenius:demo:layer_1").unwrap();
        let interior_layer_id = layers[2].id().clone(); // L1, the interior
        let from = layers[2].id().clone();
        let to = layers[4].id().clone();
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));
        let opts = ConsolidateOpts {
            preserve_history: true,
            ..ConsolidateOpts::default()
        };
        consolidate_chain("main", from, to, opts, storage, backend.as_ref())
            .expect("consolidation succeeds");

        // Run GC. Preserve mode keeps the source-side chain alive.
        let gc_storage = LayerStorage::with_persistent(Arc::clone(&backend));
        let stats = crate::gc::collect(
            crate::gc::GcRoots::from_branches(backend.as_ref()).unwrap(),
            &crate::gc::GcConfig {
                min_age: std::time::Duration::from_secs(0),
            },
            gc_storage.cache.as_ref(),
            gc_storage.bloom_cache.as_ref(),
            backend.as_ref(),
        )
        .expect("gc collect");
        assert_eq!(
            stats.layers_swept, 0,
            "preserve_history mode must not sweep any layers in this scenario"
        );

        // The interior layer's topology entry survives.
        assert!(
            backend
                .load_topology()
                .unwrap()
                .get_layer(&interior_layer_id)
                .is_some(),
            "interior layer must survive GC in preserve_history mode"
        );

        // Time-travel read against the interior still works.
        let info = backend
            .load_chain_from(&interior_layer_id)
            .unwrap()
            .unwrap();
        let storage_for_walk = LayerStorage::with_persistent(Arc::clone(&backend));
        let interior_head = crate::layer::build_chain(info, storage_for_walk);
        let resource = interior_head
            .resolve(&interior_iri)
            .expect("interior IRI resolves");
        assert_eq!(
            resource
                .get(&crate::ontology::iri::Iri::parse("urn:eigenius:core:description").unwrap())
                .and_then(|v| v.as_str()),
            Some("v1")
        );
    }

    /// Reclaim end-to-end: consolidate below-head with default
    /// `preserve_history = false`, then run GC. The interior of the
    /// consolidated range is swept; the redirect source itself stays
    /// as a tombstone-on-disk (shape 1 per D25 §12.8.1(d)), and
    /// head-rooted resolves still produce correct values through the
    /// redirect.
    #[test]
    fn reclaim_mode_sweeps_interior_but_preserves_head_resolves_through_gc() {
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let (head, layers) = build_chain_of_arc(5, Arc::clone(&backend));
        backend.put_branch("main", head.id()).unwrap();

        let interior_layer_id = layers[2].id().clone(); // L1 — will be reclaimed
        let to_layer_id = layers[4].id().clone(); // L3 — will become a tombstone

        // Pre-snapshot for the resolve-equivalence assertion.
        let before = snapshot_via_fresh_storage(Arc::clone(&backend), head.id());

        // Consolidate [L1..L3] below head with reclaim semantics.
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));
        consolidate_chain(
            "main",
            layers[2].id().clone(),
            layers[4].id().clone(),
            ConsolidateOpts::default(),
            storage,
            backend.as_ref(),
        )
        .expect("consolidation succeeds");

        // GC pass — reclaim mode should sweep the interior.
        let gc_storage = LayerStorage::with_persistent(Arc::clone(&backend));
        let stats = crate::gc::collect(
            crate::gc::GcRoots::from_branches(backend.as_ref()).unwrap(),
            &crate::gc::GcConfig {
                min_age: std::time::Duration::from_secs(0),
            },
            gc_storage.cache.as_ref(),
            gc_storage.bloom_cache.as_ref(),
            backend.as_ref(),
        )
        .expect("gc collect");
        assert!(
            stats.layers_swept >= 1,
            "reclaim mode should sweep at least one interior layer"
        );

        // The interior is gone from the topology.
        let topo = backend.load_topology().unwrap();
        assert!(
            topo.get_layer(&interior_layer_id).is_none(),
            "interior of consolidated range must be swept in reclaim mode"
        );
        // The redirect source (the `to` layer) survives as a
        // tombstone-on-disk: its on-disk topology entry remains so
        // layers above it can chain-walk through it; the redirect
        // routes resolves to L_c (D25 §12.8.1(d) shape 1).
        assert!(
            topo.get_layer(&to_layer_id).is_some(),
            "redirect source must stay alive after reclaim GC (shape 1 tombstone)"
        );

        // Head-rooted resolves still produce identical values for
        // every IRI — the redirect makes the interior content
        // available via L_c.
        let after = snapshot_via_fresh_storage(Arc::clone(&backend), head.id());
        assert_eq!(
            before, after,
            "head-rooted resolves must survive reclaim GC unchanged via the redirect"
        );
    }

    // ─── D43 §2.8 / M8.1 text-index consolidation ───────────────

    /// D43 §2.8 / M8.1 — search-equivalence under head substitution
    /// for `TEXT_MATCH`. After consolidating a range of layers that
    /// each contributed text-indexed Resources, queries at the post-
    /// consolidation head must return the same hit set as before
    /// consolidation. This is the consolidation atomicity invariant
    /// extended from triple data to the text index.
    ///
    /// Mechanism: `LayerBuilder::build` runs `populate_text_indexes`
    /// against the consolidated layer's defined Resources, writing
    /// fresh `text_term:<I>:<T>:<C>` postings with new local doc-ids.
    /// The collapsed layers' postings become unreachable after the
    /// branch advance (head-walks no longer traverse them). The
    /// hit set is the same modulo internal doc-id relabeling.
    #[test]
    fn text_search_equivalence_under_at_head_consolidation() {
        use crate::bootstrap::bootstrap;
        use crate::ontology::well_known as wk;
        use crate::query::text::analyzer::EnStemV1;
        use crate::query::text::search::run_text_search;

        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let target_prop = "urn:eigenius:test:body";

        // Bootstrap the core ontology so `is_a` resolves at the
        // discovery-layer index scan time. The bootstrap head is
        // already persisted into a backing storage; for the
        // consolidation flow we need the same backend persistence,
        // so persist the bootstrap layers into `backend` too.
        let bootstrap_ctx = bootstrap().expect("bootstrap");
        let bootstrap_head = Arc::clone(bootstrap_ctx.head());
        // Walk the bootstrap chain head→root and persist every
        // layer into our backend so `load_chain_from` can rebuild
        // it after consolidation.
        {
            let mut cursor: Option<Arc<Layer>> = Some(Arc::clone(&bootstrap_head));
            while let Some(layer) = cursor {
                backend
                    .store_layer(&layer)
                    .expect("persist bootstrap layer");
                cursor = layer.parent().cloned();
            }
        }

        // Declare a TextIndex on `body`.
        let mut ti_layer = LayerBuilder::new("ti", Some(Arc::clone(&bootstrap_head)));
        let mut ti = Resource::new(iri("urn:eigenius:test:ti"));
        ti.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::TEXT_INDEX_CLASS))]),
        );
        ti.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        ti.set(iri(wk::TEXT_ANALYZER), Value::String("en-stem-v1".into()));
        ti_layer.add_resource(ti).unwrap();
        let ti_layer = Arc::new(ti_layer.build(storage.clone()));
        backend.store_layer(&ti_layer).unwrap();

        // Four content layers, each contributing one Resource whose
        // body contains overlapping vocabulary so a TEXT_MATCH over
        // ("wal", "truncation") hits multiple subjects across the
        // range. Mix in one no-match doc to confirm filtering.
        let bodies = [
            ("doc1", "WAL truncation under concurrent commit"),
            ("doc2", "wal segment rotation"),
            ("doc3", "rollback handling unrelated to WAL"),
            ("doc4", "truncation of orphan log files"),
        ];
        let mut content_layers: Vec<Arc<Layer>> = Vec::new();
        let mut prev: Arc<Layer> = Arc::clone(&ti_layer);
        for (sid, body) in bodies {
            let mut lb = LayerBuilder::new(&format!("L_{sid}"), Some(Arc::clone(&prev)));
            let mut r = Resource::new(iri(&format!("urn:eigenius:test:{sid}")));
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
            );
            r.set(iri(target_prop), Value::String(body.to_string()));
            lb.add_resource(r).unwrap();
            let layer = Arc::new(lb.build(storage.clone()));
            backend.store_layer(&layer).unwrap();
            content_layers.push(Arc::clone(&layer));
            prev = layer;
        }
        let head_before = Arc::clone(&prev);
        backend.put_branch("main", head_before.id()).unwrap();

        // Capture the text-search hits at the pre-consolidation head.
        let analyzer = EnStemV1::new();
        let index_iri = iri("urn:eigenius:test:ti");
        let text_index = Arc::clone(&head_before.storage().text_index);
        let hits_before = run_text_search(
            &head_before,
            text_index.as_ref(),
            &index_iri,
            &analyzer,
            "wal truncation",
        )
        .expect("pre-consolidation search");
        let subjects_before: std::collections::BTreeSet<String> = hits_before
            .iter()
            .map(|h| h.subject.as_str().to_string())
            .collect();
        // Sanity: at least one expected hit landed.
        assert!(
            subjects_before.contains("urn:eigenius:test:doc1"),
            "doc1 must match 'wal truncation' pre-consolidation; got {subjects_before:?}"
        );

        // Consolidate the full content range (L_doc1..L_doc4). The
        // TextIndex Resource stays in `ti_layer` (outside the range)
        // so the post-consolidation chain still sees an active
        // TextIndex.
        let from = content_layers[0].id().clone();
        let to = head_before.id().clone();
        let outcome = consolidate_chain(
            "main",
            from,
            to,
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("consolidation succeeds");
        assert!(outcome.head_advanced);

        // Rebuild the chain from the new head and run the same
        // search. The text index storage handle is shared between
        // `storage` (used during consolidation) and the rebuilt
        // chain — `populate_text_indexes` ran against the
        // consolidated layer during build, writing fresh postings.
        let new_head_id = backend.get_branch("main").unwrap().unwrap();
        let info = backend.load_chain_from(&new_head_id).unwrap().unwrap();
        let head_after = crate::layer::build_chain(info, storage.clone());
        let hits_after = run_text_search(
            &head_after,
            text_index.as_ref(),
            &index_iri,
            &analyzer,
            "wal truncation",
        )
        .expect("post-consolidation search");
        let subjects_after: std::collections::BTreeSet<String> = hits_after
            .iter()
            .map(|h| h.subject.as_str().to_string())
            .collect();

        assert_eq!(
            subjects_before, subjects_after,
            "text-search hit set must be invariant under at-head consolidation \
             (D43 §2.8 atomicity)"
        );
    }

    /// D43 §2.8 / M8.1 — same invariant under below-head
    /// consolidation. A redirect routes head-rooted resolves
    /// through the consolidated layer; the text index must
    /// participate in that redirect so queries return the same
    /// hit set without the branch ref moving.
    #[test]
    fn text_search_equivalence_under_below_head_consolidation() {
        use crate::bootstrap::bootstrap;
        use crate::ontology::well_known as wk;
        use crate::query::text::analyzer::EnStemV1;
        use crate::query::text::search::run_text_search;

        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let target_prop = "urn:eigenius:test:body";

        let bootstrap_ctx = bootstrap().expect("bootstrap");
        let bootstrap_head = Arc::clone(bootstrap_ctx.head());
        // Walk the bootstrap chain head→root and persist every
        // layer into our backend so `load_chain_from` can rebuild
        // it after consolidation.
        {
            let mut cursor: Option<Arc<Layer>> = Some(Arc::clone(&bootstrap_head));
            while let Some(layer) = cursor {
                backend
                    .store_layer(&layer)
                    .expect("persist bootstrap layer");
                cursor = layer.parent().cloned();
            }
        }

        // TextIndex declaration.
        let mut ti_layer = LayerBuilder::new("ti", Some(Arc::clone(&bootstrap_head)));
        let mut ti = Resource::new(iri("urn:eigenius:test:ti"));
        ti.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::TEXT_INDEX_CLASS))]),
        );
        ti.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        ti.set(iri(wk::TEXT_ANALYZER), Value::String("en-stem-v1".into()));
        ti_layer.add_resource(ti).unwrap();
        let ti_layer = Arc::new(ti_layer.build(storage.clone()));
        backend.store_layer(&ti_layer).unwrap();

        // Three middle layers (to be consolidated) + two tail
        // layers (above the consolidated range, untouched).
        let bodies = [
            ("mid1", "WAL truncation in concurrent commit"),
            ("mid2", "WAL segment lifecycle"),
            ("mid3", "rollback during truncation"),
        ];
        let mut middle: Vec<Arc<Layer>> = Vec::new();
        let mut prev = Arc::clone(&ti_layer);
        for (sid, body) in bodies {
            let mut lb = LayerBuilder::new(&format!("M_{sid}"), Some(Arc::clone(&prev)));
            let mut r = Resource::new(iri(&format!("urn:eigenius:test:{sid}")));
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
            );
            r.set(iri(target_prop), Value::String(body.to_string()));
            lb.add_resource(r).unwrap();
            let layer = Arc::new(lb.build(storage.clone()));
            backend.store_layer(&layer).unwrap();
            middle.push(Arc::clone(&layer));
            prev = layer;
        }
        // Tail layer above the range.
        let mut tail_b = LayerBuilder::new("tail", Some(Arc::clone(&prev)));
        let mut r = Resource::new(iri("urn:eigenius:test:tail_doc"));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
        );
        r.set(
            iri(target_prop),
            Value::String("tail-only WAL discussion".into()),
        );
        tail_b.add_resource(r).unwrap();
        let tail = Arc::new(tail_b.build(storage.clone()));
        backend.store_layer(&tail).unwrap();
        backend.put_branch("main", tail.id()).unwrap();

        let analyzer = EnStemV1::new();
        let index_iri = iri("urn:eigenius:test:ti");
        let text_index = Arc::clone(&tail.storage().text_index);

        let hits_before = run_text_search(
            &tail,
            text_index.as_ref(),
            &index_iri,
            &analyzer,
            "wal truncation",
        )
        .expect("pre-consolidation search");
        let subjects_before: std::collections::BTreeSet<String> = hits_before
            .iter()
            .map(|h| h.subject.as_str().to_string())
            .collect();

        // Below-head consolidation: the range is the three middle
        // layers; the head (tail) stays untouched.
        let from = middle[0].id().clone();
        let to = middle[2].id().clone();
        let _outcome = consolidate_chain(
            "main",
            from,
            to,
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("below-head consolidation succeeds");

        // Branch ref didn't move — the redirect at `to` routes
        // through the consolidated layer.
        let new_head_id = backend.get_branch("main").unwrap().unwrap();
        assert_eq!(
            &new_head_id,
            tail.id(),
            "below-head consolidation must not move the branch ref"
        );
        let info = backend.load_chain_from(&new_head_id).unwrap().unwrap();
        let head_after = crate::layer::build_chain(info, storage.clone());

        let hits_after = run_text_search(
            &head_after,
            text_index.as_ref(),
            &index_iri,
            &analyzer,
            "wal truncation",
        )
        .expect("post-consolidation search");
        let subjects_after: std::collections::BTreeSet<String> = hits_after
            .iter()
            .map(|h| h.subject.as_str().to_string())
            .collect();

        assert_eq!(
            subjects_before, subjects_after,
            "text-search hit set must be invariant under below-head consolidation"
        );
    }

    // ─── D43 §2.8 / M8.2 vector-index consolidation ────────────

    /// D43 §2.8 / M8.2 — vector segment is concatenated across the
    /// collapsed range and the consolidated layer carries one
    /// segment per active VectorIndex with all surviving subjects.
    /// The pre-consolidation per-layer segments are still in
    /// storage but become unreachable through head-rooted lookups
    /// (chain walk skips them).
    ///
    /// Verifies the concat-and-relabel behaviour:
    ///   1. Pre: each layer in the range owns its own (subject, vec)
    ///      pair under `vec_seg:<I>:<L_n>`.
    ///   2. Post: a single segment under `vec_seg:<I>:<C>` contains
    ///      all surviving subjects with their pre-consolidation
    ///      vectors (deterministic via DummyEmbedder).
    #[test]
    fn vector_consolidation_concats_segments() {
        use crate::bootstrap::bootstrap;
        use crate::ontology::well_known as wk;
        use crate::program::embedder::{DummyEmbedder, EmbedderRegistry};
        use crate::query::vector::indexing::sweep_layer_vectors;

        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let target_prop = "urn:eigenius:test:body";
        let model_iri = "urn:eigenius:embed:dummy:v1";

        let bootstrap_ctx = bootstrap().expect("bootstrap");
        let bootstrap_head = Arc::clone(bootstrap_ctx.head());
        let mut cursor: Option<Arc<Layer>> = Some(Arc::clone(&bootstrap_head));
        while let Some(layer) = cursor {
            backend.store_layer(&layer).unwrap();
            cursor = layer.parent().cloned();
        }

        // VectorIndex declaration on `body`.
        let mut vi_layer = LayerBuilder::new("vi", Some(Arc::clone(&bootstrap_head)));
        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        vi_layer.add_resource(vi).unwrap();
        let vi_layer = Arc::new(vi_layer.build(storage.clone()));
        backend.store_layer(&vi_layer).unwrap();

        // Three layers contributing indexable Resources. Each layer
        // is swept after build, producing a per-layer segment with
        // exactly one (subject, vector) pair.
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model_iri, 8)));
        let bodies = [("v1", "alpha"), ("v2", "beta"), ("v3", "gamma")];
        let mut range_layers: Vec<Arc<Layer>> = Vec::new();
        let mut prev = Arc::clone(&vi_layer);
        for (sid, body) in bodies {
            let mut lb = LayerBuilder::new(&format!("L_{sid}"), Some(Arc::clone(&prev)));
            let mut r = Resource::new(iri(&format!("urn:eigenius:test:{sid}")));
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
            );
            r.set(iri(target_prop), Value::String(body.to_string()));
            lb.add_resource(r).unwrap();
            let layer = Arc::new(lb.build(storage.clone()));
            backend.store_layer(&layer).unwrap();
            sweep_layer_vectors(&layer, &reg, None).expect("sweep");
            range_layers.push(Arc::clone(&layer));
            prev = layer;
        }
        let head_before = Arc::clone(&prev);
        backend.put_branch("main", head_before.id()).unwrap();

        let vector_index = Arc::clone(&head_before.storage().vector_index);
        let index_iri = iri("urn:eigenius:test:vi");

        // Capture the pre-consolidation per-subject vectors.
        let mut pre_vectors: std::collections::BTreeMap<String, Vec<f32>> =
            std::collections::BTreeMap::new();
        for layer in &range_layers {
            if let Some(seg) = vector_index.get_segment(&index_iri, layer.id()).unwrap() {
                for (i, subject) in seg.subjects.iter().enumerate() {
                    pre_vectors.insert(subject.as_str().to_string(), seg.vector_at(i).to_vec());
                }
            }
        }
        assert_eq!(
            pre_vectors.len(),
            3,
            "each range layer must own its segment pre-consolidation"
        );

        // Consolidate the full range.
        let from = range_layers[0].id().clone();
        let to = head_before.id().clone();
        let outcome = consolidate_chain(
            "main",
            from,
            to,
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("consolidation succeeds");
        assert!(outcome.head_advanced);

        // Post-consolidation: a single segment under the consolidated
        // layer carries all three subjects with the same vectors.
        let new_head_id = backend.get_branch("main").unwrap().unwrap();
        let consolidated_seg = vector_index
            .get_segment(&index_iri, &new_head_id)
            .expect("get consolidated segment")
            .expect("consolidated segment exists");
        assert_eq!(consolidated_seg.dim, 8);
        assert_eq!(consolidated_seg.model_iri.as_str(), model_iri);
        assert_eq!(consolidated_seg.subjects.len(), 3);
        for (i, subject) in consolidated_seg.subjects.iter().enumerate() {
            let key = subject.as_str().to_string();
            let pre = pre_vectors
                .get(&key)
                .unwrap_or_else(|| panic!("subject {key} missing from pre-vectors"));
            assert_eq!(
                consolidated_seg.vector_at(i),
                pre.as_slice(),
                "consolidated vector for {key} must match pre-consolidation vector"
            );
        }
    }

    /// D43 §2.8 / M8.2 — subjects that are *tombstoned* in the
    /// collapsed range must not appear in the consolidated vector
    /// segment. Their vectors live in the now-unreachable per-layer
    /// segments, but the consolidated segment is the only one
    /// visible from the new head, so dropping them is the only way
    /// to keep query semantics consistent with the resolved-set
    /// view.
    #[test]
    fn vector_consolidation_excludes_tombstoned_subjects() {
        use crate::bootstrap::bootstrap;
        use crate::ontology::well_known as wk;
        use crate::program::embedder::{DummyEmbedder, EmbedderRegistry};
        use crate::query::vector::indexing::sweep_layer_vectors;

        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let target_prop = "urn:eigenius:test:body";
        let model_iri = "urn:eigenius:embed:dummy:v1";

        let bootstrap_ctx = bootstrap().expect("bootstrap");
        let bootstrap_head = Arc::clone(bootstrap_ctx.head());
        let mut cursor: Option<Arc<Layer>> = Some(Arc::clone(&bootstrap_head));
        while let Some(layer) = cursor {
            backend.store_layer(&layer).unwrap();
            cursor = layer.parent().cloned();
        }

        let mut vi_layer = LayerBuilder::new("vi", Some(Arc::clone(&bootstrap_head)));
        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        vi_layer.add_resource(vi).unwrap();
        let vi_layer = Arc::new(vi_layer.build(storage.clone()));
        backend.store_layer(&vi_layer).unwrap();

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model_iri, 8)));

        // L1 defines and indexes a subject that will later be
        // tombstoned in L2.
        let mut l1_b = LayerBuilder::new("L1", Some(Arc::clone(&vi_layer)));
        let mut r = Resource::new(iri("urn:eigenius:test:vanishing"));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
        );
        r.set(iri(target_prop), Value::String("about to vanish".into()));
        l1_b.add_resource(r).unwrap();
        let l1 = Arc::new(l1_b.build(storage.clone()));
        backend.store_layer(&l1).unwrap();
        sweep_layer_vectors(&l1, &reg, None).expect("sweep L1");

        // L2 tombstones the L1 subject and adds a surviving one.
        let mut l2_b = LayerBuilder::new("L2", Some(Arc::clone(&l1)));
        l2_b.tombstone(iri("urn:eigenius:test:vanishing")).unwrap();
        let mut r = Resource::new(iri("urn:eigenius:test:survivor"));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
        );
        r.set(iri(target_prop), Value::String("survivor body".into()));
        l2_b.add_resource(r).unwrap();
        let l2 = Arc::new(l2_b.build(storage.clone()));
        backend.store_layer(&l2).unwrap();
        sweep_layer_vectors(&l2, &reg, None).expect("sweep L2");
        backend.put_branch("main", l2.id()).unwrap();

        // Consolidate L1..L2. The vanishing subject is tombstoned
        // in the range, so consolidated_layer.defined_iris() doesn't
        // include it. The consolidated vector segment should contain
        // only the survivor.
        let outcome = consolidate_chain(
            "main",
            l1.id().clone(),
            l2.id().clone(),
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("consolidation succeeds");

        let vector_index = Arc::clone(&l2.storage().vector_index);
        let index_iri = iri("urn:eigenius:test:vi");
        let consolidated_seg = vector_index
            .get_segment(&index_iri, &outcome.consolidated_layer)
            .unwrap()
            .expect("consolidated segment exists");
        let subjects: Vec<String> = consolidated_seg
            .subjects
            .iter()
            .map(|s| s.as_str().to_string())
            .collect();
        assert_eq!(
            subjects,
            vec!["urn:eigenius:test:survivor".to_string()],
            "tombstoned subject must not appear in the consolidated vector segment; got {subjects:?}"
        );
    }

    /// D43 §2.8 / M8.2 — when the active VectorIndex strategy is
    /// `hnsw`, the consolidated segment must carry a freshly-built
    /// HNSW graph in its `hnsw_graph_bytes` payload. This pins the
    /// "rebuild on consolidation" deliverable from M8.
    #[test]
    fn vector_consolidation_rebuilds_hnsw_when_strategy_demands() {
        use crate::bootstrap::bootstrap;
        use crate::ontology::well_known as wk;
        use crate::program::embedder::{DummyEmbedder, EmbedderRegistry};
        use crate::query::vector::indexing::sweep_layer_vectors;

        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let target_prop = "urn:eigenius:test:body";
        let model_iri = "urn:eigenius:embed:dummy:v1";

        let bootstrap_ctx = bootstrap().expect("bootstrap");
        let bootstrap_head = Arc::clone(bootstrap_ctx.head());
        let mut cursor: Option<Arc<Layer>> = Some(Arc::clone(&bootstrap_head));
        while let Some(layer) = cursor {
            backend.store_layer(&layer).unwrap();
            cursor = layer.parent().cloned();
        }

        // VectorIndex with explicit `strategy: hnsw`.
        let mut vi_layer = LayerBuilder::new("vi", Some(Arc::clone(&bootstrap_head)));
        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        vi.set(
            iri(wk::VEC_STRATEGY),
            Value::ResourceRef(iri("urn:eigenius:core:strategies:hnsw")),
        );
        vi_layer.add_resource(vi).unwrap();
        let vi_layer = Arc::new(vi_layer.build(storage.clone()));
        backend.store_layer(&vi_layer).unwrap();

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model_iri, 8)));

        // Four content layers, each contributing one subject.
        let mut range: Vec<Arc<Layer>> = Vec::new();
        let mut prev = Arc::clone(&vi_layer);
        for i in 0..4 {
            let mut lb = LayerBuilder::new(&format!("L{i}"), Some(Arc::clone(&prev)));
            let mut r = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
            );
            r.set(iri(target_prop), Value::String(format!("body {i}")));
            lb.add_resource(r).unwrap();
            let layer = Arc::new(lb.build(storage.clone()));
            backend.store_layer(&layer).unwrap();
            sweep_layer_vectors(&layer, &reg, None).expect("sweep");
            range.push(Arc::clone(&layer));
            prev = layer;
        }
        backend.put_branch("main", prev.id()).unwrap();

        let outcome = consolidate_chain(
            "main",
            range[0].id().clone(),
            prev.id().clone(),
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("consolidation succeeds");

        let vector_index = Arc::clone(&prev.storage().vector_index);
        let index_iri = iri("urn:eigenius:test:vi");
        let consolidated_seg = vector_index
            .get_segment(&index_iri, &outcome.consolidated_layer)
            .unwrap()
            .expect("consolidated segment exists");
        assert_eq!(consolidated_seg.subjects.len(), 4);
        assert!(
            consolidated_seg.hnsw_graph_bytes.is_some(),
            "strategy=hnsw consolidated segment must carry an HNSW graph payload"
        );
        // Decode the bytes to confirm they're a real wire-format graph.
        let bytes = consolidated_seg.hnsw_graph_bytes.as_ref().unwrap();
        let layout = crate::query::vector::hnsw_format::decode(bytes)
            .expect("hnsw_graph_bytes decode to wire format");
        assert_eq!(layout.count(), 4);
    }

    /// D43 §2.8 / M8.6 — search-equivalence under at-head
    /// consolidation for `top_k_subjects`. The vector analogue of
    /// [`text_search_equivalence_under_at_head_consolidation`]:
    /// after consolidating a range of layers that each contributed
    /// vector segments, vector queries at the post-consolidation
    /// head must return the same ranked subject list (modulo the
    /// `defining_layer` label, which legitimately changes to the
    /// consolidated layer's id) as queries at the pre-consolidation
    /// head.
    ///
    /// Mechanism: [`consolidate_layer_vectors`] concatenates
    /// surviving vectors from the collapsed range, relabels their
    /// `defining_layer` to the consolidated layer's id, and writes
    /// one segment per active VectorIndex. The collapsed layers'
    /// segments become unreachable through head-rooted lookups; the
    /// candidate set is the same modulo the relabel.
    #[test]
    fn vector_search_equivalence_under_at_head_consolidation() {
        use crate::bootstrap::bootstrap;
        use crate::ontology::well_known as wk;
        use crate::program::embedder::{DummyEmbedder, Embedder, EmbedderRegistry};
        use crate::query::vector::distance::Metric;
        use crate::query::vector::indexing::sweep_layer_vectors;
        use crate::query::vector::search::top_k_subjects;

        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();
        let target_prop = "urn:eigenius:test:body";
        let model_iri = "urn:eigenius:embed:dummy:v1";

        let bootstrap_ctx = bootstrap().expect("bootstrap");
        let bootstrap_head = Arc::clone(bootstrap_ctx.head());
        let mut cursor: Option<Arc<Layer>> = Some(Arc::clone(&bootstrap_head));
        while let Some(layer) = cursor {
            backend
                .store_layer(&layer)
                .expect("persist bootstrap layer");
            cursor = layer.parent().cloned();
        }

        // VectorIndex declaration on `body`.
        let mut vi_layer = LayerBuilder::new("vi", Some(Arc::clone(&bootstrap_head)));
        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        vi_layer.add_resource(vi).unwrap();
        let vi_layer = Arc::new(vi_layer.build(storage.clone()));
        backend.store_layer(&vi_layer).unwrap();

        // Four content layers each contributing one Resource. Each
        // is swept post-build so its per-layer segment carries the
        // (subject, vector) pair.
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model_iri, 8)));
        let bodies = [
            ("doc1", "WAL truncation under concurrent commit"),
            ("doc2", "wal segment rotation"),
            ("doc3", "rollback handling unrelated to WAL"),
            ("doc4", "truncation of orphan log files"),
        ];
        let mut content_layers: Vec<Arc<Layer>> = Vec::new();
        let mut prev = Arc::clone(&vi_layer);
        for (sid, body) in bodies {
            let mut lb = LayerBuilder::new(&format!("L_{sid}"), Some(Arc::clone(&prev)));
            let mut r = Resource::new(iri(&format!("urn:eigenius:test:{sid}")));
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
            );
            r.set(iri(target_prop), Value::String(body.to_string()));
            lb.add_resource(r).unwrap();
            let layer = Arc::new(lb.build(storage.clone()));
            backend.store_layer(&layer).unwrap();
            sweep_layer_vectors(&layer, &reg, None).expect("sweep");
            content_layers.push(Arc::clone(&layer));
            prev = layer;
        }
        let head_before = Arc::clone(&prev);
        backend.put_branch("main", head_before.id()).unwrap();

        // Embed a query string and capture the ranked subject list
        // at the pre-consolidation head. The query vector is the
        // DummyEmbedder applied to a deterministic input — its
        // numerical value is irrelevant; what matters is that the
        // same vector is fed to both pre- and post-consolidation
        // probes.
        let embedder = DummyEmbedder::new(model_iri, 8);
        let query_vec = embedder.embed("wal truncation").expect("query embed");
        let index_iri = iri("urn:eigenius:test:vi");
        let model_iri_parsed = iri(model_iri);
        let vector_index = Arc::clone(&head_before.storage().vector_index);
        let hits_before = top_k_subjects(
            &head_before,
            vector_index.as_ref(),
            None,
            &index_iri,
            &query_vec,
            10,
            None,
            &model_iri_parsed,
            Metric::Cosine,
        )
        .expect("pre-consolidation probe");
        let subjects_before: Vec<String> = hits_before
            .iter()
            .map(|h| h.subject.as_str().to_string())
            .collect();
        assert!(
            !subjects_before.is_empty(),
            "pre-consolidation probe must return hits"
        );

        // Consolidate the full content range.
        let from = content_layers[0].id().clone();
        let to = head_before.id().clone();
        let outcome = consolidate_chain(
            "main",
            from,
            to,
            ConsolidateOpts::default(),
            storage.clone(),
            &backend,
        )
        .expect("consolidation succeeds");
        assert!(outcome.head_advanced);

        // Rebuild the chain from the new head and re-probe with the
        // same query vector. The consolidated segment carries every
        // surviving subject under the new head's id; the ranked
        // subject list (ignoring `defining_layer`, which moved to
        // the consolidated layer) must be invariant.
        let new_head_id = backend.get_branch("main").unwrap().unwrap();
        let info = backend.load_chain_from(&new_head_id).unwrap().unwrap();
        let head_after = crate::layer::build_chain(info, storage.clone());
        let hits_after = top_k_subjects(
            &head_after,
            vector_index.as_ref(),
            None,
            &index_iri,
            &query_vec,
            10,
            None,
            &model_iri_parsed,
            Metric::Cosine,
        )
        .expect("post-consolidation probe");
        let subjects_after: Vec<String> = hits_after
            .iter()
            .map(|h| h.subject.as_str().to_string())
            .collect();

        assert_eq!(
            subjects_before, subjects_after,
            "vector-search ranked subject list must be invariant under at-head consolidation \
             (D43 §2.8 atomicity)"
        );
        // Defining-layer for every post-consolidation hit must be
        // the new consolidated layer — the collapsed range's per-
        // layer segments are unreachable through head-rooted lookups.
        for hit in &hits_after {
            assert_eq!(
                hit.defining_layer, new_head_id,
                "post-consolidation hit must be labeled with the consolidated layer's id"
            );
        }
    }
}
