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

//! Execution context for snapshot isolation and read/write control.
//!
//! An `ExecutionContext` holds a reference to the current head layer
//! (the top of the committed chain) and a `LayerBuilder` for accumulating
//! uncommitted resources. Resources are staged on the working layer via
//! [`ExecutionContext::add_resource`]; commits go through the
//! [`crate::commit::CommitOrchestrator`] (or the simpler
//! [`crate::lattice::commit_layer_default`] helper for callers that don't
//! need orchestrator features). The handler shape is:
//!
//! 1. [`take_working`](ExecutionContext::take_working) — extract the
//!    accumulated `LayerBuilder` and reset the context to a fresh one.
//! 2. Hand the builder to the pipeline / orchestrator.
//! 3. [`advance_head`](ExecutionContext::advance_head) on success, or
//!    [`revert_head`](ExecutionContext::revert_head) if the persist
//!    didn't advance the branch (D33 §6 cache hit, D34 §G.1 witnessed
//!    merge).
//!
//! D41 Phase G removed the pre-pipeline `commit` / `commit_with_validation`
//! methods that previously bundled build + validate + persist inside
//! the context; that work now lives in the `commit::` module.

use crate::layer::{
    BloomCache, Layer, LayerBuilder, LayerError, LayerId, LayerStorage, ResourceCache,
};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::storage::ResourceBackend;
use std::fmt;
use std::sync::Arc;

/// Execution mode determining allowed operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Read-only: resolve resources but cannot add or commit.
    ReadOnly,
    /// Read-write: can add resources and commit layers.
    ReadWrite,
}

/// Errors that can occur during context operations.
///
/// D41 Phase G removed the legacy `ValidationFailed { ... }` variant
/// that previously carried structural / AutoOnLoad rejection
/// information. Validation failures are now produced by the commit
/// pipeline as [`crate::lattice::CommitError::Validation`] and reach
/// callers through the orchestrator's
/// [`crate::commit::MultiLayerOutcome::error`] field; the context never
/// has to model them.
#[derive(Debug)]
pub enum ContextError {
    /// Attempted a write operation in read-only mode.
    ReadOnly,
    /// Layer building error.
    Layer(LayerError),
    /// Head has moved since this context was created (conflict).
    StaleHead { expected: LayerId, actual: LayerId },
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextError::ReadOnly => write!(f, "cannot modify in read-only mode"),
            ContextError::Layer(e) => write!(f, "layer error: {e}"),
            ContextError::StaleHead { expected, actual } => {
                write!(
                    f,
                    "stale head: expected {expected}, actual {actual} — another commit occurred"
                )
            }
        }
    }
}

impl std::error::Error for ContextError {}

/// An execution context binding a layer chain snapshot with a working layer.
///
/// The context provides resolution (checking working layer first, then the
/// committed chain) and controlled mutation (add resources, then commit).
///
/// **Phase 14a-iii**: holds a shared `LayerStorage` bundle that flows into
/// every `LayerBuilder::build` call so all committed layers share the same
/// caches and backing store.
pub struct ExecutionContext {
    /// The topmost committed layer.
    head: Arc<Layer>,
    /// Uncommitted resources being accumulated.
    working: LayerBuilder,
    /// Read-only or read-write.
    mode: ExecutionMode,
    /// Shared storage handles. Cloned cheaply on commit and forwarded to
    /// `LayerBuilder::build`.
    storage: LayerStorage,
}

impl ExecutionContext {
    /// Create a new execution context.
    ///
    /// `head` is the topmost committed layer. `name` is the name for the
    /// working layer being built. `storage` is the shared bundle every
    /// committed layer in this context will use.
    pub fn new(head: Arc<Layer>, name: &str, mode: ExecutionMode, storage: LayerStorage) -> Self {
        let working = LayerBuilder::new(name, Some(Arc::clone(&head)));
        Self {
            head,
            working,
            mode,
            storage,
        }
    }

    /// Returns the current head layer.
    pub fn head(&self) -> &Arc<Layer> {
        &self.head
    }

    /// Returns the execution mode.
    pub fn mode(&self) -> ExecutionMode {
        self.mode
    }

    /// Returns the bundled `LayerStorage` (cache + backend + bloom cache).
    pub fn storage(&self) -> &LayerStorage {
        &self.storage
    }

    /// Returns the shared resource cache.
    pub fn cache(&self) -> &Arc<dyn ResourceCache> {
        &self.storage.cache
    }

    /// Returns the shared resource backend.
    pub fn backend(&self) -> &Arc<dyn ResourceBackend> {
        &self.storage.backend
    }

    /// Returns the shared bloom cache.
    pub fn bloom_cache(&self) -> &Arc<dyn BloomCache> {
        &self.storage.bloom_cache
    }

    /// Resolve a resource by IRI.
    ///
    /// Checks the working layer first, then walks the committed chain.
    /// Returns an owned `Arc<Resource>` because cache-backed lookups can't
    /// hand out borrowed references that outlive the cache state.
    pub fn resolve(&self, iri: &Iri) -> Option<Arc<Resource>> {
        // Check working layer first (still in-builder, holds Resource by value)
        if let Some(r) = self.working.get_resource(iri) {
            return Some(Arc::new(r.clone()));
        }
        // Then walk the committed chain (cache-backed)
        self.head.resolve(iri)
    }

    /// Add a resource to the working layer.
    ///
    /// Fails if the context is read-only or if the resource violates
    /// namespace protection.
    pub fn add_resource(&mut self, resource: Resource) -> Result<(), ContextError> {
        if self.mode == ExecutionMode::ReadOnly {
            return Err(ContextError::ReadOnly);
        }
        self.working
            .add_resource(resource)
            .map_err(ContextError::Layer)
    }

    /// Mark `iri` as tombstoned in the working layer.
    ///
    /// Used by commit-shaped RPC handlers that accept caller-supplied
    /// explicit tombstones (D41 §10.1): the IRIs flow into the working
    /// builder before [`Self::take_working`] hands it off to the
    /// commit orchestrator's root [`crate::commit::LayerEmission`].
    /// The orchestrator may then combine them with cascade-inferred
    /// tombstones under `CommitPolicy::CascadeTombstone`.
    ///
    /// Fails if the context is read-only, if the IRI is in the core
    /// namespace, or if the working layer already defines a resource
    /// for the same IRI (the layer can't simultaneously declare and
    /// suppress an IRI; [`crate::layer::LayerBuilder::tombstone`]
    /// owns the policy).
    pub fn tombstone(&mut self, iri: Iri) -> Result<(), ContextError> {
        if self.mode == ExecutionMode::ReadOnly {
            return Err(ContextError::ReadOnly);
        }
        self.working.tombstone(iri).map_err(ContextError::Layer)
    }

    /// Returns true if the working layer has any resources.
    pub fn has_changes(&self) -> bool {
        !self.working.resources().is_empty()
    }

    /// Restore `head` to a prior layer and reset the working builder
    /// to attach to it. Callers use this when a commit returned a
    /// [`CommitOutcome`] but the subsequent persist short-circuited
    /// (different-position cache hit per D33 §6) or lost the CAS
    /// (NeedsWitnessedMerge per D34 §G.1) — the in-memory `head`
    /// would otherwise point at a layer not present in storage,
    /// poisoning every later commit's LCA walk with
    /// "merge during update_branch: no common ancestor".
    pub fn revert_head(&mut self, prior_head: Arc<Layer>, name: &str) {
        self.head = prior_head;
        self.working = LayerBuilder::new(name, Some(Arc::clone(&self.head)));
    }

    /// Install `layer` as the new `head` and reset the working builder
    /// to attach to it. Callers use this from the commit orchestrator
    /// (D41 §6) once a pipeline's `persist` phase set
    /// `branch_advanced = true`. Read-only contexts reject; the
    /// caller should have built a write context for the commit-shaped
    /// RPC.
    ///
    /// D41 §9.
    pub fn advance_head(&mut self, layer: Arc<Layer>, name: &str) -> Result<(), ContextError> {
        if self.mode == ExecutionMode::ReadOnly {
            return Err(ContextError::ReadOnly);
        }
        self.head = layer;
        self.working = LayerBuilder::new(name, Some(Arc::clone(&self.head)));
        Ok(())
    }

    /// Consume the working builder and replace it with a fresh, empty
    /// one parented at `ctx.head()` and named `fresh_name`. Returns the
    /// old builder by value so the caller can route its accumulated
    /// resources / tombstones through the commit pipeline.
    ///
    /// Used by the Load handler (and other commit-shaped RPC handlers
    /// once Phase F migrates them) to hand the accumulated working
    /// builder off to the [`crate::commit::CommitOrchestrator`] as the
    /// root [`crate::commit::LayerEmission`]. The orchestrator then
    /// constructs its own builders per emission; this method makes the
    /// transition explicit at the handler boundary.
    ///
    /// Read-only contexts reject — taking the working builder is a
    /// write operation by intent, even though the working builder is
    /// always present (the handler that called `take_working` was
    /// going to commit something, and the orchestrator that consumes
    /// the returned builder needs `ctx` to be writeable for the
    /// subsequent `advance_head` calls).
    ///
    /// D41 §9.
    pub fn take_working(&mut self, fresh_name: &str) -> Result<LayerBuilder, ContextError> {
        if self.mode == ExecutionMode::ReadOnly {
            return Err(ContextError::ReadOnly);
        }
        let parent = Arc::clone(&self.head);
        Ok(std::mem::replace(
            &mut self.working,
            LayerBuilder::new(fresh_name, Some(parent)),
        ))
    }
}

#[cfg(test)]
mod tests {
    //! D41 Phase G — context tests cover the surviving
    //! [`ExecutionContext`] surface only. The committed pipeline
    //! (build, structural validate, AutoOnLoad dispatch, persist, and
    //! revert bookkeeping) is exercised in `commit::phases::tests`,
    //! `commit::orchestrator::tests`, and the storage E2E suite. The
    //! pre-D41 `commit` / `commit_with_validation` integration tests
    //! here are gone alongside the methods themselves.
    //!
    //! What remains are unit tests on:
    //! - read-only mode rejection (`add_resource`, `take_working`,
    //!   `advance_head`),
    //! - resolution precedence (working layer before chain),
    //! - the working / head interop (`take_working`, `advance_head`,
    //!   `revert_head`),
    //! - `has_changes`.
    //!
    //! Tests that need a full validated commit lean on
    //! [`crate::lattice::commit_layer_default`] (the D41 supported
    //! single-layer-commit surface) over a
    //! [`crate::storage::memory::MemoryPersistentBackend`].
    use super::*;
    use crate::lattice::commit_layer_default;
    use crate::layer::LayerBuilder;
    use crate::ontology::eigon_json;
    use crate::ontology::resource::Value;
    use crate::ontology::well_known as wk;
    use crate::storage::memory::MemoryPersistentBackend;
    use crate::storage::PersistentBackend;

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

    fn test_storage() -> LayerStorage {
        LayerStorage::in_memory()
    }

    /// Memory-backed [`LayerStorage`] paired with the [`PersistentBackend`]
    /// that drives it. Used by tests that need to route a commit through
    /// [`commit_layer_default`] (the D41 single-layer-commit surface).
    fn test_storage_with_backend() -> (LayerStorage, Arc<MemoryPersistentBackend>) {
        let backend = Arc::new(MemoryPersistentBackend::new());
        let storage =
            LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
        (storage, backend)
    }

    fn build_core_layer(storage: LayerStorage) -> Arc<Layer> {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in resources {
            builder.add_resource(r).unwrap();
        }
        Arc::new(builder.build(storage))
    }

    #[test]
    fn read_only_rejects_add() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadOnly, storage);
        let r = make_resource("urn:eigenius:test:foo", vec![]);
        assert!(matches!(ctx.add_resource(r), Err(ContextError::ReadOnly)));
    }

    #[test]
    fn read_only_rejects_take_working() {
        // `take_working` is the D41 pipeline-interop replacement for
        // `commit` (Phase E) and inherits the same ReadOnly gate.
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadOnly, storage);
        assert!(matches!(
            ctx.take_working("test"),
            Err(ContextError::ReadOnly)
        ));
    }

    #[test]
    fn read_only_rejects_advance_head() {
        // `advance_head` is the orchestrator's head-promotion call
        // (Phase D) and inherits the same ReadOnly gate as `commit` had.
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx =
            ExecutionContext::new(Arc::clone(&core), "test", ExecutionMode::ReadOnly, storage);
        assert!(matches!(
            ctx.advance_head(core, "test"),
            Err(ContextError::ReadOnly)
        ));
    }

    #[test]
    fn resolve_from_head() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadOnly, storage);
        // Should resolve core ontology resources
        assert!(ctx.resolve(&iri("urn:eigenius:core:Class")).is_some());
        assert!(ctx.resolve(&iri("urn:eigenius:core:is_a")).is_some());
    }

    #[test]
    fn resolve_working_layer_first() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadWrite, storage);

        let r = make_resource(
            "urn:eigenius:test:foo",
            vec![(
                "urn:eigenius:core:description",
                Value::String("hello".into()),
            )],
        );
        ctx.add_resource(r).unwrap();

        let resolved = ctx.resolve(&iri("urn:eigenius:test:foo")).unwrap();
        let desc = resolved.get(&iri("urn:eigenius:core:description")).unwrap();
        assert_eq!(desc.as_str(), Some("hello"));
    }

    #[test]
    fn take_working_advance_head_lands_valid_resource() {
        // Smoke test on the pipeline-interop pair: stage a valid
        // resource, take the working builder, route through
        // `commit_layer_default`, advance head. Mirrors the shape of
        // the pre-D41 `commit_valid_resource` test but uses the D41
        // surfaces directly.
        let (storage, backend) = test_storage_with_backend();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadWrite, storage);

        ctx.add_resource(make_resource(
            "urn:eigenius:test:my_prop",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                ),
                (wk::DESCRIPTION, Value::String("A test property".into())),
                (wk::SHORT_NAME, Value::String("my_prop".into())),
                (wk::DATA_TYPE_PROP, Value::String(wk::STRING.to_string())),
            ],
        ))
        .unwrap();

        let working = ctx.take_working("next").expect("take_working");
        let new_layer = commit_layer_default(working, ctx.storage().clone(), backend.as_ref())
            .expect("commit_layer_default");
        ctx.advance_head(Arc::clone(&new_layer), "next")
            .expect("advance_head");

        assert!(!new_layer.is_root());
        assert_eq!(ctx.head().id(), new_layer.id());
        assert!(ctx.resolve(&iri("urn:eigenius:core:Class")).is_some());
        assert!(ctx.resolve(&iri("urn:eigenius:test:my_prop")).is_some());
    }

    #[test]
    fn commit_layer_default_rejects_invalid_resource() {
        // Validation moved from `ExecutionContext::commit` to the
        // `commit::phases::structural_validate` phase; the surfacing
        // path is now `lattice::CommitError::Validation`.
        let (storage, backend) = test_storage_with_backend();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadWrite, storage);

        ctx.add_resource(make_resource(
            "urn:eigenius:test:bad",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                ),
                (wk::DESCRIPTION, Value::String("bad".into())),
                (wk::SHORT_NAME, Value::String("bad".into())),
                // missing data_type!
            ],
        ))
        .unwrap();

        let working = ctx.take_working("next").expect("take_working");
        let err = commit_layer_default(working, ctx.storage().clone(), backend.as_ref())
            .expect_err("missing data_type must reject");
        assert!(
            matches!(err, crate::lattice::CommitError::Validation { .. }),
            "expected CommitError::Validation, got {err:?}"
        );
    }

    #[test]
    fn has_changes() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadWrite, storage);
        assert!(!ctx.has_changes());
        ctx.add_resource(make_resource("urn:eigenius:test:x", vec![]))
            .unwrap();
        assert!(ctx.has_changes());
    }
}
