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

//! **Ambiguity attribution** — roll the packed forest's multiplicity up into ranked, NAMED factors, so
//! a unit's reading count reads as "N ≈ struct× · sense× · {which rule / which senses, at which span}"
//! instead of requiring manual λ-term archaeology (dump readings, erase senses, swap-ladder, look up
//! CUIs). Read-only over the forest the parse already built; no parser behaviour change.
//!
//! The forest is an AND-OR graph: a [`super::forest::PNode`] is an OR-node (its `edges` are alternative
//! derivations of one span); an [`Edge`] is an AND-hyperedge naming its rule. So a node with several
//! `Leaf` edges is a **sense** branch (competing senses of one shape); a node with several
//! `Combine`/`Binary`/`Unary` edges is a **structure** branch (competing derivations). This walk finds
//! every branch, labels it, and ranks by branching factor. Design: `docs/notes/dcg-ambiguity-attribution-plan.md`.

use std::collections::{HashMap, HashSet};

use crate::nbe::term::Exp;

use super::super::item::{Combinator, Item};
use super::super::pretty::pretty_term;
use super::super::rules::registry::BinRule;
use super::forest::{Edge, Forest, NodeId};

/// A local ambiguity site — one OR-node that branches.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SiteKind {
    /// Competing SENSES of one lexical shape (several `Leaf` edges).
    Sense,
    /// Competing DERIVATIONS (several rule/split edges).
    Structure,
}

pub(crate) struct Site {
    pub span: (usize, usize),
    pub text: String,
    pub kind: SiteKind,
    /// Number of local alternatives — the multiplicative branching this node introduces.
    pub factor: usize,
    /// Subtree reading count — a coarse impact proxy for ranking (see the note's limitation (a)).
    pub inside: u64,
    /// The competing senses (IRIs) or constructions (rule names), deduped.
    pub labels: Vec<String>,
}

pub(crate) struct UnitAttribution {
    pub readings: u64,
    pub sites: Vec<Site>,
}

impl Forest {
    /// Inside reading count of a node (memoised): OR = Σ over edges, AND = ∏ over an edge's children.
    /// Same recursion `kbest` enumerates; here we only COUNT. `on_stack` breaks a `Unary` same-cell
    /// cycle (treated as 1) so the walk always terminates.
    fn inside_count(
        &self,
        id: NodeId,
        memo: &mut HashMap<NodeId, u64>,
        on_stack: &mut HashSet<NodeId>,
    ) -> u64 {
        if let Some(&r) = memo.get(&id) {
            return r;
        }
        if !on_stack.insert(id) {
            return 1;
        }
        let mut total: u64 = 0;
        for e in &self.nodes[id].edges {
            let er = match e {
                Edge::Leaf(_) => 1,
                Edge::Combine { left, right } | Edge::Binary { left, right, .. } => self
                    .inside_count(*left, memo, on_stack)
                    .saturating_mul(self.inside_count(*right, memo, on_stack)),
                Edge::Unary { child, .. } => self.inside_count(*child, memo, on_stack),
            };
            total = total.saturating_add(er);
        }
        on_stack.remove(&id);
        let total = total.max(1);
        memo.insert(id, total);
        total
    }

    /// Attribute the multiplicity of the readings rooted at `top` to ranked sense/structure sites.
    pub(crate) fn attribute(&self, tokens: &[String], top: &[NodeId]) -> UnitAttribution {
        let mut memo = HashMap::new();
        let mut stack = HashSet::new();
        let readings = top
            .iter()
            .map(|&r| self.inside_count(r, &mut memo, &mut stack))
            .fold(0u64, |a, b| a.saturating_add(b));

        // Nodes actually used in some top derivation.
        let mut reach: HashSet<NodeId> = HashSet::new();
        let mut queue: Vec<NodeId> = top.to_vec();
        while let Some(id) = queue.pop() {
            if !reach.insert(id) {
                continue;
            }
            for e in &self.nodes[id].edges {
                match e {
                    Edge::Leaf(_) => {}
                    Edge::Combine { left, right } | Edge::Binary { left, right, .. } => {
                        queue.push(*left);
                        queue.push(*right);
                    }
                    Edge::Unary { child, .. } => queue.push(*child),
                }
            }
        }

        let mut sites = Vec::new();
        for &id in &reach {
            let node = &self.nodes[id];
            if node.edges.len() < 2 {
                continue;
            }
            let (i, j) = node.span;
            let text = span_text(tokens, i, j);
            let inside = *memo.get(&id).unwrap_or(&1);
            if node.edges.iter().all(|e| matches!(e, Edge::Leaf(_))) {
                let mut labels: Vec<String> = node
                    .edges
                    .iter()
                    .filter_map(|e| match e {
                        Edge::Leaf(it) => Some(sense_label(it.sem())),
                        _ => None,
                    })
                    .collect();
                labels.sort();
                labels.dedup();
                if labels.len() < 2 {
                    continue; // same sense packed twice — not a real branch
                }
                sites.push(Site {
                    span: (i, j),
                    text,
                    kind: SiteKind::Sense,
                    factor: labels.len(),
                    inside,
                    labels,
                });
            } else {
                let mut labels: Vec<String> = node
                    .edges
                    .iter()
                    .map(|e| edge_label(e, &node.rep))
                    .collect();
                labels.sort();
                labels.dedup();
                sites.push(Site {
                    span: (i, j),
                    text,
                    kind: SiteKind::Structure,
                    factor: node.edges.len(),
                    inside,
                    labels,
                });
            }
        }
        // Biggest branch first; tie-break by impact proxy, then wider span.
        sites.sort_by(|a, b| {
            b.factor
                .cmp(&a.factor)
                .then(b.inside.cmp(&a.inside))
                .then((b.span.1 - b.span.0).cmp(&(a.span.1 - a.span.0)))
        });
        UnitAttribution { readings, sites }
    }
}

impl UnitAttribution {
    /// One-block report: the raw-forest path count, its structure×/sense× upper bounds, and the top
    /// branch sites. The counts are PRE-FELICITY (the whole forest, before the top-span type-check and
    /// reranking prune it to the extracted readings) — so they over-approximate; the per-site list is
    /// the faithful part (sense sites especially). Returns `None` when there is nothing to attribute.
    pub(crate) fn render(&self, sentence: &str) -> Option<String> {
        if self.sites.is_empty() {
            return None;
        }
        let struct_prod: u64 = self
            .sites
            .iter()
            .filter(|s| s.kind == SiteKind::Structure)
            .map(|s| s.factor as u64)
            .product::<u64>()
            .max(1);
        let sense_prod: u64 = self
            .sites
            .iter()
            .filter(|s| s.kind == SiteKind::Sense)
            .map(|s| s.factor as u64)
            .product::<u64>()
            .max(1);
        let mut out = format!(
            "=== ATTRIBUTION (raw forest, pre-felicity) «{sentence}» ===\n  {} raw paths ≈ \
             structure×{struct_prod} · sense×{sense_prod} (upper bounds; felicity + ranking prune to \
             the extracted count)\n",
            self.readings
        );
        for s in self.sites.iter().take(12) {
            let kind = match s.kind {
                SiteKind::Sense => "SENSE ",
                SiteKind::Structure => "STRUCT",
            };
            out.push_str(&format!(
                "  {kind} [{}..{}] «{}» ×{} : {}\n",
                s.span.0,
                s.span.1,
                s.text,
                s.factor,
                s.labels.join(" | "),
            ));
        }
        Some(out)
    }
}

fn span_text(tokens: &[String], i: usize, j: usize) -> String {
    tokens
        .get(i..=j.min(tokens.len().saturating_sub(1)))
        .map(|s| s.join(" "))
        .unwrap_or_default()
}

/// The construction label of one structural edge — from the edge's own rule where named, else from the
/// node's `Combinator` provenance, else (for the lumped `Compound`) refined by the restrictor shape.
fn edge_label(e: &Edge, rep: &Item) -> String {
    match e {
        Edge::Leaf(_) => "leaf".to_string(),
        Edge::Unary { kind, .. } => format!("{kind:?}"),
        Edge::Binary { rule, .. } => match rule {
            BinRule::Coordinate(op) => {
                format!("coord({})", op.rsplit(':').next().unwrap_or(op))
            }
            other => format!("{other:?}"),
        },
        Edge::Combine { .. } => match rep.prov() {
            Combinator::Compound => compound_shape_label(rep.sem()),
            Combinator::Modal => "modal-scope".to_string(),
            Combinator::KindRaised => "kind-shift".to_string(),
            Combinator::TypeRaised => "type-raise".to_string(),
            Combinator::ForwardApp | Combinator::BackwardApp => "apply".to_string(),
            Combinator::ForwardComp | Combinator::BackwardComp | Combinator::CrossedComp => {
                "compose".to_string()
            }
            other => format!("{other:?}"),
        },
    }
}

/// Split the lumped `Combinator::Compound` by the refined noun's restrictor shape (the note's §3):
/// `compound_kind`/`compound` → compound-bracket, `measurements:gt`/`lt` → adjective, `prep_*` → PP,
/// `is_a` → essive. The one place a label is DERIVED (from the sem), not read off an edge.
fn compound_shape_label(sem: &Exp) -> String {
    let Exp::Sig(_, _, body) = sem else {
        return "nominal-mod".to_string();
    };
    let mut conjuncts = Vec::new();
    flatten_and(body, &mut conjuncts);
    let mut classes: Vec<&str> = conjuncts
        .iter()
        .map(|c| axiom_class(spine_head(c)))
        .collect();
    classes.sort_unstable();
    classes.dedup();
    classes.retain(|c| *c != "other");
    if classes.is_empty() {
        "nominal-mod".to_string()
    } else {
        classes.join("+")
    }
}

fn flatten_and<'a>(e: &'a Exp, out: &mut Vec<&'a Exp>) {
    if let Exp::InductiveType(decl, args) = e {
        if decl.iri.as_str() == "urn:eigenius:logic:And" && args.len() == 2 {
            flatten_and(&args[0], out);
            flatten_and(&args[1], out);
            return;
        }
    }
    out.push(e);
}

/// The predicate an App-spine ultimately applies, descending the annotation + binder a modifier's
/// un-reduced `(λx. P(x)) x` carries (mirrors `combinators::is_adjective_refined`).
fn spine_head(mut e: &Exp) -> &Exp {
    loop {
        match e {
            Exp::App(f, _) => e = f,
            Exp::Ann(inner, _) => e = inner,
            Exp::Lam(_, body) => e = body,
            _ => return e,
        }
    }
}

fn axiom_class(head: &Exp) -> &'static str {
    match head {
        Exp::EigonAxiom(iri) => {
            let s = iri.as_str();
            if s == "urn:eigenius:ontology:compound" || s == "urn:eigenius:ontology:compound_kind" {
                "compound"
            } else if s == "urn:eigenius:ontology:named" {
                "named"
            } else if s == "urn:eigenius:ontology:is_a" {
                "essive"
            } else if s.starts_with("urn:eigenius:ontology:prep_") {
                "pp"
            } else if s == "urn:eigenius:measurements:gt" || s == "urn:eigenius:measurements:lt" {
                "adjective"
            } else {
                "other"
            }
        }
        _ => "other",
    }
}

fn sense_label(sem: &Exp) -> String {
    let s = pretty_term(sem);
    let short: String = s.chars().take(30).collect();
    if s.chars().count() > 30 {
        format!("{short}…")
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    //! The DERIVED labels — the one place a construction name is computed from the sem rather than
    //! read off an edge (the note's §3). The forest-walk itself (`inside_count` / site classification)
    //! is covered by the WRN `--no-llm` sweep differential, which is where a mis-count would show.
    use super::*;
    use crate::nbe::term::Patt;
    use crate::ontology::iri::Iri;

    fn ax(s: &str) -> Exp {
        Exp::EigonAxiom(Iri::parse(s).unwrap())
    }
    fn var() -> Exp {
        Exp::Var("x".into())
    }
    /// `axiom(x, m)` — the restrictor App-spine a modifier leaves.
    fn app2(axiom: &str, m: Exp) -> Exp {
        Exp::App(
            Box::new(Exp::App(Box::new(ax(axiom)), Box::new(var()))),
            Box::new(m),
        )
    }
    /// `Σx:Gene. restr`.
    fn sigma(restr: Exp) -> Exp {
        Exp::Sig(
            Patt::Var("x".into()),
            Box::new(ax("urn:eigenius:lexicon:Gene")),
            Box::new(restr),
        )
    }

    #[test]
    fn compound_label_splits_the_lumped_combinator_by_restrictor_shape() {
        assert_eq!(
            compound_shape_label(&sigma(app2(
                "urn:eigenius:ontology:compound_kind",
                ax("urn:eigenius:lexicon:mmr")
            ))),
            "compound"
        );
        assert_eq!(
            compound_shape_label(&sigma(app2(
                "urn:eigenius:ontology:prep_of",
                ax("urn:eigenius:lexicon:x")
            ))),
            "pp"
        );
        assert_eq!(
            compound_shape_label(&sigma(app2(
                "urn:eigenius:ontology:is_a",
                ax("urn:eigenius:lexicon:x")
            ))),
            "essive"
        );
        // Adjective: the restrictor is the un-reduced `(λx. gt(deg(x), std)) x` under a bidirectional
        // `Ann` — `spine_head` must descend Ann → App → Lam → App-spine to reach `gt`.
        let adj = Exp::Ann(
            Box::new(Exp::App(
                Box::new(Exp::Lam(
                    Patt::Var("x".into()),
                    Box::new(app2(
                        "urn:eigenius:measurements:gt",
                        ax("urn:eigenius:measurements:std"),
                    )),
                )),
                Box::new(var()),
            )),
            Box::new(ax("urn:eigenius:core:Prop")),
        );
        assert_eq!(compound_shape_label(&sigma(adj)), "adjective");
    }

    #[test]
    fn axiom_class_maps_the_known_iris_and_defaults_to_other() {
        assert_eq!(axiom_class(&ax("urn:eigenius:ontology:named")), "named");
        assert_eq!(
            axiom_class(&ax("urn:eigenius:ontology:compound")),
            "compound"
        );
        assert_eq!(
            axiom_class(&ax("urn:eigenius:measurements:lt")),
            "adjective"
        );
        assert_eq!(axiom_class(&ax("urn:eigenius:ontology:prep_in")), "pp");
        assert_eq!(axiom_class(&var()), "other");
    }

    #[test]
    fn span_text_joins_and_clamps() {
        let toks: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(span_text(&toks, 1, 2), "b c");
        assert_eq!(span_text(&toks, 3, 3), "d");
        assert_eq!(span_text(&toks, 2, 99), "c d"); // out-of-range j clamps, no panic
    }
}
