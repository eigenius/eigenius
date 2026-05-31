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

//! Per-`(VectorIndex Resource, layer)` vector segment store for D43
//! vector retrieval.
//!
//! Each segment holds the embedded vectors for one layer's
//! contributions to one VectorIndex Resource, keyed by the Index's
//! IRI so divergent configurations across branches stay
//! storage-safe (D43 §2.4 / §3.1).
//!
//! Two logical key families (D43 §2.4):
//!   vec_seg:<index_iri>:<layer>     →  CBOR segment blob
//!   vec_layer:<layer>:<index_iri>   →  empty (reverse for drop_layer)
//!
//! Each segment self-describes its `model_iri`, `dim`, and `distance`
//! metric so the query path can verify them against the active
//! VectorIndex's declared values at runtime (defence in depth — the
//! atomic-reindex policy of D43 §5.7 means a model mismatch should
//! never happen in production).
//!
//! This module defines:
//! * the [`VectorIndex`] trait — the storage surface D43 §2.4's read
//!   and write paths consult,
//! * the value types ([`VectorDoc`], [`VectorSegment`],
//!   [`VectorIndexStats`]),
//! * the in-memory [`MemoryVectorIndex`] backend, used by tests and
//!   the in-memory bootstrap path.
//!
//! The RocksDB backend lands in M2.5 as
//! `storage/rocksdb/src/vector_index.rs` with the CBOR-blob-in-`cf_vec`
//! layout from D43 §2.4 (concatenated `vectors` bstr, alignment
//! padding, optional `hnsw_graph` field for M6).
//!
//! See `docs/design/d43-text-and-vector-retrieval.md` §2.4 and §3.1
//! for the design and `docs/design/d43-implementation-plan.md` M2 for
//! the sequencing.

use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::storage::StorageError;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

/// One indexable vector — a `(subject, embedded-vector)` pair
/// produced by the indexing pipeline during a layer commit (or
/// during the post-Load sweep, M5).
///
/// `vector` must have length equal to the index's declared `dim`;
/// callers (the embedding sweep) ensure this. The slice is borrowed
/// to avoid an alloc per doc on the hot path.
#[derive(Debug, Clone, Copy)]
pub struct VectorDoc<'a> {
    pub subject: &'a Iri,
    pub vector: &'a [f32],
}

/// A vector segment fetched from the index. Carries the full set of
/// `(subject, vector)` pairs that one layer contributed under one
/// VectorIndex Resource, plus the segment-level metadata needed for
/// query-time validation and SIMD-friendly access.
///
/// Memory layout: `subjects[i]` and the `dim`-sized window
/// `&vectors[i*dim..(i+1)*dim]` are parallel for index `i ∈ 0..count`,
/// where `count = subjects.len()`. The flat `vectors: Vec<f32>` shape
/// matches the SIMD brute-force k-NN access pattern (D43 §2.4 — and
/// matches what the RocksDB backend will hand back from a zero-copy
/// `cast_slice` over the CBOR `vectors` bstr).
#[derive(Debug, Clone, PartialEq)]
pub struct VectorSegment {
    pub model_iri: Iri,
    pub dim: u32,
    /// Distance metric — one of the `core:DistanceMetric` Resource
    /// short_names (`cosine`, `l2`, `dot`). Recorded per-segment for
    /// self-description; the active VectorIndex Resource is the
    /// source of truth at query time.
    pub distance: String,
    pub subjects: Vec<Iri>,
    /// Flat `count × dim` row-major layout. `subjects.len() * dim`
    /// elements always.
    pub vectors: Vec<f32>,
    /// Optional HNSW graph in the D43 §2.4 wire format
    /// (`crate::query::vector::hnsw_format`). Present when the
    /// active VectorIndex Resource's `strategy` is `hnsw` (or
    /// `auto` with a count above the threshold) and the sweep
    /// built one. Absent for flat segments. The cache-admission
    /// path (M6.3 / M6-finish.4) decodes these bytes via
    /// `hnsw_format::decode` and attaches the resulting graph to
    /// the SegmentView so the first query after a restart doesn't
    /// pay the HNSW rebuild cost.
    pub hnsw_graph_bytes: Option<Vec<u8>>,
}

impl VectorSegment {
    /// Number of vectors in this segment.
    pub fn count(&self) -> usize {
        self.subjects.len()
    }

    /// Return the row-`i` vector as a slice — SIMD-friendly access
    /// for the brute-force k-NN path (M5).
    pub fn vector_at(&self, i: usize) -> &[f32] {
        let dim = self.dim as usize;
        &self.vectors[i * dim..(i + 1) * dim]
    }
}

/// Operational counters reported by [`VectorIndex::stats`]. Mirrors
/// the existing [`crate::layer::IndexStats`] shape; implementations
/// may report zero for fields they don't track.
#[derive(Debug, Default, Clone, Copy)]
pub struct VectorIndexStats {
    pub indexes: u64,
    pub layers: u64,
    pub segments: u64,
    pub total_vectors: u64,
    pub scans: u64,
}

/// Per-`(VectorIndex Resource, layer)` vector segment store — the
/// storage trait D43 §2.4's vector retrieval path consults.
///
/// **Storage shape (per-Index, per-layer).** Segments are keyed by
/// `vec_seg:<index_iri>:<layer>`. A read at head `H` does a prefix
/// iteration of `vec_seg:<index_iri>:` (keys only), filters to
/// layers in `H`'s chain, and fetches the matching segments —
/// matches the Phase 14h pattern with the Index IRI substituting
/// for the predicate.
///
/// **Atomic with `store_layer`.** RocksDB-backed implementations
/// write both the segment blob and the reverse-index entry inside
/// the same `WriteBatch` that persists the layer's resources, blooms,
/// and topology (D43 §2.5). The in-memory implementation here uses
/// its internal `RwLock`.
///
/// **GC integration.** When a layer is swept, [`Self::drop_layer`]
/// removes every segment under that layer via the reverse-keyed
/// `vec_layer:<layer>:<index_iri>` lookup table — Phase 14h's
/// reverse-index pattern, applied per-Index.
///
/// **One structural exception to atomic-with-Load** (D43 §5.6): the
/// vector segments are the only index entries that may be backfilled
/// by a post-Load sweep (M5) rather than written atomically with the
/// originating layer. The relaxation is narrowly scoped — embedding
/// requires an IO call to an embedder, and forcing atomicity would
/// gate Load on embedder availability. The trait's surface doesn't
/// care which path populated a segment; both result in the same
/// stored shape.
pub trait VectorIndex: Send + Sync {
    /// Insert a vector segment for the given layer under a specific
    /// VectorIndex Resource.
    ///
    /// Called by the M5 post-Load embedding sweep after the
    /// embedder has produced vectors for the new content. The
    /// implementation:
    ///
    /// 1. Validates each `doc.vector.len() == dim` (returns a typed
    ///    error if any are mismatched).
    /// 2. Stores the segment under `vec_seg:<index>:<layer>` with
    ///    the model IRI, dim, distance metric, parallel subject
    ///    array, and flat vector array.
    /// 3. Records the reverse-index entry so
    ///    [`Self::drop_layer`] can enumerate what to delete.
    ///
    /// Idempotent by `(index, layer)` — re-inserting under the same
    /// pair overwrites the segment in place. This mirrors the §5.7
    /// atomic-reindex semantics where the sweep may re-materialise
    /// a layer's vector contribution under a fresh VectorIndex
    /// Resource (which gets a new IRI, so it's a different key
    /// anyway).
    ///
    /// **`hnsw_graph`** — D43 §2.4 / M6-finish.4. Optional HNSW
    /// graph in the [`crate::query::vector::hnsw_format`] wire
    /// shape, persisted alongside the vectors so the graph survives
    /// kernel restart without paying the build cost on first query.
    /// `None` for flat segments (the legacy and `strategy: flat`
    /// path); `Some(bytes)` for `hnsw` / `auto`-promoted segments.
    #[allow(clippy::too_many_arguments)]
    fn extend_layer(
        &self,
        index: &Iri,
        layer: &LayerId,
        model_iri: &Iri,
        dim: u32,
        distance: &str,
        docs: &[VectorDoc<'_>],
        hnsw_graph: Option<&[u8]>,
    ) -> Result<(), StorageError>;

    /// Drop every segment contributed by `layer` across all
    /// VectorIndex Resources. Called by GC's `delete_layer`. No-op
    /// if the layer has no segments.
    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError>;

    /// Fetch the segment at `(index, layer)`. Used by the query
    /// path (M5) for per-segment brute-force k-NN or HNSW
    /// traversal (M6).
    fn get_segment(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<VectorSegment>, StorageError>;

    /// Stream `LayerId`s for every layer that has contributed a
    /// segment under `index`. Caller filters by chain membership.
    ///
    /// Yields `Result` per item so streaming backends can surface
    /// transient errors mid-iteration. The in-memory implementation
    /// always yields `Ok`.
    fn scan_index<'a>(
        &'a self,
        index: &Iri,
    ) -> Box<dyn Iterator<Item = Result<LayerId, StorageError>> + 'a>;

    /// Snapshot of operational counters.
    fn stats(&self) -> VectorIndexStats;
}

// ---------------- MemoryVectorIndex ----------------

/// In-memory [`VectorIndex`] backend. Used by tests and the
/// in-memory bootstrap path. Holds segments in a nested `BTreeMap`
/// keyed by `(index_iri, layer_id)`.
pub struct MemoryVectorIndex {
    inner: Arc<RwLock<MemoryVectorIndexState>>,
}

#[derive(Default)]
struct MemoryVectorIndexState {
    /// `vec_seg:<index>:<layer>` → segment.
    segments: BTreeMap<(Iri, LayerId), VectorSegment>,
    /// `vec_layer:<layer>:<index>` → presence flag. The mirror of
    /// `segments` keys with the order reversed; let `drop_layer`
    /// enumerate per-layer in `O(matches)` instead of scanning the
    /// whole segments map.
    layer_index: BTreeMap<(LayerId, Iri), ()>,
    /// Cumulative scan_index calls served.
    scans: u64,
}

impl MemoryVectorIndex {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryVectorIndexState::default())),
        }
    }
}

impl Default for MemoryVectorIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorIndex for MemoryVectorIndex {
    fn extend_layer(
        &self,
        index: &Iri,
        layer: &LayerId,
        model_iri: &Iri,
        dim: u32,
        distance: &str,
        docs: &[VectorDoc<'_>],
        hnsw_graph: Option<&[u8]>,
    ) -> Result<(), StorageError> {
        if docs.is_empty() {
            return Ok(());
        }

        // Validate vector dimensionality before mutating anything.
        let dim_usize = dim as usize;
        for (i, d) in docs.iter().enumerate() {
            if d.vector.len() != dim_usize {
                return Err(StorageError::Internal(format!(
                    "vector for subject {} at index {i} has dimensionality {} (expected {dim})",
                    d.subject.as_str(),
                    d.vector.len()
                )));
            }
        }

        let count = docs.len();
        let mut subjects = Vec::with_capacity(count);
        let mut vectors = Vec::with_capacity(count * dim_usize);
        for d in docs {
            subjects.push(d.subject.clone());
            vectors.extend_from_slice(d.vector);
        }
        let segment = VectorSegment {
            model_iri: model_iri.clone(),
            dim,
            distance: distance.to_string(),
            subjects,
            vectors,
            hnsw_graph_bytes: hnsw_graph.map(|b| b.to_vec()),
        };

        let mut state = self.inner.write().expect("MemoryVectorIndex poisoned");
        state
            .segments
            .insert((index.clone(), layer.clone()), segment);
        state.layer_index.insert((layer.clone(), index.clone()), ());
        Ok(())
    }

    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        let mut state = self.inner.write().expect("MemoryVectorIndex poisoned");

        let to_remove: Vec<(LayerId, Iri)> = state
            .layer_index
            .keys()
            .filter(|(l, _)| l == layer)
            .cloned()
            .collect();

        for (l, index) in to_remove {
            state.layer_index.remove(&(l.clone(), index.clone()));
            state.segments.remove(&(index, l));
        }
        Ok(())
    }

    fn get_segment(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<VectorSegment>, StorageError> {
        let state = self.inner.read().expect("MemoryVectorIndex poisoned");
        Ok(state.segments.get(&(index.clone(), layer.clone())).cloned())
    }

    fn scan_index<'a>(
        &'a self,
        index: &Iri,
    ) -> Box<dyn Iterator<Item = Result<LayerId, StorageError>> + 'a> {
        let mut state = self.inner.write().expect("MemoryVectorIndex poisoned");
        state.scans += 1;

        let results: Vec<LayerId> = state
            .segments
            .keys()
            .filter(|(i, _)| i == index)
            .map(|(_, l)| l.clone())
            .collect();

        Box::new(results.into_iter().map(Ok))
    }

    fn stats(&self) -> VectorIndexStats {
        let state = self.inner.read().expect("MemoryVectorIndex poisoned");
        let layers: BTreeSet<&LayerId> = state.segments.keys().map(|(_, l)| l).collect();
        let indexes: BTreeSet<&Iri> = state.segments.keys().map(|(i, _)| i).collect();
        let total_vectors: u64 = state
            .segments
            .values()
            .map(|seg| seg.subjects.len() as u64)
            .sum();
        VectorIndexStats {
            indexes: indexes.len() as u64,
            layers: layers.len() as u64,
            segments: state.segments.len() as u64,
            total_vectors,
            scans: state.scans,
        }
    }
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

    /// Round-trip: extend a layer under an index, get_segment
    /// returns the stored segment with parallel subjects and the
    /// flat vector array.
    #[test]
    fn extend_then_get_segment_round_trips() {
        let idx = MemoryVectorIndex::new();
        let i1 = iri("urn:eigenius:test:vec_idx_1");
        let l1 = layer_id(1);
        let model = iri("urn:eigenius:test:embedder_dummy");
        let s_a = iri("urn:eigenius:test:a");
        let s_b = iri("urn:eigenius:test:b");

        let v_a = [1.0f32, 0.0, 0.0, 0.0];
        let v_b = [0.0f32, 1.0, 0.0, 0.0];
        let docs = [
            VectorDoc {
                subject: &s_a,
                vector: &v_a,
            },
            VectorDoc {
                subject: &s_b,
                vector: &v_b,
            },
        ];
        idx.extend_layer(&i1, &l1, &model, 4, "cosine", &docs, None)
            .unwrap();

        let seg = idx.get_segment(&i1, &l1).unwrap().unwrap();
        assert_eq!(seg.model_iri, model);
        assert_eq!(seg.dim, 4);
        assert_eq!(seg.distance, "cosine");
        assert_eq!(seg.count(), 2);
        assert_eq!(seg.subjects, vec![s_a, s_b]);
        assert_eq!(seg.vector_at(0), &v_a);
        assert_eq!(seg.vector_at(1), &v_b);
    }

    /// `extend_layer` validates that every doc's vector has length
    /// `dim`; mismatches surface as a typed error before any state
    /// is mutated.
    #[test]
    fn extend_layer_rejects_dimension_mismatch() {
        let idx = MemoryVectorIndex::new();
        let i1 = iri("urn:eigenius:test:vec_idx_1");
        let l1 = layer_id(1);
        let model = iri("urn:eigenius:test:embedder");
        let s = iri("urn:eigenius:test:s");

        let too_short = [1.0f32, 0.0, 0.0]; // 3, not 4
        let docs = [VectorDoc {
            subject: &s,
            vector: &too_short,
        }];
        let err = idx
            .extend_layer(&i1, &l1, &model, 4, "cosine", &docs, None)
            .expect_err("dim mismatch should fail");
        assert!(matches!(err, StorageError::Internal(_)));

        // No segment was written.
        assert!(idx.get_segment(&i1, &l1).unwrap().is_none());
    }

    /// Two different VectorIndex Resources have separately-addressable
    /// segments. This is the cross-chain story (D43 §3.1 per-Index
    /// keying): branches that disagree on model / strategy don't
    /// collide.
    #[test]
    fn multiple_indexes_keep_separate_segments() {
        let idx = MemoryVectorIndex::new();
        let i1 = iri("urn:eigenius:test:vec_idx_v1");
        let i2 = iri("urn:eigenius:test:vec_idx_v2");
        let l1 = layer_id(1);
        let model_a = iri("urn:eigenius:test:model_a");
        let model_b = iri("urn:eigenius:test:model_b");
        let s = iri("urn:eigenius:test:s");

        let v_for_i1 = [1.0f32, 0.0, 0.0];
        let v_for_i2 = [0.0f32, 1.0, 1.0];

        idx.extend_layer(
            &i1,
            &l1,
            &model_a,
            3,
            "cosine",
            &[VectorDoc {
                subject: &s,
                vector: &v_for_i1,
            }],
            None,
        )
        .unwrap();
        idx.extend_layer(
            &i2,
            &l1,
            &model_b,
            3,
            "l2",
            &[VectorDoc {
                subject: &s,
                vector: &v_for_i2,
            }],
            None,
        )
        .unwrap();

        let seg_i1 = idx.get_segment(&i1, &l1).unwrap().unwrap();
        let seg_i2 = idx.get_segment(&i2, &l1).unwrap().unwrap();

        assert_eq!(seg_i1.model_iri, model_a);
        assert_eq!(seg_i1.distance, "cosine");
        assert_eq!(seg_i1.vector_at(0), &v_for_i1);

        assert_eq!(seg_i2.model_iri, model_b);
        assert_eq!(seg_i2.distance, "l2");
        assert_eq!(seg_i2.vector_at(0), &v_for_i2);
    }

    /// `scan_index` yields the layers under one Index without
    /// leaking layers under other Indexes.
    #[test]
    fn scan_index_returns_only_matching_layers() {
        let idx = MemoryVectorIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let i2 = iri("urn:eigenius:test:i2");
        let l1 = layer_id(1);
        let l2 = layer_id(2);
        let l3 = layer_id(3);
        let model = iri("urn:eigenius:test:m");
        let s = iri("urn:eigenius:test:s");
        let v = [1.0f32, 2.0];
        let docs = [VectorDoc {
            subject: &s,
            vector: &v,
        }];

        // i1 contributes at L1, L2; i2 contributes at L3.
        idx.extend_layer(&i1, &l1, &model, 2, "dot", &docs, None)
            .unwrap();
        idx.extend_layer(&i1, &l2, &model, 2, "dot", &docs, None)
            .unwrap();
        idx.extend_layer(&i2, &l3, &model, 2, "dot", &docs, None)
            .unwrap();

        let i1_layers: BTreeSet<LayerId> = idx.scan_index(&i1).map(|r| r.unwrap()).collect();
        assert_eq!(i1_layers, BTreeSet::from([l1.clone(), l2.clone()]));

        let i2_layers: BTreeSet<LayerId> = idx.scan_index(&i2).map(|r| r.unwrap()).collect();
        assert_eq!(i2_layers, BTreeSet::from([l3]));
    }

    /// `drop_layer` removes every segment under that layer across
    /// all Indexes. Segments at other layers are untouched.
    #[test]
    fn drop_layer_removes_all_segments_for_that_layer() {
        let idx = MemoryVectorIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let i2 = iri("urn:eigenius:test:i2");
        let l1 = layer_id(1);
        let l2 = layer_id(2);
        let model = iri("urn:eigenius:test:m");
        let s = iri("urn:eigenius:test:s");
        let v = [1.0f32];
        let docs = [VectorDoc {
            subject: &s,
            vector: &v,
        }];

        idx.extend_layer(&i1, &l1, &model, 1, "cosine", &docs, None)
            .unwrap();
        idx.extend_layer(&i2, &l1, &model, 1, "cosine", &docs, None)
            .unwrap();
        idx.extend_layer(&i1, &l2, &model, 1, "cosine", &docs, None)
            .unwrap();

        idx.drop_layer(&l1).unwrap();

        // L1 entries gone.
        assert!(idx.get_segment(&i1, &l1).unwrap().is_none());
        assert!(idx.get_segment(&i2, &l1).unwrap().is_none());
        // L2 untouched.
        assert!(idx.get_segment(&i1, &l2).unwrap().is_some());
    }

    /// `extend_layer` is idempotent per `(index, layer)` pair — the
    /// reindex / model upgrade story (D43 §5.7) relies on this.
    #[test]
    fn extend_layer_idempotent_per_pair() {
        let idx = MemoryVectorIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let model = iri("urn:eigenius:test:m");
        let s = iri("urn:eigenius:test:s");

        let v_v1 = [1.0f32, 0.0];
        idx.extend_layer(
            &i1,
            &l1,
            &model,
            2,
            "cosine",
            &[VectorDoc {
                subject: &s,
                vector: &v_v1,
            }],
            None,
        )
        .unwrap();
        let v_v2 = [0.0f32, 1.0];
        idx.extend_layer(
            &i1,
            &l1,
            &model,
            2,
            "cosine",
            &[VectorDoc {
                subject: &s,
                vector: &v_v2,
            }],
            None,
        )
        .unwrap();

        let seg = idx.get_segment(&i1, &l1).unwrap().unwrap();
        assert_eq!(seg.vector_at(0), &v_v2, "new write overwrites prior");
    }

    /// Empty doc list is a no-op (no segment written; no reverse
    /// index entry; stats unchanged).
    #[test]
    fn empty_docs_is_noop() {
        let idx = MemoryVectorIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let model = iri("urn:eigenius:test:m");
        idx.extend_layer(&i1, &l1, &model, 4, "cosine", &[], None)
            .unwrap();
        assert!(idx.get_segment(&i1, &l1).unwrap().is_none());
        let s = idx.stats();
        assert_eq!(s.segments, 0);
        assert_eq!(s.total_vectors, 0);
    }

    /// Stats reflect indexes, layers, segments, total_vectors, and
    /// cumulative scan_index calls.
    #[test]
    fn stats_reflect_state() {
        let idx = MemoryVectorIndex::new();
        let i1 = iri("urn:eigenius:test:i1");
        let i2 = iri("urn:eigenius:test:i2");
        let l1 = layer_id(1);
        let l2 = layer_id(2);
        let model = iri("urn:eigenius:test:m");
        let s_a = iri("urn:eigenius:test:a");
        let s_b = iri("urn:eigenius:test:b");
        let v_a = [1.0f32, 0.0];
        let v_b = [0.0f32, 1.0];

        // 2 vectors under (i1, l1); 1 vector under (i1, l2);
        // 1 vector under (i2, l1).
        idx.extend_layer(
            &i1,
            &l1,
            &model,
            2,
            "cosine",
            &[
                VectorDoc {
                    subject: &s_a,
                    vector: &v_a,
                },
                VectorDoc {
                    subject: &s_b,
                    vector: &v_b,
                },
            ],
            None,
        )
        .unwrap();
        idx.extend_layer(
            &i1,
            &l2,
            &model,
            2,
            "cosine",
            &[VectorDoc {
                subject: &s_a,
                vector: &v_a,
            }],
            None,
        )
        .unwrap();
        idx.extend_layer(
            &i2,
            &l1,
            &model,
            2,
            "cosine",
            &[VectorDoc {
                subject: &s_a,
                vector: &v_a,
            }],
            None,
        )
        .unwrap();

        let s1 = idx.stats();
        assert_eq!(s1.indexes, 2);
        assert_eq!(s1.layers, 2);
        assert_eq!(s1.segments, 3);
        assert_eq!(s1.total_vectors, 4);
        assert_eq!(s1.scans, 0);

        let _ = idx.scan_index(&i1).count();
        let _ = idx.scan_index(&i2).count();
        let _ = idx.scan_index(&i1).count();
        let s2 = idx.stats();
        assert_eq!(s2.scans, 3);
    }

    /// D43 §2.4 / M6-finish.4 — `extend_layer` accepts an optional
    /// HNSW graph payload and `get_segment` returns it through
    /// `hnsw_graph_bytes`. The bytes are stored opaquely; backends
    /// do not parse them at write time.
    #[test]
    fn extend_layer_round_trips_hnsw_graph_bytes() {
        let idx = MemoryVectorIndex::new();
        let i1 = iri("urn:eigenius:test:vi");
        let l1 = layer_id(7);
        let model = iri("urn:eigenius:test:m");
        let s = iri("urn:eigenius:test:s");
        let v = [1.0f32, 0.0];
        let docs = [VectorDoc {
            subject: &s,
            vector: &v,
        }];
        let graph_bytes: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02];

        idx.extend_layer(&i1, &l1, &model, 2, "cosine", &docs, Some(&graph_bytes))
            .unwrap();
        let seg = idx.get_segment(&i1, &l1).unwrap().unwrap();
        assert_eq!(
            seg.hnsw_graph_bytes.as_deref(),
            Some(graph_bytes.as_slice())
        );
    }
}
