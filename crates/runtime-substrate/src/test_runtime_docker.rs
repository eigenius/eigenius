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

//! `TestLanguageRuntimeDocker` — **DockerSpawner-only test fixture**
//! that ties Phase 18c.1–18c.5 together: builds a real OCI image
//! containing the `eigenius-test-worker` binary, spawns it via
//! [`crate::spawner::DockerSpawner`], and runs the same bash-c
//! protocol as [`crate::test_runtime::TestLanguageRuntime`]. This is
//! the bash analogue of Phase 18d's Julia capstone.
//!
//! Feature-gated behind both `test-runtime` and `docker-spawner` —
//! pulls in `buildah` (build path) and `bollard` (run path).
//!
//! ## What this validates end-to-end
//!
//! - **18c.1** image-build pipeline composes a real Dockerfile and
//!   invokes `buildah` against a glibc-compatible base (the recommended
//!   `debian:bookworm-slim`; alpine doesn't work because the cargo-built
//!   worker is dynamically linked against glibc).
//! - **18c.2** cross-check: manifest-hash baked into the image at
//!   `/etc/eigenius-runtime-env/manifest-hash` matches the env var
//!   the substrate sets at spawn time.
//! - **18c.3** `DockerSpawner` spawns against the substrate-built
//!   image with `auto_remove`, network isolation, the DooD bind-mount
//!   discipline, and `no-new-privileges:true`. No `cap_drop` per D26
//!   §1.2 — substrate is provenance + dispatch, not adversarial
//!   containment.
//! - **18c.4** `wait_with_timeout` reaps the container.
//! - **18c.5** [`crate::invocation::DispatchTrace`] carries the
//!   substrate-built image digest and worker-reported
//!   `numerical_metadata`.
//!
//! ## Manifest-hash anchor
//!
//! SHA-256 of the worker binary bytes. Both sides of the cross-check
//! derive from the same input — the build pipeline writes this hash
//! into `manifest-hash`; `spawn_worker` sets the same hash on the
//! `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH` env var. Same binary in →
//! same hash → cross-check passes deterministically.

use crate::cross_check::{prepare_substrate_side, ProvenanceDirAction};
use crate::error::{BuildError, RunError, SpawnError};
use crate::image_build::dockerfile::LanguageAssetCopy;
use crate::image_build::{
    compose_dockerfile, BuildContext, BuildContextSpec, BuildahImageBuilder, DockerfileSpec,
    ImageBuilder, LanguageAsset,
};
use crate::invocation::{DispatchTrace, RunOutcome};
use crate::language_runtime::LanguageRuntime;
use crate::rpc::client::WorkerRpcClient;
use crate::rpc::protocol::{HealthInfo, Request, Response, TargetKind};
use crate::spawner::{DockerSpawner, WorkerSpawner};
use crate::types::{DockerfileFragments, ImageDigest, WorkerHandle, WorkerSpec};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use serde_bytes::ByteBuf;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const LANGUAGE: &str = "test";
const PROP_SOURCE: &str = "urn:eigenius:runtime:source";
const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";
const PROP_TEST_BASH_STDOUT: &str = "urn:eigenius:test:bash_stdout";
const UDS_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const WORKER_BINARY_DEST: &str = "/usr/local/bin/eigenius-test-worker";

static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `LanguageRuntime` impl that builds and runs the bash test worker
/// inside a Docker container. See module docs for what it validates.
///
/// Construction is cheap (no I/O); the OCI image is built lazily on
/// the first `spawn_worker` call (and cached thereafter) so repeated
/// invocations against the same runtime instance share the build.
pub struct TestLanguageRuntimeDocker {
    spawner: Arc<DockerSpawner>,
    worker_binary_path: PathBuf,
    base_image_ref: String,
    image_tag: String,
    cached_digest: OnceLock<ImageDigest>,
    /// Memoised hash of the worker binary bytes — used both as the
    /// cross-check anchor and (indirectly) as part of the image tag,
    /// so different binaries produce different images.
    cached_manifest_hash: OnceLock<String>,
    cached_binary_bytes: OnceLock<Vec<u8>>,
    /// Single per-process build directory under the depot. The
    /// substrate places per-invocation tempdirs alongside it, all under
    /// the same depot path so the DooD bind-mount discipline (D26 §9.5)
    /// is satisfied without translation.
    depot_path: PathBuf,
}

impl TestLanguageRuntimeDocker {
    /// Construct with an explicit worker binary path and a digest-pinned
    /// base image reference (e.g. `"alpine@sha256:<...>"`). The depot
    /// path is the substrate's well-known host directory under which all
    /// per-invocation tempdirs and the build context are materialised
    /// (D26 §9.5); it must be the same path the supplied
    /// `DockerSpawner` was configured with.
    pub fn new(
        worker_binary: PathBuf,
        base_image_ref: impl Into<String>,
        spawner: Arc<DockerSpawner>,
        depot_path: PathBuf,
    ) -> Self {
        let base = base_image_ref.into();
        // Image tag carries a short prefix of the base ref so concurrent
        // test runs with different bases don't collide. The full hash
        // is pinned in the manifest itself.
        let safe_prefix: String = base
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(24)
            .collect();
        let image_tag = format!("eigenius-substrate-test-bash-{safe_prefix}:latest");
        Self {
            spawner,
            worker_binary_path: worker_binary,
            base_image_ref: base,
            image_tag,
            cached_digest: OnceLock::new(),
            cached_manifest_hash: OnceLock::new(),
            cached_binary_bytes: OnceLock::new(),
            depot_path,
        }
    }

    fn binary_bytes(&self) -> Result<&[u8], BuildError> {
        if let Some(b) = self.cached_binary_bytes.get() {
            return Ok(b);
        }
        let bytes = std::fs::read(&self.worker_binary_path).map_err(|e| {
            BuildError::BuildInputUnavailable(format!(
                "failed to read worker binary at {}: {e}",
                self.worker_binary_path.display()
            ))
        })?;
        let _ = self.cached_binary_bytes.set(bytes);
        Ok(self.cached_binary_bytes.get().expect("just set"))
    }

    fn manifest_hash(&self) -> Result<&str, BuildError> {
        if let Some(h) = self.cached_manifest_hash.get() {
            return Ok(h);
        }
        let bytes = self.binary_bytes()?;
        let hash = format!("sha256:{:x}", Sha256::digest(bytes));
        let _ = self.cached_manifest_hash.set(hash);
        Ok(self.cached_manifest_hash.get().expect("just set"))
    }

    /// Lazy build: invoke buildah on the first call, return the cached
    /// digest on subsequent calls. Made deterministic by the upstream
    /// 18c.1 pipeline — same inputs → same image id.
    fn ensure_image(&self) -> Result<ImageDigest, BuildError> {
        if let Some(d) = self.cached_digest.get() {
            return Ok(d.clone());
        }
        let digest = self.build_image()?;
        let _ = self.cached_digest.set(digest.clone());
        Ok(digest)
    }

    fn build_image(&self) -> Result<ImageDigest, BuildError> {
        let manifest_hash = self.manifest_hash()?.to_string();
        let binary_bytes = self.binary_bytes()?.to_vec();

        let fragments = self.dockerfile_fragments_inner();
        let asset_copies = vec![LanguageAssetCopy {
            source: PathBuf::from("eigenius-test-worker"),
            destination: WORKER_BINARY_DEST.to_string(),
        }];
        let dockerfile = compose_dockerfile(&DockerfileSpec {
            base_image_ref: &self.base_image_ref,
            fragments: &fragments,
            included_packages: &[],
            has_mirror: false,
            language_asset_copies: &asset_copies,
        });

        let work_dir = self.depot_path.join("build-context");
        let _ = std::fs::remove_dir_all(&work_dir);
        std::fs::create_dir_all(&work_dir).map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!(
                "failed to create build context directory {}: {e}",
                work_dir.display()
            ))
        })?;

        let spec = BuildContextSpec {
            dockerfile,
            manifest_hash: manifest_hash.clone(),
            mirror_iri: String::new(),
            included_pkg_iris: Vec::new(),
            // built_at is part of the deterministic input set in the
            // image config, so the value must be a function of inputs
            // only — not the wall clock. The manifest hash already
            // pins the input set; reusing it gives a stable, audit-
            // friendly stamp.
            built_at: format!("manifest:{manifest_hash}"),
            packages: BTreeMap::new(),
            mirror: None,
            language_assets: vec![LanguageAsset {
                source: PathBuf::from("eigenius-test-worker"),
                content: binary_bytes,
                mode: Some(0o755),
            }],
        };
        let context = BuildContext::materialize(work_dir, &spec)?;
        // 1. buildah builds into its own local image store. Returns
        //    buildah's image id, which is *not* what the run-side
        //    Docker daemon will see — they're separate stores.
        let _ = BuildahImageBuilder::new().build(&context, &self.image_tag)?;
        // 2. Hand the image off to the local Docker daemon's store via
        //    buildah's `docker-daemon:` transport. Production
        //    deployments would push to a registry instead and let
        //    DockerSpawner pull (D26 §9.2 step 5); the test fixture
        //    short-circuits that with a local-only handoff so no
        //    registry credentials are required.
        push_to_docker_daemon(&self.image_tag)?;
        // 3. Re-resolve the digest from Docker's perspective. Docker
        //    may re-encode the manifest on import (OCI ↔ Docker
        //    manifest format), producing a different image id than
        //    buildah's local id. The id Docker reports is the one
        //    `DockerSpawner::spawn` will look up, so that's the
        //    authoritative one for the substrate's `ImageDigest`.
        resolve_docker_image_id(&self.image_tag)
    }

    fn dockerfile_fragments_inner(&self) -> DockerfileFragments {
        DockerfileFragments {
            // Empty install_runtime: the recommended base
            // (`debian:bookworm-slim`) ships with bash + glibc
            // preinstalled, which is what the worker binary needs. The
            // alpine alternative was tried first but the cargo-built
            // worker is dynamically linked against glibc; on alpine
            // (musl) it fails at the dynamic-linker step. That trade-off
            // is real for any production language runtime that ships
            // glibc-linked binaries — choose the base image to match.
            install_runtime: vec![],
            install_packages: vec![],
            install_mirror: vec![],
            bootstrap_command: vec![WORKER_BINARY_DEST.to_string()],
        }
    }
}

impl LanguageRuntime for TestLanguageRuntimeDocker {
    fn language_id(&self) -> &str {
        LANGUAGE
    }

    fn build_environment_image(
        &self,
        _env: &Resource,
        _packages: &[Resource],
        _mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError> {
        self.ensure_image()
    }

    fn dockerfile_fragments(&self, _env: &Resource) -> DockerfileFragments {
        self.dockerfile_fragments_inner()
    }

    fn run_script(
        &self,
        _env: &Resource,
        script: &Resource,
        _inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        let source = read_string_property(script, PROP_SOURCE)
            .map_err(|reason| {
                RunError::MethodSignatureMismatch(format!(
                    "RuntimeScript missing or malformed `source`: {reason}"
                ))
            })?
            .to_string();

        let mut target_cbor = Vec::new();
        ciborium::into_writer(&source, &mut target_cbor)
            .map_err(|e| RunError::WorkerRpcFailed(format!("encode bash command as CBOR: {e}")))?;

        let invocation_id = format!(
            "test-docker-inv-{}",
            INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed)
        );

        let started_at = DispatchTrace::now_rfc3339();

        let worker = self
            .spawn_internal()
            .map_err(|e| RunError::WorkerRpcFailed(format!("spawn_worker: {e}")))?;
        let (numerical_metadata, image_digest) = self.capture_health(&worker);

        let dispatch_result = self.dispatch_and_evict(&worker, target_cbor, invocation_id.clone());
        let stdout = match dispatch_result {
            Ok(stdout) => stdout,
            Err(e) => {
                let _ = self.try_evict(&worker);
                return Err(e);
            }
        };

        let completed_at = DispatchTrace::now_rfc3339();

        Ok(RunOutcome {
            output: build_output_resource(&invocation_id, stdout),
            derivations: Vec::new(),
            image_digest,
            started_at,
            completed_at,
            numerical_metadata,
            dispatched_to: None,
        })
    }

    fn call_method(
        &self,
        _env: &Resource,
        _signature: &Resource,
        _inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        Err(RunError::MethodSignatureMismatch(
            "TestLanguageRuntimeDocker does not implement call_method (use run_script with a bash one-liner)"
                .to_string(),
        ))
    }
}

impl TestLanguageRuntimeDocker {
    fn spawn_internal(&self) -> Result<WorkerHandle, SpawnError> {
        let digest = self.ensure_image().map_err(|e| SpawnError::SpawnFailed {
            backend: "docker",
            reason: format!("test-runtime-docker build_image failed: {e}"),
        })?;
        let manifest_hash = self
            .manifest_hash()
            .map_err(|e| SpawnError::SpawnFailed {
                backend: "docker",
                reason: format!("test-runtime-docker manifest_hash failed: {e}"),
            })?
            .to_string();

        let n = INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tempdir = self
            .depot_path
            .join(format!("inv-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&tempdir).map_err(|e| SpawnError::SpawnFailed {
            backend: "docker",
            reason: format!("create tempdir {} failed: {e}", tempdir.display()),
        })?;

        // Image carries the manifest-hash file baked in at build time
        // — `AssumeBaked` skips host-side write but still populates the
        // env vars the worker reads on startup (D26 §9.3). The
        // `prov_dir` argument is unused on this branch but required by
        // the helper signature.
        let cross_check_env = prepare_substrate_side(
            &digest,
            &manifest_hash,
            &tempdir,
            ProvenanceDirAction::AssumeBaked,
        )
        .map_err(|e| SpawnError::SpawnFailed {
            backend: "docker",
            reason: format!("cross-check setup failed: {e}"),
        })?;

        let mut env = BTreeMap::new();
        env.insert(
            "EIGENIUS_TEST_WORKER_UDS".to_string(),
            tempdir.join("worker.sock").to_string_lossy().into_owned(),
        );
        env.extend(cross_check_env);

        let spec = WorkerSpec {
            image_digest: Some(digest),
            command: Vec::new(), // image's CMD = WORKER_BINARY_DEST
            tempdir_host_path: tempdir,
            depot_host_path: Some(self.depot_path.clone()),
            env,
            max_wall_time_ms: 0,
            max_memory_bytes: 0,
            seccomp_profile: None,
        };
        self.spawner.spawn(spec)
    }

    fn capture_health(
        &self,
        worker: &WorkerHandle,
    ) -> (
        crate::rpc::NumericalMetadata,
        Option<crate::types::ImageDigest>,
    ) {
        match self.query_health_internal(worker) {
            Ok(info) => {
                let digest = info
                    .env_digest_in_image
                    .as_deref()
                    .and_then(|s| crate::types::ImageDigest::parse(s).ok());
                (info.numerical_metadata, digest)
            }
            Err(e) => {
                eprintln!(
                    "TestLanguageRuntimeDocker: query_health failed for worker {} ({}): {e}; \
                     dispatch will continue with empty trace fields",
                    worker.id, worker.backend
                );
                (Default::default(), None)
            }
        }
    }

    fn query_health_internal(&self, worker: &WorkerHandle) -> Result<HealthInfo, RunError> {
        let stream = connect_with_retry(&worker.uds_path, UDS_CONNECT_TIMEOUT).map_err(|e| {
            RunError::WorkerRpcFailed(format!("connect to worker UDS for health: {e}"))
        })?;
        let mut client = WorkerRpcClient::new(stream);
        let resp = client
            .call(&Request::Health)
            .map_err(|e| RunError::WorkerRpcFailed(format!("health call: {e}")))?;
        drop(client);
        match resp {
            Response::Health(info) => Ok(info),
            other => Err(RunError::WorkerRpcFailed(format!(
                "unexpected response to health: {other:?}"
            ))),
        }
    }

    fn dispatch_and_evict(
        &self,
        worker: &WorkerHandle,
        target_cbor: Vec<u8>,
        invocation_id: String,
    ) -> Result<String, RunError> {
        let stream = connect_with_retry(&worker.uds_path, UDS_CONNECT_TIMEOUT)
            .map_err(|e| RunError::WorkerRpcFailed(format!("connect to worker UDS: {e}")))?;
        let mut client = WorkerRpcClient::new(stream);

        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id: invocation_id.clone(),
                target_kind: TargetKind::Script,
                target: ByteBuf::from(target_cbor),
                inputs: vec![],
            })
            .map_err(|e| RunError::WorkerRpcFailed(format!("dispatch_method call: {e}")))?;

        let stdout = match resp {
            Response::DispatchOk { output, .. } => ciborium::from_reader::<String, _>(&output[..])
                .map_err(|e| {
                    RunError::WorkerRpcFailed(format!("decode worker output as String: {e}"))
                })?,
            Response::DispatchFailed {
                error_kind,
                message,
                ..
            } => return Err(map_dispatch_failure(&error_kind, message)),
            other => {
                return Err(RunError::WorkerRpcFailed(format!(
                    "unexpected response to dispatch_method: {other:?}"
                )));
            }
        };

        let evict_resp = client
            .call(&Request::Evict)
            .map_err(|e| RunError::WorkerRpcFailed(format!("evict call: {e}")))?;
        if !matches!(evict_resp, Response::Evicted) {
            return Err(RunError::WorkerRpcFailed(format!(
                "unexpected response to evict: {evict_resp:?}"
            )));
        }
        drop(client);

        Ok(stdout)
    }

    fn try_evict(&self, worker: &WorkerHandle) -> Result<(), RunError> {
        let stream = std::os::unix::net::UnixStream::connect(&worker.uds_path)
            .map_err(|e| RunError::WorkerRpcFailed(format!("evict-on-error connect: {e}")))?;
        let mut client = WorkerRpcClient::new(stream);
        client
            .call(&Request::Evict)
            .map_err(|e| RunError::WorkerRpcFailed(format!("evict-on-error call: {e}")))?;
        Ok(())
    }
}

fn read_string_property<'a>(r: &'a Resource, prop_iri: &str) -> Result<&'a str, String> {
    let iri = Iri::parse(prop_iri).map_err(|e| format!("malformed property IRI: {e}"))?;
    r.get(&iri)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string property `{prop_iri}`"))
}

fn connect_with_retry(uds_path: &Path, timeout: Duration) -> std::io::Result<UnixStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(uds_path) {
            Ok(s) => return Ok(s),
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
}

fn map_dispatch_failure(error_kind: &str, message: String) -> RunError {
    match error_kind {
        "method_signature_mismatch" => RunError::MethodSignatureMismatch(message),
        "sandbox_violation" => RunError::SandboxViolation(message),
        _ => RunError::RuntimeError(message),
    }
}

fn build_output_resource(invocation_id: &str, stdout: String) -> Resource {
    let iri = Iri::parse(&format!(
        "urn:eigenius:test:invocation:{invocation_id}:output"
    ))
    .expect("test invocation IRI is well-formed by construction");
    let mut r = Resource::new(iri);
    r.set(
        Iri::parse(PROP_TEST_BASH_STDOUT).expect("static IRI is well-formed"),
        Value::String(stdout),
    );
    r.set(
        Iri::parse(PROP_LANGUAGE).expect("static IRI is well-formed"),
        Value::String(LANGUAGE.to_string()),
    );
    r
}

// Suppress "unused import" warning on platforms where DispatchTrace isn't
// referenced — the import is documentation-load-bearing and links the
// Phase 18c.5 trace contract to this fixture. Tests cover the runtime
// surface; the trace assembly happens upstream in the dispatcher.
#[allow(dead_code)]
fn _link_dispatch_trace(_: DispatchTrace) {}

/// Hand the substrate-built image off to the local Docker daemon's
/// image store. Uses the universally-compatible `docker-archive`
/// transport (buildah → tar → `docker load`) rather than
/// `docker-daemon:` because the latter requires the buildah build
/// matching the Docker daemon API version. With buildah 1.23 + Docker
/// 29 (the user's environment), the direct transport fails with a
/// "client version too old" diagnostic. The tar handoff bypasses the
/// API negotiation entirely.
///
/// Test-fixture only — production deployments push to a registry per
/// D26 §9.2 step 5.
fn push_to_docker_daemon(image_tag: &str) -> Result<(), BuildError> {
    // Per-call nonce so parallel test invocations in the same cargo
    // test process don't race on the same archive path.
    static ARCHIVE_NONCE: AtomicU64 = AtomicU64::new(0);
    let nonce = ARCHIVE_NONCE.fetch_add(1, Ordering::SeqCst);
    let archive_path = std::env::temp_dir().join(format!(
        "substrate-image-{}-{}-{}.tar",
        std::process::id(),
        sanitise_for_path(image_tag),
        nonce,
    ));
    // Defensive cleanup — a previous failed run could have left a
    // partial archive at the same path. `buildah push` does not
    // overwrite atomically; pre-removing avoids partial-data ambiguity.
    let _ = std::fs::remove_file(&archive_path);

    let push = std::process::Command::new("buildah")
        .arg("push")
        .arg(image_tag)
        .arg(format!(
            "docker-archive:{}:{image_tag}",
            archive_path.display()
        ))
        .output()
        .map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!("failed to invoke `buildah push`: {e}"))
        })?;
    if !push.status.success() {
        return Err(BuildError::EnvironmentBuildFailed(format!(
            "buildah push to docker-archive failed: {}",
            String::from_utf8_lossy(&push.stderr)
        )));
    }

    let load = std::process::Command::new("docker")
        .args(["load", "-i"])
        .arg(&archive_path)
        .output()
        .map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!("failed to invoke `docker load`: {e}"))
        })?;
    let _ = std::fs::remove_file(&archive_path);
    if !load.status.success() {
        return Err(BuildError::EnvironmentBuildFailed(format!(
            "docker load failed: {}",
            String::from_utf8_lossy(&load.stderr)
        )));
    }
    Ok(())
}

fn sanitise_for_path(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Read the image id Docker assigns to `image_tag` after the push, in
/// `sha256:<hex>` shape. Returns the parsed `ImageDigest` so it can be
/// stored in `WorkerSpec` as the spawn-time anchor.
fn resolve_docker_image_id(image_tag: &str) -> Result<ImageDigest, BuildError> {
    let output = std::process::Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", image_tag])
        .output()
        .map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!(
                "failed to invoke `docker image inspect`: {e}"
            ))
        })?;
    if !output.status.success() {
        return Err(BuildError::EnvironmentBuildFailed(format!(
            "docker image inspect failed for `{image_tag}` after push: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    ImageDigest::parse(id).map_err(|e| {
        BuildError::EnvironmentBuildFailed(format!(
            "docker reported an unparseable image id for `{image_tag}`: {e}"
        ))
    })
}

// Tests for TestLanguageRuntimeDocker live in
// tests/docker_e2e_integration.rs because they need the
// `CARGO_BIN_EXE_eigenius-test-worker` env var (only available to
// integration test crates) plus a real Docker daemon and buildah.
