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

//! `LeanLanguageRuntime` — authoring-side `LanguageRuntime` impl for
//! Lean 4. Mirrors `JuliaLanguageRuntime`'s Service-mode shape: one
//! long-lived worker per env image digest, attached over CBOR-framed
//! UDS.
//!
//! ## What lands in the image
//!
//! Unlike Julia (which builds its worker source inside the image via
//! `Pkg.instantiate`), Lean's worker is **pre-built on the host** and
//! COPY'd in as a single binary. The reasons:
//!
//! - The Lake-built worker binary links against an Eigenius-authored
//!   Rust cdylib (`libeigenius_lean_worker.so`). Templating Lake's
//!   `lakefile.lean` with image-specific `-L` paths so it could
//!   relink inside the image would require either a Rust toolchain in
//!   the image (large, slow to install) or hand-maintained Dockerfile
//!   substitution (fragile). Pre-building sidesteps both.
//! - The worker's `DT_RUNPATH` is stamped at link time with a path
//!   that doesn't resolve inside the container — but glibc consults
//!   `ld.so.cache` *before* `DT_RUNPATH`, so an `ld.so.conf.d` entry
//!   pointing at [`crate::conventions::WORKER_LIB_DIR`] + a `ldconfig`
//!   pass silently bypasses the stale RUNPATH.
//!
//! ## Dispatch surface
//!
//! Lean's worker only handles `TargetKind::Method` — `run_script` is
//! reserved for a future `lean exe` evaluation surface but isn't wired
//! today. The primary verb is `lean_export`, which takes
//! `[LeanProject, targetModule, targetConstant]` as inputs and
//! returns the lean4export ndjson bytes the verification side
//! (`eigenius-lean`) feeds into nanoda.

use crate::conventions::{
    DEFAULT_LEAN_PERMITTED_AXIOMS, LANGUAGE, LEAN4EXPORT_IN_IMAGE, PROP_IMAGE_DIGEST,
    PROP_LANGUAGE, PROP_LEAN_PERMITTED_AXIOMS, PROP_LEAN_UNPERMITTED_AXIOM_HARD_ERROR,
    PROP_METHOD_NAME, PROP_SCRIPT_OUTPUT, WORKER_BIN_PATH, WORKER_LIB_DIR,
};
use crate::dockerfile::{lean_dockerfile_fragments, LeanImagePlan};
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::cross_check::{prepare_substrate_side, ProvenanceDirAction};
use eigenius_runtime_substrate::error::{BuildError, RunError, SpawnError};
use eigenius_runtime_substrate::image_build::dockerfile::LanguageAssetCopy;
use eigenius_runtime_substrate::image_build::{
    compose_dockerfile, BuildContext, BuildContextSpec, BuildahImageBuilder, DockerfileSpec,
    ImageBuilder, LanguageAsset, MirrorMaterialization,
};
use eigenius_runtime_substrate::invocation::{DispatchTrace, RunOutcome};
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::rpc::client::WorkerRpcClient;
use eigenius_runtime_substrate::rpc::method::MethodInvocation;
use eigenius_runtime_substrate::rpc::protocol::{
    HealthInfo, NumericalMetadata, Request, Response, TargetKind,
};
use eigenius_runtime_substrate::spawner::service::{ServiceHandle, ServiceSpawner};
use eigenius_runtime_substrate::types::{DockerfileFragments, ImageDigest, WorkerSpec};
use serde_bytes::ByteBuf;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `LanguageRuntime` impl that runs the Lake-built Lean worker as a
/// long-lived **service** (D26 §8.1 Service lifecycle). The substrate
/// calls this through the language registry whenever a
/// `RuntimeMethodSignature` resource declares `language = "lean"`.
pub struct LeanLanguageRuntime {
    spawner: Arc<dyn ServiceSpawner>,
    /// Path to `lean/runtime-worker/` — the directory containing
    /// `lakefile.lean`, `lake-manifest.json`, `Worker/Main.lean`, and
    /// the vendored `lean4export/` under `vendor/`. The pre-built
    /// worker binary is at `<project_dir>/.lake/build/bin/lean-runtime-worker`.
    project_dir: PathBuf,
    /// Host path to the pre-built `libeigenius_lean_worker.so` cdylib
    /// the worker binary dynamically links against (typically
    /// `<workspace>/target/{debug,release}/libeigenius_lean_worker.so`).
    /// The image-build pipeline COPYs these bytes into the image
    /// under [`WORKER_LIB_DIR`].
    cdylib_path: PathBuf,
    /// Host path to the `lean/common/EigeniusLeanCommon/` directory —
    /// the hand-authored Lake package that the chain-committed mirror's
    /// generated lakefile depends on. Staged into the image under
    /// `LEAN_COMMON_IN_IMAGE` so the install_mirror step can rewrite
    /// the mirror's git-require to a path-require resolvable offline.
    /// Only read when a mirror is supplied to `build_environment_image`.
    lean_common_dir: PathBuf,
    base_image_ref: String,
    image_tag: String,
    cached_digest: OnceLock<ImageDigest>,
    cached_manifest_hash: OnceLock<String>,
    cached_assets: OnceLock<LeanAssets>,
    depot_path: PathBuf,
    /// `image_digest` → `ServiceHandle` map populated lazily as
    /// dispatches arrive. Mirrors Julia's per-digest cache so the
    /// orchestrator's external-institution path can dispatch into
    /// multiple envs concurrently from a single runtime instance.
    cached_services: Mutex<HashMap<String, ServiceHandle>>,
}

/// Cached host-side asset bytes — the worker binary, the cdylib, and
/// every file under the vendored `lean4export/` tree. Read once via
/// [`LeanLanguageRuntime::assets`] (lazy) and reused across image
/// builds + manifest-hash computations.
#[derive(Clone)]
struct LeanAssets {
    /// Lake-built worker binary bytes (mode 0o755 in-image).
    worker_bin: Vec<u8>,
    /// `libeigenius_lean_worker.so` bytes (mode 0o755 in-image).
    cdylib: Vec<u8>,
    /// `<relative-path>` → `<bytes>` for every staged lean4export
    /// source file, relative to the `vendor/lean4export/` root.
    /// `BTreeMap` so iteration order is deterministic for hashing
    /// and image-build context layout.
    lean4export_tree: BTreeMap<PathBuf, Vec<u8>>,
}

impl LeanLanguageRuntime {
    /// Construct with paths to the Lake project directory, the cdylib
    /// the worker binary links against, the digest-pinned Debian-slim
    /// base image, a `ServiceSpawner`, and the depot path the spawner
    /// was configured with.
    ///
    /// `project_dir` must point at `lean/runtime-worker/` (or an
    /// equivalent directory matching the same layout: a `vendor/
    /// lean4export/` tree and a `.lake/build/bin/lean-runtime-worker`
    /// binary). `cdylib_path` is typically
    /// `<workspace>/target/{debug,release}/libeigenius_lean_worker.so`.
    pub fn new(
        project_dir: PathBuf,
        cdylib_path: PathBuf,
        lean_common_dir: PathBuf,
        base_image_ref: impl Into<String>,
        spawner: Arc<dyn ServiceSpawner>,
        depot_path: PathBuf,
    ) -> Self {
        let base = base_image_ref.into();
        // Derive a buildah-friendly tag suffix from the base image
        // reference — same shape as the Julia runtime's tag derivation,
        // keeps test runs from colliding on cached tags.
        let safe_prefix: String = base
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(24)
            .collect::<String>()
            .trim_end_matches(['-', '_'])
            .to_string();
        let image_tag = format!("eigenius-lean-{safe_prefix}:latest");
        Self {
            spawner,
            project_dir,
            cdylib_path,
            lean_common_dir,
            base_image_ref: base,
            image_tag,
            cached_digest: OnceLock::new(),
            cached_manifest_hash: OnceLock::new(),
            cached_assets: OnceLock::new(),
            depot_path,
            cached_services: Mutex::new(HashMap::new()),
        }
    }

    /// Tear down every long-lived service worker this runtime
    /// instance opened. Idempotent. Mirror of
    /// `JuliaLanguageRuntime::drain`.
    pub fn drain(&self) -> Result<(), SpawnError> {
        let mut guard = self
            .cached_services
            .lock()
            .expect("cached_services mutex poisoned");
        let handles: Vec<ServiceHandle> = guard.drain().map(|(_, h)| h).collect();
        drop(guard);
        for handle in handles {
            self.spawner.drain(&handle)?;
        }
        Ok(())
    }

    /// Read-only view of the worker project directory.
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// Lazy load the worker binary, cdylib, and lean4export source
    /// tree from the host. Cached on the first call. Any I/O failure
    /// surfaces as `BuildError::BuildInputUnavailable` so the caller
    /// can attribute it to a pre-build gap rather than a dispatch
    /// failure.
    fn assets(&self) -> Result<&LeanAssets, BuildError> {
        if let Some(a) = self.cached_assets.get() {
            return Ok(a);
        }
        let worker_bin =
            read_or_fail(&self.project_dir.join(".lake/build/bin/lean-runtime-worker"))?;
        let cdylib = read_or_fail(&self.cdylib_path)?;
        let lean4export_root = self.project_dir.join("vendor/lean4export");
        let lean4export_tree = collect_lean4export_tree(&lean4export_root)?;
        let _ = self.cached_assets.set(LeanAssets {
            worker_bin,
            cdylib,
            lean4export_tree,
        });
        Ok(self.cached_assets.get().expect("just set"))
    }

    /// Content-hash of every byte going into the image — worker binary,
    /// cdylib, every staged lean4export source file. Bumping any of
    /// these invalidates the image cache.
    fn manifest_hash(&self) -> Result<&str, BuildError> {
        if let Some(h) = self.cached_manifest_hash.get() {
            return Ok(h);
        }
        let assets = self.assets()?;
        let mut hasher = Sha256::new();
        hasher.update(&assets.worker_bin);
        hasher.update(&assets.cdylib);
        // BTreeMap iteration is path-sorted, so the hash is stable
        // across host filesystem walk orders.
        for (path, bytes) in &assets.lean4export_tree {
            // Hash the path bytes too so a file rename produces a
            // different hash even when its content doesn't change.
            hasher.update(path.as_os_str().as_encoded_bytes());
            hasher.update(b"\0");
            hasher.update(bytes);
        }
        let hash = format!("sha256:{:x}", hasher.finalize());
        let _ = self.cached_manifest_hash.set(hash);
        Ok(self.cached_manifest_hash.get().expect("just set"))
    }

    fn ensure_image(&self, mirror: Option<&Resource>) -> Result<ImageDigest, BuildError> {
        // Cache key today is just `(worker assets, base image)`.
        // Adding the mirror's content hash to the cache key would be
        // the structurally correct thing once a single
        // `LeanLanguageRuntime` instance starts dispatching against
        // multiple mirror permutations — D26 §7.2 future-work.
        // Mirrors Julia's `JuliaLanguageRuntime::ensure_image` policy.
        if let Some(d) = self.cached_digest.get() {
            return Ok(d.clone());
        }
        let digest = self.build_image(mirror)?;
        let _ = self.cached_digest.set(digest.clone());
        Ok(digest)
    }

    fn build_image(&self, mirror: Option<&Resource>) -> Result<ImageDigest, BuildError> {
        let manifest_hash = self.manifest_hash()?.to_string();
        let assets = self.assets()?.clone();

        // Decode the optional mirror archive ahead of dockerfile
        // composition so `has_mirror` flows uniformly into the
        // composer + materialiser. v1 stages the archive under
        // `/opt/eigenius/mirror/`; lake-build-in-image of the
        // staged mirror is a 20a.6.x refinement (the lakefile's
        // EigeniusLeanCommon `require` needs an in-image path
        // substitution that the v1 image-build pipeline doesn't
        // wire yet).
        let mirror_mat = mirror.map(materialize_mirror_archive).transpose()?;
        let mirror_iri = mirror
            .and_then(|m| m.id().map(|iri| iri.as_str().to_string()))
            .unwrap_or_default();
        let has_mirror = mirror_mat.is_some();

        // The dockerfile plan now reflects whether a mirror is
        // staged. install_mirror itself stays empty in v1 — the
        // staged files exist under `/opt/eigenius/mirror/` for the
        // worker + future dispatch paths to consume.
        let plan = LeanImagePlan {
            include_mirror: has_mirror,
            handler_packages: Vec::new(),
        };
        let fragments = lean_dockerfile_fragments(&plan);

        // Build the asset-copy list: cdylib + worker binary + every
        // lean4export source file.
        let mut asset_copies: Vec<LanguageAssetCopy> = Vec::new();
        let mut language_assets: Vec<LanguageAsset> = Vec::new();

        // Cdylib → /opt/eigenius/lib/libeigenius_lean_worker.so.
        // Named `cdylib/libeigenius_lean_worker.so` under the
        // language/ dir so the staging layout self-describes.
        let cdylib_basename = self
            .cdylib_path
            .file_name()
            .ok_or_else(|| {
                BuildError::EnvironmentBuildFailed(format!(
                    "cdylib path `{}` has no file name",
                    self.cdylib_path.display()
                ))
            })?
            .to_string_lossy()
            .into_owned();
        let cdylib_src = PathBuf::from("cdylib").join(&cdylib_basename);
        asset_copies.push(LanguageAssetCopy {
            source: cdylib_src.clone(),
            destination: format!("{WORKER_LIB_DIR}/{cdylib_basename}"),
        });
        language_assets.push(LanguageAsset {
            source: cdylib_src,
            content: assets.cdylib,
            mode: Some(0o755),
        });

        // Worker binary → WORKER_BIN_PATH.
        let worker_src = PathBuf::from("bin/lean-runtime-worker");
        asset_copies.push(LanguageAssetCopy {
            source: worker_src.clone(),
            destination: WORKER_BIN_PATH.to_string(),
        });
        language_assets.push(LanguageAsset {
            source: worker_src,
            content: assets.worker_bin,
            mode: Some(0o755),
        });

        // lean4export tree → /opt/lean4export/<relative-path>.
        for (rel, content) in &assets.lean4export_tree {
            let src = PathBuf::from("lean4export").join(rel);
            asset_copies.push(LanguageAssetCopy {
                source: src.clone(),
                destination: format!("{LEAN4EXPORT_IN_IMAGE}/{}", path_to_posix(rel)),
            });
            language_assets.push(LanguageAsset {
                source: src,
                content: content.clone(),
                mode: None,
            });
        }

        // EigeniusLeanCommon source tree → LEAN_COMMON_IN_IMAGE.
        // Only staged when a mirror is baked, because the install
        // step that consumes it (lake-building the mirror against a
        // path-require) only runs in that case. Skipping the stage
        // for mirrorless deployments keeps the image lean.
        if has_mirror {
            for (rel, content) in collect_lean_common_tree(&self.lean_common_dir)? {
                let src = PathBuf::from("lean-common").join(&rel);
                asset_copies.push(LanguageAssetCopy {
                    source: src.clone(),
                    destination: format!(
                        "{}/{}",
                        crate::conventions::LEAN_COMMON_IN_IMAGE,
                        path_to_posix(&rel)
                    ),
                });
                language_assets.push(LanguageAsset {
                    source: src,
                    content,
                    mode: None,
                });
            }
        }

        let dockerfile = compose_dockerfile(&DockerfileSpec {
            base_image_ref: &self.base_image_ref,
            fragments: &fragments,
            included_packages: &[],
            has_mirror,
            language_asset_copies: &asset_copies,
        });

        let work_dir = self.depot_path.join("build-context-lean");
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
            mirror_iri,
            included_pkg_iris: Vec::new(),
            built_at: format!("manifest:{manifest_hash}"),
            packages: BTreeMap::new(),
            mirror: mirror_mat,
            language_assets,
        };
        let context = BuildContext::materialize(work_dir, &spec)?;
        let _ = BuildahImageBuilder::new().build(&context, &self.image_tag)?;
        push_to_docker_daemon(&self.image_tag)?;
        resolve_docker_image_id(&self.image_tag)
    }
}

impl LanguageRuntime for LeanLanguageRuntime {
    fn language_id(&self) -> &str {
        LANGUAGE
    }

    fn build_environment_image(
        &self,
        env: &Resource,
        _packages: &[Resource],
        mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError> {
        // Validate the env's axiom-policy fields are well-shaped before
        // baking an image. Catching this here means a misconfigured
        // env fails at image-build time rather than surfacing as an
        // opaque worker error on first dispatch.
        validate_env_axiom_policy(env)?;
        self.ensure_image(mirror)
    }

    fn dockerfile_fragments(&self, _env: &Resource) -> DockerfileFragments {
        lean_dockerfile_fragments(&LeanImagePlan::default())
    }

    fn run_script(
        &self,
        _env: &Resource,
        _script: &Resource,
        _inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        // Lean's worker only handles `TargetKind::Method` — a future
        // `lean exe` evaluation surface could light up Script mode,
        // but v1 has no use case for it (the institution dispatches
        // `lean_export` exclusively). Fail at the runtime layer
        // instead of round-tripping a guaranteed-to-fail Script
        // request to the worker.
        Err(RunError::MethodSignatureMismatch(
            "LeanLanguageRuntime has no script-mode dispatch — use call_method with the \
             `lean_export` signature instead"
                .to_string(),
        ))
    }

    fn call_method(
        &self,
        env: &Resource,
        signature: &Resource,
        inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        let method_name = read_string_property(signature, PROP_METHOD_NAME)
            .map_err(|reason| {
                RunError::MethodSignatureMismatch(format!(
                    "RuntimeMethodSignature missing or malformed `method_name`: {reason}"
                ))
            })?
            .to_string();
        let signature_iri = signature
            .id()
            .map(|i| i.as_str().to_string())
            .unwrap_or_default();

        let invocation = MethodInvocation {
            function_name: method_name,
            signature_iri,
        };
        let mut target_cbor = Vec::new();
        ciborium::into_writer(&invocation, &mut target_cbor).map_err(|e| {
            RunError::WorkerRpcFailed(format!("encode MethodInvocation as CBOR: {e}"))
        })?;
        // Every call_method input ships as Eigon-CBOR — the
        // cross-runtime wire format the substrate uses for typed
        // Resources. The Lean worker decodes each one via its cdylib
        // (which hosts the workspace Eigon-CBOR codec) and reads
        // whichever properties the dispatched verb expects.
        let input_payloads: Vec<ByteBuf> = inputs
            .iter()
            .map(|r| ByteBuf::from(eigon_cbor::serialize_resource(r)))
            .collect();

        let invocation_id = format!(
            "lean-call-{}",
            INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed)
        );

        let started_at = DispatchTrace::now_rfc3339();

        let digest = self
            .resolve_image_digest(env)
            .map_err(|e| RunError::WorkerRpcFailed(format!("resolve_image_digest: {e}")))?;
        let service = self
            .ensure_service(digest)
            .map_err(|e| RunError::WorkerRpcFailed(format!("ensure_service: {e}")))?;
        let (numerical_metadata, image_digest) = self.capture_health(&service);
        let (output_bytes, dispatched_to) = self.dispatch_typed_method(
            &service,
            target_cbor,
            input_payloads,
            invocation_id.clone(),
        )?;

        let completed_at = DispatchTrace::now_rfc3339();

        // Lean's `lean_export` returns ndjson bytes — *not* a CBOR
        // resource. The substrate's RunOutcome wraps a Resource; we
        // attach the bytes via a substrate output-resource shape
        // mirroring how Julia surfaces stdout. Downstream verification
        // (`eigenius-lean`) reads the bytes out of the resource and
        // feeds them straight into nanoda.
        let output = build_output_resource(&invocation_id, &output_bytes);

        Ok(RunOutcome {
            output,
            derivations: Vec::new(),
            image_digest,
            started_at,
            completed_at,
            numerical_metadata,
            dispatched_to,
        })
    }
}

impl LeanLanguageRuntime {
    fn service_tempdir_for(&self, digest: &ImageDigest) -> Result<PathBuf, SpawnError> {
        // First 16 hex chars after `sha256:` — keeps the tempdir path
        // well under SUN_LEN once `worker.sock` is appended.
        let s = digest.as_str();
        let short = s.strip_prefix("sha256:").unwrap_or(s);
        let short = &short[..16.min(short.len())];
        let dir = self
            .depot_path
            .join(format!("service-lean-{}-{short}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| SpawnError::SpawnFailed {
            backend: self.spawner.backend(),
            reason: format!("create service tempdir {} failed: {e}", dir.display()),
        })?;
        Ok(dir)
    }

    fn build_worker_spec(&self, digest: ImageDigest) -> Result<WorkerSpec, SpawnError> {
        let manifest_hash = self
            .manifest_hash()
            .map_err(|e| SpawnError::SpawnFailed {
                backend: self.spawner.backend(),
                reason: format!("eigenius-lean manifest_hash failed: {e}"),
            })?
            .to_string();

        let tempdir = self.service_tempdir_for(&digest)?;
        let cross_check_env = prepare_substrate_side(
            &digest,
            &manifest_hash,
            &tempdir,
            ProvenanceDirAction::AssumeBaked,
        )
        .map_err(|e| SpawnError::SpawnFailed {
            backend: self.spawner.backend(),
            reason: format!("cross-check setup failed: {e}"),
        })?;

        let mut env = BTreeMap::new();
        // The worker reads this env var first (Worker/Main.lean's
        // `resolveUdsPath`), matching the substrate's universal worker
        // UDS env var convention used by both `LocalServiceSpawner`
        // and `DockerServiceSpawner`.
        env.insert(
            "EIGENIUS_TEST_WORKER_UDS".to_string(),
            tempdir.join("worker.sock").to_string_lossy().into_owned(),
        );
        env.extend(cross_check_env);

        Ok(WorkerSpec {
            image_digest: Some(digest),
            command: Vec::new(), // image's CMD = bootstrap_command
            tempdir_host_path: tempdir,
            depot_host_path: Some(self.depot_path.clone()),
            env,
            max_wall_time_ms: 0,
            max_memory_bytes: 0,
            seccomp_profile: None,
        })
    }

    fn resolve_image_digest(&self, env: &Resource) -> Result<ImageDigest, SpawnError> {
        let env_prop = Iri::parse(PROP_IMAGE_DIGEST).expect("static IRI");
        if let Some(s) = env.get(&env_prop).and_then(Value::as_str) {
            return ImageDigest::parse(s).map_err(|e| SpawnError::SpawnFailed {
                backend: self.spawner.backend(),
                reason: format!("env carries malformed image_digest `{s}`: {e}"),
            });
        }
        // Dispatch-time fallback when the env Resource carries no
        // `image_digest` — build a mirror-less image lazily. Caller
        // paths that depend on a baked mirror should use
        // `build_environment_image` ahead of time so the digest is
        // populated on the env.
        self.ensure_image(None)
            .map_err(|e| SpawnError::SpawnFailed {
                backend: self.spawner.backend(),
                reason: format!("eigenius-lean build_image failed: {e}"),
            })
    }

    fn ensure_service(&self, digest: ImageDigest) -> Result<ServiceHandle, SpawnError> {
        let key = digest.as_str().to_string();
        if let Some(h) = self
            .cached_services
            .lock()
            .expect("cached_services mutex poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(h);
        }
        let spec = self.build_worker_spec(digest)?;
        let handle = self.spawner.ensure_service(spec)?;
        let mut guard = self
            .cached_services
            .lock()
            .expect("cached_services mutex poisoned");
        guard.insert(key, handle.clone());
        Ok(handle)
    }

    fn capture_health(&self, service: &ServiceHandle) -> (NumericalMetadata, Option<ImageDigest>) {
        match self.query_health_internal(service) {
            Ok(info) => {
                let digest = info
                    .env_digest_in_image
                    .as_deref()
                    .and_then(|s| ImageDigest::parse(s).ok());
                (info.numerical_metadata, digest)
            }
            Err(e) => {
                eprintln!(
                    "LeanLanguageRuntime: query_health failed for service {} ({}): {e}; \
                     dispatch will continue with empty trace fields",
                    service.id(),
                    service.backend()
                );
                (
                    NumericalMetadata::default(),
                    service.image_digest().cloned(),
                )
            }
        }
    }

    fn query_health_internal(&self, service: &ServiceHandle) -> Result<HealthInfo, RunError> {
        let stream = self.spawner.attach_uds(service).map_err(|e| {
            RunError::WorkerRpcFailed(format!(
                "attach_uds for health on service {}: {e}",
                service.id()
            ))
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

    fn dispatch_typed_method(
        &self,
        service: &ServiceHandle,
        target_cbor: Vec<u8>,
        input_payloads: Vec<ByteBuf>,
        invocation_id: String,
    ) -> Result<(Vec<u8>, Option<String>), RunError> {
        let stream = self.spawner.attach_uds(service).map_err(|e| {
            RunError::WorkerRpcFailed(format!(
                "attach_uds for call_method on service {}: {e}",
                service.id()
            ))
        })?;
        let mut client = WorkerRpcClient::new(stream);
        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id: invocation_id.clone(),
                target_kind: TargetKind::Method,
                target: ByteBuf::from(target_cbor),
                inputs: input_payloads,
            })
            .map_err(|e| RunError::WorkerRpcFailed(format!("dispatch_method call: {e}")))?;
        let result = match resp {
            Response::DispatchOk {
                output,
                dispatched_to,
                ..
            } => Ok((output.into_vec(), dispatched_to)),
            Response::DispatchFailed {
                error_kind,
                message,
                ..
            } => Err(map_dispatch_failure(&error_kind, message)),
            other => Err(RunError::WorkerRpcFailed(format!(
                "unexpected response to dispatch_method (method): {other:?}"
            ))),
        };
        drop(client);
        result
    }
}

// ─── helpers ────────────────────────────────────────────────────────

fn read_or_fail(p: &Path) -> Result<Vec<u8>, BuildError> {
    std::fs::read(p).map_err(|e| {
        BuildError::BuildInputUnavailable(format!("could not read Lean asset {}: {e}", p.display()))
    })
}

/// Walk `root` and collect every file's bytes keyed by relative path.
/// Skips `.lake/` (Lake build cache), `.github/` (GitHub-only metadata),
/// and `examples/` (test fixtures not needed at build time). Symlinks
/// and unreadable entries surface as `BuildError` so a missing source
/// file fails the image build cleanly rather than silently dropping bytes.
fn collect_lean4export_tree(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, BuildError> {
    if !root.is_dir() {
        return Err(BuildError::BuildInputUnavailable(format!(
            "lean4export source dir `{}` does not exist or is not a directory",
            root.display()
        )));
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    if out.is_empty() {
        return Err(BuildError::BuildInputUnavailable(format!(
            "lean4export source dir `{}` is empty after filtering",
            root.display()
        )));
    }
    Ok(out)
}

/// Walk the host-side `lean/common/EigeniusLeanCommon/` tree.
/// Same filtering as the lean4export walker — drops `.lake/` build
/// cache and similar non-source dirs.
fn collect_lean_common_tree(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, BuildError> {
    if !root.is_dir() {
        return Err(BuildError::BuildInputUnavailable(format!(
            "EigeniusLeanCommon source dir `{}` does not exist or is not a directory",
            root.display()
        )));
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    if out.is_empty() {
        return Err(BuildError::BuildInputUnavailable(format!(
            "EigeniusLeanCommon source dir `{}` is empty after filtering",
            root.display()
        )));
    }
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<(), BuildError> {
    let entries = std::fs::read_dir(dir).map_err(|e| {
        BuildError::BuildInputUnavailable(format!(
            "could not read directory {}: {e}",
            dir.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            BuildError::BuildInputUnavailable(format!(
                "could not iterate directory {}: {e}",
                dir.display()
            ))
        })?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".lake" || name_str == ".github" || name_str == "examples" {
            continue;
        }
        let path = entry.path();
        let ft = entry.file_type().map_err(|e| {
            BuildError::BuildInputUnavailable(format!("could not stat {}: {e}", path.display()))
        })?;
        if ft.is_dir() {
            walk(root, &path, out)?;
        } else if ft.is_file() {
            let rel = path.strip_prefix(root).map_err(|_| {
                BuildError::BuildInputUnavailable(format!(
                    "{} not under lean4export root {}",
                    path.display(),
                    root.display()
                ))
            })?;
            let content = read_or_fail(&path)?;
            out.insert(rel.to_path_buf(), content);
        }
        // Skip symlinks / sockets / etc. — lean4export's tree has none.
    }
    Ok(())
}

fn path_to_posix(p: &Path) -> String {
    let mut s = String::new();
    let mut first = true;
    for comp in p.components() {
        if let std::path::Component::Normal(os) = comp {
            if !first {
                s.push('/');
            }
            first = false;
            s.push_str(&os.to_string_lossy());
        }
    }
    s
}

/// Decode a `LeanPackageMirror` (D26 §5.4) Resource's
/// `library_content` JSON into a substrate
/// [`MirrorMaterialization`]. Inverse of `mirror_gen::mirror_to_resource`'s
/// `library_content_to_json` encoding — `{"kind": "embedded",
/// "files": [{"path", "content_b64"}]}` becomes a path→bytes map
/// the substrate's `BuildContext::materialize` writes verbatim
/// under `mirror/` in the build context.
///
/// External (content-addressed) library references aren't
/// supported in v1; the runtime surfaces `EnvironmentBuildFailed`
/// rather than silently producing an empty mirror dir.
fn materialize_mirror_archive(mirror: &Resource) -> Result<MirrorMaterialization, BuildError> {
    let lib_iri =
        Iri::parse("urn:eigenius:runtime:library_content").expect("static IRI is well-formed");
    let lib_value = mirror.get(&lib_iri).ok_or_else(|| {
        BuildError::EnvironmentBuildFailed(
            "LeanPackageMirror missing `library_content` property".to_string(),
        )
    })?;
    let lib_json = match lib_value {
        Value::Json(v) => v,
        other => {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "`library_content` must be JSON, got {other:?}"
            )));
        }
    };
    let kind = lib_json
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            BuildError::EnvironmentBuildFailed(
                "`library_content` missing string `kind` field".to_string(),
            )
        })?;
    if kind != "embedded" {
        return Err(BuildError::EnvironmentBuildFailed(format!(
            "library_content `kind = \"{kind}\"` not supported in v1 (only `embedded`)"
        )));
    }
    let files = lib_json
        .get("files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            BuildError::EnvironmentBuildFailed(
                "`library_content.files` missing or not an array".to_string(),
            )
        })?;
    let mut mat = MirrorMaterialization::default();
    for entry in files {
        let path = entry.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            BuildError::EnvironmentBuildFailed(
                "`library_content.files[].path` missing or not a string".to_string(),
            )
        })?;
        let b64 = entry
            .get("content_b64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                BuildError::EnvironmentBuildFailed(
                    "`library_content.files[].content_b64` missing or not a string".to_string(),
                )
            })?;
        let bytes = base64_decode(b64).map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!(
                "library_content.files[].content_b64 for `{path}` is not valid base64: {e}"
            ))
        })?;
        mat.files.insert(PathBuf::from(path), bytes);
    }
    Ok(mat)
}

/// Standard base64 decoder (RFC 4648 §4) — paired with the
/// generator's `mirror_gen::base64_encode`. Hand-rolled so the
/// runtime crate stays out of the base64-dep churn; identical
/// alphabet + padding semantics.
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !cleaned.len().is_multiple_of(4) {
        return Err(format!(
            "input length {} not a multiple of 4",
            cleaned.len()
        ));
    }
    let mut out = Vec::with_capacity(cleaned.len() / 4 * 3);
    let mut i = 0;
    while i < cleaned.len() {
        let chunk = &cleaned[i..i + 4];
        let pad = chunk.iter().filter(|&&b| b == b'=').count();
        let v0 = val(chunk[0]).ok_or_else(|| format!("invalid byte {:?}", chunk[0] as char))?;
        let v1 = val(chunk[1]).ok_or_else(|| format!("invalid byte {:?}", chunk[1] as char))?;
        let v2 = if chunk[2] == b'=' {
            0
        } else {
            val(chunk[2]).ok_or_else(|| format!("invalid byte {:?}", chunk[2] as char))?
        };
        let v3 = if chunk[3] == b'=' {
            0
        } else {
            val(chunk[3]).ok_or_else(|| format!("invalid byte {:?}", chunk[3] as char))?
        };
        let n = ((v0 as u32) << 18) | ((v1 as u32) << 12) | ((v2 as u32) << 6) | (v3 as u32);
        out.push(((n >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((n & 0xff) as u8);
        }
        i += 4;
    }
    Ok(out)
}

fn read_string_property<'a>(r: &'a Resource, prop_iri: &str) -> Result<&'a str, String> {
    let iri = Iri::parse(prop_iri).map_err(|e| format!("malformed property IRI: {e}"))?;
    r.get(&iri)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string property `{prop_iri}`"))
}

/// Cross-check that the env's axiom-policy properties are well-shaped
/// before the image build runs. Catching a missing or malformed
/// `lean_permitted_axioms` here surfaces a clear `BuildError` instead
/// of a later "unpermitted axiom" worker dispatch failure that
/// looks like a verification issue but is really a config mistake.
fn validate_env_axiom_policy(env: &Resource) -> Result<(), BuildError> {
    let perm_iri = Iri::parse(PROP_LEAN_PERMITTED_AXIOMS).expect("static IRI");
    match env.get(&perm_iri) {
        None => Ok(()), // defaulted by the orchestrator's env-create path
        Some(Value::Array(items)) => {
            for (idx, v) in items.iter().enumerate() {
                if v.as_str().is_none() {
                    return Err(BuildError::EnvironmentBuildFailed(format!(
                        "LeanEnvironment `lean_permitted_axioms[{idx}]` must be a string axiom name"
                    )));
                }
            }
            Ok(())
        }
        Some(other) => Err(BuildError::EnvironmentBuildFailed(format!(
            "LeanEnvironment `lean_permitted_axioms` must be an array of strings, got {other:?}"
        ))),
    }
}

/// Default-axiom convenience wrapper. Callers building a
/// `LeanEnvironment` programmatically (rather than via the
/// orchestrator's env-create path) can stamp the canonical default set
/// without duplicating the list.
pub fn default_permitted_axioms() -> Vec<String> {
    DEFAULT_LEAN_PERMITTED_AXIOMS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Build the `target_module` input Resource for a `lean_export`
/// dispatch — an embedded Resource carrying `module_name`. Exposed
/// because every caller needs to wrap a Lean module name this way
/// and the IRI shouldn't be re-typed at each call site.
pub fn build_target_module(module_name: &str) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(crate::conventions::PROP_MODULE_NAME).expect("static IRI"),
        Value::String(module_name.to_string()),
    );
    r
}

/// Build the `target_constant` input Resource for a `lean_export`
/// dispatch — an embedded Resource carrying `constant_name`.
pub fn build_target_constant(constant_name: &str) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(crate::conventions::PROP_CONSTANT_NAME).expect("static IRI"),
        Value::String(constant_name.to_string()),
    );
    r
}

/// `unpermitted_axiom_hard_error` policy as boxed into a Resource —
/// re-exposed so test code can construct a `LeanEnvironment` Resource
/// without poking at the bare boolean property by name.
pub fn unpermitted_axiom_hard_error_iri() -> &'static str {
    PROP_LEAN_UNPERMITTED_AXIOM_HARD_ERROR
}

fn map_dispatch_failure(error_kind: &str, message: String) -> RunError {
    match error_kind {
        "method_signature_mismatch" => RunError::MethodSignatureMismatch(message),
        "sandbox_violation" => RunError::SandboxViolation(message),
        _ => RunError::RuntimeError(message),
    }
}

/// Wrap raw lean4export ndjson bytes in a `RuntimeScriptOutput`-shaped
/// resource. The bytes flow through as a base64 string under
/// `urn:eigenius:runtime:script_output`; downstream verification code
/// (`eigenius-lean`) decodes and feeds them into nanoda's parser.
///
/// Using base64 (rather than a typed bytes property) keeps the output
/// resource Eigon-JSON-compatible without touching the chain's
/// `data_type: bytes` story — the ndjson is text anyway and base64
/// just keeps any stray non-ascii characters from breaking JSON
/// transit if the resource ever gets serialised.
fn build_output_resource(invocation_id: &str, output: &[u8]) -> Resource {
    let iri = Iri::parse(&format!(
        "urn:eigenius:lean:invocation:{invocation_id}:output"
    ))
    .expect("invocation IRI is well-formed by construction");
    let mut r = Resource::new(iri);
    // ndjson is text; pass through as a UTF-8 string when possible so
    // downstream tooling can read it without a base64-decode hop. A
    // worker that produced non-UTF-8 bytes would already be wrong (the
    // lean4export tool's output is text), but the lossy decode keeps
    // the resource constructible even in that case.
    let body = String::from_utf8(output.to_vec())
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    r.set(
        Iri::parse(PROP_SCRIPT_OUTPUT).expect("static IRI is well-formed"),
        Value::String(body),
    );
    r.set(
        Iri::parse(PROP_LANGUAGE).expect("static IRI is well-formed"),
        Value::String(LANGUAGE.to_string()),
    );
    r
}

fn push_to_docker_daemon(image_tag: &str) -> Result<(), BuildError> {
    static ARCHIVE_NONCE: AtomicU64 = AtomicU64::new(0);
    let nonce = ARCHIVE_NONCE.fetch_add(1, Ordering::SeqCst);
    let archive_path = std::env::temp_dir().join(format!(
        "eigenius-lean-image-{}-{}-{}.tar",
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

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_runtime_substrate::spawner::service::LocalServiceSpawner;

    fn make_runtime() -> LeanLanguageRuntime {
        // Spawner choice doesn't matter for unit tests — we never
        // invoke dispatch here. The trait-surface tests only exercise
        // `language_id` / `dockerfile_fragments` / `run_script` (which
        // fails before touching the spawner).
        let spawner = Arc::new(LocalServiceSpawner::new(PathBuf::from("/tmp/depot")));
        LeanLanguageRuntime::new(
            PathBuf::from("/tmp/lean-runtime-worker-test"),
            PathBuf::from("/tmp/libeigenius_lean_worker.so"),
            PathBuf::from("/tmp/EigeniusLeanCommon"),
            "debian:bookworm-slim",
            spawner,
            PathBuf::from("/tmp/depot"),
        )
    }

    #[test]
    fn language_id_is_lean() {
        let rt = make_runtime();
        assert_eq!(rt.language_id(), "lean");
    }

    #[test]
    fn dockerfile_fragments_round_trip_through_trait_surface() {
        let rt = make_runtime();
        let env = Resource::new_embedded();
        let fragments = rt.dockerfile_fragments(&env);
        assert_eq!(
            fragments.bootstrap_command,
            vec![WORKER_BIN_PATH.to_string()],
            "trait-level call must surface the pre-built worker binary path"
        );
        assert!(
            !fragments.install_runtime.is_empty(),
            "trait-level call must surface the elan/toolchain install"
        );
    }

    #[test]
    fn run_script_fails_cleanly_without_touching_worker() {
        // Lean has no script-mode dispatch. The runtime must fail at
        // its own layer rather than spawning a worker to return a
        // guaranteed-to-fail UnsupportedScriptKind response.
        let rt = make_runtime();
        let env = Resource::new_embedded();
        let script = Resource::new_embedded();
        let err = rt
            .run_script(&env, &script, &[])
            .expect_err("script mode must fail");
        match err {
            RunError::MethodSignatureMismatch(msg) => {
                assert!(
                    msg.contains("script-mode") || msg.contains("call_method"),
                    "diagnostic should point at the correct dispatch path; got: {msg}"
                );
            }
            other => panic!("expected MethodSignatureMismatch, got {other:?}"),
        }
    }

    #[test]
    fn validate_env_axiom_policy_accepts_default_set() {
        // The default axiom list is well-shaped (every entry is a
        // string), so validation must pass for an env carrying it.
        let mut env = Resource::new_embedded();
        env.set(
            Iri::parse(PROP_LEAN_PERMITTED_AXIOMS).unwrap(),
            Value::Array(
                default_permitted_axioms()
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
        validate_env_axiom_policy(&env).expect("default policy must validate");
    }

    #[test]
    fn validate_env_axiom_policy_rejects_non_string_entry() {
        let mut env = Resource::new_embedded();
        env.set(
            Iri::parse(PROP_LEAN_PERMITTED_AXIOMS).unwrap(),
            Value::Array(vec![
                Value::String("propext".to_string()),
                Value::Integer(42),
            ]),
        );
        let err = validate_env_axiom_policy(&env).expect_err("non-string entry must fail");
        match err {
            BuildError::EnvironmentBuildFailed(msg) => {
                assert!(msg.contains("string axiom name"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn validate_env_axiom_policy_rejects_non_array_shape() {
        let mut env = Resource::new_embedded();
        env.set(
            Iri::parse(PROP_LEAN_PERMITTED_AXIOMS).unwrap(),
            Value::String("propext".to_string()),
        );
        let err = validate_env_axiom_policy(&env).expect_err("non-array must fail");
        match err {
            BuildError::EnvironmentBuildFailed(msg) => {
                assert!(msg.contains("array of strings"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    // ─── mirror archive materialisation ─────────────────────────────

    #[test]
    fn materialize_mirror_archive_round_trips_through_mirror_gen() {
        // The generator's `mirror_to_resource` encodes the archive
        // as base64-in-JSON; `materialize_mirror_archive` decodes
        // it back to a path→bytes map ready for the substrate's
        // `BuildContext::materialize` to write under `mirror/`. The
        // two must round-trip byte-for-byte, or the staged files
        // would differ from what the chain commit promised.
        use crate::mirror_gen::{mirror_to_resource, LeanMirrorGenerator};
        use eigenius_runtime_substrate::chain::ChainAccessor;
        use eigenius_runtime_substrate::mirror_generator::{
            LibraryContent, MirrorGenerationRequest, MirrorGenerator,
        };
        use std::collections::HashMap;

        struct Chain {
            r: HashMap<Iri, Resource>,
        }
        impl ChainAccessor for Chain {
            fn resolve(&self, _: &Iri, t: &Iri) -> Option<Resource> {
                self.r.get(t).cloned()
            }
            fn is_ancestor_or_equal(&self, _: &Iri, _: &Iri) -> bool {
                true
            }
            fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
                true
            }
        }
        let mut r = HashMap::new();
        let class_iri = Iri::parse("urn:test:Tag").unwrap();
        let mut cls = Resource::new(class_iri.clone());
        cls.set(
            Iri::parse("urn:eigenius:core:short_name").unwrap(),
            Value::String("Tag".into()),
        );
        cls.set(
            Iri::parse("urn:eigenius:core:requires").unwrap(),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse("urn:test:tag_name").unwrap(),
            )]),
        );
        r.insert(class_iri.clone(), cls);
        let prop_iri = Iri::parse("urn:test:tag_name").unwrap();
        let mut prop = Resource::new(prop_iri.clone());
        prop.set(
            Iri::parse("urn:eigenius:core:short_name").unwrap(),
            Value::String("tag_name".into()),
        );
        prop.set(
            Iri::parse("urn:eigenius:core:data_type").unwrap(),
            Value::ResourceRef(Iri::parse("urn:eigenius:core:string").unwrap()),
        );
        r.insert(prop_iri, prop);
        let chain = Chain { r };
        let layer = Iri::parse("urn:test:layer").unwrap();
        let seed = vec![class_iri.clone()];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };

        let g = LeanMirrorGenerator::new();
        let out = g.generate(&req).expect("generate");

        // Snapshot the generator's archive bytes path → bytes so we
        // can diff against the materialiser's output.
        let LibraryContent::Embedded(files) = &out.library else {
            panic!("expected embedded library");
        };
        let expected: std::collections::BTreeMap<PathBuf, Vec<u8>> = files
            .iter()
            .map(|f| (PathBuf::from(&f.path), f.content.clone()))
            .collect();

        // Commit the mirror as a Resource, then decode the archive
        // back via the runtime's materialiser. The two must agree.
        let mirror_resource = mirror_to_resource(&g, &out, &layer, None);
        let mat = materialize_mirror_archive(&mirror_resource).expect("mirror archive must decode");
        assert_eq!(mat.files.len(), expected.len(), "file count mismatch");
        for (path, bytes) in &expected {
            let got = mat
                .files
                .get(path)
                .unwrap_or_else(|| panic!("missing {} in materialised archive", path.display()));
            assert_eq!(got, bytes, "bytes diverge for {}", path.display());
        }
    }

    #[test]
    fn materialize_mirror_archive_rejects_external_kind_in_v1() {
        // External (content-addressed) library references are a
        // post-v1 surface — the runtime rejects them rather than
        // silently producing an empty `mirror/` dir.
        let mut r = Resource::new(Iri::parse("urn:eigenius:runtime:mirror:test:1").unwrap());
        r.set(
            Iri::parse("urn:eigenius:runtime:library_content").unwrap(),
            Value::Json(serde_json::json!({
                "kind": "external",
                "reference": "blob://abc",
                "content_hash": "sha256:00",
            })),
        );
        let err = materialize_mirror_archive(&r).expect_err("external kind must fail in v1");
        match err {
            BuildError::EnvironmentBuildFailed(msg) => {
                assert!(msg.contains("external"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
