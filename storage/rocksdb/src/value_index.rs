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

//! RocksDB-backed `ValueIndex` (D65) — the exact-value read path.
//!
//! Mirrors [`super::triple_index`]: both physical orderings are stored as
//! length-prefixed keys with empty values, sharing the kernel's
//! [`eigenius_kernel::layer::index_keys`] segment encoding:
//!
//! - `vidx_pos:<index>:<key>:<subject>:<layer>` — read path (one prefix scan
//!   per exact lookup)
//! - `vidx_layer:<layer>:<index>:<key>:<subject>` — GC path (one prefix scan
//!   per layer drop)
//!
//! Distinct `vidx_*` table prefixes keep this index's keyspace disjoint from
//! the triple index's `idx_*`. Standalone `extend_layer` / `drop_layer` own
//! their `WriteBatch`; `extend_into_batch` / `drop_into_batch` append to a
//! caller's batch so `RocksStore::store_layer` / `delete_layer` commit layer
//! content + index in a single atomic write.

use crate::run_blocking;
use eigenius_kernel::layer::index_keys;
use eigenius_kernel::layer::{LayerId, ValueEntry, ValueIndex, ValueIndexStats};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::StorageError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const POS_PREFIX: &[u8] = b"vidx_pos:";
const LAYER_PREFIX: &[u8] = b"vidx_layer:";

/// Compose the full RocksDB key for a forward (POS) entry.
fn full_pos_key(body: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(POS_PREFIX.len() + body.len());
    key.extend_from_slice(POS_PREFIX);
    key.extend_from_slice(body);
    key
}

/// Compose the full RocksDB key for a reverse (layer) entry.
fn full_layer_key(body: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(LAYER_PREFIX.len() + body.len());
    key.extend_from_slice(LAYER_PREFIX);
    key.extend_from_slice(body);
    key
}

/// Prefix bytes for an `(index, key)` exact-lookup scan.
fn pos_scan_prefix(index: &Iri, key: &str) -> Vec<u8> {
    full_pos_key(&index_keys::value_pos_prefix(index, key))
}

/// Prefix bytes for "every reverse entry contributed by `layer`".
fn layer_scan_prefix(layer: &LayerId) -> Vec<u8> {
    full_layer_key(&index_keys::layer_prefix(layer))
}

/// RocksDB-backed value index. Holds an `Arc<rocksdb::DB>` so multiple
/// `LayerStorage` clones share one physical index with no per-instance state.
pub struct RocksValueIndex {
    db: Arc<rocksdb::DB>,
    lookups: AtomicU64,
    entries_returned: AtomicU64,
}

impl RocksValueIndex {
    pub fn new(db: Arc<rocksdb::DB>) -> Self {
        Self {
            db,
            lookups: AtomicU64::new(0),
            entries_returned: AtomicU64::new(0),
        }
    }

    /// Append every entry to a caller-owned `WriteBatch`. Used by
    /// `RocksStore::store_layer` so layer content + index entries land in one
    /// atomic write.
    pub fn extend_into_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        layer: &LayerId,
        entries: &[ValueEntry<'_>],
    ) {
        for e in entries {
            let pos_body = index_keys::value_pos_key(e.index, e.key, e.subject, layer);
            let layer_body = index_keys::value_layer_key(layer, e.index, e.key, e.subject);
            batch.put(full_pos_key(&pos_body), b"");
            batch.put(full_layer_key(&layer_body), b"");
        }
    }

    /// Append every existing entry contributed by `layer` to the caller's
    /// `WriteBatch` as a delete. Walks the reverse index; the caller commits.
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
                // Decode `(index, key, subject)` from the reverse-key body so we
                // can compose the matching forward key. The body lives after the
                // `vidx_layer:` prefix.
                let body = &key[LAYER_PREFIX.len()..];
                let (index, k, subject) =
                    index_keys::decode_value_layer_key(body).map_err(|e| {
                        StorageError::Internal(format!("decode reverse value key during drop: {e}"))
                    })?;
                let pos_body = index_keys::value_pos_key(&index, &k, &subject, layer);
                batch.delete(full_pos_key(&pos_body));
                batch.delete(key);
            }
            Ok(())
        })
    }
}

impl ValueIndex for RocksValueIndex {
    fn extend_layer(
        &self,
        layer: &LayerId,
        entries: &[ValueEntry<'_>],
    ) -> Result<(), StorageError> {
        if entries.is_empty() {
            return Ok(());
        }
        run_blocking(|| {
            let mut batch = rocksdb::WriteBatch::default();
            self.extend_into_batch(&mut batch, layer, entries);
            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("value_index extend_layer: {e}")))
        })
    }

    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        run_blocking(|| {
            let mut batch = rocksdb::WriteBatch::default();
            self.drop_into_batch(&mut batch, layer)?;
            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("value_index drop_layer: {e}")))
        })
    }

    fn lookup<'a>(
        &'a self,
        index: &Iri,
        key: &str,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + 'a> {
        // Materialise into a Vec so we don't hold a borrow on `self.db` for the
        // iterator's lifetime; exact-lookup result sets are small.
        let results = run_blocking(|| {
            let prefix = pos_scan_prefix(index, key);
            let mut results: Vec<Result<(Iri, LayerId), StorageError>> = Vec::new();
            let iter = self.db.prefix_iterator(prefix.as_slice());
            for item in iter {
                match item {
                    Ok((rk, _value)) => {
                        if !rk.starts_with(prefix.as_slice()) {
                            break;
                        }
                        let body = &rk[POS_PREFIX.len()..];
                        match index_keys::decode_value_pos_key(body) {
                            Ok((_, _, subject, layer)) => results.push(Ok((subject, layer))),
                            Err(e) => results.push(Err(StorageError::Internal(format!(
                                "decode value pos key: {e}"
                            )))),
                        }
                    }
                    Err(e) => {
                        results.push(Err(StorageError::Internal(format!("lookup iter: {e}"))))
                    }
                }
            }
            results
        });
        self.lookups.fetch_add(1, Ordering::Relaxed);
        self.entries_returned
            .fetch_add(results.len() as u64, Ordering::Relaxed);
        Box::new(results.into_iter())
    }

    fn stats(&self) -> ValueIndexStats {
        // Live `entries` and `layers` would require a full scan to count
        // exactly. For RocksDB v1 we report only the cumulative operational
        // counters, mirroring `RocksTripleIndex::stats`.
        ValueIndexStats {
            entries: 0,
            layers: 0,
            lookups: self.lookups.load(Ordering::Relaxed),
            entries_returned: self.entries_returned.load(Ordering::Relaxed),
        }
    }
}
