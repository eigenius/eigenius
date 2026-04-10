//! Public API trait for Eigenius kernel services.
//!
//! The EigeniusService trait (§2.2) defines the primary interface for ontological
//! operations: loading schemas, querying instances, validating data, and reflecting
//! on reasoning behavior. Implementations must handle layer composition and consistency.

use async_trait::async_trait;
use crate::ontology::{Class, Resource};
use crate::reflection::ReasoningTrace;

/// Primary service trait for Eigenius kernel operations.
///
/// Provides async methods for schema loading, instance queries, validation,
/// and reflective reasoning over ontological operations.
#[async_trait]
pub trait EigeniusService {
    /// Loads a Class definition by URI.
    ///
    /// Resolves the class from the composed layer stack.
    async fn load(&self, class_uri: &str) -> Result<Class, String>;

    /// Queries for Resource instances matching criteria.
    ///
    /// Executes a query across the ontology and returns matching resources.
    async fn query(&self, query: &str) -> Result<Vec<Resource>, String>;

    /// Validates a Resource against its Class schema.
    ///
    /// Returns validation errors if the resource violates constraints.
    async fn validate(&self, resource: &Resource) -> Result<(), String>;

    /// Reflects on reasoning behavior via trace capture.
    ///
    /// Returns a trace of the most recent reasoning execution.
    async fn reflect(&self) -> Result<ReasoningTrace, String>;
}
