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

//! Shared constants for the Lean language runtime.
//!
//! These pin the contract between the Rust side (this crate, the
//! substrate, the orchestrator's napi-rs addon) and the Lean side
//! (the Lake worker landing in 20a.5b, generated EigonFFI mirrors
//! landing in 20a.6). Mirrors the shape of
//! [`eigenius_julia::conventions`](../eigenius-julia/src/conventions.rs)
//! so the two language runtimes stay structurally aligned — diffing
//! the two files highlights every spot where Lean diverges from
//! Julia.

use std::time::Duration;

/// `language_id` for [`LanguageRuntime`] dispatch and the value of
/// the `urn:eigenius:runtime:language` property on `RuntimeScript` /
/// `RuntimeEnvironment` / `RuntimeMethodSignature` resources this
/// runtime owns.
///
/// [`LanguageRuntime`]: eigenius_runtime_substrate::language_runtime::LanguageRuntime
pub const LANGUAGE: &str = "lean";

// ---------------------------------------------------------------------------
// Substrate property IRIs (language-agnostic; copied verbatim from
// the Julia runtime to keep the protocol contract stable across
// per-language crates).
// ---------------------------------------------------------------------------

/// Property IRI carrying the source string on a `RuntimeScript`
/// resource. v1 Lean dispatch into the worker takes a CBOR-encoded
/// `LeanProject` reference instead (see [`PROP_PROJECT_REF`]); this
/// constant is reserved so script-mode dispatch (a future
/// `lean exe` evaluation surface) can drop in without re-plumbing.
pub const PROP_SOURCE: &str = "urn:eigenius:runtime:source";

/// Property IRI carrying the method name on a
/// `RuntimeMethodSignature` resource — the unqualified Lean
/// declaration name the worker resolves for `CallRuntimeMethod`.
pub const PROP_METHOD_NAME: &str = "urn:eigenius:runtime:method_name";

/// Property IRI for the language tag on output resources.
pub const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";

/// Property IRI under which `run_script` records the worker's
/// captured stdout for diagnostic surfacing. v1 mirrors the Julia
/// convention; a future iteration may promote to a richer typed
/// shape once the Lean worker's natural output is settled.
pub const PROP_SCRIPT_OUTPUT: &str = "urn:eigenius:runtime:script_output";

/// Property IRI for `RuntimeEnvironment.image_digest`. The
/// orchestrator's external-institution dispatch path stamps this on
/// the synthesised env Resource so
/// `LeanLanguageRuntime::call_method` can pick the right worker
/// image without consulting the runtime's cached digest. Mirrors the
/// substrate-side constant in `crates/runtime-substrate/src/facade.rs`.
pub const PROP_IMAGE_DIGEST: &str = "urn:eigenius:runtime:image_digest";

/// Property IRI carrying the package name on a `RuntimePackage`
/// resource — matches the `name = "..."` field in the package's
/// `lakefile.lean` (`package` declaration) and is used as the
/// directory name under `/opt/eigenius/packages/<name>/` in the
/// built image.
pub const PROP_PACKAGE_NAME: &str = "urn:eigenius:runtime:package_name";

/// Property IRI carrying the verbatim `lakefile.lean` bytes on a
/// `RuntimePackage` resource. The substrate writes these bytes
/// directly into the package's directory in the build context; the
/// Lake configuration is read on `lake build` invocation, mirroring
/// how the Julia runtime relies on `Project.toml`.
pub const PROP_PACKAGE_MANIFEST: &str = "urn:eigenius:runtime:manifest";

/// Property IRI for the package's source-tree archive on a
/// `RuntimePackage` resource. Same shape as the Julia equivalent —
/// JSON array of `{path, content_base64}` records — so the substrate
/// staging code is language-agnostic.
pub const PROP_PACKAGE_SOURCE_TREE: &str = "urn:eigenius:runtime:source_tree";

// ---------------------------------------------------------------------------
// Lean-runtime-specific properties — declared in
// `ontologies/lean/lean-runtime-classes.eigon.json` and read by the
// Rust runtime + the Lake worker.
// ---------------------------------------------------------------------------

/// Property IRI on `LeanProject`: the `lakefile.lean` content (the
/// Lake project descriptor). Provided as a UTF-8 string — the worker
/// writes it to disk before invoking `lake build` + `lean4export`.
pub const PROP_LAKEFILE: &str = "urn:eigenius:lean:lakefile";

/// Property IRI on `LeanProject`: the `lake-manifest.json` content
/// (Lake's resolved dependency lockfile). Stored verbatim so the
/// build is byte-deterministic without requiring network access to
/// re-resolve the manifest at image-build time.
pub const PROP_LAKE_MANIFEST: &str = "urn:eigenius:lean:lake_manifest";

/// Property IRI on `LeanEnvironment`: SHA-256 hash of the
/// `lake-manifest.json` baked into the image, captured so re-runs
/// against the same env can verify the lockfile didn't drift between
/// image build and dispatch time.
pub const PROP_LAKE_LOCKFILE_HASH: &str = "urn:eigenius:lean:lake_lockfile_hash";

/// Property IRI on `LeanEnvironment`: the axiom allowlist applied
/// to every nanoda verification dispatched against proofs produced
/// from this environment. Defaults to Lean's four
/// trust-the-compiler axioms (`propext`, `Classical.choice`,
/// `Quot.sound`, `Lean.trustCompiler`) per D28 §7.1.
pub const PROP_LEAN_PERMITTED_AXIOMS: &str = "urn:eigenius:lean:lean_permitted_axioms";

/// Property IRI on `LeanEnvironment`: whether unpermitted axioms
/// cause hard error (`true`) or silent skipping (`false`). Mirrors
/// nanoda's `Config.unpermitted_axiom_hard_error` and defaults to
/// `true` — silent axiom skipping is a footgun for verification
/// audit chains.
pub const PROP_LEAN_UNPERMITTED_AXIOM_HARD_ERROR: &str =
    "urn:eigenius:lean:lean_unpermitted_axiom_hard_error";

/// Reference to a `LeanProject` resource on a worker dispatch
/// envelope. The `lean_export` verb takes one of these as its
/// argument — the worker resolves the referenced project, runs
/// `lake build` + `lean4export`, and returns the bytes. Reserved
/// for the 20a.5b runtime wiring; recorded here so the Lake worker
/// and the Rust dispatcher don't drift on the verb's input shape.
pub const PROP_PROJECT_REF: &str = "urn:eigenius:lean:project_ref";

/// Fully-qualified Lean module name (e.g. `TestProject.Foo`).
/// Carried on the `target_module` input Resource to the `lean_export`
/// worker verb — the dispatch ships every `call_method` input as an
/// Eigon-CBOR Resource and the Lake worker reads this property to
/// know which module to dump via `lake exe lean4export`.
pub const PROP_MODULE_NAME: &str = "urn:eigenius:lean:module_name";

/// Unqualified Lean declaration name (e.g. `foo`). Carried on the
/// `target_constant` input Resource to the `lean_export` worker
/// verb — pinning a single constant keeps the export bounded
/// (otherwise `lake exe lean4export <Module>` dumps the entire
/// transitive imported environment, which is hundreds of MB for
/// any project importing Lean stdlib).
pub const PROP_CONSTANT_NAME: &str = "urn:eigenius:lean:constant_name";

// ---------------------------------------------------------------------------
// Default policy values referenced from the Dockerfile + runtime.
// ---------------------------------------------------------------------------

/// Default axiom allowlist applied when a `LeanEnvironment` omits
/// `lean_permitted_axioms`. Matches Lean's four trust-the-compiler
/// primitives per D28 §7.1. Re-exposed here so the orchestrator's
/// env-create path doesn't duplicate the list.
pub const DEFAULT_LEAN_PERMITTED_AXIOMS: &[&str] = &[
    "propext",
    "Classical.choice",
    "Quot.sound",
    "Lean.trustCompiler",
];

// ---------------------------------------------------------------------------
// In-image paths and image-build constants — read by the worker
// bootstrap and stamped by the Dockerfile composer.
// ---------------------------------------------------------------------------

/// In-image path where `elan` installs the pinned Lean toolchain.
/// Re-exposed so the runtime can stamp PATH-equivalent diagnostics
/// without duplicating the Dockerfile composer's view of the world.
pub const ELAN_HOME: &str = "/opt/elan";

/// In-image path of the vendored `lean4export` Lake project. The
/// Dockerfile composer COPYs `lean/runtime-worker/vendor/lean4export/`
/// here and pre-builds it so first-invocation `lake exe lean4export`
/// from a staged `LeanProject` doesn't re-compile lean4export
/// (would add ~5-10 s per invocation).
///
/// `LeanProject` resources committed to the chain for verification
/// against this image reference this path as their `lean4export`
/// require dep — the orchestrator's env-create flow substitutes the
/// path into the lakefile string at commit time, mirroring how the
/// local-mode test computes an absolute path via `workspace_root`.
pub const LEAN4EXPORT_IN_IMAGE: &str = "/opt/lean4export";

/// In-image directory holding the Rust cdylib
/// (`libeigenius_lean_worker.so`) the worker binary links against.
/// Registered with the glibc dynamic linker via an
/// `ld.so.conf.d` entry + `ldconfig` so the worker binary's stale
/// host-side `DT_RUNPATH` is silently bypassed by `ld.so.cache`.
pub const WORKER_LIB_DIR: &str = "/opt/eigenius/lib";

/// In-image directory holding the hand-authored `EigeniusLeanCommon`
/// Lake package. The substrate composer COPYs the host-side
/// `lean/common/EigeniusLeanCommon/` tree here so the generated
/// mirror's lakefile (which `require`s EigeniusLeanCommon) can
/// resolve the dependency offline — the install_mirror step
/// rewrites the chain-committed git-require to a path-require
/// pointing at this location before invoking `lake build`.
pub const LEAN_COMMON_IN_IMAGE: &str = "/opt/eigenius/lean-common/EigeniusLeanCommon";

/// In-image directory the substrate composer materialises a staged
/// `LeanPackageMirror` archive into (D26 §9.2 — `COPY mirror/
/// /opt/eigenius/mirror/`). The install_mirror step `cd`s here to
/// rewrite the lakefile and run `lake build`.
pub const MIRROR_IN_IMAGE: &str = "/opt/eigenius/mirror";

/// In-image directory holding the Lake-built worker binary
/// (`lean-runtime-worker`). The worker is pre-built on the host and
/// COPY'd into the image rather than rebuilt inside the image —
/// rebuilding would require either templating the worker's
/// `lakefile.lean` with image-specific link args (the cdylib lives
/// in `target/debug/` on the host but needs `/opt/eigenius/lib/` in
/// the image) or carrying the entire Rust toolchain into the image
/// purely to link the worker. Pre-build sidesteps both.
pub const WORKER_BIN_DIR: &str = "/opt/eigenius/bin";

/// In-image absolute path of the pre-built worker binary. The image's
/// `CMD` invokes this directly; the worker reads its UDS path from
/// the `EIGENIUS_TEST_WORKER_UDS` env var the spawner sets.
pub const WORKER_BIN_PATH: &str = "/opt/eigenius/bin/lean-runtime-worker";

/// In-image path of the `ld.so.conf.d` snippet registering
/// [`WORKER_LIB_DIR`] with the glibc dynamic linker. Written by
/// `install_packages` and committed to `ld.so.cache` via the
/// subsequent `ldconfig` invocation in the same RUN step.
pub const LD_SO_CONF_PATH: &str = "/etc/ld.so.conf.d/eigenius-lean.conf";

/// Pinned Lean toolchain version baked into the image. The single
/// source of truth is
/// [`lean/runtime-worker/lean-toolchain`](../../lean/runtime-worker/lean-toolchain) —
/// elan reads that file natively for local Lake invocations, and
/// this crate's `build.rs` reads the same file at compile time and
/// emits `EIGENIUS_LEAN_TOOLCHAIN_VERSION` so the Dockerfile
/// composer (and any other Rust-side caller) sees the identical
/// version. Bumping the pinned toolchain is a one-line edit to
/// `lean-toolchain` — the next `cargo build` re-stamps this const
/// and triggers a rebuild of every downstream image.
///
/// The constraint is `>=4.0.0` (the `lean4export` JSON format
/// semver 3.1.x window).
pub const LEAN_TOOLCHAIN_VERSION: &str = env!("EIGENIUS_LEAN_TOOLCHAIN_VERSION");

// ---------------------------------------------------------------------------
// Timeouts.
// ---------------------------------------------------------------------------

/// Wall-clock deadline for the substrate to connect to the worker's
/// UDS after spawn. Covers Lake's cold-start (compile the worker
/// binary on first launch if it's not pre-built) plus container
/// startup. Lake's initial precompile is generally faster than
/// Julia's JIT precompile, but a Mathlib-shaped project pulled into
/// the env can extend the warm-up; keep the budget generous for v1.
pub const UDS_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
