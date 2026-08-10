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

//! Layer topology — metadata-only handles for the layer DAG.
//!
//! Phase 14 (D23) separates layer topology from resource content. This module
//! holds the topology side: `LayerHandle` is a small fixed-size description of
//! one layer (id, parents, metadata), and `LayerTopology` holds the in-memory
//! DAG of all known handles.
//!
//! The full content for a layer lives behind a `ResourceCache` (see
//! `crate::layer::cache`), backed by the persistent store. This module is
//! purely topology — no resource content passes through it.
//!
//! The kernel does not track "current state" beyond the DAG itself. There is
//! no `head`, no `tip`, no notion of a "current branch." Tasks carry their own
//! pin in `TaskRecord.layer_head` (D21); other clients carry their pin in
//! their own session state. The only kernel write operation is "append a
//! layer with these parents" — branches, named refs, current-head conventions
//! all belong above the kernel.
//!
//! Phase 14a-i ships these types as pure additions; they are not yet wired
//! into `Layer` or the persistent backend (those are 14a-ii and 14a-iii).

use crate::layer::{ContentHash, LayerId};
use crate::ontology::iri::Iri;
use std::collections::{BTreeMap, BTreeSet};

/// Metadata-only handle for a layer.
///
/// Replaces the in-memory `Arc<Layer>` chain that holds full resource maps.
/// One `LayerHandle` per committed layer; the entire collection is held in
/// memory (it's small — bounded by the number of layers, not by graph size).
///
/// `parents` is a `Vec` to support multi-parent merge layers introduced in
/// Phase 15. In Phase 14 it is always 0 or 1 entries: `[]` for the root layer,
/// `[parent_id]` for every other layer.
///
/// **Two-hash identity (D25 §11.0 / D33 §5.1).** Handles carry both the
/// position-addressed `id` and the content-only `content_hash`. Content
/// hash duplicates across positions are expected (anchored-commit cache,
/// content-hash dedup); position hashes are globally unique.
/// Serde default for [`LayerHandle::has_witness_candidates`] — see that field. Conservative:
/// a handle written before the field existed carries no information, so assume the layer may hold
/// witnesses and probe it.
fn witness_candidates_unknown() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LayerHandle {
    /// Position-addressed identifier for this layer. Folds content
    /// hash + sorted parent ids; uniquely identifies the layer's slot
    /// in the DAG.
    pub id: LayerId,

    /// Content-only hash of the layer's resources (independent of
    /// position). Two layers with identical resources at different DAG
    /// positions share this hash. See [`ContentHash`] for the
    /// position-vs-content distinction.
    pub content_hash: ContentHash,

    /// Supporting layer per D33 §4.3 — the youngest ancestor this
    /// layer explicitly depends on. `None` for the root layer, for
    /// layers with no external references, and (transiently) for
    /// pre-PR-0 layers whose supporting layer hasn't been
    /// back-filled. Carried on the topology entry so resume reads
    /// don't need to recompute on every load.
    pub supporting_layer: Option<LayerId>,

    /// Parent layer ids. Empty for the root layer; one entry for every other
    /// Phase-14 layer; multiple entries for Phase-15 merge layers.
    pub parents: Vec<LayerId>,

    /// Human-readable label for the layer. Carries over from the existing
    /// `Layer::name` field; useful for diagnostics.
    pub name: String,

    /// Number of resources defined directly *in this layer* (not the chain).
    /// Used for diagnostics and as a hint for sizing decisions; not load-
    /// bearing for correctness.
    pub resource_count: u64,

    /// Milliseconds since Unix epoch — when the layer was committed. Matches
    /// the convention used by D21's `TaskRecord`.
    pub created_at: i64,

    /// **Witness-scan skip hint (D66 slice 0).** `false` iff this layer defines *no* resource that
    /// could ever admit a `ChainWitness` — no Trace, no `InstitutionEmittedDerivation`, no
    /// `ReasoningSentence`. A `lookup_chain_witness` walk skips such a layer outright instead of
    /// probing it.
    ///
    /// Stamped by `store_layer` at write time from the layer's own immutable resources, exactly like
    /// `resource_count` and `encoded_bytes`. Layers are immutable, so it can never go stale, and it
    /// rides the handle's own write rather than a separate index batch (unlike the derived indexes —
    /// claims-audit A1).
    ///
    /// **Defaults to `true`, and must.** Handles persisted before this field existed decode without
    /// it; `true` means "no information, go and look", which costs the optimisation and nothing else.
    /// `false` would mark every pre-existing layer witness-free and silently break every citation
    /// into history — a performance hint turned into a correctness bug.
    #[serde(default = "witness_candidates_unknown")]
    pub has_witness_candidates: bool,

    /// Encoded resource bytes for this layer — the sum of
    /// `eigon_cbor::serialize_resource(...).len()` over every resource
    /// defined directly in the layer. Stamped by `store_layer` at write
    /// time and persisted alongside the rest of the handle.
    ///
    /// Used by GC's `EstimateGc` to surface a "reclaimable bytes" view
    /// for the operator (D34 §G.4 / §9.4). **Approximate by design**:
    /// excludes the per-layer bloom, topology entry, chain pointer,
    /// content-hash index, and triple-index entries — those are
    /// bounded per-layer overhead, dwarfed by resource bytes in any
    /// realistic workload. A future enrichment that wants exact storage
    /// footprint would aggregate the full WriteBatch byte count at
    /// store time.
    ///
    /// `#[serde(default)]` on the field tolerates handles persisted by
    /// older kernels that didn't carry this number — they read back as
    /// `0` and the estimate under-counts for legacy layers until they
    /// churn through GC + recommit. No migration needed.
    #[serde(default)]
    pub byte_size: u64,

    /// True for *synthetic tombstones* manufactured by `load_topology`
    /// from a [`RedirectEntry`](crate::layer::RedirectEntry). The flag
    /// lets diagnostic surfaces (`db log`, `inspect`, the notebook
    /// topology renderer) display "consolidated into <target>" rather
    /// than rendering an ordinary-looking handle whose on-disk content
    /// has been reclaimed.
    ///
    /// **Always `false`** on handles written to disk by `store_layer`
    /// — the flag is an in-memory signal only.
    pub is_redirect_source: bool,

    /// IRIs explicitly tombstoned at this layer (D20 §6.2 / §6.3 for
    /// `Rename` and `SchemaQuotient::KeepNeither` resolutions; 15g
    /// step 3). The lookup walker treats a tombstoned IRI as
    /// "removed from view": `Layer::resolve(iri)` returns `None`
    /// when the walk encounters a tombstoning layer before any
    /// defining layer.
    ///
    /// `#[serde(default)]` tolerates handles persisted by older
    /// kernels — they read back with an empty set and the lookup
    /// behaves identically to pre-15g chains.
    #[serde(default)]
    pub tombstoned_iris: BTreeSet<Iri>,
}

impl LayerHandle {
    /// Convenience: returns true if this is the root layer (no parents).
    pub fn is_root(&self) -> bool {
        self.parents.is_empty()
    }
}

/// In-memory description of the layer DAG.
///
/// Holds one `LayerHandle` per known layer. The whole structure is bounded
/// by the number of layers, not by graph content size — comfortably in-memory
/// even at 100k+ layers.
#[derive(Debug, Default, Clone)]
pub struct LayerTopology {
    layers: BTreeMap<LayerId, LayerHandle>,
}

impl LayerTopology {
    /// Create an empty topology.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a layer handle. Idempotent by `LayerId`: re-inserting
    /// the same handle is a no-op (content-addressed identity), so this is
    /// safe to call repeatedly during startup population.
    pub fn insert_layer(&mut self, handle: LayerHandle) {
        self.layers.insert(handle.id.clone(), handle);
    }

    /// Look up a handle by id. Returns `None` if unknown to this topology.
    pub fn get_layer(&self, id: &LayerId) -> Option<&LayerHandle> {
        self.layers.get(id)
    }

    /// Total number of layers known to the topology.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Iterate every known layer handle in `LayerId` order. Used by
    /// Phase 14f GC's sweep phase, which needs to enumerate the full
    /// topology to find layers not in the reachable set, and by any
    /// future caller that wants a topology-wide scan.
    pub fn iter_layers(&self) -> impl Iterator<Item = &LayerHandle> {
        self.layers.values()
    }

    /// Iterate ancestors of `start`, top-down (most recent first), via parent
    /// pointers. Stops at the root or at any unknown id. Phase 14 layers have
    /// at most one parent, so this is a linear walk; Phase 15's merge layers
    /// may introduce multi-parent walks (handled by callers as needed).
    ///
    /// Returns an empty iterator if `start` itself is unknown.
    pub fn walk_chain<'a>(&'a self, start: &'a LayerId) -> ChainIter<'a> {
        ChainIter {
            topology: self,
            next: Some(start.clone()),
        }
    }
}

/// Iterator yielded by `LayerTopology::walk_chain`. Walks parent pointers
/// top-down. For multi-parent (merge) layers, only the first parent is
/// followed; full DAG traversal is handled by GC and merge code that needs
/// it (lands in 14f / Phase 15).
pub struct ChainIter<'a> {
    topology: &'a LayerTopology,
    next: Option<LayerId>,
}

impl<'a> Iterator for ChainIter<'a> {
    type Item = &'a LayerHandle;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.next.take()?;
        let handle = self.topology.layers.get(&id)?;
        self.next = handle.parents.first().cloned();
        Some(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lid(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }

    fn handle(byte: u8, parents: Vec<LayerId>) -> LayerHandle {
        LayerHandle {
            id: lid(byte),
            content_hash: ContentHash([byte; 32]),
            supporting_layer: None,
            parents,
            name: format!("layer-{byte}"),
            resource_count: 0,
            has_witness_candidates: false,
            created_at: 0,
            byte_size: 0,
            is_redirect_source: false,
            tombstoned_iris: BTreeSet::new(),
        }
    }

    /// A handle persisted before `has_witness_candidates` existed must decode as `true`.
    ///
    /// `false` would mark every pre-existing layer witness-free and silently break every citation
    /// into history. This is the test that keeps the serde default honest.
    #[test]
    fn handle_without_witness_flag_decodes_as_unknown() {
        let mut without = std::collections::BTreeMap::new();
        without.insert("id", ciborium::Value::Bytes(vec![7u8; 32]));
        without.insert("content_hash", ciborium::Value::Bytes(vec![7u8; 32]));
        without.insert("supporting_layer", ciborium::Value::Null);
        without.insert("parents", ciborium::Value::Array(vec![]));
        without.insert("name", ciborium::Value::Text("legacy".into()));
        without.insert("resource_count", ciborium::Value::Integer(0.into()));
        without.insert("created_at", ciborium::Value::Integer(0.into()));
        without.insert("byte_size", ciborium::Value::Integer(0.into()));
        without.insert("is_redirect_source", ciborium::Value::Bool(false));
        without.insert("tombstoned_iris", ciborium::Value::Array(vec![]));
        let map = ciborium::Value::Map(
            without
                .into_iter()
                .map(|(k, v)| (ciborium::Value::Text(k.into()), v))
                .collect(),
        );
        let mut bytes = Vec::new();
        ciborium::into_writer(&map, &mut bytes).unwrap();
        let decoded: LayerHandle =
            ciborium::from_reader(bytes.as_slice()).expect("a pre-D66 handle must still decode");
        assert!(
            decoded.has_witness_candidates,
            "absent flag must mean UNKNOWN (probe the layer), never `false`"
        );
    }

    /// A kernel built before this field must still read a DB written after it — the D24 criterion
    /// for whether `SCHEMA_VERSION` needs a bump. ciborium writes structs as named maps and serde
    /// ignores unknown keys, so the extra field is skipped rather than fatal. No bump required.
    #[test]
    fn handle_with_unknown_field_still_decodes() {
        let h = handle(3, vec![]);
        let mut bytes = Vec::new();
        ciborium::into_writer(&h, &mut bytes).unwrap();
        let value: ciborium::Value = ciborium::from_reader(bytes.as_slice()).unwrap();
        let ciborium::Value::Map(mut entries) = value else {
            panic!("LayerHandle must serialise as a CBOR map with named keys");
        };
        entries.push((
            ciborium::Value::Text("a_field_from_the_future".into()),
            ciborium::Value::Bool(true),
        ));
        let mut extended = Vec::new();
        ciborium::into_writer(&ciborium::Value::Map(entries), &mut extended).unwrap();
        let decoded: LayerHandle =
            ciborium::from_reader(extended.as_slice()).expect("unknown fields must be ignored");
        assert_eq!(decoded.id, h.id);
    }

    #[test]
    fn empty_topology() {
        let topo = LayerTopology::new();
        assert_eq!(topo.layer_count(), 0);
        assert!(topo.get_layer(&lid(1)).is_none());
    }

    #[test]
    fn insert_and_lookup_layer() {
        let mut topo = LayerTopology::new();
        topo.insert_layer(handle(1, vec![]));
        assert_eq!(topo.layer_count(), 1);
        let h = topo.get_layer(&lid(1)).unwrap();
        assert_eq!(h.name, "layer-1");
        assert!(h.is_root());
    }

    #[test]
    fn insert_is_idempotent() {
        let mut topo = LayerTopology::new();
        topo.insert_layer(handle(1, vec![]));
        topo.insert_layer(handle(1, vec![]));
        assert_eq!(topo.layer_count(), 1);
    }

    #[test]
    fn walk_chain_linear() {
        // Build chain: L1 (root) <- L2 <- L3
        let mut topo = LayerTopology::new();
        topo.insert_layer(handle(1, vec![]));
        topo.insert_layer(handle(2, vec![lid(1)]));
        topo.insert_layer(handle(3, vec![lid(2)]));

        let walked: Vec<u8> = topo.walk_chain(&lid(3)).map(|h| h.id.0[0]).collect();
        assert_eq!(walked, vec![3, 2, 1]);
    }

    #[test]
    fn walk_chain_from_root_yields_only_root() {
        let mut topo = LayerTopology::new();
        topo.insert_layer(handle(1, vec![]));
        let walked: Vec<u8> = topo.walk_chain(&lid(1)).map(|h| h.id.0[0]).collect();
        assert_eq!(walked, vec![1]);
    }

    #[test]
    fn walk_chain_unknown_head_is_empty() {
        let topo = LayerTopology::new();
        assert_eq!(topo.walk_chain(&lid(99)).count(), 0);
    }

    #[test]
    fn walk_chain_stops_at_unknown_parent() {
        // L2's parent (L1) is missing from the topology — walk yields just L2.
        let mut topo = LayerTopology::new();
        topo.insert_layer(handle(2, vec![lid(1)]));
        let walked: Vec<u8> = topo.walk_chain(&lid(2)).map(|h| h.id.0[0]).collect();
        assert_eq!(walked, vec![2]);
    }

    #[test]
    fn walk_chain_follows_first_parent_only() {
        // Multi-parent (merge) layer: L3 has parents [L2, L1]. The iterator
        // only follows L2 (the first parent). Full DAG traversal is the
        // caller's responsibility.
        let mut topo = LayerTopology::new();
        topo.insert_layer(handle(1, vec![]));
        topo.insert_layer(handle(2, vec![lid(1)]));
        topo.insert_layer(handle(3, vec![lid(2), lid(1)]));
        let walked: Vec<u8> = topo.walk_chain(&lid(3)).map(|h| h.id.0[0]).collect();
        assert_eq!(walked, vec![3, 2, 1]);
    }

    #[test]
    fn handle_serde_round_trips() {
        let h = handle(42, vec![lid(7), lid(8)]);
        let mut bytes = Vec::new();
        ciborium::into_writer(&h, &mut bytes).unwrap();
        let back: LayerHandle = ciborium::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(h, back);
    }
}
