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

//! `CommitOrchestrator` — multi-layer FIFO drain loop and
//! post-drain `didDrain` hook stage.
//!
//! Every commit-shaped RPC goes `handler → orchestrator → pipeline`,
//! including the single-layer ones. A `Query INTO` with no
//! emissions runs through the orchestrator as the degenerate case
//! (one pipeline run, empty emission queue, returns immediately).
//! This keeps the handler shape uniform: build a root
//! [`super::outcome::LayerEmission`] from the RPC inputs, call
//! `orchestrator.run(root)`, translate the
//! [`super::outcome::MultiLayerOutcome`] back into RPC response
//! fields.
//!
//! The orchestrator owns:
//! - the FIFO `(depth, LayerEmission)` queue,
//! - the `MAX_EMISSION_DEPTH = 4` cap (D41 §6.3),
//! - the revert-to-`last_advanced` head bookkeeping (D41 §6.4),
//! - the `didDrain` hook stage (D41 §6.5).
//!
//! Phase A: `run` is `unimplemented!("phase A scaffolding; see d41
//! §6")`.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::context::ExecutionContext;
use crate::lattice::{CommitError, CommitPolicy};
use crate::observability::operation;
use crate::validation::CommitWorkingSetPool;

use super::hooks::{rebuild_institution_index, CommitHookHost, DidDrainHook};
use super::outcome::{LayerCommitOutcome, LayerEmission, MultiLayerOutcome};
use super::persister::LayerPersister;
use super::pipeline::{CommitPipeline, PipelineConfig, PipelineRunErr};
use super::state::{DrainState, InstitutionContext};

/// Static safety net: a phase or hook that produced emissions
/// transitively past this depth aborts the orchestrator with
/// [`CommitError`]. Today the maximum depth is 1 (Load emits
/// `verdict_provenance` and `institution_classes`); 4 leaves room
/// for two follow-up generations beyond that.
///
/// D41 §6.3.
pub const MAX_EMISSION_DEPTH: u32 = 4;

/// Multi-layer commit orchestrator.
///
/// Borrows the execution context, working-set pool, persister, and
/// (optionally) institution context for the duration of one
/// `run(root)` invocation. The orchestrator constructs one
/// `CommitPipeline` per drained emission (looked up via
/// `CommitPipeline::for_kind`) and one `CommitState` per pipeline
/// run; the working set is re-used across pipeline runs to amortise
/// allocation.
///
/// D41 §6.
pub struct CommitOrchestrator<'a> {
    /// Execution context being driven. The orchestrator advances and
    /// reverts `ctx.head` as pipeline runs land or fail to land.
    pub ctx: &'a mut ExecutionContext,
    /// Per-server pool. The orchestrator acquires one
    /// [`crate::validation::CommitWorkingSet`] for the entire drain.
    pub pool: &'a CommitWorkingSetPool,
    /// Persist seam threaded into every `CommitState`.
    pub persister: &'a dyn LayerPersister,
    /// Host seam threaded into every `CommitState` and the
    /// post-drain `DrainState`. D41 Phase D.
    pub host: &'a dyn CommitHookHost,
    /// Branch name for this orchestrator run.
    pub branch: &'a str,
    /// Global commit policy.
    pub policy: CommitPolicy,
    /// Borrowed institution context for `with_institutions` pipelines.
    pub institutions: Option<InstitutionContext<'a>>,
    /// `didDrain` hooks. The canonical orchestrator includes
    /// [`rebuild_institution_index`] — see [`Self::default_did_drain`].
    pub did_drain: &'static [DidDrainHook],
}

impl<'a> CommitOrchestrator<'a> {
    /// Default `didDrain` hook list: a single
    /// [`rebuild_institution_index`] hook. Phase C will wire this
    /// from `EigeniusService::commit_orchestrator`; for Phase A the
    /// slice is exposed as an associated function so callers don't
    /// hand-roll their own.
    ///
    /// D41 §6.5.
    pub const fn default_did_drain() -> &'static [DidDrainHook] {
        DEFAULT_DID_DRAIN
    }

    /// Run the FIFO drain starting from `root`.
    ///
    /// See the pseudocode in D41 §6.1. The body opens a
    /// `COMMIT_ORCHESTRATOR_RUN` span, drains emissions in order,
    /// runs `did_drain` hooks under a `COMMIT_DID_DRAIN` span after
    /// the queue is empty, and returns the accumulated
    /// [`MultiLayerOutcome`].
    ///
    /// **Return shape (D41 Phase E).** The orchestrator always returns
    /// a [`MultiLayerOutcome`]; the optional `error` field carries the
    /// first pipeline `Err` if one occurred. This makes the
    /// rejected-but-audited path representable: when
    /// `autoonload_dispatch` returns `Err` on a `Fails` verdict and
    /// the orchestrator rescues + lands the `verdict_provenance`
    /// Sibling, `outcome.layers` carries the audit's persist info
    /// (so the handler can surface `branch_advanced = true`) and
    /// `outcome.error` carries the user-layer rejection (so the
    /// handler can surface `success = false` with the validation
    /// errors). See [`MultiLayerOutcome`] for the structural
    /// rationale.
    ///
    /// D41 §6.
    pub fn run(self, root: LayerEmission) -> MultiLayerOutcome {
        let span = tracing::info_span!(operation::COMMIT_ORCHESTRATOR_RUN, branch = self.branch);
        let _enter = span.enter();

        // Unpack `self` once so we can move pieces independently into
        // the per-iteration `PipelineConfig`s below without borrow
        // gymnastics around `self.institutions` (which is `!Copy`).
        let Self {
            ctx,
            pool,
            persister,
            host,
            branch,
            policy,
            institutions,
            did_drain,
        } = self;

        // RAII guard — drop returns the working set to the pool.
        let mut ws_guard = pool.acquire();
        let mut pending: VecDeque<(u32, LayerEmission)> = VecDeque::from([(0, root)]);
        let mut layers: Vec<LayerCommitOutcome> = Vec::new();
        let mut first_err: Option<CommitError> = None;
        // Final successfully-landed top of branch. `None` until the
        // first pipeline advances. Used as the `top_layer` for
        // `didDrain` and as the revert target for `!branch_advanced`.
        let mut last_advanced: Option<Arc<crate::layer::Layer>> = Some(Arc::clone(ctx.head()));
        // Did any pipeline actually advance the branch this drain? `didDrain`
        // (the institution-index rebuild) is a no-op when nothing landed — the
        // chain is unchanged, so its index is too. Critically, the rebuild scans
        // the whole chain, so running it on a fully-deduped re-run (anchored-commit
        // cache hit) would pay O(chain) for nothing — pure latency on a "cached"
        // commit. Gate on a real advance.
        let mut any_advanced = false;

        while let Some((depth, em)) = pending.pop_front() {
            if depth >= MAX_EMISSION_DEPTH {
                // Record the error and stop draining. ws_guard drops
                // at function exit — RAII release.
                if first_err.is_none() {
                    first_err = Some(CommitError::EmissionDepthExceeded {
                        depth,
                        layer_name: em.name,
                    });
                }
                break;
            }

            let pre_run_head = Arc::clone(ctx.head());
            let em_name = em.name;
            let em_role = em.role;
            let em_pipeline = em.pipeline;

            // Materialise the builder from the emission's resources +
            // tombstones, parented at the current ctx.head().
            let builder = match em.materialize(&pre_run_head) {
                Ok(b) => b,
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                    continue;
                }
            };

            // Each pipeline run needs its own `PipelineConfig`; the
            // `institutions` field is `!Copy`, so synthesise a fresh
            // borrow-free clone from the orchestrator's source. This
            // is cheap (two `Arc::clone`s + a `PhantomData`).
            let institutions_for_run = institutions.as_ref().map(|i| InstitutionContext {
                index: Arc::clone(&i.index),
                runtime: Arc::clone(&i.runtime),
                _marker: std::marker::PhantomData,
            });
            let cfg = PipelineConfig {
                persister,
                host,
                branch,
                policy: policy.clone(),
                institutions: institutions_for_run,
                storage: ctx.storage().clone(),
            };
            let pipeline = CommitPipeline::for_kind(em_pipeline);
            let result = pipeline.run(em_name, em_role, builder, cfg, &mut ws_guard);

            match result {
                Ok(outcome) if outcome.persist.branch_advanced => {
                    // D41 §6.1: branch advanced — install the new
                    // layer as ctx.head, queue children, record outcome.
                    if let Err(e) = ctx.advance_head(Arc::clone(&outcome.layer), em_name) {
                        // ReadOnly context — the caller misconfigured
                        // the orchestrator. Surface as Validation.
                        if first_err.is_none() {
                            first_err = Some(CommitError::Validation {
                                errors: vec![crate::validation::ValidationError {
                                    resource_id: None,
                                    property: None,
                                    rule: crate::validation::ValidationRule::InstitutionValidation,
                                    message: format!(
                                        "commit orchestrator: advance_head rejected: {e}"
                                    ),
                                }],
                                total_violations: 1,
                            });
                        }
                        continue;
                    }
                    last_advanced = Some(Arc::clone(&outcome.layer));
                    any_advanced = true;
                    // Drain emissions in FIFO order at depth+1. Both
                    // Child and Sibling drain identically here —
                    // when the parent landed, the routing
                    // distinction is irrelevant (D41 §6.1).
                    for child in &outcome.emissions {
                        pending.push_back((depth + 1, child.clone()));
                    }
                    layers.push(outcome);
                }
                Ok(outcome) => {
                    // !branch_advanced (D41 §6.4). No-op CAS outcome:
                    // different-position cache hit, NeedsWitnessedMerge,
                    // or no-backend. Not a rejection — pre_run_head ==
                    // ctx.head() now (advance_head wasn't called).
                    // Descendants (including Siblings on
                    // outcome.emissions) are dropped: their parent did
                    // not land. didPersist hooks did not run.
                    layers.push(outcome);
                }
                Err(PipelineRunErr {
                    error,
                    sibling_emissions,
                }) => {
                    // D41 §6.1 Err arm — the heart of the
                    // AutoOnLoad-Fails audit path. Rescue any Sibling
                    // emissions that phases queued before the failing
                    // phase. ctx.head did not advance (the pipeline
                    // failed), so pre_run_head == ctx.head() now; the
                    // rescued Siblings re-queue at depth 0 and their
                    // parent will be ctx.head() (= pre_run_head).
                    for sib in sibling_emissions {
                        pending.push_back((0, sib));
                    }
                    if first_err.is_none() {
                        first_err = Some(error);
                    }
                    // Continue draining so the audit anchor lands
                    // before we surface the error.
                }
            }
        }

        // Drop the working-set guard before didDrain — the hooks
        // don't need it, and releasing early lets a future
        // hook-side commit reuse the pool slot.
        drop(ws_guard);

        // didDrain hooks. Construct a `DrainState` with the final top
        // layer (or `None` if nothing landed) and run the static hook
        // list. Hook errors flow into `drain_state.hook_errors`, then
        // onto `MultiLayerOutcome.drain_hook_errors` on the Ok path,
        // or get logged-and-dropped on the Err path (see below).
        //
        // `top_layer` is `None` only if `ctx.head` was never advanced
        // *and* it was the initial head — but we initialised
        // `last_advanced` to `Some(Arc::clone(ctx.head()))`, so
        // it stays `Some(_)` for any non-trivial drain. The
        // hook's `None` branch is reserved for a future surface
        // where the orchestrator explicitly signals "no layer landed."
        let top_layer = last_advanced;
        let mut drain_state = DrainState {
            top_layer,
            host,
            hook_errors: Vec::new(),
            _marker: std::marker::PhantomData,
        };
        if any_advanced && !did_drain.is_empty() {
            let hook_span = tracing::info_span!(operation::COMMIT_DID_DRAIN);
            let _hook_enter = hook_span.enter();
            for hook in did_drain {
                let outcome = hook(&mut drain_state);
                drain_state.hook_errors.extend(outcome.errors);
            }
        }
        let drain_hook_errors = drain_state.hook_errors;

        // D41 Phase E: always return `MultiLayerOutcome` — the
        // optional `error` carries the first pipeline `Err` if one
        // occurred. drain_hook_errors flow into the outcome either
        // way (the audit anchor may be durably on disk even on the
        // Err path, so callers want to know about index-rebuild
        // failures regardless).
        MultiLayerOutcome {
            layers,
            drain_hook_errors,
            error: first_err,
        }
    }
}

/// Default `didDrain` slice: `rebuild_institution_index` only.
static DEFAULT_DID_DRAIN: &[DidDrainHook] = &[rebuild_institution_index];

#[cfg(test)]
mod tests {
    //! D41 Phase D orchestrator tests.
    //!
    //! Each test wires a minimal core layer, a [`StubHost`] tracking
    //! `rebuild_institution_index` calls, and either the canonical
    //! [`super::super::BackendStorePersister`] (lattice path; no CAS,
    //! `branch_advanced=false`) or a custom [`StubPersister`] that
    //! advances the branch.
    //!
    //! The Sibling-rescue test is the most important: it confirms that
    //! a phase-error path still drains audit-anchor Sibling emissions
    //! before surfacing the error to the caller, which is the
    //! structural payoff of the [`super::super::EmissionKind`]
    //! addition.
    use super::*;
    use crate::commit::hooks::CommitHookHost;
    use crate::commit::persister::PersistedLayerInfo;
    use crate::commit::EmissionKind;
    use crate::commit::LayerRole;
    use crate::commit::PipelineKind;
    use crate::context::{ExecutionContext, ExecutionMode};
    use crate::layer::{Layer, LayerBuilder, LayerStorage};
    use crate::ontology::eigon_json;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::Resource;
    use crate::validation::{CommitWorkingSetPool, ValidationError};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn test_storage() -> LayerStorage {
        LayerStorage::in_memory()
    }

    fn build_core_layer(storage: LayerStorage) -> Arc<Layer> {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in resources {
            builder.add_resource(r).unwrap();
        }
        Arc::new(builder.build(storage))
    }

    /// Stub host counting rebuild calls. Phase D tests use this to
    /// confirm `didDrain` fires exactly once per orchestrator run.
    struct StubHost {
        rebuild_calls: AtomicUsize,
    }

    impl StubHost {
        fn new() -> Self {
            Self {
                rebuild_calls: AtomicUsize::new(0),
            }
        }

        fn rebuild_count(&self) -> usize {
            self.rebuild_calls.load(Ordering::SeqCst)
        }
    }

    impl CommitHookHost for StubHost {
        fn rebuild_institution_index(
            &self,
            _top_layer: &Arc<Layer>,
        ) -> Result<(), Vec<ValidationError>> {
            self.rebuild_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Persister that records calls and returns
    /// `branch_advanced` according to a configurable flag. Backed by an
    /// in-memory store so the test can inspect what landed.
    struct StubPersister {
        backend: crate::storage::memory::MemoryPersistentBackend,
        branch_advanced: bool,
        persists: Mutex<Vec<crate::layer::LayerId>>,
    }

    impl StubPersister {
        fn new(branch_advanced: bool) -> Self {
            Self {
                backend: crate::storage::memory::MemoryPersistentBackend::new(),
                branch_advanced,
                persists: Mutex::new(Vec::new()),
            }
        }

        fn persisted_ids(&self) -> Vec<crate::layer::LayerId> {
            self.persists.lock().unwrap().clone()
        }
    }

    impl LayerPersister for StubPersister {
        fn persist(
            &self,
            _branch: &str,
            layer: &Arc<Layer>,
        ) -> Result<PersistedLayerInfo, ValidationError> {
            use crate::storage::PersistentBackend;
            self.backend
                .store_layer(layer)
                .map_err(|e| ValidationError {
                    resource_id: None,
                    property: None,
                    rule: crate::validation::ValidationRule::InstitutionValidation,
                    message: format!("stub persister: {e}"),
                })?;
            self.persists.lock().unwrap().push(layer.id().clone());
            Ok(PersistedLayerInfo {
                layer_id: layer.id().clone(),
                branch_advanced: self.branch_advanced,
                merge_outcome: None,
                cache_hit_different_position: false,
            })
        }
    }

    fn make_user_resource(local: &str) -> Resource {
        // Use a urn:eigenius:user:* IRI so the core-namespace guard
        // doesn't reject. Stamp `is_a: [core:Class]` plus the
        // description + short_name that `core:Class` declares as
        // required — every resource must declare at least one is_a
        // per `Validator::validate_resource`, and instances of Class
        // must satisfy its required-properties set. The orchestrator
        // tests don't exercise class-typing semantics so the specific
        // target doesn't matter; we use Class for its built-in
        // availability in the bootstrap chain.
        use crate::ontology::resource::Value;
        let mut r = Resource::new(Iri::parse(&format!("urn:eigenius:user:{local}")).unwrap());
        r.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
        );
        r.set(
            Iri::parse("urn:eigenius:core:short_name").unwrap(),
            Value::String(local.to_string()),
        );
        r.set(
            Iri::parse("urn:eigenius:core:description").unwrap(),
            Value::String(format!("test resource {local}")),
        );
        r
    }

    /// Test 1 — single-layer success: a `WithRetroactive` root emission
    /// lands one layer; ctx.head advances; `didDrain` fires once.
    #[test]
    fn single_layer_success() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadWrite, storage);
        let pool = CommitWorkingSetPool::in_memory();
        let persister = StubPersister::new(true);
        let host = StubHost::new();

        let head_before = ctx.head().id().clone();

        let root = LayerEmission {
            role: LayerRole::User,
            name: "user",
            pipeline: PipelineKind::WithRetroactive,
            kind: EmissionKind::Child,
            resources: vec![make_user_resource("alpha")],
            tombstones: std::collections::BTreeSet::new(),
        };

        let orchestrator = CommitOrchestrator {
            ctx: &mut ctx,
            pool: &pool,
            persister: &persister,
            host: &host,
            branch: "main",
            policy: CommitPolicy::default(),
            institutions: None,
            did_drain: CommitOrchestrator::default_did_drain(),
        };

        let outcome = orchestrator.run(root);
        assert!(
            outcome.error.is_none(),
            "commit must succeed: {:?}",
            outcome.error
        );
        assert_eq!(outcome.layers.len(), 1);
        assert_eq!(outcome.drain_hook_errors.len(), 0);
        assert_ne!(*ctx.head().id(), head_before, "ctx.head must advance");
        assert_eq!(
            host.rebuild_count(),
            1,
            "rebuild_institution_index must fire exactly once"
        );
    }

    /// Test 2 — single-layer !branch_advanced (no rescue): the
    /// pipeline returned Ok but the persister reported the branch did
    /// not advance. Orchestrator records the outcome but does not move
    /// ctx.head; no Siblings are rescued (Ok path doesn't rescue).
    #[test]
    fn single_layer_not_branch_advanced() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadWrite, storage);
        let pool = CommitWorkingSetPool::in_memory();
        let persister = StubPersister::new(false);
        let host = StubHost::new();

        let head_before = ctx.head().id().clone();

        let root = LayerEmission {
            role: LayerRole::User,
            name: "user",
            pipeline: PipelineKind::WithRetroactive,
            kind: EmissionKind::Child,
            resources: vec![make_user_resource("alpha")],
            tombstones: std::collections::BTreeSet::new(),
        };

        let orchestrator = CommitOrchestrator {
            ctx: &mut ctx,
            pool: &pool,
            persister: &persister,
            host: &host,
            branch: "main",
            policy: CommitPolicy::default(),
            institutions: None,
            did_drain: CommitOrchestrator::default_did_drain(),
        };

        let outcome = orchestrator.run(root);
        assert!(
            outcome.error.is_none(),
            "Ok even on !branch_advanced (no rejection occurred)"
        );
        assert_eq!(outcome.layers.len(), 1);
        assert!(!outcome.layers[0].persist.branch_advanced);
        assert_eq!(
            *ctx.head().id(),
            head_before,
            "ctx.head must NOT advance on !branch_advanced"
        );
        // Persist still ran (the backend recorded the store).
        assert_eq!(persister.persisted_ids().len(), 1);
    }

    /// Test 3 — single-layer Err (no Siblings): structural validation
    /// fails on a malformed resource. ctx.head unchanged; no rescue;
    /// orchestrator returns Err.
    #[test]
    fn single_layer_err_no_siblings() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadWrite, storage);
        let pool = CommitWorkingSetPool::in_memory();
        let persister = StubPersister::new(true);
        let host = StubHost::new();

        let head_before = ctx.head().id().clone();

        // Malformed: a resource declaring `is_a` of a non-existent
        // class. Structural validation rejects it.
        let mut bad = Resource::new(Iri::parse("urn:eigenius:user:bogus").unwrap());
        bad.set(
            Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:user:NoSuchClass".to_string()),
        );

        let root = LayerEmission {
            role: LayerRole::User,
            name: "user",
            pipeline: PipelineKind::WithRetroactive,
            kind: EmissionKind::Child,
            resources: vec![bad],
            tombstones: std::collections::BTreeSet::new(),
        };

        let orchestrator = CommitOrchestrator {
            ctx: &mut ctx,
            pool: &pool,
            persister: &persister,
            host: &host,
            branch: "main",
            policy: CommitPolicy::default(),
            institutions: None,
            did_drain: CommitOrchestrator::default_did_drain(),
        };

        let outcome = orchestrator.run(root);
        assert!(matches!(
            outcome.error,
            Some(CommitError::Validation { .. })
        ));
        assert_eq!(*ctx.head().id(), head_before, "ctx.head must NOT advance");
        assert_eq!(
            persister.persisted_ids().len(),
            0,
            "persist must not have been called"
        );
    }

    /// Stub institution returning a Fails Verdict from every query.
    /// Lifted from `context::tests`. Used by the Sibling-rescue test
    /// below to trigger the autoonload_dispatch Err path that queues
    /// the `verdict_provenance` Sibling.
    struct AlwaysFails;
    impl crate::institution::runtime::Institution for AlwaysFails {
        fn institution_iri(&self) -> &Iri {
            static INST_IRI: std::sync::OnceLock<Iri> = std::sync::OnceLock::new();
            INST_IRI.get_or_init(|| Iri::parse("urn:eigenius:test:rescue:inst").unwrap())
        }
        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<crate::nbe::val::Val, crate::institution::error::InstitutionError> {
            unreachable!()
        }
        fn reify(
            &self,
            _: &Iri,
            _: &crate::nbe::val::Val,
            _: &ExecutionContext,
        ) -> Result<Resource, crate::institution::error::InstitutionError> {
            unreachable!()
        }
        fn query(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<
            crate::institution::runtime::QueryOutcome,
            crate::institution::error::InstitutionError,
        > {
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
                crate::ontology::resource::Value::Array(vec![
                    crate::ontology::resource::Value::String(
                        "urn:eigenius:institution:verdicts:fails".into(),
                    ),
                ]),
            );
            Ok(crate::institution::runtime::QueryOutcome::from_output(r))
        }
    }

    /// Wire an AutoOnLoad QueryClass on top of the full bootstrap
    /// chain (core + reflection + institution + runtime + ...) that
    /// gates `urn:eigenius:test:rescue:Subject` through the
    /// `AlwaysFails` institution. The bootstrap chain is needed so
    /// the rescued `verdict_provenance` follow-up's structural
    /// validation can resolve `urn:eigenius:institution:Verdict` and
    /// `urn:eigenius:reflection:DerivedResource`.
    ///
    /// Returns the chain plus its index + runtime.
    fn build_rescue_setup() -> (
        Arc<Layer>,
        Arc<crate::institution::registry::InstitutionIndex>,
        Arc<crate::institution::runtime::InstitutionRuntime>,
        LayerStorage,
    ) {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let storage = ctx.storage().clone();
        let bootstrap_head = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("test", Some(bootstrap_head));

        let inst_iri = "urn:eigenius:test:rescue:inst";
        let qc_iri = "urn:eigenius:test:rescue:check";
        let subject = "urn:eigenius:test:rescue:Subject";

        let mut qc = Resource::new(Iri::parse(qc_iri).unwrap());
        qc.set(
            Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    crate::ontology::well_known::QUERY_CLASS_CLASS.into(),
                ),
            ]),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:query_class").unwrap(),
            crate::ontology::resource::Value::String(subject.into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:result_class").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:institution:Verdict".into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:dispatch_role").unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    "urn:eigenius:institution:dispatch_roles:auto_on_load".into(),
                ),
            ]),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:query_handler").unwrap(),
            crate::ontology::resource::Value::String("urn:eigenius:test:rescue:proc:check".into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            crate::ontology::resource::Value::String(inst_iri.into()),
        );
        b.add_resource(qc).unwrap();

        // Declare the Subject class so the per-subject `is_a` shapes
        // pass structural validation. Otherwise validate rejects
        // before autoonload_dispatch runs.
        let mut subject_class =
            Resource::new(Iri::parse("urn:eigenius:test:rescue:Subject").unwrap());
        subject_class.set(
            Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    crate::ontology::well_known::CLASS.to_string(),
                ),
            ]),
        );
        subject_class.set(
            Iri::parse("urn:eigenius:core:description").unwrap(),
            crate::ontology::resource::Value::String("rescue test subject".into()),
        );
        subject_class.set(
            Iri::parse("urn:eigenius:core:short_name").unwrap(),
            crate::ontology::resource::Value::String("RescueSubject".into()),
        );
        b.add_resource(subject_class).unwrap();

        let layer = Arc::new(b.build(storage.clone()));
        let (idx, errors) = crate::institution::registry::InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");
        let mut runtime = crate::institution::runtime::InstitutionRuntime::new();
        runtime.register(Box::new(AlwaysFails)).unwrap();
        (layer, Arc::new(idx), Arc::new(runtime), storage)
    }

    /// Test 4 — Sibling rescue on Err. A `WithInstitutions` user-layer
    /// pipeline runs against an `AlwaysFails` institution; the
    /// `autoonload_dispatch` phase queues a `verdict_provenance`
    /// Sibling emission and returns Err (Fails). The orchestrator
    /// then:
    ///
    /// 1. records the first Err (Validation: Fails verdict),
    /// 2. rescues the Sibling emission via `PipelineRunErr.sibling_emissions`,
    /// 3. drains the rescued Sibling as a `StructuralFollowup` pipeline
    ///    which lands successfully (kernel-emitted content; no structural
    ///    re-validation per D41 §5),
    /// 4. advances `ctx.head` to the audit layer (the audit anchor is now
    ///    durable on the chain),
    /// 5. surfaces the *original* user-layer Err to the caller (not Ok,
    ///    despite the audit having landed).
    ///
    /// This is the structural payoff of the [`EmissionKind`] addition:
    /// the audit anchor lands even when the gated user-layer commit was
    /// rejected, AND the orchestrator surfaces the rejection to the
    /// caller so they know their content didn't make it.
    #[test]
    fn sibling_rescue_on_fails_verdict() {
        let (chain, idx, runtime, storage) = build_rescue_setup();
        let mut ctx = ExecutionContext::new(
            Arc::clone(&chain),
            "test",
            ExecutionMode::ReadWrite,
            storage,
        );
        let pool = CommitWorkingSetPool::in_memory();
        let persister = StubPersister::new(true);
        let host = StubHost::new();

        // The gated subject — typed as the QueryClass's
        // `query_class` IRI so the AutoOnLoad dispatcher fires.
        let mut subject = Resource::new(Iri::parse("urn:eigenius:test:rescue:s1").unwrap());
        subject.set(
            Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String("urn:eigenius:test:rescue:Subject".into()),
            ]),
        );

        let root = LayerEmission {
            role: LayerRole::User,
            name: "user",
            pipeline: PipelineKind::WithInstitutions,
            kind: EmissionKind::Child,
            resources: vec![subject],
            tombstones: std::collections::BTreeSet::new(),
        };

        let head_before = ctx.head().id().clone();
        let outcome = {
            let orchestrator = CommitOrchestrator {
                ctx: &mut ctx,
                pool: &pool,
                persister: &persister,
                host: &host,
                branch: "main",
                policy: CommitPolicy::default(),
                institutions: Some(InstitutionContext {
                    index: idx,
                    runtime,
                    _marker: std::marker::PhantomData,
                }),
                did_drain: CommitOrchestrator::default_did_drain(),
            };
            orchestrator.run(root)
        };

        // The orchestrator returns the *first* recorded error — the
        // user-layer pipeline's AutoOnLoad-Fails Validation. The
        // rescue queue's outcome (here, structural-validation Err
        // because the InductiveType-as-Class declaration shape isn't
        // reconciled in Phase D) does not overwrite first_err.
        match outcome.error.as_ref() {
            Some(CommitError::Validation { errors, .. }) => {
                // The error must be the AutoOnLoad-Fails one, not the
                // rescued-Sibling structural error. If first_err were
                // overwritten by the Sibling's failure, the message
                // would mention `ClassTypeMismatch` instead of
                // `returned Fails`.
                let has_fails = errors.iter().any(|e| e.message.contains("returned Fails"));
                assert!(
                    has_fails,
                    "expected AutoOnLoad-Fails error to surface as first_err; got {errors:?}"
                );
            }
            other => panic!("expected Validation Err from AlwaysFails verdict; got {other:?}"),
        }

        // The audit Sibling landed: ctx.head advanced from
        // head_before to the verdict_provenance layer. The user-layer
        // pipeline failed before persist so its content is not on the
        // chain — only the audit anchor is.
        let head_after = ctx.head().id().clone();
        assert_ne!(
            head_after, head_before,
            "audit Sibling should have landed and advanced ctx.head"
        );

        // The user subject IRI must not resolve from the new head —
        // the user layer was rejected. Only the verdict_provenance
        // resources should be reachable.
        let user_subject = Iri::parse("urn:eigenius:test:rescue:s1").unwrap();
        assert!(
            ctx.head().get_resource(&user_subject).is_none(),
            "rejected user content must not be reachable from the audit layer"
        );

        // didDrain fires exactly once even on the Err path. This is
        // the structural symmetry: drain hooks always run, so the
        // institution index gets rebuilt regardless of commit outcome.
        assert_eq!(host.rebuild_count(), 1);
    }

    /// Test 5 — depth cap: synthetic test driving the orchestrator's
    /// depth check directly by setting `MAX_EMISSION_DEPTH` semantics.
    /// We can't easily induce 4 generations of emissions through the
    /// public surface without rewriting pipelines, so instead we
    /// queue an emission with a name whose pipeline returns Ok with a
    /// child of its own — but the canned pipelines don't emit. The
    /// best we can do without pipeline plumbing: assert the cap
    /// constant equals the documented value.
    #[test]
    fn emission_depth_cap_is_documented_value() {
        // D41 §6.3: documented value is 4.
        assert_eq!(MAX_EMISSION_DEPTH, 4);
    }

    /// Test 6 — `didDrain` fires exactly once per orchestrator run
    /// regardless of layer count. Drives a single-layer success and
    /// asserts the rebuild counter is 1.
    #[test]
    fn did_drain_fires_once_per_run() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadWrite, storage);
        let pool = CommitWorkingSetPool::in_memory();
        let persister = StubPersister::new(true);
        let host = StubHost::new();

        let root = LayerEmission {
            role: LayerRole::User,
            name: "user",
            pipeline: PipelineKind::WithRetroactive,
            kind: EmissionKind::Child,
            resources: vec![make_user_resource("alpha")],
            tombstones: std::collections::BTreeSet::new(),
        };

        let orchestrator = CommitOrchestrator {
            ctx: &mut ctx,
            pool: &pool,
            persister: &persister,
            host: &host,
            branch: "main",
            policy: CommitPolicy::default(),
            institutions: None,
            did_drain: CommitOrchestrator::default_did_drain(),
        };

        let outcome = orchestrator.run(root);
        assert!(outcome.error.is_none(), "commit must succeed");
        assert_eq!(host.rebuild_count(), 1);
    }
}
