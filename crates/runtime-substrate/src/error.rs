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

//! Substrate error taxonomy.
//!
//! Three error types align with the three lifecycle phases of a hosted
//! invocation: build the environment image, spawn a worker against it,
//! run the script. Variants follow D26 §11.1.

use crate::types::ImageDigest;
use thiserror::Error;

/// Failure modes for `LanguageRuntime::build_environment_image`.
#[derive(Debug, Error)]
pub enum BuildError {
    /// The build pipeline (Dockerfile compose, buildah invoke, registry push)
    /// failed. Carries the underlying diagnostic verbatim. D26 §11.1.
    #[error("environment build failed: {0}")]
    EnvironmentBuildFailed(String),

    /// A required input to the build (a `RuntimePackage` source tree, a
    /// `RuntimePackageMirror` archive) could not be resolved against the
    /// chain.
    #[error("build input unavailable: {0}")]
    BuildInputUnavailable(String),

    /// Pushing the produced image to the configured registry failed.
    #[error("registry push failed: {0}")]
    RegistryPushFailed(String),
}

/// Failure modes for `LanguageRuntime::spawn_worker` and the underlying
/// `WorkerSpawner` backend.
#[derive(Debug, Error)]
pub enum SpawnError {
    /// The image referenced by the environment is not pullable from the
    /// configured registry. D26 §11.1.
    #[error("environment image unavailable: {digest:?}: {reason}")]
    EnvironmentImageUnavailable {
        digest: Option<ImageDigest>,
        reason: String,
    },

    /// The worker's bootstrap cross-check (D26 §9.3) detected a mismatch
    /// between the substrate-supplied digest and the in-image manifest
    /// hash. The worker refused to start. D26 §11.1.
    #[error("worker cross-check failed: {0}")]
    WorkerCrossCheckFailed(String),

    /// The configured spawner backend (Local/Docker/...) failed to start
    /// the worker process for a reason not covered by the more specific
    /// variants above. Carries the backend-level diagnostic verbatim.
    #[error("spawn failed ({backend}): {reason}")]
    SpawnFailed {
        backend: &'static str,
        reason: String,
    },

    /// The orchestrator host's runtime depot bind-mount discipline
    /// (D26 §9.5) is violated — typically the well-known host path is
    /// missing or points to an unexpected inode. The substrate refuses
    /// to spawn workers in this state.
    #[error("DooD bind-mount discipline violated: {0}")]
    DepotMountViolation(String),

    /// `wait_with_timeout` was given a non-`None` timeout and the
    /// worker did not exit within it. The spawner kills the worker
    /// before returning so the caller can rely on the worker being
    /// reaped on the way out. The dispatcher maps this to
    /// [`crate::error::RunError::ResourceLimitExceeded`] with
    /// [`crate::error::ResourceLimit::WallClock`] (D26 §8.3 / §11.1).
    #[error("worker {handle_id} did not exit within {timeout_ms}ms; killed")]
    WaitTimedOut { handle_id: String, timeout_ms: u64 },
}

/// Failure modes for `LanguageRuntime::run_script` and `call_method`.
/// Subsumes the run-side error variants from D26 §11.1.
#[derive(Debug, Error)]
pub enum RunError {
    /// A class declared on the mirror's `mirrored_classes` has been
    /// redefined between the mirror's anchor layer and the
    /// invocation's claim layer — the language-side mirror struct no
    /// longer matches the kernel's class definition. D26 §7.5 / §11.1.
    #[error("mirror version mismatch: class `{class_iri}` (mirror anchor: {mirror_layer}, claim: {claim_layer})")]
    MirrorVersionMismatch {
        class_iri: String,
        mirror_layer: String,
        claim_layer: String,
    },

    /// The mirror's `source_layer` is not ancestral-to-or-equal-with
    /// the invocation's claim layer — the two layer chains are
    /// disjoint or the claim is upstream of the mirror. Distinct from
    /// `MirrorVersionMismatch` (per-class change in a related chain)
    /// because the failure mode is about chain compatibility, not a
    /// specific class. D26 §7.5.
    #[error("mirror anchor not ancestral: anchor {mirror_layer} is not an ancestor of or equal to claim {claim_layer}")]
    MirrorAnchorNotAncestral {
        mirror_layer: String,
        claim_layer: String,
    },

    /// An input class is missing from the mirror's `mirrored_classes`
    /// declaration, even when the mirror anchor itself is ancestral.
    /// D26 §7.5.
    #[error("missing mirror struct for class `{class_iri}`")]
    MissingMirrorStruct { class_iri: String },

    /// The declared `RuntimeMethodSignature` does not exist in the pinned
    /// environment, or the resolved method's argument types do not match
    /// the input mirror structs. D26 §7.5 / §11.1.
    #[error("method signature mismatch: {0}")]
    MethodSignatureMismatch(String),

    /// The hosted runtime raised an exception. The carried diagnostic
    /// preserves the language-side stack trace where available. D26 §11.1.
    #[error("runtime error: {0}")]
    RuntimeError(String),

    /// A per-invocation resource cap (wall clock or memory) was exceeded.
    /// D26 §8.3 / §11.1.
    #[error("resource limit exceeded: {limit:?}")]
    ResourceLimitExceeded { limit: ResourceLimit },

    /// The script attempted a syscall outside the allow-list, or accessed
    /// a forbidden filesystem path. D26 §8.3 / §11.1.
    #[error("sandbox violation: {0}")]
    SandboxViolation(String),

    /// The worker RPC channel (CBOR over UDS) failed mid-invocation —
    /// connection dropped, message decode error, protocol mismatch. The
    /// invocation is left in an indeterminate state and should be
    /// considered failed; subsequent calls should spawn a fresh worker.
    #[error("worker RPC failed: {0}")]
    WorkerRpcFailed(String),

    /// A worker that was running an invocation crashed before producing
    /// a result. Distinct from `WorkerRpcFailed` (transport-level) and
    /// `RuntimeError` (language-level): the worker process is gone.
    #[error("worker exited unexpectedly: {0}")]
    WorkerExited(String),

    /// A `PinnedExternalFile` input could not be fetched/located from its
    /// `reference` (D53 §5). Unreachable backend, missing file, unsupported
    /// scheme.
    #[error("external file fetch failed for `{reference}`: {reason}")]
    ExternalFetchFailed { reference: String, reason: String },

    /// A fetched `PinnedExternalFile`'s bytes did not hash to the node's
    /// committed `content_hash` — fail closed before any computation runs
    /// (D53 §5, the correctness root).
    #[error("content hash mismatch for `{reference}`: expected {expected}, got {got}")]
    ContentHashMismatch {
        reference: String,
        expected: String,
        got: String,
    },
}

/// Per-invocation resource cap categories used by `RunError::ResourceLimitExceeded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceLimit {
    /// `WorkerSpec::max_wall_time_ms` was reached.
    WallClock,
    /// `WorkerSpec::max_memory_bytes` was reached.
    Memory,
}
