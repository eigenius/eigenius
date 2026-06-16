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

//! D53 external-file input resolution — turn an `ingest:PinnedExternalFile`
//! input into a worker-readable materialized path.
//!
//! When a `RunRuntimeScript` (D56) names a `PinnedExternalFile` as input, the
//! substrate fetches it by `reference`, verifies the bytes hash to the node's
//! committed `content_hash` (**fail closed** — the correctness root, D53 §5),
//! and hands the worker a filesystem path. The lean provision shape (D53 §7):
//! we synthesize a small input resource carrying `ingest:materialized_path`,
//! reusing the existing CBOR input channel — no RPC/proto change.
//!
//! Phase 1 implements the `file://` backend for the **same-host** spawner: the
//! worker shares the host filesystem, so the content-verified path is handed
//! through directly. Depot-cache materialization + a read-only Docker bind-mount
//! (so a containerized worker sees the bytes at the same path) is Phase 1.5;
//! `oxen://` (per-host fetch into the depot cache) is Phase 2. Both extend
//! `resolve_and_materialize`'s internals without changing its contract — it
//! always returns a worker-readable path to content-verified bytes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

use crate::content_address::content_hash_of;
use crate::error::RunError;

const PINNED_FILE_CLASS: &str = "urn:eigenius:ingest:PinnedExternalFile";
const PROP_IS_A: &str = "urn:eigenius:core:is_a";
const PROP_REFERENCE: &str = "urn:eigenius:ingest:reference";
const PROP_CONTENT_HASH: &str = "urn:eigenius:ingest:content_hash";
const PROP_MEDIA_TYPE: &str = "urn:eigenius:ingest:media_type";
/// Property carrying the substrate-materialized path on the worker input.
pub const PROP_MATERIALIZED_PATH: &str = "urn:eigenius:ingest:materialized_path";

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI")
}

fn read_str(r: &Resource, prop: &str) -> Option<String> {
    r.get(&iri(prop)).and_then(|v| {
        v.as_str()
            .map(str::to_string)
            .or_else(|| v.as_iri_str().map(str::to_string))
    })
}

/// Whether `r` is an `ingest:PinnedExternalFile` (by its `is_a`).
pub fn is_pinned_external_file(r: &Resource) -> bool {
    match r.get(&iri(PROP_IS_A)) {
        Some(Value::Array(items)) => items
            .iter()
            .any(|v| v.as_iri_str() == Some(PINNED_FILE_CLASS)),
        Some(other) => other.as_iri_str() == Some(PINNED_FILE_CLASS),
        None => false,
    }
}

/// If `input` is a `PinnedExternalFile`, fetch + content-verify + materialize it
/// and return a synthesized worker input carrying `ingest:materialized_path`.
/// Otherwise return `input` unchanged (the ordinary chain-resident path).
///
/// `cache_root` selects the provision target (D53 §7):
/// - `Some(dir)` — materialize content-verified bytes under `dir`
///   (`<dir>/<sha256-hex>/<name>`). The caller sets this to a directory **under
///   the depot**, so the depot's read-only DooD bind-mount makes the bytes
///   visible to a containerized worker at the same path — no per-file mount.
/// - `None` — same-host spawner: the worker shares the host filesystem, so the
///   verified source path is handed through directly (no copy).
pub fn prepare_input(input: Resource, cache_root: Option<&Path>) -> Result<Resource, RunError> {
    if !is_pinned_external_file(&input) {
        return Ok(input);
    }
    let reference =
        read_str(&input, PROP_REFERENCE).ok_or_else(|| RunError::ExternalFetchFailed {
            reference: "<missing>".to_string(),
            reason: "PinnedExternalFile is missing ingest:reference".to_string(),
        })?;
    let content_hash =
        read_str(&input, PROP_CONTENT_HASH).ok_or_else(|| RunError::ExternalFetchFailed {
            reference: reference.clone(),
            reason: "PinnedExternalFile is missing ingest:content_hash".to_string(),
        })?;
    let path = resolve_and_materialize(&reference, &content_hash, cache_root)?;

    // Synthesize the worker input: keep identity + is_a + media_type, add the
    // materialized path. Reuses the CBOR input channel (D53 §7 lean provision).
    let mut out = match input.id() {
        Some(id) => Resource::new(id.clone()),
        None => Resource::new_embedded(),
    };
    if let Some(is_a) = input.get(&iri(PROP_IS_A)) {
        out.set(iri(PROP_IS_A), is_a.clone());
    }
    if let Some(mt) = input.get(&iri(PROP_MEDIA_TYPE)) {
        out.set(iri(PROP_MEDIA_TYPE), mt.clone());
    }
    out.set(
        iri(PROP_MATERIALIZED_PATH),
        Value::String(path.to_string_lossy().into_owned()),
    );
    Ok(out)
}

/// Resolve a `reference` to a worker-readable path to content-verified bytes
/// (fail closed on hash mismatch — D53 §5).
///
/// With `cache_root = Some(dir)`: if `<dir>/<sha256-hex>/<name>` already exists
/// it is returned without re-fetching (content-addressed idempotence — the
/// directory name *is* the verified hash, so presence means a prior dispatch
/// already verified these bytes; this is the win that makes a large `oxen://`
/// download a once-per-host cost). Otherwise the bytes are fetched, verified,
/// and placed atomically under the cache.
///
/// With `cache_root = None` (same-host spawner): the source bytes are fetched
/// and verified in place and the source path is returned directly.
///
/// Backends: `file://` (Phase 1). `oxen://` lands in Phase 2 and *requires* a
/// cache root — there is no node-local path to hand a worker otherwise.
pub fn resolve_and_materialize(
    reference: &str,
    content_hash: &str,
    cache_root: Option<&Path>,
) -> Result<PathBuf, RunError> {
    let basename = basename_of(reference);

    if let Some(cache) = cache_root {
        let hex = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
        let dir = cache.join(hex);
        let target = dir.join(&basename);
        // Content-addressed idempotence: a present cache entry was verified
        // when it was written, so skip re-fetch + re-verify.
        if target.exists() {
            return Ok(target);
        }
        let bytes = fetch_bytes(reference)?;
        verify(reference, content_hash, &bytes)?;
        place_in_cache(reference, &dir, &target, &bytes)?;
        Ok(target)
    } else {
        // Same-host: the source path is itself worker-readable. file:// only —
        // oxen:// has no node-local path without a cache.
        let path = source_path(reference)?;
        let bytes = std::fs::read(&path).map_err(|e| RunError::ExternalFetchFailed {
            reference: reference.to_string(),
            reason: format!("read {}: {e}", path.display()),
        })?;
        verify(reference, content_hash, &bytes)?;
        Ok(path)
    }
}

/// The node-local filesystem path of a `file://` reference. Errors for any
/// other scheme (notably `oxen://`, which has no node-local path — it must go
/// through a cache root).
fn source_path(reference: &str) -> Result<PathBuf, RunError> {
    if let Some(rest) = reference.strip_prefix("file://") {
        Ok(PathBuf::from(rest))
    } else if reference.starts_with("oxen://") {
        Err(RunError::ExternalFetchFailed {
            reference: reference.to_string(),
            reason: "oxen:// requires a depot cache root (D53 Phase 2)".to_string(),
        })
    } else {
        Err(RunError::ExternalFetchFailed {
            reference: reference.to_string(),
            reason: "unsupported reference scheme (expected file:// or oxen://)".to_string(),
        })
    }
}

/// Fetch the raw bytes of a reference into memory. `file://` reads the local
/// file; `oxen://` lands in Phase 2.
fn fetch_bytes(reference: &str) -> Result<Vec<u8>, RunError> {
    if let Some(rest) = reference.strip_prefix("file://") {
        std::fs::read(rest).map_err(|e| RunError::ExternalFetchFailed {
            reference: reference.to_string(),
            reason: format!("read {rest}: {e}"),
        })
    } else if reference.starts_with("oxen://") {
        Err(RunError::ExternalFetchFailed {
            reference: reference.to_string(),
            reason: "oxen:// backend not yet implemented (D53 Phase 2)".to_string(),
        })
    } else {
        Err(RunError::ExternalFetchFailed {
            reference: reference.to_string(),
            reason: "unsupported reference scheme (expected file:// or oxen://)".to_string(),
        })
    }
}

/// Fail closed unless `bytes` hash to `content_hash`.
fn verify(reference: &str, content_hash: &str, bytes: &[u8]) -> Result<(), RunError> {
    let got = content_hash_of(bytes);
    if got != content_hash {
        return Err(RunError::ContentHashMismatch {
            reference: reference.to_string(),
            expected: content_hash.to_string(),
            got,
        });
    }
    Ok(())
}

/// Write verified `bytes` to `target` atomically (write a unique temp file in
/// the same dir, then rename) so concurrent workers materializing the same file
/// never observe a partial write.
fn place_in_cache(
    reference: &str,
    dir: &Path,
    target: &Path,
    bytes: &[u8],
) -> Result<(), RunError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(dir).map_err(|e| RunError::ExternalFetchFailed {
        reference: reference.to_string(),
        reason: format!("create cache dir {}: {e}", dir.display()),
    })?;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".tmp.{}.{n}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| RunError::ExternalFetchFailed {
        reference: reference.to_string(),
        reason: format!("write cache temp {}: {e}", tmp.display()),
    })?;
    // Rename is atomic on the same filesystem; if a racing worker won, the
    // target already holds identical (content-addressed) bytes — either way the
    // post-condition holds.
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(_) if target.exists() => {
            let _ = std::fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(RunError::ExternalFetchFailed {
                reference: reference.to_string(),
                reason: format!("rename into cache {}: {e}", target.display()),
            })
        }
    }
}

/// The file name to use for the materialized copy — the last path segment of the
/// reference, falling back to `data` when there isn't one.
fn basename_of(reference: &str) -> String {
    let without_scheme = reference
        .strip_prefix("file://")
        .or_else(|| reference.strip_prefix("oxen://"))
        .unwrap_or(reference);
    // For oxen://repo@commit/a/b/c.csv this still yields `c.csv`.
    without_scheme
        .rsplit('/')
        .find(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "data".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    static UNIQ: AtomicU64 = AtomicU64::new(0);

    fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
        let n = UNIQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "eig_extfile_test_{}_{n}_{name}",
            std::process::id()
        ));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn fresh_cache(label: &str) -> PathBuf {
        let n = UNIQ.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!(
            "eig_extfile_cache_{}_{n}_{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn pinned(reference: &str, content_hash: &str) -> Resource {
        let mut r = Resource::new(Iri::parse("urn:eigenius:ingest:file:deadbeef").unwrap());
        r.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(PINNED_FILE_CLASS))]),
        );
        r.set(iri(PROP_REFERENCE), Value::String(reference.to_string()));
        r.set(
            iri(PROP_CONTENT_HASH),
            Value::String(content_hash.to_string()),
        );
        r.set(iri(PROP_MEDIA_TYPE), Value::String("text/csv".to_string()));
        r
    }

    #[test]
    fn same_host_verifies_and_passes_source_path_through() {
        let bytes = b"a,b\n1,2\n";
        let hash = content_hash_of(bytes);
        let path = write_temp("ok.csv", bytes);
        let node = pinned(&format!("file://{}", path.display()), &hash);

        // cache_root = None → same-host: the source path is handed through.
        let out = prepare_input(node, None).unwrap();
        let materialized = read_str(&out, PROP_MATERIALIZED_PATH).unwrap();
        assert_eq!(materialized, path.to_string_lossy());
        // media_type carried over; the node identity preserved.
        assert_eq!(read_str(&out, PROP_MEDIA_TYPE).as_deref(), Some("text/csv"));
    }

    #[test]
    fn cache_materializes_under_depot_and_is_content_addressed() {
        let bytes = b"col\n42\n";
        let hash = content_hash_of(bytes);
        let src = write_temp("dep.csv", bytes);
        let cache = fresh_cache("depot");
        let node = pinned(&format!("file://{}", src.display()), &hash);

        let out = prepare_input(node, Some(&cache)).unwrap();
        let materialized = read_str(&out, PROP_MATERIALIZED_PATH).unwrap();
        let mpath = PathBuf::from(&materialized);
        // Materialized under the cache root (so the depot bind-mount covers it).
        assert!(
            mpath.starts_with(&cache),
            "{materialized} not under {cache:?}"
        );
        // Content-addressed: the parent dir is the sha256 hex; the basename is
        // the reference's last path segment (preserved for the reader's sake).
        let hex = hash.strip_prefix("sha256:").unwrap();
        assert_eq!(mpath.parent().unwrap().file_name().unwrap(), hex);
        assert_eq!(mpath.file_name(), src.file_name());
        // Bytes copied faithfully.
        assert_eq!(std::fs::read(&mpath).unwrap(), bytes);
    }

    #[test]
    fn cache_hit_skips_refetch_after_source_removed() {
        let bytes = b"once\n";
        let hash = content_hash_of(bytes);
        let src = write_temp("vanishing.csv", bytes);
        let cache = fresh_cache("hit");
        let reference = format!("file://{}", src.display());

        // First call populates the cache.
        let first = resolve_and_materialize(&reference, &hash, Some(&cache)).unwrap();
        assert!(first.exists());
        // Remove the source: a content-addressed cache hit must not re-fetch.
        std::fs::remove_file(&src).unwrap();
        let second = resolve_and_materialize(&reference, &hash, Some(&cache)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn content_hash_mismatch_fails_closed() {
        let path = write_temp("tampered.csv", b"the real bytes\n");
        let node = pinned(&format!("file://{}", path.display()), "sha256:0000");
        match prepare_input(node, None) {
            Err(RunError::ContentHashMismatch { .. }) => {}
            other => panic!("expected ContentHashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn cache_path_also_fails_closed_on_mismatch() {
        let path = write_temp("tampered2.csv", b"real\n");
        let cache = fresh_cache("mismatch");
        let node = pinned(&format!("file://{}", path.display()), "sha256:0000");
        match prepare_input(node, Some(&cache)) {
            Err(RunError::ContentHashMismatch { .. }) => {}
            other => panic!("expected ContentHashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn non_pinned_input_passes_through() {
        let mut r = Resource::new(Iri::parse("urn:eigenius:pub:wrn:some_table").unwrap());
        r.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:pub:wrn:XenograftTable",
            ))]),
        );
        let out = prepare_input(r.clone(), None).unwrap();
        assert_eq!(out, r);
        assert!(read_str(&out, PROP_MATERIALIZED_PATH).is_none());
    }

    #[test]
    fn oxen_backend_not_yet() {
        let cache = fresh_cache("oxen");
        let node = pinned("oxen://repo@c/path.csv", "sha256:abc");
        match prepare_input(node, Some(&cache)) {
            Err(RunError::ExternalFetchFailed { reason, .. }) => {
                assert!(reason.contains("oxen"), "got: {reason}")
            }
            other => panic!("expected ExternalFetchFailed, got {other:?}"),
        }
    }

    #[test]
    fn oxen_without_cache_is_rejected() {
        let node = pinned("oxen://repo@c/path.csv", "sha256:abc");
        match prepare_input(node, None) {
            Err(RunError::ExternalFetchFailed { reason, .. }) => {
                assert!(reason.contains("cache"), "got: {reason}")
            }
            other => panic!("expected ExternalFetchFailed, got {other:?}"),
        }
    }

    #[test]
    fn missing_file_fails() {
        let node = pinned("file:///no/such/eig-extfile.csv", "sha256:abc");
        assert!(matches!(
            prepare_input(node, None),
            Err(RunError::ExternalFetchFailed { .. })
        ));
    }
}
