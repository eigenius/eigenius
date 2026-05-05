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

//! `DockerServiceSpawner` — DooD-launched persistent service container
//! per env. **Local-development backend** for Service-mode envs.
//!
//! Same Bollard / DooD machinery as the per-invocation
//! [`super::super::docker::DockerSpawner`], but with `auto_remove: false`
//! so the container survives across many dispatches until the substrate
//! explicitly drains it. One container per `image_digest`; repeated
//! `ensure_service` calls for the same digest return the same handle.
//!
//! Production deployments target Azure Container Apps / Kubernetes;
//! those backends will implement the same `ServiceSpawner` trait and
//! handle scaling at the platform level.

use crate::error::SpawnError;
use crate::spawner::docker::{container, depot, lifecycle, DockerSpawnerConfig};
use crate::spawner::service::{ServiceHandle, ServiceSpawner};
use crate::types::WorkerSpec;
use bollard::Docker;
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BACKEND: &str = "docker-service";

/// How long `attach_uds` waits for the worker to bind its UDS after
/// `ensure_service`. Generous to absorb Julia's first-call cold start
/// inside a fresh container.
const UDS_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Polling cadence while the worker comes up.
const UDS_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// DooD-backed `ServiceSpawner`. Persistent container per
/// `image_digest`; repeated `ensure_service` for the same digest is
/// idempotent.
pub struct DockerServiceSpawner {
    runtime: tokio::runtime::Runtime,
    docker: Docker,
    config: DockerSpawnerConfig,
    /// `image_digest.as_str()` → live service state.
    services: Mutex<HashMap<String, ServiceState>>,
}

struct ServiceState {
    /// Bollard container ID.
    container_id: String,
    /// UDS path (under the per-service tempdir; bind-mounted into the
    /// container at the same path per DooD discipline D26 §9.5).
    uds_path: PathBuf,
    /// Per-service tempdir on the host; cleaned up on drain.
    tempdir: PathBuf,
}

impl DockerServiceSpawner {
    /// Construct, verifying DooD discipline and connecting to the
    /// daemon. Same fail-fast posture as
    /// [`crate::spawner::DockerSpawner::new`].
    pub fn new(config: DockerSpawnerConfig) -> Result<Self, SpawnError> {
        depot::verify_depot_path(&config.depot_path)?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: format!("could not build Tokio runtime: {e}"),
            })?;

        let socket = config.resolved_docker_socket();
        let docker = runtime
            .block_on(async {
                Docker::connect_with_unix(
                    socket.to_string_lossy().as_ref(),
                    120,
                    bollard::API_DEFAULT_VERSION,
                )
            })
            .map_err(|e| SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: format!("connect to Docker daemon at {}: {e}", socket.display()),
            })?;

        Ok(Self {
            runtime,
            docker,
            config,
            services: Mutex::new(HashMap::new()),
        })
    }
}

impl ServiceSpawner for DockerServiceSpawner {
    fn ensure_service(&self, spec: WorkerSpec) -> Result<ServiceHandle, SpawnError> {
        let digest = spec
            .image_digest
            .clone()
            .ok_or_else(|| SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: "DockerServiceSpawner requires WorkerSpec::image_digest to be Some(_)"
                    .into(),
            })?;
        let key = digest.as_str().to_string();

        // Fast path: existing service for this digest? Return its handle.
        {
            let services = self.services.lock().expect("services mutex poisoned");
            if services.contains_key(&key) {
                return Ok(ServiceHandle::new(
                    BACKEND,
                    key.clone(),
                    Some(digest.clone()),
                ));
            }
        }

        // Slow path: spawn a fresh container. The caller's
        // `WorkerSpec::tempdir_host_path` is the service tempdir —
        // the spawner does not invent its own. This keeps two
        // invariants in agreement: (1) the bind-mount maps the
        // caller's tempdir into the container at the same host path
        // (DooD discipline, D26 §9.5), and (2) `EIGENIUS_TEST_WORKER_UDS`
        // (set by the caller in `spec.env`) points inside that tempdir.
        // If they disagreed, the worker would bind the UDS where the
        // bind-mount can't reach it and `attach_uds` would time out.
        let tempdir = spec.tempdir_host_path.clone();
        if tempdir.as_os_str().is_empty() {
            return Err(SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: "WorkerSpec.tempdir_host_path must be set for DockerServiceSpawner — \
                         the caller is responsible for choosing the per-service tempdir so it \
                         stays short enough for SUN_LEN (108 bytes on Linux)"
                    .to_string(),
            });
        }
        std::fs::create_dir_all(&tempdir).map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("create service tempdir {}: {e}", tempdir.display()),
        })?;
        depot::verify_tempdir_under_depot(&tempdir, &self.config.depot_path)?;
        let uds_path = tempdir.join("worker.sock");
        // Stale UDS files from a previous service in the same tempdir
        // would block the worker's `bind` — clean up best-effort.
        let _ = std::fs::remove_file(&uds_path);

        let mut svc_spec = spec.clone();
        // Worker reads UDS path from this env var. Caller likely
        // already set it via `prepare_substrate_side` + their own
        // tempdir+`/worker.sock`; if not, fill in from this spawner's
        // view of the same path.
        svc_spec
            .env
            .entry("EIGENIUS_TEST_WORKER_UDS".into())
            .or_insert_with(|| uds_path.to_string_lossy().into_owned());

        let depot = self.config.depot_path.clone();
        let pull_policy = self.config.pull_policy;
        let network_mode = self.config.default_network_mode.clone();

        let container_id = self.runtime.block_on(async {
            lifecycle::pull_image_if_needed(&self.docker, &digest, pull_policy).await?;
            let plan = container::build_create_options(&container::ContainerBuildInputs {
                spec: &svc_spec,
                tempdir: &tempdir,
                depot: &depot,
                network_mode: &network_mode,
                auto_remove: false, // Service mode: container persists until drain.
            })?;
            let id = lifecycle::create_container(&self.docker, plan).await?;
            lifecycle::start_container(&self.docker, &id).await?;
            Ok::<_, SpawnError>(id)
        })?;

        let mut services = self.services.lock().expect("services mutex poisoned");
        // Race: another caller may have raced us to ensure_service. If
        // the bucket is already populated we lost — clean up our
        // container and use the winner's.
        if let Some(existing) = services.get(&key) {
            let lost = existing.container_id.clone();
            drop(services);
            let _ = self
                .runtime
                .block_on(lifecycle::remove_container(&self.docker, &container_id));
            // Best-effort tempdir cleanup; the winner has its own.
            let _ = std::fs::remove_dir_all(&tempdir);
            // Reacquire to read the winner.
            let services = self.services.lock().expect("services mutex poisoned");
            let _ = services.get(&key).expect("winner present");
            // Fall through to the handle return; tempdir we created is
            // gone, but the winner's is alive.
            let _ = lost; // silence
        } else {
            services.insert(
                key.clone(),
                ServiceState {
                    container_id,
                    uds_path,
                    tempdir,
                },
            );
        }

        Ok(ServiceHandle::new(BACKEND, key, Some(digest)))
    }

    fn attach_uds(&self, service: &ServiceHandle) -> Result<UnixStream, SpawnError> {
        let uds_path = {
            let services = self.services.lock().expect("services mutex poisoned");
            services
                .get(service.id())
                .ok_or_else(|| SpawnError::SpawnFailed {
                    backend: BACKEND,
                    reason: format!("unknown service id `{}` for attach_uds", service.id()),
                })?
                .uds_path
                .clone()
        };
        wait_for_uds(&uds_path, UDS_READY_TIMEOUT, UDS_POLL_INTERVAL)
    }

    fn drain(&self, service: &ServiceHandle) -> Result<(), SpawnError> {
        let state = {
            let mut services = self.services.lock().expect("services mutex poisoned");
            services.remove(service.id())
        };
        let Some(state) = state else {
            // Already drained or never existed — idempotent no-op.
            return Ok(());
        };

        // Best-effort: send Evict via UDS so the worker exits cleanly.
        // Fall through to docker stop/remove regardless of RPC outcome.
        let _ = (|| -> Result<(), Box<dyn std::error::Error>> {
            use crate::rpc::client::WorkerRpcClient;
            use crate::rpc::protocol::Request;
            let stream = UnixStream::connect(&state.uds_path)?;
            let mut client = WorkerRpcClient::new(stream);
            let _ = client.call(&Request::Evict)?;
            Ok(())
        })();

        let container_id = state.container_id.clone();
        self.runtime.block_on(async {
            // Best-effort kill (idempotent: kill on a stopped container
            // returns 409, which we ignore).
            let _ = lifecycle::kill_container(&self.docker, &container_id).await;
            lifecycle::remove_container(&self.docker, &container_id).await
        })?;
        let _ = std::fs::remove_file(&state.uds_path);
        let _ = std::fs::remove_dir_all(&state.tempdir);
        Ok(())
    }

    fn backend(&self) -> &'static str {
        BACKEND
    }
}

/// Block until `uds_path` is connectable, or `timeout` expires. Open
/// connections are immediately closed — this only confirms the worker
/// is bound and accepting.
fn wait_for_uds(
    uds_path: &Path,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<UnixStream, SpawnError> {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(uds_path) {
            Ok(s) => return Ok(s),
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                return Err(SpawnError::SpawnFailed {
                    backend: BACKEND,
                    reason: format!(
                        "wait_for_uds: connect to {} timed out after {timeout:?}: {e}",
                        uds_path.display()
                    ),
                });
            }
        }
    }
}
