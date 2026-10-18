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

//! Per-`(TextIndex Resource, layer)` inverted index for D43 text retrieval.
//!
//! The index stores tokenised property values per layer per active
//! `core:TextIndex` Resource, keyed by the Index's IRI so divergent
//! configurations across branches stay storage-safe (D43 §2.3 / §3.1).
//!
//! Four logical key families (D43 §2.3):
//!   text_term:<index_iri>:<term>:<layer>     →  varint(df) || roaring_bytes
//!   text_docs:<index_iri>:<layer>            →  CBOR { subjects, doc_lengths }
//!   text_stats:<index_iri>:<layer>           →  CBOR { doc_count, avg_doc_length }
//!   text_terms_layer:<layer>:<index_iri>     →  CBOR [term, ...]   (reverse for drop_layer)
//!
//! This module defines:
//! * the [`TextIndex`] trait — the storage surface D43 §2.3's read and
//!   write paths consult,
//! * the input/output value types ([`TextDoc`], [`TermHit`],
//!   [`TextDocs`], [`TextLayerStats`], [`TextIndexStats`]),
//! * the in-memory [`MemoryTextIndex`] backend, used by tests and the
//!   in-memory bootstrap path.
//!
//! The RocksDB backend lands in M2.4 as `storage/rocksdb/src/text_index.rs`.
//!
//! See `docs/design/d43-text-and-vector-retrieval.md` §2.3 and §3.1 for
//! the design and `docs/design/d43-implementation-plan.md` M2 for the
//! sequencing.

use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::storage::StorageError;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

/// One indexable document — a `(subject, tokenised-property-value)` pair
/// produced by the indexing pipeline during a layer commit.
///
/// The `tokens` slice preserves the analyzer's output in document order;
/// duplicates are retained so the indexer can compute `doc_length` (BM25
/// length normalisation) from the token count. Unique-term extraction
/// for posting lists happens inside the index implementation.
#[derive(Debug, Clone)]
pub struct TextDoc<'a> {
    pub subject: &'a Iri,
    pub tokens: &'a [String],
}

/// A per-layer posting hit returned by [`TextIndex::scan_term`].
///
/// Carries the layer that contributed the posting, the per-layer
/// document frequency for the queried term (so the chain-aware IDF
/// computation can sum without deserialising the bitmap, D43 §2.3
/// query path step 6), and the serialised bitmap of doc-ids that
/// contain the term within the layer.
///
/// Doc-ids in the bitmap are layer-local; resolving them to subject
/// IRIs requires fetching `text_docs:<index>:<layer>` via
/// [`TextIndex::get_layer_docs`].
#[derive(Debug, Clone)]
pub struct TermHit {
    pub layer: LayerId,
    pub df: u32,
    pub postings: Vec<u8>,
}

/// Per-layer per-index summary metadata. Cached at indexing time so
/// chain-aware BM25 IDF doesn't reparse the docs blob to compute the
/// per-chain `N` and `avg_doc_length`.
#[derive(Debug, Clone, Copy)]
pub struct TextLayerStats {
    pub doc_count: u32,
    pub avg_doc_length: f32,
}

/// Layer-local doc-id → subject IRI + doc_length mapping. Returned by
/// [`TextIndex::get_layer_docs`] so the scoring path can resolve hits
/// and apply BM25 length normalisation. The two arrays are parallel:
/// `subjects[i]` is the IRI for doc-id `i`, `doc_lengths[i]` is its
/// token count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDocs {
    pub subjects: Vec<Iri>,
    pub doc_lengths: Vec<u32>,
}

/// Operational counters reported by [`TextIndex::stats`]. Mirrors the
/// existing [`crate::layer::IndexStats`] shape; implementations may
/// report zero for fields they don't track.
#[derive(Debug, Default, Clone, Copy)]
pub struct TextIndexStats {
    pub indexes: u64,
    pub layers: u64,
    pub total_postings: u64,
    pub scans: u64,
}

/// Per-`(TextIndex Resource, layer)` inverted index — the storage
/// trait D43 §2.3's text retrieval path consults.
///
/// **Storage shape (per-Index, per-layer).** Posting lists embed the
/// `TextIndex` Resource IRI as the leading key segment and the
/// defining `LayerId` as a trailing key segment. A read at head `H`
/// does a global prefix scan on `text_term:<index>:<term>:` and
/// filters results to layers in `H`'s chain — same Phase 14h pattern
/// the triple index uses, with the index Resource IRI substituting
/// for the predicate.
///
/// **Atomic with `store_layer`.** RocksDB-backed implementations
/// write all four key families inside the same `WriteBatch` that
/// persists the layer's resources, blooms, and topology entries
/// (D43 §2.5). The in-memory implementation here uses its internal
/// `RwLock`.
///
/// **GC integration.** When a layer is swept, [`Self::drop_layer`]
/// removes every key contributed by `layer` via the reverse-keyed
/// `text_terms_layer:<layer>:<index>` lookup table — Phase 14h's
/// reverse-index pattern, applied per-Index.
pub trait TextIndex: Send + Sync {
    /// Insert all tokenised documents that the given layer contributes
    /// under a specific TextIndex Resource.
    ///
    /// Called by the commit path after the layer's content is
    /// materialised. The implementation:
    ///
    /// 1. Assigns per-`(index, layer)` local doc-ids to each input.
    /// 2. For each unique term across `docs`, builds a posting bitmap
    ///    of doc-ids that contain the term.
    /// 3. Records per-layer `doc_count` + `avg_doc_length` so
    ///    chain-aware BM25 IDF can be computed without re-parsing.
    /// 4. Records the term list so [`Self::drop_layer`] can enumerate
    ///    what to delete.
    ///
    /// Idempotent by `(index, layer)` — re-inserting under the same
    /// pair overwrites all four key families for that pair.
    ///
    /// `analyzer` names the tokeniser/stemmer the caller used to
    /// produce `docs[*].tokens`. The implementation records it
    /// alongside the per-layer stats so query-time consumers can
    /// verify the analyzer ID matches the active TextIndex's
    /// declared `analyzer` (D43 §2.3 — analyzer drift between index
    /// and query produces silent recall problems).
    fn extend_layer(
        &self,
        index: &Iri,
        layer: &LayerId,
        analyzer: &str,
        docs: &[TextDoc<'_>],
    ) -> Result<(), StorageError>;

    /// Drop every entry contributed by `layer` across all TextIndex
    /// Resources. Called by GC's `delete_layer`. No-op if the layer
    /// has no entries.
    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError>;

    /// Stream `TermHit`s for `(index, term)` across the entire DAG.
    ///
    /// Caller filters by chain membership and applies the BM25
    /// scoring (with chain-aware IDF) above the index. Yields one
    /// hit per layer that has at least one doc-id matching the term
    /// under the given Index.
    ///
    /// Yields `Result` per item so streaming backends can surface
    /// transient errors mid-iteration. The in-memory implementation
    /// always yields `Ok`.
    fn scan_term<'a>(
        &'a self,
        index: &Iri,
        term: &str,
    ) -> Box<dyn Iterator<Item = Result<TermHit, StorageError>> + 'a>;

    /// Get per-layer aggregate stats for a TextIndex. Used by the
    /// chain-aware BM25 IDF computation (D43 §2.3 query path step 6)
    /// to sum `N` and `avg_doc_length` across visible layers without
    /// re-parsing the docs blob.
    fn get_layer_stats(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<TextLayerStats>, StorageError>;

    /// Get the per-layer doc-id → subject IRI mapping plus per-doc
    /// lengths for BM25 length normalisation. Resolves bitmap hits
    /// returned by [`Self::scan_term`].
    fn get_layer_docs(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<TextDocs>, StorageError>;

    /// Get the analyzer ID recorded at indexing time for a
    /// `(index, layer)` pair. Used by the query path to verify that
    /// the active TextIndex Resource's declared analyzer matches
    /// the analyzer that produced the stored postings (D43 §2.3).
    fn get_layer_analyzer(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<String>, StorageError>;

    /// Compute the AND-intersection of posting lists for all
    /// `terms` under `(index, layer)`. Returns the sorted set of
    /// local doc-ids that contain every term, or an empty `Vec` if
    /// any term has no posting at this `(index, layer)` (or the
    /// term set is empty).
    ///
    /// Each backend uses its native posting representation to do
    /// the intersection (Roaring bitwise-AND for the RocksDB
    /// backend; sorted set intersection for the memory backend),
    /// so this method is significantly more efficient than the
    /// orchestrator decoding `scan_term` postings itself.
    ///
    /// Doc-ids are layer-local — callers resolve them to subject
    /// IRIs via [`Self::get_layer_docs`].
    fn intersect_layer(
        &self,
        index: &Iri,
        layer: &LayerId,
        terms: &[String],
    ) -> Result<Vec<u32>, StorageError>;

    /// Snapshot of operational counters.
    fn stats(&self) -> TextIndexStats;
}

// ---------------- MemoryTextIndex ----------------

/// In-memory [`TextIndex`] backend. Used by tests and the in-memory
/// bootstrap path. Holds the per-`(index, layer)` state as nested
/// `BTreeMap`s.
///
/// **Posting list format.** Doc-ids are serialised as a length-
/// prefixed `u32` count followed by `count × big-endian u32` ids.
/// The on-wire shape is opaque from outside the trait — callers
/// pass `postings: Vec<u8>` around without inspecting it. The
/// RocksDB backend (M2.4) substitutes Roaring bitmaps with no API
/// change.
pub struct MemoryTextIndex {
    inner: Arc<RwLock<MemoryTextIndexState>>,
}

#[derive(Default)]
struct MemoryTextIndexState {
    /// `text_term:<index>:<term>:<layer>` → posting list.
    postings: BTreeMap<(Iri, String, LayerId), TermHit>,
    /// `text_docs:<index>:<layer>` → (subjects, doc_lengths).
    docs: BTreeMap<(Iri, LayerId), TextDocs>,
    /// `text_stats:<index>:<layer>` → (doc_count, avg_doc_length).
    stats: BTreeMap<(Iri, LayerId), TextLayerStats>,
    /// Analyzer ID recorded per `(index, layer)` at indexing time —
    /// the query path verifies it matches the active TextIndex's
    /// declared analyzer.
    analyzers: BTreeMap<(Iri, LayerId), String>,
    /// `text_terms_layer:<layer>:<index>` → set of terms. Used by
    /// `drop_layer` to enumerate the term keys to delete; the inner
    /// `BTreeSet` mirrors the Phase 14h `text_terms_layer` CBOR-list
    /// value.
    terms_by_layer: BTreeMap<(LayerId, Iri), BTreeSet<String>>,
    /// Operational counter — cumulative scan_term calls served.
    scans: u64,
}

impl MemoryTextIndex {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryTextIndexState::default())),
        }
    }
}

impl Default for MemoryTextIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl TextIndex for MemoryTextIndex {
    fn extend_layer(
        &self,
        index: &Iri,
        layer: &LayerId,
        analyzer: &str,
        docs: &[TextDoc<'_>],
    ) -> Result<(), StorageError> {
        if docs.is_empty() {
            return Ok(());
        }

        let mut state = self.inner.write().expect("MemoryTextIndex poisoned");

        let n = docs.len();
        let mut subjects = Vec::with_capacity(n);
        let mut doc_lengths = Vec::with_capacity(n);
        // term → set of doc-ids containing the term.
        let mut term_postings: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();

        for (doc_id, doc) in docs.iter().enumerate() {
            subjects.push(doc.subject.clone());
            doc_lengths.push(doc.tokens.len() as u32);
            // Unique terms per doc — DF counts documents containing
            // the term, not term occurrences. Re-typing through a
            // BTreeSet de-dupes inside the doc cheaply.
            let unique: BTreeSet<&str> = doc.tokens.iter().map(|s| s.as_str()).collect();
            for term in unique {
                term_postings
                    .entry(term.to_string())
                    .or_default()
                    .insert(doc_id as u32);
            }
        }

        let avg_doc_length = if n > 0 {
            doc_lengths.iter().map(|&x| x as u64).sum::<u64>() as f32 / n as f32
        } else {
            0.0
        };

        // Record terms first so drop_layer can enumerate them on
        // overwrite/teardown.
        let terms_for_reverse: BTreeSet<String> = term_postings.keys().cloned().collect();

        // Idempotency: clear any prior entries for this (index, layer)
        // pair before inserting the new ones.
        if let Some(prev_terms) = state.terms_by_layer.remove(&(layer.clone(), index.clone())) {
            for prev_term in prev_terms {
                state
                    .postings
                    .remove(&(index.clone(), prev_term, layer.clone()));
            }
        }
        state.docs.remove(&(index.clone(), layer.clone()));
        state.stats.remove(&(index.clone(), layer.clone()));
        state.analyzers.remove(&(index.clone(), layer.clone()));

        // Insert fresh.
        for (term, doc_set) in term_postings {
            let df = doc_set.len() as u32;
            let postings = encode_doc_set(&doc_set);
            let hit = TermHit {
                layer: layer.clone(),
                df,
                postings,
            };
            state
                .postings
                .insert((index.clone(), term, layer.clone()), hit);
        }
        state.docs.insert(
            (index.clone(), layer.clone()),
            TextDocs {
                subjects,
                doc_lengths,
            },
        );
        state.stats.insert(
            (index.clone(), layer.clone()),
            TextLayerStats {
                doc_count: n as u32,
                avg_doc_length,
            },
        );
        state
            .analyzers
            .insert((index.clone(), layer.clone()), analyzer.to_string());
        state
            .terms_by_layer
            .insert((layer.clone(), index.clone()), terms_for_reverse);

        Ok(())
    }

    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        let mut state = self.inner.write().expect("MemoryTextIndex poisoned");

        // Phase 14h reverse-index walk: enumerate every Index that
        // contributed at this layer, then for each, delete the four
        // key families.
        let index_keys: Vec<(LayerId, Iri)> = state
            .terms_by_layer
            .keys()
            .filter(|(l, _)| l == layer)
            .cloned()
            .collect();

        for (l, index) in index_keys {
            if let Some(terms) = state.terms_by_layer.remove(&(l.clone(), index.clone())) {
                for term in terms {
                    state.postings.remove(&(index.clone(), term, l.clone()));
                }
            }
            state.docs.remove(&(index.clone(), l.clone()));
            state.stats.remove(&(index.clone(), l.clone()));
            state.analyzers.remove(&(index.clone(), l.clone()));
        }

        Ok(())
    }

    fn scan_term<'a>(
        &'a self,
        index: &Iri,
        term: &str,
    ) -> Box<dyn Iterator<Item = Result<TermHit, StorageError>> + 'a> {
        let term_owned = term.to_string();
        let mut state = self.inner.write().expect("MemoryTextIndex poisoned");
        state.scans += 1;

        // Collect all (index, term, *) entries. The Memory backend
        // can't do a true prefix scan on the BTreeMap key tuple
        // because the term sits between the index and layer
        // segments, so we filter linearly. Real BTreeMap range +
        // skip-out semantics arrive with the RocksDB backend in
        // M2.4 (where the key is byte-encoded and prefix scans are
        // O(matches)).
        let results: Vec<TermHit> = state
            .postings
            .iter()
            .filter(|((idx, t, _), _)| idx == index && t == &term_owned)
            .map(|(_, hit)| hit.clone())
            .collect();

        Box::new(results.into_iter().map(Ok))
    }

    fn get_layer_stats(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<TextLayerStats>, StorageError> {
        let state = self.inner.read().expect("MemoryTextIndex poisoned");
        Ok(state.stats.get(&(index.clone(), layer.clone())).copied())
    }

    fn get_layer_docs(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<TextDocs>, StorageError> {
        let state = self.inner.read().expect("MemoryTextIndex poisoned");
        Ok(state.docs.get(&(index.clone(), layer.clone())).cloned())
    }

    fn get_layer_analyzer(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<String>, StorageError> {
        let state = self.inner.read().expect("MemoryTextIndex poisoned");
        Ok(state
            .analyzers
            .get(&(index.clone(), layer.clone()))
            .cloned())
    }

    fn intersect_layer(
        &self,
        index: &Iri,
        layer: &LayerId,
        terms: &[String],
    ) -> Result<Vec<u32>, StorageError> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let state = self.inner.read().expect("MemoryTextIndex poisoned");

        // Memory backend's posting format is the length-prefixed u32
        // encoding from `encode_doc_set` / `decode_doc_set`. Decode
        // the first term into a `BTreeSet<u32>` and intersect against
        // subsequent terms in sorted-set form.
        let mut accumulator: BTreeSet<u32> =
            match state
                .postings
                .get(&(index.clone(), terms[0].clone(), layer.clone()))
            {
                Some(hit) => decode_doc_set(&hit.postings).map_err(|e| {
                    StorageError::Internal(format!(
                        "intersect_layer decode (term {}): {e}",
                        terms[0]
                    ))
                })?,
                None => return Ok(Vec::new()),
            };

        for term in &terms[1..] {
            if accumulator.is_empty() {
                break;
            }
            match state
                .postings
                .get(&(index.clone(), term.clone(), layer.clone()))
            {
                Some(hit) => {
                    let other = decode_doc_set(&hit.postings).map_err(|e| {
                        StorageError::Internal(format!("intersect_layer decode (term {term}): {e}"))
                    })?;
                    accumulator = accumulator.intersection(&other).copied().collect();
                }
                None => return Ok(Vec::new()),
            }
        }

        Ok(accumulator.into_iter().collect())
    }

    fn stats(&self) -> TextIndexStats {
        let state = self.inner.read().expect("MemoryTextIndex poisoned");
        let layers: BTreeSet<&LayerId> = state.docs.keys().map(|(_, l)| l).collect();
        let indexes: BTreeSet<&Iri> = state.docs.keys().map(|(i, _)| i).collect();
        TextIndexStats {
            indexes: indexes.len() as u64,
            layers: layers.len() as u64,
            total_postings: state.postings.len() as u64,
            scans: state.scans,
        }
    }
}

// ---------------- Posting-list encoding ----------------

/// Encode a sorted set of doc-ids as `count: u32 BE || ids: u32 BE × count`.
///
/// The in-memory backend uses this directly; the RocksDB backend
/// (M2.4) substitutes Roaring bitmap bytes. Callers don't inspect the
/// bytes — they round-trip through [`decode_doc_set`] when they need
/// to resolve doc-ids.
pub fn encode_doc_set(set: &BTreeSet<u32>) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + set.len() * 4);
    out.extend_from_slice(&(set.len() as u32).to_be_bytes());
    for &doc_id in set {
        out.extend_from_slice(&doc_id.to_be_bytes());
    }
    out
}

/// Inverse of [`encode_doc_set`]. Returns the decoded set or a
/// human-readable error string.
pub fn decode_doc_set(bytes: &[u8]) -> Result<BTreeSet<u32>, String> {
    if bytes.len() < 4 {
        return Err(format!("truncated header at {} bytes", bytes.len()));
    }
    let n = u32::from_be_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let expected = 4 + n * 4;
    if bytes.len() != expected {
        return Err(format!(
            "wrong length: got {}, expected {} for n={n}",
            bytes.len(),
            expected
        ));
    }
    let mut set = BTreeSet::new();
    for i in 0..n {
        let off = 4 + i * 4;
        set.insert(u32::from_be_bytes(bytes[off..off + 4].try_into().unwrap()));
    }
    Ok(set)
}

// ---------------- Tests ----------------

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn layer_id(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }

    fn tokens(s: &str) -> Vec<String> {
        s.split_whitespace().map(|w| w.to_string()).collect()
    }

    /// Round-trip: extend a layer under an index, scan_term returns
    /// the expected postings and df values.
    #[test]
    fn extend_then_scan_term_returns_expected_postings() {
        let idx = MemoryTextIndex::new();
        let i1 = iri("urn:eigenius:test:text_index_1");
        let l1 = layer_id(1);
        let s_a = iri("urn:eigenius:test:resource_a");
        let s_b = iri("urn:eigenius:test:resource_b");

        let toks_a = tokens("wal truncation under concurrent commit");
        let toks_b = tokens("rolling back a partial commit");
        let docs = vec![
            TextDoc {
                subject: &s_a,
                tokens: &toks_a,
            },
            TextDoc {
                subject: &s_b,
                tokens: &toks_b,
            },
        ];
        idx.extend_layer(&i1, &l1, "en-stem-v1", &docs).unwrap();

        // "commit" appears in both docs → df=2.
        let hits: Vec<TermHit> = idx.scan_term(&i1, "commit").map(|r| r.unwrap()).collect();
        assert_eq!(hits.len(), 1, "one layer contributes");
        assert_eq!(hits[0].layer, l1);
        assert_eq!(hits[0].df, 2);
        let doc_set = decode_doc_set(&hits[0].postings).unwrap();
        assert_eq!(doc_set, BTreeSet::from([0, 1]));

        // "wal" appears in doc 0 only → df=1.
        let hits: Vec<TermHit> = idx.scan_term(&i1, "wal").map(|r| r.unwrap()).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].df, 1);
        let doc_set = decode_doc_set(&hits[0].postings).unwrap();
        assert_eq!(doc_set, BTreeSet::from([0]));

        // "nonexistent" appears nowhere.
        let hits: Vec<TermHit> = idx
            .scan_term(&i1, "nonexistent")
            .map(|r| r.unwrap())
            .collect();
        assert!(hits.is_empty());
    }

    /// Multiple layers under the same Index contribute independent
    /// posting lists; scan_term yields one TermHit per contributing
    /// layer.
    #[test]
    fn multiple_layers_under_same_index_yield_separate_postings() {
        let idx = MemoryTextIndex::new();
        let i1 = iri("urn:eigenius:test:text_index_1");
        let l1 = layer_id(1);
        let l2 = layer_id(2);

        let s_a = iri("urn:eigenius:test:a");
        let s_b = iri("urn:eigenius:test:b");

        let toks_l1 = tokens("alpha beta");
        let toks_l2 = tokens("beta gamma");

        idx.extend_layer(
            &i1,
            &l1,
            "en-stem-v1",
            &[TextDoc {
                subject: &s_a,
                tokens: &toks_l1,
            }],
        )
        .unwrap();
        idx.extend_layer(
            &i1,
            &l2,
            "en-stem-v1",
            &[TextDoc {
                subject: &s_b,
                tokens: &toks_l2,
            }],
        )
        .unwrap();

        // "beta" appears in both layers.
        let hits: Vec<TermHit> = idx.scan_term(&i1, "beta").map(|r| r.unwrap()).collect();
        assert_eq!(hits.len(), 2);
        let layers: BTreeSet<LayerId> = hits.iter().map(|h| h.layer.clone()).collect();
        assert_eq!(layers, BTreeSet::from([l1.clone(), l2.clone()]));

        // df is per-layer; both equal 1 because each layer has one doc.
        for h in &hits {
            assert_eq!(h.df, 1);
        }

        // "alpha" appears only in L1.
        let hits: Vec<TermHit> = idx.scan_term(&i1, "alpha").map(|r| r.unwrap()).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].layer, l1);
    }

    /// Two different TextIndex Resources targeting the same content
    /// have separately-addressable postings — the divergent-Index
    /// cross-chain story (D43 §3.1).
    #[test]
    fn multiple_indexes_keep_separate_postings() {
        let idx = MemoryTextIndex::new();
        let i1 = iri("urn:eigenius:test:text_index_v1");
        let i2 = iri("urn:eigenius:test:text_index_v2");
        let l1 = layer_id(1);
        let s = iri("urn:eigenius:test:s");
        let toks = tokens("foo bar");

        let docs = [TextDoc {
            subject: &s,
            tokens: &toks,
        }];
        idx.extend_layer(&i1, &l1, "en-stem-v1", &docs).unwrap();
        idx.extend_layer(&i2, &l1, "en-no-stem", &docs).unwrap();

        // Each index has its own posting for "foo".
        let hits_i1: Vec<TermHit> = idx.scan_term(&i1, "foo").map(|r| r.unwrap()).collect();
        let hits_i2: Vec<TermHit> = idx.scan_term(&i2, "foo").map(|r| r.unwrap()).collect();
        assert_eq!(hits_i1.len(), 1);
        assert_eq!(hits_i2.len(), 1);

        // Analyzers recorded independently.
        assert_eq!(
            idx.get_layer_analyzer(&i1, &l1).unwrap().as_deref(),
            Some("en-stem-v1")
        );
        assert_eq!(
            idx.get_layer_analyzer(&i2, &l1).unwrap().as_deref(),
            Some("en-no-stem")
        );
    }

    /// drop_layer removes every key family for that layer across all
    /// TextIndex Resources. After drop, scans return nothing and
    /// get_layer_* return None.
    #[test]
    fn drop_layer_removes_all_keys() {
        let idx = MemoryTextIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let i2 = iri("urn:eigenius:test:i2");
        let l1 = layer_id(1);
        let l2 = layer_id(2);
        let s = iri("urn:eigenius:test:s");
        let toks = tokens("alpha beta");
        let docs = [TextDoc {
            subject: &s,
            tokens: &toks,
        }];

        // Two indexes at L1; one index at L2.
        idx.extend_layer(&i1, &l1, "en-stem-v1", &docs).unwrap();
        idx.extend_layer(&i2, &l1, "en-stem-v1", &docs).unwrap();
        idx.extend_layer(&i1, &l2, "en-stem-v1", &docs).unwrap();

        idx.drop_layer(&l1).unwrap();

        // L1 entries gone for both indexes.
        assert!(idx.scan_term(&i1, "alpha").all(|r| r.unwrap().layer != l1));
        assert!(idx.scan_term(&i2, "alpha").all(|r| r.unwrap().layer != l1));
        assert!(idx.get_layer_stats(&i1, &l1).unwrap().is_none());
        assert!(idx.get_layer_docs(&i1, &l1).unwrap().is_none());
        assert!(idx.get_layer_analyzer(&i1, &l1).unwrap().is_none());
        assert!(idx.get_layer_stats(&i2, &l1).unwrap().is_none());

        // L2 entries untouched.
        assert!(idx.get_layer_stats(&i1, &l2).unwrap().is_some());
        let l2_hits: Vec<TermHit> = idx.scan_term(&i1, "alpha").map(|r| r.unwrap()).collect();
        assert_eq!(l2_hits.len(), 1);
        assert_eq!(l2_hits[0].layer, l2);
    }

    /// Re-extending the same `(index, layer)` overwrites the prior
    /// contribution rather than accumulating.
    #[test]
    fn extend_layer_is_idempotent_per_pair() {
        let idx = MemoryTextIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let s = iri("urn:eigenius:test:s");

        let toks_v1 = tokens("old content");
        idx.extend_layer(
            &i1,
            &l1,
            "en-stem-v1",
            &[TextDoc {
                subject: &s,
                tokens: &toks_v1,
            }],
        )
        .unwrap();
        assert_eq!(idx.scan_term(&i1, "old").count(), 1);

        let toks_v2 = tokens("new content");
        idx.extend_layer(
            &i1,
            &l1,
            "en-stem-v1",
            &[TextDoc {
                subject: &s,
                tokens: &toks_v2,
            }],
        )
        .unwrap();

        // "old" no longer appears.
        assert_eq!(idx.scan_term(&i1, "old").count(), 0);
        // "new" appears.
        assert_eq!(idx.scan_term(&i1, "new").count(), 1);
    }

    /// Per-layer stats record doc_count and avg_doc_length correctly;
    /// docs map preserves parallel arrays of subjects and lengths.
    #[test]
    fn layer_stats_and_docs_recorded() {
        let idx = MemoryTextIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let s_a = iri("urn:eigenius:test:a");
        let s_b = iri("urn:eigenius:test:b");

        // doc a has 3 tokens, doc b has 5; avg = 4.
        let toks_a = tokens("one two three");
        let toks_b = tokens("alpha beta gamma delta epsilon");
        idx.extend_layer(
            &i1,
            &l1,
            "en-stem-v1",
            &[
                TextDoc {
                    subject: &s_a,
                    tokens: &toks_a,
                },
                TextDoc {
                    subject: &s_b,
                    tokens: &toks_b,
                },
            ],
        )
        .unwrap();

        let stats = idx.get_layer_stats(&i1, &l1).unwrap().unwrap();
        assert_eq!(stats.doc_count, 2);
        assert!((stats.avg_doc_length - 4.0).abs() < f32::EPSILON);

        let docs = idx.get_layer_docs(&i1, &l1).unwrap().unwrap();
        assert_eq!(docs.subjects, vec![s_a, s_b]);
        assert_eq!(docs.doc_lengths, vec![3, 5]);
    }

    /// Empty doc list is a no-op (no keys written; stats remain default).
    #[test]
    fn empty_docs_is_noop() {
        let idx = MemoryTextIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        idx.extend_layer(&i1, &l1, "en-stem-v1", &[]).unwrap();
        assert!(idx.get_layer_stats(&i1, &l1).unwrap().is_none());
        assert_eq!(idx.stats().total_postings, 0);
    }

    /// Operational counters: stats() reflects indexes, layers,
    /// total_postings, and the cumulative scan count.
    #[test]
    fn stats_count_reflects_state() {
        let idx = MemoryTextIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let i2 = iri("urn:eigenius:test:i2");
        let l1 = layer_id(1);
        let s = iri("urn:eigenius:test:s");
        let toks = tokens("alpha beta gamma");
        let docs = [TextDoc {
            subject: &s,
            tokens: &toks,
        }];

        idx.extend_layer(&i1, &l1, "en-stem-v1", &docs).unwrap();
        idx.extend_layer(&i2, &l1, "en-stem-v1", &docs).unwrap();
        // Each (index, l1) contributes 3 terms → 6 total postings.
        let s1 = idx.stats();
        assert_eq!(s1.indexes, 2);
        assert_eq!(s1.layers, 1);
        assert_eq!(s1.total_postings, 6);
        assert_eq!(s1.scans, 0);

        // Issue some scans; the counter accumulates.
        let _ = idx.scan_term(&i1, "alpha").count();
        let _ = idx.scan_term(&i2, "alpha").count();
        let _ = idx.scan_term(&i1, "missing").count();
        let s2 = idx.stats();
        assert_eq!(s2.scans, 3);
    }

    /// Posting-list encoding round-trips via the public helpers.
    #[test]
    fn encode_decode_doc_set_round_trip() {
        let cases = [
            BTreeSet::<u32>::new(),
            BTreeSet::from([0]),
            BTreeSet::from([1, 7, 42, u32::MAX]),
        ];
        for case in cases {
            let bytes = encode_doc_set(&case);
            let decoded = decode_doc_set(&bytes).unwrap();
            assert_eq!(decoded, case);
        }
    }

    #[test]
    fn decode_doc_set_rejects_short_input() {
        assert!(decode_doc_set(&[]).is_err());
        assert!(decode_doc_set(&[0, 0, 0, 1]).is_err()); // expects 1 id, none provided
    }
}
