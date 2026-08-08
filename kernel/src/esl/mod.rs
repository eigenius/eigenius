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
pub mod print;

use crate::ontology::resource::Resource;

/// Compile an ESL source string to Eigon-JSON resources.
pub fn compile(source: &str) -> Result<Vec<Resource>, Vec<error::EslError>> {
    let tokens = lexer::tokenize(source).map_err(|e| vec![e])?;
    let file = parser::parse(&tokens).map_err(|e| vec![e])?;
    compile::compile_file(&file)
}

/// Compile an ESL source string with access to an
/// [`InstitutionIndex`]. When provided, function-call IRIs in program
/// bodies that classify as Decidable QueryClasses or declared
/// Comorphisms are routed to the corresponding kernel capability via
/// specialized program resources.
///
/// [`InstitutionIndex`]: crate::institution::registry::InstitutionIndex
pub fn compile_with_institutions(
    source: &str,
    institutions: std::sync::Arc<crate::institution::registry::InstitutionIndex>,
) -> Result<Vec<Resource>, Vec<error::EslError>> {
    let tokens = lexer::tokenize(source).map_err(|e| vec![e])?;
    let file = parser::parse(&tokens).map_err(|e| vec![e])?;
    compile::compile_file_with_institutions(&file, Some(institutions))
}

/// Compile an ESL source string against a chain layer, seeding the
/// compiler's ctor table with every chain-resident inductive's
/// constructors. Required for D39 ReasoningSentence commits whose
/// `type_expr(...)` certificates reference chain-resident ctors like
/// `app` / `declared` / `observed` from `reasoning:JustifiedBy` —
/// the bare-name ctor disambiguator needs to see those entries to
/// emit the right `Exp::InductiveCtor` instead of a plain reference.
pub fn compile_against_layer(
    source: &str,
    layer: &crate::layer::Layer,
) -> Result<Vec<Resource>, Vec<error::EslError>> {
    let tokens = lexer::tokenize(source).map_err(|e| vec![e])?;
    let file = parser::parse(&tokens).map_err(|e| vec![e])?;
    let external_ctors = compile::collect_ctors_from_layer(layer);
    let external_macros = compile::collect_macros_from_layer(layer);
    compile::compile_file_with_context(&file, None, external_ctors, external_macros)
}

/// Compile an ESL source string with both an [`InstitutionIndex`]
/// AND a chain layer's external ctor + macro tables. This is the
/// shape the running server reaches for when handling `eigenius load`
/// or notebook-cell ESL — function-call IRIs need to classify against
/// the live institution index (D14 §9.5), AND cross-file references
/// to ctors / macros declared in parent layers (like
/// `stats:SingleSampleEstimate` smart constructors or
/// `reasoning:JustifiedBy.app` ctors) need to resolve against the
/// chain. Falls back to `compile_with_institutions` if no layer is
/// available; `compile_against_layer` if no institution index is.
///
/// [`InstitutionIndex`]: crate::institution::registry::InstitutionIndex
pub fn compile_full(
    source: &str,
    institutions: std::sync::Arc<crate::institution::registry::InstitutionIndex>,
    layer: &crate::layer::Layer,
) -> Result<Vec<Resource>, Vec<error::EslError>> {
    let tokens = lexer::tokenize(source).map_err(|e| vec![e])?;
    let file = parser::parse(&tokens).map_err(|e| vec![e])?;
    let external_ctors = compile::collect_ctors_from_layer(layer);
    let external_macros = compile::collect_macros_from_layer(layer);
    compile::compile_file_with_context(&file, Some(institutions), external_ctors, external_macros)
}
