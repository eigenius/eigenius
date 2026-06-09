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

//! `eigenius-statistics` — measurement-statistics institution for Eigenius.
//!
//! Implements D52's `StatisticsInstitution` over the chain-resident
//! vocabulary declared in
//! [`ontologies/statistics/statistics.esl`](../../ontologies/statistics/statistics.esl).
//! The kernel binary registers one of these at startup
//! ([`startup::register`]) and the chain-scan registration pass wires
//! it into the institution runtime whenever it sees the
//! `stats:statistics_institution` Institution resource.
//!
//! ## Phase 1 surface (D52 §9)
//!
//! - `query(validate_analysis_plan, StatisticalAnalysisPlan)` — **load-bearing**.
//!   Reads the claim's `sample_set` reference, decodes the SampleSet's
//!   product position (5 axis values from the `Bundle` ctor),
//!   dispatches to a recomputation procedure, runs the §7.4 epistemic-
//!   scope admissibility check, and returns a gate `Verdict::Holds`
//!   (the SAP ran) plus one or more `StatisticalAnalysisResult`
//!   `InstitutionEmittedDerivation`s carrying the derived
//!   `canonical_proposition` + numerics per effect. The D49 §6 witness
//!   emitter walks the result resources directly to admit the
//!   IsDerivedAs witnesses. Gate Fails covers structural failures
//!   (missing field, unwired dispatch, scope violation) — no result is
//!   emitted in those cases.
//! - `extract_typed`, `reify` — `NotImplemented` in Phase 1. Future
//!   ExportFormats (e.g., a Measurement → Reasoning extract that lifts
//!   the DerivedResource shape into a typed value for downstream
//!   reasoning composition) land in later phases.
//!
//! ## Phase 1 dispatch coverage
//!
//! Only the `(CompleteRandom, Unblocked, NoFactor, _, CrossSectional)`
//! product position (`SingleSampleEstimate`) is implemented in Phase 1.
//! All other positions return `Verdict::Fails(WrongTestForDesign)`
//! until their verifier procedures land. The `IID` two-sample case is
//! the natural Phase 1.b addition (same code path with `SingleFactor`
//! instead of `NoFactor`); the Tier 2 mixed-effects cases land in
//! Phase 4.
//!
//! ## What this crate is *not*
//!
//! It does not host the smart-constructor macros — those live in
//! [`ontologies/statistics/statistics.esl`](../../ontologies/statistics/statistics.esl)
//! and expand at compile time via the kernel's ESL `macro` extension.
//! It does not host the universal Claim schema's predicate scope
//! markers — those are class declarations in the same ontology. This
//! crate owns only the *runtime verifier*: the Rust code that
//! recomputes claims from raw replicates and emits verdicts.

pub mod institution;
pub mod numerics;
pub mod startup;
pub mod validate;

pub use institution::StatisticsInstitution;
