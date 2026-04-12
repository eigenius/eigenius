//! EigenQL query language: lexer, parser, stratification, type checker, and evaluator.
//!
//! Implements the EigenQL specification from design doc D2.

pub mod ast;
pub mod error;
pub mod evaluate;
pub mod functions;
pub mod lexer;
pub mod parser;
pub mod stratify;
pub mod type_check;

use crate::layer::Layer;
use crate::ontology::resource::Resource;
use error::QueryError;

/// Result of executing an EigenQL query.
pub struct QueryResult {
    /// The result resources.
    pub resources: Vec<Resource>,
}

/// Execute an EigenQL program against a layer chain.
///
/// Pipeline: lex → parse → stratify → type_check → evaluate.
pub fn execute(program_str: &str, layer: &Layer) -> Result<QueryResult, Vec<QueryError>> {
    // 1. Lex
    let tokens = lexer::tokenize(program_str).map_err(|e| vec![e])?;

    // 2. Parse
    let program = parser::parse(tokens).map_err(|e| vec![e])?;

    // 3. Stratification check
    stratify::stratify(&program.definitions).map_err(|e| vec![e])?;

    // 4. Type check
    let type_errors = type_check::type_check(&program, layer);
    if !type_errors.is_empty() {
        return Err(type_errors);
    }

    // 5. Evaluate
    let resources = evaluate::evaluate(&program, layer).map_err(|e| vec![e])?;

    Ok(QueryResult { resources })
}
