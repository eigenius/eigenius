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
//! Storage backends implement [`PersistentBackend`] (and its
//! supertrait [`ResourceBackend`]). The kernel's production write
//! path goes through `PersistentBackend`; the trait surface is
//! sync because the resolve hot path (`Layer::resolve`) is sync and
//! would have to be reworked to thread `.await` through every chain
//! walk otherwise. The RocksDB-backed impl wraps disk-bound bodies
//! in [`tokio::task::block_in_place`] so it doesn't starve the
//! tokio worker pool under concurrent sessions.

use crate::layer::{Layer, LayerId, LayerTopology};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use std::fmt;
#[allow(unused_imports)]
use std::sync::Arc;

pub mod content_array;

pub mod memory;

pub mod version;

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

/// One row of the anchored-commit cache (D33 §6 / Phase 20c).
///
/// The cache memoizes `commit(content, supporting_layer) → LayerId`,
/// keyed on `(content_hash, supporting_content_hash)`. A hit means
/// the same content has previously been committed against a
/// supporting layer with the same content — the cached `LayerId` is
/// the canonical representative for that combination. Used by:
/// notebook cell re-runs, institution ontology reload, mirror
/// regeneration, and any deterministic content generator whose
/// supporting context (per `compute_supporting_layer`) is the
/// dependency anchor.
///
/// Carries the cache key + value for
/// [`PersistentBackend::list_anchored_commits`] (diagnostic
/// enumeration) and for tests that assert cache state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredCommitEntry {
    pub content_hash: crate::layer::ContentHash,
    pub supporting_content_hash: crate::layer::ContentHash,
    pub layer_id: LayerId,
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

/// A persistent backend usable by the kernel server.
///
/// Combines layer storage, metadata storage (for the seed manifest from
/// D13 §4.2), branch refs (Phase 14d), and trace-store access into a
/// single trait object the kernel can carry without depending on any
/// particular storage crate. Sync surface — the boot path is synchronous
/// to avoid async-within-async.
///
/// **Phase 14g:** the pre-Phase-14 single-`head` pointer (`get_head` /
/// `set_head` / `load_chain`) is gone. Branches are the only sanctioned
/// head-pointer surface. `bootstrap_persistent`'s seed-vs-resume
/// discriminator keys off `branch:main`; chain reconstruction goes
/// through `load_chain_from(branch_head)`.
pub trait PersistentBackend: ResourceBackend + Send + Sync + 'static {
    /// Reconstruct chain metadata for a specific head `LayerId`. Used
    /// by `bootstrap_persistent`'s resume path (loading from
    /// `branch:main`), by the `at_layer` read-path extension (D21 §3.7),
    /// and by resume to re-hydrate a task's pinned head. Returns
    /// `None` if the target layer is absent from the store.
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

    /// Load a single `LayerHandle` by id. Returns `Ok(None)` when the
    /// layer is unknown to the store. Cheaper than `load_topology` for
    /// the common "I have a `LayerId`, I just want its handle" path —
    /// no full-topology scan, no allocation of a `LayerTopology` map.
    ///
    /// Used by the anchored-commit cache (D33 §6) to look up the supporting
    /// layer's `content_hash` at commit time, and by future diagnostic
    /// surfaces that want to inspect a single layer's metadata.
    fn load_handle(
        &self,
        layer_id: &LayerId,
    ) -> Result<Option<crate::layer::LayerHandle>, StorageError>;

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

    /// Arc-shared triple index view of this backend (D23 §5.9 / Phase 14h).
    ///
    /// Returned as an `Arc<dyn TripleIndex>` so it slots directly into
    /// `LayerStorage.triple_index`. Every backend implementation owns
    /// its own underlying index (in-memory `MemoryTripleIndex` for the
    /// memory backend; RocksDB prefix scans for `RocksStore`) and returns
    /// `Arc::clone`s so multiple `LayerStorage` instances share the same
    /// physical index.
    fn triple_index_arc(&self) -> Arc<dyn crate::layer::TripleIndex>;

    /// Arc-shared text index view of this backend (D43 §2.3, M2.3).
    ///
    /// Same pattern as [`Self::triple_index_arc`]: each backend owns
    /// its own [`TextIndex`](crate::layer::TextIndex) impl and hands
    /// out `Arc::clone`s for `LayerStorage.text_index`. The memory
    /// backend uses `MemoryTextIndex`; `RocksStore` exposes
    /// `RocksTextIndex` (M2.4) backed by the `cf_text` column family
    /// with the four-key schema from D43 §2.3.
    fn text_index_arc(&self) -> Arc<dyn crate::layer::TextIndex>;

    /// Arc-shared vector index view of this backend (D43 §2.4, M2.3).
    ///
    /// Same pattern as [`Self::triple_index_arc`]: each backend owns
    /// its own [`VectorIndex`](crate::layer::VectorIndex) impl and
    /// hands out `Arc::clone`s for `LayerStorage.vector_index`. The
    /// memory backend uses `MemoryVectorIndex`; `RocksStore` exposes
    /// `RocksVectorIndex` (M2.5) backed by the `cf_vec` column family
    /// with the CBOR segment layout from D43 §2.4.
    fn vector_index_arc(&self) -> Arc<dyn crate::layer::VectorIndex>;

    /// Arc-shared exact value index view of this backend (D65).
    ///
    /// Same pattern as [`Self::triple_index_arc`]: each backend owns its own
    /// [`ValueIndex`](crate::layer::ValueIndex) impl and hands out `Arc::clone`s
    /// for `LayerStorage.value_index`. The memory backend uses `MemoryValueIndex`;
    /// `RocksStore` exposes `RocksValueIndex` backed by its own column family.
    fn value_index_arc(&self) -> Arc<dyn crate::layer::ValueIndex>;

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

    // --- Tag refs (D34 §G.2 / §8) -----------------------------------
    //
    // Tags are immutable named refs into the DAG. Unlike branches, a
    // tag's target cannot be retargeted once created — there is no
    // `put_tag` after the first `create_tag` succeeds. Tags pin their
    // target (and its transitive ancestors) against GC as long as
    // they exist (§8.3 — tags are GC roots).

    /// Create a new tag at `name` pointing at `id`. Returns `false`
    /// when a tag with this name already exists (the existing target
    /// is preserved). Use [`delete_tag`] + a fresh `create_tag` if a
    /// retarget is genuinely intended; there is intentionally no
    /// "update" surface.
    fn create_tag(&self, name: &str, id: &LayerId) -> Result<bool, StorageError>;

    /// Look up the target of `name`. `None` for unknown tags.
    fn get_tag(&self, name: &str) -> Result<Option<LayerId>, StorageError>;

    /// Remove the tag. Returns `false` when the tag didn't exist
    /// (idempotent). The target layer becomes GC-eligible if no
    /// other root still reaches it.
    fn delete_tag(&self, name: &str) -> Result<bool, StorageError>;

    /// Enumerate all tag refs as `(name, layer_id)` pairs, sorted by
    /// name. Used by `ListTags` and by GC to gather tag-rooted
    /// reachability roots.
    fn list_tags(&self) -> Result<Vec<(String, LayerId)>, StorageError>;

    /// Atomically delete every storage entry associated with `layer`:
    /// the `topo:<id>` topology entry, the `bloom:<id>` shadowing bloom,
    /// the `chain:<id>` parent pointer, every `layer:<id>:res:*`
    /// resource entry, and the content-hash dedup index entry for the
    /// layer's content. Used by Phase 14f garbage collection (D23 §5.7)
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

    // --- Resolve redirects (D25 §12.8 / Phase 17f) ---
    //
    // Forward pointers installed by `consolidate_chain` when `to` is
    // below the branch head. See `RedirectEntry` for the on-disk shape
    // and [`augment_topology_with_redirects`] for the
    // synthetic-tombstone integration with `load_topology`.

    /// Install a redirect. Idempotent by `entry.source()` —
    /// overwriting an existing entry is permitted only when the
    /// consolidation algorithm explicitly asks for it (e.g., a
    /// future compose policy); the v1 chaining refusal at the
    /// algorithm level keeps this from happening accidentally.
    ///
    /// Atomic within the backend's commit primitive — RocksDB lands
    /// it in the same `WriteBatch` as the consolidated layer's
    /// `store_layer` writes; the memory backend serializes through
    /// its single `RwLock`.
    fn put_redirect(&self, entry: &crate::layer::RedirectEntry) -> Result<(), StorageError>;

    /// Resolve a redirect by source `LayerId`. `None` if the layer
    /// isn't a redirect source.
    fn lookup_redirect(
        &self,
        source: &LayerId,
    ) -> Result<Option<crate::layer::RedirectEntry>, StorageError>;

    /// Remove a redirect. Used by the (future) compose policy to
    /// replace an existing redirect with a new one; v1 doesn't call
    /// this on the consolidation path but exposes it for symmetry +
    /// future use.
    fn delete_redirect(&self, source: &LayerId) -> Result<(), StorageError>;

    /// Enumerate every installed redirect. Used by `load_topology`
    /// to build the synthetic-tombstone view at startup, and by
    /// diagnostic surfaces (future `db consolidate-summary`). Result
    /// order is unspecified; callers that care should sort.
    fn list_redirects(&self) -> Result<Vec<crate::layer::RedirectEntry>, StorageError>;

    // --- Anchored-commit cache (D33 §6 / Phase 20c) ---
    //
    // Memoizes `commit(content, supporting_layer) → LayerId`, keyed on
    // `(new layer's content_hash, supporting layer's content_hash)`.
    // A hit returns the canonical existing layer for that combination
    // — no re-validation, no re-store. The "supporting layer's
    // content_hash" — not its position hash — is what makes
    // structurally-equivalent supporting contexts hit the same cache
    // entry even when their parent linearizations differ (D33 §6
    // "supporting-equivalent context").
    //
    // **Use cases.** The cache generalizes across notebook cell
    // re-runs, institution ontology reload, mirror regeneration, and
    // any deterministic content generator that anchors to a supporting
    // layer. "Anchored" — the supporting layer is the content's
    // dependency anchor — is the structural framing; cell-output
    // reuse is one application.

    /// Look up a previously-cached layer id for `(content_hash,
    /// supporting_content_hash)`. `Ok(None)` for a cache miss; the
    /// caller falls through to the standard commit path.
    fn lookup_anchored_commit(
        &self,
        content_hash: &crate::layer::ContentHash,
        supporting_content_hash: &crate::layer::ContentHash,
    ) -> Result<Option<LayerId>, StorageError>;

    /// Record `(content_hash, supporting_content_hash) → layer_id` in
    /// the cache. Idempotent by the (content, supporting) pair —
    /// overwriting an existing entry is permitted (later commits of
    /// the same content + supporting context might choose a different
    /// representative position, e.g. after preserve-history
    /// consolidation).
    fn put_anchored_commit(
        &self,
        content_hash: &crate::layer::ContentHash,
        supporting_content_hash: &crate::layer::ContentHash,
        layer_id: &LayerId,
    ) -> Result<(), StorageError>;

    /// Remove a single cache entry. Used by future cache-management
    /// surfaces; v1 doesn't expose this on the CLI but the primitive
    /// is needed by tests and by GC paths that prune entries pointing
    /// at swept layers.
    fn delete_anchored_commit(
        &self,
        content_hash: &crate::layer::ContentHash,
        supporting_content_hash: &crate::layer::ContentHash,
    ) -> Result<(), StorageError>;

    /// Enumerate every anchored-commit cache entry. Used by
    /// diagnostic surfaces and by cross-cutting tests that assert
    /// cache contents. Result order is unspecified.
    fn list_anchored_commits(&self) -> Result<Vec<AnchoredCommitEntry>, StorageError>;

    /// Look up every layer whose content matches `content_hash`.
    ///
    /// Returns the set of position hashes (layer ids) currently in
    /// storage that share the given content hash. An empty result is
    /// normal: not every content hash that ever existed maps to a live
    /// layer. Multiple results indicate the same content has been
    /// committed at multiple DAG positions — e.g. the same notebook
    /// cell run against two different parent chains, or the same
    /// comorphism reify output produced from two different invocations.
    ///
    /// Used by:
    /// - [D25 §11.0](../../docs/design/d25-chain-consolidation.md)
    ///   consolidated-layer dedup: before producing a new consolidated
    ///   layer, check whether identical content already exists at a
    ///   compatible position to skip the redundant commit.
    /// - [D33 §6](../../docs/design/d33-partial-order-chains.md)
    ///   anchored-commit cache (joined with supporting-layer lookup
    ///   on the consumer side to form the `(content_hash,
    ///   supporting_content_hash)` cache key).
    /// - [D25 §12.1](../../docs/design/d25-chain-consolidation.md)
    ///   tag-target resolution: `chain:tag_target_content` resolves to
    ///   every position carrying that content.
    ///
    /// Result order is unspecified; callers that care should sort.
    fn lookup_by_content_hash(
        &self,
        content_hash: &crate::layer::ContentHash,
    ) -> Result<Vec<LayerId>, StorageError>;
}

/// A single operation inside a `write_batch` call.
#[derive(Debug, Clone)]
pub enum BatchOp {
    /// Put a metadata key. Same semantics as `put_meta`.
    PutMeta { key: String, value: Vec<u8> },
    /// Delete a metadata key. Same semantics as `delete_meta`.
    DeleteMeta { key: String },
}
