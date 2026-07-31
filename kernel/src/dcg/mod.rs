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
//! - [`lookup`] — the bridge (§8.8.1): `string → tree(s)` via a [`Parser`]
//!   + multi-span lemmatized seeding + CKY + the kernel felicity filter.

pub mod abbrev;
pub mod attribution;
pub mod augment;
pub mod category;
pub(crate) mod chart;
pub mod closed_class;
pub mod glossary;
mod grammar;
mod holes;
pub mod item;
pub mod lemmatizer;
pub mod lexicon;
pub mod named_entity;
pub mod parse;
pub mod pipeline;
pub mod pretty;
mod reserved;
mod rules;
pub mod segment;
pub mod sense_ranker;
pub mod skeleton;

/// Direct Anthropic tool-use client for the reasoning-layer LLM calls (sense ranker / proposers) —
/// structured output via forced `tool_choice`, replacing the `allms` prompt-inject-and-parse path.
#[cfg(feature = "use-llm")]
/// The Anthropic structured-output client. Public so the offline data pipelines (e.g. the
/// WordNet↔UMLS concept adjudicator, `crates/eigenius-lexicon-align`) reuse the same transport,
/// model default, and `temperature: 0` pin as the in-kernel proposers, rather than each rolling
/// their own.
pub mod anthropic_client;

/// Live-LLM anaphora proposer (D64 §4) — opt-in via the `use-llm` feature; default builds stay
/// LLM-free.
#[cfg(feature = "use-llm")]
pub mod resolver_llm;

pub use abbrev::{
    extract_abbreviations, extract_abbreviations_with, AbbrDef, AbbreviationProposer,
    NoAbbreviationProposer,
};
#[cfg(feature = "use-llm")]
pub use augment::AnthropicCategoryProposer;
pub use augment::{
    augment_document_only, augment_lexicon_backed, AugmentOptions, CategoryProposer, ExpectedCat,
    Gap, LexicalBinding, LexiconAugmentation, NominalCategoryProposer, Provenance,
    ResolutionMethod,
};
pub use category::{
    cat_subsumes, common_super, denote_cat, feat_meets, is_ctor, subst_cat, type_eq, unify_cat,
    CatSubst,
};
#[cfg(feature = "use-llm")]
pub use glossary::AnthropicAbbreviationProposer;
pub use glossary::{
    abbreviation_resources, document_glossary_resources, document_glossary_resources_with,
    glossary_resources, ground_abbreviation, ground_long_form, is_adjective, is_apposition_head,
    is_common_noun, named_entity_augmentation, AbbreviationBinding,
};
pub use item::{Combinator, Cost, Item};
pub use lemmatizer::{regular_plural_stem, Identity, Lemmatizer, Pos};
pub use lexicon::{
    entry_to_item, gate_entry, resolve_lexicon_profile, resolve_sem, resolve_sem_value, LexEntry,
    LexicalIndex, LexicalLookup,
};
pub use named_entity::{extract_named_entities_with, NamedEntity};
pub use parse::{
    Candidate, HoleInfo, HoleKind, OpenParse, ParseConfig, Parser, ProposeCtx, Proposer,
    SentenceOutcome, DEFAULT_FOREST_CAP,
};
pub use pipeline::{DocumentEncoding, DocumentPipeline, InProcessPipeline, SentenceEncoding};
pub use pretty::pretty_term;
pub use rules::combinators::apply;
pub use rules::constructions::{
    appose_group, cats_coordinate, complete_coord, coordinate_np, coordinate_prop, distribute,
    distribute_object, kind_subject, reciprocate, relativize, type_raise,
};
pub use rules::RightContext;
pub use segment::{is_nonprose, segment_sentences, tokenize};
#[cfg(feature = "use-llm")]
pub use sense_ranker::AnthropicSenseRanker;
pub use sense_ranker::{
    IdentityRanker, RankRecord, RankedWord, RecordingSenseRanker, ReplaySenseRanker,
    SenseCandidate, SenseRanker, WordSenses,
};
