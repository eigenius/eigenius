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

//! [`OciToolRuntime`] — the generic OCI tool runtime (D60).
//!
//! Runs a pinned **Eigenius worker binary** baked into a container image as a
//! one-shot Job (`lifecycle:Job`): the substrate spawns it, provisions inputs by
//! `content_hash`, and dispatches `DispatchMethod` over the UDS; the worker
//! returns its result as Eigon-CBOR (`DispatchOk.output`). The kernel applies the
//! `ProgramTrace` / `IsDerivedAs` witness on top. This is the same mechanism the R
//! runtime uses, minus R's FFI mirror — the worker is plain Rust linking the
//! kernel, so any pure transform (the schema.org generator is the first) plugs in
//! as a worker binary.
//!
//! Build half mirrors `TestLanguageRuntimeDocker` (bake one binary via buildah);
//! dispatch half mirrors `eigenius-r`'s `run_script` (Eigon-CBOR inputs/outputs).
//! The runtime is spawner-agnostic (`Arc<dyn WorkerSpawner>`) so the napi layer
//! wires the concrete `DockerSpawner`.

pub mod recipe;

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::cross_check::{prepare_substrate_side, ProvenanceDirAction};
use eigenius_runtime_substrate::error::{BuildError, RunError, SpawnError};
use eigenius_runtime_substrate::image_build::dockerfile::LanguageAssetCopy;
use eigenius_runtime_substrate::image_build::{
    compose_dockerfile, BuildContext, BuildContextSpec, BuildahImageBuilder, DockerfileSpec,
    ImageBuilder, LanguageAsset,
};
use eigenius_runtime_substrate::invocation::RunOutcome;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::rpc::protocol::{HealthInfo, Request, Response, TargetKind};
use eigenius_runtime_substrate::rpc::WorkerRpcClient;
use eigenius_runtime_substrate::spawner::WorkerSpawner;
use eigenius_runtime_substrate::types::{
    DockerfileFragments, ImageDigest, WorkerHandle, WorkerSpec,
};
use serde_bytes::ByteBuf;
use sha2::{Digest, Sha256};

/// `language` id for the generic OCI tool runtime.
pub const LANGUAGE: &str = "oci";
/// In-image path the worker binary is baked to (also the image CMD).
const WORKER_BINARY_DEST: &str = "/usr/local/bin/eigenius-oci-worker";
/// Name of the worker asset inside the build context (`language/`).
const WORKER_ASSET: &str = "eigenius-oci-worker";
/// Env var the worker reads to find its UDS path (matches the worker bin).
const UDS_ENV: &str = "EIGENIUS_WORKER_UDS";

const UDS_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const REAP_TIMEOUT: Duration = Duration::from_secs(10);
static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The generic OCI tool runtime. Bakes a pinned worker binary into an image and
/// dispatches one-shot conversions to it.
pub struct OciToolRuntime {
    spawner: Arc<dyn WorkerSpawner>,
    worker_binary_path: PathBuf,
    base_image_ref: String,
    image_tag: String,
    depot_path: PathBuf,
    cached_digest: OnceLock<ImageDigest>,
    cached_manifest_hash: OnceLock<String>,
    cached_binary_bytes: OnceLock<Vec<u8>>,
}

impl OciToolRuntime {
    /// Construct over a host-built worker binary, a digest-pinned base image, a
    /// spawner, and the substrate depot (same path the spawner was configured
    /// with — per-invocation tempdirs live under it, D26 §9.5).
    pub fn new(
        worker_binary: PathBuf,
        base_image_ref: impl Into<String>,
        spawner: Arc<dyn WorkerSpawner>,
        depot_path: PathBuf,
    ) -> Self {
        let base = base_image_ref.into();
        let safe_prefix: String = base
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(24)
            .collect();
        let image_tag = format!("eigenius-oci-{safe_prefix}:latest");
        Self {
            spawner,
            worker_binary_path: worker_binary,
            base_image_ref: base,
            image_tag,
            depot_path,
            cached_digest: OnceLock::new(),
            cached_manifest_hash: OnceLock::new(),
            cached_binary_bytes: OnceLock::new(),
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
        let hash = format!("sha256:{:x}", Sha256::digest(self.binary_bytes()?));
        let _ = self.cached_manifest_hash.set(hash);
        Ok(self.cached_manifest_hash.get().expect("just set"))
    }

    fn ensure_image(&self) -> Result<ImageDigest, BuildError> {
        if let Some(d) = self.cached_digest.get() {
            return Ok(d.clone());
        }
        let digest = self.build_image()?;
        let _ = self.cached_digest.set(digest.clone());
        Ok(digest)
    }

    fn dockerfile_fragments_inner(&self) -> DockerfileFragments {
        DockerfileFragments {
            install_runtime: vec![],
            install_packages: vec![],
            install_mirror: vec![],
            bootstrap_command: vec![WORKER_BINARY_DEST.to_string()],
        }
    }

    /// The composed Dockerfile this runtime builds — shared by `build_image`
    /// (what runs) and `build_recipe` (what gets recorded), so they cannot drift.
    fn composed_dockerfile(&self) -> String {
        let fragments = self.dockerfile_fragments_inner();
        let asset_copies = vec![LanguageAssetCopy {
            source: PathBuf::from(WORKER_ASSET),
            destination: WORKER_BINARY_DEST.to_string(),
        }];
        compose_dockerfile(&DockerfileSpec {
            base_image_ref: &self.base_image_ref,
            fragments: &fragments,
            included_packages: &[],
            has_mirror: false,
            language_asset_copies: &asset_copies,
        })
    }

    /// Build the kernel-tracked [`recipe::BuildRecipe`] for this image (D60 §4.2):
    /// the chain-resident record of how the image is built. `build_command` is the
    /// exact `eigenius env build …` argv that produced it. Pure (no Docker) — it
    /// records the build's inputs, which `eigenius env build --verify` later
    /// replays to reproduce the digest.
    pub fn build_recipe(&self, build_command: Vec<String>) -> Result<Resource, BuildError> {
        let manifest_hash = self.manifest_hash()?.to_string();
        let dockerfile = self.composed_dockerfile();
        let artifact_hashes = vec![format!("{WORKER_ASSET}:{manifest_hash}")];
        Ok(recipe::build_recipe_resource(&recipe::RecipeInputs {
            base_image: &self.base_image_ref,
            dockerfile: &dockerfile,
            build_command: &build_command.join(" "),
            builder: "buildah",
            builder_version: &buildah_version(),
            artifact_hashes: &artifact_hashes,
        }))
    }

    fn build_image(&self) -> Result<ImageDigest, BuildError> {
        let manifest_hash = self.manifest_hash()?.to_string();
        let binary_bytes = self.binary_bytes()?.to_vec();

        let dockerfile = self.composed_dockerfile();

        let work_dir = self.depot_path.join("build-context-oci");
        let _ = std::fs::remove_dir_all(&work_dir);
        std::fs::create_dir_all(&work_dir).map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!(
                "create build context {}: {e}",
                work_dir.display()
            ))
        })?;

        let spec = BuildContextSpec {
            dockerfile,
            manifest_hash: manifest_hash.clone(),
            mirror_iri: String::new(),
            included_pkg_iris: Vec::new(),
            built_at: format!("manifest:{manifest_hash}"),
            packages: BTreeMap::new(),
            mirror: None,
            language_assets: vec![LanguageAsset {
                source: PathBuf::from(WORKER_ASSET),
                content: binary_bytes,
                mode: Some(0o755),
            }],
        };
        let context = BuildContext::materialize(work_dir, &spec)?;
        let _ = BuildahImageBuilder::new().build(&context, &self.image_tag)?;
        push_to_docker_daemon(&self.image_tag)?;
        resolve_docker_image_id(&self.image_tag)
    }

    fn spawn_internal(&self, env: &Resource) -> Result<WorkerHandle, SpawnError> {
        // Production: the RuntimeEnvironment carries the pre-built image_digest
        // (from `eigenius env build` host-side) — spawn it, never build at dispatch
        // (the orchestrator has no buildah; the R runtime resolves the env digest
        // the same way). Dev/e2e: no digest on the env → build the image here.
        let digest = match env
            .get(&Iri::parse("urn:eigenius:runtime:image_digest").expect("static IRI"))
            .and_then(Value::as_str)
        {
            Some(s) => ImageDigest::parse(s).map_err(|e| SpawnError::SpawnFailed {
                backend: "docker",
                reason: format!("env carries malformed image_digest `{s}`: {e}"),
            })?,
            None => self.ensure_image().map_err(|e| SpawnError::SpawnFailed {
                backend: "docker",
                reason: format!("oci-tool build_image failed: {e}"),
            })?,
        };
        let manifest_hash = self
            .manifest_hash()
            .map_err(|e| SpawnError::SpawnFailed {
                backend: "docker",
                reason: format!("oci-tool manifest_hash failed: {e}"),
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

        // The manifest-hash file is baked into the image; AssumeBaked populates
        // the env vars the worker reads without a host-side write (D26 §9.3).
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
            UDS_ENV.to_string(),
            tempdir.join("worker.sock").to_string_lossy().into_owned(),
        );
        env.extend(cross_check_env);

        let spec = WorkerSpec {
            image_digest: Some(digest),
            command: Vec::new(), // image CMD = WORKER_BINARY_DEST
            tempdir_host_path: tempdir,
            depot_host_path: Some(self.depot_path.clone()),
            env,
            max_wall_time_ms: 0,
            max_memory_bytes: 0,
            seccomp_profile: None,
        };
        self.spawner.spawn(spec)
    }

    fn capture_health(&self, worker: &WorkerHandle) -> Option<ImageDigest> {
        match self.query_health(worker) {
            Ok(info) => info
                .env_digest_in_image
                .as_deref()
                .and_then(|s| ImageDigest::parse(s).ok()),
            Err(e) => {
                eprintln!(
                    "OciToolRuntime: health query failed for worker {} ({e}); \
                     continuing with empty trace fields",
                    worker.id
                );
                None
            }
        }
    }

    fn query_health(&self, worker: &WorkerHandle) -> Result<HealthInfo, RunError> {
        let stream = connect_with_retry(&worker.uds_path, UDS_CONNECT_TIMEOUT)
            .map_err(|e| RunError::WorkerRpcFailed(format!("connect for health: {e}")))?;
        let mut client = WorkerRpcClient::new(stream);
        match client
            .call(&Request::Health)
            .map_err(|e| RunError::WorkerRpcFailed(format!("health call: {e}")))?
        {
            Response::Health(info) => Ok(info),
            other => Err(RunError::WorkerRpcFailed(format!(
                "unexpected response to health: {other:?}"
            ))),
        }
    }

    fn dispatch_and_evict(
        &self,
        worker: &WorkerHandle,
        inputs: Vec<ByteBuf>,
        invocation_id: String,
    ) -> Result<Resource, RunError> {
        let stream = connect_with_retry(&worker.uds_path, UDS_CONNECT_TIMEOUT)
            .map_err(|e| RunError::WorkerRpcFailed(format!("connect for dispatch: {e}")))?;
        let mut client = WorkerRpcClient::new(stream);

        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id,
                target_kind: TargetKind::Script,
                // The oci worker is driven by `inputs`; the target is unused.
                target: ByteBuf::new(),
                inputs,
            })
            .map_err(|e| RunError::WorkerRpcFailed(format!("dispatch_method call: {e}")))?;

        let output = match resp {
            Response::DispatchOk { output, .. } => eigon_cbor::parse_resource_lenient(&output)
                .map_err(|e| {
                    RunError::WorkerRpcFailed(format!(
                        "decode worker output as Eigon resource: {e}"
                    ))
                })?,
            Response::DispatchFailed {
                error_kind,
                message,
                ..
            } => return Err(map_dispatch_failure(&error_kind, message)),
            other => {
                return Err(RunError::WorkerRpcFailed(format!(
                    "unexpected response to dispatch_method: {other:?}"
                )))
            }
        };

        let evict = client
            .call(&Request::Evict)
            .map_err(|e| RunError::WorkerRpcFailed(format!("evict call: {e}")))?;
        if !matches!(evict, Response::Evicted) {
            return Err(RunError::WorkerRpcFailed(format!(
                "unexpected response to evict: {evict:?}"
            )));
        }
        Ok(output)
    }
}

impl LanguageRuntime for OciToolRuntime {
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
        env: &Resource,
        _script: &Resource,
        inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        let input_payloads: Vec<ByteBuf> = inputs
            .iter()
            .map(|r| ByteBuf::from(eigon_cbor::serialize_resource(r)))
            .collect();
        let invocation_id = format!(
            "oci-inv-{}",
            INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let started_at = now_rfc3339();

        let worker = self
            .spawn_internal(env)
            .map_err(|e| RunError::WorkerRpcFailed(format!("spawn_worker: {e}")))?;
        let image_digest = self.capture_health(&worker);

        let dispatch = self.dispatch_and_evict(&worker, input_payloads, invocation_id);
        let result = match dispatch {
            Ok(r) => r,
            Err(e) => {
                let _ = self.spawner.kill(&worker);
                return Err(e);
            }
        };
        // Worker exits on Evict; reap bookkeeping (auto_remove handles the daemon).
        let _ = self.spawner.wait_with_timeout(&worker, Some(REAP_TIMEOUT));

        Ok(RunOutcome {
            output: result,
            derivations: Vec::new(),
            image_digest,
            started_at,
            completed_at: now_rfc3339(),
            numerical_metadata: Default::default(),
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
            "OciToolRuntime runs one-shot tools via run_script; call_method is not supported"
                .to_string(),
        ))
    }
}

fn now_rfc3339() -> String {
    eigenius_runtime_substrate::invocation::DispatchTrace::now_rfc3339()
}

fn connect_with_retry(uds_path: &Path, timeout: Duration) -> std::io::Result<UnixStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(uds_path) {
            Ok(s) => return Ok(s),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
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

/// Hand the buildah-built image to the local Docker daemon (per-crate copy of
/// the helper the R/Julia runtimes carry; not shared in the substrate).
fn push_to_docker_daemon(image_tag: &str) -> Result<(), BuildError> {
    let archive_path = std::env::temp_dir().join(format!(
        "eigenius-oci-image-{}-{}.tar",
        std::process::id(),
        sanitise_for_path(image_tag),
    ));
    let _ = std::fs::remove_file(&archive_path);

    let push = std::process::Command::new("buildah")
        .arg("push")
        .arg(image_tag)
        .arg(format!(
            "docker-archive:{}:{image_tag}",
            archive_path.display()
        ))
        .output()
        .map_err(|e| BuildError::EnvironmentBuildFailed(format!("invoke `buildah push`: {e}")))?;
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
        .map_err(|e| BuildError::EnvironmentBuildFailed(format!("invoke `docker load`: {e}")))?;
    let _ = std::fs::remove_file(&archive_path);
    if !load.status.success() {
        return Err(BuildError::EnvironmentBuildFailed(format!(
            "docker load failed: {}",
            String::from_utf8_lossy(&load.stderr)
        )));
    }
    Ok(())
}

fn resolve_docker_image_id(image_tag: &str) -> Result<ImageDigest, BuildError> {
    let output = std::process::Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", image_tag])
        .output()
        .map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!("invoke `docker image inspect`: {e}"))
        })?;
    if !output.status.success() {
        return Err(BuildError::EnvironmentBuildFailed(format!(
            "docker image inspect failed for `{image_tag}`: {}",
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

/// Best-effort `buildah` version string for the recipe's provenance. Falls back
/// to `"buildah"` when the version can't be read (the recipe stays valid).
fn buildah_version() -> String {
    std::process::Command::new("buildah")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "buildah".to_string())
}

fn sanitise_for_path(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
