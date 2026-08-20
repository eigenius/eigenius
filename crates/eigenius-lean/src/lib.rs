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
//! Wraps [`nanoda_lib`](nanoda_lib), a Lean 4 term checker, behind a
//! small [`check_proof`] surface. **nanoda is a Cargo git dependency,
//! not vendored in-tree**: `crates/eigenius-lean/Cargo.toml` pins
//! `git = "https://github.com/ammkrn/nanoda_lib", rev = "6d2f037"`,
//! which resolves to version 0.4.8-beta. Documentation that points at
//! `references/nanoda_lib/` is pointing at a git-ignored local clone
//! kept for reading — pinned at a *different* revision, `f58f2f6` —
//! and not at anything the build consumes. The crate's role per
//! [D28](../../docs/design/d28-lean-4-as-institution.md):
//!
//! - Verification side. The kernel binary links this crate and
//!   dispatches `urn:eigenius:lean:proof_check` through it. Verdicts
//!   are an in-process function call — no IPC, no orchestrator hop —
//!   so the verification TCB stays bounded by what nanoda accepts.
//! - Shipped: [`checker::check_proof`], the [`Institution`] impl in
//!   [`institution`], and the bytes → `lean:LeanExpr` chain-mirror
//!   translator in [`chain_mirror`].
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
