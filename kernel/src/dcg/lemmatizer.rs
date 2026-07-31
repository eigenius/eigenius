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

//! The lemmatizer seam — the lemmatization stage of lexical lookup: reduce an
//! inflected surface form to its base lemma(s) so the lexicon can match entries
//! keyed by lemma (D62 §8.7/§8.8). WordNet's Morphy (`eigenius-wordnet`) is the
//! reference implementation; [`Identity`] is the trivial baseline.

/// Linguistic part of speech — the lexical-lookup key. Distinct from the
/// categorial `lexicon:Cat`: POS keys morphology + the lexicon index, while
/// `Cat` drives composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pos {
    Noun,
    Verb,
    Adj,
    Adv,
}

/// Reduce an inflected surface form to its base lemma(s) in a part of speech —
/// e.g. `("mice", Noun) → ["mouse"]`, `("axes", Noun) → ["axe", "axis"]`. A form
/// already in base shape is its own lemma. The lexicon lookup tries each
/// candidate (and each POS) against its `(lemma, pos) → entries` index, so
/// morphological ambiguity becomes extra leaf items in the parser's chart.
pub trait Lemmatizer {
    fn lemmas(&self, surface: &str, pos: Pos) -> Vec<String>;

    /// An **unvalidated** regular-plural singular stem for a surface whose singular is outside the
    /// lemmatizer's own dictionary, or `None` if it is not a regular plural (D63 §5.1). The validated
    /// [`Self::lemmas`] reduces only to *known* lemmas, so a DOMAIN-lexicon plural whose singular is not
    /// in that dictionary (a UMLS `biomarkers` — `biomarker` ∉ WordNet) yields only the exact surface,
    /// which the seeder then tags SINGULAR and cannot bare-plural-shift. This offers the crude stem so the
    /// kernel can resolve it against the FULL lexicon index and, if an entry exists, get a PLURAL reading.
    /// Default `None` — a lemmatizer with no morphology ([`Identity`]) MUST NOT inject stems (else
    /// `does`→`doe` on every `-s` surface). Morphy overrides it with `morph.c`'s regular detachment
    /// *without* the `is_defined` gate.
    fn regular_plural_stem(&self, _surface: &str) -> Option<String> {
        None
    }
}

/// The **regular English plural detachment** — `morph.c`'s `-ies→-y` / `-s` rules, with no dictionary
/// check. `None` if `surface` is not a regular plural.
///
/// Free-standing and shared, for the same reason
/// [`closed_class::is_closed_class_surface`](super::closed_class::is_closed_class_surface) is: two
/// consumers must not drift. Morphy's [`Lemmatizer::regular_plural_stem`] is this function, and the
/// UMLS importer uses it to decide that a form is a regular inflection of ANOTHER form of the same
/// concept — the QC gate that keeps inflected surfaces out of a lemma-keyed lexicon. If the two rule
/// sets disagreed, the importer would keep forms the lemmatizer can already reach, or drop forms it
/// cannot, and either way a surface would lose its entry.
///
/// The exclusions are the ones that make `-s` unreliable: `-ss` (`process`), `-us` (`virus`), and `-is`
/// (`analysis`) are not plural markers, and a stem shorter than the bound is noise (`is` → `i`).
pub fn regular_plural_stem(surface: &str) -> Option<String> {
    let s = surface.trim().to_lowercase();
    if let Some(stem) = s.strip_suffix("ies") {
        if stem.len() >= 2 {
            return Some(format!("{stem}y"));
        }
    }
    if let Some(stem) = s.strip_suffix('s') {
        let skip = s.ends_with("ss") || s.ends_with("us") || s.ends_with("is");
        if stem.len() >= 3 && !skip {
            return Some(stem.to_string());
        }
    }
    None
}

/// The trivial lemmatizer — every surface form is its own lemma (no morphology).
/// The baseline before plugging in WordNet's Morphy.
pub struct Identity;

impl Lemmatizer for Identity {
    fn lemmas(&self, surface: &str, _pos: Pos) -> Vec<String> {
        vec![surface.trim().to_lowercase()]
    }
}
