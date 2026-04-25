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

//! ESL — Eigenius Surface Language.
//!
//! A human-friendly surface syntax that compiles to Eigon-JSON.
//! Two-layer design: HCL-style structural declarations (class, property,
//! resource) and ML-style expressions inside program bodies.
//!
//! See design doc D7 for the full specification.

pub mod ast;
pub mod compile;
pub mod error;
pub mod lexer;
pub mod parser;

use crate::ontology::resource::Resource;

/// Compile an ESL source string to Eigon-JSON resources.
pub fn compile(source: &str) -> Result<Vec<Resource>, Vec<error::EslError>> {
    let tokens = lexer::tokenize(source).map_err(|e| vec![e])?;
    let file = parser::parse(&tokens).map_err(|e| vec![e])?;
    compile::compile_file(&file)
}

/// Compile an ESL source string with access to an institution
/// registry (Phase 11e). When provided, function-call IRIs in
/// program bodies that classify as registered comorphisms or decide
/// predicates are routed to the corresponding kernel capability via
/// specialized program resources.
pub fn compile_with_institutions(
    source: &str,
    institutions: std::sync::Arc<crate::institution::InstitutionRegistry>,
) -> Result<Vec<Resource>, Vec<error::EslError>> {
    let tokens = lexer::tokenize(source).map_err(|e| vec![e])?;
    let file = parser::parse(&tokens).map_err(|e| vec![e])?;
    compile::compile_file_with_institutions(&file, Some(institutions))
}
