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

//! `LeanLanguageRuntime` — authoring-side `LanguageRuntime` impl for
//! Lean 4. Phase 20a.5a ships the trait skeleton, the Dockerfile
//! fragment composer, and a stable constructor surface; the actual
//! image builder + Lake worker dispatch lands in 20a.5b once the
//! Lake worker binary is authored.
//!
//! ## Why a skeleton lands first
//!
//! The substrate's `LanguageRuntime` registry is keyed by
//! `language_id`. Wiring the Lean runtime through napi-rs into the
//! Deno orchestrator (20a.5b) is mechanical once a constructor
//! exists; landing the skeleton first means 20a.5b's diff is the
//! Lake worker plus its three integration points (napi-rs binding,
//! `main.ts` startup hook, the end-to-end image-build test), not
//! the worker plus a from-scratch crate.

use std::path::{Path, PathBuf};

use eigenius_kernel::ontology::resource::Resource;
use eigenius_runtime_substrate::error::{BuildError, RunError};
use eigenius_runtime_substrate::invocation::RunOutcome;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::types::{DockerfileFragments, ImageDigest};

use crate::conventions::LANGUAGE;
use crate::dockerfile::{lean_dockerfile_fragments, LeanImagePlan};

/// `LanguageRuntime` impl that drives a Lake-built Lean worker
/// inside a deterministic OCI image. v1 mirrors the Service-mode
/// shape `JuliaLanguageRuntime` uses (one long-lived worker per env
/// image digest, attached over CBOR-framed UDS).
///
/// Constructor takes the Lake project directory (the host path
/// holding `lakefile.lean` / `lake-manifest.json` /
/// `Worker/Main.lean`) and the depot path the eventual
/// `ServiceSpawner` will use. The base image reference defaults to
/// `debian:bookworm-slim` because Lean has no canonical upstream
/// `lean:` image — `install_runtime` in
/// [`crate::dockerfile`] installs `elan` + the pinned toolchain on
/// top of that base.
///
/// 20a.5b will add a `spawner: Arc<dyn ServiceSpawner>`, a
/// digest-keyed service cache, and the actual `ensure_image`
/// implementation. The constructor signature stays additive — the
/// public surface this milestone freezes is what 20a.5b will
/// extend, not change.
pub struct LeanLanguageRuntime {
    /// Path to `lean/runtime-worker/` — the directory containing
    /// the worker's Lake project. Resolved by the caller (typically
    /// via `env!("CARGO_MANIFEST_DIR")` against a workspace-
    /// relative path; the orchestrator's startup wiring passes an
    /// absolute path).
    #[allow(dead_code)]
    project_dir: PathBuf,
    /// Upstream base image the env image extends. Defaults to a
    /// digest-pinnable Debian-slim tag because no upstream `lean:`
    /// image exists; production deployments should pass a digest-
    /// pinned form (`docker.io/library/debian@sha256:...`) for
    /// reproducibility.
    #[allow(dead_code)]
    base_image_ref: String,
    /// Tag the built image is pushed under in the configured
    /// registry. Mirrors Julia's `image_tag` field; production
    /// callers typically pass `eigenius-lean-worker:<digest-prefix>`.
    #[allow(dead_code)]
    image_tag: String,
    /// Host directory the eventual `ServiceSpawner` will use as the
    /// runtime depot (D26 §9.5). Reserved for 20a.5b; recorded here
    /// so the constructor surface is stable.
    #[allow(dead_code)]
    depot_path: PathBuf,
}

impl LeanLanguageRuntime {
    /// Construct with paths to the worker's Lake project, a base
    /// image reference (digest-pinned Debian-slim recommended),
    /// the image tag, and the depot path the spawner was
    /// configured with.
    ///
    /// 20a.5a doesn't take a `ServiceSpawner` argument because no
    /// dispatch path uses one yet. 20a.5b adds an
    /// `Arc<dyn ServiceSpawner>` parameter and a digest-keyed
    /// service cache; existing callers will need to thread a
    /// spawner through at that point.
    pub fn new(
        project_dir: PathBuf,
        base_image_ref: impl Into<String>,
        image_tag: impl Into<String>,
        depot_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            project_dir,
            base_image_ref: base_image_ref.into(),
            image_tag: image_tag.into(),
            depot_path: depot_path.into(),
        }
    }

    /// Read-only view of the worker project directory. Reserved for
    /// 20a.5b's image-build pipeline (it needs to stream
    /// `lakefile.lean` + `lake-manifest.json` + `Worker/Main.lean`
    /// into the build context).
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }
}

impl LanguageRuntime for LeanLanguageRuntime {
    fn language_id(&self) -> &str {
        LANGUAGE
    }

    fn build_environment_image(
        &self,
        _env: &Resource,
        _packages: &[Resource],
        _mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError> {
        // 20a.5b implementation: stream the Lake project + handler
        // packages into a `BuildContext`, drive
        // `BuildahImageBuilder::build` against the
        // `lean_dockerfile_fragments(...)` output, push the
        // resulting image, return the captured digest. Mirrors
        // `JuliaLanguageRuntime::ensure_image`.
        Err(BuildError::EnvironmentBuildFailed(
            "LeanLanguageRuntime::build_environment_image lands in Phase 20a.5b \
             (Lake worker authored, BuildahImageBuilder wired)"
                .to_string(),
        ))
    }

    fn dockerfile_fragments(&self, _env: &Resource) -> DockerfileFragments {
        // The substrate calls this for spec-level inspection (no
        // mirror / handler-package context). Production image build
        // (20a.5b) will pass a populated plan derived from the
        // env's `included_packages`. v1 returns the default plan
        // (no handlers, no mirror) — sufficient for the substrate's
        // spec test to round-trip the fragment shape.
        lean_dockerfile_fragments(&LeanImagePlan::default())
    }

    fn run_script(
        &self,
        _env: &Resource,
        _script: &Resource,
        _inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        // The 20a.5b worker will implement two RPC verbs: `lean_export`
        // (wraps `lake exe lean4export` against a referenced
        // `LeanProject`, returns the bytes) and `dispatch_method`
        // (resolves a `RuntimeMethodSignature` against the worker's
        // loaded environment). Until then, dispatching a Lean
        // script surfaces as a clean `RuntimeError`.
        Err(RunError::RuntimeError(
            "LeanLanguageRuntime::run_script lands in Phase 20a.5b \
             (Lake worker authored, CBOR-framed UDS RPC wired)"
                .to_string(),
        ))
    }

    fn call_method(
        &self,
        _env: &Resource,
        _signature: &Resource,
        _inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        Err(RunError::RuntimeError(
            "LeanLanguageRuntime::call_method lands in Phase 20a.5b \
             (Lake worker authored, RuntimeMethodSignature dispatch wired)"
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_runtime() -> LeanLanguageRuntime {
        LeanLanguageRuntime::new(
            PathBuf::from("/tmp/lean-runtime-worker-test"),
            "debian:bookworm-slim",
            "eigenius-lean-worker:test",
            PathBuf::from("/tmp/depot"),
        )
    }

    #[test]
    fn language_id_is_lean() {
        let rt = make_runtime();
        assert_eq!(rt.language_id(), "lean");
    }

    #[test]
    fn dockerfile_fragments_round_trip_through_trait_surface() {
        // The trait-level call goes through `dockerfile_fragments`
        // which forwards to the free function. The fragment shape
        // is the production target — bootstrap_command pointing at
        // the Lake exe, install_runtime carrying the elan-init
        // pipeline — and pinning this assertion catches accidental
        // regressions when the trait impl gets edited.
        let rt = make_runtime();
        let env = Resource::new_embedded();
        let fragments = rt.dockerfile_fragments(&env);
        assert!(
            fragments
                .bootstrap_command
                .iter()
                .any(|s| s == "lean-runtime-worker"),
            "trait-level call must surface the Lake exe target"
        );
        assert!(
            !fragments.install_runtime.is_empty(),
            "trait-level call must surface the elan/toolchain install"
        );
    }

    #[test]
    fn run_script_returns_phase_pending_diagnostic() {
        let rt = make_runtime();
        let env = Resource::new_embedded();
        let script = Resource::new_embedded();
        let err = rt
            .run_script(&env, &script, &[])
            .expect_err("20a.5a stub must error");
        match err {
            RunError::RuntimeError(msg) => {
                assert!(
                    msg.contains("20a.5b"),
                    "stub diagnostic should reference the landing milestone; got: {msg}"
                );
            }
            other => panic!("expected RuntimeError, got {other:?}"),
        }
    }
}
