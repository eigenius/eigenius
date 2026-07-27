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
//! [`PNode`] per **signature** ([`Sig`]) instead of a flat `Vec<Item>`, so the sense-product of
//! same-`cat_shape` items collapses to a single node (Billot & Lang 1989; Harper 1994). Combination is
//! decided **once per node-pair** (via `apply` on representative items — sound because a `Sig` captures
//! everything a decision consults; see the invariant on [`node_sig`]), recorded as an [`Edge::Combine`]
//! hyperedge; the token-keyed rules are [`Edge::Binary`] edges and the composed-cell shifts
//! [`Edge::Unary`] edges. The differing semantics are materialised **lazily** at k-best extraction (the
//! cube-pruning extractor, [`super::super::parse::Parser::kbest`]).
//!
//! The token-keyed constructs (coordination, relatives, appositives, `but not`, the reciprocal) are all
//! packed: WHERE each fires comes from the shared registry `Parser::binary_sites`, which the
//! unpacked CKY iterates too, so the two paths cannot drift. The router
//! ([`super::super::parse::Parser::parse_needs_unpacked`]) diverts only **pied-piping**, a ternary rule
//! this forest has no edge shape for.

use std::collections::BTreeMap;

use super::super::rules::registry::{BinRule, UnaryKind};

use super::super::item::{Combinator, Cost, Item};
use super::super::pretty::unspine;
use crate::nbe::term::Exp;

/// A packing **signature**: a category key, the Eisner normal-form provenance, and the two derivation
/// properties a rule decision consults (`is_coord` and `is_designation`, below). Two items share a node
/// iff they share a `Sig` — the equivalence class that behaves identically under all future combination.
///
/// For **index-independent** categories the key is the type-index-erased [`cat_shape`] (the coarse key
/// that collapses the sense-product — the packing win). For categories whose combinability is
/// index-DEPENDENT (a concrete selectional argument slot,
/// [`super::super::category::cat_has_selectional_slot`]) the key keeps the indices ([`cat_key`], prefixed
/// `sel:`), so e.g. two object type-raised GQs of different classes never share a node — the small
/// unpacked residue of per-cell packing (D63 §11 3d).
pub(crate) type Sig = (String, Combinator, bool, bool);

/// **The packing invariant**: a `Sig` must capture EVERY property a future combination decision can
/// consult. Two items sharing a `Sig` are decided identically, which is what licenses the packed CKY
/// to decide a node-pair ONCE on representative items ([`PNode::rep`]).
///
/// The categorial rules ([`super::item::combinable`]) are sem-blind by construction — they receive
/// only a [`super::item::CategoryPayload`] — so for them `(cat_key, prov)` suffices. But two of the
/// **token-keyed** rules do consult a sem in their DECISION, and both consult the same predicate:
///
/// - coordination ([`super::super::rules::constructions::coordinate_prop`]) refuses an operand that is already a
///   *completed* coordination (the left-branching normal form), and
/// - `but not` (`BinRule::ButNot`) refuses a coordinated right operand for the same reason.
///
/// A completed coordination is **indistinguishable by category** — `complete_coord` folds a `cat_coord`
/// back into its base category, so `HeLa affects BRCA1` and `HeLa affects BRCA1 and HeLa affects BRCA1`
/// are both `cat_s`. So the predicate ([`super::super::rules::constructions::sem_is_coordination`]) is carried in the
/// signature: a coordination-sem item and a plain item never share a node, and the representative
/// decision stays exact. Without this bit, deciding those rules on a representative would drop an edge
/// that every *other* item in the node needed — silently, and only on sentences that put both kinds of
/// sem in one cell.
///
/// **`is_designation` is the second such bit** (2026-07-26). The `DefiniteDesignation` shift
/// ([`super::super::rules::constructions::definite_designation`]) fires on a noun whose Σ-restrictor is
/// a NAMING, `cat_n(Σx:C. named(x, d), num)` — a property of the type INDEX, which `cat_shape` erases
/// by design. So `gene MSH2` (naming-refined) and `gene MSH2` read as a compound (`compound_kind`-
/// refined) collapsed into one node, the shift was decided once on whichever was the representative,
/// and when that was the compound the apposition item lost its `cat_np` edge outright. Measured, packed
/// vs unpacked on the same index: `Project Achilles affects cells.` → 0 readings packed / 4 unpacked;
/// `WRN affects project Achilles.` → 0 apposition readings packed / 8 unpacked. Carrying the bit
/// restores the exactness the invariant above demands.
pub(crate) fn node_sig(it: &Item) -> Sig {
    let key = if super::super::category::cat_has_selectional_slot(it.cat()) {
        format!("sel:{}", cat_key(it.cat()))
    } else {
        cat_shape(it.cat())
    };
    (
        key,
        it.prov(),
        super::super::rules::constructions::sem_is_coordination(it.sem()),
        super::super::rules::constructions::definite_designation(it.cat()).is_some(),
    )
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

/// A packed forest node: all derivations of one `(span, Sig)` equivalence class.
pub(crate) struct PNode {
    /// The token span `(i, j)` this node covers (inclusive) — needed to re-freshen span-pure holes
    /// (`$quant$i_j` / `$anaphor$i_j`) when a [`Edge::Unary`] transform is materialised at extraction.
    pub span: (usize, usize),
    /// A representative item — used to decide node-level combinability (`apply` on reps) and to carry
    /// the result category for signature computation. Sound under index-independence: every item in
    /// the node combines identically, so any representative gives the correct edge + result `Sig`.
    pub rep: Item,
    /// The cell's [`RightContext`](super::super::rules::RightContext) — what token follows this span.
    /// Stored on the NODE rather than recomputed at extraction because the extractor has no token
    /// stream, and because storing it makes it impossible for the build pass and the extraction pass to
    /// disagree. A property of the span, hence identical for every item in the node — which is why a
    /// rule may consult it and still be decided on `rep`.
    pub rctx: super::super::rules::RightContext,
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

    /// The node for `sig` at cell `[i][j]`, created (with representative `rep` and the cell's right
    /// context) if absent. Returns its [`NodeId`]. The `rep` of an existing node is kept (the
    /// first-seen representative); `rctx` is a function of `j` alone, so every caller for a given cell
    /// supplies the same value.
    pub fn get_or_create(
        &mut self,
        i: usize,
        j: usize,
        sig: Sig,
        rep: &Item,
        rctx: super::super::rules::RightContext,
    ) -> NodeId {
        if let Some(&id) = self.cells[i][j].get(&sig) {
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(PNode {
            span: (i, j),
            rep: rep.clone(),
            rctx,
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

/// Render a category's **structural shape** — `pretty_term` with every type INDEX
/// (an `EigonClass` / `EigonResource` / refined `Σ`) erased to `_`, while keeping the
/// constructor spine (`cat_n`, `cat_np`, `cat_s`, `fwd`, `bwd`, `cat_forall`) and the
/// FEATURE atoms (`sg`/`pl`/`num_any`/`mass`/`dcl`/`fin`/…). So `cat_n(wn:n123, sg)`
/// and `cat_n(umlscui:C1, sg)` both render `cat_n(_, sg)`.
///
/// **This is a CANONICAL KEY, not a display string.** It defines half of the packing signature
/// ([`node_sig`]): two items whose `cat_shape` collides SHARE A NODE and are decided as one. Changing
/// what it prints changes which derivations the packed forest keeps. It lived in `pretty.rs` next to
/// the pretty-printer, described as "diagnostic" — which it was, before `node_sig` reached for the
/// nearest `Exp → String` function. A formatting "improvement" there would have been a silent
/// soundness change here.
///
/// (It is still *also* the chart-cell histogram key — `super::cell_histogram` — which is fine: a
/// canonical key makes a good histogram bucket. The reverse inference is what was dangerous.)
pub(crate) fn cat_shape(e: &Exp) -> String {
    match e {
        Exp::App(_, _) => {
            let (head, args) = unspine(e);
            if args.is_empty() {
                cat_shape(head)
            } else {
                let inner = args
                    .iter()
                    .map(|a| cat_shape(a))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({inner})", cat_shape(head))
            }
        }
        Exp::Con(name, body) => format!("{name}({})", cat_shape(body)),
        Exp::InductiveCtor(_, name, args) => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner = args.iter().map(cat_shape).collect::<Vec<_>>().join(", ");
                format!("{name}({inner})")
            }
        }
        Exp::InductiveType(decl, args) => {
            if args.is_empty() {
                decl.name.clone()
            } else {
                let inner = args.iter().map(cat_shape).collect::<Vec<_>>().join(", ");
                format!("{}({inner})", decl.name)
            }
        }
        // Type indices / sense identities — erased.
        Exp::EigonClass(_) | Exp::EigonResource(_) | Exp::EigonAxiom(_) => "_".to_string(),
        // A refined noun's index is a `Σ`; keep the shape marker, erase the components.
        Exp::Sig(_, _, _) | Exp::Times(_, _) => "Σ_".to_string(),
        // A determiner category `cat_forall(num, λT. body)` carries its GQ body as a `Lam`. The body
        // SHAPE (subject GQ `S/(S\NP)` vs object GQ `(S\NP)\((S\NP)/NP)`) decides combinability, so it
        // MUST be kept (only the bound type-index `T` erases, via the `Var` arm) — else the packing
        // signature `(cat_shape, prov)` is not a combinability congruence and packs distinct
        // determiner readings into one node (D63 §11 3d).
        Exp::Lam(_, body) => format!("λ.{}", cat_shape(body)),
        Exp::Arrow(a, b) => format!("{} → {}", cat_shape(a), cat_shape(b)),
        Exp::Pi(_, a, b) => format!("{} → {}", cat_shape(a), cat_shape(b)),
        Exp::Var(_) => "_".to_string(),
        Exp::Sort(0) => "Prop".to_string(),
        Exp::Sort(1) => "Set".to_string(),
        // Sems (a leaf's `sem`, not a cat) collapse — we only shape categories.
        _ => "_".to_string(),
    }
}

/// A packing key that — unlike [`cat_shape`] — **keeps** every type index: the concrete `EigonClass`
/// IRIs in argument slots, and refined-noun Σ components. Used only for categories whose
/// combinability is index-DEPENDENT ([`super::category::cat_has_selectional_slot`]), where `cat_shape`
/// is too coarse: it erases the argument class, so two object type-raised GQs
/// `(S\NP)\((S\NP)/cat_np(gene))` and `…/cat_np(cell)` would share a node, and the packed forest's
/// representative-based edge decision would silently drop the non-representative's combinations
/// (D63 §11 3d — per-cell packing). Keying such items by the full category makes them merge only when
/// identical, so their (small) residue stays exact while the index-independent majority still packs by
/// `cat_shape`. Like `cat_shape`, it never inlines an `InductiveType` declaration (bounded output).
pub(crate) fn cat_key(e: &Exp) -> String {
    match e {
        Exp::App(_, _) => {
            let (head, args) = unspine(e);
            if args.is_empty() {
                cat_key(head)
            } else {
                let inner = args
                    .iter()
                    .map(|a| cat_key(a))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({inner})", cat_key(head))
            }
        }
        Exp::Con(name, body) => format!("{name}({})", cat_key(body)),
        Exp::InductiveCtor(_, name, args) => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner = args.iter().map(cat_key).collect::<Vec<_>>().join(", ");
                format!("{name}({inner})")
            }
        }
        Exp::InductiveType(decl, args) => {
            if args.is_empty() {
                decl.name.clone()
            } else {
                let inner = args.iter().map(cat_key).collect::<Vec<_>>().join(", ");
                format!("{}({inner})", decl.name)
            }
        }
        // KEEP the type index — the whole point of this key over `cat_shape`. Full IRI (not `local`),
        // so two classes that share a local name across namespaces never collide into one node.
        Exp::EigonClass(iri) | Exp::EigonAxiom(iri) => iri.as_str().to_string(),
        Exp::EigonResource(r) => r
            .id()
            .map(|i| i.as_str().to_string())
            .unwrap_or_else(|| "<resource>".to_string()),
        // KEEP the refined-noun Σ components — they determine the result of a combination.
        Exp::Sig(_, a, b) => format!("Σ({}, {})", cat_key(a), cat_key(b)),
        Exp::Times(a, b) => format!("×({}, {})", cat_key(a), cat_key(b)),
        Exp::Lam(_, body) => format!("λ.{}", cat_key(body)),
        Exp::Arrow(a, b) => format!("{} → {}", cat_key(a), cat_key(b)),
        Exp::Pi(_, a, b) => format!("{} → {}", cat_key(a), cat_key(b)),
        Exp::Var(n) => n.clone(),
        Exp::Sort(0) => "Prop".to_string(),
        Exp::Sort(1) => "Set".to_string(),
        _ => "_".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::{list_decl, Exp};
    use crate::ontology::iri::Iri;
    use std::sync::Arc;

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

    /// The **packing invariant** (see [`node_sig`]): a `Sig` must capture every property a future
    /// combination DECISION consults. Two of the token-keyed rules (`Coordinate`, `ButNot`) reject an
    /// operand whose SEM is a completed coordination — and a completed coordination is
    /// indistinguishable by category (`complete_coord` folds a `cat_coord` back into its base cat, so
    /// a conjoined `S` and a plain `S` are both `cat_s`). So the coordination-sem bit rides in the
    /// signature: without it those two items would share a node, the packed path would decide the rule
    /// ONCE on whichever happened to be the representative, and the edge every other item in the node
    /// needed would be silently dropped.
    #[test]
    fn node_sig_separates_a_coordination_sem_from_a_plain_one_at_the_same_cat() {
        let and_decl = Arc::new(crate::nbe::term::InductiveDecl {
            iri: Iri::parse("urn:eigenius:logic:And").unwrap(),
            name: "And".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(0),
            ctors: Vec::new(),
        });
        let s_cat = ctor("cat_s", vec![ctor("dcl", vec![]), ctor("fin", vec![])]);
        // Same category, same provenance — but one sem is an `And` (a completed coordination).
        let plain = Item::from_parts(s_cat.clone(), Exp::Unit, Combinator::Other, Cost::ZERO);
        let conjoined = Item::from_parts(
            s_cat,
            Exp::InductiveType(and_decl, vec![Exp::Unit, Exp::Unit]),
            Combinator::Other,
            Cost::ZERO,
        );
        assert_ne!(
            node_sig(&plain),
            node_sig(&conjoined),
            "a coordination sem and a plain sem at the SAME cat_shape must NOT share a node — \
             `Coordinate` / `ButNot` decide on this predicate, so it belongs in the Sig"
        );
        // The bit is the only difference: category key, provenance and the designation bit are identical.
        let (kp, pp, cp, dp) = node_sig(&plain);
        let (kc, pc, cc, dc) = node_sig(&conjoined);
        assert_eq!(
            (kp, pp, dp),
            (kc, pc, dc),
            "only the coordination bit should differ"
        );
        assert!(!cp && cc, "the coordination bit must track the sem");
    }

    /// The packing invariant again, for the SECOND sem/index-derived bit. `DefiniteDesignation` fires on
    /// a naming-refined noun, and `cat_shape` erases the type index that distinguishes one from a
    /// compound-refined noun at the same shape — so without `is_designation` in the `Sig` the two pack
    /// together and the shift is decided once, on whichever is the representative. That silently cost
    /// every bare classifier+designator its NP: `Project Achilles affects cells.` read 0 packed against
    /// 4 unpacked.
    #[test]
    fn node_sig_separates_a_naming_refined_noun_from_a_compound_refined_one() {
        let x = "G#0";
        let refined = |restr_axiom: &str| {
            let restr = Exp::App(
                Box::new(Exp::App(
                    Box::new(Exp::EigonAxiom(Iri::parse(restr_axiom).unwrap())),
                    Box::new(Exp::Var(x.into())),
                )),
                Box::new(Exp::EigonAxiom(
                    Iri::parse("urn:eigenius:lexicon:msh2").unwrap(),
                )),
            );
            let sig_ty = Exp::Sig(
                crate::nbe::term::Patt::Var(x.into()),
                Box::new(cls("urn:eigenius:lexicon:Gene")),
                Box::new(restr),
            );
            leaf(Exp::InductiveCtor(
                crate::nbe::term::list_decl(),
                "cat_n".into(),
                vec![
                    sig_ty,
                    Exp::InductiveCtor(crate::nbe::term::list_decl(), "sg".into(), vec![]),
                ],
            ))
        };
        let named = refined("urn:eigenius:ontology:named");
        let compound = refined("urn:eigenius:ontology:compound_kind");
        assert_eq!(
            cat_shape(named.cat()),
            cat_shape(compound.cat()),
            "the two are INDISTINGUISHABLE by cat_shape — that is what makes the bit necessary"
        );
        assert_ne!(
            node_sig(&named),
            node_sig(&compound),
            "a naming-refined noun and a compound-refined one must NOT share a node — \
             `DefiniteDesignation` decides on that index property"
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
        let id_a = f.get_or_create(
            0,
            0,
            node_sig(&a),
            &a,
            crate::dcg::rules::RightContext::Other,
        );
        let id_b = f.get_or_create(
            0,
            0,
            node_sig(&b),
            &b,
            crate::dcg::rules::RightContext::Other,
        );
        assert_eq!(id_a, id_b, "same Sig ⇒ same node");
        let id_n = f.get_or_create(
            0,
            0,
            node_sig(&noun),
            &noun,
            crate::dcg::rules::RightContext::Other,
        );
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
