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

//! D43 §3.4 / M6.6 — HNSW recall measurement.
//!
//! The HNSW search path returns approximate top-K results; the
//! design (§3.4) commits to surfacing a per-segment recall@K
//! estimate so callers can reason about exactness:
//!
//! > Per-segment recall depends on `ef` (typical: ~95 % recall@k at
//! > `ef = k * 2`, ~99 % at `ef = k * 8`). The final result set
//! > carries the minimum per-segment recall touched by any returned
//! > hit, so callers can reason about exactness.
//!
//! Two paths exist for measuring recall:
//!
//! 1. **Heuristic from `ef / k` ratio** — what v1 ships.
//!    Inexpensive, monotone in `ef`, anchored to the published
//!    operating points. Sufficient for the M6 "callers can reason
//!    about exactness" guarantee.
//! 2. **Sampled ground-truth comparison** — periodically run the
//!    brute-force path alongside HNSW for a small fraction of
//!    queries, average the actual top-K agreement. The result
//!    replaces the heuristic on segments where it has been
//!    measured. Deferred to a follow-up; the [`SegmentRecall`]
//!    surface accepts either source so the upgrade is additive.
//!
//! Flat segments contribute `recall = 1.0` trivially (exact
//! brute-force). The combined result set carries
//! `min(per_segment_recalls)`, so a query that crosses one HNSW
//! segment at recall 0.96 and three flat segments returns
//! `min_recall = 0.96`.

/// Per-segment recall estimate for an HNSW search.
///
/// `Exact` represents a flat-search segment (recall = 1.0). The
/// `Approx { recall, source }` variant carries both the value and
/// its provenance so callers can distinguish heuristic estimates
/// from sampled ground-truth measurements; v1 only emits
/// [`RecallSource::Heuristic`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SegmentRecall {
    /// Flat brute-force — exact by construction.
    Exact,
    /// HNSW — approximate, with the estimate's source.
    Approx { recall: f32, source: RecallSource },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallSource {
    /// Heuristic from `ef / k` per D43 §3.4. v1 default.
    Heuristic,
    /// Sampled comparison against the brute-force baseline on a
    /// fraction of queries. Reserved for the M6 follow-up.
    Sampled,
}

impl SegmentRecall {
    /// Reduce a [`SegmentRecall`] to a single `f32` for the
    /// combined-result-set `min_recall` rollup. `Exact → 1.0`.
    pub fn as_f32(self) -> f32 {
        match self {
            Self::Exact => 1.0,
            Self::Approx { recall, .. } => recall,
        }
    }

    /// Whether this segment was searched exactly.
    pub fn is_exact(self) -> bool {
        matches!(self, Self::Exact)
    }
}

/// Compute the heuristic recall@k estimate for an HNSW search at
/// the given `(k, ef)` pair. Anchored to D43 §3.4's published
/// operating points:
///
/// | ef / k | Recall |
/// |---|---|
/// | < 2     | 0.85 (under-explored; below recommended `ef`) |
/// | 2       | 0.95 |
/// | 8       | 0.99 |
/// | > 8     | 0.99 (asymptote — diminishing returns) |
/// | 2 < r < 8 | linear interpolation between 0.95 and 0.99 |
///
/// `k = 0` is undefined; returns `1.0` for the no-op case so
/// downstream `min_recall` reductions stay correct.
pub fn heuristic_recall(k: usize, ef: usize) -> f32 {
    if k == 0 {
        return 1.0;
    }
    let ratio = ef as f32 / k as f32;
    if ratio < 2.0 {
        0.85
    } else if ratio >= 8.0 {
        0.99
    } else {
        // Linear interpolate between (2.0, 0.95) and (8.0, 0.99).
        // recall = 0.95 + (ratio - 2) * (0.99 - 0.95) / (8 - 2)
        //        = 0.95 + (ratio - 2) * 0.00666...
        let t = (ratio - 2.0) / 6.0;
        0.95 + t * 0.04
    }
}

/// Combine per-segment recalls into the result-set's exposed
/// `min_recall`. The minimum across all touched segments. Returns
/// `None` for an empty result set (no segments touched).
pub fn min_recall<I: IntoIterator<Item = SegmentRecall>>(items: I) -> Option<f32> {
    let mut iter = items.into_iter();
    let first = iter.next()?;
    let mut acc = first.as_f32();
    for r in iter {
        let v = r.as_f32();
        if v < acc {
            acc = v;
        }
    }
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn exact_is_one_point_zero() {
        assert_eq!(SegmentRecall::Exact.as_f32(), 1.0);
        assert!(SegmentRecall::Exact.is_exact());
    }

    #[test]
    fn approx_carries_its_value() {
        let r = SegmentRecall::Approx {
            recall: 0.97,
            source: RecallSource::Heuristic,
        };
        assert!(close(r.as_f32(), 0.97, 1e-6));
        assert!(!r.is_exact());
    }

    #[test]
    fn heuristic_recall_at_design_anchor_points() {
        // ef = k * 2 → 0.95
        assert!(close(heuristic_recall(10, 20), 0.95, 1e-4));
        // ef = k * 8 → 0.99
        assert!(close(heuristic_recall(10, 80), 0.99, 1e-4));
        // ef = k * 4 (the §3.4 default) → ~0.97
        let mid = heuristic_recall(10, 40);
        assert!(
            mid > 0.96 && mid < 0.98,
            "ef=4k should fall between 0.95 and 0.99; got {mid}"
        );
    }

    #[test]
    fn heuristic_recall_under_explored_floors_at_0_85() {
        assert!(close(heuristic_recall(10, 5), 0.85, 1e-6));
        assert!(close(heuristic_recall(10, 19), 0.85, 1e-6));
    }

    #[test]
    fn heuristic_recall_over_explored_caps_at_0_99() {
        assert!(close(heuristic_recall(10, 100), 0.99, 1e-6));
        assert!(close(heuristic_recall(10, 1000), 0.99, 1e-6));
    }

    #[test]
    fn heuristic_recall_is_monotone_in_ef() {
        let mut prev = 0.0;
        for ef in (10..=200).step_by(10) {
            let r = heuristic_recall(10, ef);
            assert!(
                r >= prev,
                "recall should be monotone non-decreasing in ef; at ef={ef} got {r} (prev {prev})"
            );
            prev = r;
        }
    }

    #[test]
    fn heuristic_recall_k_zero_is_safe() {
        // Degenerate; should not panic and shouldn't poison the
        // `min_recall` rollup.
        assert_eq!(heuristic_recall(0, 64), 1.0);
    }

    #[test]
    fn min_recall_picks_the_minimum() {
        let segs = [
            SegmentRecall::Exact,
            SegmentRecall::Approx {
                recall: 0.96,
                source: RecallSource::Heuristic,
            },
            SegmentRecall::Approx {
                recall: 0.92,
                source: RecallSource::Heuristic,
            },
        ];
        let m = min_recall(segs).expect("non-empty");
        assert!(close(m, 0.92, 1e-6), "min should be 0.92; got {m}");
    }

    #[test]
    fn min_recall_all_exact_is_one() {
        let segs = [SegmentRecall::Exact; 3];
        assert_eq!(min_recall(segs), Some(1.0));
    }

    #[test]
    fn min_recall_empty_input_is_none() {
        let segs: [SegmentRecall; 0] = [];
        assert_eq!(min_recall(segs), None);
    }
}
