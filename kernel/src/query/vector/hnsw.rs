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

//! D43 §2.4 / M6.1 — HNSW adapter (post M6-finish.3).
//!
//! Thin search-facing wrapper around the in-tree HNSW algorithm
//! ([`crate::query::vector::hnsw_core`]) and the §2.4 wire shape
//! ([`crate::query::vector::hnsw_format::HnswGraph`]). The rest of
//! the kernel — the SegmentView's `hnsw` slot (M6.2), the search
//! orchestrator's HNSW dispatch (M6.4), the strategy-driven cache
//! admission (M6.3) — consumes one [`HnswGraph`] handle and doesn't
//! care which library produced it. After M6-finish.3 the answer is
//! "no library — our own algorithm."
//!
//! ## Memory layout
//!
//! The handle owns:
//!
//! - the wire-format `HnswGraph` ([`crate::query::vector::hnsw_format::HnswGraph`])
//!   carrying topology only (entry_point, max_level, per-node level + adjacency);
//! - an `Arc<[f32]>` copy of the vectors keyed by the same node ids
//!   the graph references. The search path needs them for per-pair
//!   distance computation as it traverses the graph.
//! - the [`Metric`] used at build time so the search emits scores
//!   in the kernel's "higher = better" similarity convention.
//!
//! The duplication with [`crate::query::vector::segment::SegmentView`]'s
//! own aligned vector backing is intentional for v1 — keeping the
//! HNSW handle self-contained avoids a lifetime tangle. A v2 sharing
//! the SegmentView's `Arc<[u32]>` backing is contained future work.

use crate::query::vector::distance::Metric;
use crate::query::vector::hnsw_core::{
    build as core_build, search as core_search, CoreBuildConfig,
};
use crate::query::vector::hnsw_format::HnswGraph as GraphLayout;
use std::sync::Arc;

/// Build-time parameters for an HNSW graph. The active VectorIndex
/// Resource carries these in `hnsw_m` and `hnsw_ef_construction`
/// slots (D43 §3.1); the sweep reads them into this struct before
/// calling [`HnswGraph::build`]. The `max_elements` knob is the
/// caller's segment count — it sizes the internal buffers but is
/// otherwise informational under the in-tree algorithm (the v1
/// vendored builder grows freely; the field is preserved on the
/// public surface so the existing sweep code doesn't need a
/// signature change).
#[derive(Debug, Clone, Copy)]
pub struct HnswBuildConfig {
    pub m: usize,
    pub ef_construction: usize,
    pub max_elements: usize,
}

impl HnswBuildConfig {
    /// Convenience: derive max_elements from the segment size and
    /// fill in v1 defaults for the rest. Sweep callers use this
    /// when the active VectorIndex doesn't declare custom values.
    pub fn for_segment(count: usize) -> Self {
        Self {
            m: 16,
            ef_construction: 200,
            max_elements: count.max(16),
        }
    }
}

/// Search-capable HNSW handle: the graph topology + the vectors it
/// indexes + the metric / dim metadata the search loop needs.
///
/// `Send + Sync` because all the inner state is `Arc`-backed and
/// the algorithm is purely functional after build (no shared
/// mutable state during search). The earlier `hnsw_rs`-wrapped
/// version needed a `Mutex`; the in-tree algorithm doesn't.
pub struct HnswGraph {
    layout: Arc<GraphLayout>,
    vectors: Arc<[f32]>,
    dim: usize,
    metric: Metric,
}

impl std::fmt::Debug for HnswGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HnswGraph")
            .field("metric", &self.metric)
            .field("dim", &self.dim)
            .field("count", &self.count())
            .field("max_level", &self.layout.max_level)
            .field("entry_point", &self.layout.entry_point)
            .finish()
    }
}

impl HnswGraph {
    /// Build an HNSW over `vectors` (flat `count × dim` slice).
    /// `subjects[i]` corresponds to `vectors[i*dim..(i+1)*dim]` and
    /// is identified by index `i` in the returned graph; the
    /// caller's `Vec<Iri>` is the index→IRI mapping.
    pub fn build(vectors: &[f32], dim: usize, metric: Metric, config: HnswBuildConfig) -> Self {
        debug_assert!(dim > 0);
        debug_assert_eq!(vectors.len() % dim, 0);
        let core_config = CoreBuildConfig {
            m: config.m,
            ef_construction: config.ef_construction,
            // The post-Load sweep doesn't pick a seed today; using
            // a fixed value keeps test reproducibility across
            // runs of `cargo test`. A future configurable seed
            // (per-`(layer, Index)` for hash-of-content reuse)
            // slots in here without touching the wire format.
            seed: 0xE16E_4143_4F52_4500, // "EIGE_ACOR_E" pun seed
        };
        let layout = core_build(vectors, dim, metric, core_config);
        Self {
            layout: Arc::new(layout),
            vectors: Arc::from(vectors.to_vec()),
            dim,
            metric,
        }
    }

    /// Reconstitute a search handle from a previously-encoded wire-
    /// format [`GraphLayout`] + its associated vectors. The
    /// RocksDB-backed reload path (M6-finish.4) calls this after
    /// pulling the `hnsw_graph` bstr out of the segment CBOR and
    /// decoding it via [`crate::query::vector::hnsw_format::decode`].
    pub fn from_layout(
        layout: GraphLayout,
        vectors: Arc<[f32]>,
        dim: usize,
        metric: Metric,
    ) -> Self {
        debug_assert!(dim > 0);
        debug_assert_eq!(vectors.len() % dim, 0);
        debug_assert_eq!(vectors.len() / dim, layout.count());
        Self {
            layout: Arc::new(layout),
            vectors,
            dim,
            metric,
        }
    }

    /// Borrow the wire-format graph (for the M6-finish.4 persist
    /// path).
    pub fn layout(&self) -> &GraphLayout {
        &self.layout
    }

    /// Search for the top-`k` nearest neighbours of `query` with
    /// per-search exploration depth `ef`. Returns `(node_index,
    /// similarity)` pairs in descending similarity order (under the
    /// "higher = better" convention shared with the brute-force
    /// path — see [`Metric::similarity`]).
    ///
    /// `ef` controls recall: typical operating points are `ef = k*2`
    /// for ~95 % recall, `ef = k*4` for ~99 %. The §3.4 default
    /// `max(k*4, 64)` is the caller's responsibility.
    pub fn search(&self, query: &[f32], k: usize, ef: usize) -> Vec<(usize, f32)> {
        debug_assert_eq!(query.len(), self.dim);
        core_search(
            self.layout.as_ref(),
            &self.vectors,
            self.dim,
            self.metric,
            query,
            k,
            ef,
        )
        .into_iter()
        .map(|(id, sim)| (id as usize, sim))
        .collect()
    }

    /// Number of indexed vectors. Used by tests and by the recall
    /// measurement (M6.6) which reports per-segment coverage.
    pub fn count(&self) -> usize {
        self.layout.count()
    }

    /// Distance metric this graph was built under.
    pub fn metric(&self) -> Metric {
        self.metric
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_vec_2d(angle_deg: f32) -> Vec<f32> {
        let a = angle_deg.to_radians();
        vec![a.cos(), a.sin()]
    }

    fn known_2d_corpus() -> (Vec<f32>, usize) {
        let mut data: Vec<f32> = Vec::new();
        for &deg in &[0.0f32, 90.0, 180.0, 270.0] {
            data.extend(unit_vec_2d(deg));
        }
        (data, 2)
    }

    #[test]
    fn build_then_search_finds_self_for_unit_vectors() {
        let (data, dim) = known_2d_corpus();
        let graph = HnswGraph::build(&data, dim, Metric::Cosine, HnswBuildConfig::for_segment(4));
        for i in 0..4 {
            let q = &data[i * dim..(i + 1) * dim];
            let hits = graph.search(q, 1, 16);
            assert_eq!(hits.len(), 1, "i={i} should return one hit");
            assert_eq!(hits[0].0, i, "i={i} should be top-1");
            assert!(
                (hits[0].1 - 1.0).abs() < 1e-5,
                "self-similarity ≈ 1 at i={i}; got {}",
                hits[0].1
            );
        }
    }

    #[test]
    fn search_returns_hits_in_descending_similarity_order() {
        let mut data: Vec<f32> = Vec::new();
        for k in 0..8 {
            let deg = (k as f32) * 45.0;
            data.extend(unit_vec_2d(deg));
        }
        let graph = HnswGraph::build(&data, 2, Metric::Cosine, HnswBuildConfig::for_segment(8));
        let q = unit_vec_2d(45.0);
        let hits = graph.search(&q, 4, 32);
        assert!(!hits.is_empty());
        for w in hits.windows(2) {
            assert!(
                w[0].1 >= w[1].1,
                "hits must be descending by similarity; got {} then {}",
                w[0].1,
                w[1].1
            );
        }
        assert_eq!(hits[0].0, 1);
        assert!((hits[0].1 - 1.0).abs() < 1e-4);
    }

    #[test]
    fn larger_corpus_returns_self_for_self_query() {
        use sha2::{Digest, Sha256};
        let dim = 16;
        let count = 200;
        let mut data: Vec<f32> = Vec::with_capacity(count * dim);
        for i in 0..count {
            let mut h = Sha256::new();
            h.update((i as u64).to_le_bytes());
            let digest = h.finalize();
            for j in 0..dim {
                let chunk_idx = j % 8;
                let bytes = [
                    digest[chunk_idx * 4],
                    digest[chunk_idx * 4 + 1],
                    digest[chunk_idx * 4 + 2],
                    digest[chunk_idx * 4 + 3],
                ];
                let u = u32::from_le_bytes(bytes);
                let scaled = (u as f32 / u32::MAX as f32) * 2.0 - 1.0;
                data.push(scaled);
            }
        }
        let graph = HnswGraph::build(
            &data,
            dim,
            Metric::Cosine,
            HnswBuildConfig {
                m: 16,
                ef_construction: 100,
                max_elements: count,
            },
        );

        let q_offset = 42 * dim;
        let q = &data[q_offset..q_offset + dim];
        let hits = graph.search(q, 5, 64);
        assert!(!hits.is_empty(), "should return hits");
        assert_eq!(hits[0].0, 42, "top-1 should be self");
        assert!(
            (hits[0].1 - 1.0).abs() < 1e-4,
            "self-similarity ≈ 1; got {}",
            hits[0].1
        );
    }

    #[test]
    fn search_respects_k_truncation() {
        let dim = 4;
        let count = 50;
        let mut data: Vec<f32> = Vec::with_capacity(count * dim);
        for i in 0..count {
            for j in 0..dim {
                data.push((i * 7 + j) as f32 * 0.01);
            }
        }
        let graph = HnswGraph::build(&data, dim, Metric::L2, HnswBuildConfig::for_segment(count));
        let q = &data[0..dim];
        let hits = graph.search(q, 3, 30);
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn count_matches_inserted_size() {
        let (data, dim) = known_2d_corpus();
        let graph = HnswGraph::build(&data, dim, Metric::Cosine, HnswBuildConfig::for_segment(4));
        assert_eq!(graph.count(), 4);
    }

    #[test]
    fn metric_accessor_returns_build_metric() {
        let (data, dim) = known_2d_corpus();
        let graph = HnswGraph::build(&data, dim, Metric::L2, HnswBuildConfig::for_segment(4));
        assert_eq!(graph.metric(), Metric::L2);
    }

    #[test]
    fn from_layout_round_trips_via_wire_format() {
        // Build, encode, decode, reload via from_layout; the
        // reloaded graph must search to the same self-query top-1
        // as the original.
        use crate::query::vector::hnsw_format::{decode, encode};
        let (data, dim) = known_2d_corpus();
        let g = HnswGraph::build(&data, dim, Metric::Cosine, HnswBuildConfig::for_segment(4));
        let bytes = encode(g.layout());
        let layout = decode(&bytes).expect("decode");
        let reloaded = HnswGraph::from_layout(layout, Arc::from(data.clone()), dim, Metric::Cosine);
        let q = &data[0..dim];
        let original_hits = g.search(q, 1, 16);
        let reloaded_hits = reloaded.search(q, 1, 16);
        assert_eq!(original_hits[0].0, reloaded_hits[0].0);
        assert!((original_hits[0].1 - reloaded_hits[0].1).abs() < 1e-6);
    }
}
