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

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::layer::Layer;
use crate::ontology::Resource;
use crate::validation::ValidationError;

use super::outcome::{LayerEmission, LayerRole};
use super::pipeline::PipelineKind;
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
/// No-op [`CommitHookHost`] for callers that don't need WASM
/// registration or institution-index rebuilds.
///
/// Used by [`crate::lattice::commit_layer`] / `commit_layer_default`
/// (CLI commits, bootstrap, GC tests, storage E2E tests). Both
/// methods return `Ok` with empty bodies — the hooks built on top
/// (`register_wasm_components`, `rebuild_institution_index`) become
/// no-ops because the pipelines those callers run don't include any
/// `didPersist` slot and the lattice path doesn't run an orchestrator.
///
/// D41 Phase D.
pub struct NoopHost;

impl CommitHookHost for NoopHost {
    fn register_wasm_for_layer(
        &self,
        _layer: &Arc<Layer>,
    ) -> Result<Vec<Resource>, Vec<ValidationError>> {
        Ok(Vec::new())
    }

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
    /// Inspect the just-persisted layer for WASM components and external
    /// institutions; register them in the kernel's WASM runtime /
    /// institution registry.
    ///
    /// Returns institution-class resources to commit as a follow-up
    /// layer (the `register_wasm_components` `didPersist` hook queues
    /// them as a `Child` emission). On `Err`, the errors flow into
    /// `state.hook_errors` and the commit stands — registration is a
    /// side-effect on kernel runtime state, not a commit gate.
    ///
    /// D41 §3.6.
    fn register_wasm_for_layer(
        &self,
        layer: &Arc<Layer>,
    ) -> Result<Vec<Resource>, Vec<ValidationError>>;

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
    /// Best-effort like the WASM-registration hook: on `Err`, the
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

/// `didPersist` hook for the `with_institutions` pipeline.
///
/// Reads the just-persisted user layer (the WASM components are part
/// of its content), delegates registration to the host via
/// [`CommitHookHost::register_wasm_for_layer`], and queues a
/// `LayerEmission { name: "institution_classes",
/// pipeline: StructuralFollowup, kind: Child, ... }` carrying the
/// returned resources whenever the host produced any. Errors from the
/// host are routed into `state.hook_errors` — the user-layer commit
/// stands either way (the layer is already on disk).
///
/// Lifts the logic currently in `register_wasm_from_layer` in
/// `server/mod.rs`.
///
/// D41 §3.6.
pub fn register_wasm_components(state: &mut CommitState<'_>) -> HookOutcome {
    let layer = state
        .layer
        .as_ref()
        .expect("register_wasm_components runs after persist; layer must be Some")
        .clone();
    match state.host.register_wasm_for_layer(&layer) {
        Ok(resources) => {
            if !resources.is_empty() {
                // D41 §3.6: Child emission — the institution_classes
                // follow-up only makes sense if the parent layer
                // (just persisted) landed, which it did. On `Err`
                // from the queuing pipeline this Child would be
                // dropped; on `Ok` (this path) it drains as expected.
                state.emissions.push(LayerEmission {
                    role: LayerRole::InstitutionClasses,
                    name: "institution_classes",
                    pipeline: PipelineKind::StructuralFollowup,
                    kind: super::outcome::EmissionKind::Child,
                    resources,
                    tombstones: BTreeSet::new(),
                });
            }
            HookOutcome::default()
        }
        Err(errors) => {
            // D41 §3.6: routed host errors are state-level (commit
            // stands; layer is durable). They flow into
            // state.hook_errors and onto LayerCommitOutcome.
            state.hook_errors.extend(errors);
            HookOutcome::default()
        }
    }
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
/// Like [`register_wasm_components`], errors flow into
/// `state.hook_errors` and the commit stands.
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
/// the dispatch index + WASM runtime. Replaces today's three
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

#[cfg(test)]
mod tests {
    //! D41 Phase F.5 — `register_wasm_components` hook coverage.
    //!
    //! The orchestrator tests in `orchestrator.rs` exercise the
    //! end-to-end flow but only with a `StubHost` returning empty
    //! resources (so no `institution_classes` follow-up emission ever
    //! lands). These tests target the hook directly with a stubbed
    //! host that returns non-empty resources and a host that returns
    //! errors — confirming the queued emission shape and the
    //! state.hook_errors routing.

    use super::*;
    use crate::commit::outcome::{DispatchEntry, EmissionKind, LayerEmission, LayerRole};
    use crate::commit::persister::{LayerPersister, PersistedLayerInfo};
    use crate::commit::state::CommitState;
    use crate::lattice::CommitPolicy;
    use crate::layer::{LayerBuilder, LayerStorage};
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known;
    use crate::ontology::Iri;
    use crate::validation::{CommitWorkingSet, ValidationError, ValidationRule};
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    /// Configurable stub for [`CommitHookHost`].
    struct ConfigurableHost {
        register_result: Mutex<Result<Vec<Resource>, Vec<ValidationError>>>,
    }

    impl ConfigurableHost {
        fn ok(resources: Vec<Resource>) -> Self {
            Self {
                register_result: Mutex::new(Ok(resources)),
            }
        }
        fn err(errors: Vec<ValidationError>) -> Self {
            Self {
                register_result: Mutex::new(Err(errors)),
            }
        }
    }

    impl CommitHookHost for ConfigurableHost {
        fn register_wasm_for_layer(
            &self,
            _layer: &Arc<Layer>,
        ) -> Result<Vec<Resource>, Vec<ValidationError>> {
            self.register_result
                .lock()
                .unwrap()
                .as_ref()
                .map(|v| v.clone())
                .map_err(|errs| errs.clone())
        }
        fn rebuild_institution_index(
            &self,
            _top_layer: &Arc<Layer>,
        ) -> Result<(), Vec<ValidationError>> {
            Ok(())
        }
    }

    /// Persister stub — unused by `register_wasm_components` but
    /// required to satisfy the `CommitState` trait borrow.
    struct UnusedPersister;
    impl LayerPersister for UnusedPersister {
        fn persist(
            &self,
            _branch: &str,
            _layer: &Arc<Layer>,
        ) -> Result<PersistedLayerInfo, ValidationError> {
            unreachable!("register_wasm_components does not call persist");
        }
    }

    /// Build a minimal layer for the hook to consume. Resource is
    /// trivial; the hook only reads `state.layer` to thread the layer
    /// into `host.register_wasm_for_layer`.
    fn build_test_layer(storage: LayerStorage) -> Arc<Layer> {
        let builder = LayerBuilder::new("test_layer", None);
        Arc::new(builder.build(storage))
    }

    /// Construct a `CommitState` ready for the hook to consume.
    /// `host` is borrowed for the state's lifetime.
    fn make_state<'a>(
        host: &'a dyn CommitHookHost,
        persister: &'a UnusedPersister,
        storage: LayerStorage,
        layer: Arc<Layer>,
        working_set: &'a mut CommitWorkingSet,
    ) -> CommitState<'a> {
        CommitState {
            storage,
            persist: persister,
            host,
            policy: CommitPolicy::default(),
            branch: "main",
            institutions: None,
            builder: LayerBuilder::new("ignored", None),
            layer: Some(layer),
            cascade_tombstones: BTreeSet::new(),
            cascade_iterations: 0,
            dispatched_verdicts: Vec::<DispatchEntry>::new(),
            provenance_resources: Vec::new(),
            emissions: Vec::new(),
            hook_errors: Vec::new(),
            working_set,
            persisted: None,
        }
    }

    fn make_dummy_resource(local: &str) -> Resource {
        let mut r = Resource::new(Iri::parse(&format!("urn:eigenius:user:{local}")).unwrap());
        r.set(
            Iri::parse(well_known::IS_A).unwrap(),
            Value::Array(vec![Value::String(well_known::CLASS.into())]),
        );
        r
    }

    /// Hook queues an `institution_classes` Child emission with the
    /// host-provided resources whenever the host returns non-empty.
    #[test]
    fn register_wasm_components_queues_institution_classes_child_emission() {
        let class_resources = vec![make_dummy_resource("Inst1"), make_dummy_resource("Inst2")];
        let host = ConfigurableHost::ok(class_resources.clone());
        let persister = UnusedPersister;
        let storage = LayerStorage::in_memory();
        let layer = build_test_layer(storage.clone());
        let mut ws = CommitWorkingSet::in_memory();
        let mut state = make_state(&host, &persister, storage, layer, &mut ws);

        let outcome = register_wasm_components(&mut state);

        // Hook outcome: no errors.
        assert!(outcome.errors.is_empty());
        // Emission queued: exactly one entry, with the documented shape.
        assert_eq!(state.emissions.len(), 1);
        let em: &LayerEmission = &state.emissions[0];
        assert_eq!(em.role, LayerRole::InstitutionClasses);
        assert_eq!(em.name, "institution_classes");
        assert_eq!(em.pipeline, PipelineKind::StructuralFollowup);
        assert_eq!(em.kind, EmissionKind::Child);
        // Resources match what the host produced.
        assert_eq!(em.resources.len(), class_resources.len());
        for (a, b) in em.resources.iter().zip(class_resources.iter()) {
            assert_eq!(a.id(), b.id());
        }
        // No tombstones (institution-classes follow-ups don't suppress).
        assert!(em.tombstones.is_empty());
        // No hook_errors on state either.
        assert!(state.hook_errors.is_empty());
    }

    /// Hook does NOT queue an emission when the host returns an empty
    /// vector (no WASM components in the layer).
    #[test]
    fn register_wasm_components_no_emission_when_host_returns_empty() {
        let host = ConfigurableHost::ok(Vec::new());
        let persister = UnusedPersister;
        let storage = LayerStorage::in_memory();
        let layer = build_test_layer(storage.clone());
        let mut ws = CommitWorkingSet::in_memory();
        let mut state = make_state(&host, &persister, storage, layer, &mut ws);

        let outcome = register_wasm_components(&mut state);
        assert!(outcome.errors.is_empty());
        assert!(
            state.emissions.is_empty(),
            "no emission expected when host returns empty resources"
        );
        assert!(state.hook_errors.is_empty());
    }

    /// Host error path: errors flow into `state.hook_errors` and the
    /// hook itself returns a default `HookOutcome` (its `errors`
    /// vector stays empty — the hook's own surface reports the
    /// invocation as clean). No emission is queued.
    ///
    /// D41 §3.6: "routed host errors are state-level (commit stands;
    /// layer is durable)."
    #[test]
    fn register_wasm_components_routes_host_errors_to_state() {
        let host_errors = vec![
            ValidationError {
                resource_id: None,
                property: None,
                rule: ValidationRule::InstitutionValidation,
                message: "synthetic register failure".to_string(),
            },
            ValidationError {
                resource_id: None,
                property: None,
                rule: ValidationRule::InstitutionValidation,
                message: "second failure".to_string(),
            },
        ];
        let host = ConfigurableHost::err(host_errors.clone());
        let persister = UnusedPersister;
        let storage = LayerStorage::in_memory();
        let layer = build_test_layer(storage.clone());
        let mut ws = CommitWorkingSet::in_memory();
        let mut state = make_state(&host, &persister, storage, layer, &mut ws);

        let outcome = register_wasm_components(&mut state);

        // The hook's own outcome does not surface the errors (they
        // flow through state, not through the return value, because
        // routed host errors are "state-level" per D41 §3.6).
        assert!(outcome.errors.is_empty());
        // No emission queued — host failed to produce class resources.
        assert!(state.emissions.is_empty());
        // Errors landed on state.hook_errors so the orchestrator can
        // surface them via LayerCommitOutcome.hook_errors.
        assert_eq!(state.hook_errors.len(), 2);
        assert_eq!(state.hook_errors[0].message, "synthetic register failure");
        assert_eq!(state.hook_errors[1].message, "second failure");
    }
}
