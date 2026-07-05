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

//! **Document lexicon augmentation** (D63, `docs/notes/d63-lexicon-augmentation.md`) — the data model +
//! the deterministic `DocumentOnly` transducer that generalize Stage A's abbreviation glossary into
//! "resolve every lexical gap to a grounded, typed entry, exposing the augmentation as a first-class,
//! composable value."
//!
//! A [`LexicalBinding`] **wraps a proposed, un-committed `lexicon:LexicalEntry`** (the same type the parser
//! seeds) plus [`Provenance`] — how it was produced and how far to trust it. It is *not* a rival to the
//! committed entry; it is the proposal envelope in propose → gate → commit, and running the pipeline
//! **harvests** these as candidate permanent lexicon additions. A detected OOV that no proposal closes is a
//! [`Gap`] (a fail-closed finding, never a silent drop). [`LexiconAugmentation`] is the transducer's exposed
//! state: `added` (the harvest) + `missing_oov` (the residual).
//!
//! Phase 1 (here) implements the `DocumentOnly` source (the document's own abbreviation definitions +
//! the OOV pre-pass). `LexiconBacked` (text-retrieval grounding) and `LlmBacked` (synthesis) are Phase 2/3.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::layer::Layer;
use crate::ontology::resource::Resource;
use crate::ontology::Iri;

use super::glossary::{
    extract_abbreviations_with, glossary_resources, ground_abbreviation, AbbreviationProposer,
};
use super::lemmatizer::Lemmatizer;
use super::lookup::{tokenize, LexicalIndex};

const LEXICAL_ENTRY: &str = "urn:eigenius:lexicon:LexicalEntry";

/// How a proposed lexical entry was resolved — a **trust signal** on the binding, most-trusted first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionMethod {
    /// The document itself defined it (Schwartz-Hearst / a definitional pattern). Deterministic.
    DefinitionExtracted,
    /// A retrieval hit against the committed lexicon (the form / description text index) grounded it.
    RetrievalGrounded,
    /// An LLM synthesized a provisional type/grounding from a retrieved definition. Lowest trust.
    LlmSynthesized,
}

/// The provenance envelope on a [`LexicalBinding`]: how the wrapped entry was produced + how far to trust
/// it (`docs/notes/d63-lexicon-augmentation.md` §3). Carried on the proposal, not on the committed entry.
#[derive(Clone, Debug)]
pub struct Provenance {
    /// The surface the gap was found under (pre-normalization).
    pub surface: String,
    /// The intra-document definition — `Some` for an abbreviation, `None` for a bare OOV term.
    pub long_form: Option<String>,
    /// The source window (grounding retries + audit).
    pub context: String,
    /// How the entry was resolved — the trust signal driving the promotion filter.
    pub method: ResolutionMethod,
    /// The ontology concept the entry aliases, when grounding succeeded (`None` ⇒ ungrounded / minted class).
    pub grounded_to: Option<Iri>,
    /// Retrieval / LLM confidence, when applicable.
    pub confidence: Option<f32>,
}

/// A proposed, un-committed `lexicon:LexicalEntry` + its [`Provenance`] — the unit the pipeline harvests
/// and the kernel gates before committing (§3). Wraps the committed type; it does not rival it.
#[derive(Clone, Debug)]
pub struct LexicalBinding {
    pub proposed: Resource,
    pub provenance: Provenance,
}

/// A detected OOV surface that **no proposal closed** — a fail-closed finding, never silently dropped (§7).
#[derive(Clone, Debug)]
pub struct Gap {
    pub surface: String,
    pub context: String,
    /// The resolution methods attempted (empty in `DocumentOnly` — nothing beyond abbreviation extraction).
    pub tried: Vec<ResolutionMethod>,
}

/// The lexicon-augmentation transducer's exposed state (§6): the harvested proposals + the residual gaps.
/// `supporting` holds non-entry resources a binding references (e.g. a fresh doc-local class minted on a
/// grounding miss) that must be committed alongside the entries.
#[derive(Clone, Debug, Default)]
pub struct LexiconAugmentation {
    pub added: Vec<LexicalBinding>,
    pub supporting: Vec<Resource>,
    pub missing_oov: Vec<Gap>,
}

impl LexiconAugmentation {
    /// Every resource to commit into the document's chained lexicon layer: the proposed entries + the
    /// supporting resources (in that order).
    pub fn resources(&self) -> Vec<Resource> {
        self.added
            .iter()
            .map(|b| b.proposed.clone())
            .chain(self.supporting.iter().cloned())
            .collect()
    }

    /// No entries added and no gaps recorded.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.missing_oov.is_empty()
    }
}

/// Which sources may generate entries (§6). Phase 1 implements [`AugmentOptions::DocumentOnly`];
/// `LexiconBacked` (text-retrieval grounding, scoped by a `lexicon:LexiconProfile`) and `LlmBacked`
/// (synthesis) are Phase 2/3.
#[derive(Clone, Debug)]
pub enum AugmentOptions {
    DocumentOnly,
    LexiconBacked(Iri),
    LlmBacked,
}

/// The **`DocumentOnly`** augmentation (Phase 1): from the document's own abbreviation definitions build
/// grounded [`LexicalBinding`]s (method [`ResolutionMethod::DefinitionExtracted`]), and flag every remaining
/// OOV token as a [`Gap`]. Deterministic — no retrieval, no LLM. Generalizes the Stage-A abbreviation
/// glossary into the augmentation shape (§2/§3): the same `extract → ground → emit` tail, but wrapped as
/// proposals with provenance, plus the fail-closed OOV pre-pass.
pub fn augment_document_only(
    base: &Arc<Layer>,
    document: &str,
    proposer: &dyn AbbreviationProposer,
    lemmatizer: &dyn Lemmatizer,
) -> LexiconAugmentation {
    let Ok(entry_class) = Iri::parse(LEXICAL_ENTRY) else {
        return LexiconAugmentation::default();
    };

    // Stage A → proposals. For each extracted definition, ground it and emit its alias entry, then wrap
    // the entry as a binding (the fresh doc-local class on a grounding miss becomes a supporting resource).
    let defs = extract_abbreviations_with(document, proposer);
    let mut added = Vec::new();
    let mut supporting = Vec::new();
    let mut known: BTreeSet<String> = BTreeSet::new();
    for d in &defs {
        let grounded_to = ground_abbreviation(base, &d.short_form, &d.long_form, &d.context);
        let (entries, extra): (Vec<Resource>, Vec<Resource>) =
            glossary_resources(base, std::slice::from_ref(d))
                .into_iter()
                .partition(|r| r.is_instance_of(&entry_class));
        supporting.extend(extra);
        for proposed in entries {
            added.push(LexicalBinding {
                proposed,
                provenance: Provenance {
                    surface: d.short_form.clone(),
                    long_form: Some(d.long_form.clone()),
                    context: d.context.clone(),
                    method: ResolutionMethod::DefinitionExtracted,
                    grounded_to: grounded_to.clone(),
                    confidence: None,
                },
            });
        }
        known.insert(d.short_form.trim().to_lowercase());
    }

    // OOV pre-pass (fail-closed): every single token the base lexicon does not know — and that we did not
    // just add as an abbreviation — is a `Gap`. `LexiconBacked`/`LlmBacked` (Phase 2/3) would try to ground
    // these; `DocumentOnly` reports them as-is.
    let index = LexicalIndex::build(Arc::clone(base));
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut missing_oov = Vec::new();
    for tok in tokenize(document) {
        let t = tok.trim().to_lowercase();
        if t.is_empty() || known.contains(&t) || !seen.insert(t.clone()) {
            continue;
        }
        if !index.has_token(&t, lemmatizer) {
            missing_oov.push(Gap {
                surface: tok,
                context: String::new(),
                tried: Vec::new(),
            });
        }
    }

    LexiconAugmentation {
        added,
        supporting,
        missing_oov,
    }
}
