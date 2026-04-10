//! Normalization by Evaluation (NBE) type-checking and evaluation.
//!
//! Port of Mini-TT (Coquand et al.) for dependent type checking (§4.2).
//! Implements the evaluation and readback phases of NBE with proper handling
//! of dependent types and universe levels.

use crate::reflection::UniverseLevel;

/// A semantic value in the evaluation context.
///
/// Represents the result of evaluating a term in a particular environment,
/// used during normal form computation.
#[derive(Debug, Clone)]
pub enum Val {
    /// A neutral term that cannot be reduced further
    Neutral(Box<Neut>),

    /// A lambda abstraction with captured value
    Lambda {
        /// Parameter name
        param: String,
        /// Captured value (closure)
        body: Box<Val>,
    },

    /// A Pi type (dependent function type)
    Pi {
        /// Domain type
        domain: Box<Val>,
        /// Codomain (function to type)
        codomain: Box<Val>,
    },

    /// Universe of types at a given level
    Universe(UniverseLevel),
}

/// A neutral term that cannot be reduced further.
///
/// Neutral terms arise from variables and eliminations that cannot be
/// evaluated without additional information.
#[derive(Debug, Clone)]
pub enum Neut {
    /// A variable reference
    Var(String),

    /// Application of a neutral to an argument
    App(Box<Neut>, Box<Val>),
}

/// Evaluates a term to a semantic value.
///
/// Placeholder: full evaluation with environment handling.
pub fn eval(_term: &str, _env: &[Val]) -> Val {
    todo!("Implement evaluation with proper environment handling")
}

/// Reads back a semantic value to normal form syntax.
///
/// Placeholder: conversion of values back to term representation.
pub fn readback(_val: &Val) -> String {
    todo!("Implement readback to normal form")
}

/// Type-checks a term against a type.
///
/// Placeholder: dependent type checking with proper universe handling.
pub fn check(_term: &str, _ty: &Val, _universe: UniverseLevel) -> Result<(), String> {
    todo!("Implement type checking with universe polymorphism")
}
