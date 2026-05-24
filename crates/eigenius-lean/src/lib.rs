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

//! `eigenius-lean` — Lean 4 verification institution for Eigenius.
//!
//! Wraps [`nanoda_lib`](nanoda_lib) (a vendored Lean 4 term checker)
//! behind a small [`check_proof`] surface. The crate's role per
//! [D28](../../docs/design/d28-lean-4-as-institution.md):
//!
//! - Verification side. The kernel binary links this crate and
//!   dispatches `urn:eigenius:lean:proof_check` through it. Verdicts
//!   are an in-process function call — no IPC, no orchestrator hop —
//!   so the verification TCB stays bounded by what nanoda accepts.
//! - 20a.3 (this milestone) ships the `check_proof` skeleton only.
//!   The `Institution` trait impl, the bytes → `lean:LeanExpr`
//!   chain-mirror translator, and the registration hook into
//!   `EigeniusService` land in 20a.4.
//!
//! Authoring side (`lean_export`, env images, mirror generator) lives
//! in [`eigenius-lean-runtime`](../eigenius-lean-runtime/) (Phase
//! 20a.5+).

pub mod chain_mirror;
pub mod checker;
pub mod institution;
pub mod startup;

pub use chain_mirror::{bytes_to_lean_expr, ChainMirrorError};
pub use checker::{check_proof, CheckError, Verdict};
pub use institution::LeanInstitution;
