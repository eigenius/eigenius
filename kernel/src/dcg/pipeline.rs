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

use crate::commit::{BackendPersister, LayerPersister};
use crate::layer::{Layer, LayerBuilder, LayerStorage};
use crate::storage::PersistentBackend;

use super::abbrev::AbbreviationProposer;
use super::augment::{
    augment_document_only, augment_lexicon_backed, AugmentOptions, CategoryProposer,
    LexiconAugmentation, NominalCategoryProposer,
};
use super::lemmatizer::Lemmatizer;
use super::parse::{ClaimLander, Parser, Proposer, SelectionOutcome, SentenceOutcome};
use super::reading_ranker::ReadingRanker;
use super::segment::segment_sentences;

/// The document→encoding pipeline: raw document text → typed propositions, one [`SentenceOutcome`] per
/// body sentence. Fail-closed — an un-encodable sentence is `Open`/`Gap`, never a wrong closed parse.
/// Fallible: a pipeline configured with persistent storage (D67 §2) can fail at the doc-layer
/// commit; the in-memory arm never errs.
pub trait DocumentPipeline {
    fn encode(&self, document: &str) -> Result<DocumentEncoding, PipelineError>;
}

/// A pipeline failure — today only the persistent doc-layer commit (D67 §2). Parse-level
/// failures are per-sentence [`SentenceOutcome`]s, never errors: a document with gaps still
/// encodes, honestly.
#[derive(Debug)]
pub enum PipelineError {
    /// The doc layer could not be committed to its branch (storage error, or the branch CAS did
    /// not advance — e.g. a concurrent writer moved `doc-<id>`).
    Persist(String),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineError::Persist(m) => write!(f, "doc-layer commit failed: {m}"),
        }
    }
}

impl std::error::Error for PipelineError {}

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

/// One body sentence's encoding: its surface text, the classified [`SentenceOutcome`], and the
/// audit records — the selection record when a reading ranker collapsed an ambiguous forest
/// (emitted downstream as the claim's `enc:DecisionPoint`) and the binding record when the
/// encoded reading is a resolved open parse (emitted as `enc:AnaphorBinding`s, D67 §3).
#[derive(Clone)]
pub struct SentenceEncoding {
    pub text: String,
    pub outcome: SentenceOutcome,
    pub selection: Option<SelectionOutcome>,
    pub resolution: Option<super::parse::ResolutionOutcome>,
}

/// The Phase-1 **in-process** pipeline: every stage runs in Rust, with the LLM steps behind the proposer
/// traits. By default it chains the document glossary onto `base` in an **in-memory** layer (fine
/// for tests and small bases); over a DB-backed `base` configure [`Self::with_storage`] — the doc
/// layer is then built ON the persistent store and committed to a `doc-<id>` branch (D67 §2; an
/// in-memory overlay over the persisted lexicon OOMs, §7-2).
pub struct InProcessPipeline<'a> {
    base: Arc<Layer>,
    lemmatizer: &'a dyn Lemmatizer,
    abbreviation_proposer: &'a dyn AbbreviationProposer,
    anaphora_proposer: &'a dyn Proposer,
    category_proposer: &'a dyn CategoryProposer,
    augment_options: AugmentOptions,
    /// The reading-selection stage (`docs/notes/d63-reading-selection.md`). `None` (the default)
    /// keeps ambiguous sentences `Ambiguous` — the deterministic no-regression arm.
    reading_ranker: Option<&'a dyn ReadingRanker>,
    /// D67 §2: `Some((backend, doc_id))` builds the doc layer on the persistent store and
    /// commits it to branch `doc-<doc_id>` (drop-and-recreate lifecycle — pre-production).
    storage: Option<(Arc<dyn PersistentBackend>, String)>,
    /// The LANDING seam (D67 §4 / D68): lands each encoded sentence's claim inside the
    /// discourse loop so later sentences can refer to it («These findings…»). `None` = no
    /// landing, no claim antecedents.
    claim_lander: Option<&'a dyn ClaimLander>,
    /// Configures the internally-built [`Parser`] — sense cap, cell beam, sense-rank
    /// record/replay, document context — the same knobs the measurement harness sets
    /// (`with_sense_cap(2)`, `with_cell_beam(64)`, …). Without one the parser runs on its
    /// defaults, which is only safe over small bases; over the full lexicon the caps and the
    /// rank replay are load-bearing.
    parser_setup: Option<&'a dyn Fn(Parser) -> Parser>,
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
            reading_ranker: None,
            storage: None,
            claim_lander: None,
            parser_setup: None,
        }
    }

    /// Install the claim lander (D68) — landed claims join the discourse candidate set.
    pub fn with_claim_lander(mut self, lander: &'a dyn ClaimLander) -> Self {
        self.claim_lander = Some(lander);
        self
    }

    /// Install the parser-configuration hook (see the field doc) — applied to the freshly-built
    /// [`Parser`] before any sentence is parsed.
    pub fn with_parser_setup(mut self, setup: &'a dyn Fn(Parser) -> Parser) -> Self {
        self.parser_setup = Some(setup);
        self
    }

    /// Install the (untrusted) [`ReadingRanker`] that collapses an ambiguous sentence to one
    /// reading, in document context. Without one, ambiguous sentences stay `Ambiguous`.
    pub fn with_reading_ranker(mut self, ranker: &'a dyn ReadingRanker) -> Self {
        self.reading_ranker = Some(ranker);
        self
    }

    /// Persist the doc layer (D67 §2): build it ON `backend`'s storage — never as an in-memory
    /// overlay, which both OOMs over a DB-backed base (§7-2) and would skip the index lifecycle
    /// (derived indexes populate in `store_layer`) — and commit it to branch `doc-<doc_id>`,
    /// pointed at `base`'s head first so the persister's CAS creates-or-advances it.
    /// **Drop-and-recreate lifecycle** (pre-production): a rerun of the same `doc_id` replaces
    /// the branch; the interactive chain is never advanced by the pipeline — landing onto it is
    /// an explicit downstream load of the generated artifact.
    pub fn with_storage(mut self, backend: Arc<dyn PersistentBackend>, doc_id: &str) -> Self {
        self.storage = Some((backend, doc_id.to_string()));
        self
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

    /// Like [`DocumentPipeline::encode`], but also returns the doc-glossary layer the sentences
    /// were parsed over (`base` + the glossary) — in-memory by default, the committed `doc-<id>`
    /// branch head under [`Self::with_storage`]. An in-process downstream stage — claim grading
    /// in `eigenius-reasoning` — commits onto *this* layer, so a claim whose proposition
    /// references a doc-glossary-only concept (a grounding-miss minted class) still resolves in
    /// the chain. The trait's [`DocumentPipeline::encode`] drops it; a served realization returns
    /// a committed branch instead, which is why the layer is exposed here (inherent), not on the
    /// trait.
    pub fn encode_with_layer(
        &self,
        document: &str,
    ) -> Result<(DocumentEncoding, Arc<Layer>), PipelineError> {
        // Stage A — the lexicon augmentation: harvest the document's abbreviation definitions (and, under
        // `LexiconBacked`, ground residual OOV atoms against the form text index) as grounded proposals (+
        // the residual OOV gaps), and commit its resources as a doc-scoped lexicon layer on `base`.
        // Note what does NOT happen here: `LayerBuilder::add_resource` runs no validation. It
        // checks for a missing `@id`, rejects a core-namespace write, and inserts. The felicity
        // gate (`dcg::lexicon::gate_entry`) is not on this path and is not called anywhere under
        // `kernel/src/{validation,layer,commit}` — its ten call sites are the importer binaries,
        // one CLI subcommand and tests. What filters a mis-extraction here is the proposer's own
        // fail-closed minting above, not a kernel gate.
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
        let doc_layer = match &self.storage {
            None => Arc::new(builder.build(LayerStorage::in_memory())),
            Some((backend, doc_id)) => {
                // D67 §2: build the layer ON the storage it is persisted to (the index-lifecycle
                // invariant — derived indexes populate in `store_layer`), then commit it to the
                // doc branch. Drop-and-recreate: point the branch at the layer's PARENT first,
                // so the persister's CAS (expected_old = parent) creates-or-replaces it
                // deterministically whatever a previous run left behind.
                let branch = format!("doc-{doc_id}");
                let layer =
                    Arc::new(builder.build(LayerStorage::with_persistent(Arc::clone(backend))));
                let _ = backend.delete_branch(&branch);
                backend
                    .put_branch(&branch, self.base.id())
                    .map_err(|e| PipelineError::Persist(format!("point {branch} at base: {e}")))?;
                let persister = BackendPersister::new(Some(Arc::clone(backend)));
                let info = persister
                    .persist(&branch, &layer)
                    .map_err(|e| PipelineError::Persist(format!("{e:?}")))?;
                if !info.branch_advanced {
                    return Err(PipelineError::Persist(format!(
                        "branch {branch} did not advance (merge outcome {:?})",
                        info.merge_outcome
                    )));
                }
                layer
            }
        };

        // Stage B + C — parse each body sentence over base + doc-glossary and resolve its referent holes
        // against the threaded discourse (the untrusted proposer suggests, the kernel re-gates).
        let mut index = Parser::build(Arc::clone(&doc_layer));
        if let Some(setup) = self.parser_setup {
            index = setup(index);
        }
        let bodies: Vec<String> = segment_sentences(document)
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect();
        let refs: Vec<&str> = bodies.iter().map(String::as_str).collect();
        let resolutions = index.resolve_document(
            document,
            &refs,
            self.lemmatizer,
            self.anaphora_proposer,
            self.reading_ranker,
            self.claim_lander,
        );

        let sentences = bodies
            .into_iter()
            .zip(resolutions)
            .map(|(text, r)| SentenceEncoding {
                text,
                outcome: r.outcome,
                selection: r.selection,
                resolution: r.resolution,
            })
            .collect();
        Ok((
            DocumentEncoding {
                augmentation,
                sentences,
            },
            doc_layer,
        ))
    }
}

impl DocumentPipeline for InProcessPipeline<'_> {
    /// Composes Stage A (glossary → doc layer, in-memory or persisted per configuration) →
    /// Stage B+C (`resolve_document`). See [`InProcessPipeline::encode_with_layer`] for the
    /// variant that also returns the doc layer, which downstream in-process claim grading
    /// commits onto.
    fn encode(&self, document: &str) -> Result<DocumentEncoding, PipelineError> {
        Ok(self.encode_with_layer(document)?.0)
    }
}
