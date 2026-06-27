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

//! `MorphyLemmatizer` — WordNet's Morphy as the kernel's [`Lemmatizer`]: the
//! reference implementation of the lookup bridge's surface→lemma seam
//! (D62 §8.8.1). It wraps the exception lists + the lemma-membership oracle and
//! drives [`morphstr`](crate::morphy::morphstr); the kernel's `dcg::LexicalIndex`
//! calls it to reduce inflected surface forms to the base lemmas its entries are
//! keyed by.

use std::path::Path;

use eigenius_kernel::dcg::{Lemmatizer, Pos as DcgPos};

use crate::morphy::{morphstr, ExcLists, LemmaSet};
use crate::wndb::{read_data_file, Pos as WnPos};

/// Morphy-backed lemmatizer: the `.exc` lists + the `LemmaSet` membership oracle.
/// Build from a WordNet dict directory ([`Self::load`]) or in-memory tables
/// ([`Self::new`]).
pub struct MorphyLemmatizer {
    exc: ExcLists,
    lemmas: LemmaSet,
}

impl MorphyLemmatizer {
    pub fn new(exc: ExcLists, lemmas: LemmaSet) -> Self {
        Self { exc, lemmas }
    }

    /// Load `*.exc` and build the `LemmaSet` from `data.{noun,verb,adj}` under
    /// `dict` (the in-repo default is `references/WordNet-3.0/dict`).
    pub fn load(dict: &Path) -> std::io::Result<Self> {
        let exc = ExcLists::load(dict)?;
        let noun = read_data_file(&dict.join("data.noun"))?;
        let verb = read_data_file(&dict.join("data.verb"))?;
        let adj = read_data_file(&dict.join("data.adj"))?;
        let lemmas = LemmaSet::from_synsets([&noun, &verb, &adj]);
        Ok(Self::new(exc, lemmas))
    }
}

fn wn_pos(p: DcgPos) -> WnPos {
    match p {
        DcgPos::Noun => WnPos::Noun,
        DcgPos::Verb => WnPos::Verb,
        DcgPos::Adj => WnPos::Adj,
        DcgPos::Adv => WnPos::Adv,
    }
}

impl Lemmatizer for MorphyLemmatizer {
    fn lemmas(&self, surface: &str, pos: DcgPos) -> Vec<String> {
        // Morphy returns only *reduced* forms (empty for an already-base word), so
        // also include the surface itself — a base form is its own lemma (the
        // trait contract). Lowercased; the base is appended only if absent.
        let mut out = morphstr(surface, wn_pos(pos), &self.exc, &self.lemmas);
        let base = surface.trim().to_lowercase();
        if !out.iter().any(|l| l.eq_ignore_ascii_case(&base)) {
            out.push(base);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::dcg::{Lemmatizer, Pos};

    fn mini() -> MorphyLemmatizer {
        let mut lemmas = LemmaSet::new();
        lemmas.insert("dog", WnPos::Noun);
        lemmas.insert("depend", WnPos::Verb);
        MorphyLemmatizer::new(ExcLists::parse("", "", "", ""), lemmas)
    }

    #[test]
    fn reduces_inflected_to_base() {
        assert!(mini()
            .lemmas("dogs", Pos::Noun)
            .contains(&"dog".to_string()));
        assert!(mini()
            .lemmas("depends", Pos::Verb)
            .contains(&"depend".to_string()));
    }

    #[test]
    fn already_base_form_is_its_own_lemma() {
        // morphstr returns nothing for a base word; the trait contract still
        // requires the form itself.
        assert_eq!(mini().lemmas("dog", Pos::Noun), vec!["dog".to_string()]);
    }
}
