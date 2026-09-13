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

//! The κ–τ institution (D91) — governed abduction over chain conclusions.
//!
//! Implements the commitment discipline of the κ–τ logic (arXiv:2608.08192) against
//! the chain-resident vocabulary in
//! [`ontologies/kappatau/kappatau.esl`](../../ontologies/kappatau/kappatau.esl). A
//! conclusion commits only if its score clears the threshold τ AND its separation
//! from every active, negatively interacting rival clears the margin δ(τ, −κ*).
//!
//! **It computes; it does not prove.** The threshold comparison is the same shape as
//! the statistics institution's p-value-against-alpha: evaluated in Rust, asserted as
//! a proposition, recorded by a `ProgramTrace`, earning no witness. So this is FORM
//! ONE of the D90 result contract — a proposition with a declared `expected_type` —
//! and there is no `logic_κτ` to mint, because `holds(logic, t, P)` would assert that
//! a checker verified `t` and no checker did.
//!
//! **It establishes `Commits(policy, φ)`, never `φ`.** Crossing that gap takes a
//! declared bridge attributed to an owner. Nothing in the contract enforces it, so it
//! is discipline backed by attribution rather than a guarantee of the type system.
//!
//! Two modules, split so the semantics can be checked without a chain:
//!
//! - [`scoring`] — the scoring semantics as arithmetic over plain data, tested
//!   against the paper's own worked examples.
//! - [`institution`] — reads the assessment off the chain, applies the policy's declared
//!   margin form, and emits the verdict and the decision.
//!
//! The kernel binary constructs one in `cli/src/main.rs` alongside Lean and Statistics.
//! Registering it costs nothing on a chain that has not loaded `kappatau.esl`: the
//! chain-scan registration pass finds no matching `institution:Institution` declaration
//! and nothing dispatches. Loading that ontology as a layer is what activates it, which
//! is what lets a pilot run without rebuilding the kernel.

pub mod institution;
pub mod iris;
pub mod scoring;

pub use institution::KappaTauInstitution;
