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

pub(crate) mod attribute;
pub(crate) mod forest;
pub(crate) mod packed;
pub(crate) mod trace;
pub(crate) mod unpacked;

use std::collections::{BTreeMap, BTreeSet};

use super::item::Item;
use super::rules::combinators::cat_n_number;

/// The multi-token spans `[a, b]` (`b > a`) a seeded multiword leaf covers. `leaves` are the SEEDED
/// leaf cells (before composition), so a `cat_n` leaf in a multi-token cell is necessarily a lexicalized
/// multiword — and a `cat_group` leaf is necessarily an **RNR head-distribution** seed (D63
/// `docs/notes/d63-rnr-head-distribution.md`; compositional groups are built later in the CKY, never
/// seeded), whose span is protected the same way so the guessed cat_mod/composition of the coordination
/// is pruned in favour of the lexicalized-kind union (M-RNR-3, multiword-preference for the distributed
/// case — with the same widen fallback preserving `grammar-gap 0`).
///
/// A multiword `cat_np` leaf is a lexicalized **named entity** — a document-glossary named individual
/// ("Project Achilles", "project DRIVE") or a multiword proper name — so its span is protected too: the
/// atomic named-entity reading SHADOWS the compositional re-bracketing of its component words (the
/// "project"(N/V) + name split that crowds the coordinated-subject beam, D63
/// `docs/notes/d63-named-entity-glossary-source.md` §3c). Same widen fallback preserves `grammar-gap 0`.
/// Shared by [`multiword_protected_splits`] and the tracer's context header.
pub(crate) fn multiword_spans(leaves: &[Vec<Vec<Item>>]) -> BTreeSet<(usize, usize)> {
    let mut spans = BTreeSet::new();
    for (a, row) in leaves.iter().enumerate() {
        for (b, cell) in row.iter().enumerate().skip(a + 1) {
            if cell.iter().any(|it| {
                cat_n_number(it.cat()).is_some()
                    || super::category::is_ctor(it.cat(), "cat_group").is_some()
                    || super::category::is_ctor(it.cat(), "cat_np").is_some()
            }) {
                spans.insert((a, b));
            }
        }
    }
    spans
}

/// **Multiword span integrity** (base-cap only) — the interior split points to PROTECT so no
/// compositional constituent tears a lexicalized multiword `cat_n` leaf. For each multiword span
/// `[a, b]` (`b > a`), protects every interior split `{a..b-1}`, so a driver skips those splits: the
/// atomic multiword is kept and its compositional re-bracketings (and any left/right-branching that
/// crosses it) are pruned. Widen-on-failure passes `prefer_multiword = false` → all-`false`,
/// re-admitting every split so `grammar-gap 0` is preserved.
///
/// **Boundary-exception (overlapping multiwords).** Two lexicalized multiwords can share a token —
/// `aDNA`[0,1] ∩ `DNA repair pathway`[1,3], `cancer cell`[0,1] ∩ `cell lines`[1,2] — and are then
/// mutually exclusive in any one derivation. Protecting each one's interior *globally* would veto the
/// split the OTHER needs (the determiner boundary / the alternative bracketing), so the sentence gaps
/// at base cap and explodes on widen. So an interior split `k` is left OPEN where a **different**
/// multiword abuts it — one starts just after (`k+1 ∈ starts`) or ends just before (`k ∈ ends`). An
/// ISOLATED multiword (no other multiword touching its interior) is still fully protected, which is
/// where the pruning does its work; the exception fires only at a genuine multiword-boundary overlap.
///
/// **Single source of truth** for both chart drivers ([`packed::Grammar::build_forest`],
/// [`unpacked`]) — the logic was verbatim-duplicated, so a change to one silently diverged the paths
/// the differential oracle keeps identical.
pub(crate) fn multiword_protected_splits(
    leaves: &[Vec<Vec<Item>>],
    prefer_multiword: bool,
) -> Vec<bool> {
    let n = leaves.len();
    let mut protected = vec![false; n];
    if !prefer_multiword {
        return protected;
    }
    let spans = multiword_spans(leaves);
    // A split `k` is a multiword BOUNDARY iff some multiword starts at `k+1` or ends at `k`. For an
    // interior split of a span `[a, b]` (`a ≤ k < b`) the span itself never matches (it starts at
    // `a ≤ k` and ends at `b > k`), so a match means a *different*, overlapping multiword abuts here.
    let starts: BTreeSet<usize> = spans.iter().map(|&(a, _)| a).collect();
    let ends: BTreeSet<usize> = spans.iter().map(|&(_, b)| b).collect();
    for &(a, b) in &spans {
        for (k, slot) in protected.iter_mut().enumerate().take(b).skip(a) {
            if starts.contains(&(k + 1)) || ends.contains(&k) {
                continue; // shared boundary with an overlapping multiword — leave it open
            }
            *slot = true;
        }
    }
    protected
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dcg::item::{Combinator, Cost};
    use crate::nbe::term::{list_decl, Exp};
    use crate::ontology::iri::Iri;

    /// A seeded multiword `cat_n` leaf (the number/class are irrelevant to span detection).
    fn cat_n_leaf() -> Item {
        let cat = Exp::InductiveCtor(
            list_decl(),
            "cat_n".into(),
            vec![
                Exp::EigonClass(Iri::parse("urn:eigenius:umlscui:C1").unwrap()),
                Exp::InductiveCtor(list_decl(), "sg".into(), vec![]),
            ],
        );
        Item::from_parts(cat, Exp::Unit, Combinator::Other, Cost::ZERO)
    }

    /// An `n × n` seeded-leaf grid with a multiword `cat_n` leaf at each given span `[a, b]`.
    fn leaves_with(n: usize, spans: &[(usize, usize)]) -> Vec<Vec<Vec<Item>>> {
        let mut leaves = vec![vec![Vec::new(); n]; n];
        for &(a, b) in spans {
            leaves[a][b].push(cat_n_leaf());
        }
        leaves
    }

    #[test]
    fn isolated_multiword_protects_its_interior() {
        // "deletion mutations" at [6..7] in an 8-token sentence — no overlapping multiword, so its
        // interior split k=6 stays protected (this is where the compound-pile pruning happens).
        let p = multiword_protected_splits(&leaves_with(8, &[(6, 7)]), true);
        assert!(p[6], "k=6 protected");
        assert_eq!(p.iter().filter(|&&b| b).count(), 1, "only k=6 protected");
    }

    #[test]
    fn overlapping_multiwords_open_the_shared_boundary() {
        // `cancer cell`[0,1] ∩ `cell lines`[1,2]: k=0 opens (cell-lines starts at 1), k=1 opens
        // (cancer-cell ends at 1). Neither interior is protected → both bracketings build, no gap.
        let p = multiword_protected_splits(&leaves_with(5, &[(0, 1), (1, 2)]), true);
        assert!(!p[0], "k=0 open (a different multiword starts at k+1=1)");
        assert!(!p[1], "k=1 open (a different multiword ends at k=1)");
        assert_eq!(p.iter().filter(|&&b| b).count(), 0);
    }

    #[test]
    fn determiner_boundary_stays_open_under_overlap() {
        // `aDNA`[0,1] ∩ `dna repair`[1,2] ∩ `dna repair pathway`[1,3]: the determiner boundary k=0
        // (where "a" must detach from the C1511689 multiword) is left open by the exception.
        let p = multiword_protected_splits(&leaves_with(6, &[(0, 1), (1, 2), (1, 3)]), true);
        assert!(!p[0], "determiner boundary k=0 open");
    }

    #[test]
    fn isolated_and_overlap_coexist_in_one_sentence() {
        // An overlap at the front (`cancer cell`∩`cell lines`, [0,1]/[1,2]) and an ISOLATED multiword
        // at the back ([5,6]): the exception fires only at the overlap; the isolated one keeps k=5.
        let p = multiword_protected_splits(&leaves_with(8, &[(0, 1), (1, 2), (5, 6)]), true);
        assert!(!p[0] && !p[1], "overlap boundaries open");
        assert!(p[5], "isolated multiword still protected");
    }

    #[test]
    fn widen_lifts_all_protection() {
        // `prefer_multiword = false` (a widen rung) re-admits every split — the grammar-gap-0 escape.
        let p = multiword_protected_splits(&leaves_with(8, &[(6, 7)]), false);
        assert!(p.iter().all(|&b| !b));
    }
}
