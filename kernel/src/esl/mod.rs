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
