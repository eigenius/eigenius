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

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::layer::Layer;
use crate::nbe::term::Exp;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;

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
    /// Number of local alternatives in the RAW forest — pre-felicity, so an upper bound.
    pub factor: usize,
    /// How many of those alternatives actually occur in a SURVIVING reading. For a sense site this is
    /// the real multiplicity (the felicity intersection); for a structure site it is `factor`, because
    /// intersecting bracketings needs per-reading derivations that `kbest` does not record — so a
    /// structure `factor` must NOT be read as an impact ranking.
    pub felicitous: usize,
    /// Subtree reading count — a coarse impact proxy for ranking (see the note's limitation (a)).
    pub inside: u64,
    /// The competing senses (resolved to name + semantic type where the chain knows them) or the
    /// competing constructions (rule names), deduped.
    pub labels: Vec<String>,
    /// A WordNet sense AND a UMLS sense both SURVIVE at this span. These are the pairs the
    /// WordNet↔UMLS reconciliation either never considered or adjudicated as distinct — and each one
    /// costs a real reading. Reported separately so "should alignment have merged this?" is a list,
    /// not a guess.
    pub cross_lexicon: bool,
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

    /// Attribute the multiplicity of the readings rooted at `top` to sense/structure sites.
    ///
    /// `readings` are the SURVIVING readings (post-felicity, post-dedup) — a sense alternative that
    /// appears in none of them was pruned and is not real multiplicity, so it is excluded from the
    /// site's `felicitous` count. `layer` resolves a sense IRI to its name and semantic type, so the
    /// report reads `C0018905 "Hemagglutination test" [T059]` instead of an opaque CUI.
    pub(crate) fn attribute(
        &self,
        tokens: &[String],
        top: &[NodeId],
        readings: &[Item],
        layer: &Layer,
    ) -> UnitAttribution {
        let reading_atoms: Vec<BTreeSet<String>> = readings
            .iter()
            .map(|r| sense_atoms(&pretty_term(r.sem())))
            .collect();
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
                // One entry per distinct sense: (label, does it survive into a reading?).
                let mut seen: BTreeSet<String> = BTreeSet::new();
                let mut entries: Vec<(String, bool)> = Vec::new();
                let (mut saw_wordnet, mut saw_umls) = (false, false);
                for e in &node.edges {
                    let Edge::Leaf(it) = e else { continue };
                    // Category first: a SURVIVING `N`/`NP` on a surface that is syntactically a verb
                    // (or vice versa) is a mis-categorised lexicon entry that reached a real parse —
                    // a fundamentally wrong reading, not polysemy. That is the signal to look for.
                    let label =
                        format!("{} {}", cat_tag(it.cat()), describe_sense(it.sem(), layer));
                    if !seen.insert(label.clone()) {
                        continue; // same sense packed twice — not a real branch
                    }
                    let atoms = sense_atoms(&pretty_term(it.sem()));
                    // No sense-identifying atom ⇒ cannot discriminate; count it as surviving rather
                    // than silently dropping a branch we failed to measure.
                    let survives =
                        atoms.is_empty() || reading_atoms.iter().any(|r| atoms.is_subset(r));
                    if survives {
                        match lexicon_of(&atoms) {
                            Some(Lexicon::WordNet) => saw_wordnet = true,
                            Some(Lexicon::Umls) => saw_umls = true,
                            None => {}
                        }
                    }
                    entries.push((label, survives));
                }
                if entries.len() < 2 {
                    continue;
                }
                let felicitous = entries.iter().filter(|(_, s)| *s).count();
                let labels = entries
                    .into_iter()
                    .map(|(l, s)| if s { l } else { format!("{l} [pruned]") })
                    .collect();
                sites.push(Site {
                    span: (i, j),
                    text,
                    kind: SiteKind::Sense,
                    factor: seen.len(),
                    felicitous,
                    inside,
                    labels,
                    cross_lexicon: saw_wordnet && saw_umls,
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
                    felicitous: node.edges.len(), // not intersectable — see `Site::felicitous`
                    inside,
                    labels,
                    cross_lexicon: false,
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
        // NO structure×/sense× products here. They were inflated ~60x against the extracted counts AND
        // internally incoherent (their product exceeded the raw path count, because sites are not
        // independent) — a caveat line did not stop them being misread, so they are not emitted.
        let mut out = format!(
            "=== ATTRIBUTION «{sentence}» ({} raw forest paths, pre-felicity) ===\n\
             SENSE sites show surviving/raw senses — surviving is the real multiplicity. STRUCTURE\n\
             sites are RAW branching only (not intersectable: kbest records no per-reading derivation),\n\
             so they rank nothing.\n",
            self.readings
        );
        for s in self.sites.iter().take(12) {
            let (kind, count) = match s.kind {
                SiteKind::Sense => ("SENSE ", format!("{}/{}", s.felicitous, s.factor)),
                SiteKind::Structure => ("STRUCT", format!("{} raw", s.factor)),
            };
            out.push_str(&format!(
                "  {kind} [{}..{}] «{}» ×{count} : {}\n",
                s.span.0,
                s.span.1,
                s.text,
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

/// The **sense-identifying** atoms of a pretty-printed term: tokens carrying a run of ≥4 digits —
/// the same signal `erase_senses` uses to erase a sense. `n08430568`, `C0018905`, `deg_a00740336`
/// qualify; `compound_kind`, `gt`, `And` do not. A sense occurs in a reading iff all of its atoms do.
fn sense_atoms(pretty: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for tok in pretty.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        let mut run = 0usize;
        let mut max_run = 0usize;
        for c in tok.chars() {
            if c.is_ascii_digit() {
                run += 1;
                max_run = max_run.max(run);
            } else {
                run = 0;
            }
        }
        if max_run >= 4 {
            out.insert(tok.to_string());
        }
    }
    out
}

/// Which lexicon a sense id came from: WordNet offsets are `n/v/a/r` + digits (`n07342049`,
/// `v02203362_t`, `deg_a00740336`), UMLS CUIs are `C` + digits (`C0205341`).
enum Lexicon {
    WordNet,
    Umls,
}

fn lexicon_of(atoms: &BTreeSet<String>) -> Option<Lexicon> {
    for a in atoms {
        let core = a.trim_start_matches("deg_").trim_start_matches("std_");
        let mut cs = core.chars();
        match cs.next() {
            Some('C') if cs.clone().all(|c| c.is_ascii_digit()) => return Some(Lexicon::Umls),
            Some('n' | 'v' | 'a' | 'r') => return Some(Lexicon::WordNet),
            _ => continue,
        }
    }
    None
}

/// A short syntactic tag for a lexical category — enough to spot a mis-POS'd entry at a glance.
/// `N`/`NP` are nominal; `FN(…)` is a functor (verb, modifier, preposition) shown with its result so
/// `FN(S)` (a verb) reads differently from `FN(N)` (a nominal modifier).
fn cat_tag(cat: &Exp) -> String {
    let p = pretty_term(cat);
    for (prefix, tag) in [
        ("cat_n(", "N"),
        ("cat_np(", "NP"),
        ("cat_s(", "S"),
        ("cat_q(", "Q"),
        ("cat_pp_arg(", "PP"),
        ("cat_pp(", "PP"),
    ] {
        if p.starts_with(prefix) {
            return tag.to_string();
        }
    }
    if p.starts_with("fwd(") || p.starts_with("bwd(") {
        // The functor's RESULT is the informative half — strip one layer and tag that.
        let inner = &p[4..];
        for (prefix, tag) in [
            ("cat_s(", "S"),
            ("cat_n(", "N"),
            ("cat_np(", "NP"),
            ("bwd(cat_s(", "S"),
            ("fwd(cat_s(", "S"),
        ] {
            if inner.starts_with(prefix) {
                return format!("FN({tag})");
            }
        }
        return "FN".to_string();
    }
    p.chars().take(6).collect()
}

/// The class IRI a leaf sense denotes, if it denotes one directly (`C0018905`, `n10529231`). An
/// adjective's `λx. gt(deg_a…(x), std_a…)` has no single class and falls back to the pretty form.
fn class_iri(sem: &Exp) -> Option<&Iri> {
    match sem {
        Exp::EigonClass(i) => Some(i),
        Exp::Ann(inner, _) => class_iri(inner),
        _ => None,
    }
}

/// Resolve a sense to `<id> "<name>" [<semantic types>]` using the chain the parser already holds —
/// the concept class carries `core:description`, and its `core:is_a` parents ARE its UMLS semantic
/// types (`umlssty:<TUI>`). This is what turns an opaque `C0018905` into `"Hemagglutination test"`
/// without a `MRSTY`/`MRCONSO` side-lookup.
fn describe_sense(sem: &Exp, layer: &Layer) -> String {
    let Some(iri) = class_iri(sem) else {
        return sense_label(sem);
    };
    let short = iri.as_str().rsplit(':').next().unwrap_or(iri.as_str());
    let Some(res) = layer.resolve(iri) else {
        return short.to_string();
    };
    let name = res
        .get(&iri_of("urn:eigenius:core:description"))
        .and_then(value_text)
        .map(|d| {
            let d = d.trim_end_matches('.');
            let d: String = d.chars().take(48).collect();
            format!(" \"{d}\"")
        })
        .unwrap_or_default();
    let types = res
        .get(&iri_of("urn:eigenius:core:is_a"))
        .map(|v| {
            let mut t: Vec<String> = value_refs(v)
                .into_iter()
                .filter_map(|r| {
                    let local = r.as_str().rsplit(':').next()?.to_string();
                    r.as_str().contains(":umlssty:").then_some(local)
                })
                .collect();
            t.sort();
            t.dedup();
            t
        })
        .unwrap_or_default();
    let types = if types.is_empty() {
        String::new()
    } else {
        format!(" [{}]", types.join(","))
    };
    format!("{short}{name}{types}")
}

fn iri_of(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI")
}

fn value_text(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// The `ResourceRef` IRIs a property value holds (a lone ref, or an array of them).
fn value_refs(v: &Value) -> Vec<&Iri> {
    match v {
        Value::ResourceRef(i) => vec![i],
        Value::Array(xs) => xs
            .iter()
            .filter_map(|x| match x {
                Value::ResourceRef(i) => Some(i),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
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
    fn sense_atoms_picks_only_sense_identifying_tokens() {
        // Sense ids (≥4-digit run) are kept; structural vocabulary is not.
        let a = sense_atoms("subclass_of(ΣG#0:n08430568. compound_kind(G#0, C0205258))");
        assert!(a.contains("n08430568") && a.contains("C0205258"), "{a:?}");
        assert!(
            !a.contains("compound_kind") && !a.contains("subclass_of"),
            "{a:?}"
        );
        // An adjective's degree axioms identify it even with no class IRI.
        let b = sense_atoms("λx. gt(deg_a00740336(x), std_a00740336)");
        assert!(b.contains("deg_a00740336"), "{b:?}");
        assert!(!b.contains("gt"), "{b:?}");
        // Subset test is what decides survival: a sense occurs in a reading iff all its atoms do.
        assert!(sense_atoms("C0205258").is_subset(&a));
        assert!(!sense_atoms("C9999999").is_subset(&a));
    }

    #[test]
    fn span_text_joins_and_clamps() {
        let toks: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(span_text(&toks, 1, 2), "b c");
        assert_eq!(span_text(&toks, 3, 3), "d");
        assert_eq!(span_text(&toks, 2, 99), "c d"); // out-of-range j clamps, no panic
    }
}
