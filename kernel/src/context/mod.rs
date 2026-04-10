//! Execution context providing isolated reasoning environments.
//!
//! ExecutionContext (§8) captures the state for a single reasoning session,
//! including the active layer stack, mode (read-only or read-write), and
//! snapshot tracking for consistency and rollback semantics.

use crate::layer::LayerStack;
use serde::{Serialize, Deserialize};

/// Execution mode determining allowed operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Read-only queries and reasoning
    ReadOnly,

    /// Read and write operations permitted
    ReadWrite,
}

/// Execution context for a single reasoning session.
///
/// Captures layer composition, snapshot identity, and operation mode.
/// Provides isolation and consistency guarantees for reasoning tasks.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Snapshot identifier for consistency
    pub snapshot_id: String,

    /// Stack of active layers
    pub layer_stack: LayerStack,

    /// Current execution mode
    pub mode: ExecutionMode,
}

impl ExecutionContext {
    /// Creates a new ExecutionContext.
    pub fn new(snapshot_id: String, mode: ExecutionMode) -> Self {
        Self {
            snapshot_id,
            layer_stack: LayerStack::new(),
            mode,
        }
    }

    /// Checks if this context permits write operations.
    pub fn is_writable(&self) -> bool {
        self.mode == ExecutionMode::ReadWrite
    }

    /// Checks if this context is read-only.
    pub fn is_readonly(&self) -> bool {
        self.mode == ExecutionMode::ReadOnly
    }
}
