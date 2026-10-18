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

//! Phase 18d Julia capstone test. Builds a substrate-built OCI image
//! that extends `julia:1.12-bookworm`, spawns a worker, dispatches a
//! Julia one-liner, and verifies the trace.
//!
//! Skipped when:
//! - Built without `--features test-runtime,docker-spawner`
//! - `buildah` not installed
//! - Docker daemon unreachable
//! - Julia base image cannot be pulled (offline / no registry access)
//!
//! First run (cold): pulls `julia:1.12-bookworm` (~500MB) +
//! `Pkg.instantiate` + `Pkg.precompile` (~1-2 min) → ~3-5 min total.
//! Subsequent runs hit Docker's layer cache and the substrate's
//! image cache → <30s.

#![cfg(all(feature = "test-runtime", feature = "docker-spawner"))]

use eigenius_julia::JuliaLanguageRuntime;
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::facade::SubstrateDispatcher;
use eigenius_runtime_substrate::is_buildah_available;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::spawner::service::DockerServiceSpawner;
use eigenius_runtime_substrate::spawner::DockerSpawnerConfig;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-julia-it-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create depot");
    dir
}

/// Resolve the path to `julia/runtime-worker/` from the crate. The
/// crate's Cargo.toml lives at `crates/runtime-substrate/Cargo.toml`,
/// so the workspace root is two levels up.
fn julia_project_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("julia")
        .join("runtime-worker")
        .canonicalize()
        .expect("julia/runtime-worker/ must exist relative to runtime-substrate's Cargo.toml")
}

fn is_docker_available() -> bool {
    Path::new("/var/run/docker.sock").exists()
}

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
            "expected RepoDigests entry like `julia@sha256:...`, got `{pinned}`"
        ));
    }
    // Buildah requires fully-qualified refs (or a short-name alias in
    // /etc/containers/registries.conf.d/shortnames.conf). Docker's
    // RepoDigests returns bare short names like `julia@sha256:...`
    // for images pulled from Docker Hub. Alpine/Debian/Ubuntu happen
    // to have aliases in the system shortnames db so they round-trip
    // by accident; Julia doesn't. Qualify explicitly so the test
    // works regardless of the host's shortnames coverage.
    let qualified = if pinned.contains('/') {
        // Already has a registry component (e.g. `docker.io/library/...`).
        pinned
    } else {
        format!("docker.io/library/{pinned}")
    };
    Ok(qualified)
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

#[ignore = "heavy E2E: full Julia capstone (env image + cross-check trace)."]
#[test]
fn julia_capstone_full_e2e() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping 18d julia capstone: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping 18d julia capstone (could not pin base image): {e}");
            return;
        }
    };

    let depot = fresh_depot("e2e");
    let spawner = match DockerServiceSpawner::new(DockerSpawnerConfig::new(depot.clone())) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("skipping (DockerServiceSpawner construction failed): {e}");
            return;
        }
    };

    let project_dir = julia_project_dir();
    let runtime = Arc::new(JuliaLanguageRuntime::new(
        project_dir,
        pinned_base,
        spawner.clone(),
        depot.clone(),
    ));

    // Build is the substantive cold step — exercises buildah, multi-
    // asset materialisation (Project.toml + Manifest.toml +
    // src/JuliaWorker.jl), and `Pkg.instantiate` + `Pkg.precompile`
    // inside `julia:1.12-bookworm`.
    let language_runtime: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let env = Resource::new_embedded();
    let digest = match language_runtime.build_environment_image(&env, &[], None) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("substrate-built julia image failed: {e}");
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("build_environment_image: {e}");
        }
    };
    assert!(
        digest.as_str().starts_with("sha256:"),
        "expected sha256-shaped digest, got {digest}"
    );

    let mut dispatcher = SubstrateDispatcher::new();
    dispatcher
        .register_language_runtime(language_runtime)
        .expect("register julia runtime");

    // A deterministic Julia one-liner. The worker stringifies the
    // expression's value (no stdout capture in 18d's minimal worker
    // — see the worker's `dispatch_julia` for the rationale).
    let argument = build_argument("julia", "uppercase(\"phase 18d capstone\")");
    let outcome = dispatcher
        .dispatch_run_runtime_script(&[], &argument)
        .expect("dispatch julia hello-world");

    let output = eigon_cbor::parse_resource_lenient(&outcome.output_cbor).expect("decode output");
    // Phase 19a.1: production crate uses `urn:eigenius:runtime:script_output`
    // (was `urn:eigenius:test:julia_output` on the deprecated test fixture).
    let julia_output = output
        .get(&Iri::parse("urn:eigenius:runtime:script_output").unwrap())
        .and_then(Value::as_str)
        .expect("script_output property on output");
    assert_eq!(julia_output, "PHASE 18D CAPSTONE");

    // Drain at end so the docker container doesn't linger.
    let cleanup_runtime = runtime.clone();
    let _ = cleanup_runtime.drain();

    // Trace fields: language=julia, image_digest=substrate-built,
    // worker reports host_kernel="julia-test-runtime" (so we know
    // the Julia worker is the one that answered).
    let inv = eigon_cbor::parse_resource_lenient(&outcome.partial_invocation_cbor)
        .expect("decode partial invocation");
    assert_eq!(
        inv.get(&Iri::parse("urn:eigenius:runtime:language").unwrap())
            .and_then(Value::as_str),
        Some("julia")
    );
    assert_eq!(
        inv.get(&Iri::parse("urn:eigenius:runtime:image_digest").unwrap())
            .and_then(Value::as_str),
        Some(digest.as_str()),
        "trace must echo the substrate-built image digest"
    );
    let metadata = inv
        .get(&Iri::parse("urn:eigenius:runtime:numerical_metadata").unwrap())
        .expect("numerical_metadata present");
    match metadata {
        Value::Json(json) => {
            assert_eq!(
                json.get("host_kernel").and_then(serde_json::Value::as_str),
                Some("julia-test-runtime"),
                "Julia worker must report host_kernel = julia-test-runtime via Health"
            );
        }
        other => panic!("expected Value::Json for numerical_metadata, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&depot);
}

#[ignore = "heavy E2E: full Julia capstone with provenance-tampering check."]
#[test]
fn julia_capstone_cross_check_tampering_fires() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping 18d cross-check tampering test: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping (base image pin failed): {e}");
            return;
        }
    };

    let depot = fresh_depot("xcheck");
    let spawner = match DockerServiceSpawner::new(DockerSpawnerConfig::new(depot.clone())) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("skipping (DockerServiceSpawner construction failed): {e}");
            return;
        }
    };

    let project_dir = julia_project_dir();
    let runtime = Arc::new(JuliaLanguageRuntime::new(
        project_dir,
        pinned_base,
        spawner.clone(),
        depot.clone(),
    ));

    // Build the image (cached if a previous test ran), then spawn
    // with deliberately mismatched cross-check env. The worker reads
    // the env, compares against the in-image manifest-hash, sees the
    // mismatch, exits 78 — DockerSpawner's attach_uds path surfaces
    // this as SpawnError::WorkerCrossCheckFailed.
    let env = Resource::new_embedded();
    let _digest = runtime
        .build_environment_image(&env, &[], None)
        .expect("build image");

    // Spawn directly via the runtime, then tamper with the worker
    // spec's manifest-hash before the worker reads it. We do this by
    // calling spawn_worker through a sneaked-in hook? Actually,
    // simpler: call query_health on a deliberately-mismatched spawn.
    //
    // The JuliaLanguageRuntime constructs cross-check env from
    // its cached manifest_hash. To force a mismatch we'd need to
    // either (a) tamper after spawn (race), or (b) build the image
    // with one hash and spawn with a different one. Cleanest: spawn
    // a worker with a manifest-hash override that doesn't match the
    // image's baked-in hash.
    //
    // Rather than re-architect for tamper-injection, we exercise the
    // same code path via the existing `docker_spawner_integration`
    // test (which uses an alpine image emitting exit 78 directly) —
    // that already covers DockerSpawner's classification logic. For
    // the Julia-specific path, we verify the *image-side* cross-check
    // by running the worker with a manually-constructed env that
    // mismatches the baked-in hash, then checking the container
    // exits with code 78.
    //
    // Implementation: spawn a one-shot container manually with
    // mismatched env vars and inspect its exit code. This pins
    // "Julia worker correctly enforces cross-check" without needing
    // to plumb tamper-injection through the substrate runtime.
    let _digest_str = _digest.as_str().to_string();
    // Run via `docker run --rm` with a deliberately-wrong manifest hash:
    let output = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--network",
            "none",
            "-e",
            "EIGENIUS_TEST_WORKER_UDS=/tmp/should-never-exist.sock",
            "-e",
            &format!("EIGENIUS_RUNTIME_ENV_DIGEST={}", _digest_str),
            "-e",
            "EIGENIUS_RUNTIME_ENV_MANIFEST_HASH=sha256:deliberately-wrong-hash-for-tampering-test-0000000000000000",
        ])
        .arg(_digest_str)
        .output()
        .expect("docker run");
    // Worker exits 78 on cross-check failure.
    assert_eq!(
        output.status.code(),
        Some(78),
        "expected JuliaWorker to exit 78 on cross-check tampering; \
         stdout={:?} stderr={:?} status={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        output.status,
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cross-check failed"),
        "expected stderr to mention cross-check failure; got: {:?}",
        String::from_utf8_lossy(&output.stderr),
    );

    // Suppress unused-spawner warning — kept in scope so it's clear
    // the test is configured via the same spawner the e2e test uses,
    // even though the cross-check tampering check bypasses the
    // spawner and runs `docker run` directly for tamper-injection.
    let _ = spawner;
    let _ = std::fs::remove_dir_all(&depot);
}
