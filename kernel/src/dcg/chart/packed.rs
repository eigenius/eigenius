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
use super::super::item::Item;
use super::super::rules::combinators::apply;
use super::super::rules::registry::{unary_shifts, BinRule, UnaryKind};
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
                    let rctx = forest.nodes[node_id].rctx;
                    self.cube(&lk, &rk, k, &mut cands, |l, r| apply(l, r, layer, rctx));
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
                        self.materialize_unary(
                            it,
                            kind,
                            span,
                            forest.nodes[node_id].rctx,
                            &mut cands,
                        );
                    }
                }
            }
        }
        cands.sort_by_key(|it| it.cost());
        // Spend `k` on DISTINCT (category, sem) pairs. A node's edges routinely materialise the same
        // item twice — after core-en's `bnp` a bare kind reaches an argument slot both as a plain
        // `cat_np` and as its raised copy, and the two assemble identical sems — and each duplicate
        // otherwise consumes a k-best slot, evicting a genuinely different reading at the SAME cost
        // of a cheaper one. Witnessed: "The MSI relationship compared favourably to other strong
        // biomarkers for vulnerabilities." saturated k=256 with 52 distinct readings and lost its
        // nested-PP bracketing. Keyed on the category too, not the sem alone: a refined `cat_n`
        // carries its restrictor INSIDE the category, so two items sharing a sem can still combine
        // differently upstream. Cost-sorted first, so the survivor is the cheapest derivation.
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        cands.retain(|it| seen.insert(format!("{:?}|{:?}", it.cat(), it.sem())));
        // …and spend `k` on distinct STRUCTURES, not on a flat cost prefix. Sense multiplicity
        // inside a node dwarfs structural multiplicity — "PARP-1 inhibitors are successful in
        // cancers with deficiencies in homologous recombination." reaches 2048 candidates carrying
        // 297 structures — so a prefix fills with sense-variants of the cheapest bracketing and the
        // deeper (correct) nestings never reach the felicity gate at all. Before this, that unit's
        // skeleton count tracked `k` itself: 256 -> 2, 1024 -> 6, 2048 -> 13. A result that scales
        // smoothly with an arbitrary constant is the signature of a budget deciding the grammar.
        // Keyed on the CATEGORY too: a refined `cat_n` carries its restrictor inside the category,
        // so two items sharing a sem structure can still combine differently upstream.
        let mut cands = super::super::skeleton::spread_over_keys(cands, |it| {
            format!(
                "{}|{}",
                super::super::skeleton::skeleton_of(it.cat()),
                super::super::skeleton::skeleton_of(it.sem())
            )
        });
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
        rctx: super::super::rules::RightContext,
        out: &mut Vec<Item>,
    ) {
        // Comma absorption carries the sentence-premodifier through unchanged (it now spans the
        // trailing comma). The child is already `is_sentence_premod` (checked at forest build), so no
        // re-check here; the span widens but cat/sem/cost are identical. Every other shift comes from
        // the shared `unary_shifts()` table (Phase 2d), so materialisation cannot drift from build.
        match kind {
            UnaryKind::AbsorbComma => out.push(it.clone()),
            _ => {
                if let Some(shift) = unary_shifts().iter().find(|s| s.kind == kind) {
                    out.extend(shift.run(self, it, span, rctx));
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
        prefer_multiword: bool,
    ) -> packed::Forest {
        use packed::node_sig;
        let n = tokens.len();
        // Multiword span integrity (base cap only): a lexicalized multiword `cat_n` leaf at [a,b]
        // (b>a) is atomic — its interior split points {a..b-1} are protected so no compositional
        // constituent ever crosses it. This subsumes the redundant-compound case (a compositional
        // compound over [a,b] requires an interior split) AND blocks the left-branching split
        // ("[MSI cell] lines" over the lexicalized "cell lines"). Widen-on-failure clears
        // `prefer_multiword`, re-admitting the splits if the multiword won't compose in context, so
        // `grammar-gap 0` is preserved. The marking is shared with the unpacked driver and the tracer.
        let protected_split = super::multiword_protected_splits(leaves, prefer_multiword);
        let mut forest = Forest::new(n);
        // Group leaf items into nodes (one `Leaf` edge each; same-`Sig` items share a node).
        for (i, row) in leaves.iter().enumerate() {
            for (j, cell) in row.iter().enumerate().skip(i) {
                let rctx = super::super::rules::RightContext::after(&self.reserved, tokens, j);
                for it in cell {
                    let id = forest.get_or_create(i, j, node_sig(it), it, rctx);
                    forest.push_edge(id, Edge::Leaf(it.clone()));
                }
            }
        }
        // Node-level CKY: decide each node-pair ONCE via `apply` on representatives.
        for len in 2..=n {
            for i in 0..=(n - len) {
                let j = i + len - 1;
                // The cell's right context: a function of `j` alone, hence identical for every item in
                // every node of this cell — which is what keeps a rule that consults it decidable on a
                // node's representative.
                let rctx = super::super::rules::RightContext::after(&self.reserved, tokens, j);
                // Collect combinations first (immutable borrow of `forest`), then insert.
                let mut edges: Vec<(Sig, Item, NodeId, NodeId)> = Vec::new();
                for k in i..j {
                    // Span integrity: never combine across the interior of a multiword span.
                    if protected_split.get(k).copied().unwrap_or(false) {
                        continue;
                    }
                    let lefts: Vec<NodeId> = forest.cells[i][k].values().copied().collect();
                    let rights: Vec<NodeId> = forest.cells[k + 1][j].values().copied().collect();
                    for &l in &lefts {
                        for &r in &rights {
                            let lrep = forest.nodes[l].rep.clone();
                            let rrep = forest.nodes[r].rep.clone();
                            if let Some(result) = apply(&lrep, &rrep, &self.layer, rctx) {
                                edges.push((node_sig(&result), result, l, r));
                            }
                        }
                    }
                }
                for (sig, result, l, r) in edges {
                    let id = forest.get_or_create(i, j, sig, &result, rctx);
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
                    let id = forest.get_or_create(i, j, sig, &item, rctx);
                    forest.push_edge(id, Edge::Binary { left, right, rule });
                }

                // Composed-cell UNARY shifts (§11 3c.4b) — the shared `unary_shifts()` table (Phase 2d),
                // applied per node's representative and recorded as `Unary` edges (3d re-applies them
                // per item at extraction). Iterated IN TABLE ORDER, and each shift's edges are added to
                // the cell BEFORE the next shift reads it — so the type-raise sees the bare-NP shifts,
                // matching the unpacked CKY. Freshening only touches the sem, never `cat_shape`, so it
                // does not affect the signature.
                let mut unary: Vec<(Sig, Item, NodeId, UnaryKind)> = Vec::new();
                for shift in unary_shifts() {
                    for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                        let rep = forest.nodes[id].rep.clone();
                        for item in shift.run(self, &rep, (i, j), rctx) {
                            unary.push((node_sig(&item), item, id, shift.kind));
                        }
                    }
                    for (sig, item, child, kind) in unary.drain(..) {
                        let nid = forest.get_or_create(i, j, sig, &item, rctx);
                        forest.push_edge(nid, Edge::Unary { child, kind });
                    }
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
                        let nid = forest.get_or_create(i, j, sig, &item, rctx);
                        forest.push_edge(nid, Edge::Unary { child, kind });
                    }
                }
                // Targeted cell dump (`EIGENIUS_DUMP_CELL=i..j`) — the PACKED path's twin of the ones
                // in `chart::unpacked` and `parse::seed`. This is the PRODUCTION path, so a difference
                // that only shows here (packing collapses distinct derivations onto one representative)
                // is invisible in the other two. Prints each node's representative category.
                if let Ok(want) = std::env::var("EIGENIUS_DUMP_CELL") {
                    if want == format!("{i}..{j}") {
                        eprintln!(
                            "  ===== DUMP packed[{i}..{j}] ({} nodes) =====",
                            forest.cells[i][j].len()
                        );
                        for nid in forest.cells[i][j].values() {
                            let n = &forest.nodes[*nid];
                            let edges: Vec<String> = n
                                .edges
                                .iter()
                                .map(|e| match e {
                                    Edge::Leaf(_) => "Leaf".to_string(),
                                    Edge::Combine { left, right } => {
                                        format!(
                                            "Combine({}+{})",
                                            super::super::pretty::pretty_term(
                                                forest.nodes[*left].rep.cat()
                                            ),
                                            super::super::pretty::pretty_term(
                                                forest.nodes[*right].rep.cat()
                                            )
                                        )
                                    }
                                    Edge::Unary { kind, .. } => format!("Unary({kind:?})"),
                                    Edge::Binary { .. } => "Binary".to_string(),
                                })
                                .collect();
                            eprintln!(
                                "    [{:?}] {}\n        <- {}",
                                n.rep.prov(),
                                super::super::pretty::pretty_term(n.rep.cat()),
                                edges.join("\n        <- ")
                            );
                        }
                    }
                }
            }
        }
        forest
    }
}
