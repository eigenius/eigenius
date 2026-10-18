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

//! **The structural skeleton of a reading** — its bracketing, with lexical sense identity erased.
//!
//! Two readings that differ only in WHICH sense fills a slot are the SAME structure; a grammar is
//! scored on the structures it derives, not on the sense product over them. This module owns that
//! notion for the whole system:
//!
//! - the **measurement** side (`experiments/parsing`, `total-skeletons`, the expected-reading pins)
//!   erases senses to compare a unit's bracketings across runs and across reranker draws;
//! - the **parser** side spends its bounded felicity budget per skeleton
//!   ([`crate::dcg::parse::CLASSIFY_BUDGET`]), so a unit with 869 candidates over 5 structures
//!   evaluates all 5 rather than 256 sense-variants of the cheapest one.
//!
//! It lives in the kernel because those two must not drift: if the parser's idea of "same structure"
//! were coarser than the gate's, the parser could drop a bracketing the gate then reports as a lost
//! reading — the exact failure this module was extracted to prevent (2026-07-25, see
//! `docs/notes/grammar-defect-analysis-method.md` §5b′).

use super::pretty_term;
use crate::nbe::term::Exp;
use std::collections::BTreeMap;

/// The skeleton of a `sem`: pretty-print it, then erase sense identity ([`erase_senses`]).
pub fn skeleton_of(sem: &Exp) -> String {
    erase_senses(&pretty_term(sem))
}

/// Erase sense identity to `§`, leaving the STRUCTURE, so two readings that differ only in WHICH
/// sense fills a slot collapse to one skeleton.
///
/// **Token-normalised.** The erasure replaces the WHOLE token carrying a run of ≥4 digits, not just
/// the digits. An earlier version erased only the digit run and kept the lexicon prefix, so
/// `n07342049` → `n§` and `C0205341` → `C§` stayed DISTINCT — a cross-lexicon sense pair for ONE
/// word was then counted as two *structural* skeletons though the bracketing is identical. That
/// artifact was **86 of 326** skeletons on the reference page (26%), i.e. a quarter of the tracked
/// structural lever was sense noise. Erasing the whole token makes the count measure bracketing
/// alone, which is what grammar work is scored against. See `experiments/parsing/README.md` §7b.
pub fn erase_senses(s: &str) -> String {
    let erased: String = s
        .split_inclusive(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map(|tok| {
            let (word, tail) = match tok.chars().last() {
                Some(c) if !(c.is_ascii_alphanumeric() || c == '_') => (
                    &tok[..tok.len() - c.len_utf8()],
                    &tok[tok.len() - c.len_utf8()..],
                ),
                _ => (tok, ""),
            };
            let (mut run, mut max_run) = (0usize, 0usize);
            for c in word.chars() {
                if c.is_ascii_digit() {
                    run += 1;
                    max_run = max_run.max(run);
                } else {
                    run = 0;
                }
            }
            if max_run >= 4 {
                format!("§{tail}")
            } else {
                format!("{word}{tail}")
            }
        })
        .collect();
    normalize_holes(&erased)
}

/// Canonicalise referent/quant HOLE binder names so a skeleton is span-INDEPENDENT. A hole variable
/// `$name$i_j` is position-keyed by [`hole_base`](super::holes::hole_base) (D64), so the SAME open
/// reading freshened at a different derivation site prints a different name — a derivation artifact,
/// not structure. Two α-equivalent open readings must collapse to ONE skeleton, else the structural
/// count inflates and a pin breaks the moment a grammar change moves the freshening site (as the
/// `elided_than` shift did — `$anaphor$6_60` → `$anaphor$0_90`).
///
/// Each distinct `$name$i_j` token is renamed to `$name$<ordinal>` by first appearance, per name
/// prefix — preserving co-reference (same token → same canonical) and distinctness (two holes stay
/// two). Same spirit as the whole-token sense-erasure: strip a derivation-specific detail that was
/// silently being counted as structure. `$` occurs only in hole tokens in a pretty-printed sem, so
/// the scan keys on it.
pub fn normalize_holes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut counters: BTreeMap<String, usize> = BTreeMap::new();
    let mut canon: BTreeMap<String, String> = BTreeMap::new();
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'$' {
            let rest = &s[i + 1..];
            if let Some(d2) = rest.find('$') {
                let name = &rest[..d2];
                let after = &rest[d2 + 1..];
                let span_len = after
                    .bytes()
                    .take_while(|b| b.is_ascii_digit() || *b == b'_')
                    .count();
                let span = &after[..span_len];
                if !name.is_empty()
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    && span.contains('_')
                    && span.bytes().next().is_some_and(|b| b.is_ascii_digit())
                {
                    let token = format!("${name}${span}");
                    let canonical = canon
                        .entry(token)
                        .or_insert_with(|| {
                            let n = counters.entry(name.to_string()).or_insert(0);
                            let c = format!("${name}${n}");
                            *n += 1;
                            c
                        })
                        .clone();
                    out.push_str(&canonical);
                    i += 1 + d2 + 1 + span_len; // '$' + name + '$' + span
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Reorder a **cost-sorted** list so a later `truncate(k)` covers distinct `key`s before it spends
/// depth on any one of them: group by `key`, then round-robin the groups.
///
/// This is how every bounded stage of the parse should spend its budget. A flat cost prefix is
/// biased **systematically**, not incidentally: a deeper derivation costs more, so the readings a
/// prefix drops first are exactly the deeply-NESTED ones — the correct PP attachments. Witnessed
/// 2026-07-25 on "PARP-1 inhibitors are successful in cancers with deficiencies in homologous
/// recombination.", where the fully-nested reading (`in [cancers with [deficiencies in HR]]`) was
/// the LAST of 13 structures and so fell outside every prefix; the unit reported 2 skeletons.
///
/// Order WITHIN a group stays cost-ascending, and groups are emitted in the order their cheapest
/// member appeared, so the preference order is unchanged wherever the budget does not bind.
pub(crate) fn spread_over_keys<T>(items: Vec<T>, key: impl Fn(&T) -> String) -> Vec<T> {
    let mut groups: Vec<Vec<T>> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for it in items {
        let k = key(&it);
        match index.get(&k) {
            Some(&i) => groups[i].push(it),
            None => {
                index.insert(k, groups.len());
                groups.push(vec![it]);
            }
        }
    }
    let widest = groups.iter().map(Vec::len).max().unwrap_or(0);
    let mut out: Vec<T> = Vec::with_capacity(groups.iter().map(Vec::len).sum());
    // Cursors rather than `remove(0)`, so one huge group stays linear.
    let mut cursors: Vec<std::vec::IntoIter<T>> =
        groups.into_iter().map(|g| g.into_iter()).collect();
    for _ in 0..widest {
        for g in cursors.iter_mut() {
            if let Some(it) = g.next() {
                out.push(it);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The budget must reach every structure before it takes a second reading of any one.
    #[test]
    fn spread_over_keys_covers_groups_before_depth() {
        // Cost-sorted input where one structure ("a") dominates the cheap end.
        let items = vec!["a1", "a2", "a3", "a4", "b1", "c1", "b2"];
        let out = spread_over_keys(items, |s| s[..1].to_string());
        assert_eq!(out, vec!["a1", "b1", "c1", "a2", "b2", "a3", "a4"]);
        // A budget of 3 now sees all three structures; a flat prefix would have seen only "a".
        assert_eq!(
            out[..3]
                .iter()
                .map(|s| &s[..1])
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
    }

    /// Pins the eraser semantics the structural metric depends on (README §7b). If this ever fails,
    /// the tracked structural lever has started counting SENSE differences as structure again.
    #[test]
    fn erase_senses_collapses_cross_lexicon_sense_pairs() {
        // One bracketing, a WordNet sense vs a UMLS sense in the same slot ⇒ ONE skeleton.
        let wn = erase_senses("compound_kind(G#0, n07342049)");
        let umls = erase_senses("compound_kind(G#0, C0205341)");
        assert_eq!(
            wn, umls,
            "cross-lexicon sense pair must not read as a structural difference"
        );
        assert_eq!(wn, "compound_kind(G#0, §)");
    }

    /// A short digit run is NOT a sense: `G#0` / `.1` / `2` must survive, else the erasure would
    /// collapse genuinely different bracketings (projection indices, binder ordinals).
    #[test]
    fn erase_senses_keeps_structure_bearing_numbers() {
        assert_eq!(
            erase_senses("the(ΣG#0:§. p(G#0)).1"),
            "the(ΣG#0:§. p(G#0)).1"
        );
    }

    /// The same open reading freshened at two different spans is ONE skeleton.
    #[test]
    fn normalize_holes_is_span_independent() {
        let a = erase_senses("gt(f($anaphor$6_60), std)");
        let b = erase_senses("gt(f($anaphor$0_90), std)");
        assert_eq!(a, b, "hole freshening site is a derivation artifact");
        // Distinct holes stay distinct, and co-reference is preserved.
        let two = erase_senses("p($anaphor$0_1, $anaphor$2_3, $anaphor$0_1)");
        assert_eq!(two, "p($anaphor$0, $anaphor$1, $anaphor$0)");
    }
}
