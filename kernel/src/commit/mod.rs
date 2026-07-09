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

//! Commit pipeline — one home for the "land a layer on a branch"
//! work. Pre-D41 this work was smeared across
//! `context::commit_with_validation`, `lattice::commit_layer`, the
//! Load handler in `server::mod`, and `server::persist_layer_if_backend`;
//! Phases A–G of D41 consolidated all of it here.
//!
//! The module splits into two layers:
//!
//! - [`pipeline::CommitPipeline`] — single layer. Walks a
//!   `&'static [Phase]` slice over a fresh
//!   [`state::CommitState`] arena, optionally runs
//!   `&'static [DidPersistHook]` after a successful persist, and
//!   returns a [`outcome::LayerCommitOutcome`]. Four canned shapes;
//!   see [`pipeline::CommitPipeline::structural_only`] and friends.
//! - [`orchestrator::CommitOrchestrator`] — multi-layer FIFO drain
//!   over emissions, post-drain `didDrain` hook stage, returns a
//!   [`outcome::MultiLayerOutcome`].
//!
//! Phase A: scaffolding only. Every type / trait / function compiles
//! cleanly; phase / hook / `run` bodies are `unimplemented!()`. No
//! existing call sites have been migrated yet. See D41 for the full
//! design; this module's structure follows D41 §11.1.
//!
//! Re-exports:
//!
//! - [`crate::lattice::CommitPolicy`] is **canonical in `lattice`**
//!   and re-exported here so commit call sites can write
//!   `crate::commit::CommitPolicy` without a second name in scope.
//! - [`crate::lattice::CommitError`] is likewise re-exported. Phase
//!   B / E will determine whether the enum needs splitting between
//!   commit and lattice; for Phase A the existing enum is reused.
//! - [`crate::validation::CommitWorkingSet`] and
//!   [`crate::validation::CommitWorkingSetPool`] are re-exported so
//!   commit consumers don't need to know they live in `validation`.

pub mod backend_persister;
pub mod hooks;
pub mod orchestrator;
pub mod outcome;
pub mod persister;
pub mod phases;
pub mod pipeline;
pub mod state;

// --- Re-exports of canonical types living elsewhere ---

pub use crate::lattice::{CommitError, CommitPolicy};
pub use crate::validation::{CommitWorkingSet, CommitWorkingSetPool};

// --- Re-exports of the commit module's own surface ---

pub use backend_persister::BackendPersister;
pub use hooks::{
    rebuild_institution_index, CommitHookHost, DidDrainHook, DidPersistHook, HookOutcome, NoopHost,
};
pub use orchestrator::{CommitOrchestrator, MAX_EMISSION_DEPTH};
pub use outcome::{
    DispatchEntry, EmissionKind, LayerCommitOutcome, LayerEmission, LayerRole, MultiLayerOutcome,
};
pub use persister::{BackendStorePersister, LayerPersister, PersistedLayerInfo};
pub use phases::{
    autoonload_dispatch, build, persist, retroactive_with_cascade, structural_validate,
};
pub use pipeline::{
    CommitPipeline, Phase, PhaseControl, PipelineConfig, PipelineKind, PipelineRunErr,
};
pub use state::{CommitState, DrainState, InstitutionContext};
