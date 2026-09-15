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

//! Task-management RPCs: `ListTasks`, `GetTaskStatus`, `CancelTask`, `StartReindex`.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::observability::{field, operation, RpcGuard};
use std::sync::Arc;
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_list_tasks(
        &self,
        _req: ListTasksRequest,
    ) -> Result<Response<ListTasksResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LIST_TASKS);
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
        Ok(Response::new(ListTasksResponse { tasks }))
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
            handle.cancel();
            return Ok(Response::new(CancelTaskResponse {
                success: true,
                status: format!("{:?}", crate::task::TaskStatus::Cancelling),
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
            .handle_list_tasks(ListTasksRequest {})
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
    }

    /// No coordinator — no embedders — means no index tasks and no crash.
    #[tokio::test]
    async fn task_rpcs_are_unchanged_without_a_coordinator() {
        let service = EigeniusService::new().expect("service");
        let resp = service
            .handle_list_tasks(ListTasksRequest {})
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
