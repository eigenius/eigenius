//! Reflective capabilities for introspection and meta-reasoning.
//!
//! Reflection system (§11) enables the kernel to reason about its own reasoning,
//! capturing trace information and universe level tracking for impredicative
//! reasoning within the type hierarchy.

use serde::{Serialize, Deserialize};

/// Universe level for impredicative type reasoning.
///
/// Tracks hierarchy level in the type universe to prevent impredicativity issues
/// and ensure proper stratification of type-level reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UniverseLevel(pub usize);

impl UniverseLevel {
    /// Creates a universe level at index 0 (ground level).
    pub fn ground() -> Self {
        UniverseLevel(0)
    }

    /// Increments to the next universe level.
    pub fn succ(&self) -> Self {
        UniverseLevel(self.0 + 1)
    }

    /// Gets the numeric level.
    pub fn level(&self) -> usize {
        self.0
    }
}

/// Trace record for a single reasoning step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    /// Step identifier
    pub step_id: usize,

    /// Description of the reasoning operation
    pub operation: String,

    /// Input to the operation
    pub input: Vec<u8>,

    /// Output from the operation
    pub output: Vec<u8>,
}

/// Complete trace of a reasoning execution.
///
/// Captures all reasoning steps for debugging, verification, and
/// meta-reasoning about the reasoning process itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningTrace {
    /// Unique trace identifier
    pub id: String,

    /// Universe level for this reasoning context
    pub universe_level: UniverseLevel,

    /// Ordered sequence of reasoning steps
    pub entries: Vec<TraceEntry>,
}

impl ReasoningTrace {
    /// Creates a new empty ReasoningTrace.
    pub fn new(id: String, universe_level: UniverseLevel) -> Self {
        Self {
            id,
            universe_level,
            entries: Vec::new(),
        }
    }

    /// Adds an entry to the trace.
    pub fn add_entry(&mut self, entry: TraceEntry) {
        self.entries.push(entry);
    }
}
