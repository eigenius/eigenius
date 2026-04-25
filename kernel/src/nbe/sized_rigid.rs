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

//! Rigid-variable hypothesis tracker for sized types (Phase 11b step 15b).
//!
//! Direct port of MiniAgda's [`TreeShapedOrder.hs`](../../../references/miniagda/src/TreeShapedOrder.hs).
//! Complements the meta-resolution solver in [`sized`](super::sized)
//! with a specialised data structure for tracking strict inequalities
//! between *rigid* size variables — the hypotheses a type checker
//! accumulates as it walks through size-parameter binders.
//!
//! ## Why a separate structure from [`sized`]
//!
//! MiniAgda runs two cooperating constraint systems (documented in
//! `TCM.hs:755` and neighbours):
//!
//! 1. [`sized`] (Warshall) — meta-variable resolution. Constraints
//!    always involve at least one flexible (meta) endpoint. The
//!    output is a solution assigning size expressions to each meta.
//! 2. This module (TSO) — rigid-only hypothesis tracking. Accumulated
//!    as `addSizeRel v_i n v_j` calls (`TCM.hs:755`) meaning
//!    `v_i + n ≤ v_j`. Stored as a forest of upside-down trees where
//!    each child→parent link carries a non-negative distance.
//!
//! This structure is used to answer:
//! - **Entailment queries**: is a queried inequality `v_a + k ≤ v_b`
//!   derivable from the stored hypotheses? Answered by walking up
//!   the ancestor chain of `v_a` looking for `v_b`.
//! - **Consistency checks**: would adding a new hypothesis
//!   `v_a + n ≤ v_b` be inconsistent with the existing forest? The
//!   `increases_height` check rejects insertions that would violate
//!   an existing minimal valuation.
//!
//! ## Distance semantics
//!
//! A child→parent edge `a → (n, b)` encodes `a + n ≤ b`:
//! - `n = 0`: `a ≤ b`
//! - `n = 1`: `a < b` (strict)
//! - `n ≥ 2`: `a` is at least `n` units below `b`
//!
//! Distances are non-negative. Walking up an ancestor chain
//! accumulates distances: if `a → (n₁, b) → (n₂, c)`, then `a + n₁ + n₂ ≤ c`.

use std::collections::BTreeMap;

/// Tree-shaped partial order over identifiers of type `u32`.
///
/// Stored as `child → (distance, parent)`. A node without an entry
/// is a root (or not yet in the order).
///
/// Distances on links are non-negative integers, where `0` means
/// `child ≤ parent`, `1` means `child < parent`, and larger values
/// encode "at least n units smaller."
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tso {
    /// `links[&child] = (distance, parent)` — the immediate parent
    /// of each non-root node.
    links: BTreeMap<u32, (u32, u32)>,
}

impl Tso {
    /// Create an empty TSO (no nodes, no links).
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert the link `child → (distance, parent)`.
    ///
    /// Does not check for cycles or structural violations — callers
    /// typically gate this behind [`Self::increases_height`] and
    /// [`Self::is_ancestor`] for consistency.
    pub fn insert(&mut self, child: u32, distance: u32, parent: u32) {
        self.links.insert(child, (distance, parent));
    }

    /// Build a TSO from a list of `(child, distance, parent)` tuples
    /// in order; later insertions shadow earlier ones for the same
    /// child.
    pub fn from_list(items: &[(u32, u32, u32)]) -> Self {
        let mut t = Self::new();
        for &(c, d, p) in items {
            t.insert(c, d, p);
        }
        t
    }

    /// Walk the chain of ancestors from `a` upward, yielding each
    /// `(distance, ancestor)` pair. The first pair is `a`'s immediate
    /// parent (if any); the last pair is a root's immediate parent
    /// or the iterator ends at a root.
    pub fn parents(&self, a: u32) -> Vec<(u32, u32)> {
        let mut result = Vec::new();
        let mut cursor = self.links.get(&a).copied();
        while let Some((n, b)) = cursor {
            result.push((n, b));
            cursor = self.links.get(&b).copied();
        }
        result
    }

    /// The immediate parent of `a`, if any.
    pub fn parent(&self, a: u32) -> Option<(u32, u32)> {
        self.links.get(&a).copied()
    }

    /// If `b` is an ancestor of `a` (including `a == b`), return the
    /// total distance from `a` up to `b`. Otherwise return `None`.
    ///
    /// Corresponds to MiniAgda's `isAncestor a b o`. A return of
    /// `Some(k)` entails the inequality `a + k ≤ b` in the stored
    /// hypotheses (with the distance interpretation of ≤/<).
    pub fn is_ancestor(&self, a: u32, b: u32) -> Option<u32> {
        if a == b {
            return Some(0);
        }
        let mut acc = 0u32;
        for (n, ancestor) in self.parents(a) {
            acc = acc.saturating_add(n);
            if ancestor == b {
                return Some(acc);
            }
        }
        None
    }

    /// Signed distance between `a` and `b`:
    /// - `Some(k)` with `k ≥ 0` if `b` is reachable walking up from `a` in k steps
    /// - `Some(-k)` with `k > 0` if `a` is reachable walking up from `b` in k steps
    /// - `None` if the two are on disjoint chains.
    ///
    /// Matches MiniAgda's `diff a b o`.
    pub fn diff(&self, a: u32, b: u32) -> Option<i64> {
        if let Some(k) = self.is_ancestor(a, b) {
            return Some(k as i64);
        }
        self.is_ancestor(b, a).map(|k| -(k as i64))
    }

    /// Longest distance from `a` down to a leaf of its subtree.
    /// Returns `None` if `a` is not in the TSO at all (has no
    /// ancestors and is not a parent of any other node).
    pub fn height(&self, a: u32) -> Option<u32> {
        let children_map = self.invert();
        if !children_map.contains_key(&a) {
            return None;
        }
        fn longest(a: u32, children_map: &BTreeMap<u32, Vec<(u32, u32)>>) -> u32 {
            let Some(children) = children_map.get(&a) else {
                return 0;
            };
            if children.is_empty() {
                return 0;
            }
            children
                .iter()
                .map(|&(dist, child)| dist.saturating_add(longest(child, children_map)))
                .max()
                .unwrap_or(0)
        }
        Some(longest(a, &children_map))
    }

    /// Would inserting `child → (distance, parent)` break an existing
    /// minimal valuation of the forest?
    ///
    /// Specifically: returns `true` if `distance > height(parent)`.
    /// Adding such a link would force the parent's subtree to extend
    /// further than its current deepest descendant allows.
    ///
    /// Used by the type checker before `addSizeRel` to reject
    /// inconsistent hypothesis accumulation (`TCM.hs:761`).
    pub fn increases_height(&self, _child: u32, distance: u32, parent: u32) -> bool {
        distance > self.height(parent).unwrap_or(0)
    }

    /// Build `parent → Vec<(distance, child)>` inverse map (not
    /// exposed; used for height computation). Every node that
    /// appears anywhere in the forest (as parent or child) is keyed.
    fn invert(&self) -> BTreeMap<u32, Vec<(u32, u32)>> {
        let mut m: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
        for (&child, &(dist, parent)) in &self.links {
            m.entry(child).or_default();
            m.entry(parent).or_default().push((dist, child));
        }
        m
    }

    /// Nodes that have no children — the leaves of each tree.
    /// Included only if they appear somewhere in the forest.
    pub fn leaves(&self) -> Vec<u32> {
        self.invert()
            .into_iter()
            .filter_map(|(node, children)| {
                if children.is_empty() {
                    Some(node)
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tso_has_no_parents_or_leaves() {
        let t = Tso::new();
        assert!(t.parents(0).is_empty());
        assert!(t.parent(0).is_none());
        assert!(t.leaves().is_empty());
        assert_eq!(t.height(0), None);
        assert_eq!(t.is_ancestor(0, 1), None);
    }

    #[test]
    fn single_chain_parents_walk_upward() {
        // 1 <1< 2 <1< 3
        let t = Tso::from_list(&[(1, 1, 2), (2, 1, 3)]);
        let ps = t.parents(1);
        assert_eq!(ps, vec![(1, 2), (1, 3)]);
    }

    #[test]
    fn is_ancestor_on_chain_returns_distance() {
        // 1 <1< 2 <1< 3 <1< 4
        let t = Tso::from_list(&[(1, 1, 2), (2, 1, 3), (3, 1, 4)]);
        assert_eq!(t.is_ancestor(1, 4), Some(3));
        assert_eq!(t.is_ancestor(1, 3), Some(2));
        assert_eq!(t.is_ancestor(1, 2), Some(1));
        assert_eq!(t.is_ancestor(1, 1), Some(0));
        // 4 is not below 1
        assert_eq!(t.is_ancestor(4, 1), None);
    }

    #[test]
    fn diff_is_signed() {
        let t = Tso::from_list(&[(1, 2, 2)]);
        assert_eq!(t.diff(1, 2), Some(2));
        assert_eq!(t.diff(2, 1), Some(-2));
        assert_eq!(t.diff(1, 1), Some(0));
        assert_eq!(t.diff(1, 42), None);
    }

    #[test]
    fn branching_tree_has_multiple_leaves() {
        // 1 < 3, 2 < 3 — two children sharing parent 3
        let t = Tso::from_list(&[(1, 1, 3), (2, 1, 3)]);
        let mut leaves = t.leaves();
        leaves.sort();
        assert_eq!(leaves, vec![1, 2]);
    }

    #[test]
    fn height_is_longest_descent_to_leaf() {
        // Tree: 3 is root. 3 has child 1 (dist 1) and child 2 (dist 5).
        // height(3) should be 5 (longer branch via 2).
        let t = Tso::from_list(&[(1, 1, 3), (2, 5, 3)]);
        assert_eq!(t.height(3), Some(5));
        assert_eq!(t.height(1), Some(0));
        assert_eq!(t.height(2), Some(0));
    }

    #[test]
    fn height_accumulates_through_chains() {
        // 1 <2< 2 <3< 3 — height(3) should be 2+3 = 5, height(2) = 2
        let t = Tso::from_list(&[(1, 2, 2), (2, 3, 3)]);
        assert_eq!(t.height(3), Some(5));
        assert_eq!(t.height(2), Some(2));
        assert_eq!(t.height(1), Some(0));
    }

    #[test]
    fn increases_height_rejects_distance_beyond_existing_subtree() {
        // 3 has child 1 at distance 5 → height(3) = 5.
        // Inserting 2 <- (6, 3) would need height(3) = 6, exceeding
        // the current minimum. increases_height should return true.
        let t = Tso::from_list(&[(1, 5, 3)]);
        assert!(t.increases_height(2, 6, 3));
        assert!(!t.increases_height(2, 5, 3));
        assert!(!t.increases_height(2, 0, 3));
    }

    #[test]
    fn increases_height_allows_insertion_under_leaf() {
        // Fresh parent with no existing subtree: height(p) = 0 (p
        // doesn't appear in the forest). Any non-zero distance
        // exceeds that.
        let t = Tso::new();
        assert!(t.increases_height(1, 1, 99));
        assert!(!t.increases_height(1, 0, 99));
    }

    #[test]
    fn miniagda_l1_example_roundtrips() {
        // Reproduce MiniAgda's own `l1` at TreeShapedOrder.hs:157:
        //   i0 <1< i1, i1 <1< i2, i2 <1< i3, i3 <1< i4, j2 <1< i3
        // Encode with ids: i0=0 .. i4=4, j2=12.
        let t = Tso::from_list(&[
            (0, 1, 1),  // i0 <1< i1
            (1, 1, 2),  // i1 <1< i2
            (2, 1, 3),  // i2 <1< i3
            (3, 1, 4),  // i3 <1< i4
            (12, 1, 3), // j2 <1< i3
        ]);

        // MiniAgda's t1 = diff "i2" "i1" o1 — "i2" is above "i1"
        // by 1 step; as diff(i2, i1), the walk up from i2 doesn't
        // reach i1 (i1 is below), so diff = -(is_ancestor(i1, i2)) = -1.
        // But MiniAgda's t1 definition matches against isAncestor
        // first for `a` then for `b`. For a=i2, b=i1: i1 is NOT
        // an ancestor of i2; i2 IS an ancestor of i1 (i1 <1< i2).
        // So diff = -isAncestor(i1, i2) = -1.
        assert_eq!(t.diff(2, 1), Some(-1));

        // t2 = diff "i2" "j2" — i2 and j2 are on disjoint chains
        // above their common ancestor i3. Neither is an ancestor of
        // the other, so None.
        assert_eq!(t.diff(2, 12), None);

        // t3 = height "i2" — i2's subtree reaches i0 via i1, a chain
        // of distance 1+1 = 2.
        assert_eq!(t.height(2), Some(2));

        // t4 = height "i4" — i4 is root; longest descent is
        // i4 → i3 (d=1) → i2 (d=1) → i1 (d=1) → i0 (d=1) = 4
        // or i4 → i3 (d=1) → j2 (d=1) = 2. Max = 4.
        assert_eq!(t.height(4), Some(4));

        // t5 = height "k" — k doesn't appear, Nothing.
        assert_eq!(t.height(999), None);
    }
}
