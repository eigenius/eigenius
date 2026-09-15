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

//! Task-management RPCs: `ListTasks`, `GetTaskStatus`, `CancelTask`, `DeleteTask`,
//! `PruneTasks`, `StartReindex`, `StartSweep`.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::observability::{field, operation, RpcGuard};
use std::sync::Arc;
use tonic::{Response, Status};

/// Tasks per `ListTasks` response when the caller names no limit.
const DEFAULT_TASK_PAGE: usize = 100;
/// Ceiling on a caller-supplied limit, so one request cannot ask for the whole store.
const MAX_TASK_PAGE: usize = 1000;

/// `<created_at_ms>:<task_id>` — the position of the last entry of a page.
///
/// Opaque to callers by contract; parsed here rather than trusted, so a hand-written or
/// stale cursor is an `invalid_argument` rather than a silently wrong page.
fn parse_task_cursor(cursor: &str) -> Result<(i64, String), Status> {
    let (created_at, task_id) = cursor
        .split_once(':')
        .ok_or_else(|| Status::invalid_argument("malformed cursor"))?;
    let created_at: i64 = created_at
        .parse()
        .map_err(|_| Status::invalid_argument("malformed cursor"))?;
    if task_id.is_empty() {
        return Err(Status::invalid_argument("malformed cursor"));
    }
    Ok((created_at, task_id.to_string()))
}

impl EigeniusService {
    pub(super) async fn handle_list_tasks(
        &self,
        req: ListTasksRequest,
    ) -> Result<Response<ListTasksResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LIST_TASKS);
        let limit = match req.limit {
            0 => DEFAULT_TASK_PAGE,
            n => (n as usize).min(MAX_TASK_PAGE),
        };
        let cursor = if req.cursor.is_empty() {
            None
        } else {
            Some(parse_task_cursor(&req.cursor)?)
        };
        let mut tasks: Vec<_> = match &self.task_store {
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
        // In-flight index tasks live in the SweepRegistry, not the TaskStore — a sweep
        // fires on every commit and persisting one record per commit would swamp the
        // store, so the registry is their live view (eigenius#254). They carry a
        // `task_id` like any other task, so they list beside the stored ones. A finished
        // reindex is also in the store, so filter by id to avoid reporting it twice.
        if let Some(coord) = self.sweep_coordinator.as_ref() {
            for record in coord.registry().list_records() {
                let id = record.task_id.to_string();
                if !tasks.iter().any(|t| t.task_id == id) {
                    tasks.push(task_record_to_info(record));
                }
            }
        }

        // Newest first, task_id breaking ties — a total order, so a page boundary cannot
        // skip or repeat an entry. Sorting happens after the registry merge so in-flight
        // index tasks take their place by age rather than landing in a clump at the end.
        tasks.sort_by(|a, b| {
            b.created_at_ms
                .cmp(&a.created_at_ms)
                .then_with(|| a.task_id.cmp(&b.task_id))
        });
        if let Some((created_at, task_id)) = cursor {
            tasks.retain(|t| {
                t.created_at_ms < created_at
                    || (t.created_at_ms == created_at && t.task_id > task_id)
            });
        }
        let next_cursor = if tasks.len() > limit {
            tasks.truncate(limit);
            tasks
                .last()
                .map(|t| format!("{}:{}", t.created_at_ms, t.task_id))
                .unwrap_or_default()
        } else {
            String::new()
        };
        Ok(Response::new(ListTasksResponse { tasks, next_cursor }))
    }

    pub(super) async fn handle_get_task_status(
        &self,
        req: GetTaskStatusRequest,
    ) -> Result<Response<GetTaskStatusResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_GET_TASK_STATUS);
        tracing::debug!(
            { field::OPERATION } = operation::RPC_GET_TASK_STATUS,
            { field::TASK_ID } = %req.task_id,
            "get_task_status target"
        );
        let task_id = uuid::Uuid::parse_str(&req.task_id)
            .map_err(|e| Status::invalid_argument(format!("invalid task_id: {e}")))?;
        // The registry first: an in-flight index task is not in the store, and where a
        // reindex is in both the registry has the live status (eigenius#254).
        if let Some(handle) = self
            .sweep_coordinator
            .as_ref()
            .and_then(|coord| coord.registry().find_by_task_id(&task_id))
        {
            return Ok(Response::new(GetTaskStatusResponse {
                found: true,
                task: Some(task_record_to_info(handle.record_snapshot())),
            }));
        }
        let store = match &self.task_store {
            Some(s) => s,
            None => {
                return Ok(Response::new(GetTaskStatusResponse {
                    found: false,
                    task: None,
                }))
            }
        };
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

    /// Delete one terminal task and everything it owns (eigenius#258).
    ///
    /// Refuses anything not terminal. All three pin-gathering sites key on
    /// `!is_terminal()`, so deleting a live record would drop a GC root, a `CheckPins`
    /// block and a consolidation block at once, while the task still intends to resume
    /// against that layer. Cancelling is what gives a task a terminal state; that is the
    /// order.
    ///
    /// An in-flight *index* task is not in the store at all — it lives in the
    /// `SweepRegistry` and leaves no record unless it failed — so there is nothing here
    /// to delete for one.
    pub(super) async fn handle_delete_task(
        &self,
        req: DeleteTaskRequest,
    ) -> Result<Response<DeleteTaskResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_DELETE_TASK);
        let Some(store) = self.task_store.as_ref() else {
            return Ok(Response::new(DeleteTaskResponse {
                success: false,
                status: String::new(),
                error: "no persistent backend; tasks are not tracked".to_string(),
            }));
        };
        let task_id = uuid::Uuid::parse_str(&req.task_id)
            .map_err(|e| Status::invalid_argument(format!("invalid task_id: {e}")))?;
        let session_id = self.session.read().await.session_id;
        let record = match store.get_task(&session_id, &task_id) {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Ok(Response::new(DeleteTaskResponse {
                    success: false,
                    status: String::new(),
                    error: format!("task not found: {task_id}"),
                }));
            }
            Err(e) => return Err(Status::internal(format!("get_task failed: {e}"))),
        };
        if !record.status.is_terminal() {
            return Ok(Response::new(DeleteTaskResponse {
                success: false,
                status: format!("{:?}", record.status),
                error: format!(
                    "task {task_id} is {:?}, not terminal — it pins its layer against GC \
                     and a delete would drop that pin silently; cancel it first",
                    record.status
                ),
            }));
        }
        if let Err(e) = store.delete_task(&session_id, &task_id) {
            return Err(Status::internal(format!("delete_task failed: {e}")));
        }
        tracing::info!(
            { field::OPERATION } = operation::RPC_DELETE_TASK,
            { field::TASK_ID } = %task_id,
            status = ?record.status,
            "task deleted"
        );
        Ok(Response::new(DeleteTaskResponse {
            success: true,
            status: format!("{:?}", record.status),
            error: String::new(),
        }))
    }

    /// Delete every terminal task older than a cutoff (eigenius#258).
    ///
    /// Same refusal as [`Self::handle_delete_task`], applied silently rather than as an
    /// error: a non-terminal record is counted into `retained_non_terminal` and left
    /// alone, so a caller can see that the store is not empty for a reason.
    ///
    /// Age is measured from `updated_at`, not `created_at` — a long-running task that
    /// finished recently is recent, whatever time it started.
    pub(super) async fn handle_prune_tasks(
        &self,
        req: PruneTasksRequest,
    ) -> Result<Response<PruneTasksResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_PRUNE_TASKS);
        let Some(store) = self.task_store.as_ref() else {
            return Ok(Response::new(PruneTasksResponse {
                success: false,
                task_ids: Vec::new(),
                retained_non_terminal: 0,
                error: "no persistent backend; tasks are not tracked".to_string(),
            }));
        };
        let session_id = self.session.read().await.session_id;
        let records = store
            .list_tasks(&session_id)
            .map_err(|e| Status::internal(format!("list_tasks failed: {e}")))?;

        let cutoff = now_millis().saturating_sub(req.older_than_ms as i64);
        let mut task_ids = Vec::new();
        let mut retained_non_terminal: u32 = 0;
        for record in records {
            if !record.status.is_terminal() {
                retained_non_terminal += 1;
                continue;
            }
            if record.updated_at > cutoff {
                continue;
            }
            if !req.dry_run {
                if let Err(e) = store.delete_task(&session_id, &record.task_id) {
                    return Err(Status::internal(format!("delete_task failed: {e}")));
                }
            }
            task_ids.push(record.task_id.to_string());
        }
        tracing::info!(
            { field::OPERATION } = operation::RPC_PRUNE_TASKS,
            { field::COUNT } = task_ids.len(),
            retained_non_terminal = retained_non_terminal,
            dry_run = req.dry_run,
            "task prune"
        );
        Ok(Response::new(PruneTasksResponse {
            success: true,
            task_ids,
            retained_non_terminal,
            error: String::new(),
        }))
    }

    /// D43 §5.5 — re-run the post-Load sweep for layers it never covered (eigenius#254).
    ///
    /// The commit hook sweeps each layer once and nothing retries, so a sweep that failed
    /// leaves that layer's content invisible to vector search, silently — a missing
    /// segment contributes no candidates rather than an error. Committing new data does
    /// not help: the hook sweeps only the layer being committed.
    ///
    /// `StartReindex` does not cover this. Its predicate is "a visible segment's model
    /// differs from the declared one", and a layer with no segment contributes nothing to
    /// that comparison.
    ///
    /// With no layer given, [`crate::layer::detect_unswept_layers`] names the set.
    pub(super) async fn handle_start_sweep(
        &self,
        req: StartSweepRequest,
    ) -> Result<Response<StartSweepResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_START_SWEEP);
        let Some(coord) = self.sweep_coordinator.clone() else {
            return Ok(Response::new(StartSweepResponse {
                success: false,
                layers: Vec::new(),
                error: "no embedders registered; this kernel cannot sweep".to_string(),
            }));
        };

        // An explicit layer is swept whether or not it looks unswept — re-running is
        // idempotent by `(index, layer)`, and the operator may be repairing a segment
        // that exists but is wrong.
        // Every sweep resolves its Indexes against this head, not against the layer being
        // swept — an unswept layer is usually an ancestor, and Indexes declared above it
        // are invisible from there (eigenius#254).
        let head = self.resolve_read_layer("", "").await?;
        let targets: Vec<Arc<crate::layer::Layer>> = if req.layer.is_empty() {
            let unswept = crate::layer::detect_unswept_layers(&head)
                .map_err(|e| Status::internal(format!("unswept detection failed: {e}")))?;
            // One sweep per layer covers every Index active at the head, so collapse the
            // per-Index pairs to the distinct layers and resolve each to a `Layer`.
            let wanted: std::collections::BTreeSet<crate::layer::LayerId> =
                unswept.into_iter().map(|u| u.layer_id).collect();
            let mut visited = std::collections::BTreeSet::new();
            let mut out = Vec::new();
            let mut queue: Vec<Arc<crate::layer::Layer>> = vec![Arc::clone(&head)];
            while let Some(layer) = queue.pop() {
                if !visited.insert(layer.id().clone()) {
                    continue;
                }
                if wanted.contains(layer.id()) {
                    out.push(Arc::clone(&layer));
                }
                queue.extend(layer.parents().iter().map(Arc::clone));
            }
            out
        } else {
            vec![self.resolve_read_layer(&req.layer, "").await?]
        };

        if targets.is_empty() {
            return Ok(Response::new(StartSweepResponse {
                success: true,
                layers: Vec::new(),
                error: String::new(),
            }));
        }
        let layers: Vec<String> = targets.iter().map(|l| hex::encode(l.id().0)).collect();
        tracing::info!(
            { field::OPERATION } = operation::RPC_START_SWEEP,
            n_layers = targets.len(),
            "operator-requested vector sweep"
        );

        let task_store = self.task_store.clone();
        let head_for_task = Arc::clone(&head);
        tokio::spawn(async move {
            for layer in targets {
                let layer_id_disp = format!("{}", layer.id());
                let Some(outcome) = coord.trigger_sweep_at(&head_for_task, layer).await else {
                    // No Index is active at the head at all — detection said otherwise a
                    // moment ago, so the declaration was removed in between.
                    tracing::debug!(
                        { field::OPERATION } = operation::RPC_START_SWEEP,
                        { field::LAYER_ID } = %layer_id_disp,
                        "sweep skipped: no active index at head (race after detection)"
                    );
                    continue;
                };
                match &outcome.result {
                    Ok(report) => tracing::info!(
                        { field::OPERATION } = operation::RPC_START_SWEEP,
                        { field::LAYER_ID } = %layer_id_disp,
                        total_subjects = report.total_subjects,
                        "operator-requested sweep completed"
                    ),
                    Err(e) => {
                        tracing::warn!(
                            { field::OPERATION } = operation::RPC_START_SWEEP,
                            { field::ERROR_KIND } = "vector_sweep_failed",
                            { field::LAYER_ID } = %layer_id_disp,
                            { field::ERROR_MESSAGE } = %e,
                            "operator-requested sweep failed"
                        );
                        // Same rule as the commit hook: a sweep that did not complete is
                        // the one worth a durable record.
                        if let Some(store) = task_store.as_ref() {
                            let record = outcome.handle.record_snapshot();
                            if let Err(e) = store.put_task(&record) {
                                tracing::warn!(
                                    { field::OPERATION } = operation::RPC_START_SWEEP,
                                    { field::ERROR_KIND } =
                                        "vector_sweep_record_not_persisted",
                                    { field::TASK_ID } = %record.task_id,
                                    { field::ERROR_MESSAGE } = %e,
                                    "sweep failed and its task record could not be \
                                     stored either"
                                );
                            }
                        }
                    }
                }
            }
        });
        Ok(Response::new(StartSweepResponse {
            success: true,
            layers,
            error: String::new(),
        }))
    }

    /// D43 §5.7 — start a chain-wide reindex on demand (eigenius#254).
    ///
    /// The commit hook already fires this for the layer it persisted, so the operator case
    /// is repair: a reindex that failed or was cancelled leaves segments split across two
    /// models, and without this the only way to retry is to commit something.
    ///
    /// `detect_reindex_targets` is the pre-check — with every visible segment already on
    /// the declared model it returns nothing and no task starts. Detection runs inline so
    /// the response can say what was found; the reindexes themselves are detached, because
    /// re-embedding a chain runs for minutes and must not hold an RPC open.
    pub(super) async fn handle_start_reindex(
        &self,
        req: StartReindexRequest,
    ) -> Result<Response<StartReindexResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_START_REINDEX);
        let Some(coord) = self.sweep_coordinator.clone() else {
            return Ok(Response::new(StartReindexResponse {
                success: false,
                indexes: Vec::new(),
                error: "no embedders registered; this kernel cannot reindex".to_string(),
            }));
        };
        let layer = self.resolve_read_layer(&req.layer, "").await?;

        // Detection is synchronous and bounded (one pass over the segment models per
        // active Index), so it happens here: an operator asking for a reindex should be
        // told whether one started, not handed an empty success.
        let targets = crate::layer::detect_reindex_targets(&layer)
            .map_err(|e| Status::internal(format!("reindex detection failed: {e}")))?;
        if targets.is_empty() {
            return Ok(Response::new(StartReindexResponse {
                success: true,
                indexes: Vec::new(),
                error: String::new(),
            }));
        }
        let indexes: Vec<String> = targets
            .iter()
            .map(|t| t.index_iri.as_str().to_string())
            .collect();
        tracing::info!(
            { field::OPERATION } = operation::RPC_START_REINDEX,
            { field::LAYER_ID } = %layer.id(),
            n_targets = targets.len(),
            "operator-requested reindex"
        );

        let task_store = self.task_store.clone();
        let layer_for_task = Arc::clone(&layer);
        tokio::spawn(async move {
            match coord.trigger_reindex_async(layer_for_task).await {
                Ok(handles) => {
                    for handle in &handles {
                        // Same terminal-record persistence the commit hook does: the
                        // registry entry is gone once the task ends, and an operator who
                        // asked for this reindex needs its outcome afterwards.
                        let record = handle.record_snapshot();
                        if let Some(store) = task_store.as_ref() {
                            if let Err(e) = store.put_task(&record) {
                                tracing::warn!(
                                    { field::OPERATION } = operation::RPC_START_REINDEX,
                                    { field::ERROR_KIND } =
                                        "vector_reindex_record_not_persisted",
                                    { field::TASK_ID } = %record.task_id,
                                    { field::ERROR_MESSAGE } = %e,
                                    "reindex finished but its task record could not be stored"
                                );
                            }
                        }
                        tracing::info!(
                            { field::OPERATION } = operation::RPC_START_REINDEX,
                            { field::TASK_ID } = %record.task_id,
                            status = ?handle.status(),
                            "operator-requested reindex finished"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        { field::OPERATION } = operation::RPC_START_REINDEX,
                        { field::ERROR_KIND } = "vector_reindex_failed",
                        { field::ERROR_MESSAGE } = %e,
                        "operator-requested reindex failed"
                    );
                }
            }
        });
        Ok(Response::new(StartReindexResponse {
            success: true,
            indexes,
            error: String::new(),
        }))
    }

    pub(super) async fn handle_cancel_task(
        &self,
        req: CancelTaskRequest,
    ) -> Result<Response<CancelTaskResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_CANCEL_TASK);
        tracing::debug!(
            { field::OPERATION } = operation::RPC_CANCEL_TASK,
            { field::TASK_ID } = %req.task_id,
            "cancel_task target"
        );
        let task_id = uuid::Uuid::parse_str(&req.task_id)
            .map_err(|e| Status::invalid_argument(format!("invalid task_id: {e}")))?;
        // An index task cancels through its cooperative flag, which is what the sweep and
        // the reindex drivers poll — not through a persisted status the way a ProgramRun
        // does (eigenius#254). Unlike #134's `Cancelling`, this one has an exit: the
        // driver returns `SweepError::Cancelled` at its next check and stamps the record
        // `Cancelled` itself.
        if let Some(handle) = self
            .sweep_coordinator
            .as_ref()
            .and_then(|coord| coord.registry().find_by_task_id(&task_id))
        {
            let status = handle.status();
            if status.is_terminal() {
                return Ok(Response::new(CancelTaskResponse {
                    success: true,
                    status: format!("{status:?}"),
                    error: String::new(),
                }));
            }
            // Stamp the record too, not just the flag. Reporting `Cancelling` while the
            // record still read `Running` made `GetTaskStatus` contradict the
            // `CancelTask` response that had just been returned. The driver overwrites
            // this with its own terminal status at the next check.
            handle.cancel();
            handle.mark_cancelling();
            return Ok(Response::new(CancelTaskResponse {
                success: true,
                status: format!("{:?}", handle.status()),
                error: String::new(),
            }));
        }
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

        // `Cancelling` means "waiting on the cooperative grace window", and that only
        // describes a task something is actually driving. `Suspended` is defined as
        // "persisted mid-flight but not being driven right now", so there is no window to
        // wait on and no evaluator to hear the request: cancellation is complete the
        // moment it is asked for (eigenius#134).
        //
        // The distinction matters because all three pin-gathering sites — GC roots,
        // `DeleteBranch`'s `CheckPins`, and `build_consolidate_opts` — key on
        // `!is_terminal()`. Parking a suspended task in `Cancelling` would pin its
        // `layer_head` against every one of them for no reason.
        //
        // `Running` still gets `Cancelling`: it may be driven by a live evaluator in this
        // process, and whether that evaluator stops is eigenius#51. If it is instead a
        // leftover from a crash, the resume sweep finalises it at the next restart.
        record.status = if record.status == crate::task::TaskStatus::Suspended {
            crate::task::TaskStatus::Cancelled
        } else {
            crate::task::TaskStatus::Cancelling
        };
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::sweep_registry::SweepHandle;
    use crate::task::{TaskRecord, TaskStatus};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, RwLock};

    /// A service with an embedder registered, so it owns a `SweepCoordinator`, plus one
    /// in-flight sweep registered against a layer.
    fn service_with_in_flight_sweep() -> (EigeniusService, uuid::Uuid, Arc<SweepHandle>) {
        let mut embedders = crate::program::embedder::EmbedderRegistry::new();
        embedders.register(Arc::new(crate::program::embedder::DummyEmbedder::new(
            "urn:eigenius:embed:dummy:v1",
            8,
        )));
        let service = EigeniusService::new()
            .expect("service")
            .with_embedders(embedders, 32);

        let layer = crate::layer::LayerId([9u8; 32]);
        let index = crate::ontology::iri::Iri::parse("urn:eigenius:test:vi").unwrap();
        let record = TaskRecord::new_vector_sweep(
            uuid::Uuid::new_v4(),
            vec![index.as_str().to_string()],
            layer.clone(),
            0,
        );
        let task_id = record.task_id;
        let handle = Arc::new(SweepHandle::from_parts(
            Arc::new(AtomicBool::new(false)),
            Arc::new(RwLock::new(record)),
            vec![index],
        ));
        service
            .sweep_coordinator
            .as_ref()
            .expect("coordinator")
            .registry()
            .register(layer, Arc::clone(&handle));
        (service, task_id, handle)
    }

    /// An in-flight sweep is not in the TaskStore — this service has none at all — and
    /// still lists, because the registry is its live view (eigenius#254).
    #[tokio::test]
    async fn list_tasks_reports_in_flight_index_tasks() {
        let (service, task_id, _handle) = service_with_in_flight_sweep();
        let resp = service
            .handle_list_tasks(ListTasksRequest::default())
            .await
            .expect("list")
            .into_inner();
        let task = resp
            .tasks
            .iter()
            .find(|t| t.task_id == task_id.to_string())
            .expect("the in-flight sweep must be listed");
        assert_eq!(task.kind, "VectorSweep");
        assert_eq!(task.indexes, vec!["urn:eigenius:test:vi".to_string()]);
        assert!(
            task.program_iri.is_empty(),
            "a sweep is not a program run and must not name one"
        );
    }

    #[tokio::test]
    async fn get_task_status_finds_an_index_task_by_task_id() {
        let (service, task_id, _handle) = service_with_in_flight_sweep();
        let resp = service
            .handle_get_task_status(GetTaskStatusRequest {
                task_id: task_id.to_string(),
            })
            .await
            .expect("status")
            .into_inner();
        assert!(resp.found, "the registry is consulted before the store");
        let task = resp.task.expect("task");
        assert_eq!(task.status, format!("{:?}", TaskStatus::Running));
        assert_eq!(task.kind, "VectorSweep");
    }

    /// Cancelling reaches the cooperative flag the sweep loop polls. Without a task store
    /// the old handler refused outright; an index task does not need one.
    #[tokio::test]
    async fn cancel_task_raises_the_index_task_flag() {
        let (service, task_id, handle) = service_with_in_flight_sweep();
        assert!(!handle.is_cancelled());
        let resp = service
            .handle_cancel_task(CancelTaskRequest {
                task_id: task_id.to_string(),
            })
            .await
            .expect("cancel")
            .into_inner();
        assert!(resp.success, "error was: {}", resp.error);
        assert_eq!(resp.status, format!("{:?}", TaskStatus::Cancelling));
        assert!(
            handle.is_cancelled(),
            "CancelTask must raise the flag the driver polls, not just report success"
        );
        // And the record agrees, so a GetTaskStatus right after does not contradict the
        // response the caller just received.
        let resp = service
            .handle_get_task_status(GetTaskStatusRequest {
                task_id: task_id.to_string(),
            })
            .await
            .expect("status")
            .into_inner();
        assert_eq!(
            resp.task.expect("task").status,
            format!("{:?}", TaskStatus::Cancelling)
        );
    }

    /// No coordinator — no embedders — means no index tasks and no crash.
    #[tokio::test]
    async fn task_rpcs_are_unchanged_without_a_coordinator() {
        let service = EigeniusService::new().expect("service");
        let resp = service
            .handle_list_tasks(ListTasksRequest::default())
            .await
            .expect("list")
            .into_inner();
        assert!(resp.tasks.is_empty());
        let resp = service
            .handle_get_task_status(GetTaskStatusRequest {
                task_id: uuid::Uuid::new_v4().to_string(),
            })
            .await
            .expect("status")
            .into_inner();
        assert!(!resp.found);
    }

    /// Build a service with a real task store, and a helper to seed records.
    fn service_with_task_store() -> (EigeniusService, Arc<dyn crate::task::TaskStore>) {
        let backend: Arc<dyn crate::storage::PersistentBackend> =
            Arc::new(crate::storage::memory::MemoryPersistentBackend::new());
        let store: Arc<dyn crate::task::TaskStore> =
            Arc::new(crate::task::BackendTaskStore::new(Arc::clone(&backend)));
        let mut service = EigeniusService::new().expect("service");
        service.task_store = Some(Arc::clone(&store));
        (service, store)
    }

    fn seed_task(
        store: &Arc<dyn crate::task::TaskStore>,
        session_id: uuid::Uuid,
        status: TaskStatus,
        updated_at: i64,
    ) -> uuid::Uuid {
        let task_id = uuid::Uuid::new_v4();
        let mut record = TaskRecord::new_running(
            session_id,
            task_id,
            "urn:eigenius:test:program:p".to_string(),
            "urn:eigenius:test:input:i".to_string(),
            crate::layer::LayerId([5u8; 32]),
            0,
        );
        record.status = status;
        record.updated_at = updated_at;
        store.put_task(&record).expect("put");
        task_id
    }

    /// Paging walks every task exactly once (eigenius#258).
    ///
    /// Several records share a `created_at`, which is the case a cursor keyed on time
    /// alone would get wrong — it would either re-emit the whole tied group on the next
    /// page or skip the rest of it. The tie-break on `task_id` is what makes the order
    /// total.
    #[tokio::test]
    async fn list_tasks_pages_without_skipping_or_repeating() {
        let (service, store) = service_with_task_store();
        let session_id = service.session.read().await.session_id;

        // 25 tasks over 5 distinct timestamps — 5 ties at each.
        let mut expected = std::collections::BTreeSet::new();
        for i in 0..25 {
            let task_id = uuid::Uuid::new_v4();
            let mut record = TaskRecord::new_running(
                session_id,
                task_id,
                "urn:eigenius:test:program:p".to_string(),
                "urn:eigenius:test:input:i".to_string(),
                crate::layer::LayerId([5u8; 32]),
                1_000 + (i % 5) as i64,
            );
            record.status = TaskStatus::Completed;
            store.put_task(&record).expect("put");
            expected.insert(task_id.to_string());
        }

        let mut seen: Vec<String> = Vec::new();
        let mut cursor = String::new();
        let mut pages = 0;
        loop {
            let resp = service
                .handle_list_tasks(ListTasksRequest {
                    limit: 7,
                    cursor: cursor.clone(),
                })
                .await
                .expect("list")
                .into_inner();
            pages += 1;
            assert!(resp.tasks.len() <= 7, "a page must respect the limit");
            seen.extend(resp.tasks.iter().map(|t| t.task_id.clone()));
            if resp.next_cursor.is_empty() {
                break;
            }
            cursor = resp.next_cursor;
            assert!(pages < 20, "paging did not terminate");
        }

        assert_eq!(pages, 4, "25 tasks at 7 per page");
        assert_eq!(seen.len(), 25, "every task appears, none repeated");
        let unique: std::collections::BTreeSet<String> = seen.iter().cloned().collect();
        assert_eq!(unique, expected, "and they are the ones that were stored");

        // Newest first across the whole walk, not merely within a page.
        let mut times: Vec<i64> = Vec::new();
        for id in &seen {
            let resp = service
                .handle_get_task_status(GetTaskStatusRequest {
                    task_id: id.clone(),
                })
                .await
                .expect("status")
                .into_inner();
            times.push(resp.task.expect("task").created_at_ms);
        }
        assert!(
            times.windows(2).all(|w| w[0] >= w[1]),
            "pages must continue the ordering, not restart it: {times:?}"
        );
    }

    /// An unparseable cursor is rejected rather than silently yielding page one.
    #[tokio::test]
    async fn a_malformed_cursor_is_an_error() {
        let (service, _store) = service_with_task_store();
        let err = service
            .handle_list_tasks(ListTasksRequest {
                limit: 0,
                cursor: "not-a-cursor".to_string(),
            })
            .await
            .expect_err("a malformed cursor must not be treated as 'start over'");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    /// Deleting a task removes everything it owns, not just its record (eigenius#258).    /// Deleting a task removes everything it owns, not just its record (eigenius#258).
    ///
    /// The record is the only index into a task's checkpoints and traces, so a delete
    /// that left them behind would strand them: unreachable, and still occupying the
    /// space the delete was for.
    #[tokio::test]
    async fn deleting_a_task_removes_its_checkpoints_and_traces_too() {
        let (service, store) = service_with_task_store();
        let session_id = service.session.read().await.session_id;
        let task_id = seed_task(&store, session_id, TaskStatus::Completed, 0);

        store
            .put_checkpoint(&crate::task::Checkpoint {
                session_id,
                task_id,
                step_seq: 1,
                state: vec![0xa0],
                created_at: 0,
            })
            .expect("checkpoint");
        assert!(store
            .get_checkpoint(&session_id, &task_id, 1)
            .expect("get")
            .is_some());

        let resp = service
            .handle_delete_task(DeleteTaskRequest {
                task_id: task_id.to_string(),
            })
            .await
            .expect("delete")
            .into_inner();
        assert!(resp.success, "error was: {}", resp.error);

        assert!(store
            .get_task(&session_id, &task_id)
            .expect("get")
            .is_none());
        assert!(
            store
                .get_checkpoint(&session_id, &task_id, 1)
                .expect("get")
                .is_none(),
            "the checkpoint must go with the task, not outlive it unreachable"
        );
    }

    /// A non-terminal task is refused. Its record is a GC root, a `CheckPins` block and a
    /// consolidation block; deleting it would drop all three while the task still means
    /// to resume against that layer.
    #[tokio::test]
    async fn deleting_a_live_task_is_refused_with_a_reason() {
        let (service, store) = service_with_task_store();
        let session_id = service.session.read().await.session_id;
        let task_id = seed_task(&store, session_id, TaskStatus::Suspended, 0);

        let resp = service
            .handle_delete_task(DeleteTaskRequest {
                task_id: task_id.to_string(),
            })
            .await
            .expect("delete")
            .into_inner();
        assert!(!resp.success);
        assert_eq!(resp.status, format!("{:?}", TaskStatus::Suspended));
        assert!(resp.error.contains("not terminal"), "got: {}", resp.error);
        assert!(
            store
                .get_task(&session_id, &task_id)
                .expect("get")
                .is_some(),
            "a refused delete must not have deleted anything"
        );
    }

    /// Prune takes terminal tasks past the cutoff, leaves recent ones, and never touches a
    /// live one however old it is.
    #[tokio::test]
    async fn prune_respects_the_cutoff_and_never_deletes_a_live_task() {
        let (service, store) = service_with_task_store();
        let session_id = service.session.read().await.session_id;
        let now = now_millis();

        let old_done = seed_task(&store, session_id, TaskStatus::Completed, now - 86_400_000);
        let recent_done = seed_task(&store, session_id, TaskStatus::Failed, now - 1_000);
        let old_live = seed_task(&store, session_id, TaskStatus::Suspended, now - 86_400_000);

        // Dry run reports and changes nothing.
        let resp = service
            .handle_prune_tasks(PruneTasksRequest {
                older_than_ms: 3_600_000,
                dry_run: true,
            })
            .await
            .expect("prune")
            .into_inner();
        assert_eq!(resp.task_ids, vec![old_done.to_string()]);
        assert_eq!(resp.retained_non_terminal, 1);
        assert!(
            store
                .get_task(&session_id, &old_done)
                .expect("get")
                .is_some(),
            "a dry run must not delete"
        );

        let resp = service
            .handle_prune_tasks(PruneTasksRequest {
                older_than_ms: 3_600_000,
                dry_run: false,
            })
            .await
            .expect("prune")
            .into_inner();
        assert_eq!(resp.task_ids, vec![old_done.to_string()]);
        assert!(store
            .get_task(&session_id, &old_done)
            .expect("get")
            .is_none());
        assert!(
            store
                .get_task(&session_id, &recent_done)
                .expect("get")
                .is_some(),
            "inside the cutoff"
        );
        assert!(
            store
                .get_task(&session_id, &old_live)
                .expect("get")
                .is_some(),
            "age does not make a live task deletable"
        );
    }

    /// eigenius#134 — cancelling a `Suspended` task terminates it outright.    /// eigenius#134 — cancelling a `Suspended` task terminates it outright.
    ///
    /// `Cancelling` means "waiting on the cooperative grace window", and a suspended task
    /// is by definition not being driven, so there is no window and nothing to hear the
    /// request. Leaving it non-terminal would pin its `layer_head` against GC, block its
    /// branch from deletion under `CheckPins`, and refuse consolidation over it.
    #[tokio::test]
    async fn cancelling_a_suspended_task_reaches_terminal_immediately() {
        let backend: Arc<dyn crate::storage::PersistentBackend> =
            Arc::new(crate::storage::memory::MemoryPersistentBackend::new());
        let store: Arc<dyn crate::task::TaskStore> =
            Arc::new(crate::task::BackendTaskStore::new(Arc::clone(&backend)));
        let mut service = EigeniusService::new().expect("service");
        service.task_store = Some(Arc::clone(&store));

        let session_id = service.session.read().await.session_id;
        let task_id = uuid::Uuid::new_v4();
        let mut record = TaskRecord::new_running(
            session_id,
            task_id,
            "urn:eigenius:test:program:p".to_string(),
            "urn:eigenius:test:input:i".to_string(),
            crate::layer::LayerId([3u8; 32]),
            0,
        );
        record.status = TaskStatus::Suspended;
        store.put_task(&record).expect("put");

        let resp = service
            .handle_cancel_task(CancelTaskRequest {
                task_id: task_id.to_string(),
            })
            .await
            .expect("cancel")
            .into_inner();
        assert!(resp.success, "error was: {}", resp.error);
        assert_eq!(resp.status, format!("{:?}", TaskStatus::Cancelled));

        let stored = store
            .get_task(&session_id, &task_id)
            .expect("get")
            .expect("record");
        assert_eq!(stored.status, TaskStatus::Cancelled);
        assert!(
            stored.status.is_terminal(),
            "terminal is what releases the three pins"
        );
    }

    /// A `Running` task still goes to `Cancelling`: it may be driven by a live evaluator
    /// in this process, and whether that evaluator stops is eigenius#51. The resume sweep
    /// finalises it if it turns out to be a crash leftover.
    #[tokio::test]
    async fn cancelling_a_running_task_still_awaits_the_grace_window() {
        let backend: Arc<dyn crate::storage::PersistentBackend> =
            Arc::new(crate::storage::memory::MemoryPersistentBackend::new());
        let store: Arc<dyn crate::task::TaskStore> =
            Arc::new(crate::task::BackendTaskStore::new(Arc::clone(&backend)));
        let mut service = EigeniusService::new().expect("service");
        service.task_store = Some(Arc::clone(&store));

        let session_id = service.session.read().await.session_id;
        let task_id = uuid::Uuid::new_v4();
        let record = TaskRecord::new_running(
            session_id,
            task_id,
            "urn:eigenius:test:program:p".to_string(),
            "urn:eigenius:test:input:i".to_string(),
            crate::layer::LayerId([3u8; 32]),
            0,
        );
        store.put_task(&record).expect("put");

        let resp = service
            .handle_cancel_task(CancelTaskRequest {
                task_id: task_id.to_string(),
            })
            .await
            .expect("cancel")
            .into_inner();
        assert_eq!(resp.status, format!("{:?}", TaskStatus::Cancelling));
    }

    /// `StartReindex` without embedders is a refusal with a reason, not a silent success.
    #[tokio::test]
    async fn start_reindex_without_embedders_refuses() {
        let service = EigeniusService::new().expect("service");
        let resp = service
            .handle_start_reindex(StartReindexRequest {
                layer: String::new(),
            })
            .await
            .expect("start")
            .into_inner();
        assert!(!resp.success);
        assert!(resp.error.contains("no embedders"), "got: {}", resp.error);
    }

    /// With embedders but nothing stale, detection answers "nothing to do" and no task
    /// starts — the pre-check the RPC relies on.
    #[tokio::test]
    async fn start_reindex_with_no_stale_segments_starts_nothing() {
        let (service, _task_id, _handle) = service_with_in_flight_sweep();
        let resp = service
            .handle_start_reindex(StartReindexRequest {
                layer: String::new(),
            })
            .await
            .expect("start")
            .into_inner();
        assert!(resp.success, "error was: {}", resp.error);
        assert!(resp.indexes.is_empty());
    }
}
