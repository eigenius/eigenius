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

//! EigenTT type theory and NbE evaluator.
//!
//! A Rust port of the EigenTT reference implementation (Coquand et al.),
//! extended with Eigon ontology ground types. Provides:
//! - Dependent function types (Pi), dependent pair types (Sigma), labeled sums
//! - Normalization by Evaluation (NbE) for type checking and partial evaluation
//! - Bidirectional type checking (check/infer)

pub mod check;
pub mod env;
pub mod eval;
pub mod positivity;
pub mod readback;
pub mod recursor;
pub mod sized;
pub mod sized_rigid;
pub mod subst;
pub mod term;
pub mod unify;
pub mod val;
