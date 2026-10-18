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

//! DooD bind-mount discipline check (D26 §9.5).
//!
//! The substrate refuses to come up if the configured depot path
//! doesn't satisfy the discipline:
//!
//! 1. Path exists.
//! 2. Path is a directory.
//! 3. Path is writable by the orchestrator.
//!
//! Inode-level cross-check (parsing `/proc/self/mountinfo` and verifying
//! the bind-mount source matches the host inode of the same path) is
//! deliberately *not* in this milestone — it requires careful handling
//! of mount-namespace edge cases and benefits from a focused round of
//! its own. Tracked as a follow-up. The current check catches the
//! mistakes that show up in practice (path missing, wrong type,
//! permission misconfigured) before any worker is spawned.

use crate::error::SpawnError;
use std::path::Path;

/// Verify the depot path satisfies the DooD bind-mount discipline.
///
/// Returns [`SpawnError::DepotMountViolation`] with a precise diagnostic
/// on failure — this is the error variant the substrate logs at startup
/// when a deployment is misconfigured.
pub fn verify_depot_path(depot_path: &Path) -> Result<(), SpawnError> {
    let meta = std::fs::metadata(depot_path).map_err(|e| {
        SpawnError::DepotMountViolation(format!(
            "depot path {} cannot be stat'd: {e}",
            depot_path.display()
        ))
    })?;
    if !meta.is_dir() {
        return Err(SpawnError::DepotMountViolation(format!(
            "depot path {} exists but is not a directory",
            depot_path.display()
        )));
    }
    // Writability probe: try to create + remove a tiny temp file. The
    // PermissionDenied bit on `meta` doesn't reflect ACLs / chmod
    // bits seen across mount-namespace boundaries; an actual write is
    // the only honest test.
    let probe = depot_path.join(".eigenius-substrate-write-probe");
    let _ = std::fs::remove_file(&probe);
    std::fs::write(&probe, b"probe").map_err(|e| {
        SpawnError::DepotMountViolation(format!(
            "depot path {} is not writable: {e}",
            depot_path.display()
        ))
    })?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Verify a per-invocation tempdir is anchored under the depot path.
/// This is what makes paths the substrate hands to a worker valid in
/// both the orchestrator's and the host's filesystem view (D26 §9.5)
/// without translation — the depot bind-mount is the only place where
/// that property holds.
pub fn verify_tempdir_under_depot(tempdir: &Path, depot: &Path) -> Result<(), SpawnError> {
    // Compare canonicalised paths so symlinks within the depot still
    // satisfy the check. The depot itself must canonicalise as well
    // (it's checked separately by `verify_depot_path`).
    let depot_canonical = depot.canonicalize().map_err(|e| {
        SpawnError::DepotMountViolation(format!(
            "depot path {} could not be canonicalised: {e}",
            depot.display()
        ))
    })?;
    let tempdir_canonical = tempdir.canonicalize().map_err(|e| {
        SpawnError::DepotMountViolation(format!(
            "tempdir {} could not be canonicalised: {e}",
            tempdir.display()
        ))
    })?;
    if !tempdir_canonical.starts_with(&depot_canonical) {
        return Err(SpawnError::DepotMountViolation(format!(
            "tempdir {} is not under the depot path {}",
            tempdir.display(),
            depot.display(),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn fresh_dir(label: &str) -> PathBuf {
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("substrate-depot-{pid}-{label}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create dir");
        dir
    }

    #[test]
    fn verify_depot_path_accepts_existing_writable_directory() {
        let dir = fresh_dir("writable-dir");
        verify_depot_path(&dir).expect("ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_depot_path_rejects_missing_path() {
        let missing = std::env::temp_dir().join("substrate-depot-does-not-exist-zzzz-1234");
        let _ = std::fs::remove_dir_all(&missing);
        let err = verify_depot_path(&missing).expect_err("must reject missing");
        assert!(matches!(err, SpawnError::DepotMountViolation(_)));
    }

    #[test]
    fn verify_depot_path_rejects_file_at_path() {
        let dir = fresh_dir("file-not-dir");
        let file = dir.join("placeholder");
        std::fs::write(&file, b"x").expect("write");
        let err = verify_depot_path(&file).expect_err("must reject file");
        match err {
            SpawnError::DepotMountViolation(msg) => assert!(msg.contains("not a directory")),
            other => panic!("unexpected: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_tempdir_under_depot_accepts_inside() {
        let depot = fresh_dir("depot-inside");
        let tempdir = depot.join("inv-1");
        std::fs::create_dir_all(&tempdir).expect("create inv tempdir");
        verify_tempdir_under_depot(&tempdir, &depot).expect("ok");
        let _ = std::fs::remove_dir_all(&depot);
    }

    #[test]
    fn verify_tempdir_under_depot_rejects_outside() {
        let depot = fresh_dir("depot-outside");
        let other = fresh_dir("not-under-depot");
        let err = verify_tempdir_under_depot(&other, &depot).expect_err("must reject");
        assert!(matches!(err, SpawnError::DepotMountViolation(_)));
        let _ = std::fs::remove_dir_all(&depot);
        let _ = std::fs::remove_dir_all(&other);
    }
}
