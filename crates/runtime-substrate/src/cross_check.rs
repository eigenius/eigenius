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

//! Worker bootstrap cross-check (D26 §9.3).
//!
//! Two sides, one protocol:
//!
//! - **Substrate side** ([`prepare_substrate_side`]) — at spawn time the
//!   substrate writes the in-image manifest-hash file, populates the
//!   start-time env vars, and hands the env-var map to the spawner via
//!   [`crate::types::WorkerSpec::env`]. In production the file path is
//!   `/etc/eigenius-runtime-env/manifest-hash` (baked into the OCI image
//!   by [`crate::image_build`]); under [`crate::spawner::LocalSpawner`]
//!   the substrate writes it into the per-invocation tempdir and points
//!   the worker at that location via [`ENV_PROVENANCE_DIR_VAR`].
//!
//! - **Worker side** ([`verify_in_worker`]) — first thing the worker does
//!   on startup is read both env vars and the in-image file. If any
//!   value is missing or the env hash disagrees with the file hash, the
//!   worker prints a diagnostic to stderr and exits with
//!   [`EXIT_CODE_CROSS_CHECK_FAILURE`] (`78` / `EX_CONFIG`). The worker
//!   never binds its UDS in that case, so any caller observing
//!   "process exited 78" knows unambiguously that the cross-check
//!   failed.
//!
//! ## Why cross-check is mandatory
//!
//! D26 §9.3: "If the env var says digest X and the in-image
//! manifest-hash doesn't correspond to the manifest registered for that
//! digest, the worker refuses to start." Making it opt-in would mean a
//! misconfigured environment silently runs invocations against the wrong
//! manifest, producing results whose `RuntimeInvocation.image_digest`
//! does not actually witness the runtime they were produced under. This
//! is the failure the cross-check exists to prevent, so it must always
//! fire.
//!
//! ## Surfacing failures upstream
//!
//! [`is_cross_check_failure`] interprets a child's [`std::process::ExitStatus`]
//! and returns `true` for the reserved exit code. Spawner-backed callers
//! use this to surface
//! [`crate::error::SpawnError::WorkerCrossCheckFailed`] when a worker
//! dies before binding its UDS — distinguishing the cross-check failure
//! from generic spawn-time errors. Phase 18c.3 wires this into
//! [`crate::spawner::DockerSpawner`]; the helper itself is spawner-agnostic
//! and works for any backend that surfaces exit codes.

use crate::types::ImageDigest;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use thiserror::Error;

/// Env var carrying the substrate-supplied image digest the worker is
/// expected to be running under. Worker reads this and echoes it on
/// `health` for self-reporting.
pub const ENV_DIGEST_VAR: &str = "EIGENIUS_RUNTIME_ENV_DIGEST";

/// Env var carrying the substrate-supplied manifest hash. The worker
/// asserts that this matches the on-disk
/// `<prov_dir>/manifest-hash` file content.
pub const ENV_MANIFEST_HASH_VAR: &str = "EIGENIUS_RUNTIME_ENV_MANIFEST_HASH";

/// Env var that overrides the in-image provenance dir. Used by tests
/// that run workers under [`crate::spawner::LocalSpawner`] (no container,
/// so `/etc` is the host's, not the image's). Production
/// (`DockerSpawner`-backed) deployments leave this unset and the worker
/// falls back to [`DEFAULT_PROVENANCE_DIR`].
pub const ENV_PROVENANCE_DIR_VAR: &str = "EIGENIUS_RUNTIME_ENV_DIR";

/// Default in-image provenance directory (D26 §9.2). Baked into images
/// by [`crate::image_build::context`].
pub const DEFAULT_PROVENANCE_DIR: &str = "/etc/eigenius-runtime-env";

/// File within the provenance directory holding the manifest hash.
pub const MANIFEST_HASH_FILE: &str = "manifest-hash";

/// Reserved exit code the worker uses when the cross-check fails. `78`
/// is `EX_CONFIG` in `<sysexits.h>` — "configuration error", which is
/// exactly what a manifest-hash mismatch is. Choosing a documented code
/// (rather than `1`) lets the substrate distinguish cross-check failures
/// from generic worker crashes.
pub const EXIT_CODE_CROSS_CHECK_FAILURE: i32 = 78;

/// Substrate-side: assemble the cross-check env vars and write the
/// in-image manifest-hash file.
///
/// `prov_dir` is the directory where the worker will look for the
/// manifest-hash file. For production (`DockerSpawner`) this is the path
/// inside the image (`/etc/eigenius-runtime-env`), and the file is
/// already baked in at build time, so callers pass `prov_dir_action =
/// `[`ProvenanceDirAction::AssumeBaked`] to skip writing. For
/// `LocalSpawner`-backed tests, callers pass
/// [`ProvenanceDirAction::WriteFile`] and the host-visible directory
/// path; the env-var override is set so the worker reads the
/// host-visible file.
///
/// Returns the env-var entries the caller must merge into
/// [`crate::types::WorkerSpec::env`]. Caller controls whether to insert
/// or overwrite — the function does not touch any existing env state.
pub fn prepare_substrate_side(
    image_digest: &ImageDigest,
    manifest_hash: &str,
    prov_dir: &Path,
    action: ProvenanceDirAction,
) -> Result<BTreeMap<String, String>, SubstratePrepareError> {
    if manifest_hash.is_empty() {
        return Err(SubstratePrepareError::EmptyManifestHash);
    }
    let mut env = BTreeMap::new();
    env.insert(
        ENV_DIGEST_VAR.to_string(),
        image_digest.as_str().to_string(),
    );
    env.insert(ENV_MANIFEST_HASH_VAR.to_string(), manifest_hash.to_string());
    match action {
        ProvenanceDirAction::AssumeBaked => {
            // In-image file is part of the OCI layer; the worker reads
            // it from DEFAULT_PROVENANCE_DIR. No env override.
        }
        ProvenanceDirAction::WriteFile => {
            std::fs::create_dir_all(prov_dir).map_err(|e| {
                SubstratePrepareError::ProvenanceWriteFailed {
                    path: prov_dir.to_path_buf(),
                    reason: e.to_string(),
                }
            })?;
            let file = prov_dir.join(MANIFEST_HASH_FILE);
            std::fs::write(&file, manifest_hash.as_bytes()).map_err(|e| {
                SubstratePrepareError::ProvenanceWriteFailed {
                    path: file,
                    reason: e.to_string(),
                }
            })?;
            env.insert(
                ENV_PROVENANCE_DIR_VAR.to_string(),
                prov_dir.to_string_lossy().into_owned(),
            );
        }
    }
    Ok(env)
}

/// Tells [`prepare_substrate_side`] whether to write the manifest-hash
/// file or assume it's already baked into the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceDirAction {
    /// File is part of the OCI image layer (production / `DockerSpawner`).
    AssumeBaked,
    /// Substrate must write the file (test fixture / `LocalSpawner`).
    WriteFile,
}

/// Failure modes for [`prepare_substrate_side`].
#[derive(Debug, Error)]
pub enum SubstratePrepareError {
    #[error("manifest_hash must be non-empty")]
    EmptyManifestHash,
    #[error("could not write provenance file at {path}: {reason}")]
    ProvenanceWriteFailed { path: PathBuf, reason: String },
}

/// Worker-side: read the env vars and the in-image file, compare.
///
/// Pure function over the process environment and filesystem — the
/// worker calls this once at startup, before binding its UDS. On
/// success returns the verified pair; on any failure (missing env,
/// missing file, hash mismatch) returns a typed [`CrossCheckError`]
/// the worker translates into a `stderr` line + exit-78.
pub fn verify_in_worker() -> Result<CrossCheckOutcome, CrossCheckError> {
    let env_digest = std::env::var(ENV_DIGEST_VAR).map_err(|_| CrossCheckError::MissingEnvVar {
        name: ENV_DIGEST_VAR,
    })?;
    let env_manifest_hash =
        std::env::var(ENV_MANIFEST_HASH_VAR).map_err(|_| CrossCheckError::MissingEnvVar {
            name: ENV_MANIFEST_HASH_VAR,
        })?;
    let prov_dir = std::env::var(ENV_PROVENANCE_DIR_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_PROVENANCE_DIR));
    let file = prov_dir.join(MANIFEST_HASH_FILE);
    let in_image = std::fs::read_to_string(&file).map_err(|e| {
        CrossCheckError::ManifestHashFileUnreadable {
            path: file.clone(),
            reason: e.to_string(),
        }
    })?;
    let in_image = in_image.trim().to_string();
    if in_image != env_manifest_hash {
        return Err(CrossCheckError::ManifestHashMismatch {
            env_value: env_manifest_hash,
            in_image_value: in_image,
            file,
        });
    }
    Ok(CrossCheckOutcome {
        image_digest: env_digest,
        manifest_hash: env_manifest_hash,
        provenance_dir: prov_dir,
    })
}

/// Successful outcome of [`verify_in_worker`]. Carries the values the
/// worker should report on `health` requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossCheckOutcome {
    pub image_digest: String,
    pub manifest_hash: String,
    pub provenance_dir: PathBuf,
}

/// Failure modes the worker surfaces on stderr before exiting with
/// [`EXIT_CODE_CROSS_CHECK_FAILURE`].
#[derive(Debug, Error)]
pub enum CrossCheckError {
    #[error("required env var `{name}` is not set")]
    MissingEnvVar { name: &'static str },
    #[error("manifest-hash file at {path} is unreadable: {reason}")]
    ManifestHashFileUnreadable { path: PathBuf, reason: String },
    #[error(
        "manifest-hash mismatch: env `{env_value}` vs in-image `{in_image_value}` at {path}",
        path = file.display(),
    )]
    ManifestHashMismatch {
        env_value: String,
        in_image_value: String,
        file: PathBuf,
    },
}

/// Returns `true` when `status` indicates the worker exited because the
/// cross-check failed. Spawner-backed callers use this to surface
/// [`crate::error::SpawnError::WorkerCrossCheckFailed`] (D26 §11.1)
/// instead of the generic "worker died before binding UDS" diagnostic.
pub fn is_cross_check_failure(status: ExitStatus) -> bool {
    status.code() == Some(EXIT_CODE_CROSS_CHECK_FAILURE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    /// `verify_in_worker` reads from `std::env`, which is process-global —
    /// concurrent tests racing on the same env vars produce non-deterministic
    /// failures. Serialise the worker-side tests through this lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn fresh_dir(label: &str) -> PathBuf {
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("substrate-cross-check-{pid}-{label}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create dir");
        dir
    }

    fn dummy_digest() -> ImageDigest {
        ImageDigest::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("parse digest")
    }

    #[test]
    fn prepare_writes_file_and_returns_env_with_dir_override() {
        let dir = fresh_dir("write");
        let env = prepare_substrate_side(
            &dummy_digest(),
            "manifest-abc",
            &dir,
            ProvenanceDirAction::WriteFile,
        )
        .expect("prepare");
        assert_eq!(
            env.get(ENV_DIGEST_VAR).map(String::as_str),
            Some(dummy_digest().as_str())
        );
        assert_eq!(
            env.get(ENV_MANIFEST_HASH_VAR).map(String::as_str),
            Some("manifest-abc")
        );
        assert_eq!(
            env.get(ENV_PROVENANCE_DIR_VAR).map(String::as_str),
            Some(dir.to_string_lossy().as_ref())
        );
        let written =
            std::fs::read_to_string(dir.join(MANIFEST_HASH_FILE)).expect("read manifest-hash");
        assert_eq!(written, "manifest-abc");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_assume_baked_skips_file_and_dir_override() {
        let dir = fresh_dir("baked");
        let env = prepare_substrate_side(
            &dummy_digest(),
            "manifest-xyz",
            &dir,
            ProvenanceDirAction::AssumeBaked,
        )
        .expect("prepare");
        assert!(!env.contains_key(ENV_PROVENANCE_DIR_VAR));
        assert!(!dir.join(MANIFEST_HASH_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_rejects_empty_manifest_hash() {
        let dir = fresh_dir("empty-hash");
        let err = prepare_substrate_side(&dummy_digest(), "", &dir, ProvenanceDirAction::WriteFile)
            .expect_err("must reject empty hash");
        assert!(matches!(err, SubstratePrepareError::EmptyManifestHash));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn set_or_remove(name: &str, value: Option<&str>) {
        // SAFETY: tests are serialised via ENV_LOCK; std::env mutators
        // are unsafe-by-default in newer rustcs to flag this exact case.
        unsafe {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
    }

    fn with_env<R>(
        digest: Option<&str>,
        hash: Option<&str>,
        dir: Option<&Path>,
        body: impl FnOnce() -> R,
    ) -> R {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_or_remove(ENV_DIGEST_VAR, digest);
        set_or_remove(ENV_MANIFEST_HASH_VAR, hash);
        set_or_remove(
            ENV_PROVENANCE_DIR_VAR,
            dir.map(|p| p.to_str().expect("ascii test path")),
        );
        let r = body();
        set_or_remove(ENV_DIGEST_VAR, None);
        set_or_remove(ENV_MANIFEST_HASH_VAR, None);
        set_or_remove(ENV_PROVENANCE_DIR_VAR, None);
        r
    }

    #[test]
    fn verify_succeeds_when_env_and_file_match() {
        let dir = fresh_dir("match");
        std::fs::write(dir.join(MANIFEST_HASH_FILE), b"hash-1").expect("write file");
        let outcome = with_env(
            Some(dummy_digest().as_str()),
            Some("hash-1"),
            Some(&dir),
            verify_in_worker,
        )
        .expect("verify");
        assert_eq!(outcome.image_digest, dummy_digest().as_str());
        assert_eq!(outcome.manifest_hash, "hash-1");
        assert_eq!(outcome.provenance_dir, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_trims_trailing_newline_in_file() {
        let dir = fresh_dir("trim");
        std::fs::write(dir.join(MANIFEST_HASH_FILE), b"hash-2\n").expect("write file");
        let outcome = with_env(
            Some(dummy_digest().as_str()),
            Some("hash-2"),
            Some(&dir),
            verify_in_worker,
        )
        .expect("verify");
        assert_eq!(outcome.manifest_hash, "hash-2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_fails_when_env_digest_missing() {
        let dir = fresh_dir("miss-digest");
        std::fs::write(dir.join(MANIFEST_HASH_FILE), b"hash").expect("write file");
        let err = with_env(None, Some("hash"), Some(&dir), verify_in_worker)
            .expect_err("missing digest must fail");
        assert!(matches!(
            err,
            CrossCheckError::MissingEnvVar { name } if name == ENV_DIGEST_VAR
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_fails_when_env_manifest_hash_missing() {
        let dir = fresh_dir("miss-hash");
        std::fs::write(dir.join(MANIFEST_HASH_FILE), b"hash").expect("write file");
        let err = with_env(
            Some(dummy_digest().as_str()),
            None,
            Some(&dir),
            verify_in_worker,
        )
        .expect_err("missing hash must fail");
        assert!(matches!(
            err,
            CrossCheckError::MissingEnvVar { name } if name == ENV_MANIFEST_HASH_VAR
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_fails_when_in_image_file_missing() {
        let dir = fresh_dir("miss-file");
        let err = with_env(
            Some(dummy_digest().as_str()),
            Some("hash"),
            Some(&dir),
            verify_in_worker,
        )
        .expect_err("missing file must fail");
        assert!(matches!(
            err,
            CrossCheckError::ManifestHashFileUnreadable { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_fails_on_hash_mismatch() {
        let dir = fresh_dir("mismatch");
        std::fs::write(dir.join(MANIFEST_HASH_FILE), b"actual-hash").expect("write file");
        let err = with_env(
            Some(dummy_digest().as_str()),
            Some("expected-hash"),
            Some(&dir),
            verify_in_worker,
        )
        .expect_err("mismatch must fail");
        match err {
            CrossCheckError::ManifestHashMismatch {
                env_value,
                in_image_value,
                ..
            } => {
                assert_eq!(env_value, "expected-hash");
                assert_eq!(in_image_value, "actual-hash");
            }
            other => panic!("unexpected: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_cross_check_failure_recognises_exit_78() {
        use std::os::unix::process::ExitStatusExt;
        // Linux ExitStatus::from_raw expects the wait()-style word; encode
        // exit-78 by shifting into the high byte.
        let exit_78 = ExitStatus::from_raw(78 << 8);
        assert!(is_cross_check_failure(exit_78));
        let exit_1 = ExitStatus::from_raw(1 << 8);
        assert!(!is_cross_check_failure(exit_1));
        let exit_0 = ExitStatus::from_raw(0);
        assert!(!is_cross_check_failure(exit_0));
    }
}
