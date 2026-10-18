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

//! Registry records, field names verbatim from the PMI Case Registry
//! (`experiments/uab/UAB Round 1/02-synthetic-pmi-registry/registry-schema.md`). Fields the converter
//! does not read are not modelled; [`UNCONVERTED_FIELDS`] names them, with why, for the report.

use serde::Deserialize;

/// The package: the cases, with their linked HP-term and variant records inlined.
#[derive(Debug, Deserialize)]
pub struct Package {
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
pub struct Case {
    #[serde(rename = "Case ID")]
    pub case_id: String,
    #[serde(rename = "Year of Birth")]
    pub year_of_birth: Option<i64>,
    #[serde(rename = "Gender", default)]
    pub gender: Vec<String>,
    #[serde(
        rename = "Consented to Consultations in Precision Medicine Study",
        default
    )]
    pub study_consent: bool,
    #[serde(rename = "Consented to genomic data sharing", default)]
    pub genomic_consent: bool,
    #[serde(rename = "Case Created Timestamp")]
    pub created: Option<String>,
    #[serde(rename = "Diagnosis")]
    pub diagnosis: Option<String>,
    #[serde(rename = "Symptoms")]
    pub symptoms: Option<String>,
    #[serde(rename = "Case History")]
    pub case_history: Option<String>,
    #[serde(rename = "Participant/Family Goal")]
    pub family_goal: Option<String>,
    #[serde(rename = "Medications")]
    pub medications: Option<String>,
    #[serde(rename = "Case Review Next Steps")]
    pub next_steps: Option<String>,
    #[serde(rename = "Outcome (Details)")]
    pub outcome_details: Option<String>,
    /// A link to the tier-3 records (clinic notes, lab reports). Not loaded: it becomes a source stub.
    #[serde(rename = "Medical Records")]
    pub medical_records: Option<String>,
    /// A link to the tier-2 analyst notes. Not loaded: it becomes a source stub.
    #[serde(rename = "Case Notes")]
    pub case_notes: Option<String>,
    #[serde(default)]
    pub linked_records: Linked,
}

#[derive(Debug, Default, Deserialize)]
pub struct Linked {
    #[serde(rename = "HP Terms", default)]
    pub hp_terms: Vec<HpTerm>,
    #[serde(rename = "Gene Info", default)]
    pub gene_info: Vec<GeneRow>,
}

#[derive(Debug, Deserialize)]
pub struct HpTerm {
    #[serde(rename = "Phenotype")]
    pub phenotype: String,
    #[serde(rename = "HP ID")]
    pub hp_id: String,
}

#[derive(Debug, Deserialize)]
pub struct GeneRow {
    #[serde(rename = "Gene Name")]
    pub gene_name: String,
    #[serde(rename = "HGNC ID")]
    pub hgnc_id: Option<String>,
    #[serde(rename = "RefSeq")]
    pub refseq: Option<String>,
    #[serde(rename = "Genotype")]
    pub genotype: Option<String>,
    #[serde(rename = "Protein Sequence Change")]
    pub protein_change: Option<String>,
    #[serde(rename = "HGVS Notation")]
    pub hgvs: Option<String>,
    #[serde(rename = "Zygosity")]
    pub zygosity: Option<String>,
    #[serde(rename = "ACMG Classification")]
    pub acmg: Option<String>,
    #[serde(rename = "Variant Impact")]
    pub impact: Option<String>,
    #[serde(rename = "Confirmed de novo", default)]
    pub de_novo: bool,
    #[serde(rename = "Notes")]
    pub notes: Option<String>,
}

/// Registry fields the converter does not convert, with why. D99 §6 types most of them; the
/// vocabularies they need (regions under a part-of relation, OMB categories, workflow states,
/// outcome records) are not built, and none is needed for the SYN-26-002 claims.
pub const UNCONVERTED_FIELDS: &[(&str, &str)] = &[
    (
        "Participant Location",
        "a region under a part-of relation (D99 §7); no region vocabulary",
    ),
    (
        "Ethnicity",
        "an OMB category (D99 §6); no category vocabulary",
    ),
    (
        "Status, New Status Tags, Analyst Case Status, Case Category, Case Origin",
        "workflow state with its timestamp (D99 §6); no workflow vocabulary",
    ),
    (
        "timestamps other than Case Created",
        "the timestamp of a workflow state change (D99 §6); no workflow vocabulary",
    ),
    (
        "Outcome, Research Report Sent, Action Was Taken on Research Report",
        "outcome records (D99 §6); no outcome vocabulary",
    ),
    (
        "Age, Age Calculation",
        "not stored: age is computed from Year of Birth for a stated date (D99 §7)",
    ),
    (
        "Gene Info: Variant Type, Genomic Region",
        "not typed by D99",
    ),
];
