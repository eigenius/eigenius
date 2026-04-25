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

//! Structured error types for EigenQL.

use std::fmt;

/// A position in the source string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// The phase in which an error occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorPhase {
    Lexer,
    Parser,
    TypeCheck,
    Stratification,
    Evaluation,
}

/// A structured query error.
#[derive(Debug, Clone)]
pub struct QueryError {
    pub position: Option<Position>,
    pub phase: ErrorPhase,
    pub rule: String,
    pub message: String,
}

impl QueryError {
    pub fn lexer(pos: Position, message: impl Into<String>) -> Self {
        Self {
            position: Some(pos),
            phase: ErrorPhase::Lexer,
            rule: "lexer".to_string(),
            message: message.into(),
        }
    }

    pub fn parser(pos: Option<Position>, message: impl Into<String>) -> Self {
        Self {
            position: pos,
            phase: ErrorPhase::Parser,
            rule: "parser".to_string(),
            message: message.into(),
        }
    }

    pub fn type_check(rule: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            position: None,
            phase: ErrorPhase::TypeCheck,
            rule: rule.into(),
            message: message.into(),
        }
    }

    pub fn stratification(message: impl Into<String>) -> Self {
        Self {
            position: None,
            phase: ErrorPhase::Stratification,
            rule: "stratification".to_string(),
            message: message.into(),
        }
    }

    pub fn evaluation(message: impl Into<String>) -> Self {
        Self {
            position: None,
            phase: ErrorPhase::Evaluation,
            rule: "evaluation".to_string(),
            message: message.into(),
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(pos) = &self.position {
            write!(f, "{}:{}: ", pos.line, pos.column)?;
        }
        write!(f, "[{:?}] {}", self.phase, self.message)
    }
}

impl std::error::Error for QueryError {}
