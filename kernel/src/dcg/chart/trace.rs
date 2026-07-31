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
//! **Forest derivation tracing** — walk the packed shared forest's [`Edge`] DAG and render *how* each
//! node was built, not just *what* it is. The cell dump (`EIGENIUS_DUMP_CELL`) prints a flat bag of
//! items per cell with no derivation structure; this prints the **hyperedge tree**: for each node,
//! which child cells combine, under which rule (`Combine` / `Unary(kind)` / `Binary(rule)` / `Leaf`),
//! and at which split point — so an investigation can see which derivations a given cell admits and,
//! by comparing a base-cap run against a widened one, which edges multiword span-integrity
//! (`protected_split`) removed.
//!
//! This is the reusable INSTRUMENT: pure over [`Forest`] (grammar data) + [`pretty_term`], with no
//! lexicon and no felicity. WHEN to call it is the parser's decision (the `EIGENIUS_TRACE_FOREST`
//! trigger in `parse_packed_at_cap`), which also supplies the context header (cap, `prefer_multiword`,
//! the `protected_split` vector).
//!
//! **Caveat — reps, not readings.** A packed node stores one *representative* item; the differing
//! sems are materialised lazily at k-best. So the tree shows the derivation SKELETON and the rep's
//! sem, not every reading the node packs. That is exactly right for structural questions ("which
//! split builds this constituent, and is it protected"); it is not a substitute for k-best when the
//! question is about a specific materialised sem.
//!
//! ## `EIGENIUS_TRACE_FOREST` grammar
//!
//! ```text
//! spec     := mode (";" modifier)*
//! mode     := "all"            # one line per node, every cell, with its edges
//!           | "cell:" i ".." j # same, restricted to cell [i..j]
//!           | "deriv:" i ".." j# full derivation TREE of every node at cell [i..j]
//!           | "top"            # derivation tree of the top-span finite-clause / question nodes
//! modifier := "cat:" substr   # keep only nodes whose pretty category contains `substr`
//!           | "depth:" N       # max tree depth for deriv/top (default 60)
//! ```

use std::collections::BTreeSet;

use super::super::pretty::pretty_term;
use super::forest::{Edge, Forest, NodeId};

/// Which nodes a skeleton dump keeps.
pub(crate) struct TraceFilter {
    /// Restrict to this cell `[i..j]` (inclusive). `None` = every cell.
    pub span: Option<(usize, usize)>,
    /// Keep only nodes whose `pretty_term(cat)` contains this substring. `None` = no category filter.
    pub cat_contains: Option<String>,
}

/// The token text of span `[i..j]`, space-joined — for reading a node against the sentence.
fn span_text(tokens: &[String], i: usize, j: usize) -> String {
    tokens
        .get(i..=j.min(tokens.len().saturating_sub(1)))
        .map(|s| s.join(" "))
        .unwrap_or_default()
}

impl Forest {
    /// `[a..b]` tag for a node's span — used to annotate an edge's child references.
    fn span_tag(&self, id: NodeId) -> String {
        let (a, b) = self.nodes[id].span;
        format!("[{a}..{b}]")
    }

    /// A compact one-line description of one edge, naming the child nodes it consumes (with their
    /// spans) and — for a binary `Combine` — the split point `k` (`left = [i..k]`, `right =
    /// [k+1..j]`), which is exactly the index `protected_split[k]` a base-cap run may have removed.
    fn edge_brief(&self, e: &Edge) -> String {
        match e {
            Edge::Leaf(_) => "Leaf".to_string(),
            Edge::Combine { left, right } => format!(
                "Combine@k={} #{left}{} + #{right}{}",
                self.nodes[*left].span.1,
                self.span_tag(*left),
                self.span_tag(*right),
            ),
            Edge::Unary { child, kind } => {
                format!("Unary({kind:?}) #{child}{}", self.span_tag(*child))
            }
            Edge::Binary { left, right, rule } => format!(
                "Binary({rule:?}) #{left}{} + #{right}{}",
                self.span_tag(*left),
                self.span_tag(*right),
            ),
        }
    }

    /// One SKELETON line for a node: id, span (+ its token text), pretty category, provenance, and the
    /// brief of every edge. Many nodes, one line each — the map of what a cell admits.
    fn skeleton_line(&self, tokens: &[String], id: NodeId) -> String {
        let node = &self.nodes[id];
        let (i, j) = node.span;
        let edges: Vec<String> = node.edges.iter().map(|e| self.edge_brief(e)).collect();
        format!(
            "#{id} [{i}..{j}] «{}» cat={} prov={:?} edges={}: {}",
            span_text(tokens, i, j),
            pretty_term(node.rep.cat()),
            node.rep.prov(),
            node.edges.len(),
            edges.join(" | "),
        )
    }

    /// Render the forest as one skeleton line per node, cells in `(len, i)` order, filtered by
    /// `filter`. See [`Self::skeleton_line`].
    pub(crate) fn render_skeleton(&self, tokens: &[String], filter: &TraceFilter) -> String {
        let n = self.cells.len();
        let mut out = String::new();
        for len in 1..=n {
            for i in 0..=n.saturating_sub(len) {
                let j = i + len - 1;
                if matches!(filter.span, Some(s) if s != (i, j)) {
                    continue;
                }
                for &id in self.cells[i][j].values() {
                    let cat = pretty_term(self.nodes[id].rep.cat());
                    if matches!(&filter.cat_contains, Some(sub) if !cat.contains(sub.as_str())) {
                        continue;
                    }
                    out.push_str(&self.skeleton_line(tokens, id));
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Render a node's derivations as an indented TREE, expanding child nodes recursively down to
    /// leaves. DAG-safe: a node reached a second time prints its header with `↑(shown above)` and is
    /// not re-expanded (the forest is a shared DAG; this both bounds the output and makes the sharing
    /// visible). Bounded by `max_depth`. Each node line carries its span text, category, and the
    /// representative sem (the load-bearing detail for a structural read).
    pub(crate) fn render_derivation(
        &self,
        tokens: &[String],
        root: NodeId,
        max_depth: usize,
    ) -> String {
        let mut out = String::new();
        let mut visited = BTreeSet::new();
        self.render_node(tokens, root, 0, max_depth, &mut visited, &mut out);
        out
    }

    fn render_node(
        &self,
        tokens: &[String],
        id: NodeId,
        depth: usize,
        max_depth: usize,
        visited: &mut BTreeSet<NodeId>,
        out: &mut String,
    ) {
        let indent = "  ".repeat(depth);
        let node = &self.nodes[id];
        let (i, j) = node.span;
        let head = format!(
            "#{id} [{i}..{j}] «{}» cat={} sem={}",
            span_text(tokens, i, j),
            pretty_term(node.rep.cat()),
            pretty_term(node.rep.sem()),
        );
        if visited.contains(&id) {
            out.push_str(&format!("{indent}{head}  ↑(shown above)\n"));
            return;
        }
        visited.insert(id);
        out.push_str(&format!("{indent}{head}\n"));
        if depth >= max_depth {
            out.push_str(&format!("{indent}  … (max depth)\n"));
            return;
        }
        for e in &node.edges {
            out.push_str(&format!("{indent}  ├ {}\n", self.edge_brief(e)));
            for child in edge_children(e) {
                self.render_node(tokens, child, depth + 2, max_depth, visited, out);
            }
        }
    }
}

/// The child node ids an edge consumes (a `Leaf` has none).
fn edge_children(e: &Edge) -> Vec<NodeId> {
    match e {
        Edge::Leaf(_) => Vec::new(),
        Edge::Combine { left, right } | Edge::Binary { left, right, .. } => vec![*left, *right],
        Edge::Unary { child, .. } => vec![*child],
    }
}

/// Parse an `i..j` span, bounded by `n` (the token count). `None` if malformed or out of range —
/// callers turn that into the usage line rather than a silent empty dump.
fn parse_span(s: &str, n: usize) -> Option<(usize, usize)> {
    let (a, b) = s.split_once("..")?;
    let (i, j) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (i <= j && j < n).then_some((i, j))
}

fn usage() -> String {
    "EIGENIUS_TRACE_FOREST usage: all | cell:i..j | deriv:i..j | top  [;cat:<substr>] [;depth:N]\n"
        .to_string()
}

/// Render the view of `forest` requested by `spec` (the `EIGENIUS_TRACE_FOREST` value), prefixed by
/// the caller's `header` (cap / `prefer_multiword` / `protected_split` context). `top` is the
/// top-span finite-clause / question nodes the parser already computed (only the `top` mode reads it).
/// An unrecognised or out-of-range `spec` renders the usage line — a mistyped trace request says so
/// rather than silently printing nothing.
pub(crate) fn forest_trace(
    forest: &Forest,
    tokens: &[String],
    top: &[NodeId],
    spec: &str,
    header: &str,
) -> String {
    let n = tokens.len();
    let mut out = String::new();
    out.push_str(header);
    out.push('\n');

    let mut parts = spec.split(';');
    let mode = parts.next().unwrap_or("").trim();
    let mut cat_contains: Option<String> = None;
    let mut max_depth = 60usize;
    for m in parts {
        let m = m.trim();
        if let Some(sub) = m.strip_prefix("cat:") {
            cat_contains = Some(sub.to_string());
        } else if let Some(d) = m.strip_prefix("depth:") {
            if let Ok(v) = d.parse() {
                max_depth = v;
            }
        }
    }

    if mode == "all" {
        out.push_str(&forest.render_skeleton(
            tokens,
            &TraceFilter {
                span: None,
                cat_contains,
            },
        ));
    } else if let Some(sp) = mode.strip_prefix("cell:") {
        match parse_span(sp, n) {
            Some(span) => out.push_str(&forest.render_skeleton(
                tokens,
                &TraceFilter {
                    span: Some(span),
                    cat_contains,
                },
            )),
            None => out.push_str(&usage()),
        }
    } else if let Some(sp) = mode.strip_prefix("deriv:") {
        match parse_span(sp, n) {
            Some((i, j)) => {
                let ids: Vec<NodeId> = forest.cells[i][j].values().copied().collect();
                if ids.is_empty() {
                    out.push_str(&format!("(cell [{i}..{j}] is empty)\n"));
                }
                for id in ids {
                    out.push_str(&forest.render_derivation(tokens, id, max_depth));
                }
            }
            None => out.push_str(&usage()),
        }
    } else if mode == "top" {
        if top.is_empty() {
            out.push_str(
                "(no top finite-clause / question node — the sentence gaps at this cap)\n",
            );
        }
        for &id in top {
            out.push_str(&forest.render_derivation(tokens, id, max_depth));
        }
    } else {
        out.push_str(&usage());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::super::item::{Combinator, Cost, Item};
    use super::super::super::rules::registry::UnaryKind;
    use super::super::forest::node_sig;
    use super::*;
    use crate::nbe::term::{list_decl, Exp};
    use crate::ontology::iri::Iri;

    fn ctor(name: &str, args: Vec<Exp>) -> Exp {
        Exp::InductiveCtor(list_decl(), name.into(), args)
    }
    fn cls(iri: &str) -> Exp {
        Exp::EigonClass(Iri::parse(iri).unwrap())
    }
    fn cat_n(iri: &str) -> Exp {
        ctor("cat_n", vec![cls(iri), ctor("sg", vec![])])
    }
    fn leaf(cat: Exp, sem: Exp) -> Item {
        Item::from_parts(cat, sem, Combinator::Other, Cost::ZERO)
    }

    /// Build a 3-token forest: two leaves at `[0..0]` and `[1..2]` (a multiword `cat_n`), with a
    /// `Combine` node at `[0..2]`. `tokens = ["a", "cell", "line"]`.
    fn tiny_forest() -> (Forest, Vec<String>) {
        let tokens: Vec<String> = ["a", "cell", "line"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut f = Forest::new(3);
        let det = leaf(ctor("cat_forall", vec![ctor("sg", vec![])]), Exp::Unit);
        let mw = leaf(
            cat_n("urn:eigenius:umlscui:C1"),
            cls("urn:eigenius:umlscui:C1"),
        );
        let det_id = f.get_or_create(
            0,
            0,
            node_sig(&det),
            &det,
            crate::dcg::rules::RightContext::Other,
        );
        f.push_edge(det_id, Edge::Leaf(det.clone()));
        let mw_id = f.get_or_create(
            1,
            2,
            node_sig(&mw),
            &mw,
            crate::dcg::rules::RightContext::Other,
        );
        f.push_edge(mw_id, Edge::Leaf(mw.clone()));
        // A combined node spanning [0..2].
        let np = leaf(
            ctor("cat_np", vec![cls("urn:eigenius:umlscui:C1")]),
            Exp::Unit,
        );
        let np_id = f.get_or_create(
            0,
            2,
            node_sig(&np),
            &np,
            crate::dcg::rules::RightContext::Other,
        );
        f.push_edge(
            np_id,
            Edge::Combine {
                left: det_id,
                right: mw_id,
            },
        );
        (f, tokens)
    }

    #[test]
    fn skeleton_lists_every_node_with_its_edges() {
        let (f, tokens) = tiny_forest();
        let s = f.render_skeleton(
            &tokens,
            &TraceFilter {
                span: None,
                cat_contains: None,
            },
        );
        // The multiword leaf, with its span text and a Leaf edge.
        assert!(s.contains("[1..2] «cell line»"), "skeleton: {s}");
        assert!(s.contains("edges=1: Leaf"), "leaf edge brief: {s}");
        // The combined node names the split point and both children.
        assert!(
            s.contains("Combine@k=0 #0[0..0] + #1[1..2]"),
            "combine brief with split k: {s}"
        );
    }

    #[test]
    fn skeleton_span_filter_restricts_to_one_cell() {
        let (f, tokens) = tiny_forest();
        let s = f.render_skeleton(
            &tokens,
            &TraceFilter {
                span: Some((1, 2)),
                cat_contains: None,
            },
        );
        assert!(s.contains("[1..2]"), "kept the target cell: {s}");
        assert!(!s.contains("[0..0]"), "filtered other cells: {s}");
        assert!(!s.contains("[0..2]"), "filtered other cells: {s}");
    }

    #[test]
    fn skeleton_cat_filter_matches_substring() {
        let (f, tokens) = tiny_forest();
        let s = f.render_skeleton(
            &tokens,
            &TraceFilter {
                span: None,
                cat_contains: Some("cat_forall".to_string()),
            },
        );
        assert!(s.contains("cat_forall"), "kept the determiner: {s}");
        assert!(!s.contains("cat_np"), "dropped the non-matching nodes: {s}");
    }

    #[test]
    fn derivation_expands_children_to_leaves() {
        let (f, tokens) = tiny_forest();
        // The combined node is #2 ([0..2]); expand it.
        let s = f.render_derivation(&tokens, 2, 60);
        assert!(s.contains("#2 [0..2]"), "root: {s}");
        assert!(s.contains("├ Combine@k=0"), "edge line: {s}");
        assert!(s.contains("#0 [0..0] «a»"), "left child expanded: {s}");
        assert!(
            s.contains("#1 [1..2] «cell line»"),
            "right child expanded: {s}"
        );
    }

    #[test]
    fn derivation_dedups_a_reshared_node() {
        // A node whose two edges both reference the same child prints the child once, then `↑`.
        let tokens: Vec<String> = ["x", "y"].iter().map(|s| s.to_string()).collect();
        let mut f = Forest::new(2);
        let child = leaf(cat_n("urn:eigenius:umlscui:C9"), Exp::Unit);
        let cid = f.get_or_create(
            0,
            0,
            node_sig(&child),
            &child,
            crate::dcg::rules::RightContext::Other,
        );
        f.push_edge(cid, Edge::Leaf(child.clone()));
        let parent = leaf(
            ctor("cat_np", vec![cls("urn:eigenius:umlscui:C9")]),
            Exp::Unit,
        );
        let pid = f.get_or_create(
            0,
            1,
            node_sig(&parent),
            &parent,
            crate::dcg::rules::RightContext::Other,
        );
        // Two Unary edges onto the same child → the second must be deduped.
        f.push_edge(
            pid,
            Edge::Unary {
                child: cid,
                kind: UnaryKind::Raise,
            },
        );
        f.push_edge(
            pid,
            Edge::Unary {
                child: cid,
                kind: UnaryKind::BareNp,
            },
        );
        let s = f.render_derivation(&tokens, pid, 60);
        assert_eq!(
            s.matches("↑(shown above)").count(),
            1,
            "the reshared child is expanded once then marked: {s}"
        );
    }

    #[test]
    fn forest_trace_top_reports_a_gap_when_empty() {
        let (f, tokens) = tiny_forest();
        let s = forest_trace(&f, &tokens, &[], "top", "HDR");
        assert!(s.starts_with("HDR\n"), "header first: {s}");
        assert!(s.contains("gaps at this cap"), "empty top ⇒ gap note: {s}");
    }

    #[test]
    fn forest_trace_bad_spec_prints_usage() {
        let (f, tokens) = tiny_forest();
        let s = forest_trace(&f, &tokens, &[], "cell:9..9", "HDR");
        assert!(s.contains("usage:"), "out-of-range span ⇒ usage: {s}");
    }
}
