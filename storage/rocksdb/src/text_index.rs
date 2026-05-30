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

//! RocksDB-backed `TextIndex` (D43 M2.4 / §2.3).
//!
//! Stores four key families in the `cf_text` column family, all with
//! length-prefixed variable segments so prefix scans are unambiguous:
//!
//! - `text_term:<index_iri>:<term>:<layer>`    →  `varint(df) || roaring_bytes`
//! - `text_docs:<index_iri>:<layer>`           →  CBOR { subjects, doc_lengths }
//! - `text_stats:<index_iri>:<layer>`          →  CBOR { doc_count, avg_doc_length, analyzer_id }
//! - `text_terms_layer:<layer>:<index_iri>`    →  CBOR [term, ...]   (reverse for drop_layer)
//!
//! The posting list is a Roaring bitmap (D43 §2.3); the `varint(df)`
//! prefix lets chain-aware BM25 IDF sum across visible layers
//! without deserialising the bitmap. The CBOR values are CBOR-encoded
//! shapes that mirror the in-memory equivalents.
//!
//! Standalone `extend_layer` / `drop_layer` create their own
//! `WriteBatch`; `extend_into_batch` / `drop_into_batch` append to a
//! caller-supplied batch so `RocksStore::store_layer` can commit
//! layer + index in a single atomic write (D43 §2.5).

use crate::{run_blocking, CF_TEXT};
use eigenius_kernel::layer::{
    LayerId, TermHit, TextDoc, TextDocs, TextIndex, TextIndexStats, TextLayerStats,
};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::StorageError;
use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const TEXT_TERM_PREFIX: &[u8] = b"text_term:";
const TEXT_DOCS_PREFIX: &[u8] = b"text_docs:";
const TEXT_STATS_PREFIX: &[u8] = b"text_stats:";
const TEXT_TERMS_LAYER_PREFIX: &[u8] = b"text_terms_layer:";

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

/// `text_term:<index_iri>:<term>:<layer>`
fn text_term_key(index: &Iri, term: &str, layer: &LayerId) -> Vec<u8> {
    let mut key =
        Vec::with_capacity(TEXT_TERM_PREFIX.len() + 4 + index.as_str().len() + 4 + term.len() + 32);
    key.extend_from_slice(TEXT_TERM_PREFIX);
    write_segment(&mut key, index.as_str().as_bytes());
    write_segment(&mut key, term.as_bytes());
    key.extend_from_slice(&layer.0);
    key
}

/// `text_docs:<index_iri>:<layer>`
fn text_docs_key(index: &Iri, layer: &LayerId) -> Vec<u8> {
    let mut key = Vec::with_capacity(TEXT_DOCS_PREFIX.len() + 4 + index.as_str().len() + 32);
    key.extend_from_slice(TEXT_DOCS_PREFIX);
    write_segment(&mut key, index.as_str().as_bytes());
    key.extend_from_slice(&layer.0);
    key
}

/// `text_stats:<index_iri>:<layer>`
fn text_stats_key(index: &Iri, layer: &LayerId) -> Vec<u8> {
    let mut key = Vec::with_capacity(TEXT_STATS_PREFIX.len() + 4 + index.as_str().len() + 32);
    key.extend_from_slice(TEXT_STATS_PREFIX);
    write_segment(&mut key, index.as_str().as_bytes());
    key.extend_from_slice(&layer.0);
    key
}

/// `text_terms_layer:<layer>:<index_iri>`
fn text_terms_layer_key(layer: &LayerId, index: &Iri) -> Vec<u8> {
    let mut key = Vec::with_capacity(TEXT_TERMS_LAYER_PREFIX.len() + 32 + 4 + index.as_str().len());
    key.extend_from_slice(TEXT_TERMS_LAYER_PREFIX);
    key.extend_from_slice(&layer.0);
    write_segment(&mut key, index.as_str().as_bytes());
    key
}

/// Prefix bytes for "every `(term, layer)` under `index`" — used to
/// scan all postings for a given Index. Not used yet (M3 uses the
/// finer-grained `text_term_scan_prefix`) but kept for symmetry.
#[allow(dead_code)]
fn text_term_index_prefix(index: &Iri) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(TEXT_TERM_PREFIX.len() + 4 + index.as_str().len());
    prefix.extend_from_slice(TEXT_TERM_PREFIX);
    write_segment(&mut prefix, index.as_str().as_bytes());
    prefix
}

/// Prefix bytes for "every layer that has term `T` under index `I`".
/// This is the [`TextIndex::scan_term`] read path's primary access
/// pattern — one prefix scan yields all contributing layers.
fn text_term_scan_prefix(index: &Iri, term: &str) -> Vec<u8> {
    let mut prefix =
        Vec::with_capacity(TEXT_TERM_PREFIX.len() + 4 + index.as_str().len() + 4 + term.len());
    prefix.extend_from_slice(TEXT_TERM_PREFIX);
    write_segment(&mut prefix, index.as_str().as_bytes());
    write_segment(&mut prefix, term.as_bytes());
    prefix
}

/// Prefix bytes for "every reverse entry contributed by `layer`" —
/// used by [`TextIndex::drop_layer`] to enumerate every Index that
/// contributed at this layer.
fn text_terms_layer_scan_prefix(layer: &LayerId) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(TEXT_TERMS_LAYER_PREFIX.len() + 32);
    prefix.extend_from_slice(TEXT_TERMS_LAYER_PREFIX);
    prefix.extend_from_slice(&layer.0);
    prefix
}

/// Decode the `text_terms_layer:<layer>:<index_iri>` key body
/// (i.e. the bytes after the `text_terms_layer:` prefix) into its
/// `(layer, index_iri)` pair. Used by `drop_layer` when iterating the
/// reverse index.
fn decode_terms_layer_key_body(body: &[u8]) -> Result<(LayerId, Iri), String> {
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

// ---------------- CBOR value shapes ----------------

#[derive(Debug, Serialize, Deserialize)]
struct TextDocsCbor {
    subjects: Vec<String>,
    doc_lengths: Vec<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TextStatsCbor {
    doc_count: u32,
    avg_doc_length: f32,
    analyzer_id: String,
}

// ---------------- Posting-list value layout (varint(df) || roaring_bytes) ----------------

/// Encode the `text_term` value as `varint(df) || roaring_bytes`.
/// The varint prefix lets chain-aware BM25 read DF without
/// deserialising the bitmap.
fn encode_posting_value(df: u32, bitmap: &RoaringBitmap) -> Vec<u8> {
    let mut out = Vec::new();
    encode_varint_u32(df, &mut out);
    bitmap
        .serialize_into(&mut out)
        .expect("Roaring serialize cannot fail on Vec<u8>");
    out
}

/// Decode the `varint(df)` prefix without touching the bitmap. Used
/// by chain-aware IDF in the M3 query path to sum DFs cheaply.
#[allow(dead_code)] // public M3 helper — used once chain-aware BM25 lands
pub fn decode_df_prefix(value: &[u8]) -> Result<u32, String> {
    decode_varint_u32(value).map(|(v, _)| v)
}

/// Decode the bitmap portion after the varint(df) prefix.
fn decode_posting_value(value: &[u8]) -> Result<(u32, RoaringBitmap), String> {
    let (df, consumed) = decode_varint_u32(value)?;
    let bitmap = RoaringBitmap::deserialize_from(&value[consumed..])
        .map_err(|e| format!("Roaring deserialize: {e}"))?;
    Ok((df, bitmap))
}

/// Standard LEB128 varint encoding for u32. Small DFs (the common
/// case) encode in 1-2 bytes.
fn encode_varint_u32(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(((value & 0x7F) | 0x80) as u8);
        value >>= 7;
    }
    out.push(value as u8);
}

fn decode_varint_u32(bytes: &[u8]) -> Result<(u32, usize), String> {
    let mut result: u32 = 0;
    let mut shift = 0u32;
    for (i, &b) in bytes.iter().enumerate() {
        if i >= 5 {
            return Err("varint exceeds u32 capacity".into());
        }
        result |= ((b & 0x7F) as u32) << shift;
        if b & 0x80 == 0 {
            return Ok((result, i + 1));
        }
        shift += 7;
    }
    Err("varint truncated".into())
}

// ---------------- RocksTextIndex ----------------

/// RocksDB-backed `TextIndex`. Holds an `Arc<rocksdb::DB>` so
/// multiple `LayerStorage` clones share the same physical index.
pub struct RocksTextIndex {
    db: Arc<rocksdb::DB>,
    scans: AtomicU64,
}

impl RocksTextIndex {
    pub fn new(db: Arc<rocksdb::DB>) -> Self {
        Self {
            db,
            scans: AtomicU64::new(0),
        }
    }

    /// Resolve the `cf_text` column-family handle, returning a typed
    /// error if it isn't registered (shouldn't happen — `RocksStore::open`
    /// declares it).
    fn cf_text(&self) -> Result<&rocksdb::ColumnFamily, StorageError> {
        self.db
            .cf_handle(CF_TEXT)
            .ok_or_else(|| StorageError::Internal(format!("missing column family {CF_TEXT}")))
    }

    /// Append all four key families' updates for one `(index, layer)`
    /// pair to a caller-owned `WriteBatch`. The caller is responsible
    /// for committing the batch — used by `RocksStore::store_layer`
    /// to bundle layer + indexes in one atomic write per D43 §2.5.
    pub fn extend_into_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        index: &Iri,
        layer: &LayerId,
        analyzer: &str,
        docs: &[TextDoc<'_>],
    ) -> Result<(), StorageError> {
        if docs.is_empty() {
            return Ok(());
        }

        let cf = self.cf_text()?;

        let n = docs.len();
        let mut subjects = Vec::with_capacity(n);
        let mut doc_lengths = Vec::with_capacity(n);
        let mut term_postings: std::collections::BTreeMap<String, RoaringBitmap> =
            std::collections::BTreeMap::new();

        for (doc_id, doc) in docs.iter().enumerate() {
            subjects.push(doc.subject.as_str().to_string());
            doc_lengths.push(doc.tokens.len() as u32);
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

        // Idempotency: enumerate prior terms for this (index, layer)
        // pair and delete their text_term entries. Otherwise a
        // re-extend on the same pair leaks stale postings for terms
        // dropped between the two extends.
        if let Some(prior) = self
            .db
            .get_cf(&cf, text_terms_layer_key(layer, index))
            .map_err(|e| StorageError::Internal(format!("text_terms_layer get: {e}")))?
        {
            let prior_terms: Vec<String> = ciborium::from_reader(prior.as_slice())
                .map_err(|e| StorageError::Internal(format!("text_terms_layer decode: {e}")))?;
            for term in prior_terms {
                batch.delete_cf(&cf, text_term_key(index, &term, layer));
            }
        }

        // Posting lists.
        let terms_for_reverse: Vec<String> = term_postings.keys().cloned().collect();
        for (term, bitmap) in term_postings {
            let df = bitmap.len() as u32;
            let value = encode_posting_value(df, &bitmap);
            batch.put_cf(&cf, text_term_key(index, &term, layer), value);
        }

        // text_docs blob.
        let docs_cbor = TextDocsCbor {
            subjects,
            doc_lengths,
        };
        let mut docs_bytes = Vec::new();
        ciborium::into_writer(&docs_cbor, &mut docs_bytes)
            .map_err(|e| StorageError::Internal(format!("text_docs encode: {e}")))?;
        batch.put_cf(&cf, text_docs_key(index, layer), docs_bytes);

        // text_stats blob (includes analyzer ID).
        let stats_cbor = TextStatsCbor {
            doc_count: n as u32,
            avg_doc_length,
            analyzer_id: analyzer.to_string(),
        };
        let mut stats_bytes = Vec::new();
        ciborium::into_writer(&stats_cbor, &mut stats_bytes)
            .map_err(|e| StorageError::Internal(format!("text_stats encode: {e}")))?;
        batch.put_cf(&cf, text_stats_key(index, layer), stats_bytes);

        // Reverse index: term list (for drop_layer).
        let mut reverse_bytes = Vec::new();
        ciborium::into_writer(&terms_for_reverse, &mut reverse_bytes)
            .map_err(|e| StorageError::Internal(format!("text_terms_layer encode: {e}")))?;
        batch.put_cf(&cf, text_terms_layer_key(layer, index), reverse_bytes);

        Ok(())
    }

    /// Append deletes for every key contributed by `layer` across
    /// all TextIndex Resources to the caller's `WriteBatch`. Used by
    /// `RocksStore::delete_layer` to bundle index cleanup with the
    /// layer drop.
    pub fn drop_into_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        layer: &LayerId,
    ) -> Result<(), StorageError> {
        run_blocking(|| {
            let cf = self.cf_text()?;
            let prefix = text_terms_layer_scan_prefix(layer);
            let iter = self.db.prefix_iterator_cf(&cf, prefix.as_slice());
            for item in iter {
                let (key, value) =
                    item.map_err(|e| StorageError::Internal(format!("drop iter: {e}")))?;
                if !key.starts_with(prefix.as_slice()) {
                    break;
                }

                // Decode the (layer, index) pair from the reverse-key
                // body (everything after the `text_terms_layer:` prefix).
                let body = &key[TEXT_TERMS_LAYER_PREFIX.len()..];
                let (l, index) = decode_terms_layer_key_body(body).map_err(|e| {
                    StorageError::Internal(format!("decode reverse key during drop: {e}"))
                })?;
                debug_assert_eq!(&l, layer, "reverse key layer must match prefix");

                // Decode the per-(layer, index) term list to enumerate
                // every text_term key to delete.
                let terms: Vec<String> = ciborium::from_reader(value.as_ref())
                    .map_err(|e| StorageError::Internal(format!("term-list decode: {e}")))?;
                for term in terms {
                    batch.delete_cf(&cf, text_term_key(&index, &term, layer));
                }

                // Delete the per-(layer, index) blobs and the reverse
                // entry itself.
                batch.delete_cf(&cf, text_docs_key(&index, layer));
                batch.delete_cf(&cf, text_stats_key(&index, layer));
                batch.delete_cf(&cf, key);
            }
            Ok(())
        })
    }
}

impl TextIndex for RocksTextIndex {
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
        run_blocking(|| {
            let mut batch = rocksdb::WriteBatch::default();
            self.extend_into_batch(&mut batch, index, layer, analyzer, docs)?;
            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("text_index extend_layer: {e}")))
        })
    }

    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        run_blocking(|| {
            let mut batch = rocksdb::WriteBatch::default();
            self.drop_into_batch(&mut batch, layer)?;
            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("text_index drop_layer: {e}")))
        })
    }

    fn scan_term<'a>(
        &'a self,
        index: &Iri,
        term: &str,
    ) -> Box<dyn Iterator<Item = Result<TermHit, StorageError>> + 'a> {
        let prefix = text_term_scan_prefix(index, term);
        let results: Vec<Result<TermHit, StorageError>> = run_blocking(|| {
            let cf = match self.cf_text() {
                Ok(cf) => cf,
                Err(e) => return vec![Err(e)],
            };
            let mut out: Vec<Result<TermHit, StorageError>> = Vec::new();
            let iter = self.db.prefix_iterator_cf(&cf, prefix.as_slice());
            for item in iter {
                match item {
                    Ok((key, value)) => {
                        if !key.starts_with(prefix.as_slice()) {
                            break;
                        }
                        // Trailing 32 bytes after the prefix are the layer.
                        if key.len() < prefix.len() + 32 {
                            out.push(Err(StorageError::Internal(format!(
                                "text_term key too short: {}",
                                key.len()
                            ))));
                            continue;
                        }
                        let mut layer_bytes = [0u8; 32];
                        layer_bytes.copy_from_slice(&key[prefix.len()..prefix.len() + 32]);
                        // Decode DF prefix + (re-)serialize bitmap as
                        // the `postings` opaque bytes. The trait
                        // doesn't expose the Roaring shape; callers
                        // round-trip via this opaque field.
                        match decode_posting_value(&value) {
                            Ok((df, bitmap)) => {
                                let mut postings = Vec::new();
                                bitmap
                                    .serialize_into(&mut postings)
                                    .expect("Roaring serialize cannot fail on Vec<u8>");
                                out.push(Ok(TermHit {
                                    layer: LayerId(layer_bytes),
                                    df,
                                    postings,
                                }));
                            }
                            Err(e) => out
                                .push(Err(StorageError::Internal(format!("decode posting: {e}")))),
                        }
                    }
                    Err(e) => out.push(Err(StorageError::Internal(format!("scan_term iter: {e}")))),
                }
            }
            out
        });
        self.scans.fetch_add(1, Ordering::Relaxed);
        Box::new(results.into_iter())
    }

    fn get_layer_stats(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<TextLayerStats>, StorageError> {
        run_blocking(|| {
            let cf = self.cf_text()?;
            match self
                .db
                .get_cf(&cf, text_stats_key(index, layer))
                .map_err(|e| StorageError::Internal(format!("text_stats get: {e}")))?
            {
                Some(bytes) => {
                    let cbor: TextStatsCbor = ciborium::from_reader(bytes.as_slice())
                        .map_err(|e| StorageError::Internal(format!("text_stats decode: {e}")))?;
                    Ok(Some(TextLayerStats {
                        doc_count: cbor.doc_count,
                        avg_doc_length: cbor.avg_doc_length,
                    }))
                }
                None => Ok(None),
            }
        })
    }

    fn get_layer_docs(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<TextDocs>, StorageError> {
        run_blocking(|| {
            let cf = self.cf_text()?;
            match self
                .db
                .get_cf(&cf, text_docs_key(index, layer))
                .map_err(|e| StorageError::Internal(format!("text_docs get: {e}")))?
            {
                Some(bytes) => {
                    let cbor: TextDocsCbor = ciborium::from_reader(bytes.as_slice())
                        .map_err(|e| StorageError::Internal(format!("text_docs decode: {e}")))?;
                    let subjects: Result<Vec<Iri>, _> = cbor
                        .subjects
                        .into_iter()
                        .map(|s| Iri::parse(&s).map_err(|e| format!("invalid IRI: {e}")))
                        .collect();
                    let subjects =
                        subjects.map_err(|e| StorageError::Internal(format!("text_docs: {e}")))?;
                    Ok(Some(TextDocs {
                        subjects,
                        doc_lengths: cbor.doc_lengths,
                    }))
                }
                None => Ok(None),
            }
        })
    }

    fn get_layer_analyzer(
        &self,
        index: &Iri,
        layer: &LayerId,
    ) -> Result<Option<String>, StorageError> {
        run_blocking(|| {
            let cf = self.cf_text()?;
            match self
                .db
                .get_cf(&cf, text_stats_key(index, layer))
                .map_err(|e| StorageError::Internal(format!("text_stats get: {e}")))?
            {
                Some(bytes) => {
                    let cbor: TextStatsCbor = ciborium::from_reader(bytes.as_slice())
                        .map_err(|e| StorageError::Internal(format!("text_stats decode: {e}")))?;
                    Ok(Some(cbor.analyzer_id))
                }
                None => Ok(None),
            }
        })
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
        run_blocking(|| {
            let cf = self.cf_text()?;

            // Roaring bitwise-AND. The first term seeds the
            // accumulator; subsequent terms intersect in place.
            // An absent posting at any layer means the AND is empty
            // — short-circuit.
            let mut accumulator: RoaringBitmap = match self
                .db
                .get_cf(&cf, text_term_key(index, &terms[0], layer))
                .map_err(|e| StorageError::Internal(format!("text_term get: {e}")))?
            {
                Some(bytes) => {
                    let (_, bitmap) = decode_posting_value(&bytes).map_err(|e| {
                        StorageError::Internal(format!(
                            "intersect_layer decode (term {}): {e}",
                            terms[0]
                        ))
                    })?;
                    bitmap
                }
                None => return Ok(Vec::new()),
            };

            for term in &terms[1..] {
                if accumulator.is_empty() {
                    break;
                }
                match self
                    .db
                    .get_cf(&cf, text_term_key(index, term, layer))
                    .map_err(|e| StorageError::Internal(format!("text_term get: {e}")))?
                {
                    Some(bytes) => {
                        let (_, other) = decode_posting_value(&bytes).map_err(|e| {
                            StorageError::Internal(format!(
                                "intersect_layer decode (term {term}): {e}"
                            ))
                        })?;
                        accumulator &= other;
                    }
                    None => return Ok(Vec::new()),
                }
            }

            Ok(accumulator.iter().collect())
        })
    }

    fn stats(&self) -> TextIndexStats {
        // Live counts would require a full scan; for v1 we only
        // report the cumulative scan counter. Precise sizing via
        // `approximate_sizes` is deferrable until a workload demands
        // it.
        TextIndexStats {
            indexes: 0,
            layers: 0,
            total_postings: 0,
            scans: self.scans.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::storage::PersistentBackend;
    use tempfile::TempDir;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn layer_id(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }

    fn tokens(s: &str) -> Vec<String> {
        s.split_whitespace().map(|w| w.to_string()).collect()
    }

    fn open_temp_store() -> (Arc<crate::RocksStore>, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(crate::RocksStore::open(dir.path()).unwrap());
        (store, dir)
    }

    /// Round-trip: extend a layer under an index, scan_term returns
    /// the expected df and a deserialisable Roaring bitmap with the
    /// expected doc-ids.
    #[test]
    fn extend_then_scan_returns_expected_df_and_bitmap() {
        let dir = TempDir::new().unwrap();
        let db = Arc::new(
            rocksdb::DB::open_cf_descriptors(
                &{
                    let mut o = rocksdb::Options::default();
                    o.create_if_missing(true);
                    o.create_missing_column_families(true);
                    o
                },
                dir.path(),
                vec![rocksdb::ColumnFamilyDescriptor::new(
                    CF_TEXT,
                    rocksdb::Options::default(),
                )],
            )
            .unwrap(),
        );
        let idx = RocksTextIndex::new(db);

        let i1 = iri("urn:eigenius:test:ti1");
        let l1 = layer_id(1);
        let s_a = iri("urn:eigenius:test:a");
        let s_b = iri("urn:eigenius:test:b");

        let toks_a = tokens("wal truncation concurrent commit");
        let toks_b = tokens("rolling back partial commit");
        let docs = [
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

        // "commit" → df=2, bitmap {0, 1}
        let hits: Vec<TermHit> = idx.scan_term(&i1, "commit").map(|r| r.unwrap()).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].df, 2);
        let bitmap = RoaringBitmap::deserialize_from(&hits[0].postings[..]).unwrap();
        let doc_ids: BTreeSet<u32> = bitmap.iter().collect();
        assert_eq!(doc_ids, BTreeSet::from([0, 1]));

        // "wal" → df=1, bitmap {0}
        let hits: Vec<TermHit> = idx.scan_term(&i1, "wal").map(|r| r.unwrap()).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].df, 1);
    }

    /// Posting values round-trip via the varint(df) || roaring layout.
    #[test]
    fn posting_value_round_trips() {
        let mut bitmap = RoaringBitmap::new();
        bitmap.insert(0);
        bitmap.insert(42);
        bitmap.insert(1_000_000);

        let value = encode_posting_value(3, &bitmap);
        let df = decode_df_prefix(&value).unwrap();
        let (df2, decoded) = decode_posting_value(&value).unwrap();
        assert_eq!(df, 3);
        assert_eq!(df2, 3);
        assert_eq!(decoded.iter().collect::<Vec<_>>(), vec![0, 42, 1_000_000]);
    }

    /// LEB128 varint round-trip over representative values.
    #[test]
    fn varint_round_trip() {
        for v in [0u32, 1, 127, 128, 16_383, 16_384, u32::MAX] {
            let mut buf = Vec::new();
            encode_varint_u32(v, &mut buf);
            let (decoded, consumed) = decode_varint_u32(&buf).unwrap();
            assert_eq!(decoded, v);
            assert_eq!(consumed, buf.len());
        }
    }

    /// get_layer_stats and get_layer_docs decode the CBOR blobs
    /// correctly; get_layer_analyzer pulls the stored analyzer ID.
    #[test]
    fn per_layer_metadata_round_trips() {
        let (store, _dir) = open_temp_store();
        let idx = RocksTextIndex::new(Arc::clone(&store.db));
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let s_a = iri("urn:eigenius:test:a");
        let s_b = iri("urn:eigenius:test:b");
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

        let analyzer = idx.get_layer_analyzer(&i1, &l1).unwrap().unwrap();
        assert_eq!(analyzer, "en-stem-v1");
    }

    /// drop_layer removes every key contributed by a layer across all
    /// indexes; other layers' content is untouched.
    #[test]
    fn drop_layer_removes_all_keys_for_that_layer() {
        let (store, _dir) = open_temp_store();
        let idx = RocksTextIndex::new(Arc::clone(&store.db));
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

        idx.extend_layer(&i1, &l1, "en-stem-v1", &docs).unwrap();
        idx.extend_layer(&i2, &l1, "en-stem-v1", &docs).unwrap();
        idx.extend_layer(&i1, &l2, "en-stem-v1", &docs).unwrap();

        idx.drop_layer(&l1).unwrap();

        // L1 entries gone for both indexes.
        assert!(idx.get_layer_stats(&i1, &l1).unwrap().is_none());
        assert!(idx.get_layer_stats(&i2, &l1).unwrap().is_none());
        assert!(idx.get_layer_docs(&i1, &l1).unwrap().is_none());
        assert!(idx.get_layer_analyzer(&i1, &l1).unwrap().is_none());

        // L2 untouched.
        assert!(idx.get_layer_stats(&i1, &l2).unwrap().is_some());
        let l2_hits: Vec<TermHit> = idx.scan_term(&i1, "alpha").map(|r| r.unwrap()).collect();
        assert_eq!(l2_hits.len(), 1);
        assert_eq!(l2_hits[0].layer, l2);
    }

    /// Re-extending the same (index, layer) drops stale postings for
    /// terms that no longer appear in the new content.
    #[test]
    fn re_extend_drops_stale_postings() {
        let (store, _dir) = open_temp_store();
        let idx = RocksTextIndex::new(Arc::clone(&store.db));
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

        // Stale term gone.
        assert_eq!(idx.scan_term(&i1, "old").count(), 0);
        // New term present.
        assert_eq!(idx.scan_term(&i1, "new").count(), 1);
    }

    /// Multiple TextIndex Resources have separately-addressable
    /// postings — the divergent-Index cross-chain story.
    #[test]
    fn multiple_indexes_dont_collide() {
        let (store, _dir) = open_temp_store();
        let idx = RocksTextIndex::new(Arc::clone(&store.db));
        let i1 = iri("urn:eigenius:test:i1");
        let i2 = iri("urn:eigenius:test:i2");
        let l1 = layer_id(1);
        let s = iri("urn:eigenius:test:s");
        let toks = tokens("foo bar");
        let docs = [TextDoc {
            subject: &s,
            tokens: &toks,
        }];

        idx.extend_layer(&i1, &l1, "en-stem-v1", &docs).unwrap();
        idx.extend_layer(&i2, &l1, "en-no-stem", &docs).unwrap();

        assert_eq!(
            idx.get_layer_analyzer(&i1, &l1).unwrap().as_deref(),
            Some("en-stem-v1")
        );
        assert_eq!(
            idx.get_layer_analyzer(&i2, &l1).unwrap().as_deref(),
            Some("en-no-stem")
        );

        assert_eq!(idx.scan_term(&i1, "foo").count(), 1);
        assert_eq!(idx.scan_term(&i2, "foo").count(), 1);
    }

    /// Data persists across reopen — the atomic-with-WriteBatch
    /// invariant under the standalone `extend_layer` path.
    #[test]
    fn data_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let s = iri("urn:eigenius:test:s");
        let toks = tokens("alpha beta gamma");

        {
            let store = crate::RocksStore::open(dir.path()).unwrap();
            let idx = RocksTextIndex::new(Arc::clone(&store.db));
            idx.extend_layer(
                &i1,
                &l1,
                "en-stem-v1",
                &[TextDoc {
                    subject: &s,
                    tokens: &toks,
                }],
            )
            .unwrap();
        }

        let store = crate::RocksStore::open(dir.path()).unwrap();
        let idx = RocksTextIndex::new(Arc::clone(&store.db));
        let stats = idx.get_layer_stats(&i1, &l1).unwrap().unwrap();
        assert_eq!(stats.doc_count, 1);
        assert_eq!(
            idx.scan_term(&i1, "alpha").filter_map(|r| r.ok()).count(),
            1,
            "postings survive reopen"
        );

        // Verify the backend's text_index_arc handle picks up the
        // existing data — once M2.4 wires RocksTextIndex through
        // PersistentBackend (a separate step below) the in-memory
        // placeholder no longer applies. For now, this verifies the
        // standalone path.
        let _ = store.text_index_arc(); // smoke
    }

    /// D43 M3.3 — `intersect_layer` returns the AND of multiple
    /// posting lists. Verifies the Roaring-bitwise-AND path
    /// against a small corpus.
    #[test]
    fn intersect_layer_returns_and_of_postings() {
        let (store, _dir) = open_temp_store();
        let idx = RocksTextIndex::new(Arc::clone(&store.db));
        let i1 = iri("urn:eigenius:test:i1");
        let l1 = layer_id(1);
        let s_a = iri("urn:eigenius:test:sa");
        let s_b = iri("urn:eigenius:test:sb");
        let s_c = iri("urn:eigenius:test:sc");

        let toks_a = tokens("alpha beta gamma");
        let toks_b = tokens("alpha gamma");
        let toks_c = tokens("alpha beta");
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
                TextDoc {
                    subject: &s_c,
                    tokens: &toks_c,
                },
            ],
        )
        .unwrap();

        // Doc-ids: 0 = s_a, 1 = s_b, 2 = s_c.
        // "alpha" hits all three; "beta" hits s_a and s_c; "gamma" hits s_a and s_b.
        // AND("alpha", "beta") = {0, 2}.
        let ids = idx
            .intersect_layer(&i1, &l1, &["alpha".into(), "beta".into()])
            .unwrap();
        assert_eq!(ids, vec![0, 2]);

        // AND("alpha", "beta", "gamma") = {0}.
        let ids = idx
            .intersect_layer(&i1, &l1, &["alpha".into(), "beta".into(), "gamma".into()])
            .unwrap();
        assert_eq!(ids, vec![0]);

        // Term not in index → empty result, short-circuits.
        let ids = idx
            .intersect_layer(&i1, &l1, &["alpha".into(), "missing".into()])
            .unwrap();
        assert!(ids.is_empty());

        // Empty terms list → empty result.
        let ids = idx.intersect_layer(&i1, &l1, &[]).unwrap();
        assert!(ids.is_empty());
    }
}
