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

//! The lookup bridge (D62 §8.8.1): a surface string → the forest of typed
//! parses. It joins the three pieces already built — the [`Lemmatizer`] seam
//! ([`super::lemmatizer`]), the committed lexicon (`lexicon:LexicalEntry`
//! resources in a layer), and the CKY composition parser ([`super::parser`]) —
//! into the kernel-attached `string → tree(s)` library:
//!
//! 1. **tokenize** the input;
//! 2. **seed** the chart: for every token span (bounded by the longest multiword
//!    form), reduce the surface to candidate lemmas via the [`Lemmatizer`] and
//!    look them up in the [`LexicalIndex`] — so a multiword entry (`cell line`,
//!    `act on`) seeds a multi-token span *alongside* the single-token items for
//!    its parts (the MWE-vs-compositional ambiguity, §8.4, carried as competing
//!    chart edges, not resolved here);
//! 3. **compose** with CKY over the seeded chart ([`super::parser::apply`]);
//! 4. **filter** to every full-span `S` parse whose assembled sem type-checks to
//!    `Prop` — the kernel as the felicity oracle.
//!
//! The library returns the WHOLE forest (no selection, no commit). Selecting one
//! parse and committing it as a `lexicon:Sentence` is the encoding institution's
//! job (§8.8.2–8.8.3); an empty forest is a first-class outcome (no admissible
//! parse), not an error.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::check::{check, CheckCtx};
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::readback::readback_val;
use crate::nbe::term::Exp;
use crate::ontology::resource::Value;
use crate::ontology::Iri;

use super::category::{
    cats_coordinate, coordinate_np, coordinate_sem, denote_cat, is_ctor, reciprocate, relativize,
    type_raise,
};
use super::lemmatizer::{Lemmatizer, Pos};
use super::lexicon::entry_to_item;
use super::parser::{apply, Combinator, Item};

/// Split prose into lowercased word tokens. Each token is trimmed of leading and
/// trailing non-alphanumerics (so `"BRCA1,"` → `"brca1"`); empties are dropped.
/// Multiword forms are recovered by re-joining spans at lookup time, not here.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|t| {
            t.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// A `form → entries` index over a layer's committed `lexicon:LexicalEntry`
/// resources, with every entry pre-resolved to a parse [`Item`] (category +
/// sem). Built once per layer; `parse` reuses it. Keys are **lowercased** forms
/// (case-insensitive lookup, the v1 choice; case-sensitive acronym
/// disambiguation is a refinement).
pub struct LexicalIndex {
    layer: Arc<Layer>,
    by_form: BTreeMap<String, Vec<Item>>,
    /// Word count of the longest indexed form — the multi-span seeding window.
    max_words: usize,
}

impl LexicalIndex {
    /// Scan the layer chain for `lexicon:LexicalEntry` resources and index each by
    /// its (lowercased) `lexicon:form`, resolving its `cat`/`sem` to an [`Item`].
    /// Entries whose `cat`/`sem` fail to resolve are skipped (they would have been
    /// caught by the felicity gate at import; a parse cannot use them regardless).
    pub fn build(layer: Arc<Layer>) -> Self {
        let entry_class = iri("urn:eigenius:lexicon:LexicalEntry");
        let form_prop = iri("urn:eigenius:lexicon:form");
        let mut by_form: BTreeMap<String, Vec<Item>> = BTreeMap::new();
        let mut max_words = 1;
        for (_id, r) in layer.iter_all_resources() {
            if !r.is_instance_of(&entry_class) {
                continue;
            }
            let Some(Value::String(form)) = r.get(&form_prop) else {
                continue;
            };
            let Ok(item) = entry_to_item(&layer, r.as_ref()) else {
                continue;
            };
            let key = form.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            max_words = max_words.max(key.split_whitespace().count());
            by_form.entry(key).or_default().push(item);
        }
        LexicalIndex {
            layer,
            by_form,
            max_words,
        }
    }

    /// Number of distinct indexed forms.
    pub fn len(&self) -> usize {
        self.by_form.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_form.is_empty()
    }

    /// The lexical items for one token span's surface: the raw surface plus every
    /// lemma the [`Lemmatizer`] yields across all parts of speech (so an inflected
    /// or collocated form resolves to its base entries). Candidate strings are
    /// de-duplicated before lookup.
    fn lookup_span(&self, surface: &str, lemmatizer: &dyn Lemmatizer) -> Vec<Item> {
        let s_lc = surface.trim().to_lowercase();
        let mut candidates: BTreeSet<String> = BTreeSet::new();
        candidates.insert(s_lc.clone());
        for pos in [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv] {
            for lemma in lemmatizer.lemmas(surface, pos) {
                candidates.insert(lemma.trim().to_lowercase());
            }
        }
        let mut out = Vec::new();
        for c in &candidates {
            if let Some(items) = self.by_form.get(c) {
                // Morphological number (D63 §5.1, the Slice-1 deferral): a surface
                // that morphology *reduced* to this lemma was inflected (plural,
                // for nouns); a surface equal to the lemma is singular. Refine the
                // common noun's underspecified `num_any` to that number so
                // determiner/noun agreement (`every gene` ✓ / `every genes` ✗)
                // bites at composition.
                let num = if *c == s_lc { "sg" } else { "pl" };
                out.extend(items.iter().map(|it| with_noun_num(it, num)));
            }
        }
        // Bare-plural → kind-subject shift (D63 §8.5 Slice 3c): a plural common noun
        // also seeds a `cat_kind` edge (the kind it denotes), so "genes" can serve as
        // a kind subject ("Genes are cell lines" → subclass_of(Gene, CellLine))
        // alongside its ordinary common-noun reading.
        let kinds: Vec<Item> = out
            .iter()
            .filter_map(|it| {
                crate::dcg::kind_subject(&it.cat, &it.sem).map(|(cat, sem)| Item::new(cat, sem))
            })
            .collect();
        out.extend(kinds);
        out
    }

    /// Parse prose into the forest of typed sentence parses: every full-span `S`
    /// derivation whose assembled sem type-checks to `Prop`. Returns the WHOLE
    /// forest (ambiguity included); an empty `Vec` means no admissible parse.
    pub fn parse(&self, text: &str, lemmatizer: &dyn Lemmatizer) -> Vec<Item> {
        let tokens = tokenize(text);
        let n = tokens.len();
        if n == 0 {
            return Vec::new();
        }

        // chart[i][j] = every item spanning tokens i..=j.
        let mut chart: Vec<Vec<Vec<Item>>> = vec![vec![Vec::new(); n]; n];

        // Coordinator positions (D63 §8.4 Phase 3): `and`/`or` are parser-level
        // reserved words (NOT lexical entries — coordination is polymorphic over
        // `Cat`, which `⟦·⟧` can't denote), handled by the coordination rule below.
        let coord_op: Vec<Option<&str>> = tokens
            .iter()
            .map(|t| match t.as_str() {
                "and" => Some("urn:eigenius:logic:And"),
                "or" => Some("urn:eigenius:logic:Or"),
                _ => None,
            })
            .collect();

        // 1. Seed lexical spans (multi-span MWE seeding). A multiword form at
        //    [i,j] is seeded ALONGSIDE the items of its parts, so both readings
        //    survive into the chart.
        for i in 0..n {
            let last = (i + self.max_words).min(n);
            for j in i..last {
                let surface = tokens[i..=j].join(" ");
                let items = self.lookup_span(&surface, lemmatizer);
                chart[i][j].extend(items);
            }
        }

        // Forward bounded type-raising `T` (D63 §8.9 Slice 6-T) at the LEAF cells: a
        // name `NP` lifts to `S/(S\NP)`, so it can forward-compose into a relative
        // clause's object-extraction body `S/NP` ("HeLa affects [gap]"). Applied once
        // per leaf cell here; multi-token / composed cells are raised once each in the
        // CKY loop below. ENF (the `TypeRaised` provenance) keeps these inert outside
        // extraction — a raised functor may only compose, never apply.
        for (i, row) in chart.iter_mut().enumerate() {
            let raised = raise_nps(&row[i], &self.layer);
            row[i].extend(raised);
        }

        // 2. CKY composition, appending combined items to each cell's seeds (so a
        //    multiword leaf and a compositional derivation of the same span both
        //    remain available).
        for len in 2..=n {
            for i in 0..=(n - len) {
                let j = i + len - 1;
                let mut produced = Vec::new();
                for k in i..j {
                    let lefts = chart[i][k].clone();
                    let rights = chart[k + 1][j].clone();
                    for l in &lefts {
                        for r in &rights {
                            if let Some(item) = apply(l, r, &self.layer) {
                                produced.push(item);
                            }
                        }
                    }
                }
                // Coordination: `[X] and/or [Y] → [X]` for same-category,
                // Prop-ending conjuncts, with the generalized-conjunction sem
                // (pointwise-lifted connective — D63 §8.4 Phase 3).
                for c in (i + 1)..j {
                    let Some(op) = coord_op[c] else { continue };
                    let lefts = chart[i][c - 1].clone();
                    let rights = chart[c + 1][j].clone();
                    for l in &lefts {
                        for r in &rights {
                            // Left-branching normal form (the spurious-ambiguity
                            // control our grammar needs — D63 §8.4 Phase 4): a
                            // coordination's RIGHT conjunct may not itself be a
                            // coordination, so `A and B and C` parses *only* as
                            // `(A and B) and C`. (Classic Eisner — composition /
                            // type-raising normal forms — does not apply: we have
                            // application + lexical type-raising + coordination,
                            // no composition rule. It returns when one lands.)
                            if is_coordination(&r.sem) {
                                continue;
                            }
                            if cats_coordinate(&l.cat, &r.cat, &self.layer) {
                                if let Some(sem) =
                                    coordinate_sem(op, &l.cat, &l.sem, &r.sem, &self.layer)
                                {
                                    produced.push(Item::new(l.cat.clone(), sem));
                                }
                            } else if let Some((cat, sem)) =
                                coordinate_np(op, &l.cat, &l.sem, &r.cat, &r.sem, &self.layer)
                            {
                                // NP coordination → a member-retaining `cat_group`
                                // tagged with its connective (D63 §8.4 Phase 6).
                                // Distinct from the Prop-ending generalized
                                // conjunction above: a coordinated NP is not
                                // conjoinable as a `Prop`; it denotes the group
                                // `List C`, distributed (∧ for `and`, ∨ for `or`) or
                                // collected at the verb. `coordinate_np` requires the
                                // right conjunct to be a plain NP, keeping groups
                                // left-branching for the n-ary case (the
                                // `is_coordination` analogue here).
                                produced.push(Item::new(cat, sem));
                            }
                        }
                    }
                }
                // Reciprocal: `[group] <TV> each other → S` (D63 §8.4 Phase 6).
                // "each other" is a reserved anaphor (not a lexical entry); the verb
                // is related over every ordered distinct pair of the subject group's
                // members. Keyed on the trailing "each other" tokens, mirroring how
                // coordination keys on `and`/`or`. The subject must be a (conjunctive)
                // group — a reciprocal needs ≥2 distinct participants.
                if j >= 3 && tokens[j - 1] == "each" && tokens[j] == "other" {
                    // Verb spans [s, j-2]; subject group spans [i, s-1].
                    for s in (i + 1)..=(j - 2) {
                        let subjects = chart[i][s - 1].clone();
                        let verbs = chart[s][j - 2].clone();
                        for subj in &subjects {
                            for tv in &verbs {
                                if let Some((cat, sem)) =
                                    reciprocate(&subj.cat, &subj.sem, &tv.cat, &tv.sem, &self.layer)
                                {
                                    produced.push(Item::new(cat, sem));
                                }
                            }
                        }
                    }
                }
                // Relative clause (D63 §8.9 Slice 6-rel): `[noun] that [body]` → a
                // **refined noun** `cat_n(Σx:C. body(x))`. `that` is a reserved
                // relativizer (like `and`/`or`, `each other`); the body is a
                // subject-relative VP `S\NP` ("that affects HeLa") or an object-relative
                // `S/NP` ("that HeLa affects", built by `T` + forward `B`). Both have
                // sem `body : X → Prop`, so a single rule Σ-refines the noun over the
                // concrete `C` (reusing 3b). The noun spans `[i, c-1]`, the body
                // `[c+1, j]`. The refined noun then rides 3b's determiner+`Fst` rule.
                for c in (i + 1)..j {
                    if tokens[c] != "that" {
                        continue;
                    }
                    let nouns = chart[i][c - 1].clone();
                    let bodies = chart[c + 1][j].clone();
                    for noun in &nouns {
                        for body in &bodies {
                            if let Some((cat, sem)) = relativize(&noun.cat, &body.cat, &body.sem) {
                                produced.push(Item::new(cat, sem));
                            }
                        }
                    }
                }
                chart[i][j].extend(produced);
                // Type-raise `T` (D63 §8.9 Slice 6-T) the cell's name NPs (after its
                // composition + relativizer items are in place), so a non-leaf / composed
                // NP can also seed an extraction body. Raised once per cell.
                let raised = raise_nps(&chart[i][j], &self.layer);
                chart[i][j].extend(raised);
            }
        }

        // 3. The forest: full-span `S` items whose assembled sem — once **NbE-
        //    reduced** (the determiner lambdas β-apply away to a normal form) — the
        //    kernel confirms inhabits `Prop`. Reducing first is essential: a
        //    composed determiner sentence is a redex-heavy `App(λ…, …)` tree, and
        //    `check_infer` cannot synthesize a bare lambda's type.
        chart[0][n - 1]
            .iter()
            .filter(|it| {
                // Complete results: a **finite** declarative/polar `S` (denotes `Prop`)
                // or a wh-question `Q(T)` (denotes `T → Prop`, D63 §8.5). The finiteness
                // gate rejects a bare base/infinitival clause (`S[_,bse]` — the VP an
                // auxiliary selects) as a standalone root, so `*HeLa affect BRCA1` does
                // not parse. Partial functors are dropped.
                is_finite_clause(&it.cat) || is_ctor(&it.cat, "cat_q").is_some()
            })
            .filter_map(|it| self.reduced_felicitous(it))
            .collect()
    }

    /// Normalize `it.sem` (NbE β-reduction → a normal form) and keep the item —
    /// carrying the reduced sem — only if the kernel confirms it **inhabits `⟦cat⟧`**:
    /// `Prop` for a declarative `S`, `T → Prop` for a wh-question `Q(T)`. Uses
    /// check-mode (not `check_infer`) so a wh-question's answer-property *lambda* —
    /// which `check_infer` cannot synthesize — is checked against its expected Π/→.
    fn reduced_felicitous(&self, it: &Item) -> Option<Item> {
        let expected = denote_cat(&it.cat).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        let nf = readback_val(0, &eval(&it.sem, &Rho::Nil).ok()?);
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.layer));
        check(&mut ctx, &nf, &expected_val).ok()?;
        Some(Item {
            cat: it.cat.clone(),
            sem: nf,
            prov: it.prov,
        })
    }
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("valid lexicon iri")
}

/// Forward bounded type-raise (D63 §8.9 Slice 6-T) every name `NP` in a cell's items
/// to `S/(S\NP)`, tagged `Combinator::TypeRaised` so ENF lets it only *compose*.
/// Non-`NP` items (functors, groups, kinds, determined NPs) yield nothing.
fn raise_nps(items: &[Item], layer: &Arc<Layer>) -> Vec<Item> {
    items
        .iter()
        .filter_map(|it| {
            type_raise(&it.cat, &it.sem, layer).map(|(cat, sem)| Item {
                cat,
                sem,
                prov: Combinator::TypeRaised,
            })
        })
        .collect()
}

/// A complete clause root must be **finite**: `cat_s(_, fin | fin_any)`. A base /
/// infinitival clause (`cat_s(_, bse)` — the VP an auxiliary selects) is never a
/// standalone sentence (D63 §8.5, Slice 5a). Non-`cat_s` categories are not clauses.
fn is_finite_clause(cat: &Exp) -> bool {
    match is_ctor(cat, "cat_s") {
        Some([_mood, fin]) => {
            matches!(fin, Exp::InductiveCtor(_, n, _) if n == "fin" || n == "fin_any")
        }
        _ => false,
    }
}

/// Whether a sem was produced by the coordination rule — i.e. (after peeling any
/// `λ`s from a `VP`/`TV` pointwise lift) it is headed by `logic:And`/`logic:Or`.
/// In this grammar those connectives arise *only* from coordination, so this is
/// the derivation marker the left-branching normal form keys on (D63 §8.4 Ph 4).
fn is_coordination(sem: &Exp) -> bool {
    let mut e = sem;
    while let Exp::Lam(_, body) = e {
        e = body;
    }
    matches!(e, Exp::InductiveType(decl, _)
        if matches!(decl.iri.as_str(), "urn:eigenius:logic:And" | "urn:eigenius:logic:Or"))
}

/// Instantiate a common noun's underspecified `num_any` with the surface number.
/// Only a `cat_n(T, num_any)` item is refined (to `cat_n(T, <num>)`); verbs,
/// names, and multiword leaves pass through unchanged. The `lexicon:Num` decl is
/// reused from the existing `num_any` ctor, so no decl lookup is needed.
fn with_noun_num(it: &Item, num_name: &str) -> Item {
    if let Exp::InductiveCtor(decl, name, args) = &it.cat {
        if name == "cat_n" && args.len() == 2 {
            if let Exp::InductiveCtor(num_decl, n, _) = &args[1] {
                if n == "num_any" {
                    let num =
                        Exp::InductiveCtor(num_decl.clone(), num_name.to_string(), Vec::new());
                    return Item {
                        cat: Exp::InductiveCtor(
                            decl.clone(),
                            name.clone(),
                            vec![args[0].clone(), num],
                        ),
                        sem: it.sem.clone(),
                        prov: it.prov,
                    };
                }
            }
        }
    }
    it.clone()
}

#[cfg(test)]
mod tests {
    use super::tokenize;

    #[test]
    fn tokenize_lowercases_and_strips_edge_punctuation() {
        assert_eq!(
            tokenize("HeLa depends on BRCA1."),
            ["hela", "depends", "on", "brca1"]
        );
        assert_eq!(tokenize("  A,  b!  "), ["a", "b"]);
        assert!(tokenize("   ").is_empty());
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn tokenize_keeps_internal_alphanumerics() {
        // intra-token digits/letters survive; only the edges are trimmed.
        assert_eq!(tokenize("p53, (BRCA1)"), ["p53", "brca1"]);
    }
}
