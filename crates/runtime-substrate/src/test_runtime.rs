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

//! `TestLanguageRuntime` — **LocalSpawner-only test mock** wrapping the
//! `eigenius-test-worker` binary as the bash-c smoke runtime.
//!
//! Feature-gated behind `test-runtime`. Used by the substrate's own
//! integration tests and by downstream tests that want a fixture
//! runtime without standing up Julia or another real interpreter.
//! **Not for production use** — there is no image build path
//! (`build_environment_image` explicitly errors), no container
//! isolation, no resource caps; the worker runs as a host subprocess
//! with the substrate's PATH inherited.
//!
//! The runtime expects a `RuntimeScript` resource whose
//! `urn:eigenius:runtime:source` property carries a bash one-liner.
//! Inputs are ignored (the test worker doesn't pass them through).
//! The produced output is a top-level Resource with the bash stdout
//! captured under `urn:eigenius:test:bash_stdout` — a deliberately
//! test-only property so the smoke runtime cannot leak into production
//! code that depends on it.
//!
//! ## Path-3 trait shape
//!
//! Per the Phase 19a.2 trait refactor, the runtime owns its dispatch
//! lifecycle: `run_script` spawns the worker, attaches the RPC
//! channel, fires Health (for trace metadata), dispatches the bash
//! command, sends Evict, captures the worker's exit, and returns a
//! [`RunOutcome`] with output + trace fields. `WorkerHandle` and the
//! spawn/wait orchestration are implementation details.

use crate::cross_check::{prepare_substrate_side, ProvenanceDirAction};
use crate::error::{BuildError, RunError, SpawnError};
use crate::invocation::{DispatchTrace, RunOutcome};
use crate::language_runtime::LanguageRuntime;
use crate::rpc::client::WorkerRpcClient;
use crate::rpc::protocol::{HealthInfo, NumericalMetadata, Request, Response, TargetKind};
use crate::spawner::{LocalSpawner, WorkerSpawner};
use crate::types::{DockerfileFragments, ImageDigest, WorkerHandle, WorkerSpec};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use serde_bytes::ByteBuf;
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const LANGUAGE: &str = "test";
const PROP_SOURCE: &str = "urn:eigenius:runtime:source";
const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";
const PROP_TEST_BASH_STDOUT: &str = "urn:eigenius:test:bash_stdout";
const UDS_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Synthetic cross-check anchors used by the test runtime — there is no
/// real built image under [`LocalSpawner`], so the digest and manifest
/// hash are picked once and threaded through both sides of the
/// cross-check so it always matches at startup.
const TEST_IMAGE_DIGEST: &str =
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const TEST_MANIFEST_HASH: &str = "test-runtime-manifest";

static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Bash-c smoke runtime. Spawns the `eigenius-test-worker` binary (built
/// when the `test-runtime` feature is on) via [`LocalSpawner`] and
/// dispatches via the substrate's CBOR RPC.
pub struct TestLanguageRuntime {
    spawner: LocalSpawner,
    worker_binary: PathBuf,
}

impl TestLanguageRuntime {
    /// Construct with an explicit path to the `eigenius-test-worker`
    /// binary. Tests in this crate find the path via
    /// `env!("CARGO_BIN_EXE_eigenius-test-worker")`; downstream
    /// consumers point at whatever binary they ship.
    pub fn with_worker_binary(worker_binary: PathBuf) -> Self {
        Self {
            spawner: LocalSpawner::new(),
            worker_binary,
        }
    }

    /// Spawn-dispatch-cleanup orchestration shared by `run_script` and
    /// `call_method`. Returns the worker's stdout plus the trace
    /// fields needed to build a `RunOutcome`.
    fn dispatch_with_target(
        &self,
        target_cbor: Vec<u8>,
        invocation_id: String,
    ) -> Result<DispatchOutput, RunError> {
        let started_at = DispatchTrace::now_rfc3339();

        let worker = self
            .spawn_internal()
            .map_err(|e| RunError::WorkerRpcFailed(format!("spawn_worker: {e}")))?;

        // Capture health before dispatch — gives us numerical_metadata +
        // image_digest_in_image for the trace. Health failures are
        // best-effort; the dispatch contract is not.
        let (numerical_metadata, image_digest) = self.capture_health(&worker);

        let stdout = self
            .dispatch_and_evict(&worker, target_cbor, invocation_id.clone())
            .inspect_err(|_| {
                // On error, also try to evict so the worker doesn't
                // linger if Drop alone wouldn't tear it down.
                let _ = self.try_evict(&worker);
            })?;

        let completed_at = DispatchTrace::now_rfc3339();

        Ok(DispatchOutput {
            invocation_id,
            stdout,
            numerical_metadata,
            image_digest,
            started_at,
            completed_at,
        })
    }

    fn spawn_internal(&self) -> Result<WorkerHandle, SpawnError> {
        let n = INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tempdir = std::env::temp_dir().join(format!(
            "eigenius-test-runtime-{}-{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&tempdir).map_err(|e| SpawnError::SpawnFailed {
            backend: "test-runtime",
            reason: format!("create tempdir failed: {e}"),
        })?;

        let uds_path = tempdir.join("worker.sock");
        let mut env = BTreeMap::new();
        env.insert(
            "EIGENIUS_TEST_WORKER_UDS".to_string(),
            uds_path.to_string_lossy().into_owned(),
        );

        // Cross-check (D26 §9.3): write the manifest-hash file into a
        // host-visible provenance dir under the per-invocation tempdir
        // and populate the matching env vars so the worker's startup
        // check passes. `LocalSpawner` doesn't run a container so there
        // is no `/etc/eigenius-runtime-env` — the override is required.
        let prov_dir = tempdir.join("provenance");
        let cross_check_digest =
            ImageDigest::parse(TEST_IMAGE_DIGEST).expect("static test digest is well-formed");
        let cross_check_env = prepare_substrate_side(
            &cross_check_digest,
            TEST_MANIFEST_HASH,
            &prov_dir,
            ProvenanceDirAction::WriteFile,
        )
        .map_err(|e| SpawnError::SpawnFailed {
            backend: "test-runtime",
            reason: format!("cross-check setup failed: {e}"),
        })?;
        env.extend(cross_check_env);

        // PATH passes through so the worker can find /bin/bash. Other
        // host-env values are deliberately not inherited so the test
        // surface stays deterministic.
        if let Ok(path) = std::env::var("PATH") {
            env.insert("PATH".to_string(), path);
        }

        let spec = WorkerSpec {
            image_digest: None,
            command: vec![self.worker_binary.to_string_lossy().into_owned()],
            tempdir_host_path: tempdir,
            depot_host_path: None,
            env,
            max_wall_time_ms: 0,
            max_memory_bytes: 0,
            seccomp_profile: None,
        };
        self.spawner.spawn(spec)
    }

    fn capture_health(&self, worker: &WorkerHandle) -> (NumericalMetadata, Option<ImageDigest>) {
        match self.query_health_internal(worker) {
            Ok(info) => {
                let digest = info
                    .env_digest_in_image
                    .as_deref()
                    .and_then(|s| ImageDigest::parse(s).ok());
                (info.numerical_metadata, digest)
            }
            Err(e) => {
                eprintln!(
                    "TestLanguageRuntime: query_health failed for worker {} ({}): {e}; \
                     dispatch will continue with empty trace fields",
                    worker.id, worker.backend
                );
                (NumericalMetadata::default(), None)
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

        // Evict so the worker exits cleanly.
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
        let stream = UnixStream::connect(&worker.uds_path)
            .map_err(|e| RunError::WorkerRpcFailed(format!("evict-on-error connect: {e}")))?;
        let mut client = WorkerRpcClient::new(stream);
        client
            .call(&Request::Evict)
            .map_err(|e| RunError::WorkerRpcFailed(format!("evict-on-error call: {e}")))?;
        Ok(())
    }
}

/// Internal trace bundle for `dispatch_with_target`'s callers.
struct DispatchOutput {
    invocation_id: String,
    stdout: String,
    numerical_metadata: NumericalMetadata,
    image_digest: Option<ImageDigest>,
    started_at: String,
    completed_at: String,
}

impl LanguageRuntime for TestLanguageRuntime {
    fn language_id(&self) -> &str {
        LANGUAGE
    }

    fn build_environment_image(
        &self,
        _env: &Resource,
        _packages: &[Resource],
        _mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError> {
        Err(BuildError::EnvironmentBuildFailed(
            "TestLanguageRuntime has no image build path; use LocalSpawner deployment shape (c)"
                .to_string(),
        ))
    }

    fn dockerfile_fragments(&self, _env: &Resource) -> DockerfileFragments {
        DockerfileFragments::default()
    }

    fn run_script(
        &self,
        _env: &Resource,
        script: &Resource,
        _inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        let source = read_string_property(script, PROP_SOURCE).map_err(|reason| {
            RunError::MethodSignatureMismatch(format!(
                "RuntimeScript missing or malformed `source`: {reason}"
            ))
        })?;

        let mut target_cbor = Vec::new();
        ciborium::into_writer(source, &mut target_cbor)
            .map_err(|e| RunError::WorkerRpcFailed(format!("encode bash command as CBOR: {e}")))?;

        let invocation_id = format!(
            "test-inv-{}",
            INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed)
        );

        let DispatchOutput {
            invocation_id,
            stdout,
            numerical_metadata,
            image_digest,
            started_at,
            completed_at,
        } = self.dispatch_with_target(target_cbor, invocation_id)?;

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
            "TestLanguageRuntime does not implement call_method (use run_script with a bash one-liner)"
                .to_string(),
        ))
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
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }
}

fn map_dispatch_failure(error_kind: &str, message: String) -> RunError {
    match error_kind {
        "method_signature_mismatch" => RunError::MethodSignatureMismatch(message),
        "sandbox_violation" => RunError::SandboxViolation(message),
        // Unknown variants fold into RuntimeError so caller still gets
        // the diagnostic.
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

// Tests for TestLanguageRuntime live in tests/test_runtime_integration.rs
// because they need env!("CARGO_BIN_EXE_eigenius-test-worker") which is
// only available in integration tests, not in lib unit tests.
