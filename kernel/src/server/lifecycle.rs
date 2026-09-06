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

    // Resume is a PROGRAM-RUN operation. A formalization task pins no program to re-resolve, and
    // D71 §6 defers its resume deliberately: the pipeline's live state is the discourse candidate
    // set plus the ranker's prior-selection context, which is recovered by replaying the prefix
    // (deterministic once the run's draws are on its branch), not by restoring a serialized state.
    // Marking it Failed with a stated reason beats silently reporting "program not found".
    let (program_iri, input_iri) = match (&record.kind.program_iri(), &record.kind.input_iri()) {
        (Some(p), Some(i)) => ((*p).to_string(), (*i).to_string()),
        _ => {
            tracing::info!(
                { field::OPERATION } = operation::TASK_RESUME,
                { field::TASK_ID } = ?record.task_id,
                kind = record.kind.label(),
                "task kind does not resume; marking Failed — re-run it instead"
            );
            record.status = TaskStatus::Failed;
            record.updated_at = now_millis();
            let _ = task_store.put_task(&record);
            return;
        }
    };

    // Resolve program and input resources from the pinned layer.
    let program = match Iri::parse(&program_iri)
        .ok()
        .and_then(|i| layer.resolve(&i).map(|arc| (*arc).clone()))
    {
        Some(p) => p,
        None => {
            tracing::warn!(
                { field::OPERATION } = operation::TASK_RESUME,
                { field::ERROR_KIND } = "program_missing",
                { field::TASK_ID } = ?record.task_id,
                { field::PROGRAM_IRI } = %program_iri,
                "task program not found at pinned head"
            );
            record.status = TaskStatus::Failed;
            record.updated_at = now_millis();
            let _ = task_store.put_task(&record);
            return;
        }
    };
    let input = match Iri::parse(&input_iri)
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

    let started_at_ms = super::helpers::now_millis();
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
        Ok(exec) => {
            // Commit what the run produced (eigenius#148). The success arm used to be
            // `Ok(_) => status = Completed`, dropping the `NbeExecutionResult` whole: a
            // task interrupted and resumed reported success and left no output resource
            // and no trace, while the non-resumed path commits both.
            //
            // **Detached, by necessity.** A `TaskRecord` pins a `layer_head`, not a
            // branch, so there is no ref to advance and no way to invent one without
            // either a schema change or a policy for a branch that moved during the
            // crash. The result lands as a layer off the pinned head and its id goes in
            // `result_layer_head` — the field the live path already sets for exactly this,
            // and what `GetTaskStatus` clients resolve.
            //
            // The records come from `build_run_records`, shared with `execute_program`, so
            // a resumed run and a live one cannot drift apart.
            record.status = TaskStatus::Completed;
            match commit_resumed_result(&record, exec, &program, &input, &backend, started_at_ms) {
                Ok(head) => record.result_layer_head = Some(head),
                Err(e) => {
                    tracing::warn!(
                        { field::OPERATION } = operation::TASK_RESUME,
                        { field::ERROR_KIND } = "result_commit_failed",
                        { field::TASK_ID } = ?task_id,
                        { field::ERROR_MESSAGE } = %e,
                        "resumed task ran but its result could not be committed"
                    );
                    record.status = TaskStatus::Failed;
                }
            }
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
///
/// `pub(crate)` so `bootstrap::tests::every_declared_component_is_implemented` can check the
/// declarations against the two places a component is actually implemented.
pub(crate) const REMOTE_COMPONENTS: &[&str] = &[
    "urn:eigenius:program:components:CompleteText",
    "urn:eigenius:program:components:CompleteJson",
    "urn:eigenius:program:components:HttpRequest",
    // Substrate-backed script execution (D26 §4.1): the orchestrator's
    // handler routes it to `dispatchRunRuntimeScript` → `SubstrateDispatcher`
    // → the language runtime (e.g. R/lme4). A program applies it with the
    // input table as component input and the `RuntimeScript` (+ env) as the
    // component argument; the run's `ProgramTrace` records that the run happened
    // (D56 §3.1). It mints no witness — a run record grounds nothing.
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
///
/// # Security: this server is unauthenticated and unencrypted
///
/// It binds `0.0.0.0:<port>` — every interface — and serves plaintext
/// gRPC and gRPC-Web. There is no `ServerTlsConfig`, no server identity,
/// no authentication interceptor, no bearer-token or API-key check, and
/// no authorization check on any RPC. `crate::capability` is institution
/// registration, not access control. Anything that can reach the port can
/// read the whole chain and commit to any branch.
///
/// The same holds for the orchestrator's HTTP listener and its `/mcp`
/// handler, and the compose stack publishes both ports on every host
/// interface while mounting the host Docker socket into the orchestrator
/// container.
///
/// **The deployment assumption is a trusted network**: run this only where
/// the port is reachable solely by trusted callers — loopback, a private
/// network segment, or behind a reverse proxy that terminates TLS and
/// authenticates before forwarding. Do not expose it to the internet.
pub async fn start_server(
    port: u16,
    orchestrator_endpoint: Option<&str>,
    backend: Option<Arc<dyn crate::storage::PersistentBackend>>,
    in_process_institutions: Vec<Arc<dyn crate::institution::runtime::Institution>>,
    embedders: EmbedderStartupConfig,
    parse_config: super::ParseConfig,
    // D71 §7.1 — the `FormalizeDocument` implementation. `None` serves a kernel whose
    // `FormalizeDocument` returns `unimplemented`, the honest answer for a build without the
    // crates that can emit an artifact.
    formalizer: Option<Arc<dyn crate::dcg::formalizer::DocumentFormalizer>>,
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
    if let Some(f) = formalizer {
        service = service.with_formalizer(f);
    }

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

/// Commit a resumed run's output and records as a layer off the task's pinned head.
///
/// Detached: no branch advances. A `TaskRecord` pins a `layer_head` rather than a ref, so
/// there is nothing to CAS against and nothing to conflict with a branch that moved while
/// the process was down. The caller records the returned id in `result_layer_head`.
///
/// **Validated before it is stored.** `LayerBuilder::build` assembles; it does not run
/// `Validator::validate` — that happens in `commit::phases`, which needs a branch to CAS
/// against and so cannot serve a detached commit. A resumed run's output has never been
/// validated (the original crashed before committing), so this runs the validator directly
/// and refuses to store on any error rather than landing an unchecked layer.
///
/// What that still skips, relative to a live commit: the AutoOnLoad institution dispatch and
/// the commit hooks. A resumed run therefore commits its output and records but fires no
/// institution gate. Recorded rather than silently accepted — closing it needs the branch
/// question a `TaskRecord` cannot answer.
#[allow(clippy::too_many_arguments)]
fn commit_resumed_result(
    record: &crate::task::TaskRecord,
    exec: crate::program::eval_io::NbeExecutionResult,
    program: &Resource,
    input: &Resource,
    backend: &Arc<dyn crate::storage::PersistentBackend>,
    started_at_ms: i64,
) -> Result<crate::layer::LayerId, String> {
    use crate::server::programs::{build_run_records, RunRecordInputs, RunRecords};

    let completed_at_ms = super::helpers::now_millis();
    let metrics = crate::program::trace::ProgramMetrics::from_trace(&exec.root_trace);
    let trace_iri = format!("urn:eigenius:trace:resume-{}", uuid::Uuid::new_v4());

    let RunRecords {
        program_trace,
        observation_trace,
        ..
    } = build_run_records(RunRecordInputs {
        trace_iri: &trace_iri,
        output: &exec.output,
        program,
        input,
        root_trace: exec.root_trace.as_ref(),
        started_at_ms,
        completed_at_ms,
        total_tokens: metrics.total_tokens,
        executed_steps: metrics.executed_steps,
    });

    let info = backend
        .load_chain_from(&record.layer_head)
        .map_err(|e| format!("load pinned chain: {e}"))?
        .ok_or_else(|| "pinned layer vanished between resume and commit".to_string())?;
    let head = crate::layer::build_chain(
        info,
        crate::layer::LayerStorage::with_persistent(Arc::clone(backend)),
    );

    let mut b = crate::layer::LayerBuilder::new("task-resume-result", Some(head));
    for r in exec.produced_resources {
        b.add_resource(r).map_err(|e| format!("produced: {e}"))?;
    }
    if exec.output.id().is_some() {
        b.add_resource(exec.output)
            .map_err(|e| format!("output: {e}"))?;
    }
    b.add_resource(program_trace)
        .map_err(|e| format!("ProgramTrace: {e}"))?;
    b.add_resource(observation_trace)
        .map_err(|e| format!("ObservationTrace: {e}"))?;

    let layer = Arc::new(
        b.build(crate::layer::LayerStorage::with_persistent(Arc::clone(
            backend,
        ))),
    );

    let errors = crate::validation::Validator::new(Arc::clone(&layer)).validate();
    if !errors.is_empty() {
        return Err(format!(
            "resumed result failed validation ({} error(s)); first: {}",
            errors.len(),
            errors
                .first()
                .map(|e| e.message.as_str())
                .unwrap_or("<none>")
        ));
    }

    backend
        .store_layer(&layer)
        .map_err(|e| format!("persist result layer: {e}"))
}

#[cfg(test)]
mod resume_tests {
    use super::*;
    use crate::ontology::eigon_json;
    use crate::ontology::resource::Resource;
    use crate::storage::memory::MemoryPersistentBackend;
    use crate::storage::PersistentBackend;

    /// A layer holding a runnable program and its input, persisted so a task record can
    /// pin it. Returns the backend, the pinned layer id, and the two IRIs.
    fn pinned_run(backend: &Arc<dyn PersistentBackend>) -> (crate::layer::LayerId, String, String) {
        let ctx = crate::bootstrap::bootstrap_persistent(Arc::clone(backend))
            .expect("bootstrap over the memory backend");
        let mut b = crate::layer::LayerBuilder::new("resume-fixture", Some(Arc::clone(ctx.head())));

        let program = eigon_json::parse_document(
            r#"{
                "@id": "urn:eigenius:test:resume:prog",
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
                "urn:eigenius:program:input_type": "urn:eigenius:prov:Agent",
                "urn:eigenius:program:output_type": "urn:eigenius:prov:Agent",
                "urn:eigenius:program:body": {
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Construct"],
                    "urn:eigenius:program:class": "urn:eigenius:prov:Agent",
                    "urn:eigenius:program:fields": {
                        "urn:eigenius:core:short_name": {
                            "urn:eigenius:core:is_a": ["urn:eigenius:program:Literal"],
                            "urn:eigenius:program:value": "resumed"
                        }
                    }
                }
            }"#,
        )
        .expect("program parses")
        .remove(0);
        let mut input = Resource::new(Iri::parse("urn:eigenius:test:resume:input").unwrap());
        input.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String("urn:eigenius:prov:Agent".to_string()),
            ]),
        );
        b.add_resource(program).expect("add program");
        b.add_resource(input).expect("add input");

        let layer = b.build(crate::layer::LayerStorage::with_persistent(Arc::clone(
            backend,
        )));
        let id = backend
            .store_layer(&layer)
            .expect("persist the pinned layer");
        (
            id,
            "urn:eigenius:test:resume:prog".to_string(),
            "urn:eigenius:test:resume:input".to_string(),
        )
    }

    /// §3.2 / eigenius#148 — a resumed task commits what it produced.
    ///
    /// The success arm was `Ok(_) => { record.status = TaskStatus::Completed; }`: the
    /// `NbeExecutionResult` was dropped whole, so a task interrupted and resumed reported
    /// success and left no output resource and no trace, while the non-resumed path
    /// through `execute_program` commits all of them.
    ///
    /// The result lands **detached**: a layer off the record's pinned `layer_head`, with
    /// its id in `result_layer_head`. No branch advances, because a `TaskRecord` carries
    /// no branch — it pins a layer, not a ref — and inventing one would either need a
    /// schema change or a policy for a branch that moved during the crash.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_resumed_task_commits_its_output_and_traces() {
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let (layer_head, program_iri, input_iri) = pinned_run(&backend);

        let task_store: Arc<dyn crate::task::TaskStore> =
            Arc::new(crate::task::BackendTaskStore::new(Arc::clone(&backend)));
        let session_id = uuid::Uuid::new_v4();
        let task_id = uuid::Uuid::new_v4();
        let rec = crate::task::TaskRecord::new_running(
            session_id,
            task_id,
            program_iri,
            input_iri,
            layer_head,
            crate::server::helpers::now_millis(),
        );
        task_store.put_task(&rec).expect("seed the Running record");

        let trace_store: Arc<dyn TraceStore> =
            Arc::new(crate::server::BackendTraceStore::new(Arc::clone(&backend)));
        resume_one_task(
            rec,
            Arc::clone(&task_store),
            Arc::clone(&backend),
            trace_store,
            Arc::new(ComponentRegistry::default()),
            1,
        )
        .await;

        let after = task_store
            .get_task(&session_id, &task_id)
            .expect("read back")
            .expect("the record survives resume");
        assert_eq!(
            after.status,
            crate::task::TaskStatus::Completed,
            "the re-execution succeeds"
        );

        let result_head = after.result_layer_head.expect(
            "a resumed task records where its result landed — without this the run reports \
             success and commits nothing (eigenius#148)",
        );
        let info = backend
            .load_chain_from(&result_head)
            .expect("load the result chain")
            .expect("the result layer is persisted");
        let layer = crate::layer::build_chain(
            info,
            crate::layer::LayerStorage::with_persistent(Arc::clone(&backend)),
        );
        let mut program_traces = 0;
        let mut observation_traces = 0;
        for (_iri, r) in layer.iter_resources() {
            for c in r.is_a() {
                match c.as_str() {
                    "urn:eigenius:prov:ProgramTrace" => program_traces += 1,
                    "urn:eigenius:prov:ObservationTrace" => observation_traces += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(
            (program_traces, observation_traces),
            (1, 1),
            "the resumed run leaves the same pair as a live one — build_run_records is \
             shared so the two cannot drift"
        );
    }
}
