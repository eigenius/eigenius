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

//! `BackendPersister` — server-quality [`LayerPersister`] backed by a
//! [`crate::storage::PersistentBackend`]. Owns the anchored-commit cache
//! probe (D33 §6) + the branch-CAS dispatch (D34 §G.1 / lattice).
//!
//! Carved out of `EigeniusService` so persistence is not coupled to the
//! gRPC service struct. The body is testable in isolation against any
//! `PersistentBackend` (notably `MemoryPersistentBackend`); the gRPC
//! service holds an `Arc<BackendPersister>` and passes `&*persister` to
//! orchestrator runs.
//!
//! Distinct from [`crate::commit::BackendStorePersister`] (the
//! lattice-side adapter) by design: that one is `store_layer`-only —
//! no cache, no CAS — and serves callers (CLI, GC tests, bootstrap)
//! that don't have branch semantics. `BackendPersister` here is the
//! full server-side path. Two types, distinct concerns; see D41 Phase F
//! discussion.

use std::sync::Arc;

use crate::commit::persister::{LayerPersister, PersistedLayerInfo};
use crate::layer::{Layer, LayerId, LayerStorage};
use crate::observability::{field, operation};
use crate::storage::PersistentBackend;
use crate::validation::{ValidationError, ValidationRule};

/// Server-quality [`LayerPersister`].
///
/// Holds an `Option<Arc<dyn PersistentBackend>>` because the kernel
/// supports an in-memory mode (`EigeniusService::new()` with no
/// backend). When `backend` is `None`, `persist` short-circuits with
/// `branch_advanced = true` — `ctx.head` is the session's only state
/// in that mode and the orchestrator must advance to the
/// freshly-built layer; returning `false` would leave `ctx.head` at
/// bootstrap and silently drop every committed resource. See the
/// commentary in `persist`'s no-backend arm for the full rationale.
pub struct BackendPersister {
    backend: Option<Arc<dyn PersistentBackend>>,
}

impl BackendPersister {
    /// Construct a persister bound to `backend`. `None` selects the
    /// in-memory branch (see struct docs).
    pub fn new(backend: Option<Arc<dyn PersistentBackend>>) -> Self {
        Self { backend }
    }

    /// Borrow the underlying backend handle, if any. Useful for code
    /// paths that need direct backend access alongside the persister
    /// (cache reads, branch listing, etc.).
    pub fn backend(&self) -> Option<&Arc<dyn PersistentBackend>> {
        self.backend.as_ref()
    }

    /// Compute the anchored-commit cache key for `layer` and probe
    /// the backend. Returns `None` when the layer has no supporting
    /// layer (root / self-referential) or when no cache entry exists.
    /// Verifies the cached layer is still in storage before returning
    /// — a stale entry (cached layer was reclaimed by GC) is treated
    /// as a cache miss.
    fn probe_anchored_commit(
        &self,
        backend: &dyn PersistentBackend,
        layer: &Layer,
    ) -> Option<LayerId> {
        let supporting_id = layer.supporting_layer()?;
        let supporting_handle = backend.load_handle(supporting_id).ok().flatten()?;
        let cached_id = backend
            .lookup_anchored_commit(layer.content_hash(), &supporting_handle.content_hash)
            .ok()
            .flatten()?;
        // Verify the cached layer still exists. If GC has reclaimed
        // it (or it was never persisted for some reason), treat as a
        // miss so the caller re-persists.
        backend.load_handle(&cached_id).ok().flatten()?;
        Some(cached_id)
    }

    /// Insert the freshly-committed layer into the anchored-commit
    /// cache. Best-effort — failures log a warning but don't propagate.
    fn put_anchored_commit_for_layer(&self, backend: &dyn PersistentBackend, layer: &Layer) {
        let Some(supporting_id) = layer.supporting_layer() else {
            return;
        };
        let Some(supporting_handle) = backend.load_handle(supporting_id).ok().flatten() else {
            return;
        };
        if let Err(e) = backend.put_anchored_commit(
            layer.content_hash(),
            &supporting_handle.content_hash,
            layer.id(),
        ) {
            tracing::warn!(
                { field::OPERATION } = operation::LAYER_COMMIT,
                { field::ERROR_KIND } = "anchored_commit_cache_put_failed",
                { field::LAYER_ID } = %layer.id(),
                { field::ERROR_MESSAGE } = %e,
                "failed to update anchored-commit cache (commit succeeded)"
            );
        }
    }

    /// Advance `branch` to `layer` via the lattice's CAS primitive.
    /// Shared by both the cache-miss path and the same-position
    /// cache-hit path inside `persist`.
    ///
    /// Returns the lattice's [`crate::lattice::UpdateOutcome`]
    /// verbatim so the caller can distinguish `FastForward` /
    /// `TrivialMerge` / `NeedsWitnessedMerge` and correctly compute
    /// `branch_advanced` — `NeedsWitnessedMerge` means the branch did
    /// **not** advance (the layer is stored but unreachable from any
    /// branch ref). Pre-D34 §G.1 this swallowed all `Ok` variants as
    /// `Ok(())`, masking the `NeedsWitnessedMerge` failure as success.
    fn advance_branch_for_layer(
        &self,
        branch: &str,
        layer: &Layer,
        backend: &dyn PersistentBackend,
    ) -> Result<crate::lattice::UpdateOutcome, ValidationError> {
        let expected_old = layer.parent().map(|p| p.id().clone());
        let storage = LayerStorage::with_persistent(
            self.backend
                .as_ref()
                .expect("advance_branch_for_layer called only when backend is Some")
                .clone(),
        );
        match crate::lattice::update_branch(
            branch,
            expected_old,
            layer.id().clone(),
            crate::lattice::ConflictPolicy::AllowTrivial,
            storage,
            backend,
        ) {
            Ok(outcome) => {
                tracing::debug!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::LAYER_ID } = %layer.id(),
                    branch = branch,
                    outcome = ?outcome,
                    "branch CAS attempted"
                );
                Ok(outcome)
            }
            Err(e) => {
                tracing::warn!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::ERROR_KIND } = "branch_update_failed",
                    { field::LAYER_ID } = %layer.id(),
                    branch = branch,
                    { field::ERROR_MESSAGE } = %e,
                    "failed to advance branch"
                );
                Err(ValidationError {
                    resource_id: None,
                    property: None,
                    rule: ValidationRule::InstitutionValidation,
                    message: format!("advance_branch failed: {e}"),
                })
            }
        }
    }
}

impl LayerPersister for BackendPersister {
    /// A positive anchored-commit probe means this exact `(content, supporting content)`
    /// was already committed-and-validated, so the pipeline can skip revalidation
    /// (D33 §6). A no-backend persister or a miss returns `false` (full validation).
    fn already_validated(&self, layer: &Layer) -> bool {
        let Some(backend) = self.backend.as_ref() else {
            return false;
        };
        self.probe_anchored_commit(backend.as_ref(), layer)
            .is_some()
    }

    fn persist(
        &self,
        branch: &str,
        layer: &Arc<Layer>,
    ) -> Result<PersistedLayerInfo, ValidationError> {
        let layer = layer.as_ref();
        let Some(backend) = self.backend.as_ref() else {
            // No persistent backend — the layer lives in-memory only.
            // There is no durable branch ref to advance and no CAS
            // attempted (merge_outcome = None), but `ctx.head` IS the
            // session's source of truth in this mode, so the
            // orchestrator must advance to the freshly-built layer.
            // Returning `branch_advanced = false` here would tell
            // `CommitOrchestrator::run` to leave `ctx.head` at the
            // bootstrap, silently dropping every committed resource
            // from session reads (see kernel/tests/server_integration.rs
            // `load_and_query`). The field's contract is "should
            // `ctx.head` advance to this layer?" — in no-backend mode
            // the answer is yes.
            return Ok(PersistedLayerInfo {
                layer_id: layer.id().clone(),
                branch_advanced: true,
                merge_outcome: None,
                cache_hit_different_position: false,
            });
        };

        // Cache probe. The cache key is the layer's content_hash and
        // the supporting layer's content_hash. Layers with no
        // supporting layer (roots, pure self-referential commits) can't
        // be keyed and fall through to the standard persist path.
        let cache_hit = self.probe_anchored_commit(backend.as_ref(), layer);

        if let Some(cached_id) = cache_hit {
            if cached_id == *layer.id() {
                // Same-position cache hit — the layer is already on
                // disk. Skip `store_layer`; still attempt the branch
                // CAS (the caller wanted to publish on top of the
                // current head, which is the layer's parent). The CAS
                // may still race or conflict, so the outcome is the
                // full taxonomy.
                tracing::debug!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::LAYER_ID } = %layer.id(),
                    branch = branch,
                    cache = "hit_same_position",
                    "anchored-commit cache hit (same position) — skipping store_layer"
                );
                let outcome = self.advance_branch_for_layer(branch, layer, backend.as_ref())?;
                let branch_advanced = !matches!(
                    outcome,
                    crate::lattice::UpdateOutcome::NeedsWitnessedMerge { .. }
                );
                return Ok(PersistedLayerInfo {
                    layer_id: layer.id().clone(),
                    branch_advanced,
                    merge_outcome: Some(outcome),
                    cache_hit_different_position: false,
                });
            }
            // Different-position cache hit — the canonical layer has a different parent
            // chain. Only treat it as "branch unchanged" (D33 §6 supporting-equivalent
            // context) if that canonical layer is actually **reachable on this branch**
            // — i.e. it's an ancestor of the current head, so its content is already
            // visible here. If it lives on a *different* chain (a fork or another
            // branch), reusing it and leaving the branch unchanged would silently drop
            // the content from this branch: a content-equivalent layer existing
            // somewhere is not the same as it being committed here. In that case fall
            // through to the standard store + advance so the freshly-built layer is
            // published onto this branch.
            let reachable_here = layer
                .parent()
                .map(|p| crate::layer::collect_ancestors(p.as_ref()).contains(&cached_id))
                .unwrap_or(false);
            if reachable_here {
                tracing::debug!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::LAYER_ID } = %layer.id(),
                    cached_layer = %cached_id,
                    branch = branch,
                    cache = "hit_different_position_reachable",
                    "anchored-commit cache hit (different position, reachable) — branch unchanged"
                );
                return Ok(PersistedLayerInfo {
                    layer_id: cached_id,
                    branch_advanced: false,
                    merge_outcome: None,
                    cache_hit_different_position: true,
                });
            }
            tracing::debug!(
                { field::OPERATION } = operation::LAYER_COMMIT,
                { field::LAYER_ID } = %layer.id(),
                cached_layer = %cached_id,
                branch = branch,
                cache = "hit_different_position_off_branch",
                "anchored-commit equivalent is off-branch — committing onto this branch"
            );
            // fall through to the standard store + advance path below.
        }

        // Cache miss — standard persist path.
        if let Err(e) = backend.store_layer(layer) {
            tracing::warn!(
                { field::OPERATION } = operation::LAYER_COMMIT,
                { field::ERROR_KIND } = "persist_layer_failed",
                { field::LAYER_ID } = %layer.id(),
                { field::ERROR_MESSAGE } = %e,
                "failed to persist layer to backend"
            );
            return Err(ValidationError {
                resource_id: None,
                property: None,
                rule: ValidationRule::InstitutionValidation,
                message: format!("persist_layer failed: {e}"),
            });
        }

        // Insert into the anchored-commit cache for future short-circuit
        // (D33 §6). Best-effort: a failure here doesn't fail the
        // commit, but we log it so chain audits can spot drift between
        // the cache and the topology.
        self.put_anchored_commit_for_layer(backend.as_ref(), layer);

        // Attempt the CAS. On `NeedsWitnessedMerge` the layer is on
        // disk but not reachable from any branch ref — the fix for
        // D34 §G.1's silent-success bug is reporting branch_advanced
        // = false here so clients know to recover.
        let outcome = self.advance_branch_for_layer(branch, layer, backend.as_ref())?;
        let branch_advanced = !matches!(
            outcome,
            crate::lattice::UpdateOutcome::NeedsWitnessedMerge { .. }
        );
        Ok(PersistedLayerInfo {
            layer_id: layer.id().clone(),
            branch_advanced,
            merge_outcome: Some(outcome),
            cache_hit_different_position: false,
        })
    }
}

#[cfg(test)]
mod already_validated_tests {
    //! Task 2 — the anchored-commit revalidation skip seam. A `(content, supporting
    //! content)` already recorded in the anchored-commit cache is proven valid, so
    //! `already_validated` returns true and the commit pipeline skips re-running
    //! structural + retroactive validation on an identical re-commit.

    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};
    use crate::storage::memory::MemoryPersistentBackend;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn res_of(id: &str, class: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(iri(class))]),
        );
        r
    }

    #[test]
    fn already_validated_true_only_for_cached_content_and_support() {
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));

        // Root R defines a class the child depends on, so the child has a *supporting
        // layer* (the anchored-commit key is keyed on it).
        let mut rb = LayerBuilder::new("root", None);
        rb.add_resource(res_of("urn:eigenius:demo:Kind", "urn:eigenius:core:Class"))
            .unwrap();
        let root = Arc::new(rb.build(storage.clone()));
        backend.store_layer(&root).unwrap();

        // Child C is_a demo:Kind (defined in R) ⇒ its supporting layer is R.
        let mut cb = LayerBuilder::new("child", Some(Arc::clone(&root)));
        cb.add_resource(res_of("urn:eigenius:demo:C", "urn:eigenius:demo:Kind"))
            .unwrap();
        let child = Arc::new(cb.build(storage.clone()));
        backend.store_layer(&child).unwrap();
        assert_eq!(child.supporting_layer(), Some(root.id()));

        let persister = BackendPersister::new(Some(Arc::clone(&backend)));

        // No cache entry yet ⇒ must revalidate.
        assert!(
            !persister.already_validated(&child),
            "no anchored-commit entry ⇒ not yet proven valid"
        );

        // Record the anchored-commit entry a successful prior commit would write.
        let support_hash = backend
            .load_handle(root.id())
            .unwrap()
            .unwrap()
            .content_hash;
        backend
            .put_anchored_commit(child.content_hash(), &support_hash, child.id())
            .unwrap();

        // Identical (content, support) ⇒ proven valid ⇒ skip revalidation.
        assert!(
            persister.already_validated(&child),
            "cached (content, support) ⇒ already validated"
        );

        // Different content (uncached) ⇒ must revalidate.
        let mut cb2 = LayerBuilder::new("child2", Some(Arc::clone(&root)));
        cb2.add_resource(res_of("urn:eigenius:demo:Other", "urn:eigenius:demo:Kind"))
            .unwrap();
        let child2 = Arc::new(cb2.build(storage.clone()));
        assert!(
            !persister.already_validated(&child2),
            "uncached content ⇒ must revalidate"
        );

        // A root (no supporting layer) is never short-circuited.
        assert!(
            !persister.already_validated(&root),
            "a layer with no supporting layer is never skipped"
        );
    }

    #[test]
    fn already_validated_false_without_backend() {
        let persister = BackendPersister::new(None);
        let storage = LayerStorage::in_memory();
        let mut rb = LayerBuilder::new("root", None);
        rb.add_resource(res_of("urn:eigenius:demo:R", "urn:eigenius:core:Class"))
            .unwrap();
        let root = Arc::new(rb.build(storage));
        assert!(
            !persister.already_validated(&root),
            "no backend ⇒ no cache ⇒ always revalidate"
        );
    }

    /// The anchored-commit different-position dedup must NOT suppress a commit when the
    /// content-equivalent canonical layer lives on a *different chain* (not reachable
    /// from the branch being committed to). Otherwise the content is silently dropped
    /// from this branch. Here a layer `A` (content X, supporting R) sits on the `R→M→A`
    /// chain; committing the same content `B` (supporting R) onto a branch whose head is
    /// `R` must publish `B` and advance the branch — not dedup to the off-branch `A`.
    #[test]
    fn off_branch_equivalent_does_not_suppress_commit() {
        let backend: Arc<dyn PersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));

        // R defines Kind.
        let mut rb = LayerBuilder::new("root", None);
        rb.add_resource(res_of("urn:eigenius:demo:Kind", "urn:eigenius:core:Class"))
            .unwrap();
        let root = Arc::new(rb.build(storage.clone()));
        backend.store_layer(&root).unwrap();

        // M on R — an intervening layer, so A (on M) and B (on R) get different parents.
        let mut mb = LayerBuilder::new("mid", Some(Arc::clone(&root)));
        mb.add_resource(res_of("urn:eigenius:demo:M", "urn:eigenius:demo:Kind"))
            .unwrap();
        let mid = Arc::new(mb.build(storage.clone()));
        backend.store_layer(&mid).unwrap();

        // Same content X (is_a Kind ⇒ supporting layer R) on two different parents:
        // A on M, B on R ⇒ same content_hash + same supporting content, different position.
        let x = || res_of("urn:eigenius:demo:X", "urn:eigenius:demo:Kind");
        let a = {
            let mut b = LayerBuilder::new("A", Some(Arc::clone(&mid)));
            b.add_resource(x()).unwrap();
            Arc::new(b.build(storage.clone()))
        };
        backend.store_layer(&a).unwrap();
        let b_layer = {
            let mut b = LayerBuilder::new("B", Some(Arc::clone(&root)));
            b.add_resource(x()).unwrap();
            Arc::new(b.build(storage.clone()))
        };
        backend.store_layer(&b_layer).unwrap();

        assert_eq!(a.content_hash(), b_layer.content_hash(), "same content");
        assert_ne!(
            a.id(),
            b_layer.id(),
            "different position (different parent)"
        );
        assert_eq!(a.supporting_layer(), Some(root.id()));
        assert_eq!(b_layer.supporting_layer(), Some(root.id()));

        // Seed the anchored-commit cache so probe(B) finds A (the off-branch equivalent).
        let r_hash = backend
            .load_handle(root.id())
            .unwrap()
            .unwrap()
            .content_hash;
        backend
            .put_anchored_commit(a.content_hash(), &r_hash, a.id())
            .unwrap();

        // `main` is at R; A (on the R→M→A chain) is NOT reachable from R.
        backend.put_branch("main", root.id()).unwrap();

        let persister = BackendPersister::new(Some(Arc::clone(&backend)));
        let info = persister.persist("main", &b_layer).expect("persist B");

        assert!(
            !info.cache_hit_different_position,
            "must NOT dedup to the off-branch equivalent"
        );
        assert_eq!(
            info.layer_id,
            *b_layer.id(),
            "B itself is committed, not the cached off-branch A"
        );
        assert!(
            info.branch_advanced,
            "the branch must advance so the content lands here"
        );
        assert_eq!(
            backend.get_branch("main").unwrap().as_ref(),
            Some(b_layer.id()),
            "main advanced to B"
        );
    }
}
