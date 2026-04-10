//! Uri newtype wrapping String with parsing and namespace operations.
//!
//! Part of the Eigenius Ontology Layer (§6). Uris provide typed identifiers
//! for all ontological entities (Classes, Properties, Resources). This module
//! supports URI parsing and namespace extraction.

use serde::{Serialize, Deserialize};

/// A strongly-typed URI identifier for ontological entities.
///
/// Wraps a String to provide URI-specific operations like namespace extraction
/// and parsing validation. All URIs in Eigenius are globally unique identifiers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Uri(pub String);

impl Uri {
    /// Creates a new Uri from a string.
    pub fn new(s: String) -> Self {
        Uri(s)
    }

    /// Parses a string into a Uri with validation.
    ///
    /// Validates URI format constraints. Returns None if invalid.
    pub fn parse(s: &str) -> Option<Self> {
        // Placeholder: basic non-empty check
        if s.is_empty() {
            None
        } else {
            Some(Uri(s.to_string()))
        }
    }

    /// Extracts the namespace portion of this URI.
    ///
    /// Returns the prefix up to and including the last '#' or '/',
    /// or the full URI if no namespace separator is found.
    pub fn namespace(&self) -> &str {
        if let Some(pos) = self.0.rfind('#') {
            &self.0[..=pos]
        } else if let Some(pos) = self.0.rfind('/') {
            &self.0[..=pos]
        } else {
            &self.0
        }
    }

    /// Extracts the local name portion of this URI.
    pub fn local_name(&self) -> &str {
        if let Some(pos) = self.0.rfind('#') {
            &self.0[pos + 1..]
        } else if let Some(pos) = self.0.rfind('/') {
            &self.0[pos + 1..]
        } else {
            &self.0
        }
    }
}

impl AsRef<str> for Uri {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
