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

//! **Packed shared forest** for CKY parsing (D63 blueprint / GH#97 Option A). A chart cell holds one
//! [`PNode`] per **signature** `(cat_shape, ENF-provenance)` instead of a flat `Vec<Item>`, so the
//! sense-product of same-`cat_shape` items collapses to a single node (Billot & Lang 1989; Harper
//! 1994). Combination is decided **once per node-pair** (via `apply` on representative items — sound
//! because the packing router gates on the grammar being *index-independent*, so combinability is a
//! function of `cat_shape` alone), recorded as an [`Edge::Combine`] hyperedge; the composed-cell
//! shifts are [`Edge::Unary`] edges. The differing semantics are materialised **lazily** at k-best
//! extraction (the cube-pruning extractor, [`super::lookup::LexicalIndex::kbest`]).
//!
//! `cat_group` never appears here: the router
//! ([`super::lookup::LexicalIndex::parse_needs_unpacked`]) sends any sentence with a coordinator (or
//! other token-keyed sem-reading construct) to the unpacked path (the §4 carve-out).

use std::collections::BTreeMap;

use super::parser::{Combinator, Cost, Item};
use super::pretty::cat_shape;

/// A packing **signature**: the category shape (type-indices erased, [`cat_shape`]) plus the Eisner
/// normal-form provenance. Two items share a node iff they share a `Sig` — the equivalence class
/// that behaves identically under all future combination (given the index-independence precondition).
pub(crate) type Sig = (String, Combinator);

/// The signature of an item — its packing key.
pub(crate) fn node_sig(it: &Item) -> Sig {
    (cat_shape(it.cat()), it.prov())
}

/// Index of a [`PNode`] in [`Forest::nodes`].
pub(crate) type NodeId = usize;

/// A derivation of a node: a lexical **leaf** item, a binary **combination** of two child nodes (a
/// hyperedge; the child cross-product is materialised lazily at extraction), or a **unary** transform
/// of one child node (type-raise / bare-plural·mass shift / fronted participial — the composed-cell
/// shifts of the unpacked CKY, applied per item at extraction).
pub(crate) enum Edge {
    Leaf(Item),
    Combine {
        left: NodeId,
        right: NodeId,
    },
    Unary {
        child: NodeId,
        kind: UnaryKind,
    },
    /// A **token-keyed sem-reading binary rule** (D63 §11 3g.3): the reserved word(s) between (or
    /// after) the two spans have no node, the DECISION is category-based, and the result embeds the
    /// children's sems — materialised per (left, right) item-pair at extraction, via [`BinRule`].
    /// Covers relative clauses, coordination, `but not`, the reciprocal, and appositives.
    Binary {
        left: NodeId,
        right: NodeId,
        rule: BinRule,
    },
}

/// Which token-keyed binary rule a [`Edge::Binary`] applies at materialisation (D63 §11 3g.3). Each
/// has a cat-based *decision* (checked on representatives at construction) and a sem-*building* rule.
#[derive(Clone, Copy)]
pub(crate) enum BinRule {
    /// `[noun] that/which [body] → refined noun` (`relativize`).
    Relativize,
    /// `[X] and/or [Y]` → a Prop-conjunction (same category) or a `cat_group` (`coordinate_sem` /
    /// `coordinate_np`); `op` is the connective IRI (`logic:And` / `logic:Or`).
    Coordinate(&'static str),
    /// `[O₁] but not [O₂]` → contrastive `a ∧ ¬b` or a `conn_but_not` group.
    ButNot,
    /// `[group] <TV> each other → S` (`reciprocate`).
    Reciprocal,
    /// Non-restrictive appositive `[NP] , that/which [body]` — subject/prep-object position
    /// (`relativize_appos`).
    AppositiveSubj,
    /// Non-restrictive appositive in verb-object position (the in-situ object raise).
    AppositiveObj,
}

/// Which composed-cell unary shift a [`Edge::Unary`] represents (D63 blueprint §11 3c.4b).
#[derive(Clone, Copy)]
pub(crate) enum UnaryKind {
    /// Forward bounded type-raising `T`: `NP → S/(S\NP)` (`raise_nps`).
    Raise,
    /// Bare-plural / bare-mass argument shift: a plural/mass `cat_n` → a deferred-quantifier NP.
    BareNp,
    /// Fronted participial adjunct: a subject-gapped `ger` VP → a sentence pre-modifier `S/S`.
    FrontParticipial,
    /// Fronted-modifier comma absorption (D62 §2 #5): a sentence-initial `S/S` pre-modifier at
    /// `[0, j-1]` absorbs a trailing comma at `j`, yielding the same modifier over `[0, j]` (so it can
    /// forward-apply across the otherwise node-less comma to the matrix clause). The child is the
    /// narrower `[0, j-1]` node, NOT a same-span transform; the item is carried through unchanged.
    AbsorbComma,
}

/// A packed forest node: all derivations of one `(span, Sig)` equivalence class.
pub(crate) struct PNode {
    /// The token span `(i, j)` this node covers (inclusive) — needed to re-freshen span-pure holes
    /// (`$quant$i_j` / `$anaphor$i_j`) when a [`Edge::Unary`] transform is materialised at extraction.
    pub span: (usize, usize),
    /// A representative item — used to decide node-level combinability (`apply` on reps) and to carry
    /// the result category for signature computation. Sound under index-independence: every item in
    /// the node combines identically, so any representative gives the correct edge + result `Sig`.
    pub rep: Item,
    pub edges: Vec<Edge>,
}

/// The packed chart: a flat node arena + a per-cell `Sig → NodeId` map (`cells[i][j]` spans tokens
/// `i..=j`). `BTreeMap` for deterministic iteration (the project-wide convention).
pub(crate) struct Forest {
    pub nodes: Vec<PNode>,
    pub cells: Vec<Vec<BTreeMap<Sig, NodeId>>>,
}

impl Forest {
    pub fn new(n: usize) -> Self {
        Forest {
            nodes: Vec::new(),
            cells: vec![vec![BTreeMap::new(); n]; n],
        }
    }

    /// The node for `sig` at cell `[i][j]`, created (with representative `rep`) if absent. Returns its
    /// [`NodeId`]. The `rep` of an existing node is kept (the first-seen representative).
    pub fn get_or_create(&mut self, i: usize, j: usize, sig: Sig, rep: &Item) -> NodeId {
        if let Some(&id) = self.cells[i][j].get(&sig) {
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(PNode {
            span: (i, j),
            rep: rep.clone(),
            edges: Vec::new(),
        });
        self.cells[i][j].insert(sig, id);
        id
    }

    /// Append a derivation to a node.
    pub fn push_edge(&mut self, id: NodeId, edge: Edge) {
        self.nodes[id].edges.push(edge);
    }
}

/// A cube-pruning candidate (Huang & Chiang 2005): the `(li, ri)` grid coordinate into a
/// combination's two cost-sorted child k-best lists, keyed by the combined child cost. Ordered so a
/// `BinaryHeap` (max-heap) pops the LOWEST `(cost, li, ri)` first — the `(li, ri)` tie-break makes
/// extraction byte-deterministic across runs (both child lists are deterministically sorted).
pub(crate) struct CubeCandidate {
    pub cost: Cost,
    pub li: usize,
    pub ri: usize,
}

impl PartialEq for CubeCandidate {
    fn eq(&self, o: &Self) -> bool {
        (self.cost, self.li, self.ri) == (o.cost, o.li, o.ri)
    }
}
impl Eq for CubeCandidate {}
impl Ord for CubeCandidate {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        // Invert: `BinaryHeap` is a max-heap, so the smallest key pops first.
        (o.cost, o.li, o.ri).cmp(&(self.cost, self.li, self.ri))
    }
}
impl PartialOrd for CubeCandidate {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::{list_decl, Exp};
    use crate::ontology::iri::Iri;

    fn ctor(name: &str, args: Vec<Exp>) -> Exp {
        Exp::InductiveCtor(list_decl(), name.into(), args)
    }
    fn cls(iri: &str) -> Exp {
        Exp::EigonClass(Iri::parse(iri).unwrap())
    }
    fn cat_np(ty: Exp) -> Exp {
        ctor("cat_np", vec![ty, ctor("num_any", vec![])])
    }

    // A leaf item with the given category (sem/cost irrelevant to the signature).
    fn leaf(cat: Exp) -> Item {
        Item::from_parts(cat, Exp::Unit, Combinator::Other, Cost::ZERO)
    }

    #[test]
    fn node_sig_erases_indices_but_keeps_shape_and_prov() {
        // Two NPs of different concrete types share a signature (cat_shape erases the index).
        let a = leaf(cat_np(cls("urn:eigenius:lexicon:Gene")));
        let b = leaf(cat_np(cls("urn:eigenius:lexicon:CellLine")));
        assert_eq!(
            node_sig(&a),
            node_sig(&b),
            "same cat_shape + prov ⇒ same Sig"
        );
    }

    #[test]
    fn get_or_create_dedups_by_sig_and_edges_accumulate() {
        let a = leaf(cat_np(cls("urn:eigenius:lexicon:Gene")));
        let b = leaf(cat_np(cls("urn:eigenius:lexicon:CellLine")));
        // A distinct shape: a bare cat_n vs cat_np.
        let noun = leaf(ctor(
            "cat_n",
            vec![cls("urn:eigenius:lexicon:Gene"), ctor("sg", vec![])],
        ));
        let mut f = Forest::new(1);
        let id_a = f.get_or_create(0, 0, node_sig(&a), &a);
        let id_b = f.get_or_create(0, 0, node_sig(&b), &b);
        assert_eq!(id_a, id_b, "same Sig ⇒ same node");
        let id_n = f.get_or_create(0, 0, node_sig(&noun), &noun);
        assert_ne!(id_a, id_n, "different cat_shape ⇒ different node");
        f.push_edge(id_a, Edge::Leaf(a));
        f.push_edge(id_a, Edge::Leaf(b));
        f.push_edge(
            id_n,
            Edge::Combine {
                left: id_a,
                right: id_n,
            },
        );
        assert_eq!(f.nodes[id_a].edges.len(), 2);
        assert_eq!(f.nodes.len(), 2, "two distinct signatures ⇒ two nodes");
    }

    #[test]
    fn cube_candidate_pops_lowest_cost_then_grid_order() {
        use std::collections::BinaryHeap;
        let c = |lo: u32, li: usize, ri: usize| CubeCandidate {
            cost: Cost::from_sense_rank(lo),
            li,
            ri,
        };
        let mut h: BinaryHeap<CubeCandidate> = BinaryHeap::new();
        h.push(c(5, 0, 0));
        h.push(c(2, 1, 0));
        h.push(c(2, 0, 1)); // same cost as (1,0) → (li,ri) tie-break: (0,1) < (1,0)
        let p1 = h.pop().unwrap();
        assert_eq!((p1.cost.sense_rank, p1.li, p1.ri), (2, 0, 1));
        let p2 = h.pop().unwrap();
        assert_eq!((p2.cost.sense_rank, p2.li, p2.ri), (2, 1, 0));
        let p3 = h.pop().unwrap();
        assert_eq!(p3.cost.sense_rank, 5);
    }
}
