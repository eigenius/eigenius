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
