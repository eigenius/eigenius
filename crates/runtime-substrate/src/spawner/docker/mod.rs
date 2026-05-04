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

//! `DockerSpawner` — Bollard-backed [`crate::spawner::WorkerSpawner`]
//! implementation. Production default on Linux deployments.
//!
//! Phase 18c.3 milestone (D26 §8.2 / §9.5):
//!
//! - Talks to the host Docker daemon via the Unix socket
//!   ([`config::DEFAULT_DOCKER_SOCKET`]). DooD: spawns sibling
//!   containers, never nested.
//! - Enforces the depot bind-mount discipline (D26 §9.5) at
//!   construction; refuses to come up if the depot path doesn't satisfy
//!   it ([`crate::error::SpawnError::DepotMountViolation`]).
//! - Spawn-per-invocation Job model: `auto_remove: true`,
//!   `network_mode: none`, `cap_drop: ALL`. Custom seccomp / AppArmor
//!   hardening lands with 18c.4.
//! - Detects worker-bootstrap cross-check failures (D26 §9.3) at
//!   [`crate::spawner::WorkerSpawner::attach_uds`] time: if the
//!   container exits with [`crate::cross_check::EXIT_CODE_CROSS_CHECK_FAILURE`]
//!   (78) before binding its UDS, the substrate surfaces
//!   [`crate::error::SpawnError::WorkerCrossCheckFailed`].
//!
//! Internally a sync trait-impl `block_on`s a Tokio runtime owned by
//! the spawner. Callers see plain blocking calls; the runtime is an
//! implementation detail. Bollard requires a Tokio runtime — this is
//! the simplest way to integrate with the otherwise-sync substrate.

pub mod config;
pub mod container;
pub mod depot;
pub mod lifecycle;

pub use config::{DockerSpawnerConfig, NetworkMode, PullPolicy, DEFAULT_DOCKER_SOCKET};

use crate::error::SpawnError;
use crate::spawner::WorkerSpawner;
use crate::types::{WorkerHandle, WorkerSpec};
use bollard::Docker;
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) const BACKEND: &str = "docker";

/// Labels-version string stamped on every container the substrate
/// spawns. Bumped (manually) when the substrate's labelling convention
/// changes; lets ops tools tell stale-by-version containers apart.
pub(crate) const SUBSTRATE_LABEL_VERSION: &str = "1";

/// How long [`DockerSpawner::attach_uds`] waits for the worker to bind
/// its UDS before either returning the cross-check verdict (if the
/// container has exited 78) or surfacing a generic `SpawnFailed`.
const UDS_READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Polling cadence for [`DockerSpawner::attach_uds`].
const UDS_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Bollard-backed [`crate::spawner::WorkerSpawner`].
///
/// Owns a Tokio runtime so the surrounding substrate (which is sync)
/// can drive Bollard's async API. The runtime has a single worker
/// thread — substrate dispatch is mostly serial per invocation, and
/// pulls / waits are I/O-bound rather than CPU-bound.
pub struct DockerSpawner {
    runtime: tokio::runtime::Runtime,
    docker: Docker,
    config: DockerSpawnerConfig,
    /// container_id → tempdir to clean up on `wait` / `kill`. The
    /// tempdir is created by the substrate (under the depot) and owned
    /// for the lifetime of the spawn.
    bookkeeping: Mutex<HashMap<String, PathBuf>>,
}

impl std::fmt::Debug for DockerSpawner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerSpawner")
            .field("config", &self.config)
            .field(
                "bookkeeping_len",
                &self.bookkeeping.lock().map(|m| m.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl DockerSpawner {
    /// Construct, verifying DooD discipline and connecting to the
    /// daemon. Refuses to come up if either fails — there is no point
    /// queueing a worker against a misconfigured environment, and the
    /// cost of failing fast at orchestrator startup is orders of
    /// magnitude lower than failing per-invocation.
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

        // D26 §9.5 security-posture acknowledgement. One line, every
        // construction — deployment doc + this log are the only places
        // operators are reminded.
        eprintln!(
            "eigenius-runtime-substrate: DockerSpawner active. \
             {} access is root-equivalent on host (D26 §9.5). \
             The orchestrator host is the substrate's security boundary — \
             no untrusted RPC surfaces.",
            socket.display()
        );

        Ok(Self {
            runtime,
            docker,
            config,
            bookkeeping: Mutex::new(HashMap::new()),
        })
    }

    fn record(&self, container_id: String, tempdir: PathBuf) {
        if let Ok(mut g) = self.bookkeeping.lock() {
            g.insert(container_id, tempdir);
        }
    }

    fn forget(&self, container_id: &str) -> Option<PathBuf> {
        self.bookkeeping
            .lock()
            .ok()
            .and_then(|mut g| g.remove(container_id))
    }
}

impl WorkerSpawner for DockerSpawner {
    fn spawn(&self, spec: WorkerSpec) -> Result<WorkerHandle, SpawnError> {
        let digest = spec
            .image_digest
            .clone()
            .ok_or_else(|| SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: "DockerSpawner requires WorkerSpec::image_digest to be Some(_)".into(),
            })?;
        depot::verify_tempdir_under_depot(&spec.tempdir_host_path, &self.config.depot_path)?;
        let tempdir = spec.tempdir_host_path.clone();
        let uds_path = tempdir.join("worker.sock");
        let depot = self.config.depot_path.clone();
        let pull_policy = self.config.pull_policy;
        let network_mode = self.config.default_network_mode.clone();

        let container_id = self.runtime.block_on(async {
            lifecycle::pull_image_if_needed(&self.docker, &digest, pull_policy).await?;
            let plan = container::build_create_options(&container::ContainerBuildInputs {
                spec: &spec,
                tempdir: &tempdir,
                depot: &depot,
                network_mode: &network_mode,
                auto_remove: true,
            })?;
            let id = lifecycle::create_container(&self.docker, plan).await?;
            lifecycle::start_container(&self.docker, &id).await?;
            Ok::<_, SpawnError>(id)
        })?;

        self.record(container_id.clone(), tempdir);
        Ok(WorkerHandle {
            id: container_id,
            uds_path,
            backend: BACKEND,
        })
    }

    fn wait_with_timeout(
        &self,
        handle: &WorkerHandle,
        timeout: Option<Duration>,
    ) -> Result<ExitStatus, SpawnError> {
        let result = self.runtime.block_on(async {
            match timeout {
                None => lifecycle::wait_container(&self.docker, &handle.id)
                    .await
                    .map(WaitOutcome::Exited),
                Some(t) => match tokio::time::timeout(
                    t,
                    lifecycle::wait_container(&self.docker, &handle.id),
                )
                .await
                {
                    Ok(r) => r.map(WaitOutcome::Exited),
                    Err(_elapsed) => {
                        // Wall-clock cap reached. Kill the container so
                        // the contract "WaitTimedOut implies the worker
                        // is gone" holds, then reap exit (best-effort —
                        // auto_remove may have already taken it).
                        let _ = lifecycle::kill_container(&self.docker, &handle.id).await;
                        let _ = lifecycle::wait_container(&self.docker, &handle.id).await;
                        Ok(WaitOutcome::TimedOut)
                    }
                },
            }
        })?;
        // Reap bookkeeping; tempdir cleanup is the caller's choice (the
        // substrate's per-invocation contract is that tempdir contents
        // are inputs to the `RuntimeInvocation` resource, so the
        // dispatcher decides when to delete).
        let _ = self.forget(&handle.id);
        match result {
            WaitOutcome::Exited(code) => Ok(exit_code_to_status(code)),
            WaitOutcome::TimedOut => Err(SpawnError::WaitTimedOut {
                handle_id: handle.id.clone(),
                timeout_ms: timeout.map(|t| t.as_millis() as u64).unwrap_or_default(),
            }),
        }
    }

    fn kill(&self, handle: &WorkerHandle) -> Result<(), SpawnError> {
        self.runtime
            .block_on(lifecycle::kill_container(&self.docker, &handle.id))?;
        // Best-effort wait so the container fully exits before we lose
        // bookkeeping. `auto_remove` will clean it up on the daemon
        // side; we don't surface its exit code from `kill`. Bounded
        // tightly because `kill_container` already sent SIGKILL — if
        // we're not reaped within a few seconds, something is deeply
        // wrong on the daemon side and falling through is the right
        // behavior (the WorkerHandle is gone from bookkeeping either
        // way).
        let _ = self.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(5),
                lifecycle::wait_container(&self.docker, &handle.id),
            )
            .await
        });
        let _ = self.forget(&handle.id);
        Ok(())
    }

    fn attach_uds(&self, handle: &WorkerHandle) -> Result<UnixStream, SpawnError> {
        let deadline = Instant::now() + UDS_READY_TIMEOUT;
        loop {
            if let Ok(s) = UnixStream::connect(&handle.uds_path) {
                return Ok(s);
            }
            // Worker hasn't bound the UDS yet. Check whether it's still
            // alive — if it has exited 78, surface the cross-check
            // failure unambiguously. Other exit codes fold into
            // SpawnFailed with the code in the diagnostic.
            let observation = self
                .runtime
                .block_on(lifecycle::inspect_container(&self.docker, &handle.id))?;
            if !observation.running {
                if let Some(code) = observation.exit_code {
                    return Err(lifecycle::classify_exit_code(&handle.id, code));
                }
                // Container is gone (auto-removed) but we never read an
                // exit code — `wait` would have captured it. Fall back
                // to a generic message.
                return Err(SpawnError::SpawnFailed {
                    backend: BACKEND,
                    reason: format!(
                        "container {} exited before binding its UDS (auto-removed; \
                         exit code unrecoverable — call wait() before attach_uds() \
                         to capture it)",
                        handle.id,
                    ),
                });
            }
            if Instant::now() > deadline {
                return Err(SpawnError::SpawnFailed {
                    backend: BACKEND,
                    reason: format!(
                        "container {} did not bind its UDS at {} within {:?}",
                        handle.id,
                        handle.uds_path.display(),
                        UDS_READY_TIMEOUT,
                    ),
                });
            }
            std::thread::sleep(UDS_POLL_INTERVAL);
        }
    }

    fn backend(&self) -> &'static str {
        BACKEND
    }
}

/// Outcome of the inner async wait — exit-with-code or hit-the-wall-clock-cap.
enum WaitOutcome {
    Exited(i64),
    TimedOut,
}

/// Convert a Bollard-reported exit code to a [`std::process::ExitStatus`].
/// Linux convention: shift the exit code into the high byte of the
/// wait()-style status word.
fn exit_code_to_status(code: i64) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw((code as i32) << 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_round_trip() {
        let s = exit_code_to_status(78);
        assert_eq!(s.code(), Some(78));
        let s = exit_code_to_status(0);
        assert_eq!(s.code(), Some(0));
    }
}
