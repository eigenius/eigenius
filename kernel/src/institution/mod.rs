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

//! Institution support.
//!
//! Institutions are declared in the layer chain as ordinary Eigon
//! resources (D14 §3, §4). The kernel reads them through:
//!
//! - [`registry::InstitutionIndex`] — derived index over the chain's
//!   `Institution`, `QueryClass`, `Comorphism`, `ExportFormat`, and
//!   `ImportFormat` declarations.
//! - [`runtime::InstitutionRuntime`] — registry of `Institution` trait
//!   implementations keyed by institution IRI.
//! - [`dispatch`] — D14 §9 dispatch helpers: AutoOnLoad QueryClass
//!   firing on commit and the post-translation validation invariant.
//!
//! See design document D14 for the canonical specification.

pub mod dispatch;
pub mod error;
pub mod eval_hooks;
pub mod in_process_registry;
pub mod marshal;
pub mod registry;
pub mod runtime;

/// Three-valued result of a `Decidable` QueryClass dispatch (D14 §9.2).
///
/// `Holds` and `Fails` reduce a `Constraint::Institution` predicate
/// at type-check time — the former produces `Refl`, the latter a
/// failing neutral. `Undecidable` leaves the constraint as a
/// passthrough neutral so later reduction may resolve it.
///
/// This is the kernel-internal three-valued tag. The user-facing
/// shape is the [`Verdict`](crate::ontology::well_known::VERDICT)
/// inductive type that institutions return as Resources; the
/// evaluator parses those into `DecResult` via `parse_verdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecResult {
    /// Predicate holds — the kernel reduces the surrounding
    /// `NativeDecide` to `Val::Refl(value)`.
    Holds,
    /// Predicate explicitly fails — the kernel emits a failing
    /// neutral; the type-checker rejects the constraint.
    Fails,
    /// Predicate cannot be determined at the call site — the
    /// `NativeDecide` stays as a passthrough neutral.
    Undecidable,
}
