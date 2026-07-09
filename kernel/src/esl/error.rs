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

//! Error types for the ESL compiler pipeline.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A position in ESL source.
///
/// `Serialize` + `Deserialize` are derived so the AST types that
/// carry positions (notably `MacroDecl` per D52 §12 #1 cross-file
/// macro storage) can round-trip through the chain via the resource-Value pipeline. Position info is preserved across the
/// round-trip for diagnostic locality at re-hydration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
