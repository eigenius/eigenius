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

//! D43 §3.3 / M3.1 — text-index analyzer pipeline.
//!
//! An [`Analyzer`] is a deterministic, owned function over a string:
//! it produces a vector of tokens in document order, with duplicates
//! preserved so the indexing pipeline can compute the per-document
//! length for BM25 normalisation (D43 §2.3 indexing pipeline).
//!
//! v1 ships two registered analyzers:
//!
//! - `"en-stem-v1"` — Unicode segmentation + lowercase + Porter
//!   English stemmer. The default for English text properties.
//! - `"en-no-stem"` — Unicode segmentation + lowercase only. Useful
//!   for proper nouns and short identifiers where stemming over-
//!   collapses distinct terms.
//!
//! Adding a new analyzer is additive — register a new ID in
//! [`registry::analyzer_for`] and the entire indexing + query
//! pipeline picks it up. Analyzer ID strings are recorded per
//! `(index, layer)` in `text_stats` (the RocksDB-backed
//! `TextStatsCbor` blob), so the query path can verify at runtime
//! that the indexed-side analyzer matches the query-side analyzer
//! for the active TextIndex Resource — defence-in-depth against
//! silent recall regressions.
//!
//! The implementation deliberately keeps the trait surface narrow.
//! The tokeniser plays no part in scoring (BM25 / chain-aware IDF
//! lives in M3.2) and no part in query parsing (M3.7 handles the
//! `TEXT_MATCH(?prop, "literal")` surface) — those concerns are
//! orthogonal and shouldn't leak into analyzer interfaces.

use rust_stemmers::{Algorithm, Stemmer};
use unicode_segmentation::UnicodeSegmentation;

/// Default analyzer identifier when an `ActiveTextIndex` omits the
/// explicit `text_analyzer` slot (D43 §3.1 — defaults to
/// `"en-stem-v1"`). Reused as the canonical id string in
/// [`registry::DEFAULT_ANALYZER`].
pub const DEFAULT_ANALYZER_ID: &str = "en-stem-v1";

/// Tokenise a string into the ordered, possibly-duplicated token
/// stream the indexing pipeline needs.
///
/// **Determinism.** Analyzers must be referentially transparent —
/// the same input string produces the same token stream every
/// time. The Roaring-bitmap-backed posting lists rely on this so
/// re-extending an idempotent `(index, layer)` pair (M2.4) yields
/// the same on-disk shape.
///
/// **Empty input.** Returning an empty `Vec` for empty input is
/// the standard contract. A document with zero tokens contributes
/// nothing to any term's posting list and has `doc_length = 0` for
/// BM25 normalisation; both fall out naturally.
pub trait Analyzer: Send + Sync {
    /// The registered identifier (e.g. `"en-stem-v1"`). Stored in
    /// `text_stats` at indexing time so query-time consumers can
    /// verify the active analyzer matches.
    fn id(&self) -> &str;

    /// Tokenise `input` into the document-ordered token stream.
    fn tokenize(&self, input: &str) -> Vec<String>;
}

// ---------------- en-stem-v1 ----------------

/// Standard English analyzer: Unicode word segmentation, lowercase,
/// Porter stemming.
///
/// Pipeline per token candidate:
///
/// 1. Unicode-segment the input into word-shaped slices via
///    `unicode-segmentation`'s `unicode_words`. This treats CJK
///    syllables, Arabic ligatures, etc. as individual word units;
///    the only `&str` slices returned are letter / digit runs.
/// 2. Lowercase each slice using Unicode's casefold mapping
///    (`str::to_lowercase`).
/// 3. Apply the Porter (English) stemmer to the lowercased form.
/// 4. Emit the result in document order, duplicates preserved.
///
/// Edge cases:
///
/// - Pure-numeric tokens (`"42"`) pass through the stemmer
///   unchanged.
/// - Tokens that contain non-letter characters (e.g. `"don't"`)
///   are emitted by `unicode_words` as a single slice; the Porter
///   stemmer handles them.
/// - Multi-codepoint emoji and punctuation between letter runs
///   split into separate tokens — desired behavior.
pub struct EnStemV1 {
    stemmer: Stemmer,
}

impl EnStemV1 {
    pub fn new() -> Self {
        Self {
            stemmer: Stemmer::create(Algorithm::English),
        }
    }
}

impl Default for EnStemV1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for EnStemV1 {
    fn id(&self) -> &str {
        "en-stem-v1"
    }

    fn tokenize(&self, input: &str) -> Vec<String> {
        let mut out = Vec::new();
        for word in input.unicode_words() {
            let lowered = word.to_lowercase();
            let stemmed = self.stemmer.stem(&lowered).into_owned();
            if !stemmed.is_empty() {
                out.push(stemmed);
            }
        }
        out
    }
}

// ---------------- en-no-stem ----------------

/// No-stem English analyzer: Unicode word segmentation + lowercase.
///
/// Useful for properties where stemming over-collapses meaningful
/// distinctions — proper nouns, identifiers, short names — at the
/// cost of recall vs. the stemmed analyzer.
pub struct EnNoStem;

impl Analyzer for EnNoStem {
    fn id(&self) -> &str {
        "en-no-stem"
    }

    fn tokenize(&self, input: &str) -> Vec<String> {
        input
            .unicode_words()
            .map(|w| w.to_lowercase())
            .filter(|w| !w.is_empty())
            .collect()
    }
}

// ---------------- Registry ----------------

/// Analyzer registry — maps an analyzer-id string to a concrete
/// implementation. Used at both index time (caller resolves the
/// active TextIndex's `text_analyzer` slot via this lookup) and
/// query time (caller resolves the active TextIndex's analyzer the
/// same way).
pub mod registry {
    use super::*;
    use std::sync::Arc;

    /// Default analyzer id when the active TextIndex omits the
    /// `text_analyzer` slot. Mirrors [`super::DEFAULT_ANALYZER_ID`].
    pub const DEFAULT_ANALYZER: &str = super::DEFAULT_ANALYZER_ID;

    /// Resolve an analyzer-id string to an `Arc<dyn Analyzer>`.
    ///
    /// Returns `None` for unknown ids — callers should treat this
    /// as a registration error (an Index Resource declares an
    /// analyzer the kernel doesn't ship). M3.5 surfaces this at
    /// Load time so the failure shows up before any indexing
    /// runs.
    pub fn analyzer_for(id: &str) -> Option<Arc<dyn Analyzer>> {
        match id {
            "en-stem-v1" => Some(Arc::new(EnStemV1::new())),
            "en-no-stem" => Some(Arc::new(EnNoStem)),
            _ => None,
        }
    }

    /// List the analyzer ids the kernel ships. Useful for
    /// diagnostics and for surface-level validation (M3.5 +
    /// M3.6).
    pub fn known_analyzers() -> &'static [&'static str] {
        &["en-stem-v1", "en-no-stem"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `en-stem-v1` lowercases, stems, and preserves document order
    /// (with duplicates for BM25 doc-length).
    #[test]
    fn en_stem_v1_lowercases_stems_preserves_order() {
        let a = EnStemV1::new();
        let tokens = a.tokenize("Running Runs Ran");
        // Porter stems "running" → "run", "runs" → "run", "ran" → "ran".
        assert_eq!(tokens, vec!["run", "run", "ran"]);
    }

    /// `en-no-stem` lowercases and segments but does not collapse
    /// morphological variants.
    #[test]
    fn en_no_stem_skips_stemming() {
        let a = EnNoStem;
        let tokens = a.tokenize("Running Runs Ran");
        assert_eq!(tokens, vec!["running", "runs", "ran"]);
    }

    /// Empty input yields an empty token stream.
    #[test]
    fn empty_input_is_empty_tokens() {
        for a in [
            &EnStemV1::new() as &dyn Analyzer,
            &EnNoStem as &dyn Analyzer,
        ] {
            assert!(a.tokenize("").is_empty());
            assert!(a.tokenize("   \t\n").is_empty());
        }
    }

    /// Unicode segmentation handles punctuation as separators
    /// (not as part of adjacent tokens).
    #[test]
    fn punctuation_splits_tokens() {
        let a = EnNoStem;
        let tokens = a.tokenize("alpha, beta; gamma.");
        assert_eq!(tokens, vec!["alpha", "beta", "gamma"]);
    }

    /// Tokens preserve case-folded form across Unicode scripts
    /// (Greek, Cyrillic, German `ß` casefold).
    #[test]
    fn unicode_lowercase_works() {
        let a = EnNoStem;
        let tokens = a.tokenize("Αλφα Πα STRASSE");
        // Greek capital alpha → small alpha; Cyrillic preserved.
        // `STRASSE` lowercases to `strasse` (the casefold of ß
        // expansion is the reverse direction, not relevant here).
        assert_eq!(tokens, vec!["αλφα", "πα", "strasse"]);
    }

    /// Repeated tokens are emitted N times — BM25 length
    /// normalisation depends on the document token count.
    #[test]
    fn duplicates_preserved_for_doc_length() {
        let a = EnNoStem;
        let tokens = a.tokenize("the the the quick");
        assert_eq!(tokens, vec!["the", "the", "the", "quick"]);
    }

    /// English stemmer collapses common morphological variants.
    #[test]
    fn en_stem_v1_collapses_variants() {
        let a = EnStemV1::new();
        // "running", "runs", "runner" all share the Porter stem "run".
        let toks = a.tokenize("running runs runner");
        assert_eq!(toks, vec!["run", "run", "runner"]);
        // Note: Porter is conservative on derivational morphology —
        // it doesn't collapse "runner" with "run", and it stems
        // "decision" → "decis" but leaves "decide" as "decid".
        // The choice is intentional: false-positive merges across
        // semantically distinct words would hurt precision more than
        // recall. v2 may add an alternate analyzer with a lemmatiser
        // for use cases where derivational stemming pays off.
    }

    /// Analyzer ids round-trip through the registry, and unknown
    /// ids return None.
    #[test]
    fn registry_lookup() {
        use super::registry::*;
        assert!(analyzer_for("en-stem-v1").is_some());
        assert!(analyzer_for("en-no-stem").is_some());
        assert!(analyzer_for("nonexistent").is_none());
        assert_eq!(DEFAULT_ANALYZER, "en-stem-v1");
        assert!(known_analyzers().contains(&"en-stem-v1"));
        assert!(known_analyzers().contains(&"en-no-stem"));
    }

    /// Analyzer id is stable across invocations and matches the
    /// registry constant — relied on by the M2.4 `text_stats`
    /// storage path to record the right id.
    #[test]
    fn analyzer_id_is_stable() {
        let a = EnStemV1::new();
        assert_eq!(a.id(), "en-stem-v1");
        assert_eq!(a.id(), a.id());
        let b = EnNoStem;
        assert_eq!(b.id(), "en-no-stem");
    }

    /// Numeric tokens pass through the pipeline; the Porter
    /// stemmer is a no-op on pure-numeric inputs.
    #[test]
    fn numeric_tokens_pass_through() {
        let a = EnStemV1::new();
        let tokens = a.tokenize("WAL truncation 42 docs");
        assert_eq!(tokens, vec!["wal", "truncat", "42", "doc"]);
    }
}
