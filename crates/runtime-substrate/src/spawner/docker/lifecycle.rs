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

//! Async lifecycle helpers driving the Bollard client. The sync
//! [`super::DockerSpawner`] `block_on`s these from the
//! [`crate::spawner::WorkerSpawner`] trait methods.

use crate::cross_check::EXIT_CODE_CROSS_CHECK_FAILURE;
use crate::error::SpawnError;
use crate::spawner::docker::config::PullPolicy;
use crate::spawner::docker::BACKEND;
use crate::types::ImageDigest;
use bollard::query_parameters::{
    CreateImageOptionsBuilder, InspectContainerOptions, KillContainerOptionsBuilder,
    StartContainerOptionsBuilder, WaitContainerOptionsBuilder,
};
use bollard::Docker;
use futures_util::stream::StreamExt;

/// Apply the configured pull policy: bring the image into the local
/// daemon's cache when needed, leave it alone when not.
pub async fn pull_image_if_needed(
    docker: &Docker,
    digest: &ImageDigest,
    policy: PullPolicy,
) -> Result<(), SpawnError> {
    let present = match docker.inspect_image(digest.as_str()).await {
        Ok(_) => true,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => false,
        Err(e) => {
            return Err(SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: format!("inspect_image failed for {digest}: {e}"),
            });
        }
    };

    let should_pull = match policy {
        PullPolicy::Always => true,
        PullPolicy::IfMissing => !present,
        PullPolicy::Never => {
            if present {
                false
            } else {
                return Err(SpawnError::EnvironmentImageUnavailable {
                    digest: Some(digest.clone()),
                    reason: "PullPolicy::Never and image not in local cache".into(),
                });
            }
        }
    };

    if !should_pull {
        return Ok(());
    }
    let opts = CreateImageOptionsBuilder::new()
        .from_image(digest.as_str())
        .build();
    let mut stream = docker.create_image(Some(opts), None, None);
    while let Some(event) = stream.next().await {
        // We don't surface progress; we just need to drain the stream
        // so the pull actually runs to completion.
        if let Err(e) = event {
            return Err(SpawnError::EnvironmentImageUnavailable {
                digest: Some(digest.clone()),
                reason: format!("pull failed: {e}"),
            });
        }
    }
    Ok(())
}

/// Create a container from a built [`super::container::CreatePlan`].
pub async fn create_container(
    docker: &Docker,
    plan: super::container::CreatePlan,
) -> Result<String, SpawnError> {
    let resp = docker
        .create_container(Some(plan.options), plan.body)
        .await
        .map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("create_container failed: {e}"),
        })?;
    Ok(resp.id)
}

/// Start a previously created container.
pub async fn start_container(docker: &Docker, container_id: &str) -> Result<(), SpawnError> {
    let opts = StartContainerOptionsBuilder::new().build();
    docker
        .start_container(container_id, Some(opts))
        .await
        .map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("start_container({container_id}) failed: {e}"),
        })?;
    Ok(())
}

/// Wait for the container to reach a terminal state and return its exit
/// code. Subscribes to the wait-stream *before* the container has a
/// chance to be auto-removed; the daemon holds the result long enough
/// for an active waiter to read it.
pub async fn wait_container(docker: &Docker, container_id: &str) -> Result<i64, SpawnError> {
    let opts = WaitContainerOptionsBuilder::new().build();
    let mut stream = docker.wait_container(container_id, Some(opts));
    let mut last_exit: Option<i64> = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(state) => {
                last_exit = Some(state.status_code);
            }
            Err(bollard::errors::Error::DockerContainerWaitError { code, .. }) => {
                // Some daemon versions surface non-zero exits as an
                // error on the stream rather than as Ok(state). Treat
                // it the same way — `code` is the container's exit code.
                last_exit = Some(code);
            }
            Err(e) => {
                return Err(SpawnError::SpawnFailed {
                    backend: BACKEND,
                    reason: format!("wait_container({container_id}) failed: {e}"),
                });
            }
        }
    }
    last_exit.ok_or_else(|| SpawnError::SpawnFailed {
        backend: BACKEND,
        reason: format!("wait_container({container_id}) yielded no terminal state"),
    })
}

/// Send SIGKILL to the container. Used by [`crate::spawner::WorkerSpawner::kill`];
/// after this returns the substrate must still call `wait_container` to
/// reap the exit status.
pub async fn kill_container(docker: &Docker, container_id: &str) -> Result<(), SpawnError> {
    let opts = KillContainerOptionsBuilder::new().signal("SIGKILL").build();
    docker
        .kill_container(container_id, Some(opts))
        .await
        .map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("kill_container({container_id}) failed: {e}"),
        })?;
    Ok(())
}

/// Remove a stopped container (Service-mode drain — `auto_remove: false`
/// containers don't self-clean). `force = true` so the call also
/// terminates a still-running container; `v: true` removes anonymous
/// volumes the container created.
pub async fn remove_container(docker: &Docker, container_id: &str) -> Result<(), SpawnError> {
    let opts = bollard::query_parameters::RemoveContainerOptionsBuilder::new()
        .force(true)
        .v(true)
        .build();
    match docker.remove_container(container_id, Some(opts)).await {
        Ok(_) => Ok(()),
        // Already gone — drain is idempotent.
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => Ok(()),
        Err(e) => Err(SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("remove_container({container_id}) failed: {e}"),
        }),
    }
}

/// State of a container as observed via `inspect_container`. Only the
/// bits the substrate cares about — everything else from the inspect
/// response is discarded.
#[derive(Debug, Clone, Copy)]
pub struct ContainerObservation {
    pub running: bool,
    pub exit_code: Option<i64>,
}

pub async fn inspect_container(
    docker: &Docker,
    container_id: &str,
) -> Result<ContainerObservation, SpawnError> {
    match docker
        .inspect_container(container_id, None::<InspectContainerOptions>)
        .await
    {
        Ok(resp) => {
            let state = resp.state.unwrap_or_default();
            let running = state.running.unwrap_or(false);
            let exit_code = state.exit_code;
            Ok(ContainerObservation { running, exit_code })
        }
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            // Container is gone (auto-removed). The exit code is no
            // longer recoverable through the API, but `wait_container`
            // would have captured it earlier; for cross-check purposes
            // we treat this as "exited, code unknown".
            Ok(ContainerObservation {
                running: false,
                exit_code: None,
            })
        }
        Err(e) => Err(SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("inspect_container({container_id}) failed: {e}"),
        }),
    }
}

/// Map a container's exit code to the right [`SpawnError`] variant for
/// the "worker died before binding UDS" path. Cross-check failures get
/// their own variant per D26 §11.1; everything else is generic
/// `SpawnFailed` with the exit code in the diagnostic.
pub fn classify_exit_code(container_id: &str, exit_code: i64) -> SpawnError {
    if exit_code == EXIT_CODE_CROSS_CHECK_FAILURE as i64 {
        SpawnError::WorkerCrossCheckFailed(format!(
            "container {container_id} exited with EXIT_CODE_CROSS_CHECK_FAILURE \
             (78) before binding its UDS — manifest-hash cross-check failed"
        ))
    } else {
        SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!(
                "container {container_id} exited with code {exit_code} before binding its UDS"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_exit_code_recognises_cross_check_failure() {
        let err = classify_exit_code("abc123", EXIT_CODE_CROSS_CHECK_FAILURE as i64);
        match err {
            SpawnError::WorkerCrossCheckFailed(msg) => assert!(msg.contains("abc123")),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn classify_exit_code_other_codes_are_generic_spawn_failures() {
        let err = classify_exit_code("abc123", 1);
        match err {
            SpawnError::SpawnFailed { reason, .. } => {
                assert!(reason.contains("code 1"));
                assert!(reason.contains("abc123"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
