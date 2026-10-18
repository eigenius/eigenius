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
//
//! Integration-level coverage for `worker_listen` — exercises the
//! UDS bind/accept path the in-crate unit tests skip by constructing
//! `WorkerHandle` directly from a `UnixStream::pair`.
//!
//! Builds a temporary UDS path, spawns the worker on a background
//! thread that drives the polling API as a Lean-like consumer (exactly
//! the shape `lean/runtime-worker/Worker/Main.lean` will use), has
//! the test's main thread connect as the substrate-side client,
//! runs a handful of requests through, and asserts the worker exits
//! cleanly when sent `Evict`.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use eigenius_lean_worker::{
    worker_close, worker_free_owned_bytes, worker_listen, worker_next_request_kind,
    worker_request_function_name, worker_request_input_count, worker_send_dispatch_failed,
    worker_send_dispatch_ok, worker_send_evicted, worker_send_health, worker_send_instantiated,
    worker_send_mirror_registered, Bytes, OwnedBytes, RequestKind,
};
use eigenius_runtime_substrate::rpc::client::WorkerRpcClient;
use eigenius_runtime_substrate::rpc::method::MethodInvocation;
use eigenius_runtime_substrate::rpc::protocol::{Request, Response, TargetKind};

fn unique_uds_path() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "eigenius-lean-worker-test-{}-{}.sock",
        std::process::id(),
        n
    ));
    path
}

fn connect_with_retry(path: &std::path::Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(path) {
            Ok(s) => return s,
            Err(e) if Instant::now() < deadline => {
                if e.kind() != std::io::ErrorKind::NotFound
                    && e.kind() != std::io::ErrorKind::ConnectionRefused
                {
                    panic!("unexpected connect error: {e}");
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("failed to connect to worker UDS within timeout: {e}"),
        }
    }
}

fn decode_kind(i: i32) -> RequestKind {
    match i {
        0 => RequestKind::Health,
        1 => RequestKind::Instantiate,
        2 => RequestKind::RegisterMirror,
        3 => RequestKind::DispatchMethod,
        4 => RequestKind::Evict,
        -1 => RequestKind::Closed,
        -2 => RequestKind::TransportError,
        -3 => RequestKind::UnsupportedScriptKind,
        -4 => RequestKind::MalformedMethodInvocation,
        other => panic!("unknown request kind from worker: {other}"),
    }
}

fn owned_to_string(b: OwnedBytes) -> String {
    let s = unsafe { std::slice::from_raw_parts(b.ptr, b.len) }.to_vec();
    let result = String::from_utf8(s).expect("utf-8");
    unsafe { worker_free_owned_bytes(b) };
    result
}

/// Run a Lean-like worker loop against the given handle. Treats
/// `lean_export` as a fixed-bytes responder; everything else gets a
/// canned response. Mirrors what `Worker/Main.lean` will do, in
/// Rust so we can drive the production API end-to-end without
/// needing the Lean toolchain in scope for this test.
unsafe fn drive_worker_loop(
    handle: *mut eigenius_lean_worker::WorkerHandle,
    lean_export_response: &[u8],
) {
    loop {
        let kind = decode_kind(unsafe { worker_next_request_kind(handle) });
        match kind {
            RequestKind::Health => {
                let _ = unsafe { worker_send_health(handle) };
            }
            RequestKind::Instantiate => {
                let _ = unsafe { worker_send_instantiated(handle, true) };
            }
            RequestKind::RegisterMirror => {
                let _ = unsafe {
                    worker_send_mirror_registered(handle, Bytes::from_slice(b"unused-iri"))
                };
            }
            RequestKind::DispatchMethod => {
                let function_name =
                    owned_to_string(unsafe { worker_request_function_name(handle) });
                if function_name == "lean_export" {
                    let _ = unsafe {
                        worker_send_dispatch_ok(
                            handle,
                            Bytes::from_slice(lean_export_response),
                            Bytes::from_slice(&[]),
                        )
                    };
                } else {
                    let msg = format!("function `{function_name}` not implemented");
                    let _ = unsafe {
                        worker_send_dispatch_failed(
                            handle,
                            Bytes::from_slice(b"not_implemented"),
                            Bytes::from_slice(msg.as_bytes()),
                        )
                    };
                }
            }
            RequestKind::Evict => {
                let _ = unsafe { worker_send_evicted(handle) };
                return;
            }
            RequestKind::UnsupportedScriptKind | RequestKind::MalformedMethodInvocation => {
                let _ = unsafe {
                    worker_send_dispatch_failed(
                        handle,
                        Bytes::from_slice(b"method_signature_mismatch"),
                        Bytes::from_slice(b"unsupported dispatch shape"),
                    )
                };
            }
            RequestKind::Closed | RequestKind::TransportError => return,
        }
    }
}

#[test]
fn worker_listen_binds_uds_and_round_trips_lean_export() {
    let uds_path = unique_uds_path();
    let canned_response = b"\x01\x02\x03 lean export bytes".to_vec();

    let server_path = uds_path.clone();
    let server_response = canned_response.clone();
    let server_handle = thread::spawn(move || {
        let path_bytes = server_path.to_string_lossy().into_owned();
        let handle = unsafe { worker_listen(path_bytes.as_ptr(), path_bytes.len()) };
        assert!(
            !handle.is_null(),
            "worker_listen must succeed against a fresh path"
        );
        unsafe { drive_worker_loop(handle, &server_response) };
        unsafe { worker_close(handle) };
    });

    let stream = connect_with_retry(&uds_path);
    let mut client = WorkerRpcClient::new(stream);

    let health = client.call(&Request::Health).expect("health");
    assert!(matches!(health, Response::Health(_)));

    let mi = MethodInvocation {
        function_name: "lean_export".to_string(),
        signature_iri: "urn:eigenius:test:lean:methods:lean_export".to_string(),
    };
    let mut target_cbor = Vec::new();
    ciborium::into_writer(&mi, &mut target_cbor).expect("encode");
    let dispatch = client
        .call(&Request::DispatchMethod {
            invocation_id: "uds-inv-1".to_string(),
            target_kind: TargetKind::Method,
            target: serde_bytes::ByteBuf::from(target_cbor),
            inputs: vec![],
        })
        .expect("dispatch");
    match dispatch {
        Response::DispatchOk {
            invocation_id,
            output,
            derivations: _,
            dispatched_to,
        } => {
            assert_eq!(invocation_id, "uds-inv-1");
            assert_eq!(output.as_ref(), canned_response.as_slice());
            assert_eq!(
                dispatched_to.as_deref(),
                Some("urn:eigenius:test:lean:methods:lean_export")
            );
        }
        other => panic!("expected DispatchOk, got {other:?}"),
    }

    let evicted = client.call(&Request::Evict).expect("evict");
    assert!(matches!(evicted, Response::Evicted));
    drop(client);

    server_handle.join().expect("worker thread join");

    let _ = std::fs::remove_file(&uds_path);
}

#[test]
fn worker_listen_recovers_from_stale_socket_path() {
    let uds_path = unique_uds_path();

    // Pre-create a file at the UDS path — `worker_listen` should
    // remove it before binding.
    std::fs::write(&uds_path, b"leftover").expect("create stale file");

    let server_path = uds_path.clone();
    let server_handle = thread::spawn(move || {
        let path_bytes = server_path.to_string_lossy().into_owned();
        let handle = unsafe { worker_listen(path_bytes.as_ptr(), path_bytes.len()) };
        assert!(
            !handle.is_null(),
            "worker_listen must succeed past stale path"
        );
        unsafe { drive_worker_loop(handle, b"") };
        unsafe { worker_close(handle) };
    });

    let stream = connect_with_retry(&uds_path);
    let mut client = WorkerRpcClient::new(stream);
    let _ = client
        .call(&Request::Health)
        .expect("health on rebound socket");
    let _ = client.call(&Request::Evict);
    drop(client);

    server_handle.join().expect("worker thread join");
    let _ = std::fs::remove_file(&uds_path);
}

#[test]
fn dispatch_with_inputs_exposes_input_count_through_accessor() {
    // Exercises the input-count + per-input accessor combo against
    // a real UDS connection. Same shape the Lean code will follow:
    // ask how many inputs, then iterate by index.
    let uds_path = unique_uds_path();
    let server_path = uds_path.clone();
    let server_handle = thread::spawn(move || {
        let path_bytes = server_path.to_string_lossy().into_owned();
        let handle = unsafe { worker_listen(path_bytes.as_ptr(), path_bytes.len()) };
        assert!(!handle.is_null());
        loop {
            let kind = decode_kind(unsafe { worker_next_request_kind(handle) });
            match kind {
                RequestKind::Health => {
                    let _ = unsafe { worker_send_health(handle) };
                }
                RequestKind::DispatchMethod => {
                    let count = unsafe { worker_request_input_count(handle) };
                    let payload = format!("inputs={count}");
                    let _ = unsafe {
                        worker_send_dispatch_ok(
                            handle,
                            Bytes::from_slice(payload.as_bytes()),
                            Bytes::from_slice(&[]),
                        )
                    };
                }
                RequestKind::Evict => {
                    let _ = unsafe { worker_send_evicted(handle) };
                    break;
                }
                _ => break,
            }
        }
        unsafe { worker_close(handle) };
    });

    let stream = connect_with_retry(&uds_path);
    let mut client = WorkerRpcClient::new(stream);

    let mi = MethodInvocation {
        function_name: "lean_export".to_string(),
        signature_iri: "urn:eigenius:test:lean:methods:lean_export".to_string(),
    };
    let mut target_cbor = Vec::new();
    ciborium::into_writer(&mi, &mut target_cbor).expect("encode");
    let resp = client
        .call(&Request::DispatchMethod {
            invocation_id: "uds-inv-2".to_string(),
            target_kind: TargetKind::Method,
            target: serde_bytes::ByteBuf::from(target_cbor),
            inputs: vec![
                serde_bytes::ByteBuf::from(vec![0x01]),
                serde_bytes::ByteBuf::from(vec![0x02]),
                serde_bytes::ByteBuf::from(vec![0x03]),
            ],
        })
        .expect("dispatch");
    match resp {
        Response::DispatchOk { output, .. } => {
            assert_eq!(output.as_ref(), b"inputs=3");
        }
        other => panic!("expected DispatchOk, got {other:?}"),
    }

    let _ = client.call(&Request::Evict);
    drop(client);
    server_handle.join().expect("worker thread join");
    let _ = std::fs::remove_file(&uds_path);
}
