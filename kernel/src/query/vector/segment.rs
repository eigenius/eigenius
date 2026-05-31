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

//! D43 §2.4 / M5.10 — zero-copy vector segment reader primitives.
//!
//! The plan's hot-loop design for vector retrieval:
//!
//! > One `db.get` into `Arc<[u8]>` at SegmentCache admission; CBOR
//! > header parse → `VectorSegmentLayout` with byte ranges;
//! > `bytemuck::cast_slice::<u8, f32>(...)` for the SIMD-ready
//! > `&[f32]`.
//!
//! Two halves of that vision land separately:
//!
//! 1. **The reader-side abstraction** — [`SegmentView`] holds an
//!    aligned `Arc<[u8]>` of vector bytes + the structural metadata
//!    (model_iri, dim, distance, subjects). [`SegmentView::vectors`]
//!    returns a `&[f32]` via `bytemuck::cast_slice` without copying.
//!    This is what M6 (HNSW) and the SIMD distance kernels (M5.9)
//!    consume; it's the stable surface other modules target.
//!
//! 2. **The storage-side single-`get` path** — the RocksDB backend's
//!    `get_segment` today does a CBOR decode that allocates a fresh
//!    `Vec<f32>`. Making it return `Arc<[u8]>` directly requires the
//!    encoder to insert alignment padding (a `_pad` field of zero
//!    bytes sized so the `vectors` bstr starts at a 4-byte boundary
//!    inside the CBOR blob) and a partial CBOR parser that records
//!    byte offsets without materialising values. That work is the
//!    M5.10 follow-up — see the module's `storage_side_zero_copy`
//!    doc-comment below.
//!
//! What ships now: the [`SegmentView`] surface, an aligned-bytes
//! converter [`SegmentView::from_segment`] that copies the f32 data
//! once at admission into a 4-byte-aligned buffer, and the
//! [`SegmentCache`](crate::query::vector::cache::SegmentCache)
//! integration so the brute-force kNN hot loop reads aligned bytes
//! via `bytemuck::cast_slice` instead of the originally-decoded
//! `Vec<f32>` indirection. M6 will share the same `Arc<[u8]>`
//! backing for HNSW graphs.
//!
//! ## Why alignment matters
//!
//! [`bytemuck::cast_slice::<u8, f32>`] requires the source slice's
//! pointer to be aligned to `align_of::<f32>() == 4`. A
//! freshly-decoded `Vec<u8>` from the CBOR codec is `u8`-aligned
//! (i.e., any address); casting it to `&[f32]` panics or returns an
//! error depending on the bytemuck function chosen. The fix is to
//! own the bytes through a buffer that's allocated with the right
//! alignment. We achieve that by allocating as `Vec<u32>` (which has
//! the same align as `f32`) and re-interpreting back to bytes.
//!
//! ## storage_side_zero_copy
//!
//! When the RocksDB backend gains the alignment-aware encoder + the
//! partial parser, its [`crate::layer::VectorIndex::get_segment`]
//! will produce `SegmentView` directly without decoding the f32 vec.
//! The surface of this module — `SegmentView::vectors() → &[f32]` —
//! doesn't change; consumers don't need to know which path produced
//! the bytes. The aligned-buffer copy in [`SegmentView::from_segment`]
//! is the v1 stand-in; the v2 path skips it.

use crate::layer::VectorSegment;
use crate::ontology::iri::Iri;
use crate::query::vector::distance::Metric;
use crate::query::vector::hnsw::{HnswBuildConfig, HnswGraph};
use std::sync::Arc;

/// Structural metadata for a segment plus a borrowed view of its
/// aligned vector bytes. Used as the [`SegmentView`] borrow target
/// in the query path.
#[derive(Debug, Clone)]
pub struct VectorSegmentLayout {
    pub model_iri: Iri,
    pub dim: u32,
    pub distance: String,
    pub subjects: Vec<Iri>,
}

impl VectorSegmentLayout {
    pub fn count(&self) -> usize {
        self.subjects.len()
    }
}

/// Cacheable, allocation-stable view of one `(VectorIndex, layer)`
/// segment. Holds the `Arc<[u8]>` backing for the flat
/// `count × dim` f32 payload — the segment cache (M5.6) keeps these
/// alive across query handlers without re-allocating the f32 vec.
///
/// Optionally also holds the segment's [`HnswGraph`] (M6.2). When
/// present, the query path dispatches to HNSW traversal; when
/// absent, it falls back to brute-force per-segment k-NN. The
/// strategy decision (build HNSW or not) lives with the
/// SegmentCache admission helpers per the active VectorIndex's
/// `strategy` slot (§3.1).
///
/// `Send + Sync` because the inner `Arc<[u8]>` is.
#[derive(Debug, Clone)]
pub struct SegmentView {
    layout: Arc<VectorSegmentLayout>,
    /// Raw bytes of the concatenated vector payload, 4-byte aligned
    /// at the pointer so [`bytemuck::cast_slice`] is sound. Length
    /// is always `count * dim * 4`.
    bytes: Arc<[u32]>,
    /// Optional HNSW graph over the segment's vectors. Built at
    /// admission per the active VectorIndex's strategy.
    hnsw: Option<Arc<HnswGraph>>,
}

impl SegmentView {
    /// Convert a freshly-decoded [`VectorSegment`] into an aligned
    /// view. Allocates an aligned buffer and copies the f32 payload
    /// in once; subsequent reads through [`Self::vectors`] are
    /// `bytemuck::cast_slice` over the existing allocation.
    ///
    /// Use this as the SegmentCache admission converter. The v2
    /// storage-side path bypasses this by producing `Arc<[u32]>`
    /// directly from RocksDB; see module docs.
    pub fn from_segment(segment: VectorSegment) -> Self {
        let count = segment.subjects.len();
        let dim = segment.dim as usize;
        debug_assert_eq!(
            segment.vectors.len(),
            count * dim,
            "VectorSegment::vectors must have length subjects.len() * dim"
        );

        // Allocate as Vec<u32> so the backing pointer is 4-byte
        // aligned (= align_of::<f32>()), then write each f32 by
        // value into a u32 lane via `to_bits`. This avoids
        // dereferencing a misaligned u8 slice as f32, which is UB
        // on platforms where strict alignment is required.
        let mut buf: Vec<u32> = Vec::with_capacity(count * dim);
        for &v in &segment.vectors {
            buf.push(v.to_bits());
        }
        // Sanity: pointer alignment.
        debug_assert_eq!(buf.as_ptr() as usize % std::mem::align_of::<f32>(), 0);

        let layout = VectorSegmentLayout {
            model_iri: segment.model_iri,
            dim: segment.dim,
            distance: segment.distance,
            subjects: segment.subjects,
        };
        SegmentView {
            layout: Arc::new(layout),
            bytes: Arc::from(buf.into_boxed_slice()),
            hnsw: None,
        }
    }

    /// Construct directly from owned aligned bytes + layout. The
    /// storage-side zero-copy path uses this constructor when it
    /// produces `Arc<[u32]>` from a single RocksDB `get`.
    /// `bytes.len()` must equal `layout.count() * layout.dim`.
    pub fn from_aligned_bytes(layout: VectorSegmentLayout, bytes: Arc<[u32]>) -> Self {
        debug_assert_eq!(
            bytes.len(),
            layout.count() * layout.dim as usize,
            "aligned-bytes length must equal count * dim"
        );
        SegmentView {
            layout: Arc::new(layout),
            bytes,
            hnsw: None,
        }
    }

    /// Attach a freshly-built HNSW graph to this view. Returns the
    /// modified view (consuming the original) so cache admission
    /// helpers can chain: `SegmentView::from_segment(s).with_hnsw(...)`.
    ///
    /// The graph's `dim` and `count` must match the underlying
    /// vector payload; otherwise the segment's HNSW dispatch path
    /// would return out-of-bounds subject indices.
    pub fn with_hnsw(mut self, graph: Arc<HnswGraph>) -> Self {
        debug_assert_eq!(
            graph.count(),
            self.count(),
            "HNSW graph count must match segment count"
        );
        self.hnsw = Some(graph);
        self
    }

    /// Build an HNSW graph over the segment's vectors and attach it.
    /// Convenience wrapper used by [`Self::admit`] and by tests; the
    /// sweep can also call this directly when it materialises the
    /// segment in-memory.
    pub fn build_and_attach_hnsw(self, metric: Metric, config: HnswBuildConfig) -> Self {
        let graph = HnswGraph::build(self.vectors(), self.dim() as usize, metric, config);
        self.with_hnsw(Arc::new(graph))
    }

    /// Borrowed access to the HNSW graph, if one is attached.
    pub fn hnsw(&self) -> Option<&HnswGraph> {
        self.hnsw.as_deref()
    }

    /// Borrowed access to the structural metadata.
    pub fn layout(&self) -> &VectorSegmentLayout {
        &self.layout
    }

    pub fn model_iri(&self) -> &Iri {
        &self.layout.model_iri
    }
    pub fn dim(&self) -> u32 {
        self.layout.dim
    }
    pub fn distance(&self) -> &str {
        &self.layout.distance
    }
    pub fn subjects(&self) -> &[Iri] {
        &self.layout.subjects
    }
    pub fn count(&self) -> usize {
        self.layout.count()
    }

    /// Flat `&[f32]` view of the concatenated `count × dim` vector
    /// payload. SIMD-ready — see M5.9 distance kernels.
    pub fn vectors(&self) -> &[f32] {
        // bytemuck::cast_slice<u32, f32>: both have the same size
        // (4 bytes) and alignment (4 bytes), and f32 is `Pod`.
        bytemuck::cast_slice::<u32, f32>(&self.bytes)
    }

    /// `&[f32]` for the i-th subject's vector. Equivalent to the
    /// `VectorSegment::vector_at` shape — see [`VectorSegment::vector_at`].
    pub fn vector_at(&self, i: usize) -> &[f32] {
        let dim = self.dim() as usize;
        &self.vectors()[i * dim..(i + 1) * dim]
    }
}

/// Per-segment strategy decision. Mirrors the §3.1 `strategy` slot
/// on a VectorIndex Resource. The `auto` variant carries the
/// threshold the SegmentCache compares the segment's `count`
/// against — default 50 000 per D43 §2.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStrategy {
    Flat,
    Hnsw,
    Auto { threshold: usize },
}

/// Map the VectorIndex Resource's `strategy` IRI (per the
/// `urn:eigenius:core:strategies:*` enum) to a [`SegmentStrategy`].
/// Unknown IRIs default to `Auto` with the v1 threshold — the
/// typechecker's `allows_only` constraint on the slot should make
/// this branch unreachable in practice, but it's the right
/// conservative default for forward compatibility with future
/// strategy variants.
pub fn strategy_from_iri(iri: &Iri) -> SegmentStrategy {
    match iri.as_str() {
        "urn:eigenius:core:strategies:flat" => SegmentStrategy::Flat,
        "urn:eigenius:core:strategies:hnsw" => SegmentStrategy::Hnsw,
        _ => SegmentStrategy::auto_default(),
    }
}

impl SegmentStrategy {
    /// Default `auto` strategy with the D43 §2.4 v1 threshold.
    pub fn auto_default() -> Self {
        Self::Auto { threshold: 50_000 }
    }

    /// Decide whether to build HNSW for a segment of `count`
    /// vectors. `Flat → false`, `Hnsw → true`, `Auto → count >
    /// threshold`. Used by [`admit_segment`] (M6.3) and by tests.
    pub fn should_build_hnsw(self, count: usize) -> bool {
        match self {
            Self::Flat => false,
            Self::Hnsw => true,
            Self::Auto { threshold } => count > threshold,
        }
    }
}

/// SegmentCache admission helper that bundles the strategy
/// decision: builds the [`SegmentView`] from the decoded
/// [`VectorSegment`], optionally building an HNSW graph per
/// `strategy` and `metric` (§2.4 strategy dispatch). M5.6's
/// `fetch_segment` calls this on cache miss.
///
/// When the segment carries a persisted `hnsw_graph_bytes` payload
/// (D43 §2.4 / M6-finish.4), the decoder is consulted instead of
/// rebuilding the graph. Decode failure on what should be a valid
/// graph is silently demoted to a rebuild so a corrupt cached
/// segment doesn't poison query traffic; the build path is the
/// authoritative producer.
pub fn admit_segment(
    segment: VectorSegment,
    metric: Metric,
    strategy: SegmentStrategy,
    hnsw_config: HnswBuildConfig,
) -> SegmentView {
    use crate::query::vector::hnsw_format;
    let count = segment.subjects.len();
    let dim_usize = segment.dim as usize;
    let persisted_graph = segment.hnsw_graph_bytes.clone();
    let view = SegmentView::from_segment(segment);

    if let Some(bytes) = persisted_graph {
        match hnsw_format::decode(&bytes) {
            Ok(layout) if layout.count() == count => {
                let vectors_arc: Arc<[f32]> = Arc::from(view.vectors().to_vec());
                let graph = HnswGraph::from_layout(layout, vectors_arc, dim_usize, metric);
                return view.with_hnsw(Arc::new(graph));
            }
            Ok(_) | Err(_) => {
                // Demote to rebuild — see fn doc.
            }
        }
    }

    if strategy.should_build_hnsw(count) {
        view.build_and_attach_hnsw(metric, hnsw_config)
    } else {
        view
    }
}

/// Sweep-side helper: build an HNSW graph over `vectors` if
/// `strategy` (resolved from the active VectorIndex's `strategy`
/// slot) calls for it, and encode the result via the §2.4 wire
/// format. Returns `None` for the flat path; returns `Some(bytes)`
/// for `hnsw` / `auto`-promoted segments. The bytes are what the
/// sweep hands to [`crate::layer::VectorIndex::extend_layer`] so the
/// graph survives kernel restart.
pub fn build_hnsw_graph_bytes(
    vectors: &[f32],
    dim: usize,
    count: usize,
    metric: Metric,
    strategy: SegmentStrategy,
    hnsw_config: HnswBuildConfig,
) -> Option<Vec<u8>> {
    use crate::query::vector::hnsw_format;
    if !strategy.should_build_hnsw(count) {
        return None;
    }
    let graph = HnswGraph::build(vectors, dim, metric, hnsw_config);
    Some(hnsw_format::encode(graph.layout()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::iri::Iri;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn dummy_segment(n: usize, dim: u32) -> VectorSegment {
        let count = n * dim as usize;
        VectorSegment {
            model_iri: iri("urn:eigenius:embed:test"),
            dim,
            distance: "cosine".into(),
            subjects: (0..n)
                .map(|i| iri(&format!("urn:eigenius:test:s{i}")))
                .collect(),
            vectors: (0..count).map(|i| i as f32 * 0.125).collect(),
            hnsw_graph_bytes: None,
        }
    }

    #[test]
    fn vectors_pointer_is_f32_aligned() {
        let view = SegmentView::from_segment(dummy_segment(3, 4));
        let ptr = view.vectors().as_ptr() as usize;
        assert_eq!(
            ptr % std::mem::align_of::<f32>(),
            0,
            "vectors slice must be f32-aligned for SIMD reads"
        );
    }

    #[test]
    fn vectors_round_trip_via_view() {
        let segment = dummy_segment(2, 5);
        let original = segment.vectors.clone();
        let view = SegmentView::from_segment(segment);
        let viewed: Vec<f32> = view.vectors().to_vec();
        assert_eq!(viewed, original);
    }

    #[test]
    fn vector_at_matches_segment_slice() {
        let segment = dummy_segment(4, 3);
        let from_segment = segment.clone();
        let view = SegmentView::from_segment(segment);
        for i in 0..4 {
            assert_eq!(view.vector_at(i), from_segment.vector_at(i));
        }
    }

    #[test]
    fn subjects_metadata_preserved() {
        let segment = dummy_segment(3, 4);
        let expected_subjects = segment.subjects.clone();
        let expected_model = segment.model_iri.clone();
        let view = SegmentView::from_segment(segment);
        assert_eq!(view.subjects(), expected_subjects.as_slice());
        assert_eq!(view.model_iri(), &expected_model);
        assert_eq!(view.dim(), 4);
        assert_eq!(view.distance(), "cosine");
        assert_eq!(view.count(), 3);
    }

    #[test]
    fn cloning_shares_aligned_storage() {
        // SegmentView's clones must share the `Arc<[u32]>` — that's
        // the cache-friendly property the SegmentCache relies on.
        let view = SegmentView::from_segment(dummy_segment(2, 4));
        let clone = view.clone();
        assert_eq!(
            view.vectors().as_ptr(),
            clone.vectors().as_ptr(),
            "clones must share the same aligned backing"
        );
    }

    #[test]
    fn from_aligned_bytes_round_trips() {
        // The storage-side zero-copy path's entry point: a layout +
        // raw aligned bytes. Verify it produces the same vectors
        // slice as `from_segment` would have.
        let segment = dummy_segment(2, 3);
        let layout = VectorSegmentLayout {
            model_iri: segment.model_iri.clone(),
            dim: segment.dim,
            distance: segment.distance.clone(),
            subjects: segment.subjects.clone(),
        };
        let bytes: Arc<[u32]> = Arc::from(
            segment
                .vectors
                .iter()
                .map(|f| f.to_bits())
                .collect::<Vec<_>>(),
        );
        let view = SegmentView::from_aligned_bytes(layout, bytes);
        assert_eq!(view.vectors().to_vec(), segment.vectors);
    }

    #[test]
    fn realistic_embedding_dim_is_correctly_viewed() {
        // Pin the f32 cast under a realistic embedding size (768
        // dim, 100 vectors → 76,800 floats → 307,200 bytes). Catches
        // regressions on the cast-slice length math.
        let segment = dummy_segment(100, 768);
        let view = SegmentView::from_segment(segment);
        assert_eq!(view.vectors().len(), 100 * 768);
        assert_eq!(view.vector_at(99).len(), 768);
    }

    // ─── HNSW slot (M6.2) ────────────────────────────────────

    #[test]
    fn hnsw_is_none_by_default() {
        let view = SegmentView::from_segment(dummy_segment(8, 4));
        assert!(view.hnsw().is_none(), "fresh SegmentView has no HNSW");
    }

    #[test]
    fn strategy_flat_skips_hnsw_build() {
        let view = admit_segment(
            dummy_segment(100, 4),
            Metric::Cosine,
            SegmentStrategy::Flat,
            HnswBuildConfig::for_segment(100),
        );
        assert!(view.hnsw().is_none());
    }

    #[test]
    fn strategy_hnsw_always_builds() {
        let view = admit_segment(
            dummy_segment(8, 4),
            Metric::Cosine,
            SegmentStrategy::Hnsw,
            HnswBuildConfig::for_segment(8),
        );
        let graph = view.hnsw().expect("HNSW should be built");
        assert_eq!(graph.count(), 8);
    }

    #[test]
    fn strategy_auto_builds_above_threshold_only() {
        let strategy = SegmentStrategy::Auto { threshold: 10 };
        assert!(!strategy.should_build_hnsw(5));
        assert!(!strategy.should_build_hnsw(10));
        assert!(strategy.should_build_hnsw(11));
    }

    #[test]
    fn strategy_auto_default_threshold_matches_design() {
        // §2.4: auto threshold ~50K.
        assert_eq!(
            SegmentStrategy::auto_default(),
            SegmentStrategy::Auto { threshold: 50_000 }
        );
    }

    #[test]
    fn hnsw_attached_graph_count_matches_segment_count() {
        let view = SegmentView::from_segment(dummy_segment(12, 4));
        let view = view.build_and_attach_hnsw(Metric::Cosine, HnswBuildConfig::for_segment(12));
        assert_eq!(view.hnsw().unwrap().count(), 12);
    }

    // ─── Persisted HNSW (M6-finish.4) ───────────────────────────

    /// `build_hnsw_graph_bytes` returns `None` under `Flat` and
    /// returns wire-format bytes that decode under `Hnsw`.
    #[test]
    fn build_hnsw_graph_bytes_respects_strategy() {
        use crate::query::vector::hnsw_format;
        let segment = dummy_segment(8, 4);
        let count = segment.subjects.len();
        let dim = segment.dim as usize;

        let none_bytes = build_hnsw_graph_bytes(
            &segment.vectors,
            dim,
            count,
            Metric::Cosine,
            SegmentStrategy::Flat,
            HnswBuildConfig::for_segment(count),
        );
        assert!(none_bytes.is_none(), "Flat strategy emits no graph");

        let some_bytes = build_hnsw_graph_bytes(
            &segment.vectors,
            dim,
            count,
            Metric::Cosine,
            SegmentStrategy::Hnsw,
            HnswBuildConfig::for_segment(count),
        )
        .expect("Hnsw strategy emits bytes");
        let layout = hnsw_format::decode(&some_bytes).expect("encoded bytes decode");
        assert_eq!(layout.count(), count);
    }

    /// When the segment carries `hnsw_graph_bytes` from storage,
    /// `admit_segment` must skip the rebuild and attach the decoded
    /// graph. The contract observable from outside is: the cache
    /// admission yields a SegmentView whose `hnsw()` is `Some` even
    /// when `strategy = Flat` (the persisted graph beats the
    /// strategy slot when present).
    #[test]
    fn admit_segment_uses_persisted_hnsw_bytes() {
        use crate::query::vector::hnsw_format;
        let segment = dummy_segment(8, 4);
        let dim = segment.dim as usize;
        let count = segment.subjects.len();

        // Build the same graph the sweep would have written.
        let bytes = build_hnsw_graph_bytes(
            &segment.vectors,
            dim,
            count,
            Metric::Cosine,
            SegmentStrategy::Hnsw,
            HnswBuildConfig::for_segment(count),
        )
        .unwrap();

        let with_persisted = VectorSegment {
            hnsw_graph_bytes: Some(bytes.clone()),
            ..segment.clone()
        };

        // strategy=Flat is the load-bearing assertion: persisted
        // bytes win, no rebuild needed.
        let view = admit_segment(
            with_persisted,
            Metric::Cosine,
            SegmentStrategy::Flat,
            HnswBuildConfig::for_segment(count),
        );
        let graph = view.hnsw().expect("persisted bytes attached");
        assert_eq!(graph.count(), count);

        // The decoded graph's topology equals the original wire shape.
        let expected = hnsw_format::decode(&bytes).unwrap();
        assert_eq!(graph.layout(), &expected);
    }

    /// Corrupt `hnsw_graph_bytes` are silently demoted to a rebuild
    /// or a flat view — the read path must not crash query traffic
    /// on a malformed segment.
    #[test]
    fn admit_segment_falls_back_when_persisted_bytes_corrupt() {
        let mut segment = dummy_segment(8, 4);
        segment.hnsw_graph_bytes = Some(vec![0xFF; 8]); // garbage

        // strategy=Hnsw → fall back to rebuild
        let view = admit_segment(
            segment,
            Metric::Cosine,
            SegmentStrategy::Hnsw,
            HnswBuildConfig::for_segment(8),
        );
        assert!(view.hnsw().is_some(), "corrupt bytes demote to rebuild");
    }
}
