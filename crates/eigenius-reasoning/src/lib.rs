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

//! `eigenius-reasoning` — Justification Logic institution for Eigenius.
//!
//! Implements D39's `ReasoningInstitution` over the foundational
//! reasoning ontology declared in
//! [`ontologies/reasoning/reasoning.esl`](../../ontologies/reasoning/reasoning.esl).
//! The kernel binary registers one of these at startup
//! ([`startup::register`]) and the chain-scan registration pass wires
//! it into the institution runtime whenever it sees the
//! `reasoning:reasoning_institution` Institution resource.
//!
//! ## Phase 6 surface
//!
//! - `query(validate_justification, ReasoningSentence)` — **load-bearing**.
//!   Decodes proposition + certificate via the D47 codec, decodes
//!   justification via the chain inductive-value codec (D32 §3.7),
//!   constructs `JustifiedBy justification proposition`, type-checks
//!   the certificate against it via the kernel's NbE checker. Returns
//!   `Verdict::Holds | Fails { diagnostic }`.
//! - `query(entailment_query, _)` — `NotImplemented`. Phase 7.
//! - `query(consistency_check, _)` — `NotImplemented`. Phase 7.
//! - `extract_typed`, `reify` — `NotImplemented`. The Reasoning
//!   institution declares no `ExportFormat` / `ImportFormat` in v1
//!   (ReasoningSentences are user-authored directly, not constructed
//!   via cross-institution reify); the trait surface is preserved so a
//!   future ExportFormat (e.g. for a Reasoning → Lean comorphism per
//!   gh #73) can be added without changing dispatch.
//!
//! ## Why this is structurally light
//!
//! No `chain_mirror.rs` (parallel to `eigenius-lean/src/chain_mirror.rs`)
//! is needed because `JustificationTerm` and `JustifiedBy` are authored
//! via the eigenius#72 Layer-2 ESL surface and consumed by existing
//! kernel inductive machinery. No `checker.rs` is needed because there's
//! no external term checker — the validator routes directly through
//! `eigenius-kernel`'s NbE pipeline.
//!
//! `extract.rs` is required despite this, because the validate handler
//! needs the `justification` property (a D32 §3.7-shaped chain
//! inductive value) lifted into a kernel `Val` to construct
//! `JustifiedBy(j, p)` for type-checking. The lift goes through
//! `extract_typed` rather than a free helper so the abstraction stays
//! aligned with the kernel's standard "lift chain resource → typed Val"
//! shape — same surface every other institution uses.

pub mod consistency;
pub mod entailment;
pub mod extract;
pub mod grade;
pub mod ingest;
pub mod institution;
pub mod startup;
pub mod validate;

pub use grade::{
    ClaimGrader, ClaimSource, DeclaredClaimGrader, Grade, GradeError, GradedClaim, Warrant,
};
pub use ingest::{
    ClaimVerdict, DocumentIngestion, InProcessIngestion, IngestedDocument, IngestedSentence,
};
pub use institution::ReasoningInstitution;
