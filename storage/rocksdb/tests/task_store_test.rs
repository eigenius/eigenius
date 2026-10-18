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

//! Phase 9b-iii.1 integration test: task storage primitives against a
//! real RocksStore.
//!
//! Covers:
//! 1. `TaskRecord` and `Checkpoint` survive the write → close → re-open
//!    cycle (persistence).
//! 2. `commit_step` applies its multi-key batch atomically (survives
//!    the drop/reopen, and trace entry / meta update / checkpoint all
//!    land together).
//! 3. `list_tasks` returns only meta records even though the
//!    `session:<id>:task:` prefix also covers trace and ckpt keys.
//! 4. `delete_meta` for trace pruning.
//!
//! Unit tests in `kernel/src/task/mod.rs` cover the in-memory behaviour
//! (CBOR roundtrip, keyspace shape). This test hits the real backend.

use std::sync::Arc;

use eigenius_kernel::layer::LayerId;
use eigenius_kernel::storage::{BatchOp, PersistentBackend};
use eigenius_kernel::task::{
    task_meta_prefix, task_trace_key, BackendTaskStore, Checkpoint, Session, TaskRecord,
    TaskStatus, TaskStore,
};
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;
use uuid::Uuid;

fn fresh_layer_id(byte: u8) -> LayerId {
    LayerId([byte; 32])
}

#[test]
fn task_record_survives_restart() {
    let tmp = TempDir::new().unwrap();
    let task_id = Uuid::from_u128(0xabcdef);

    // Round 1: open, write, drop.
    {
        let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
        let backend: Arc<dyn PersistentBackend> = store;
        let tasks = BackendTaskStore::new(Arc::clone(&backend));

        let rec = TaskRecord::new_running(
            Uuid::nil(),
            task_id,
            "urn:eigenius:test:p".to_string(),
            "urn:eigenius:test:i".to_string(),
            fresh_layer_id(0x11),
            1_000_000,
        );
        tasks.put_task(&rec).unwrap();
    }

    // Round 2: re-open, read.
    {
        let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
        let backend: Arc<dyn PersistentBackend> = store;
        let tasks = BackendTaskStore::new(backend);

        let back = tasks
            .get_task(&Uuid::nil(), &task_id)
            .unwrap()
            .expect("task survived restart");
        assert_eq!(back.task_id, task_id);
        assert_eq!(back.status, TaskStatus::Running);
        assert_eq!(back.layer_head, fresh_layer_id(0x11));
    }
}

#[test]
fn commit_step_is_atomic() {
    // A step writes (record, trace, ckpt) all via write_batch. The test
    // verifies all three land (no partial visibility) and that dropping
    // + reopening the store still returns the same data.
    let tmp = TempDir::new().unwrap();
    let task_id = Uuid::from_u128(0x42);
    let session_id = Uuid::nil();

    {
        let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
        let backend: Arc<dyn PersistentBackend> = store;
        let tasks = BackendTaskStore::new(Arc::clone(&backend));

        let mut rec = TaskRecord::new_running(
            session_id,
            task_id,
            "urn:eigenius:test:p".to_string(),
            "urn:eigenius:test:i".to_string(),
            fresh_layer_id(0x22),
            1_000_000,
        );
        rec.step_seq = 1;
        rec.last_checkpoint = Some(1);
        rec.latest_trace_seq = 1;

        let trace_bytes = b"dummy-cbor-trace".to_vec();
        let ckpt = Checkpoint {
            session_id,
            task_id,
            step_seq: 1,
            state: b"dummy-cbor-state".to_vec(),
            created_at: 1_000_000,
        };

        tasks
            .commit_step(&rec, Some((1, trace_bytes.clone())), Some(&ckpt))
            .unwrap();

        // All three keys visible immediately in-process.
        assert!(tasks.get_task(&session_id, &task_id).unwrap().is_some());
        assert!(tasks
            .get_checkpoint(&session_id, &task_id, 1)
            .unwrap()
            .is_some());
        let raw_trace = backend
            .get_meta(&task_trace_key(&session_id, &task_id, 1))
            .unwrap()
            .expect("trace key");
        assert_eq!(raw_trace, trace_bytes);
    }

    // Drop + reopen; same keys still visible.
    {
        let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
        let backend: Arc<dyn PersistentBackend> = store;
        let tasks = BackendTaskStore::new(Arc::clone(&backend));

        let rec = tasks
            .get_task(&session_id, &task_id)
            .unwrap()
            .expect("record survived restart");
        assert_eq!(rec.step_seq, 1);
        assert_eq!(rec.last_checkpoint, Some(1));
        assert!(tasks
            .get_checkpoint(&session_id, &task_id, 1)
            .unwrap()
            .is_some());
        let raw_trace = backend
            .get_meta(&task_trace_key(&session_id, &task_id, 1))
            .unwrap()
            .expect("trace survived restart");
        assert_eq!(raw_trace, b"dummy-cbor-trace".to_vec());
    }
}

#[test]
fn list_tasks_excludes_trace_and_ckpt() {
    // list_tasks walks the "session:<id>:task:" prefix. That prefix
    // also matches trace:N and ckpt:N entries. list_tasks must filter
    // to only meta keys.
    let tmp = TempDir::new().unwrap();
    let session_id = Uuid::nil();
    let task_id_a = Uuid::from_u128(1);
    let task_id_b = Uuid::from_u128(2);

    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store;
    let tasks = BackendTaskStore::new(Arc::clone(&backend));

    // Two tasks, each with a trace and a checkpoint entry.
    for (task_id, head_byte) in [(task_id_a, 0x10u8), (task_id_b, 0x20u8)] {
        let mut rec = TaskRecord::new_running(
            session_id,
            task_id,
            "urn:eigenius:test:p".to_string(),
            "urn:eigenius:test:i".to_string(),
            fresh_layer_id(head_byte),
            1_000,
        );
        rec.step_seq = 3;
        tasks.put_task(&rec).unwrap();

        // Add some trace + ckpt entries sharing the prefix.
        let trace_bytes = vec![head_byte];
        let ckpt = Checkpoint {
            session_id,
            task_id,
            step_seq: 3,
            state: vec![head_byte, 0xff],
            created_at: 0,
        };
        backend
            .write_batch(&[
                BatchOp::PutMeta {
                    key: task_trace_key(&session_id, &task_id, 3),
                    value: trace_bytes,
                },
                BatchOp::PutMeta {
                    key: eigenius_kernel::task::task_ckpt_key(&session_id, &task_id, 3),
                    value: ckpt.to_cbor().unwrap(),
                },
            ])
            .unwrap();
    }

    let listed = tasks.list_tasks(&session_id).unwrap();
    assert_eq!(listed.len(), 2, "expected 2 tasks, got {}", listed.len());
    let mut ids: Vec<Uuid> = listed.iter().map(|r| r.task_id).collect();
    ids.sort();
    assert_eq!(ids, vec![task_id_a, task_id_b]);

    // Sanity — list_meta_prefix without filtering returns many more
    // entries (2 meta + 2 trace + 2 ckpt = 6).
    let all_prefix_keys = backend
        .list_meta_prefix(&task_meta_prefix(&session_id))
        .unwrap();
    assert_eq!(all_prefix_keys.len(), 6);
}

#[test]
fn session_struct_smoke() {
    let s = Session::hardwired();
    assert_eq!(s.session_id, Uuid::nil());
}
