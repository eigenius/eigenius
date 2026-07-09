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

//! Server lifecycle: [`start_server`] (process entrypoint), the
//! background [`resume_sweep`] (D21 §6), and [`BackendTraceStore`]
//! (a small `TraceStore`-over-`PersistentBackend` adapter the service
//! holds onto). None of this is RPC-handler logic; it sits next to
//! the handler files so the module surface stays self-contained.

use super::helpers::DEFAULT_BRANCH;
use super::EigeniusService;
use crate::observability::{field, operation};
use crate::ontology::{Iri, Resource};
use crate::program::component::ComponentRegistry;
use crate::program::trace::TraceStore;
use std::sync::Arc;

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
    use super::helpers::now_millis;
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

    let result = crate::program::eval_io::execute_program_nbe_with_institutions(
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
    record.updated_at = super::helpers::now_millis();
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

/// Known remote component IRIs that should be dispatched to the orchestrator.
const REMOTE_COMPONENTS: &[&str] = &[
    "urn:eigenius:program:components:CompleteText",
    "urn:eigenius:program:components:CompleteJson",
    "urn:eigenius:program:components:HttpRequest",
    // Substrate-backed script execution (D26 §4.1): the orchestrator's
    // handler routes it to `dispatchRunRuntimeScript` → `SubstrateDispatcher`
    // → the language runtime (e.g. R/lme4). A program applies it with the
    // input table as component input and the `RuntimeScript` (+ env) as the
    // component argument; the run's `ProgramTrace` mints the `IsDerivedAs`
    // witness over the output (D56 §3.1).
    "urn:eigenius:program:components:RunRuntimeScript",
];

/// Embedder-side configuration handed to [`start_server`] by the
/// orchestrator. Kept here (not in `eigenius-config`) so the kernel
/// crate stays config-crate-independent — the CLI's `cmd_serve`
/// translates the loaded TOML into this struct at the call site.
pub struct EmbedderStartupConfig {
    /// Constructed embedders, ready to register. Empty → no
    /// embedders → vector retrieval is unavailable; the service
    /// still starts unless `fail_fast_on_missing_model` is set and
    /// the bootstrap/rehydrated head declares an active VectorIndex.
    pub embedders: Vec<Arc<dyn crate::program::embedder::Embedder>>,
    /// Per-sweep batch size — [`crate::query::vector::indexing::DEFAULT_BATCH_SIZE`]
    /// if unsure. Forwarded to every
    /// [`crate::task::sweep::VectorSweepDriver`] the
    /// [`crate::task::sweep_registry::SweepCoordinator`] spawns.
    pub batch_size: usize,
    /// If `true`, the service refuses to start when the
    /// bootstrap/rehydrated head declares any active VectorIndex
    /// Resource whose `vec_model` IRI is not in `embedders`. If
    /// `false`, missing models surface at query time.
    pub fail_fast_on_missing_model: bool,
}

impl Default for EmbedderStartupConfig {
    fn default() -> Self {
        Self {
            embedders: Vec::new(),
            batch_size: crate::query::vector::indexing::DEFAULT_BATCH_SIZE,
            fail_fast_on_missing_model: true,
        }
    }
}

/// Start the gRPC server on the given port.
///
/// If `orchestrator_endpoint` is provided, remote components are registered
/// that dispatch IO calls to the orchestrator via ComponentExecutor gRPC.
///
/// If `backend` is `Some`, the server runs in durable mode: layers, traces
/// and institution registrations survive restart. An empty backend is seeded
/// with the embedded ontologies; a populated one is rehydrated. See D13.
///
/// `embedders` carries the registered Embedder Components (D43 §5.2);
/// pass [`EmbedderStartupConfig::default`] (empty) when vector
/// retrieval isn't wanted.
pub async fn start_server(
    port: u16,
    orchestrator_endpoint: Option<&str>,
    backend: Option<Arc<dyn crate::storage::PersistentBackend>>,
    in_process_institutions: Vec<Arc<dyn crate::institution::runtime::Institution>>,
    embedders: EmbedderStartupConfig,
    parse_config: super::ParseConfig,
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

    let (mut service, _is_persistent) = match backend {
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

    // D43 §5.2 — install the configured embedder pool. The
    // coordinator wraps the registry; subsequent post-Load sweeps
    // dispatch through it. Empty pool = no coordinator installed, and
    // the `didPersist` hook becomes a no-op.
    if !embedders.embedders.is_empty() {
        let mut registry = crate::program::embedder::EmbedderRegistry::new();
        for e in embedders.embedders {
            tracing::info!(
                { field::OPERATION } = operation::CAPABILITY_INSTALL,
                model_iri = %e.model_iri(),
                dim = e.dim(),
                "registered embedder"
            );
            registry.register(e);
        }
        service = service.with_embedders(registry, embedders.batch_size);
    }

    // D63/GH#97 Lever 1 — install the ParseSentence parse config (lemmatizer + cap/beam + opt-in
    // reranker). The binary injects a real lemmatizer here (the kernel can't depend on WordNet).
    service = service.with_parse_config(parse_config);

    // Phase 20a.1+: pre-register every in-process institution the
    // binary links (Lean today, future verification institutions
    // tomorrow). Must happen before the institution-index rebuild so
    // the chain-scan registration pass sees them when it walks
    // `runtime: in_process` declarations.
    for institution in in_process_institutions {
        tracing::info!(
            { field::OPERATION } = operation::CAPABILITY_INSTALL,
            { field::INSTITUTION_IRI } = %institution.institution_iri(),
            host = "in_process",
            "registered in-process institution"
        );
        service.register_in_process_institution(institution);
    }

    // Build the institution index from the bootstrap / rehydrated
    // chain so subsequent Loads dispatch AutoOnLoad QueryClasses
    // declared in the persisted chain.
    let ctx_arc = service
        .get_branch_context(DEFAULT_BRANCH)
        .await
        .expect("default branch context");
    let head = Arc::clone(ctx_arc.read().await.head());
    service.rebuild_institution_index(&head).await;

    // D43 §5.2 — fail-fast: refuse to start if any active VectorIndex
    // Resource visible at the bootstrap / rehydrated head declares a
    // `vec_model` IRI for which no embedder is registered. A service
    // that quietly runs without the embedders its schema declares
    // would be a silent correctness regression; better to error
    // loudly at startup than at first query. Opt out via
    // `fail_fast_on_missing_model = false` in `[embedder]` config.
    if embedders.fail_fast_on_missing_model {
        let active = crate::layer::resolve_active_vector_indexes(&head);
        let missing: Vec<String> = active
            .iter()
            .filter(|a| service.embedders.get(&a.model).is_none())
            .map(|a| format!("{} (requires {})", a.iri, a.model))
            .collect();
        if !missing.is_empty() {
            let msg = format!(
                "fail-fast: {} active VectorIndex Resource(s) declare embedder \
                 model(s) that aren't registered: [{}]. \
                 Add entries to `[embedder].enabled` in your eigenius.toml, \
                 or set `fail_fast_on_missing_model = false` to defer the \
                 check to query time.",
                missing.len(),
                missing.join("; ")
            );
            tracing::error!(
                { field::OPERATION } = operation::SERVER_START,
                { field::ERROR_KIND } = "missing_embedder",
                "{msg}"
            );
            return Err(msg.into());
        }
    }

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

    // Raise gRPC message size limits to 128 MB to accommodate large
    // layer-load batches and external-institution dispatch payloads
    // (which can be multiple MB).
    //
    // `GrpcWebLayer` wraps the server so it accepts the gRPC-Web wire
    // protocol (HTTP/1.1) alongside native gRPC (HTTP/2). The
    // orchestrator's Deno-side `KernelClient` uses gRPC-Web through
    // `fetch()` to avoid `node:http2`'s slow / session-reuse-hanging
    // behaviour. CLI / kernel-binary clients continue to use native
    // gRPC. `accept_http1(true)` is required for the HTTP/1.1
    // handshake — tonic's default is HTTP/2-only. (tonic 0.14 removed
    // the `tonic_web::enable(...)` per-service wrapper in favour of
    // this server-wide layer.)
    tonic::transport::Server::builder()
        .accept_http1(true)
        .layer(tonic_web::GrpcWebLayer::new())
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

pub(super) struct BackendTraceStore {
    backend: Arc<dyn crate::storage::PersistentBackend>,
}

impl BackendTraceStore {
    pub(super) fn new(backend: Arc<dyn crate::storage::PersistentBackend>) -> Self {
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
