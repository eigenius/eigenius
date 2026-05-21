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
//! Phase 20a.5b.2 capstone: spawn the Lake-built Lean worker
//! binary, connect over UDS as the substrate-side client, drive
//! the protocol through Health + DispatchMethod{lean_export} +
//! Evict, and verify each round-trip.
//!
//! The Rust cdylib + C bridge + Lean `@[extern]` declarations are
//! all exercised by this single test — if it passes, the FFI
//! plumbing is wired correctly end to end.
//!
//! ## Why this is `#[ignore]`'d by default
//!
//! The test requires the Lake-built worker binary at
//! `lean/runtime-worker/.lake/build/bin/lean-runtime-worker` —
//! which in turn requires Lean + Lake on `PATH` and a `lake build`
//! to have run. CI without a Lean toolchain shouldn't fail on this;
//! the `#[ignore]` keeps the test out of the default workspace
//! check. Run explicitly via:
//!
//! ```text
//! cargo test -p eigenius-lean-worker --test lean_worker_e2e -- --ignored
//! ```

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use eigenius_runtime_substrate::rpc::client::WorkerRpcClient;
use eigenius_runtime_substrate::rpc::method::MethodInvocation;
use eigenius_runtime_substrate::rpc::protocol::{Request, Response, TargetKind};

/// Locate the Lake-built worker binary. Returns `None` if it
/// hasn't been built — the test self-skips in that case rather
/// than asserting on a missing artifact.
fn locate_worker_binary() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // crates/eigenius-lean-worker/
    let workspace_root = manifest_dir.parent()?.parent()?; // crates/ -> root
    let candidate = workspace_root
        .join("lean")
        .join("runtime-worker")
        .join(".lake")
        .join("build")
        .join("bin")
        .join("lean-runtime-worker");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn unique_uds_path() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "eigenius-lean-worker-e2e-{}-{}.sock",
        std::process::id(),
        n
    ));
    path
}

/// Spawn the worker binary with the given UDS path as argv[1].
/// Returns the child handle so the test can clean up. Inherits
/// stderr so the worker's `IO.eprintln` diagnostics surface in
/// the test output.
fn spawn_worker(binary: &PathBuf, uds_path: &PathBuf) -> Child {
    Command::new(binary)
        .arg(uds_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn lean-runtime-worker")
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
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("failed to connect to worker UDS within timeout: {e}"),
        }
    }
}

/// Wait for the child to exit, with a timeout. Returns the exit
/// status; kills the child on timeout to avoid leaking processes.
fn wait_for_exit(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("try_wait failed: {e}"),
        }
    }
    let _ = child.kill();
    child.wait().expect("wait after kill")
}

#[test]
#[ignore = "requires lake-built worker binary; run with --ignored after `lake build`"]
fn lean_worker_round_trips_health_dispatch_evict() {
    let binary = match locate_worker_binary() {
        Some(p) => p,
        None => {
            eprintln!(
                "Lake-built worker binary not found — skipping. \
                 Run `(cd lean/runtime-worker && lake build)` first."
            );
            return;
        }
    };

    let uds_path = unique_uds_path();
    let mut child = spawn_worker(&binary, &uds_path);

    // Connect (worker is binding + accepting in parallel).
    let stream = connect_with_retry(&uds_path);
    let mut client = WorkerRpcClient::new(stream);

    // --- Health round-trip ---
    let health = client.call(&Request::Health).expect("health");
    match health {
        Response::Health(_) => {}
        other => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("expected Response::Health, got {other:?}");
        }
    }

    // --- DispatchMethod{lean_export} round-trip ---
    // The Lean handler stub returns DispatchFailed{not_implemented}
    // for 20a.5b.2; we verify that exact shape so the routing
    // path is confirmed (function_name decoded Rust-side, passed
    // through FFI to Lean, Lean dispatched to runLeanExport,
    // Lean called sendDispatchFailed, response wire-encoded).
    let mi = MethodInvocation {
        function_name: "lean_export".to_string(),
        signature_iri: "urn:eigenius:test:lean:methods:lean_export".to_string(),
    };
    let mut target_cbor = Vec::new();
    ciborium::into_writer(&mi, &mut target_cbor).expect("encode");
    let dispatch = client
        .call(&Request::DispatchMethod {
            invocation_id: "e2e-inv-1".to_string(),
            target_kind: TargetKind::Method,
            target: serde_bytes::ByteBuf::from(target_cbor),
            inputs: vec![],
        })
        .expect("dispatch");
    match dispatch {
        Response::DispatchFailed {
            invocation_id,
            error_kind,
            message,
        } => {
            assert_eq!(invocation_id, "e2e-inv-1");
            assert_eq!(error_kind, "not_implemented");
            assert!(
                message.contains("20a.5b.3"),
                "expected Lean's 20a.5b.3-pending diagnostic, got: {message}"
            );
        }
        other => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("expected DispatchFailed, got {other:?}");
        }
    }

    // --- Evict round-trip + clean exit ---
    let evicted = client.call(&Request::Evict).expect("evict");
    assert!(matches!(evicted, Response::Evicted));
    drop(client);

    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    assert!(
        status.success(),
        "worker should exit cleanly after Evict; got {status:?}"
    );

    let _ = std::fs::remove_file(&uds_path);
}

#[test]
#[ignore = "requires lake-built worker binary; run with --ignored after `lake build`"]
fn lean_worker_unknown_function_routes_to_dispatch_failed() {
    // Confirms the Lean side reads `function_name` from the FFI
    // accessor, compares against "lean_export", and falls through
    // to the unknown-function DispatchFailed branch. Tightens
    // coverage on the `requestFunctionName` → `asString` →
    // string-compare path.
    let binary = match locate_worker_binary() {
        Some(p) => p,
        None => return,
    };

    let uds_path = unique_uds_path();
    let mut child = spawn_worker(&binary, &uds_path);
    let stream = connect_with_retry(&uds_path);
    let mut client = WorkerRpcClient::new(stream);

    let mi = MethodInvocation {
        function_name: "compute_some_user_thing".to_string(),
        signature_iri: "urn:eigenius:test:lean:methods:user_thing".to_string(),
    };
    let mut target_cbor = Vec::new();
    ciborium::into_writer(&mi, &mut target_cbor).expect("encode");
    let resp = client
        .call(&Request::DispatchMethod {
            invocation_id: "e2e-inv-2".to_string(),
            target_kind: TargetKind::Method,
            target: serde_bytes::ByteBuf::from(target_cbor),
            inputs: vec![],
        })
        .expect("dispatch");
    match resp {
        Response::DispatchFailed {
            error_kind,
            message,
            ..
        } => {
            assert_eq!(error_kind, "not_implemented");
            assert!(
                message.contains("compute_some_user_thing"),
                "expected message to name the unknown function, got: {message}"
            );
        }
        other => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("expected DispatchFailed, got {other:?}");
        }
    }

    let _ = client.call(&Request::Evict);
    drop(client);
    let _ = wait_for_exit(&mut child, Duration::from_secs(5));
    let _ = std::fs::remove_file(&uds_path);
}
