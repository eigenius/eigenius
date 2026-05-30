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

//! Task-management RPCs: `ListTasks`, `GetTaskStatus`, `CancelTask`.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::observability::{field, operation, RpcGuard};
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_list_tasks(
        &self,
        _req: ListTasksRequest,
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
}
