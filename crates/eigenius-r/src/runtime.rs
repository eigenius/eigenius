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
use eigenius_runtime_substrate::error::{BuildError, RunError};
use eigenius_runtime_substrate::invocation::{DispatchTrace, RunOutcome};
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::rpc::protocol::{NumericalMetadata, Request, Response, TargetKind};
use eigenius_runtime_substrate::rpc::WorkerRpcClient;
use eigenius_runtime_substrate::spawner::service::ServiceSpawner;
use eigenius_runtime_substrate::types::{DockerfileFragments, ImageDigest, WorkerSpec};
use serde_bytes::ByteBuf;

use crate::conventions;

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
}

impl RLanguageRuntime {
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
        }
    }

    /// `WorkerSpec` for the local (host-subprocess) backend: command
    /// `Rscript <driver>`, no image, cdylib path supplied via env. The
    /// Docker backend (P3) will instead set `image_digest` + an empty
    /// command (the image's CMD launches the worker) — only this method
    /// changes, not the dispatch path.
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
        Err(BuildError::EnvironmentBuildFailed(
            "R image build lands in D55 P3 (RImagePlan + renv.lock + buildah); \
             use LocalServiceSpawner until then"
                .to_string(),
        ))
    }

    fn dockerfile_fragments(&self, _env: &Resource) -> DockerfileFragments {
        // P3 fills these (pinned bioconductor base + renv restore + worker
        // copy). Default is valid for LocalServiceSpawner-only deployments.
        DockerfileFragments::default()
    }

    fn run_script(
        &self,
        _env: &Resource,
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
            .ensure_service(self.local_worker_spec())
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
                output: build_output_resource(&invocation_id, output.into_vec()),
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
