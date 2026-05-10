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

//! Long-lived service-style worker lifecycle.
//!
//! Per [D26 §8.2](../../../../../docs/design/d26-runtime-substrate.md):
//! `ServiceSpawner` is the second of two parallel traits — the first
//! is [`crate::spawner::WorkerSpawner`] (the Job role; spawn → exit).
//! Service backends keep workers alive across many dispatches.
//!
//! **Pooling deferred.** Production-target backends — Azure Container
//! Apps, Kubernetes — handle scaling, max-replica enforcement, idle
//! eviction, and liveness/readiness probing at the platform level
//! (HPA / KEDA / ACA scale rules). Substrate-side pooling would
//! duplicate and potentially conflict with the platform's decisions.
//! Local subprocess and Docker backends are dev-only; their concurrent
//! dispatch story is "one long-lived worker per env, dispatches share
//! it" — sufficient for dev usage without a pool layer.
//!
//! The trait surface is therefore minimal:
//!
//! - `ensure_service` is idempotent — repeated calls for the same
//!   `(env_iri, image_digest)` return the same `ServiceHandle`.
//! - `attach_uds` opens a CBOR RPC channel to the long-lived worker
//!   for one dispatch. The connection is short-lived; the *worker* is
//!   long-lived. Future K8s / ACA backends will need a non-UDS
//!   variant (`attach_endpoint`?) but the shape is the same.
//! - `drain` tears down the service. Used at orchestrator shutdown
//!   and env retirement.
//! - `backend` identifies the spawner for telemetry.

#[cfg(feature = "docker-spawner")]
pub mod docker;
pub mod local;

#[cfg(feature = "docker-spawner")]
pub use docker::DockerServiceSpawner;
pub use local::LocalServiceSpawner;

use crate::error::SpawnError;
use crate::types::{ImageDigest, WorkerSpec};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

/// Service identity — what `ensure_service` returns and what
/// `lease_worker` / `release_worker` / `drain` are scoped to.
///
/// Two `ServiceHandle`s are equal iff they refer to the same backing
/// service. The handle is `Arc`-shared so callers (the pool, the
/// substrate facade) can hold long-lived references without coupling
/// the backend's internal state shape.
#[derive(Debug, Clone)]
pub struct ServiceHandle {
    inner: Arc<ServiceHandleInner>,
}

#[derive(Debug)]
struct ServiceHandleInner {
    /// Backend identifier (`"local"`, `"docker"`, ...) — populated
    /// from `ServiceSpawner::backend()` for telemetry / audit.
    backend: &'static str,
    /// Stable identity for the service. Backends choose the shape:
    /// for `LocalServiceSpawner` this is a synthetic string;
    /// for `DockerServiceSpawner` this is the container ID.
    id: String,
    /// The image the service runs. `None` for `LocalServiceSpawner`
    /// (no image). Used by the pool to key services by env image
    /// digest so the same env always lands on the same warm pool.
    image_digest: Option<ImageDigest>,
}

impl ServiceHandle {
    /// Construct a handle. Used by spawner backends; not part of the
    /// public substrate surface.
    pub(crate) fn new(
        backend: &'static str,
        id: String,
        image_digest: Option<ImageDigest>,
    ) -> Self {
        Self {
            inner: Arc::new(ServiceHandleInner {
                backend,
                id,
                image_digest,
            }),
        }
    }

    pub fn backend(&self) -> &'static str {
        self.inner.backend
    }

    pub fn id(&self) -> &str {
        &self.inner.id
    }

    pub fn image_digest(&self) -> Option<&ImageDigest> {
        self.inner.image_digest.as_ref()
    }
}

/// Long-lived service backend abstraction.
///
/// Backends own the container / process lifecycle. Concurrent
/// dispatch against a single service is the worker's concern (the
/// worker accepts multiple connections), or the platform's (k8s,
/// ACA) — not the substrate's.
pub trait ServiceSpawner: Send + Sync {
    /// Get-or-start the service backing `spec`. Idempotent: repeated
    /// calls for the same `image_digest` (or, for backends that key
    /// on other identity, the same `(env_iri, image_digest)` pair)
    /// return the same `ServiceHandle`.
    fn ensure_service(&self, spec: WorkerSpec) -> Result<ServiceHandle, SpawnError>;

    /// Open a CBOR RPC channel to the service for one dispatch. The
    /// connection is short-lived (per dispatch); the *worker* is
    /// long-lived (across dispatches, until `drain`).
    ///
    /// Future K8s / ACA backends route through the platform's
    /// service endpoint rather than a UDS path; that variant will
    /// add a parallel `attach_endpoint` (or generalise the return
    /// type) — explicitly out of v1 scope.
    fn attach_uds(&self, service: &ServiceHandle) -> Result<UnixStream, SpawnError>;

    /// Graceful tear-down of the service. Used at orchestrator
    /// shutdown and env retirement.
    fn drain(&self, service: &ServiceHandle) -> Result<(), SpawnError>;

    /// Backend identifier — `"local"`, `"docker"`, etc. Used for
    /// telemetry and to populate `ServiceHandle::backend()`.
    fn backend(&self) -> &'static str;
}
