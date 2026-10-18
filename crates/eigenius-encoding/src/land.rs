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

//! **Claim landing** — the [`ClaimLander`](eigenius_kernel::dcg::ClaimLander) the DCG discourse
//! loop calls once per encoded sentence.
//!
//! [`DerivedClaimLander`] builds the parsed-claim cluster ([`crate::grade::ParsedClaimGrader`])
//! and assigns the claim's discourse kind (D68 §4/§6) from the deterministic frame table, else the
//! installed [`KindClassifier`](crate::claim_kind::KindClassifier), else the `enc:Assertion`
//! default. It owns the clusters it builds; the caller collects [`DerivedClaimLander::take_landed`]
//! afterwards to commit or emit them.
//!
//! **Moved here from `eigenius-reasoning::ingest`.** The rest of that module was
//! `InProcessIngestion`, the pre-D67 second pipeline, superseded by this crate's
//! [`pipeline`](crate::pipeline) and deleted with the move.

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;

use crate::claim_kind::{frame_kind, KindClassifier, KIND_ASSERTION};
use crate::grade::{ClaimGrader, ClaimSource, GradedClaim, ParsedClaimGrader};

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
    /// Constructor argument names for the D47 codec, read from the chain this lander's claims
    /// will be committed to (D85 §5 step 4). A claim's proposition is encoded here.
    codec: eigenius_kernel::program::eigentt_type_mirror::CodecNames,
    landed: std::cell::RefCell<Vec<GradedClaim>>,
}

impl<'a> DerivedClaimLander<'a> {
    pub fn new(
        doc_id: &str,
        classifier: &'a dyn KindClassifier,
        codec: eigenius_kernel::program::eigentt_type_mirror::CodecNames,
    ) -> Self {
        Self {
            doc_id: doc_id.to_string(),
            // An agent IRI, not a program's name: `declared_by` is resource-typed since
            // D72 §3.2, and "which program computed this" is already recorded as the
            // ProgramTrace's `prov:was_generated_by`. A caller who knows the asserting agent
            // supplies it; absent that, the honest value is the absence marker.
            declared_by: crate::grade::UNATTRIBUTED_AGENT.to_string(),
            timestamp: "2026-08-03T00:00:00Z".to_string(),
            emission_ns: None,
            source_label: None,
            classifier,
            codec,
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
            "eigenius-encoding lander: DCG parse (D63) of {}, sentence {ordinal} «{sentence}»",
            self.source_label.as_deref().unwrap_or(&self.doc_id)
        );
        let claim = match &self.emission_ns {
            // The emitter's identity: `{ns}:claim_{n}` / `{ns}:trace_{n}`, 1-based. Built through
            // the SAME `cluster()` constructor the trait path uses, so the shape is identical and
            // only the naming differs.
            Some(ns) => {
                let n = ordinal + 1;
                let (claim, trace) = ParsedClaimGrader::cluster(
                    &format!("{ns}:claim_{n}"),
                    &format!("{ns}:trace_{n}"),
                    item.sem(),
                    &self.declared_by,
                    &self.timestamp,
                    &kinds,
                    &self.codec,
                )
                .ok()?;
                let claim_iri = claim.id().expect("cluster sets the claim id").clone();
                GradedClaim {
                    resources: vec![claim, trace],
                    claim_iri,
                    gate_sentence: None,
                }
            }
            None => ParsedClaimGrader
                .grade(
                    item.sem(),
                    &ClaimSource {
                        stem: &format!("urn:eigenius:doc:{}:s{}", self.doc_id, ordinal),
                        declared_by: &self.declared_by,
                        timestamp: &self.timestamp,
                        provenance: &provenance,
                        kind_classes: &kinds,
                    },
                    &self.codec,
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
