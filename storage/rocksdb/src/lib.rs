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

//! RocksDB storage backend for Eigenius.
//!
//! Implements `LayerStore` and `ResourceStore` using RocksDB as the
//! persistent ordered key-value store. Key encoding follows D4.
//!
//! Key scheme:
//!   layer:<layer_id_hex>:res:<iri>    → Resource (CBOR)
//!   chain:<layer_id_hex>              → Parent layer ID hex (or empty)
//!   head                              → Current head layer ID hex
//!   topo:<layer_id_hex>               → LayerHandle (CBOR, Phase 14a-ii)
//!   trace:<key_hex>                   → ComponentTrace (CBOR)
//!   meta:<key>                        → Generic metadata KV

mod text_index;
mod triple_index;
mod value_index;
mod vector_index;

#[cfg(test)]
use eigenius_kernel::layer::LayerBuilder;
use eigenius_kernel::layer::{
    BloomFilter, ContentHash, Layer, LayerHandle, LayerId, LayerTopology, RedirectEntry,
};
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::storage::{ResourceBackend, StorageError};
use std::path::Path;
use std::sync::Arc;
use text_index::RocksTextIndex;
use triple_index::RocksTripleIndex;
use value_index::RocksValueIndex;
use vector_index::RocksVectorIndex;

/// Run a sync disk-bound block in a way that doesn't starve the
/// tokio worker pool.
///
/// When called inside a multi-threaded tokio runtime,
/// [`tokio::task::block_in_place`] relocates other tasks off the
/// current worker thread, runs the closure synchronously, then lets
/// the worker resume normal scheduling. This avoids the failure mode
/// where N concurrent gRPC handlers all block on RocksDB's sync WAL
/// flush and starve all other RPCs from making progress.
///
/// Outside a multi-threaded runtime (kernel tests not annotated
/// `#[tokio::test(flavor = "multi_thread")]`, bootstrap before the
/// gRPC server starts, CLI commands), the closure runs directly.
/// `block_in_place` would panic in a current-thread runtime; the
/// fallback keeps the impl callable from every context.
///
/// Concurrency ceiling under this design is `tokio_workers ≈ num_cpus`
/// — that's enough for the workloads we're solving for. If we ever
/// need hundreds of concurrent disk-bound calls, the trait would
/// have to become async and the impl would use
/// [`tokio::task::spawn_blocking`] against the much larger blocking
/// pool. Deferred until usage justifies the trait churn.
#[inline]
pub(crate) fn run_blocking<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(handle) if matches!(handle.runtime_flavor(), RuntimeFlavor::MultiThread) => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

const TOPO_PREFIX: &str = "topo:";
const BLOOM_PREFIX: &str = "bloom:";
const BRANCH_PREFIX: &str = "branch:";
/// Tag-ref storage (D34 §G.2 / §8). Immutable named refs into the
/// DAG. Keys: `tag:<name>` → hex-encoded `LayerId`. Tags are GC
/// roots — `gc::collect` enumerates them alongside branches when
/// computing reachability.
const TAG_PREFIX: &str = "tag:";
/// Content-hash dedup index (D25 §11.0 / D33 §6).
///
/// Keys: `content:<content_hash_hex>:<position_hash_hex>` → empty value.
/// Prefix-scanning with `content:<content_hash_hex>:` yields every
/// position sharing that content hash; the position hash hex follows
/// the content hash hex inside the key so the scan returns positions
/// in lexicographic order without a secondary lookup.
const CONTENT_INDEX_PREFIX: &str = "content:";
/// Resolve-redirect storage (D25 §12.8 / Phase 17f).
///
/// Keys: `redirect:<source_layer_hex>` → CBOR-encoded
/// [`eigenius_kernel::layer::RedirectEntry`]. One entry per
/// consolidation where `to` was below the branch head. Carries the
/// source layer's `LayerHandle` snapshot so `load_topology` can
/// manufacture the in-memory synthetic tombstone (D25 §12.8.1(d))
/// even after the original handle has been reclaimed.
const REDIRECT_PREFIX: &str = "redirect:";
/// Anchored-commit cache storage (D33 §6 / Phase 20c).
///
/// Keys: `anchored:<content_hex>:<supporting_content_hex>` → 32 bytes
/// of position-hash. Probed at commit time so any deterministic
/// content generator (notebook cells, institution ontology reload,
/// mirror regeneration) that anchors to a supporting layer reuses
/// the existing layer's id when content + supporting context are
/// byte-equivalent to a previous commit (no re-execution, no new
/// chain commit).
const ANCHORED_COMMIT_PREFIX: &str = "anchored:";

/// Column family for D43's custom layer-aware text inverted index
/// (D43 §2.3). Holds `text_term:<index_iri>:<term>:<layer>`,
/// `text_docs:<index_iri>:<layer>`, `text_stats:<index_iri>:<layer>`,
/// and `text_terms_layer:<layer>:<index_iri>` keys. Isolated from the
/// default CF so text-segment compaction doesn't interleave with
/// layer / topology / triple-index churn.
pub const CF_TEXT: &str = "cf_text";

/// Column family for D43's vector index segments (D43 §2.4). Holds
/// `vec_seg:<index_iri>:<layer>` CBOR blobs and the
/// `vec_layer:<layer>:<index_iri>` reverse index. Separate CF because
/// vector blobs have different size and update profiles than text
/// postings.
pub const CF_VEC: &str = "cf_vec";

/// Column family for D43's content-addressed embedding cache
/// (D43 §5.3). Keys: `(blake3(content), model_iri)` → vector bytes.
/// Lifecycle independent of layers and traces — survives kernel
/// restarts and layer GC; evicted by LRU under a configurable budget.
pub const CF_EMBED_CACHE: &str = "cf_embed_cache";

/// All non-default column families opened by `RocksStore::open`. The
/// existing single-CF data (`layer:`, `chain:`, `topo:`, `bloom:`,
/// `branch:`, `idx_pos:`, `idx_layer:`, `meta:`, `trace:`, …) stays
/// on the default CF; D43 populates the dedicated CFs declared here
/// once M2 lands.
const D43_COLUMN_FAMILIES: &[&str] = &[CF_TEXT, CF_VEC, CF_EMBED_CACHE];

// `now_millis` removed — `LayerHandle.created_at` is now sourced from
// `Layer.created_at()` (stamped at `LayerBuilder::build` time), so the
// backend no longer generates its own timestamp.

/// RocksDB-backed storage.
pub struct RocksStore {
    db: Arc<rocksdb::DB>,
    /// RocksDB-backed `TripleIndex` (Phase 14h / D23 §5.9). Shares the
    /// same `Arc<rocksdb::DB>` as `db` so commit + index-update writes
    /// land in the same physical store. The index's atomic-batch
    /// methods (`extend_into_batch` / `drop_into_batch`) participate in
    /// `store_layer` / `delete_layer`'s `WriteBatch` so layer + index
    /// commits stay atomic per D23 §6.3.
    triple_index: Arc<RocksTripleIndex>,
    /// D43 §2.3 text index (M2.4). RocksDB-backed; shares the same
    /// `Arc<rocksdb::DB>` as `db` and `triple_index` so commits land
    /// in the same physical store. Its `extend_into_batch` /
    /// `drop_into_batch` participate in `store_layer` /
    /// `delete_layer`'s `WriteBatch` for atomic-with-layer-commit
    /// semantics (D43 §2.5).
    text_index: Arc<RocksTextIndex>,
    /// D43 §2.4 vector index (M2.5). RocksDB-backed; shares the same
    /// `Arc<rocksdb::DB>` as `db`. Segments are stored as CBOR blobs
    /// in `cf_vec` with the §2.4 layout (concatenated `vectors`
    /// bstr). Its `extend_into_batch` / `drop_into_batch` participate
    /// in `store_layer` / `delete_layer`'s `WriteBatch` (D43 §2.5).
    vector_index: Arc<RocksVectorIndex>,
    /// D65 exact value index. RocksDB-backed; shares the same
    /// `Arc<rocksdb::DB>` as `db`. Its `extend_into_batch` /
    /// `drop_into_batch` participate in `store_layer` / `delete_layer`'s
    /// `WriteBatch` for atomic-with-layer-commit semantics.
    value_index: Arc<RocksValueIndex>,
}

impl RocksStore {
    /// Open or create a RocksDB database at the given path.
    ///
    /// Opens with the three D43 column families (`cf_text`, `cf_vec`,
    /// `cf_embed_cache`) declared alongside the default CF. New DBs
    /// receive the CFs at creation time via
    /// `create_missing_column_families(true)`. Reads and writes for
    /// the existing key prefixes continue to target the default CF;
    /// D43's M2 storage substrate will route the new prefixes to
    /// their dedicated CFs.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

        let cf_descriptors: Vec<rocksdb::ColumnFamilyDescriptor> = D43_COLUMN_FAMILIES
            .iter()
            .map(|name| {
                let mut cf_opts = rocksdb::Options::default();
                cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
                rocksdb::ColumnFamilyDescriptor::new(*name, cf_opts)
            })
            .collect();

        let db = rocksdb::DB::open_cf_descriptors(&opts, path, cf_descriptors)
            .map_err(|e| StorageError::Internal(format!("failed to open RocksDB: {e}")))?;
        let db = Arc::new(db);
        let triple_index = Arc::new(RocksTripleIndex::new(Arc::clone(&db)));
        let text_index = Arc::new(RocksTextIndex::new(Arc::clone(&db)));
        let vector_index = Arc::new(RocksVectorIndex::new(Arc::clone(&db)));
        let value_index = Arc::new(RocksValueIndex::new(Arc::clone(&db)));

        Ok(Self {
            db,
            triple_index,
            text_index,
            vector_index,
            value_index,
        })
    }

    /// Trigger manual compaction on the entire database.
    pub fn compact(&self) {
        self.db.compact_range::<&[u8], &[u8]>(None, None);
    }

    /// Get a layer's parent ID from the chain.
    fn get_chain(&self, layer_id: &LayerId) -> Result<Option<LayerId>, StorageError> {
        run_blocking(|| {
            let key = format!("chain:{}", hex::encode(layer_id.0));
            match self.db.get(key.as_bytes()) {
                Ok(Some(bytes)) => {
                    let hex_str = String::from_utf8(bytes)
                        .map_err(|e| StorageError::Internal(format!("invalid chain value: {e}")))?;
                    if hex_str.is_empty() {
                        Ok(None) // Root layer
                    } else {
                        Ok(Some(hex_to_layer_id(&hex_str)?))
                    }
                }
                Ok(None) => Err(StorageError::NotFound(format!(
                    "chain entry for layer {}",
                    hex::encode(layer_id.0)
                ))),
                Err(e) => Err(StorageError::Internal(format!("failed to get chain: {e}"))),
            }
        })
    }

    /// Build a `ChainInfo` describing the chain from root → `head`. Phase 14a-iii:
    /// returns metadata only, no resource bodies; the caller turns this into
    /// an `Arc<Layer>` chain via `crate::layer::build_chain`.
    pub fn build_chain_info(
        &self,
        head: &LayerId,
    ) -> Result<Option<eigenius_kernel::storage::ChainInfo>, StorageError> {
        run_blocking(|| {
            // Walk parent pointers head → root, redirect-aware (D25 §12.8
            // / Phase 17f-F). When the walk reaches a layer that's a
            // redirect source, switch to walking the target's chain
            // instead of continuing through the (potentially reclaimed)
            // original parent pointer. v1's refuse-chaining policy
            // guarantees a single hop is enough — no cycles.
            let mut chain_ids = vec![head.clone()];
            let mut current = head.clone();
            loop {
                if let Some(redirect_entry) =
                    <Self as eigenius_kernel::storage::PersistentBackend>::lookup_redirect(
                        self, &current,
                    )?
                {
                    chain_ids.push(redirect_entry.target.clone());
                    current = redirect_entry.target;
                    continue;
                }
                match self.get_chain(&current)? {
                    Some(parent_id) => {
                        chain_ids.push(parent_id.clone());
                        current = parent_id;
                    }
                    None => break,
                }
            }
            chain_ids.reverse();

            let mut handles = Vec::with_capacity(chain_ids.len());
            let mut defined_iris_per_layer = std::collections::BTreeMap::new();
            for id in &chain_ids {
                let topo_key = format!("{TOPO_PREFIX}{}", hex::encode(id.0));
                let bytes = self
                    .db
                    .get(topo_key.as_bytes())
                    .map_err(|e| StorageError::Internal(format!("get topo entry: {e}")))?
                    .ok_or_else(|| {
                        StorageError::NotFound(format!(
                            "topo entry for layer {}",
                            hex::encode(id.0)
                        ))
                    })?;
                let handle: LayerHandle = ciborium::from_reader(bytes.as_slice())
                    .map_err(|e| StorageError::Internal(format!("decode LayerHandle: {e}")))?;
                let iris = ResourceBackend::list_layer_iris(self, id)?;
                handles.push(handle);
                defined_iris_per_layer.insert(id.clone(), iris);
            }

            Ok(Some(eigenius_kernel::storage::ChainInfo {
                head: head.clone(),
                handles,
                defined_iris_per_layer,
            }))
        })
    }

    /// Load the full `LayerHandle` for a known layer.
    ///
    /// Reads the canonical CBOR `topo:<id>` entry. There is no legacy
    /// fallback — pre-Phase-14 DBs are not supported; recovery is to drop
    /// the DB and re-load from source files.
    fn load_layer_handle(&self, layer_id: &LayerId) -> Result<LayerHandle, StorageError> {
        run_blocking(|| {
            let topo_key = format!("{TOPO_PREFIX}{}", hex::encode(layer_id.0));
            let bytes = self
                .db
                .get(topo_key.as_bytes())
                .map_err(|e| StorageError::Internal(format!("failed to load topo entry: {e}")))?
                .ok_or_else(|| {
                    StorageError::NotFound(format!("layer {}", hex::encode(layer_id.0)))
                })?;
            ciborium::from_reader(bytes.as_slice())
                .map_err(|e| StorageError::Internal(format!("decode LayerHandle: {e}")))
        })
    }

    /// Read all `topo:<id>` entries into an in-memory `LayerTopology`. Returns
    /// an empty topology if no entries exist (caller decides whether to
    /// migrate from the legacy `chain:` layout).
    fn read_topology_entries(&self) -> Result<LayerTopology, StorageError> {
        run_blocking(|| {
            let mut topology = LayerTopology::new();
            let iter = self.db.prefix_iterator(TOPO_PREFIX.as_bytes());
            for item in iter {
                let (key, value) =
                    item.map_err(|e| StorageError::Internal(format!("topology iter: {e}")))?;
                // Prefix iterator may overshoot into later keyspaces — e.g.
                // `vidx_*` value-index keys, which sort after `topo:` (`'v'` >
                // `'t'`) and carry binary layer-ids. Stop at the first key
                // outside the `topo:` prefix, checking raw bytes *before* any
                // utf-8 interpretation so a non-utf8 neighbour key can't trip
                // the decode.
                if !key.starts_with(TOPO_PREFIX.as_bytes()) {
                    break;
                }
                let handle: LayerHandle = ciborium::from_reader(value.as_ref())
                    .map_err(|e| StorageError::Internal(format!("decode LayerHandle: {e}")))?;
                topology.insert_layer(handle);
            }
            Ok(topology)
        })
    }
}

// --- Trace Store ---

use eigenius_kernel::program::trace::{ComponentMetrics, ComponentTrace, TraceStore};

impl TraceStore for RocksStore {
    fn get_component_trace(&self, key: &[u8; 32]) -> Option<ComponentTrace> {
        run_blocking(|| {
            let db_key = format!("trace:{}", hex::encode(key));
            match self.db.get(db_key.as_bytes()) {
                Ok(Some(bytes)) => deserialize_component_trace(&bytes).ok(),
                _ => None,
            }
        })
    }

    fn put_component_trace(&self, key: [u8; 32], trace: ComponentTrace) {
        run_blocking(|| {
            let db_key = format!("trace:{}", hex::encode(key));
            if let Ok(bytes) = serialize_component_trace(&trace) {
                let _ = self.db.put(db_key.as_bytes(), bytes);
            }
        });
    }
}

/// CBOR-serializable wrapper for ComponentTrace storage. The `output` is
/// pre-encoded as canonical CBOR bytes using `eigon_cbor::serialize_resource`
/// (the same encoding used for `layer:<id>:res:<iri>` entries) so we don't
/// need a generic serde impl on `Resource`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredTrace {
    component: String,
    input_hash: [u8; 32],
    argument_hash: Option<[u8; 32]>,
    output_cbor: Vec<u8>,
    metrics: Option<StoredMetrics>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredMetrics {
    provider: String,
    model: String,
    prompt_tokens: i64,
    completion_tokens: i64,
    latency_ms: i64,
}

/// Serialize a ComponentTrace to CBOR bytes for storage.
fn serialize_component_trace(trace: &ComponentTrace) -> Result<Vec<u8>, StorageError> {
    let stored = StoredTrace {
        component: trace.component.clone(),
        input_hash: trace.input_hash,
        argument_hash: trace.argument_hash,
        output_cbor: eigon_cbor::serialize_resource(&trace.output),
        metrics: trace.metrics.as_ref().map(|m| StoredMetrics {
            provider: m.provider.clone(),
            model: m.model.clone(),
            prompt_tokens: m.prompt_tokens,
            completion_tokens: m.completion_tokens,
            latency_ms: m.latency_ms,
        }),
    };
    let mut bytes = Vec::new();
    ciborium::into_writer(&stored, &mut bytes)
        .map_err(|e| StorageError::Internal(format!("serialize trace: {e}")))?;
    Ok(bytes)
}

/// Deserialize a ComponentTrace from CBOR bytes.
fn deserialize_component_trace(bytes: &[u8]) -> Result<ComponentTrace, StorageError> {
    let stored: StoredTrace = ciborium::from_reader(bytes)
        .map_err(|e| StorageError::Internal(format!("deserialize trace: {e}")))?;
    let output = eigon_cbor::parse_resource(&stored.output_cbor)
        .map_err(|e| StorageError::Internal(format!("parse trace output: {e}")))?;
    let metrics = stored.metrics.map(|m| ComponentMetrics {
        provider: m.provider,
        model: m.model,
        prompt_tokens: m.prompt_tokens,
        completion_tokens: m.completion_tokens,
        latency_ms: m.latency_ms,
    });
    Ok(ComponentTrace {
        component: stored.component,
        input_hash: stored.input_hash,
        argument_hash: stored.argument_hash,
        output,
        cached: false, // When loaded from storage, it will be marked cached by the caller
        metrics,
    })
}

fn hex_to_layer_id(hex_str: &str) -> Result<LayerId, StorageError> {
    let bytes =
        hex::decode(hex_str).map_err(|e| StorageError::Internal(format!("invalid hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(StorageError::Internal(format!(
            "layer ID must be 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes);
    Ok(LayerId(id))
}

// --- ResourceBackend (Phase 14a-iii: sync single-resource lookup) ---

impl ResourceBackend for RocksStore {
    fn load_resource(&self, layer_id: &LayerId, iri: &Iri) -> Option<Resource> {
        // Panic on storage error: matches the kernel's broken-disk failure
        // model. Use try_load_resource for fallible callers.
        match self.try_load_resource(layer_id, iri) {
            Ok(opt) => opt,
            Err(e) => panic!("RocksStore::load_resource failed: {e}"),
        }
    }

    fn try_load_resource(
        &self,
        layer_id: &LayerId,
        iri: &Iri,
    ) -> Result<Option<Resource>, StorageError> {
        run_blocking(|| {
            let key = format!("layer:{}:res:{}", hex::encode(layer_id.0), iri.as_str());
            match self.db.get(key.as_bytes()) {
                Ok(Some(bytes)) => {
                    let resource = eigon_cbor::parse_resource(&bytes)
                        .map_err(|e| StorageError::Internal(format!("CBOR parse error: {e}")))?;
                    Ok(Some(resource))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(StorageError::Internal(format!(
                    "failed to load resource: {e}"
                ))),
            }
        })
    }

    fn list_layer_iris(
        &self,
        layer_id: &LayerId,
    ) -> Result<std::collections::BTreeSet<Iri>, StorageError> {
        run_blocking(|| {
            let prefix = format!("layer:{}:res:", hex::encode(layer_id.0));
            let mut iris = std::collections::BTreeSet::new();
            let iter = self.db.prefix_iterator(prefix.as_bytes());
            for item in iter {
                let (key, _) =
                    item.map_err(|e| StorageError::Internal(format!("list_layer_iris iter: {e}")))?;
                let key_str = String::from_utf8_lossy(&key);
                if !key_str.starts_with(&prefix) {
                    break;
                }
                if let Some(iri_str) = key_str.strip_prefix(&prefix) {
                    if let Ok(iri) = Iri::parse(iri_str) {
                        iris.insert(iri);
                    }
                }
            }
            Ok(iris)
        })
    }
}

// --- PersistentBackend (D13) ---

impl eigenius_kernel::storage::PersistentBackend for RocksStore {
    fn load_chain_from(
        &self,
        head_id: &LayerId,
    ) -> Result<Option<eigenius_kernel::storage::ChainInfo>, StorageError> {
        self.build_chain_info(head_id)
    }

    fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError> {
        // D65 index lifecycle: materialise the layer's derived indexes
        // (triple → text → value) into this backend's index keyspace. `store_layer`
        // is the post-validation persist point every commit path funnels through,
        // so population happens here rather than eagerly at build — a rejected
        // commit never reaches `store_layer`, and a seeded/committed layer's
        // indexes are durable. Writes through `layer.storage()`, which is this
        // backend (the layer was built on it). Idempotent.
        eigenius_kernel::layer::populate_layer_indexes(layer);
        run_blocking(|| {
            // Per D23 §6.3, a layer commit must atomically write the topology
            // entry, the per-layer bloom (Phase 14b), every `layer:<id>:res:`
            // entry, and the chain pointer. We bundle them into one
            // `WriteBatch`; RocksDB guarantees atomicity across the batch so a
            // partial commit is impossible. (The pre-14b code used individual
            // `put` calls and relied on commit ordering — fine in practice but
            // not what the spec promises.)
            let id = layer.id().clone();
            // 14e: persist all topological parents in the LayerHandle so
            // multi-parent merge layers round-trip correctly. The
            // `chain:<id>` key below stores `parents.first()` as the
            // canonical parent for chain-walk reconstruction — consistent
            // with `Layer::parent()` semantics.
            let all_parents: Vec<LayerId> =
                layer.parents().iter().map(|p| p.id().clone()).collect();
            let canonical_parent = all_parents.first().cloned();

            // Pre-serialize resources so we can both stamp the handle's
            // `byte_size` (sum of encoded resource bytes — drives GC's
            // reclaim estimate) and write the values into the batch
            // without re-encoding.
            // One walk, three stamps: the encoded bytes, `byte_size`, and the D66
            // witness-scan skip hint.
            let mut has_witness_candidates = false;
            let encoded: Vec<(Iri, Vec<u8>)> = layer
                .iter_resources()
                .map(|(iri, resource)| {
                    has_witness_candidates |=
                        eigenius_kernel::layer::is_witness_candidate(&resource);
                    (iri, eigon_cbor::serialize_resource(&resource))
                })
                .collect();
            let byte_size = encoded.iter().map(|(_, v)| v.len() as u64).sum::<u64>();

            let handle = LayerHandle {
                id: id.clone(),
                content_hash: layer.content_hash().clone(),
                supporting_layer: layer.supporting_layer().cloned(),
                parents: all_parents,
                name: layer.name().to_string(),
                resource_count: layer.defined_iris().len() as u64,
                has_witness_candidates,
                // Copy the build-time stamp set by `LayerBuilder::build`
                // rather than taking `now_millis()` here; keeps the
                // in-memory Layer and persisted handle consistent on
                // `created_at`.
                created_at: layer.created_at(),
                byte_size,
                is_redirect_source: false,
                // 15g step 3: persist tombstones onto the handle so the
                // round-trip preserves them through `load_chain_from`.
                tombstoned_iris: layer.tombstoned_iris().clone(),
            };
            let bloom = BloomFilter::for_layer(layer.defined_iris(), layer.tombstoned_iris());

            // Encode CBOR payloads outside the batch — encoding is CPU work
            // and can fail; no point holding the batch while computing.
            let mut handle_bytes = Vec::new();
            ciborium::into_writer(&handle, &mut handle_bytes)
                .map_err(|e| StorageError::Internal(format!("encode LayerHandle: {e}")))?;
            let mut bloom_bytes = Vec::new();
            ciborium::into_writer(&bloom, &mut bloom_bytes)
                .map_err(|e| StorageError::Internal(format!("encode BloomFilter: {e}")))?;

            let mut batch = rocksdb::WriteBatch::default();

            let topo_key = format!("{TOPO_PREFIX}{}", hex::encode(id.0));
            batch.put(topo_key.as_bytes(), &handle_bytes);

            let bloom_key = format!("{BLOOM_PREFIX}{}", hex::encode(id.0));
            batch.put(bloom_key.as_bytes(), &bloom_bytes);

            for (iri, value) in &encoded {
                let key = format!("layer:{}:res:{}", hex::encode(id.0), iri.as_str());
                batch.put(key.as_bytes(), value);
            }

            let chain_key = format!("chain:{}", hex::encode(id.0));
            let chain_value = match canonical_parent.as_ref() {
                Some(pid) => hex::encode(pid.0),
                None => String::new(),
            };
            batch.put(chain_key.as_bytes(), chain_value.as_bytes());

            // Content-hash dedup index (D25 §11.0 / D33 §6). Each entry is a
            // `content:<content_hex>:<position_hex>` key with an empty value;
            // the key existence is the signal. Idempotent by
            // `(content_hash, position_hash)` so re-storing the same layer
            // is a structural no-op at the index's logical level.
            let content_key = format!(
                "{CONTENT_INDEX_PREFIX}{}:{}",
                hex::encode(layer.content_hash().0),
                hex::encode(id.0)
            );
            batch.put(content_key.as_bytes(), []);

            // Phase 14h: index entries are populated by `LayerBuilder::build`
            // (same precomputation pattern as the bloom). The persistent
            // index is shared (`RocksStore.triple_index` ↔ `LayerStorage.triple_index`)
            // so the build-time `extend_layer` already wrote them to RocksDB.
            // No duplicate population here.

            // Sync write: layer commits are durability-critical. The kernel
            // writes the layer + branch CAS sequentially, and a verdict
            // resource committed via AutoOnLoad must survive a SIGKILL'd
            // restart (the audit-chain promise of D28 §5.7). RocksDB's
            // default async-WAL-flush mode lets writes return before the
            // WAL fsyncs, opening a sub-second window where a forced kill
            // loses the just-committed layer. `sync = true` makes the WAL
            // fsync inline. Per-commit latency cost is ~10ms on local disk —
            // dwarfed by the nanoda proof-check costs we already accept on
            // institution-gated commits.
            let mut write_opts = rocksdb::WriteOptions::default();
            write_opts.set_sync(true);
            self.db
                .write_opt(batch, &write_opts)
                .map_err(|e| StorageError::Internal(format!("store_layer batch: {e}")))?;
            // Resources are now durable on the backend — drain this layer's `pending`
            // stage (D23 write path) so its in-memory copy is released; later reads page
            // through the bounded cache. Drained only on success, and only when the
            // layer's storage is backed by a persistent backend (so reads can page the
            // resources back): for backend-less `in_memory()` storage the stage is the
            // only read home, so it must persist.
            if layer.storage().persistent_backend.is_some() {
                layer
                    .storage()
                    .pending
                    .write()
                    .expect("pending stage poisoned")
                    .remove(layer.id());
            }
            Ok(id)
        })
    }

    fn load_topology(&self) -> Result<LayerTopology, StorageError> {
        run_blocking(|| {
            let mut topology = self.read_topology_entries()?;
            // D25 §12.8.1(d): manufacture synthetic tombstones for every
            // redirect source whose original handle was reclaimed.
            let entries = eigenius_kernel::storage::PersistentBackend::list_redirects(self)?;
            eigenius_kernel::layer::augment_topology_with_redirects(&mut topology, &entries);
            Ok(topology)
        })
    }

    fn load_handle(&self, layer_id: &LayerId) -> Result<Option<LayerHandle>, StorageError> {
        run_blocking(|| {
            // Real handle: read `topo:<id>` directly.
            let topo_key = format!("{TOPO_PREFIX}{}", hex::encode(layer_id.0));
            match self.db.get(topo_key.as_bytes()) {
                Ok(Some(bytes)) => {
                    let handle = ciborium::from_reader(bytes.as_slice())
                        .map_err(|e| StorageError::Internal(format!("decode LayerHandle: {e}")))?;
                    return Ok(Some(handle));
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(StorageError::Internal(format!(
                        "failed to load topo entry: {e}"
                    )))
                }
            }
            // Synthetic tombstone via the redirect CF (D25 §12.8.1(d)) —
            // matches `load_topology`'s view for any redirect source whose
            // original on-disk handle has been reclaimed.
            if let Some(entry) =
                eigenius_kernel::storage::PersistentBackend::lookup_redirect(self, layer_id)?
            {
                return Ok(Some(eigenius_kernel::layer::manufacture_tombstone(&entry)));
            }
            Ok(None)
        })
    }

    fn get_meta(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        run_blocking(|| {
            let db_key = format!("meta:{key}");
            self.db
                .get(db_key.as_bytes())
                .map_err(|e| StorageError::Internal(format!("meta get: {e}")))
        })
    }

    fn put_meta(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        run_blocking(|| {
            let db_key = format!("meta:{key}");
            self.db
                .put(db_key.as_bytes(), value)
                .map_err(|e| StorageError::Internal(format!("meta put: {e}")))
        })
    }

    fn delete_meta(&self, key: &str) -> Result<(), StorageError> {
        run_blocking(|| {
            let db_key = format!("meta:{key}");
            self.db
                .delete(db_key.as_bytes())
                .map_err(|e| StorageError::Internal(format!("meta delete: {e}")))
        })
    }

    fn write_batch(&self, ops: &[eigenius_kernel::storage::BatchOp]) -> Result<(), StorageError> {
        run_blocking(|| {
            use eigenius_kernel::storage::BatchOp;
            let mut batch = rocksdb::WriteBatch::default();
            for op in ops {
                match op {
                    BatchOp::PutMeta { key, value } => {
                        let db_key = format!("meta:{key}");
                        batch.put(db_key.as_bytes(), value);
                    }
                    BatchOp::DeleteMeta { key } => {
                        let db_key = format!("meta:{key}");
                        batch.delete(db_key.as_bytes());
                    }
                }
            }
            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("write_batch: {e}")))
        })
    }

    fn list_meta_prefix(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        run_blocking(|| {
            let db_prefix = format!("meta:{prefix}");
            let mut out = Vec::new();
            let iter = self.db.prefix_iterator(db_prefix.as_bytes());
            for item in iter {
                let (k, _v) =
                    item.map_err(|e| StorageError::Internal(format!("list_meta_prefix: {e}")))?;
                let key_str = std::str::from_utf8(&k)
                    .map_err(|e| StorageError::Internal(format!("non-utf8 meta key: {e}")))?;
                // Prefix iterator may overshoot — trim.
                if !key_str.starts_with(&db_prefix) {
                    break;
                }
                out.push(key_str["meta:".len()..].to_string());
            }
            Ok(out)
        })
    }

    fn as_trace_store(&self) -> &(dyn eigenius_kernel::program::trace::TraceStore + Send + Sync) {
        self
    }

    fn triple_index_arc(&self) -> Arc<dyn eigenius_kernel::layer::TripleIndex> {
        Arc::clone(&self.triple_index) as Arc<dyn eigenius_kernel::layer::TripleIndex>
    }

    fn text_index_arc(&self) -> Arc<dyn eigenius_kernel::layer::TextIndex> {
        Arc::clone(&self.text_index) as Arc<dyn eigenius_kernel::layer::TextIndex>
    }

    fn vector_index_arc(&self) -> Arc<dyn eigenius_kernel::layer::VectorIndex> {
        Arc::clone(&self.vector_index) as Arc<dyn eigenius_kernel::layer::VectorIndex>
    }

    fn value_index_arc(&self) -> Arc<dyn eigenius_kernel::layer::ValueIndex> {
        Arc::clone(&self.value_index) as Arc<dyn eigenius_kernel::layer::ValueIndex>
    }

    fn load_bloom(&self, layer: &LayerId) -> Result<Option<BloomFilter>, StorageError> {
        run_blocking(|| {
            let key = format!("{BLOOM_PREFIX}{}", hex::encode(layer.0));
            match self.db.get(key.as_bytes()) {
                Ok(Some(bytes)) => {
                    let bloom: BloomFilter = ciborium::from_reader(bytes.as_slice())
                        .map_err(|e| StorageError::Internal(format!("decode BloomFilter: {e}")))?;
                    Ok(Some(bloom))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(StorageError::Internal(format!("load_bloom: {e}"))),
            }
        })
    }

    fn store_bloom(&self, layer: &LayerId, bloom: &BloomFilter) -> Result<(), StorageError> {
        run_blocking(|| {
            let key = format!("{BLOOM_PREFIX}{}", hex::encode(layer.0));
            let mut bytes = Vec::new();
            ciborium::into_writer(bloom, &mut bytes)
                .map_err(|e| StorageError::Internal(format!("encode BloomFilter: {e}")))?;
            self.db
                .put(key.as_bytes(), bytes)
                .map_err(|e| StorageError::Internal(format!("store_bloom: {e}")))
        })
    }

    fn get_branch(&self, name: &str) -> Result<Option<LayerId>, StorageError> {
        run_blocking(|| {
            let key = format!("{BRANCH_PREFIX}{name}");
            match self.db.get(key.as_bytes()) {
                Ok(Some(bytes)) => {
                    let hex_str = String::from_utf8(bytes).map_err(|e| {
                        StorageError::Internal(format!("invalid branch ref value: {e}"))
                    })?;
                    Ok(Some(hex_to_layer_id(&hex_str)?))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(StorageError::Internal(format!("get_branch: {e}"))),
            }
        })
    }

    fn put_branch(&self, name: &str, id: &LayerId) -> Result<(), StorageError> {
        run_blocking(|| {
            // Sync write: the branch ref is the entry point for every chain
            // walk. A `put_branch` returning ok without the WAL fsync'd
            // means a SIGKILL'd restart sees the *old* branch ref — the
            // newly-committed layer (already stored via `store_layer` with
            // sync, above) becomes an unreachable orphan, and the audit
            // chain's `verified` claims silently disappear (D28 §5.7).
            // Same ~10ms latency cost as the `store_layer` sync write.
            let key = format!("{BRANCH_PREFIX}{name}");
            let mut write_opts = rocksdb::WriteOptions::default();
            write_opts.set_sync(true);
            self.db
                .put_opt(key.as_bytes(), hex::encode(id.0), &write_opts)
                .map_err(|e| StorageError::Internal(format!("put_branch: {e}")))
        })
    }

    fn delete_branch(&self, name: &str) -> Result<(), StorageError> {
        run_blocking(|| {
            let key = format!("{BRANCH_PREFIX}{name}");
            self.db
                .delete(key.as_bytes())
                .map_err(|e| StorageError::Internal(format!("delete_branch: {e}")))
        })
    }

    fn delete_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        run_blocking(|| {
            let id_hex = hex::encode(layer.0);
            // Per D23 §6.3, layer-shape mutations land via one WriteBatch.
            // Atomic across the topology entry, bloom, chain pointer,
            // every resource entry, the content-hash index entry, and
            // (Phase 14h) every triple-index entry — no partial state
            // visible after a crash mid-delete.

            // Read the topology entry *before* the batch builds — we need
            // the content hash to know which content-index key to delete.
            // If the layer is absent, delete_layer is a no-op (idempotent
            // contract); a `None` content hash falls through to the rest of
            // the cleanup which is also no-op on absent keys.
            let content_hash_hex = self
                .load_layer_handle(layer)
                .ok()
                .map(|h| hex::encode(h.content_hash.0));

            let mut batch = rocksdb::WriteBatch::default();

            let topo_key = format!("{TOPO_PREFIX}{id_hex}");
            batch.delete(topo_key.as_bytes());

            if let Some(ch_hex) = content_hash_hex {
                let content_key = format!("{CONTENT_INDEX_PREFIX}{ch_hex}:{id_hex}");
                batch.delete(content_key.as_bytes());
            }

            let bloom_key = format!("{BLOOM_PREFIX}{id_hex}");
            batch.delete(bloom_key.as_bytes());

            let chain_key = format!("chain:{id_hex}");
            batch.delete(chain_key.as_bytes());

            // Resource entries: prefix-scan + per-key delete inside the
            // batch. RocksDB's `delete_range` is faster but has subtle
            // interactions with snapshot iterators we don't want to pull
            // in for v1; per-key delete is correct and fast enough for
            // typical layer sizes.
            let res_prefix = format!("layer:{id_hex}:res:");
            let iter = self.db.prefix_iterator(res_prefix.as_bytes());
            for item in iter {
                let (k, _v) =
                    item.map_err(|e| StorageError::Internal(format!("delete_layer iter: {e}")))?;
                if !k.starts_with(res_prefix.as_bytes()) {
                    break;
                }
                batch.delete(&k);
            }

            // Phase 14h: drop both index orderings for this layer in the
            // same atomic batch. Walks the reverse `idx_layer:<L>:` prefix
            // to discover which forward `idx_pos:` entries to delete.
            self.triple_index.drop_into_batch(&mut batch, layer)?;

            // D43 M2.7: drop the per-Index text and vector entries
            // contributed by this layer in the same atomic batch.
            // Both walks use their reverse-index prefix
            // (`text_terms_layer:<L>:` / `vec_layer:<L>:`) so cleanup
            // cost is proportional to the layer's contributions, not
            // the total index size.
            self.text_index.drop_into_batch(&mut batch, layer)?;
            self.vector_index.drop_into_batch(&mut batch, layer)?;

            // D65: drop this layer's exact-value-index entries in the same
            // atomic batch, walking the reverse `vidx_layer:<L>:` prefix.
            self.value_index.drop_into_batch(&mut batch, layer)?;

            self.db
                .write(batch)
                .map_err(|e| StorageError::Internal(format!("delete_layer batch: {e}")))?;
            Ok(())
        })
    }

    fn list_branches(&self) -> Result<Vec<(String, LayerId)>, StorageError> {
        run_blocking(|| {
            let mut out = Vec::new();
            let iter = self.db.prefix_iterator(BRANCH_PREFIX.as_bytes());
            for item in iter {
                let (k, v) =
                    item.map_err(|e| StorageError::Internal(format!("list_branches iter: {e}")))?;
                let key_str = std::str::from_utf8(&k)
                    .map_err(|e| StorageError::Internal(format!("non-utf8 branch key: {e}")))?;
                // Prefix iterator may overshoot.
                if !key_str.starts_with(BRANCH_PREFIX) {
                    break;
                }
                let name = key_str[BRANCH_PREFIX.len()..].to_string();
                let hex_str = std::str::from_utf8(&v)
                    .map_err(|e| StorageError::Internal(format!("non-utf8 branch value: {e}")))?;
                let id = hex_to_layer_id(hex_str)?;
                out.push((name, id));
            }
            // BTreeMap-style sort for deterministic ordering even though
            // prefix-scan already yields sorted keys; defensive against
            // future column-family layout changes.
            out.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(out)
        })
    }

    fn create_tag(&self, name: &str, id: &LayerId) -> Result<bool, StorageError> {
        run_blocking(|| {
            let key = format!("{TAG_PREFIX}{name}");
            // Read-then-write under a single column family is safe enough
            // for tag creation: tags are administrator-driven operations,
            // not a hot write path, and a race between two `CreateTag`
            // calls just means whichever lost gets `Ok(false)` — the
            // intended "AlreadyExists" semantic. For genuine atomicity
            // we'd need a transactional DB; v1 ships with the simpler
            // shape.
            match self.db.get(key.as_bytes()) {
                Ok(Some(_)) => Ok(false),
                Ok(None) => {
                    self.db
                        .put(key.as_bytes(), hex::encode(id.0))
                        .map_err(|e| StorageError::Internal(format!("create_tag: {e}")))?;
                    Ok(true)
                }
                Err(e) => Err(StorageError::Internal(format!("create_tag get: {e}"))),
            }
        })
    }

    fn get_tag(&self, name: &str) -> Result<Option<LayerId>, StorageError> {
        run_blocking(|| {
            let key = format!("{TAG_PREFIX}{name}");
            match self.db.get(key.as_bytes()) {
                Ok(Some(bytes)) => {
                    let hex_str = String::from_utf8(bytes)
                        .map_err(|e| StorageError::Internal(format!("invalid tag value: {e}")))?;
                    Ok(Some(hex_to_layer_id(&hex_str)?))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(StorageError::Internal(format!("get_tag: {e}"))),
            }
        })
    }

    fn delete_tag(&self, name: &str) -> Result<bool, StorageError> {
        run_blocking(|| {
            let key = format!("{TAG_PREFIX}{name}");
            // Same read-then-delete shape: tag deletion is rare and the
            // race window is harmless (both deleters succeed; second
            // returns `false`).
            let existed = self
                .db
                .get(key.as_bytes())
                .map_err(|e| StorageError::Internal(format!("delete_tag get: {e}")))?
                .is_some();
            if existed {
                self.db
                    .delete(key.as_bytes())
                    .map_err(|e| StorageError::Internal(format!("delete_tag: {e}")))?;
            }
            Ok(existed)
        })
    }

    fn list_tags(&self) -> Result<Vec<(String, LayerId)>, StorageError> {
        run_blocking(|| {
            let mut out = Vec::new();
            let iter = self.db.prefix_iterator(TAG_PREFIX.as_bytes());
            for item in iter {
                let (k, v) =
                    item.map_err(|e| StorageError::Internal(format!("list_tags iter: {e}")))?;
                let key_str = std::str::from_utf8(&k)
                    .map_err(|e| StorageError::Internal(format!("non-utf8 tag key: {e}")))?;
                if !key_str.starts_with(TAG_PREFIX) {
                    break;
                }
                let name = key_str[TAG_PREFIX.len()..].to_string();
                let hex_str = std::str::from_utf8(&v)
                    .map_err(|e| StorageError::Internal(format!("non-utf8 tag value: {e}")))?;
                let id = hex_to_layer_id(hex_str)?;
                out.push((name, id));
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(out)
        })
    }

    fn lookup_by_content_hash(
        &self,
        content_hash: &ContentHash,
    ) -> Result<Vec<LayerId>, StorageError> {
        run_blocking(|| {
            // Prefix-scan `content:<content_hex>:` — every key matching the
            // prefix names a position currently in storage that shares the
            // given content hash. The position-hash hex follows the content
            // hash and the separator inside the key, so a substring slice
            // recovers it without parsing.
            let prefix = format!("{CONTENT_INDEX_PREFIX}{}:", hex::encode(content_hash.0));
            let mut out = Vec::new();
            let iter = self.db.prefix_iterator(prefix.as_bytes());
            for item in iter {
                let (k, _v) = item.map_err(|e| {
                    StorageError::Internal(format!("lookup_by_content_hash iter: {e}"))
                })?;
                let key_str = std::str::from_utf8(&k).map_err(|e| {
                    StorageError::Internal(format!("non-utf8 content-index key: {e}"))
                })?;
                // Prefix iterator may overshoot — trim.
                if !key_str.starts_with(&prefix) {
                    break;
                }
                let pos_hex = &key_str[prefix.len()..];
                out.push(hex_to_layer_id(pos_hex)?);
            }
            Ok(out)
        })
    }

    fn put_redirect(&self, entry: &RedirectEntry) -> Result<(), StorageError> {
        run_blocking(|| {
            let key = format!("{REDIRECT_PREFIX}{}", hex::encode(entry.source().0));
            let mut bytes = Vec::new();
            ciborium::into_writer(entry, &mut bytes)
                .map_err(|e| StorageError::Internal(format!("encode RedirectEntry: {e}")))?;
            self.db
                .put(key.as_bytes(), bytes)
                .map_err(|e| StorageError::Internal(format!("put redirect: {e}")))
        })
    }

    fn lookup_redirect(&self, source: &LayerId) -> Result<Option<RedirectEntry>, StorageError> {
        run_blocking(|| {
            let key = format!("{REDIRECT_PREFIX}{}", hex::encode(source.0));
            match self.db.get(key.as_bytes()) {
                Ok(Some(bytes)) => {
                    let entry: RedirectEntry =
                        ciborium::from_reader(bytes.as_slice()).map_err(|e| {
                            StorageError::Internal(format!("decode RedirectEntry: {e}"))
                        })?;
                    Ok(Some(entry))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(StorageError::Internal(format!("get redirect: {e}"))),
            }
        })
    }

    fn delete_redirect(&self, source: &LayerId) -> Result<(), StorageError> {
        run_blocking(|| {
            let key = format!("{REDIRECT_PREFIX}{}", hex::encode(source.0));
            self.db
                .delete(key.as_bytes())
                .map_err(|e| StorageError::Internal(format!("delete redirect: {e}")))
        })
    }

    fn list_redirects(&self) -> Result<Vec<RedirectEntry>, StorageError> {
        run_blocking(|| {
            let mut out = Vec::new();
            let iter = self.db.prefix_iterator(REDIRECT_PREFIX.as_bytes());
            for item in iter {
                let (k, v) =
                    item.map_err(|e| StorageError::Internal(format!("list_redirects iter: {e}")))?;
                // Prefix iterator may overshoot — trim.
                if !k.starts_with(REDIRECT_PREFIX.as_bytes()) {
                    break;
                }
                let entry: RedirectEntry = ciborium::from_reader(v.as_ref())
                    .map_err(|e| StorageError::Internal(format!("decode RedirectEntry: {e}")))?;
                out.push(entry);
            }
            Ok(out)
        })
    }

    fn lookup_anchored_commit(
        &self,
        content_hash: &ContentHash,
        supporting_content_hash: &ContentHash,
    ) -> Result<Option<LayerId>, StorageError> {
        run_blocking(|| {
            let key = format!(
                "{ANCHORED_COMMIT_PREFIX}{}:{}",
                hex::encode(content_hash.0),
                hex::encode(supporting_content_hash.0)
            );
            match self.db.get(key.as_bytes()) {
                Ok(Some(bytes)) => {
                    if bytes.len() != 32 {
                        return Err(StorageError::Internal(format!(
                            "anchored_commit value has length {}, expected 32",
                            bytes.len()
                        )));
                    }
                    let mut id = [0u8; 32];
                    id.copy_from_slice(&bytes);
                    Ok(Some(LayerId(id)))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(StorageError::Internal(format!("get anchored_commit: {e}"))),
            }
        })
    }

    fn put_anchored_commit(
        &self,
        content_hash: &ContentHash,
        supporting_content_hash: &ContentHash,
        layer_id: &LayerId,
    ) -> Result<(), StorageError> {
        run_blocking(|| {
            let key = format!(
                "{ANCHORED_COMMIT_PREFIX}{}:{}",
                hex::encode(content_hash.0),
                hex::encode(supporting_content_hash.0)
            );
            self.db
                .put(key.as_bytes(), layer_id.0)
                .map_err(|e| StorageError::Internal(format!("put anchored_commit: {e}")))
        })
    }

    fn delete_anchored_commit(
        &self,
        content_hash: &ContentHash,
        supporting_content_hash: &ContentHash,
    ) -> Result<(), StorageError> {
        run_blocking(|| {
            let key = format!(
                "{ANCHORED_COMMIT_PREFIX}{}:{}",
                hex::encode(content_hash.0),
                hex::encode(supporting_content_hash.0)
            );
            self.db
                .delete(key.as_bytes())
                .map_err(|e| StorageError::Internal(format!("delete anchored_commit: {e}")))
        })
    }

    fn list_anchored_commits(
        &self,
    ) -> Result<Vec<eigenius_kernel::storage::AnchoredCommitEntry>, StorageError> {
        run_blocking(|| {
            let mut out = Vec::new();
            let iter = self.db.prefix_iterator(ANCHORED_COMMIT_PREFIX.as_bytes());
            for item in iter {
                let (k, v) = item.map_err(|e| {
                    StorageError::Internal(format!("list_anchored_commit iter: {e}"))
                })?;
                if !k.starts_with(ANCHORED_COMMIT_PREFIX.as_bytes()) {
                    break;
                }
                let key_str = std::str::from_utf8(&k).map_err(|e| {
                    StorageError::Internal(format!("non-utf8 anchored_commit key: {e}"))
                })?;
                // Parse `cell:<content_hex>:<supporting_content_hex>`.
                let rest = &key_str[ANCHORED_COMMIT_PREFIX.len()..];
                let mut parts = rest.splitn(2, ':');
                let content_hex = parts.next().ok_or_else(|| {
                    StorageError::Internal("malformed anchored_commit key".to_string())
                })?;
                let supporting_hex = parts.next().ok_or_else(|| {
                    StorageError::Internal(
                        "malformed anchored_commit key (no supporting)".to_string(),
                    )
                })?;
                let content_hash = hex_to_content_hash(content_hex)?;
                let supporting_content_hash = hex_to_content_hash(supporting_hex)?;
                if v.len() != 32 {
                    return Err(StorageError::Internal(format!(
                        "anchored_commit value has length {}, expected 32",
                        v.len()
                    )));
                }
                let mut id = [0u8; 32];
                id.copy_from_slice(&v);
                out.push(eigenius_kernel::storage::AnchoredCommitEntry {
                    content_hash,
                    supporting_content_hash,
                    layer_id: LayerId(id),
                });
            }
            Ok(out)
        })
    }
}

fn hex_to_content_hash(s: &str) -> Result<ContentHash, StorageError> {
    let bytes =
        hex::decode(s).map_err(|e| StorageError::Internal(format!("bad content_hash hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(StorageError::Internal(format!(
            "content_hash hex must be 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut h = [0u8; 32];
    h.copy_from_slice(&bytes);
    Ok(ContentHash(h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::ontology::eigon_json;
    use eigenius_kernel::ontology::resource::Value;
    use tempfile::TempDir;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_resource(id: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        for (k, v) in props {
            r.set(iri(k), v);
        }
        r
    }

    fn open_temp_store() -> (RocksStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = RocksStore::open(dir.path()).unwrap();
        (store, dir)
    }

    // Phase-0 async `LayerStore` / `ResourceStore` smoke tests were
    // removed when those traits were deleted. The `PersistentBackend`
    // (sync) equivalents are exercised by the integration tests in
    // `storage/rocksdb/tests/` and by the `cbor_coverage_tests` and
    // `topology_tests` sub-modules below.

    // Phase 14g: the legacy `head_pointer` test was removed. The
    // pre-Phase-14 single-head pointer (`set_head`/`get_head`) is gone;
    // branches via `put_branch`/`get_branch` are the only head-pointer
    // surface. Branch-ref round-trip is exercised by
    // `cbor_coverage_tests::branch_refs_round_trip` below.

    /// Round-trip pin for PR 0: every hash carried by a layer
    /// (`content_hash`, `supporting_layer`, plus the position hash via
    /// `id`) survives store + reload through the RocksDB topology
    /// entry. Catches any drift between `LayerHandle`'s on-disk shape
    /// and `Layer`'s constructor.
    #[test]
    fn pr0_two_hash_and_supporting_layer_round_trip() {
        use eigenius_kernel::storage::PersistentBackend;
        let (store, _dir) = open_temp_store();

        // Root layer defines a class the child will reference.
        let mut root_b = LayerBuilder::new("root", None);
        root_b
            .add_resource(make_resource("urn:eigenius:core:ClassA", vec![]))
            .unwrap();
        let root = Arc::new(root_b.build(eigenius_kernel::layer::LayerStorage::in_memory()));
        PersistentBackend::store_layer(&store, &root).unwrap();

        // Child layer references the root class so its supporting
        // layer resolves to a concrete ancestor (not `None`).
        let mut child_b = LayerBuilder::new("child", Some(Arc::clone(&root)));
        let mut r = Resource::new(iri("urn:eigenius:demo:X"));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:core:ClassA"))]),
        );
        child_b.add_resource(r).unwrap();
        let child = child_b.build(eigenius_kernel::layer::LayerStorage::in_memory());
        let expected_position = child.id().clone();
        let expected_content = child.content_hash().clone();
        let expected_supporting = child.supporting_layer().cloned();
        assert_eq!(expected_supporting.as_ref(), Some(root.id()));
        PersistentBackend::store_layer(&store, &child).unwrap();

        // Reload the topology entry directly — this is the on-disk
        // shape the resume path consults.
        let handle = store.load_layer_handle(&expected_position).unwrap();
        assert_eq!(handle.id, expected_position);
        assert_eq!(handle.content_hash, expected_content);
        assert_eq!(handle.supporting_layer, expected_supporting);

        // Reload the full chain via the production path and confirm
        // the reconstructed `Layer` carries the same hashes.
        let info = PersistentBackend::load_chain_from(&store, &expected_position)
            .unwrap()
            .expect("chain present");
        let rebuilt = eigenius_kernel::layer::build_chain(
            info,
            eigenius_kernel::layer::LayerStorage::in_memory(),
        );
        assert_eq!(rebuilt.id(), &expected_position);
        assert_eq!(rebuilt.content_hash(), &expected_content);
        assert_eq!(rebuilt.supporting_layer(), expected_supporting.as_ref());
    }

    /// Phase 20c: anchored-commit cache round-trip on the RocksDB
    /// backend, including persistence across store reopen — the
    /// cache is on-disk state, not just an in-memory map.
    #[test]
    fn anchored_commit_cache_round_trip_rocksdb() {
        use eigenius_kernel::layer::{ContentHash, LayerId};
        use eigenius_kernel::storage::PersistentBackend;

        let dir = TempDir::new().unwrap();
        let content_a = ContentHash([1u8; 32]);
        let support_x = ContentHash([3u8; 32]);
        let layer_one = LayerId([0x10; 32]);
        let content_b = ContentHash([2u8; 32]);
        let support_y = ContentHash([4u8; 32]);
        let layer_two = LayerId([0x20; 32]);

        // Write: insert two entries.
        {
            let store = RocksStore::open(dir.path()).unwrap();
            store
                .put_anchored_commit(&content_a, &support_x, &layer_one)
                .unwrap();
            store
                .put_anchored_commit(&content_b, &support_y, &layer_two)
                .unwrap();
        }

        // Reopen: both entries persist; list returns them.
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let hit = store
                .lookup_anchored_commit(&content_a, &support_x)
                .unwrap()
                .expect("cache entry survives reopen");
            assert_eq!(hit, layer_one);

            let mut entries = store.list_anchored_commits().unwrap();
            entries.sort_by_key(|e| e.content_hash.0);
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].layer_id, layer_one);
            assert_eq!(entries[1].layer_id, layer_two);

            // Different content + different supporting → miss.
            assert!(store
                .lookup_anchored_commit(&content_b, &support_x)
                .unwrap()
                .is_none());
            assert!(store
                .lookup_anchored_commit(&content_a, &support_y)
                .unwrap()
                .is_none());

            // Delete one entry; the other remains.
            store
                .delete_anchored_commit(&content_a, &support_x)
                .unwrap();
        }

        // Reopen once more: the deletion persisted.
        {
            let store = RocksStore::open(dir.path()).unwrap();
            assert!(store
                .lookup_anchored_commit(&content_a, &support_x)
                .unwrap()
                .is_none());
            let remaining = store.list_anchored_commits().unwrap();
            assert_eq!(remaining.len(), 1);
            assert_eq!(remaining[0].layer_id, layer_two);
        }
    }

    /// Phase 17f-A: redirect entries round-trip through RocksDB and
    /// survive a store reopen. After deleting the redirect-source's
    /// topology entry, `load_topology` manufactures a synthetic
    /// tombstone with `is_redirect_source = true` from the redirect
    /// CF — matches the in-memory backend's behavior in
    /// `redirect_round_trip_and_synthetic_tombstone`.
    #[test]
    fn redirect_round_trip_persists_across_reopen() {
        use eigenius_kernel::storage::PersistentBackend;
        let dir = TempDir::new().unwrap();
        let target_id = eigenius_kernel::layer::LayerId([0xab; 32]);

        let source_id;
        let source_name;

        // Write: store a layer, install a redirect against it, reclaim
        // the original topology entry.
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let mut sb = LayerBuilder::new("redirect-source", None);
            sb.add_resource(make_resource("urn:eigenius:core:r", vec![]))
                .unwrap();
            let source =
                std::sync::Arc::new(sb.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            PersistentBackend::store_layer(&store, &source).unwrap();
            source_id = source.id().clone();

            let topo = PersistentBackend::load_topology(&store).unwrap();
            source_name = topo.get_layer(&source_id).unwrap().name.clone();
            let source_handle = topo.get_layer(&source_id).unwrap().clone();
            let entry = eigenius_kernel::layer::RedirectEntry {
                target: target_id.clone(),
                source_handle,
                preserve_history: false,
            };
            PersistentBackend::put_redirect(&store, &entry).unwrap();
            PersistentBackend::delete_layer(&store, &source_id).unwrap();
        }

        // Reopen: redirect persists; load_topology manufactures the
        // synthetic tombstone with the original metadata preserved.
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let entry = PersistentBackend::lookup_redirect(&store, &source_id)
                .unwrap()
                .expect("redirect persisted");
            assert_eq!(entry.target, target_id);
            assert_eq!(entry.source(), &source_id);
            assert_eq!(entry.source_handle.name, source_name);

            let topo = PersistentBackend::load_topology(&store).unwrap();
            let tombstone = topo
                .get_layer(&source_id)
                .expect("synthetic tombstone manufactured");
            assert!(tombstone.is_redirect_source);
            assert_eq!(tombstone.id, source_id);
            assert_eq!(tombstone.name, source_name);

            // list_redirects returns the same entry.
            let listed = PersistentBackend::list_redirects(&store).unwrap();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].target, target_id);
        }
    }

    /// Phase 17f-B: resolve walk follows an installed redirect.
    /// Set up an alternate-content target layer, install a redirect
    /// from a source layer to that target, rebuild the chain, and
    /// verify head-rooted reads now return the *target's* content
    /// for IRIs defined only there. The redirect short-circuit is
    /// the only path that could change the answer — the source
    /// layer's own content is unchanged.
    #[test]
    fn resolve_walk_follows_installed_redirect() {
        use eigenius_kernel::storage::PersistentBackend;
        let dir = TempDir::new().unwrap();
        let store_arc: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(dir.path()).unwrap());

        // Root holds the property declarations the chain references.
        let storage_for_build = eigenius_kernel::layer::LayerStorage::in_memory();
        let mut rb = LayerBuilder::new("root", None);
        rb.add_resource(make_resource("urn:eigenius:core:Class", vec![]))
            .unwrap();
        rb.add_resource(make_resource("urn:eigenius:core:description", vec![]))
            .unwrap();
        let root = Arc::new(rb.build(storage_for_build.clone()));
        store_arc.store_layer(&root).unwrap();

        // Source layer claims `demo:X` with value "from-source".
        let mut sb = LayerBuilder::new("source", Some(Arc::clone(&root)));
        sb.add_resource(make_resource(
            "urn:eigenius:demo:X",
            vec![(
                "urn:eigenius:core:description",
                Value::String("from-source".into()),
            )],
        ))
        .unwrap();
        let source = Arc::new(sb.build(storage_for_build.clone()));
        store_arc.store_layer(&source).unwrap();

        // Target layer (the would-be consolidated layer) claims
        // `demo:X` with a different value "from-target". Parent is
        // root, just like `source`.
        let mut tb = LayerBuilder::new("target", Some(Arc::clone(&root)));
        tb.add_resource(make_resource(
            "urn:eigenius:demo:X",
            vec![(
                "urn:eigenius:core:description",
                Value::String("from-target".into()),
            )],
        ))
        .unwrap();
        let target = Arc::new(tb.build(storage_for_build.clone()));
        store_arc.store_layer(&target).unwrap();

        // Pre-condition: rebuilding `source`'s chain via a
        // persistent-backed `LayerStorage` (no redirect installed yet)
        // resolves `demo:X` to "from-source".
        let info_pre = store_arc.load_chain_from(source.id()).unwrap().unwrap();
        let pre_storage =
            eigenius_kernel::layer::LayerStorage::with_persistent(Arc::clone(&store_arc));
        let pre_head = eigenius_kernel::layer::build_chain(info_pre, pre_storage);
        let pre = pre_head
            .resolve(&iri("urn:eigenius:demo:X"))
            .expect("demo:X resolves before redirect");
        assert_eq!(
            pre.get(&iri("urn:eigenius:core:description"))
                .and_then(|v| v.as_str()),
            Some("from-source")
        );
        assert!(
            pre_head.redirect_target().is_none(),
            "pre-condition: no redirect installed yet"
        );

        // Install the redirect: source → target. Topology entry for
        // `source` stays in place (reclaim happens in a later phase).
        let source_handle = store_arc
            .load_topology()
            .unwrap()
            .get_layer(source.id())
            .unwrap()
            .clone();
        let entry = eigenius_kernel::layer::RedirectEntry {
            target: target.id().clone(),
            source_handle,
            preserve_history: false,
        };
        store_arc.put_redirect(&entry).unwrap();

        // Post-condition: a freshly-built chain sees the redirect.
        // `build_chain` reads `redirect_map` (which `with_persistent`
        // populated at construction time, so we need a *fresh*
        // `LayerStorage` to pick up the new redirect).
        let info_post = store_arc.load_chain_from(source.id()).unwrap().unwrap();
        let post_storage =
            eigenius_kernel::layer::LayerStorage::with_persistent(Arc::clone(&store_arc));
        let post_head = eigenius_kernel::layer::build_chain(info_post, post_storage);
        assert!(
            post_head.redirect_target().is_some(),
            "build_chain should populate redirect_target for redirect-source layers"
        );
        let post = post_head
            .resolve(&iri("urn:eigenius:demo:X"))
            .expect("demo:X resolves through the redirect");
        assert_eq!(
            post.get(&iri("urn:eigenius:core:description"))
                .and_then(|v| v.as_str()),
            Some("from-target"),
            "resolve must follow the redirect and return the target's value"
        );
    }

    /// Two layers with identical resources committed against different
    /// parents share a `ContentHash` but get distinct `PositionHash`es;
    /// `lookup_by_content_hash` returns both, and `delete_layer` prunes
    /// only the deleted entry. Mirrors the in-kernel memory-backend
    /// test of the same shape (`content_hash_index_dedup_and_cleanup`)
    /// against the persistent RocksDB index.
    #[test]
    fn content_hash_index_dedup_and_cleanup_rocksdb() {
        use eigenius_kernel::storage::PersistentBackend;
        let (store, _dir) = open_temp_store();

        let mut a = LayerBuilder::new("root_a", None);
        a.add_resource(make_resource(
            "urn:eigenius:core:root_a_marker",
            vec![("urn:eigenius:core:description", Value::String("a".into()))],
        ))
        .unwrap();
        let root_a = Arc::new(a.build(eigenius_kernel::layer::LayerStorage::in_memory()));
        PersistentBackend::store_layer(&store, &root_a).unwrap();

        let mut b = LayerBuilder::new("root_b", None);
        b.add_resource(make_resource(
            "urn:eigenius:core:root_b_marker",
            vec![("urn:eigenius:core:description", Value::String("b".into()))],
        ))
        .unwrap();
        let root_b = Arc::new(b.build(eigenius_kernel::layer::LayerStorage::in_memory()));
        PersistentBackend::store_layer(&store, &root_b).unwrap();

        let build_child = |parent: Arc<Layer>| -> Layer {
            let mut cb = LayerBuilder::new("child", Some(parent));
            cb.add_resource(make_resource(
                "urn:eigenius:demo:shared",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("shared".into()),
                )],
            ))
            .unwrap();
            cb.build(eigenius_kernel::layer::LayerStorage::in_memory())
        };
        let child_a = build_child(Arc::clone(&root_a));
        let child_b = build_child(Arc::clone(&root_b));
        assert_eq!(child_a.content_hash(), child_b.content_hash());
        assert_ne!(child_a.id(), child_b.id());

        PersistentBackend::store_layer(&store, &child_a).unwrap();
        PersistentBackend::store_layer(&store, &child_b).unwrap();

        let mut hits =
            PersistentBackend::lookup_by_content_hash(&store, child_a.content_hash()).unwrap();
        hits.sort();
        let mut expected = vec![child_a.id().clone(), child_b.id().clone()];
        expected.sort();
        assert_eq!(hits, expected);

        PersistentBackend::delete_layer(&store, child_a.id()).unwrap();
        let remaining =
            PersistentBackend::lookup_by_content_hash(&store, child_a.content_hash()).unwrap();
        assert_eq!(remaining, vec![child_b.id().clone()]);

        PersistentBackend::delete_layer(&store, child_b.id()).unwrap();
        let empty =
            PersistentBackend::lookup_by_content_hash(&store, child_a.content_hash()).unwrap();
        assert!(empty.is_empty());
    }

    /// D43 M1 — `RocksStore::open` declares the three D43 column
    /// families (`cf_text`, `cf_vec`, `cf_embed_cache`) so that
    /// subsequent milestones (M2 storage substrate onward) can route
    /// their key prefixes to dedicated compaction streams.
    ///
    /// Verifies: (a) a freshly-opened store exposes a handle for each
    /// D43 CF, (b) writes to each CF persist across reopen, and (c)
    /// data in one CF does not leak into another (each CF is its own
    /// keyspace).
    #[test]
    fn d43_column_families_open_persist_and_isolate() {
        let dir = TempDir::new().unwrap();

        // Round 1: open, verify CFs exist, write a sentinel value to each.
        {
            let store = RocksStore::open(dir.path()).unwrap();
            for cf_name in D43_COLUMN_FAMILIES {
                let cf = store
                    .db
                    .cf_handle(cf_name)
                    .unwrap_or_else(|| panic!("CF {cf_name} should exist after open"));
                let key = format!("sentinel:{cf_name}");
                let val = format!("value-in-{cf_name}");
                store
                    .db
                    .put_cf(&cf, key.as_bytes(), val.as_bytes())
                    .unwrap();
            }
        }

        // Round 2: reopen, verify each sentinel persisted and CFs are isolated.
        {
            let store = RocksStore::open(dir.path()).unwrap();
            for cf_name in D43_COLUMN_FAMILIES {
                let cf = store
                    .db
                    .cf_handle(cf_name)
                    .unwrap_or_else(|| panic!("CF {cf_name} should persist across reopen"));
                let key = format!("sentinel:{cf_name}");
                let expected = format!("value-in-{cf_name}");
                let got = store
                    .db
                    .get_cf(&cf, key.as_bytes())
                    .unwrap()
                    .expect("sentinel value should persist");
                assert_eq!(got, expected.as_bytes(), "value in {cf_name} after reopen");

                // Other CFs must not see this sentinel key — CFs are
                // independent keyspaces.
                for other in D43_COLUMN_FAMILIES.iter().filter(|n| *n != cf_name) {
                    let other_cf = store.db.cf_handle(other).unwrap();
                    let leak = store.db.get_cf(&other_cf, key.as_bytes()).unwrap();
                    assert!(
                        leak.is_none(),
                        "CF {other} should not see sentinel key from {cf_name}"
                    );
                }
            }

            // The default CF should also not see the D43 sentinels —
            // existing key prefixes continue to target the default CF
            // unaffected by the new keyspaces.
            for cf_name in D43_COLUMN_FAMILIES {
                let key = format!("sentinel:{cf_name}");
                let default_leak = store.db.get(key.as_bytes()).unwrap();
                assert!(
                    default_leak.is_none(),
                    "default CF should not see D43 sentinel for {cf_name}"
                );
            }
        }
    }

    /// D43 M2.7 — `RocksStore::delete_layer` participates in the
    /// atomic-with-layer-drop envelope: the same `WriteBatch` that
    /// removes the layer's resource rows, topology entry, and
    /// triple-index entries also removes the layer's text-index
    /// postings (`text_term:`, `text_docs:`, `text_stats:`,
    /// `text_terms_layer:`) and vector-index segments (`vec_seg:`,
    /// `vec_layer:`). Verifies cleanup across both new CFs in one
    /// commit per D43 §2.5.
    #[test]
    fn delete_layer_drops_text_and_vector_indexes_atomically() {
        use eigenius_kernel::layer::{
            LayerBuilder, MemoryBloomCache, MemoryResourceBackend, MemoryResourceCache,
            NoRedirects, TextDoc, VectorDoc,
        };
        use eigenius_kernel::ontology::iri::Iri;
        use eigenius_kernel::storage::PersistentBackend;

        let (store, _dir) = {
            let dir = TempDir::new().unwrap();
            let store = RocksStore::open(dir.path()).unwrap();
            (Arc::new(store), dir)
        };

        // Use a minimal layer + the store's index Arcs directly.
        let storage = eigenius_kernel::layer::LayerStorage {
            cache: Arc::new(MemoryResourceCache::new()),
            backend: Arc::new(MemoryResourceBackend::new()),
            bloom_cache: Arc::new(MemoryBloomCache::cache_only()),
            triple_index: store.triple_index_arc(),
            text_index: store.text_index_arc(),
            vector_index: store.vector_index_arc(),
            value_index: store.value_index_arc(),
            redirect_map: Arc::new(NoRedirects),
            persistent_backend: None,
            pending: eigenius_kernel::layer::PendingStage::default(),
        };
        let builder = LayerBuilder::new("test", None);
        let layer = builder.build(storage);
        let layer_id = layer.id().clone();
        store.store_layer(&layer).unwrap();

        // Populate text + vector indexes against this layer.
        let index_iri = Iri::parse("urn:eigenius:test:idx").unwrap();
        let subject = Iri::parse("urn:eigenius:test:s").unwrap();
        let model_iri = Iri::parse("urn:eigenius:test:embed").unwrap();
        let tokens = vec!["alpha".to_string(), "beta".to_string()];
        let vec_data = [1.0f32, 0.5, 0.25];

        store
            .text_index_arc()
            .extend_layer(
                &index_iri,
                &layer_id,
                "en-stem-v1",
                &[TextDoc {
                    subject: &subject,
                    tokens: &tokens,
                }],
            )
            .unwrap();
        store
            .vector_index_arc()
            .extend_layer(
                &index_iri,
                &layer_id,
                &model_iri,
                3,
                "cosine",
                &[VectorDoc {
                    subject: &subject,
                    vector: &vec_data,
                }],
                None,
            )
            .unwrap();

        // Sanity: data is present.
        assert!(store
            .text_index_arc()
            .get_layer_stats(&index_iri, &layer_id)
            .unwrap()
            .is_some());
        assert!(store
            .vector_index_arc()
            .get_segment(&index_iri, &layer_id)
            .unwrap()
            .is_some());

        // delete_layer fires the atomic batch covering all three indexes.
        store.delete_layer(&layer_id).unwrap();

        // Text-index entries gone.
        assert!(store
            .text_index_arc()
            .get_layer_stats(&index_iri, &layer_id)
            .unwrap()
            .is_none());
        assert!(store
            .text_index_arc()
            .get_layer_docs(&index_iri, &layer_id)
            .unwrap()
            .is_none());
        assert!(store
            .text_index_arc()
            .get_layer_analyzer(&index_iri, &layer_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .text_index_arc()
                .scan_term(&index_iri, "alpha")
                .count(),
            0,
            "text postings dropped"
        );

        // Vector-index segment gone.
        assert!(store
            .vector_index_arc()
            .get_segment(&index_iri, &layer_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store.vector_index_arc().scan_index(&index_iri).count(),
            0,
            "vector segments dropped"
        );
    }

    /// D65 — `RocksValueIndex` round-trips exact entries through the shared
    /// `Arc<rocksdb::DB>`: `extend_layer` across two layers, exact lookup
    /// returns every subject + its defining layer, and `delete_layer` drops
    /// only the named layer's contributions inside the atomic batch.
    #[test]
    fn value_index_extend_lookup_and_delete_layer() {
        use eigenius_kernel::layer::{LayerId, ValueEntry};
        use eigenius_kernel::ontology::iri::Iri;
        use eigenius_kernel::storage::PersistentBackend;

        let dir = TempDir::new().unwrap();
        let store = RocksStore::open(dir.path()).unwrap();
        let vi = store.value_index_arc();

        let index = Iri::parse("urn:eigenius:lexicon:form_index").unwrap();
        let (l1, l2) = (LayerId([1; 32]), LayerId([2; 32]));
        let (e1, e2, e3) = (
            Iri::parse("urn:eigenius:wn:e_cellline").unwrap(),
            Iri::parse("urn:eigenius:wn:e_cellline2").unwrap(),
            Iri::parse("urn:eigenius:umls:e_cellline").unwrap(),
        );

        vi.extend_layer(
            &l1,
            &[
                ValueEntry {
                    index: &index,
                    key: "cell line",
                    subject: &e1,
                },
                ValueEntry {
                    index: &index,
                    key: "cell line",
                    subject: &e2,
                },
                ValueEntry {
                    index: &index,
                    key: "gene",
                    subject: &e1,
                },
            ],
        )
        .unwrap();
        vi.extend_layer(
            &l2,
            &[ValueEntry {
                index: &index,
                key: "cell line",
                subject: &e3,
            }],
        )
        .unwrap();

        // Exact lookup returns all subjects + their defining layers.
        let mut hits: Vec<(Iri, LayerId)> =
            vi.lookup(&index, "cell line").map(Result::unwrap).collect();
        hits.sort();
        let mut expected = vec![
            (e1.clone(), l1.clone()),
            (e2.clone(), l1.clone()),
            (e3.clone(), l2.clone()),
        ];
        expected.sort();
        assert_eq!(hits, expected);

        // Keys are exact, whole-string — no tokenisation, no implicit folding.
        assert_eq!(vi.lookup(&index, "cell").count(), 0);
        assert_eq!(vi.lookup(&index, "Cell Line").count(), 0);
        assert_eq!(vi.lookup(&index, "gene").count(), 1);

        // `delete_layer` drops only l1's contributions via the atomic batch.
        PersistentBackend::delete_layer(&store, &l1).unwrap();
        let after: Vec<(Iri, LayerId)> =
            vi.lookup(&index, "cell line").map(Result::unwrap).collect();
        assert_eq!(after, vec![(e3, l2)]);
        assert_eq!(
            vi.lookup(&index, "gene").count(),
            0,
            "l1's gene entry dropped"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persistence_across_reopen() {
        let dir = TempDir::new().unwrap();

        // Write data
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let mut builder = LayerBuilder::new("persisted", None);
            builder
                .add_resource(make_resource(
                    "urn:eigenius:core:persistent",
                    vec![(
                        "urn:eigenius:core:description",
                        Value::String("survives restart".into()),
                    )],
                ))
                .unwrap();
            let layer = builder.build(eigenius_kernel::layer::LayerStorage::in_memory());
            let id = layer.id().clone();
            eigenius_kernel::storage::PersistentBackend::store_layer(&store, &layer).unwrap();
            // Phase 14g: track the head via `branch:main` instead of
            // the removed `set_head`.
            eigenius_kernel::storage::PersistentBackend::put_branch(&store, "main", &id).unwrap();
        }

        // Reopen and verify
        {
            let store = Arc::new(RocksStore::open(dir.path()).unwrap());
            let head =
                eigenius_kernel::storage::PersistentBackend::get_branch(store.as_ref(), "main")
                    .unwrap()
                    .expect("branch:main survives reopen");

            // Resolve the layer via the sync `PersistentBackend` surface;
            // the old async `load_layer` is gone. We rebuild the chain
            // (one-element here) and inspect the head through it.
            let info =
                eigenius_kernel::storage::PersistentBackend::load_chain_from(store.as_ref(), &head)
                    .unwrap()
                    .expect("chain present");
            let layer_storage =
                eigenius_kernel::layer::LayerStorage::with_persistent(Arc::clone(&store) as _);
            let head_layer = eigenius_kernel::layer::build_chain(info, layer_storage);
            assert_eq!(head_layer.name(), "persisted");
            assert!(head_layer
                .resolve(&iri("urn:eigenius:core:persistent"))
                .is_some());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn chain_reconstruction() {
        let (store, _dir) = open_temp_store();

        // Build and store root layer
        let mut root_builder = LayerBuilder::new("core", None);
        root_builder
            .add_resource(make_resource("urn:eigenius:core:Class", vec![]))
            .unwrap();
        let root = Arc::new(root_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
        eigenius_kernel::storage::PersistentBackend::store_layer(&store, &root).unwrap();

        // Build and store child layer
        let mut child_builder = LayerBuilder::new("domain", Some(Arc::clone(&root)));
        child_builder
            .add_resource(make_resource(
                "urn:eigenius:example:Dog",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("A dog".into()),
                )],
            ))
            .unwrap();
        let child = child_builder.build(eigenius_kernel::layer::LayerStorage::in_memory());
        let child_id = child.id().clone();
        eigenius_kernel::storage::PersistentBackend::store_layer(&store, &child).unwrap();
        // Phase 14g: track head via `branch:main`; load chain via
        // `load_chain_from(branch_head)` rather than the removed
        // no-arg `load_chain()`.
        eigenius_kernel::storage::PersistentBackend::put_branch(&store, "main", &child_id).unwrap();

        let main_head = eigenius_kernel::storage::PersistentBackend::get_branch(&store, "main")
            .unwrap()
            .expect("branch:main present");
        let info = eigenius_kernel::storage::PersistentBackend::load_chain_from(&store, &main_head)
            .unwrap()
            .expect("chain present");
        let storage = eigenius_kernel::layer::LayerStorage::in_memory();
        // Pre-warm the caches from the persistent store so resolve hits succeed.
        for handle in &info.handles {
            if let Some(iris) = info.defined_iris_per_layer.get(&handle.id) {
                for iri_h in iris {
                    if let Some(r) = ResourceBackend::load_resource(&store, &handle.id, iri_h) {
                        storage.cache.put(
                            eigenius_kernel::layer::ResourceKey::new(
                                handle.id.clone(),
                                iri_h.clone(),
                            ),
                            Arc::new(r),
                            eigenius_kernel::layer::CacheTier::Active,
                        );
                    }
                }
            }
            if let Ok(Some(bloom)) =
                eigenius_kernel::storage::PersistentBackend::load_bloom(&store, &handle.id)
            {
                storage.bloom_cache.put(handle.id.clone(), Arc::new(bloom));
            }
        }
        let head = eigenius_kernel::layer::build_chain(info, storage);
        assert!(!head.is_root());
        // Should resolve resources from both layers
        assert!(head.resolve(&iri("urn:eigenius:core:Class")).is_some());
        assert!(head.resolve(&iri("urn:eigenius:example:Dog")).is_some());
    }

    // Replaced by `cbor_coverage_tests::core_ontology_field_level_equality`,
    // which checks every property survives the round-trip rather than just
    // resource count.

    #[tokio::test]
    async fn trace_store_round_trip() {
        let (store, _dir) = open_temp_store();

        let key = [42u8; 32];
        assert!(store.get_component_trace(&key).is_none());

        let trace = ComponentTrace {
            component: "urn:eigenius:program:components:CompleteText".to_string(),
            input_hash: key,
            argument_hash: None,
            output: make_resource(
                "urn:eigenius:test:output",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("LLM output".into()),
                )],
            ),
            cached: false,
            metrics: Some(ComponentMetrics {
                provider: "anthropic".to_string(),
                model: "claude-sonnet".to_string(),
                prompt_tokens: 100,
                completion_tokens: 50,
                latency_ms: 500,
            }),
        };

        store.put_component_trace(key, trace);
        let loaded = store.get_component_trace(&key).unwrap();

        assert_eq!(
            loaded.component,
            "urn:eigenius:program:components:CompleteText"
        );
        assert_eq!(loaded.input_hash, key);
        assert!(loaded.metrics.is_some());
        let m = loaded.metrics.unwrap();
        assert_eq!(m.provider, "anthropic");
        assert_eq!(m.prompt_tokens, 100);
        assert_eq!(m.completion_tokens, 50);
        assert_eq!(m.latency_ms, 500);
        assert_eq!(
            loaded
                .output
                .get(&iri("urn:eigenius:core:description"))
                .unwrap()
                .as_str(),
            Some("LLM output")
        );
    }

    // --- Phase 14a-ii: topology storage tests ---
    //
    // Wrapped in a sub-module so the `use PersistentBackend` import doesn't
    // leak into the parent test module — both `LayerStore` and
    // `PersistentBackend` define `store_layer`, and bringing both into the
    // same scope creates method-resolution ambiguity on the older async tests.
    mod topology_tests {
        use super::*;
        use eigenius_kernel::storage::PersistentBackend as PB;

        #[test]
        fn topology_round_trip_via_store_layer() {
            // PB::store_layer must populate `topo:<id>` so load_topology returns
            // the layer's handle.
            let (store, _dir) = open_temp_store();

            let mut builder = LayerBuilder::new("root", None);
            builder
                .add_resource(make_resource("urn:eigenius:core:A", vec![]))
                .unwrap();
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let id = layer.id().clone();

            PB::store_layer(&store, &layer).unwrap();

            let topology = PB::load_topology(&store).unwrap();
            assert_eq!(topology.layer_count(), 1);
            let handle = topology.get_layer(&id).expect("handle present");
            assert_eq!(handle.name, "root");
            assert!(handle.is_root());
            assert_eq!(handle.resource_count, 1);
            // created_at was populated via now_millis() on commit (non-sentinel).
            assert!(handle.created_at > 0);
        }

        #[test]
        fn topology_walk_chain_after_multiple_commits() {
            let (store, _dir) = open_temp_store();

            let mut root_builder = LayerBuilder::new("root", None);
            root_builder
                .add_resource(make_resource("urn:eigenius:core:A", vec![]))
                .unwrap();
            let root =
                Arc::new(root_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let root_id = root.id().clone();

            let mut child_builder = LayerBuilder::new("child", Some(Arc::clone(&root)));
            child_builder
                .add_resource(make_resource("urn:eigenius:example:B", vec![]))
                .unwrap();
            let child =
                Arc::new(child_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let child_id = child.id().clone();

            PB::store_layer(&store, &root).unwrap();
            PB::store_layer(&store, &child).unwrap();

            let topology = PB::load_topology(&store).unwrap();
            assert_eq!(topology.layer_count(), 2);

            // Walk from child should yield [child, root].
            let walked: Vec<&str> = topology
                .walk_chain(&child_id)
                .map(|h| h.name.as_str())
                .collect();
            assert_eq!(walked, vec!["child", "root"]);

            // Walk from root yields just [root].
            let walked_root: Vec<&str> = topology
                .walk_chain(&root_id)
                .map(|h| h.name.as_str())
                .collect();
            assert_eq!(walked_root, vec!["root"]);
        }

        #[test]
        fn topology_persists_across_reopen() {
            let dir = TempDir::new().unwrap();
            let layer_id;

            // Write via PersistentBackend; close.
            {
                let store = RocksStore::open(dir.path()).unwrap();
                let mut builder = LayerBuilder::new("persisted", None);
                builder
                    .add_resource(make_resource("urn:eigenius:core:X", vec![]))
                    .unwrap();
                let layer =
                    Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
                layer_id = layer.id().clone();
                PB::store_layer(&store, &layer).unwrap();
            }

            // Reopen; topology entry must be there without re-storing.
            {
                let store = RocksStore::open(dir.path()).unwrap();
                let topology = PB::load_topology(&store).unwrap();
                assert_eq!(topology.layer_count(), 1);
                assert!(topology.get_layer(&layer_id).is_some());
            }
        }

        #[test]
        fn topology_load_from_empty_db_is_empty() {
            let (store, _dir) = open_temp_store();
            let topology = PB::load_topology(&store).unwrap();
            assert_eq!(topology.layer_count(), 0);
        }
    } // mod topology_tests

    // --- CBOR-coverage tests for the persistent backend ---
    //
    // Wrapped in a sub-module so the `use PersistentBackend` import doesn't
    // collide with the older `LayerStore::store_layer` async tests above.
    mod cbor_coverage_tests {
        use super::*;
        use eigenius_kernel::storage::PersistentBackend as PB;
        use eigenius_kernel::storage::{BatchOp, ChainInfo};

        /// All wire-typed `Value` variants survive `store_layer` →
        /// `load_resource` through CBOR with structural equality. Variants
        /// excluded here (`ResourceRef`, `Json`) are in-memory convenience
        /// shapes that normalize to the wire-typed form on round-trip; their
        /// behavior is pinned by `value_variants_round_trip_normalizations`
        /// below.
        #[test]
        fn value_variants_round_trip() {
            let (store, _dir) = open_temp_store();

            let mut inner = Resource::new_embedded();
            inner.set(
                iri("urn:eigenius:test:city"),
                Value::String("Berlin".into()),
            );

            let mut r = Resource::new(iri("urn:eigenius:test:variants"));
            r.set(iri("urn:eigenius:test:s"), Value::String("hello".into()));
            r.set(iri("urn:eigenius:test:i"), Value::Integer(-12345));
            r.set(iri("urn:eigenius:test:f"), Value::Float(1.234567890123));
            r.set(iri("urn:eigenius:test:b"), Value::Boolean(true));
            r.set(
                iri("urn:eigenius:test:emb"),
                Value::Embedded(Box::new(inner)),
            );
            r.set(
                iri("urn:eigenius:test:arr"),
                Value::Array(vec![
                    Value::Integer(1),
                    Value::String("two".into()),
                    Value::Boolean(false),
                ]),
            );
            r.set(
                iri("urn:eigenius:test:nested_arr"),
                Value::Array(vec![Value::Array(vec![Value::Integer(42)])]),
            );

            let original = r.clone();
            let mut builder = LayerBuilder::new("variants", None);
            builder.add_resource(r).unwrap();
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let layer_id = layer.id().clone();

            PB::store_layer(&store, &layer).unwrap();

            // Read directly via the ResourceBackend surface (not load_layer,
            // which warms a cache — we want the on-disk CBOR decode path).
            let loaded = ResourceBackend::load_resource(
                &store,
                &layer_id,
                &iri("urn:eigenius:test:variants"),
            )
            .expect("resource present");

            // Resource derives PartialEq: full structural equality.
            assert_eq!(loaded, original);
        }

        /// Pins the intentional CBOR normalizations: `ResourceRef` and `Json`
        /// are in-memory convenience variants that the wire layer collapses
        /// into wire-typed forms (`String` / `Integer` / `Bool` / etc.). The
        /// String-vs-ResourceRef discrimination happens at validation time
        /// based on the property's declared `data_type`. If this test starts
        /// failing, the CBOR layer has changed its typing contract and that
        /// needs a deliberate decision (and content-addressing implications),
        /// not a silent drift.
        #[test]
        fn value_variants_round_trip_normalizations() {
            let (store, _dir) = open_temp_store();

            let mut r = Resource::new(iri("urn:eigenius:test:lossy"));
            r.set(
                iri("urn:eigenius:test:ref"),
                Value::ResourceRef(iri("urn:eigenius:test:other")),
            );
            r.set(
                iri("urn:eigenius:test:json_str"),
                Value::Json(serde_json::Value::String("hi".into())),
            );
            r.set(
                iri("urn:eigenius:test:json_num"),
                Value::Json(serde_json::Value::Number(7i64.into())),
            );

            let mut builder = LayerBuilder::new("lossy", None);
            builder.add_resource(r).unwrap();
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let layer_id = layer.id().clone();
            PB::store_layer(&store, &layer).unwrap();

            let loaded =
                ResourceBackend::load_resource(&store, &layer_id, &iri("urn:eigenius:test:lossy"))
                    .expect("resource present");

            // ResourceRef → String (same wire bytes; discrimination at
            // validation time using the property's data_type).
            assert_eq!(
                loaded.get(&iri("urn:eigenius:test:ref")),
                Some(&Value::String("urn:eigenius:test:other".into()))
            );
            // Json(String) → String, Json(Number) → Integer.
            assert_eq!(
                loaded.get(&iri("urn:eigenius:test:json_str")),
                Some(&Value::String("hi".into()))
            );
            assert_eq!(
                loaded.get(&iri("urn:eigenius:test:json_num")),
                Some(&Value::Integer(7))
            );
        }

        /// Phase 18c.5 / D26 §5.5: `Value::Json(Object)` and
        /// `Value::Json(Array)` round-trip as `Value::Json` (tagged via
        /// `EIGENIUS_JSON_TAG` on the wire). Scalars still flatten —
        /// see `value_variants_round_trip_normalizations` above — but
        /// non-scalar JSON shapes can't collapse safely (object keys
        /// aren't IRIs; arrays would need empty-array discrimination)
        /// so they preserve the `Value::Json` variant explicitly. This
        /// test pins that asymmetric contract; if it fails, the codec
        /// drifted away from "tag only objects/arrays."
        #[test]
        fn value_json_object_and_array_round_trip_as_json() {
            let (store, _dir) = open_temp_store();

            let mut obj = serde_json::Map::new();
            obj.insert(
                "host_kernel".into(),
                serde_json::Value::String("linux-6.6".into()),
            );
            obj.insert("fma_enabled".into(), serde_json::Value::Bool(true));
            let object_val = serde_json::Value::Object(obj);
            let array_val =
                serde_json::Value::Array(vec![serde_json::json!(1), serde_json::json!("two")]);

            let mut r = Resource::new(iri("urn:eigenius:test:json_shapes"));
            r.set(
                iri("urn:eigenius:test:metadata"),
                Value::Json(object_val.clone()),
            );
            r.set(
                iri("urn:eigenius:test:tensor_shape"),
                Value::Json(array_val.clone()),
            );

            let mut builder = LayerBuilder::new("json-shapes", None);
            builder.add_resource(r).unwrap();
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let layer_id = layer.id().clone();
            PB::store_layer(&store, &layer).unwrap();

            let loaded = ResourceBackend::load_resource(
                &store,
                &layer_id,
                &iri("urn:eigenius:test:json_shapes"),
            )
            .expect("resource present");

            assert_eq!(
                loaded.get(&iri("urn:eigenius:test:metadata")),
                Some(&Value::Json(object_val))
            );
            assert_eq!(
                loaded.get(&iri("urn:eigenius:test:tensor_shape")),
                Some(&Value::Json(array_val))
            );
        }

        /// Every resource in the core ontology must round-trip with full
        /// structural equality, not just preserved count. Catches any
        /// encoder/decoder regression that drops or mangles fields.
        #[test]
        fn core_ontology_field_level_equality() {
            let (store, _dir) = open_temp_store();
            let core_json = include_str!("../../../ontologies/core/core-ontology.json");
            let resources = eigon_json::parse_document(core_json).unwrap();

            let mut originals: std::collections::BTreeMap<Iri, Resource> =
                std::collections::BTreeMap::new();
            for r in &resources {
                originals.insert(r.id().expect("core resource has @id").clone(), r.clone());
            }

            let mut builder = LayerBuilder::new("core", None);
            for r in resources {
                builder.add_resource(r).unwrap();
            }
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let id = layer.id().clone();

            PB::store_layer(&store, &layer).unwrap();

            // Read each one back through the backend and compare.
            for (iri, original) in &originals {
                let loaded = ResourceBackend::load_resource(&store, &id, iri)
                    .unwrap_or_else(|| panic!("missing core resource {iri}"));
                assert_eq!(&loaded, original, "round-trip mismatch for {iri}");
            }

            // And nothing extra appeared.
            let loaded_iris = ResourceBackend::list_layer_iris(&store, &id).unwrap();
            assert_eq!(
                loaded_iris,
                originals
                    .keys()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>()
            );
        }

        /// `build_chain` against the live `RocksStore` backend with a fresh
        /// cache: every `resolve` must hit the backend's CBOR-decode path,
        /// since the cache starts empty. This is the path that production
        /// uses but no existing test exercises end-to-end.
        #[test]
        fn chain_resolve_with_cold_cache() {
            let (store, _dir) = open_temp_store();
            let store_arc: Arc<RocksStore> = Arc::new(store);

            // Build root with one resource.
            let mut root_builder = LayerBuilder::new("root", None);
            root_builder
                .add_resource(make_resource(
                    "urn:eigenius:core:Class",
                    vec![(
                        "urn:eigenius:core:description",
                        Value::String("class".into()),
                    )],
                ))
                .unwrap();
            let root =
                Arc::new(root_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));

            // Build child with another resource.
            let mut child_builder = LayerBuilder::new("domain", Some(Arc::clone(&root)));
            child_builder
                .add_resource(make_resource(
                    "urn:eigenius:example:Dog",
                    vec![("urn:eigenius:core:description", Value::String("dog".into()))],
                ))
                .unwrap();
            let child =
                Arc::new(child_builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let child_id = child.id().clone();

            PB::store_layer(&*store_arc, &root).unwrap();
            PB::store_layer(&*store_arc, &child).unwrap();
            // Phase 14g: head pointer via `branch:main`.
            PB::put_branch(&*store_arc, "main", &child_id).unwrap();

            // Drop the original layer Arcs so their throwaway caches go away.
            drop(root);
            drop(child);

            // Reconstruct the chain pointing at the live RocksStore — fresh
            // cache, real backend.
            let main_head = PB::get_branch(&*store_arc, "main").unwrap().unwrap();
            let info = PB::load_chain_from(&*store_arc, &main_head)
                .unwrap()
                .expect("chain present");
            // Storage backed by the live RocksStore — fresh resource cache
            // (cold), bloom cache backed by the same store so cold-resolve
            // exercises both backend probes.
            let pb_arc: Arc<dyn eigenius_kernel::storage::PersistentBackend> =
                Arc::clone(&store_arc) as _;
            let storage = eigenius_kernel::layer::LayerStorage::with_persistent(pb_arc);
            let head = eigenius_kernel::layer::build_chain(info, storage.clone());

            // Cache is empty: this resolve must traverse the parent chain and
            // decode CBOR from RocksDB.
            let class = head
                .resolve(&iri("urn:eigenius:core:Class"))
                .expect("Class resolves through cold cache");
            assert_eq!(
                class
                    .get(&iri("urn:eigenius:core:description"))
                    .and_then(|v| v.as_str()),
                Some("class")
            );

            let dog = head
                .resolve(&iri("urn:eigenius:example:Dog"))
                .expect("Dog resolves through cold cache");
            assert_eq!(
                dog.get(&iri("urn:eigenius:core:description"))
                    .and_then(|v| v.as_str()),
                Some("dog")
            );

            // Cache should now have populated entries (proving misses fell
            // through to the backend rather than silently failing).
            assert!(storage.cache.stats().entries >= 2);
        }

        /// `meta:` key/value surface — `put_meta`/`get_meta`/`delete_meta`/
        /// `list_meta_prefix`. This is the substrate D21 task storage runs on,
        /// previously untested at the `PersistentBackend` level.
        #[test]
        fn meta_kv_round_trip() {
            let (store, _dir) = open_temp_store();

            assert!(PB::get_meta(&store, "absent").unwrap().is_none());

            PB::put_meta(&store, "session:abc", b"value-abc").unwrap();
            PB::put_meta(&store, "session:def", b"value-def").unwrap();
            PB::put_meta(&store, "other:xyz", b"value-xyz").unwrap();

            assert_eq!(
                PB::get_meta(&store, "session:abc").unwrap().as_deref(),
                Some(b"value-abc".as_ref())
            );
            assert_eq!(
                PB::get_meta(&store, "session:def").unwrap().as_deref(),
                Some(b"value-def".as_ref())
            );

            // list_meta_prefix scopes correctly.
            let session_keys = PB::list_meta_prefix(&store, "session:").unwrap();
            let mut session_sorted = session_keys.clone();
            session_sorted.sort();
            assert_eq!(session_sorted, vec!["session:abc", "session:def"]);

            // delete_meta on present key removes it.
            PB::delete_meta(&store, "session:abc").unwrap();
            assert!(PB::get_meta(&store, "session:abc").unwrap().is_none());

            // delete_meta on absent key is a no-op (per trait contract).
            PB::delete_meta(&store, "session:never_existed").unwrap();

            // Other prefix unaffected.
            assert_eq!(
                PB::get_meta(&store, "other:xyz").unwrap().as_deref(),
                Some(b"value-xyz".as_ref())
            );
        }

        /// `write_batch` must apply every operation. Per D21 §8 step
        /// atomicity, this is the single-commit primitive task steps use;
        /// correctness here is structural.
        #[test]
        fn write_batch_applies_all_ops() {
            let (store, _dir) = open_temp_store();

            // Pre-populate one key so we can verify a delete inside the batch.
            PB::put_meta(&store, "to_delete", b"old").unwrap();

            let ops = vec![
                BatchOp::PutMeta {
                    key: "k1".into(),
                    value: b"v1".to_vec(),
                },
                BatchOp::PutMeta {
                    key: "k2".into(),
                    value: b"v2".to_vec(),
                },
                BatchOp::DeleteMeta {
                    key: "to_delete".into(),
                },
                BatchOp::PutMeta {
                    key: "k3".into(),
                    value: b"v3".to_vec(),
                },
            ];
            PB::write_batch(&store, &ops).unwrap();

            assert_eq!(
                PB::get_meta(&store, "k1").unwrap().as_deref(),
                Some(b"v1".as_ref())
            );
            assert_eq!(
                PB::get_meta(&store, "k2").unwrap().as_deref(),
                Some(b"v2".as_ref())
            );
            assert_eq!(
                PB::get_meta(&store, "k3").unwrap().as_deref(),
                Some(b"v3".as_ref())
            );
            assert!(PB::get_meta(&store, "to_delete").unwrap().is_none());
        }

        /// `load_chain_from(head_id)` walks from an arbitrary layer, not
        /// just the persisted head. Critical for `at_layer` reads and task
        /// resume that pin specific heads. Multi-head test: two children
        /// off one parent must each rebuild the correct chain.
        #[test]
        fn load_chain_from_specific_head() {
            let (store, _dir) = open_temp_store();

            let mut root_b = LayerBuilder::new("root", None);
            root_b
                .add_resource(make_resource("urn:eigenius:core:R", vec![]))
                .unwrap();
            let root = Arc::new(root_b.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let root_id = root.id().clone();

            // Two distinct children off the same root — distinct because
            // they define different IRIs.
            let mut a_b = LayerBuilder::new("child_a", Some(Arc::clone(&root)));
            a_b.add_resource(make_resource("urn:eigenius:example:A", vec![]))
                .unwrap();
            let child_a = Arc::new(a_b.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let a_id = child_a.id().clone();

            let mut b_b = LayerBuilder::new("child_b", Some(Arc::clone(&root)));
            b_b.add_resource(make_resource("urn:eigenius:example:B", vec![]))
                .unwrap();
            let child_b = Arc::new(b_b.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let b_id = child_b.id().clone();

            PB::store_layer(&store, &root).unwrap();
            PB::store_layer(&store, &child_a).unwrap();
            PB::store_layer(&store, &child_b).unwrap();
            // Note: no `set_head` — load_chain_from must not depend on it.

            let info_a: ChainInfo = PB::load_chain_from(&store, &a_id)
                .unwrap()
                .expect("chain for a");
            assert_eq!(info_a.head, a_id);
            let names_a: Vec<&str> = info_a.handles.iter().map(|h| h.name.as_str()).collect();
            assert_eq!(names_a, vec!["root", "child_a"]);
            assert!(info_a.defined_iris_per_layer.contains_key(&root_id));
            assert!(info_a.defined_iris_per_layer.contains_key(&a_id));

            let info_b: ChainInfo = PB::load_chain_from(&store, &b_id)
                .unwrap()
                .expect("chain for b");
            assert_eq!(info_b.head, b_id);
            let names_b: Vec<&str> = info_b.handles.iter().map(|h| h.name.as_str()).collect();
            assert_eq!(names_b, vec!["root", "child_b"]);
            assert!(info_b.defined_iris_per_layer.contains_key(&b_id));

            // Asking for the root alone yields a one-element chain.
            let info_root: ChainInfo = PB::load_chain_from(&store, &root_id)
                .unwrap()
                .expect("chain for root");
            assert_eq!(info_root.head, root_id);
            let names_root: Vec<&str> = info_root.handles.iter().map(|h| h.name.as_str()).collect();
            assert_eq!(names_root, vec!["root"]);
        }

        /// Phase 14b: `store_layer` writes a `bloom:<id>` entry and
        /// `load_bloom` reads it back. Round-trips through CBOR via
        /// `ciborium`. Verified by reconstructing the same bloom from the
        /// original IRI set and asserting structural equality, plus
        /// confirming `might_contain` agrees on every inserted IRI.
        #[test]
        fn bloom_round_trip_via_store_layer() {
            use eigenius_kernel::layer::BloomFilter;

            let (store, _dir) = open_temp_store();

            let mut builder = LayerBuilder::new("bloom_layer", None);
            for i in 0..200 {
                builder
                    .add_resource(make_resource(&format!("urn:eigenius:test:r{i}"), vec![]))
                    .unwrap();
            }
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let id = layer.id().clone();
            let original_iris = layer.defined_iris().clone();

            PB::store_layer(&store, &layer).unwrap();

            let loaded = PB::load_bloom(&store, &id).unwrap().expect("bloom present");
            let expected = BloomFilter::for_iris(&original_iris);
            assert_eq!(
                loaded, expected,
                "bloom must survive CBOR round-trip intact"
            );
            for iri_h in &original_iris {
                assert!(loaded.might_contain(iri_h));
            }
        }

        /// Bloom + topology + content + chain must all be visible after
        /// `store_layer`. This validates the D23 §6.3 atomic-commit
        /// contract — the new `WriteBatch` shape applies them as one
        /// commit; nothing should land partially.
        #[test]
        fn store_layer_writes_all_keys_atomically() {
            let (store, _dir) = open_temp_store();

            let mut builder = LayerBuilder::new("atomic", None);
            builder
                .add_resource(make_resource("urn:eigenius:test:a", vec![]))
                .unwrap();
            let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
            let id = layer.id().clone();

            PB::store_layer(&store, &layer).unwrap();

            // Topology entry present.
            let topology = PB::load_topology(&store).unwrap();
            assert!(topology.get_layer(&id).is_some());
            // Bloom present.
            assert!(PB::load_bloom(&store, &id).unwrap().is_some());
            // Resource present.
            assert!(
                ResourceBackend::load_resource(&store, &id, &iri("urn:eigenius:test:a")).is_some()
            );
            // Chain entry present (root layer — empty parent).
            let info = PB::load_chain_from(&store, &id).unwrap().expect("chain");
            assert_eq!(info.handles.len(), 1);
            assert!(info.handles[0].is_root());
        }

        /// `store_bloom` standalone path (separate from `store_layer`'s
        /// commit batch). Useful for migrations and tests.
        #[test]
        fn store_bloom_standalone_round_trip() {
            use eigenius_kernel::layer::BloomFilter;
            use std::collections::BTreeSet;

            let (store, _dir) = open_temp_store();
            let layer_id = LayerId([13u8; 32]);

            // No bloom yet.
            assert!(PB::load_bloom(&store, &layer_id).unwrap().is_none());

            let iris: BTreeSet<_> = (0..50)
                .map(|i| iri(&format!("urn:eigenius:test:s{i}")))
                .collect();
            let bloom = BloomFilter::for_iris(&iris);
            PB::store_bloom(&store, &layer_id, &bloom).unwrap();

            let loaded = PB::load_bloom(&store, &layer_id).unwrap().expect("present");
            assert_eq!(loaded, bloom);
        }

        /// Phase 14d: branch ref round-trip through RocksDB. Validates
        /// `branch:<name>` key encoding, multi-branch enumeration order,
        /// and persistence across reopen (key is plain bytes, no CBOR
        /// surface to drift).
        #[test]
        fn branch_refs_round_trip() {
            let dir = TempDir::new().unwrap();
            let id_a = LayerId([7u8; 32]);
            let id_b = LayerId([8u8; 32]);

            // Write + close.
            {
                let store = RocksStore::open(dir.path()).unwrap();
                assert!(PB::get_branch(&store, "main").unwrap().is_none());
                assert!(PB::list_branches(&store).unwrap().is_empty());

                PB::put_branch(&store, "main", &id_a).unwrap();
                PB::put_branch(&store, "auto-divergent-1", &id_b).unwrap();

                let listed = PB::list_branches(&store).unwrap();
                assert_eq!(listed.len(), 2);
                // Sorted by name.
                assert_eq!(listed[0], ("auto-divergent-1".into(), id_b.clone()));
                assert_eq!(listed[1], ("main".into(), id_a.clone()));
            }

            // Reopen — branch refs survive.
            {
                let store = RocksStore::open(dir.path()).unwrap();
                assert_eq!(PB::get_branch(&store, "main").unwrap(), Some(id_a.clone()));
                assert_eq!(
                    PB::get_branch(&store, "auto-divergent-1").unwrap(),
                    Some(id_b.clone())
                );

                // Delete + verify.
                PB::delete_branch(&store, "main").unwrap();
                assert!(PB::get_branch(&store, "main").unwrap().is_none());
                let remaining = PB::list_branches(&store).unwrap();
                assert_eq!(remaining.len(), 1);
                assert_eq!(remaining[0].0, "auto-divergent-1");

                // Delete on absent is a no-op.
                PB::delete_branch(&store, "main").unwrap();
            }
        }
    } // mod cbor_coverage_tests

    #[tokio::test]
    async fn trace_store_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let key = [99u8; 32];

        // Write trace
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let trace = ComponentTrace {
                component: "urn:test:comp".to_string(),
                input_hash: key,
                argument_hash: None,
                output: Resource::new(iri("urn:test:out")),
                cached: false,
                metrics: None,
            };
            store.put_component_trace(key, trace);
        }

        // Reopen and verify
        {
            let store = RocksStore::open(dir.path()).unwrap();
            let loaded = store.get_component_trace(&key);
            assert!(loaded.is_some());
            assert_eq!(loaded.unwrap().component, "urn:test:comp");
        }
    }
}
