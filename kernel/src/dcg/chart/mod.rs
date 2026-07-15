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
//! **The chart** — the CKY table itself, and the primitives every stage shares.
//!
//! A chart is a triangular table of cells; a cell is the bag of parse [`Item`]s spanning some token
//! range. Both stages that touch cells need the same two operations, and neither owns them: `seed`
//! beams the LEAF cells it fills, and the unpacked driver beams the COMPOSED cells it builds. `beam_cell`
//! lived in the unpacked driver, so seeding had to reach into a chart driver to prune its own output —
//! a dependency that said nothing true about the code.
//!
//! (The drivers themselves — `lookup::chart_packed` / `lookup::chart_unpacked` — belong here too; they
//! are still under `lookup` only because a few helpers have yet to be re-homed.)

pub(crate) mod forest;
pub(crate) mod packed;
pub(crate) mod unpacked;

use std::collections::BTreeMap;

use super::item::Item;

/// The CKY table: `chart[i][j]` holds every item spanning tokens `i..=j`. Named, because a bare
/// `Vec<Vec<Vec<Item>>>` in a signature tells the reader nothing.
pub(super) type Chart = Vec<Vec<Vec<Item>>>;

/// The sort key the per-lemma sense cap (D63 §8.7 / GH #97) truncates by: contextually-ranked
/// senses first (ordered by the reranker's `ranks` position), then the rest by static `sense_rank`
/// (most-frequent first). The leading `bool` puts `Some(ctx)` (`false`) ahead of unranked
/// (`true`). With `ranks = None` every sense is unranked, collapsing to the pure-`sense_rank`
/// order — the behaviour-identical static cap.
/// Cap a CKY chart cell to its `beam` lowest-[`Cost`] items (Lever B — per-cell beam, GH #97),
/// returning how many were dropped. A **stable** sort by `Cost` keeps the cheapest
/// (most-frequent-sense / preferred-lexicon) derivations and preserves insertion order within a
/// cost tie (so closed-class / cost-0 cells are order-preserved and deterministic). Inexact: a
/// dropped constituent may have been the only route to a full parse — the beam/A* tradeoff, why the
/// beam is opt-in.
pub(super) fn beam_cell(cell: &mut Vec<Item>, beam: usize) -> usize {
    if cell.len() <= beam {
        return 0;
    }
    let dropped = cell.len() - beam;
    cell.sort_by_key(|it| it.cost());
    cell.truncate(beam);
    dropped
}

/// Diagnostic (PARSE_DEBUG): a compact category-SHAPE histogram of a chart cell — total
/// items, count of distinct shapes ([`super::cat_shape`], type-indices erased), and the top
/// shapes by frequency. Many items under ONE shape ⇒ lexical/sense variation (a type-narrowing
/// candidate, GH#93); many distinct shapes ⇒ structural ambiguity (type-narrowing won't help).
pub(super) fn cell_histogram(cell: &[Item]) -> String {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for it in cell {
        *counts.entry(forest::cat_shape(it.cat())).or_default() += 1;
    }
    let distinct = counts.len();
    let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top: Vec<String> = pairs
        .iter()
        .take(4)
        .map(|(s, c)| format!("{s}×{c}"))
        .collect();
    format!("shapes={distinct} top: {}", top.join(", "))
}
