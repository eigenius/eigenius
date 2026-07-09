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
//! - [`CommitHookHost::trigger_vector_sweep_for_layer`] — `didPersist`
//!   on `with_institutions`. Runs the vector-index sweep against the
//!   just-persisted user layer.
//! - [`CommitHookHost::rebuild_institution_index`] — `didDrain` on the
//!   orchestrator. Replaces today's three intra-Load rebuild calls with
//!   one post-drain rebuild.

use std::sync::Arc;

use crate::layer::Layer;
use crate::validation::ValidationError;

use super::state::{CommitState, DrainState};

/// Host seam between the commit pipeline / orchestrator and the
/// kernel-side state that the two concrete hooks need to mutate.
///
/// Hooks are sync (`fn` pointers), but the methods they delegate to on
/// `EigeniusService` are async (`&self`, `await`-ing on tokio
/// `RwLock`s and the orchestrator client). The `CommitHookHost` trait
/// hides that: the impl in `server::mod` wraps the async call with
/// `tokio::task::block_in_place` + `Handle::current().block_on(...)`.
/// Hook bodies do not see the async-to-sync bridge.
///
/// **Error taxonomy.** The trait surface uses kernel-side
/// [`ValidationError`] rather than the proto type so the host doesn't
/// leak proto into the commit module. The server-side impl converts
/// from proto to kernel at the trait boundary; see Phase C's
/// `LayerPersister` impl for the same pattern.
///
/// No-op [`CommitHookHost`] for callers that don't need the
/// institution-index rebuild or vector sweep.
///
/// Used by [`crate::lattice::commit_layer`] / `commit_layer_default`
/// (CLI commits, bootstrap, GC tests, storage E2E tests). Both
/// methods return `Ok` with empty bodies — the hooks built on top
/// (`trigger_vector_sweep_for_layer`, `rebuild_institution_index`)
/// become no-ops because the pipelines those callers run don't include
/// any `didPersist` slot and the lattice path doesn't run an
/// orchestrator.
///
/// D41 Phase D.
pub struct NoopHost;

impl CommitHookHost for NoopHost {
    fn rebuild_institution_index(
        &self,
        _top_layer: &Arc<Layer>,
    ) -> Result<(), Vec<ValidationError>> {
        Ok(())
    }

    fn trigger_vector_sweep_for_layer(
        &self,
        _layer: &Arc<Layer>,
    ) -> Result<(), Vec<ValidationError>> {
        Ok(())
    }
}

/// D41 §3.6.
pub trait CommitHookHost: Send + Sync {
    /// Walk the chain from `top_layer` and rebuild the in-process
    /// institution dispatch index + runtime.
    ///
    /// Called once per orchestrator run after the FIFO drain completes,
    /// with the final top layer in hand. Best-effort: errors surface
    /// via `MultiLayerOutcome.drain_hook_errors` but do not unwind.
    ///
    /// D41 §6.5.
    fn rebuild_institution_index(&self, top_layer: &Arc<Layer>)
        -> Result<(), Vec<ValidationError>>;

    /// D43 §5.5 — fire a vector-index sweep against the just-
    /// persisted layer if any active VectorIndex Resource is visible
    /// at it. The host's impl looks up the
    /// [`crate::task::sweep_registry::SweepCoordinator`] (if any),
    /// calls `trigger_blocking` or `trigger_async` as the deployment
    /// shape dictates, and threads the resulting
    /// [`crate::task::sweep_registry::SweepHandle`] into its task
    /// registry for observability.
    ///
    /// Best-effort: on `Err`, the
    /// errors flow into `state.hook_errors` and the commit stands.
    /// A no-op default impl is provided so hosts that haven't been
    /// updated for vector retrieval still typecheck — `NoopHost`
    /// returns `Ok(())` regardless. The default impl also makes the
    /// trait method backward-compatible across the kernel test
    /// suite, which has dozens of bespoke `CommitHookHost` impls.
    fn trigger_vector_sweep_for_layer(
        &self,
        _layer: &Arc<Layer>,
    ) -> Result<(), Vec<ValidationError>> {
        Ok(())
    }
}

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

/// D43 §5.5 — `didPersist` hook that schedules the post-Load
/// vector-index sweep against the just-persisted layer.
///
/// Delegates to the host's
/// [`CommitHookHost::trigger_vector_sweep_for_layer`], which decides
/// whether to dispatch synchronously (tests / CLI commit modes) or
/// onto a tokio task (the gRPC service path). The hook is a no-op
/// when the host has no `SweepCoordinator` attached or no active
/// VectorIndex Resource is visible at the layer — neither is an
/// error.
///
/// Errors flow into `state.hook_errors` and the commit stands.
pub fn trigger_vector_sweep(state: &mut CommitState<'_>) -> HookOutcome {
    let layer = state
        .layer
        .as_ref()
        .expect("trigger_vector_sweep runs after persist; layer must be Some")
        .clone();
    if let Err(errors) = state.host.trigger_vector_sweep_for_layer(&layer) {
        state.hook_errors.extend(errors);
    }
    HookOutcome::default()
}

/// `didDrain` hook on the orchestrator.
///
/// Runs once after the FIFO drain completes, with the final top
/// layer in hand. Delegates to the host's
/// [`CommitHookHost::rebuild_institution_index`], which walks
/// institution declarations reachable from `top_layer` and rebuilds
/// the dispatch index + institution runtime. Replaces today's three
/// intra-Load rebuild calls in `server/mod.rs`.
///
/// The collapse from three rebuilds to one is semantically
/// equivalent because nothing inside a single Load actually consumes
/// the rebuilt index; only the next RPC's `InstitutionContext`
/// snapshot reads it.
///
/// Errors land in `multi.drain_hook_errors`.
///
/// If no layer landed in the drain (e.g. immediate Err on the first
/// emission with no Sibling rescue), the hook skips the rebuild —
/// the institution index is still correct because no new layer was
/// incorporated.
///
/// D41 §6.5.
pub fn rebuild_institution_index(drain_state: &mut DrainState<'_>) -> HookOutcome {
    let Some(top_layer) = drain_state.top_layer.as_ref() else {
        // Empty drain: no layer landed. Index is unchanged; nothing
        // to rebuild.
        return HookOutcome::default();
    };
    match drain_state.host.rebuild_institution_index(top_layer) {
        Ok(()) => HookOutcome::default(),
        Err(errors) => HookOutcome { errors },
    }
}
