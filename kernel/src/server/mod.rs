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
use crate::context::ExecutionContext;
use crate::observability::{field, operation, RpcGuard};
use crate::ontology::{eigon_cbor, eigon_json, Iri, Resource};
use crate::program::component::ComponentRegistry;
use crate::program::expr;
use crate::program::trace::{InMemoryTraceStore, TraceStore};
use crate::query;
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
    // Rehydrate the pinned layer chain from the backend.
    let layer = match backend.load_chain_from(&record.layer_head) {
        Ok(Some(l)) => l,
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
        .and_then(|i| layer.resolve(&i).cloned())
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
        .and_then(|i| layer.resolve(&i).cloned())
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

    let result = crate::program::eval_io::execute_program_nbe_with_institutions(
        &program,
        &input,
        layer,
        components,
        Arc::new(crate::institution::InstitutionRegistry::new()),
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
pub struct EigeniusService {
    context: Arc<RwLock<ExecutionContext>>,
    /// Outer lock allows swapping the registry (for WASM registration on load).
    /// Inner Arc allows cheap cloning for passing to the evaluator.
    components: Arc<RwLock<Arc<ComponentRegistry>>>,
    trace_store: Arc<dyn TraceStore>,
    institutions: Arc<RwLock<crate::institution::InstitutionRegistry>>,
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
            context: Arc::new(RwLock::new(ctx)),
            components: Arc::new(RwLock::new(Arc::new(components))),
            trace_store: Arc::new(InMemoryTraceStore::new()),
            institutions: Arc::new(RwLock::new(crate::institution::InstitutionRegistry::new())),
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
        let ctx = bootstrap::bootstrap_persistent(backend.as_ref())
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
            context: Arc::new(RwLock::new(ctx)),
            components: Arc::new(RwLock::new(Arc::new(components))),
            trace_store,
            institutions: Arc::new(RwLock::new(crate::institution::InstitutionRegistry::new())),
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
            context: Arc::new(RwLock::new(ctx)),
            components: Arc::new(RwLock::new(Arc::new(components))),
            trace_store,
            institutions: Arc::new(RwLock::new(crate::institution::InstitutionRegistry::new())),
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

    /// Persist a freshly-committed layer through the backend, if one is
    /// attached. No-op otherwise. See D13 §5.
    ///
    /// Returns a validation-like error on storage failure so the caller
    /// can surface it to clients without crashing the server.
    /// Resolve the target layer for a read RPC (D21 §3.6 `at_layer`).
    ///
    /// Empty / invalid hex falls back to the session's active top
    /// (`context.head()`). When `at_layer` is set and a backend is
    /// attached, reconstructs the layer chain rooted at that id. Errors
    /// propagate as `Status::invalid_argument` / `Status::not_found`
    /// so the client sees a clear failure rather than a silent fallback.
    async fn resolve_read_layer(&self, at_layer: &str) -> Result<Arc<crate::layer::Layer>, Status> {
        if at_layer.is_empty() {
            let ctx = self.context.read().await;
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
            Ok(Some(layer)) => Ok(layer),
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

    fn persist_layer_if_backend(&self, layer: &crate::layer::Layer) -> Option<ValidationError> {
        let backend = self.backend.as_ref()?;
        if let Err(e) = backend.store_layer(layer) {
            tracing::warn!(
                { field::OPERATION } = operation::LAYER_COMMIT,
                { field::ERROR_KIND } = "persist_layer_failed",
                { field::LAYER_ID } = %layer.id(),
                { field::ERROR_MESSAGE } = %e,
                "failed to persist layer to backend"
            );
            return Some(ValidationError {
                resource_iri: String::new(),
                property_iri: String::new(),
                rule: "persist_layer".to_string(),
                message: format!("{e}"),
                severity: "error".to_string(),
            });
        }
        if let Err(e) = backend.set_head(layer.id()) {
            tracing::warn!(
                { field::OPERATION } = operation::LAYER_COMMIT,
                { field::ERROR_KIND } = "persist_head_failed",
                { field::LAYER_ID } = %layer.id(),
                { field::ERROR_MESSAGE } = %e,
                "failed to advance persisted head"
            );
            return Some(ValidationError {
                resource_iri: String::new(),
                property_iri: String::new(),
                rule: "persist_head".to_string(),
                message: format!("{e}"),
                severity: "error".to_string(),
            });
        }
        tracing::debug!(
            { field::OPERATION } = operation::LAYER_COMMIT,
            { field::LAYER_ID } = %layer.id(),
            persisted = true,
            "layer persisted to backend and head advanced"
        );
        None
    }

    /// Parse resources from CBOR, JSON, or ESL based on content_type.
    #[allow(clippy::result_large_err)]
    fn parse_resources(data: &[u8], content_type: &str) -> Result<Vec<Resource>, Status> {
        if content_type.contains("cbor") {
            eigon_cbor::parse_document(data)
                .map_err(|e| Status::invalid_argument(format!("CBOR parse error: {e}")))
        } else if content_type.contains("esl") {
            let source = std::str::from_utf8(data)
                .map_err(|e| Status::invalid_argument(format!("invalid UTF-8: {e}")))?;
            crate::esl::compile(source).map_err(|errors| {
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
        *self.institution_index.write().await = Arc::new(idx);
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
        let head = {
            let ctx = self.context.read().await;
            Arc::clone(ctx.head())
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
        program: Resource,
        input: Resource,
    ) -> Result<Response<RunProgramResponse>, Status> {
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
                    let ctx = self.context.read().await;
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
            let ctx = self.context.read().await;
            let components = Arc::clone(&*self.components.read().await);
            let index = Arc::clone(&*self.institution_index.read().await);
            let runtime = Arc::clone(&*self.institution_runtime.read().await);
            // Pass an empty legacy institution registry — fiber queries
            // go through the FiberQuery RPC, not through program
            // dispatch. The D14 index + runtime carry institution
            // dispatch on the new path.
            match crate::program::eval_io::execute_program_nbe_with_institutions_d14(
                &program,
                &input,
                Arc::clone(ctx.head()),
                components,
                Arc::new(crate::institution::InstitutionRegistry::new()),
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
                    }));
                }
            }
        };

        let completed_at_ms = now_millis();
        let mut output = exec_result.output;
        let dispatched_traces = exec_result.dispatched_traces;
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

        // Auto-commit trace layer: ProgramTrace + all IO ComponentTraces
        let result_layer_head = {
            let mut ctx = self.context.write().await;
            // Add ProgramTrace
            if let Err(e) = ctx.add_resource(trace_resource) {
                tracing::warn!(
                    { field::OPERATION } = operation::PROGRAM_RUN,
                    { field::ERROR_KIND } = "trace_add_failed",
                    { field::ERROR_MESSAGE } = %e,
                    "failed to add ProgramTrace resource to layer"
                );
            }
            // Add each IO ComponentTrace as a resource
            for ct in &dispatched_traces {
                let ct_resource = crate::program::trace::trace_to_resource(
                    &crate::program::trace::Trace::Component(ct.clone()),
                );
                let _ = ctx.add_resource(ct_resource);
            }
            match ctx.commit("trace") {
                Ok(layer) => {
                    // Best-effort persist of the trace layer. A failure here
                    // logs but doesn't fail the RunProgram call — the output
                    // is still valid, the trace just isn't durable.
                    if let Some(err) = self.persist_layer_if_backend(&layer) {
                        tracing::warn!(
                            { field::OPERATION } = operation::LAYER_COMMIT,
                            { field::ERROR_KIND } = "trace_persist_failed",
                            { field::LAYER_ID } = %layer.id(),
                            { field::ERROR_MESSAGE } = %err.message,
                            "failed to persist trace layer (output still returned)"
                        );
                    }
                    Some(layer.id().clone())
                }
                Err(e) => {
                    tracing::warn!(
                        { field::OPERATION } = operation::LAYER_COMMIT,
                        { field::ERROR_KIND } = "trace_commit_failed",
                        { field::ERROR_MESSAGE } = %e,
                        "trace layer commit failed (output still returned)"
                    );
                    None
                }
            }
        };

        // Mark the task Completed and record its result_layer_head so
        // clients that polled via GetTaskStatus can resolve it (D21
        // §3.7). `result_layer_head` is the trace layer committed
        // above — the program's observable outputs.
        if let (Some(store), Some(tc)) = (&self.task_store, task_context.as_ref()) {
            if let Ok(Some(mut rec)) = store.get_task(&tc.session_id, &tc.task_id) {
                rec.status = crate::task::TaskStatus::Completed;
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

        Ok(Response::new(RunProgramResponse {
            success: true,
            output: Self::serialize_resource(&output),
            errors: Vec::new(),
            trace_iri: trace_iri_str,
            task_id: task_id_str,
        }))
    }
}

#[allow(clippy::result_large_err)]
#[tonic::async_trait]
impl EigeniusKernel for EigeniusService {
    async fn load(&self, request: Request<LoadRequest>) -> Result<Response<LoadResponse>, Status> {
        let mut guard = RpcGuard::start(operation::RPC_LOAD);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_LOAD,
            { field::CONTENT_TYPE } = %req.content_type,
            { field::SIZE_BYTES } = req.resources.len(),
            "load payload"
        );
        let resources = Self::parse_resources(&req.resources, &req.content_type)?;
        let count = resources.len() as u32;

        let mut ctx = self.context.write().await;
        for resource in resources {
            ctx.add_resource(resource)
                .map_err(|e| Status::failed_precondition(format!("load error: {e}")))?;
        }

        let mut layer_id = String::new();
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
                Ok(layer) => {
                    layer_id = layer.id().to_string();
                    tracing::info!(
                        { field::OPERATION } = operation::LAYER_COMMIT,
                        { field::LAYER_ID } = %layer_id,
                        { field::COUNT } = count,
                        "layer committed"
                    );
                    drop(ctx);
                    if let Some(err) = self.persist_layer_if_backend(&layer) {
                        errors.push(err);
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
                        let mut ctx = self.context.write().await;
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
                            match ctx.commit("institution_classes") {
                                Ok(extra) => {
                                    drop(ctx);
                                    if let Some(err) = self.persist_layer_if_backend(&extra) {
                                        errors.push(err);
                                    }
                                    self.rebuild_institution_index(&extra).await;
                                }
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
                Err(crate::context::ContextError::ValidationFailed(verrs)) => {
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

        let response = LoadResponse {
            success: errors.is_empty(),
            errors,
            layer_id,
            resource_count: count,
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

        let layer = self.resolve_read_layer(&req.at_layer).await?;
        match layer.resolve(&iri) {
            Some(resource) => Ok(Response::new(InspectResponse {
                found: true,
                resource: Self::serialize_resource(resource),
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
        let layer = self.resolve_read_layer(&req.at_layer).await?;
        let ctx = self.context.read().await;
        let index = Arc::clone(&*self.institution_index.read().await);
        let inst_runtime = Arc::clone(&*self.institution_runtime.read().await);

        let runtime = query::evaluate::FiberRuntime {
            index: Some(&index),
            runtime: Some(&inst_runtime),
            ctx: Some(&ctx),
        };

        let document = match query::execute_with(&req.eigenql, &layer, runtime) {
            Ok(doc) => doc,
            Err(errors) => {
                let msgs: Vec<String> = errors.iter().map(|e| format!("{e}")).collect();
                guard.fail("query_failed");
                tracing::warn!(
                    { field::OPERATION } = operation::QUERY_EVALUATE,
                    { field::COUNT } = errors.len(),
                    { field::ERROR_MESSAGE } = %msgs.join("; "),
                    "query failed"
                );
                return Ok(Response::new(QueryResponse {
                    success: false,
                    document: Vec::new(),
                    content_type: String::new(),
                    error: format!("query error: {}", msgs.join("; ")),
                }));
            }
        };

        Ok(Response::new(QueryResponse {
            success: true,
            document: eigon_cbor::serialize_document(&document),
            content_type: "application/cbor".to_string(),
            error: String::new(),
        }))
    }

    async fn validate_program(
        &self,
        request: Request<ValidateProgramRequest>,
    ) -> Result<Response<ValidateProgramResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_VALIDATE_PROGRAM);
        let req = request.into_inner();
        let resources = Self::parse_resources(&req.program, &req.content_type)?;
        let program = resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no program resource"))?;

        let ctx = self.context.read().await;

        match expr::parse_program(&program, ctx.head()) {
            Ok((_term, typ)) => {
                // Validate template references against input type
                let mut template_errors = Vec::new();
                let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
                let input_type_prop = Iri::parse("urn:eigenius:program:input_type").unwrap();
                if let (
                    Some(crate::ontology::resource::Value::String(input_type_str)),
                    Some(crate::ontology::resource::Value::Embedded(body)),
                ) = (program.get(&input_type_prop), program.get(&body_prop))
                {
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
        let program_resources = Self::parse_resources(&req.program, &req.content_type)?;
        let program = program_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no program resource"))?;

        let input_resources = Self::parse_resources(&req.input, &req.content_type)?;
        let input = input_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no input resource"))?;

        self.execute_program(program, input).await
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

        let layer = self.resolve_read_layer(&req.at_layer).await?;
        let program = layer
            .resolve(&program_iri)
            .ok_or_else(|| {
                Status::not_found(format!("program resource not found: {}", req.program_iri))
            })?
            .clone();
        let input = layer
            .resolve(&input_iri)
            .ok_or_else(|| {
                Status::not_found(format!("input resource not found: {}", req.input_iri))
            })?
            .clone();

        self.execute_program(program, input).await
    }

    async fn reflect(
        &self,
        request: Request<ReflectRequest>,
    ) -> Result<Response<ReflectResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_REFLECT);
        let req = request.into_inner();
        let resources = Self::parse_resources(&req.trace, &req.content_type)?;

        if resources.is_empty() {
            return Ok(Response::new(ReflectResponse {
                success: false,
                trace_iri: String::new(),
            }));
        }

        // The first resource should be a trace (ProgramTrace, DeclarationTrace, etc.)
        let trace_resource = &resources[0];
        let trace_iri = trace_resource
            .id()
            .map(|i| i.as_str().to_string())
            .unwrap_or_default();

        // Commit all trace resources to a new layer
        let mut ctx = self.context.write().await;
        for resource in resources {
            ctx.add_resource(resource)
                .map_err(|e| Status::failed_precondition(format!("reflect error: {e}")))?;
        }
        let layer = ctx
            .commit("reflect")
            .map_err(|e| Status::internal(format!("reflect commit failed: {e}")))?;
        if let Some(err) = self.persist_layer_if_backend(&layer) {
            return Err(Status::internal(format!(
                "reflect persist failed: {}",
                err.message
            )));
        }

        Ok(Response::new(ReflectResponse {
            success: true,
            trace_iri,
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
        let ctx = self.context.read().await;
        let all = ctx.head().all_resources();

        // D21 §6 resume observability — populated by the resume
        // sweep when it's active.
        use std::sync::atomic::Ordering;
        let resume_in_progress = self.resume_state.in_progress.load(Ordering::SeqCst);
        let tasks_resuming = self.resume_state.remaining.load(Ordering::SeqCst);

        Ok(Response::new(HealthResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            layer_count: 2, // core + program ontology
            resource_count: all.len() as u64,
            resume_in_progress,
            tasks_resuming,
        }))
    }

    async fn fiber_query(
        &self,
        request: Request<FiberQueryRequest>,
    ) -> Result<Response<FiberQueryResponse>, Status> {
        let mut guard = RpcGuard::start(operation::RPC_FIBER_QUERY);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_FIBER_QUERY,
            { field::INSTITUTION_IRI } = %req.institution_iri,
            "fiber_query target"
        );
        let inst_iri = Iri::parse(&req.institution_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid institution IRI: {e}")))?;

        let institutions = self.institutions.read().await;
        let reasoner = institutions
            .get(&inst_iri)
            .ok_or_else(|| Status::not_found(format!("institution not found: {inst_iri}")))?;

        let query_resources = Self::parse_resources(&req.query, &req.content_type)?;
        let query = query_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no query resource"))?;

        let ctx = self.context.read().await;
        match reasoner.query(&query, &ctx) {
            Ok(result) => Ok(Response::new(FiberQueryResponse {
                success: true,
                result: Self::serialize_resource(&result),
                error: String::new(),
            })),
            Err(e) => {
                guard.fail("fiber_query_failed");
                Ok(Response::new(FiberQueryResponse {
                    success: false,
                    result: Vec::new(),
                    error: format!("{e}"),
                }))
            }
        }
    }

    async fn discover_morphisms(
        &self,
        request: Request<DiscoverMorphismsRequest>,
    ) -> Result<Response<DiscoverMorphismsResponse>, Status> {
        let mut guard = RpcGuard::start(operation::RPC_DISCOVER_MORPHISMS);
        let req = request.into_inner();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_DISCOVER_MORPHISMS,
            { field::INSTITUTION_IRI } = %req.institution_iri,
            "discover_morphisms target"
        );
        let inst_iri = Iri::parse(&req.institution_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid institution IRI: {e}")))?;

        let institutions = self.institutions.read().await;
        let reasoner = institutions
            .get(&inst_iri)
            .ok_or_else(|| Status::not_found(format!("institution not found: {inst_iri}")))?;

        let mut resources = Vec::new();
        for data in &req.resources {
            let parsed = Self::parse_resources(data, &req.content_type)?;
            resources.extend(parsed);
        }

        let ctx = self.context.read().await;
        match reasoner.discover_morphisms(&resources, &ctx) {
            Ok(morphisms) => {
                let serialized: Vec<Vec<u8>> =
                    morphisms.iter().map(Self::serialize_resource).collect();
                Ok(Response::new(DiscoverMorphismsResponse {
                    success: true,
                    morphisms: serialized,
                    error: String::new(),
                }))
            }
            Err(e) => {
                guard.fail("discover_morphisms_failed");
                Ok(Response::new(DiscoverMorphismsResponse {
                    success: false,
                    morphisms: Vec::new(),
                    error: format!("{e}"),
                }))
            }
        }
    }

    async fn list_institutions(
        &self,
        request: Request<ListInstitutionsRequest>,
    ) -> Result<Response<ListInstitutionsResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LIST_INSTITUTIONS);
        // `at_layer` is accepted but currently has no effect —
        // institutions live in a kernel-global runtime registry, not
        // in the layer chain. If Phase 14 ever introduces per-session
        // institution scoping, this is where the layer-aware lookup
        // would branch. For 9b-iii we validate + ignore.
        let req = request.into_inner();
        if !req.at_layer.is_empty() {
            let _ = self.resolve_read_layer(&req.at_layer).await?;
        }
        let institutions = self.institutions.read().await;
        let infos: Vec<proto::InstitutionInfo> = institutions
            .list()
            .iter()
            .map(|info| proto::InstitutionInfo {
                iri: info.iri.as_str().to_string(),
                name: info.name.clone(),
                morphism_types: info
                    .morphism_type_iris
                    .iter()
                    .map(|i| i.as_str().to_string())
                    .collect(),
                query_types: info
                    .query_type_iris
                    .iter()
                    .map(|i| i.as_str().to_string())
                    .collect(),
            })
            .collect();

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

        let layer = self.resolve_read_layer(&req.at_layer).await?;
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
        let layer = self.resolve_read_layer(&req.root_layer).await?;
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
    let head = Arc::clone(service.context.read().await.head());
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
