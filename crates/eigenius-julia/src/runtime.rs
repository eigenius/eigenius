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

//! `JuliaLanguageRuntime` — the production `LanguageRuntime` impl for
//! Julia. Phase 19a.1 ships the per-invocation Docker spawner path
//! (inherited from the 18d capstone fixture); 19a.2 introduces the
//! `ServiceSpawner` warm-pool path; 19a.3 lights up the mirror
//! generator and 19a.4 wires `CallRuntimeMethod` against typed mirror
//! struct dispatch.
//!
//! This module is intentionally thin Rust over the substrate's
//! existing image-build + spawn machinery. The Julia-specific work is
//! the worker (`JuliaWorker.jl` in `julia/runtime-worker/`) and, in
//! 19a.3, the generated mirror packages. From the substrate's view,
//! this crate just composes Dockerfile fragments and routes RPC.

use crate::conventions::{
    LANGUAGE, PROP_LANGUAGE, PROP_SCRIPT_OUTPUT, PROP_SOURCE, UDS_CONNECT_TIMEOUT,
    WORKER_PROJECT_DIR,
};
use crate::dockerfile::julia_dockerfile_fragments;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::cross_check::{prepare_substrate_side, ProvenanceDirAction};
use eigenius_runtime_substrate::error::{BuildError, RunError, SpawnError};
use eigenius_runtime_substrate::image_build::dockerfile::LanguageAssetCopy;
use eigenius_runtime_substrate::image_build::{
    compose_dockerfile, BuildContext, BuildContextSpec, BuildahImageBuilder, DockerfileSpec,
    ImageBuilder, LanguageAsset,
};
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::rpc::client::WorkerRpcClient;
use eigenius_runtime_substrate::rpc::protocol::{HealthInfo, Request, Response};
use eigenius_runtime_substrate::spawner::{DockerSpawner, WorkerSpawner};
use eigenius_runtime_substrate::types::{
    DockerfileFragments, ImageDigest, WorkerHandle, WorkerSpec,
};
use serde_bytes::ByteBuf;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `LanguageRuntime` impl that builds and runs the Julia worker
/// inside a Docker container. The substrate calls this through the
/// language registry whenever a `RuntimeScript` /
/// `RuntimeMethodSignature` resource declares `language = "julia"`.
///
/// 19a.1 path: per-invocation `DockerSpawner`, container removed on
/// exit. 19a.2 introduces `ServiceSpawner` for warm-pool dispatch.
pub struct JuliaLanguageRuntime {
    spawner: Arc<DockerSpawner>,
    /// Path to `julia/runtime-worker/` — the directory containing
    /// `Project.toml`, `Manifest.toml`, and `src/JuliaWorker.jl`.
    /// Resolved by the caller (typically via `env!("CARGO_MANIFEST_DIR")`
    /// against a workspace-relative path).
    project_dir: PathBuf,
    base_image_ref: String,
    image_tag: String,
    cached_digest: OnceLock<ImageDigest>,
    cached_manifest_hash: OnceLock<String>,
    cached_assets: OnceLock<JuliaAssets>,
    depot_path: PathBuf,
}

#[derive(Clone)]
struct JuliaAssets {
    project_toml: Vec<u8>,
    manifest_toml: Vec<u8>,
    worker_jl: Vec<u8>,
}

impl JuliaLanguageRuntime {
    /// Construct with paths to the Julia project directory, the
    /// digest-pinned Julia base image (e.g. `julia:1.12-bookworm` or
    /// `docker.io/library/julia@sha256:...`), the substrate's
    /// `DockerSpawner`, and the depot path the spawner was configured
    /// with.
    pub fn new(
        project_dir: PathBuf,
        base_image_ref: impl Into<String>,
        spawner: Arc<DockerSpawner>,
        depot_path: PathBuf,
    ) -> Self {
        let base = base_image_ref.into();
        let safe_prefix: String = base
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(24)
            .collect();
        let image_tag = format!("eigenius-julia-{safe_prefix}:latest");
        Self {
            spawner,
            project_dir,
            base_image_ref: base,
            image_tag,
            cached_digest: OnceLock::new(),
            cached_manifest_hash: OnceLock::new(),
            cached_assets: OnceLock::new(),
            depot_path,
        }
    }

    fn assets(&self) -> Result<&JuliaAssets, BuildError> {
        if let Some(a) = self.cached_assets.get() {
            return Ok(a);
        }
        let project_toml = read_or_fail(&self.project_dir.join("Project.toml"))?;
        let manifest_toml = read_or_fail(&self.project_dir.join("Manifest.toml"))?;
        let worker_jl = read_or_fail(&self.project_dir.join("src/JuliaWorker.jl"))?;
        let _ = self.cached_assets.set(JuliaAssets {
            project_toml,
            manifest_toml,
            worker_jl,
        });
        Ok(self.cached_assets.get().expect("just set"))
    }

    fn manifest_hash(&self) -> Result<&str, BuildError> {
        if let Some(h) = self.cached_manifest_hash.get() {
            return Ok(h);
        }
        let assets = self.assets()?;
        // Hash all three project files as a single byte stream so any
        // edit to any of them produces a different manifest hash.
        // Order is fixed by code (not the filesystem), which is what
        // determinism wants.
        let mut hasher = Sha256::new();
        hasher.update(&assets.project_toml);
        hasher.update(&assets.manifest_toml);
        hasher.update(&assets.worker_jl);
        let hash = format!("sha256:{:x}", hasher.finalize());
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

    fn build_image(&self) -> Result<ImageDigest, BuildError> {
        let manifest_hash = self.manifest_hash()?.to_string();
        let assets = self.assets()?.clone();

        let fragments = julia_dockerfile_fragments();
        let asset_copies = vec![
            LanguageAssetCopy {
                source: PathBuf::from("Project.toml"),
                destination: format!("{WORKER_PROJECT_DIR}/Project.toml"),
            },
            LanguageAssetCopy {
                source: PathBuf::from("Manifest.toml"),
                destination: format!("{WORKER_PROJECT_DIR}/Manifest.toml"),
            },
            LanguageAssetCopy {
                source: PathBuf::from("src/JuliaWorker.jl"),
                destination: format!("{WORKER_PROJECT_DIR}/src/JuliaWorker.jl"),
            },
        ];
        let dockerfile = compose_dockerfile(&DockerfileSpec {
            base_image_ref: &self.base_image_ref,
            fragments: &fragments,
            included_packages: &[],
            has_mirror: false,
            language_asset_copies: &asset_copies,
        });

        let work_dir = self.depot_path.join("build-context-julia");
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
            built_at: format!("manifest:{manifest_hash}"),
            packages: BTreeMap::new(),
            mirror: None,
            language_assets: vec![
                LanguageAsset {
                    source: PathBuf::from("Project.toml"),
                    content: assets.project_toml,
                    mode: None,
                },
                LanguageAsset {
                    source: PathBuf::from("Manifest.toml"),
                    content: assets.manifest_toml,
                    mode: None,
                },
                LanguageAsset {
                    source: PathBuf::from("src/JuliaWorker.jl"),
                    content: assets.worker_jl,
                    mode: None,
                },
            ],
        };
        let context = BuildContext::materialize(work_dir, &spec)?;
        let _ = BuildahImageBuilder::new().build(&context, &self.image_tag)?;
        push_to_docker_daemon(&self.image_tag)?;
        resolve_docker_image_id(&self.image_tag)
    }
}

impl LanguageRuntime for JuliaLanguageRuntime {
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

    fn spawn_worker(
        &self,
        _env: &Resource,
        _image_digest: Option<&ImageDigest>,
    ) -> Result<WorkerHandle, SpawnError> {
        let digest = self.ensure_image().map_err(|e| SpawnError::SpawnFailed {
            backend: "docker",
            reason: format!("eigenius-julia build_image failed: {e}"),
        })?;
        let manifest_hash = self
            .manifest_hash()
            .map_err(|e| SpawnError::SpawnFailed {
                backend: "docker",
                reason: format!("eigenius-julia manifest_hash failed: {e}"),
            })?
            .to_string();

        let n = INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tempdir = self
            .depot_path
            .join(format!("inv-julia-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&tempdir).map_err(|e| SpawnError::SpawnFailed {
            backend: "docker",
            reason: format!("create tempdir {} failed: {e}", tempdir.display()),
        })?;

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
            command: Vec::new(), // image's CMD = bootstrap_command
            tempdir_host_path: tempdir,
            depot_host_path: Some(self.depot_path.clone()),
            env,
            // Julia cold-start + dispatch should be well under a
            // minute even for the precompile-uncached first run; the
            // wall-clock cap is enforced by the dispatcher in
            // production.
            max_wall_time_ms: 0,
            max_memory_bytes: 0,
            seccomp_profile: None,
        };
        self.spawner.spawn(spec)
    }

    fn run_script(
        &self,
        worker: &WorkerHandle,
        script: &Resource,
        _inputs: &[Resource],
    ) -> Result<Resource, RunError> {
        let source = read_string_property(script, PROP_SOURCE).map_err(|reason| {
            RunError::MethodSignatureMismatch(format!(
                "RuntimeScript missing or malformed `source`: {reason}"
            ))
        })?;

        let stream = connect_with_retry(&worker.uds_path, UDS_CONNECT_TIMEOUT)
            .map_err(|e| RunError::WorkerRpcFailed(format!("connect to worker UDS: {e}")))?;
        let mut client = WorkerRpcClient::new(stream);

        let mut target_cbor = Vec::new();
        ciborium::into_writer(source, &mut target_cbor)
            .map_err(|e| RunError::WorkerRpcFailed(format!("encode julia source as CBOR: {e}")))?;

        let invocation_id = format!(
            "julia-inv-{}",
            INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id: invocation_id.clone(),
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

        Ok(build_output_resource(&invocation_id, stdout))
    }

    fn call_method(
        &self,
        _worker: &WorkerHandle,
        _signature: &Resource,
        _inputs: &[Resource],
    ) -> Result<Resource, RunError> {
        // 19a.4 lights this up against typed mirror struct dispatch
        // (worker-side method registry walking the generated mirror
        // packages' exports). Until then, dispatch a script via
        // `RunRuntimeScript` instead.
        Err(RunError::MethodSignatureMismatch(
            "JuliaLanguageRuntime::call_method is not yet implemented (lands in Phase 19a.4)"
                .to_string(),
        ))
    }

    fn dockerfile_fragments(&self, _env: &Resource) -> DockerfileFragments {
        julia_dockerfile_fragments()
    }

    fn query_health(&self, worker: &WorkerHandle) -> Result<HealthInfo, RunError> {
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
}

fn read_or_fail(p: &Path) -> Result<Vec<u8>, BuildError> {
    std::fs::read(p).map_err(|e| {
        BuildError::BuildInputUnavailable(format!(
            "could not read Julia project file {}: {e}",
            p.display()
        ))
    })
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
                std::thread::sleep(Duration::from_millis(100));
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

fn build_output_resource(invocation_id: &str, output: String) -> Resource {
    let iri = Iri::parse(&format!("urn:eigenius:julia:invocation:{invocation_id}:output"))
        .expect("invocation IRI is well-formed by construction");
    let mut r = Resource::new(iri);
    r.set(
        Iri::parse(PROP_SCRIPT_OUTPUT).expect("static IRI is well-formed"),
        Value::String(output),
    );
    r.set(
        Iri::parse(PROP_LANGUAGE).expect("static IRI is well-formed"),
        Value::String(LANGUAGE.to_string()),
    );
    r
}

/// Hand the substrate-built image off to Docker via tar archive
/// (matches the pattern in `runtime-substrate`'s test fixtures —
/// keeps cross-buildah/cross-Docker-version interop irrelevant).
fn push_to_docker_daemon(image_tag: &str) -> Result<(), BuildError> {
    // Per-call nonce so parallel test invocations in the same cargo
    // test process don't race on the same archive path.
    static ARCHIVE_NONCE: AtomicU64 = AtomicU64::new(0);
    let nonce = ARCHIVE_NONCE.fetch_add(1, Ordering::SeqCst);
    let archive_path = std::env::temp_dir().join(format!(
        "eigenius-julia-image-{}-{}-{}.tar",
        std::process::id(),
        sanitise_for_path(image_tag),
        nonce,
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

fn sanitise_for_path(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
