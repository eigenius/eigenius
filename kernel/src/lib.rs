//! Eigenius Kernel
//!
//! The formally verified core of the Eigenius platform. Responsible for:
//! - Eigon structural type system (ontology validation)
//! - Layer management (immutable layers, stack resolution)
//! - Capability dispatch (class-anchored extensibility)
//! - Execution context management (snapshot isolation)
//! - Reflection layer (reasoning traces, universe stratification)
//! - NbE type checker (Mini-TT dependent type theory for DAGs)
//! - Bootstrap sequence (Core Ontology + Foundation Layer)

pub mod ontology;
pub mod layer;
pub mod context;
pub mod capability;
pub mod storage;
pub mod reflection;
pub mod nbe;
pub mod bootstrap;
pub mod api;
