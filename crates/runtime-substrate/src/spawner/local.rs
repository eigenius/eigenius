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

//! `LocalSpawner` — host subprocess backend, no container.
//!
//! Used for dev, CI, and the substrate's smoke tests. Reduced sandbox:
//! per-invocation tempdir + the env vars from `WorkerSpec`, but no
//! namespacing, cgroups, or seccomp filtering — those require
//! `DockerSpawner` (Phase 18c) or another container backend. The
//! substrate emits a one-line warning at every dispatch under
//! `LocalSpawner` (D26 §8.3) so it cannot silently substitute for a
//! production backend.
//!
//! Resource caps in `WorkerSpec` (`max_wall_time_ms`, `max_memory_bytes`)
//! are not enforced in v1 — wiring `setrlimit` is a 18c concern that
//! lands alongside the proper sandbox.

use super::WorkerSpawner;
use crate::error::SpawnError;
use crate::types::{WorkerHandle, WorkerSpec};
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, ExitStatus};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

const BACKEND: &str = "local";

/// Polling cadence for [`LocalSpawner::wait_with_timeout`] when a
/// timeout is set. Tighter cadence catches the worker exit faster but
/// burns CPU; looser cadence delays cleanup. 50ms matches
/// `DockerSpawner::attach_uds`.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Emit the LocalSpawner security-posture warning once per process.
/// D26 §8.3 — "the orchestrator emits a one-line warning at every
/// dispatch under `LocalSpawner` so it cannot be silently used in
/// production." Once-per-process is the right cadence: enough to alert
/// an operator on first dispatch, no log spam thereafter.
fn warn_local_spawner_once() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        eprintln!(
            "eigenius-runtime-substrate: LocalSpawner active — no namespacing, \
             no seccomp, no capability drop, no resource isolation beyond \
             per-invocation tempdir + env. Dev/CI only. Production must use \
             DockerSpawner (D26 §8.3)."
        );
    });
}

/// Host-subprocess implementation of [`WorkerSpawner`].
///
/// Bookkeeping: spawned [`Child`] handles are stored in a process table
/// keyed by PID-as-string so that `wait` / `kill` can re-find them. The
/// table is the only state the spawner carries — handles outlive the
/// spawner only as long as the process table does.
#[derive(Default)]
pub struct LocalSpawner {
    /// PID-as-string → Child. Indirected through `Mutex` because a
    /// shared `&self` spawner is used across worker dispatches.
    children: Mutex<HashMap<String, Child>>,
}

impl LocalSpawner {
    pub fn new() -> Self {
        Self::default()
    }
}

impl WorkerSpawner for LocalSpawner {
    fn spawn(&self, spec: WorkerSpec) -> Result<WorkerHandle, SpawnError> {
        warn_local_spawner_once();
        if spec.image_digest.is_some() {
            return Err(SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: "LocalSpawner does not run images; spec.image_digest must be None"
                    .to_string(),
            });
        }
        if spec.command.is_empty() {
            return Err(SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: "LocalSpawner requires spec.command to be non-empty".to_string(),
            });
        }
        if !spec.tempdir_host_path.exists() {
            return Err(SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: format!(
                    "tempdir host path does not exist: {}",
                    spec.tempdir_host_path.display()
                ),
            });
        }

        let mut cmd = Command::new(&spec.command[0]);
        cmd.args(&spec.command[1..])
            .current_dir(&spec.tempdir_host_path)
            .env_clear()
            .envs(&spec.env);

        let child = cmd.spawn().map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("failed to spawn `{}`: {e}", spec.command[0]),
        })?;

        let pid = child.id();
        let id = pid.to_string();
        // The worker is expected to create a UDS at this well-known
        // path within its tempdir. Phase 18a does not yet produce a
        // worker that does so — the path is recorded for forthcoming
        // RPC milestones.
        let uds_path = spec.tempdir_host_path.join("worker.sock");

        self.children
            .lock()
            .expect("children mutex poisoned")
            .insert(id.clone(), child);

        Ok(WorkerHandle {
            id,
            uds_path,
            backend: BACKEND,
        })
    }

    fn wait_with_timeout(
        &self,
        handle: &WorkerHandle,
        timeout: Option<Duration>,
    ) -> Result<ExitStatus, SpawnError> {
        let mut child = self
            .children
            .lock()
            .expect("children mutex poisoned")
            .remove(&handle.id)
            .ok_or_else(|| SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: format!("no spawned child for handle id `{}`", handle.id),
            })?;
        match timeout {
            None => child.wait().map_err(|e| SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: format!("wait failed for handle id `{}`: {e}", handle.id),
            }),
            Some(t) => wait_polled(&mut child, &handle.id, t),
        }
    }

    fn kill(&self, handle: &WorkerHandle) -> Result<(), SpawnError> {
        let mut table = self.children.lock().expect("children mutex poisoned");
        let child = table
            .get_mut(&handle.id)
            .ok_or_else(|| SpawnError::SpawnFailed {
                backend: BACKEND,
                reason: format!("no spawned child for handle id `{}`", handle.id),
            })?;
        child.kill().map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!("kill failed for handle id `{}`: {e}", handle.id),
        })?;
        // Reap so the entry doesn't linger as a zombie.
        let _ = child.wait();
        table.remove(&handle.id);
        Ok(())
    }

    fn attach_uds(&self, handle: &WorkerHandle) -> Result<UnixStream, SpawnError> {
        UnixStream::connect(&handle.uds_path).map_err(|e| SpawnError::SpawnFailed {
            backend: BACKEND,
            reason: format!(
                "could not connect to worker UDS at {}: {e}",
                handle.uds_path.display()
            ),
        })
    }

    fn backend(&self) -> &'static str {
        BACKEND
    }
}

/// Poll a child for `try_wait` completion until either it exits
/// (returns `Ok(status)`) or `timeout` expires (kills the child,
/// returns `Err(SpawnError::WaitTimedOut)`). On the timeout path the
/// child is reaped before return so the caller can rely on
/// "WaitTimedOut implies the process is gone."
fn wait_polled(
    child: &mut Child,
    handle_id: &str,
    timeout: Duration,
) -> Result<ExitStatus, SpawnError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(SpawnError::WaitTimedOut {
                        handle_id: handle_id.to_string(),
                        timeout_ms: timeout.as_millis() as u64,
                    });
                }
                std::thread::sleep(WAIT_POLL_INTERVAL);
            }
            Err(e) => {
                return Err(SpawnError::SpawnFailed {
                    backend: BACKEND,
                    reason: format!("try_wait failed for handle id `{handle_id}`: {e}"),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::time::Duration;

    /// Per-test tempdir keyed on PID + an atomic counter so tests
    /// running in parallel don't race on the same path.
    fn tempdir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "eigenius-runtime-substrate-test-{}-{}-{}",
            std::process::id(),
            label,
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }

    fn spec_with(command: Vec<&str>, dir: PathBuf) -> WorkerSpec {
        WorkerSpec {
            image_digest: None,
            command: command.into_iter().map(String::from).collect(),
            tempdir_host_path: dir,
            depot_host_path: None,
            env: BTreeMap::new(),
            max_wall_time_ms: 0,
            max_memory_bytes: 0,
            seccomp_profile: None,
        }
    }

    #[test]
    fn spawn_and_wait_for_short_lived_process() {
        let dir = tempdir("spawn_and_wait");
        let spawner = LocalSpawner::new();
        let handle = spawner
            .spawn(spec_with(vec!["/bin/true"], dir.clone()))
            .expect("spawn /bin/true");
        assert_eq!(handle.backend, BACKEND);
        let status = spawner.wait_with_timeout(&handle, None).expect("wait");
        assert!(status.success());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_terminates_long_running_process() {
        let dir = tempdir("kill");
        let spawner = LocalSpawner::new();
        let handle = spawner
            .spawn(spec_with(vec!["/bin/sleep", "30"], dir.clone()))
            .expect("spawn /bin/sleep");
        std::thread::sleep(Duration::from_millis(50));
        spawner.kill(&handle).expect("kill");
        // After kill, wait should fail because the entry was removed.
        let err = spawner
            .wait_with_timeout(&handle, None)
            .expect_err("wait after kill");
        assert!(matches!(err, SpawnError::SpawnFailed { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_vars_propagate_to_child() {
        let dir = tempdir("env_propagate");
        let spawner = LocalSpawner::new();
        let mut spec = spec_with(
            vec!["/bin/sh", "-c", "test \"$EIGENIUS_TEST_VAR\" = expected"],
            dir.clone(),
        );
        spec.env
            .insert("EIGENIUS_TEST_VAR".to_string(), "expected".to_string());
        let handle = spawner.spawn(spec).expect("spawn");
        let status = spawner.wait_with_timeout(&handle, None).expect("wait");
        assert!(status.success(), "child exited with {:?}", status.code());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_image_digest() {
        let dir = tempdir("reject_digest");
        let spawner = LocalSpawner::new();
        let mut spec = spec_with(vec!["/bin/true"], dir.clone());
        spec.image_digest = Some(
            crate::types::ImageDigest::parse(
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .unwrap(),
        );
        let err = spawner.spawn(spec).expect_err("should reject image_digest");
        assert!(matches!(err, SpawnError::SpawnFailed { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_empty_command() {
        let dir = tempdir("reject_empty_cmd");
        let spawner = LocalSpawner::new();
        let err = spawner
            .spawn(spec_with(vec![], dir.clone()))
            .expect_err("should reject empty command");
        assert!(matches!(err, SpawnError::SpawnFailed { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_missing_tempdir() {
        let spawner = LocalSpawner::new();
        let nonexistent = PathBuf::from("/tmp/eigenius-definitely-does-not-exist-zzzzz");
        let err = spawner
            .spawn(spec_with(vec!["/bin/true"], nonexistent))
            .expect_err("should reject missing tempdir");
        assert!(matches!(err, SpawnError::SpawnFailed { .. }));
    }

    #[test]
    fn wait_with_timeout_fires_and_kills_long_running_process() {
        let dir = tempdir("wait_timeout");
        let spawner = LocalSpawner::new();
        let handle = spawner
            .spawn(spec_with(vec!["/bin/sleep", "30"], dir.clone()))
            .expect("spawn /bin/sleep");
        let err = spawner
            .wait_with_timeout(&handle, Some(Duration::from_millis(150)))
            .expect_err("must time out");
        match err {
            SpawnError::WaitTimedOut {
                handle_id,
                timeout_ms,
            } => {
                assert_eq!(handle_id, handle.id);
                assert_eq!(timeout_ms, 150);
            }
            other => panic!("unexpected error: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_with_timeout_returns_status_when_process_exits_first() {
        let dir = tempdir("wait_timeout_short");
        let spawner = LocalSpawner::new();
        let handle = spawner
            .spawn(spec_with(vec!["/bin/true"], dir.clone()))
            .expect("spawn /bin/true");
        // Generous timeout — /bin/true exits in microseconds.
        let status = spawner
            .wait_with_timeout(&handle, Some(Duration::from_secs(5)))
            .expect("wait");
        assert!(status.success());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nonzero_exit_status_surfaces() {
        let dir = tempdir("nonzero_exit");
        let spawner = LocalSpawner::new();
        let handle = spawner
            .spawn(spec_with(vec!["/bin/sh", "-c", "exit 7"], dir.clone()))
            .expect("spawn");
        let status = spawner.wait_with_timeout(&handle, None).expect("wait");
        assert_eq!(status.code(), Some(7));
        // Sanity-check Linux ExitStatus::from_raw round-trips.
        let _: ExitStatus = ExitStatusExt::from_raw(0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
