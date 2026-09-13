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

//! The IRIs this crate dispatches on, pinned against
//! [`ontologies/kappatau/kappatau.esl`](../../../ontologies/kappatau/kappatau.esl).

// ── Institution + procedure ──────────────────────────────────────────
pub const INSTITUTION: &str = "urn:eigenius:kappatau:kappa_tau_institution";
pub const PROC_ASSESS_COMMITMENT: &str = "urn:eigenius:kappatau:proc:assess_commitment";

// ── Classes ──────────────────────────────────────────────────────────
pub const COMMITMENT_ASSESSMENT: &str = "urn:eigenius:kappatau:CommitmentAssessment";
pub const COMMITMENT_DECISION: &str = "urn:eigenius:kappatau:CommitmentDecision";
pub const RIVAL_MARGIN: &str = "urn:eigenius:kappatau:RivalMargin";

// ── CommitmentPolicy ─────────────────────────────────────────────────
pub const PROP_THRESHOLD: &str = "urn:eigenius:kappatau:threshold";
pub const PROP_ACTIVATION: &str = "urn:eigenius:kappatau:activation";
pub const PROP_INTERACTION_SCALE: &str = "urn:eigenius:kappatau:interaction_scale";
pub const PROP_MARGIN_FORM: &str = "urn:eigenius:kappatau:margin_form";
pub const PROP_MARGIN_COEFFICIENT: &str = "urn:eigenius:kappatau:margin_coefficient";

// ── Margin forms ─────────────────────────────────────────────────────
pub const MARGIN_FORM_ZERO: &str = "urn:eigenius:kappatau:margin_forms:zero";
pub const MARGIN_FORM_BY_RIVALRY: &str = "urn:eigenius:kappatau:margin_forms:by_rivalry";
pub const MARGIN_FORM_BY_STAKES_AND_RIVALRY: &str =
    "urn:eigenius:kappatau:margin_forms:by_stakes_and_rivalry";

// ── Estimates ────────────────────────────────────────────────────────
pub const PROP_ESTIMATE_OF: &str = "urn:eigenius:kappatau:estimate_of";
pub const PROP_WEIGHT: &str = "urn:eigenius:kappatau:weight";
pub const PROP_BETWEEN: &str = "urn:eigenius:kappatau:between";
pub const PROP_KAPPA: &str = "urn:eigenius:kappatau:kappa";

// ── Rival set ────────────────────────────────────────────────────────
pub const PROP_RIVAL_OF: &str = "urn:eigenius:kappatau:rival_of";
pub const PROP_RIVALS: &str = "urn:eigenius:kappatau:rivals";

// ── Assessment ───────────────────────────────────────────────────────
pub const PROP_SUBJECT: &str = "urn:eigenius:kappatau:subject";
pub const PROP_POLICY: &str = "urn:eigenius:kappatau:policy";
pub const PROP_RIVAL_SET: &str = "urn:eigenius:kappatau:rival_set";
pub const PROP_PLAUSIBILITY_ESTIMATES: &str = "urn:eigenius:kappatau:plausibility_estimates";
pub const PROP_INTERACTION_ESTIMATES: &str = "urn:eigenius:kappatau:interaction_estimates";

// ── Decision ─────────────────────────────────────────────────────────
pub const PROP_FROM_ASSESSMENT: &str = "urn:eigenius:kappatau:from_assessment";
pub const PROP_COMPUTED_SCORE: &str = "urn:eigenius:kappatau:computed_score";
pub const PROP_RIVAL_MARGINS: &str = "urn:eigenius:kappatau:rival_margins";
pub const PROP_BINDING_RIVAL: &str = "urn:eigenius:kappatau:binding_rival";
pub const PROP_AGAINST: &str = "urn:eigenius:kappatau:against";
pub const PROP_RIVAL_SCORE: &str = "urn:eigenius:kappatau:rival_score";
pub const PROP_LIFTED_INTERACTION: &str = "urn:eigenius:kappatau:lifted_interaction";
pub const PROP_REQUIRED_MARGIN: &str = "urn:eigenius:kappatau:required_margin";
pub const PROP_ACHIEVED_SEPARATION: &str = "urn:eigenius:kappatau:achieved_separation";
pub const PROP_CLEARED: &str = "urn:eigenius:kappatau:cleared";

// ── Term vocabulary ──────────────────────────────────────────────────
pub const KT_COMMITS: &str = "urn:eigenius:kappatau:Commits";
pub const KT_SUSPENDED_AT: &str = "urn:eigenius:kappatau:SuspendedAt";
pub const KT_SCORE_OF: &str = "urn:eigenius:kappatau:score_of";

// ── Shared ───────────────────────────────────────────────────────────
pub const PROP_PROPOSITION: &str = "urn:eigenius:eigentt:proposition";
pub const PROP_GROUNDS_JUDGEMENT: &str = "urn:eigenius:justification:grounds_judgement";
