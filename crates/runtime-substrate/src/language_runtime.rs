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
//! ## Path-3 trait shape (Phase 19a.2 refactor)
//!
//! The trait is intentionally small: it exposes *intent* (build an env
//! image; dispatch a script; dispatch a method) and lets each runtime
//! impl own its dispatch lifecycle internally. Spawn/lease/cleanup is
//! per-runtime — a Job-mode runtime spawns-runs-cleans-up; a Service-
//! mode runtime ensures-and-attaches; a future K8s/ACA runtime routes
//! through its platform's service endpoint.
//!
//! By keeping `WorkerHandle` / `ServiceHandle` out of the trait
//! signature, the substrate facade stays mode-agnostic and adding a
//! new lifecycle later (K8s, ACA) doesn't require changing the trait
//! or the facade.
//!
//! ## What stays in the substrate
//!
//! The substrate provides RPC framing (CBOR over UDS), image-build
//! pipeline (`buildah`-driven), boundary check, provenance assembly
//! (`DispatchTrace` → `RuntimeInvocation`), and the test fixtures.
//! Per-language crates own: Dockerfile fragments, the worker-side
//! bootstrap script, the dispatch lifecycle (spawn vs ensure-service),
//! and the typed mirror — not on the trait surface but inside the
//! crate.

use crate::error::{BuildError, RunError};
use crate::invocation::RunOutcome;
use crate::types::{DockerfileFragments, ImageDigest};
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
/// Methods are split into:
///
/// - **Build phase** (called by `eigenius env create`): produce a
///   pinned image from a `RuntimeEnvironment` + its constituents.
/// - **Dispatch** (called per invocation): run a script or call a
///   method, returning the outcome bundle. The runtime owns spawn,
///   lifecycle, and cleanup internally.
/// - **Image-build helper**: emit the Dockerfile fragments the
///   substrate composes into a final Dockerfile during the build phase.
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

    /// Emit the Dockerfile fragments the substrate's build pipeline
    /// composes into a final Dockerfile (D26 §9.2). Per-language
    /// fragments install the runtime, instantiate dependencies, register
    /// the mirror, and bake build-time provenance.
    ///
    /// Returning [`DockerfileFragments::default`] is acceptable for
    /// `LocalSpawner`-only deployments that never run
    /// `build_environment_image`.
    fn dockerfile_fragments(&self, env: &Resource) -> DockerfileFragments;

    /// Run a script against this runtime.
    ///
    /// The runtime owns the dispatch lifecycle: spawn (Job mode) or
    /// ensure-service (Service mode), attach the RPC channel, dispatch,
    /// capture the worker-reported metadata, clean up. Returns a
    /// [`RunOutcome`] bundling the output resource with the trace
    /// fields the substrate facade needs to assemble a
    /// `RuntimeInvocation`.
    ///
    /// Boundary-check failures (D26 §7.5) surface as
    /// [`RunError::MirrorVersionMismatch`] /
    /// [`RunError::MissingMirrorStruct`]; runtime-level exceptions as
    /// [`RunError::RuntimeError`].
    fn run_script(
        &self,
        env: &Resource,
        script: &Resource,
        inputs: &[Resource],
    ) -> Result<RunOutcome, RunError>;

    /// Call a single declared method by signature.
    ///
    /// Same shape as `run_script` but with a declared
    /// `RuntimeMethodSignature` resource instead of a script body.
    /// Sharper surface for the "library call" use case;
    /// implementations may share most of the marshalling logic with
    /// `run_script`.
    fn call_method(
        &self,
        env: &Resource,
        signature: &Resource,
        inputs: &[Resource],
    ) -> Result<RunOutcome, RunError>;
}

/// Blanket impl so `Arc<R>` is itself a `LanguageRuntime` whenever
/// `R` is. Lets a test or orchestrator hold one runtime instance
/// shared between the dispatcher (which takes ownership of a
/// `Box<dyn LanguageRuntime>`) and the caller (which keeps its own
/// reference for backend-specific lifecycle operations like
/// `JuliaLanguageRuntime::drain`). Trait methods delegate to the
/// inner `R` via `Arc::deref` — `&self` only.
impl<R: LanguageRuntime + ?Sized> LanguageRuntime for std::sync::Arc<R> {
    fn language_id(&self) -> &str {
        (**self).language_id()
    }

    fn build_environment_image(
        &self,
        env: &Resource,
        packages: &[Resource],
        mirror: Option<&Resource>,
    ) -> Result<ImageDigest, BuildError> {
        (**self).build_environment_image(env, packages, mirror)
    }

    fn dockerfile_fragments(&self, env: &Resource) -> DockerfileFragments {
        (**self).dockerfile_fragments(env)
    }

    fn run_script(
        &self,
        env: &Resource,
        script: &Resource,
        inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        (**self).run_script(env, script, inputs)
    }

    fn call_method(
        &self,
        env: &Resource,
        signature: &Resource,
        inputs: &[Resource],
    ) -> Result<RunOutcome, RunError> {
        (**self).call_method(env, signature, inputs)
    }
}
