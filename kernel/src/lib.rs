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

//! Eigenius Kernel
//!
//! The formally verified core of the Eigenius platform. Responsible for:
//! - Eigon structural type system (ontology validation)
//! - Layer management (immutable layers, stack resolution)
//! - Capability dispatch (class-anchored extensibility)
//! - Execution context management (snapshot isolation)
//! - Reflection layer (reasoning traces, universe stratification)
//! - NbE type checker (EigenTT dependent type theory for programs)
//! - Bootstrap sequence (Core Ontology + Foundation Layer)

pub mod bootstrap;
pub mod capability;
pub mod commit;
pub mod context;
pub mod dcg;
pub mod esl;
pub mod gc;
pub mod institution;
pub mod lattice;
pub mod layer;
pub mod nbe;
pub mod observability;
pub mod ontology;
pub mod program;
pub mod query;
pub mod runtime;
pub mod server;
pub mod storage;
pub mod task;
pub mod validation;
pub mod witness;
