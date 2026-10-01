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

//! **Grammatical analyses** — a sentence's competing readings as a grammar book shows them
//! (eigenius#264): the sentence itself, bracketed where the analyses group its words differently,
//! and the grammatical function of each phrase on which they differ.
//!
//! ```text
//! We ascertained [MSI status] with sequencing.     «with sequencing» is an adverbial of «ascertained»
//! We ascertained [MSI status with sequencing].     «with sequencing» postmodifies «status»
//! ```
//!
//! The brackets come from each reading's derivation ([`crate::dcg::derivation`]), the functions
//! from its links ([`crate::dcg::verbalize::structure_links`]). Only what differs is shown: a
//! constituent every analysis builds, or a function every analysis gives, decides nothing. A
//! multi-word concept is one TERM (`⟨double-stranded DNA breaks⟩`), not a phrase over the same
//! words: an analysis taking the words as one concept and one composing them differ there, and
//! both show it.

use std::collections::BTreeSet;

use super::verbalize::{Function, Link};

/// One reading, as an analysis is computed from it.
pub struct Parse<'a> {
    /// Its constituents' token spans (a derivation's, [`crate::dcg::derivation::Derivation::constituents`]).
    pub constituents: &'a [(usize, usize)],
    /// The spans of its multi-word terms: leaves over several tokens that name one concept.
    pub terms: &'a [(usize, usize)],
    pub links: &'a [Link],
    /// What its predication says, where its form makes a difference no link shows
    /// ([`crate::dcg::verbalize::predication`]).
    pub predication: Option<&'a str>,
}

/// A reading's grammatical analysis against the others it is shown with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Analysis {
    /// The sentence, bracketed around the constituents this analysis builds and some other does not.
    pub bracketed: String,
    /// The grammatical function of each phrase on which the analyses differ, and the predication
    /// where the analyses' predications differ.
    pub functions: Vec<String>,
}

/// What a constituent is: a phrase composed of its parts, or a term — several words naming one
/// concept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Phrase,
    Term,
}

/// Each reading's analysis against the others.
pub fn analyses(tokens: &[String], parses: &[Parse]) -> Vec<Analysis> {
    let sets: Vec<BTreeSet<((usize, usize), Group)>> = parses
        .iter()
        .map(|p| {
            p.constituents
                .iter()
                .chain(p.terms)
                .map(|&s| {
                    (
                        s,
                        if p.terms.contains(&s) {
                            Group::Term
                        } else {
                            Group::Phrase
                        },
                    )
                })
                .collect()
        })
        .collect();
    let common_spans = intersection(&sets);
    let link_sets: Vec<BTreeSet<&Link>> = parses.iter().map(|p| p.links.iter().collect()).collect();
    let common_links = intersection(&link_sets);
    let predications: BTreeSet<Option<&str>> = parses.iter().map(|p| p.predication).collect();
    let whole = (0, tokens.len().saturating_sub(1));
    parses
        .iter()
        .zip(&sets)
        .map(|(p, spans)| {
            let shown: BTreeSet<((usize, usize), Group)> = spans
                .difference(&common_spans)
                .copied()
                .filter(|&((i, j), _)| j > i && (i, j) != whole)
                .collect();
            let terms = shown
                .iter()
                .filter(|(_, g)| *g == Group::Term)
                .filter_map(|&((i, j), _)| tokens.get(i..=j))
                .map(|w| format!("«{}» is one term, a single named concept", w.join(" ")));
            let mut functions: Vec<String> = terms
                .chain(
                    p.links
                        .iter()
                        .filter(|l| !common_links.contains(l))
                        .map(function_line),
                )
                .collect();
            if predications.len() > 1 {
                functions.extend(p.predication.map(str::to_string));
            }
            Analysis {
                bracketed: bracket(tokens, &shown),
                functions,
            }
        })
        .collect()
}

fn intersection<T: Ord + Clone>(sets: &[BTreeSet<T>]) -> BTreeSet<T> {
    let mut it = sets.iter();
    let first = it.next().cloned().unwrap_or_default();
    it.fold(first, |acc, s| acc.intersection(s).cloned().collect())
}

/// The sentence with `[…]` around each phrase and `⟨…⟩` around each term. The spans of one
/// derivation nest or are disjoint, so at each token the brackets open outermost first and close
/// innermost first.
pub fn bracket(tokens: &[String], spans: &BTreeSet<((usize, usize), Group)>) -> String {
    let marks = |g: Group| match g {
        Group::Phrase => ('[', ']'),
        Group::Term => ('⟨', '⟩'),
    };
    let mut out = String::new();
    for (k, t) in tokens.iter().enumerate() {
        let mut opening: Vec<&((usize, usize), Group)> =
            spans.iter().filter(|((i, _), _)| *i == k).collect();
        opening.sort_by_key(|((_, j), _)| std::cmp::Reverse(*j));
        let mut closing: Vec<&((usize, usize), Group)> =
            spans.iter().filter(|((_, j), _)| *j == k).collect();
        closing.sort_by_key(|((i, _), _)| std::cmp::Reverse(*i));
        let glued = matches!(t.as_str(), "," | "." | ";" | ":" | "?" | "!" | ")");
        if k > 0 && !(glued && opening.is_empty()) {
            out.push(' ');
        }
        out.extend(opening.iter().map(|(_, g)| marks(*g).0));
        out.push_str(t);
        out.extend(closing.iter().map(|(_, g)| marks(*g).1));
    }
    out
}

/// A link as a grammar book states it.
pub fn function_line(l: &Link) -> String {
    let (d, h) = (&l.dependent, &l.host);
    match &l.function {
        Function::Subject => format!("«{d}» is the subject of «{h}»"),
        Function::Object => format!("«{d}» is the object of «{h}»"),
        Function::PrepositionalObject => {
            format!(
                "«{d}» is the prepositional object of «{h}», which takes it through a preposition"
            )
        }
        Function::ObjectComplement(p) => format!("«{p} {d}» is the object complement of «{h}»"),
        Function::NounModifier => format!("«{d}» is a noun modifier of «{h}»"),
        Function::Adjective => format!("«{d}» is an adjective describing «{h}»"),
        Function::PredicateAdjective => format!("«{d}» is predicated of «{h}»"),
        Function::Numeral => format!("a numeral gives the number of «{h}»"),
        Function::Postmodifier(p) => {
            format!("«{p} {d}» postmodifies «{h}»: it says which or what kind of {h}")
        }
        Function::Adverbial(p) => {
            format!("«{p} {d}» is an adverbial of «{h}»: it says how, where, when or why")
        }
        Function::PredicateAdverbial(p) => {
            format!("«{p} {d}» is an adverbial of the predicate «{h}»: it says where, when or how it holds")
        }
        Function::SecondPredicate(p) => {
            format!("«{p} {d}» is a second predicate of «{h}»: {h} is {p} {d}")
        }
        Function::AdjectiveComplement(p) => {
            format!("«{p} {d}» is the complement of the adjective «{h}»")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_string).collect()
    }

    fn link(function: Function, dependent: &str, host: &str) -> Link {
        Link {
            function,
            dependent: dependent.into(),
            host: host.into(),
        }
    }

    /// «We ascertained MSI status with sequencing.»: the verb's adjunct against the noun's
    /// postmodifier, shown as a grammar book shows them.
    #[test]
    fn an_attachment_is_shown_by_its_brackets_and_its_function() {
        let tokens = words("We ascertained MSI status with sequencing .");
        let shared = [(0, 6), (2, 3), (4, 5)];
        let verb: Vec<(usize, usize)> = shared.iter().copied().chain([(1, 3), (1, 5)]).collect();
        let noun: Vec<(usize, usize)> = shared.iter().copied().chain([(2, 5), (1, 5)]).collect();
        let common = link(Function::NounModifier, "MSI", "status");
        let verb_links = [
            common.clone(),
            link(
                Function::Adverbial("with".into()),
                "sequencing",
                "ascertained",
            ),
        ];
        let noun_links = [
            common,
            link(
                Function::Postmodifier("with".into()),
                "sequencing",
                "status",
            ),
        ];
        let a = analyses(
            &tokens,
            &[
                Parse {
                    constituents: &verb,
                    terms: &[],
                    links: &verb_links,
                    predication: None,
                },
                Parse {
                    constituents: &noun,
                    terms: &[],
                    links: &noun_links,
                    predication: None,
                },
            ],
        );
        assert_eq!(
            a[0].bracketed,
            "We [ascertained MSI status] with sequencing."
        );
        assert_eq!(
            a[0].functions,
            vec!["«with sequencing» is an adverbial of «ascertained»: it says how, where, when or why"]
        );
        assert_eq!(
            a[1].bracketed,
            "We ascertained [MSI status with sequencing]."
        );
        assert_eq!(
            a[1].functions,
            vec!["«with sequencing» postmodifies «status»: it says which or what kind of status"]
        );
    }

    /// Two analyses that differ only in what their predication says show it.
    #[test]
    fn a_predication_is_shown_where_it_differs() {
        let tokens = words("Nucleotide repeat regions are microsatellites .");
        let spans = [(0, 2), (0, 5)];
        let a = analyses(
            &tokens,
            &[
                Parse {
                    constituents: &spans,
                    terms: &[],
                    links: &[],
                    predication: Some("every one is"),
                },
                Parse {
                    constituents: &spans,
                    terms: &[],
                    links: &[],
                    predication: Some("the kind is"),
                },
            ],
        );
        assert_eq!(
            a[0].bracketed,
            "Nucleotide repeat regions are microsatellites."
        );
        assert_eq!(a[0].functions, vec!["every one is"]);
        assert_eq!(a[1].functions, vec!["the kind is"]);
    }

    #[test]
    fn nested_spans_bracket_in_order() {
        let tokens = words("a b c d");
        let spans: BTreeSet<((usize, usize), Group)> = [
            ((1, 3), Group::Phrase),
            ((2, 3), Group::Term),
            ((1, 1), Group::Phrase),
        ]
        .into_iter()
        .collect();
        assert_eq!(bracket(&tokens, &spans), "a [[b] ⟨c d⟩]");
    }

    /// «Depletion of WRN induced double-stranded DNA breaks.»: the pinned reading takes the three
    /// words as one concept (C1511667), a rival composes them. Every reading builds a constituent
    /// over the words; the term and the phrase differ, and both are shown.
    #[test]
    fn a_term_is_shown_against_a_phrase_over_the_same_words() {
        let tokens = words("Depletion of WRN induced double-stranded DNA breaks");
        let shared = [(0, 6), (0, 2), (3, 6), (4, 6)];
        let composed: Vec<(usize, usize)> = shared.iter().copied().chain([(5, 6)]).collect();
        let a = analyses(
            &tokens,
            &[
                Parse {
                    constituents: &shared,
                    terms: &[(4, 6)],
                    links: &[],
                    predication: None,
                },
                Parse {
                    constituents: &composed,
                    terms: &[],
                    links: &[],
                    predication: None,
                },
            ],
        );
        assert_eq!(
            a[0].bracketed,
            "Depletion of WRN induced ⟨double-stranded DNA breaks⟩"
        );
        assert_eq!(
            a[0].functions,
            vec!["«double-stranded DNA breaks» is one term, a single named concept"]
        );
        assert_eq!(
            a[1].bracketed,
            "Depletion of WRN induced [double-stranded [DNA breaks]]"
        );
        // A term every analysis takes decides nothing and is not shown.
        let both = analyses(
            &tokens,
            &[
                Parse {
                    constituents: &shared,
                    terms: &[(4, 6)],
                    links: &[],
                    predication: None,
                },
                Parse {
                    constituents: &[(0, 6), (0, 3), (4, 6)],
                    terms: &[(4, 6)],
                    links: &[],
                    predication: None,
                },
            ],
        );
        assert!(both[0].functions.is_empty());
    }
}
