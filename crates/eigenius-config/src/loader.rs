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

//! Layered loader that resolves a [`Config`] from four sources in
//! decreasing-precedence order: explicit construction-time overrides,
//! env vars, a TOML file, and finally the schema defaults. Consumers
//! call [`Loader::load`] once at startup and pass the resulting
//! [`Config`] into the substrate / kernel / orchestrator subsystems.
//!
//! The loader is deliberately minimal — no hot-reload, no audit log,
//! no per-namespace overrides. Those land in a follow-on phase if and
//! when concrete needs arise; v1 ships the *primitive*, not the full
//! settings system. Per-spawn env vars (digest, manifest hash, UDS
//! path, …) are explicitly *not* config and stay as direct
//! `std::env::var` reads in the worker bootstrap.

use crate::config::Config;
use std::path::{Path, PathBuf};

/// Function shape a loader caller plugs in to read environment
/// variables. Defaulted to `std::env::var` in production; tests pin a
/// synthetic environment so they don't have to mutate the process's
/// real env vars.
type EnvProvider = dyn Fn(&str) -> Option<String> + Send + Sync;

/// Last-precedence override hook applied to the resolved config.
type OverrideFn = dyn FnOnce(&mut Config) + Send;

/// Errors the loader can surface. Carry enough context (path, parser
/// message) to fix the underlying file without a debugger.
#[derive(thiserror::Error, Debug)]
pub enum LoaderError {
    /// The candidate config file existed but couldn't be read.
    #[error("could not read config file {path}: {source}")]
    FileRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The TOML parser rejected the file.
    #[error("config parse error in {source_label}: {message}")]
    Parse {
        source_label: String,
        message: String,
    },
}

/// Layered config loader. Build via [`Loader::new`], adjust precedence
/// hooks if needed (mostly used in tests), call [`Loader::load`] once.
#[derive(Default)]
pub struct Loader {
    /// Explicit path to the config file. If `Some`, this overrides
    /// the search path; if missing on disk it's a hard error.
    explicit_file: Option<PathBuf>,

    /// Environment provider — defaulted to `std::env::var` but
    /// swappable in tests so precedence can be exercised without
    /// mutating the process's actual environment.
    env_provider: Option<Box<EnvProvider>>,

    /// Disable the search-path lookup. When `true`, no file is
    /// considered unless [`Loader::explicit_file`] was set.
    no_search_path: bool,

    /// Construction-time overrides applied last (highest precedence).
    /// Stored as a closure rather than a partial config so callers
    /// can mutate any nested field without an explicit "unset"
    /// sentinel in the schema.
    override_fn: Option<Box<OverrideFn>>,
}

impl Loader {
    /// Construct a fresh loader with default behaviour: search path =
    /// `$EIGENIUS_CONFIG → ./eigenius.toml → ~/.config/eigenius/config.toml`,
    /// env vars provided by `std::env::var`, no overrides.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin the loader to a specific file. Replaces the search path —
    /// useful for tests and explicit deployments.
    pub fn with_file(mut self, path: PathBuf) -> Self {
        self.explicit_file = Some(path);
        self
    }

    /// Skip the search-path lookup entirely. Combined with no
    /// explicit file, defaults are the only source the loader uses
    /// outside of env vars + overrides. Used by tests that need to
    /// guarantee no `eigenius.toml` from the developer's working
    /// directory leaks into the run.
    pub fn no_search_path(mut self) -> Self {
        self.no_search_path = true;
        self
    }

    /// Provide an alternate env-var source. The default reads from
    /// `std::env::var`; tests pin a synthetic environment.
    pub fn with_env_provider<F>(mut self, f: F) -> Self
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        self.env_provider = Some(Box::new(f));
        self
    }

    /// Run the closure against the resolved config last (highest
    /// precedence). Used in tests and as a programmatic escape hatch
    /// for callers that need to override one field without writing a
    /// dedicated config file.
    pub fn with_overrides<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut Config) + Send + 'static,
    {
        self.override_fn = Some(Box::new(f));
        self
    }

    /// Resolve the config. Layering order — later layers win:
    ///   1. Schema [`Default`]s.
    ///   2. TOML file (search path or explicit).
    ///   3. Environment variables.
    ///   4. Construction-time overrides ([`Loader::with_overrides`]).
    pub fn load(self) -> Result<Config, LoaderError> {
        let env = self.env_provider.unwrap_or_else(default_env_provider);

        // 1. Defaults.
        let mut cfg = Config::default();

        // 2. File. Explicit path is mandatory if set; search path is
        //    best-effort (silent miss → fall through to defaults).
        if let Some(path) = self.explicit_file {
            cfg = merge_file(cfg, &path, /*required=*/ true)?;
        } else if !self.no_search_path {
            for path in search_paths(env.as_ref()) {
                if path.is_file() {
                    cfg = merge_file(cfg, &path, /*required=*/ false)?;
                    break;
                }
            }
        }

        // 3. Env vars.
        apply_env(&mut cfg, env.as_ref());

        // 4. Construction-time overrides.
        if let Some(f) = self.override_fn {
            f(&mut cfg);
        }

        Ok(cfg)
    }
}

/// Search paths in priority order. The loader uses the first one
/// that exists on disk:
///   1. `$EIGENIUS_CONFIG` — explicit, never inferred.
///   2. `./eigenius.toml` — project-local checkout.
///   3. `~/.config/eigenius/config.toml` — XDG default.
fn search_paths(env: &EnvProvider) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = env("EIGENIUS_CONFIG") {
        out.push(PathBuf::from(p));
    }
    out.push(PathBuf::from("eigenius.toml"));
    if let Some(home) = env("HOME") {
        out.push(
            PathBuf::from(home)
                .join(".config")
                .join("eigenius")
                .join("config.toml"),
        );
    }
    out
}

/// Read a TOML file and deserialise into a [`Config`]. `required =
/// true` makes a missing file an error; `required = false` lets the
/// search path fall through. Missing fields in the file are filled
/// from [`Default`] via `#[serde(default)]`, so the parsed config is
/// the new running state — no separate merge pass needed.
fn merge_file(cfg: Config, path: &Path, required: bool) -> Result<Config, LoaderError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if !required && e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(cfg);
        }
        Err(e) => {
            return Err(LoaderError::FileRead {
                path: path.to_path_buf(),
                source: e,
            });
        }
    };
    toml::from_str(&raw).map_err(|e| LoaderError::Parse {
        source_label: format!("file {}", path.display()),
        message: e.to_string(),
    })
}

/// Apply env-var overrides to the running config. The mapping is a
/// flat translation of the TOML structure: each nested key becomes
/// `EIGENIUS_<SECTION>_<FIELD>`, screaming snake case. Unset vars
/// leave the file/default value alone.
fn apply_env(cfg: &mut Config, env: &EnvProvider) {
    if let Some(v) = env("EIGENIUS_IMAGE_REGISTRY_URL") {
        cfg.substrate.image.registry_url = if v.is_empty() { None } else { Some(v) };
    }
    if let Some(v) = env("EIGENIUS_IMAGE_REGISTRY_CREDENTIALS_ENV") {
        cfg.substrate.image.registry_credentials_env = if v.is_empty() { None } else { Some(v) };
    }
    if let Some(v) = env("EIGENIUS_DOCKER_DAEMON_SOCKET") {
        cfg.substrate.docker.daemon_socket = PathBuf::from(v);
    }
    if let Some(v) = env("EIGENIUS_LOCAL_JULIA_BINARY") {
        cfg.substrate.local.julia_binary = PathBuf::from(v);
    }
    // ── [embedder] section ───────────────────────────────────────
    if let Some(v) = env("EIGENIUS_EMBEDDER_ENABLED") {
        // Comma-separated list; empty string → empty list.
        cfg.embedder.enabled = v
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    if let Some(v) = env("EIGENIUS_EMBEDDER_DEVICE") {
        if let Some(d) = crate::embedder::DeviceSelection::parse(&v) {
            cfg.embedder.device = d;
        }
        // Silently leave the value alone on unrecognised input; the
        // service-side path emits a structured diagnostic when the
        // typed value is consumed.
    }
    if let Some(v) = env("EIGENIUS_EMBEDDER_BATCH_SIZE") {
        if let Ok(n) = v.parse::<usize>() {
            if n > 0 {
                cfg.embedder.batch_size = n;
            }
        }
    }
    if let Some(v) = env("EIGENIUS_EMBEDDER_FAIL_FAST_ON_MISSING_MODEL") {
        cfg.embedder.fail_fast_on_missing_model =
            matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
    }
}

fn default_env_provider() -> Box<EnvProvider> {
    Box::new(|name: &str| std::env::var(name).ok())
}
