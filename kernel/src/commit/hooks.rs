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

//! `didPersist` and `didDrain` hooks.
//!
//! Hooks run *after* a successful persist. They cannot abort the
//! commit; errors they raise are surfaced to the caller but the
//! commit stands. See D41 §3.6 and §6.5.
//!
//! Two hook flavours:
//!
//! - **`didPersist`** — runs per pipeline run, after `persist`
//!   advanced the branch. Receives `&mut CommitState` and can push
//!   follow-up emissions onto `state.emissions` for the orchestrator
//!   to drain.
//! - **`didDrain`** — runs once per orchestrator run, after the FIFO
//!   drain has emptied the queue. Receives `&mut DrainState`; cannot
//!   emit (the drain is over).
//!
//! Phase A: signatures + the two concrete hooks as
//! `unimplemented!("hook X")` stubs.
//!
//! Concrete hooks today:
//!
//! - [`register_wasm_components`] — `didPersist` on
//!   `with_institutions`. Registers WASM components from the
//!   just-persisted user layer and queues the
//!   `institution_classes` follow-up emission. Lifts the logic in
//!   `register_wasm_from_layer` in `server/mod.rs`.
//! - [`rebuild_institution_index`] — `didDrain` on the orchestrator.
//!   Replaces today's three intra-Load rebuild calls with one
//!   post-drain rebuild.

use crate::validation::ValidationError;

use super::state::{CommitState, DrainState};

/// Hook fn type for the post-persist stage of a single pipeline run.
///
/// The hook receives the same [`CommitState`] the phases used, so it
/// can read the just-persisted layer (via `state.layer` and
/// `state.persisted`) and push follow-up [`super::outcome::LayerEmission`]s
/// onto `state.emissions` for the orchestrator to drain.
pub type DidPersistHook = fn(&mut CommitState<'_>) -> HookOutcome;

/// Hook fn type for the post-drain stage of one orchestrator run.
///
/// The hook receives a [`DrainState`] carrying the final top layer
/// plus `&mut MultiLayerOutcome`. It cannot queue further work — the
/// drain is over — but it can mutate kernel state derived from the
/// full set of landed layers.
pub type DidDrainHook = fn(&mut DrainState<'_>) -> HookOutcome;

/// Non-unwinding outcome of a hook execution.
///
/// Hooks run after a successful persist; errors they raise are
/// surfaced to the caller but the commit stands (see D41 §3.6 for
/// why this is structurally correct: the layer is durable, the hook
/// side-effect is not).
#[derive(Debug, Default)]
pub struct HookOutcome {
    /// Errors collected during this hook invocation. The orchestrator
    /// appends them to `LayerCommitOutcome.hook_errors` (for
    /// `didPersist`) or `MultiLayerOutcome.drain_hook_errors` (for
    /// `didDrain`).
    pub errors: Vec<ValidationError>,
}

/// `didPersist` hook for the `with_institutions` pipeline.
///
/// Reads the just-persisted user layer (the WASM components are part
/// of its content), registers components against
/// `state.institutions.runtime`, and queues a
/// `LayerEmission { name: "institution_classes",
/// pipeline: StructuralFollowup, ... }` carrying the registered
/// classes for the institution-classes follow-up layer.
///
/// Lifts the logic currently in `register_wasm_from_layer` in
/// `server/mod.rs`. Errors registering components flow into
/// `state.hook_errors` — the user-layer commit stands either way.
///
/// D41 §3.6.
pub fn register_wasm_components(_state: &mut CommitState<'_>) -> HookOutcome {
    unimplemented!("hook register_wasm_components")
}

/// `didDrain` hook on the orchestrator.
///
/// Runs once after the FIFO drain completes, with the final top
/// layer in hand. Walks institution declarations reachable from
/// `top_layer` and rebuilds the dispatch index on the institution
/// runtime. Replaces today's three intra-Load rebuild calls in
/// `server/mod.rs`.
///
/// The collapse from three rebuilds to one is semantically
/// equivalent because nothing inside a single Load actually consumes
/// the rebuilt index; only the next RPC's `InstitutionContext`
/// snapshot reads it.
///
/// Errors land in `multi.drain_hook_errors`.
///
/// D41 §6.5.
pub fn rebuild_institution_index(_state: &mut DrainState<'_>) -> HookOutcome {
    unimplemented!("hook rebuild_institution_index")
}
