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

//! [`RLanguageRuntime`] — the R [`LanguageRuntime`] over the substrate's
//! [`ServiceSpawner`] (D55 P2).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::cross_check::{prepare_substrate_side, ProvenanceDirAction};
use eigenius_runtime_substrate::error::{BuildError, RunError};
use eigenius_runtime_substrate::image_build::{
    BuildContext, BuildContextSpec, BuildahImageBuilder, ImageBuilder, LanguageAsset,
};
use eigenius_runtime_substrate::invocation::{DispatchTrace, RunOutcome};
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::rpc::protocol::{NumericalMetadata, Request, Response, TargetKind};
use eigenius_runtime_substrate::rpc::WorkerRpcClient;
use eigenius_runtime_substrate::spawner::service::ServiceSpawner;
use eigenius_runtime_substrate::types::{DockerfileFragments, ImageDigest, WorkerSpec};
use serde_bytes::ByteBuf;
use sha2::{Digest, Sha256};

use crate::conventions;
use crate::dockerfile::{compose_image_dockerfile, r_dockerfile_fragments, RImagePlan};

/// Default base: a digest-pinned Bioconductor image (R + BiocManager).
/// `RELEASE_3_18` is a representative tag — pin a real `@sha256:` digest
/// when first building for production. The built image's own `ImageDigest`
/// is the binding pin regardless.
/// The Bioconductor base image the R env build extends by default. Public so
/// the CLI can pass it to [`RLanguageRuntime::with_build_config`] when building
/// with an explicit package plan (the build script path).
pub const DEFAULT_BASE_IMAGE: &str = "docker.io/bioconductor/bioconductor_docker:RELEASE_3_18";

/// Binds the runtime to a pinned R image (the production / Docker backend):
/// the built image `digest` + the `manifest_hash` (the `renv.lock` hash) the
/// cross-check verifies. Absent ⇒ the LocalServiceSpawner dev backend.
#[derive(Debug, Clone)]
pub struct RImageBinding {
    pub digest: ImageDigest,
    pub manifest_hash: String,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The R language runtime. Holds the substrate [`ServiceSpawner`] (the
/// same abstraction `eigenius-julia` uses) plus the paths it needs to
/// describe the R worker to a `WorkerSpec`. The dev backend is
/// `LocalServiceSpawner` (host `Rscript`); the production backend is a
/// `DockerServiceSpawner` with the digest-pinned R image (P3) — the
/// dispatch code below is identical for both.
pub struct RLanguageRuntime {
    spawner: Arc<dyn ServiceSpawner>,
    /// Path to `EigeniusRWorker.R` (the worker the local backend runs).
    driver_path: PathBuf,
    /// Path to the `eigenius-r-worker` cdylib the driver `dyn.load`s.
    cdylib_path: PathBuf,
    /// Depot directory under which per-service tempdirs (and the worker
    /// UDS) are created.
    depot_path: PathBuf,
    /// `Some` ⇒ the production Docker backend: dispatch picks
    /// [`Self::docker_worker_spec`] (pinned `image_digest` + cross-check).
    /// `None` ⇒ the LocalServiceSpawner dev backend
    /// ([`Self::local_worker_spec`]). The `run_script` dispatch path is the
    /// same for both — only the `WorkerSpec` differs.
    image: Option<RImageBinding>,
    /// Base image the env build extends. Defaults to [`DEFAULT_BASE_IMAGE`];
    /// overridable via [`Self::with_build_config`] (e.g. a lighter base for
    /// a fast e2e).
    base_image_ref: String,
    /// The image recipe (Bioconductor release + packages). Defaults to
    /// [`RImagePlan::default`]; overridable via [`Self::with_build_config`].
    image_plan: RImagePlan,
}

impl RLanguageRuntime {
    /// Dev backend: `LocalServiceSpawner` runs `Rscript <driver_path>` on
    /// the host, `dyn.load`ing the on-disk `cdylib_path`. No pinned image.
    pub fn new(
        spawner: Arc<dyn ServiceSpawner>,
        driver_path: PathBuf,
        cdylib_path: PathBuf,
        depot_path: PathBuf,
    ) -> Self {
        Self {
            spawner,
            driver_path,
            cdylib_path,
            depot_path,
            image: None,
            base_image_ref: DEFAULT_BASE_IMAGE.to_string(),
            image_plan: RImagePlan::default(),
        }
    }

    /// Override the base image + recipe used by `build_environment_image`
    /// (e.g. a lighter base + empty package list for a fast e2e, or a
    /// digest-pinned production base). Builder-style; returns `self`.
    pub fn with_build_config(
        mut self,
        base_image_ref: impl Into<String>,
        image_plan: RImagePlan,
    ) -> Self {
        self.base_image_ref = base_image_ref.into();
        self.image_plan = image_plan;
        self
    }

    /// Production backend: a Docker `ServiceSpawner` runs the digest-pinned
    /// R image (driver + cdylib baked at the in-image paths; the image CMD
    /// launches the worker). `driver_path`/`cdylib_path` are unused on this
    /// path (the image supplies them), so this constructor takes only the
    /// `image` binding + depot.
    pub fn with_image(
        spawner: Arc<dyn ServiceSpawner>,
        depot_path: PathBuf,
        image: RImageBinding,
    ) -> Self {
        Self {
            spawner,
            driver_path: PathBuf::from(conventions::DRIVER_IN_IMAGE),
            cdylib_path: PathBuf::from(conventions::CDYLIB_IN_IMAGE),
            depot_path,
            image: Some(image),
            base_image_ref: DEFAULT_BASE_IMAGE.to_string(),
            image_plan: RImagePlan::default(),
        }
    }

    /// The `WorkerSpec` for this runtime's backend. One dispatch path; the
    /// spec is the only thing that differs between Local and Docker.
    fn worker_spec(&self) -> Result<WorkerSpec, RunError> {
        match &self.image {
            Some(binding) => self.docker_worker_spec(binding),
            None => Ok(self.local_worker_spec()),
        }
    }

    /// Resolve the `WorkerSpec` for a dispatch, preferring the **env
    /// Resource's** `image_digest` over the construction-time backing.
    /// This mirrors `eigenius-julia`'s `resolve_image_digest`: the
    /// orchestrator's `SubstrateDispatcher` synthesises an env Resource
    /// carrying the `RuntimeEnvironment.image_digest` for each dispatch, so
    /// one registered runtime serves whatever image the env declares (D26
    /// §5.3 / §5.5). The cross-check `manifest_hash` comes from this
    /// runtime's own recipe (`image_manifest_hash` — the composed
    /// Dockerfile + driver script; the cdylib is excluded, see
    /// `manifest_hash`), exactly as Julia computes it from `self`; an env
    /// built from the same recipe matches, and a mismatch fails the boot
    /// cross-check
    /// closed (D26 §9.3). Falls back to [`Self::worker_spec`] (the
    /// construction-time `self.image` / local backend) when the env carries
    /// no digest — the `LocalServiceSpawner` dev + test path.
    fn worker_spec_for_env(&self, env: &Resource) -> Result<WorkerSpec, RunError> {
        const PROP_IMAGE_DIGEST: &str = "urn:eigenius:runtime:image_digest";
        let digest_str = env
            .get(&Iri::parse(PROP_IMAGE_DIGEST).expect("static IRI"))
            .and_then(Value::as_str);
        match digest_str {
            Some(s) => {
                let digest = ImageDigest::parse(s).map_err(|e| {
                    RunError::WorkerRpcFailed(format!(
                        "env carries malformed image_digest `{s}`: {e}"
                    ))
                })?;
                let manifest_hash = self
                    .image_manifest_hash()
                    .map_err(|e| RunError::WorkerRpcFailed(format!("manifest_hash: {e}")))?;
                self.docker_worker_spec(&RImageBinding {
                    digest,
                    manifest_hash,
                })
            }
            None => self.worker_spec(),
        }
    }

    /// `WorkerSpec` for the local (host-subprocess) backend: command
    /// `Rscript <driver>`, no image, cdylib path supplied via env.
    fn local_worker_spec(&self) -> WorkerSpec {
        let tempdir = self
            .depot_path
            .join(format!("service-r-{}", std::process::id()));
        let mut env = BTreeMap::new();
        env.insert(
            conventions::ENV_CDYLIB.to_string(),
            self.cdylib_path.to_string_lossy().into_owned(),
        );
        WorkerSpec {
            image_digest: None,
            command: vec![
                "Rscript".to_string(),
                self.driver_path.to_string_lossy().into_owned(),
            ],
            tempdir_host_path: tempdir,
            depot_host_path: Some(self.depot_path.clone()),
            env,
            max_wall_time_ms: 0,
            max_memory_bytes: 0,
            seccomp_profile: None,
        }
    }

    /// `WorkerSpec` for the production Docker backend: the pinned
    /// `image_digest`, an empty command (the image CMD runs the baked
    /// worker), and the cross-check env (`prepare_substrate_side`,
    /// `AssumeBaked` — the `manifest-hash` file is in the image layer) so
    /// the worker's boot cross-check (D26 §9.3) can fail closed on a
    /// pinned-environment mismatch. The cdylib env points at the in-image
    /// baked path.
    fn docker_worker_spec(&self, binding: &RImageBinding) -> Result<WorkerSpec, RunError> {
        let tempdir = self
            .depot_path
            .join(format!("service-r-docker-{}", std::process::id()));
        let cross_env = prepare_substrate_side(
            &binding.digest,
            &binding.manifest_hash,
            &tempdir,
            ProvenanceDirAction::AssumeBaked,
        )
        .map_err(|e| RunError::WorkerRpcFailed(format!("cross-check setup: {e}")))?;

        let mut env = BTreeMap::new();
        env.insert(
            conventions::ENV_CDYLIB.to_string(),
            conventions::CDYLIB_IN_IMAGE.to_string(),
        );
        env.extend(cross_env);

        Ok(WorkerSpec {
            image_digest: Some(binding.digest.clone()),
            command: Vec::new(), // image CMD = bootstrap_command (Rscript driver)
            tempdir_host_path: tempdir,
            depot_host_path: Some(self.depot_path.clone()),
            env,
            max_wall_time_ms: 0,
            max_memory_bytes: 0,
            seccomp_profile: None,
        })
    }

    /// Read the worker assets to bake (the on-disk driver + cdylib from
    /// `new`'s host paths). `build_environment_image` is a `new`-runtime
    /// operation; calling it on a `with_image` runtime reads the in-image
    /// paths, which don't exist on the host (a clear error).
    fn read_assets(&self) -> Result<(Vec<u8>, Vec<u8>), BuildError> {
        let driver = std::fs::read(&self.driver_path).map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!(
                "read driver {}: {e}",
                self.driver_path.display()
            ))
        })?;
        let cdylib = std::fs::read(&self.cdylib_path).map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!(
                "read cdylib {}: {e}",
                self.cdylib_path.display()
            ))
        })?;
        Ok((driver, cdylib))
    }

    /// The manifest hash the built image bakes (and the cross-check
    /// verifies). Deterministic from the recipe + baked assets, so the
    /// production wiring recomputes it after `build_environment_image` to
    /// construct the [`RImageBinding`] for [`Self::with_image`].
    pub fn image_manifest_hash(&self) -> Result<String, BuildError> {
        // `read_assets` still validates the cdylib exists (it's baked into
        // the image), but only the driver feeds the hash (see `manifest_hash`).
        let (driver, _cdylib) = self.read_assets()?;
        let dockerfile = compose_image_dockerfile(&self.base_image_ref, &self.image_plan);
        Ok(manifest_hash(&dockerfile, &driver))
    }
}

impl LanguageRuntime for RLanguageRuntime {
    fn language_id(&self) -> &str {
        conventions::LANGUAGE
    }

    fn build_environment_image(
        &self,
        _env: &Resource,
        _packages: &[Resource],
        _mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError> {
        let (driver, cdylib) = self.read_assets()?;
        let dockerfile = compose_image_dockerfile(&self.base_image_ref, &self.image_plan);
        let manifest_hash = manifest_hash(&dockerfile, &driver);

        let work_dir = self.depot_path.join("build-context-r");
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
            language_assets: vec![
                LanguageAsset {
                    source: PathBuf::from("EigeniusRWorker.R"),
                    content: driver,
                    mode: None,
                },
                LanguageAsset {
                    source: PathBuf::from("libeigenius_r_worker.so"),
                    content: cdylib,
                    mode: None,
                },
            ],
        };
        let context = BuildContext::materialize(work_dir, &spec)?;
        let image_tag = format!("eigenius-r:{}", short_hash(&manifest_hash));
        let _ = BuildahImageBuilder::new().build(&context, &image_tag)?;
        push_to_docker_daemon(&image_tag)?;
        resolve_docker_image_id(&image_tag)
    }

    fn dockerfile_fragments(&self, _env: &Resource) -> DockerfileFragments {
        // The image recipe (D55 P3): bioconductor base ships R, install_packages
        // restores the pinned renv.lock, bootstrap runs the worker driver. The
        // mirror step (P4) is off until the S4 mirror generator lands.
        r_dockerfile_fragments(&RImagePlan::default())
    }

    fn run_script(
        &self,
        env: &Resource,
        script: &Resource,
        inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        let source = read_source(script)?;

        let mut target = Vec::new();
        ciborium::into_writer(&source, &mut target)
            .map_err(|e| RunError::WorkerRpcFailed(format!("encode R source as CBOR: {e}")))?;
        let input_payloads: Vec<ByteBuf> = inputs
            .iter()
            .map(|r| ByteBuf::from(eigon_cbor::serialize_resource(r)))
            .collect();

        let invocation_id = format!("r-inv-{}", COUNTER.fetch_add(1, Ordering::Relaxed));
        let started_at = DispatchTrace::now_rfc3339();

        let handle = self
            .spawner
            .ensure_service(self.worker_spec_for_env(env)?)
            .map_err(|e| RunError::WorkerRpcFailed(format!("ensure_service: {e}")))?;

        // Dispatch on a fresh connection, then always drain. Per-invocation
        // lifecycle for P2; the warm-pool reuse the Julia runtime does is a
        // later optimisation (and is a spawner concern, not this code path).
        let dispatch = (|| -> Result<Response, RunError> {
            let stream = self
                .spawner
                .attach_uds(&handle)
                .map_err(|e| RunError::WorkerRpcFailed(format!("attach_uds: {e}")))?;
            let mut client = WorkerRpcClient::new(stream);
            client
                .call(&Request::DispatchMethod {
                    invocation_id: invocation_id.clone(),
                    target_kind: TargetKind::Script,
                    target: ByteBuf::from(target),
                    inputs: input_payloads,
                })
                .map_err(|e| RunError::WorkerRpcFailed(format!("dispatch: {e}")))
        })();
        let _ = self.spawner.drain(&handle);

        let completed_at = DispatchTrace::now_rfc3339();

        match dispatch? {
            Response::DispatchOk { output, .. } => Ok(RunOutcome {
                // The worker returns either the CBOR of an Eigon resource
                // (the script built one via the marshalling helpers — the
                // P5+ path) or opaque bytes (the simple P2 path). Parse the
                // former into the typed output `Resource`; wrap the latter.
                output: match eigon_cbor::parse_resource_lenient(&output) {
                    Ok(resource) => resource,
                    Err(_) => build_output_resource(&invocation_id, output.into_vec()),
                },
                derivations: Vec::new(),
                image_digest: None,
                started_at,
                completed_at,
                numerical_metadata: NumericalMetadata::default(),
                dispatched_to: None,
            }),
            Response::DispatchFailed {
                error_kind,
                message,
                ..
            } => Err(RunError::RuntimeError(format!(
                "R worker: {error_kind}: {message}"
            ))),
            other => Err(RunError::WorkerRpcFailed(format!(
                "unexpected response to DispatchMethod: {other:?}"
            ))),
        }
    }

    fn call_method(
        &self,
        _env: &Resource,
        _signature: &Resource,
        _inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        Err(RunError::WorkerRpcFailed(
            "typed call_method lands in D55 P4 (S4 mirror); use run_script".to_string(),
        ))
    }
}

/// Read the `source` string off a `RuntimeScript` resource.
fn read_source(script: &Resource) -> Result<String, RunError> {
    let iri = Iri::parse(conventions::PROP_SOURCE).expect("static IRI is well-formed");
    match script.get(&iri) {
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(RunError::MethodSignatureMismatch(
            "RuntimeScript missing or malformed string `source`".to_string(),
        )),
    }
}

/// Wrap the worker's output bytes in a provisional output resource (the
/// Julia runtime's 19a.1 anchor shape; the typed Eigon `DerivedResource`
/// output lands with the matrix marshalling in P5).
fn build_output_resource(invocation_id: &str, output: Vec<u8>) -> Resource {
    let iri = Iri::parse(&format!("urn:eigenius:r:invocation:{invocation_id}:output"))
        .expect("invocation IRI is well-formed by construction");
    let mut r = Resource::new(iri);
    r.set(
        Iri::parse(conventions::PROP_SCRIPT_OUTPUT).expect("static IRI"),
        Value::String(String::from_utf8_lossy(&output).into_owned()),
    );
    r.set(
        Iri::parse(conventions::PROP_LANGUAGE).expect("static IRI"),
        Value::String(conventions::LANGUAGE.to_string()),
    );
    r
}

/// The image's manifest hash: `sha256(dockerfile || driver)` — the
/// reproducibility surface (the Bioconductor base + pinned R packages, via
/// the composed Dockerfile, plus the driver script source). The boot
/// cross-check (D26 §9.3) verifies it against the in-image `manifest-hash`
/// file.
///
/// The compiled `eigenius-r-worker` **cdylib is deliberately excluded**.
/// It's our transport shim (pinned by the worker crate version), not a
/// scientific-reproducibility variable, and it's built independently on
/// two hosts in this flow — the CLI's `env build` (host `rustc`, workspace
/// path `/home/...`) and the orchestrator image (`rust:…-bookworm`,
/// `/build`). Rust release binaries aren't byte-reproducible across
/// differing toolchains + embedded source paths, so hashing the cdylib
/// bytes makes the cross-check fail spuriously across build environments
/// without adding reproducibility value. Pinning a single canonical cdylib
/// (build once, copy to both) would be the way to include it; hashing two
/// independent builds is not.
fn manifest_hash(dockerfile: &str, driver: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(dockerfile.as_bytes());
    hasher.update(driver);
    format!("sha256:{:x}", hasher.finalize())
}

/// First 16 hex chars of a `sha256:<hex>` manifest hash — a short,
/// filesystem/tag-safe service/image tag.
fn short_hash(manifest_hash: &str) -> String {
    let hex = manifest_hash
        .strip_prefix("sha256:")
        .unwrap_or(manifest_hash);
    hex[..16.min(hex.len())].to_string()
}

/// Push a buildah-built image into the local docker daemon (so a
/// `DockerServiceSpawner` can run it) and return its docker image id.
/// Per-crate copy of the helper the Julia/Lean runtimes use (these are not
/// shared in the substrate).
fn push_to_docker_daemon(image_tag: &str) -> Result<(), BuildError> {
    let archive_path = std::env::temp_dir().join(format!(
        "eigenius-r-image-{}-{}.tar",
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

fn sanitise_for_path(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_runtime_substrate::spawner::service::LocalServiceSpawner;

    fn dummy_spawner(depot: &std::path::Path) -> Arc<dyn ServiceSpawner> {
        Arc::new(LocalServiceSpawner::new(depot.to_path_buf()))
    }

    #[test]
    fn local_backend_spec_has_command_and_no_image() {
        let depot = std::env::temp_dir().join("eigenius-r-spec-local");
        let rt = RLanguageRuntime::new(
            dummy_spawner(&depot),
            PathBuf::from("/x/EigeniusRWorker.R"),
            PathBuf::from("/x/libeigenius_r_worker.so"),
            depot,
        );
        let spec = rt.worker_spec().expect("local spec");
        assert!(
            spec.image_digest.is_none(),
            "local backend has no pinned image"
        );
        assert_eq!(spec.command.first().map(String::as_str), Some("Rscript"));
        assert_eq!(
            spec.env.get(conventions::ENV_CDYLIB).map(String::as_str),
            Some("/x/libeigenius_r_worker.so"),
            "cdylib path passed via env"
        );
    }

    #[test]
    fn docker_backend_spec_is_pinned_with_cross_check() {
        let depot = std::env::temp_dir().join("eigenius-r-spec-docker");
        let digest = ImageDigest::parse(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("valid digest");
        let rt = RLanguageRuntime::with_image(
            dummy_spawner(&depot),
            depot,
            RImageBinding {
                digest,
                manifest_hash: "renvlockhash123".to_string(),
            },
        );
        let spec = rt.worker_spec().expect("docker spec");
        assert!(
            spec.image_digest.is_some(),
            "Docker spec carries the pinned image"
        );
        assert!(spec.command.is_empty(), "image CMD runs the baked worker");
        // cdylib env points at the in-image baked path.
        assert_eq!(
            spec.env.get(conventions::ENV_CDYLIB).map(String::as_str),
            Some(conventions::CDYLIB_IN_IMAGE)
        );
        // cross-check env carries the manifest hash the worker verifies (D26 §9.3).
        assert_eq!(
            spec.env
                .get("EIGENIUS_RUNTIME_ENV_MANIFEST_HASH")
                .map(String::as_str),
            Some("renvlockhash123")
        );
    }
}
