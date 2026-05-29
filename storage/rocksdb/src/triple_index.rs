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

//! RocksDB-backed `TripleIndex` (Phase 14h commit 2 / D23 §5.9).
//!
//! Stores both physical orderings as length-prefixed keys with empty
//! values:
//!
//! - `idx_pos:<pos_key>` — read path (one prefix scan per query)
//! - `idx_layer:<layer_key>` — GC path (one prefix scan per layer drop)
//!
//! Where `<pos_key>` and `<layer_key>` come from
//! [`eigenius_kernel::layer::index_keys`]. Standalone `extend_layer` and
//! `drop_layer` create their own `WriteBatch` so the trait method is
//! atomic on its own; `extend_into_batch` / `drop_into_batch` append to
//! a caller-supplied batch so `RocksStore::store_layer` and
//! `RocksStore::delete_layer` can commit layer + index in a single
//! atomic write per D23 §6.3.

use crate::run_blocking;
use eigenius_kernel::layer::index_keys;
use eigenius_kernel::layer::{IndexStats, LayerId, Triple, TripleIndex};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::StorageError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const POS_PREFIX: &[u8] = b"idx_pos:";
const LAYER_PREFIX: &[u8] = b"idx_layer:";

/// Compose the full RocksDB key for a forward (POS) entry.
fn full_pos_key(pos_body: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(POS_PREFIX.len() + pos_body.len());
    key.extend_from_slice(POS_PREFIX);
    key.extend_from_slice(pos_body);
    key
}

/// Compose the full RocksDB key for a reverse (layer) entry.
fn full_layer_key(layer_body: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(LAYER_PREFIX.len() + layer_body.len());
    key.extend_from_slice(LAYER_PREFIX);
    key.extend_from_slice(layer_body);
    key
}

/// Prefix bytes for a `(predicate, object)` POS scan.
fn pos_scan_prefix(p: &Iri, o: &Iri) -> Vec<u8> {
    let body = index_keys::pos_prefix(p, o);
    full_pos_key(&body)
}

/// Prefix bytes for "every reverse entry contributed by `layer`".
fn layer_scan_prefix(layer: &LayerId) -> Vec<u8> {
    let body = index_keys::layer_prefix(layer);
    full_layer_key(&body)
}

/// RocksDB-backed triple index. Holds an `Arc<rocksdb::DB>` so multiple
/// `LayerStorage` clones can share the same physical index without any
/// per-instance state.
pub struct RocksTripleIndex {
    db: Arc<rocksdb::DB>,
    scans: AtomicU64,
    entries_returned: AtomicU64,
}

impl RocksTripleIndex {
    pub fn new(db: Arc<rocksdb::DB>) -> Self {
        Self {
            db,
            scans: AtomicU64::new(0),
            entries_returned: AtomicU64::new(0),
        }
    }

    /// Append every triple to a caller-owned `WriteBatch`, leaving the
    /// commit responsibility with the caller. Used by
    /// `RocksStore::store_layer` so layer content + index entries land
    /// in one atomic write.
    pub fn extend_into_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        layer: &LayerId,
        triples: &[Triple<'_>],
    ) {
        for t in triples {
            let pos_body = index_keys::pos_key(t.predicate, t.object, t.subject, layer);
            let layer_body = index_keys::layer_key(layer, t.predicate, t.object, t.subject);
            batch.put(full_pos_key(&pos_body), b"");
            batch.put(full_layer_key(&layer_body), b"");
        }
    }

    /// Append every existing entry contributed by `layer` to the
    /// caller's `WriteBatch` as a delete. Used by
    /// `RocksStore::delete_layer`. Walks the reverse index to find what
    /// to delete; the caller commits.
    pub fn drop_into_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        layer: &LayerId,
    ) -> Result<(), StorageError> {
        run_blocking(|| {
            let prefix = layer_scan_prefix(layer);
            let iter = self.db.prefix_iterator(prefix.as_slice());
            for item in iter {
                let (key, _) =
                    item.map_err(|e| StorageError::Internal(format!("drop_into_batch iter: {e}")))?;
                if !key.starts_with(prefix.as_slice()) {
                    break;
                }
                // Decode `(p, o, s)` from the reverse-key body so we can
                // compose the matching forward key. The body lives after
                // the `idx_layer:` prefix.
                let body = &key[LAYER_PREFIX.len()..];
                let (p, o, s) = index_keys::decode_layer_key(body).map_err(|e| {
                    StorageError::Internal(format!("decode reverse key during drop: {e}"))
                })?;
                let pos_body = index_keys::pos_key(&p, &o, &s, layer);
                batch.delete(full_pos_key(&pos_body));
                batch.delete(key);
            }
            Ok(())
        })
    }
}

impl TripleIndex for RocksTripleIndex {
    fn extend_layer(&self, layer: &LayerId, triples: &[Triple<'_>]) -> Result<(), StorageError> {
        if triples.is_empty() {
            return Ok(());
        }
        run_blocking(|| {
            let mut batch = rocksdb::WriteBatch::default();
            self.extend_into_batch(&mut batch, layer, triples);
            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("triple_index extend_layer: {e}")))
        })
    }

    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        run_blocking(|| {
            let mut batch = rocksdb::WriteBatch::default();
            self.drop_into_batch(&mut batch, layer)?;
            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("triple_index drop_layer: {e}")))
        })
    }

    fn scan_predicate_object<'a>(
        &'a self,
        p: &Iri,
        o: &Iri,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + 'a> {
        // Materialise into a Vec so we don't hold a borrow on `self.db`
        // for the iterator's lifetime; the trait's iterator return type
        // can then erase the lifetime cleanly. For typical query answer
        // sizes (10s–1000s of subjects) the materialisation cost is
        // negligible.
        let results = run_blocking(|| {
            let prefix = pos_scan_prefix(p, o);
            let mut results: Vec<Result<(Iri, LayerId), StorageError>> = Vec::new();
            let iter = self.db.prefix_iterator(prefix.as_slice());
            for item in iter {
                match item {
                    Ok((key, _value)) => {
                        if !key.starts_with(prefix.as_slice()) {
                            break;
                        }
                        let body = &key[POS_PREFIX.len()..];
                        match index_keys::decode_pos_key(body) {
                            Ok((_, _, subject, layer)) => results.push(Ok((subject, layer))),
                            Err(e) => results
                                .push(Err(StorageError::Internal(format!("decode pos key: {e}")))),
                        }
                    }
                    Err(e) => results.push(Err(StorageError::Internal(format!(
                        "scan_predicate_object iter: {e}"
                    )))),
                }
            }
            results
        });
        self.scans.fetch_add(1, Ordering::Relaxed);
        self.entries_returned
            .fetch_add(results.len() as u64, Ordering::Relaxed);
        Box::new(results.into_iter())
    }

    fn stats(&self) -> IndexStats {
        // Live `triples` and `layers` would require a full scan to count
        // exactly. For RocksDB v1 we only report the cumulative
        // operational counters; precise sizing can use `approximate_sizes`
        // later if a workload demands it.
        IndexStats {
            triples: 0,
            layers: 0,
            scans: self.scans.load(Ordering::Relaxed),
            entries_returned: self.entries_returned.load(Ordering::Relaxed),
        }
    }
}
