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

//! `dcg` — the **dependent categorial grammar** engine (the Chatzikyriakidis &
//! Luo DCGs, `chatzikyriakidis-luo-2020`; D62 §8.6): the trusted half of the
//! prose → typed-trees pipeline, mapping categorial structure over `lexicon:Cat`
//! to type-checked EigenTT trees. The kernel is the felicity *oracle*; an
//! untrusted source (an LLM, or the WordNet import) only ever proposes — the
//! kernel admits or rejects.
//!
//! (The *lexicon* is the data — the `lexicon:` namespace, `ontologies/lexicon/`,
//! the WordNet import; this module is the engine that consumes it.)
//!
//! Organized into pipeline components, with the public API re-exported flat:
//! - [`category`] — the `⟦·⟧ : Cat → EigenTT type` homomorphism, definitional
//!   equality, and categorial subsumption.
//! - [`parser`] — parse items + forward/backward application + the CKY chart.
//! - [`lexicon`] — lexical-entry handling + the felicity [`gate_entry`].
//! - [`lemmatizer`] — the surface→lemma seam for the lookup stage (Morphy in
//!   `eigenius-wordnet` is the reference impl).
//! - [`lookup`] — the bridge (§8.8.1): `string → tree(s)` via a [`LexicalIndex`]
//!   + multi-span lemmatized seeding + CKY + the kernel felicity filter.

pub mod category;
pub mod glossary;
pub mod lemmatizer;
pub mod lexicon;
pub mod lookup;
mod packed;
pub mod parser;
pub mod pretty;
mod reserved;
pub mod segment;
pub mod sense_ranker;

/// Live-LLM anaphora proposer (D64 §4) — opt-in via the `allms` feature; default builds stay
/// LLM-free.
#[cfg(feature = "allms")]
pub mod resolver_allms;

pub use category::{
    cat_subsumes, cats_coordinate, common_super, coordinate_np, coordinate_sem, denote_cat,
    distribute, distribute_object, feat_meets, is_ctor, kind_subject, reciprocate, relativize,
    subst_cat, type_eq, type_raise, unify_cat, CatSubst,
};
#[cfg(feature = "allms")]
pub use glossary::AnthropicAbbreviationProposer;
pub use glossary::{
    abbreviation_resources, document_glossary_resources, document_glossary_resources_with,
    extract_abbreviations, extract_abbreviations_with, glossary_resources, ground_abbreviation,
    ground_long_form, AbbrDef, AbbreviationBinding, AbbreviationProposer, NoAbbreviationProposer,
};
pub use lemmatizer::{Identity, Lemmatizer, Pos};
pub use lexicon::{entry_to_item, gate_entry, resolve_sem, resolve_sem_value};
pub use lookup::{
    resolve_lexicon_profile, tokenize, Candidate, HoleInfo, HoleKind, LexicalIndex, OpenParse,
    ProposeCtx, Proposer, DEFAULT_FOREST_CAP,
};
pub use parser::{apply, cky_parse, Combinator, Cost, Item};
pub use pretty::{cat_shape, pretty_term};
pub use segment::{is_nonprose, segment_sentences};
#[cfg(feature = "allms")]
pub use sense_ranker::AnthropicSenseRanker;
pub use sense_ranker::{IdentityRanker, SenseCandidate, SenseRanker, WordSenses};
