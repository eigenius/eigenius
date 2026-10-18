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

//! Image-build backend abstraction (D26 §9.2).
//!
//! [`ImageBuilder`] is the seam between the substrate's deterministic
//! Dockerfile-and-context construction (see [`super::dockerfile`] /
//! [`super::context`]) and the actual image-producing tool. The trait is
//! intentionally narrow:
//!
//! - **Input.** A materialised [`super::context::BuildContext`] and an
//!   image tag.
//! - **Output.** An [`ImageDigest`] identifying the produced image.
//!
//! The build path stays decoupled from the run path (D26 §9.2): the
//! production [`BuildahImageBuilder`] shells out to `buildah` rather than
//! driving a Docker daemon, so build determinism is governed by buildah's
//! `--timestamp 0 --layers --jobs 1` flags rather than by run-side
//! caching state.
//!
//! ## On the returned digest
//!
//! [`BuildahImageBuilder::build`] returns the local image ID — the
//! sha256 of the OCI image config, as reported by `buildah images
//! --format '{{.ID}}'`. This is *not* the registry manifest digest you
//! get after pushing; that digest only exists post-push. For 18c.1's
//! deterministic-build acceptance test (build twice, same id), the local
//! image ID is the right anchor: identical inputs to buildah → identical
//! image config → identical id, with no registry round-trip required.
//!
//! Registry push and the manifest-digest capture (D26 §9.2 step 5) land
//! with 18c.3, where `DockerSpawner` consumes the pushed digest at spawn
//! time.

use crate::error::BuildError;
use crate::image_build::context::BuildContext;
use crate::types::ImageDigest;
use std::process::{Command, Stdio};

const BACKEND: &str = "buildah";

/// Backend abstraction for the substrate's image-build pipeline.
///
/// Implementors take a materialised [`BuildContext`] and produce an
/// [`ImageDigest`]. They must not mutate the context; the substrate
/// guarantees the directory is populated and stable for the duration of
/// the call.
pub trait ImageBuilder: Send + Sync {
    /// Build an image from `context`, tagging it as `image_tag` for
    /// subsequent local lookups, and return the resulting digest.
    fn build(&self, context: &BuildContext, image_tag: &str) -> Result<ImageDigest, BuildError>;

    /// Backend identifier — `"buildah"`, etc. Used for telemetry and
    /// surfaced in [`BuildError::EnvironmentBuildFailed`] diagnostics.
    fn backend(&self) -> &'static str;
}

/// `buildah`-driven [`ImageBuilder`]. Production default on Linux.
///
/// Construction succeeds even when buildah is not installed — the failure
/// surfaces at [`BuildahImageBuilder::build`] time so callers can choose
/// the backend up-front (e.g. by configuration) and still see a tidy
/// diagnostic if the binary is missing.
#[derive(Debug, Default, Clone)]
pub struct BuildahImageBuilder {
    /// Path to the `buildah` binary; defaults to looking it up on
    /// `PATH`. Override for tests or unusual deployments.
    binary: Option<String>,
}

impl BuildahImageBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with an explicit `buildah` binary path. Useful for
    /// hermetic test environments.
    pub fn with_binary(binary: impl Into<String>) -> Self {
        Self {
            binary: Some(binary.into()),
        }
    }

    fn binary_name(&self) -> &str {
        self.binary.as_deref().unwrap_or("buildah")
    }
}

impl ImageBuilder for BuildahImageBuilder {
    fn build(&self, context: &BuildContext, image_tag: &str) -> Result<ImageDigest, BuildError> {
        if image_tag.is_empty() {
            return Err(BuildError::EnvironmentBuildFailed(
                "image_tag must be non-empty".into(),
            ));
        }
        let work_dir = context.work_dir();
        if !work_dir.is_dir() {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "build context work_dir does not exist: {}",
                work_dir.display()
            )));
        }

        let bud = Command::new(self.binary_name())
            .arg("bud")
            .arg("--timestamp")
            .arg("0")
            .arg("--layers")
            .arg("--jobs")
            .arg("1")
            .arg("--format")
            .arg("oci")
            .arg("-t")
            .arg(image_tag)
            .arg(work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                BuildError::EnvironmentBuildFailed(format!(
                    "failed to invoke `{}`: {e}",
                    self.binary_name()
                ))
            })?;
        if !bud.status.success() {
            let stderr = String::from_utf8_lossy(&bud.stderr);
            let stdout = String::from_utf8_lossy(&bud.stdout);
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "buildah bud exited with {}: stderr={stderr}; stdout={stdout}",
                bud.status
            )));
        }

        let inspect = Command::new(self.binary_name())
            .arg("images")
            .arg("--no-trunc")
            .arg("--format")
            .arg("{{.ID}}")
            .arg(image_tag)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                BuildError::EnvironmentBuildFailed(format!(
                    "failed to inspect built image via `{}`: {e}",
                    self.binary_name()
                ))
            })?;
        if !inspect.status.success() {
            let stderr = String::from_utf8_lossy(&inspect.stderr);
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "buildah images failed: {stderr}"
            )));
        }
        let raw = String::from_utf8_lossy(&inspect.stdout);
        let id = raw
            .lines()
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "buildah did not report an image id for tag `{image_tag}`"
            )));
        }
        let digest = if id.starts_with("sha256:") {
            id
        } else {
            format!("sha256:{id}")
        };
        ImageDigest::parse(digest).map_err(|e| {
            BuildError::EnvironmentBuildFailed(format!(
                "buildah returned an unparseable image id for tag `{image_tag}`: {e}"
            ))
        })
    }

    fn backend(&self) -> &'static str {
        BACKEND
    }
}

/// Probe whether `buildah` is callable on `PATH`. Used by integration
/// tests to skip gracefully on hosts without buildah installed.
pub fn is_buildah_available() -> bool {
    Command::new("buildah")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_build::context::{BuildContext, BuildContextSpec};

    fn fresh_work_dir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("substrate-builder-{pid}-{label}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create work_dir");
        dir
    }

    #[test]
    fn build_rejects_empty_tag() {
        let work = fresh_work_dir("empty-tag");
        let ctx = BuildContext::materialize(
            work.clone(),
            &BuildContextSpec {
                dockerfile: "FROM scratch\n".into(),
                built_at: "stamp".into(),
                ..Default::default()
            },
        )
        .expect("materialise");
        let builder = BuildahImageBuilder::new();
        let err = builder.build(&ctx, "").expect_err("empty tag must fail");
        match err {
            BuildError::EnvironmentBuildFailed(m) => assert!(m.contains("image_tag")),
            other => panic!("unexpected error: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn build_surfaces_missing_binary() {
        let work = fresh_work_dir("missing-binary");
        let ctx = BuildContext::materialize(
            work.clone(),
            &BuildContextSpec {
                dockerfile: "FROM scratch\n".into(),
                built_at: "stamp".into(),
                ..Default::default()
            },
        )
        .expect("materialise");
        let builder =
            BuildahImageBuilder::with_binary("/this/path/should/not/exist/buildah-binary-xyz");
        let err = builder
            .build(&ctx, "test:latest")
            .expect_err("missing binary must fail");
        match err {
            BuildError::EnvironmentBuildFailed(m) => {
                assert!(m.contains("buildah") || m.contains("invoke"))
            }
            other => panic!("unexpected error: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn backend_string_is_buildah() {
        assert_eq!(BuildahImageBuilder::new().backend(), "buildah");
    }
}
