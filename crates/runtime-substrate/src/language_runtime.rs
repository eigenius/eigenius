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

//! The `LanguageRuntime` trait — the seam per-language crates implement
//! to plug a hosted runtime into the substrate.
//!
//! Per D26 §3, the trait is intentionally small: most of the work each
//! language crate does is in `build_environment_image` (the
//! language-specific Dockerfile shape) and the worker-side bootstrap
//! script. The substrate provides the rest — RPC framing, image
//! push/pull (when applicable), boundary check, provenance assembly,
//! sandbox enforcement.
//!
//! ## Deviation from D26 §3
//!
//! D26 §3 sketches the trait with typed view structs as parameters
//! (`script: &RuntimeScript`, `signature: &RuntimeMethodSignature`,
//! `env: &RuntimeEnvironment`). This implementation takes
//! `&kernel::Resource` everywhere instead. Reasons:
//!
//! 1. The kernel's existing `Institution` trait uses `&Resource` for the
//!    same dispatch-boundary role, and the substrate is best understood
//!    as a sibling pattern — uniform shape across the two trait
//!    surfaces beats a parallel hierarchy of typed views.
//! 2. The substrate's *typed* access story is the
//!    `RuntimePackageMirror` (D26 §7) — language-side mirror structs
//!    generated from Eigon class definitions. Typed access happens
//!    *inside* the worker against a mirror, not at the trait boundary,
//!    so typed views at this seam would be redundant.
//! 3. Keeping the boundary parameter type uniform (always `&Resource`)
//!    means that future class additions don't churn the trait signature.

use crate::error::{BuildError, RunError, SpawnError};
use crate::types::{DockerfileFragments, ImageDigest, WorkerHandle};
use eigenius_kernel::ontology::resource::Resource;

/// The interface a per-language crate implements to register a hosted
/// runtime with the substrate.
///
/// Implementors live in language-specific crates (e.g. `eigenius-julia`,
/// `eigenius-lean`). The substrate keeps a registry keyed by
/// [`LanguageRuntime::language_id`] and dispatches to the matching impl
/// based on the `language` property on `RuntimeScript` /
/// `RuntimeEnvironment` / `RuntimeMethodSignature` resources.
///
/// Methods are grouped by lifecycle phase:
///
/// - **Build phase** (called by `eigenius env create`): produce a pinned
///   image from a `RuntimeEnvironment` + its constituents.
/// - **Spawn phase** (called per invocation in v1's spawn-per-call
///   model, or per-environment with the warm-worker pool added in
///   Phase 19c): instantiate a worker against an image.
/// - **Run phase** (called per invocation): dispatch a script or method
///   into a spawned worker.
/// - **Image-build helper**: emit the Dockerfile fragments the substrate
///   composes into a final Dockerfile during the build phase.
pub trait LanguageRuntime: Send + Sync {
    /// Identifier — `"julia"`, `"python"`, `"lean"`, etc. Used to
    /// namespace IRIs and to dispatch a `Resource` (whose `language`
    /// property declares which runtime owns it) to the matching impl.
    fn language_id(&self) -> &str;

    /// Build the OCI image for a `RuntimeEnvironment` resource.
    ///
    /// The substrate composes the per-language Dockerfile fragments
    /// (see `dockerfile_fragments`) with shared base layers, materialises
    /// `included_packages` source trees + the mirror archive into the
    /// build context, invokes `buildah` deterministically (D26 §9.2),
    /// and pushes to the configured registry. The captured digest is
    /// returned and stored on the `RuntimeEnvironment.image_digest`
    /// property.
    ///
    /// Phase 18c milestone — `LocalSpawner`-only deployments may skip
    /// this entirely (deployment shape (c), D26 §10.1) and operate with
    /// `image_digest: None`.
    fn build_environment_image(
        &self,
        env: &Resource,
        packages: &[Resource],
        mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError>;

    /// Instantiate a worker against a built image.
    ///
    /// The substrate's `WorkerSpawner` backend handles the actual
    /// container/process lifecycle (D26 §8.2); this method gives the
    /// per-language impl a chance to add language-specific bootstrap
    /// (e.g. choose the worker entry-point script). Most impls will
    /// just delegate to the substrate's spawner with a `WorkerSpec`
    /// derived from the environment.
    ///
    /// In v1's spawn-per-invocation model (D26 §8.1) this is called
    /// once per `RunRuntimeScript` / `CallRuntimeMethod` dispatch.
    fn spawn_worker(
        &self,
        env: &Resource,
        image_digest: Option<&ImageDigest>,
    ) -> Result<WorkerHandle, SpawnError>;

    /// Run a script inside a freshly spawned worker.
    ///
    /// The substrate has already resolved `script` and `inputs` from the
    /// graph; this call passes them across the worker RPC and waits for
    /// the output. The implementation must:
    ///
    /// 1. Marshal `inputs` into mirror-struct values using the
    ///    `RuntimePackageMirror` baked into the worker's image.
    /// 2. Dispatch into the language-side entry point declared on
    ///    `script.entry_point`.
    /// 3. Marshal the language-side return value back into a `Resource`.
    ///
    /// Boundary-check failures (D26 §7.5) surface as
    /// [`RunError::MirrorVersionMismatch`] /
    /// [`RunError::MissingMirrorStruct`]; runtime-level exceptions as
    /// [`RunError::RuntimeError`].
    fn run_script(
        &self,
        worker: &WorkerHandle,
        script: &Resource,
        inputs: &[Resource],
    ) -> Result<Resource, RunError>;

    /// Call a single declared method by signature.
    ///
    /// Same shape as `run_script` but with a declared
    /// `RuntimeMethodSignature` resource instead of a script body.
    /// Sharper surface for the "library call" use case;
    /// implementations may share most of the marshalling logic with
    /// `run_script`.
    fn call_method(
        &self,
        worker: &WorkerHandle,
        signature: &Resource,
        inputs: &[Resource],
    ) -> Result<Resource, RunError>;

    /// Emit the Dockerfile fragments the substrate's build pipeline
    /// composes into a final Dockerfile (D26 §9.2). Per-language
    /// fragments install the runtime, instantiate dependencies, register
    /// the mirror, and bake build-time provenance.
    ///
    /// Returning [`DockerfileFragments::default`] is acceptable for
    /// `LocalSpawner`-only deployments that never run
    /// `build_environment_image`.
    fn dockerfile_fragments(&self, env: &Resource) -> DockerfileFragments;
}
