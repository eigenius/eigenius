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
//! **The packed chart driver** — the algorithms over the packed shared forest (D63 Option A / GH#97).
//! The forest's DATA (`Forest` / `PNode` / `Edge` / `Sig`) lives in `super::super::super::packed`; this is what
//! builds and reads it.
//!
//! `build_forest` runs a NODE-level CKY: a chart cell holds one node per signature, so combination is
//! decided ONCE per node-pair (on representative items — sound because a `Sig` captures everything a
//! decision consults) and recorded as a hyperedge. That collapses the sense-product of same-shape items
//! that makes the flat chart blow up over a dense lexicon.
//!
//! `kbest` then materialises the differing SEMANTICS lazily, by cube pruning (Huang & Chiang 2005) over
//! the children's cost-sorted k-best lists — so the forest is built once but only the low-cost readings
//! are ever assembled.
//!
//! WHERE the token-keyed rules fire, and HOW each pair combines, both come from `super::rules` — the
//! same registry the flat chart uses, so the two drivers cannot drift apart.

use super::super::category::is_sentence_premod;
use super::super::grammar::Grammar;
use super::super::holes::{freshen_anaphor, hole_base};
use super::super::item::Item;
use super::super::rules::combinators::apply;
use super::super::rules::constructions::{complete_coord, front_participial};
use super::super::rules::registry::{BinRule, UnaryKind};
use super::forest::{self as packed, CubeCandidate, Edge, Forest, NodeId, Sig};

impl Grammar {
    /// Lazy k-best extraction from a packed-forest node (D63 §11 3d). Merges the node's edges — `Leaf`
    /// (the item), `Combine` (cube pruning over the two children's k-best, materialised by `apply` per
    /// pop in `(cost, li, ri)` order, bounded by `max_pops`), `Unary` (the composed-cell shift applied
    /// to each child item) — then cost-sorts and keeps `k`. Memoised per node (the forest is a DAG by
    /// span). **No felicity here** — the felicity pop-filter runs once at the top span, matching the
    /// unpacked path (which type-checks only the full span).
    pub(crate) fn kbest(
        &self,
        forest: &packed::Forest,
        node_id: packed::NodeId,
        k: usize,
        memo: &mut Vec<Option<Vec<Item>>>,
    ) -> Vec<Item> {
        if let Some(cached) = &memo[node_id] {
            return cached.clone();
        }
        memo[node_id] = Some(Vec::new()); // DAG re-entrancy guard (no cycles expected).
        let span = forest.nodes[node_id].span;
        let mut cands: Vec<Item> = Vec::new();
        for e in 0..forest.nodes[node_id].edges.len() {
            match &forest.nodes[node_id].edges[e] {
                packed::Edge::Leaf(it) => cands.push(it.clone()),
                packed::Edge::Combine { left, right } => {
                    let (l, r) = (*left, *right);
                    let lk = self.kbest(forest, l, k, memo);
                    let rk = self.kbest(forest, r, k, memo);
                    let layer = &self.layer;
                    self.cube(&lk, &rk, k, &mut cands, |l, r| apply(l, r, layer));
                }
                packed::Edge::Binary { left, right, rule } => {
                    let (l, r, rule) = (*left, *right, *rule);
                    let lk = self.kbest(forest, l, k, memo);
                    let rk = self.kbest(forest, r, k, memo);
                    self.cube(&lk, &rk, k, &mut cands, |l, r| {
                        self.apply_bin_rule(rule, l, r)
                    });
                }
                packed::Edge::Unary { child, kind } => {
                    let (child, kind) = (*child, *kind);
                    let ck = self.kbest(forest, child, k, memo);
                    for it in &ck {
                        self.materialize_unary(it, kind, span, &mut cands);
                    }
                }
            }
        }
        cands.sort_by_key(|it| it.cost());
        cands.truncate(k);
        memo[node_id] = Some(cands.clone());
        cands
    }

    /// Cube pruning (Huang & Chiang 2005) over a binary edge: enumerate `combine(lk[li], rk[ri])`
    /// best-first by combined child cost, pushing the two grid neighbours after each pop, until `k`
    /// results or the `max_pops` circuit-breaker trips (a dense pocket of non-combining pairs — the
    /// child lists are already combinability-homogeneous under index-independence, so this rarely
    /// fires). `combine` is the edge's binary rule (`apply` for `Combine`, `relativize` for
    /// `Relativize`). Appends materialised items to `out`.
    fn cube<F: Fn(&Item, &Item) -> Option<Item>>(
        &self,
        lk: &[Item],
        rk: &[Item],
        k: usize,
        out: &mut Vec<Item>,
        combine: F,
    ) {
        use std::collections::{BTreeSet, BinaryHeap};
        if lk.is_empty() || rk.is_empty() {
            return;
        }
        let mut heap: BinaryHeap<CubeCandidate> = BinaryHeap::new();
        let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
        heap.push(CubeCandidate {
            cost: lk[0].cost().saturating_add(rk[0].cost()),
            li: 0,
            ri: 0,
        });
        seen.insert((0, 0));
        let (mut kept, mut pops) = (0usize, 0usize);
        let max_pops = k.saturating_mul(10).max(64);
        while let Some(cc) = heap.pop() {
            pops += 1;
            if pops > max_pops {
                // Circuit-breaker (a dense pocket of non-combining pairs). Never silent — log the
                // shortfall so a partial cube is visible (D63 §11 3d.3).
                eprintln!(
                    "dcg::parse (packed): cube max_pops={max_pops} hit ({kept} kept of a \
                     {}×{} grid) — extraction may be partial",
                    lk.len(),
                    rk.len(),
                );
                break;
            }
            if let Some(item) = combine(&lk[cc.li], &rk[cc.ri]) {
                out.push(item);
                kept += 1;
                if kept >= k {
                    break;
                }
            }
            if cc.li + 1 < lk.len() && seen.insert((cc.li + 1, cc.ri)) {
                heap.push(CubeCandidate {
                    cost: lk[cc.li + 1].cost().saturating_add(rk[cc.ri].cost()),
                    li: cc.li + 1,
                    ri: cc.ri,
                });
            }
            if cc.ri + 1 < rk.len() && seen.insert((cc.li, cc.ri + 1)) {
                heap.push(CubeCandidate {
                    cost: lk[cc.li].cost().saturating_add(rk[cc.ri + 1].cost()),
                    li: cc.li,
                    ri: cc.ri + 1,
                });
            }
        }
    }

    /// Collect the [`Edge::Binary`] derivations for `rule` over a left span `ls = (i, k)` and a right
    /// span `rs = (k', j)` (both `(start, end)` inclusive cell coordinates), the token-keyed reserved
    /// word(s) between/after them having no node. For each `(left, right)` node-pair whose
    /// REPRESENTATIVES combine under [`Self::apply_bin_rule`], appends `(result-Sig, result-item,
    /// left, right, rule)` to `out` — the caller inserts them as [`Edge::Binary`] edges once the
    /// forest borrow is released. Sound under index-independence: the decision is representative-based.
    fn binary_edges(
        &self,
        forest: &packed::Forest,
        ls: (usize, usize),
        rs: (usize, usize),
        rule: BinRule,
        out: &mut Vec<(packed::Sig, Item, packed::NodeId, packed::NodeId, BinRule)>,
    ) {
        let lefts: Vec<packed::NodeId> = forest.cells[ls.0][ls.1].values().copied().collect();
        let rights: Vec<packed::NodeId> = forest.cells[rs.0][rs.1].values().copied().collect();
        for lid in lefts {
            for &rid in &rights {
                if let Some(item) =
                    self.apply_bin_rule(rule, &forest.nodes[lid].rep, &forest.nodes[rid].rep)
                {
                    out.push((packed::node_sig(&item), item, lid, rid, rule));
                }
            }
        }
    }

    /// Materialise a `Unary` edge for one child item — the composed-cell shift for [`UnaryKind`],
    /// with span-pure hole re-freshening (`$quant$i_j` / `$anaphor$i_j`). Mirrors the unpacked path's
    /// per-item shifts ([`Self::seed_leaves`] / the CKY loop). Appends to `out`.
    fn materialize_unary(
        &self,
        it: &Item,
        kind: UnaryKind,
        span: (usize, usize),
        out: &mut Vec<Item>,
    ) {
        let (i, j) = span;
        match kind {
            UnaryKind::BareNp => out.extend(self.bare_nominal_shifts(it)),
            UnaryKind::Raise => out.extend(self.raise_nps(std::slice::from_ref(it))),
            UnaryKind::FrontParticipial => {
                if let Some((cat, sem)) = front_participial(it.cat(), it.sem(), &self.layer) {
                    let sem = freshen_anaphor(&sem, &hole_base(i, j));
                    out.push(Item::with_cost(cat, sem, it.cost()));
                }
            }
            // Comma absorption carries the sentence-premodifier through unchanged (it now spans the
            // trailing comma). The child is already `is_sentence_premod` (checked at forest build), so
            // no re-check is needed here; the span widens but the cat/sem/cost are identical.
            UnaryKind::AbsorbComma => out.push(it.clone()),
            UnaryKind::CoordComplete => {
                if let Some((cat, sem)) = complete_coord(it.cat(), it.sem(), &self.layer) {
                    out.push(Item::with_cost(cat, sem, it.cost()));
                }
            }
        }
    }

    /// Build the **packed shared forest** over a sentence (D63 blueprint §11 3c.3/3c.4). Seeds the
    /// leaf cells (shared [`Self::seed_leaves`], `beam = None` — packing bounds via k-best), groups
    /// each cell's items into [`packed::PNode`]s by [`packed::node_sig`], then runs a
    /// node-level CKY loop: for each adjacent node-pair, `apply` on their REPRESENTATIVE items decides
    /// combinability + the result signature ONCE (the O(1)-per-node-pair win — sound because the
    /// packing router gated on the grammar being index-independent), recorded as an
    /// [`packed::Edge::Combine`] hyperedge. The differing item-pairs are materialised lazily by
    /// the cube-pruning extractor (3d).
    ///
    /// After each cell's binary combinations come the **token-keyed sem-reading binary rules** (§11
    /// 3g.3) — relatives, coordination, `but not`, the reciprocal, appositives — as
    /// [`packed::Edge::Binary`] edges (materialised per item-pair at extraction via
    /// [`Self::apply_bin_rule`]), then the composed-cell UNARY shifts (3c.4b) as
    /// [`packed::Edge::Unary`] edges, in the unpacked CKY's order: bare-plural/mass NP shift,
    /// type-raising (which sees the shifted NPs), the fronted participial, and the fronted-modifier
    /// comma absorption. The packed CKY now mirrors every construct the unpacked CKY has, so the router
    /// ([`Self::parse_needs_unpacked`]) only diverts pied-piping (`[prep] which`) and selectional
    /// lexicons — everything else is packed and gated on the differential oracle (3f).
    pub(crate) fn build_forest(
        &self,
        leaves: &[Vec<Vec<Item>>],
        tokens: &[String],
    ) -> packed::Forest {
        use packed::node_sig;
        let n = tokens.len();
        let mut forest = Forest::new(n);
        // Group leaf items into nodes (one `Leaf` edge each; same-`Sig` items share a node).
        for (i, row) in leaves.iter().enumerate() {
            for (j, cell) in row.iter().enumerate().skip(i) {
                for it in cell {
                    let id = forest.get_or_create(i, j, node_sig(it), it);
                    forest.push_edge(id, Edge::Leaf(it.clone()));
                }
            }
        }
        // Node-level CKY: decide each node-pair ONCE via `apply` on representatives.
        for len in 2..=n {
            for i in 0..=(n - len) {
                let j = i + len - 1;
                // Collect combinations first (immutable borrow of `forest`), then insert.
                let mut edges: Vec<(Sig, Item, NodeId, NodeId)> = Vec::new();
                for k in i..j {
                    let lefts: Vec<NodeId> = forest.cells[i][k].values().copied().collect();
                    let rights: Vec<NodeId> = forest.cells[k + 1][j].values().copied().collect();
                    for &l in &lefts {
                        for &r in &rights {
                            let lrep = forest.nodes[l].rep.clone();
                            let rrep = forest.nodes[r].rep.clone();
                            if let Some(result) = apply(&lrep, &rrep, &self.layer) {
                                edges.push((node_sig(&result), result, l, r));
                            }
                        }
                    }
                }
                for (sig, result, l, r) in edges {
                    let id = forest.get_or_create(i, j, sig, &result);
                    forest.push_edge(id, Edge::Combine { left: l, right: r });
                }

                // Token-keyed sem-reading binary rules (§11 3g.3): relative clauses, coordination,
                // `but not`, appositives, and the reciprocal. WHERE each fires comes from the shared
                // registry ([`Self::binary_sites`]) — the same list the unpacked CKY iterates — so the
                // two paths cannot drift. Each site's node-pairs are decided on representatives
                // (`binary_edges`), recorded as `Binary` edges, and materialised per item-pair at
                // extraction ([`Self::apply_bin_rule`]). Run before the unary shifts so a resulting
                // refined noun / group can shift or feed larger cells.
                let mut bin: Vec<(Sig, Item, NodeId, NodeId, BinRule)> = Vec::new();
                for site in self.binary_sites(tokens, i, j) {
                    self.binary_edges(&forest, site.left, site.right, site.rule, &mut bin);
                }
                for (sig, item, left, right, rule) in bin {
                    let id = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(id, Edge::Binary { left, right, rule });
                }

                // Composed-cell UNARY shifts (§11 3c.4b), applied per node's representative and
                // recorded as `Unary` edges (3d re-applies them per item at extraction). Order matches
                // the unpacked CKY: (1) bare-plural/mass NP shift, (2) type-raise over the updated
                // cell (so it sees the shifted NPs), (3) fronted participial. Freshening only touches
                // the sem, never `cat_shape`, so it does not affect the signature — but it is applied
                // here so the representative sems stay consistent with the unpacked path.
                let mut unary: Vec<(Sig, Item, NodeId, UnaryKind)> = Vec::new();
                // Coordination list-completion (D63 §8.4 Phase 3): fold each prop-ending `cat_coord`
                // node in this cell into its base category. The `cat_coord` node stays (a longer list
                // extends it); the completed base-category node is what a copula / matrix consumes.
                for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                    let rep = forest.nodes[id].rep.clone();
                    if let Some((cat, sem)) = complete_coord(rep.cat(), rep.sem(), &self.layer) {
                        let item = Item::with_cost(cat, sem, rep.cost());
                        unary.push((node_sig(&item), item, id, UnaryKind::CoordComplete));
                    }
                }
                for (sig, item, child, kind) in unary.drain(..) {
                    let nid = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(nid, Edge::Unary { child, kind });
                }
                for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                    let rep = forest.nodes[id].rep.clone();
                    for np in self.bare_nominal_shifts(&rep) {
                        unary.push((node_sig(&np), np, id, UnaryKind::BareNp));
                    }
                }
                for (sig, item, child, kind) in unary.drain(..) {
                    let nid = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(nid, Edge::Unary { child, kind });
                }
                for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                    let rep = forest.nodes[id].rep.clone();
                    for raised in self.raise_nps(std::slice::from_ref(&rep)) {
                        unary.push((node_sig(&raised), raised, id, UnaryKind::Raise));
                    }
                }
                for (sig, item, child, kind) in unary.drain(..) {
                    let nid = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(nid, Edge::Unary { child, kind });
                }
                for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                    let rep = forest.nodes[id].rep.clone();
                    if let Some((cat, sem)) = front_participial(rep.cat(), rep.sem(), &self.layer) {
                        let sem = freshen_anaphor(&sem, &hole_base(i, j));
                        let item = Item::with_cost(cat, sem, rep.cost());
                        unary.push((node_sig(&item), item, id, UnaryKind::FrontParticipial));
                    }
                }
                for (sig, item, child, kind) in unary.drain(..) {
                    let nid = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(nid, Edge::Unary { child, kind });
                }
                // Fronted-modifier comma absorption (§11 3g.3): a sentence-initial `S/S` pre-modifier
                // at `[0, j-1]` carries over a trailing comma at `j` to span `[0, j]`, so it can then
                // forward-apply across the node-less comma to the matrix clause. Keyed on `i == 0` (so
                // it never competes with list-coordination commas); the child keeps its `Sig`, so the
                // absorbed node packs identically. Mirrors the unpacked CKY's comma-absorption.
                if i == 0 && j >= 1 && self.reserved.is_comma(&tokens[j]) {
                    for cid in forest.cells[0][j - 1].values().copied().collect::<Vec<_>>() {
                        let rep = forest.nodes[cid].rep.clone();
                        if is_sentence_premod(rep.cat()) {
                            unary.push((node_sig(&rep), rep, cid, UnaryKind::AbsorbComma));
                        }
                    }
                    for (sig, item, child, kind) in unary.drain(..) {
                        let nid = forest.get_or_create(i, j, sig, &item);
                        forest.push_edge(nid, Edge::Unary { child, kind });
                    }
                }
            }
        }
        forest
    }
}
