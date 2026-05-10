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

//! Construction-time configuration for [`super::DockerSpawner`].

use std::path::PathBuf;

/// Path to the host's Docker socket — `/var/run/docker.sock` is the
/// universal default, mounted into the orchestrator container under DooD
/// (D26 §9.5).
pub const DEFAULT_DOCKER_SOCKET: &str = "/var/run/docker.sock";

/// What the spawner should do when an image isn't already in the local
/// daemon's cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullPolicy {
    /// Pull on every spawn. Slow but always-current. Useful when an
    /// upstream registry rotates a tag without bumping the digest (rare
    /// in production where the substrate addresses by digest).
    Always,
    /// Pull only when the image isn't present locally. Default — works
    /// well with digest-pinned images where `Always` would be wasted
    /// network.
    IfMissing,
    /// Never pull. Spawn fails with `EnvironmentImageUnavailable` if the
    /// image isn't in the cache. Production hardening setting once a
    /// build pipeline has primed the cache; also the right choice for
    /// air-gapped deployments.
    Never,
}

/// Container network mode applied via `HostConfig.NetworkMode` at spawn
/// time. The substrate's Job containers are self-contained (packages
/// baked into the image, no runtime fetches) so the hardened default is
/// total network isolation; deployments that need otherwise can opt in.
#[derive(Debug, Clone)]
pub enum NetworkMode {
    /// `--network none`. No network namespace, no loopback, no DNS.
    /// Hardened default for the substrate's spawn-per-invocation Job
    /// model.
    None,
    /// `--network bridge`. Default Docker bridge — a NATted private
    /// network with outbound internet access. Only sensible when an
    /// invocation legitimately needs the network (e.g. fetching a
    /// dataset at runtime, which is off-spec for the Job model but may
    /// arise in dev workflows).
    Bridge,
    /// Free-form network name for callers using a pre-existing custom
    /// network. Substrate makes no guarantees about isolation.
    Named(String),
}

impl NetworkMode {
    /// Render the mode as Docker's `NetworkMode` string.
    pub fn as_docker_string(&self) -> &str {
        match self {
            NetworkMode::None => "none",
            NetworkMode::Bridge => "bridge",
            NetworkMode::Named(s) => s.as_str(),
        }
    }
}

/// Construction-time configuration for [`super::DockerSpawner`].
///
/// The depot path is the load-bearing piece of the DooD discipline (D26
/// §9.5): every per-invocation tempdir, every read-only mount, every
/// substrate-managed artifact lives under this single host path, and
/// the same path must exist (bind-mounted from the host) inside the
/// orchestrator container. The substrate refuses to start when the
/// depot path doesn't satisfy the discipline.
#[derive(Debug, Clone)]
pub struct DockerSpawnerConfig {
    pub depot_path: PathBuf,
    pub docker_socket: Option<PathBuf>,
    pub pull_policy: PullPolicy,
    pub default_network_mode: NetworkMode,
}

impl DockerSpawnerConfig {
    /// Construct with the production defaults: `IfMissing` pull policy,
    /// `none` network mode, default Docker socket. Caller supplies the
    /// depot path — it has no defensible default.
    pub fn new(depot_path: impl Into<PathBuf>) -> Self {
        Self {
            depot_path: depot_path.into(),
            docker_socket: None,
            pull_policy: PullPolicy::IfMissing,
            default_network_mode: NetworkMode::None,
        }
    }

    /// Construct from an [`eigenius_config::SubstrateConfig`], reading
    /// the Docker daemon socket from the config layer. Use this in
    /// production callers (orchestrator startup, CLI commands) so
    /// rootless / macOS Docker setups don't need code edits — the
    /// socket override is config-driven.
    pub fn from_substrate_config(
        depot_path: impl Into<PathBuf>,
        cfg: &eigenius_config::SubstrateConfig,
    ) -> Self {
        Self {
            depot_path: depot_path.into(),
            docker_socket: Some(cfg.docker.daemon_socket.clone()),
            pull_policy: PullPolicy::IfMissing,
            default_network_mode: NetworkMode::None,
        }
    }

    /// Resolve the configured Docker socket path, falling back to the
    /// default.
    pub fn resolved_docker_socket(&self) -> PathBuf {
        self.docker_socket
            .clone()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DOCKER_SOCKET))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_secure_and_digest_friendly() {
        let cfg = DockerSpawnerConfig::new("/var/lib/eigenius-runtime");
        assert_eq!(cfg.pull_policy, PullPolicy::IfMissing);
        assert!(matches!(cfg.default_network_mode, NetworkMode::None));
        assert_eq!(
            cfg.resolved_docker_socket(),
            PathBuf::from(DEFAULT_DOCKER_SOCKET)
        );
    }

    #[test]
    fn network_mode_renders_to_docker_strings() {
        assert_eq!(NetworkMode::None.as_docker_string(), "none");
        assert_eq!(NetworkMode::Bridge.as_docker_string(), "bridge");
        assert_eq!(
            NetworkMode::Named("private-net".to_string()).as_docker_string(),
            "private-net"
        );
    }

    #[test]
    fn from_substrate_config_picks_up_daemon_socket_override() {
        let mut sub = eigenius_config::SubstrateConfig::default();
        sub.docker.daemon_socket = PathBuf::from("/run/user/1000/docker.sock");
        let cfg = DockerSpawnerConfig::from_substrate_config("/var/lib/eigenius-runtime", &sub);
        assert_eq!(
            cfg.resolved_docker_socket(),
            PathBuf::from("/run/user/1000/docker.sock")
        );
        // Other defaults unchanged.
        assert_eq!(cfg.pull_policy, PullPolicy::IfMissing);
        assert!(matches!(cfg.default_network_mode, NetworkMode::None));
    }
}
