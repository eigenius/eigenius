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

//! Per-layer **exact value index** (D65) — the read path for a declared
//! `core:ValueIndex`.
//!
//! The third declared index kind alongside [`super::TripleIndex`] (IRI-valued
//! `(p,o)→s`) and the BM25 / vector text indices: `ValueIndex` keys a target
//! Property's **normalized string value** to its subjects, for **exact** lookup
//! (`value → subjects`). It fills the gap neither of the others covers — a
//! literal/string key (a lexical form, a gene symbol, an accession): the triple
//! index skips literal-valued predicates, and BM25 is tokenised + ranked.
//!
//! Entries are keyed by the **`ValueIndex` Resource IRI** (not the target
//! Property IRI), exactly as text/vector segments are — so divergent indexing
//! configurations across branches stay storage-safe. Like the triple index, the
//! index is **pre-populated at `LayerBuilder::build`** and re-written idempotently
//! at `store_layer`; a query at head `H` looks up `(index, key)` across the DAG
//! and filters the results to layers in `H`'s chain, shadow-checking each subject
//! via the per-layer bloom cache (the same dedup mechanic `Layer::resolve` uses).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::storage::StorageError;

/// One `(index, normalized-key, subject)` entry, borrowed from a resource's
/// property value at indexing time.
#[derive(Debug, Clone, Copy)]
pub struct ValueEntry<'a> {
    /// The `core:ValueIndex` Resource IRI this entry belongs to. Entries are
    /// keyed by it (branch-safe across divergent index configurations).
    pub index: &'a Iri,
    /// The normalized property value — the exact lookup key.
    pub key: &'a str,
    /// The subject resource carrying the value.
    pub subject: &'a Iri,
}

/// Owned [`ValueEntry`] — extraction produces these (it can't borrow the layer
/// across the index write), then hands `as_borrowed()` slices to `extend_layer`.
#[derive(Debug, Clone)]
pub struct OwnedValueEntry {
    pub index: Iri,
    pub key: String,
    pub subject: Iri,
}

impl OwnedValueEntry {
    pub fn as_borrowed(&self) -> ValueEntry<'_> {
        ValueEntry {
            index: &self.index,
            key: &self.key,
            subject: &self.subject,
        }
    }
}

/// Operational counters reported by [`ValueIndex::stats`].
#[derive(Debug, Default, Clone, Copy)]
pub struct ValueIndexStats {
    /// Live entries.
    pub entries: u64,
    /// Distinct layers contributing entries.
    pub layers: u64,
    /// Cumulative `lookup` calls served.
    pub lookups: u64,
    /// Cumulative entries returned from `lookup`.
    pub entries_returned: u64,
}

/// Apply a `core:ValueNormalizer` (by its Resource IRI) to a value, producing the
/// exact lookup key (D65). Used at BOTH index time (population) and query time (the
/// caller normalizes its lookup key the same way), so they always agree. An unknown
/// normalizer falls back to `identity` (verbatim) — fail-open to exact match.
pub fn normalize_value(normalizer: &Iri, value: &str) -> String {
    match normalizer.as_str() {
        "urn:eigenius:core:normalizers:lowercase" => value.to_lowercase(),
        "urn:eigenius:core:normalizers:lowercase_trim" => value.trim().to_lowercase(),
        _ => value.to_string(), // identity + unknown
    }
}

/// Per-layer exact value index — the declared-`core:ValueIndex` read path.
pub trait ValueIndex: Send + Sync {
    /// Insert every value entry the layer defines. Called by the commit path
    /// (and the build-time pre-population) after the layer's content is
    /// materialised. Idempotent by `(layer, index, key, subject)`.
    fn extend_layer(&self, layer: &LayerId, entries: &[ValueEntry<'_>])
        -> Result<(), StorageError>;

    /// Drop every entry contributed by `layer`. Called by GC's `delete_layer`.
    /// No-op if the layer has no entries.
    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError>;

    /// Iterate `(subject, defining_layer)` for the exact key `(index, key)`,
    /// across the entire DAG. The caller filters by chain membership and
    /// shadow-checks via the per-layer bloom cache.
    fn lookup<'a>(
        &'a self,
        index: &Iri,
        key: &str,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + 'a>;

    /// Snapshot of operational counters.
    fn stats(&self) -> ValueIndexStats;
}

/// In-memory [`ValueIndex`] for tests, the in-memory bootstrap path, and the
/// `MemoryPersistentBackend` fixture. Production deployments use the
/// RocksDB-backed implementation.
///
/// Holds a forward map `(index, key) → {(subject, layer)}` for lookup and a
/// per-layer reverse list for `drop_layer`. Owned keys (not byte-encoded) — for
/// in-memory workloads clarity beats the micro-optimisation the RocksDB impl needs.
pub struct MemoryValueIndex {
    inner: RwLock<MemoryValueIndexState>,
}

#[derive(Default)]
struct MemoryValueIndexState {
    /// `(index, normalized_key) → {(subject, defining_layer)}`.
    forward: BTreeMap<(Iri, String), BTreeSet<(Iri, LayerId)>>,
    /// `layer → [(index, key, subject)]` — what to remove on `drop_layer`.
    by_layer: BTreeMap<LayerId, Vec<(Iri, String, Iri)>>,
    lookups: u64,
    entries_returned: u64,
}

impl MemoryValueIndex {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(MemoryValueIndexState::default()),
        }
    }
}

impl Default for MemoryValueIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ValueIndex for MemoryValueIndex {
    fn extend_layer(
        &self,
        layer: &LayerId,
        entries: &[ValueEntry<'_>],
    ) -> Result<(), StorageError> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut st = self.inner.write().expect("MemoryValueIndex poisoned");
        for e in entries {
            st.forward
                .entry((e.index.clone(), e.key.to_string()))
                .or_default()
                .insert((e.subject.clone(), layer.clone()));
            st.by_layer.entry(layer.clone()).or_default().push((
                e.index.clone(),
                e.key.to_string(),
                e.subject.clone(),
            ));
        }
        Ok(())
    }

    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        let mut st = self.inner.write().expect("MemoryValueIndex poisoned");
        let Some(entries) = st.by_layer.remove(layer) else {
            return Ok(());
        };
        for (index, key, subject) in entries {
            let k = (index, key);
            if let Some(set) = st.forward.get_mut(&k) {
                set.remove(&(subject, layer.clone()));
                if set.is_empty() {
                    st.forward.remove(&k);
                }
            }
        }
        Ok(())
    }

    fn lookup<'a>(
        &'a self,
        index: &Iri,
        key: &str,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + 'a> {
        let mut st = self.inner.write().expect("MemoryValueIndex poisoned");
        st.lookups += 1;
        // Materialise into a Vec: the RwLock can't be held across the iterator's
        // lifetime, and in-memory result sets are small.
        let hits: Vec<(Iri, LayerId)> = st
            .forward
            .get(&(index.clone(), key.to_string()))
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        st.entries_returned += hits.len() as u64;
        Box::new(hits.into_iter().map(Ok))
    }

    fn stats(&self) -> ValueIndexStats {
        let st = self.inner.read().expect("MemoryValueIndex poisoned");
        ValueIndexStats {
            entries: st.forward.values().map(|s| s.len() as u64).sum(),
            layers: st.by_layer.len() as u64,
            lookups: st.lookups,
            entries_returned: st.entries_returned,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }
    fn layer(b: u8) -> LayerId {
        LayerId([b; 32])
    }

    #[test]
    fn extend_lookup_and_drop() {
        let idx = MemoryValueIndex::new();
        let vi = iri("urn:eigenius:lexicon:form_index");
        let (l1, l2) = (layer(1), layer(2));
        let (e1, e2, e3) = (
            iri("urn:eigenius:wn:e_cellline"),
            iri("urn:eigenius:wn:e_cellline2"),
            iri("urn:eigenius:umls:e_cellline"),
        );

        // "cell line" defined in l1 (two entries) and l2 (one).
        idx.extend_layer(
            &l1,
            &[
                ValueEntry {
                    index: &vi,
                    key: "cell line",
                    subject: &e1,
                },
                ValueEntry {
                    index: &vi,
                    key: "cell line",
                    subject: &e2,
                },
                ValueEntry {
                    index: &vi,
                    key: "gene",
                    subject: &e1,
                },
            ],
        )
        .unwrap();
        idx.extend_layer(
            &l2,
            &[ValueEntry {
                index: &vi,
                key: "cell line",
                subject: &e3,
            }],
        )
        .unwrap();

        // Exact lookup returns all subjects + their defining layers.
        let mut hits: Vec<(Iri, LayerId)> =
            idx.lookup(&vi, "cell line").map(Result::unwrap).collect();
        hits.sort();
        let mut expected = vec![
            (e1.clone(), l1.clone()),
            (e2.clone(), l1.clone()),
            (e3.clone(), l2.clone()),
        ];
        expected.sort();
        assert_eq!(hits, expected);

        // A different key is independent; a miss is empty.
        assert_eq!(idx.lookup(&vi, "gene").count(), 1);
        assert_eq!(idx.lookup(&vi, "absent").count(), 0);

        // Dropping l1 removes only its contributions; l2's survive.
        idx.drop_layer(&l1).unwrap();
        let after: Vec<(Iri, LayerId)> = idx.lookup(&vi, "cell line").map(Result::unwrap).collect();
        assert_eq!(after, vec![(e3, l2)]);
        assert_eq!(
            idx.lookup(&vi, "gene").count(),
            0,
            "l1's `gene` entry dropped"
        );
    }

    #[test]
    fn keys_are_exact_not_tokenised_or_folded() {
        // Exact, whole-string keys: "cell line" ≠ "cell"; case is the caller's
        // (normalizer) job, not the index's — the index matches bytes.
        let idx = MemoryValueIndex::new();
        let vi = iri("urn:eigenius:lexicon:form_index");
        let e = iri("urn:eigenius:wn:e1");
        idx.extend_layer(
            &layer(1),
            &[ValueEntry {
                index: &vi,
                key: "cell line",
                subject: &e,
            }],
        )
        .unwrap();
        assert_eq!(idx.lookup(&vi, "cell").count(), 0, "no tokenisation");
        assert_eq!(
            idx.lookup(&vi, "Cell Line").count(),
            0,
            "no implicit case-folding"
        );
        assert_eq!(idx.lookup(&vi, "cell line").count(), 1);
    }
}
