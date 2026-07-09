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

//! Sessions, tasks, and checkpoints — storage primitives (D21).
//!
//! This module owns the value types and storage helpers that back
//! Phase 9b-iii's resumable task model. Higher layers (the evaluator,
//! the gRPC server) compose these primitives into the `RunProgram`
//! async flow, the `ListTasks` / `CancelTask` RPCs, and the startup
//! resume sweep. See D21 for the full design.
//!
//! Keyspace (under `PersistentBackend::put_meta` / `write_batch`):
//!
//! ```text
//!   session:<session_id>:task:<task_id>:meta      -> TaskRecord (CBOR)
//!   session:<session_id>:task:<task_id>:trace:<N> -> ComponentTrace (CBOR)
//!   session:<session_id>:task:<task_id>:ckpt:<N>  -> Checkpoint (CBOR)
//! ```
//!
//! In 9b-iii, every `session_id` is `Uuid::nil()` — the single
//! hardwired session (D21 §3.7). Multi-session lands in Phase 14.

pub mod reindex;
pub mod sweep;
pub mod sweep_registry;

use crate::layer::LayerId;
use crate::storage::{BatchOp, PersistentBackend, StorageError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// Failure modes specific to task I/O.
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("CBOR decode error: {0}")]
    Decode(String),
    #[error("CBOR encode error: {0}")]
    Encode(String),
    #[error("task not found: {0}")]
    NotFound(Uuid),
}

/// A session — the client-scoped unit that tasks attach to (D21 §3.7).
///
/// In 9b-iii there is exactly one session per running kernel, with
/// `session_id = Uuid::nil()`. The session's *active top* is the
/// kernel's current head (they are synonyms in v1 — see D21 §3.7).
/// The `EigeniusService` reads `context.head()` to recover the
/// active top; `Session` itself only carries identity.
#[derive(Debug, Clone, Copy)]
pub struct Session {
    pub session_id: Uuid,
}

impl Session {
    /// The single hardwired session for 9b-iii (D21 §3.7).
    pub fn hardwired() -> Self {
        Self {
            session_id: Uuid::nil(),
        }
    }
}

/// Lifecycle state of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TaskStatus {
    /// Actively evaluating.
    Running,
    /// Persisted mid-flight but not being driven right now (e.g.,
    /// kernel crashed and we haven't picked it back up yet).
    Suspended,
    /// Cancel requested; waiting on the cooperative grace window
    /// (D21 §8 cancellation).
    Cancelling,
    /// Terminated successfully; `result_layer_head` is set.
    Completed,
    /// Terminated with an error.
    Failed,
    /// Cancel completed.
    Cancelled,
}

impl TaskStatus {
    /// Whether a task in this state should be enqueued by the resume
    /// sweep on kernel startup.
    pub fn is_resumable(&self) -> bool {
        matches!(self, TaskStatus::Running | TaskStatus::Suspended)
    }

    /// Whether a task in this state is in a terminal state and will
    /// not run again.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }
}

/// Persistent metadata about a task (D21 §3.1, §7).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskRecord {
    /// The enclosing session. Always `Uuid::nil()` in 9b-iii.
    pub session_id: Uuid,
    pub task_id: Uuid,
    pub program_iri: String,
    pub input_iri: String,
    /// Pinned layer at `RunProgram` entry (D21 §8.1). All Read
    /// dispatches during the task read against this specific layer;
    /// resume reconstructs the chain from here.
    pub layer_head: LayerId,
    pub status: TaskStatus,
    /// Monotonic counter — incremented on every IO dispatch.
    pub step_seq: u64,
    /// `step_seq` of the latest committed checkpoint, if any.
    pub last_checkpoint: Option<u64>,
    /// `step_seq` of the latest stored trace. Tracks pruning.
    pub latest_trace_seq: u64,
    /// Milliseconds since Unix epoch.
    pub created_at: i64,
    pub updated_at: i64,
    /// Set on completion with the `parent = layer_head` result layer
    /// (D21 §3.7).
    pub result_layer_head: Option<LayerId>,
    /// Per-task audit retention override — when `true`, observation
    /// traces survive the `--audit-retention` TTL (D21 §8).
    pub retain_forever: bool,
}

impl TaskRecord {
    /// Construct a fresh `Running` task record for a freshly-started
    /// `RunProgram` invocation.
    pub fn new_running(
        session_id: Uuid,
        task_id: Uuid,
        program_iri: String,
        input_iri: String,
        layer_head: LayerId,
        now_millis: i64,
    ) -> Self {
        Self {
            session_id,
            task_id,
            program_iri,
            input_iri,
            layer_head,
            status: TaskStatus::Running,
            step_seq: 0,
            last_checkpoint: None,
            latest_trace_seq: 0,
            created_at: now_millis,
            updated_at: now_millis,
            result_layer_head: None,
            retain_forever: false,
        }
    }

    /// Encode this record to CBOR for persistence.
    pub fn to_cbor(&self) -> Result<Vec<u8>, TaskError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|e| TaskError::Encode(e.to_string()))?;
        Ok(buf)
    }

    /// Decode a persisted record.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, TaskError> {
        ciborium::from_reader(bytes).map_err(|e| TaskError::Decode(e.to_string()))
    }
}

/// A checkpoint — a snapshot of task state at a safe resumption
/// boundary (D21 §4).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub session_id: Uuid,
    pub task_id: Uuid,
    pub step_seq: u64,
    /// CBOR-encoded Resource — the program's declared task state at
    /// this step.
    pub state: Vec<u8>,
    pub created_at: i64,
}

impl Checkpoint {
    pub fn to_cbor(&self) -> Result<Vec<u8>, TaskError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|e| TaskError::Encode(e.to_string()))?;
        Ok(buf)
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<Self, TaskError> {
        ciborium::from_reader(bytes).map_err(|e| TaskError::Decode(e.to_string()))
    }
}

/// The live evaluation context for an executing task.
///
/// Threaded through the IO effect engine so the evaluator can route IO
/// dispatches through per-task positional trace keys (D21 §3.2)
/// instead of the cross-task content-address cache.
///
/// Fields are shared (Arc / Atomic) because cooperative cancellation
/// (D21 §8 cancellation) and the async RunProgram path both hand the
/// same `TaskContext` across tokio tasks.
pub struct TaskContext {
    pub session_id: Uuid,
    pub task_id: Uuid,
    /// Monotonic step counter. Incremented at each IO dispatch or
    /// replay consumption.
    pub step_seq: AtomicU64,
    /// The store this task's traces + record + checkpoints are
    /// persisted through.
    pub task_store: Arc<dyn TaskStore>,
    /// Cooperative cancellation flag (D21 §8). The evaluator checks
    /// this between IO dispatches; `CancelTask` flips it.
    pub cancel_requested: std::sync::atomic::AtomicBool,
}

impl TaskContext {
    pub fn new(session_id: Uuid, task_id: Uuid, task_store: Arc<dyn TaskStore>) -> Self {
        Self {
            session_id,
            task_id,
            step_seq: AtomicU64::new(0),
            task_store,
            cancel_requested: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Atomically fetch the current step seq and increment.
    pub fn next_step(&self) -> u64 {
        self.step_seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Read (without incrementing) the next step seq the evaluator
    /// will consume.
    pub fn current_step(&self) -> u64 {
        self.step_seq.load(Ordering::SeqCst)
    }

    /// Cooperative cancellation check. Callers should test this
    /// between IO dispatches; `true` means the evaluator should
    /// unwind via a `CancelTask`-induced error path.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }

    /// Flip the cancellation flag.
    pub fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
    }
}

impl std::fmt::Debug for TaskContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskContext")
            .field("session_id", &self.session_id)
            .field("task_id", &self.task_id)
            .field("step_seq", &self.current_step())
            .field("cancel_requested", &self.is_cancelled())
            .finish()
    }
}

// --- Keyspace helpers -------------------------------------------------

/// `session:<id>:task:<id>:meta`
pub fn task_meta_key(session_id: &Uuid, task_id: &Uuid) -> String {
    format!("session:{session_id}:task:{task_id}:meta")
}

/// `session:<id>:task:<id>:trace:<step_seq>`
pub fn task_trace_key(session_id: &Uuid, task_id: &Uuid, step_seq: u64) -> String {
    format!("session:{session_id}:task:{task_id}:trace:{step_seq}")
}

/// `session:<id>:task:<id>:ckpt:<step_seq>`
pub fn task_ckpt_key(session_id: &Uuid, task_id: &Uuid, step_seq: u64) -> String {
    format!("session:{session_id}:task:{task_id}:ckpt:{step_seq}")
}

/// Prefix used by the resume sweep to list all task meta records in a
/// session.
pub fn task_meta_prefix(session_id: &Uuid) -> String {
    format!("session:{session_id}:task:")
}

// --- Task store trait & backend adapter -------------------------------

/// Persistence API for tasks. Mirrors `BackendTraceStore` in shape.
///
/// The default implementation, `BackendTaskStore`, forwards to a
/// `PersistentBackend`. In-memory alternatives (tests, ephemeral
/// kernels) can provide their own impl without reaching for RocksDB.
pub trait TaskStore: Send + Sync {
    fn put_task(&self, record: &TaskRecord) -> Result<(), TaskError>;
    fn get_task(&self, session_id: &Uuid, task_id: &Uuid) -> Result<Option<TaskRecord>, TaskError>;
    fn delete_task(&self, session_id: &Uuid, task_id: &Uuid) -> Result<(), TaskError>;
    fn list_tasks(&self, session_id: &Uuid) -> Result<Vec<TaskRecord>, TaskError>;

    fn put_checkpoint(&self, ckpt: &Checkpoint) -> Result<(), TaskError>;
    fn get_checkpoint(
        &self,
        session_id: &Uuid,
        task_id: &Uuid,
        step_seq: u64,
    ) -> Result<Option<Checkpoint>, TaskError>;

    /// Read the stored trace bytes for a specific step. Callers decode
    /// these as a `ComponentTrace` output (typically the output
    /// `Resource` in CBOR). Returns `Ok(None)` if that step has not
    /// been traced yet — the caller's cue to actually dispatch the
    /// component (D21 §6 resume protocol).
    fn get_trace_bytes(
        &self,
        session_id: &Uuid,
        task_id: &Uuid,
        step_seq: u64,
    ) -> Result<Option<Vec<u8>>, TaskError>;

    /// Apply a task-step write atomically: the new `TaskRecord`, the
    /// new `ComponentTrace` bytes, and — on checkpoint steps — the
    /// new `Checkpoint`. All three land via one backend `write_batch`
    /// call (D21 §8 step atomicity).
    fn commit_step(
        &self,
        record: &TaskRecord,
        trace_bytes: Option<(u64, Vec<u8>)>,
        checkpoint: Option<&Checkpoint>,
    ) -> Result<(), TaskError>;
}

/// Adapter that backs a `TaskStore` with a `PersistentBackend`.
pub struct BackendTaskStore {
    backend: Arc<dyn PersistentBackend>,
}

impl BackendTaskStore {
    pub fn new(backend: Arc<dyn PersistentBackend>) -> Self {
        Self { backend }
    }
}

impl TaskStore for BackendTaskStore {
    fn put_task(&self, record: &TaskRecord) -> Result<(), TaskError> {
        let key = task_meta_key(&record.session_id, &record.task_id);
        let bytes = record.to_cbor()?;
        self.backend.put_meta(&key, &bytes)?;
        Ok(())
    }

    fn get_task(&self, session_id: &Uuid, task_id: &Uuid) -> Result<Option<TaskRecord>, TaskError> {
        let key = task_meta_key(session_id, task_id);
        match self.backend.get_meta(&key)? {
            Some(bytes) => Ok(Some(TaskRecord::from_cbor(&bytes)?)),
            None => Ok(None),
        }
    }

    fn delete_task(&self, session_id: &Uuid, task_id: &Uuid) -> Result<(), TaskError> {
        let key = task_meta_key(session_id, task_id);
        self.backend.delete_meta(&key)?;
        Ok(())
    }

    fn list_tasks(&self, session_id: &Uuid) -> Result<Vec<TaskRecord>, TaskError> {
        let prefix = task_meta_prefix(session_id);
        let keys = self.backend.list_meta_prefix(&prefix)?;
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            if !k.ends_with(":meta") {
                continue; // also matches trace/ckpt entries under the shared prefix
            }
            if let Some(bytes) = self.backend.get_meta(&k)? {
                out.push(TaskRecord::from_cbor(&bytes)?);
            }
        }
        Ok(out)
    }

    fn put_checkpoint(&self, ckpt: &Checkpoint) -> Result<(), TaskError> {
        let key = task_ckpt_key(&ckpt.session_id, &ckpt.task_id, ckpt.step_seq);
        let bytes = ckpt.to_cbor()?;
        self.backend.put_meta(&key, &bytes)?;
        Ok(())
    }

    fn get_checkpoint(
        &self,
        session_id: &Uuid,
        task_id: &Uuid,
        step_seq: u64,
    ) -> Result<Option<Checkpoint>, TaskError> {
        let key = task_ckpt_key(session_id, task_id, step_seq);
        match self.backend.get_meta(&key)? {
            Some(bytes) => Ok(Some(Checkpoint::from_cbor(&bytes)?)),
            None => Ok(None),
        }
    }

    fn get_trace_bytes(
        &self,
        session_id: &Uuid,
        task_id: &Uuid,
        step_seq: u64,
    ) -> Result<Option<Vec<u8>>, TaskError> {
        let key = task_trace_key(session_id, task_id, step_seq);
        Ok(self.backend.get_meta(&key)?)
    }

    fn commit_step(
        &self,
        record: &TaskRecord,
        trace_bytes: Option<(u64, Vec<u8>)>,
        checkpoint: Option<&Checkpoint>,
    ) -> Result<(), TaskError> {
        let mut ops = Vec::with_capacity(3);
        ops.push(BatchOp::PutMeta {
            key: task_meta_key(&record.session_id, &record.task_id),
            value: record.to_cbor()?,
        });
        if let Some((seq, bytes)) = trace_bytes {
            ops.push(BatchOp::PutMeta {
                key: task_trace_key(&record.session_id, &record.task_id, seq),
                value: bytes,
            });
        }
        if let Some(ckpt) = checkpoint {
            ops.push(BatchOp::PutMeta {
                key: task_ckpt_key(&ckpt.session_id, &ckpt.task_id, ckpt.step_seq),
                value: ckpt.to_cbor()?,
            });
        }
        self.backend.write_batch(&ops)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_layer_id() -> LayerId {
        LayerId([0x11; 32])
    }

    #[test]
    fn task_record_cbor_roundtrip() {
        let rec = TaskRecord::new_running(
            Uuid::nil(),
            Uuid::from_u128(0x1234_5678_9abc_def0_0000_0000_0000_0001),
            "urn:eigenius:test:program:foo".to_string(),
            "urn:eigenius:test:input:1".to_string(),
            fresh_layer_id(),
            1_700_000_000_000,
        );
        let bytes = rec.to_cbor().unwrap();
        let back = TaskRecord::from_cbor(&bytes).unwrap();
        assert_eq!(back.task_id, rec.task_id);
        assert_eq!(back.status, TaskStatus::Running);
        assert_eq!(back.layer_head, rec.layer_head);
        assert!(back.last_checkpoint.is_none());
    }

    #[test]
    fn checkpoint_cbor_roundtrip() {
        let ckpt = Checkpoint {
            session_id: Uuid::nil(),
            task_id: Uuid::from_u128(42),
            step_seq: 7,
            state: vec![0xde, 0xad, 0xbe, 0xef],
            created_at: 1,
        };
        let back = Checkpoint::from_cbor(&ckpt.to_cbor().unwrap()).unwrap();
        assert_eq!(back.step_seq, 7);
        assert_eq!(back.state, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn task_status_transitions() {
        assert!(TaskStatus::Running.is_resumable());
        assert!(TaskStatus::Suspended.is_resumable());
        assert!(!TaskStatus::Completed.is_resumable());
        assert!(TaskStatus::Completed.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
    }

    #[test]
    fn keyspace_shape() {
        let s = Uuid::nil();
        let t = Uuid::from_u128(1);
        assert_eq!(task_meta_key(&s, &t), format!("session:{s}:task:{t}:meta"));
        assert_eq!(
            task_trace_key(&s, &t, 5),
            format!("session:{s}:task:{t}:trace:5")
        );
        assert_eq!(
            task_ckpt_key(&s, &t, 5),
            format!("session:{s}:task:{t}:ckpt:5")
        );
        assert!(task_meta_prefix(&s).starts_with("session:"));
        assert!(task_meta_prefix(&s).ends_with(":task:"));
    }

    #[test]
    fn hardwired_session() {
        let s = Session::hardwired();
        assert_eq!(s.session_id, Uuid::nil());
    }
}
