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
//! function of `cat_shape` alone), recorded as a [`Edge::Combine`] hyperedge. The differing
//! semantics are materialised **lazily** at k-best extraction (the cube-pruning extractor, burn-down
//! 3d) — this module is only the forest data structure + construction (3c).
//!
//! `cat_group` never appears here: the router ([`super::lookup::LexicalIndex::parse_needs_unpacked`])
//! sends any sentence with a coordinator to the unpacked path (the §4 carve-out), so the forest has
//! only [`Edge::Leaf`] and [`Edge::Combine`].

// TEMPORARY (burn-down 3c.1/3c.2): the forest is constructed + unit-tested here, but its edges/rep
// are not *consumed* until the packed CKY construction (3c.4) and the cube-pruning extractor (3d)
// read them. Remove this `allow` when 3d wires `parse_packed` to build + extract the forest.
#![allow(dead_code)]

use std::collections::BTreeMap;

use super::parser::{Combinator, Item};
use super::pretty::cat_shape;

#[cfg(test)]
use super::parser::Cost;

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

/// A derivation of a node: either a lexical **leaf** item, or a binary **combination** of two child
/// nodes (a hyperedge; the cross-product of the children's items is materialised lazily at extraction).
pub(crate) enum Edge {
    Leaf(Item),
    Combine { left: NodeId, right: NodeId },
}

/// A packed forest node: all derivations of one `(span, Sig)` equivalence class.
pub(crate) struct PNode {
    pub sig: Sig,
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
            sig: sig.clone(),
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
}
