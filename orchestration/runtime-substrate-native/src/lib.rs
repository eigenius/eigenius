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

//! napi-rs native addon hosting the Eigenius runtime substrate
//! dispatcher for the Deno orchestrator.
//!
//! Phase 18a delivers the `RunRuntimeScript` and `CallRuntimeMethod`
//! orchestrator-side wiring against the substrate. Component
//! dispatches arrive from the kernel via `ComponentExecutor` gRPC,
//! reach the orchestrator's TypeScript handlers, get re-encoded as
//! Eigon-CBOR (matching the kernel ↔ orchestrator codec consolidated
//! in Phase 18e), and cross into Rust through the napi exports below.
//!
//! ## Exports
//!
//! - `registerTestLanguageRuntime(workerBinaryPath)` — register the
//!   bash-c [`TestLanguageRuntime`] for dev / CI / smoke tests. The
//!   path points at the substrate crate's `eigenius-test-worker`
//!   binary (resolved via `CARGO_BIN_EXE_eigenius-test-worker` in
//!   tests).
//! - `dispatchRunRuntimeScript(input, argument)` — dispatch a
//!   `RunRuntimeScript` invocation. `input` and `argument` are
//!   Eigon-CBOR `Buffer`s (the kernel sends them via gRPC; the
//!   orchestrator's handler re-encodes from JS objects via
//!   `wasm/cbor.ts`). Returns the output Resource as Eigon-CBOR.
//! - `dispatchCallRuntimeMethod(input, argument)` — same pattern for
//!   the method-call surface. Service-lifecycle envs land in 19a;
//!   today this returns a `MethodSignatureMismatch` error from any
//!   `JobSpawner`-backed runtime.
//!
//! ## Threading
//!
//! Each dispatch wraps blocking spawn / RPC work in
//! `tokio::task::spawn_blocking` so the napi-rs tokio runtime stays
//! responsive. The dispatcher itself is a process-singleton behind a
//! `Mutex` — fine for v1's spawn-per-invocation, will be revisited
//! when 19a brings the warm-worker pool.

#![deny(clippy::all)]

use eigenius_config::Loader as ConfigLoader;
use eigenius_julia::JuliaLanguageRuntime;
use eigenius_lean_runtime::LeanLanguageRuntime;
use eigenius_oci::OciToolRuntime;
use eigenius_r::RLanguageRuntime;
use eigenius_runtime_substrate::facade::{DispatchOutcome, SubstrateDispatcher};
use eigenius_runtime_substrate::spawner::service::DockerServiceSpawner;
use eigenius_runtime_substrate::spawner::{DockerSpawner, DockerSpawnerConfig};
use eigenius_runtime_substrate::test_runtime::TestLanguageRuntime;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Mirror of [`eigenius_runtime_substrate::facade::DispatchOutcome`] on
/// the napi boundary. Phase 18c.5 split: dispatchers now return both
/// the language-side output Resource and a partial `RuntimeInvocation`
/// Resource carrying the substrate-captured trace (numerical_metadata,
/// timestamps, image digest). The TS handler forwards the output
/// downstream and routes the partial invocation toward provenance
/// commit (D26 §5.5).
#[napi(object)]
pub struct JsDispatchOutcome {
    pub output: Buffer,
    pub partial_invocation: Buffer,
}

impl From<DispatchOutcome> for JsDispatchOutcome {
    fn from(o: DispatchOutcome) -> Self {
        Self {
            output: Buffer::from(o.output_cbor),
            partial_invocation: Buffer::from(o.partial_invocation_cbor),
        }
    }
}

static DISPATCHER: OnceLock<Mutex<SubstrateDispatcher>> = OnceLock::new();

fn dispatcher() -> &'static Mutex<SubstrateDispatcher> {
    DISPATCHER.get_or_init(|| Mutex::new(SubstrateDispatcher::new()))
}

fn lock_err(e: impl std::fmt::Display) -> Error {
    Error::new(
        Status::GenericFailure,
        format!("substrate dispatcher mutex poisoned: {e}"),
    )
}

fn into_napi_err(e: impl std::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, e.to_string())
}

/// Register the bash-backed [`TestLanguageRuntime`] under the `"test"`
/// language ID. Idempotent within a process — calling twice is a
/// programming error and surfaces an explicit
/// `RegistryError::AlreadyRegistered`.
///
/// `worker_binary_path` should resolve to the substrate crate's
/// `eigenius-test-worker` binary. In tests this is
/// `env!("CARGO_BIN_EXE_eigenius-test-worker")`; in dev/prod
/// deployments it is wherever the operator places it.
#[napi]
pub fn register_test_language_runtime(worker_binary_path: String) -> Result<()> {
    let mut d = dispatcher().lock().map_err(lock_err)?;
    d.register_language_runtime(Box::new(TestLanguageRuntime::with_worker_binary(
        PathBuf::from(worker_binary_path),
    )))
    .map_err(into_napi_err)
}

/// Register the [`JuliaLanguageRuntime`] under language_id="julia".
/// Idempotent within a process — calling twice surfaces an explicit
/// `RegistryError::AlreadyRegistered`.
///
/// `worker_project_dir` points at `julia/runtime-worker/` (the directory
/// containing `Project.toml`, `Manifest.toml`, and `src/JuliaWorker.jl`)
/// — copied into the orchestrator image at build time. `base_image_ref`
/// is the digest-pinned Julia base image. `depot_path` is the shared
/// host/container path used for substrate artifacts and worker UDS
/// sockets — must match the orchestrator's bind-mount in
/// `docker-compose.yml` so DooD-spawned worker containers see the same
/// path the orchestrator wrote (D26 §9.5).
#[napi]
pub fn register_julia_language_runtime(
    worker_project_dir: String,
    base_image_ref: String,
    depot_path: String,
) -> Result<()> {
    // Load substrate config so daemon-socket / registry overrides
    // declared in `eigenius.toml` (or `EIGENIUS_DOCKER_DAEMON_SOCKET`)
    // flow through to the spawner without code edits.
    let config = ConfigLoader::new()
        .load()
        .map_err(|e| into_napi_err(format!("eigenius-config load: {e}")))?;

    let depot = PathBuf::from(&depot_path);
    let spawner_config =
        DockerSpawnerConfig::from_substrate_config(depot.clone(), &config.substrate);
    let spawner = DockerServiceSpawner::new(spawner_config)
        .map_err(|e| into_napi_err(format!("DockerServiceSpawner::new: {e}")))?;
    let runtime = JuliaLanguageRuntime::new(
        PathBuf::from(worker_project_dir),
        base_image_ref,
        Arc::new(spawner),
        depot,
    );
    let mut d = dispatcher().lock().map_err(lock_err)?;
    // D53 §7 / Phase 1.5: materialize PinnedExternalFile inputs under the
    // depot so the depot's read-only bind-mount exposes them to the worker.
    d.set_extfile_cache_root(PathBuf::from(&depot_path).join("extfile-cache"));
    d.register_language_runtime(Box::new(runtime))
        .map_err(into_napi_err)
}

/// Register the [`LeanLanguageRuntime`] under language_id="lean".
/// Idempotent within a process — calling twice surfaces an explicit
/// `RegistryError::AlreadyRegistered`.
///
/// `worker_project_dir` points at `lean/runtime-worker/` (the directory
/// containing `lakefile.lean`, the worker Lake project, and the
/// vendored `lean4export/` tree) — copied into the orchestrator image
/// at build time. `cdylib_path` is the host path to
/// `libeigenius_lean_worker.so` the worker binary dynamically links
/// against. `lean_common_dir` points at `lean/common/EigeniusLeanCommon/`
/// — the hand-authored Lake package that generated mirrors depend on;
/// staged into the image so the in-image lake-build of the mirror can
/// resolve `require EigeniusLeanCommon` offline (Phase 20a.6.x).
/// `base_image_ref` is the digest-pinned Debian-slim (or equivalent)
/// base image the Lean toolchain is installed onto. `depot_path` is
/// the shared host/container path used for substrate artifacts and
/// worker UDS sockets — must match the orchestrator's bind-mount in
/// `docker-compose.yml` so DooD-spawned worker containers see the same
/// path the orchestrator wrote (D26 §9.5).
#[napi]
pub fn register_lean_language_runtime(
    worker_project_dir: String,
    cdylib_path: String,
    lean_common_dir: String,
    base_image_ref: String,
    depot_path: String,
) -> Result<()> {
    let config = ConfigLoader::new()
        .load()
        .map_err(|e| into_napi_err(format!("eigenius-config load: {e}")))?;

    let depot = PathBuf::from(&depot_path);
    let spawner_config =
        DockerSpawnerConfig::from_substrate_config(depot.clone(), &config.substrate);
    let spawner = DockerServiceSpawner::new(spawner_config)
        .map_err(|e| into_napi_err(format!("DockerServiceSpawner::new: {e}")))?;
    let runtime = LeanLanguageRuntime::new(
        PathBuf::from(worker_project_dir),
        PathBuf::from(cdylib_path),
        PathBuf::from(lean_common_dir),
        base_image_ref,
        Arc::new(spawner),
        depot,
    );
    let mut d = dispatcher().lock().map_err(lock_err)?;
    // D53 §7 / Phase 1.5: see `register_julia_language_runtime`.
    d.set_extfile_cache_root(PathBuf::from(&depot_path).join("extfile-cache"));
    d.register_language_runtime(Box::new(runtime))
        .map_err(into_napi_err)
}

/// Register the [`OciToolRuntime`] under language_id="oci" (D60). Idempotent
/// within a process — calling twice surfaces an explicit
/// `RegistryError::AlreadyRegistered`.
///
/// `worker_binary_path` is the host path to a pinned Eigenius worker binary
/// (e.g. `eigenius-schemaorg-worker`) staged into the orchestrator image; the
/// runtime bakes it into the tool image at env-build time and computes the
/// cross-check `manifest_hash` from its bytes (so the same binary must back the
/// built image, or the boot cross-check fails closed, D26 §9.3). `base_image_ref`
/// is the digest-pinned base the worker is baked onto (glibc, e.g.
/// `debian:bookworm-slim`). `depot_path` is the shared host/container path for
/// substrate artifacts + worker UDS sockets — must match the orchestrator's
/// bind-mount in `docker-compose.yml` (D26 §9.5). Uses the Job-model
/// `DockerSpawner` (run-once = `lifecycle:Job`), not the warm-pool
/// `DockerServiceSpawner` the language runtimes use.
#[napi]
pub fn register_oci_tool_runtime(
    worker_binary_path: String,
    base_image_ref: String,
    depot_path: String,
) -> Result<()> {
    let config = ConfigLoader::new()
        .load()
        .map_err(|e| into_napi_err(format!("eigenius-config load: {e}")))?;

    let depot = PathBuf::from(&depot_path);
    let spawner_config =
        DockerSpawnerConfig::from_substrate_config(depot.clone(), &config.substrate);
    let spawner = DockerSpawner::new(spawner_config)
        .map_err(|e| into_napi_err(format!("DockerSpawner::new: {e}")))?;
    let runtime = OciToolRuntime::new(
        PathBuf::from(worker_binary_path),
        base_image_ref,
        Arc::new(spawner),
        depot,
    );
    let mut d = dispatcher().lock().map_err(lock_err)?;
    // D53 §7 / Phase 1.5: materialize PinnedExternalFile inputs under the depot
    // so the depot's read-only bind-mount exposes them to the worker.
    d.set_extfile_cache_root(PathBuf::from(&depot_path).join("extfile-cache"));
    d.register_language_runtime(Box::new(runtime))
        .map_err(into_napi_err)
}

/// Register the [`RLanguageRuntime`] under language_id="r". Idempotent
/// within a process — calling twice surfaces an explicit
/// `RegistryError::AlreadyRegistered`.
///
/// `driver_path` points at `EigeniusRWorker.R` and `cdylib_path` at
/// `libeigenius_r_worker.so` — both copied into the orchestrator image at
/// build time and baked into the R worker image (D55 §6). At dispatch the
/// runtime resolves the image from the env Resource's `image_digest`
/// (D26 §5.3) and computes the cross-check `manifest_hash` from this
/// recipe — so `driver`/`cdylib`/`base_image_ref` here must match the
/// recipe that built the R worker image, or the boot cross-check fails
/// closed (D26 §9.3). `depot_path` is the shared host/container path for
/// substrate artifacts + worker UDS sockets — must match the
/// orchestrator's bind-mount in `docker-compose.yml` (D26 §9.5).
#[napi]
pub fn register_r_language_runtime(
    driver_path: String,
    cdylib_path: String,
    base_image_ref: String,
    depot_path: String,
) -> Result<()> {
    let config = ConfigLoader::new()
        .load()
        .map_err(|e| into_napi_err(format!("eigenius-config load: {e}")))?;

    let depot = PathBuf::from(&depot_path);
    let spawner_config =
        DockerSpawnerConfig::from_substrate_config(depot.clone(), &config.substrate);
    let spawner = DockerServiceSpawner::new(spawner_config)
        .map_err(|e| into_napi_err(format!("DockerServiceSpawner::new: {e}")))?;
    let runtime = RLanguageRuntime::new(
        Arc::new(spawner),
        PathBuf::from(driver_path),
        PathBuf::from(cdylib_path),
        depot,
    )
    .with_build_config(base_image_ref, eigenius_r::RImagePlan::default());
    let mut d = dispatcher().lock().map_err(lock_err)?;
    // D53 §7 / Phase 1.5: see `register_julia_language_runtime`.
    d.set_extfile_cache_root(PathBuf::from(&depot_path).join("extfile-cache"));
    d.register_language_runtime(Box::new(runtime))
        .map_err(into_napi_err)
}

/// Dispatch a `RunRuntimeScript` invocation. Async to avoid blocking
/// the napi-rs tokio runtime on spawn / RPC; the inner work runs in a
/// `spawn_blocking` task.
#[napi]
pub async fn dispatch_run_runtime_script(
    input: Buffer,
    argument: Buffer,
    additional_inputs: Option<Vec<Buffer>>,
) -> Result<JsDispatchOutcome> {
    let input_bytes = input.to_vec();
    let argument_bytes = argument.to_vec();
    // Auxiliary `PinnedExternalFile` inputs for a multi-file join (D53 §4.3) —
    // each materialized + content-verified alongside the primary input and
    // bound as `eigenius_inputs[[2..N]]` in the worker.
    let additional_bytes: Vec<Vec<u8>> = additional_inputs
        .unwrap_or_default()
        .into_iter()
        .map(|b| b.to_vec())
        .collect();
    tokio::task::spawn_blocking(move || -> Result<DispatchOutcome> {
        let d = dispatcher().lock().map_err(lock_err)?;
        d.dispatch_run_runtime_script_multi(&input_bytes, &additional_bytes, &argument_bytes)
            .map_err(into_napi_err)
    })
    .await
    .map_err(|e| {
        Error::new(
            Status::GenericFailure,
            format!("dispatch_run_runtime_script join failed: {e}"),
        )
    })?
    .map(JsDispatchOutcome::from)
}

/// Dispatch a `CallRuntimeMethod` invocation. Same shape as
/// [`dispatch_run_runtime_script`]. Phase 18a's `TestLanguageRuntime`
/// returns `MethodSignatureMismatch` here — `CallRuntimeMethod`
/// requires a `lifecycle: Service` env (D26 §5.3.1) and the
/// service-backed dispatcher lands in 19a.
#[napi]
pub async fn dispatch_call_runtime_method(
    input: Buffer,
    argument: Buffer,
) -> Result<JsDispatchOutcome> {
    let input_bytes = input.to_vec();
    let argument_bytes = argument.to_vec();
    tokio::task::spawn_blocking(move || -> Result<DispatchOutcome> {
        let d = dispatcher().lock().map_err(lock_err)?;
        d.dispatch_call_runtime_method(&input_bytes, &argument_bytes)
            .map_err(into_napi_err)
    })
    .await
    .map_err(|e| {
        Error::new(
            Status::GenericFailure,
            format!("dispatch_call_runtime_method join failed: {e}"),
        )
    })?
    .map(JsDispatchOutcome::from)
}

/// Dispatch an external-institution invocation (D31 §6.2 / Phase
/// 19a.5.c). Same `JsDispatchOutcome` shape as the script / method
/// dispatchers above, but the kernel's gRPC handler sends structured
/// metadata fields rather than a single argument Resource — so this
/// entry point takes them as direct parameters and packs them into
/// the substrate's synthesised signature inside
/// `SubstrateDispatcher::dispatch_external_institution`.
///
/// `input_cbors` carries the multi-input list (D31 §6.5). For an
/// AutoOnLoad / Decidable QueryClass dispatch it is a single-element
/// list; for OnDemand / `CallRuntimeMethod` surfaces it can be longer.
#[napi]
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_external_institution(
    language: String,
    env_iri: String,
    image_digest: String,
    method_name: String,
    signature_iri: String,
    input_cbors: Vec<Buffer>,
) -> Result<JsDispatchOutcome> {
    let inputs: Vec<Vec<u8>> = input_cbors.into_iter().map(|b| b.to_vec()).collect();
    tokio::task::spawn_blocking(move || -> Result<DispatchOutcome> {
        let d = dispatcher().lock().map_err(lock_err)?;
        d.dispatch_external_institution(
            &language,
            &env_iri,
            &image_digest,
            &method_name,
            &signature_iri,
            &inputs,
        )
        .map_err(into_napi_err)
    })
    .await
    .map_err(|e| {
        Error::new(
            Status::GenericFailure,
            format!("dispatch_external_institution join failed: {e}"),
        )
    })?
    .map(JsDispatchOutcome::from)
}

/// List the language IDs of currently-registered runtimes. Useful for
/// the orchestrator's startup banner and `/health` reporting.
#[napi]
pub fn list_registered_languages() -> Result<Vec<String>> {
    let d = dispatcher().lock().map_err(lock_err)?;
    Ok(d.registry().languages().map(String::from).collect())
}
