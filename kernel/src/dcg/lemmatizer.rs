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
}

/// The trivial lemmatizer — every surface form is its own lemma (no morphology).
/// The baseline before plugging in WordNet's Morphy.
pub struct Identity;

impl Lemmatizer for Identity {
    fn lemmas(&self, surface: &str, _pos: Pos) -> Vec<String> {
        vec![surface.trim().to_lowercase()]
    }
}
