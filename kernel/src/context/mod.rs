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
//! uncommitted resources. On commit, the working layer is built, validated,
//! and becomes the new head.

use crate::layer::{
    BloomCache, Layer, LayerBuilder, LayerError, LayerId, LayerStorage, ResourceCache,
};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::storage::ResourceBackend;
use crate::validation::Validator;
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
#[derive(Debug)]
pub enum ContextError {
    /// Attempted a write operation in read-only mode.
    ReadOnly,
    /// Layer building error.
    Layer(LayerError),
    /// Validation failed on commit.
    ValidationFailed(Vec<crate::validation::ValidationError>),
    /// Head has moved since this context was created (conflict).
    StaleHead { expected: LayerId, actual: LayerId },
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextError::ReadOnly => write!(f, "cannot modify in read-only mode"),
            ContextError::Layer(e) => write!(f, "layer error: {e}"),
            ContextError::ValidationFailed(errors) => {
                writeln!(f, "validation failed with {} error(s):", errors.len())?;
                for e in errors {
                    writeln!(f, "  {e}")?;
                }
                Ok(())
            }
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

    /// Returns true if the working layer has any resources.
    pub fn has_changes(&self) -> bool {
        !self.working.resources().is_empty()
    }

    /// Commit the working layer.
    ///
    /// Builds the working layer, validates it against the chain,
    /// updates head to the new layer, and starts a fresh working layer.
    ///
    /// Returns the new layer (which is now the head).
    pub fn commit(&mut self, name: &str) -> Result<Arc<Layer>, ContextError> {
        if self.mode == ExecutionMode::ReadOnly {
            return Err(ContextError::ReadOnly);
        }

        // Build the layer using the context's shared cache + backend.
        let working = std::mem::replace(
            &mut self.working,
            LayerBuilder::new(name, Some(Arc::clone(&self.head))),
        );
        let new_layer = Arc::new(working.build(self.storage.clone()));

        // Validate the new layer
        let validator = Validator::new(&new_layer);
        let errors = validator.validate();
        if !errors.is_empty() {
            // Validation failed — restore working layer state
            // (the resources are lost since we consumed the builder,
            // but that's acceptable — the caller should fix and retry)
            return Err(ContextError::ValidationFailed(errors));
        }

        // Update head
        self.head = Arc::clone(&new_layer);

        // Reset working layer to point to new head
        self.working = LayerBuilder::new(name, Some(Arc::clone(&self.head)));

        Ok(new_layer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::eigon_json;
    use crate::ontology::resource::Value;
    use crate::ontology::well_known as wk;

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
    fn read_only_rejects_commit() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadOnly, storage);
        assert!(matches!(ctx.commit("test"), Err(ContextError::ReadOnly)));
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
    fn commit_valid_resource() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx =
            ExecutionContext::new(core.clone(), "test", ExecutionMode::ReadWrite, storage);

        // Add a valid Property resource
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

        let new_layer = ctx.commit("next").unwrap();
        assert!(!new_layer.is_root());
        // Head should now be the new layer
        assert_eq!(ctx.head().id(), new_layer.id());
        // Should still be able to resolve core resources through the chain
        assert!(ctx.resolve(&iri("urn:eigenius:core:Class")).is_some());
        // And the newly committed resource
        assert!(ctx.resolve(&iri("urn:eigenius:test:my_prop")).is_some());
    }

    #[test]
    fn commit_invalid_resource_fails() {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut ctx = ExecutionContext::new(core, "test", ExecutionMode::ReadWrite, storage);

        // Add a Property missing required 'data_type'
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

        assert!(matches!(
            ctx.commit("next"),
            Err(ContextError::ValidationFailed(_))
        ));
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
