//! Eigenius Kernel
//!
//! The formally verified core of the Eigenius platform. Responsible for:
//! - Eigon structural type system (ontology validation)
//! - Layer management (immutable layers, stack resolution)
//! - Capability dispatch (class-anchored extensibility)
//! - Execution context management (snapshot isolation)
//! - Reflection layer (reasoning traces, universe stratification)
//! - NbE type checker (Mini-TT dependent type theory for programs)
//! - Bootstrap sequence (Core Ontology + Foundation Layer)

pub mod api;
pub mod bootstrap;
pub mod capability;
pub mod context;
pub mod esl;
pub mod layer;
pub mod nbe;
pub mod ontology;
pub mod program;
pub mod query;
pub mod reflection;
pub mod server;
pub mod storage;
pub mod validation;
