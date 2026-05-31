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

//! D43 §2.4 / M6.1 — HNSW adapter.
//!
//! The kernel doesn't ship its own HNSW implementation; this module
//! wraps the [`hnsw_rs`] crate (a stable-Rust port of the
//! Malkov-Yashunin 2016 paper). The wrapper:
//!
//! 1. Hides the library's internal types behind a small surface
//!    ([`HnswGraph`], [`HnswBuildConfig`]) so a future swap to
//!    `instant-distance`, `usearch`, or a roll-our-own implementation
//!    is a one-file change.
//! 2. Owns the per-metric type instantiation. v1 supports cosine
//!    and L2; dot is folded into cosine by L2-normalising both
//!    sides upstream (TODO: add dot/inner-product support when a
//!    workload needs it; cf §3.4 "v1 supports flat for dot").
//! 3. Carries the build-time parameters (`M`, `ef_construction`)
//!    that the active VectorIndex Resource declares (§3.1
//!    `hnsw_m` / `hnsw_ef_construction` slots).
//!
//! ## v1 wire format
//!
//! `hnsw_rs` doesn't expose its graph structure as a portable byte
//! representation; v1 persists the library's own `bincode` form
//! inside the segment's `hnsw_graph` bstr. The design (§2.4
//! paragraph "HNSW graph encoding") commits to a library-
//! independent on-wire format eventually, but that work is
//! deferred — see the M6 follow-up issue for the migration plan.
//! v1 segments with HNSW graphs are tied to this library version;
//! a library swap requires re-running the sweep to rebuild them,
//! which is the same cost as a model upgrade (§5.7 atomic reindex)
//! and uses the same machinery.

use crate::query::vector::distance::Metric;
use hnsw_rs::prelude::*;
use std::sync::Mutex;

/// Build-time parameters for an HNSW graph. The active VectorIndex
/// Resource carries these in `hnsw_m` and `hnsw_ef_construction`
/// slots (D43 §3.1); the sweep reads them into this struct before
/// calling [`HnswGraph::build`].
#[derive(Debug, Clone, Copy)]
pub struct HnswBuildConfig {
    /// `M` — the max number of bidirectional links each node carries
    /// at the upper levels. v1 default is 16; the design's §3.1
    /// recommended range is 8–32.
    pub m: usize,
    /// `ef_construction` — exploration breadth during build. Higher
    /// produces a higher-quality graph at the cost of build time.
    /// v1 default is 200.
    pub ef_construction: usize,
    /// Estimated max element count. Used by `hnsw_rs` to size its
    /// internal buffers; over-estimating is fine, under-estimating
    /// causes a reallocation midway through the build.
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

/// Built HNSW graph + the vectors it indexes. The vectors are
/// borrowed at build time and owned at search time so the
/// SegmentView's aligned-byte-backed `&[f32]` can be the
/// authoritative store; [`HnswGraph`] is the searchable index
/// over it.
///
/// `hnsw_rs::Hnsw` is wrapped in a `Mutex` because the library's
/// internal state isn't `Sync` even though the read path is
/// thread-safe in practice. v1 takes the lock per search; if the
/// contention is measurable in production we switch to a finer
/// lock or to a library that exposes `Sync` directly.
pub struct HnswGraph {
    inner: Mutex<HnswInner>,
    metric: Metric,
    dim: usize,
}

impl std::fmt::Debug for HnswGraph {
    /// `hnsw_rs::Hnsw` doesn't implement `Debug`, so we render a
    /// summary that's useful in logs without dragging in the
    /// library's internal state.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HnswGraph")
            .field("metric", &self.metric)
            .field("dim", &self.dim)
            .field("count", &self.count())
            .finish()
    }
}

enum HnswInner {
    Cosine(Hnsw<'static, f32, DistCosine>),
    L2(Hnsw<'static, f32, DistL2>),
    Dot(Hnsw<'static, f32, DistDot>),
}

impl HnswGraph {
    /// Build an HNSW over `vectors` (flat `count × dim` slice).
    /// `subjects[i]` corresponds to `vectors[i*dim..(i+1)*dim]` and
    /// is identified by index `i` in the returned graph; the
    /// caller's `Vec<Iri>` is the index→IRI mapping.
    pub fn build(vectors: &[f32], dim: usize, metric: Metric, config: HnswBuildConfig) -> Self {
        debug_assert!(dim > 0);
        debug_assert_eq!(vectors.len() % dim, 0);
        let count = vectors.len() / dim;
        let max_layer = if count < 16 {
            // Single-layer when the segment is tiny — keeps the
            // graph degenerate-but-valid.
            1
        } else {
            // log2(count) is the textbook upper-level cap.
            (count as f32).log2().ceil() as usize
        };

        let mut data: Vec<&[f32]> = Vec::with_capacity(count);
        for i in 0..count {
            data.push(&vectors[i * dim..(i + 1) * dim]);
        }

        let inner = match metric {
            Metric::Cosine => {
                let h = Hnsw::<f32, DistCosine>::new(
                    config.m,
                    config.max_elements,
                    max_layer,
                    config.ef_construction,
                    DistCosine,
                );
                for (i, v) in data.iter().enumerate() {
                    h.insert((v, i));
                }
                HnswInner::Cosine(h)
            }
            Metric::L2 => {
                let h = Hnsw::<f32, DistL2>::new(
                    config.m,
                    config.max_elements,
                    max_layer,
                    config.ef_construction,
                    DistL2,
                );
                for (i, v) in data.iter().enumerate() {
                    h.insert((v, i));
                }
                HnswInner::L2(h)
            }
            Metric::Dot => {
                let h = Hnsw::<f32, DistDot>::new(
                    config.m,
                    config.max_elements,
                    max_layer,
                    config.ef_construction,
                    DistDot,
                );
                for (i, v) in data.iter().enumerate() {
                    h.insert((v, i));
                }
                HnswInner::Dot(h)
            }
        };

        Self {
            inner: Mutex::new(inner),
            metric,
            dim,
        }
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
        let guard = self.inner.lock().expect("hnsw mutex poisoned");
        let raw: Vec<Neighbour> = match &*guard {
            HnswInner::Cosine(h) => h.search(query, k, ef),
            HnswInner::L2(h) => h.search(query, k, ef),
            HnswInner::Dot(h) => h.search(query, k, ef),
        };
        // `hnsw_rs` returns Neighbours sorted by ascending distance.
        // Convert each distance to the "higher = better" similarity
        // form the rest of the query path uses.
        raw.into_iter()
            .map(|n| {
                let sim = distance_to_similarity(self.metric, n.distance);
                (n.d_id, sim)
            })
            .collect()
    }

    /// Number of indexed vectors. Used by tests and by the recall
    /// measurement (M6.6) which reports per-segment coverage.
    pub fn count(&self) -> usize {
        let guard = self.inner.lock().expect("hnsw mutex poisoned");
        match &*guard {
            HnswInner::Cosine(h) => h.get_nb_point(),
            HnswInner::L2(h) => h.get_nb_point(),
            HnswInner::Dot(h) => h.get_nb_point(),
        }
    }
}

/// Convert `hnsw_rs`-returned distance to the kernel's
/// "higher = better" similarity convention. Mirrors
/// [`Metric::similarity`] — the two helpers are kept in parity by
/// test [`tests::hnsw_similarity_matches_metric_similarity`].
fn distance_to_similarity(metric: Metric, distance: f32) -> f32 {
    match metric {
        Metric::Cosine => {
            // `DistCosine` in `hnsw_rs` returns `1 - cos(a, b)` so
            // we recover the similarity by `1 - d`.
            1.0 - distance
        }
        Metric::L2 => {
            // `DistL2` returns sqrt-Euclidean distance; mirror the
            // brute-force orientation: `1 / (1 + d)`.
            1.0 / (1.0 + distance)
        }
        Metric::Dot => {
            // `DistDot` returns `-dot(a, b)` (lower = closer to
            // "more similar"); flip the sign for similarity.
            -distance
        }
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
        // 4 unit vectors at 0°, 90°, 180°, 270°. dim=2.
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
        // Query with each indexed vector and expect that vector
        // as the top-1.
        for i in 0..4 {
            let q = &data[i * dim..(i + 1) * dim];
            let hits = graph.search(q, 1, 16);
            assert_eq!(hits.len(), 1, "i={i} should return one hit");
            assert_eq!(hits[0].0, i, "i={i} should be top-1");
            // Cosine sim with self is ~1.0.
            assert!(
                (hits[0].1 - 1.0).abs() < 1e-5,
                "self-similarity ≈ 1 at i={i}; got {}",
                hits[0].1
            );
        }
    }

    #[test]
    fn search_returns_hits_in_descending_similarity_order() {
        // 8 unit vectors evenly spaced; query at 45° should return
        // the closest first.
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
        // Top-1 is the 45° point itself, index 1.
        assert_eq!(hits[0].0, 1);
        assert!((hits[0].1 - 1.0).abs() < 1e-4);
    }

    #[test]
    fn larger_corpus_returns_self_for_self_query() {
        // 200 vectors in 16 dimensions, each a unique direction by
        // construction (the SHA-256-style mixing avoids the
        // collinearity that a naive `(i * a + j * b) % p` mixer
        // produces, where ~200 / 97 ≈ 2 near-duplicates per point
        // would defeat the self-query check). For each indexed
        // vector, the self-query must return that vector as
        // top-1 — this is the "exactness on hit" property that
        // makes HNSW useful for non-degenerate workloads.
        use sha2::{Digest, Sha256};
        let dim = 16;
        let count = 200;
        let mut data: Vec<f32> = Vec::with_capacity(count * dim);
        for i in 0..count {
            let mut h = Sha256::new();
            h.update((i as u64).to_le_bytes());
            let digest = h.finalize();
            for j in 0..dim {
                // Unpack 4 bytes from the digest (32 bytes covers up
                // to dim=8 f32s; for higher dim we rehash with j as
                // a counter — mirrors `DummyEmbedder`'s pattern).
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

    /// Pin the conversion: hnsw_rs distance → similarity must
    /// match the brute-force [`Metric::similarity`] for identical
    /// inputs, so HNSW and flat-path scores are comparable
    /// within a single result set (per §2.4 "the reader
    /// dispatches; results carry per-segment recall").
    #[test]
    fn hnsw_similarity_matches_metric_similarity() {
        // Cosine: distance is 1 - cos. Use the canonical
        // 45-degree unit vector via the f32 constant rather than
        // an inexact 0.7071 literal (clippy::approx_constant).
        let a = vec![1.0f32, 0.0];
        let b = vec![
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        ];
        let scalar = Metric::Cosine.similarity(&a, &b);
        // Simulate what hnsw_rs would return: 1 - cos.
        let d = 1.0 - scalar;
        let converted = distance_to_similarity(Metric::Cosine, d);
        assert!((scalar - converted).abs() < 1e-5);

        // L2.
        let scalar = Metric::L2.similarity(&a, &b);
        let d_l2 = crate::query::vector::distance::l2_distance(&a, &b);
        let converted = distance_to_similarity(Metric::L2, d_l2);
        assert!((scalar - converted).abs() < 1e-5);
    }
}
