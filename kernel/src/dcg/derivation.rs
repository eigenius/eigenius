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

//! **Derivations** — how a parse item was built: which tokens each constituent spans, and the step
//! that built it from its children (eigenius#264).
//!
//! The chart drivers record one per item as they build it: seeding marks every item of a cell as a
//! [`Step::Leaf`] over the cell's span, and each composition records its children. The rules never
//! see it, so it cannot change what combines. A reading's derivation is what its term does not
//! keep: where in the sentence each constituent sits — a repeated word, or two words naming one
//! concept, cannot be told apart in the term. It is shared, not copied ([`Arc`]): a composition adds
//! one node over its children's derivations.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::item::Combinator;

/// One constituent: the tokens it spans, the step that built it, and the constituents it was built
/// from, in sentence order.
#[derive(Debug)]
pub struct Derivation {
    /// The token span `(i, j)`, inclusive — the chart cell the item sits in.
    pub span: (usize, usize),
    pub step: Step,
    pub children: Vec<Arc<Derivation>>,
    /// A leaf's sense atoms, by key (`v00644583`, `C0388246`, `ni_project_achilles`): the concepts
    /// its tokens contribute to the reading. Empty above the leaves. What names a reading's concepts
    /// by the words that introduced them — a word list cannot, when two words share a sense.
    pub atoms: Vec<String>,
}

/// How a constituent was built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// An item seeded at its cell: a lexical entry, or one lifted from it at seeding.
    Leaf,
    /// A categorial combination of two adjacent constituents; the result's provenance.
    Combine(Combinator),
    /// A unary shift of one constituent over the same span (the shift's name).
    Unary(&'static str),
    /// A token-keyed construction over two constituents and the reserved word(s) between or after
    /// them, which have no constituent of their own (the rule's name).
    Binary(&'static str),
    /// A pied-piped relative: noun, preposition, subject and verb phrase (`which` has no
    /// constituent).
    PiedPipe,
    /// A sentence-initial modifier absorbing the comma after it.
    AbsorbComma,
}

impl Derivation {
    /// A seeded item over `span`, contributing `atoms`.
    pub fn leaf(span: (usize, usize), atoms: Vec<String>) -> Arc<Self> {
        Arc::new(Derivation {
            span,
            step: Step::Leaf,
            children: Vec::new(),
            atoms,
        })
    }

    /// A constituent over `span` built by `step` from `children`.
    pub fn node(span: (usize, usize), step: Step, children: Vec<Arc<Derivation>>) -> Arc<Self> {
        Arc::new(Derivation {
            span,
            step,
            children,
            atoms: Vec::new(),
        })
    }

    /// Every constituent's span, this one's included. A unary step spans what its child spans, so
    /// it adds no constituent.
    pub fn constituents(&self) -> BTreeSet<(usize, usize)> {
        let mut out = BTreeSet::new();
        self.collect_spans(&mut out);
        out
    }

    fn collect_spans(&self, out: &mut BTreeSet<(usize, usize)>) {
        out.insert(self.span);
        for c in &self.children {
            c.collect_spans(out);
        }
    }

    /// The leaves, in sentence order.
    pub fn leaves(&self) -> Vec<&Derivation> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves<'a>(&'a self, out: &mut Vec<&'a Derivation>) {
        if self.children.is_empty() {
            out.push(self);
        }
        for c in &self.children {
            c.collect_leaves(out);
        }
    }

    /// Each sense atom the leaves contribute, with the words that introduced it: `tokens` are the
    /// sentence's, as the parser split it. An atom two leaves contribute keeps the first's words.
    pub fn leaf_words(&self, tokens: &[String]) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for leaf in self.leaves() {
            let (i, j) = leaf.span;
            let Some(words) = tokens.get(i..=j) else {
                continue;
            };
            for atom in &leaf.atoms {
                out.entry(atom.clone()).or_insert_with(|| words.join(" "));
            }
        }
        out
    }

    /// Whether every constituent's children lie inside it, in sentence order and without
    /// overlapping — what any derivation the drivers record must satisfy.
    pub fn is_well_formed(&self) -> bool {
        let inside = self
            .children
            .iter()
            .all(|c| c.span.0 >= self.span.0 && c.span.1 <= self.span.1);
        let ordered = self.children.windows(2).all(|w| w[0].span.1 < w[1].span.0);
        inside && ordered && self.children.iter().all(|c| c.is_well_formed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «We ascertained [MSI status] [with sequencing]» against «We ascertained [MSI status with
    /// sequencing]»: the two derivations differ in the constituents they build.
    #[test]
    fn two_attachments_differ_in_their_constituents() {
        let l = |span| Derivation::leaf(span, Vec::new());
        let combine = |span, children| {
            Derivation::node(span, Step::Combine(Combinator::ForwardApp), children)
        };
        // We(0) ascertained(1) MSI(2) status(3) with(4) sequencing(5)
        let np = combine((2, 3), vec![l((2, 2)), l((3, 3))]);
        let pp = combine((4, 5), vec![l((4, 4)), l((5, 5))]);
        let verb_attached = combine(
            (0, 5),
            vec![
                l((0, 0)),
                combine(
                    (1, 5),
                    vec![combine((1, 3), vec![l((1, 1)), np.clone()]), pp.clone()],
                ),
            ],
        );
        let noun_attached = combine(
            (0, 5),
            vec![
                l((0, 0)),
                combine((1, 5), vec![l((1, 1)), combine((2, 5), vec![np, pp])]),
            ],
        );
        assert!(verb_attached.is_well_formed() && noun_attached.is_well_formed());
        let (v, n) = (verb_attached.constituents(), noun_attached.constituents());
        assert_eq!(v.difference(&n).copied().collect::<Vec<_>>(), vec![(1, 3)]);
        assert_eq!(n.difference(&v).copied().collect::<Vec<_>>(), vec![(2, 5)]);
        assert_eq!(verb_attached.leaves().len(), 6);
    }

    #[test]
    fn overlapping_children_are_not_well_formed() {
        let bad = Derivation::node(
            (0, 2),
            Step::Combine(Combinator::ForwardApp),
            vec![
                Derivation::leaf((0, 1), Vec::new()),
                Derivation::leaf((1, 2), Vec::new()),
            ],
        );
        assert!(!bad.is_well_formed());
    }
}
