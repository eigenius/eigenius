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

use crate::bootstrap;
use crate::context::{ExecutionContext, ExecutionMode};
use crate::layer::{build_chain, LayerStorage};
use crate::observability::{field, operation, RpcGuard};
use crate::ontology::{eigon_cbor, eigon_json, Iri, Resource};
use crate::program::component::ComponentRegistry;
use crate::program::expr;
use crate::program::trace::{InMemoryTraceStore, TraceStore};
use crate::query;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("eigenius.v1");
}

pub mod topology;

use proto::eigenius_kernel_server::{EigeniusKernel, EigeniusKernelServer};
use proto::*;

/// Current time in milliseconds since the Unix epoch. Used to stamp
/// `TaskRecord.{created_at, updated_at}`.
fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Convert milliseconds since epoch to ISO 8601 string.
fn millis_to_iso8601(ms: i64) -> String {
    use std::time::Duration;
    let d = Duration::from_millis(ms as u64);
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    let days = secs / 86400;
    let rem = secs % 86400;
    let hours = rem / 3600;
    let minutes = (rem % 3600) / 60;
    let seconds = rem % 60;
    // Simple date calculation from days since epoch
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hours, minutes, seconds, millis
    )
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Civil calendar algorithm from Howard Hinnant
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1461 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Live state of the startup resume sweep (D21 §6). `Health` reads
/// this so clients can tell when resumed tasks have finished draining.
#[derive(Debug, Default)]
pub struct ResumeState {
    /// `true` while the resume sweep is still enqueuing or draining
    /// tasks. Flips to `false` once the sweep's top-level await
    /// completes.
    pub in_progress: std::sync::atomic::AtomicBool,
    /// Count of tasks currently in the resume queue (enqueued but
    /// not yet terminal).
    pub remaining: std::sync::atomic::AtomicU32,
}

/// Dependencies the resume sweep needs. Extracted from `EigeniusService`
/// before the service is consumed by `into_server`.
pub struct ResumeInputs {
    pub task_store: Arc<dyn crate::task::TaskStore>,
    pub backend: Arc<dyn crate::storage::PersistentBackend>,
    pub trace_store: Arc<dyn TraceStore>,
    pub resume_state: Arc<ResumeState>,
}

/// Configuration knobs for the resume sweep (D21 §6, §8).
#[derive(Debug, Clone, Copy)]
pub struct ResumeConfig {
    /// Maximum tasks rehydrated concurrently. Prevents thundering the
    /// orchestrator on a cold restart with many running tasks.
    pub max_parallel: usize,
    /// Upper bound on how many times a task is retried within one
    /// sweep pass. v1 ships with 1 — a task that fails its resume
    /// run transitions straight to `Failed`.
    pub max_attempts: u32,
}

impl Default for ResumeConfig {
    fn default() -> Self {
        Self {
            max_parallel: 4,
            max_attempts: 1,
        }
    }
}

/// Run the startup resume sweep (D21 §6).
///
/// Scans the persistent task store for `Running` / `Suspended` tasks,
/// rehydrates each task's pinned layer chain, and re-executes the
/// program with a fresh `TaskContext`. The evaluator's positional
/// trace cache (D21 §3.2) short-circuits any IO calls that already
/// completed in the pre-crash run, so repeated starts are idempotent
/// modulo the program and input being resolvable.
///
/// Runs as a background task so gRPC listeners are free during the
/// sweep. Callers that want synchronous wait semantics can `.await`
/// the returned `JoinHandle`.
pub async fn resume_sweep(
    inputs: ResumeInputs,
    session_id: uuid::Uuid,
    components: Arc<ComponentRegistry>,
    config: ResumeConfig,
) {
    use std::sync::atomic::Ordering;

    let records = match inputs.task_store.list_tasks(&session_id) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                { field::OPERATION } = operation::TASK_RESUME,
                { field::ERROR_KIND } = "list_tasks_failed",
                { field::SESSION_ID } = ?session_id,
                { field::ERROR_MESSAGE } = %e,
                "resume sweep: list_tasks failed"
            );
            return;
        }
    };
    let mut resumable: Vec<crate::task::TaskRecord> = records
        .into_iter()
        .filter(|r| r.status.is_resumable())
        .collect();
    if resumable.is_empty() {
        return;
    }
    // Oldest first.
    resumable.sort_by_key(|r| r.created_at);

    let total = resumable.len() as u32;
    inputs
        .resume_state
        .in_progress
        .store(true, Ordering::SeqCst);
    inputs.resume_state.remaining.store(total, Ordering::SeqCst);
    tracing::info!(
        { field::OPERATION } = operation::TASK_RESUME,
        { field::COUNT } = total,
        max_parallel = config.max_parallel,
        max_attempts = config.max_attempts,
        "resuming tasks from persistent store"
    );

    let semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_parallel));
    let mut handles = Vec::new();
    for record in resumable {
        let permit_sem = Arc::clone(&semaphore);
        let task_store = Arc::clone(&inputs.task_store);
        let backend = Arc::clone(&inputs.backend);
        let trace_store = Arc::clone(&inputs.trace_store);
        let resume_state = Arc::clone(&inputs.resume_state);
        let components = Arc::clone(&components);
        let max_attempts = config.max_attempts;

        let handle = tokio::spawn(async move {
            let _permit = permit_sem.acquire_owned().await.ok();
            resume_one_task(
                record,
                task_store,
                backend,
                trace_store,
                components,
                max_attempts,
            )
            .await;
            resume_state.remaining.fetch_sub(1, Ordering::SeqCst);
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }
    inputs
        .resume_state
        .in_progress
        .store(false, Ordering::SeqCst);
    tracing::info!(
        { field::OPERATION } = operation::TASK_RESUME,
        "resume sweep complete"
    );
}

/// Rehydrate a single task: resolve program + input in the pinned
/// layer, re-execute with a TaskContext, and update the record
/// based on the outcome.
async fn resume_one_task(
    mut record: crate::task::TaskRecord,
    task_store: Arc<dyn crate::task::TaskStore>,
    backend: Arc<dyn crate::storage::PersistentBackend>,
    trace_store: Arc<dyn TraceStore>,
    components: Arc<ComponentRegistry>,
    _max_attempts: u32,
) {
    use crate::task::TaskStatus;
    // Rehydrate the pinned layer chain from the backend. ChainInfo
    // gives us the metadata; `LayerStorage::with_persistent` wraps the
    // real RocksDB-backed PB so cold-cache reads hit storage on demand.
    let layer = match backend.load_chain_from(&record.layer_head) {
        Ok(Some(info)) => crate::layer::build_chain(
            info,
            crate::layer::LayerStorage::with_persistent(Arc::clone(&backend)),
        ),
        _ => {
            tracing::warn!(
                { field::OPERATION } = operation::TASK_RESUME,
                { field::ERROR_KIND } = "pinned_layer_missing",
                { field::TASK_ID } = ?record.task_id,
                { field::LAYER_ID } = %hex::encode(record.layer_head.0),
                "task pinned layer not in store; marking Failed"
            );
            record.status = TaskStatus::Failed;
            record.updated_at = now_millis();
            let _ = task_store.put_task(&record);
            return;
        }
    };

    // Resolve program and input resources from the pinned layer.
    let program = match Iri::parse(&record.program_iri)
        .ok()
        .and_then(|i| layer.resolve(&i).map(|arc| (*arc).clone()))
    {
        Some(p) => p,
        None => {
            tracing::warn!(
                { field::OPERATION } = operation::TASK_RESUME,
                { field::ERROR_KIND } = "program_missing",
                { field::TASK_ID } = ?record.task_id,
                { field::PROGRAM_IRI } = %record.program_iri,
                "task program not found at pinned head"
            );
            record.status = TaskStatus::Failed;
            record.updated_at = now_millis();
            let _ = task_store.put_task(&record);
            return;
        }
    };
    let input = match Iri::parse(&record.input_iri)
        .ok()
        .and_then(|i| layer.resolve(&i).map(|arc| (*arc).clone()))
    {
        Some(r) => r,
        None => {
            // Input may legitimately have been inline (no IRI in layer);
            // synthesize a minimal resource for now. A richer resume
            // story would persist the input bytes inside the TaskRecord.
            Resource::new_embedded()
        }
    };

    let session_id = record.session_id;
    let task_id = record.task_id;
    let tc = Arc::new(crate::task::TaskContext::new(
        session_id,
        task_id,
        Arc::clone(&task_store),
    ));

    let result = crate::program::eval_io::execute_program_nbe_with_institutions_d14(
        &program,
        &input,
        layer,
        components,
        None,
        None,
        Some(trace_store),
        Some(tc),
    );

    match result {
        Ok(_) => {
            record.status = TaskStatus::Completed;
        }
        Err(e) => {
            tracing::warn!(
                { field::OPERATION } = operation::TASK_RESUME,
                { field::ERROR_KIND } = "execution_failed",
                { field::TASK_ID } = ?task_id,
                { field::ERROR_MESSAGE } = %e,
                "resumed task failed during execution"
            );
            record.status = TaskStatus::Failed;
        }
    }
    record.updated_at = now_millis();
    if let Err(e) = task_store.put_task(&record) {
        tracing::warn!(
            { field::OPERATION } = operation::TASK_RESUME,
            { field::ERROR_KIND } = "task_record_update_failed",
            { field::TASK_ID } = ?task_id,
            { field::ERROR_MESSAGE } = %e,
            "failed to update task record after resume"
        );
    }
}

/// The Eigenius gRPC service implementation.
/// Default branch name for requests that omit `branch`. Phase 14g.
pub const DEFAULT_BRANCH: &str = "main";

/// Resolve a request's branch field (empty → "main").
fn resolve_branch_name(req_branch: &str) -> &str {
    if req_branch.is_empty() {
        DEFAULT_BRANCH
    } else {
        req_branch
    }
}

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
struct BranchContextCache {
    contexts: RwLock<HashMap<String, Arc<RwLock<ExecutionContext>>>>,
}

impl BranchContextCache {
    fn new(main_ctx: ExecutionContext) -> Self {
        let mut map = HashMap::new();
        map.insert(DEFAULT_BRANCH.to_string(), Arc::new(RwLock::new(main_ctx)));
        Self {
            contexts: RwLock::new(map),
        }
    }
}

/// Outcome of [`EigeniusService::persist_layer_if_backend`] — the
/// canonical `LayerId` for the committed content paired with the
/// merge outcome and a derived `branch_advanced` flag.
///
/// **`branch_advanced` semantics** (D33 §6 + D23 §5.4):
///
/// - `true` — the durable branch ref moved as a result of this
///   persist. Holds for cache misses, same-position cache hits, and
///   both `FastForward` / `TrivialMerge` CAS outcomes.
/// - `false` — the branch ref did **not** move. Holds for: no
///   persistent backend, different-position cache hit, and the
///   `NeedsWitnessedMerge` CAS outcome (the layer is stored but
///   unreachable from any branch ref).
///
/// `merge_outcome` is `Some(...)` whenever a CAS attempt actually ran
/// (cache miss or same-position cache hit). It is `None` for the
/// no-backend path and for different-position cache hits — in both
/// cases there is no merge taxonomy because no CAS happened. The
/// proto boundary maps `None` to [`proto::MergeOutcome::Unspecified`].
#[derive(Debug, Clone)]
struct PersistedLayerInfo {
    layer_id: crate::layer::LayerId,
    branch_advanced: bool,
    merge_outcome: Option<crate::lattice::UpdateOutcome>,
    /// `true` iff the persist short-circuited because the
    /// anchored-commit cache (D33 §6) found a content-equivalent layer
    /// at a different chain position. `layer_id` in that case is the
    /// cached layer's id, not the freshly-built one. Distinguished
    /// from the no-backend / no-CAS case (where `merge_outcome` is
    /// also `None` and `branch_advanced` is also `false`) so the
    /// response can carry a `MERGE_OUTCOME_CACHED_DIFFERENT_POSITION`
    /// signal that consumers can render distinctly from "no commit
    /// shape information available".
    cache_hit_different_position: bool,
}

/// Build a wire-format [`proto::MergeInfo`] from an optional
/// [`PersistedLayerInfo`].
///
/// Resolves all post-persist states callers care about into the
/// proto's [`proto::MergeOutcome`] taxonomy:
///
/// - `None`: persist didn't run at all (commit attempted but errored
///   before reaching `persist_layer_if_backend`, e.g. backend I/O
///   error captured as a `ValidationError`). Emit `UNSPECIFIED`.
/// - **Different-position cache hit** (`info.cache_hit_different_position`):
///   `CACHED_DIFFERENT_POSITION` with `merge_layer_id = info.layer_id`
///   (the cached canonical layer's id). Branch did not advance.
/// - **Lattice CAS ran** (`info.merge_outcome = Some(_)`): map by
///   variant — FastForward / TrivialMerge / NeedsWitnessedMerge.
/// - **CAS skipped, no cache hit** (`info.merge_outcome = None`,
///   `!cache_hit_…`): the no-backend path. Emit `UNSPECIFIED`.
fn merge_info_from_persist_info(info: Option<&PersistedLayerInfo>) -> proto::MergeInfo {
    use crate::lattice::UpdateOutcome;
    let Some(info) = info else {
        return proto::MergeInfo {
            outcome: proto::MergeOutcome::Unspecified as i32,
            merge_layer_id: String::new(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
        };
    };
    if info.cache_hit_different_position {
        return proto::MergeInfo {
            outcome: proto::MergeOutcome::CachedDifferentPosition as i32,
            merge_layer_id: info.layer_id.to_string(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
        };
    }
    match info.merge_outcome.as_ref() {
        None => proto::MergeInfo {
            outcome: proto::MergeOutcome::Unspecified as i32,
            merge_layer_id: String::new(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
        },
        Some(UpdateOutcome::FastForward) => proto::MergeInfo {
            outcome: proto::MergeOutcome::FastForward as i32,
            merge_layer_id: String::new(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
        },
        Some(UpdateOutcome::TrivialMerge { merge_layer }) => proto::MergeInfo {
            outcome: proto::MergeOutcome::TrivialMerge as i32,
            merge_layer_id: merge_layer.to_string(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
        },
        Some(UpdateOutcome::NeedsWitnessedMerge {
            current_head,
            conflicting_iris,
        }) => proto::MergeInfo {
            outcome: proto::MergeOutcome::NeedsWitnessedMerge as i32,
            merge_layer_id: String::new(),
            conflicting_iris: conflicting_iris
                .iter()
                .map(|iri| iri.as_str().to_string())
                .collect(),
            current_head: current_head.to_string(),
        },
    }
}

pub struct EigeniusService {
    /// Per-branch ExecutionContext cache. `"main"` is always present.
    branch_contexts: Arc<BranchContextCache>,
    /// Outer lock allows swapping the registry (for WASM registration on load).
    /// Inner Arc allows cheap cloning for passing to the evaluator.
    components: Arc<RwLock<Arc<ComponentRegistry>>>,
    trace_store: Arc<dyn TraceStore>,
    /// D14 institution index — derived view of the layer chain rebuilt
    /// after every commit. Outer lock allows swapping; inner Arc lets
    /// the evaluator clone cheaply when constructing `EvalCtx::IO`.
    institution_index: Arc<RwLock<Arc<crate::institution::registry::InstitutionIndex>>>,
    /// D14 institution runtime — `Box<dyn Institution>` per
    /// institution IRI. Populated when D14-shaped WASM institutions
    /// are installed (B3+ wiring); otherwise empty.
    institution_runtime: Arc<RwLock<Arc<crate::institution::runtime::InstitutionRuntime>>>,
    /// Optional persistent backend. When present, committed layers,
    /// the seed manifest, and trace state all live here; absent means
    /// the server is in-memory-only (the pre-Phase-9a behaviour).
    /// See D13.
    backend: Option<Arc<dyn crate::storage::PersistentBackend>>,
    /// Persistent task store (D21 §3.1). `Some` whenever a backend
    /// is attached — every `RunProgram` allocates a task record so
    /// trace lookups can route through per-task positional keys and
    /// a mid-flight crash leaves a recoverable `Running` task for
    /// the resume sweep to pick up.
    task_store: Option<Arc<dyn crate::task::TaskStore>>,
    /// Single hardwired session (D21 §3.7). Tracks the session's
    /// active_top; advances on every successful Load and on
    /// fast-forward task completion. In 9b-iii there is exactly one
    /// of these per running kernel.
    session: Arc<RwLock<crate::task::Session>>,
    /// Live state of the startup resume sweep (D21 §6). Shared with
    /// the background sweep task so `Health` can report progress.
    resume_state: Arc<ResumeState>,
    /// Optional gRPC client for the orchestrator. Used to forward IO-capability
    /// WASM components and to dispatch remote IO components during program
    /// execution. None means no orchestrator is configured — IO WASM installs
    /// will be rejected with a clear error in that case.
    orchestrator_client: Option<
        Arc<
            tokio::sync::Mutex<
                proto::component_executor_client::ComponentExecutorClient<
                    tonic::transport::Channel,
                >,
            >,
        >,
    >,
}

impl EigeniusService {
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
            backend: None,
            task_store: None,
            session: Arc::new(RwLock::new(crate::task::Session::hardwired())),
            resume_state: Arc::new(ResumeState::default()),
            orchestrator_client: None,
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
            backend: Some(backend),
            task_store: Some(task_store),
            session: Arc::new(RwLock::new(crate::task::Session::hardwired())),
            resume_state: Arc::new(ResumeState::default()),
            orchestrator_client: None,
        })
    }

    /// Attach an orchestrator client so IO-capability WASM components can be
    /// forwarded to the orchestrator and remote components dispatched back.
    pub fn with_orchestrator_client(
        mut self,
        client: Arc<
            tokio::sync::Mutex<
                proto::component_executor_client::ComponentExecutorClient<
                    tonic::transport::Channel,
                >,
            >,
        >,
    ) -> Self {
        self.orchestrator_client = Some(client);
        self
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
            backend: None,
            task_store: None,
            session: Arc::new(RwLock::new(crate::task::Session::hardwired())),
            resume_state: Arc::new(ResumeState::default()),
            orchestrator_client: None,
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
    async fn get_branch_context(
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

    /// Persist a freshly-committed layer through the backend, if one is
    /// attached. No-op otherwise. See D13 §5.
    ///
    /// Returns a validation-like error on storage failure so the caller
    /// can surface it to clients without crashing the server.
    /// Resolve the target layer for a read RPC (D21 §3.6 `at_layer`).
    ///
    /// Empty / invalid hex falls back to the named branch's head (or
    /// `"main"` if `branch` is also empty). When `at_layer` is set and
    /// a backend is attached, reconstructs the layer chain rooted at
    /// that id. `at_layer` and `branch` are mutually exclusive — if
    /// both are set, returns `Status::invalid_argument`.
    async fn resolve_read_layer(
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
            let branch_name = resolve_branch_name(branch);
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

    /// Persist a built `Layer` and advance `branch` to it — or
    /// short-circuit through the anchored-commit cache (D33 §6).
    ///
    /// Returns a [`PersistedLayerInfo`] carrying the **canonical**
    /// `LayerId` for the committed content together with a
    /// `branch_advanced` flag that signals whether this call moved
    /// the branch ref:
    ///
    /// - **Cache miss:** the fresh layer is stored, the branch advances
    ///   to it, the cache is updated. `layer_id` = fresh id,
    ///   `branch_advanced` = `true`.
    /// - **Cache hit at the same position** (cached id == fresh id):
    ///   `store_layer` is skipped (the layer is already on disk); the
    ///   branch still advances (the caller meant to publish on top of
    ///   the current head, and the cached layer occupies that
    ///   position). `layer_id` = fresh id, `branch_advanced` = `true`.
    /// - **Cache hit at a different position:** the content lives
    ///   canonically at the cached layer's id, which is in a different
    ///   chain context. `store_layer` and `update_branch` are both
    ///   skipped — the branch stays put. `layer_id` = cached id,
    ///   `branch_advanced` = `false`.
    ///
    /// The third case is the structural payoff: callers report
    /// "your content is already canonical at X" while the branch
    /// tracker sees no movement. Notebook cell-output reuse, mirror
    /// regeneration against shifted parent chains, and any content
    /// generator anchored to a supporting layer benefit transparently.
    ///
    /// When no persistent backend is configured, returns the fresh
    /// layer's id with `branch_advanced` = `false` (the in-memory
    /// chain may have changed, but there is no durable branch ref to
    /// move).
    fn persist_layer_if_backend(
        &self,
        branch: &str,
        layer: &crate::layer::Layer,
    ) -> Result<PersistedLayerInfo, ValidationError> {
        let Some(backend) = self.backend.as_ref() else {
            // No persistent backend — the layer lives in-memory only.
            // There is no durable branch ref to advance, and no CAS
            // attempted (merge_outcome = None).
            return Ok(PersistedLayerInfo {
                layer_id: layer.id().clone(),
                branch_advanced: false,
                merge_outcome: None,
                cache_hit_different_position: false,
            });
        };

        // Cache probe. The cache key is the layer's content_hash and
        // the supporting layer's content_hash. Layers with no
        // supporting layer (roots, pure self-referential commits) can't
        // be keyed and fall through to the standard persist path.
        let cache_hit = self.probe_anchored_commit(backend.as_ref(), layer);

        if let Some(cached_id) = cache_hit {
            if cached_id == *layer.id() {
                // Same-position cache hit — the layer is already on
                // disk. Skip `store_layer`; still attempt the branch
                // CAS (the caller wanted to publish on top of the
                // current head, which is the layer's parent). The CAS
                // may still race or conflict, so the outcome is the
                // full taxonomy.
                tracing::debug!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::LAYER_ID } = %layer.id(),
                    branch = branch,
                    cache = "hit_same_position",
                    "anchored-commit cache hit (same position) — skipping store_layer"
                );
                let outcome = self.advance_branch_for_layer(branch, layer, backend.as_ref())?;
                let branch_advanced = !matches!(
                    outcome,
                    crate::lattice::UpdateOutcome::NeedsWitnessedMerge { .. }
                );
                return Ok(PersistedLayerInfo {
                    layer_id: layer.id().clone(),
                    branch_advanced,
                    merge_outcome: Some(outcome),
                    cache_hit_different_position: false,
                });
            }
            // Different-position cache hit — the canonical layer is
            // elsewhere. Skip both `store_layer` and `update_branch`;
            // the branch stays where it is (D33 §6 supporting-
            // equivalent context). No CAS attempted, so merge_outcome
            // is None.
            tracing::debug!(
                { field::OPERATION } = operation::LAYER_COMMIT,
                { field::LAYER_ID } = %layer.id(),
                cached_layer = %cached_id,
                branch = branch,
                cache = "hit_different_position",
                "anchored-commit cache hit (different position) — branch unchanged"
            );
            return Ok(PersistedLayerInfo {
                layer_id: cached_id,
                branch_advanced: false,
                merge_outcome: None,
                cache_hit_different_position: true,
            });
        }

        // Cache miss — standard persist path.
        if let Err(e) = backend.store_layer(layer) {
            tracing::warn!(
                { field::OPERATION } = operation::LAYER_COMMIT,
                { field::ERROR_KIND } = "persist_layer_failed",
                { field::LAYER_ID } = %layer.id(),
                { field::ERROR_MESSAGE } = %e,
                "failed to persist layer to backend"
            );
            return Err(ValidationError {
                resource_iri: String::new(),
                property_iri: String::new(),
                rule: "persist_layer".to_string(),
                message: format!("{e}"),
                severity: "error".to_string(),
            });
        }

        // Insert into the anchored-commit cache for future short-circuit
        // (D33 §6). Best-effort: a failure here doesn't fail the
        // commit, but we log it so chain audits can spot drift between
        // the cache and the topology.
        self.put_anchored_commit_for_layer(backend.as_ref(), layer);

        // Attempt the CAS. On `NeedsWitnessedMerge` the layer is on
        // disk but not reachable from any branch ref — the fix for
        // D34 §G.1's silent-success bug is reporting branch_advanced
        // = false here so clients know to recover.
        let outcome = self.advance_branch_for_layer(branch, layer, backend.as_ref())?;
        let branch_advanced = !matches!(
            outcome,
            crate::lattice::UpdateOutcome::NeedsWitnessedMerge { .. }
        );
        Ok(PersistedLayerInfo {
            layer_id: layer.id().clone(),
            branch_advanced,
            merge_outcome: Some(outcome),
            cache_hit_different_position: false,
        })
    }

    /// Compute the anchored-commit cache key for `layer` and probe the
    /// backend. Returns `None` when the layer has no supporting layer
    /// (root / self-referential) or when no cache entry exists.
    /// Verifies the cached layer is still in storage before returning
    /// — a stale entry (cached layer was reclaimed by GC) is treated
    /// as a cache miss.
    fn probe_anchored_commit(
        &self,
        backend: &dyn crate::storage::PersistentBackend,
        layer: &crate::layer::Layer,
    ) -> Option<crate::layer::LayerId> {
        let supporting_id = layer.supporting_layer()?;
        let supporting_handle = backend.load_handle(supporting_id).ok().flatten()?;
        let cached_id = backend
            .lookup_anchored_commit(layer.content_hash(), &supporting_handle.content_hash)
            .ok()
            .flatten()?;
        // Verify the cached layer still exists. If GC has reclaimed
        // it (or it was never persisted for some reason), treat as a
        // miss so the caller re-persists.
        backend.load_handle(&cached_id).ok().flatten()?;
        Some(cached_id)
    }

    /// Insert the freshly-committed layer into the anchored-commit
    /// cache. Best-effort — failures log a warning but don't propagate.
    fn put_anchored_commit_for_layer(
        &self,
        backend: &dyn crate::storage::PersistentBackend,
        layer: &crate::layer::Layer,
    ) {
        let Some(supporting_id) = layer.supporting_layer() else {
            return;
        };
        let Some(supporting_handle) = backend.load_handle(supporting_id).ok().flatten() else {
            return;
        };
        if let Err(e) = backend.put_anchored_commit(
            layer.content_hash(),
            &supporting_handle.content_hash,
            layer.id(),
        ) {
            tracing::warn!(
                { field::OPERATION } = operation::LAYER_COMMIT,
                { field::ERROR_KIND } = "anchored_commit_cache_put_failed",
                { field::LAYER_ID } = %layer.id(),
                { field::ERROR_MESSAGE } = %e,
                "failed to update anchored-commit cache (commit succeeded)"
            );
        }
    }

    /// Advance `branch` to `layer` via the lattice's CAS primitive.
    /// Carved out of `persist_layer_if_backend` so both the
    /// cache-miss path and the same-position cache-hit path can
    /// share the logic.
    ///
    /// Returns the lattice's [`UpdateOutcome`](crate::lattice::UpdateOutcome)
    /// verbatim so the caller can:
    ///
    /// - distinguish `FastForward` (clean CAS) from `TrivialMerge`
    ///   (concurrent disjoint-IRI contributions; kernel produced a
    ///   merge layer) from `NeedsWitnessedMerge` (concurrent
    ///   conflicting contributions; branch unchanged);
    /// - correctly compute `branch_advanced` — in particular,
    ///   `NeedsWitnessedMerge` means the branch did **not** advance
    ///   (the layer is stored but unreachable from any branch ref).
    ///
    /// Pre-D34 §G.1 this method swallowed all `Ok` variants as
    /// `Ok(())`, masking the `NeedsWitnessedMerge` failure as success.
    fn advance_branch_for_layer(
        &self,
        branch: &str,
        layer: &crate::layer::Layer,
        backend: &dyn crate::storage::PersistentBackend,
    ) -> Result<crate::lattice::UpdateOutcome, ValidationError> {
        let expected_old = layer.parent().map(|p| p.id().clone());
        let storage = LayerStorage::with_persistent(
            self.backend
                .as_ref()
                .expect("advance_branch_for_layer called only when backend is Some")
                .clone(),
        );
        match crate::lattice::update_branch(
            branch,
            expected_old,
            layer.id().clone(),
            crate::lattice::ConflictPolicy::AllowTrivial,
            storage,
            backend,
        ) {
            Ok(outcome) => {
                tracing::debug!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::LAYER_ID } = %layer.id(),
                    branch = branch,
                    outcome = ?outcome,
                    "branch CAS attempted"
                );
                Ok(outcome)
            }
            Err(e) => {
                tracing::warn!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::ERROR_KIND } = "branch_update_failed",
                    { field::LAYER_ID } = %layer.id(),
                    branch = branch,
                    { field::ERROR_MESSAGE } = %e,
                    "failed to advance branch"
                );
                Err(ValidationError {
                    resource_iri: String::new(),
                    property_iri: String::new(),
                    rule: "advance_branch".to_string(),
                    message: format!("{e}"),
                    severity: "error".to_string(),
                })
            }
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
    #[allow(clippy::result_large_err)]
    async fn parse_resources(
        &self,
        data: &[u8],
        content_type: &str,
    ) -> Result<Vec<Resource>, Status> {
        if content_type.contains("cbor") {
            eigon_cbor::parse_document(data)
                .map_err(|e| Status::invalid_argument(format!("CBOR parse error: {e}")))
        } else if content_type.contains("esl") {
            let source = std::str::from_utf8(data)
                .map_err(|e| Status::invalid_argument(format!("invalid UTF-8: {e}")))?;
            let index = Arc::clone(&*self.institution_index.read().await);
            crate::esl::compile_with_institutions(source, index).map_err(|errors| {
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
    fn serialize_resource(resource: &Resource) -> Vec<u8> {
        eigon_cbor::serialize_resource(resource)
    }

    /// Scan a layer for WASM components/institutions and register them.
    ///
    /// Errors encountered during registration are added to `errors` as
    /// validation errors (but do not fail the load — a malformed WASM
    /// resource is reported but other resources in the same layer still
    /// load successfully).
    ///
    /// Returns the set of declared class/property resources the
    /// institution registration produced. The caller is expected to
    /// commit these to a follow-up layer so they become queryable.
    /// RESUME callers should instead use
    /// [`Self::rehydrate_wasm_from_layer`] which skips publishing.
    /// Rebuild the D14 [`InstitutionIndex`] from the given layer
    /// (which is the new head of the chain). Called after every
    /// successful commit + after Phase 9a rehydration.
    ///
    /// Walks the entire chain from the supplied layer downward; any
    /// per-resource parse errors are logged at warn-level and skipped
    /// (the well-formed entries still index — same shape as the
    /// existing capability-scan flow).
    ///
    /// Also rebuilds the [`InstitutionRuntime`] by scanning the chain
    /// for Institution declarations whose `runtime` is
    /// `urn:eigenius:institution:runtimes:wasm` and constructing a
    /// [`WasmInstitution`] for each. In-process / external runtime
    /// declarations are skipped — those callers register
    /// programmatically via the runtime API. This closes the
    /// "ontology-first" loop for WASM institutions: declaring an
    /// Institution + `wasm_binary` in the chain auto-installs its
    /// dispatcher on commit.
    ///
    /// [`InstitutionRuntime`]: crate::institution::runtime::InstitutionRuntime
    /// [`WasmInstitution`]: crate::capability::wasm_institution_d14::WasmInstitution
    async fn rebuild_institution_index(&self, layer: &crate::layer::Layer) {
        let (idx, errors) = crate::institution::registry::InstitutionIndex::from_layer(layer);
        for err in &errors {
            tracing::warn!(
                { field::OPERATION } = operation::INSTITUTION_REGISTER,
                kind = err.kind,
                resource_iri = err
                    .resource_iri
                    .as_ref()
                    .map(|i| i.as_str())
                    .unwrap_or(""),
                { field::ERROR_MESSAGE } = %err.reason,
                "institution-index parse error"
            );
        }
        let idx_arc = Arc::new(idx);
        *self.institution_index.write().await = Arc::clone(&idx_arc);

        // Rebuild the runtime from chain-declared WASM institutions,
        // then layer in any external-runtime institutions (D31 §5).
        let (mut runtime, mut report) =
            crate::capability::registration::build_wasm_institution_runtime(layer);
        if let Some(client) = self.orchestrator_client.as_ref() {
            crate::capability::registration::register_external_institutions(
                layer,
                idx_arc.as_ref(),
                &mut runtime,
                Arc::clone(client),
                &mut report,
            );
        } else {
            // No orchestrator wired — external institutions cannot
            // dispatch. Surface this once per rebuild rather than per
            // institution so the operator sees it.
            let has_external = idx_arc.institutions().any(|e| {
                matches!(
                    e.runtime,
                    Some(crate::institution::registry::RuntimeKind::External)
                )
            });
            if has_external {
                tracing::warn!(
                    { field::OPERATION } = operation::INSTITUTION_REGISTER,
                    "chain declares `runtime: external` institutions but the kernel was started \
                     without --orchestrator; their dispatch will fail"
                );
            }
        }
        for err in &report.errors {
            tracing::warn!(
                { field::OPERATION } = operation::INSTITUTION_REGISTER,
                resource_iri = %err.resource_iri,
                { field::ERROR_MESSAGE } = %err.message,
                "institution registration error"
            );
        }
        for inst_iri in &report.institutions_registered {
            tracing::info!(
                { field::OPERATION } = operation::INSTITUTION_REGISTER,
                { field::INSTITUTION_IRI } = %inst_iri,
                host = "kernel",
                "registered institution"
            );
        }
        *self.institution_runtime.write().await = Arc::new(runtime);
    }

    /// Walk a newly committed layer and register every WASM component
    /// (kernel-hosted or IO-class) declared therein. WASM-institution
    /// registration is **no longer** performed here — D14 institutions
    /// register through the chain via the [`InstitutionIndex`] +
    /// [`InstitutionRuntime`] populated by [`Self::rebuild_institution_index`].
    async fn register_wasm_from_layer(
        &self,
        layer: &crate::layer::Layer,
        errors: &mut Vec<ValidationError>,
    ) -> Vec<Resource> {
        // Build a new ComponentRegistry layered on top of the current one.
        let mut new_registry = {
            let current = self.components.read().await;
            ComponentRegistry::new_with_parent(Arc::clone(&current))
        };

        let scan_result =
            crate::capability::registration::scan_and_register(layer, &mut new_registry);

        for e in &scan_result.report.errors {
            errors.push(ValidationError {
                resource_iri: e.resource_iri.clone(),
                property_iri: String::new(),
                rule: "wasm_registration".to_string(),
                message: e.message.clone(),
                severity: "error".to_string(),
            });
        }
        for w in &scan_result.report.warnings {
            tracing::warn!(
                { field::OPERATION } = operation::CAPABILITY_INSTALL,
                "wasm scan warning: {}",
                w
            );
        }

        // Forward IO WASM components to the orchestrator and register a
        // RemoteComponent locally so the kernel can dispatch to them.
        let mut any_kernel_component_added = !scan_result.report.components_registered.is_empty()
            && scan_result.pending_io_components.is_empty();
        for pending in scan_result.pending_io_components {
            match self.register_io_wasm(&pending).await {
                Ok(remote) => {
                    tracing::info!(
                        { field::OPERATION } = operation::CAPABILITY_INSTALL,
                        { field::COMPONENT_IRI } = %pending.resource_iri,
                        host = "orchestrator",
                        "registered IO WASM component"
                    );
                    new_registry.register(pending.resource_iri.clone(), remote);
                    any_kernel_component_added = true;
                }
                Err(e) => {
                    errors.push(ValidationError {
                        resource_iri: pending.resource_iri,
                        property_iri: String::new(),
                        rule: "wasm_registration".to_string(),
                        message: e,
                        severity: "error".to_string(),
                    });
                }
            }
        }

        for iri in &scan_result.report.components_registered {
            tracing::info!(
                { field::OPERATION } = operation::CAPABILITY_INSTALL,
                { field::COMPONENT_IRI } = %iri,
                host = "kernel",
                "registered WASM component"
            );
        }

        if any_kernel_component_added {
            let mut guard = self.components.write().await;
            *guard = Arc::new(new_registry);
        }

        // No institution-published resources under D14 — declarations
        // ride into the chain as ordinary Eigon resources. Returns an
        // empty Vec for source-compatibility with the Load handler's
        // follow-up-commit logic (which is now a no-op).
        Vec::new()
    }

    /// RESUME counterpart of [`Self::register_wasm_from_layer`]. Walks a
    /// rehydrated layer and re-registers every WASM component it
    /// finds. IO components are forwarded to the orchestrator again
    /// (same semantics as fresh install; the orchestrator may reject
    /// if it already has the component). WASM institutions register
    /// via D14 (chain scan + InstitutionRuntime) — no per-layer
    /// rehydration call here.
    async fn rehydrate_wasm_from_layer(
        &self,
        layer: &crate::layer::Layer,
        errors: &mut Vec<ValidationError>,
    ) {
        let mut new_registry = {
            let current = self.components.read().await;
            ComponentRegistry::new_with_parent(Arc::clone(&current))
        };

        let scan_result =
            crate::capability::registration::scan_and_register(layer, &mut new_registry);

        for e in &scan_result.report.errors {
            errors.push(ValidationError {
                resource_iri: e.resource_iri.clone(),
                property_iri: String::new(),
                rule: "wasm_rehydrate".to_string(),
                message: e.message.clone(),
                severity: "error".to_string(),
            });
        }

        let mut any_kernel_component_added = !scan_result.report.components_registered.is_empty()
            && scan_result.pending_io_components.is_empty();
        for pending in scan_result.pending_io_components {
            match self.register_io_wasm(&pending).await {
                Ok(remote) => {
                    tracing::info!(
                        { field::OPERATION } = operation::CAPABILITY_INSTALL,
                        { field::COMPONENT_IRI } = %pending.resource_iri,
                        host = "orchestrator",
                        rehydrated = true,
                        "rehydrated IO WASM component"
                    );
                    new_registry.register(pending.resource_iri.clone(), remote);
                    any_kernel_component_added = true;
                }
                Err(e) => {
                    errors.push(ValidationError {
                        resource_iri: pending.resource_iri,
                        property_iri: String::new(),
                        rule: "wasm_rehydrate".to_string(),
                        message: e,
                        severity: "error".to_string(),
                    });
                }
            }
        }

        if any_kernel_component_added {
            let mut guard = self.components.write().await;
            *guard = Arc::new(new_registry);
        }
    }

    /// Walk the persisted chain from root to head and rehydrate every
    /// WASM capability resource found in each layer. Called once by the
    /// server at startup when a persistent backend is attached.
    pub async fn rehydrate_wasm_from_chain(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let head = match self.get_branch_context(DEFAULT_BRANCH).await {
            Ok(ctx_arc) => {
                let ctx = ctx_arc.read().await;
                Arc::clone(ctx.head())
            }
            Err(status) => {
                errors.push(ValidationError {
                    resource_iri: String::new(),
                    property_iri: String::new(),
                    rule: "rehydrate".to_string(),
                    message: format!("get main context: {status}"),
                    severity: "error".to_string(),
                });
                return errors;
            }
        };
        // Collect root-to-head order so earlier layers register first.
        let mut chain: Vec<Arc<crate::layer::Layer>> = Vec::new();
        let mut cursor = Some(head);
        while let Some(layer) = cursor {
            let parent = layer.parent().cloned();
            chain.push(layer);
            cursor = parent;
        }
        chain.reverse();

        for layer in &chain {
            self.rehydrate_wasm_from_layer(layer, &mut errors).await;
        }
        errors
    }

    /// Forward an IO WASM component to the orchestrator and produce a
    /// local `RemoteComponent` wrapper that dispatches `Execute` calls
    /// back to the orchestrator.
    async fn register_io_wasm(
        &self,
        pending: &crate::capability::registration::PendingIoComponent,
    ) -> Result<Box<dyn crate::program::component::BuiltinComponent>, String> {
        let client = self.orchestrator_client.as_ref().ok_or_else(|| {
            "IO WASM components require an orchestrator to be configured \
                 (pass --orchestrator to `serve`)"
                .to_string()
        })?;

        let request = proto::RegisterWasmComponentRequest {
            component_iri: pending.resource_iri.clone(),
            wasm_binary: pending.wasm_binary.clone(),
            fuel_limit: pending.fuel_limit,
            memory_limit_pages: pending.memory_limit_pages as u64,
        };

        let response = {
            let mut c = client.lock().await;
            c.register_wasm_component(tonic::Request::new(request))
                .await
                .map_err(|e| format!("RegisterWasmComponent gRPC call failed: {e}"))?
        };
        let resp = response.into_inner();
        if !resp.success {
            return Err(format!(
                "orchestrator rejected WASM registration: {}",
                resp.error
            ));
        }

        // Build a local RemoteComponent that forwards Execute calls.
        Ok(Box::new(crate::program::remote::RemoteComponent::new(
            pending.resource_iri.clone(),
            Arc::clone(client),
        )))
    }

    /// Shared execution path for `RunProgram` and `RunProgramByIri`.
    ///
    /// Both RPCs end up here once they have a resolved program +
    /// input Resource. This method handles task allocation (D21 §3.1),
    /// NbE evaluation in IO mode, ProgramTrace assembly, derived-output
    /// stamping (D6b §6), and trace-layer commit.
    async fn execute_program(
        &self,
        branch: &str,
        program: Resource,
        input: Resource,
    ) -> Result<Response<RunProgramResponse>, Status> {
        // Resolve the per-branch ExecutionContext up front. Same Arc is
        // used for the layer-head snapshot below (task pin), the eval
        // step (read), and the trace-layer commit (write).
        let ctx_arc = self.get_branch_context(branch).await?;

        // D21 §3.1: allocate a task for this invocation. When a task
        // store is attached (persistent backend), the record is
        // persisted on entry and again on completion so a mid-flight
        // crash leaves a recoverable `Running` record for the resume
        // sweep. The evaluator routes IO dispatches through a
        // TaskContext so repeated calls with the same input each
        // occupy their own step_seq slot (D21 §3.2).
        let (task_context, task_id_str, layer_head, session_id) = match &self.task_store {
            Some(store) => {
                let session_id = self.session.read().await.session_id;
                let task_id = uuid::Uuid::new_v4();
                let layer_head = {
                    let ctx = ctx_arc.read().await;
                    ctx.head().id().clone()
                };
                let program_iri = program
                    .id()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default();
                let input_iri = input
                    .id()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default();
                let record = crate::task::TaskRecord::new_running(
                    session_id,
                    task_id,
                    program_iri,
                    input_iri,
                    layer_head.clone(),
                    now_millis(),
                );
                if let Err(e) = store.put_task(&record) {
                    return Err(Status::internal(format!("failed to persist task: {e}")));
                }
                let tc = Arc::new(crate::task::TaskContext::new(
                    session_id,
                    task_id,
                    Arc::clone(store),
                ));
                (Some(tc), task_id.to_string(), Some(layer_head), session_id)
            }
            None => (None, String::new(), None, uuid::Uuid::nil()),
        };

        // Execute via NbE in IO mode
        let started_at_ms = now_millis();
        let exec_result = {
            let ctx = ctx_arc.read().await;
            let components = Arc::clone(&*self.components.read().await);
            let index = Arc::clone(&*self.institution_index.read().await);
            let runtime = Arc::clone(&*self.institution_runtime.read().await);
            match crate::program::eval_io::execute_program_nbe_with_institutions_d14(
                &program,
                &input,
                Arc::clone(ctx.head()),
                components,
                Some(index),
                Some(runtime),
                Some(Arc::clone(&self.trace_store)),
                task_context.clone(),
            ) {
                Ok(result) => result,
                Err(e) => {
                    // Record the failure if we have a task store.
                    if let (Some(store), Some(head)) = (&self.task_store, layer_head.as_ref()) {
                        if let Some(tid) = task_context.as_ref().map(|tc| tc.task_id) {
                            let mut rec = crate::task::TaskRecord::new_running(
                                session_id,
                                tid,
                                String::new(),
                                String::new(),
                                head.clone(),
                                now_millis(),
                            );
                            rec.status = crate::task::TaskStatus::Failed;
                            rec.updated_at = now_millis();
                            let _ = store.put_task(&rec);
                        }
                    }
                    // Eval errored before the commit attempt — no CAS
                    // happened, so `merge` stays None. (Sending an
                    // `UNSPECIFIED` MergeInfo here would render as a
                    // misleading `cached` badge in notebook UIs.)
                    return Ok(Response::new(RunProgramResponse {
                        success: false,
                        output: Vec::new(),
                        errors: vec![ValidationError {
                            resource_iri: String::new(),
                            property_iri: String::new(),
                            rule: "execution".to_string(),
                            message: format!("{e}"),
                            severity: "error".to_string(),
                        }],
                        trace_iri: String::new(),
                        task_id: task_id_str.clone(),
                        output_resource_iris: Vec::new(),
                        branch_advanced: false,
                        merge: None,
                    }));
                }
            }
        };

        let completed_at_ms = now_millis();
        let mut output = exec_result.output;
        let dispatched_traces = exec_result.dispatched_traces;
        let produced_resources = exec_result.produced_resources;
        let root_trace = exec_result.root_trace;

        // Compute metrics from the tree-structured trace (preferred) or
        // flat dispatched_traces list (fallback).
        let metrics = crate::program::trace::ProgramMetrics::from_trace(&root_trace);
        let total_tokens = metrics.total_tokens;
        let executed_steps = metrics.executed_steps;

        // Build ProgramTrace with all required fields (D6b §2)
        let trace_iri_str = format!("urn:eigenius:trace:exec-{}", uuid::Uuid::new_v4());

        // Attach DerivedResource epistemic stamp to the output (D6b §6, Phase 10b Step 4)
        {
            use crate::ontology::well_known as wk;
            let is_a_iri = Iri::parse("urn:eigenius:core:is_a").unwrap();
            let mut types = match output.get(&is_a_iri) {
                Some(crate::ontology::resource::Value::Array(arr)) => arr.clone(),
                _ => Vec::new(),
            };
            types.push(crate::ontology::resource::Value::String(
                wk::DERIVED_RESOURCE.to_string(),
            ));
            output.set(is_a_iri, crate::ontology::resource::Value::Array(types));
            output.set(
                Iri::parse(wk::DERIVATION).unwrap(),
                crate::ontology::resource::Value::String(trace_iri_str.clone()),
            );
            output.set(
                Iri::parse(wk::EPISTEMIC_STATUS).unwrap(),
                crate::ontology::resource::Value::String(wk::EPISTEMIC_DERIVED.to_string()),
            );
        }

        let mut trace_resource = Resource::new(Iri::parse(&trace_iri_str).unwrap());
        trace_resource.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    "urn:eigenius:reflection:ProgramTrace".to_string(),
                ),
            ]),
        );
        if let Some(prog_id) = program.id() {
            trace_resource.set(
                Iri::parse("urn:eigenius:reflection:program").unwrap(),
                crate::ontology::resource::Value::String(prog_id.as_str().to_string()),
            );
        }
        // Required: trace_tree — serialized tree-structured trace
        if let Some(ref trace) = root_trace {
            let trace_tree = crate::program::trace::trace_to_resource(trace);
            trace_resource.set(
                Iri::parse("urn:eigenius:reflection:trace_tree").unwrap(),
                crate::ontology::resource::Value::Embedded(Box::new(trace_tree)),
            );
        }
        // Required: started_at, completed_at (ISO 8601)
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:started_at").unwrap(),
            crate::ontology::resource::Value::String(millis_to_iso8601(started_at_ms)),
        );
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:completed_at").unwrap(),
            crate::ontology::resource::Value::String(millis_to_iso8601(completed_at_ms)),
        );
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:total_tokens").unwrap(),
            crate::ontology::resource::Value::Integer(total_tokens),
        );
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:executed_steps").unwrap(),
            crate::ontology::resource::Value::Integer(executed_steps),
        );
        // Recommended: universe_level = 0 (traces about domain resources)
        trace_resource.set(
            Iri::parse(crate::ontology::well_known::UNIVERSE_LEVEL).unwrap(),
            crate::ontology::resource::Value::Integer(0),
        );

        // Auto-commit program-run layer: produced domain resources
        // (comorphism reify outputs, program-final output) +
        // ProgramTrace + all IO ComponentTraces. The commit goes
        // through `commit_with_validation` so AutoOnLoad QueryClasses
        // bound to the produced resources' classes fire on chain
        // entry (D14 §9.3 step 5).
        let output_resource_iris: Vec<String> = produced_resources
            .iter()
            .filter_map(|r| r.id().map(|i| i.as_str().to_string()))
            .collect();
        // `branch_advanced` reports whether the durable branch ref
        // moved as a result of this run's commits. We OR the outcomes
        // of the user-layer and provenance-layer persists: a fresh
        // commit or same-position cache hit advances the branch;
        // a different-position cache hit (D33 §6) does not.
        //
        // `merge_outcome` carries the user-layer's CAS outcome only —
        // surfacing both user-layer and provenance outcomes would
        // conflate two distinct events. See proto comment on
        // `RunProgramResponse.merge` for the rationale.
        // `branch_advanced` reports whether the durable branch ref moved
        // as a result of this run's commits. We OR the user-layer and
        // provenance-layer persists: a fresh commit or same-position
        // cache hit advances the branch; a different-position cache hit
        // (D33 §6) does not.
        //
        // `merge_outcome` carries the user-layer's CAS outcome only —
        // surfacing both user-layer and provenance outcomes would
        // conflate two distinct events. See proto comment on
        // `RunProgramResponse.merge` for the rationale.
        //
        // `errors` accumulates every failure that should turn this
        // response into a `success=false` (D34 §6 trace-not-found bug
        // — previously these were `warn!`'d and silently discarded,
        // leaving the caller holding a `trace_iri` that pointed at a
        // layer the chain never accepted).
        let mut branch_advanced = false;
        // The user-layer's persist info. We stash the full struct so
        // the response can disambiguate `CACHED_DIFFERENT_POSITION`
        // from `UNSPECIFIED` via `info.cache_hit_different_position`;
        // surfacing only `merge_outcome` would conflate them.
        let mut user_persist_info: Option<PersistedLayerInfo> = None;
        // True iff we reached `persist_layer_if_backend` for any layer.
        // Distinguishes "the run committed (or tried to) — report the
        // outcome" from "we never got to the commit step — say nothing
        // about merge state." The notebook UI keys its cell-footer
        // badges on this distinction (D34 §6.1).
        let mut commit_attempted = false;
        let mut errors: Vec<ValidationError> = Vec::new();
        let result_layer_head = {
            let mut ctx = ctx_arc.write().await;

            // Add domain resources produced by the run (chain-resident
            // outputs of comorphism reify and the program's final
            // Resource value). Every resource added here is
            // kernel-generated — a failure to add one is an internal
            // bug (malformed IRI, conflicting type, etc.) and must
            // surface as a kernel-internal error, not be swallowed.
            for r in &produced_resources {
                if let Err(e) = ctx.add_resource(r.clone()) {
                    errors.push(ValidationError {
                        resource_iri: r.id().map(|i| i.as_str().to_string()).unwrap_or_default(),
                        property_iri: String::new(),
                        rule: "internal".to_string(),
                        message: format!("failed to add produced resource: {e}"),
                        severity: "error".to_string(),
                    });
                }
            }
            // Capture the trace IRI before moving the resource — needed
            // for the failure path's error message (trace_iri_str is
            // semantically the same value, but reading it off the
            // resource ties the error to the actual object that
            // failed).
            let trace_iri_for_err = trace_resource
                .id()
                .map(|i| i.as_str().to_string())
                .unwrap_or_default();
            if let Err(e) = ctx.add_resource(trace_resource) {
                errors.push(ValidationError {
                    resource_iri: trace_iri_for_err,
                    property_iri: String::new(),
                    rule: "internal".to_string(),
                    message: format!("failed to add ProgramTrace: {e}"),
                    severity: "error".to_string(),
                });
            }
            // ComponentTraces are designed to be embedded inside the
            // ProgramTrace's `trace_tree` (see `Resource::new_embedded`
            // in `trace_to_resource`), not added as standalone chain
            // resources — they have no `@id`. The flat `dispatched_traces`
            // list is purely for metrics aggregation (see
            // `ProgramMetrics::from_trace` above); the audit-anchor copy
            // lives in `trace_tree` via `root_trace`. Suppress the
            // `dispatched_traces` variable to make the intent explicit.
            let _ = &dispatched_traces;

            if !errors.is_empty() {
                // Don't attempt the commit if any kernel-generated
                // resource failed to add — the layer would be missing
                // the trace or an output and the response would be
                // structurally inconsistent.
                None
            } else {
                let index = Arc::clone(&*self.institution_index.read().await);
                let runtime = Arc::clone(&*self.institution_runtime.read().await);
                match ctx.commit_with_validation("program-run", &index, &runtime) {
                    Ok(outcome) => {
                        // Persist user layer; provenance layer (when
                        // present, from Holds/Undecidable AutoOnLoad
                        // verdicts) follows. A failure to persist
                        // either is a backend I/O issue, distinct from
                        // a chain rejection — log it and surface as an
                        // error so the caller doesn't see a dangling
                        // trace_iri.
                        commit_attempted = true;
                        let mut user_advanced = false;
                        match self.persist_layer_if_backend(branch, &outcome.user_layer) {
                            Ok(info) => {
                                branch_advanced |= info.branch_advanced;
                                user_advanced = info.branch_advanced;
                                user_persist_info = Some(info);
                            }
                            Err(err) => {
                                tracing::warn!(
                                    { field::OPERATION } = operation::LAYER_COMMIT,
                                    { field::ERROR_KIND } = "program_run_persist_failed",
                                    { field::LAYER_ID } = %outcome.user_layer.id(),
                                    { field::ERROR_MESSAGE } = %err.message,
                                    "failed to persist program-run layer"
                                );
                                errors.push(err);
                            }
                        }
                        // If the user layer didn't make it onto the
                        // durable branch — different-position cache
                        // hit, NeedsWitnessedMerge, or a backend I/O
                        // error — `ctx.head` was advanced to an
                        // in-memory-only layer. Revert so the next
                        // RPC's commit doesn't fail with
                        // "merge during update_branch: no common
                        // ancestor" when its LCA walk hits the ghost.
                        // Skip the provenance persist (its parent is
                        // the never-stored user_layer).
                        if !user_advanced && self.backend.is_some() {
                            ctx.revert_head(outcome.prior_head, "program-run");
                            None
                        } else {
                            if let Some(prov) = outcome.provenance_layer.as_ref() {
                                let mut prov_advanced = false;
                                match self.persist_layer_if_backend(branch, prov) {
                                    Ok(info) => {
                                        branch_advanced |= info.branch_advanced;
                                        prov_advanced = info.branch_advanced;
                                    }
                                    Err(err) => {
                                        tracing::warn!(
                                            { field::OPERATION } = operation::LAYER_COMMIT,
                                            { field::ERROR_KIND } = "program_run_provenance_persist_failed",
                                            { field::LAYER_ID } = %prov.id(),
                                            { field::ERROR_MESSAGE } = %err.message,
                                            "failed to persist program-run provenance layer"
                                        );
                                        errors.push(err);
                                    }
                                }
                                if !prov_advanced && self.backend.is_some() {
                                    // Provenance didn't go in. The
                                    // user_layer did — leave ctx.head
                                    // on user_layer (revert from
                                    // provenance to user_layer).
                                    ctx.revert_head(Arc::clone(&outcome.user_layer), "program-run");
                                }
                            }
                            Some(
                                outcome
                                    .provenance_layer
                                    .as_ref()
                                    .map(|l| l.id().clone())
                                    .unwrap_or_else(|| outcome.user_layer.id().clone()),
                            )
                        }
                    }
                    Err(crate::context::ContextError::ValidationFailed {
                        errors: verrs,
                        provenance_layer,
                        prior_head,
                    }) => {
                        // The chain refused the run's output (AutoOnLoad
                        // `Fails`, structural validation error, etc.).
                        // Per D31 §6.3 the provenance layer recording
                        // the rejection is still persisted as the audit
                        // anchor; the run itself is surfaced as a
                        // failure to the caller.
                        for ve in &verrs {
                            tracing::warn!(
                                { field::OPERATION } = operation::VALIDATE_RESOURCE,
                                { field::ERROR_KIND } = ?ve.rule,
                                { field::RESOURCE_IRI } = ve.resource_id.as_ref().map(|i| i.as_str()).unwrap_or(""),
                                { field::PROPERTY_IRI } = ve.property.as_ref().map(|i| i.as_str()).unwrap_or(""),
                                { field::ERROR_MESSAGE } = %ve.message,
                                "program-run validation error"
                            );
                        }
                        for ve in verrs {
                            errors.push(ValidationError {
                                resource_iri: ve
                                    .resource_id
                                    .as_ref()
                                    .map(|i| i.as_str().to_string())
                                    .unwrap_or_default(),
                                property_iri: ve
                                    .property
                                    .as_ref()
                                    .map(|i| i.as_str().to_string())
                                    .unwrap_or_default(),
                                rule: format!("{:?}", ve.rule),
                                message: ve.message,
                                severity: "error".to_string(),
                            });
                        }
                        commit_attempted = true;
                        let mut prov_advanced = false;
                        if let Some(prov) = provenance_layer.as_ref() {
                            match self.persist_layer_if_backend(branch, prov) {
                                Ok(info) => {
                                    branch_advanced |= info.branch_advanced;
                                    prov_advanced = info.branch_advanced;
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        { field::OPERATION } = operation::LAYER_COMMIT,
                                        { field::ERROR_KIND } = "program_run_provenance_persist_failed",
                                        { field::LAYER_ID } = %prov.id(),
                                        { field::ERROR_MESSAGE } = %err.message,
                                        "failed to persist program-run failure provenance layer"
                                    );
                                    errors.push(err);
                                }
                            }
                        }
                        // If the provenance didn't go through (or
                        // there was no provenance to commit), revert
                        // ctx.head to the original head. The Fails arm
                        // of `commit_with_validation` had already
                        // advanced ctx.head to the provenance layer
                        // (or left it at prior_head if there was no
                        // provenance to commit) — but if persist
                        // cache-hits we need to revert to keep
                        // ctx.head in storage.
                        if !prov_advanced && self.backend.is_some() {
                            if let Some(ph) = prior_head {
                                ctx.revert_head(ph, "program-run");
                            }
                        }
                        // Return the provenance layer's id so the task
                        // record can point at the audit anchor for the
                        // failed run.
                        provenance_layer.map(|l| l.id().clone())
                    }
                    Err(e) => {
                        tracing::warn!(
                            { field::OPERATION } = operation::LAYER_COMMIT,
                            { field::ERROR_KIND } = "program_run_commit_failed",
                            { field::ERROR_MESSAGE } = %e,
                            "program-run layer commit failed"
                        );
                        errors.push(ValidationError {
                            resource_iri: String::new(),
                            property_iri: String::new(),
                            rule: "commit".to_string(),
                            message: format!("{e}"),
                            severity: "error".to_string(),
                        });
                        None
                    }
                }
            }
        };

        let success = errors.is_empty();

        // Record the task's final state. A successful run records the
        // result layer id so clients that polled via GetTaskStatus can
        // resolve it (D21 §3.7); a failed run records `Failed` and the
        // provenance layer id (if any) so the failure audit is also
        // discoverable through the same path.
        if let (Some(store), Some(tc)) = (&self.task_store, task_context.as_ref()) {
            if let Ok(Some(mut rec)) = store.get_task(&tc.session_id, &tc.task_id) {
                rec.status = if success {
                    crate::task::TaskStatus::Completed
                } else {
                    crate::task::TaskStatus::Failed
                };
                rec.result_layer_head = result_layer_head;
                rec.updated_at = now_millis();
                if let Err(e) = store.put_task(&rec) {
                    tracing::warn!(
                        { field::OPERATION } = operation::TASK_CHECKPOINT,
                        { field::ERROR_KIND } = "task_record_update_failed",
                        { field::TASK_ID } = ?tc.task_id,
                        { field::ERROR_MESSAGE } = %e,
                        "failed to update task record after run completion"
                    );
                }
            }
        }

        // On failure, blank the response's `output` / `trace_iri` /
        // `output_resource_iris` — those IRIs reference resources the
        // chain didn't accept, so returning them gives clients a
        // dangling pointer (the exact bug this fix closes).
        Ok(Response::new(RunProgramResponse {
            success,
            output: if success {
                Self::serialize_resource(&output)
            } else {
                Vec::new()
            },
            errors,
            trace_iri: if success {
                trace_iri_str
            } else {
                String::new()
            },
            task_id: task_id_str,
            output_resource_iris: if success {
                output_resource_iris
            } else {
                Vec::new()
            },
            branch_advanced,
            // Only populate `merge` when persist actually ran — see
            // `commit_attempted`'s declaration. A failure that aborts
            // before persist (add_resource on a kernel-generated
            // resource, eval error) sends `merge=None` so the notebook
            // doesn't render a misleading badge.
            merge: if commit_attempted {
                Some(merge_info_from_persist_info(user_persist_info.as_ref()))
            } else {
                None
            },
        }))
    }
}

#[allow(clippy::result_large_err)]
#[tonic::async_trait]
impl EigeniusKernel for EigeniusService {
    async fn load(&self, request: Request<LoadRequest>) -> Result<Response<LoadResponse>, Status> {
        let mut guard = RpcGuard::start(operation::RPC_LOAD);
        let req = request.into_inner();
        let branch = resolve_branch_name(&req.branch).to_string();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_LOAD,
            { field::CONTENT_TYPE } = %req.content_type,
            { field::SIZE_BYTES } = req.resources.len(),
            branch = %branch,
            "load payload"
        );
        let resources = self
            .parse_resources(&req.resources, &req.content_type)
            .await?;
        let count = resources.len() as u32;

        let ctx_arc = self.get_branch_context(&branch).await?;
        let mut ctx = ctx_arc.write().await;
        for resource in resources {
            ctx.add_resource(resource)
                .map_err(|e| Status::failed_precondition(format!("load error: {e}")))?;
        }

        let mut layer_id = String::new();
        let mut branch_advanced = false;
        // The user-layer's persist info — the user-facing one. Any
        // follow-up persists (AutoOnLoad provenance, institution_classes)
        // log on failure but their outcomes are not surfaced in the
        // response; see proto comment on `RunProgramResponse.merge`
        // for the design rationale. Stashed as the full struct so the
        // response can disambiguate `CACHED_DIFFERENT_POSITION` from
        // `UNSPECIFIED` via `info.cache_hit_different_position`.
        let mut user_persist_info: Option<PersistedLayerInfo> = None;
        let mut errors = Vec::new();

        if req.auto_commit {
            // Snapshot the current D14 institution index + runtime to
            // pass to commit_with_validation. Newly committed
            // resources are gated by AutoOnLoad QueryClasses already
            // declared in the chain; QueryClasses declared in the
            // same Load batch take effect on subsequent loads (the
            // index gets rebuilt below).
            let index_snapshot = Arc::clone(&*self.institution_index.read().await);
            let runtime_snapshot = Arc::clone(&*self.institution_runtime.read().await);
            match ctx.commit_with_validation("loaded", &index_snapshot, &runtime_snapshot) {
                Ok(outcome) => {
                    let layer = outcome.user_layer;
                    let prior_head = outcome.prior_head;
                    let provenance = outcome.provenance_layer;
                    // Default to the freshly-built layer's id; the
                    // persist step below may substitute the canonical
                    // id from the anchored-commit cache (D33 §6) if
                    // this exact content + supporting context was
                    // committed before.
                    layer_id = layer.id().to_string();
                    tracing::info!(
                        { field::OPERATION } = operation::LAYER_COMMIT,
                        { field::LAYER_ID } = %layer_id,
                        { field::COUNT } = count,
                        branch = %branch,
                        "layer committed"
                    );
                    drop(ctx);
                    // `user_advanced`: did the durable branch ref move
                    // to (or through) the user layer? Drives whether we
                    // can safely build on it for the provenance commit,
                    // and whether the in-memory `ctx.head` is allowed
                    // to stay where `commit_with_validation` advanced it.
                    let mut user_advanced = false;
                    match self.persist_layer_if_backend(&branch, &layer) {
                        Ok(info) => {
                            // On a cache hit at a different position
                            // the canonical id differs from the
                            // freshly-built one — surface that to the
                            // caller so they see the chain's canonical
                            // representative for this content.
                            // `branch_advanced` distinguishes a fresh
                            // commit / same-position hit (branch moved)
                            // from a different-position hit (branch
                            // stayed put). `merge_outcome` carries the
                            // CAS taxonomy (FastForward / TrivialMerge /
                            // NeedsWitnessedMerge) when a CAS ran.
                            layer_id = info.layer_id.to_string();
                            branch_advanced = info.branch_advanced;
                            user_advanced = info.branch_advanced;
                            user_persist_info = Some(info);
                        }
                        Err(err) => errors.push(err),
                    }
                    // If the user layer didn't go into the durable
                    // chain — different-position cache hit or
                    // NeedsWitnessedMerge — `ctx.head` was advanced to
                    // an in-memory-only layer whose parent isn't in
                    // storage. Revert so subsequent commits build on
                    // the same head the durable branch ref points at;
                    // skip the provenance persist (whose parent is
                    // the never-stored user_layer) and the
                    // institution-class follow-up commit (same reason).
                    if !user_advanced && self.backend.is_some() {
                        let mut ctx_w = ctx_arc.write().await;
                        ctx_w.revert_head(prior_head, "loaded");
                        drop(ctx_w);
                        let response = LoadResponse {
                            success: errors.is_empty(),
                            errors,
                            layer_id,
                            resource_count: count,
                            branch,
                            branch_advanced,
                            merge: Some(merge_info_from_persist_info(user_persist_info.as_ref())),
                        };
                        if !response.success {
                            guard.fail("validation_failed");
                        }
                        return Ok(Response::new(response));
                    }
                    // Persist the AutoOnLoad provenance layer too if
                    // one was produced (D31 §6.3 Holds/Undecidable
                    // path). The kernel's commit pipeline advances
                    // `ctx.head` to the provenance layer, so failing
                    // to persist it leaves `ctx.head` pointing at a
                    // backend-unknown layer — the next commit's CAS
                    // would surface as "merge during update_branch:
                    // no common ancestor" when the LCA walk hit the
                    // ghost.
                    if let Some(prov) = provenance {
                        let mut prov_advanced = false;
                        match self.persist_layer_if_backend(&branch, &prov) {
                            Ok(info) => prov_advanced = info.branch_advanced,
                            Err(err) => errors.push(err),
                        }
                        if !prov_advanced && self.backend.is_some() {
                            // Revert ctx.head to the user_layer — the
                            // provenance layer didn't make it through.
                            let mut ctx_w = ctx_arc.write().await;
                            ctx_w.revert_head(Arc::clone(&layer), "loaded");
                        } else {
                            self.rebuild_institution_index(&prov).await;
                        }
                    }
                    // Rebuild the D14 institution index from the new
                    // chain so subsequent commits see the just-loaded
                    // declarations.
                    self.rebuild_institution_index(&layer).await;
                    // Scan the newly committed layer for WASM components/institutions.
                    // For institutions, this returns the declared morphism / query
                    // class resources that the registration produced.
                    let published = self.register_wasm_from_layer(&layer, &mut errors).await;

                    // Commit the published institution classes as a follow-up layer
                    // so they become queryable via EigenQL / inspect. Closes #15.
                    if !published.is_empty() {
                        let mut ctx = ctx_arc.write().await;
                        for resource in published {
                            if let Err(e) = ctx.add_resource(resource) {
                                errors.push(ValidationError {
                                    resource_iri: String::new(),
                                    property_iri: String::new(),
                                    rule: "institution_publish".to_string(),
                                    message: format!(
                                        "failed to add institution-declared class: {e}"
                                    ),
                                    severity: "error".to_string(),
                                });
                            }
                        }
                        if ctx.has_changes() {
                            // Save the pre-commit head so we can
                            // revert if the institution_classes
                            // persist short-circuits (different-position
                            // cache hit). Same invariant as the
                            // user-layer + provenance handling above.
                            let inst_prior_head = Arc::clone(ctx.head());
                            match ctx.commit("institution_classes") {
                                Ok(extra) => match self.persist_layer_if_backend(&branch, &extra) {
                                    Ok(info) => {
                                        if info.branch_advanced {
                                            drop(ctx);
                                            self.rebuild_institution_index(&extra).await;
                                        } else if self.backend.is_some() {
                                            ctx.revert_head(inst_prior_head, "loaded");
                                        }
                                    }
                                    Err(err) => {
                                        errors.push(err);
                                        if self.backend.is_some() {
                                            ctx.revert_head(inst_prior_head, "loaded");
                                        }
                                    }
                                },
                                Err(e) => {
                                    errors.push(ValidationError {
                                        resource_iri: String::new(),
                                        property_iri: String::new(),
                                        rule: "institution_publish".to_string(),
                                        message: format!("commit failed: {e}"),
                                        severity: "error".to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
                Err(crate::context::ContextError::ValidationFailed {
                    errors: verrs,
                    provenance_layer,
                    prior_head,
                }) => {
                    // Per D31 §6.3: a Fails AutoOnLoad commits Verdict
                    // + RuntimeInvocation as a separate provenance
                    // layer even though the gated resource was
                    // rejected. Persist that layer so the audit
                    // anchor lives on the chain. The Load itself
                    // still surfaces as a failure to the caller.
                    let mut prov_advanced = false;
                    if let Some(layer) = provenance_layer {
                        match self.persist_layer_if_backend(&branch, &layer) {
                            Ok(info) => prov_advanced = info.branch_advanced,
                            Err(err) => errors.push(err),
                        }
                        if prov_advanced {
                            self.rebuild_institution_index(&layer).await;
                        }
                    }
                    // If the provenance layer didn't make it onto the
                    // durable branch (cache hit / I/O error) and
                    // commit_with_validation had advanced ctx.head to
                    // it, revert to the saved prior_head so the next
                    // commit's LCA walk doesn't fall off the chain.
                    if !prov_advanced && self.backend.is_some() {
                        if let Some(ph) = prior_head {
                            let mut ctx_w = ctx_arc.write().await;
                            ctx_w.revert_head(ph, "loaded");
                        }
                    }
                    // Per-error logging — each rule violation gets its
                    // own warn-level event so dashboards can group on
                    // `error_kind` (the rule's debug label) without
                    // having to parse a flattened blob.
                    for ve in &verrs {
                        tracing::warn!(
                            { field::OPERATION } = operation::VALIDATE_RESOURCE,
                            { field::ERROR_KIND } = ?ve.rule,
                            { field::RESOURCE_IRI } = ve.resource_id.as_ref().map(|i| i.as_str()).unwrap_or(""),
                            { field::PROPERTY_IRI } = ve.property.as_ref().map(|i| i.as_str()).unwrap_or(""),
                            { field::ERROR_MESSAGE } = %ve.message,
                            "validation error"
                        );
                    }
                    for ve in verrs {
                        errors.push(ValidationError {
                            resource_iri: ve
                                .resource_id
                                .as_ref()
                                .map(|i| i.as_str().to_string())
                                .unwrap_or_default(),
                            property_iri: ve
                                .property
                                .as_ref()
                                .map(|i| i.as_str().to_string())
                                .unwrap_or_default(),
                            rule: format!("{:?}", ve.rule),
                            message: ve.message,
                            severity: "error".to_string(),
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        { field::OPERATION } = operation::LAYER_COMMIT,
                        { field::ERROR_KIND } = "commit_failed",
                        { field::ERROR_MESSAGE } = %e,
                        "layer commit failed"
                    );
                    errors.push(ValidationError {
                        resource_iri: String::new(),
                        property_iri: String::new(),
                        rule: "commit".to_string(),
                        message: format!("{e}"),
                        severity: "error".to_string(),
                    });
                }
            }
        }

        // `merge` is populated only when a commit was actually
        // attempted. `auto_commit=false` (validate-only Load) skips the
        // entire commit block above, so no CAS happened; sending an
        // `UNSPECIFIED` MergeInfo in that case would render as a
        // misleading `cached` badge in notebook UIs.
        let response = LoadResponse {
            success: errors.is_empty(),
            errors,
            layer_id,
            resource_count: count,
            branch,
            branch_advanced,
            merge: if req.auto_commit {
                Some(merge_info_from_persist_info(user_persist_info.as_ref()))
            } else {
                None
            },
        };
        if !response.success {
            guard.fail("validation_failed");
            tracing::warn!(
                { field::OPERATION } = operation::RPC_LOAD,
                { field::COUNT } = response.errors.len(),
                "load completed with errors"
            );
        }
        Ok(Response::new(response))
    }

    async fn inspect(
        &self,
        request: Request<InspectRequest>,
    ) -> Result<Response<InspectResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_INSPECT);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_INSPECT,
            { field::RESOURCE_IRI } = %req.iri,
            "inspect target"
        );
        let iri = Iri::parse(&req.iri)
            .map_err(|e| Status::invalid_argument(format!("invalid IRI: {e}")))?;

        let layer = self.resolve_read_layer(&req.at_layer, &req.branch).await?;
        match layer.resolve(&iri) {
            Some(resource) => Ok(Response::new(InspectResponse {
                found: true,
                resource: Self::serialize_resource(&resource),
            })),
            None => Ok(Response::new(InspectResponse {
                found: false,
                resource: Vec::new(),
            })),
        }
    }

    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let mut guard = RpcGuard::start(operation::RPC_QUERY);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_QUERY,
            { field::SIZE_BYTES } = req.eigenql.len(),
            "query payload"
        );
        let layer = self.resolve_read_layer(&req.at_layer, &req.branch).await?;
        let branch_name = resolve_branch_name(&req.branch).to_string();
        let ctx_arc = self.get_branch_context(&branch_name).await?;
        let index = Arc::clone(&*self.institution_index.read().await);
        let inst_runtime = Arc::clone(&*self.institution_runtime.read().await);
        let components = Arc::clone(&*self.components.read().await);

        let outcome = {
            let ctx = ctx_arc.read().await;
            let runtime = query::evaluate::FiberRuntime {
                index: Some(&index),
                runtime: Some(&inst_runtime),
                components: Some(&components),
                overlay: None,
                ctx: Some(&ctx),
            };

            match query::execute_with_into(&req.eigenql, &layer, runtime) {
                Ok(o) => o,
                Err(errors) => {
                    let msgs: Vec<String> = errors.iter().map(|e| format!("{e}")).collect();
                    guard.fail("query_failed");
                    tracing::warn!(
                        { field::OPERATION } = operation::QUERY_EVALUATE,
                        { field::COUNT } = errors.len(),
                        { field::ERROR_MESSAGE } = %msgs.join("; "),
                        "query failed"
                    );
                    // Query parse/eval errored before any FIBER INTO
                    // commit could run — `merge` stays None (no CAS
                    // happened).
                    return Ok(Response::new(QueryResponse {
                        success: false,
                        document: Vec::new(),
                        content_type: String::new(),
                        error: format!("query error: {}", msgs.join("; ")),
                        output_resource_iris: Vec::new(),
                        branch_advanced: false,
                        merge: None,
                    }));
                }
            }
        };

        // FIBER ... INTO produced chain-bound resources — commit them
        // through `commit_with_validation` so AutoOnLoad QueryClasses
        // bound to their classes fire on chain entry (D14 §9.3 step 5
        // chain-reinsertion via EigenQL).
        //
        // `commit_attempted` distinguishes "this query just read"
        // (`merge` should be None) from "this query attempted a
        // FIBER INTO" (`merge` reports the CAS outcome). Without it,
        // every transient read would render as a misleading `cached`
        // badge in the notebook.
        let mut branch_advanced = false;
        let mut user_persist_info: Option<PersistedLayerInfo> = None;
        let mut commit_attempted = false;
        let output_resource_iris: Vec<String> = if outcome.into_resources.is_empty() {
            Vec::new()
        } else {
            let iris: Vec<String> = outcome
                .into_resources
                .iter()
                .filter_map(|r| r.id().map(|i| i.as_str().to_string()))
                .collect();
            let mut ctx = ctx_arc.write().await;
            for r in &outcome.into_resources {
                if let Err(e) = ctx.add_resource(r.clone()) {
                    tracing::warn!(
                        { field::OPERATION } = operation::RPC_QUERY,
                        { field::ERROR_KIND } = "fiber_into_add_failed",
                        { field::ERROR_MESSAGE } = %e,
                        resource_iri = ?r.id(),
                        "failed to add FIBER INTO resource to chain layer"
                    );
                }
            }
            match ctx.commit_with_validation("eigenql-into", &index, &inst_runtime) {
                Ok(co) => {
                    commit_attempted = true;
                    let mut user_advanced = false;
                    match self.persist_layer_if_backend(&branch_name, &co.user_layer) {
                        Ok(info) => {
                            branch_advanced |= info.branch_advanced;
                            user_advanced = info.branch_advanced;
                            user_persist_info = Some(info);
                        }
                        Err(err) => tracing::warn!(
                            { field::OPERATION } = operation::LAYER_COMMIT,
                            { field::ERROR_KIND } = "eigenql_into_persist_failed",
                            { field::LAYER_ID } = %co.user_layer.id(),
                            { field::ERROR_MESSAGE } = %err.message,
                            "failed to persist eigenql-into layer (query result still returned)"
                        ),
                    }
                    if !user_advanced && self.backend.is_some() {
                        // FIBER INTO's user layer didn't go through —
                        // revert ctx.head to keep it on the durable
                        // chain. See D34 §6.1 / the trace-not-found fix.
                        ctx.revert_head(co.prior_head, "eigenql-into");
                    } else if let Some(prov) = co.provenance_layer.as_ref() {
                        let mut prov_advanced = false;
                        match self.persist_layer_if_backend(&branch_name, prov) {
                            Ok(info) => {
                                branch_advanced |= info.branch_advanced;
                                prov_advanced = info.branch_advanced;
                            }
                            Err(err) => tracing::warn!(
                                { field::OPERATION } = operation::LAYER_COMMIT,
                                { field::ERROR_KIND } = "eigenql_into_provenance_persist_failed",
                                { field::LAYER_ID } = %prov.id(),
                                { field::ERROR_MESSAGE } = %err.message,
                                "failed to persist eigenql-into provenance layer"
                            ),
                        }
                        if !prov_advanced && self.backend.is_some() {
                            ctx.revert_head(Arc::clone(&co.user_layer), "eigenql-into");
                        }
                    }
                }
                Err(e) => {
                    guard.fail("eigenql_into_commit_failed");
                    let msg = format!("{e}");
                    tracing::warn!(
                        { field::OPERATION } = operation::LAYER_COMMIT,
                        { field::ERROR_KIND } = "eigenql_into_commit_failed",
                        { field::ERROR_MESSAGE } = %msg,
                        "FIBER INTO commit failed; surfacing error to caller"
                    );
                    // If the failure was a Fails-verdict
                    // ValidationFailed, ctx.head was advanced to a
                    // verdict_provenance layer that we didn't persist.
                    // Revert to the carried prior_head so ctx.head
                    // matches the durable branch state.
                    if let crate::context::ContextError::ValidationFailed {
                        prior_head: Some(ph),
                        ..
                    } = &e
                    {
                        if self.backend.is_some() {
                            ctx.revert_head(Arc::clone(ph), "eigenql-into");
                        }
                    }
                    // commit_with_validation rejected the FIBER INTO
                    // resources — no CAS reached `persist_layer_if_backend`.
                    // Send `merge=None` so the notebook surfaces this
                    // as the (already-populated) error rather than as
                    // a misleading cache/merge badge.
                    return Ok(Response::new(QueryResponse {
                        success: false,
                        document: Vec::new(),
                        content_type: String::new(),
                        error: format!("FIBER INTO commit failed: {msg}"),
                        output_resource_iris: Vec::new(),
                        branch_advanced: false,
                        merge: None,
                    }));
                }
            }
            iris
        };

        Ok(Response::new(QueryResponse {
            success: true,
            document: eigon_cbor::serialize_document(&outcome.document),
            content_type: "application/cbor".to_string(),
            error: String::new(),
            output_resource_iris,
            branch_advanced,
            merge: if commit_attempted {
                Some(merge_info_from_persist_info(user_persist_info.as_ref()))
            } else {
                None
            },
        }))
    }

    async fn validate_program(
        &self,
        request: Request<ValidateProgramRequest>,
    ) -> Result<Response<ValidateProgramResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_VALIDATE_PROGRAM);
        let req = request.into_inner();
        let resources = self
            .parse_resources(&req.program, &req.content_type)
            .await?;
        let program = resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no program resource"))?;

        let ctx_arc = self.get_branch_context(DEFAULT_BRANCH).await?;
        let ctx = ctx_arc.read().await;

        match expr::parse_program(&program, ctx.head()) {
            Ok((_term, typ)) => {
                // Validate template references against input type
                let mut template_errors = Vec::new();
                let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
                let input_type_prop = Iri::parse("urn:eigenius:program:input_type").unwrap();
                // `program:input_type` is `data_type: resource`; after
                // canonicalisation the value is `ResourceRef`. Match
                // both shapes via `as_iri_str` so template validation
                // actually runs on production-shaped programs.
                if let (
                    Some(input_type_str),
                    Some(crate::ontology::resource::Value::Embedded(body)),
                ) = (
                    program.get(&input_type_prop).and_then(|v| v.as_iri_str()),
                    program.get(&body_prop),
                ) {
                    if let Ok(input_type_iri) = Iri::parse(input_type_str) {
                        let comp_arg_prop =
                            Iri::parse("urn:eigenius:program:component_argument").unwrap();
                        // Walk expression tree looking for component arguments
                        fn find_comp_args(resource: &Resource, prop: &Iri) -> Vec<Resource> {
                            let mut args = Vec::new();
                            if let Some(crate::ontology::resource::Value::Embedded(arg)) =
                                resource.get(prop)
                            {
                                args.push(arg.as_ref().clone());
                            }
                            // Recurse into embedded resources
                            for val in resource.properties().values() {
                                if let crate::ontology::resource::Value::Embedded(child) = val {
                                    args.extend(find_comp_args(child, prop));
                                }
                            }
                            args
                        }
                        for comp_arg in find_comp_args(body, &comp_arg_prop) {
                            let errs = crate::program::schema::validate_component_templates(
                                &comp_arg,
                                &input_type_iri,
                                ctx.head(),
                            );
                            for e in errs {
                                template_errors.push(ValidationError {
                                    resource_iri: String::new(),
                                    property_iri: String::new(),
                                    rule: "template".to_string(),
                                    message: format!("{e}"),
                                    severity: "error".to_string(),
                                });
                            }
                        }
                    }
                }

                // Validate output schemas (bijectivity check, D8 §4)
                for e in crate::program::schema::validate_output_schemas(&program, ctx.head()) {
                    template_errors.push(ValidationError {
                        resource_iri: String::new(),
                        property_iri: String::new(),
                        rule: "schema_bijectivity".to_string(),
                        message: format!("{e}"),
                        severity: "error".to_string(),
                    });
                }

                if template_errors.is_empty() {
                    tracing::debug!(
                        { field::OPERATION } = operation::PROGRAM_TYPE_CHECK,
                        program_iri = program.id().map(|i| i.as_str()).unwrap_or(""),
                        program_type = ?typ,
                        "program type-check succeeded"
                    );
                    Ok(Response::new(ValidateProgramResponse {
                        valid: true,
                        errors: Vec::new(),
                        program_type: format!("{typ:?}"),
                    }))
                } else {
                    Ok(Response::new(ValidateProgramResponse {
                        valid: false,
                        errors: template_errors,
                        program_type: format!("{typ:?}"),
                    }))
                }
            }
            Err(e) => Ok(Response::new(ValidateProgramResponse {
                valid: false,
                errors: vec![ValidationError {
                    resource_iri: String::new(),
                    property_iri: String::new(),
                    rule: "type_check".to_string(),
                    message: e,
                    severity: "error".to_string(),
                }],
                program_type: String::new(),
            })),
        }
    }

    async fn run_program(
        &self,
        request: Request<RunProgramRequest>,
    ) -> Result<Response<RunProgramResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_RUN_PROGRAM);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_RUN_PROGRAM,
            { field::CONTENT_TYPE } = %req.content_type,
            "run_program payload"
        );
        let program_resources = self
            .parse_resources(&req.program, &req.content_type)
            .await?;
        let program = program_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no program resource"))?;

        let input_resources = self.parse_resources(&req.input, &req.content_type).await?;
        let input = input_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no input resource"))?;

        let branch = resolve_branch_name(&req.branch).to_string();
        self.execute_program(&branch, program, input).await
    }

    async fn run_program_by_iri(
        &self,
        request: Request<RunProgramByIriRequest>,
    ) -> Result<Response<RunProgramResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_RUN_PROGRAM_BY_IRI);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_RUN_PROGRAM_BY_IRI,
            { field::PROGRAM_IRI } = %req.program_iri,
            { field::RESOURCE_IRI } = %req.input_iri,
            "run_program_by_iri target"
        );
        if req.program_iri.is_empty() {
            return Err(Status::invalid_argument("program_iri is required"));
        }
        if req.input_iri.is_empty() {
            return Err(Status::invalid_argument("input_iri is required"));
        }

        let program_iri = Iri::parse(&req.program_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid program_iri: {e}")))?;
        let input_iri = Iri::parse(&req.input_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid input_iri: {e}")))?;

        let layer = self.resolve_read_layer(&req.at_layer, &req.branch).await?;
        let program = layer
            .resolve(&program_iri)
            .map(|arc| (*arc).clone())
            .ok_or_else(|| {
                Status::not_found(format!("program resource not found: {}", req.program_iri))
            })?;
        let input = layer
            .resolve(&input_iri)
            .map(|arc| (*arc).clone())
            .ok_or_else(|| {
                Status::not_found(format!("input resource not found: {}", req.input_iri))
            })?;

        let branch = resolve_branch_name(&req.branch).to_string();
        self.execute_program(&branch, program, input).await
    }

    async fn reflect(
        &self,
        request: Request<ReflectRequest>,
    ) -> Result<Response<ReflectResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_REFLECT);
        let req = request.into_inner();
        let resources = self.parse_resources(&req.trace, &req.content_type).await?;

        if resources.is_empty() {
            // No resources to commit — `merge` stays None (no CAS).
            return Ok(Response::new(ReflectResponse {
                success: false,
                trace_iri: String::new(),
                branch_advanced: false,
                merge: None,
            }));
        }

        // The first resource should be a trace (ProgramTrace, DeclarationTrace, etc.)
        let trace_resource = &resources[0];
        let trace_iri = trace_resource
            .id()
            .map(|i| i.as_str().to_string())
            .unwrap_or_default();

        // Commit all trace resources to a new layer
        let branch = resolve_branch_name(&req.branch).to_string();
        let ctx_arc = self.get_branch_context(&branch).await?;
        let mut ctx = ctx_arc.write().await;
        // Capture the pre-commit head so we can revert ctx.head if the
        // persist short-circuits (different-position cache hit /
        // NeedsWitnessedMerge). `ctx.commit()` advances head to the
        // freshly-built layer; if that layer doesn't make it onto the
        // durable branch, leaving ctx.head there poisons the next
        // commit's LCA walk.
        let prior_head = Arc::clone(ctx.head());
        for resource in resources {
            ctx.add_resource(resource)
                .map_err(|e| Status::failed_precondition(format!("reflect error: {e}")))?;
        }
        let layer = ctx
            .commit("reflect")
            .map_err(|e| Status::internal(format!("reflect commit failed: {e}")))?;
        let info = self
            .persist_layer_if_backend(&branch, &layer)
            .map_err(|err| Status::internal(format!("reflect persist failed: {}", err.message)))?;
        if !info.branch_advanced && self.backend.is_some() {
            ctx.revert_head(prior_head, "reflect");
        }
        drop(ctx);

        Ok(Response::new(ReflectResponse {
            success: true,
            trace_iri,
            branch_advanced: info.branch_advanced,
            merge: Some(merge_info_from_persist_info(Some(&info))),
        }))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        // Guard fires at debug level — invisible at the default
        // `info` filter, so frequent probes don't add log noise but
        // remain inspectable when debugging readiness/liveness.
        let _guard = RpcGuard::start(operation::RPC_HEALTH);
        let ctx_arc = self.get_branch_context(DEFAULT_BRANCH).await?;
        let ctx = ctx_arc.read().await;
        let resource_count = ctx.head().iter_all_resources().count() as u64;

        // D21 §6 resume observability — populated by the resume
        // sweep when it's active.
        use std::sync::atomic::Ordering;
        let resume_in_progress = self.resume_state.in_progress.load(Ordering::SeqCst);
        let tasks_resuming = self.resume_state.remaining.load(Ordering::SeqCst);

        Ok(Response::new(HealthResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            layer_count: 2, // core + program ontology
            resource_count,
            resume_in_progress,
            tasks_resuming,
        }))
    }

    async fn list_institutions(
        &self,
        request: Request<ListInstitutionsRequest>,
    ) -> Result<Response<ListInstitutionsResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LIST_INSTITUTIONS);
        let req = request.into_inner();
        if !req.at_layer.is_empty() {
            let _ = self.resolve_read_layer(&req.at_layer, "").await?;
        }
        // D14 list-from-index. Each `InstitutionInfo` carries the
        // QueryClass input-class IRIs declared by the institution.
        // Comorphisms / formats are not surfaced through this RPC
        // (a future proto revision can expand the surface).
        let index = Arc::clone(&*self.institution_index.read().await);
        let mut infos: Vec<proto::InstitutionInfo> = index
            .institutions()
            .map(|inst| {
                let query_types: Vec<String> = index
                    .query_classes()
                    .filter(|qc| qc.institution_ref == inst.iri)
                    .map(|qc| qc.query_class.as_str().to_string())
                    .collect();
                proto::InstitutionInfo {
                    iri: inst.iri.as_str().to_string(),
                    name: inst.name.clone(),
                    query_types,
                }
            })
            .collect();
        infos.sort_by(|a, b| a.iri.cmp(&b.iri));

        Ok(Response::new(ListInstitutionsResponse {
            institutions: infos,
        }))
    }

    async fn get_schema(
        &self,
        request: Request<GetSchemaRequest>,
    ) -> Result<Response<GetSchemaResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_GET_SCHEMA);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_GET_SCHEMA,
            { field::CLASS_IRI } = %req.class_iri,
            "get_schema target"
        );
        let class_iri = Iri::parse(&req.class_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid IRI: {e}")))?;

        let layer = self.resolve_read_layer(&req.at_layer, "").await?;
        match crate::program::schema::schema_for_class(&class_iri, &layer) {
            Ok((schema, _table)) => Ok(Response::new(GetSchemaResponse {
                success: true,
                json_schema: serde_json::to_string_pretty(&schema).unwrap_or_default(),
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(GetSchemaResponse {
                success: false,
                json_schema: String::new(),
                error: format!("{e}"),
            })),
        }
    }

    async fn list_tasks(
        &self,
        _request: Request<ListTasksRequest>,
    ) -> Result<Response<ListTasksResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LIST_TASKS);
        let tasks = match &self.task_store {
            Some(store) => {
                let session_id = self.session.read().await.session_id;
                match store.list_tasks(&session_id) {
                    Ok(records) => records.into_iter().map(task_record_to_info).collect(),
                    Err(e) => {
                        return Err(Status::internal(format!("list_tasks failed: {e}")));
                    }
                }
            }
            None => Vec::new(),
        };
        Ok(Response::new(ListTasksResponse { tasks }))
    }

    async fn get_task_status(
        &self,
        request: Request<GetTaskStatusRequest>,
    ) -> Result<Response<GetTaskStatusResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_GET_TASK_STATUS);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_GET_TASK_STATUS,
            { field::TASK_ID } = %req.task_id,
            "get_task_status target"
        );
        let store = match &self.task_store {
            Some(s) => s,
            None => {
                return Ok(Response::new(GetTaskStatusResponse {
                    found: false,
                    task: None,
                }))
            }
        };
        let task_id = uuid::Uuid::parse_str(&req.task_id)
            .map_err(|e| Status::invalid_argument(format!("invalid task_id: {e}")))?;
        let session_id = self.session.read().await.session_id;
        match store.get_task(&session_id, &task_id) {
            Ok(Some(record)) => Ok(Response::new(GetTaskStatusResponse {
                found: true,
                task: Some(task_record_to_info(record)),
            })),
            Ok(None) => Ok(Response::new(GetTaskStatusResponse {
                found: false,
                task: None,
            })),
            Err(e) => Err(Status::internal(format!("get_task failed: {e}"))),
        }
    }

    async fn cancel_task(
        &self,
        request: Request<CancelTaskRequest>,
    ) -> Result<Response<CancelTaskResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_CANCEL_TASK);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_CANCEL_TASK,
            { field::TASK_ID } = %req.task_id,
            "cancel_task target"
        );
        let store = match &self.task_store {
            Some(s) => s,
            None => {
                return Ok(Response::new(CancelTaskResponse {
                    success: false,
                    status: String::new(),
                    error: "no persistent backend; tasks are not tracked".to_string(),
                }))
            }
        };
        let task_id = uuid::Uuid::parse_str(&req.task_id)
            .map_err(|e| Status::invalid_argument(format!("invalid task_id: {e}")))?;
        let session_id = self.session.read().await.session_id;
        let mut record = match store.get_task(&session_id, &task_id) {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Ok(Response::new(CancelTaskResponse {
                    success: false,
                    status: String::new(),
                    error: format!("task not found: {task_id}"),
                }));
            }
            Err(e) => {
                return Err(Status::internal(format!("get_task failed: {e}")));
            }
        };

        // If already terminal, just echo the current status — there's
        // nothing to cancel.
        if record.status.is_terminal() {
            let status = format!("{:?}", record.status);
            return Ok(Response::new(CancelTaskResponse {
                success: true,
                status,
                error: String::new(),
            }));
        }

        // Flip the persisted status to Cancelling. 9b-iii.4 will
        // switch this to a cooperative cancellation that the running
        // evaluator picks up between IO dispatches; for synchronous
        // 9b-iii.3, CancelTask is effectively an "abandoned" marker
        // until the next resume sweep re-evaluates the task and sees
        // it as Cancelling.
        record.status = crate::task::TaskStatus::Cancelling;
        record.updated_at = now_millis();
        if let Err(e) = store.put_task(&record) {
            return Err(Status::internal(format!("put_task failed: {e}")));
        }

        Ok(Response::new(CancelTaskResponse {
            success: true,
            status: format!("{:?}", record.status),
            error: String::new(),
        }))
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
        _request: Request<ListBranchesRequest>,
    ) -> Result<Response<ListBranchesResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LIST_BRANCHES);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;
        let branches = backend
            .list_branches()
            .map_err(|e| Status::internal(format!("list_branches failed: {e}")))?;
        let branches = branches
            .into_iter()
            .map(|(name, head)| BranchInfo {
                name,
                head_layer: hex::encode(head.0),
            })
            .collect();
        Ok(Response::new(ListBranchesResponse { branches }))
    }

    async fn get_branch(
        &self,
        request: Request<GetBranchRequest>,
    ) -> Result<Response<GetBranchResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_GET_BRANCH);
        let req = request.into_inner();
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;
        match backend
            .get_branch(&req.name)
            .map_err(|e| Status::internal(format!("get_branch failed: {e}")))?
        {
            Some(head) => Ok(Response::new(GetBranchResponse {
                found: true,
                head_layer: hex::encode(head.0),
            })),
            None => Ok(Response::new(GetBranchResponse {
                found: false,
                head_layer: String::new(),
            })),
        }
    }

    async fn create_branch(
        &self,
        request: Request<CreateBranchRequest>,
    ) -> Result<Response<CreateBranchResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_CREATE_BRANCH);
        let req = request.into_inner();
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;
        // Validate from_layer is a known layer.
        let bytes = hex::decode(&req.from_layer)
            .map_err(|e| Status::invalid_argument(format!("from_layer not valid hex: {e}")))?;
        if bytes.len() != 32 {
            return Err(Status::invalid_argument(
                "from_layer must be a 32-byte SHA-256 (64 hex chars)",
            ));
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);
        let from_layer = crate::layer::LayerId(id);
        match backend.load_chain_from(&from_layer) {
            Ok(Some(_)) => {}
            Ok(None) | Err(crate::storage::StorageError::NotFound(_)) => {
                return Err(Status::not_found(format!(
                    "from_layer {} not in store",
                    req.from_layer
                )))
            }
            Err(e) => return Err(Status::internal(format!("load_chain_from failed: {e}"))),
        }

        let storage = crate::layer::LayerStorage::with_persistent(Arc::clone(backend));
        match crate::lattice::update_branch(
            &req.name,
            None,
            from_layer.clone(),
            crate::lattice::ConflictPolicy::StrictFastForward,
            storage,
            backend.as_ref(),
        ) {
            Ok(crate::lattice::UpdateOutcome::FastForward) => {
                Ok(Response::new(CreateBranchResponse {
                    success: true,
                    head_layer: hex::encode(from_layer.0),
                    error: String::new(),
                }))
            }
            Ok(_) => unreachable!(
                "CreateBranch passes None expected_old_head; only FastForward or error possible"
            ),
            Err(crate::lattice::BranchUpdateError::InvalidBranchName(_)) => {
                Err(Status::invalid_argument(format!(
                    "invalid branch name: {:?} (must match [A-Za-z0-9_-]+, max 256 chars)",
                    req.name
                )))
            }
            Err(crate::lattice::BranchUpdateError::StrictFastForwardViolation { .. }) => {
                // Branch already exists.
                Ok(Response::new(CreateBranchResponse {
                    success: false,
                    head_layer: String::new(),
                    error: format!("branch {:?} already exists", req.name),
                }))
            }
            Err(crate::lattice::BranchUpdateError::Storage(e)) => {
                Err(Status::internal(format!("storage error: {e}")))
            }
        }
    }

    async fn delete_branch(
        &self,
        request: Request<DeleteBranchRequest>,
    ) -> Result<Response<DeleteBranchResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_DELETE_BRANCH);
        let req = request.into_inner();
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;

        // Gather active task pins for the CheckPins safety policy. With
        // force=true we skip this scan entirely.
        let pins: Vec<crate::layer::LayerId> = if req.force {
            Vec::new()
        } else if let Some(store) = self.task_store.as_ref() {
            let session_id = self.session.read().await.session_id;
            match store.list_tasks(&session_id) {
                Ok(records) => records
                    .into_iter()
                    .filter(|r| !r.status.is_terminal())
                    .map(|r| r.layer_head)
                    .collect(),
                Err(e) => return Err(Status::internal(format!("list_tasks failed: {e}"))),
            }
        } else {
            Vec::new()
        };

        let safety = if req.force {
            crate::lattice::PruneSafety::Force
        } else {
            crate::lattice::PruneSafety::CheckPins(&pins)
        };

        match crate::lattice::prune_branch(&req.name, safety, backend.as_ref()) {
            Ok(crate::lattice::PruneOutcome::Pruned { previous_head }) => {
                Ok(Response::new(DeleteBranchResponse {
                    success: true,
                    deleted: true,
                    previous_head: hex::encode(previous_head.0),
                    error: String::new(),
                }))
            }
            Ok(crate::lattice::PruneOutcome::NotFound) => Ok(Response::new(DeleteBranchResponse {
                success: true,
                deleted: false,
                previous_head: String::new(),
                error: String::new(),
            })),
            Err(crate::lattice::PruneError::InvalidBranchName(_)) => {
                Err(Status::invalid_argument(format!(
                    "invalid branch name: {:?} (must match [A-Za-z0-9_-]+, max 256 chars)",
                    req.name
                )))
            }
            Err(crate::lattice::PruneError::InUse { branch, head }) => {
                Ok(Response::new(DeleteBranchResponse {
                    success: false,
                    deleted: false,
                    previous_head: String::new(),
                    error: format!(
                        "branch {branch:?} is in use (head {head} matches an active task pin); pass force=true to delete anyway",
                    ),
                }))
            }
            Err(crate::lattice::PruneError::Storage(e)) => {
                Err(Status::internal(format!("storage error: {e}")))
            }
        }
    }

    async fn consolidate_chain(
        &self,
        request: Request<ConsolidateChainRequest>,
    ) -> Result<Response<ConsolidateChainResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_CONSOLIDATE_CHAIN);
        let req = request.into_inner();
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("consolidation requires a persistent backend")
        })?;

        let branch = if req.branch.is_empty() {
            "main"
        } else {
            req.branch.as_str()
        };
        let from = parse_layer_id(&req.from_layer, "from_layer")?;
        let to = parse_layer_id(&req.to_layer, "to_layer")?;

        let storage = LayerStorage::with_persistent(Arc::clone(backend));
        let opts =
            build_consolidate_opts(&req.max_walk_entries, req.preserve_history, &self).await?;

        match crate::layer::consolidate_chain(branch, from, to, opts, storage, backend.as_ref()) {
            Ok(outcome) => Ok(Response::new(ConsolidateChainResponse {
                success: true,
                consolidated_layer: hex::encode(outcome.consolidated_layer.0),
                collapsed_layer_count: outcome.collapsed_layer_count,
                head_advanced: outcome.head_advanced,
                error_kind: ConsolidateErrorKind::None as i32,
                error: String::new(),
                error_layer: String::new(),
                error_count: 0,
            })),
            Err(err) => Ok(Response::new(consolidate_error_to_response(err))),
        }
    }

    async fn estimate_consolidation(
        &self,
        request: Request<EstimateConsolidationRequest>,
    ) -> Result<Response<EstimateConsolidationResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_ESTIMATE_CONSOLIDATION);
        let req = request.into_inner();
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("consolidation requires a persistent backend")
        })?;

        let branch = if req.branch.is_empty() {
            "main"
        } else {
            req.branch.as_str()
        };
        let from = parse_layer_id(&req.from_layer, "from_layer")?;
        let to = parse_layer_id(&req.to_layer, "to_layer")?;

        let storage = LayerStorage::with_persistent(Arc::clone(backend));
        let opts =
            build_consolidate_opts(&req.max_walk_entries, req.preserve_history, &self).await?;

        match crate::layer::estimate_consolidation(
            branch,
            from,
            to,
            opts,
            storage,
            backend.as_ref(),
        ) {
            Ok(estimate) => Ok(Response::new(EstimateConsolidationResponse {
                success: true,
                predicted_consolidated_layer: hex::encode(estimate.predicted_consolidated_layer.0),
                collapsed_layer_count: estimate.collapsed_layer_count,
                predicted_walk_entries: estimate.predicted_walk_entries,
                actual_walk_entries: estimate.actual_walk_entries,
                error_kind: ConsolidateErrorKind::None as i32,
                error: String::new(),
                error_layer: String::new(),
                error_count: 0,
            })),
            Err(err) => Ok(Response::new(estimate_error_to_response(err))),
        }
    }
}

/// Parse a hex-encoded LayerId from the wire, returning a typed
/// `Status::invalid_argument` on malformed input.
#[allow(clippy::result_large_err)]
fn parse_layer_id(hex_str: &str, field: &str) -> Result<crate::layer::LayerId, Status> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| Status::invalid_argument(format!("{field} not valid hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(Status::invalid_argument(format!(
            "{field} must be a 32-byte SHA-256 (64 hex chars)"
        )));
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes);
    Ok(crate::layer::LayerId(id))
}

/// Build a `ConsolidateOpts` from the wire request. Pulls active task
/// pins from the session's task store (matches `delete_branch`'s
/// pattern). A `max_walk_entries` of 0 means "use the kernel default."
async fn build_consolidate_opts(
    max_walk_entries: &u64,
    preserve_history: bool,
    service: &EigeniusService,
) -> Result<crate::layer::ConsolidateOpts, Status> {
    let mut opts = crate::layer::ConsolidateOpts::default();
    if *max_walk_entries > 0 {
        opts.max_walk_entries = *max_walk_entries;
    }
    opts.preserve_history = preserve_history;
    if let Some(store) = service.task_store.as_ref() {
        let session_id = service.session.read().await.session_id;
        match store.list_tasks(&session_id) {
            Ok(records) => {
                for record in records {
                    if record.status.is_terminal() {
                        continue;
                    }
                    *opts.pinned_layers.entry(record.layer_head).or_insert(0) += 1;
                }
            }
            Err(e) => return Err(Status::internal(format!("list_tasks failed: {e}"))),
        }
    }
    Ok(opts)
}

/// Convert a `ConsolidateError` into the wire response. Both
/// `ConsolidateChain` and `EstimateConsolidation` use the same kind
/// enum + offending-layer/count fields; the two helpers differ only
/// in response shape (success-path fields are zeroed).
fn consolidate_error_to_response(err: crate::layer::ConsolidateError) -> ConsolidateChainResponse {
    let (kind, error_layer, error_count) = consolidate_error_parts(&err);
    ConsolidateChainResponse {
        success: false,
        consolidated_layer: String::new(),
        collapsed_layer_count: 0,
        head_advanced: false,
        error_kind: kind as i32,
        error: err.to_string(),
        error_layer,
        error_count,
    }
}

fn estimate_error_to_response(
    err: crate::layer::ConsolidateError,
) -> EstimateConsolidationResponse {
    let (kind, error_layer, error_count) = consolidate_error_parts(&err);
    EstimateConsolidationResponse {
        success: false,
        predicted_consolidated_layer: String::new(),
        collapsed_layer_count: 0,
        predicted_walk_entries: 0,
        actual_walk_entries: 0,
        error_kind: kind as i32,
        error: err.to_string(),
        error_layer,
        error_count,
    }
}

fn consolidate_error_parts(
    err: &crate::layer::ConsolidateError,
) -> (ConsolidateErrorKind, String, u64) {
    use crate::layer::ConsolidateError as E;
    match err {
        E::RangeNotAncestral { from, .. } => (
            ConsolidateErrorKind::RangeNotAncestral,
            hex::encode(from.0),
            0,
        ),
        E::BranchAdvancedConcurrently { observed_head, .. } => (
            ConsolidateErrorKind::BranchAdvanced,
            observed_head
                .as_ref()
                .map(|h| hex::encode(h.0))
                .unwrap_or_default(),
            0,
        ),
        E::RangeContainsMergeNode { merge_layer } => (
            ConsolidateErrorKind::RangeContainsMergeNode,
            hex::encode(merge_layer.0),
            0,
        ),
        E::RangeContainsTracePin {
            pinned_layer,
            trace_count,
        } => (
            ConsolidateErrorKind::RangeContainsTracePin,
            hex::encode(pinned_layer.0),
            *trace_count,
        ),
        E::CostExceedsCap { predicted_entries } => (
            ConsolidateErrorKind::CostExceedsCap,
            String::new(),
            *predicted_entries,
        ),
        E::ToNotReachableFromHead { observed_head, .. } => (
            ConsolidateErrorKind::ToNotReachableFromHead,
            hex::encode(observed_head.0),
            0,
        ),
        E::RangeCrossesExistingRedirect { offending_layer } => (
            ConsolidateErrorKind::RangeCrossesExistingRedirect,
            hex::encode(offending_layer.0),
            0,
        ),
        E::WriteFailed(_) | E::Internal(_) => (ConsolidateErrorKind::Internal, String::new(), 0),
    }
}

// `NotebookService` is defined in the proto and generates Rust server
// stubs here, but the kernel does not implement it — the orchestrator
// implements `NotebookService` in TypeScript (D22 §3.2 / §4) and proxies
// to `EigeniusKernel.LayerTopology` above. The Rust stubs exist for
// future symmetry / testability and incur no compile-time obligation.

/// Convert a `TaskRecord` to the gRPC `TaskInfo` view.
fn task_record_to_info(record: crate::task::TaskRecord) -> TaskInfo {
    TaskInfo {
        task_id: record.task_id.to_string(),
        session_id: record.session_id.to_string(),
        program_iri: record.program_iri,
        input_iri: record.input_iri,
        status: format!("{:?}", record.status),
        layer_head: hex::encode(record.layer_head.0),
        step_seq: record.step_seq,
        latest_trace_seq: record.latest_trace_seq,
        last_checkpoint_step: record
            .last_checkpoint
            .map(|n| n.to_string())
            .unwrap_or_default(),
        result_layer_head: record
            .result_layer_head
            .map(|id| hex::encode(id.0))
            .unwrap_or_default(),
        created_at_ms: record.created_at,
        updated_at_ms: record.updated_at,
        retain_forever: record.retain_forever,
    }
}

/// Known remote component IRIs that should be dispatched to the orchestrator.
const REMOTE_COMPONENTS: &[&str] = &[
    "urn:eigenius:program:components:CompleteText",
    "urn:eigenius:program:components:CompleteJson",
    "urn:eigenius:program:components:HttpRequest",
];

/// Start the gRPC server on the given port.
///
/// If `orchestrator_endpoint` is provided, remote components are registered
/// that dispatch IO calls to the orchestrator via ComponentExecutor gRPC.
///
/// If `backend` is `Some`, the server runs in durable mode: layers, traces
/// and WASM capabilities survive restart. An empty backend is seeded with
/// the embedded ontologies; a populated one is rehydrated. See D13.
pub async fn start_server(
    port: u16,
    orchestrator_endpoint: Option<&str>,
    backend: Option<Arc<dyn crate::storage::PersistentBackend>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{port}").parse()?;

    let mut registry = ComponentRegistry::default();
    let mut orchestrator_client: Option<crate::program::remote::SharedOrchestratorClient> = None;

    if let Some(endpoint) = orchestrator_endpoint {
        tracing::info!(
            { field::OPERATION } = operation::SERVER_START,
            endpoint = %endpoint,
            "connecting to orchestrator"
        );
        match crate::program::remote::connect_orchestrator(endpoint, REMOTE_COMPONENTS).await {
            Ok((client, components)) => {
                for (iri, component) in components {
                    tracing::info!(
                        { field::OPERATION } = operation::CAPABILITY_INSTALL,
                        { field::COMPONENT_IRI } = %iri,
                        host = "orchestrator",
                        "registered remote component"
                    );
                    registry.register(iri, component);
                }
                orchestrator_client = Some(client);
            }
            Err(e) => {
                tracing::warn!(
                    { field::OPERATION } = operation::SERVER_START,
                    { field::ERROR_KIND } = "orchestrator_connect_failed",
                    { field::ERROR_MESSAGE } = %e,
                    "failed to connect to orchestrator; IO components will not be available"
                );
            }
        }
    }

    let (mut service, is_persistent) = match backend {
        Some(b) => {
            tracing::info!(
                { field::OPERATION } = operation::SERVER_START,
                mode = "persistent",
                "persistent backend attached; using SEED-or-RESUME bootstrap (D13)"
            );
            (EigeniusService::with_persistent_backend(registry, b)?, true)
        }
        None => {
            tracing::info!(
                { field::OPERATION } = operation::SERVER_START,
                mode = "in-memory",
                "in-memory mode (no --db); all state lost on exit"
            );
            (EigeniusService::with_components(registry)?, false)
        }
    };
    if let Some(client) = orchestrator_client {
        service = service.with_orchestrator_client(client);
    }

    // On a persistent backend, walk the rehydrated chain and
    // re-register every WASM capability it finds. Institutions go
    // through `register_rehydrated` (doesn't re-publish classes).
    if is_persistent {
        let errors = service.rehydrate_wasm_from_chain().await;
        for e in errors {
            tracing::warn!(
                { field::OPERATION } = operation::CAPABILITY_INSTALL,
                { field::ERROR_KIND } = "rehydrate_failed",
                { field::RESOURCE_IRI } = %e.resource_iri,
                { field::ERROR_MESSAGE } = %e.message,
                "WASM rehydrate produced an error"
            );
        }
    }

    // Build the D14 institution index from the bootstrap / rehydrated
    // chain so subsequent Loads dispatch AutoOnLoad QueryClasses
    // declared in the persisted chain.
    let ctx_arc = service
        .get_branch_context(DEFAULT_BRANCH)
        .await
        .expect("default branch context");
    let head = Arc::clone(ctx_arc.read().await.head());
    service.rebuild_institution_index(&head).await;

    // Background task resume sweep (D21 §6). Runs detached so the
    // gRPC listener is available immediately; clients can poll
    // `Health.resume_in_progress` / `tasks_resuming` to see when
    // pre-crash tasks have finished draining.
    if let Some(inputs) = service.resume_inputs() {
        let session_id = service.session_id().await;
        let components = service.components_snapshot().await;
        tokio::spawn(resume_sweep(
            inputs,
            session_id,
            components,
            ResumeConfig::default(),
        ));
    }

    tracing::info!(
        { field::OPERATION } = operation::SERVER_START,
        addr = %addr,
        "gRPC server listening"
    );

    // Raise gRPC message size limits to 128 MB to accommodate WASM component
    // binaries (which are base64-encoded and can be multiple MB).
    tonic::transport::Server::builder()
        .add_service(
            service
                .into_server()
                .max_decoding_message_size(128 * 1024 * 1024)
                .max_encoding_message_size(128 * 1024 * 1024),
        )
        .serve(addr)
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// BackendTraceStore — forwards TraceStore calls to a PersistentBackend.
// Lets the service hold `Arc<dyn TraceStore>` without needing to hand out
// two Arc types of the same RocksStore.
// ---------------------------------------------------------------------------

struct BackendTraceStore {
    backend: Arc<dyn crate::storage::PersistentBackend>,
}

impl BackendTraceStore {
    fn new(backend: Arc<dyn crate::storage::PersistentBackend>) -> Self {
        Self { backend }
    }
}

impl TraceStore for BackendTraceStore {
    fn get_component_trace(&self, key: &[u8; 32]) -> Option<crate::program::trace::ComponentTrace> {
        self.backend.as_trace_store().get_component_trace(key)
    }

    fn put_component_trace(&self, key: [u8; 32], trace: crate::program::trace::ComponentTrace) {
        self.backend
            .as_trace_store()
            .put_component_trace(key, trace);
    }
}

#[cfg(test)]
mod merge_info_tests {
    //! Unit tests for [`merge_info_from_persist_info`]. The lattice
    //! tests already pin the three [`UpdateOutcome`] variants on the
    //! production side; these tests pin the **conversion** to the
    //! proto wire format — making sure each persist-info shape maps
    //! to the correct enum value with the right side fields populated.
    //!
    //! Combined with the e2e tests in `storage/rocksdb/tests/`, this
    //! gives us defense in depth against regressions of D34 §G.1's
    //! silent-`NeedsWitnessedMerge` bug and the cache-hit conflation
    //! the `CachedDifferentPosition` variant resolves.
    use super::*;
    use crate::lattice::UpdateOutcome;
    use crate::layer::LayerId;
    use crate::ontology::iri::Iri;
    use hex::encode as hex_encode;
    use proto::MergeOutcome;
    fn lid(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }
    fn pli(
        layer_id: LayerId,
        branch_advanced: bool,
        merge_outcome: Option<UpdateOutcome>,
        cache_hit_different_position: bool,
    ) -> PersistedLayerInfo {
        PersistedLayerInfo {
            layer_id,
            branch_advanced,
            merge_outcome,
            cache_hit_different_position,
        }
    }
    #[test]
    fn no_persist_info_maps_to_unspecified_with_empty_fields() {
        let info = merge_info_from_persist_info(None);
        assert_eq!(info.outcome, MergeOutcome::Unspecified as i32);
        assert!(info.merge_layer_id.is_empty());
        assert!(info.conflicting_iris.is_empty());
        assert!(info.current_head.is_empty());
    }
    #[test]
    fn no_cas_with_no_cache_hit_maps_to_unspecified() {
        // The no-backend path: persist ran, returned no merge_outcome,
        // and didn't hit the anchored-commit cache.
        let pi = pli(lid(0xFF), false, None, false);
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::Unspecified as i32);
        assert!(info.merge_layer_id.is_empty());
    }
    #[test]
    fn fast_forward_maps_with_empty_side_fields() {
        let pi = pli(lid(0x01), true, Some(UpdateOutcome::FastForward), false);
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::FastForward as i32);
        assert!(info.merge_layer_id.is_empty());
        assert!(info.conflicting_iris.is_empty());
        assert!(info.current_head.is_empty());
    }
    #[test]
    fn trivial_merge_carries_merge_layer_id_as_hex() {
        let merge_layer = lid(0xAB);
        let pi = pli(
            lid(0x01),
            true,
            Some(UpdateOutcome::TrivialMerge {
                merge_layer: merge_layer.clone(),
            }),
            false,
        );
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::TrivialMerge as i32);
        assert_eq!(info.merge_layer_id, hex_encode(merge_layer.0));
        assert!(info.conflicting_iris.is_empty());
        assert!(info.current_head.is_empty());
    }
    #[test]
    fn needs_witnessed_merge_carries_head_and_iris() {
        let current_head = lid(0xCD);
        let conflicting_iris = vec![
            Iri::parse("urn:eigenius:demo:A").unwrap(),
            Iri::parse("urn:eigenius:demo:B").unwrap(),
        ];
        let pi = pli(
            lid(0x01),
            false,
            Some(UpdateOutcome::NeedsWitnessedMerge {
                current_head: current_head.clone(),
                conflicting_iris: conflicting_iris.clone(),
            }),
            false,
        );
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::NeedsWitnessedMerge as i32);
        assert!(info.merge_layer_id.is_empty());
        assert_eq!(info.current_head, hex_encode(current_head.0));
        assert_eq!(
            info.conflicting_iris,
            vec![
                "urn:eigenius:demo:A".to_string(),
                "urn:eigenius:demo:B".to_string()
            ]
        );
    }
    #[test]
    fn cache_hit_different_position_maps_with_cached_layer_id() {
        // Distinct from `UNSPECIFIED`: the persist short-circuited
        // because the content is canonical at the carried layer_id,
        // and the branch ref did **not** advance.
        let cached = lid(0x77);
        let pi = pli(cached.clone(), false, None, true);
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::CachedDifferentPosition as i32);
        assert_eq!(info.merge_layer_id, hex_encode(cached.0));
        assert!(info.conflicting_iris.is_empty());
        assert!(info.current_head.is_empty());
    }
    #[test]
    fn cache_hit_flag_dominates_over_any_merge_outcome() {
        // Defensive: if a caller ever sets both `cache_hit_different_position`
        // and `merge_outcome=Some(...)`, the cache-hit signal wins.
        // `persist_layer_if_backend` doesn't actually produce that
        // combination today, but pinning the precedence keeps the
        // mapping unambiguous.
        let cached = lid(0x55);
        let pi = pli(
            cached.clone(),
            false,
            Some(UpdateOutcome::FastForward),
            true,
        );
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::CachedDifferentPosition as i32);
        assert_eq!(info.merge_layer_id, hex_encode(cached.0));
    }
}
