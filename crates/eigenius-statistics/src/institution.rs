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
use eigenius_kernel::storage::content_array::ContentArrayStore;

use crate::validate::do_validate_analysis_plan;

/// Canonical IRIs the statistics institution dispatches on. Pinned
/// here so a downstream caller (the bootstrap registration hook, a
/// test harness building synthetic StatisticalAnalysisPlans) reaches for the
/// same strings the chain ontology declared.
///
/// Matches the resource declarations in
/// [`ontologies/statistics/statistics.esl`](../../../ontologies/statistics/statistics.esl).
pub mod iris {
    // ── Institution + procedure ──────────────────────────────────────
    pub const INSTITUTION: &str = "urn:eigenius:measurements:statistics_institution";
    pub const PROC_VALIDATE_ANALYSIS_PLAN: &str =
        "urn:eigenius:measurements:proc:validate_analysis_plan";

    // ── StatisticalAnalysisPlan property IRIs (D52 §3) ──────────────────────
    pub const PROP_SAMPLE_SET: &str = "urn:eigenius:measurements:sample_set";
    // D52 reads the predicate the SAP's analysis attests from the
    // inherited `reflection:canonical_proposition` slot on the
    // per-effect `StatisticalAnalysisResult` derivation, not from the
    // SAP itself — the verifier derives the proposition from the
    // SAP's parameters at validation time.
    pub const PROP_CANONICAL_PROPOSITION: &str = "urn:eigenius:reflection:canonical_proposition";
    pub const PROP_ALPHA: &str = "urn:eigenius:measurements:alpha";
    pub const PROP_EFFECT_SIZE: &str = "urn:eigenius:measurements:effect_size";
    pub const PROP_DIRECTIONALITY: &str = "urn:eigenius:measurements:directionality";
    pub const PROP_VARIANCE_ASSUMPTION: &str = "urn:eigenius:measurements:variance_assumption";
    pub const PROP_OUTLIER_EXCLUSION: &str = "urn:eigenius:measurements:outlier_exclusion";
    pub const PROP_AUTOCORRELATION_STRUCTURE: &str =
        "urn:eigenius:measurements:autocorrelation_structure";
    pub const PROP_MULTIPLE_COMPARISON_CORRECTION: &str =
        "urn:eigenius:measurements:multiple_comparison_correction";

    // ── Replicate property IRIs ──────────────────────────────────────
    pub const PROP_VALUE: &str = "urn:eigenius:measurements:value";
    pub const PROP_UNIT_ID: &str = "urn:eigenius:measurements:unit_id";
    pub const PROP_TREATMENT_LEVEL: &str = "urn:eigenius:measurements:treatment_level";

    // ── Inductive type / class IRIs ──────────────────────────────────
    pub const SAMPLE_SET: &str = "urn:eigenius:measurements:SampleSet";
    pub const REPLICATE: &str = "urn:eigenius:measurements:Replicate";
    pub const STATISTICAL_ANALYSIS_PLAN: &str = "urn:eigenius:measurements:StatisticalAnalysisPlan";
    pub const STATISTICAL_ANALYSIS_RESULT: &str =
        "urn:eigenius:measurements:StatisticalAnalysisResult";
    pub const POPULATION_LEVEL: &str = "urn:eigenius:measurements:PopulationLevel";
    pub const MEASUREMENT_LEVEL: &str = "urn:eigenius:measurements:MeasurementLevel";
    pub const IMPOSSIBILITY_WITNESS: &str = "urn:eigenius:measurements:ImpossibilityWitness";
    pub const METHOD_COMPARISON_ANALYSIS_PLAN: &str =
        "urn:eigenius:measurements:MethodComparisonAnalysisPlan";
    pub const CLASSIFICATION_ANALYSIS_PLAN: &str =
        "urn:eigenius:measurements:ClassificationAnalysisPlan";
    // Nested + crossed two-way ANOVA are no longer plan classes — they are
    // stats:Nested / stats:Crossed SampleSet smart-constructors whose
    // blocking ctor (NestedBlocking / CrossedBlocking) drives dispatch
    // (D52 §4.2). The plan that references them is a plain
    // StatisticalAnalysisPlan, so no per-class IRI const is needed here.

    // ── ClassificationAnalysisPlan property IRIs (D52 §2.2) ──────────────
    pub const PROP_CLASSIFICATION_THRESHOLD: &str =
        "urn:eigenius:measurements:classification_threshold";
    pub const PROP_MIN_PPV: &str = "urn:eigenius:measurements:min_ppv";
    pub const PROP_MIN_SENSITIVITY: &str = "urn:eigenius:measurements:min_sensitivity";

    // ── Nested/crossed two-way ANOVA subgroup partition (D52 §2.2 / §4.2) ──
    // The block (guide) partition sizes now live IN the SampleSet (carried as
    // elements [2] and [3] of the stats:Nested / stats:Crossed observations
    // wrapper `[group_a, group_b, subgroup_sizes_a, subgroup_sizes_b]`), not as
    // properties on the plan. The verifier reads them straight off the decoded
    // bundle's observations slot, so no plan-property IRI const is needed.

    // ── StatisticalAnalysisResult property IRIs (per-effect derivation shape) ─
    pub const PROP_VERDICT_CTOR: &str = "urn:eigenius:measurements:verdict_ctor";
    pub const PROP_COMPUTED_STATISTIC: &str = "urn:eigenius:measurements:computed_statistic";
    pub const PROP_COMPUTED_P_VALUE: &str = "urn:eigenius:measurements:computed_p_value";
    pub const PROP_DUAL_VERDICT_PAIR: &str = "urn:eigenius:measurements:dual_verdict_pair";
    pub const PROP_EFFECT_NAME: &str = "urn:eigenius:measurements:effect_name";

    // ── Parameter-symbol axioms + propositional primitives (D52 §3) ──
    //
    // Used by the canonical-proposition derivation: the institution
    // builds a chain-resident D47 type-fragment value whose ConstRef
    // leaves point at these declared resources. The hash of that value
    // is what the D49 witness index keys on; the consumer side
    // (D39 reasoning) constructs the matching Exp from a proof term
    // and arrives at the same hash via `encode_type → hash_proposition_value`.
    pub const STATS_FALSE: &str = "urn:eigenius:measurements:False";
    pub const STATS_MEAN_OF: &str = "urn:eigenius:measurements:mean_of";
    pub const STATS_VARIANCE_OF: &str = "urn:eigenius:measurements:variance_of";
    pub const STATS_MEDIAN_OF: &str = "urn:eigenius:measurements:median_of";
    pub const STATS_MEAN_DIFF_OF: &str = "urn:eigenius:measurements:mean_diff_of";
    pub const STATS_SLOPE_OF: &str = "urn:eigenius:measurements:slope_of";
    pub const STATS_INTERCEPT_OF: &str = "urn:eigenius:measurements:intercept_of";
    pub const STATS_SPEARMAN_RHO: &str = "urn:eigenius:measurements:spearman_rho";
    pub const STATS_PPV: &str = "urn:eigenius:measurements:ppv";
    pub const STATS_SENSITIVITY: &str = "urn:eigenius:measurements:sensitivity";
    pub const STATS_LT: &str = "urn:eigenius:measurements:lt";
    pub const STATS_LE: &str = "urn:eigenius:measurements:le";
    pub const STATS_GT: &str = "urn:eigenius:measurements:gt";
    pub const STATS_GE: &str = "urn:eigenius:measurements:ge";

    // ── ANOVA effect predicates + method-comparison predicate ────────
    pub const STATS_FACTOR_EFFECT_OF: &str = "urn:eigenius:measurements:factor_effect_of";
    pub const STATS_INTERACTION_EFFECT_OF: &str = "urn:eigenius:measurements:interaction_effect_of";
    pub const STATS_METHODS_AGREE: &str = "urn:eigenius:measurements:methods_agree";
}

/// In-process measurement-statistics institution.
pub struct StatisticsInstitution {
    iri: Iri,
    /// Reads content-verified columns from materialized `PinnedExternalFile`s
    /// for file-backed SampleSets (D53 §6.1). Defaults to `file://`-only; a
    /// deployment with an Oxen-backed depot constructs it with the depot's
    /// content-cache root via [`Self::with_content_store`].
    content_store: ContentArrayStore,
}

impl StatisticsInstitution {
    /// Construct a fresh institution bound to the canonical
    /// `urn:eigenius:measurements:statistics_institution` IRI. File-backed
    /// SampleSets resolve `file://` references only (no content cache).
    pub fn new() -> Self {
        Self {
            iri: Iri::parse(iris::INSTITUTION).expect("static institution IRI"),
            content_store: ContentArrayStore::new(),
        }
    }

    /// Construct with a content-array store backed by a local content-addressed
    /// cache (the depot's `extfile-cache`), so an Oxen-fetched
    /// `PinnedExternalFile` resolves by `content_hash` (D53 §6.1).
    pub fn with_content_store(content_store: ContentArrayStore) -> Self {
        Self {
            iri: Iri::parse(iris::INSTITUTION).expect("static institution IRI"),
            content_store,
        }
    }

    /// The content-array capability used for file-backed SampleSet observations.
    pub fn content_store(&self) -> &ContentArrayStore {
        &self.content_store
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
            iris::PROC_VALIDATE_ANALYSIS_PLAN => do_validate_analysis_plan(self, input, ctx),
            _ => Err(InstitutionError::NotImplemented(format!(
                "StatisticsInstitution has no query handler for procedure `{procedure_iri}`"
            ))),
        }
    }
}
