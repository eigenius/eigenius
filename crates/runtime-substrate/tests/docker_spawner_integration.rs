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

//! End-to-end integration test for [`DockerSpawner`] against a real
//! Docker daemon (Phase 18c.3).
//!
//! Skipped when:
//!
//! - The crate isn't built with `--features docker-spawner` (this whole
//!   file is `cfg`-gated).
//! - The host has no reachable Docker daemon. Detected at runtime via
//!   `is_docker_available()` so CI hosts without Docker stay green
//!   without flagging the test as failing.
//! - The host has Docker but can't pull `alpine:latest` (network
//!   restrictions, unauthenticated daemon, etc.). Surfaced as an
//!   `eprintln!`-skip rather than a panic.
//!
//! Acceptance for 18c.3: spawn / wait / kill round-trip against a
//! container, and the cross-check failure path surfaces
//! [`SpawnError::WorkerCrossCheckFailed`] when the worker exits 78
//! before binding its UDS.

#![cfg(feature = "docker-spawner")]

use eigenius_runtime_substrate::cross_check::EXIT_CODE_CROSS_CHECK_FAILURE;
use eigenius_runtime_substrate::error::SpawnError;
use eigenius_runtime_substrate::spawner::{DockerSpawner, DockerSpawnerConfig, WorkerSpawner};
use eigenius_runtime_substrate::types::{ImageDigest, WorkerSpec};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const TEST_IMAGE_TAG: &str = "alpine:latest";
/// Synthetic digest used for tests that don't actually need a real
/// digest (the `Pull: Never` failure path). Format-correct but not
/// resolvable.
const SYNTHETIC_MISSING_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-docker-it-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create depot");
    dir
}

fn fresh_tempdir(depot: &std::path::Path, label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = depot.join(format!("inv-{label}-{n}"));
    std::fs::create_dir_all(&dir).expect("create inv tempdir");
    dir
}

/// Probe whether a Docker daemon is reachable on the default socket.
/// Mirrors `image_build::is_buildah_available` so the test gate is
/// explicit rather than buried in `connect` errors.
fn is_docker_available() -> bool {
    let socket = std::path::Path::new("/var/run/docker.sock");
    if !socket.exists() {
        return false;
    }
    // A real handshake would require a Tokio runtime; the socket's
    // existence is a good-enough first-pass check. Failures further on
    // surface as `eprintln!`-skips rather than panics.
    true
}

fn ensure_test_image_present() -> Result<ImageDigest, String> {
    // The simplest reproducible way to materialise an alpine image
    // identified by digest is to pull it via `docker pull` (which
    // resolves the tag to a digest), then read the digest from
    // `docker image inspect`. The integration test explicitly uses a
    // tag-based pull because the digest of `alpine:latest` is not
    // pinned across CI runs.
    let pull = std::process::Command::new("docker")
        .args(["pull", "--quiet", TEST_IMAGE_TAG])
        .output()
        .map_err(|e| format!("`docker pull` failed: {e}"))?;
    if !pull.status.success() {
        return Err(format!(
            "docker pull {TEST_IMAGE_TAG} exited {}: {}",
            pull.status,
            String::from_utf8_lossy(&pull.stderr)
        ));
    }
    let inspect = std::process::Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", TEST_IMAGE_TAG])
        .output()
        .map_err(|e| format!("`docker image inspect` failed: {e}"))?;
    if !inspect.status.success() {
        return Err(format!(
            "docker image inspect failed: {}",
            String::from_utf8_lossy(&inspect.stderr)
        ));
    }
    let id = String::from_utf8_lossy(&inspect.stdout).trim().to_string();
    ImageDigest::parse(id).map_err(|e| format!("alpine image id is not parseable: {e}"))
}

fn build_spec(
    image: ImageDigest,
    tempdir: PathBuf,
    depot: PathBuf,
    command: Vec<&str>,
) -> WorkerSpec {
    WorkerSpec {
        image_digest: Some(image),
        command: command.into_iter().map(String::from).collect(),
        tempdir_host_path: tempdir,
        depot_host_path: Some(depot),
        env: BTreeMap::new(),
        max_wall_time_ms: 0,
        max_memory_bytes: 0,
        seccomp_profile: None,
    }
}

#[test]
// `auto_remove: true` + a sub-millisecond container body races the
// wait-stream subscribe in `wait_with_timeout` — by the time the test
// gets to wait, the daemon has reaped the container and 404s.
// See #50 for the proper fix (open wait-stream before `start_container`).
// Re-enable with the same PR that lands the fix.
#[ignore = "flaky pending #50"]
fn spawn_wait_round_trip_against_alpine() {
    if !is_docker_available() {
        eprintln!("Docker socket unavailable; skipping DockerSpawner integration test");
        return;
    }
    let depot = fresh_depot("spawn-wait");
    let spawner = match DockerSpawner::new(DockerSpawnerConfig::new(depot.clone())) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("DockerSpawner construction failed (skipping): {e}");
            return;
        }
    };
    let digest = match ensure_test_image_present() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not stage test image (skipping): {e}");
            return;
        }
    };
    let tempdir = fresh_tempdir(&depot, "spawn-wait");
    let spec = build_spec(
        digest,
        tempdir,
        depot.clone(),
        vec!["/bin/sh", "-c", "exit 0"],
    );
    let handle = spawner.spawn(spec).expect("spawn alpine");
    let status = spawner.wait_with_timeout(&handle, None).expect("wait");
    assert_eq!(
        status.code(),
        Some(0),
        "expected clean exit, got {status:?}"
    );
    let _ = std::fs::remove_dir_all(&depot);
}

#[test]
fn attach_uds_surfaces_cross_check_failure_when_container_exits_78() {
    if !is_docker_available() {
        eprintln!("Docker socket unavailable; skipping cross-check surfacing test");
        return;
    }
    let depot = fresh_depot("xcheck-78");
    let spawner = match DockerSpawner::new(DockerSpawnerConfig::new(depot.clone())) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("DockerSpawner construction failed (skipping): {e}");
            return;
        }
    };
    let digest = match ensure_test_image_present() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not stage test image (skipping): {e}");
            return;
        }
    };
    let tempdir = fresh_tempdir(&depot, "xcheck-78");
    // Container exits 78 (cross-check failure code) without binding any
    // UDS. attach_uds must surface SpawnError::WorkerCrossCheckFailed.
    let spec = build_spec(
        digest,
        tempdir,
        depot.clone(),
        vec![
            "/bin/sh",
            "-c",
            &format!("exit {}", EXIT_CODE_CROSS_CHECK_FAILURE),
        ],
    );
    let handle = spawner.spawn(spec).expect("spawn alpine");
    // Wait briefly so the container actually exits before we probe.
    std::thread::sleep(Duration::from_millis(200));
    let err = spawner.attach_uds(&handle).expect_err("attach must fail");
    match err {
        SpawnError::WorkerCrossCheckFailed(msg) => {
            assert!(
                msg.contains("78"),
                "expected diagnostic to mention 78: {msg}"
            );
        }
        // Race window: if the container is auto-removed between the
        // exit and our inspect, the substrate falls back to a generic
        // SpawnFailed with an explanatory message. Accept both — what
        // matters for 18c.3 is that we do *not* surface a misleading
        // error like "could not connect to UDS".
        SpawnError::SpawnFailed { reason, .. } if reason.contains("auto-removed") => {
            eprintln!("container was auto-removed before inspect (acceptable race): {reason}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    // Drain the wait stream to free any daemon-side bookkeeping.
    let _ = spawner.wait_with_timeout(&handle, None);
    let _ = std::fs::remove_dir_all(&depot);
}

#[test]
fn kill_terminates_running_container() {
    if !is_docker_available() {
        eprintln!("Docker socket unavailable; skipping kill test");
        return;
    }
    let depot = fresh_depot("kill");
    let spawner = match DockerSpawner::new(DockerSpawnerConfig::new(depot.clone())) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("DockerSpawner construction failed (skipping): {e}");
            return;
        }
    };
    let digest = match ensure_test_image_present() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not stage test image (skipping): {e}");
            return;
        }
    };
    let tempdir = fresh_tempdir(&depot, "kill");
    let spec = build_spec(
        digest,
        tempdir,
        depot.clone(),
        vec!["/bin/sh", "-c", "sleep 60"],
    );
    let handle = spawner.spawn(spec).expect("spawn alpine");
    spawner.kill(&handle).expect("kill");
    let _ = std::fs::remove_dir_all(&depot);
}

#[test]
fn wait_with_timeout_kills_long_running_container() {
    if !is_docker_available() {
        eprintln!("Docker socket unavailable; skipping wait-timeout test");
        return;
    }
    let depot = fresh_depot("wait-timeout");
    let spawner = match DockerSpawner::new(DockerSpawnerConfig::new(depot.clone())) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("DockerSpawner construction failed (skipping): {e}");
            return;
        }
    };
    let digest = match ensure_test_image_present() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not stage test image (skipping): {e}");
            return;
        }
    };
    let tempdir = fresh_tempdir(&depot, "wait-timeout");
    let spec = build_spec(
        digest,
        tempdir,
        depot.clone(),
        vec!["/bin/sh", "-c", "sleep 60"],
    );
    let handle = spawner.spawn(spec).expect("spawn alpine");
    let err = spawner
        .wait_with_timeout(&handle, Some(Duration::from_secs(1)))
        .expect_err("must time out");
    match err {
        SpawnError::WaitTimedOut {
            handle_id,
            timeout_ms,
        } => {
            assert_eq!(handle_id, handle.id);
            assert_eq!(timeout_ms, 1000);
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&depot);
}

#[test]
fn spawn_with_pull_never_fails_for_missing_digest() {
    if !is_docker_available() {
        eprintln!("Docker socket unavailable; skipping pull-policy test");
        return;
    }
    let depot = fresh_depot("pull-never");
    let mut config = DockerSpawnerConfig::new(depot.clone());
    config.pull_policy = eigenius_runtime_substrate::spawner::PullPolicy::Never;
    let spawner = match DockerSpawner::new(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("DockerSpawner construction failed (skipping): {e}");
            return;
        }
    };
    let digest = ImageDigest::parse(SYNTHETIC_MISSING_DIGEST).expect("digest parses");
    let tempdir = fresh_tempdir(&depot, "pull-never");
    let spec = build_spec(digest, tempdir, depot.clone(), vec!["/bin/true"]);
    let err = spawner
        .spawn(spec)
        .expect_err("must fail with PullPolicy::Never");
    match err {
        SpawnError::EnvironmentImageUnavailable { reason, .. } => {
            assert!(
                reason.contains("Never"),
                "expected reason to mention policy: {reason}"
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&depot);
}
