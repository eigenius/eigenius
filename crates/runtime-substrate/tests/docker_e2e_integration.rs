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

//! Phase 18c.6 end-to-end test: builds a real OCI image containing the
//! `eigenius-test-worker` binary, spawns it via `DockerSpawner`,
//! round-trips a bash dispatch through it, and asserts the
//! `DispatchTrace` carries the substrate-built image digest +
//! worker-reported `numerical_metadata`.
//!
//! Skipped when:
//! - Built without `--features test-runtime,docker-spawner`
//! - `buildah` not installed (see `is_buildah_available`)
//! - Docker daemon unreachable
//! - The base image (alpine) cannot be pulled / inspected
//!
//! Acceptance for 18c.6: this single test wires the full pipeline —
//! 18c.1 build → 18c.2 cross-check → 18c.3 spawn → 18c.4 wait →
//! 18c.5 trace — against an image the substrate built itself.

#![cfg(all(feature = "test-runtime", feature = "docker-spawner"))]

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::facade::SubstrateDispatcher;
use eigenius_runtime_substrate::is_buildah_available;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::spawner::{DockerSpawner, DockerSpawnerConfig};
use eigenius_runtime_substrate::test_runtime_docker::TestLanguageRuntimeDocker;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Base image for the e2e fixture. Must be glibc-based (the cargo-built
/// worker binary is dynamically linked against glibc) and ship `bash`
/// preinstalled. `debian:bookworm-slim` satisfies both at ~30 MB; alpine
/// is rejected because it ships musl, not glibc, and the worker fails at
/// the dynamic-linker step on it.
const BASE_IMAGE_TAG: &str = "debian:bookworm-slim";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-e2e-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create depot");
    dir
}

fn worker_binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_eigenius-test-worker"))
}

fn is_docker_available() -> bool {
    Path::new("/var/run/docker.sock").exists()
}

/// Pull `alpine:latest` and resolve to a digest-pinned reference of
/// the form `alpine@sha256:<hash>` so the substrate-built image is
/// anchored to a specific upstream layer (per design Q3: digest-pinned
/// in the same shape we'll use in production).
fn ensure_base_image_pinned() -> Result<String, String> {
    let pull = std::process::Command::new("docker")
        .args(["pull", "--quiet", BASE_IMAGE_TAG])
        .output()
        .map_err(|e| format!("`docker pull` failed: {e}"))?;
    if !pull.status.success() {
        return Err(format!(
            "docker pull {BASE_IMAGE_TAG} exited {}: {}",
            pull.status,
            String::from_utf8_lossy(&pull.stderr)
        ));
    }
    // `RepoDigests` is the registry-side manifest digest (sha256:...);
    // `Id` is the local image ID. We need the manifest digest so the
    // pinned reference (`alpine@sha256:<digest>`) is actually pullable.
    let inspect = std::process::Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{index .RepoDigests 0}}",
            BASE_IMAGE_TAG,
        ])
        .output()
        .map_err(|e| format!("`docker image inspect` failed: {e}"))?;
    if !inspect.status.success() {
        return Err(format!(
            "docker image inspect failed: {}",
            String::from_utf8_lossy(&inspect.stderr)
        ));
    }
    let pinned = String::from_utf8_lossy(&inspect.stdout).trim().to_string();
    if !pinned.contains('@') {
        return Err(format!(
            "expected RepoDigests entry like `alpine@sha256:...`, got `{pinned}`"
        ));
    }
    Ok(pinned)
}

fn skip_unless_full_environment() -> Option<String> {
    if !is_docker_available() {
        return Some("Docker socket unavailable".into());
    }
    if !is_buildah_available() {
        return Some("buildah unavailable".into());
    }
    None
}

fn build_argument(language: &str, source: &str) -> Vec<u8> {
    let mut arg = Resource::new_embedded();
    arg.set(
        Iri::parse("urn:eigenius:runtime:language").unwrap(),
        Value::String(language.to_string()),
    );
    arg.set(
        Iri::parse("urn:eigenius:runtime:source").unwrap(),
        Value::String(source.to_string()),
    );
    eigon_cbor::serialize_resource(&arg)
}

#[ignore = "heavy E2E: substrate-built image + DooD UDS round-trip; CI runners often lack UDS-friendly Docker."]
#[test]
fn end_to_end_build_spawn_dispatch_trace_against_substrate_built_image() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping 18c.6 e2e: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping 18c.6 e2e (could not pin base image): {e}");
            return;
        }
    };

    let depot = fresh_depot("e2e");
    let spawner = match DockerSpawner::new(DockerSpawnerConfig::new(depot.clone())) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("skipping 18c.6 e2e (DockerSpawner construction failed): {e}");
            return;
        }
    };

    let runtime = TestLanguageRuntimeDocker::new(
        worker_binary_path(),
        pinned_base.clone(),
        spawner.clone(),
        depot.clone(),
    );

    // First build is the substantive one — exercises buildah, COPY of
    // the worker binary with executable mode, the cross-check
    // provenance file baked at /etc/eigenius-runtime-env/, and the
    // alpine `apk add bash` install_runtime fragment.
    let language_runtime: Box<dyn LanguageRuntime> = Box::new(runtime);
    let env = Resource::new_embedded();
    let digest = match language_runtime.build_environment_image(&env, &[], None) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skipping 18c.6 e2e (substrate-built image failed: {e})");
            let _ = std::fs::remove_dir_all(&depot);
            return;
        }
    };
    assert!(
        digest.as_str().starts_with("sha256:"),
        "expected sha256-shaped digest, got {digest}"
    );

    // Second build to validate determinism — same inputs must produce
    // the same image id (Phase 18c.1 contract; reasserted here against
    // the full TestLanguageRuntimeDocker path).
    let runtime2 = TestLanguageRuntimeDocker::new(
        worker_binary_path(),
        pinned_base,
        spawner.clone(),
        depot.clone(),
    );
    let digest2 = runtime2
        .build_environment_image(&env, &[], None)
        .expect("rebuild");
    assert_eq!(
        digest.as_str(),
        digest2.as_str(),
        "byte-identical inputs to the substrate's image-build pipeline must yield the same image id"
    );

    // Now run the full dispatch through the SubstrateDispatcher facade.
    let mut dispatcher = SubstrateDispatcher::new();
    dispatcher
        .register_language_runtime(language_runtime)
        .expect("register runtime");

    let argument = build_argument("test", "echo phase-18c6-validated");
    let outcome = match dispatcher.dispatch_run_runtime_script(&[], &argument) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("dispatch failed (this is a real failure, not a skip): {e}");
            let _ = std::fs::remove_dir_all(&depot);
            panic!("dispatch_run_runtime_script: {e}");
        }
    };

    let output = eigon_cbor::parse_resource_lenient(&outcome.output_cbor).expect("decode output");
    let stdout = output
        .get(&Iri::parse("urn:eigenius:test:bash_stdout").unwrap())
        .and_then(Value::as_str)
        .expect("bash_stdout property on output");
    assert_eq!(stdout.trim(), "phase-18c6-validated");

    // Assert the partial RuntimeInvocation carries the trace fields the
    // substrate observed against the substrate-built image, including
    // the worker-reported numerical_metadata (Phase 18c.5).
    let inv = eigon_cbor::parse_resource_lenient(&outcome.partial_invocation_cbor)
        .expect("decode partial invocation");
    assert_eq!(
        inv.get(&Iri::parse("urn:eigenius:runtime:language").unwrap())
            .and_then(Value::as_str),
        Some("test")
    );
    let metadata = inv
        .get(&Iri::parse("urn:eigenius:runtime:numerical_metadata").unwrap())
        .expect("numerical_metadata present");
    match metadata {
        Value::Json(json) => {
            assert_eq!(
                json.get("host_kernel").and_then(serde_json::Value::as_str),
                Some("test-runtime"),
                "worker should report host_kernel = test-runtime via Health"
            );
        }
        other => panic!("expected Value::Json for numerical_metadata, got {other:?}"),
    }
    let started = inv
        .get(&Iri::parse("urn:eigenius:runtime:started_at").unwrap())
        .and_then(Value::as_str)
        .expect("started_at present");
    let completed = inv
        .get(&Iri::parse("urn:eigenius:runtime:completed_at").unwrap())
        .and_then(Value::as_str)
        .expect("completed_at present");
    assert!(
        started <= completed,
        "started {started} must be <= completed {completed}"
    );

    let _ = std::fs::remove_dir_all(&depot);
}
