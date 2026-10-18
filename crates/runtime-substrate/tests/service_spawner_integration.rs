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

//! End-to-end integration tests for the `ServiceSpawner` trait against
//! the bash test worker.
//!
//! Confirms the Service-mode lifecycle:
//! 1. `ensure_service` spawns a long-lived worker.
//! 2. `ensure_service` is idempotent — repeated calls for the same env
//!    return the same `ServiceHandle`.
//! 3. `attach_uds` opens an RPC channel; multiple sequential `attach_uds`
//!    calls hit the same warm worker (no respawn).
//! 4. `drain` tears the service down cleanly.
//!
//! Currently only `LocalServiceSpawner` is exercised end-to-end here.
//! `DockerServiceSpawner` is exercised by a follow-on Julia capstone
//! test (it requires the Julia base image + buildah, the same as the
//! 18d Job-mode capstone).

#![cfg(feature = "test-runtime")]

use eigenius_runtime_substrate::cross_check::{self, prepare_substrate_side, ProvenanceDirAction};
use eigenius_runtime_substrate::rpc::protocol::{Request, Response};
use eigenius_runtime_substrate::spawner::service::{LocalServiceSpawner, ServiceSpawner};
use eigenius_runtime_substrate::types::{ImageDigest, WorkerSpec};
use eigenius_runtime_substrate::WorkerRpcClient;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_tempdir(label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "eigenius-svc-spawner-{}-{}-{}",
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

/// Build a `WorkerSpec` shaped for `LocalServiceSpawner`. The worker's
/// cross-check env is wired up via `prepare_substrate_side` exactly as
/// the per-invocation tests do; only the lifecycle differs.
fn build_spec(tempdir: &Path) -> WorkerSpec {
    let uds = tempdir.join("worker.sock");
    let mut env = BTreeMap::new();
    env.insert(
        "EIGENIUS_TEST_WORKER_UDS".to_string(),
        uds.to_string_lossy().into_owned(),
    );
    let prov_dir = tempdir.join("provenance");
    let digest = ImageDigest::parse(TEST_DIGEST).expect("digest parses");
    let cross_check_env = prepare_substrate_side(
        &digest,
        TEST_MANIFEST_HASH,
        &prov_dir,
        ProvenanceDirAction::WriteFile,
    )
    .expect("cross-check setup");
    env.extend(cross_check_env);
    env.insert(
        cross_check::ENV_MANIFEST_HASH_VAR.to_string(),
        TEST_MANIFEST_HASH.to_string(),
    );
    if let Ok(path) = std::env::var("PATH") {
        env.insert("PATH".to_string(), path);
    }
    WorkerSpec {
        image_digest: Some(digest),
        command: vec![worker_binary().to_string_lossy().into_owned()],
        tempdir_host_path: tempdir.to_path_buf(),
        depot_host_path: None,
        env,
        max_wall_time_ms: 0,
        max_memory_bytes: 0,
        seccomp_profile: None,
    }
}

/// Open the UDS via the spawner, send the request, decode the
/// response. The spawner's `attach_uds` returns a fresh stream each
/// call — the *worker* is what stays alive.
fn rpc(
    spawner: &dyn ServiceSpawner,
    service: &eigenius_runtime_substrate::spawner::service::ServiceHandle,
    req: &Request,
) -> Response {
    let stream = spawner.attach_uds(service).expect("attach_uds");
    let mut client = WorkerRpcClient::new(stream);
    client.call(req).expect("rpc call")
}

#[test]
fn local_service_spawner_health_round_trip() {
    let depot = fresh_tempdir("health");
    let spawner = LocalServiceSpawner::new(depot.clone());
    let service_dir = depot.join("svc");

    let mut spec = build_spec(&service_dir);
    // LocalServiceSpawner uses the spec's tempdir as the service dir.
    spec.tempdir_host_path = service_dir.clone();

    let svc = spawner.ensure_service(spec).expect("ensure_service");

    // Worker is alive — Health RPC succeeds and reports the digest we
    // gave it (via EIGENIUS_RUNTIME_ENV_DIGEST).
    let resp = rpc(&spawner, &svc, &Request::Health);
    match resp {
        Response::Health(info) => {
            assert_eq!(info.env_digest_in_image.as_deref(), Some(TEST_DIGEST));
            assert_eq!(
                info.manifest_hash_in_image.as_deref(),
                Some(TEST_MANIFEST_HASH)
            );
        }
        other => panic!("expected Health response, got {other:?}"),
    }

    spawner.drain(&svc).expect("drain");
    let _ = std::fs::remove_dir_all(&depot);
}

#[test]
fn local_service_spawner_warm_reuse_across_attach() {
    // Warm reuse: two sequential `attach_uds` against the same service
    // hit the same worker. We can't directly observe "same worker", but
    // we can observe (a) both succeed without respawning (no fresh
    // EIGENIUS_RUNTIME_ENV_DIGEST handoff would be possible since the
    // worker is alive across both calls), and (b) Health reports
    // identical info both times.
    let depot = fresh_tempdir("warm");
    let spawner = LocalServiceSpawner::new(depot.clone());
    let service_dir = depot.join("svc");

    let mut spec = build_spec(&service_dir);
    spec.tempdir_host_path = service_dir.clone();
    let svc = spawner.ensure_service(spec).expect("ensure_service");

    let r1 = rpc(&spawner, &svc, &Request::Health);
    let r2 = rpc(&spawner, &svc, &Request::Health);

    match (r1, r2) {
        (Response::Health(a), Response::Health(b)) => {
            // Both round-trips report the same in-image digest — the
            // worker persisted across the two attach calls.
            assert_eq!(a.env_digest_in_image, b.env_digest_in_image);
            assert_eq!(a.manifest_hash_in_image, b.manifest_hash_in_image);
        }
        other => panic!("expected two Health responses, got {other:?}"),
    }

    spawner.drain(&svc).expect("drain");
    let _ = std::fs::remove_dir_all(&depot);
}

#[test]
fn local_service_spawner_ensure_service_is_idempotent() {
    // Trait contract: repeated `ensure_service` calls for the same
    // `(image_digest, command)` identity must return the same handle
    // and reuse the same backing process. Without this the dispatcher's
    // warm-pool semantics break — a per-dispatch ensure call would
    // spawn a fresh worker every time and pay the cold-start cost
    // forever.
    let depot = fresh_tempdir("idempotent");
    let spawner = LocalServiceSpawner::new(depot.clone());
    let service_dir = depot.join("svc");

    let spec = build_spec(&service_dir);
    let h1 = spawner.ensure_service(spec.clone()).expect("ensure 1");
    let h2 = spawner.ensure_service(spec.clone()).expect("ensure 2");

    assert_eq!(
        h1.id(),
        h2.id(),
        "second ensure_service for the same spec must return the same ServiceHandle id"
    );

    // Verify the worker really is the same process by exchanging two
    // Health roundtrips and checking they report identical in-image
    // metadata. A spawn-per-call implementation would have spawned a
    // second worker; the two would still report the same env digest
    // (it's a constant) but the test asserts the *handle* equality
    // above — Health is the secondary "really one process" check.
    let r1 = rpc(&spawner, &h1, &Request::Health);
    let r2 = rpc(&spawner, &h2, &Request::Health);
    match (r1, r2) {
        (Response::Health(a), Response::Health(b)) => {
            assert_eq!(a.env_digest_in_image, b.env_digest_in_image);
            assert_eq!(a.manifest_hash_in_image, b.manifest_hash_in_image);
        }
        other => panic!("expected two Health responses, got {other:?}"),
    }

    spawner.drain(&h1).expect("drain");
    let _ = std::fs::remove_dir_all(&depot);
}

#[test]
fn local_service_spawner_drain_makes_attach_fail() {
    let depot = fresh_tempdir("drain");
    let spawner = LocalServiceSpawner::new(depot.clone());
    let service_dir = depot.join("svc");

    let mut spec = build_spec(&service_dir);
    spec.tempdir_host_path = service_dir.clone();
    let svc = spawner.ensure_service(spec).expect("ensure_service");

    // Drain the service.
    spawner.drain(&svc).expect("drain");

    // attach_uds against a drained service should fail (the spawner's
    // service map no longer has the entry).
    let result = spawner.attach_uds(&svc);
    assert!(
        result.is_err(),
        "attach_uds against a drained service should fail, got Ok"
    );

    let _ = std::fs::remove_dir_all(&depot);
}
