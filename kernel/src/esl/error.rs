//! Error types for the ESL compiler pipeline.

use std::fmt;

/// A position in ESL source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// Error phase in the ESL pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EslPhase {
    Lexer,
    Parser,
    Compiler,
}

/// A structured ESL error.
#[derive(Debug, Clone)]
pub struct EslError {
    pub position: Option<Position>,
    pub phase: EslPhase,
    pub message: String,
}

impl EslError {
    pub fn lexer(pos: Position, message: impl Into<String>) -> Self {
        Self {
            position: Some(pos),
            phase: EslPhase::Lexer,
            message: message.into(),
        }
    }

    pub fn parser(pos: Option<Position>, message: impl Into<String>) -> Self {
        Self {
            position: pos,
            phase: EslPhase::Parser,
            message: message.into(),
        }
    }

    pub fn compiler(pos: Option<Position>, message: impl Into<String>) -> Self {
        Self {
            position: pos,
            phase: EslPhase::Compiler,
            message: message.into(),
        }
    }
}

impl fmt::Display for EslError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(pos) = &self.position {
            write!(f, "{}:{}: ", pos.line, pos.column)?;
        }
        write!(f, "[{:?}] {}", self.phase, self.message)
    }
}

impl std::error::Error for EslError {}
