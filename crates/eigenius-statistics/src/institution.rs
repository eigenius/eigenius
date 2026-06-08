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

//! `StatisticsInstitution` — the D52 measurement-statistics institution.
//!
//! Stateless: every `query` call resolves the SampleSet, decodes its
//! product position, and runs the recomputation procedure afresh. No
//! per-Layer caching in Phase 1.

use std::sync::Arc;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::{Institution, QueryOutcome};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;

use crate::validate::do_validate_measurement_claim;

/// Canonical IRIs the statistics institution dispatches on. Pinned
/// here so a downstream caller (the bootstrap registration hook, a
/// test harness building synthetic MeasurementClaims) reaches for the
/// same strings the chain ontology declared.
///
/// Matches the resource declarations in
/// [`ontologies/statistics/statistics.esl`](../../../ontologies/statistics/statistics.esl).
pub mod iris {
    // ── Institution + procedure ──────────────────────────────────────
    pub const INSTITUTION: &str = "urn:eigenius:measurements:statistics_institution";
    pub const PROC_VALIDATE_MEASUREMENT_CLAIM: &str =
        "urn:eigenius:measurements:proc:validate_measurement_claim";

    // ── MeasurementClaim property IRIs (D52 §3) ──────────────────────
    pub const PROP_SAMPLE_SET: &str = "urn:eigenius:measurements:sample_set";
    pub const PROP_NULL_HYPOTHESIS: &str = "urn:eigenius:measurements:null_hypothesis";
    pub const PROP_ALTERNATIVE_HYPOTHESIS: &str =
        "urn:eigenius:measurements:alternative_hypothesis";
    // D52 reads the predicate the claim establishes from the
    // inherited `reflection:canonical_proposition` slot — the
    // statistics ontology no longer declares a parallel
    // `stats:derived_proposition` property (one slot, one source of
    // truth across the four reflection-ontology resource classes).
    pub const PROP_CANONICAL_PROPOSITION: &str = "urn:eigenius:reflection:canonical_proposition";
    pub const PROP_ALPHA: &str = "urn:eigenius:measurements:alpha";
    pub const PROP_EFFECT_SIZE: &str = "urn:eigenius:measurements:effect_size";
    pub const PROP_DIRECTIONALITY: &str = "urn:eigenius:measurements:directionality";
    pub const PROP_VARIANCE_ASSUMPTION: &str = "urn:eigenius:measurements:variance_assumption";
    pub const PROP_OUTLIER_EXCLUSION: &str = "urn:eigenius:measurements:outlier_exclusion";
    pub const PROP_AUTOCORRELATION_STRUCTURE: &str =
        "urn:eigenius:measurements:autocorrelation_structure";

    // ── Replicate property IRIs ──────────────────────────────────────
    pub const PROP_VALUE: &str = "urn:eigenius:measurements:value";
    pub const PROP_UNIT_ID: &str = "urn:eigenius:measurements:unit_id";
    pub const PROP_TREATMENT_LEVEL: &str = "urn:eigenius:measurements:treatment_level";

    // ── Inductive type / class IRIs ──────────────────────────────────
    pub const SAMPLE_SET: &str = "urn:eigenius:measurements:SampleSet";
    pub const REPLICATE: &str = "urn:eigenius:measurements:Replicate";
    pub const MEASUREMENT_CLAIM: &str = "urn:eigenius:measurements:MeasurementClaim";
    pub const MEASUREMENT_VERDICT: &str = "urn:eigenius:measurements:MeasurementVerdict";
    pub const POPULATION_LEVEL: &str = "urn:eigenius:measurements:PopulationLevel";
    pub const MEASUREMENT_LEVEL: &str = "urn:eigenius:measurements:MeasurementLevel";

    // ── MeasurementVerdict property IRIs (Holds-output shape) ────────
    pub const PROP_SOURCE_CLAIM: &str = "urn:eigenius:measurements:source_claim";
    pub const PROP_VERDICT_CTOR: &str = "urn:eigenius:measurements:verdict_ctor";
    pub const PROP_COMPUTED_STATISTIC: &str = "urn:eigenius:measurements:computed_statistic";
    pub const PROP_COMPUTED_P_VALUE: &str = "urn:eigenius:measurements:computed_p_value";
    pub const PROP_DUAL_VERDICT_PAIR: &str = "urn:eigenius:measurements:dual_verdict_pair";
}

/// In-process measurement-statistics institution.
pub struct StatisticsInstitution {
    iri: Iri,
}

impl StatisticsInstitution {
    /// Construct a fresh institution bound to the canonical
    /// `urn:eigenius:measurements:statistics_institution` IRI.
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

impl Default for StatisticsInstitution {
    fn default() -> Self {
        Self::new()
    }
}

impl Institution for StatisticsInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        _resource: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "StatisticsInstitution has no extract_typed handler for `{procedure_iri}` \
             (Phase 1 declares no ExportFormat resources)"
        )))
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        _value: &Val,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "StatisticsInstitution has no reify handler for `{procedure_iri}` \
             (Phase 1 declares no ImportFormat resources)"
        )))
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        match procedure_iri.as_str() {
            iris::PROC_VALIDATE_MEASUREMENT_CLAIM => {
                do_validate_measurement_claim(self, input, ctx)
            }
            _ => Err(InstitutionError::NotImplemented(format!(
                "StatisticsInstitution has no query handler for procedure `{procedure_iri}`"
            ))),
        }
    }
}
