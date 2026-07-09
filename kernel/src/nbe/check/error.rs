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

//! Typed error for the bidirectional type checker.
//!
//! Mirrors [`crate::nbe::eval::EvalError`] on the evaluation side: the
//! checker previously threaded `Result<_, String>` throughout `check/`,
//! which erased the failure category and forced every eval call to
//! `.map_err(|e| e.to_string())`. `CheckError` restores the category
//! (so a consumer can distinguish "type mismatch" from "cannot infer")
//! while keeping the human-readable detail in the payload string, and
//! its `From<EvalError>` lets `?` propagate evaluation failures without
//! a manual conversion at every call site.

use crate::nbe::eval::EvalError;

/// A type-checking failure, categorized by kind.
///
/// The detail string carries the site-specific diagnostic (the term,
/// the mismatched types, the constructor name, …). Consumers that only
/// want the message call [`ToString::to_string`]; the boundary in
/// `program`/`validation`/`query` does exactly that.
#[derive(Debug, Clone)]
pub enum CheckError {
    /// Two types or values are not definitionally equal — the core
    /// conversion failure. Covers `eq_nf`, `def_eq_at_type`,
    /// `subtype_of` (including universe and size subtyping), class
    /// non-inhabitation, and constructor/inductive head mismatch.
    TypeMismatch(String),
    /// A term's type could not be synthesized in inference mode
    /// (`check_infer` fell through — e.g. a bare `Match`, or a
    /// constructor whose inductive can't be recovered).
    CannotInfer(String),
    /// Inference expected a Pi (function) type but the head was
    /// something else (application of a non-function).
    ExpectedPi(String),
    /// Inference expected a Sigma (dependent pair) type
    /// (projection of a non-pair).
    ExpectedSigma(String),
    /// A position required a `Sort` (a type): an `Ann` annotation or a
    /// Pi binder's domain/codomain that did not normalise to a sort.
    ExpectedSort(String),
    /// A recursor or `match` scrutinee required an inductive type.
    ExpectedInductive(String),
    /// An observation required a codata value.
    ExpectedCodata(String),
    /// A declaration or term is structurally ill-formed: constructor
    /// telescopes and conclusions, index/parameter arities, positivity,
    /// recursor motive and minor shape, `match`-arm coverage,
    /// corecursive guardedness, sized-binder constraints, or duplicate
    /// names.
    IllFormed(String),
    /// Evaluation (normalisation, application, observation, or ground
    /// resolution) failed while checking. Wraps the underlying
    /// [`EvalError`]; produced automatically by `?` on any eval call.
    Eval(EvalError),
}

impl From<EvalError> for CheckError {
    fn from(e: EvalError) -> Self {
        CheckError::Eval(e)
    }
}

impl From<String> for CheckError {
    /// Bridge for the pure-TT core helpers (`env::up_gamma`,
    /// `positivity::check_positivity`, `env::lookup_gamma`, …) that
    /// return `Result<_, String>`. Those modules are the Eigenius-free
    /// core and must not depend on this check-module type, so their
    /// `String` errors — all structural well-formedness failures —
    /// surface here as [`CheckError::IllFormed`]. Lets `?` propagate
    /// them without a per-site `.map_err`.
    fn from(s: String) -> Self {
        CheckError::IllFormed(s)
    }
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TypeMismatch(s) => write!(f, "{s}"),
            Self::CannotInfer(s) => write!(f, "{s}"),
            Self::ExpectedPi(s) => write!(f, "{s}"),
            Self::ExpectedSigma(s) => write!(f, "{s}"),
            Self::ExpectedSort(s) => write!(f, "{s}"),
            Self::ExpectedInductive(s) => write!(f, "{s}"),
            Self::ExpectedCodata(s) => write!(f, "{s}"),
            Self::IllFormed(s) => write!(f, "{s}"),
            Self::Eval(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CheckError {}
