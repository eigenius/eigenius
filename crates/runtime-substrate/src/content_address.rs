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

//! Content-addressed IRI minting for `RuntimeScript` resources (D26 §5.1).
//!
//! A `RuntimeScript`'s identity is a deterministic function of the fields
//! that define what it computes — `language`, `source`, `entry_point`,
//! `entry_point_signature`, and `requires_environment`. Two notebooks
//! publishing the same script body with the same declared signature and
//! environment therefore mint the same IRI, so the graph deduplicates
//! them automatically and a `RuntimeInvocation` that pins a script IRI
//! pins exactly that body+signature+environment.
//!
//! The hash is taken over a length-prefixed encoding of the fields so
//! that no concatenation of distinct field values can collide with
//! another (e.g. `("ab", "c")` and `("a", "bc")` hash differently).

use sha2::{Digest, Sha256};

/// IRI prefix for content-addressed `RuntimeScript` resources.
pub const RUNTIME_SCRIPT_IRI_PREFIX: &str = "urn:eigenius:runtime:script:";

/// IRI prefix for content-addressed `ingest:PinnedExternalFile` nodes (D53 §3).
pub const PINNED_EXTERNAL_FILE_IRI_PREFIX: &str = "urn:eigenius:ingest:file:";

/// Compute the Eigenius content hash of a byte slice — the `sha256:<64 hex>`
/// form used for `ingest:content_hash` and for content-addressed verification
/// at provision (D53 §5). This is *Eigenius's own* hash over the materialized
/// bytes, independent of any backend's internal addressing (the correctness
/// root, D53 §2).
pub fn content_hash_of(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Streaming form of [`content_hash_of`] for a file on disk — reads in fixed
/// chunks so a genome-scale matrix is never loaded into memory at once (D53 is
/// the large-data path; the whole point is to avoid buffering the bytes). Same
/// `sha256:<64 hex>` output as [`content_hash_of`] over the same bytes.
pub fn content_hash_of_file(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Mint the content-addressed IRI for a `PinnedExternalFile` from its
/// `content_hash`: `urn:eigenius:ingest:file:<64 hex>`. The file's identity *is*
/// its content, so byte-identical files converge to one node regardless of where
/// they're referenced from (D53 §3). Accepts the canonical `sha256:<hex>` form
/// or a bare `<hex>`; rejects anything that isn't 64 lowercase hex digits.
pub fn pinned_external_file_iri(content_hash: &str) -> Result<String, ContentAddressError> {
    let hex = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
    let valid = hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    if !valid {
        return Err(ContentAddressError::MalformedContentHash(
            content_hash.to_string(),
        ));
    }
    Ok(format!("{PINNED_EXTERNAL_FILE_IRI_PREFIX}{hex}"))
}

/// Error minting a content-addressed IRI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentAddressError {
    #[error("content_hash must be `sha256:<64 lowercase hex>` (or bare 64 hex), got `{0}`")]
    MalformedContentHash(String),
}

/// The defining fields of a `RuntimeScript`, in the order they feed the
/// content hash. Optional fields (`entry_point`, `entry_point_signature`)
/// are encoded as their absence-vs-presence plus value, so a top-level
/// script (no entry point) and a script that happens to declare an empty
/// entry point name mint distinct IRIs.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeScriptIdentity<'a> {
    pub language: &'a str,
    pub source: &'a str,
    pub entry_point: Option<&'a str>,
    pub entry_point_signature: Option<&'a str>,
    pub requires_environment: &'a str,
}

impl RuntimeScriptIdentity<'_> {
    /// Mint the content-addressed IRI: `urn:eigenius:runtime:script:<64 hex>`.
    pub fn content_addressed_iri(&self) -> String {
        let mut hasher = Sha256::new();
        feed(&mut hasher, b"language", self.language.as_bytes());
        feed(&mut hasher, b"source", self.source.as_bytes());
        feed_opt(&mut hasher, b"entry_point", self.entry_point);
        feed_opt(
            &mut hasher,
            b"entry_point_signature",
            self.entry_point_signature,
        );
        feed(
            &mut hasher,
            b"requires_environment",
            self.requires_environment.as_bytes(),
        );
        format!("{RUNTIME_SCRIPT_IRI_PREFIX}{:x}", hasher.finalize())
    }
}

/// Feed a (label, value) pair length-prefixed so field boundaries are
/// unambiguous.
fn feed(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_le_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

/// Feed an optional field: a single discriminant byte (0 absent / 1
/// present) followed, when present, by the length-prefixed value.
fn feed_opt(hasher: &mut Sha256, label: &[u8], value: Option<&str>) {
    match value {
        None => {
            hasher.update((label.len() as u64).to_le_bytes());
            hasher.update(label);
            hasher.update([0u8]);
        }
        Some(v) => {
            hasher.update((label.len() as u64).to_le_bytes());
            hasher.update(label);
            hasher.update([1u8]);
            hasher.update((v.len() as u64).to_le_bytes());
            hasher.update(v.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> RuntimeScriptIdentity<'static> {
        RuntimeScriptIdentity {
            language: "r",
            source: "print(1)\n",
            entry_point: None,
            entry_point_signature: None,
            requires_environment: "urn:eigenius:runtime:env:r-bioc",
        }
    }

    #[test]
    fn iri_is_deterministic_and_prefixed() {
        let a = base().content_addressed_iri();
        let b = base().content_addressed_iri();
        assert_eq!(a, b);
        assert!(a.starts_with(RUNTIME_SCRIPT_IRI_PREFIX));
        // 64 hex chars after the prefix.
        assert_eq!(a[RUNTIME_SCRIPT_IRI_PREFIX.len()..].len(), 64);
    }

    #[test]
    fn distinct_source_distinct_iri() {
        let mut other = base();
        other.source = "print(2)\n";
        assert_ne!(
            base().content_addressed_iri(),
            other.content_addressed_iri()
        );
    }

    #[test]
    fn distinct_environment_distinct_iri() {
        let mut other = base();
        other.requires_environment = "urn:eigenius:runtime:env:other";
        assert_ne!(
            base().content_addressed_iri(),
            other.content_addressed_iri()
        );
    }

    #[test]
    fn absent_vs_empty_entry_point_distinct() {
        let mut empty = base();
        empty.entry_point = Some("");
        assert_ne!(
            base().content_addressed_iri(),
            empty.content_addressed_iri()
        );
    }

    #[test]
    fn content_hash_is_sha256_prefixed_and_deterministic() {
        let h = content_hash_of(b"hello\n");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 64);
        assert_eq!(h, content_hash_of(b"hello\n"));
        assert_ne!(h, content_hash_of(b"world\n"));
    }

    #[test]
    fn content_hash_of_file_matches_in_memory() {
        let bytes = b"some\nfile\nbytes\n";
        let p = std::env::temp_dir().join(format!("eig_cah_test_{}.bin", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        assert_eq!(content_hash_of_file(&p).unwrap(), content_hash_of(bytes));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn pinned_file_iri_from_hash() {
        let h = content_hash_of(b"some bytes");
        let iri = pinned_external_file_iri(&h).unwrap();
        assert!(iri.starts_with(PINNED_EXTERNAL_FILE_IRI_PREFIX));
        // byte-identical content → same IRI, regardless of the sha256: prefix form.
        let bare = h.strip_prefix("sha256:").unwrap();
        assert_eq!(iri, pinned_external_file_iri(bare).unwrap());
    }

    #[test]
    fn pinned_file_iri_rejects_malformed() {
        assert!(pinned_external_file_iri("sha256:nothex").is_err());
        assert!(pinned_external_file_iri("sha256:ABCDEF").is_err()); // uppercase + short
        assert!(pinned_external_file_iri("").is_err());
    }

    #[test]
    fn no_field_boundary_collision() {
        let mut a = base();
        a.language = "ab";
        a.source = "c";
        let mut b = base();
        b.language = "a";
        b.source = "bc";
        assert_ne!(a.content_addressed_iri(), b.content_addressed_iri());
    }
}
