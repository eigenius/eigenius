//! Mini-TT type theory and NbE evaluator.
//!
//! A Rust port of the Mini-TT reference implementation (Coquand et al.),
//! extended with Eigon ontology ground types. Provides:
//! - Dependent function types (Pi), dependent pair types (Sigma), labeled sums
//! - Normalization by Evaluation (NbE) for type checking and partial evaluation
//! - Bidirectional type checking (check/infer)

pub mod check;
pub mod env;
pub mod eval;
pub mod readback;
pub mod term;
pub mod val;
