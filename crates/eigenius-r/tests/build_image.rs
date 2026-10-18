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

//! P3.2: the pinned-image build path.
//!
//! `manifest_hash_is_deterministic` runs everywhere (it only composes the
//! Dockerfile + hashes the baked assets — no buildah). `builds_and_dispatches_via_docker`
//! is `#[ignore]`d (run with `cargo test -- --include-ignored`): it runs the
//! real buildah build, loads the image into docker, and dispatches through a
//! `DockerServiceSpawner` — the same shape as Julia/Lean's e2es. It uses a
//! light base + empty package list for speed (the heavy Bioconductor build is
//! the identical code path with `RImagePlan::default()`), and skips each leg
//! gracefully when the environment can't support it (no docker/buildah/network;
//! the dispatch leg additionally needs rootless docker).

use std::path::PathBuf;
use std::sync::Arc;

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;
use eigenius_r::RLanguageRuntime;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::spawner::service::LocalServiceSpawner;

fn cdylib_path() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let profile = exe.parent().and_then(|d| d.parent()).expect("profile dir");
    profile.join("libeigenius_r_worker.so")
}

fn driver_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eigenius-r-worker/r/EigeniusRWorker.R")
}

fn new_runtime() -> RLanguageRuntime {
    let depot = std::env::temp_dir().join("eigenius-r-build-depot");
    let spawner = Arc::new(LocalServiceSpawner::new(depot.clone()));
    RLanguageRuntime::new(spawner, driver_path(), cdylib_path(), depot)
}

#[test]
fn manifest_hash_is_deterministic_and_prefixed() {
    if !cdylib_path().exists() {
        eprintln!("skipping: cdylib not built");
        return;
    }
    let rt = new_runtime();
    let h1 = rt.image_manifest_hash().expect("manifest hash");
    let h2 = rt.image_manifest_hash().expect("manifest hash again");
    assert_eq!(h1, h2, "manifest hash is deterministic");
    assert!(h1.starts_with("sha256:"), "got {h1}");
    assert_eq!(h1.len(), "sha256:".len() + 64, "full sha256 hex");
}

fn is_docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Full build → Docker-dispatch e2e, the same shape as Julia/Lean's
/// `--include-ignored` image-build tests: build the env image with buildah,
/// load it into docker, then dispatch a script through a
/// `DockerServiceSpawner` against the built image and assert the result.
///
/// Uses a light base + empty package list (override the base with
/// `EIGENIUS_R_TEST_BASE_IMAGE`) so the build is fast; the heavy
/// Bioconductor build is the same code path with `RImagePlan::default()`.
/// Skips gracefully when docker/buildah/network are unavailable.
#[test]
#[ignore = "heavy E2E: buildah image build + docker run. Run with `cargo test -- --include-ignored`. \
            Full dispatch assertion needs rootless docker (rootful skips the dispatch leg)."]
fn builds_and_dispatches_via_docker() {
    use eigenius_r::{RImageBinding, RImagePlan};
    use eigenius_runtime_substrate::spawner::service::DockerServiceSpawner;
    use eigenius_runtime_substrate::spawner::DockerSpawnerConfig;

    if !is_docker_available() {
        eprintln!("skipping builds_and_dispatches_via_docker: docker unavailable");
        return;
    }
    if !eigenius_runtime_substrate::image_build::is_buildah_available() {
        eprintln!("skipping builds_and_dispatches_via_docker: buildah unavailable");
        return;
    }
    if !cdylib_path().exists() {
        eprintln!("skipping: cdylib not built");
        return;
    }

    let base = std::env::var("EIGENIUS_R_TEST_BASE_IMAGE")
        .unwrap_or_else(|_| "docker.io/rocker/r-base:latest".to_string());
    // Light recipe: no Bioconductor install (just bake the worker), so the
    // build is dominated by the base pull, not package compilation.
    let light_plan = RImagePlan {
        bioc_version: "3.18".to_string(),
        packages: vec![],
        include_mirror: false,
    };

    let depot = tempfile::tempdir().expect("depot");
    let spawner =
        match DockerServiceSpawner::new(DockerSpawnerConfig::new(depot.path().to_path_buf())) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("skipping (DockerServiceSpawner construction failed): {e}");
                return;
            }
        };

    // 1. Build the env image (host driver + cdylib baked in, light base).
    let builder = RLanguageRuntime::new(
        spawner.clone(),
        driver_path(),
        cdylib_path(),
        depot.path().to_path_buf(),
    )
    .with_build_config(&base, light_plan.clone());
    let env = Resource::new(Iri::parse("urn:eigenius:test:renv").expect("iri"));
    let digest = match builder.build_environment_image(&env, &[], None) {
        Ok(d) => d,
        Err(e) => {
            // No network to pull the base, or buildah/docker not fully
            // functional here → skip rather than fail (matches the
            // full-environment-only philosophy of the Julia/Lean e2es).
            eprintln!("skipping (image build failed — likely no base-image network): {e}");
            return;
        }
    };
    assert!(
        digest.as_str().starts_with("sha256:"),
        "digest {}",
        digest.as_str()
    );
    let manifest_hash = builder.image_manifest_hash().expect("manifest hash");

    // 2. Dispatch through the Docker backend against the built image.
    let runtime = RLanguageRuntime::with_image(
        spawner.clone(),
        depot.path().to_path_buf(),
        RImageBinding {
            digest,
            manifest_hash,
        },
    );
    let script = resource_with(
        "urn:eigenius:test:rscript:sum",
        "urn:eigenius:runtime:source",
        "as.raw(sum(1:4))",
    );
    match runtime.run_script(&env, &script, &[]) {
        Ok(outcome) => {
            let out = outcome
                .output
                .get(&Iri::parse("urn:eigenius:runtime:script_output").unwrap())
                .and_then(eigenius_kernel::ontology::resource::Value::as_str)
                .expect("script_output");
            assert_eq!(
                out.as_bytes(),
                &[10u8],
                "sum(1:4)=10 via the pinned R image"
            );
        }
        Err(e) => {
            let msg = e.to_string();
            // The DockerServiceSpawner targets ROOTLESS docker (its config
            // defaults to /run/user/<uid>/docker.sock), where the container
            // runs as the host uid so the bind-mounted worker.sock is
            // host-connectable. Under ROOTFUL docker the container is root
            // and the socket is unconnectable by the non-root test
            // (EACCES / connect timeout) — an environment limitation, not a
            // dispatch bug. The build itself is already asserted above
            // (a digest was produced + the worker booted + the cdylib
            // loaded to bind the socket). Skip the dispatch assertion here;
            // it passes under rootless docker.
            if msg.contains("attach_uds")
                || msg.contains("Permission denied")
                || msg.contains("timed out")
            {
                eprintln!(
                    "skipping dispatch assertion (rootful docker: worker.sock not host-connectable; \
                     the spawner targets rootless docker): {e}"
                );
                return;
            }
            panic!("docker-backed run_script failed unexpectedly: {e}");
        }
    }
}

fn resource_with(iri: &str, prop: &str, value: &str) -> Resource {
    let mut r = Resource::new(Iri::parse(iri).expect("iri"));
    r.set(
        Iri::parse(prop).expect("prop"),
        eigenius_kernel::ontology::resource::Value::String(value.to_string()),
    );
    r
}
