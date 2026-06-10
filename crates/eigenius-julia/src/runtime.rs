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
    LANGUAGE, PROP_IMAGE_DIGEST, PROP_LANGUAGE, PROP_METHOD_NAME, PROP_PACKAGE_MANIFEST,
    PROP_PACKAGE_NAME, PROP_PACKAGE_SOURCE_TREE, PROP_SCRIPT_OUTPUT, PROP_SOURCE,
    WORKER_PROJECT_DIR,
};
use crate::dockerfile::{julia_dockerfile_fragments, JuliaImagePlan};
use crate::eigenius_common::{self, COMMON_PACKAGE_NAME};
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::cross_check::{prepare_substrate_side, ProvenanceDirAction};
use eigenius_runtime_substrate::error::{BuildError, RunError, SpawnError};
use eigenius_runtime_substrate::image_build::dockerfile::{IncludedPackage, LanguageAssetCopy};
use eigenius_runtime_substrate::image_build::{
    compose_dockerfile, BuildContext, BuildContextSpec, BuildahImageBuilder, DockerfileSpec,
    ImageBuilder, LanguageAsset, MirrorMaterialization, PackageMaterialization,
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

/// Return shape of [`JuliaRuntime::dispatch_typed_method`]:
/// `(output_cbor, derivations_cbor, dispatched_to)`.
type DispatchTypedMethodOutput = (Vec<u8>, Vec<Vec<u8>>, Option<String>);

/// `LanguageRuntime` impl that runs the Julia worker as a long-lived
/// **service** (D26 §8.1 Service lifecycle). The substrate calls this
/// through the language registry whenever a `RuntimeScript` /
/// `RuntimeMethodSignature` resource declares `language = "julia"`.
///
/// Phase 19a.3.e wires this against [`ServiceSpawner`] so a single
/// long-lived worker per env amortises Julia's cold-start across many
/// dispatches. The previous per-invocation `DockerSpawner` path
/// (19a.1) shipped before the warm-pool design and is no longer used —
/// `CallRuntimeMethod` (19a.4) needs sub-second latency that
/// per-invocation spawns can't deliver for Julia.
pub struct JuliaLanguageRuntime {
    spawner: Arc<dyn ServiceSpawner>,
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
    /// `image_digest` → `ServiceHandle` map populated lazily as
    /// dispatches arrive. One service per distinct digest — the
    /// orchestrator's external-institution path can dispatch into
    /// multiple envs concurrently from a single runtime instance, so
    /// the cache must be keyed by digest rather than holding one
    /// service. `drain` walks every cached handle to tear them down
    /// at shutdown.
    cached_services: Mutex<HashMap<String, ServiceHandle>>,
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
    /// `docker.io/library/julia@sha256:...`), a `ServiceSpawner`
    /// (typically `DockerServiceSpawner` for local development or a
    /// future Container Apps / Kubernetes backend), and the depot
    /// path the spawner was configured with.
    pub fn new(
        project_dir: PathBuf,
        base_image_ref: impl Into<String>,
        spawner: Arc<dyn ServiceSpawner>,
        depot_path: PathBuf,
    ) -> Self {
        let base = base_image_ref.into();
        // Derive a buildah-friendly tag suffix from the base image
        // reference. Docker tag syntax forbids a `-` immediately
        // before the `:tag` separator, so we trim trailing separators
        // after truncation. `take(24)` keeps the suffix short enough
        // for buildah's name length limits.
        let safe_prefix: String = base
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(24)
            .collect::<String>()
            .trim_end_matches(['-', '_'])
            .to_string();
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
            cached_services: Mutex::new(HashMap::new()),
        }
    }

    /// Tear down every long-lived service worker this runtime
    /// instance opened. Idempotent. Used at orchestrator shutdown
    /// and by tests for clean cleanup. After `drain`, the next
    /// dispatch transparently re-spawns the service it needs.
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

    fn ensure_image(
        &self,
        packages: &[Resource],
        mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError> {
        // Cache key today is just `(project files, base image)` — the
        // packages and mirror parameters do not invalidate the cache
        // because v1 generates a single mirror + handler-package set
        // per `JuliaLanguageRuntime` instance and the caller supplies
        // the same value across calls. Once multiple
        // mirrors/handler-package sets per runtime become a thing
        // (D27 §3.6 future-work), the cache key must include each
        // input's content hash.
        if let Some(d) = self.cached_digest.get() {
            return Ok(d.clone());
        }
        let digest = self.build_image(packages, mirror)?;
        let _ = self.cached_digest.set(digest.clone());
        Ok(digest)
    }

    fn build_image(
        &self,
        handler_packages: &[Resource],
        mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError> {
        let manifest_hash = self.manifest_hash()?.to_string();
        let assets = self.assets()?.clone();

        // Always bake the hand-authored EigeniusJuliaCommon package —
        // it's the import target every generated mirror uses, and it's
        // tiny. The substrate's image cache shares layers across envs
        // that share the same Common version, so the cost is paid once.
        let mut packages: BTreeMap<String, PackageMaterialization> = BTreeMap::new();
        packages.insert(
            COMMON_PACKAGE_NAME.to_string(),
            eigenius_common::package_materialization(),
        );
        let mut included_packages = vec![IncludedPackage {
            name: COMMON_PACKAGE_NAME.to_string(),
        }];

        // Bake institution handler packages. Each `RuntimePackage`
        // resource carries a `package_name`, a verbatim `Project.toml`
        // (`manifest`), and a `source_tree` JSON archive. The
        // substrate writes these under `/opt/eigenius/packages/<name>/`
        // and the dockerfile composer emits a matching `Pkg.develop`
        // call so the package's `[deps]` resolve into the worker
        // project's manifest at instantiate time.
        let mut handler_pkg_iris: Vec<String> = Vec::new();
        let mut handler_pkg_names: Vec<String> = Vec::new();
        for pkg in handler_packages {
            let mat = runtime_package_to_materialization(pkg)?;
            if packages.contains_key(&mat.name) {
                return Err(BuildError::EnvironmentBuildFailed(format!(
                    "duplicate package name `{}` (clashes with EigeniusJuliaCommon or another handler package)",
                    mat.name
                )));
            }
            packages.insert(mat.name.clone(), mat.materialization);
            included_packages.push(IncludedPackage {
                name: mat.name.clone(),
            });
            handler_pkg_names.push(mat.name);
            if let Some(iri) = pkg.id() {
                handler_pkg_iris.push(iri.as_str().to_string());
            }
        }

        // Materialise the mirror archive when one was supplied.
        let mirror_iri = mirror
            .and_then(|m| m.id().map(|iri| iri.as_str().to_string()))
            .unwrap_or_default();
        let mirror_mat = mirror.map(materialize_mirror).transpose()?;

        let plan = JuliaImagePlan {
            include_common: true,
            include_mirror: mirror.is_some(),
            handler_packages: handler_pkg_names,
        };
        let fragments = julia_dockerfile_fragments(&plan);

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
            included_packages: &included_packages,
            has_mirror: mirror_mat.is_some(),
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
            mirror_iri,
            included_pkg_iris: handler_pkg_iris,
            built_at: format!("manifest:{manifest_hash}"),
            packages,
            mirror: mirror_mat,
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
        packages: &[Resource],
        mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError> {
        self.ensure_image(packages, mirror)
    }

    fn dockerfile_fragments(&self, _env: &Resource) -> DockerfileFragments {
        // The substrate calls this for spec-level inspection (no mirror
        // context). Production image build goes through
        // `build_environment_image` which builds the plan from the
        // env's mirror; this surface is the reference fragment shape.
        julia_dockerfile_fragments(&JuliaImagePlan {
            include_common: true,
            include_mirror: false,
            handler_packages: Vec::new(),
        })
    }

    fn run_script(
        &self,
        env: &Resource,
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
            .map_err(|e| RunError::WorkerRpcFailed(format!("encode julia source as CBOR: {e}")))?;

        let invocation_id = format!(
            "julia-inv-{}",
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
        let stdout = self.dispatch_script(&service, target_cbor, invocation_id.clone())?;

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
        env: &Resource,
        signature: &Resource,
        inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        // 1. Read method_name from the signature. The signature's
        // own IRI flows through as `dispatched_to`'s context anchor
        // (the worker echoes it back so the trace records what the
        // dispatch was supposed to satisfy).
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

        // 2. Encode the MethodInvocation directive (function_name +
        // signature_iri) and each input resource for the wire.
        let invocation = MethodInvocation {
            function_name: method_name,
            signature_iri,
        };
        let mut target_cbor = Vec::new();
        ciborium::into_writer(&invocation, &mut target_cbor).map_err(|e| {
            RunError::WorkerRpcFailed(format!("encode MethodInvocation as CBOR: {e}"))
        })?;
        let input_payloads: Vec<ByteBuf> = inputs
            .iter()
            .map(|r| ByteBuf::from(eigon_cbor::serialize_resource(r)))
            .collect();

        let invocation_id = format!(
            "julia-call-{}",
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
        let (output_bytes, derivation_byte_lists, dispatched_to) =
            self.dispatch_typed_method(&service, target_cbor, input_payloads, invocation_id)?;

        let completed_at = DispatchTrace::now_rfc3339();

        // 3. Decode the output as an Eigon resource. The mirror's
        // `encode_<C>` produces the standard JSON-LD-shaped dict; the
        // kernel's lenient parser accepts that shape directly.
        let output = eigon_cbor::parse_resource_lenient(&output_bytes).map_err(|e| {
            RunError::WorkerRpcFailed(format!("decode worker output as Eigon resource: {e}"))
        })?;

        // 4. Decode each per-effect derivation the Julia institution
        // emitted (D52 §6 — InstitutionEmittedDerivation). Empty for
        // institutions whose only job is the pass/fail gate.
        let mut derivations = Vec::with_capacity(derivation_byte_lists.len());
        for (i, bytes) in derivation_byte_lists.iter().enumerate() {
            let r = eigon_cbor::parse_resource_lenient(bytes).map_err(|e| {
                RunError::WorkerRpcFailed(format!(
                    "decode worker derivation #{i} as Eigon resource: {e}"
                ))
            })?;
            derivations.push(r);
        }

        Ok(RunOutcome {
            output,
            derivations,
            image_digest,
            started_at,
            completed_at,
            numerical_metadata,
            dispatched_to,
        })
    }
}

impl JuliaLanguageRuntime {
    /// Per-service host directory. Created lazily on first dispatch
    /// and reused across subsequent dispatches against the same
    /// service. The spawner bind-mounts this into the container
    /// (DooD discipline, D26 §9.5) so the worker's UDS shows up here.
    /// Per-digest service tempdir under the depot. The UDS path
    /// inside is unique per digest so two services running for
    /// different envs don't collide on the same socket. Idempotent —
    /// the directory creation is safe to repeat.
    fn service_tempdir_for(&self, digest: &ImageDigest) -> Result<PathBuf, SpawnError> {
        // Take the first 16 hex chars after the `sha256:` prefix as a
        // short, filesystem-safe service tag. Keeps the resulting
        // tempdir path well under SUN_LEN (108 bytes) when joined
        // with the depot path + `worker.sock`.
        let s = digest.as_str();
        let short = s.strip_prefix("sha256:").unwrap_or(s);
        let short = &short[..16.min(short.len())];
        let dir = self
            .depot_path
            .join(format!("service-julia-{}-{short}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| SpawnError::SpawnFailed {
            backend: self.spawner.backend(),
            reason: format!("create service tempdir {} failed: {e}", dir.display()),
        })?;
        Ok(dir)
    }

    /// Construct a `WorkerSpec` for the env's service. Caller passes
    /// the digest the worker should run against — extracted from the
    /// dispatch's env Resource (D31 §6.2 path) or, as a fallback, the
    /// runtime's lazily-built image (the `build_environment_image`
    /// path used by tests). The spawner's
    /// `ensure_service` keys on `image_digest`, so identical inputs
    /// resolve to the same `ServiceHandle`.
    fn build_worker_spec(&self, digest: ImageDigest) -> Result<WorkerSpec, SpawnError> {
        let manifest_hash = self
            .manifest_hash()
            .map_err(|e| SpawnError::SpawnFailed {
                backend: self.spawner.backend(),
                reason: format!("eigenius-julia manifest_hash failed: {e}"),
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

    /// Resolve which image to dispatch into. Reads `image_digest`
    /// from the env Resource first — the orchestrator's external-
    /// institution path stamps it on the synthesised env so each
    /// dispatch can land on a different image without a per-runtime
    /// cached digest. Falls back to `ensure_image` (lazy build via
    /// `cached_digest`) for the legacy `RunRuntimeScript` /
    /// `CallRuntimeMethod` callers that pre-populate the cache via
    /// `build_environment_image`.
    fn resolve_image_digest(&self, env: &Resource) -> Result<ImageDigest, SpawnError> {
        let env_prop = Iri::parse(PROP_IMAGE_DIGEST).expect("static IRI");
        if let Some(s) = env.get(&env_prop).and_then(Value::as_str) {
            return ImageDigest::parse(s).map_err(|e| SpawnError::SpawnFailed {
                backend: self.spawner.backend(),
                reason: format!("env carries malformed image_digest `{s}`: {e}"),
            });
        }
        self.ensure_image(&[], None)
            .map_err(|e| SpawnError::SpawnFailed {
                backend: self.spawner.backend(),
                reason: format!("eigenius-julia build_image failed: {e}"),
            })
    }

    /// Get-or-start the long-lived service worker for the given
    /// digest. The spawner's `ensure_service` is idempotent for the
    /// same `image_digest`; we additionally cache the handle locally
    /// keyed by digest so [`drain`] can find every service this
    /// runtime instance opened.
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
                    "JuliaLanguageRuntime: query_health failed for service {} ({}): {e}; \
                     dispatch will continue with empty trace fields",
                    service.id(),
                    service.backend()
                );
                // Fall back to the digest the spawner remembered when
                // it created the service — better than nothing.
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

    /// Dispatch a script through the warm service. No `Evict` after —
    /// the worker stays alive for the next dispatch (D26 §8.1
    /// Service-mode lifecycle).
    fn dispatch_script(
        &self,
        service: &ServiceHandle,
        target_cbor: Vec<u8>,
        invocation_id: String,
    ) -> Result<String, RunError> {
        let stream = self.spawner.attach_uds(service).map_err(|e| {
            RunError::WorkerRpcFailed(format!(
                "attach_uds for dispatch on service {}: {e}",
                service.id()
            ))
        })?;
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
        drop(client);
        Ok(stdout)
    }

    /// Dispatch a typed method call through the warm service. Returns
    /// the raw output bytes (a CBOR-encoded mirror dict that the
    /// caller decodes against the signature's `output_type`) plus
    /// per-derivation payloads and the `dispatched_to` string
    /// captured by the worker via `which()`.
    fn dispatch_typed_method(
        &self,
        service: &ServiceHandle,
        target_cbor: Vec<u8>,
        input_payloads: Vec<ByteBuf>,
        invocation_id: String,
    ) -> Result<DispatchTypedMethodOutput, RunError> {
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
                derivations,
                dispatched_to,
                ..
            } => {
                let derivation_bytes = derivations.into_iter().map(ByteBuf::into_vec).collect();
                Ok((output.into_vec(), derivation_bytes, dispatched_to))
            }
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

fn read_or_fail(p: &Path) -> Result<Vec<u8>, BuildError> {
    std::fs::read(p).map_err(|e| {
        BuildError::BuildInputUnavailable(format!(
            "could not read Julia project file {}: {e}",
            p.display()
        ))
    })
}

/// Decode a `RuntimePackageMirror` resource's `library_content` JSON
/// payload back into the file map the substrate's image-build pipeline
/// materialises under `mirror/`. Inverse of
/// [`crate::mirror_gen::mirror_to_resource`]'s embedded encoding —
/// `{"kind": "embedded", "files": [{"path": ..., "content_b64": ...}]}`.
///
/// External library references are deferred (D26 §7.2 future-work);
/// substrate-side mirrors stay in-band today.
fn materialize_mirror(mirror: &Resource) -> Result<MirrorMaterialization, BuildError> {
    let lib_iri = Iri::parse("urn:eigenius:runtime:library_content")
        .expect("library_content IRI is well-formed by construction");
    let lib_value = mirror.get(&lib_iri).ok_or_else(|| {
        BuildError::EnvironmentBuildFailed(
            "mirror resource missing `library_content` property".to_string(),
        )
    })?;
    let lib_json = match lib_value {
        Value::Json(v) => v,
        other => {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "mirror `library_content` must be JSON, got {other:?}"
            )));
        }
    };
    let kind = lib_json
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            BuildError::EnvironmentBuildFailed(
                "mirror `library_content` missing string `kind` field".to_string(),
            )
        })?;
    if kind != "embedded" {
        return Err(BuildError::EnvironmentBuildFailed(format!(
            "mirror `library_content.kind = \"{kind}\"` not yet supported (only `embedded`)"
        )));
    }
    let files = lib_json
        .get("files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            BuildError::EnvironmentBuildFailed(
                "mirror `library_content.files` missing or not an array".to_string(),
            )
        })?;
    let mut mat = MirrorMaterialization::default();
    for entry in files {
        let path = entry.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            BuildError::EnvironmentBuildFailed(
                "mirror `library_content.files[].path` missing or not a string".to_string(),
            )
        })?;
        let b64 = entry
            .get("content_b64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                BuildError::EnvironmentBuildFailed(
                    "mirror `library_content.files[].content_b64` missing or not a string"
                        .to_string(),
                )
            })?;
        let content = base64_decode(b64).map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!(
                "mirror `library_content.files[].content_b64` for `{path}` is not valid base64: {e}"
            ))
        })?;
        mat.files.insert(PathBuf::from(path), content);
    }
    Ok(mat)
}

/// Decode standard base64 (RFC 4648 §4) — pair to the encoder used by
/// `mirror_gen::base64_encode`. The decoder is permissive on
/// whitespace inside the payload (none expected, but a stray newline
/// shouldn't fail loudly) and strict on illegal chars.
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

/// Parsed view of a `RuntimePackage` Resource ready to bake into the
/// build context. Holds the package's directory name (used as both the
/// `IncludedPackage::name` and the in-image directory under
/// `/opt/eigenius/packages/`) plus the file map the substrate's
/// materialiser writes verbatim.
#[derive(Debug)]
struct ParsedHandlerPackage {
    name: String,
    materialization: PackageMaterialization,
}

/// Parse a `RuntimePackage` Resource into a [`ParsedHandlerPackage`].
/// Reads:
///   - `runtime:package_name` → directory name in the build context.
///   - `runtime:manifest`     → verbatim `Project.toml` bytes.
///   - `runtime:source_tree`  → JSON archive (`[{path, content_base64}]`).
///     Each entry's bytes get written under
///     `packages/<name>/<path>` in the build context.
///
/// Surface errors as [`BuildError::EnvironmentBuildFailed`] so the
/// caller (`build_image`) can attribute the failure to the build
/// step rather than a generic invocation error.
fn runtime_package_to_materialization(
    resource: &Resource,
) -> Result<ParsedHandlerPackage, BuildError> {
    let name = read_string_property(resource, PROP_PACKAGE_NAME)
        .map_err(|e| BuildError::EnvironmentBuildFailed(format!("RuntimePackage: {e}")))?
        .to_string();
    if name.is_empty() {
        return Err(BuildError::EnvironmentBuildFailed(
            "RuntimePackage: `package_name` must not be empty".into(),
        ));
    }
    let manifest = read_string_property(resource, PROP_PACKAGE_MANIFEST)
        .map_err(|e| BuildError::EnvironmentBuildFailed(format!("RuntimePackage `{name}`: {e}")))?
        .to_string();

    let source_tree_iri =
        Iri::parse(PROP_PACKAGE_SOURCE_TREE).expect("static source_tree IRI must parse");
    let source_tree = match resource.get(&source_tree_iri) {
        Some(Value::Json(v)) => v,
        Some(other) => {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "RuntimePackage `{name}`: `source_tree` must be a JSON value, got {other:?}"
            )));
        }
        None => {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "RuntimePackage `{name}`: `source_tree` is required"
            )));
        }
    };
    let entries = source_tree.as_array().ok_or_else(|| {
        BuildError::EnvironmentBuildFailed(format!(
            "RuntimePackage `{name}`: `source_tree` must be a JSON array of `{{path, content_base64}}` objects"
        ))
    })?;

    let mut files: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
    files.insert(PathBuf::from("Project.toml"), manifest.into_bytes());

    for (idx, entry) in entries.iter().enumerate() {
        let obj = entry.as_object().ok_or_else(|| {
            BuildError::EnvironmentBuildFailed(format!(
                "RuntimePackage `{name}`: `source_tree[{idx}]` must be an object"
            ))
        })?;
        let path = obj.get("path").and_then(serde_json::Value::as_str).ok_or_else(|| {
            BuildError::EnvironmentBuildFailed(format!(
                "RuntimePackage `{name}`: `source_tree[{idx}].path` is required and must be a string"
            ))
        })?;
        let b64 = obj
            .get("content_base64")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                BuildError::EnvironmentBuildFailed(format!(
                    "RuntimePackage `{name}`: `source_tree[{idx}].content_base64` is required and must be a string"
                ))
            })?;
        let content = base64_decode(b64).map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!(
                "RuntimePackage `{name}`: failed to base64-decode `source_tree[{idx}]` (path `{path}`): {e}"
            ))
        })?;
        // Reject anything that escapes the package directory — `..`
        // segments and absolute paths would break out of the
        // materialised tree under `packages/<name>/`.
        let p = PathBuf::from(path);
        if p.is_absolute()
            || p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "RuntimePackage `{name}`: `source_tree[{idx}].path` `{path}` must be relative and stay inside the package directory"
            )));
        }
        if files.insert(p, content).is_some() {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "RuntimePackage `{name}`: duplicate `source_tree[{idx}].path` `{path}`"
            )));
        }
    }

    Ok(ParsedHandlerPackage {
        name,
        materialization: PackageMaterialization { files },
    })
}

fn map_dispatch_failure(error_kind: &str, message: String) -> RunError {
    match error_kind {
        "method_signature_mismatch" => RunError::MethodSignatureMismatch(message),
        "sandbox_violation" => RunError::SandboxViolation(message),
        _ => RunError::RuntimeError(message),
    }
}

fn build_output_resource(invocation_id: &str, output: String) -> Resource {
    let iri = Iri::parse(&format!(
        "urn:eigenius:julia:invocation:{invocation_id}:output"
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
    use eigenius_runtime_substrate::chain::ChainAccessor;
    use eigenius_runtime_substrate::mirror_generator::{
        LibraryContent, MirrorGenerationRequest, MirrorGenerator,
    };
    use std::collections::HashMap;

    /// Hand-build a tiny chain with one class to exercise the
    /// generator → resource → materialiser pipeline without standing
    /// up the full kinase fixture.
    struct OneClassChain {
        resources: HashMap<Iri, Resource>,
    }

    impl OneClassChain {
        fn new() -> Self {
            let mut resources = HashMap::new();

            let class_iri = Iri::parse("urn:eigenius:test:Demo").unwrap();
            let mut cls = Resource::new(class_iri.clone());
            cls.set(
                Iri::parse("urn:eigenius:core:short_name").unwrap(),
                Value::String("Demo".into()),
            );
            cls.set(
                Iri::parse("urn:eigenius:core:requires").unwrap(),
                Value::Array(vec![Value::ResourceRef(
                    Iri::parse("urn:eigenius:test:name").unwrap(),
                )]),
            );
            resources.insert(class_iri, cls);

            let prop_iri = Iri::parse("urn:eigenius:test:name").unwrap();
            let mut prop = Resource::new(prop_iri.clone());
            prop.set(
                Iri::parse("urn:eigenius:core:short_name").unwrap(),
                Value::String("name".into()),
            );
            prop.set(
                Iri::parse("urn:eigenius:core:data_type").unwrap(),
                Value::ResourceRef(Iri::parse("urn:eigenius:core:string").unwrap()),
            );
            resources.insert(prop_iri, prop);

            Self { resources }
        }
    }

    impl ChainAccessor for OneClassChain {
        fn resolve(&self, _claim_layer: &Iri, target: &Iri) -> Option<Resource> {
            self.resources.get(target).cloned()
        }
        fn is_ancestor_or_equal(&self, _: &Iri, _: &Iri) -> bool {
            true
        }
        fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
            true
        }
    }

    /// End-to-end on the substrate side (no Docker): generator emits a
    /// library archive, `mirror_to_resource` commits it, and
    /// `materialize_mirror` decodes it back. Together these three steps
    /// are the contract D26 §7 places on the chain — every byte that
    /// goes onto the resource has to come back at image-build time, or
    /// the worker won't get the source it expects.
    #[test]
    fn chain_to_mirror_to_materialization_round_trip() {
        let g = JuliaMirrorGenerator::new();
        let chain = OneClassChain::new();
        let layer = Iri::parse("urn:eigenius:test:layer").unwrap();
        let seed = vec![Iri::parse("urn:eigenius:test:Demo").unwrap()];

        let out = g
            .generate(&MirrorGenerationRequest {
                source_layer: &layer,
                seed_classes: &seed,
                chain: &chain,
            })
            .expect("generate");
        let resource = mirror_to_resource(&g, &out, &layer, Some("1970-01-01T00:00:00Z"));

        let mat = materialize_mirror(&resource).expect("materialize");

        // Files materialised back must equal the generator's output —
        // path-by-path, byte-by-byte.
        let LibraryContent::Embedded(files) = &out.library else {
            panic!("expected embedded library");
        };
        assert_eq!(mat.files.len(), files.len());
        for f in files {
            let got = mat
                .files
                .get(&PathBuf::from(&f.path))
                .unwrap_or_else(|| panic!("materialised mirror missing `{}`", f.path));
            assert_eq!(
                got, &f.content,
                "byte-identical round-trip for `{}`",
                f.path
            );
        }
    }

    #[test]
    fn materialize_mirror_rejects_external_kind() {
        // External library references aren't supported in v1 — a
        // resource carrying `kind = "external"` must fail loudly so
        // the build path doesn't silently produce an empty mirror dir.
        let mut r = Resource::new(Iri::parse("urn:eigenius:runtime:mirror:test:1").unwrap());
        r.set(
            Iri::parse("urn:eigenius:runtime:library_content").unwrap(),
            Value::Json(serde_json::json!({
                "kind": "external",
                "reference": "blob://store/abc",
                "content_hash": "sha256:00",
            })),
        );
        let err = materialize_mirror(&r).expect_err("external must fail");
        match err {
            BuildError::EnvironmentBuildFailed(msg) => {
                assert!(msg.contains("external"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn materialize_mirror_rejects_missing_library_content() {
        let r = Resource::new(Iri::parse("urn:eigenius:runtime:mirror:test:2").unwrap());
        let err = materialize_mirror(&r).expect_err("missing library_content must fail");
        match err {
            BuildError::EnvironmentBuildFailed(msg) => {
                assert!(msg.contains("library_content"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    // ─── runtime_package_to_materialization ────────────────────────

    /// Build a `RuntimePackage` Resource carrying the given files
    /// (path → string content). The content is base64-encoded into
    /// the JSON `source_tree` shape the parser expects.
    fn build_package_resource(name: &str, manifest: &str, files: &[(&str, &[u8])]) -> Resource {
        // Minimal base64 helper that mirrors the production decoder's
        // input shape — `STANDARD` charset, '=' padding.
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        fn encode(input: &[u8]) -> String {
            let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
            for chunk in input.chunks(3) {
                let b0 = chunk[0];
                let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
                let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
                out.push(ALPHABET[(b0 >> 2) as usize] as char);
                out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
                if chunk.len() > 1 {
                    out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
                } else {
                    out.push('=');
                }
                if chunk.len() > 2 {
                    out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
                } else {
                    out.push('=');
                }
            }
            out
        }
        let mut r = Resource::new(
            Iri::parse(&format!("urn:eigenius:test:pkg:{name}")).expect("static IRI"),
        );
        r.set(
            Iri::parse(PROP_PACKAGE_NAME).unwrap(),
            Value::String(name.to_string()),
        );
        r.set(
            Iri::parse(PROP_PACKAGE_MANIFEST).unwrap(),
            Value::String(manifest.to_string()),
        );
        let entries: Vec<serde_json::Value> = files
            .iter()
            .map(|(path, bytes)| {
                serde_json::json!({
                    "path": path,
                    "content_base64": encode(bytes),
                })
            })
            .collect();
        r.set(
            Iri::parse(PROP_PACKAGE_SOURCE_TREE).unwrap(),
            Value::Json(serde_json::Value::Array(entries)),
        );
        r
    }

    #[test]
    fn runtime_package_to_materialization_round_trips_files() {
        let manifest = "name = \"EigeniusIntervals\"\nuuid = \"...\"\n";
        let src = b"module EigeniusIntervals end\n";
        let r = build_package_resource(
            "EigeniusIntervals",
            manifest,
            &[("src/EigeniusIntervals.jl", src)],
        );
        let parsed = runtime_package_to_materialization(&r).expect("parse");
        assert_eq!(parsed.name, "EigeniusIntervals");
        // Project.toml comes from `manifest`, src/ files from
        // `source_tree`; both materialised under the package directory.
        let project_bytes = parsed
            .materialization
            .files
            .get(&PathBuf::from("Project.toml"))
            .expect("Project.toml present");
        assert_eq!(project_bytes, manifest.as_bytes());
        let jl_bytes = parsed
            .materialization
            .files
            .get(&PathBuf::from("src/EigeniusIntervals.jl"))
            .expect("source file present");
        assert_eq!(jl_bytes, src);
    }

    #[test]
    fn runtime_package_to_materialization_rejects_path_escape() {
        // `..` segments must be rejected — they'd materialise files
        // outside the package directory under `packages/<name>/`.
        let r = build_package_resource(
            "BadPkg",
            "name = \"BadPkg\"\n",
            &[("../escape.jl", b"oops")],
        );
        let err = runtime_package_to_materialization(&r).expect_err("path escape must fail");
        match err {
            BuildError::EnvironmentBuildFailed(msg) => {
                assert!(msg.contains("must be relative"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn runtime_package_to_materialization_rejects_missing_source_tree() {
        let mut r = Resource::new(Iri::parse("urn:eigenius:test:pkg:NoSrc").unwrap());
        r.set(
            Iri::parse(PROP_PACKAGE_NAME).unwrap(),
            Value::String("NoSrc".into()),
        );
        r.set(
            Iri::parse(PROP_PACKAGE_MANIFEST).unwrap(),
            Value::String("name = \"NoSrc\"\n".into()),
        );
        let err =
            runtime_package_to_materialization(&r).expect_err("missing source_tree must fail");
        match err {
            BuildError::EnvironmentBuildFailed(msg) => {
                assert!(msg.contains("source_tree"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
