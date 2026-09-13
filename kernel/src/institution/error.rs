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

//! Error types for the institution protocol.

use std::fmt;

/// Errors from institution operations.
#[derive(Debug, Clone)]
pub enum InstitutionError {
    /// The query/morphism type is not recognized by this institution.
    UnknownType(String),
    /// Internal computation error.
    ComputationFailed(String),
    /// The institution requires resources not available in the context.
    MissingDependency(String),
    /// The institution declares the relevant procedure (export, import,
    /// or query handler) only by reference; it has no runtime
    /// implementation for it. Used by the default `Institution::query`
    /// impl for institutions whose QueryClasses are all
    /// Component-implemented (D14 §6.2 / §8).
    NotImplemented(String),
    /// The input is not one this institution can adjudicate — a reference that
    /// resolves to the wrong kind of thing, a parameter naming a form the institution
    /// does not implement, a value nobody supplied that has no safe default.
    ///
    /// Distinct from a negative VERDICT, and the distinction is the point. A verdict
    /// is what the institution decided after running; this is the institution
    /// declining to run at all. Collapsing the two publishes a verdict a reader cannot
    /// tell apart from a real one — which matters most for an institution whose
    /// `Undecidable` is itself a meaningful epistemic state rather than a shrug.
    InvalidInput(String),
}

impl fmt::Display for InstitutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstitutionError::UnknownType(msg) => write!(f, "unknown type: {msg}"),
            InstitutionError::ComputationFailed(msg) => write!(f, "computation failed: {msg}"),
            InstitutionError::MissingDependency(msg) => write!(f, "missing dependency: {msg}"),
            InstitutionError::NotImplemented(msg) => write!(f, "not implemented: {msg}"),
            InstitutionError::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
        }
    }
}

impl std::error::Error for InstitutionError {}
