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

/// Compile an ESL source string against a chain layer.
///
/// **The layer is required, and that is the point.** Compiling seeds the compiler's ctor and
/// macro tables from every chain-resident inductive, which is what lets the bare-name ctor
/// disambiguator emit an `Exp::InductiveCtor` rather than a plain reference — and, under D85
/// §6.1, what lets a value name its constructor's arguments, since those names live in the
/// inductive's `core:arg_types` and nowhere else.
///
/// There were two layerless entry points until `2026-09-01`, `compile` and
/// `compile_with_institutions`. They existed for callers that turned out to have a chain
/// anyway: `bootstrap`'s parentless branch was unreachable (the first ESL ontology sits at
/// index 4, so every ESL layer has a parent), the CLI's file loaders were called from commands
/// holding an `ExecutionContext`, and the server's no-branch arm had `"main"`, which is always
/// present. Keeping them would have forced the argument names into a second table inside the
/// D47 codec, to be kept in step with the ontology by hand — the same shape as the
/// `binder_name` / `core:binder_name` drift (eigenius#221) and the declared-but-undecodable
/// `SizeSort` (eigenius#218).
///
/// A source that genuinely references nothing chain-resident passes an empty root layer, and
/// that is visible at the call site rather than hidden behind an overload.
pub fn compile(
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
/// `justification:Certificate.app` ctors) need to resolve against the
/// chain. Use [`compile`] when there is no institution index.
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
