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

//! `LocalServiceSpawner` — host-subprocess service backend.
//!
//! For dev / CI / smoke tests where Docker is unavailable or
//! unwarranted. Spawns the worker as a long-lived child process; the
//! worker's UDS lives directly on the host filesystem (no DooD bind
//! mounts).
//!
//! Sandbox guarantees are weaker than `DockerServiceSpawner` (process-level
//! isolation only, no namespacing). The substrate is provenance + dispatch
//! for trusted toolchains, not adversarial containment (D26 §1.2), so this
//! is acceptable for dev/CI usage. Production Service envs deploy via
//! Azure Container Apps or Kubernetes (deferred); those backends will
//! implement the same `ServiceSpawner` trait.

use crate::error::SpawnError;
use crate::rpc::client::WorkerRpcClient;
use crate::rpc::protocol::Request;
use crate::spawner::service::{ServiceHandle, ServiceSpawner};
use crate::types::WorkerSpec;
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BACKEND: &str = "local-service";

/// How long `attach_uds` waits for the worker's UDS to appear after
/// spawn. Long-lived workers go through their cold-start path (e.g.
/// Julia precompile) the first time, so a generous deadline.
const UDS_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

static SERVICE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Local-subprocess `ServiceSpawner`. Each `ensure_service` call
/// spawns a child process that owns its UDS and stays alive until
/// `drain`. Concurrent dispatches against one service share the
/// underlying child — the worker accepts multiple sequential
/// connections (see `JuliaWorker.jl`'s accept loop). This is
/// sufficient for dev usage; production scaling is the platform's
/// concern.
pub struct LocalServiceSpawner {
    /// Depot directory the worker tempdirs live under.
    #[allow(dead_code)]
    depot_path: PathBuf,
    /// Live services keyed by `ServiceHandle.id()`.
    services: Mutex<HashMap<String, ServiceState>>,
    /// Idempotence index: `(image_digest, command)` → service id. Honours the
    /// trait contract that repeated `ensure_service` calls for the same
    /// effective worker identity return the same `ServiceHandle`. The
    /// command is part of the key because Local has no image — two
    /// invocations with different binaries are different services even
    /// if they share an image_digest stub.
    by_identity: Mutex<HashMap<ServiceIdentity, String>>,
}

/// Cache key for [`LocalServiceSpawner`]'s idempotence map.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ServiceIdentity {
    image_digest: Option<String>,
    command: Vec<String>,
}

struct ServiceState {
    /// The subprocess hosting the worker.
    child: Child,
    /// UDS path the worker is listening on.
    uds_path: PathBuf,
    /// Tempdir the worker is running in.
    tempdir: PathBuf,
    /// Cached handle returned by `ensure_service`; reused on
    /// idempotent re-calls so callers compare equal.
    handle: ServiceHandle,
    /// Identity that keyed this service in `by_identity` — used at
    /// `drain` time to evict the cache entry alongside the
    /// `services` map.
    identity: ServiceIdentity,
}

impl LocalServiceSpawner {
    /// Construct with the depot directory worker tempdirs are placed
    /// under. The directory must exist; the spawner does not create
    /// it.
    pub fn new(depot_path: PathBuf) -> Self {
        Self {
            depot_path,
            services: Mutex::new(HashMap::new()),
            by_identity: Mutex::new(HashMap::new()),
        }
    }

    fn next_service_id(&self) -> String {
        let n = SERVICE_COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("local-svc-{}-{n}", std::process::id())
    }
}

impl ServiceSpawner for LocalServiceSpawner {
    fn ensure_service(&self, spec: WorkerSpec) -> Result<ServiceHandle, SpawnError> {
        if spec.command.is_empty() {
            return Err(SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: "WorkerSpec.command must be non-empty for LocalServiceSpawner \
                         (no image CMD to fall back to)"
                    .into(),
            });
        }

        let identity = ServiceIdentity {
            image_digest: spec.image_digest.as_ref().map(|d| d.as_str().to_string()),
            command: spec.command.clone(),
        };

        // Fast path: existing service for this identity? Return the
        // cached handle so callers compare equal across calls.
        {
            let by_identity = self.by_identity.lock().expect("by_identity mutex poisoned");
            if let Some(existing_id) = by_identity.get(&identity) {
                let services = self.services.lock().expect("services mutex poisoned");
                if let Some(state) = services.get(existing_id) {
                    return Ok(state.handle.clone());
                }
            }
        }

        let id = self.next_service_id();
        let tempdir = spec.tempdir_host_path.clone();
        std::fs::create_dir_all(&tempdir).map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("create service tempdir {}: {e}", tempdir.display()),
        })?;
        let uds_path = tempdir.join("worker.sock");
        let _ = std::fs::remove_file(&uds_path);

        let mut env = spec.env.clone();
        env.entry("EIGENIUS_TEST_WORKER_UDS".into())
            .or_insert_with(|| uds_path.to_string_lossy().into_owned());

        let mut cmd = std::process::Command::new(&spec.command[0]);
        cmd.args(&spec.command[1..])
            .envs(&env)
            .current_dir(&tempdir);

        let child = cmd.spawn().map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("spawn {}: {e}", spec.command[0]),
        })?;

        let handle = ServiceHandle::new(BACKEND, id.clone(), spec.image_digest.clone());
        let state = ServiceState {
            child,
            uds_path,
            tempdir,
            handle: handle.clone(),
            identity: identity.clone(),
        };

        let mut services = self.services.lock().expect("services mutex poisoned");
        services.insert(id.clone(), state);
        self.by_identity
            .lock()
            .expect("by_identity mutex poisoned")
            .insert(identity, id);

        Ok(handle)
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
        connect_with_retry(&uds_path, UDS_CONNECT_TIMEOUT).map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("attach_uds connect to {}: {e}", uds_path.display()),
        })
    }

    fn drain(&self, service: &ServiceHandle) -> Result<(), SpawnError> {
        let mut services = self.services.lock().expect("services mutex poisoned");
        let mut state = services
            .remove(service.id())
            .ok_or_else(|| SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: format!("unknown service id `{}` for drain", service.id()),
            })?;
        self.by_identity
            .lock()
            .expect("by_identity mutex poisoned")
            .remove(&state.identity);

        // Best-effort graceful shutdown: send Evict via RPC, then
        // wait briefly for the child to exit cleanly. Fall back to
        // SIGKILL on timeout.
        let _ = (|| -> Result<(), Box<dyn std::error::Error>> {
            let stream = UnixStream::connect(&state.uds_path)?;
            let mut client = WorkerRpcClient::new(stream);
            let _ = client.call(&Request::Evict)?;
            Ok(())
        })();

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match state.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => {
                    return Err(SpawnError::SpawnFailed {
                        backend: BACKEND,
                        reason: format!("try_wait on drain: {e}"),
                    });
                }
            }
        }
        if let Ok(None) = state.child.try_wait() {
            let _ = state.child.kill();
        }
        let _ = state.child.wait();
        let _ = std::fs::remove_file(&state.uds_path);
        let _ = std::fs::remove_dir_all(&state.tempdir);
        Ok(())
    }

    fn backend(&self) -> &'static str {
        BACKEND
    }
}

fn connect_with_retry(
    uds_path: &std::path::Path,
    timeout: Duration,
) -> std::io::Result<UnixStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(uds_path) {
            Ok(s) => return Ok(s),
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }
}
