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

//! Resolve redirects — forward pointers for consolidating below the
//! branch head (D25 §12.8).
//!
//! A redirect lives outside the layer-id hash domain. When `Layer::resolve`
//! walks head→root and reaches a layer that's a redirect source, the walk
//! short-circuits to the redirect target (the consolidated `L_c`) and
//! continues from there. The original layer's topology slot can be
//! reclaimed on disk because [`PersistentBackend::load_topology`]
//! manufactures a synthetic in-memory [`LayerHandle`] from each
//! `RedirectEntry` at startup — every topology-walk caller sees a
//! consistent DAG (D25 §12.8.1(d)).

use crate::layer::{LayerHandle, LayerId};

/// Persistent record of one resolve redirect installed by
/// `consolidate_chain` when `to` is below the branch head.
///
/// Carries enough of the original `to` layer's metadata to manufacture
/// the in-memory synthetic tombstone in `load_topology` even when the
/// original `LayerHandle` has been reclaimed. The fields mirror
/// `LayerHandle` directly — the redirect entry is "a `LayerHandle` plus
/// the redirect target."
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RedirectEntry {
    /// Position hash of the consolidated layer (the redirect target).
    /// Resolves above this layer follow the redirect and continue
    /// walking from `target`.
    pub target: LayerId,
    /// Snapshot of `to`'s `LayerHandle` at install time. Used by
    /// `load_topology` to manufacture the in-memory tombstone so
    /// every parent-pointer walk in the kernel sees a consistent
    /// topology DAG (D25 §12.8.1(d)). The `id` field on this handle
    /// is the redirect source.
    pub source_handle: LayerHandle,
}

impl RedirectEntry {
    /// Convenience: the `LayerId` of the layer this redirect replaces.
    pub fn source(&self) -> &LayerId {
        &self.source_handle.id
    }
}

/// Manufacture the in-memory synthetic tombstone for this redirect.
///
/// The returned `LayerHandle` matches the original `to` layer's
/// structure (id, parents, content_hash, supporting_layer, name,
/// resource_count, created_at) and additionally has
/// `is_redirect_source = true` so diagnostic surfaces can render it
/// as "consolidated into <target>" rather than as an ordinary
/// (empty-looking) layer.
pub fn manufacture_tombstone(entry: &RedirectEntry) -> LayerHandle {
    LayerHandle {
        is_redirect_source: true,
        ..entry.source_handle.clone()
    }
}

/// Augment a `LayerTopology` with synthetic tombstones for every
/// redirect whose source isn't already present in the topology.
///
/// Called by `PersistentBackend::load_topology` after the topology
/// CF has been read. Idempotent: redirects whose source is still on
/// disk (preserve-history mode) leave the topology entry alone;
/// reclaimed sources get a synthetic entry inserted.
pub fn augment_topology_with_redirects(
    topology: &mut crate::layer::LayerTopology,
    redirects: &[RedirectEntry],
) {
    for entry in redirects {
        if topology.get_layer(entry.source()).is_none() {
            topology.insert_layer(manufacture_tombstone(entry));
        }
    }
    // Note: we deliberately do NOT touch entries that already exist.
    // Preserve-history mode leaves both the original handle and the
    // redirect in storage; the original handle wins. The redirect
    // still drives the resolve-walk short-circuit (handled in
    // `Layer::redirect_target`); only the topology slot is shared.
}
