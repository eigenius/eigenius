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

//! `DockerSpawner` — Bollard + Docker-outside-of-Docker.
//!
//! Phase 18a ships a stub that errors at construction. The real impl
//! lands in 18c per [D26](../../../../docs/design/d26-runtime-substrate.md)
//! §8.2 / §9.5: Bollard against `/var/run/docker.sock`, sibling
//! containers (DooD), per-invocation tempdir bind-mounted from the
//! well-known host depot path, custom seccomp profile, capability drop
//! to minimum, `auto_remove: true`.
//!
//! The trait shape is in place from day one (see [`crate::spawner`]) so
//! the seam is correct from the start.

use super::WorkerSpawner;
use crate::error::SpawnError;
use crate::types::{WorkerHandle, WorkerSpec};
use std::os::unix::net::UnixStream;
use std::process::ExitStatus;

const BACKEND: &str = "docker";

/// Stub for Phase 18a. Construction fails until Phase 18c implements
/// the Bollard-backed sibling-container backend.
#[derive(Debug)]
pub struct DockerSpawner {
    _phantom: (),
}

impl DockerSpawner {
    pub fn new() -> Result<Self, SpawnError> {
        Err(SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: "DockerSpawner is not yet implemented (lands in Phase 18c)".to_string(),
        })
    }
}

impl WorkerSpawner for DockerSpawner {
    fn spawn(&self, _spec: WorkerSpec) -> Result<WorkerHandle, SpawnError> {
        unimplemented!("DockerSpawner::spawn lands in Phase 18c")
    }

    fn wait(&self, _handle: &WorkerHandle) -> Result<ExitStatus, SpawnError> {
        unimplemented!("DockerSpawner::wait lands in Phase 18c")
    }

    fn kill(&self, _handle: &WorkerHandle) -> Result<(), SpawnError> {
        unimplemented!("DockerSpawner::kill lands in Phase 18c")
    }

    fn attach_uds(&self, _handle: &WorkerHandle) -> Result<UnixStream, SpawnError> {
        unimplemented!("DockerSpawner::attach_uds lands in Phase 18c")
    }

    fn backend(&self) -> &'static str {
        BACKEND
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_errors_with_phase_18c_message() {
        let err = DockerSpawner::new().expect_err("should not construct in 18a");
        match err {
            SpawnError::SpawnFailed { backend, reason } => {
                assert_eq!(backend, BACKEND);
                assert!(reason.contains("Phase 18c"));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}
