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

//! gRPC server for the Eigenius kernel.
//!
//! Wraps the kernel's existing functionality as a tonic gRPC service.
//! See design doc D5 for the full API specification.
//!
//! ## Module layout
//!
//! Handlers are split per-domain into sibling modules: [`load`],
//! [`query`], [`programs`], [`reflect`], [`inspect`], [`tasks`],
//! [`branches`], [`tags`], [`gc`], [`consolidate`], and [`topology`].
//! Each handler's implementation lives as an inherent `impl
//! EigeniusService` block in its file (Rust permits multiple inherent
//! impls for the same type across files in the same module tree). The
//! single `impl EigeniusKernel for EigeniusService` block at the
//! bottom of this file is a thin delegate layer — one match arm per
//! RPC routing to the corresponding `handle_*` inherent method. This
//! keeps the gRPC trait registration coherent (the trait surface is
//! one block in one file) without forcing every handler body into
//! this module.
//!
//! Shared proto-translation helpers live in [`helpers`]; the
//! `CommitHookHost` impl + its async-to-sync delegate plumbing lives
//! in [`hooks`].

use crate::bootstrap;
use crate::context::{ExecutionContext, ExecutionMode};
use crate::layer::{build_chain, LayerStorage};
use crate::observability::{field, operation, RpcGuard};
use crate::ontology::{eigon_cbor, eigon_json, Resource};
use crate::program::component::ComponentRegistry;
use crate::program::trace::{InMemoryTraceStore, TraceStore};
use crate::server::lifecycle::BackendTraceStore;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("eigenius.v1");
}

mod branches;
mod consolidate;
mod gc;
mod helpers;
mod hooks;
mod inspect;
mod lifecycle;
mod load;
mod parse;
pub use parse::ParseConfig;
mod programs;
mod query;
mod reflect;
mod tags;
mod tasks;
pub mod topology;

pub use helpers::DEFAULT_BRANCH;
pub use lifecycle::{
    resume_sweep, start_server, EmbedderStartupConfig, ResumeConfig, ResumeInputs, ResumeState,
};

use proto::eigenius_kernel_server::{EigeniusKernel, EigeniusKernelServer};
use proto::*;

/// Per-branch ExecutionContext cache (Phase 14g).
///
/// Each branch the server has touched lives in this map as an
/// `Arc<RwLock<ExecutionContext>>`. The cache is populated lazily:
/// requests targeting an unseen branch trigger a `get_branch` lookup
/// against the backend and a chain rehydration via `load_chain_from`.
///
/// `"main"` is seeded eagerly at construction time so the in-memory
/// (no-backend) path keeps working — that path can never serve any
/// branch but `"main"`.
///
/// **Concurrency.** The outer `RwLock<HashMap>` is held only for the
/// lookup/insert; per-branch operations work against the inner
/// `Arc<RwLock<ExecutionContext>>` so different branches don't
/// contend on each other.
pub(crate) struct BranchContextCache {
    pub(crate) contexts: RwLock<HashMap<String, Arc<RwLock<ExecutionContext>>>>,
}

impl BranchContextCache {
    fn new(main_ctx: ExecutionContext) -> Self {
        let mut map = HashMap::new();
        map.insert(DEFAULT_BRANCH.to_string(), Arc::new(RwLock::new(main_ctx)));
        Self {
            contexts: RwLock::new(map),
        }
    }

    /// Drop the cached `ExecutionContext` for `branch`. The next
    /// `get_branch_context(branch)` call rebuilds it from the
    /// backend's current branch ref (slow path).
    ///
    /// Use after any operation that mutates the branch ref or the
    /// redirect map outside `ExecutionContext` itself — notably
    /// `consolidate_chain`, which advances the backend's branch ref
    /// (at-head) or installs a redirect (below-head) without going
    /// through `ctx.commit()`. Without invalidation here, subsequent
    /// reads against the branch use the stale in-memory `Layer`
    /// graph and miss the new tip / redirect.
    ///
    /// In-flight requests holding an `Arc<RwLock<ExecutionContext>>`
    /// continue against their stale ctx for the rest of their call;
    /// new requests see the rebuilt one. Consolidation runs under
    /// `with_branch_lock`, which serialises it with other branch-
    /// mutating operations, so the transient window between "backend
    /// advanced" and "cache invalidated" is the duration of this
    /// method, not a real race.
    pub(super) async fn invalidate(&self, branch: &str) {
        let mut cache = self.contexts.write().await;
        cache.remove(branch);
    }
}

pub struct EigeniusService {
    /// Per-branch ExecutionContext cache. `"main"` is always present.
    pub(crate) branch_contexts: Arc<BranchContextCache>,
    /// Outer lock allows swapping the registry on load.
    /// Inner Arc allows cheap cloning for passing to the evaluator.
    pub(crate) components: Arc<RwLock<Arc<ComponentRegistry>>>,
    pub(crate) trace_store: Arc<dyn TraceStore>,
    /// institution index — derived view of the layer chain rebuilt
    /// after every commit. Outer lock allows swapping; inner Arc lets
    /// the evaluator clone cheaply when constructing the IO effect engine.
    pub(crate) institution_index: Arc<RwLock<Arc<crate::institution::registry::InstitutionIndex>>>,
    /// institution runtime — `Box<dyn Institution>` per
    /// institution IRI. Populated by `rebuild_institution_index` after
    /// each commit with the external and in-process institutions
    /// declared in the layer chain; otherwise empty.
    pub(crate) institution_runtime:
        Arc<RwLock<Arc<crate::institution::runtime::InstitutionRuntime>>>,
    /// Process-global registry of in-process institution
    /// implementations (Phase 20a.1, D28). Statically-linked
    /// institution crates (e.g. `eigenius-lean`) pre-register their
    /// `Institution` impl here at orchestrator startup, and the
    /// chain-scan registration pass looks them up by IRI when it
    /// encounters `runtime: in_process` declarations.
    pub(crate) in_process_registry:
        Arc<crate::institution::in_process_registry::InProcessInstitutionRegistry>,
    /// Optional persistent backend. When present, committed layers,
    /// the seed manifest, and trace state all live here; absent means
    /// the server is in-memory-only (the pre-Phase-9a behaviour).
    /// See D13.
    pub(crate) backend: Option<Arc<dyn crate::storage::PersistentBackend>>,
    /// Persistent task store (D21 §3.1). `Some` whenever a backend
    /// is attached — every `RunProgram` allocates a task record so
    /// trace lookups can route through per-task positional keys and
    /// a mid-flight crash leaves a recoverable `Running` task for
    /// the resume sweep to pick up.
    pub(crate) task_store: Option<Arc<dyn crate::task::TaskStore>>,
    /// Single hardwired session (D21 §3.7). Tracks the session's
    /// active_top; advances on every successful Load and on
    /// fast-forward task completion. In 9b-iii there is exactly one
    /// of these per running kernel.
    pub(crate) session: Arc<RwLock<crate::task::Session>>,
    /// Live state of the startup resume sweep (D21 §6). Shared with
    /// the background sweep task so `Health` can report progress.
    pub(crate) resume_state: Arc<ResumeState>,
    /// Optional gRPC client for the orchestrator. Used to dispatch
    /// external institutions and remote IO components to the
    /// orchestrator substrate during program execution. None means no
    /// orchestrator is configured — chains declaring `runtime: external`
    /// institutions will fail to dispatch (surfaced with a clear warning
    /// at index-rebuild time).
    pub(crate) orchestrator_client: Option<
        Arc<
            tokio::sync::Mutex<
                proto::component_executor_client::ComponentExecutorClient<
                    crate::program::remote::OrchestratorTransport,
                >,
            >,
        >,
    >,
    /// Per-server pool of pooled [`crate::validation::CommitWorkingSet`]
    /// instances. Each commit-shaped RPC orchestrator run acquires one
    /// working set and re-uses it across every pipeline run in that
    /// invocation. Per D41 §11.2: branches commit serially (per-branch
    /// lock), so per-server is sufficient — a per-branch pool buys
    /// nothing today.
    ///
    /// D41 Phase E.
    pub(crate) commit_ws_pool: crate::validation::CommitWorkingSetPool,

    /// D43 §5.2 — registry of registered Embedder Components keyed by
    /// `vec_model` IRI. Installed once at startup by
    /// [`Self::with_embedders`] (the CLI loads config and builds them
    /// before calling [`start_server`]); the query handlers clone the
    /// `Arc` cheaply for each request. `EmbedderRegistry::new()`
    /// (empty) is the no-vector-retrieval case — the service still
    /// starts and queries that don't hit VectorIndex Resources work
    /// normally.
    pub(crate) embedders: Arc<crate::program::embedder::EmbedderRegistry>,

    /// D43 §5.5 / M5.8 — coordinator owning the per-layer sweep +
    /// reindex registries and the embedder/cache dispatch surface.
    /// `None` when no embedders are registered; the `didPersist`
    /// hook short-circuits in that case (the sweep is a no-op).
    pub(crate) sweep_coordinator: Option<Arc<crate::task::sweep_registry::SweepCoordinator>>,

    /// The [`crate::commit::LayerPersister`] threaded into every
    /// orchestrator run. Owns the anchored-commit cache probe + branch
    /// CAS dispatch (D34 §G.1 / D33 §6). Extracted out of
    /// `EigeniusService` so persistence isn't coupled to the gRPC
    /// struct; the impl is testable in isolation against any
    /// `PersistentBackend`. Wraps the same `Option<Arc<dyn
    /// PersistentBackend>>` value as `self.backend`, so the no-backend
    /// case is handled internally with the `branch_advanced = true`
    /// in-memory shape.
    pub(crate) persister: Arc<crate::commit::BackendPersister>,

    /// `ParseSentence` parse-path configuration (D63/GH#97 Lever 1): the sense cap, cell beam,
    /// injected lemmatizer, and opt-in LLM reranker the handler builds each request's index with.
    /// Defaults to [`ParseConfig::default`] (cap+beam on, `Identity` lemmatizer, ranker off); a
    /// binary swaps in a real lemmatizer (and turns the ranker on) via [`Self::with_parse_config`].
    pub(crate) parse_config: ParseConfig,
}

impl EigeniusService {
    /// Install the `ParseSentence` [`ParseConfig`] (D63/GH#97 Lever 1). Called by the orchestrator/CLI
    /// startup to inject a real lemmatizer (`MorphyLemmatizer`, which the kernel can't depend on) and
    /// to enable the contextual reranker. Builder-style; default is [`ParseConfig::default`].
    pub fn with_parse_config(mut self, config: ParseConfig) -> Self {
        self.parse_config = config;
        self
    }
    /// Create a new service by bootstrapping the kernel.
    pub fn new() -> Result<Self, String> {
        Self::with_components(ComponentRegistry::default())
    }

    /// Create a new service with a custom component registry.
    ///
    /// Uses the in-memory bootstrap path. See
    /// [`Self::with_persistent_backend`] for the durable variant.
    pub fn with_components(components: ComponentRegistry) -> Result<Self, String> {
        let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap failed: {e}"))?;
        Ok(Self {
            branch_contexts: Arc::new(BranchContextCache::new(ctx)),
            components: Arc::new(RwLock::new(Arc::new(components))),
            trace_store: Arc::new(InMemoryTraceStore::new()),
            institution_index: Arc::new(RwLock::new(Arc::new(
                crate::institution::registry::InstitutionIndex::new(),
            ))),
            institution_runtime: Arc::new(RwLock::new(Arc::new(
                crate::institution::runtime::InstitutionRuntime::new(),
            ))),
            in_process_registry: Arc::new(
                crate::institution::in_process_registry::InProcessInstitutionRegistry::new(),
            ),
            backend: None,
            task_store: None,
            session: Arc::new(RwLock::new(crate::task::Session::hardwired())),
            resume_state: Arc::new(ResumeState::default()),
            orchestrator_client: None,
            commit_ws_pool: crate::validation::CommitWorkingSetPool::in_memory(),
            persister: Arc::new(crate::commit::BackendPersister::new(None)),
            embedders: Arc::new(crate::program::embedder::EmbedderRegistry::new()),
            sweep_coordinator: None,
            parse_config: ParseConfig::default(),
        })
    }

    /// Create a new service backed by a persistent store.
    ///
    /// Implements the SEED and RESUME paths from D13 §4:
    /// - Empty backend: commit the four embedded ontologies and a
    ///   seed manifest, then treat the backend as authoritative.
    /// - Non-empty backend: reconstruct the `ExecutionContext` from
    ///   the persisted layer chain, verifying the seed manifest against
    ///   the current embedded ontologies (refuse to boot on drift).
    ///
    /// The backend also supplies the trace store, so
    /// `ComponentTrace` reads/writes flow through the same DB.
    pub fn with_persistent_backend(
        components: ComponentRegistry,
        backend: Arc<dyn crate::storage::PersistentBackend>,
    ) -> Result<Self, String> {
        let ctx = bootstrap::bootstrap_persistent(Arc::clone(&backend))
            .map_err(|e| format!("persistent bootstrap failed: {e}"))?;

        // Wrap the backend's trace-store view into an Arc<dyn TraceStore>
        // so the service can hold it independently. We do this by keeping
        // the backend alive via `trace_store_arc_from_backend` — the
        // returned Arc shares ownership with `backend`.
        let trace_store: Arc<dyn TraceStore> =
            Arc::new(BackendTraceStore::new(Arc::clone(&backend)));

        let task_store: Arc<dyn crate::task::TaskStore> =
            Arc::new(crate::task::BackendTaskStore::new(Arc::clone(&backend)));

        Ok(Self {
            branch_contexts: Arc::new(BranchContextCache::new(ctx)),
            components: Arc::new(RwLock::new(Arc::new(components))),
            trace_store,
            institution_index: Arc::new(RwLock::new(Arc::new(
                crate::institution::registry::InstitutionIndex::new(),
            ))),
            institution_runtime: Arc::new(RwLock::new(Arc::new(
                crate::institution::runtime::InstitutionRuntime::new(),
            ))),
            in_process_registry: Arc::new(
                crate::institution::in_process_registry::InProcessInstitutionRegistry::new(),
            ),
            backend: Some(Arc::clone(&backend)),
            task_store: Some(task_store),
            session: Arc::new(RwLock::new(crate::task::Session::hardwired())),
            resume_state: Arc::new(ResumeState::default()),
            orchestrator_client: None,
            commit_ws_pool: crate::validation::CommitWorkingSetPool::in_memory(),
            persister: Arc::new(crate::commit::BackendPersister::new(Some(backend))),
            embedders: Arc::new(crate::program::embedder::EmbedderRegistry::new()),
            sweep_coordinator: None,
            parse_config: ParseConfig::default(),
        })
    }

    /// Attach an orchestrator client so external institutions and remote
    /// IO components can be dispatched to the orchestrator substrate.
    pub fn with_orchestrator_client(
        mut self,
        client: Arc<
            tokio::sync::Mutex<
                proto::component_executor_client::ComponentExecutorClient<
                    crate::program::remote::OrchestratorTransport,
                >,
            >,
        >,
    ) -> Self {
        self.orchestrator_client = Some(client);
        self
    }

    /// D43 §5.2 — install a populated [`EmbedderRegistry`] and the
    /// [`SweepCoordinator`] that wraps it. Called by the orchestrator
    /// startup (the `cmd_serve` path in the CLI) *before* the gRPC
    /// listener comes up so the post-Load `didPersist` hook can see
    /// the coordinator the very first time it fires.
    ///
    /// `batch_size` is the per-sweep batch passed to
    /// [`crate::query::vector::indexing::SweepOptions::batch_size`].
    /// 32 is the v1 default; tune up for GPU sweeps, down if peak RAM
    /// is a constraint.
    ///
    /// When called with an empty registry the coordinator is
    /// installed anyway so the hook surfaces a consistent diagnostic
    /// on a per-query miss (vs. silently dropping the sweep).
    pub fn with_embedders(
        mut self,
        embedders: crate::program::embedder::EmbedderRegistry,
        batch_size: usize,
    ) -> Self {
        let embedders = Arc::new(embedders);
        let coord = Arc::new(
            crate::task::sweep_registry::SweepCoordinator::new(Arc::clone(&embedders), None)
                .with_default_batch_size(batch_size),
        );
        self.embedders = embedders;
        self.sweep_coordinator = Some(coord);
        self
    }

    /// Pre-register an in-process institution implementation (Phase
    /// 20a.1, D28). Statically-linked institution crates call this at
    /// orchestrator startup, *before* the kernel server begins serving
    /// requests, so the chain-scan registration pass can look the
    /// impl up by IRI when it encounters a `runtime: in_process`
    /// declaration.
    ///
    /// Idempotent: re-registering the same IRI replaces the prior
    /// entry, matching the runtime's `replace` discipline.
    pub fn register_in_process_institution(
        &self,
        institution: Arc<dyn crate::institution::runtime::Institution>,
    ) {
        self.in_process_registry.register(institution);
    }

    /// Create a new service with a custom component registry and trace store.
    pub fn with_trace_store(
        components: ComponentRegistry,
        trace_store: Arc<dyn TraceStore>,
    ) -> Result<Self, String> {
        let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap failed: {e}"))?;
        Ok(Self {
            branch_contexts: Arc::new(BranchContextCache::new(ctx)),
            components: Arc::new(RwLock::new(Arc::new(components))),
            trace_store,
            institution_index: Arc::new(RwLock::new(Arc::new(
                crate::institution::registry::InstitutionIndex::new(),
            ))),
            institution_runtime: Arc::new(RwLock::new(Arc::new(
                crate::institution::runtime::InstitutionRuntime::new(),
            ))),
            in_process_registry: Arc::new(
                crate::institution::in_process_registry::InProcessInstitutionRegistry::new(),
            ),
            backend: None,
            task_store: None,
            session: Arc::new(RwLock::new(crate::task::Session::hardwired())),
            resume_state: Arc::new(ResumeState::default()),
            orchestrator_client: None,
            commit_ws_pool: crate::validation::CommitWorkingSetPool::in_memory(),
            persister: Arc::new(crate::commit::BackendPersister::new(None)),
            embedders: Arc::new(crate::program::embedder::EmbedderRegistry::new()),
            sweep_coordinator: None,
            parse_config: ParseConfig::default(),
        })
    }

    /// Create a tonic server from this service.
    pub fn into_server(self) -> EigeniusKernelServer<Self> {
        EigeniusKernelServer::new(self)
    }

    /// Borrow the task store + backend + related Arcs needed to run
    /// the startup resume sweep (D21 §6). Returns `None` when no
    /// persistent backend is attached — nothing to resume.
    pub fn resume_inputs(&self) -> Option<ResumeInputs> {
        let task_store = Arc::clone(self.task_store.as_ref()?);
        let backend = Arc::clone(self.backend.as_ref()?);
        Some(ResumeInputs {
            task_store,
            backend,
            trace_store: Arc::clone(&self.trace_store),
            resume_state: Arc::clone(&self.resume_state),
        })
    }

    /// Snapshot of the current `ComponentRegistry`. Used by the
    /// startup resume sweep, which needs a ComponentRegistry Arc to
    /// hand to `execute_program_nbe_with_institutions` without
    /// holding a lock on `self.components` across an await point.
    pub async fn components_snapshot(&self) -> Arc<ComponentRegistry> {
        Arc::clone(&*self.components.read().await)
    }

    /// Session id of the hardwired session (9b-iii). Read asynchronously
    /// because the session lives behind a `RwLock` in anticipation of
    /// multi-session support landing in Phase 14.
    pub async fn session_id(&self) -> uuid::Uuid {
        self.session.read().await.session_id
    }

    /// Look up — and lazy-build — the `ExecutionContext` for `branch`.
    ///
    /// Phase 14g per-branch dispatch. `"main"` is always present (seeded
    /// at construction). Other branches are loaded on first reference by
    /// reading `backend.get_branch(name)` and rehydrating the chain via
    /// `load_chain_from`.
    ///
    /// Returns:
    /// - `Status::not_found` when the branch ref doesn't exist.
    /// - `Status::failed_precondition` when the in-memory variant is
    ///   asked for any branch other than `"main"`.
    pub(super) async fn get_branch_context(
        &self,
        branch: &str,
    ) -> Result<Arc<RwLock<ExecutionContext>>, Status> {
        // Hot path: cache hit.
        {
            let cache = self.branch_contexts.contexts.read().await;
            if let Some(ctx) = cache.get(branch) {
                return Ok(Arc::clone(ctx));
            }
        }

        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition(format!(
                "branch {branch:?} not available: in-memory mode only serves {DEFAULT_BRANCH:?}"
            ))
        })?;

        // Slow path: write-lock + double-check + lazy build.
        let mut cache = self.branch_contexts.contexts.write().await;
        if let Some(ctx) = cache.get(branch) {
            return Ok(Arc::clone(ctx));
        }
        let head_id = backend
            .get_branch(branch)
            .map_err(|e| Status::internal(format!("get_branch failed: {e}")))?
            .ok_or_else(|| Status::not_found(format!("branch {branch:?} does not exist")))?;
        let storage = LayerStorage::with_persistent(Arc::clone(backend));
        let info = backend
            .load_chain_from(&head_id)
            .map_err(|e| Status::internal(format!("load_chain_from failed: {e}")))?
            .ok_or_else(|| {
                Status::not_found(format!("branch {branch:?} head {head_id} not in store"))
            })?;
        let head = build_chain(info, storage.clone());
        let ctx = ExecutionContext::new(head, branch, ExecutionMode::ReadWrite, storage);
        let ctx_arc = Arc::new(RwLock::new(ctx));
        cache.insert(branch.to_string(), Arc::clone(&ctx_arc));
        Ok(ctx_arc)
    }

    /// Resolve the target layer for a read RPC (D21 §3.6 `at_layer`).
    ///
    /// Empty / invalid hex falls back to the named branch's head (or
    /// `"main"` if `branch` is also empty). When `at_layer` is set and
    /// a backend is attached, reconstructs the layer chain rooted at
    /// that id. `at_layer` and `branch` are mutually exclusive — if
    /// both are set, returns `Status::invalid_argument`.
    pub(super) async fn resolve_read_layer(
        &self,
        at_layer: &str,
        branch: &str,
    ) -> Result<Arc<crate::layer::Layer>, Status> {
        if !at_layer.is_empty() && !branch.is_empty() {
            return Err(Status::invalid_argument(
                "at_layer and branch are mutually exclusive",
            ));
        }
        if at_layer.is_empty() {
            let branch_name = helpers::resolve_branch_name(branch);
            let ctx_arc = self.get_branch_context(branch_name).await?;
            let ctx = ctx_arc.read().await;
            return Ok(Arc::clone(ctx.head()));
        }
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "at_layer requires a persistent backend; none attached".to_string(),
            )
        })?;
        let bytes = hex::decode(at_layer)
            .map_err(|e| Status::invalid_argument(format!("at_layer not valid hex: {e}")))?;
        if bytes.len() != 32 {
            return Err(Status::invalid_argument(
                "at_layer must be a 32-byte SHA-256 (64 hex chars)".to_string(),
            ));
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);
        let layer_id = crate::layer::LayerId(id);
        match backend.load_chain_from(&layer_id) {
            Ok(Some(info)) => Ok(crate::layer::build_chain(
                info,
                crate::layer::LayerStorage::with_persistent(Arc::clone(backend)),
            )),
            Ok(None) => Err(Status::not_found(format!(
                "layer {} not in store",
                at_layer
            ))),
            // RocksStore::load_chain_from walks the chain via
            // `get_chain` which reports missing entries as
            // StorageError::NotFound. Treat that as "layer not in
            // store" rather than an internal error.
            Err(crate::storage::StorageError::NotFound(_)) => Err(Status::not_found(format!(
                "layer {} not in store",
                at_layer
            ))),
            Err(e) => Err(Status::internal(format!("load_chain_from failed: {e}"))),
        }
    }

    /// Parse resources from CBOR, JSON, or ESL based on content_type.
    ///
    /// For ESL inputs, the kernel's live `InstitutionIndex` is threaded
    /// into the compiler so function-call IRIs can be classified as
    /// Comorphism / Decidable QueryClass / OnDemand QueryClass per
    /// D14 §9.5. Without the index, qualified-name function calls
    /// fall through to plain `Apply(Var, ...)` and the comorphism
    /// dispatch path is silently bypassed at runtime.
    ///
    /// When `branch` is supplied, the branch's current head layer is
    /// also fed into the compiler so cross-layer ctor and macro
    /// references resolve — required for any ESL that invokes a
    /// chain-resident smart-constructor macro (e.g.
    /// `stats:SingleSampleEstimate(...)`) or references ctors
    /// declared in a parent layer's inductive (e.g.
    /// `reasoning:JustifiedBy.app` consumed from a `type_expr(...)`
    /// certificate body). When `branch` is None, falls back to
    /// `compile_with_institutions` (institution-aware, layer-blind).
    #[allow(clippy::result_large_err)]
    pub(super) async fn parse_resources(
        &self,
        data: &[u8],
        content_type: &str,
        branch: Option<&str>,
    ) -> Result<Vec<Resource>, Status> {
        if content_type.contains("cbor") {
            eigon_cbor::parse_document(data)
                .map_err(|e| Status::invalid_argument(format!("CBOR parse error: {e}")))
        } else if content_type.contains("esl") {
            let source = std::str::from_utf8(data)
                .map_err(|e| Status::invalid_argument(format!("invalid UTF-8: {e}")))?;
            let index = Arc::clone(&*self.institution_index.read().await);
            let result = match branch {
                Some(branch_name) => {
                    let ctx_arc = self.get_branch_context(branch_name).await?;
                    let ctx = ctx_arc.read().await;
                    let layer = Arc::clone(ctx.head());
                    drop(ctx);
                    crate::esl::compile_full(source, index, &layer)
                }
                None => crate::esl::compile_with_institutions(source, index),
            };
            result.map_err(|errors| {
                let msgs: Vec<String> = errors.iter().map(|e| format!("{e}")).collect();
                Status::invalid_argument(format!("ESL compile error: {}", msgs.join("; ")))
            })
        } else {
            let json_str = std::str::from_utf8(data)
                .map_err(|e| Status::invalid_argument(format!("invalid UTF-8: {e}")))?;
            eigon_json::parse_document(json_str)
                .map_err(|e| Status::invalid_argument(format!("JSON parse error: {e}")))
        }
    }

    /// Serialize a resource to CBOR bytes.
    pub(super) fn serialize_resource(resource: &Resource) -> Vec<u8> {
        eigon_cbor::serialize_resource(resource)
    }
}

#[allow(clippy::result_large_err)]
#[tonic::async_trait]
impl EigeniusKernel for EigeniusService {
    async fn load(&self, request: Request<LoadRequest>) -> Result<Response<LoadResponse>, Status> {
        self.handle_load(request.into_inner()).await
    }

    async fn inspect(
        &self,
        request: Request<InspectRequest>,
    ) -> Result<Response<InspectResponse>, Status> {
        self.handle_inspect(request.into_inner()).await
    }

    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        self.handle_query(request.into_inner()).await
    }

    async fn validate_program(
        &self,
        request: Request<ValidateProgramRequest>,
    ) -> Result<Response<ValidateProgramResponse>, Status> {
        self.handle_validate_program(request.into_inner()).await
    }

    async fn run_program(
        &self,
        request: Request<RunProgramRequest>,
    ) -> Result<Response<RunProgramResponse>, Status> {
        self.handle_run_program(request.into_inner()).await
    }

    async fn run_program_by_iri(
        &self,
        request: Request<RunProgramByIriRequest>,
    ) -> Result<Response<RunProgramResponse>, Status> {
        self.handle_run_program_by_iri(request.into_inner()).await
    }

    async fn reflect(
        &self,
        request: Request<ReflectRequest>,
    ) -> Result<Response<ReflectResponse>, Status> {
        self.handle_reflect(request.into_inner()).await
    }

    async fn health(
        &self,
        request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        self.handle_health(request.into_inner()).await
    }

    async fn list_institutions(
        &self,
        request: Request<ListInstitutionsRequest>,
    ) -> Result<Response<ListInstitutionsResponse>, Status> {
        self.handle_list_institutions(request.into_inner()).await
    }

    async fn get_schema(
        &self,
        request: Request<GetSchemaRequest>,
    ) -> Result<Response<GetSchemaResponse>, Status> {
        self.handle_get_schema(request.into_inner()).await
    }

    async fn list_tasks(
        &self,
        request: Request<ListTasksRequest>,
    ) -> Result<Response<ListTasksResponse>, Status> {
        self.handle_list_tasks(request.into_inner()).await
    }

    async fn get_task_status(
        &self,
        request: Request<GetTaskStatusRequest>,
    ) -> Result<Response<GetTaskStatusResponse>, Status> {
        self.handle_get_task_status(request.into_inner()).await
    }

    async fn cancel_task(
        &self,
        request: Request<CancelTaskRequest>,
    ) -> Result<Response<CancelTaskResponse>, Status> {
        self.handle_cancel_task(request.into_inner()).await
    }

    async fn layer_topology(
        &self,
        request: Request<LayerTopologyRequest>,
    ) -> Result<Response<LayerTopologyResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LAYER_TOPOLOGY);
        let req = request.into_inner();
        let layer = self.resolve_read_layer(&req.root_layer, "").await?;
        let topo = topology::walk(&layer, req.max_depth, req.include_resources);
        tracing::debug!(
            { field::OPERATION } = operation::RPC_LAYER_TOPOLOGY,
            include_resources = req.include_resources,
            nodes = topo.nodes.len(),
            edges = topo.edges.len(),
            "layer_topology computed"
        );
        Ok(Response::new(topo))
    }

    async fn list_branches(
        &self,
        request: Request<ListBranchesRequest>,
    ) -> Result<Response<ListBranchesResponse>, Status> {
        self.handle_list_branches(request.into_inner()).await
    }

    async fn get_branch(
        &self,
        request: Request<GetBranchRequest>,
    ) -> Result<Response<GetBranchResponse>, Status> {
        self.handle_get_branch(request.into_inner()).await
    }

    async fn create_branch(
        &self,
        request: Request<CreateBranchRequest>,
    ) -> Result<Response<CreateBranchResponse>, Status> {
        self.handle_create_branch(request.into_inner()).await
    }

    async fn delete_branch(
        &self,
        request: Request<DeleteBranchRequest>,
    ) -> Result<Response<DeleteBranchResponse>, Status> {
        self.handle_delete_branch(request.into_inner()).await
    }

    async fn merge_branches(
        &self,
        request: Request<MergeBranchesRequest>,
    ) -> Result<Response<MergeBranchesResponse>, Status> {
        self.handle_merge_branches(request.into_inner()).await
    }

    async fn submit_resolution(
        &self,
        request: Request<SubmitResolutionRequest>,
    ) -> Result<Response<SubmitResolutionResponse>, Status> {
        self.handle_submit_resolution(request.into_inner()).await
    }

    async fn preview_cascade(
        &self,
        request: Request<PreviewCascadeRequest>,
    ) -> Result<Response<PreviewCascadeResponse>, Status> {
        self.handle_preview_cascade(request.into_inner()).await
    }

    async fn prepare_merge(
        &self,
        request: Request<PrepareMergeRequest>,
    ) -> Result<Response<PrepareMergeResponse>, Status> {
        self.handle_prepare_merge(request.into_inner()).await
    }

    async fn preview_merge(
        &self,
        request: Request<PreviewMergeRequest>,
    ) -> Result<Response<PreviewMergeResponse>, Status> {
        self.handle_preview_merge(request.into_inner()).await
    }

    async fn consolidate_chain(
        &self,
        request: Request<ConsolidateChainRequest>,
    ) -> Result<Response<ConsolidateChainResponse>, Status> {
        self.handle_consolidate_chain(request.into_inner()).await
    }

    async fn estimate_consolidation(
        &self,
        request: Request<EstimateConsolidationRequest>,
    ) -> Result<Response<EstimateConsolidationResponse>, Status> {
        self.handle_estimate_consolidation(request.into_inner())
            .await
    }

    async fn create_tag(
        &self,
        request: Request<CreateTagRequest>,
    ) -> Result<Response<CreateTagResponse>, Status> {
        self.handle_create_tag(request.into_inner()).await
    }

    async fn list_tags(
        &self,
        request: Request<ListTagsRequest>,
    ) -> Result<Response<ListTagsResponse>, Status> {
        self.handle_list_tags(request.into_inner()).await
    }

    async fn delete_tag(
        &self,
        request: Request<DeleteTagRequest>,
    ) -> Result<Response<DeleteTagResponse>, Status> {
        self.handle_delete_tag(request.into_inner()).await
    }

    async fn estimate_gc(
        &self,
        request: Request<EstimateGcRequest>,
    ) -> Result<Response<EstimateGcResponse>, Status> {
        self.handle_estimate_gc(request.into_inner()).await
    }

    async fn run_gc(
        &self,
        request: Request<RunGcRequest>,
    ) -> Result<Response<RunGcResponse>, Status> {
        self.handle_run_gc(request.into_inner()).await
    }

    async fn parse_sentence(
        &self,
        request: Request<ParseSentenceRequest>,
    ) -> Result<Response<ParseSentenceResponse>, Status> {
        self.handle_parse_sentence(request.into_inner()).await
    }
}

// `NotebookService` is defined in the proto and generates Rust server
// stubs here, but the kernel does not implement it — the orchestrator
// implements `NotebookService` in TypeScript (D22 §3.2 / §4) and proxies
// to `EigeniusKernel.LayerTopology` above. The Rust stubs exist for
// future symmetry / testability and incur no compile-time obligation.

#[cfg(test)]
mod layer_persister_dispatch_tests {
    //! Phase C of D41 wired `EigeniusService` as the canonical
    //! `LayerPersister` impl. Phase F inlined the persist body into
    //! the trait impl and deleted the inherent
    //! `persist_layer_if_backend` method; this test pins the
    //! trait-dispatch handshake the orchestrator depends on.
    //!
    //! D41 §7 / Phase F.
    use super::*;
    use crate::commit::persister::LayerPersister;

    /// `EigeniusService::new()` (no-backend) is enough to exercise the
    /// no-backend branch of `LayerPersister::persist`:
    /// `branch_advanced = true`, `merge_outcome = None`,
    /// `cache_hit_different_position = false`, `layer_id` =
    /// `layer.id()`. (`branch_advanced = true` because in no-backend
    /// mode `ctx.head` is the session's source of truth — see the
    /// persister's body for the rationale.)
    #[tokio::test]
    async fn dyn_dispatch_on_no_backend_path() {
        let service = EigeniusService::new().expect("bootstrap should succeed");

        // Grab the main-branch head layer. `EigeniusService::new()`
        // seeds `"main"` eagerly (see `BranchContextCache::new`), so
        // the cache hit is guaranteed.
        let head: Arc<crate::layer::Layer> = {
            let cache = service.branch_contexts.contexts.read().await;
            let ctx = cache
                .get(DEFAULT_BRANCH)
                .expect("main branch context seeded at construction");
            let ctx = ctx.read().await;
            ctx.head().clone()
        };

        // Trait dispatch through `&dyn LayerPersister` — proves the
        // impl is object-safe and the vtable resolves to the persist
        // body. The orchestrator holds the persister exactly this way.
        let persister: &dyn LayerPersister = &*service.persister;
        let via_trait = persister
            .persist(DEFAULT_BRANCH, &head)
            .expect("no-backend path never errors");

        // The no-backend signal is `branch_advanced = true` +
        // `merge_outcome = None` + `cache_hit_different_position = false`.
        // `branch_advanced = true` because in no-backend mode `ctx.head`
        // is the session's source of truth and the orchestrator must
        // advance to the freshly-built layer (see the persister's body
        // for the rationale).
        assert_eq!(via_trait.layer_id, *head.id());
        assert!(via_trait.branch_advanced);
        assert!(!via_trait.cache_hit_different_position);
        assert!(via_trait.merge_outcome.is_none());
    }
}
