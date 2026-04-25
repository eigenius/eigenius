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

//! IRI (Internationalized Resource Identifier) type for Eigenius.
//!
//! All identifiers in Eigenius are IRIs (RFC 3987). Internal identifiers
//! use the `urn:` scheme: `urn:eigenius:<namespace>:<local-name>`.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum allowed length for an IRI in characters.
const MAX_IRI_LENGTH: usize = 512;

/// Core namespace prefix — only the root layer may contain resources with this prefix.
const CORE_PREFIX: &str = "urn:eigenius:core:";

/// A validated IRI (Internationalized Resource Identifier).
///
/// IRIs are the universal identifier type in Eigenius. They wrap a `String`
/// and guarantee basic syntactic validity (non-empty, max length, scheme present).
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Iri(String);

/// Errors that can occur when parsing an IRI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IriError {
    Empty,
    TooLong { length: usize, max: usize },
    MissingScheme,
}

impl fmt::Display for IriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IriError::Empty => write!(f, "IRI must not be empty"),
            IriError::TooLong { length, max } => {
                write!(f, "IRI length {length} exceeds maximum {max}")
            }
            IriError::MissingScheme => write!(f, "IRI must contain a scheme (e.g., 'urn:')"),
        }
    }
}

impl std::error::Error for IriError {}

impl Iri {
    /// Parse and validate a string as an IRI.
    ///
    /// Validates:
    /// - Non-empty
    /// - Max 512 characters
    /// - Contains a `:` (scheme separator)
    pub fn parse(s: &str) -> Result<Self, IriError> {
        if s.is_empty() {
            return Err(IriError::Empty);
        }
        if s.len() > MAX_IRI_LENGTH {
            return Err(IriError::TooLong {
                length: s.len(),
                max: MAX_IRI_LENGTH,
            });
        }
        if !s.contains(':') {
            return Err(IriError::MissingScheme);
        }
        Ok(Iri(s.to_string()))
    }

    /// Returns the full IRI string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Extracts the namespace portion of this IRI.
    ///
    /// For URNs (`urn:eigenius:core:Class`), returns everything up to
    /// and including the last `:` → `urn:eigenius:core:`.
    pub fn namespace(&self) -> &str {
        if let Some(pos) = self.0.rfind(':') {
            &self.0[..=pos]
        } else {
            &self.0
        }
    }

    /// Extracts the local name portion of this IRI.
    ///
    /// For URNs (`urn:eigenius:core:Class`), returns the part after
    /// the last `:` → `Class`.
    pub fn local_name(&self) -> &str {
        if let Some(pos) = self.0.rfind(':') {
            &self.0[pos + 1..]
        } else {
            &self.0
        }
    }

    /// Returns true if this IRI is in the core namespace (`urn:eigenius:core:`).
    pub fn is_core(&self) -> bool {
        self.0.starts_with(CORE_PREFIX)
    }
}

impl fmt::Display for Iri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Iri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Iri(\"{}\")", self.0)
    }
}

impl AsRef<str> for Iri {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_urn() {
        let iri = Iri::parse("urn:eigenius:core:Class").unwrap();
        assert_eq!(iri.as_str(), "urn:eigenius:core:Class");
    }

    #[test]
    fn parse_valid_url() {
        let iri = Iri::parse("https://example.com/resource").unwrap();
        assert_eq!(iri.as_str(), "https://example.com/resource");
    }

    #[test]
    fn parse_empty_fails() {
        assert_eq!(Iri::parse(""), Err(IriError::Empty));
    }

    #[test]
    fn parse_too_long_fails() {
        let long = "urn:".to_string() + &"a".repeat(510);
        assert!(matches!(Iri::parse(&long), Err(IriError::TooLong { .. })));
    }

    #[test]
    fn parse_no_scheme_fails() {
        assert_eq!(Iri::parse("no-scheme-here"), Err(IriError::MissingScheme));
    }

    #[test]
    fn namespace_urn() {
        let iri = Iri::parse("urn:eigenius:core:Class").unwrap();
        assert_eq!(iri.namespace(), "urn:eigenius:core:");
    }

    #[test]
    fn local_name_urn() {
        let iri = Iri::parse("urn:eigenius:core:Class").unwrap();
        assert_eq!(iri.local_name(), "Class");
    }

    #[test]
    fn namespace_deep() {
        let iri = Iri::parse("urn:eigenius:example:animals:properties:breed").unwrap();
        assert_eq!(iri.namespace(), "urn:eigenius:example:animals:properties:");
        assert_eq!(iri.local_name(), "breed");
    }

    #[test]
    fn is_core() {
        assert!(Iri::parse("urn:eigenius:core:Class").unwrap().is_core());
        assert!(Iri::parse("urn:eigenius:core:is_a").unwrap().is_core());
        assert!(!Iri::parse("urn:eigenius:example:Dog").unwrap().is_core());
        assert!(!Iri::parse("https://example.com/foo").unwrap().is_core());
    }

    #[test]
    fn ordering() {
        let a = Iri::parse("urn:a:b").unwrap();
        let b = Iri::parse("urn:a:c").unwrap();
        assert!(a < b);
    }

    #[test]
    fn equality_and_hash() {
        use std::collections::HashSet;
        let a = Iri::parse("urn:eigenius:core:Class").unwrap();
        let b = Iri::parse("urn:eigenius:core:Class").unwrap();
        assert_eq!(a, b);
        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }
}
