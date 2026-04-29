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

//! Per-layer triple index for EigenQL read acceleration (Phase 14h / D23 §5.9).
//!
//! The index stores `(predicate, object, subject, layer)` tuples for every
//! IRI-valued property in every layer. EigenQL patterns of the shape
//! `MATCH ?x : Class { is_a = Class }` (where the predicate's `data_type`
//! is `resource` or `resource_array`) become a single prefix scan against
//! the index, deduplicated against the head's chain via per-layer blooms.
//!
//! Two physical orderings persist:
//! - `idx_pos:<p>:<o>:<s>:<layer>` — read path (one prefix scan per query)
//! - `idx_layer:<layer>:<p>:<o>:<s>` — GC path (one prefix scan per layer drop)
//!
//! Both use length-prefixed keys (4-byte big-endian `u32` length followed
//! by raw bytes per IRI segment; layer is a fixed 32 bytes). The encoder
//! lives in [`index_keys`].
//!
//! See `docs/design/phase-14h-indexed-reads.md` for the full design.

use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::storage::StorageError;
use std::collections::BTreeSet;
use std::sync::RwLock;

/// A single subject-predicate-object triple, borrowed from a `Resource`'s
/// property values at indexing time. All three positions are IRIs in v1
/// (literal-valued properties are skipped — see the indexability rule in
/// the design doc).
#[derive(Debug, Clone, Copy)]
pub struct Triple<'a> {
    pub subject: &'a Iri,
    pub predicate: &'a Iri,
    pub object: &'a Iri,
}

/// Counters reported by [`TripleIndex::stats`]. Implementations may report
/// zero for fields they don't track.
#[derive(Debug, Default, Clone, Copy)]
pub struct IndexStats {
    /// Live triples (sum of `idx_pos:` entries).
    pub triples: u64,
    /// Distinct layers contributing entries.
    pub layers: u64,
    /// Total `scan_predicate_object` calls served (cumulative).
    pub scans: u64,
    /// Cumulative entries returned from `scan_predicate_object`.
    pub entries_returned: u64,
}

/// Per-layer triple index — the storage trait Phase 14h's read path
/// consults.
///
/// **Storage shape (per-layer, globally scannable).** Index entries embed
/// the defining `LayerId` as the trailing key segment of the forward
/// (`idx_pos`) ordering and as the leading segment of the reverse
/// (`idx_layer`) ordering. A query at head `H` does one global prefix scan
/// on `(predicate, object)`, filters results to layers in `H`'s chain,
/// and shadow-checks each surviving subject against the per-layer blooms
/// — same dedup mechanic `Layer::resolve` already uses.
///
/// **Atomic with `store_layer`.** RocksDB-backed implementations write
/// index entries inside the same `WriteBatch` that persists the layer's
/// resources, blooms, and topology — partial drift is impossible.
/// In-memory implementations write under their existing lock.
///
/// **GC integration.** When a layer is swept (Phase 14f), `drop_layer`
/// removes both orderings' entries for that layer in one atomic operation.
pub trait TripleIndex: Send + Sync {
    /// Insert all triples that the given layer defines. Called by the
    /// commit path after the layer's content is materialised. Idempotent
    /// by `(layer, p, o, s)` — re-inserting a triple is a no-op.
    fn extend_layer(&self, layer: &LayerId, triples: &[Triple<'_>]) -> Result<(), StorageError>;

    /// Drop every entry contributed by `layer` from both orderings.
    /// Called by GC's `delete_layer`. No-op if the layer has no entries.
    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError>;

    /// Iterate `(subject, defining_layer)` pairs matching `(p, o)`,
    /// across the entire DAG. Caller filters by chain membership and
    /// shadow-checks via the per-layer bloom cache.
    ///
    /// Yields `Result` per item so streaming backends can surface
    /// transient errors mid-iteration. The in-memory implementation
    /// always yields `Ok`.
    fn scan_predicate_object<'a>(
        &'a self,
        p: &Iri,
        o: &Iri,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + 'a>;

    /// Snapshot of operational counters.
    fn stats(&self) -> IndexStats;
}

/// In-memory `TripleIndex` for tests, the in-memory bootstrap path, and
/// the `MemoryPersistentBackend` fixture. Production deployments use
/// the RocksDB-backed implementation that lands in commit 2.
///
/// Stores both orderings as sorted `BTreeSet<Vec<u8>>` of length-prefixed
/// keys. `scan_predicate_object` materialises matching entries into a
/// `Vec` because the inner `RwLock` can't be held across the iterator's
/// lifetime; for in-memory workloads the materialisation cost is
/// negligible.
pub struct MemoryTripleIndex {
    inner: RwLock<MemoryTripleIndexState>,
}

struct MemoryTripleIndexState {
    /// Forward keys: `pos_key(p, o, s, layer)`.
    pos: BTreeSet<Vec<u8>>,
    /// Reverse keys: `layer_key(layer, p, o, s)`.
    layer: BTreeSet<Vec<u8>>,
    /// Distinct layers represented in the index.
    layers: BTreeSet<LayerId>,
    /// Cumulative scan + return counters.
    scans: u64,
    entries_returned: u64,
}

impl MemoryTripleIndex {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(MemoryTripleIndexState {
                pos: BTreeSet::new(),
                layer: BTreeSet::new(),
                layers: BTreeSet::new(),
                scans: 0,
                entries_returned: 0,
            }),
        }
    }
}

impl Default for MemoryTripleIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl TripleIndex for MemoryTripleIndex {
    fn extend_layer(&self, layer: &LayerId, triples: &[Triple<'_>]) -> Result<(), StorageError> {
        if triples.is_empty() {
            return Ok(());
        }
        let mut state = self.inner.write().expect("MemoryTripleIndex poisoned");
        state.layers.insert(layer.clone());
        for t in triples {
            let pos = index_keys::pos_key(t.predicate, t.object, t.subject, layer);
            let lay = index_keys::layer_key(layer, t.predicate, t.object, t.subject);
            state.pos.insert(pos);
            state.layer.insert(lay);
        }
        Ok(())
    }

    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        let mut state = self.inner.write().expect("MemoryTripleIndex poisoned");

        // Walk the reverse index for this layer to find every (p, o, s)
        // it contributed; remove the matching forward entries; then
        // remove the reverse entries themselves.
        let prefix = index_keys::layer_prefix(layer);
        let to_remove: Vec<Vec<u8>> = state
            .layer
            .range(prefix.clone()..)
            .take_while(|k| k.starts_with(&prefix))
            .cloned()
            .collect();

        for lay_key in &to_remove {
            let (p, o, s) = index_keys::decode_layer_key(lay_key)
                .expect("MemoryTripleIndex stored a malformed reverse key");
            let pos_key = index_keys::pos_key(&p, &o, &s, layer);
            state.pos.remove(&pos_key);
            state.layer.remove(lay_key);
        }
        state.layers.remove(layer);
        Ok(())
    }

    fn scan_predicate_object<'a>(
        &'a self,
        p: &Iri,
        o: &Iri,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + 'a> {
        let prefix = index_keys::pos_prefix(p, o);
        let mut results = Vec::new();
        {
            let mut state = self.inner.write().expect("MemoryTripleIndex poisoned");
            state.scans += 1;
            for key in state
                .pos
                .range(prefix.clone()..)
                .take_while(|k| k.starts_with(&prefix))
            {
                match index_keys::decode_pos_key(key) {
                    Ok((_, _, s, layer)) => results.push(Ok((s, layer))),
                    Err(e) => results.push(Err(StorageError::Internal(format!(
                        "MemoryTripleIndex decode error: {e}"
                    )))),
                }
            }
            state.entries_returned += results.len() as u64;
        }
        Box::new(results.into_iter())
    }

    fn stats(&self) -> IndexStats {
        let state = self.inner.read().expect("MemoryTripleIndex poisoned");
        IndexStats {
            triples: state.pos.len() as u64,
            layers: state.layers.len() as u64,
            scans: state.scans,
            entries_returned: state.entries_returned,
        }
    }
}

/// Length-prefixed key encoders for the two physical orderings.
///
/// Each variable-length segment (an IRI's UTF-8 bytes) is preceded by a
/// 4-byte big-endian length. The fixed-length 32-byte `LayerId` carries
/// no prefix — its position in the key is unambiguous.
///
/// Centralised here so the in-memory and RocksDB implementations agree
/// on byte-for-byte layout without duplicating logic.
pub mod index_keys {
    use crate::layer::LayerId;
    use crate::ontology::iri::Iri;

    fn write_segment(out: &mut Vec<u8>, segment: &[u8]) {
        let len: u32 = segment
            .len()
            .try_into()
            .expect("IRI segment exceeds u32::MAX bytes");
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(segment);
    }

    fn read_segment(buf: &[u8], pos: usize) -> Result<(&[u8], usize), String> {
        if pos + 4 > buf.len() {
            return Err(format!(
                "truncated length prefix at pos {pos} (buf len {})",
                buf.len()
            ));
        }
        let len = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        let start = pos + 4;
        let end = start + len;
        if end > buf.len() {
            return Err(format!(
                "truncated segment of length {len} at pos {start} (buf len {})",
                buf.len()
            ));
        }
        Ok((&buf[start..end], end))
    }

    /// `idx_pos:<p>:<o>:<s>:<layer>` — read-path key.
    pub fn pos_key(p: &Iri, o: &Iri, s: &Iri, layer: &LayerId) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(32 + p.as_str().len() + o.as_str().len() + s.as_str().len() + 16);
        write_segment(&mut out, p.as_str().as_bytes());
        write_segment(&mut out, o.as_str().as_bytes());
        write_segment(&mut out, s.as_str().as_bytes());
        out.extend_from_slice(&layer.0);
        out
    }

    /// Prefix matching every entry for a given `(p, o)` across all
    /// subjects and layers.
    pub fn pos_prefix(p: &Iri, o: &Iri) -> Vec<u8> {
        let mut out = Vec::with_capacity(p.as_str().len() + o.as_str().len() + 8);
        write_segment(&mut out, p.as_str().as_bytes());
        write_segment(&mut out, o.as_str().as_bytes());
        out
    }

    /// `idx_layer:<layer>:<p>:<o>:<s>` — GC-path reverse key.
    pub fn layer_key(layer: &LayerId, p: &Iri, o: &Iri, s: &Iri) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(32 + p.as_str().len() + o.as_str().len() + s.as_str().len() + 12);
        out.extend_from_slice(&layer.0);
        write_segment(&mut out, p.as_str().as_bytes());
        write_segment(&mut out, o.as_str().as_bytes());
        write_segment(&mut out, s.as_str().as_bytes());
        out
    }

    /// Prefix matching every entry contributed by a given layer.
    pub fn layer_prefix(layer: &LayerId) -> Vec<u8> {
        layer.0.to_vec()
    }

    /// Decode a forward `(p, o, s, layer)` key.
    pub fn decode_pos_key(key: &[u8]) -> Result<(Iri, Iri, Iri, LayerId), String> {
        let (p_bytes, pos) = read_segment(key, 0)?;
        let (o_bytes, pos) = read_segment(key, pos)?;
        let (s_bytes, pos) = read_segment(key, pos)?;
        if pos + 32 != key.len() {
            return Err(format!(
                "expected 32-byte LayerId trailer; got {} bytes at pos {pos}",
                key.len() - pos
            ));
        }
        let mut layer_bytes = [0u8; 32];
        layer_bytes.copy_from_slice(&key[pos..pos + 32]);
        let p = Iri::parse(std::str::from_utf8(p_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("predicate IRI: {e}"))?;
        let o = Iri::parse(std::str::from_utf8(o_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("object IRI: {e}"))?;
        let s = Iri::parse(std::str::from_utf8(s_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("subject IRI: {e}"))?;
        Ok((p, o, s, LayerId(layer_bytes)))
    }

    /// Decode a reverse `(layer, p, o, s)` key (returns just `(p, o, s)`
    /// — caller already knows the layer).
    pub fn decode_layer_key(key: &[u8]) -> Result<(Iri, Iri, Iri), String> {
        if key.len() < 32 {
            return Err(format!(
                "reverse key shorter than 32-byte LayerId prefix: {} bytes",
                key.len()
            ));
        }
        let pos = 32;
        let (p_bytes, pos) = read_segment(key, pos)?;
        let (o_bytes, pos) = read_segment(key, pos)?;
        let (s_bytes, pos) = read_segment(key, pos)?;
        if pos != key.len() {
            return Err(format!(
                "trailing {} bytes after reverse key segments",
                key.len() - pos
            ));
        }
        let p = Iri::parse(std::str::from_utf8(p_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("predicate IRI: {e}"))?;
        let o = Iri::parse(std::str::from_utf8(o_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("object IRI: {e}"))?;
        let s = Iri::parse(std::str::from_utf8(s_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("subject IRI: {e}"))?;
        Ok((p, o, s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::iri::Iri;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn lid(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }

    #[test]
    fn pos_key_roundtrip() {
        let p = iri("urn:eigenius:core:is_a");
        let o = iri("urn:eigenius:test:Dog");
        let s = iri("urn:eigenius:test:rex");
        let layer = lid(0xab);

        let key = index_keys::pos_key(&p, &o, &s, &layer);
        let (p2, o2, s2, layer2) = index_keys::decode_pos_key(&key).unwrap();
        assert_eq!(p, p2);
        assert_eq!(o, o2);
        assert_eq!(s, s2);
        assert_eq!(layer, layer2);
    }

    #[test]
    fn pos_prefix_matches_full_key() {
        let p = iri("urn:eigenius:core:is_a");
        let o = iri("urn:eigenius:test:Dog");
        let s = iri("urn:eigenius:test:rex");
        let layer = lid(0xab);

        let prefix = index_keys::pos_prefix(&p, &o);
        let key = index_keys::pos_key(&p, &o, &s, &layer);
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn layer_key_roundtrip() {
        let layer = lid(0x01);
        let p = iri("urn:eigenius:core:is_a");
        let o = iri("urn:eigenius:test:Dog");
        let s = iri("urn:eigenius:test:rex");

        let key = index_keys::layer_key(&layer, &p, &o, &s);
        let (p2, o2, s2) = index_keys::decode_layer_key(&key).unwrap();
        assert_eq!(p, p2);
        assert_eq!(o, o2);
        assert_eq!(s, s2);
        assert!(key.starts_with(&index_keys::layer_prefix(&layer)));
    }

    #[test]
    fn extend_and_scan() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        let p = iri("urn:eigenius:core:is_a");
        let dog = iri("urn:eigenius:test:Dog");
        let rex = iri("urn:eigenius:test:rex");
        let buddy = iri("urn:eigenius:test:buddy");

        index
            .extend_layer(
                &layer,
                &[
                    Triple {
                        subject: &rex,
                        predicate: &p,
                        object: &dog,
                    },
                    Triple {
                        subject: &buddy,
                        predicate: &p,
                        object: &dog,
                    },
                ],
            )
            .unwrap();

        let hits: Vec<(Iri, LayerId)> = index
            .scan_predicate_object(&p, &dog)
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits.len(), 2);
        // Ordered by subject IRI thanks to BTreeSet/key ordering.
        assert!(hits.iter().any(|(s, l)| s == &rex && l == &layer));
        assert!(hits.iter().any(|(s, l)| s == &buddy && l == &layer));

        let stats = index.stats();
        assert_eq!(stats.triples, 2);
        assert_eq!(stats.layers, 1);
        assert!(stats.scans >= 1);
        assert!(stats.entries_returned >= 2);
    }

    #[test]
    fn scan_filters_by_predicate_object() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        let is_a = iri("urn:eigenius:core:is_a");
        let dog = iri("urn:eigenius:test:Dog");
        let cat = iri("urn:eigenius:test:Cat");
        let rex = iri("urn:eigenius:test:rex");
        let mittens = iri("urn:eigenius:test:mittens");

        index
            .extend_layer(
                &layer,
                &[
                    Triple {
                        subject: &rex,
                        predicate: &is_a,
                        object: &dog,
                    },
                    Triple {
                        subject: &mittens,
                        predicate: &is_a,
                        object: &cat,
                    },
                ],
            )
            .unwrap();

        let dogs: Vec<Iri> = index
            .scan_predicate_object(&is_a, &dog)
            .map(|r| r.unwrap().0)
            .collect();
        assert_eq!(dogs, vec![rex.clone()]);

        let cats: Vec<Iri> = index
            .scan_predicate_object(&is_a, &cat)
            .map(|r| r.unwrap().0)
            .collect();
        assert_eq!(cats, vec![mittens.clone()]);
    }

    #[test]
    fn drop_layer_removes_all_entries() {
        let index = MemoryTripleIndex::new();
        let layer_a = lid(0x01);
        let layer_b = lid(0x02);
        let is_a = iri("urn:eigenius:core:is_a");
        let dog = iri("urn:eigenius:test:Dog");
        let rex = iri("urn:eigenius:test:rex");
        let buddy = iri("urn:eigenius:test:buddy");

        index
            .extend_layer(
                &layer_a,
                &[Triple {
                    subject: &rex,
                    predicate: &is_a,
                    object: &dog,
                }],
            )
            .unwrap();
        index
            .extend_layer(
                &layer_b,
                &[Triple {
                    subject: &buddy,
                    predicate: &is_a,
                    object: &dog,
                }],
            )
            .unwrap();

        index.drop_layer(&layer_a).unwrap();

        let hits: Vec<(Iri, LayerId)> = index
            .scan_predicate_object(&is_a, &dog)
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, buddy);
        assert_eq!(hits[0].1, layer_b);

        let stats = index.stats();
        assert_eq!(stats.triples, 1);
        assert_eq!(stats.layers, 1);
    }

    #[test]
    fn drop_layer_idempotent() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        index.drop_layer(&layer).unwrap();
        index.drop_layer(&layer).unwrap();
        assert_eq!(index.stats().triples, 0);
    }

    #[test]
    fn extend_idempotent_on_duplicate_triple() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        let is_a = iri("urn:eigenius:core:is_a");
        let dog = iri("urn:eigenius:test:Dog");
        let rex = iri("urn:eigenius:test:rex");

        let triple = Triple {
            subject: &rex,
            predicate: &is_a,
            object: &dog,
        };
        index.extend_layer(&layer, &[triple]).unwrap();
        index.extend_layer(&layer, &[triple]).unwrap();

        assert_eq!(index.stats().triples, 1);
    }

    #[test]
    fn extend_empty_is_noop() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        index.extend_layer(&layer, &[]).unwrap();
        assert_eq!(index.stats().triples, 0);
        assert_eq!(index.stats().layers, 0);
    }
}
