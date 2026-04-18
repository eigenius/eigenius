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
}

impl fmt::Display for InstitutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstitutionError::UnknownType(msg) => write!(f, "unknown type: {msg}"),
            InstitutionError::ComputationFailed(msg) => write!(f, "computation failed: {msg}"),
            InstitutionError::MissingDependency(msg) => write!(f, "missing dependency: {msg}"),
        }
    }
}

impl std::error::Error for InstitutionError {}

/// Result of validating a morphism against an institution's domain logic.
#[derive(Debug, Clone)]
pub enum MorphismValidation {
    /// The morphism is valid according to the institution's domain logic.
    Valid,
    /// The morphism is invalid, with a reason.
    Invalid(String),
    /// The institution cannot determine validity.
    Undecidable,
}
