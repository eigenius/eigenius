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

//! RocksDB-backed `VectorIndex` (D43 M2.5 / §2.4).
//!
//! Stores two key families in the `cf_vec` column family:
//!
//! - `vec_seg:<index_iri>:<layer>`     →  CBOR segment blob
//! - `vec_layer:<layer>:<index_iri>`   →  empty (reverse for drop_layer)
//!
//! The segment blob is a CBOR map carrying `model_iri`, `dim`,
//! `distance`, parallel `subjects` array, and a concatenated
//! `vectors` byte string of `count × dim × 4` bytes of fp32. M6
//! extends this layout with an optional `hnsw_graph` field; the
//! current v1 layout omits it so segments use the brute-force k-NN
//! path (M5).
//!
//! Standalone `extend_layer` / `drop_layer` create their own
//! `WriteBatch`; `extend_into_batch` / `drop_into_batch` append to a
//! caller-supplied batch so `RocksStore::store_layer` can commit
//! layer + indexes in a single atomic write (D43 §2.5 / §5.6 — the
//! vector segments are the one structural exception that may also
//! be backfilled by the M5 post-Load sweep).

use crate::{run_blocking, CF_VEC};
use eigenius_kernel::layer::{LayerId, VectorDoc, VectorIndex, VectorIndexStats, VectorSegment};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::StorageError;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const VEC_SEG_PREFIX: &[u8] = b"vec_seg:";
const VEC_LAYER_PREFIX: &[u8] = b"vec_layer:";

// ---------------- Key encoders / decoders ----------------

/// Encode a variable-length segment as `4-byte BE length || bytes`.
fn write_segment(out: &mut Vec<u8>, segment: &[u8]) {
    let len: u32 = segment
        .len()
        .try_into()
        .expect("segment exceeds u32::MAX bytes");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(segment);
}

/// Read a length-prefixed segment at `pos`. Returns `(bytes, new_pos)`.
fn read_segment(buf: &[u8], pos: usize) -> Result<(&[u8], usize), String> {
    if pos + 4 > buf.len() {
        return Err(format!("truncated length prefix at pos {pos}"));
    }
    let len = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
    let start = pos + 4;
    let end = start + len;
    if end > buf.len() {
        return Err(format!(
            "segment runs past buffer end ({end} > {})",
            buf.len()
        ));
    }
    Ok((&buf[start..end], end))
}

/// `vec_seg:<index_iri>:<layer>`
fn vec_seg_key(index: &Iri, layer: &LayerId) -> Vec<u8> {
    let mut key = Vec::with_capacity(VEC_SEG_PREFIX.len() + 4 + index.as_str().len() + 32);
    key.extend_from_slice(VEC_SEG_PREFIX);
    write_segment(&mut key, index.as_str().as_bytes());
    key.extend_from_slice(&layer.0);
    key
}

/// `vec_layer:<layer>:<index_iri>`
fn vec_layer_key(layer: &LayerId, index: &Iri) -> Vec<u8> {
    let mut key = Vec::with_capacity(VEC_LAYER_PREFIX.len() + 32 + 4 + index.as_str().len());
    key.extend_from_slice(VEC_LAYER_PREFIX);
    key.extend_from_slice(&layer.0);
    write_segment(&mut key, index.as_str().as_bytes());
    key
}

/// Prefix bytes for "every layer that contributed a segment under
/// `index`" — used by [`VectorIndex::scan_index`].
fn vec_seg_index_prefix(index: &Iri) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(VEC_SEG_PREFIX.len() + 4 + index.as_str().len());
    prefix.extend_from_slice(VEC_SEG_PREFIX);
    write_segment(&mut prefix, index.as_str().as_bytes());
    prefix
}

/// Prefix bytes for "every reverse entry contributed by `layer`" —
/// used by [`VectorIndex::drop_layer`] to enumerate every Index that
/// contributed at this layer.
fn vec_layer_scan_prefix(layer: &LayerId) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(VEC_LAYER_PREFIX.len() + 32);
    prefix.extend_from_slice(VEC_LAYER_PREFIX);
    prefix.extend_from_slice(&layer.0);
    prefix
}

/// Decode the `vec_layer:<layer>:<index_iri>` key body (the bytes
/// after the `vec_layer:` prefix) into its `(layer, index_iri)`
/// pair. Used by `drop_layer` when iterating the reverse index.
fn decode_layer_key_body(body: &[u8]) -> Result<(LayerId, Iri), String> {
    if body.len() < 32 {
        return Err(format!("body shorter than LayerId: {} bytes", body.len()));
    }
    let mut layer_bytes = [0u8; 32];
    layer_bytes.copy_from_slice(&body[..32]);
    let (iri_bytes, _) = read_segment(body, 32)?;
    let iri_str = std::str::from_utf8(iri_bytes).map_err(|e| format!("non-UTF8 IRI: {e}"))?;
    let iri = Iri::parse(iri_str).map_err(|e| format!("invalid IRI: {e}"))?;
    Ok((LayerId(layer_bytes), iri))
}

// ---------------- CBOR segment shape ----------------

/// On-wire CBOR shape for a vector segment. `vectors` is a single
/// concatenated byte string of `count × dim × 4` bytes of
/// little-endian fp32 — matches D43 §2.4's SIMD-friendly layout
/// (the in-RAM consumer casts the byte slice to `&[f32]` via
/// `bytemuck`).
///
/// `hnsw_graph` is the optional D43 §2.4 wire-format topology blob
/// (M6-finish.1's `kernel::query::vector::hnsw_format`). Present
/// for `strategy: hnsw` / `auto`-promoted segments; absent for
/// flat segments. The serde `default` + `skip_serializing_if` pair
/// keeps the on-disk shape backward-compatible with pre-M6-finish.4
/// segments written without the field.
#[derive(Debug, Serialize, Deserialize)]
struct VectorSegmentCbor {
    model_iri: String,
    dim: u32,
    distance: String,
    subjects: Vec<String>,
    /// `count × dim × 4` bytes, little-endian fp32.
    vectors: serde_bytes::ByteBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hnsw_graph: Option<serde_bytes::ByteBuf>,
}

fn segment_to_cbor(segment: &VectorSegment) -> Vec<u8> {
    let mut vector_bytes = Vec::with_capacity(segment.vectors.len() * 4);
    for &v in &segment.vectors {
        vector_bytes.extend_from_slice(&v.to_le_bytes());
    }
    let cbor = VectorSegmentCbor {
        model_iri: segment.model_iri.as_str().to_string(),
        dim: segment.dim,
        distance: segment.distance.clone(),
        subjects: segment
            .subjects
            .iter()
            .map(|s| s.as_str().to_string())
            .collect(),
        vectors: serde_bytes::ByteBuf::from(vector_bytes),
        hnsw_graph: segment
            .hnsw_graph_bytes
            .as_ref()
            .map(|b| serde_bytes::ByteBuf::from(b.clone())),
    };
    let mut buf = Vec::new();
    ciborium::into_writer(&cbor, &mut buf).expect("CBOR encode VectorSegment cannot fail");
    buf
}

fn segment_from_cbor(bytes: &[u8]) -> Result<VectorSegment, StorageError> {
    let cbor: VectorSegmentCbor = ciborium::from_reader(bytes)
        .map_err(|e| StorageError::Internal(format!("vec_seg decode: {e}")))?;
    let model_iri = Iri::parse(&cbor.model_iri)
        .map_err(|e| StorageError::Internal(format!("vec_seg model_iri: {e}")))?;
    let subjects: Result<Vec<Iri>, _> = cbor
        .subjects
        .into_iter()
        .map(|s| Iri::parse(&s).map_err(|e| format!("subject IRI: {e}")))
        .collect();
    let subjects = subjects.map_err(StorageError::Internal)?;

    // Decode the byte string back into Vec<f32>.
    let vector_bytes = cbor.vectors.into_vec();
    if !vector_bytes.len().is_multiple_of(4) {
        return Err(StorageError::Internal(format!(
            "vec_seg byte length {} not a multiple of 4",
            vector_bytes.len()
        )));
    }
    let mut vectors = Vec::with_capacity(vector_bytes.len() / 4);
    for chunk in vector_bytes.chunks_exact(4) {
        vectors.push(f32::from_le_bytes(chunk.try_into().unwrap()));
    }
    let expected = subjects.len() * cbor.dim as usize;
    if vectors.len() != expected {
        return Err(StorageError::Internal(format!(
            "vec_seg vectors len {} != subjects.len * dim ({} * {} = {})",
            vectors.len(),
            subjects.len(),
            cbor.dim,
            expected
        )));
    }
    Ok(VectorSegment {
        model_iri,
        dim: cbor.dim,
        distance: cbor.distance,
        subjects,
        vectors,
        hnsw_graph_bytes: cbor.hnsw_graph.map(|b| b.into_vec()),
    })
}

// ---------------- RocksVectorIndex ----------------

/// RocksDB-backed `VectorIndex`. Holds an `Arc<rocksdb::DB>` so
/// multiple `LayerStorage` clones share the same physical index.
pub struct RocksVectorIndex {
    db: Arc<rocksdb::DB>,
    scans: AtomicU64,
}

impl RocksVectorIndex {
    pub fn new(db: Arc<rocksdb::DB>) -> Self {
        Self {
            db,
            scans: AtomicU64::new(0),
        }
    }

    /// Resolve the `cf_vec` column-family handle, returning a typed
    /// error if it isn't registered (shouldn't happen — `RocksStore::open`
    /// declares it).
    fn cf_vec(&self) -> Result<&rocksdb::ColumnFamily, StorageError> {
        self.db
            .cf_handle(CF_VEC)
            .ok_or_else(|| StorageError::Internal(format!("missing column family {CF_VEC}")))
    }

    /// Append all two key families' updates for one `(index, layer)`
    /// pair to a caller-owned `WriteBatch`. The caller is responsible
    /// for committing the batch — used by `RocksStore::store_layer`
    /// (for atomic-with-Load when the sweep runs synchronously) and
    /// by the M5 post-Load sweep (for atomic per-`(layer, Index)`
    /// materialisation).
    #[allow(clippy::too_many_arguments)] // The args mirror the trait method.
    pub fn extend_into_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
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

        // Validate dimensionality before mutating anything.
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

        let cf = self.cf_vec()?;

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

        let blob = segment_to_cbor(&segment);
        batch.put_cf(&cf, vec_seg_key(index, layer), blob);
        // Reverse index — empty value, presence is the signal.
        batch.put_cf(&cf, vec_layer_key(layer, index), [] as [u8; 0]);
        Ok(())
    }

    /// Append deletes for every segment contributed by `layer` across
    /// all VectorIndex Resources to the caller's `WriteBatch`. Used
    /// by `RocksStore::delete_layer` to bundle index cleanup with the
    /// layer drop.
    pub fn drop_into_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        layer: &LayerId,
    ) -> Result<(), StorageError> {
        run_blocking(|| {
            let cf = self.cf_vec()?;
            let prefix = vec_layer_scan_prefix(layer);
            let iter = self.db.prefix_iterator_cf(&cf, prefix.as_slice());
            for item in iter {
                let (key, _value) =
                    item.map_err(|e| StorageError::Internal(format!("drop iter: {e}")))?;
                if !key.starts_with(prefix.as_slice()) {
                    break;
                }

                let body = &key[VEC_LAYER_PREFIX.len()..];
                let (l, index) = decode_layer_key_body(body).map_err(|e| {
                    StorageError::Internal(format!("decode reverse key during drop: {e}"))
                })?;
                debug_assert_eq!(&l, layer, "reverse key layer must match prefix");

                batch.delete_cf(&cf, vec_seg_key(&index, layer));
                batch.delete_cf(&cf, key);
            }
            Ok(())
        })
    }
}

impl VectorIndex for RocksVectorIndex {
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
        run_blocking(|| {
            let mut batch = rocksdb::WriteBatch::default();
            self.extend_into_batch(
                &mut batch, index, layer, model_iri, dim, distance, docs, hnsw_graph,
            )?;
            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("vec_index extend_layer: {e}")))
        })
    }

    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        run_blocking(|| {
            let mut batch = rocksdb::WriteBatch::default();
            self.drop_into_batch(&mut batch, layer)?;
            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("vec_index drop_layer: {e}")))
        })
    }

    fn get_segment(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<VectorSegment>, StorageError> {
        run_blocking(|| {
            let cf = self.cf_vec()?;
            match self
                .db
                .get_cf(&cf, vec_seg_key(index, layer))
                .map_err(|e| StorageError::Internal(format!("vec_seg get: {e}")))?
            {
                Some(bytes) => Ok(Some(segment_from_cbor(&bytes)?)),
                None => Ok(None),
            }
        })
    }

    fn scan_index<'a>(
        &'a self,
        index: &Iri,
    ) -> Box<dyn Iterator<Item = Result<LayerId, StorageError>> + 'a> {
        let prefix = vec_seg_index_prefix(index);
        let results: Vec<Result<LayerId, StorageError>> = run_blocking(|| {
            let cf = match self.cf_vec() {
                Ok(cf) => cf,
                Err(e) => return vec![Err(e)],
            };
            let mut out: Vec<Result<LayerId, StorageError>> = Vec::new();
            let iter = self.db.prefix_iterator_cf(&cf, prefix.as_slice());
            for item in iter {
                match item {
                    Ok((key, _value)) => {
                        if !key.starts_with(prefix.as_slice()) {
                            break;
                        }
                        if key.len() < prefix.len() + 32 {
                            out.push(Err(StorageError::Internal(format!(
                                "vec_seg key too short: {}",
                                key.len()
                            ))));
                            continue;
                        }
                        let mut layer_bytes = [0u8; 32];
                        layer_bytes.copy_from_slice(&key[prefix.len()..prefix.len() + 32]);
                        out.push(Ok(LayerId(layer_bytes)));
                    }
                    Err(e) => {
                        out.push(Err(StorageError::Internal(format!("scan_index iter: {e}"))))
                    }
                }
            }
            out
        });
        self.scans.fetch_add(1, Ordering::Relaxed);
        Box::new(results.into_iter())
    }

    fn stats(&self) -> VectorIndexStats {
        // Live counts would require a full scan; for v1 we only
        // report the cumulative scan counter.
        VectorIndexStats {
            indexes: 0,
            layers: 0,
            segments: 0,
            total_vectors: 0,
            scans: self.scans.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use tempfile::TempDir;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn layer_id(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }

    fn open_temp_store() -> (Arc<crate::RocksStore>, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(crate::RocksStore::open(dir.path()).unwrap());
        (store, dir)
    }

    /// Round-trip: extend a layer under an index, get_segment returns
    /// the segment with intact metadata, subjects, and vectors.
    #[test]
    fn extend_then_get_segment_round_trips() {
        let (store, _dir) = open_temp_store();
        let idx = RocksVectorIndex::new(Arc::clone(&store.db));
        let i1 = iri("urn:eigenius:test:vi1");
        let l1 = layer_id(1);
        let model = iri("urn:eigenius:test:embedder");
        let s_a = iri("urn:eigenius:test:a");
        let s_b = iri("urn:eigenius:test:b");
        let v_a = [1.0f32, 0.0, 0.5, 0.25];
        let v_b = [0.0f32, 1.0, 0.5, 0.75];

        idx.extend_layer(
            &i1,
            &l1,
            &model,
            4,
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

        let seg = idx.get_segment(&i1, &l1).unwrap().unwrap();
        assert_eq!(seg.model_iri, model);
        assert_eq!(seg.dim, 4);
        assert_eq!(seg.distance, "cosine");
        assert_eq!(seg.count(), 2);
        assert_eq!(seg.subjects, vec![s_a, s_b]);
        assert_eq!(seg.vector_at(0), &v_a);
        assert_eq!(seg.vector_at(1), &v_b);
    }

    /// Dim mismatch fails before any write happens.
    #[test]
    fn extend_layer_rejects_dimension_mismatch() {
        let (store, _dir) = open_temp_store();
        let idx = RocksVectorIndex::new(Arc::clone(&store.db));
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let model = iri("urn:eigenius:test:embedder");
        let s = iri("urn:eigenius:test:s");
        let too_short = [1.0f32, 0.5];
        let err = idx
            .extend_layer(
                &i1,
                &l1,
                &model,
                4,
                "cosine",
                &[VectorDoc {
                    subject: &s,
                    vector: &too_short,
                }],
                None,
            )
            .expect_err("dim mismatch should fail");
        assert!(matches!(err, StorageError::Internal(_)));
        assert!(idx.get_segment(&i1, &l1).unwrap().is_none());
    }

    /// scan_index lists only the layers contributing under that Index.
    #[test]
    fn scan_index_yields_only_matching_layers() {
        let (store, _dir) = open_temp_store();
        let idx = RocksVectorIndex::new(Arc::clone(&store.db));
        let i1 = iri("urn:eigenius:test:i1");
        let i2 = iri("urn:eigenius:test:i2");
        let l1 = layer_id(1);
        let l2 = layer_id(2);
        let l3 = layer_id(3);
        let model = iri("urn:eigenius:test:m");
        let s = iri("urn:eigenius:test:s");
        let v = [1.0f32, 0.5];
        let docs = [VectorDoc {
            subject: &s,
            vector: &v,
        }];

        idx.extend_layer(&i1, &l1, &model, 2, "cosine", &docs, None)
            .unwrap();
        idx.extend_layer(&i1, &l2, &model, 2, "cosine", &docs, None)
            .unwrap();
        idx.extend_layer(&i2, &l3, &model, 2, "cosine", &docs, None)
            .unwrap();

        let i1_layers: BTreeSet<LayerId> = idx.scan_index(&i1).map(|r| r.unwrap()).collect();
        assert_eq!(i1_layers, BTreeSet::from([l1, l2]));
        let i2_layers: BTreeSet<LayerId> = idx.scan_index(&i2).map(|r| r.unwrap()).collect();
        assert_eq!(i2_layers, BTreeSet::from([l3]));
    }

    /// drop_layer removes every segment under that layer; segments
    /// under other layers untouched.
    #[test]
    fn drop_layer_removes_all_segments() {
        let (store, _dir) = open_temp_store();
        let idx = RocksVectorIndex::new(Arc::clone(&store.db));
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
        assert!(idx.get_segment(&i1, &l1).unwrap().is_none());
        assert!(idx.get_segment(&i2, &l1).unwrap().is_none());
        assert!(idx.get_segment(&i1, &l2).unwrap().is_some());
    }

    /// extend_layer is idempotent per (index, layer) — the §5.7
    /// atomic-reindex semantics relies on this.
    #[test]
    fn extend_layer_idempotent() {
        let (store, _dir) = open_temp_store();
        let idx = RocksVectorIndex::new(Arc::clone(&store.db));
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
        assert_eq!(seg.vector_at(0), &v_v2);
    }

    /// Segments persist across reopen — the atomic-with-WriteBatch
    /// invariant under the standalone `extend_layer` path.
    #[test]
    fn data_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let model = iri("urn:eigenius:test:m");
        let s = iri("urn:eigenius:test:s");
        let v = [1.0f32, 0.5, 0.25];

        {
            let store = crate::RocksStore::open(dir.path()).unwrap();
            let idx = RocksVectorIndex::new(Arc::clone(&store.db));
            idx.extend_layer(
                &i1,
                &l1,
                &model,
                3,
                "cosine",
                &[VectorDoc {
                    subject: &s,
                    vector: &v,
                }],
                None,
            )
            .unwrap();
        }

        let store = crate::RocksStore::open(dir.path()).unwrap();
        let idx = RocksVectorIndex::new(Arc::clone(&store.db));
        let seg = idx.get_segment(&i1, &l1).unwrap().unwrap();
        assert_eq!(seg.dim, 3);
        assert_eq!(seg.vector_at(0), &v);
    }

    /// CBOR segment encoding round-trips full segment shape exactly.
    #[test]
    fn segment_cbor_round_trips() {
        let model = iri("urn:eigenius:test:m");
        let s_a = iri("urn:eigenius:test:a");
        let s_b = iri("urn:eigenius:test:b");
        let seg = VectorSegment {
            model_iri: model.clone(),
            dim: 3,
            distance: "l2".to_string(),
            subjects: vec![s_a.clone(), s_b.clone()],
            vectors: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            hnsw_graph_bytes: None,
        };
        let bytes = segment_to_cbor(&seg);
        let decoded = segment_from_cbor(&bytes).unwrap();
        assert_eq!(decoded, seg);
    }

    /// D43 §2.4 / M6-finish.4 — CBOR round-trips the optional
    /// `hnsw_graph` payload alongside the vectors. Backward-compat
    /// with pre-M6-finish.4 segments (no `hnsw_graph` field at all)
    /// is exercised by the no-bytes variant above.
    #[test]
    fn segment_cbor_round_trips_with_hnsw_graph() {
        let model = iri("urn:eigenius:test:m");
        let s = iri("urn:eigenius:test:s");
        let graph_bytes: Vec<u8> = vec![0x45, 0x47, 0x48, 0x53, 0x01, 0xAA, 0xBB];
        let seg = VectorSegment {
            model_iri: model.clone(),
            dim: 2,
            distance: "cosine".to_string(),
            subjects: vec![s.clone()],
            vectors: vec![1.0, 0.0],
            hnsw_graph_bytes: Some(graph_bytes.clone()),
        };
        let bytes = segment_to_cbor(&seg);
        let decoded = segment_from_cbor(&bytes).unwrap();
        assert_eq!(decoded, seg);
        assert_eq!(
            decoded.hnsw_graph_bytes.as_deref(),
            Some(graph_bytes.as_slice())
        );
    }

    /// D43 §2.4 / M6-finish.4 — `extend_layer` persists the HNSW
    /// graph payload through to a fresh storage handle. The restart
    /// path is the load-bearing case: a kernel that came up cold
    /// should be able to query without paying the HNSW build cost.
    #[test]
    fn hnsw_graph_bytes_persist_across_reopen() {
        let dir = TempDir::new().unwrap();
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let model = iri("urn:eigenius:test:m");
        let s_a = iri("urn:eigenius:test:a");
        let s_b = iri("urn:eigenius:test:b");
        let v_a = [1.0f32, 0.0];
        let v_b = [0.0f32, 1.0];
        let graph_bytes: Vec<u8> = vec![0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0x42];

        {
            let store = crate::RocksStore::open(dir.path()).unwrap();
            let idx = RocksVectorIndex::new(Arc::clone(&store.db));
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
                Some(&graph_bytes),
            )
            .unwrap();
        }

        let store = crate::RocksStore::open(dir.path()).unwrap();
        let idx = RocksVectorIndex::new(Arc::clone(&store.db));
        let seg = idx.get_segment(&i1, &l1).unwrap().unwrap();
        assert_eq!(
            seg.hnsw_graph_bytes.as_deref(),
            Some(graph_bytes.as_slice())
        );
        assert_eq!(seg.vector_at(0), &v_a);
        assert_eq!(seg.vector_at(1), &v_b);
    }
}
