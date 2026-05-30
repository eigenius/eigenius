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

//! D43 §3.4 / M5 — distance/similarity kernels for vector retrieval.
//!
//! Three metrics, mapping to the `core:DistanceMetric` Resource
//! instances [`urn:eigenius:core:distances:cosine`],
//! [`urn:eigenius:core:distances:l2`], and
//! [`urn:eigenius:core:distances:dot`]. Implementations are
//! scalar-only in v1; SIMD (`std::simd`, AVX-2, NEON) is the M5
//! follow-up — the trait surface here doesn't change.
//!
//! ## Similarity convention
//!
//! Every metric in this module is exposed in a **"higher = better"**
//! orientation so the top-K heap and the `VECTOR_SIM` return value
//! have one consistent direction. For cosine and dot, that's
//! natural — both are already similarity scores. For L2, which is
//! a distance (lower = closer), the [`Metric::similarity`] form
//! returns `1 / (1 + d)` so that two identical vectors score `1.0`
//! and the score monotonically decreases as the vectors diverge.
//! That preserves "higher = better" without introducing negatives
//! and is robust to L2 magnitudes that would otherwise overflow a
//! naive `-d` form into a wide negative range.
//!
//! Raw distances are still available via [`Metric::distance`] when
//! a caller specifically wants the L2 magnitude (e.g. for debug
//! traces); use [`Metric::similarity`] for ranking.

use std::cmp::Ordering;

/// The three v1-supported metrics. Mirrors the
/// `urn:eigenius:core:DistanceMetric` Resource instances declared
/// in the core ontology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Metric {
    /// Cosine similarity: `dot(a,b) / (||a|| * ||b||)`. Range
    /// `[-1.0, 1.0]`. Output is already a similarity.
    Cosine,
    /// Euclidean (L2) distance: `sqrt(sum((a_i - b_i)^2))`. Range
    /// `[0, ∞)`. Reported as similarity via the `1 / (1 + d)`
    /// monotone transform; the unmodified distance is available
    /// through [`Self::distance`].
    L2,
    /// Inner product: `sum(a_i * b_i)`. No bounds — magnitude
    /// depends on input scale. Useful when vectors are L2-normalised
    /// at index time (then it equals cosine) or when the embedder
    /// emits explicit "importance" magnitudes.
    Dot,
}

impl Metric {
    /// Parse a `core:DistanceMetric` short_name (`"cosine"`, `"l2"`,
    /// `"dot"`) or the full IRI form. Returns `None` for any other
    /// string — the typechecker's responsibility, but the parse
    /// surface is exposed here so the storage layer can validate
    /// segments without taking a dependency on `well_known`.
    pub fn from_short_name(s: &str) -> Option<Self> {
        match s {
            "cosine" | "urn:eigenius:core:distances:cosine" => Some(Self::Cosine),
            "l2" | "urn:eigenius:core:distances:l2" => Some(Self::L2),
            "dot" | "urn:eigenius:core:distances:dot" => Some(Self::Dot),
            _ => None,
        }
    }

    /// Short name (lowercase, no namespace) for storage / logging.
    pub fn short_name(self) -> &'static str {
        match self {
            Self::Cosine => "cosine",
            Self::L2 => "l2",
            Self::Dot => "dot",
        }
    }

    /// Compute the raw distance/dissimilarity between `a` and `b`.
    /// Use [`Self::similarity`] for ranking — see module docs.
    ///
    /// Panics if `a.len() != b.len()`. Length parity is the caller's
    /// invariant; the typechecker enforces it at parse time so a
    /// panic here is a programming error (segment with the wrong
    /// dim, or query vector that bypassed type checking).
    pub fn distance(self, a: &[f32], b: &[f32]) -> f32 {
        debug_assert_eq!(a.len(), b.len(), "distance: vectors must be equal length");
        match self {
            Self::Cosine => {
                // Cosine "distance" = 1 - similarity. Defined here so
                // both paths share the same dot/norm pass.
                1.0 - cosine_similarity(a, b)
            }
            Self::L2 => l2_distance(a, b),
            Self::Dot => -dot_product(a, b),
        }
    }

    /// Compute the similarity score between `a` and `b` under
    /// the "higher = better" convention. The output is comparable
    /// across rows but not across metrics (cosine and L2-similarity
    /// have different ranges).
    ///
    /// Panics if `a.len() != b.len()` (see [`Self::distance`]).
    pub fn similarity(self, a: &[f32], b: &[f32]) -> f32 {
        debug_assert_eq!(a.len(), b.len(), "similarity: vectors must be equal length");
        match self {
            Self::Cosine => cosine_similarity(a, b),
            Self::L2 => {
                let d = l2_distance(a, b);
                // 1 / (1 + d) — monotonically decreasing in d,
                // bounded in (0, 1], hits 1.0 for identical vectors.
                1.0 / (1.0 + d)
            }
            Self::Dot => dot_product(a, b),
        }
    }
}

/// SIMD chunk size — `f32x8` (256-bit AVX-2, NEON×2 lanes
/// double-packed, or SSE2 emulation on x86 pre-AVX). The crate
/// auto-selects the best implementation at compile time per
/// `target_feature`; we expose only the lane count as a tuning
/// surface here.
const LANES: usize = 8;

/// Sum-of-lanes helper. `wide`'s `f32x8` doesn't expose a
/// branch-free horizontal-add intrinsic; the small fold is fine on
/// the perf-sensitive paths because it runs once per call rather
/// than once per vector element.
#[inline]
fn horizontal_sum(v: wide::f32x8) -> f32 {
    let arr = v.to_array();
    arr.iter().sum()
}

/// Cosine similarity, vectorised. Walks the inputs in `LANES`-sized
/// chunks accumulating dot product and per-side squared norms
/// in `f32x8` lanes; the scalar tail handles the residue. Returns
/// `0.0` if either vector has zero norm — see the M5.1 docs for
/// the "no signal" rationale.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut dot = wide::f32x8::ZERO;
    let mut na = wide::f32x8::ZERO;
    let mut nb = wide::f32x8::ZERO;

    let len = a.len();
    let chunks = len / LANES;
    for i in 0..chunks {
        let off = i * LANES;
        let av = wide::f32x8::new(a[off..off + LANES].try_into().unwrap());
        let bv = wide::f32x8::new(b[off..off + LANES].try_into().unwrap());
        dot += av * bv;
        na += av * av;
        nb += bv * bv;
    }

    let mut dot_s = horizontal_sum(dot);
    let mut na_s = horizontal_sum(na);
    let mut nb_s = horizontal_sum(nb);
    for i in (chunks * LANES)..len {
        dot_s += a[i] * b[i];
        na_s += a[i] * a[i];
        nb_s += b[i] * b[i];
    }

    let denom = na_s.sqrt() * nb_s.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot_s / denom
    }
}

/// Squared L2 distance. Same SIMD chunking pattern as
/// [`cosine_similarity`]. Useful internally; callers usually want
/// [`l2_distance`].
pub fn l2_distance_squared(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = wide::f32x8::ZERO;

    let len = a.len();
    let chunks = len / LANES;
    for i in 0..chunks {
        let off = i * LANES;
        let av = wide::f32x8::new(a[off..off + LANES].try_into().unwrap());
        let bv = wide::f32x8::new(b[off..off + LANES].try_into().unwrap());
        let d = av - bv;
        acc += d * d;
    }

    let mut sum = horizontal_sum(acc);
    for i in (chunks * LANES)..len {
        let d = a[i] - b[i];
        sum += d * d;
    }
    sum
}

/// Euclidean (L2) distance.
pub fn l2_distance(a: &[f32], b: &[f32]) -> f32 {
    l2_distance_squared(a, b).sqrt()
}

/// Inner product, vectorised.
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = wide::f32x8::ZERO;

    let len = a.len();
    let chunks = len / LANES;
    for i in 0..chunks {
        let off = i * LANES;
        let av = wide::f32x8::new(a[off..off + LANES].try_into().unwrap());
        let bv = wide::f32x8::new(b[off..off + LANES].try_into().unwrap());
        acc += av * bv;
    }

    let mut sum = horizontal_sum(acc);
    for i in (chunks * LANES)..len {
        sum += a[i] * b[i];
    }
    sum
}

/// Total ordering on `f32` similarity scores. `NaN` is treated as
/// the smallest possible value so it can't poison the top-K heap.
/// Used by the brute-force search to push the lowest-scoring
/// candidate out as new ones arrive.
pub fn compare_similarity(a: f32, b: f32) -> Ordering {
    match a.partial_cmp(&b) {
        Some(o) => o,
        None => {
            // One of the two is NaN; treat NaN as less-than for the
            // heap's purposes. If both are NaN they compare equal.
            match (a.is_nan(), b.is_nan()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => Ordering::Equal, // unreachable
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn metric_round_trips_short_name() {
        for &m in &[Metric::Cosine, Metric::L2, Metric::Dot] {
            assert_eq!(Metric::from_short_name(m.short_name()), Some(m));
        }
        // Full-IRI forms are accepted too.
        assert_eq!(
            Metric::from_short_name("urn:eigenius:core:distances:cosine"),
            Some(Metric::Cosine)
        );
        assert_eq!(
            Metric::from_short_name("urn:eigenius:core:distances:l2"),
            Some(Metric::L2)
        );
        assert_eq!(
            Metric::from_short_name("urn:eigenius:core:distances:dot"),
            Some(Metric::Dot)
        );
        assert_eq!(Metric::from_short_name("manhattan"), None);
    }

    // ─── Cosine ─────────────────────────────────────────────────

    #[test]
    fn cosine_identical_vectors_is_one() {
        let v = vec![1.0f32, 2.0, 3.0];
        assert!(close(cosine_similarity(&v, &v), 1.0, 1e-6));
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];
        assert!(close(cosine_similarity(&a, &b), 0.0, 1e-6));
    }

    #[test]
    fn cosine_opposite_is_minus_one() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![-1.0f32, -2.0, -3.0];
        assert!(close(cosine_similarity(&a, &b), -1.0, 1e-6));
    }

    #[test]
    fn cosine_known_value() {
        // a=(1,2,3), b=(2,3,4): dot=2+6+12=20; |a|=sqrt(14); |b|=sqrt(29)
        // cos = 20 / sqrt(14 * 29) = 20 / sqrt(406) ≈ 0.9925833...
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![2.0f32, 3.0, 4.0];
        let expected = 20.0 / (14.0f32 * 29.0).sqrt();
        assert!(close(cosine_similarity(&a, &b), expected, 1e-6));
    }

    #[test]
    fn cosine_zero_vector_returns_zero_not_nan() {
        let zero = vec![0.0f32, 0.0, 0.0];
        let v = vec![1.0f32, 2.0, 3.0];
        let s = cosine_similarity(&zero, &v);
        assert!(!s.is_nan(), "should not return NaN for zero norm");
        assert_eq!(s, 0.0);
    }

    // ─── L2 ─────────────────────────────────────────────────────

    #[test]
    fn l2_identical_is_zero() {
        let v = vec![1.0f32, 2.0, 3.0];
        assert_eq!(l2_distance(&v, &v), 0.0);
    }

    #[test]
    fn l2_known_value() {
        // a=(1,2), b=(4,6): d=(3,4), |d|=5
        let a = vec![1.0f32, 2.0];
        let b = vec![4.0f32, 6.0];
        assert!(close(l2_distance(&a, &b), 5.0, 1e-6));
    }

    #[test]
    fn l2_squared_matches_squared_l2() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![4.0f32, 0.0, -1.0];
        let l2 = l2_distance(&a, &b);
        assert!(close(l2_distance_squared(&a, &b), l2 * l2, 1e-4));
    }

    #[test]
    fn l2_similarity_one_for_identical() {
        let v = vec![1.0f32, 2.0, 3.0];
        assert_eq!(Metric::L2.similarity(&v, &v), 1.0);
    }

    #[test]
    fn l2_similarity_decreases_with_distance() {
        let a = vec![0.0f32, 0.0];
        let near = vec![0.1f32, 0.1];
        let far = vec![10.0f32, 10.0];
        let s_near = Metric::L2.similarity(&a, &near);
        let s_far = Metric::L2.similarity(&a, &far);
        assert!(
            s_near > s_far,
            "L2-similarity should be higher for closer vectors: near={s_near} far={s_far}"
        );
    }

    // ─── Dot ────────────────────────────────────────────────────

    #[test]
    fn dot_basic() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![4.0f32, 5.0, 6.0];
        // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
        assert!(close(dot_product(&a, &b), 32.0, 1e-6));
    }

    #[test]
    fn dot_similarity_equals_dot() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![4.0f32, 5.0, 6.0];
        assert!(close(
            Metric::Dot.similarity(&a, &b),
            dot_product(&a, &b),
            1e-6
        ));
    }

    // ─── Distance / similarity duality ────────────────────────

    #[test]
    fn cosine_distance_is_one_minus_similarity() {
        let a = vec![1.0f32, 0.5, -0.3];
        let b = vec![0.2f32, 0.4, 0.1];
        let sim = Metric::Cosine.similarity(&a, &b);
        let dist = Metric::Cosine.distance(&a, &b);
        assert!(close(dist, 1.0 - sim, 1e-6));
    }

    // ─── compare_similarity ────────────────────────────────────

    #[test]
    fn compare_similarity_orders_floats() {
        use std::cmp::Ordering;
        assert_eq!(compare_similarity(0.5, 0.7), Ordering::Less);
        assert_eq!(compare_similarity(0.9, 0.1), Ordering::Greater);
        assert_eq!(compare_similarity(0.3, 0.3), Ordering::Equal);
    }

    #[test]
    fn compare_similarity_handles_nan() {
        use std::cmp::Ordering;
        let nan = f32::NAN;
        // NaN treated as smallest so it can't displace a real score
        // from a top-K heap.
        assert_eq!(compare_similarity(nan, 0.5), Ordering::Less);
        assert_eq!(compare_similarity(0.5, nan), Ordering::Greater);
        assert_eq!(compare_similarity(nan, nan), Ordering::Equal);
    }

    // ─── SIMD tail-loop alignment edge cases ──────────────────
    //
    // The SIMD kernels process `f32x8` chunks and handle the
    // residue with a scalar tail. Any length that isn't a multiple
    // of `LANES` (=8) exercises the tail. These tests pin the tail
    // path for the four "boundary" residue sizes — 1, 3, 7, 9 (one
    // chunk + 1) — against a deliberately-scalar reference computed
    // here, not against a hand-coded expected value. That keeps the
    // tests SIMD-implementation-independent: if the chunked path
    // and the scalar path disagree, the tail logic is wrong.

    /// Reference scalar implementations the SIMD path must agree
    /// with. Kept hand-rolled (no dispatch back into the public
    /// kernels) so a regression in the chunked code is observable.
    fn scalar_cosine(a: &[f32], b: &[f32]) -> f32 {
        let mut dot = 0.0f32;
        let mut na = 0.0f32;
        let mut nb = 0.0f32;
        for (x, y) in a.iter().zip(b.iter()) {
            dot += x * y;
            na += x * x;
            nb += y * y;
        }
        let denom = na.sqrt() * nb.sqrt();
        if denom == 0.0 {
            0.0
        } else {
            dot / denom
        }
    }
    fn scalar_l2_squared(a: &[f32], b: &[f32]) -> f32 {
        let mut s = 0.0f32;
        for (x, y) in a.iter().zip(b.iter()) {
            let d = x - y;
            s += d * d;
        }
        s
    }
    fn scalar_dot(a: &[f32], b: &[f32]) -> f32 {
        let mut s = 0.0f32;
        for (x, y) in a.iter().zip(b.iter()) {
            s += x * y;
        }
        s
    }

    fn fill(n: usize, seed: f32) -> Vec<f32> {
        (0..n).map(|i| seed + i as f32 * 0.013).collect()
    }

    #[test]
    fn tail_loop_matches_scalar_for_sub_lane_length() {
        // dim < LANES → entirely tail, zero SIMD chunks.
        for &dim in &[1usize, 3, 7] {
            let a = fill(dim, 0.1);
            let b = fill(dim, 0.25);
            assert!(
                close(cosine_similarity(&a, &b), scalar_cosine(&a, &b), 1e-5),
                "cosine mismatch at dim={dim}"
            );
            assert!(
                close(l2_distance_squared(&a, &b), scalar_l2_squared(&a, &b), 1e-4),
                "l2² mismatch at dim={dim}"
            );
            assert!(
                close(dot_product(&a, &b), scalar_dot(&a, &b), 1e-5),
                "dot mismatch at dim={dim}"
            );
        }
    }

    #[test]
    fn tail_loop_matches_scalar_for_one_chunk_plus_residue() {
        // dim = LANES + 1 = 9 → one full SIMD chunk plus a 1-element tail.
        let dim = LANES + 1;
        let a = fill(dim, -0.5);
        let b = fill(dim, 0.3);
        assert!(close(
            cosine_similarity(&a, &b),
            scalar_cosine(&a, &b),
            1e-5
        ));
        assert!(close(
            l2_distance_squared(&a, &b),
            scalar_l2_squared(&a, &b),
            1e-4
        ));
        assert!(close(dot_product(&a, &b), scalar_dot(&a, &b), 1e-5));
    }

    #[test]
    fn tail_loop_matches_scalar_for_exact_lane_multiple() {
        // dim = 16 → 2 SIMD chunks, no tail. Pin the no-tail path
        // separately so a future refactor of the chunk loop is caught
        // even if the tail logic is correct.
        let dim = 2 * LANES;
        let a = fill(dim, 1.0);
        let b = fill(dim, -0.7);
        assert!(close(
            cosine_similarity(&a, &b),
            scalar_cosine(&a, &b),
            1e-5
        ));
        assert!(close(
            l2_distance_squared(&a, &b),
            scalar_l2_squared(&a, &b),
            1e-4
        ));
        assert!(close(dot_product(&a, &b), scalar_dot(&a, &b), 1e-5));
    }

    #[test]
    fn simd_matches_scalar_for_realistic_embedding_dims() {
        // Sample the embedding-typical sizes (256, 384, 768, 1024)
        // and a deliberate odd one (777) to hit a non-trivial tail.
        for &dim in &[256usize, 384, 768, 777, 1024] {
            let a = fill(dim, 0.02);
            let b = fill(dim, -0.015);
            let sim_simd = cosine_similarity(&a, &b);
            let sim_scalar = scalar_cosine(&a, &b);
            assert!(
                close(sim_simd, sim_scalar, 1e-4),
                "cosine dim={dim}: simd={sim_simd}, scalar={sim_scalar}"
            );
            let l2_simd = l2_distance(&a, &b);
            let l2_scalar = scalar_l2_squared(&a, &b).sqrt();
            assert!(
                close(l2_simd, l2_scalar, 1e-3),
                "l2 dim={dim}: simd={l2_simd}, scalar={l2_scalar}"
            );
        }
    }
}
