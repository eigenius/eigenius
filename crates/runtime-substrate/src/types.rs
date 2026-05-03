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

//! Substrate-level value types shared across the trait surface and the
//! spawner backends.

use std::collections::BTreeMap;
use std::path::PathBuf;
use thiserror::Error;

/// OCI image digest in `sha256:<64 hex>` form. D26 §5.3.
///
/// Constructed via [`ImageDigest::parse`] which validates the syntactic
/// shape; no remote interaction. The digest is the primary
/// reproducibility anchor at runtime — every `RuntimeInvocation` echoes
/// the digest it dispatched against so audits can verify byte-identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageDigest(String);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageDigestError {
    #[error("image digest must be of the form `sha256:<64 hex>`, got `{0}`")]
    Malformed(String),
}

impl ImageDigest {
    /// Parse and validate. Accepts only the lowercase `sha256:` form.
    pub fn parse(s: impl Into<String>) -> Result<Self, ImageDigestError> {
        let s = s.into();
        if let Some(rest) = s.strip_prefix("sha256:") {
            if rest.len() == 64
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Ok(Self(s));
            }
        }
        Err(ImageDigestError::Malformed(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ImageDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Handle for a spawned worker, returned by [`crate::spawner::WorkerSpawner::spawn`]
/// and consumed by the substrate's RPC layer.
///
/// The handle is opaque to language-side code — its fields are populated
/// by the spawner backend that produced it. The substrate uses
/// `uds_path` to attach the CBOR RPC socket and `id` for telemetry.
#[derive(Debug)]
pub struct WorkerHandle {
    /// Spawner-assigned identifier (e.g. container ID for `DockerSpawner`,
    /// PID for `LocalSpawner`).
    pub id: String,
    /// Filesystem path of the worker's RPC Unix-domain socket.
    /// Always resolved against the host depot path under DooD discipline
    /// (D26 §9.5) so it is valid in both the orchestrator's and the
    /// worker's filesystem view.
    pub uds_path: PathBuf,
    /// Backend identifier for telemetry: `"local"`, `"docker"`, ...
    pub backend: &'static str,
}

/// Inputs to [`crate::spawner::WorkerSpawner::spawn`]. Carries everything
/// a backend needs to materialise a worker process: the image (or
/// command) to run, the host paths to bind-mount in (per D26 §9.5), the
/// env vars used by the worker bootstrap cross-check (D26 §9.3), and
/// the per-invocation resource caps that map to D26 §8.3 sandbox
/// enforcement.
///
/// Phase 18a uses a subset of these (command + paths + env); the
/// seccomp profile, capability set, and resource-cap enforcement arrive
/// with `DockerSpawner` in 18c.
#[derive(Debug, Clone)]
pub struct WorkerSpec {
    /// Image to spawn, by digest. Required for `DockerSpawner`. `None`
    /// for `LocalSpawner` (no container; `command` is launched directly
    /// on the host).
    pub image_digest: Option<ImageDigest>,
    /// Worker bootstrap command — `[exe, arg1, arg2, ...]`. Required
    /// for `LocalSpawner`. For `DockerSpawner`, an empty `Vec` defers
    /// to the image's `CMD`; a non-empty `Vec` overrides it.
    pub command: Vec<String>,
    /// Host path that becomes the worker's per-invocation tempdir
    /// (read-write, mounted under the well-known depot path per
    /// D26 §9.5). The substrate creates this directory before spawn.
    pub tempdir_host_path: PathBuf,
    /// Host path of the read-only runtime depot mount (per-language
    /// caches, etc.). `None` for backends that don't use a depot.
    pub depot_host_path: Option<PathBuf>,
    /// Environment variables passed to the worker. The substrate
    /// always populates `EIGENIUS_RUNTIME_ENV_DIGEST` and
    /// `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH` for the cross-check
    /// (D26 §9.3).
    pub env: BTreeMap<String, String>,
    /// Wall-clock cap. `0` means "unbounded" (debug only — production
    /// must always set a positive cap). Enforced by `DockerSpawner` via
    /// container limits in 18c; `LocalSpawner` does not enforce in v1.
    pub max_wall_time_ms: u64,
    /// Memory cap in bytes. `0` means "unbounded" (debug only).
    /// Enforcement same as `max_wall_time_ms`.
    pub max_memory_bytes: u64,
    /// Optional seccomp profile JSON applied via `HostConfig.security_opt`
    /// for `DockerSpawner` (D26 §8.3). Phase 18c populates this; ignored
    /// by `LocalSpawner`.
    pub seccomp_profile: Option<String>,
}

/// Per-language Dockerfile fragments composed into a final Dockerfile by
/// the substrate's image-build pipeline (D26 §9.2). Each field
/// contributes the corresponding stage of the build.
///
/// The substrate orders fragments deterministically: install_runtime →
/// install_packages → install_mirror → bootstrap_command. A per-language
/// crate fills in whichever pieces it needs and leaves the rest empty.
#[derive(Debug, Default, Clone)]
pub struct DockerfileFragments {
    /// Lines installing the language runtime itself (e.g. `juliaup`,
    /// `pyenv`, `elan`).
    pub install_runtime: Vec<String>,
    /// Lines instantiating registry packages from the lockfile (e.g.
    /// `Pkg.instantiate()`, `uv sync`).
    pub install_packages: Vec<String>,
    /// Lines registering the `RuntimePackageMirror` archive with the
    /// language's package manager.
    pub install_mirror: Vec<String>,
    /// Worker bootstrap command — `[exe, arg1, arg2, ...]`. Becomes the
    /// image's `CMD` in exec form (`CMD ["exe", "arg1", "arg2"]`) so
    /// shell metacharacter handling is unambiguous and the bootstrap
    /// process is PID 1 inside the container (signal forwarding, exit
    /// code propagation).
    pub bootstrap_command: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_digest_parses_canonical_form() {
        let d = ImageDigest::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("parse");
        assert_eq!(
            d.as_str(),
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn image_digest_rejects_uppercase_hex() {
        let err = ImageDigest::parse(
            "sha256:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap_err();
        assert!(matches!(err, ImageDigestError::Malformed(_)));
    }

    #[test]
    fn image_digest_rejects_wrong_length() {
        assert!(matches!(
            ImageDigest::parse("sha256:abc").unwrap_err(),
            ImageDigestError::Malformed(_)
        ));
    }

    #[test]
    fn image_digest_rejects_wrong_algorithm() {
        assert!(matches!(
            ImageDigest::parse(
                "sha512:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            )
            .unwrap_err(),
            ImageDigestError::Malformed(_)
        ));
    }

    #[test]
    fn image_digest_rejects_non_hex() {
        assert!(matches!(
            ImageDigest::parse(
                "sha256:gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg"
            )
            .unwrap_err(),
            ImageDigestError::Malformed(_)
        ));
    }
}
