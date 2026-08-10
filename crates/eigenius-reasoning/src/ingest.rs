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

//! **Ingestion** — the document→graded-claims path (D63): the composition of the DCG pipeline
//! ([`eigenius_kernel::dcg::DocumentPipeline`]) with claim grading ([`crate::grade::ClaimGrader`]).
//!
//! This is the "layer up": [`DocumentPipeline`] turns prose into per-sentence closed propositions;
//! [`ClaimGrader`] turns each closed proposition into a graded, kernel-checked claim. [`DocumentIngestion`]
//! runs both — encode, then grade every `Encoded` sentence, commit the claim clusters onto the same doc
//! chain the sentences were parsed over, and validate each through the D39 gate. It is the first-class
//! form of the end-to-end "algorithm works" harness (previously inline in test code).
//!
//! **Fail-closed, in-process caveat.** The in-process impl validates *post-hoc* and records the verdict
//! per sentence — a `Fails` is surfaced as a finding, never silently passed. The **served** path commits
//! through the registered AutoOnLoad gate, which *rejects* a `Fails` sentence at commit (hard
//! fail-closed); that is the Phase-2 realization, behind the same [`DocumentIngestion`] contract.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::dcg::{
    AbbreviationProposer, InProcessPipeline, Lemmatizer, LexiconAugmentation, Proposer,
    SentenceEncoding, SentenceOutcome,
};
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

use crate::grade::{ClaimGrader, ClaimSource, GradedClaim, Warrant};
use crate::validate::do_validate_justification;
use crate::ReasoningInstitution;

/// Encode a document all the way to graded, validated claims: prose → per-sentence closed propositions
/// (the pipeline) → graded claims committed + checked (the grader + the D39 gate).
pub trait DocumentIngestion {
    /// Ingest `document`, rooting the claim IRIs under `doc_id` (an IRI-safe document identifier).
    fn ingest(&self, doc_id: &str, document: &str) -> IngestedDocument;
}

/// The D39 gate's verdict on one committed claim.
#[derive(Debug)]
pub enum ClaimVerdict {
    /// The certificate type-checks against the admitted witness.
    Holds,
    /// The certificate does not type-check; carries the gate diagnostic (surfaced, not dropped).
    Fails(String),
}

/// One sentence's ingestion result: the pipeline outcome, the graded claim built for it (for an
/// `Encoded` reading), and the gate's verdict on that claim.
pub struct IngestedSentence {
    pub text: String,
    /// The pipeline's parse/resolve classification.
    pub outcome: SentenceOutcome,
    /// The graded claim built from a closed reading. `Some` only for `Encoded`; `None` for
    /// `Ambiguous` / `Open` / `Gap`, or if the proposition failed to grade (recorded, not silently dropped).
    pub claim: Option<GradedClaim>,
    /// The D39 gate's verdict on [`Self::claim`]. `Some` iff a claim was built and validated.
    pub verdict: Option<ClaimVerdict>,
}

/// The ingestion of a whole document: the Stage-A lexicon augmentation, one result per body sentence, and
/// the committed doc-claims layer (base → glossary → the claim clusters).
pub struct IngestedDocument {
    pub augmentation: LexiconAugmentation,
    pub sentences: Vec<IngestedSentence>,
    /// The committed layer carrying every claim cluster, chained on the parsed doc chain.
    pub layer: Arc<Layer>,
}

impl IngestedDocument {
    /// The sentences that closed *and* validated `Holds` — the trustworthy graded claims.
    pub fn encoded_holds(&self) -> impl Iterator<Item = &IngestedSentence> {
        self.sentences
            .iter()
            .filter(|s| matches!(s.verdict, Some(ClaimVerdict::Holds)))
    }
}

/// The Phase-1 **in-process** ingestion: composes an [`InProcessPipeline`] with a [`ClaimGrader`], all
/// in Rust (LLM steps behind the proposer traits, `--features use-llm`). A served realization swaps the
/// proposers for RPC-backed ones and commits through the gated path — same [`DocumentIngestion`] contract.
pub struct InProcessIngestion<'a> {
    base: Arc<Layer>,
    lemmatizer: &'a dyn Lemmatizer,
    abbreviation_proposer: &'a dyn AbbreviationProposer,
    anaphora_proposer: &'a dyn Proposer,
    grader: &'a dyn ClaimGrader,
}

impl<'a> InProcessIngestion<'a> {
    pub fn new(
        base: Arc<Layer>,
        lemmatizer: &'a dyn Lemmatizer,
        abbreviation_proposer: &'a dyn AbbreviationProposer,
        anaphora_proposer: &'a dyn Proposer,
        grader: &'a dyn ClaimGrader,
    ) -> Self {
        Self {
            base,
            lemmatizer,
            abbreviation_proposer,
            anaphora_proposer,
            grader,
        }
    }
}

impl DocumentIngestion for InProcessIngestion<'_> {
    fn ingest(&self, doc_id: &str, document: &str) -> IngestedDocument {
        // Stage A/B/C — parse + resolve, keeping the doc-glossary layer so claims commit onto the same
        // chain (a claim's proposition may reference a doc-glossary-only concept).
        let pipeline = InProcessPipeline::new(
            Arc::clone(&self.base),
            self.lemmatizer,
            self.abbreviation_proposer,
            self.anaphora_proposer,
        );
        let (encoding, doc_layer) = pipeline.encode_with_layer(document);

        // Grade each closed sentence into its claim cluster; collect the cluster resources to commit.
        let mut sentences: Vec<IngestedSentence> = Vec::with_capacity(encoding.sentences.len());
        let mut cluster_resources: Vec<Resource> = Vec::new();
        for (i, SentenceEncoding { text, outcome }) in encoding.sentences.into_iter().enumerate() {
            let claim = if let SentenceOutcome::Encoded(item) = &outcome {
                let stem = format!("urn:eigenius:doc:{doc_id}:s{i}");
                match self.grader.grade(
                    item.sem(),
                    &ClaimSource {
                        stem: &stem,
                        warrant: Warrant::Declared,
                        declared_by: "encoding-pipeline",
                        timestamp: "2026-08-03T00:00:00Z",
                    },
                ) {
                    Ok(claim) => {
                        cluster_resources.extend(claim.resources.iter().cloned());
                        Some(claim)
                    }
                    // Fail-closed: an un-gradable proposition yields no claim (recorded as None), never
                    // a silently-passed one.
                    Err(_) => None,
                }
            } else {
                None
            };
            sentences.push(IngestedSentence {
                text,
                outcome,
                claim,
                verdict: None,
            });
        }

        // Commit every cluster onto the doc chain. Witness admission is answered by direct
        // lookup against the layer (D66 slice 0), so there is nothing to pre-build here.
        let mut builder = LayerBuilder::new("doc-claims", Some(Arc::clone(&doc_layer)));
        for r in cluster_resources {
            let _ = builder.add_resource(r);
        }
        let claims_layer = Arc::new(builder.build(LayerStorage::in_memory()));

        // Validate each claim through the D39 gate against the committed chain; record the verdict.
        let ctx = ExecutionContext::new(
            Arc::clone(&claims_layer),
            "ingest",
            ExecutionMode::ReadOnly,
            LayerStorage::in_memory(),
        );
        let institution = ReasoningInstitution::new();
        for sentence in &mut sentences {
            let Some(claim) = &sentence.claim else {
                continue;
            };
            let Some(sentence_res) = claim
                .resources
                .iter()
                .find(|r| r.id() == Some(&claim.sentence_iri))
            else {
                continue;
            };
            sentence.verdict = Some(
                match do_validate_justification(&institution, sentence_res, &ctx) {
                    Ok(outcome) if verdict_ctor(&outcome.output) == wk::VERDICT_HOLDS => {
                        ClaimVerdict::Holds
                    }
                    Ok(outcome) => {
                        ClaimVerdict::Fails(verdict_diagnostic(&outcome.output).unwrap_or_default())
                    }
                    Err(e) => ClaimVerdict::Fails(format!("{e:?}")),
                },
            );
        }

        IngestedDocument {
            augmentation: encoding.augmentation,
            sentences,
            layer: claims_layer,
        }
    }
}

/// Read the `ctor_name` discriminator off a verdict resource (`Holds` vs `Fails`).
fn verdict_ctor(r: &Resource) -> String {
    r.get(&Iri::parse(wk::CTOR_NAME).expect("static ctor_name IRI"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default()
}

/// Read the `diagnostic` field off a `Fails` verdict resource.
fn verdict_diagnostic(r: &Resource) -> Option<String> {
    r.get(&Iri::parse("urn:eigenius:institution:diagnostic").expect("static diagnostic IRI"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}
