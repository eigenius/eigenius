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
use crate::validation::ValidationError;

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

/// Result of a single [`LayerPersister::persist`] call.
///
/// Mirrors today's `crate::server::PersistedLayerInfo` shape (see
/// `kernel/src/server/mod.rs`) so that Phase C can collapse the two
/// definitions into one without changing call sites. The new struct is
/// shadow-defined here for Phase A; the server's copy is still the one
/// in use until Phase C wires the persister.
///
/// Field semantics:
///
/// - `layer_id`: the **canonical** layer id. For a cache hit at a
///   different position (D33 §6) this is the cached layer's id, not the
///   freshly-built one.
/// - `branch_advanced`: `true` iff the persist actually moved the
///   branch ref. Drives the orchestrator's drain / revert decision and
///   the `didPersist` hook gate (see D41 §6.1).
/// - `merge_outcome`: `Some(...)` whenever a CAS actually ran (cache
///   miss or same-position cache hit); `None` otherwise.
/// - `cache_hit_different_position`: `true` iff the persist
///   short-circuited because the anchored-commit cache (D33 §6) found a
///   content-equivalent layer at a different chain position.
///
/// D41 §7.
#[derive(Debug, Clone)]
pub struct PersistedLayerInfo {
    /// Canonical layer id (cached or freshly written).
    pub layer_id: crate::layer::LayerId,
    /// `true` iff the persist actually moved the branch ref.
    pub branch_advanced: bool,
    /// `Some(...)` iff a CAS actually ran; `None` for the no-backend path
    /// and for different-position cache hits.
    pub merge_outcome: Option<crate::lattice::UpdateOutcome>,
    /// `true` iff the anchored-commit cache (D33 §6) short-circuited
    /// with a different-position hit.
    pub cache_hit_different_position: bool,
}
