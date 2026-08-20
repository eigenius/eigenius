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

use crate::claim_kind::{frame_kind, KindClassifier, KIND_ASSERTION};
use crate::grade::{ClaimGrader, ClaimSource, DerivedClaimGrader, GradedClaim, Warrant};
use crate::validate::do_validate_justification;
use crate::ReasoningInstitution;

/// The [`ClaimLander`] realization (D68 §4/§6): lands each encoded sentence as the Derived
/// cluster, with the discourse kind assigned by (1) the deterministic frame table, else (2) the
/// installed [`KindClassifier`] (recorded, untrusted), else (3) the `enc:Assertion` default
/// (unreferable, fail-closed). The lander OWNS the built clusters — the discourse loop threads
/// only the claim resource + surface; the caller collects [`Self::take_landed`] afterwards to
/// commit/emit them.
pub struct DerivedClaimLander<'a> {
    doc_id: String,
    declared_by: String,
    timestamp: String,
    /// When set, claims are named `{ns}:claim_{ordinal+1}` instead of the trait path's
    /// `urn:eigenius:doc:{doc_id}:s{ordinal}:claim` — see [`Self::with_emission_namespace`].
    emission_ns: Option<String>,
    /// What the trace says the claim was derived FROM. Defaults to the doc id; a caller with real
    /// source identity supplies it via [`Self::with_source`].
    source_label: Option<String>,
    classifier: &'a dyn KindClassifier,
    landed: std::cell::RefCell<Vec<GradedClaim>>,
}

impl<'a> DerivedClaimLander<'a> {
    pub fn new(doc_id: &str, classifier: &'a dyn KindClassifier) -> Self {
        Self {
            doc_id: doc_id.to_string(),
            declared_by: "encoding-pipeline".to_string(),
            timestamp: "2026-08-03T00:00:00Z".to_string(),
            emission_ns: None,
            source_label: None,
            classifier,
            landed: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Name landed claims the way the EMITTER will name them (`{ns}:claim_{n}`, 1-based — the
    /// `emit_document` convention).
    ///
    /// A claim landed in-loop becomes an anaphora ANTECEDENT, and the binding records the
    /// antecedent by IRI. If the lander and the emitter name the same claim differently, the
    /// artifact's `enc:AnaphorBinding` points at a resource the artifact does not contain — a
    /// dangling reference that no validation catches, because the emitted document simply lacks
    /// the target. One claim, one identity: a driver that emits its own document sets this so
    /// both halves agree. Without it the trait's own naming stands (ingestion commits the
    /// clusters the lander built, so there is no second namer).
    pub fn with_emission_namespace(mut self, ns: &str) -> Self {
        self.emission_ns = Some(ns.to_string());
        self
    }

    /// Name the SOURCE in each trace's provenance, rather than the working-branch id.
    ///
    /// The default names `doc_id`, which is the `doc-<id>` WORKING branch — scaffolding that gets
    /// pruned (D71 §5). A durable trace naming a prunable branch is backwards, and it also makes
    /// the artifact depend on the branch name: the same source formalized under two doc ids
    /// produced two different artifacts, which the served-vs-CLI byte comparison caught
    /// (`2026-08-19`). Pass `"<path> (sha256 <hex>)"` — what the claim was actually derived from.
    pub fn with_source(mut self, label: &str) -> Self {
        self.source_label = Some(label.to_string());
        self
    }

    /// The accumulated claim clusters, in landing order (drains the lander).
    pub fn take_landed(&self) -> Vec<GradedClaim> {
        self.landed.take()
    }
}

impl eigenius_kernel::dcg::ClaimLander for DerivedClaimLander<'_> {
    fn land(
        &self,
        ordinal: usize,
        sentence: &str,
        gloss: &str,
        item: &eigenius_kernel::dcg::Item,
    ) -> Option<(Resource, String)> {
        // The frame table first (the prose marks the kind explicitly — «We hypothesized that…»);
        // only an unmarked sentence is put to the classifier.
        let mut kinds = match frame_kind(sentence) {
            Some(k) => vec![k],
            None => self.classifier.classify(ordinal, sentence, gloss).kinds,
        };
        if kinds.is_empty() {
            kinds = vec![Iri::parse(KIND_ASSERTION).expect("static kind IRI")];
        }
        let provenance = format!(
            "eigenius-reasoning lander: DCG parse (D63) of {}, sentence {ordinal} «{sentence}»",
            self.source_label.as_deref().unwrap_or(&self.doc_id)
        );
        let claim = match &self.emission_ns {
            // The emitter's identity: `{ns}:claim_{n}` / `{ns}:trace_{n}`, 1-based. Built through
            // the SAME `cluster()` constructor the trait path uses, so the shape is identical and
            // only the naming differs.
            Some(ns) => {
                let n = ordinal + 1;
                let (claim, trace) = DerivedClaimGrader::cluster(
                    &format!("{ns}:claim_{n}"),
                    &format!("{ns}:trace_{n}"),
                    item.sem(),
                    &provenance,
                    &self.timestamp,
                    &kinds,
                )
                .ok()?;
                let claim_iri = claim.id().expect("cluster sets the claim id").clone();
                GradedClaim {
                    resources: vec![claim, trace],
                    claim_iri,
                    gate_sentence: None,
                    grade: Warrant::Derived.grade(),
                }
            }
            None => DerivedClaimGrader
                .grade(
                    item.sem(),
                    &ClaimSource {
                        stem: &format!("urn:eigenius:doc:{}:s{}", self.doc_id, ordinal),
                        warrant: Warrant::Derived,
                        declared_by: &self.declared_by,
                        timestamp: &self.timestamp,
                        provenance: &provenance,
                        kind_classes: &kinds,
                    },
                )
                .ok()?, // un-gradable ⇒ nothing lands (fail closed, run breaks)
        };
        let resource = claim
            .resources
            .iter()
            .find(|r| r.id() == Some(&claim.claim_iri))?
            .clone();
        let surface = format!("claim {}: {}", ordinal + 1, gloss);
        self.landed.borrow_mut().push(claim);
        Some((resource, surface))
    }
}

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
        let (encoding, doc_layer) = pipeline
            .encode_with_layer(document)
            .expect("the in-memory pipeline arm is infallible");

        // Grade each closed sentence into its claim cluster; collect the cluster resources to commit.
        let mut sentences: Vec<IngestedSentence> = Vec::with_capacity(encoding.sentences.len());
        let mut cluster_resources: Vec<Resource> = Vec::new();
        for (i, SentenceEncoding { text, outcome, .. }) in
            encoding.sentences.into_iter().enumerate()
        {
            let claim = if let SentenceOutcome::Encoded(item) = &outcome {
                let stem = format!("urn:eigenius:doc:{doc_id}:s{i}");
                let provenance = format!(
                    "eigenius-reasoning ingest: DCG parse (D63) of document {doc_id}, sentence \
                     {i} «{text}»"
                );
                match self.grader.grade(
                    item.sem(),
                    &ClaimSource {
                        stem: &stem,
                        // Parsed sentences land DERIVED (D67 §1): a program (the parser)
                        // produced the claim from the source text; the trace is the warrant.
                        warrant: Warrant::Derived,
                        declared_by: "encoding-pipeline",
                        timestamp: "2026-08-03T00:00:00Z",
                        provenance: &provenance,
                        kind_classes: &[],
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
            // Only clusters carrying a gate-validatable ReasoningSentence get a verdict; a
            // Derived cluster's trust story is its ProgramTrace (D67 §1) — verdict stays `None`.
            let Some(gate_iri) = &claim.gate_sentence else {
                continue;
            };
            let Some(sentence_res) = claim.resources.iter().find(|r| r.id() == Some(gate_iri))
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
