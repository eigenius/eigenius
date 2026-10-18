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

//! End-to-end smoke test for the substrate skeleton.
//!
//! Drives the full Phase 18a stack from outside the crate:
//!
//! 1. [`LocalSpawner`] launches the `eigenius-test-worker` binary
//!    against a per-test tempdir, with the UDS path passed via
//!    `EIGENIUS_TEST_WORKER_UDS`.
//! 2. The substrate connects via [`WorkerRpcClient`] once the worker
//!    has bound its socket.
//! 3. Each of the five RPC verbs is exercised; payload bytes
//!    round-trip through CBOR.
//! 4. `Evict` shuts the worker down cleanly; the spawner observes a
//!    zero exit status.
//!
//! This is the test the implementation plan calls for in Phase 18a:
//! "smoke language test … wraps a long-lived bash worker speaking the
//! substrate's CBOR RPC, so the skeleton can be exercised end-to-end
//! without dragging in a real interpreter."

#![cfg(feature = "test-runtime")]

use eigenius_runtime_substrate::cross_check::{self, prepare_substrate_side, ProvenanceDirAction};
use eigenius_runtime_substrate::rpc::protocol::{Request, Response, TargetKind};
use eigenius_runtime_substrate::spawner::{LocalSpawner, WorkerSpawner};
use eigenius_runtime_substrate::types::{ImageDigest, WorkerHandle, WorkerSpec};
use eigenius_runtime_substrate::WorkerRpcClient;
use serde_bytes::ByteBuf;
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_tempdir(label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "eigenius-test-worker-{}-{}-{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn worker_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_eigenius-test-worker"))
}

const TEST_DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const TEST_MANIFEST_HASH: &str = "test-manifest";

fn build_spec(tempdir: &Path) -> WorkerSpec {
    build_spec_with_cross_check(tempdir, TEST_MANIFEST_HASH, TEST_MANIFEST_HASH)
}

/// Build a spec where the env-supplied manifest hash and the in-image
/// file's hash can be set independently — used by the cross-check
/// failure tests below to produce a mismatch.
fn build_spec_with_cross_check(
    tempdir: &Path,
    env_manifest_hash: &str,
    in_image_manifest_hash: &str,
) -> WorkerSpec {
    let uds = tempdir.join("worker.sock");
    let mut env = BTreeMap::new();
    env.insert(
        "EIGENIUS_TEST_WORKER_UDS".to_string(),
        uds.to_string_lossy().into_owned(),
    );
    let prov_dir = tempdir.join("provenance");
    let digest = ImageDigest::parse(TEST_DIGEST).expect("digest parses");
    // Substrate-side helper writes the in-image hash; we then overwrite
    // the env entry below if the test wants a deliberate mismatch.
    let cross_check_env = prepare_substrate_side(
        &digest,
        in_image_manifest_hash,
        &prov_dir,
        ProvenanceDirAction::WriteFile,
    )
    .expect("cross-check setup");
    env.extend(cross_check_env);
    env.insert(
        cross_check::ENV_MANIFEST_HASH_VAR.to_string(),
        env_manifest_hash.to_string(),
    );
    // Inherit PATH so the worker can find /bin/bash without explicit
    // wiring. PATH is the only host-env passthrough; everything else
    // is set explicitly to keep the test deterministic.
    if let Ok(path) = std::env::var("PATH") {
        env.insert("PATH".to_string(), path);
    }
    WorkerSpec {
        image_digest: None,
        command: vec![worker_binary().to_string_lossy().into_owned()],
        tempdir_host_path: tempdir.to_path_buf(),
        depot_host_path: None,
        env,
        max_wall_time_ms: 0,
        max_memory_bytes: 0,
        seccomp_profile: None,
    }
}

fn connect_when_ready(uds_path: &Path, timeout: Duration) -> UnixStream {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(uds_path) {
            Ok(s) => return s,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("worker UDS at {} never came up: {e}", uds_path.display()),
        }
    }
}

fn evict_and_wait(spawner: &LocalSpawner, client: &mut WorkerRpcClient, handle: &WorkerHandle) {
    let resp = client.call(&Request::Evict).expect("evict call");
    assert!(
        matches!(resp, Response::Evicted),
        "expected Evicted, got {resp:?}"
    );
    let status = spawner.wait_with_timeout(handle, None).expect("wait");
    assert!(
        status.success(),
        "worker exited with status {:?}",
        status.code()
    );
}

#[test]
fn full_rpc_round_trip_via_local_spawner() {
    let tempdir = fresh_tempdir("full_rpc");
    let spawner = LocalSpawner::new();
    let handle = spawner.spawn(build_spec(&tempdir)).expect("spawn worker");

    let stream = connect_when_ready(&handle.uds_path, Duration::from_secs(5));
    let mut client = WorkerRpcClient::new(stream);

    // Health: cross-check signals echo back and host_kernel is set.
    let resp = client.call(&Request::Health).expect("health call");
    match resp {
        Response::Health(info) => {
            assert_eq!(
                info.manifest_hash_in_image.as_deref(),
                Some("test-manifest")
            );
            assert_eq!(
                info.env_digest_in_image.as_deref(),
                Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            );
            assert_eq!(
                info.numerical_metadata.host_kernel.as_deref(),
                Some("test-runtime")
            );
        }
        other => panic!("unexpected health response: {other:?}"),
    }

    // Instantiate: ready=true (no real bootstrap in the test runtime).
    let resp = client
        .call(&Request::Instantiate {
            env_iri: "urn:eigenius:test:env:bash".to_string(),
            image_digest: None,
        })
        .expect("instantiate call");
    assert!(matches!(resp, Response::Instantiated { ready: true }));

    // RegisterMirror: idempotent echo.
    let resp = client
        .call(&Request::RegisterMirror {
            mirror_iri: "urn:eigenius:test:mirror:bash".to_string(),
            library_content: ByteBuf::from(vec![0u8; 8]),
        })
        .expect("register_mirror call");
    match resp {
        Response::MirrorRegistered { mirror_iri } => {
            assert_eq!(mirror_iri, "urn:eigenius:test:mirror:bash");
        }
        other => panic!("unexpected register_mirror response: {other:?}"),
    }

    // DispatchMethod: bash one-liner echoes a known string; we expect
    // the CBOR-encoded stdout to come back as a String.
    let mut target_cbor = Vec::new();
    ciborium::into_writer(&"echo eigenius-rocks".to_string(), &mut target_cbor)
        .expect("encode target");
    let resp = client
        .call(&Request::DispatchMethod {
            invocation_id: "inv-1".to_string(),
            target_kind: TargetKind::Script,
            target: ByteBuf::from(target_cbor),
            inputs: vec![],
        })
        .expect("dispatch call");
    match resp {
        Response::DispatchOk {
            invocation_id,
            output,
            ..
        } => {
            assert_eq!(invocation_id, "inv-1");
            let stdout: String =
                ciborium::from_reader(&output[..]).expect("decode output as String");
            assert_eq!(stdout.trim(), "eigenius-rocks");
        }
        other => panic!("unexpected dispatch response: {other:?}"),
    }

    evict_and_wait(&spawner, &mut client, &handle);
    let _ = std::fs::remove_dir_all(&tempdir);
}

#[test]
fn dispatch_returns_dispatch_failed_when_bash_exits_nonzero() {
    let tempdir = fresh_tempdir("dispatch_failed");
    let spawner = LocalSpawner::new();
    let handle = spawner.spawn(build_spec(&tempdir)).expect("spawn worker");
    let stream = connect_when_ready(&handle.uds_path, Duration::from_secs(5));
    let mut client = WorkerRpcClient::new(stream);

    let mut target_cbor = Vec::new();
    ciborium::into_writer(&"echo whoops 1>&2; exit 11".to_string(), &mut target_cbor)
        .expect("encode target");
    let resp = client
        .call(&Request::DispatchMethod {
            invocation_id: "inv-fail".to_string(),
            target_kind: TargetKind::Script,
            target: ByteBuf::from(target_cbor),
            inputs: vec![],
        })
        .expect("dispatch call");
    match resp {
        Response::DispatchFailed {
            invocation_id,
            error_kind,
            message,
        } => {
            assert_eq!(invocation_id, "inv-fail");
            assert_eq!(error_kind, "runtime_error");
            assert!(
                message.contains("11") && message.contains("whoops"),
                "expected exit-11 + stderr in message, got `{message}`"
            );
        }
        other => panic!("unexpected dispatch response: {other:?}"),
    }

    evict_and_wait(&spawner, &mut client, &handle);
    let _ = std::fs::remove_dir_all(&tempdir);
}

/// D26 §9.3: hash mismatch between the substrate-supplied env var and
/// the in-image manifest-hash file makes the worker exit with the
/// reserved cross-check failure code, before it ever binds the UDS.
#[test]
fn worker_refuses_to_start_on_cross_check_hash_mismatch() {
    let tempdir = fresh_tempdir("xcheck_mismatch");
    let spawner = LocalSpawner::new();
    let spec = build_spec_with_cross_check(&tempdir, "env-says-this", "file-says-that");
    let handle = spawner.spawn(spec).expect("spawn worker");
    let status = spawner
        .wait_with_timeout(&handle, None)
        .expect("wait for worker");
    assert!(
        cross_check::is_cross_check_failure(status),
        "expected EXIT_CODE_CROSS_CHECK_FAILURE, got {:?}",
        status.code()
    );
    // UDS must not have been bound — the worker exits before listen().
    assert!(
        !tempdir.join("worker.sock").exists(),
        "worker should not have bound its UDS on cross-check failure"
    );
    let _ = std::fs::remove_dir_all(&tempdir);
}

/// D26 §9.3: the substrate must always set the cross-check env vars;
/// a worker started without them refuses to come up. Same exit code as
/// the mismatch case so callers don't need to distinguish.
#[test]
fn worker_refuses_to_start_when_cross_check_env_missing() {
    let tempdir = fresh_tempdir("xcheck_no_env");
    let spawner = LocalSpawner::new();
    let mut spec = build_spec(&tempdir);
    // Drop both substrate-supplied cross-check env vars; keep only the
    // UDS path and PATH so the failure is unambiguously the cross-check.
    spec.env.remove(cross_check::ENV_DIGEST_VAR);
    spec.env.remove(cross_check::ENV_MANIFEST_HASH_VAR);
    let handle = spawner.spawn(spec).expect("spawn worker");
    let status = spawner
        .wait_with_timeout(&handle, None)
        .expect("wait for worker");
    assert!(
        cross_check::is_cross_check_failure(status),
        "expected EXIT_CODE_CROSS_CHECK_FAILURE, got {:?}",
        status.code()
    );
    let _ = std::fs::remove_dir_all(&tempdir);
}

/// D26 §9.3: env vars set, in-image file missing — worker still
/// refuses, since the substrate cannot prove it is talking to the image
/// it thinks it is.
#[test]
fn worker_refuses_to_start_when_in_image_file_missing() {
    let tempdir = fresh_tempdir("xcheck_no_file");
    let spawner = LocalSpawner::new();
    let spec = build_spec(&tempdir);
    // Delete the file the substrate just wrote, simulating an image
    // whose `/etc/eigenius-runtime-env/manifest-hash` is absent.
    let _ = std::fs::remove_file(
        tempdir
            .join("provenance")
            .join(cross_check::MANIFEST_HASH_FILE),
    );
    let handle = spawner.spawn(spec).expect("spawn worker");
    let status = spawner
        .wait_with_timeout(&handle, None)
        .expect("wait for worker");
    assert!(
        cross_check::is_cross_check_failure(status),
        "expected EXIT_CODE_CROSS_CHECK_FAILURE, got {:?}",
        status.code()
    );
    let _ = std::fs::remove_dir_all(&tempdir);
}

#[test]
fn malformed_target_yields_method_signature_mismatch() {
    let tempdir = fresh_tempdir("malformed_target");
    let spawner = LocalSpawner::new();
    let handle = spawner.spawn(build_spec(&tempdir)).expect("spawn worker");
    let stream = connect_when_ready(&handle.uds_path, Duration::from_secs(5));
    let mut client = WorkerRpcClient::new(stream);

    // CBOR `0xff` outside an indefinite-length context is not a valid
    // top-level value, let alone a String.
    let resp = client
        .call(&Request::DispatchMethod {
            invocation_id: "inv-mal".to_string(),
            target_kind: TargetKind::Script,
            target: ByteBuf::from(vec![0xff]),
            inputs: vec![],
        })
        .expect("dispatch call");
    match resp {
        Response::DispatchFailed {
            invocation_id,
            error_kind,
            ..
        } => {
            assert_eq!(invocation_id, "inv-mal");
            assert_eq!(error_kind, "method_signature_mismatch");
        }
        other => panic!("unexpected dispatch response: {other:?}"),
    }

    evict_and_wait(&spawner, &mut client, &handle);
    let _ = std::fs::remove_dir_all(&tempdir);
}
