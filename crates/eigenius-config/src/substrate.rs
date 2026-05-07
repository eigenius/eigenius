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

//! Schema for the `[substrate]` section of `eigenius.toml`.
//!
//! The substrate concerns covered by this schema are *image and
//! backend selection* — what registry to push to, which Docker socket
//! to talk to, which Julia binary the local spawner should invoke.
//! Per-spawn parameters (`EIGENIUS_RUNTIME_ENV_DIGEST`,
//! `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH`, …) are *not* config; they are
//! per-invocation values the substrate passes to each worker via env
//! vars and stay as direct env reads in the worker bootstrap.
//!
//! Pool/scaling tunables are deliberately absent — production scaling
//! is the platform's concern (HPA / KEDA / ACA scale rules) and the
//! substrate's `ServiceSpawner` API is intentionally pool-free.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Substrate-level configuration. Consumed by `runtime-substrate`
/// to derive `DockerSpawnerConfig` defaults and `LocalServiceSpawner`
/// invocation knobs without reaching into env vars directly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SubstrateConfig {
    pub image: ImageConfig,
    pub docker: DockerConfig,
    pub local: LocalConfig,
}

/// Image-related concerns: where the substrate pushes built env
/// images and how it authenticates. v1 of the substrate pushes only
/// to the local Docker daemon (via `buildah push docker-archive: →
/// docker load`), so these fields have no consumer yet — they're
/// reserved here so the schema doesn't churn when the registry-push
/// path lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageConfig {
    /// Registry to push built env images to (`registry.example.com:5000`).
    /// `None` keeps the v1 local-daemon-only behaviour.
    pub registry_url: Option<String>,
    /// Name of the env var holding the registry's auth token. Indirect
    /// so the token itself never lands in the config file.
    pub registry_credentials_env: Option<String>,
}

/// Docker-backed spawner concerns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DockerConfig {
    /// Path to the Docker daemon's UDS. Default
    /// `/var/run/docker.sock` matches every Linux distribution's
    /// stock placement; override when running rootless Docker or on
    /// macOS where the socket lives under `~/.docker/run/`.
    pub daemon_socket: PathBuf,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            daemon_socket: PathBuf::from("/var/run/docker.sock"),
        }
    }
}

/// Local-subprocess spawner concerns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalConfig {
    /// Julia binary the local spawner invokes when the worker spec's
    /// command is unset. PATH lookup if relative; absolute path
    /// pinned otherwise. Today the spec always carries an explicit
    /// command, so this is a forward-looking default.
    pub julia_binary: PathBuf,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            julia_binary: PathBuf::from("julia"),
        }
    }
}
