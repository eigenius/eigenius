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
}

impl fmt::Display for InstitutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstitutionError::UnknownType(msg) => write!(f, "unknown type: {msg}"),
            InstitutionError::ComputationFailed(msg) => write!(f, "computation failed: {msg}"),
            InstitutionError::MissingDependency(msg) => write!(f, "missing dependency: {msg}"),
            InstitutionError::NotImplemented(msg) => write!(f, "not implemented: {msg}"),
        }
    }
}

impl std::error::Error for InstitutionError {}
