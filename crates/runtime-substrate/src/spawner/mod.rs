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

//! Worker spawning — the substrate's container/process-lifecycle seam.
//!
//! The [`WorkerSpawner`] trait abstracts the question "how do I get a
//! worker process running against this `WorkerSpec`?" so that the rest
//! of the substrate doesn't bind to any specific backend (Docker daemon
//! via Bollard, podman, raw subprocess, future k8s). D26 §8.2.
//!
//! Backends planned:
//!
//! - [`LocalSpawner`] — host subprocess, no container. Used for dev,
//!   CI, and the substrate's smoke tests. Reduced sandbox (process-level
//!   only, no namespacing).
//! - [`DockerSpawner`] — Bollard + Docker-outside-of-Docker (D26 §9.5).
//!   Production default on Linux. Phase 18a ships a stub that errors
//!   at construction; the real implementation lands in 18c.
//! - `PodmanSpawner` — deferred. Same trait, rootless-friendly.
//! - k8s-aware backend — deferred.
//!
//! ## Pool layering
//!
//! v1 ships spawn-per-invocation (D26 §8.1). When warm-worker reuse
//! becomes worth the complexity (Phase 19c, Julia's cold-start cost),
//! a `WorkerPool` will wrap a `dyn WorkerSpawner` and cache handles by
//! `image_digest`. The pool sits *above* this trait, not inside it —
//! spawner backends stay ignorant of pooling.

#[cfg(feature = "docker-spawner")]
pub mod docker;
pub mod local;
pub mod service;

#[cfg(feature = "docker-spawner")]
pub use docker::{DockerSpawner, DockerSpawnerConfig, NetworkMode, PullPolicy};
pub use local::LocalSpawner;
#[cfg(feature = "docker-spawner")]
pub use service::DockerServiceSpawner;
pub use service::{LocalServiceSpawner, ServiceHandle, ServiceSpawner};

use crate::error::SpawnError;
use crate::types::{WorkerHandle, WorkerSpec};
use std::os::unix::net::UnixStream;
use std::process::ExitStatus;
use std::time::Duration;

/// Backend abstraction for the substrate's worker lifecycle.
///
/// Implementors run a worker against a [`WorkerSpec`] and return an
/// opaque [`WorkerHandle`]. The substrate then communicates with the
/// worker via the CBOR RPC protocol over the worker's UDS (resolved by
/// [`WorkerSpawner::attach_uds`]).
///
/// All four methods are spawner-agnostic: a `WorkerHandle` produced by
/// `LocalSpawner::spawn` is valid input to `LocalSpawner::wait` /
/// `kill` / `attach_uds` only — handles do not migrate across backends.
/// Each backend owns its own bookkeeping (process tables for
/// `LocalSpawner`, container IDs for `DockerSpawner`).
pub trait WorkerSpawner: Send + Sync {
    /// Spawn a worker process matching the spec and return its handle.
    ///
    /// The substrate guarantees `spec.tempdir_host_path` exists on disk
    /// before calling. The backend chooses how to bind it into the
    /// worker's filesystem view (Docker bind-mount, direct host path
    /// for `LocalSpawner`).
    fn spawn(&self, spec: WorkerSpec) -> Result<WorkerHandle, SpawnError>;

    /// Block until the worker exits, returning its exit status. Bounded
    /// by `timeout` if `Some` — on expiry the spawner kills the worker
    /// and returns [`SpawnError::WaitTimedOut`]. `None` means wait
    /// indefinitely (existing behaviour for callers that do not enforce
    /// a wall-clock cap; tests, `Evict` shutdown, etc.).
    ///
    /// Implementors must guarantee that on the timeout path the worker
    /// is reaped before the call returns — callers depend on
    /// "WaitTimedOut implies the process is gone" for tempdir cleanup
    /// and `auto_remove` accounting (D26 §8.3).
    fn wait_with_timeout(
        &self,
        handle: &WorkerHandle,
        timeout: Option<Duration>,
    ) -> Result<ExitStatus, SpawnError>;

    /// Send a termination signal (`SIGTERM` then `SIGKILL` after a
    /// short grace period for `LocalSpawner`; `docker stop` for
    /// `DockerSpawner`).
    fn kill(&self, handle: &WorkerHandle) -> Result<(), SpawnError>;

    /// Open a CBOR RPC channel to the worker via its UDS.
    /// `handle.uds_path` is resolved against the host depot path so the
    /// orchestrator and the worker see the same path under DooD
    /// discipline (D26 §9.5).
    fn attach_uds(&self, handle: &WorkerHandle) -> Result<UnixStream, SpawnError>;

    /// Backend identifier — `"local"`, `"docker"`, etc. Used for
    /// telemetry and to populate `WorkerHandle::backend`.
    fn backend(&self) -> &'static str;
}
