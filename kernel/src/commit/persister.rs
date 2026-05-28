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

//! `LayerPersister` boundary and the [`PersistedLayerInfo`] it returns.
//!
//! The pipeline's `persist` phase (see `phases::persist`) calls into a
//! `LayerPersister`. `EigeniusService` will implement the trait in Phase C
//! by lifting the body of today's `persist_layer_if_backend` (see
//! `kernel/src/server/mod.rs`). Tests will inject in-memory implementations
//! that return canned [`PersistedLayerInfo`] values.
//!
//! Phase A: trait + struct are defined. No implementations.
//!
//! See D41 §7 for the contract and §11.1 for module layout.

use std::sync::Arc;

use crate::layer::Layer;
use crate::storage::PersistentBackend;
use crate::validation::{ValidationError, ValidationRule};

/// The persist seam between the commit pipeline and storage.
///
/// The pipeline's `persist` phase calls [`LayerPersister::persist`] once
/// per layer. The implementation owns:
///
/// - the anchored-commit cache probe (D33 §6),
/// - the `backend.store_layer` write,
/// - the `update_branch` CAS,
/// - the trivial-merge handling.
///
/// The pipeline does not interpret the [`PersistedLayerInfo`]; the
/// orchestrator inspects [`PersistedLayerInfo::branch_advanced`] to decide
/// whether to drain emissions or skip descendants.
///
/// See D41 §7.
pub trait LayerPersister: Send + Sync {
    /// Persist `layer` against `branch`, returning the canonical id and
    /// merge / cache outcome. Errors are surfaced as
    /// [`ValidationError`] so they slot into [`crate::lattice::CommitError`]
    /// reporting without an additional wrapper layer (Phase A; Phase B / E
    /// may split this).
    fn persist(
        &self,
        branch: &str,
        layer: &Arc<Layer>,
    ) -> Result<PersistedLayerInfo, ValidationError>;
}

/// Result of a single [`LayerPersister::persist`] call — the
/// canonical [`crate::layer::LayerId`] for the committed content
/// paired with the merge outcome and a derived `branch_advanced`
/// flag.
///
/// **Canonical home (D41 §7 / §11.1):** this is the single struct
/// definition for the persist result. The server-side duplicate that
/// previously lived in `crate::server::mod` was deleted in Phase C;
/// the server now imports this type and continues to use the same
/// field names.
///
/// **`branch_advanced` semantics** (D33 §6 + D23 §5.4):
///
/// - `true` — the durable branch ref moved as a result of this
///   persist. Holds for cache misses, same-position cache hits, and
///   both `FastForward` / `TrivialMerge` CAS outcomes.
/// - `false` — the branch ref did **not** move. Holds for: no
///   persistent backend, different-position cache hit, and the
///   `NeedsWitnessedMerge` CAS outcome (the layer is stored but
///   unreachable from any branch ref).
///
/// **`merge_outcome` semantics:**
///
/// `Some(...)` whenever a CAS attempt actually ran (cache miss or
/// same-position cache hit); `None` for the no-backend path and for
/// different-position cache hits — in both cases there is no merge
/// taxonomy because no CAS happened. The proto boundary maps `None`
/// to `proto::MergeOutcome::Unspecified`.
///
/// **Why `Option<UpdateOutcome>` + `cache_hit_different_position`
/// instead of D41's `update_outcome: UpdateOutcome` + `cache_hit: bool`
/// spec.** The current shape encodes three distinct post-persist
/// states the proto wire format needs to distinguish: cache hit at a
/// different position, CAS ran, and no CAS attempted. Collapsing
/// `Option<UpdateOutcome>` into a non-optional `UpdateOutcome` would
/// force a synthetic "did not attempt" variant on
/// [`crate::lattice::UpdateOutcome`], which is the wrong shape for an
/// enum that names CAS results. Phase C of D41 deferred reconciliation
/// of the doc spec to a docs follow-up rather than reshape the
/// survivor; the canonical-struct goal of §7 is achieved by deduping,
/// not by reshaping.
///
/// D41 §7.
#[derive(Debug, Clone)]
pub struct PersistedLayerInfo {
    /// Canonical layer id. For a cache hit at a different position
    /// (D33 §6) this is the cached layer's id, not the freshly-built
    /// one.
    pub layer_id: crate::layer::LayerId,
    /// `true` iff the persist actually moved the branch ref. Drives
    /// the orchestrator's drain / revert decision and the
    /// `didPersist` hook gate (see D41 §6.1).
    pub branch_advanced: bool,
    /// `Some(...)` iff a CAS actually ran; `None` for the no-backend
    /// path and for different-position cache hits.
    pub merge_outcome: Option<crate::lattice::UpdateOutcome>,
    /// `true` iff the persist short-circuited because the
    /// anchored-commit cache (D33 §6) found a content-equivalent
    /// layer at a different chain position. Distinguished from the
    /// no-backend / no-CAS case (where `merge_outcome` is also `None`
    /// and `branch_advanced` is also `false`) so the response can
    /// carry a `MERGE_OUTCOME_CACHED_DIFFERENT_POSITION` signal that
    /// consumers can render distinctly from "no commit shape
    /// information available".
    pub cache_hit_different_position: bool,
}

/// Minimal [`LayerPersister`] for callers that just need
/// `PersistentBackend::store_layer` — no anchored-commit cache, no
/// branch CAS. Used by [`crate::lattice::commit_layer`] and
/// [`crate::lattice::commit_layer_default`] (CLI commits, bootstrap,
/// GC tests, storage E2E tests). Returns a [`PersistedLayerInfo`] with
/// `merge_outcome = None` and `branch_advanced = false` — there is no
/// branch CAS in this path, so "did the branch advance?" is a question
/// the lattice wrapper deliberately doesn't answer.
///
/// `EigeniusService` will implement [`LayerPersister`] directly with
/// cache + CAS (D41 Phase C); only the simple commit path uses this
/// adapter. The adapter exists during Phase B so the new
/// [`crate::commit::CommitPipeline::with_retroactive`] pipeline can
/// service the lattice's pre-D41 callers without yet pulling the
/// server's persistence stack into the kernel core.
///
/// D41 §7 / Phase B.
pub struct BackendStorePersister<'a> {
    /// Backend the persister writes through.
    pub backend: &'a dyn PersistentBackend,
}

impl LayerPersister for BackendStorePersister<'_> {
    /// `branch` is ignored — the lattice path is branch-agnostic.
    /// Storage errors translate to a synthetic [`ValidationError`]
    /// (rule [`ValidationRule::InstitutionValidation`] as a Phase B
    /// stand-in; the persister-returns-`ValidationError` shape is a
    /// known transitional fiction that Phase C resolves by widening
    /// the persister error type).
    fn persist(
        &self,
        _branch: &str,
        layer: &Arc<Layer>,
    ) -> Result<PersistedLayerInfo, ValidationError> {
        self.backend
            .store_layer(layer)
            .map_err(|e| ValidationError {
                resource_id: None,
                property: None,
                // D41 Phase B: no `ValidationRule` variant fits "I/O
                // failure". Phase E may revisit the persister error
                // type; for now use `InstitutionValidation` as the
                // most policy-shaped existing variant and carry the
                // backend message verbatim so callers can identify
                // the underlying cause.
                rule: ValidationRule::InstitutionValidation,
                message: format!("persist_layer failed: {e}"),
            })?;
        Ok(PersistedLayerInfo {
            layer_id: layer.id().clone(),
            // No CAS in this path — the lattice wrapper does not
            // advance any branch ref. Phase D / E will revisit how
            // this surfaces to orchestrator drains; today it never
            // matters because the lattice wrapper unpacks the layer
            // directly and discards the rest of [`PersistedLayerInfo`].
            branch_advanced: false,
            merge_outcome: None,
            cache_hit_different_position: false,
        })
    }
}
