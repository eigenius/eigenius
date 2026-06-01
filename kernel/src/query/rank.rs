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

//! D43 §6.4 / M7.2 — per-source rank materialisation for RRF.
//!
//! RRF (Reciprocal Rank Fusion) is per-row-rank-based, not per-row-
//! score-based. Computing the fused score for a row therefore needs
//! the row's rank in each source's full ordering — which is only
//! observable after walking the entire row set per source. The
//! planner runs this materialisation in a pre-pass before row-by-row
//! evaluation; this module is the kernel of that pre-pass.
//!
//! The contract is small and intentionally generic in the row-id
//! type so the same machinery serves both the v1 binding-index keying
//! (the in-tree evaluator) and the eventual planner-side per-source
//! retrieval (M7.4 push-down).
//!
//! Determinism: ranks are integers starting at 1; ties are broken by
//! the row-id's natural ordering (which for binding indices is
//! insertion order, and for IRIs is lexicographic). The total order
//! is therefore stable across runs.

use std::collections::BTreeMap;

/// Assign 1-indexed ranks to a slice of `(row_id, score)` pairs
/// under "higher score = better rank". Ties are broken by `row_id`
/// ordering (smaller id wins). Returns a `BTreeMap<row_id, rank>`
/// for O(log n) per-row lookup during the RRF row-by-row pass.
///
/// NaN scores are treated as the lowest possible score so they sort
/// last but still receive a finite rank. This matches D43 §6.4's
/// "missing rank = infinity" only by approximation — the proper
/// "missing" handling lives in the caller (an absent `(row_id,
/// score)` entry maps to rank ∞ via `lookup.get(...)` returning
/// `None`).
///
/// `K` must be `Ord + Clone` so the result map can use it as a key.
pub fn assign_ranks_desc<K: Ord + Clone>(scored: &[(K, f64)]) -> BTreeMap<K, usize> {
    let mut indexed: Vec<(usize, K, f64)> = scored
        .iter()
        .enumerate()
        .map(|(i, (k, s))| (i, k.clone(), *s))
        .collect();
    // NaN sorts last under "higher is better".
    indexed.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or_else(|| {
                // At most one of a.2 / b.2 is NaN under partial_cmp's
                // contract; map NaN to "less than anything finite" so
                // it sorts last under our DESC ordering.
                match (a.2.is_nan(), b.2.is_nan()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater, // a is NaN → push later
                    (false, true) => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Equal,
                }
            })
            .then_with(|| a.1.cmp(&b.1))
    });

    let mut out = BTreeMap::new();
    for (rank_minus_1, (_orig_idx, key, _score)) in indexed.into_iter().enumerate() {
        out.insert(key, rank_minus_1 + 1);
    }
    out
}

/// D43 §3.6 — fused score for one row given its per-source ranks
/// and the RRF constant `k`. Missing-source ranks (the row didn't
/// appear in that source's ordering) contribute 0 — modeled as
/// `None` in the per-source slice.
///
/// `sum_i 1 / (k + rank_i)` per the Cormack-Clarke-Buettcher formula.
pub fn rrf_score(per_source_ranks: &[Option<usize>], k: u32) -> f64 {
    let k = k as f64;
    per_source_ranks
        .iter()
        .filter_map(|r| r.map(|rank| 1.0 / (k + rank as f64)))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Highest score → rank 1, descending. No ties.
    #[test]
    fn assign_ranks_descending_distinct() {
        let scored = vec![("a", 0.5), ("b", 0.9), ("c", 0.7)];
        let ranks = assign_ranks_desc(&scored);
        assert_eq!(ranks[&"b"], 1);
        assert_eq!(ranks[&"c"], 2);
        assert_eq!(ranks[&"a"], 3);
    }

    /// Ties are broken by row-id ordering (deterministic across runs).
    #[test]
    fn assign_ranks_ties_broken_by_id() {
        let scored = vec![("z", 1.0), ("a", 1.0), ("m", 1.0)];
        let ranks = assign_ranks_desc(&scored);
        // Same score → smaller id ranks first.
        assert_eq!(ranks[&"a"], 1);
        assert_eq!(ranks[&"m"], 2);
        assert_eq!(ranks[&"z"], 3);
    }

    /// NaN sorts last; finite-scored rows rank ahead of it.
    #[test]
    fn assign_ranks_nan_sorts_last() {
        let scored = vec![("a", f64::NAN), ("b", 0.1)];
        let ranks = assign_ranks_desc(&scored);
        assert_eq!(ranks[&"b"], 1);
        assert_eq!(ranks[&"a"], 2);
    }

    /// Empty input → empty output.
    #[test]
    fn assign_ranks_empty() {
        let empty: Vec<(&str, f64)> = Vec::new();
        let ranks = assign_ranks_desc(&empty);
        assert!(ranks.is_empty());
    }

    /// RRF score with two sources at ranks 1 and 1, k=60, should be
    /// `2 / (60+1)` per the formula.
    #[test]
    fn rrf_score_both_sources_rank_1() {
        let s = rrf_score(&[Some(1), Some(1)], 60);
        assert!((s - 2.0 / 61.0).abs() < 1e-12, "got {s}");
    }

    /// RRF score with one source rank 1 and the other missing
    /// contributes only the present source's term.
    #[test]
    fn rrf_score_one_source_missing() {
        let s = rrf_score(&[Some(1), None], 60);
        assert!((s - 1.0 / 61.0).abs() < 1e-12, "got {s}");
    }

    /// All sources missing → 0.0.
    #[test]
    fn rrf_score_all_missing() {
        let s = rrf_score(&[None, None], 60);
        assert_eq!(s, 0.0);
    }

    /// k decreases monotonically affect the top-rank term: a smaller
    /// k amplifies rank-1 contributions, a larger k dampens them.
    /// This is a sanity check on the formula's monotonicity, not a
    /// floating-point precision test.
    #[test]
    fn rrf_score_k_dampens_top_rank() {
        let s_small_k = rrf_score(&[Some(1)], 1);
        let s_large_k = rrf_score(&[Some(1)], 1000);
        assert!(
            s_small_k > s_large_k,
            "smaller k should amplify rank-1: {s_small_k} vs {s_large_k}"
        );
    }
}
