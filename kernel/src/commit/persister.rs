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

//! `LayerPersister` boundary and the [`PersistedLayerInfo`] it returns.
//!
//! The pipeline's `persist` phase (see `phases::persist`) calls into a
//! `LayerPersister`. `EigeniusService` will implement the trait in Phase C
//! by lifting the body of today's `persist_layer_if_backend` (see
//! `kernel/src/server/mod.rs`). Tests will inject in-memory implementations
//! that return canned [`PersistedLayerInfo`] values.
//!
//! Phase A: trait + struct are defined. No implementations.
//!
//! See D41 §7 for the contract and §11.1 for module layout.

use std::sync::Arc;

use crate::layer::Layer;
use crate::storage::PersistentBackend;
use crate::validation::{ValidationError, ValidationRule};

/// The persist seam between the commit pipeline and storage.
///
/// The pipeline's `persist` phase calls [`LayerPersister::persist`] once
/// per layer. The implementation owns:
///
/// - the anchored-commit cache probe (D33 §6),
/// - the `backend.store_layer` write,
/// - the `update_branch` CAS,
/// - the trivial-merge handling.
///
/// The pipeline does not interpret the [`PersistedLayerInfo`]; the
/// orchestrator inspects [`PersistedLayerInfo::branch_advanced`] to decide
/// whether to drain emissions or skip descendants.
///
/// See D41 §7.
pub trait LayerPersister: Send + Sync {
    /// Persist `layer` against `branch`, returning the canonical id and
    /// merge / cache outcome. Errors are surfaced as
    /// [`ValidationError`] so they slot into [`crate::lattice::CommitError`]
    /// reporting without an additional wrapper layer (Phase A; Phase B / E
    /// may split this).
    fn persist(
        &self,
        branch: &str,
        layer: &Arc<Layer>,
    ) -> Result<PersistedLayerInfo, ValidationError>;

    /// Whether `layer` is already proven valid in this exact context — i.e. the
    /// anchored-commit cache (D33 §6) holds an entry for `(content_hash, supporting
    /// content_hash)`. That entry is written only after a full, *validated* commit, and
    /// a layer's validity is a function of its content plus the chain below it (both
    /// pinned by the key). So a hit means structural + retroactive validation already
    /// passed for this exact content-on-this-exact-support, and the commit pipeline can
    /// skip re-running them (it still runs `persist`, where the same cache hit also
    /// skips `store_layer`). Default `false` — only a cache-backed persister overrides.
    fn already_validated(&self, _layer: &Layer) -> bool {
        false
    }
}

/// Result of a single [`LayerPersister::persist`] call — the
/// canonical [`crate::layer::LayerId`] for the committed content
/// paired with the merge outcome and a derived `branch_advanced`
/// flag.
///
/// **Canonical home (D41 §7 / §11.1):** this is the single struct
/// definition for the persist result. The server-side duplicate that
/// previously lived in `crate::server::mod` was deleted in Phase C;
/// the server now imports this type and continues to use the same
/// field names.
///
/// **`branch_advanced` semantics** (D33 §6 + D23 §5.4):
///
/// - `true` — the durable branch ref moved as a result of this
///   persist. Holds for cache misses, same-position cache hits, and
///   both `FastForward` / `TrivialMerge` CAS outcomes.
/// - `false` — the branch ref did **not** move. Holds for: no
///   persistent backend, different-position cache hit, and the
///   `NeedsWitnessedMerge` CAS outcome (the layer is stored but
///   unreachable from any branch ref).
///
/// **`merge_outcome` semantics:**
///
/// `Some(...)` whenever a CAS attempt actually ran (cache miss or
/// same-position cache hit); `None` for the no-backend path and for
/// different-position cache hits — in both cases there is no merge
/// taxonomy because no CAS happened. The proto boundary maps `None`
/// to `proto::MergeOutcome::Unspecified`.
///
/// **Why `Option<UpdateOutcome>` + `cache_hit_different_position`
/// instead of D41's `update_outcome: UpdateOutcome` + `cache_hit: bool`
/// spec.** The current shape encodes three distinct post-persist
/// states the proto wire format needs to distinguish: cache hit at a
/// different position, CAS ran, and no CAS attempted. Collapsing
/// `Option<UpdateOutcome>` into a non-optional `UpdateOutcome` would
/// force a synthetic "did not attempt" variant on
/// [`crate::lattice::UpdateOutcome`], which is the wrong shape for an
/// enum that names CAS results. Phase C of D41 deferred reconciliation
/// of the doc spec to a docs follow-up rather than reshape the
/// survivor; the canonical-struct goal of §7 is achieved by deduping,
/// not by reshaping.
///
/// D41 §7.
#[derive(Debug, Clone)]
pub struct PersistedLayerInfo {
    /// Canonical layer id. For a cache hit at a different position
    /// (D33 §6) this is the cached layer's id, not the freshly-built
    /// one.
    pub layer_id: crate::layer::LayerId,
    /// `true` iff the persist actually moved the branch ref. Drives
    /// the orchestrator's drain / revert decision and the
    /// `didPersist` hook gate (see D41 §6.1).
    pub branch_advanced: bool,
    /// `Some(...)` iff a CAS actually ran; `None` for the no-backend
    /// path and for different-position cache hits.
    pub merge_outcome: Option<crate::lattice::UpdateOutcome>,
    /// `true` iff the persist short-circuited because the
    /// anchored-commit cache (D33 §6) found a content-equivalent
    /// layer at a different chain position. Distinguished from the
    /// no-backend / no-CAS case (where `merge_outcome` is also `None`
    /// and `branch_advanced` is also `false`) so the response can
    /// carry a `MERGE_OUTCOME_CACHED_DIFFERENT_POSITION` signal that
    /// consumers can render distinctly from "no commit shape
    /// information available".
    pub cache_hit_different_position: bool,
}

/// Minimal [`LayerPersister`] for callers that just need
/// `PersistentBackend::store_layer` — no anchored-commit cache, no
/// branch CAS. Used by [`crate::lattice::commit_layer`] and
/// [`crate::lattice::commit_layer_default`] (CLI commits, bootstrap,
/// GC tests, storage E2E tests). Returns a [`PersistedLayerInfo`] with
/// `merge_outcome = None` and `branch_advanced = false` — there is no
/// branch CAS in this path, so "did the branch advance?" is a question
/// the lattice wrapper deliberately doesn't answer.
///
/// `EigeniusService` will implement [`LayerPersister`] directly with
/// cache + CAS (D41 Phase C); only the simple commit path uses this
/// adapter. The adapter exists during Phase B so the new
/// [`crate::commit::CommitPipeline::with_retroactive`] pipeline can
/// service the lattice's pre-D41 callers without yet pulling the
/// server's persistence stack into the kernel core.
///
/// D41 §7 / Phase B.
pub struct BackendStorePersister<'a> {
    /// Backend the persister writes through.
    pub backend: &'a dyn PersistentBackend,
}

impl LayerPersister for BackendStorePersister<'_> {
    /// `branch` is ignored — the lattice path is branch-agnostic.
    /// Storage errors translate to a synthetic [`ValidationError`]
    /// (rule [`ValidationRule::InstitutionValidation`] as a Phase B
    /// stand-in; the persister-returns-`ValidationError` shape is a
    /// known transitional fiction that Phase C resolves by widening
    /// the persister error type).
    fn persist(
        &self,
        _branch: &str,
        layer: &Arc<Layer>,
    ) -> Result<PersistedLayerInfo, ValidationError> {
        self.backend
            .store_layer(layer)
            .map_err(|e| ValidationError {
                resource_id: None,
                property: None,
                // D41 Phase B: no `ValidationRule` variant fits "I/O
                // failure". Phase E may revisit the persister error
                // type; for now use `InstitutionValidation` as the
                // most policy-shaped existing variant and carry the
                // backend message verbatim so callers can identify
                // the underlying cause.
                rule: ValidationRule::InstitutionValidation,
                message: format!("persist_layer failed: {e}"),
            })?;
        Ok(PersistedLayerInfo {
            layer_id: layer.id().clone(),
            // No CAS in this path — the lattice wrapper does not
            // advance any branch ref. Phase D / E will revisit how
            // this surfaces to orchestrator drains; today it never
            // matters because the lattice wrapper unpacks the layer
            // directly and discards the rest of [`PersistedLayerInfo`].
            branch_advanced: false,
            merge_outcome: None,
            cache_hit_different_position: false,
        })
    }
}

#[cfg(test)]
mod tests {
    //! D41 Phase F.5 — `BackendStorePersister` error-mapping and
    //! happy-path field-shape coverage.

    use super::*;
    use crate::layer::{Layer, LayerBuilder, LayerStorage};
    use crate::storage::memory::MemoryPersistentBackend;
    use crate::storage::{
        AnchoredCommitEntry, BatchOp, ChainInfo, PersistentBackend, ResourceBackend, StorageError,
    };
    use std::collections::BTreeSet;
    use std::sync::Arc;

    /// Backend wrapper whose `store_layer` always returns
    /// `StorageError::Internal` so the persister's error mapping can be
    /// asserted. Every other method delegates to an inner
    /// [`MemoryPersistentBackend`] so the trait object compiles cleanly
    /// without manually implementing the long surface.
    struct FailingStoreBackend {
        inner: MemoryPersistentBackend,
    }
    impl FailingStoreBackend {
        fn new() -> Self {
            Self {
                inner: MemoryPersistentBackend::new(),
            }
        }
    }

    impl ResourceBackend for FailingStoreBackend {
        fn load_resource(
            &self,
            layer_id: &crate::layer::LayerId,
            iri: &crate::ontology::iri::Iri,
        ) -> Option<crate::ontology::resource::Resource> {
            self.inner.load_resource(layer_id, iri)
        }
        fn try_load_resource(
            &self,
            layer_id: &crate::layer::LayerId,
            iri: &crate::ontology::iri::Iri,
        ) -> Result<Option<crate::ontology::resource::Resource>, StorageError> {
            self.inner.try_load_resource(layer_id, iri)
        }
        fn list_layer_iris(
            &self,
            layer_id: &crate::layer::LayerId,
        ) -> Result<BTreeSet<crate::ontology::iri::Iri>, StorageError> {
            self.inner.list_layer_iris(layer_id)
        }
    }

    impl PersistentBackend for FailingStoreBackend {
        fn load_chain_from(
            &self,
            head_id: &crate::layer::LayerId,
        ) -> Result<Option<ChainInfo>, StorageError> {
            self.inner.load_chain_from(head_id)
        }
        fn store_layer(&self, _layer: &Layer) -> Result<crate::layer::LayerId, StorageError> {
            Err(StorageError::Internal(
                "synthetic store_layer failure".into(),
            ))
        }
        fn load_topology(&self) -> Result<crate::layer::LayerTopology, StorageError> {
            self.inner.load_topology()
        }
        fn load_handle(
            &self,
            layer_id: &crate::layer::LayerId,
        ) -> Result<Option<crate::layer::LayerHandle>, StorageError> {
            self.inner.load_handle(layer_id)
        }
        fn get_meta(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
            self.inner.get_meta(key)
        }
        fn put_meta(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
            self.inner.put_meta(key, value)
        }
        fn delete_meta(&self, key: &str) -> Result<(), StorageError> {
            self.inner.delete_meta(key)
        }
        fn write_batch(&self, ops: &[BatchOp]) -> Result<(), StorageError> {
            self.inner.write_batch(ops)
        }
        fn list_meta_prefix(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
            self.inner.list_meta_prefix(prefix)
        }
        fn as_trace_store(&self) -> &(dyn crate::program::trace::TraceStore + Send + Sync) {
            self.inner.as_trace_store()
        }
        fn triple_index_arc(&self) -> Arc<dyn crate::layer::TripleIndex> {
            self.inner.triple_index_arc()
        }
        fn text_index_arc(&self) -> Arc<dyn crate::layer::TextIndex> {
            self.inner.text_index_arc()
        }
        fn vector_index_arc(&self) -> Arc<dyn crate::layer::VectorIndex> {
            self.inner.vector_index_arc()
        }
        fn value_index_arc(&self) -> Arc<dyn crate::layer::ValueIndex> {
            self.inner.value_index_arc()
        }
        fn load_bloom(
            &self,
            layer: &crate::layer::LayerId,
        ) -> Result<Option<crate::layer::BloomFilter>, StorageError> {
            self.inner.load_bloom(layer)
        }
        fn store_bloom(
            &self,
            layer: &crate::layer::LayerId,
            bloom: &crate::layer::BloomFilter,
        ) -> Result<(), StorageError> {
            self.inner.store_bloom(layer, bloom)
        }
        fn get_branch(&self, name: &str) -> Result<Option<crate::layer::LayerId>, StorageError> {
            self.inner.get_branch(name)
        }
        fn put_branch(&self, name: &str, id: &crate::layer::LayerId) -> Result<(), StorageError> {
            self.inner.put_branch(name, id)
        }
        fn delete_branch(&self, name: &str) -> Result<(), StorageError> {
            self.inner.delete_branch(name)
        }
        fn list_branches(&self) -> Result<Vec<(String, crate::layer::LayerId)>, StorageError> {
            self.inner.list_branches()
        }
        fn create_tag(&self, name: &str, id: &crate::layer::LayerId) -> Result<bool, StorageError> {
            self.inner.create_tag(name, id)
        }
        fn get_tag(&self, name: &str) -> Result<Option<crate::layer::LayerId>, StorageError> {
            self.inner.get_tag(name)
        }
        fn delete_tag(&self, name: &str) -> Result<bool, StorageError> {
            self.inner.delete_tag(name)
        }
        fn list_tags(&self) -> Result<Vec<(String, crate::layer::LayerId)>, StorageError> {
            self.inner.list_tags()
        }
        fn delete_layer(&self, layer: &crate::layer::LayerId) -> Result<(), StorageError> {
            self.inner.delete_layer(layer)
        }
        fn put_redirect(&self, entry: &crate::layer::RedirectEntry) -> Result<(), StorageError> {
            self.inner.put_redirect(entry)
        }
        fn lookup_redirect(
            &self,
            source: &crate::layer::LayerId,
        ) -> Result<Option<crate::layer::RedirectEntry>, StorageError> {
            self.inner.lookup_redirect(source)
        }
        fn delete_redirect(&self, source: &crate::layer::LayerId) -> Result<(), StorageError> {
            self.inner.delete_redirect(source)
        }
        fn list_redirects(&self) -> Result<Vec<crate::layer::RedirectEntry>, StorageError> {
            self.inner.list_redirects()
        }
        fn lookup_anchored_commit(
            &self,
            content_hash: &crate::layer::ContentHash,
            supporting_content_hash: &crate::layer::ContentHash,
        ) -> Result<Option<crate::layer::LayerId>, StorageError> {
            self.inner
                .lookup_anchored_commit(content_hash, supporting_content_hash)
        }
        fn put_anchored_commit(
            &self,
            content_hash: &crate::layer::ContentHash,
            supporting_content_hash: &crate::layer::ContentHash,
            layer_id: &crate::layer::LayerId,
        ) -> Result<(), StorageError> {
            self.inner
                .put_anchored_commit(content_hash, supporting_content_hash, layer_id)
        }
        fn delete_anchored_commit(
            &self,
            content_hash: &crate::layer::ContentHash,
            supporting_content_hash: &crate::layer::ContentHash,
        ) -> Result<(), StorageError> {
            self.inner
                .delete_anchored_commit(content_hash, supporting_content_hash)
        }
        fn list_anchored_commits(&self) -> Result<Vec<AnchoredCommitEntry>, StorageError> {
            self.inner.list_anchored_commits()
        }
        fn lookup_by_content_hash(
            &self,
            content_hash: &crate::layer::ContentHash,
        ) -> Result<Vec<crate::layer::LayerId>, StorageError> {
            self.inner.lookup_by_content_hash(content_hash)
        }
    }

    /// Build a trivial root layer to hand to the persister. The
    /// layer's content is irrelevant — the persister only invokes
    /// `backend.store_layer(&layer)` and inspects the result.
    fn build_trivial_layer() -> Arc<Layer> {
        let storage = LayerStorage::in_memory();
        let builder = LayerBuilder::new("trivial", None);
        Arc::new(builder.build(storage))
    }

    /// Hole 6 — backend failure surfaces as `ValidationError` carrying
    /// the original backend message under
    /// `ValidationRule::InstitutionValidation` (the documented Phase B
    /// stand-in).
    #[test]
    fn backend_store_persister_returns_validation_error_on_store_failure() {
        let backend = FailingStoreBackend::new();
        let persister = BackendStorePersister { backend: &backend };
        let layer = build_trivial_layer();

        let result = persister.persist("main", &layer);
        let err = result.expect_err("store_layer Err must surface");

        assert!(matches!(err.rule, ValidationRule::InstitutionValidation));
        assert!(
            err.message.contains("synthetic store_layer failure"),
            "error message must carry the underlying backend message verbatim; \
             got `{}`",
            err.message
        );
        assert!(
            err.message.starts_with("persist_layer failed:"),
            "message must carry the documented `persist_layer failed:` prefix; \
             got `{}`",
            err.message
        );
    }

    /// Happy path: confirm the PersistedLayerInfo shape documented for
    /// the no-CAS lattice path.
    #[test]
    fn backend_store_persister_returns_no_branch_advanced() {
        let backend = MemoryPersistentBackend::new();
        let persister = BackendStorePersister { backend: &backend };
        let layer = build_trivial_layer();

        let info = persister
            .persist("main", &layer)
            .expect("happy path returns Ok");

        assert_eq!(info.layer_id, *layer.id());
        assert!(
            !info.branch_advanced,
            "no CAS in the lattice path; branch_advanced is always false"
        );
        assert!(
            info.merge_outcome.is_none(),
            "merge_outcome is None when no CAS attempted"
        );
        assert!(
            !info.cache_hit_different_position,
            "no anchored-commit probe in the simple lattice path"
        );
    }
}
