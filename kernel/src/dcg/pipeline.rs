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

//! **The document→encoding pipeline** (D63, `docs/notes/d63-document-preprocessing-scope.md`): raw
//! document text → per-sentence typed propositions. It composes the three stages behind one contract,
//! [`DocumentPipeline`]:
//!
//! - **Stage A — preprocess:** extract abbreviation definitions and emit the *document glossary* (a
//!   doc-scoped lexicon layer chained on the base), so bare domain abbreviations (`MSI`) parse.
//! - **Stage B — parse:** parse each body sentence over base + doc-glossary.
//! - **Stage C — resolve:** resolve referent holes (pronouns / `these X`) against the threaded discourse.
//!
//! The LLM-backed steps live entirely behind the proposer traits ([`AbbreviationProposer`],
//! [`Proposer`]) — a deterministic mock in tests, the live `Anthropic*` proposers under `--features
//! use-llm`. So the **Phase-2 orchestrator** becomes a different set of proposer impls (RPC-backed)
//! *without changing this contract* — the trait is the seam between "the pipeline" and "how its LLM
//! steps run".

use std::sync::Arc;

use crate::layer::{Layer, LayerBuilder, LayerStorage};

use super::abbrev::AbbreviationProposer;
use super::augment::{
    augment_document_only, augment_lexicon_backed, AugmentOptions, CategoryProposer,
    LexiconAugmentation, NominalCategoryProposer,
};
use super::lemmatizer::Lemmatizer;
use super::parse::{Parser, Proposer, SentenceOutcome};
use super::segment::segment_sentences;

/// The document→encoding pipeline: raw document text → typed propositions, one [`SentenceOutcome`] per
/// body sentence. Fail-closed — an un-encodable sentence is `Open`/`Gap`, never a wrong closed parse.
pub trait DocumentPipeline {
    fn encode(&self, document: &str) -> DocumentEncoding;
}

/// The encoding of a whole document: the lexicon augmentation that was harvested + injected (Stage A) and
/// one outcome per body sentence, in document order.
#[derive(Clone)]
pub struct DocumentEncoding {
    /// The Stage-A lexicon augmentation: the grounded entries added (each a proposal + provenance) and the
    /// residual OOV gaps (`docs/notes/d63-lexicon-augmentation.md`).
    pub augmentation: LexiconAugmentation,
    /// One result per body (prose) sentence, in order.
    pub sentences: Vec<SentenceEncoding>,
}

/// One body sentence's encoding: its surface text and the classified [`SentenceOutcome`].
#[derive(Clone)]
pub struct SentenceEncoding {
    pub text: String,
    pub outcome: SentenceOutcome,
}

/// The Phase-1 **in-process** pipeline: every stage runs in Rust, with the LLM steps behind the proposer
/// traits. It chains the document glossary onto `base` in an **in-memory** layer — a small demo; a
/// DB-backed `base` needs a persistent doc layer instead (an in-memory overlay over the persisted
/// lexicon OOMs, §7-2), a `with_storage` constructor left for that path.
pub struct InProcessPipeline<'a> {
    base: Arc<Layer>,
    lemmatizer: &'a dyn Lemmatizer,
    abbreviation_proposer: &'a dyn AbbreviationProposer,
    anaphora_proposer: &'a dyn Proposer,
    category_proposer: &'a dyn CategoryProposer,
    augment_options: AugmentOptions,
}

/// The default (deterministic) POS proposer — a `'static` ZST so [`InProcessPipeline::new`] can hand out
/// a `&dyn CategoryProposer` without the caller supplying one. Grounding stays nominal-only (the (A)
/// behaviour) until [`InProcessPipeline::with_category_proposer`] installs a live one.
static NOMINAL_CATEGORY_PROPOSER: NominalCategoryProposer = NominalCategoryProposer;

impl<'a> InProcessPipeline<'a> {
    pub fn new(
        base: Arc<Layer>,
        lemmatizer: &'a dyn Lemmatizer,
        abbreviation_proposer: &'a dyn AbbreviationProposer,
        anaphora_proposer: &'a dyn Proposer,
    ) -> Self {
        Self {
            base,
            lemmatizer,
            abbreviation_proposer,
            anaphora_proposer,
            // Default: nominal-only POS proposer (deterministic) — grounding matches the (A) behaviour
            // until a live one is installed via [`Self::with_category_proposer`].
            category_proposer: &NOMINAL_CATEGORY_PROPOSER,
            // Default: `DocumentOnly` (no retrieval) — deterministic, no `base`-index dependency. Opt into
            // `LexiconBacked` (form-`TextIndex` OOV grounding) via [`Self::with_augment_options`].
            augment_options: AugmentOptions::DocumentOnly,
        }
    }

    /// Set the Stage-A augmentation source (`DocumentOnly` default vs `LexiconBacked` form-index grounding,
    /// D63 `docs/notes/d63-lexicon-augmentation.md` §6/§6a). `LexiconBacked` requires an active
    /// `core:TextIndex` over `lexicon:form` in `base`'s chain; without one it degrades to `DocumentOnly`.
    pub fn with_augment_options(mut self, opts: AugmentOptions) -> Self {
        self.augment_options = opts;
        self
    }

    /// Install the (untrusted) POS [`CategoryProposer`] the `LexiconBacked` resolver consults to make
    /// gloss grounding POS-aware (§6a, the (B) step) — a verb/adjective OOV grounds to its predicate
    /// `eigentt:Axiom`, a nominal OOV to a class. Default is [`NominalCategoryProposer`] (the (A)
    /// nominal-only behaviour); pass `AnthropicCategoryProposer` (`use-llm`) for the live proposer.
    pub fn with_category_proposer(mut self, proposer: &'a dyn CategoryProposer) -> Self {
        self.category_proposer = proposer;
        self
    }

    /// Like [`DocumentPipeline::encode`], but also returns the in-memory doc-glossary layer the
    /// sentences were parsed over (`base` + the glossary). An in-process downstream stage — claim
    /// grading in `eigenius-reasoning` — commits onto *this* layer, so a claim whose proposition
    /// references a doc-glossary-only concept (a grounding-miss minted class) still resolves in the
    /// chain. The trait's [`DocumentPipeline::encode`] drops it; a served realization returns a
    /// committed branch instead, which is why the layer is exposed here (inherent), not on the trait.
    pub fn encode_with_layer(&self, document: &str) -> (DocumentEncoding, Arc<Layer>) {
        // Stage A — the lexicon augmentation: harvest the document's abbreviation definitions (and, under
        // `LexiconBacked`, ground residual OOV atoms against the form text index) as grounded proposals (+
        // the residual OOV gaps), and commit its resources as a doc-scoped lexicon layer on `base`.
        // Fail-closed: a proposal the felicity gate rejects at `add_resource` is skipped, so a
        // mis-extraction never enters the lexicon.
        let augmentation = match self.augment_options {
            AugmentOptions::LexiconBacked(_) => augment_lexicon_backed(
                &self.base,
                document,
                self.abbreviation_proposer,
                self.category_proposer,
                self.lemmatizer,
            ),
            // `DocumentOnly` and (until Phase 3) `LlmBacked` use the deterministic document-only harvest.
            _ => augment_document_only(
                &self.base,
                document,
                self.abbreviation_proposer,
                self.lemmatizer,
            ),
        };
        let mut builder = LayerBuilder::new("doc-glossary", Some(Arc::clone(&self.base)));
        for r in augmentation.resources() {
            let _ = builder.add_resource(r);
        }
        let doc_layer = Arc::new(builder.build(LayerStorage::in_memory()));

        // Stage B + C — parse each body sentence over base + doc-glossary and resolve its referent holes
        // against the threaded discourse (the untrusted proposer suggests, the kernel re-gates).
        let index = Parser::build(Arc::clone(&doc_layer));
        let bodies: Vec<String> = segment_sentences(document)
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect();
        let refs: Vec<&str> = bodies.iter().map(String::as_str).collect();
        let outcomes = index.resolve_document(&refs, self.lemmatizer, self.anaphora_proposer);

        let sentences = bodies
            .into_iter()
            .zip(outcomes)
            .map(|(text, outcome)| SentenceEncoding { text, outcome })
            .collect();
        (
            DocumentEncoding {
                augmentation,
                sentences,
            },
            doc_layer,
        )
    }
}

impl DocumentPipeline for InProcessPipeline<'_> {
    /// Composes Stage A (glossary → in-memory doc layer) → Stage B+C (`resolve_document`). See
    /// [`InProcessPipeline::encode_with_layer`] for the variant that also returns the doc layer, which
    /// downstream in-process claim grading commits onto.
    fn encode(&self, document: &str) -> DocumentEncoding {
        self.encode_with_layer(document).0
    }
}
