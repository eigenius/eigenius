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

//! Storage interface traits for persisting layers and resources.
//!
//! Storage backends implement these traits. Phase 0 uses the in-memory
//! backend; SQLite and TiKV come in later phases.

use crate::layer::{Layer, LayerId, LayerTopology};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use async_trait::async_trait;
use std::fmt;
#[allow(unused_imports)]
use std::sync::Arc;

#[cfg(test)]
pub(crate) mod memory;

/// Sync, single-resource read surface for `Layer`.
///
/// `PersistentBackend` is a supertrait, so every persistent backend
/// automatically satisfies this; the smaller surface exists so test backends
/// don't have to implement the full `PersistentBackend` (head/chain/meta/...)
/// just to be plugged into a `Layer`.
///
/// Two flavours of read:
///
/// - [`load_resource`](ResourceBackend::load_resource) — panics on storage
///   error. Matches the kernel's "broken disk = process death" failure model
///   for RocksDB. Use this for normal lookups; supervisor restarts handle the
///   rare disk-failure case.
/// - [`try_load_resource`](ResourceBackend::try_load_resource) — returns
///   `Result` so callers that want to handle backend failures explicitly can.
///   Phase 14 doesn't use this internally; it exists so that future networked
///   backends (TiKV) and storage-aware tooling can adopt fallible reads
///   without forcing the panic path through another rewrite.
pub trait ResourceBackend: Send + Sync {
    /// Look up `iri` in the layer's stored content. Panics on storage error
    /// (treats it as kernel-fatal — for RocksDB this means corruption or
    /// disk failure, neither of which is recoverable in-process).
    fn load_resource(&self, layer_id: &LayerId, iri: &Iri) -> Option<Resource>;

    /// Same lookup, but returns the storage error explicitly. Use when you
    /// want to handle transient backend failures.
    fn try_load_resource(
        &self,
        layer_id: &LayerId,
        iri: &Iri,
    ) -> Result<Option<Resource>, StorageError>;

    /// Enumerate all IRIs defined directly in `layer_id`. Used by chain
    /// reconstruction to populate `Layer::defined_iris` without loading
    /// resource bodies eagerly.
    fn list_layer_iris(
        &self,
        layer_id: &LayerId,
    ) -> Result<std::collections::BTreeSet<Iri>, StorageError>;
}

/// Chain reconstruction metadata returned by `PersistentBackend::load_chain`.
///
/// Carries everything needed to construct a chain of `Layer`s without
/// holding any resource content — just `LayerHandle`s and per-layer IRI
/// sets. The actual `Arc<Layer>` chain is built by
/// [`crate::layer::build_chain`] given this info plus a cache + backend Arc.
#[derive(Debug, Clone)]
pub struct ChainInfo {
    /// Head LayerId; last entry of `handles` should match this.
    pub head: LayerId,
    /// Handles ordered root → head.
    pub handles: Vec<crate::layer::LayerHandle>,
    /// IRIs defined per layer.
    pub defined_iris_per_layer:
        std::collections::BTreeMap<LayerId, std::collections::BTreeSet<Iri>>,
}

/// Errors from storage operations.
#[derive(Debug)]
pub enum StorageError {
    NotFound(String),
    Internal(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::NotFound(msg) => write!(f, "not found: {msg}"),
            StorageError::Internal(msg) => write!(f, "storage error: {msg}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// Trait for storing and retrieving committed layers.
#[async_trait]
pub trait LayerStore: Send + Sync {
    /// Store a committed layer.
    async fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError>;

    /// Load a layer by its content-addressed ID.
    async fn load_layer(&self, id: &LayerId) -> Result<Layer, StorageError>;

    /// List all stored layer IDs.
    async fn list_layers(&self) -> Result<Vec<LayerId>, StorageError>;
}

/// Trait for storing and retrieving individual resources within a layer.
#[async_trait]
pub trait ResourceStore: Send + Sync {
    /// Store a resource associated with a layer.
    async fn store_resource(
        &self,
        layer_id: &LayerId,
        resource: &Resource,
    ) -> Result<(), StorageError>;

    /// Load a resource by IRI within a layer.
    async fn load_resource(
        &self,
        layer_id: &LayerId,
        iri: &Iri,
    ) -> Result<Option<Resource>, StorageError>;

    /// List all resource IRIs in a layer.
    async fn list_resources(&self, layer_id: &LayerId) -> Result<Vec<Iri>, StorageError>;
}

/// A persistent backend usable by the kernel server.
///
/// Combines layer storage, metadata storage (for the seed manifest from
/// D13 §4.2), and trace-store access into a single trait object the
/// kernel can carry without depending on any particular storage crate.
/// The sync-flavored head/chain methods are used at boot, so we keep
/// them synchronous rather than going async-within-async.
pub trait PersistentBackend: ResourceBackend + Send + Sync + 'static {
    /// Read the current head layer ID, if any.
    fn get_head(&self) -> Result<Option<LayerId>, StorageError>;

    /// Write the current head layer ID atomically.
    fn set_head(&self, id: &LayerId) -> Result<(), StorageError>;

    /// Reconstruct chain metadata from the persisted head.
    ///
    /// Returns the `ChainInfo` describing the chain from root → head, with
    /// handles and per-layer IRI sets. The caller turns this into a
    /// `Arc<Layer>` chain via [`crate::layer::build_chain`], passing in
    /// the cache and an `Arc<dyn ResourceBackend>` (typically obtained by
    /// upcasting the `Arc<dyn PersistentBackend>` they hold).
    ///
    /// Returns `None` if no head is set.
    fn load_chain(&self) -> Result<Option<ChainInfo>, StorageError>;

    /// Reconstruct chain metadata for a specific head `LayerId`. Used by
    /// the `at_layer` read-path extension (D21 §3.7) and by resume to
    /// re-hydrate a task's pinned head. Returns `None` if the target
    /// layer is absent from the store.
    fn load_chain_from(&self, head_id: &LayerId) -> Result<Option<ChainInfo>, StorageError>;

    /// Store a layer (metadata + resources + chain pointer + topology
    /// handle). Idempotent by layer id (content-addressed).
    ///
    /// Phase 14a-ii adds a `topo:<id>` entry per stored layer alongside the
    /// existing `layer:` and `chain:` entries; `load_topology` (below) reads
    /// those back. The topology entry is purely metadata — small fixed-size
    /// `LayerHandle` carrying id, parents, name, resource_count, and creation
    /// time.
    fn store_layer(&self, layer: &Layer) -> Result<LayerId, StorageError>;

    /// Load the in-memory layer topology — every known layer's `LayerHandle`,
    /// keyed by `LayerId`, ready for in-memory walks via `walk_chain` etc.
    ///
    /// No migration from earlier layouts is supported: a DB written by a
    /// pre-Phase-14 kernel must be re-built from source files. Returns an
    /// empty topology for an empty DB.
    fn load_topology(&self) -> Result<LayerTopology, StorageError>;

    /// Generic metadata key-value store. Used for the seed manifest
    /// (D13 §4.2) and for future configuration that shouldn't live in
    /// an Eigon resource.
    fn get_meta(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError>;

    /// Store a metadata value at `key`.
    fn put_meta(&self, key: &str, value: &[u8]) -> Result<(), StorageError>;

    /// Delete a metadata value at `key`. Used by the task-retention
    /// pruner (D21 §5). No-op if the key is absent.
    fn delete_meta(&self, key: &str) -> Result<(), StorageError>;

    /// Apply a batch of metadata operations atomically.
    ///
    /// Per D21 §8 "step atomicity" — every task step must write its
    /// IO trace, its meta update, and (on checkpoint steps) its
    /// checkpoint as a single commit so a crash cannot leave a
    /// half-applied task step on disk.
    ///
    /// RocksDB maps this to `rocksdb::WriteBatch`. In-memory backends
    /// apply the ops sequentially under their existing lock, which is
    /// trivially atomic because nothing else observes the store during
    /// the batch.
    fn write_batch(&self, ops: &[BatchOp]) -> Result<(), StorageError>;

    /// Enumerate metadata keys sharing a given prefix. Used by the
    /// task-resume sweep to find all `session:<id>:task:<id>:meta`
    /// records. Ordering is not guaranteed by the trait; callers must
    /// impose their own (typically `created_at` from the decoded
    /// record).
    fn list_meta_prefix(&self, prefix: &str) -> Result<Vec<String>, StorageError>;

    /// Borrow the trace store view of this backend. Lets the server
    /// route `ComponentTrace` reads/writes through the same storage.
    fn as_trace_store(&self) -> &(dyn crate::program::trace::TraceStore + Send + Sync);

    /// Read a layer's persisted shadowing bloom (D23 §5.2). Returns
    /// `None` if no bloom was persisted — a layer written by an
    /// older kernel build, or any layer for which `store_layer`
    /// hasn't run since the bloom was added.
    ///
    /// Phase 14b: `store_layer` writes the bloom atomically alongside
    /// the layer's other entries; `BloomCache::get_or_load` reads it
    /// here on cache miss. Sync surface to match `get_head` /
    /// `set_head` and the rest of the hot-path read API.
    fn load_bloom(
        &self,
        layer: &LayerId,
    ) -> Result<Option<crate::layer::BloomFilter>, StorageError>;

    /// Persist a bloom for `layer`. Used by tests and by migrations
    /// that retroactively populate blooms; production commit goes
    /// through `store_layer` which writes the bloom in the same
    /// atomic batch as the layer's other entries.
    fn store_bloom(
        &self,
        layer: &LayerId,
        bloom: &crate::layer::BloomFilter,
    ) -> Result<(), StorageError>;

    // --- Branch refs (D23 §5.5 / Phase 14d) ---
    //
    // Branches are named pointers into the layer DAG. The kernel never
    // tracks "the head" beyond per-branch refs — `crate::lattice::update_branch`
    // is the only sanctioned write path. The `head` key set by `set_head`
    // remains for the legacy single-head boot path; future migration folds
    // it into `branch:main`.

    /// Read the current head of `branch`. Returns `None` if the branch
    /// doesn't exist; callers wanting to create a new branch pass
    /// `expected_old_head: None` to `update_branch` and that's
    /// indistinguishable from "branch absent" at this layer.
    fn get_branch(&self, name: &str) -> Result<Option<LayerId>, StorageError>;

    /// Set `branch` to point at `id`. Overwrites any existing value.
    /// **Not** a CAS primitive on its own — `crate::lattice::update_branch`
    /// is the safe write surface; this is the storage primitive
    /// `update_branch` lowers to once it has confirmed the CAS.
    fn put_branch(&self, name: &str, id: &LayerId) -> Result<(), StorageError>;

    /// Remove the branch ref. The layers it pointed at remain in the DAG
    /// until GC (Phase 14f) reclaims layers reachable only through the
    /// pruned branch. Used by `eigenius db delete-branch` and the
    /// soon-to-arrive `prune-branch` (14g) operations.
    fn delete_branch(&self, name: &str) -> Result<(), StorageError>;

    /// Enumerate all branch refs as `(name, head)` pairs, sorted by
    /// name. Used by `eigenius db branch list` and by GC to gather
    /// branch-head roots.
    fn list_branches(&self) -> Result<Vec<(String, LayerId)>, StorageError>;

    /// Atomically delete every storage entry associated with `layer`:
    /// the `topo:<id>` topology entry, the `bloom:<id>` shadowing bloom,
    /// the `chain:<id>` parent pointer, and every `layer:<id>:res:*`
    /// resource entry. Used by Phase 14f garbage collection (D23 §5.7)
    /// to reclaim storage for unreachable layers.
    ///
    /// The delete is one atomic write (per D23 §6.3) — partial deletion
    /// is impossible. After this returns, the layer is gone from
    /// storage; in-memory caches must be evicted separately by the
    /// caller (`ResourceCache::evict_layer`, `BloomCache::evict_layer`).
    ///
    /// No-op if the layer doesn't exist (idempotent — safe to call
    /// during a re-run of GC against the same id).
    fn delete_layer(&self, layer: &LayerId) -> Result<(), StorageError>;
}

/// A single operation inside a `write_batch` call.
#[derive(Debug, Clone)]
pub enum BatchOp {
    /// Put a metadata key. Same semantics as `put_meta`.
    PutMeta { key: String, value: Vec<u8> },
    /// Delete a metadata key. Same semantics as `delete_meta`.
    DeleteMeta { key: String },
}
