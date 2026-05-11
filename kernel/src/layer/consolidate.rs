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
}
