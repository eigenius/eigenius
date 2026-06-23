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
use crate::nbe::check::{check_infer, CheckCtx};
use crate::nbe::env::Rho;
use crate::nbe::readback::readback_val;
use crate::nbe::term::Exp;
use crate::ontology::resource::Value;
use crate::ontology::Iri;

use super::category::is_ctor;
use super::lemmatizer::{Lemmatizer, Pos};
use super::lexicon::entry_to_item;
use super::parser::{apply, Item};

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
        let mut candidates: BTreeSet<String> = BTreeSet::new();
        candidates.insert(surface.trim().to_lowercase());
        for pos in [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv] {
            for lemma in lemmatizer.lemmas(surface, pos) {
                candidates.insert(lemma.trim().to_lowercase());
            }
        }
        let mut out = Vec::new();
        for c in &candidates {
            if let Some(items) = self.by_form.get(c) {
                out.extend(items.iter().cloned());
            }
        }
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
                chart[i][j].extend(produced);
            }
        }

        // 3. The forest: full-span `S` items whose sem the kernel confirms is a
        //    Prop (felicity of the whole sentence, kernel-attested).
        chart[0][n - 1]
            .iter()
            .filter(|it| is_ctor(&it.cat, "cat_s").is_some())
            .filter(|it| self.checks_to_prop(&it.sem))
            .cloned()
            .collect()
    }

    /// The kernel felicity oracle: does `sem` infer the type `Prop` (`Sort 0`)?
    fn checks_to_prop(&self, sem: &Exp) -> bool {
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.layer));
        match check_infer(&mut ctx, sem) {
            Ok(ty) => matches!(readback_val(0, &ty), Exp::Sort(0)),
            Err(_) => false,
        }
    }
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("valid lexicon iri")
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
