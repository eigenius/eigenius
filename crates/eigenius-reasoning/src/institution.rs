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

//! `ReasoningInstitution` — the D39 Justification Logic institution.
//!
//! Stateless: every `query` call resolves the justification:Certificate + justification:Term
//! inductives from the layer chain afresh. A future revision may cache
//! per-Layer-id decl resolution; the `Arc<I>` blanket impl in
//! [`eigenius_kernel::institution::runtime`] already permits state without
//! reconstructing on each registry rebuild.

use std::sync::Arc;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::{Institution, QueryOutcome};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;

use crate::consistency::do_consistency_check;
use crate::entailment::do_entailment_query;
use crate::extract::extract_justification;
use crate::validate::do_validate_justification;

/// Canonical IRIs the Reasoning institution dispatches on. Pinned here
/// so a downstream caller (the bootstrap registration hook, a test
/// harness building synthetic ReasoningSentences) reaches for the
/// same strings the chain ontology declared.
///
/// Matches the resource declarations in
/// [`ontologies/justification/justification.esl`](../../../ontologies/justification/justification.esl).
pub mod iris {
    /// The institution itself.
    pub const INSTITUTION: &str = "urn:eigenius:reasoning:reasoning_institution";

    /// AutoOnLoad procedure: type-check a justification:Conclusion's
    /// certificate against `justification:Certificate(justification, proposition)`.
    /// Load-bearing — every committed justification:Conclusion triggers it.
    pub const PROC_VALIDATE_JUSTIFICATION: &str =
        "urn:eigenius:reasoning:proc:validate_justification";

    /// ExportFormat procedure: lift a `justification:Conclusion`'s
    /// `justification` property (a D32 §3.7-shaped justification:Term
    /// chain value) into a typed `Val::InductiveVal`. Used by the
    /// validate handler to construct `justification:Certificate(j, p)`; available
    /// as a standalone extract route so any future cross-institution
    /// consumer can lift a justification through the same path.
    pub const PROC_EXTRACT_JUSTIFICATION: &str =
        "urn:eigenius:reasoning:proc:extract_justification";

    /// ExportFormat resource IRI referencing the procedure above.
    pub const EF_JUSTIFICATION: &str = "urn:eigenius:reasoning:ef_justification";

    /// OnDemand procedure: search for a justification:Term over Γ that
    /// witnesses a candidate Prop. v1 returns NotImplemented (Phase 7).
    pub const PROC_ENTAILMENT_QUERY: &str = "urn:eigenius:reasoning:proc:entailment_query";

    /// Decidable procedure: check propositional-fragment consistency
    /// over a committed-sentence set. v1 returns NotImplemented (Phase 7).
    pub const PROC_CONSISTENCY_CHECK: &str = "urn:eigenius:reasoning:proc:consistency_check";

    /// D73 §1.2 / eigenius#204 — project a retained justification:Term onto its support.
    pub const PROC_PROJECT_JUSTIFICATION: &str =
        "urn:eigenius:reasoning:proc:project_justification";
    pub const JUSTIFICATION_PROJECTION: &str = "urn:eigenius:justification:Projection";
    pub const PROP_SUBJECT_SENTENCE: &str = "urn:eigenius:justification:subject_sentence";
    pub const PROP_COUNTERFACTUAL_IRI: &str = "urn:eigenius:justification:counterfactual_iri";
    pub const PROP_SUPPORT_COUNT: &str = "urn:eigenius:justification:support_count";
    pub const PROP_FULLY_VERIFIED: &str = "urn:eigenius:justification:fully_verified";
    pub const PROP_DECLARED_GROUNDS: &str = "urn:eigenius:justification:declared_grounds";
    pub const PROP_OBSERVED_GROUNDS: &str = "urn:eigenius:justification:observed_grounds";
    pub const PROP_DERIVED_GROUNDS: &str = "urn:eigenius:justification:derived_grounds";
    pub const PROP_VERIFIED_GROUNDS: &str = "urn:eigenius:justification:verified_grounds";
    pub const PROP_SURVIVES_WITHOUT: &str = "urn:eigenius:justification:survives_without";

    // Property IRIs on justification:Conclusion — used by the validate handler
    // to read the three fields.
    /// The conclusion's single required judgement. The proposition and the
    /// justification term are projected out of its type rather than read from
    /// slots of their own.
    pub const PROP_JUDGEMENT: &str = "urn:eigenius:justification:judgement";

    // Inductive type IRIs the certificate type-check builds against.
    pub const JUSTIFICATION_TERM: &str = "urn:eigenius:justification:Term";
    pub const JUSTIFIED_BY: &str = "urn:eigenius:justification:Certificate";

    // EntailmentRequest / ConsistencyRequest property IRIs — used by
    // the OnDemand / Decidable handlers to read their inputs.
    pub const PROP_CANDIDATE_PROPOSITION: &str = "urn:eigenius:justification:candidate_proposition";
    pub const PROP_SENTENCE_SET: &str = "urn:eigenius:justification:sentence_set";
}

/// In-process Justification Logic institution.
pub struct ReasoningInstitution {
    iri: Iri,
}

impl ReasoningInstitution {
    /// Construct a fresh institution bound to the canonical
    /// `urn:eigenius:reasoning:reasoning_institution` IRI.
    pub fn new() -> Self {
        Self {
            iri: Iri::parse(iris::INSTITUTION).expect("static institution IRI"),
        }
    }

    /// Wrap a fresh institution in an `Arc<dyn Institution>` ready to
    /// hand to the kernel's in-process registry.
    pub fn arc() -> Arc<dyn Institution> {
        Arc::new(Self::new())
    }
}

impl Default for ReasoningInstitution {
    fn default() -> Self {
        Self::new()
    }
}

impl Institution for ReasoningInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        match procedure_iri.as_str() {
            iris::PROC_EXTRACT_JUSTIFICATION => extract_justification(resource, ctx),
            _ => Err(InstitutionError::NotImplemented(format!(
                "ReasoningInstitution has no extract_typed handler for `{procedure_iri}`"
            ))),
        }
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        _value: &Val,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        // v1 declares no ImportFormat resources. ReasoningSentences
        // are authored directly via Load, not constructed via reify.
        Err(InstitutionError::NotImplemented(format!(
            "ReasoningInstitution has no reify handler for `{procedure_iri}` \
             (v1 declares no ImportFormat resources)"
        )))
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        match procedure_iri.as_str() {
            iris::PROC_VALIDATE_JUSTIFICATION => do_validate_justification(self, input, ctx),
            iris::PROC_ENTAILMENT_QUERY => do_entailment_query(input, ctx),
            iris::PROC_CONSISTENCY_CHECK => do_consistency_check(input, ctx),
            iris::PROC_PROJECT_JUSTIFICATION => {
                crate::project::do_project_justification(input, ctx)
            }
            _ => Err(InstitutionError::NotImplemented(format!(
                "ReasoningInstitution has no query handler for procedure `{procedure_iri}`"
            ))),
        }
    }
}
