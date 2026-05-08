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
    /// Validation failed on commit. `provenance_layer` is `Some` when
    /// the commit was rejected by an institutional gate (D31 §6.3) but
    /// the audit-anchor `Verdict` + `RuntimeInvocation` resources still
    /// landed on a separate provenance-only layer. `None` for the plain
    /// structural-validation failure path where nothing committed.
    ValidationFailed {
        errors: Vec<crate::validation::ValidationError>,
        provenance_layer: Option<Arc<Layer>>,
    },
    /// Head has moved since this context was created (conflict).
    StaleHead { expected: LayerId, actual: LayerId },
}

impl ContextError {
    /// Convenience constructor for the "validation failed, no
    /// provenance committed" case — the structural / installer /
    /// handler-error paths.
    pub fn validation_failed(errors: Vec<crate::validation::ValidationError>) -> Self {
        Self::ValidationFailed {
            errors,
            provenance_layer: None,
        }
    }
}

/// Successful outcome of [`ExecutionContext::commit_with_validation`].
///
/// Carries both the user-authored layer (the gated subject's commit)
/// and the optional follow-up `verdict_provenance` layer ([D31 §6.3])
/// that AutoOnLoad dispatches add when at least one Holds /
/// Undecidable Verdict landed against the gated subject. Both must
/// be persisted by the caller — the provenance layer is the parent
/// of the next user commit (`ctx.head` is advanced to it), so a
/// caller that persists only `user_layer` would leave the
/// provenance layer as an in-memory ghost. The next commit would
/// reference a parent the backend doesn't know about, and
/// `update_branch`'s LCA walk would fail with
/// "merge during update_branch: no common ancestor". Persisting
/// both keeps the chain on the backend in lockstep with `ctx.head`.
#[derive(Debug, Clone)]
pub struct CommitOutcome {
    /// The committed layer carrying the user-authored resources from
    /// this batch.
    pub user_layer: Arc<Layer>,
    /// The follow-up `verdict_provenance` layer — `Some` iff at least
    /// one AutoOnLoad QueryClass on a Holds / Undecidable path
    /// produced a chain-side `Verdict` + `RuntimeInvocation` audit
    /// pair. Mirrors the Fails-path `ContextError::ValidationFailed
    /// { provenance_layer, .. }` shape so the caller's persist logic
    /// is identical across both branches.
    pub provenance_layer: Option<Arc<Layer>>,
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextError::ReadOnly => write!(f, "cannot modify in read-only mode"),
            ContextError::Layer(e) => write!(f, "layer error: {e}"),
            ContextError::ValidationFailed { errors, .. } => {
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
            return Err(ContextError::validation_failed(errors));
        }

        // Update head
        self.head = Arc::clone(&new_layer);

        // Reset working layer to point to new head
        self.working = LayerBuilder::new(name, Some(Arc::clone(&self.head)));

        Ok(new_layer)
    }

    /// Commit the working layer with D14 institution-aware validation
    /// (D14 §9.1). Same as [`commit`] but, after structural validation
    /// succeeds, runs every AutoOnLoad QueryClass declared in the
    /// chain against the new layer's resources. A QueryClass returning
    /// `Fails` aborts the commit with `ContextError::ValidationFailed`;
    /// `Holds` and `Undecidable` accept.
    ///
    /// The institution dispatch needs `&ExecutionContext` to pass to
    /// the institution's `query` handler, so the commit promotes head
    /// to the new layer *before* dispatching, then reverts head if any
    /// AutoOnLoad QueryClass rejects. This lets a QueryClass resolve
    /// freshly-loaded references via the chain.
    ///
    /// Used by the Load RPC. RunProgram-style commits stay on plain
    /// [`commit`] — those produce trusted resources and don't need
    /// institutional re-checking.
    pub fn commit_with_validation(
        &mut self,
        name: &str,
        index: &crate::institution::registry::InstitutionIndex,
        runtime: &crate::institution::runtime::InstitutionRuntime,
    ) -> Result<CommitOutcome, ContextError> {
        if self.mode == ExecutionMode::ReadOnly {
            return Err(ContextError::ReadOnly);
        }

        let working = std::mem::replace(
            &mut self.working,
            LayerBuilder::new(name, Some(Arc::clone(&self.head))),
        );
        let new_layer = Arc::new(working.build(self.storage.clone()));

        // Structural validation first — institutions assume well-formed
        // morphism resources (D14 §9.1).
        let validator = Validator::new(&new_layer);
        let errors = validator.validate();
        if !errors.is_empty() {
            return Err(ContextError::validation_failed(errors));
        }

        // D31 §5 install-time cross-check — every external Institution
        // declaration in the new layer must resolve its env_ref, image
        // digest, and per-QueryClass dispatch metadata against the
        // chain. Run before head promotion so an incomplete external
        // institution fails the Load cleanly rather than landing in
        // the chain and surfacing only at runtime.
        let new_index = crate::institution::registry::InstitutionIndex::from_layer(&new_layer).0;
        let (_, ext_errors) = crate::capability::registration::validate_external_institution_chain(
            &new_layer, &new_index,
        );
        if !ext_errors.is_empty() {
            let mapped: Vec<crate::validation::ValidationError> = ext_errors
                .into_iter()
                .map(|e| crate::validation::ValidationError {
                    resource_id: Iri::parse(&e.institution_iri).ok(),
                    property: None,
                    rule: crate::validation::ValidationRule::InstitutionValidation,
                    message: e.message,
                })
                .collect();
            return Err(ContextError::validation_failed(mapped));
        }

        // Promote head to the new layer so AutoOnLoad QueryClasses can
        // resolve cross-references freshly loaded in the same batch.
        let prior_head = std::mem::replace(&mut self.head, Arc::clone(&new_layer));
        self.working = LayerBuilder::new(name, Some(Arc::clone(&self.head)));

        // Read-only ExecutionContext over the promoted-head state.
        // Cloning self gives institution handlers a snapshot view —
        // they can call `resolve` etc. against the new chain.
        let snapshot = ExecutionContext::new(
            Arc::clone(&new_layer),
            "__validate__",
            ExecutionMode::ReadOnly,
            self.storage.clone(),
        );
        let auto_outcome = crate::institution::dispatch::dispatch_auto_on_load_for_layer(
            &new_layer, index, runtime, &snapshot,
        );

        // Handler-side errors (missing institution, malformed Verdict,
        // etc.) have no provenance — surface them as plain
        // ValidationFailed and revert head. These are bugs in the
        // institution wiring, not domain rejections.
        if !auto_outcome.errors.is_empty() {
            self.head = prior_head;
            self.working = LayerBuilder::new(name, Some(Arc::clone(&self.head)));
            return Err(ContextError::validation_failed(auto_outcome.errors));
        }

        // Build chain-side `RuntimeInvocation` + `Verdict` resources
        // for every dispatch — Holds, Fails, and Undecidable each
        // produce one of each per [D31 §6.3]. Dispatches against
        // embedded subjects (no `@id`) skip provenance entirely but
        // still apply the gate.
        let mut provenance: Vec<crate::ontology::resource::Resource> = Vec::new();
        let mut fail_errors: Vec<crate::validation::ValidationError> = Vec::new();
        for dispatch in &auto_outcome.dispatches {
            let invocation_iri = crate::institution::dispatch::allocate_invocation_iri();
            let invocation = crate::institution::dispatch::build_runtime_invocation_resource(
                dispatch,
                &invocation_iri,
                &derive_verdict_iri(&invocation_iri),
            );
            let verdict = crate::institution::dispatch::build_verdict_resource(
                dispatch,
                invocation.as_ref().map(|_| &invocation_iri),
                None,
                None,
            );
            if matches!(
                dispatch.verdict,
                crate::institution::dispatch::VerdictReading::Fails
            ) {
                let verdict_ref = verdict
                    .as_ref()
                    .and_then(|v| v.id().map(|i| i.as_str().to_string()))
                    .unwrap_or_else(|| "<embedded>".to_string());
                fail_errors.push(crate::validation::ValidationError {
                    resource_id: dispatch.subject_iri.clone(),
                    property: None,
                    rule: crate::validation::ValidationRule::InstitutionValidation,
                    message: format!(
                        "AutoOnLoad QueryClass `{}` returned Fails (Verdict `{}`)",
                        dispatch.query_class_iri, verdict_ref
                    ),
                });
            }
            if let Some(inv) = invocation {
                provenance.push(inv);
            }
            if let Some(v) = verdict {
                provenance.push(v);
            }
        }

        if !fail_errors.is_empty() {
            // Per [D31 §6.3]: gated resources are dropped on Fails,
            // but RuntimeInvocation + Verdict provenance commits as
            // the audit anchor explaining the rejection. Revert head
            // (drops the gated layer) and commit a fresh
            // provenance-only layer in its place.
            self.head = prior_head;
            self.working = LayerBuilder::new("verdict_provenance", Some(Arc::clone(&self.head)));
            for r in provenance {
                self.working.add_resource(r).map_err(ContextError::Layer)?;
            }
            let provenance_layer = if self.working.resources().is_empty() {
                None
            } else {
                let working = std::mem::replace(
                    &mut self.working,
                    LayerBuilder::new(name, Some(Arc::clone(&self.head))),
                );
                let layer = Arc::new(working.build(self.storage.clone()));
                self.head = Arc::clone(&layer);
                self.working = LayerBuilder::new(name, Some(Arc::clone(&self.head)));
                Some(layer)
            };
            return Err(ContextError::ValidationFailed {
                errors: fail_errors,
                provenance_layer,
            });
        }

        // Holds / Undecidable path: keep the gated commit and add a
        // follow-up `verdict_provenance` layer carrying the
        // RuntimeInvocation + Verdict resources. Both layers commit
        // before this method returns — that's the [D31 §6.3]
        // "transactional" guarantee from the caller's perspective.
        // The caller MUST persist both via the [`CommitOutcome`] —
        // returning only `user_layer` would leave `prov_layer` as an
        // in-memory ghost (`ctx.head` advances to it but the backend
        // never sees it), and the next commit would reference a
        // parent the backend doesn't know about, surfacing later as
        // "merge during update_branch: no common ancestor" when the
        // LCA walk fails to load the missing layer's parent chain.
        let provenance_layer = if !provenance.is_empty() {
            self.working = LayerBuilder::new("verdict_provenance", Some(Arc::clone(&self.head)));
            for r in provenance {
                self.working.add_resource(r).map_err(ContextError::Layer)?;
            }
            let working = std::mem::replace(
                &mut self.working,
                LayerBuilder::new(name, Some(Arc::clone(&self.head))),
            );
            let prov_layer = Arc::new(working.build(self.storage.clone()));
            self.head = Arc::clone(&prov_layer);
            self.working = LayerBuilder::new(name, Some(Arc::clone(&self.head)));
            Some(prov_layer)
        } else {
            None
        };

        Ok(CommitOutcome {
            user_layer: new_layer,
            provenance_layer,
        })
    }
}

/// Derive the deterministic Verdict IRI for a given RuntimeInvocation
/// per [D31 §6.3] — `urn:eigenius:invocation:<inv-id>:verdict`.
fn derive_verdict_iri(invocation_iri: &Iri) -> Iri {
    Iri::parse(&format!("{}:verdict", invocation_iri.as_str())).expect("derived Verdict IRI parses")
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
            Err(ContextError::ValidationFailed { .. })
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

    // ─── commit_with_validation (D14 M7) ───────────────────────────

    use crate::institution::error::InstitutionError;
    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::nbe::val::Val;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::Resource;

    /// Stub institution that returns `Verdict::Fails` from every
    /// query — used to confirm `commit_with_validation` aborts when
    /// AutoOnLoad rejects.
    struct AlwaysFails;
    impl Institution for AlwaysFails {
        fn institution_iri(&self) -> &Iri {
            static INST_IRI: std::sync::OnceLock<Iri> = std::sync::OnceLock::new();
            INST_IRI.get_or_init(|| Iri::parse("urn:eigenius:test:cwv:inst").unwrap())
        }
        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<Val, InstitutionError> {
            unreachable!()
        }
        fn reify(
            &self,
            _: &Iri,
            _: &Val,
            _: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            unreachable!()
        }
        fn query(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<crate::institution::runtime::QueryOutcome, InstitutionError> {
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String(
                    "urn:eigenius:institution:verdicts:fails".into(),
                )]),
            );
            Ok(crate::institution::runtime::QueryOutcome::from_output(r))
        }
    }

    /// Build a chain layered on top of core that declares an
    /// AutoOnLoad QueryClass for `urn:eigenius:test:cwv:Subject`.
    /// Returns the chain plus a derived index + runtime registering
    /// `AlwaysFails` for the institution IRI.
    fn build_cwv_setup() -> (
        Arc<crate::layer::Layer>,
        Arc<InstitutionIndex>,
        Arc<InstitutionRuntime>,
        LayerStorage,
    ) {
        let storage = test_storage();
        let core = build_core_layer(storage.clone());
        let mut b = LayerBuilder::new("test", Some(core));

        let inst_iri = "urn:eigenius:test:cwv:inst";
        let qc_iri = "urn:eigenius:test:cwv:check";
        let subject = "urn:eigenius:test:cwv:Subject";

        let mut qc = Resource::new(Iri::parse(qc_iri).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:query_class").unwrap(),
            Value::String(subject.into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:result_class").unwrap(),
            Value::String("urn:eigenius:institution:Verdict".into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:dispatch_role").unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:institution:dispatch_roles:auto_on_load".into(),
            )]),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:query_handler").unwrap(),
            Value::String("urn:eigenius:test:cwv:proc:check".into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            Value::String(inst_iri.into()),
        );
        b.add_resource(qc).unwrap();

        let layer = Arc::new(b.build(storage.clone()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");
        let mut runtime = InstitutionRuntime::new();
        runtime.register(Box::new(AlwaysFails)).unwrap();
        (layer, Arc::new(idx), Arc::new(runtime), storage)
    }

    #[test]
    fn commit_with_validation_accepts_when_no_auto_on_load_class_matches() {
        let (chain, idx, runtime, storage) = build_cwv_setup();
        let mut ctx = ExecutionContext::new(chain, "test", ExecutionMode::ReadWrite, storage);

        // Resource of an unrelated class — no AutoOnLoad matches.
        ctx.add_resource(make_resource(
            "urn:eigenius:test:cwv:unrelated",
            vec![(
                wk::IS_A,
                Value::Array(vec![Value::String("urn:eigenius:test:Other".into())]),
            )],
        ))
        .unwrap();

        let outcome = ctx
            .commit_with_validation("loaded", &idx, &runtime)
            .expect("commit_with_validation");
        assert!(!outcome.user_layer.is_root());
        assert!(
            outcome.provenance_layer.is_none(),
            "no AutoOnLoad QueryClass matched, so no provenance layer"
        );
    }

    /// Stub institution that returns a Holds Verdict alongside a
    /// partial RuntimeInvocation — exercises the D31 §6.3 commit
    /// pipeline that folds the partial into a chain-side
    /// RuntimeInvocation and commits it transactionally with the
    /// gated resource and the Verdict.
    struct HoldsWithProvenance;
    impl Institution for HoldsWithProvenance {
        fn institution_iri(&self) -> &Iri {
            static INST_IRI: std::sync::OnceLock<Iri> = std::sync::OnceLock::new();
            INST_IRI.get_or_init(|| Iri::parse("urn:eigenius:test:cwv:inst").unwrap())
        }
        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<Val, InstitutionError> {
            unreachable!()
        }
        fn reify(
            &self,
            _: &Iri,
            _: &Val,
            _: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            unreachable!()
        }
        fn query(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<crate::institution::runtime::QueryOutcome, InstitutionError> {
            // Verdict carrying ctor_name=Holds.
            let mut output = Resource::new_embedded();
            output.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String(wk::VERDICT.into())]),
            );
            output.set(
                Iri::parse(wk::CTOR_NAME).unwrap(),
                Value::String("Holds".into()),
            );
            // Partial RuntimeInvocation mirroring the substrate's
            // `DispatchTrace::into_partial_invocation` shape — an
            // embedded resource with the timestamp-ish properties
            // the substrate would normally capture.
            let mut partial = Resource::new_embedded();
            partial.set(
                Iri::parse("urn:eigenius:runtime:language").unwrap(),
                Value::String("test".into()),
            );
            partial.set(
                Iri::parse("urn:eigenius:runtime:started_at").unwrap(),
                Value::String("2026-05-05T00:00:00.000Z".into()),
            );
            partial.set(
                Iri::parse("urn:eigenius:runtime:completed_at").unwrap(),
                Value::String("2026-05-05T00:00:00.001Z".into()),
            );
            Ok(crate::institution::runtime::QueryOutcome {
                output,
                partial_invocation: Some(partial),
            })
        }
    }

    #[test]
    fn commit_with_validation_commits_provenance_on_holds() {
        // Reuse `build_cwv_setup`'s chain shape (one Subject class,
        // one AutoOnLoad QueryClass, one Institution declaration) but
        // register the Holds-with-provenance stub instead of the
        // AlwaysFails one.
        let (chain, idx, _runtime, storage) = build_cwv_setup();
        let prior_head_id = chain.id().to_string();
        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(HoldsWithProvenance))
            .expect("register");
        let runtime = Arc::new(runtime);

        let mut ctx = ExecutionContext::new(chain, "test", ExecutionMode::ReadWrite, storage);
        ctx.add_resource(make_resource(
            "urn:eigenius:test:cwv:good",
            vec![(
                wk::IS_A,
                Value::Array(vec![Value::String("urn:eigenius:test:cwv:Subject".into())]),
            )],
        ))
        .unwrap();

        let outcome = ctx
            .commit_with_validation("loaded", &idx, &runtime)
            .expect("Holds dispatch must commit cleanly");
        let gated_layer = outcome.user_layer.clone();
        // Gated resource is on the gated layer.
        assert!(
            gated_layer
                .get_resource(&iri("urn:eigenius:test:cwv:good"))
                .is_some(),
            "gated resource must commit on Holds"
        );

        // Head advances past the gated layer to a follow-up
        // verdict_provenance layer per D31 §6.3 — and that prov
        // layer is exposed on the outcome so callers can persist it.
        assert_ne!(ctx.head().id().to_string(), prior_head_id);
        assert_ne!(
            ctx.head().id().to_string(),
            gated_layer.id().to_string(),
            "Holds dispatch must commit a separate provenance layer on top of the gated layer"
        );
        let outcome_prov = outcome
            .provenance_layer
            .as_ref()
            .expect("Holds outcome carries the provenance layer for the caller to persist");
        assert_eq!(
            outcome_prov.id().to_string(),
            ctx.head().id().to_string(),
            "outcome.provenance_layer must equal ctx.head() so the caller can persist it"
        );

        // The provenance layer carries one RuntimeInvocation and one
        // Verdict, both stamped DerivedResource, with the
        // RuntimeInvocation referenced from the Verdict.
        let prov_layer = ctx.head().clone();
        let mut invocation_iri: Option<Iri> = None;
        let mut verdict_iri: Option<Iri> = None;
        for (resource_iri, resource) in prov_layer.iter_resources() {
            let is_a: Vec<String> = resource.is_a().into_iter().map(|i| i.to_string()).collect();
            if is_a
                .iter()
                .any(|s| s == "urn:eigenius:runtime:RuntimeInvocation")
            {
                invocation_iri = Some(resource_iri.clone());
            }
            if is_a.iter().any(|s| s == wk::VERDICT) {
                verdict_iri = Some(resource_iri.clone());
            }
        }
        let invocation_iri = invocation_iri.expect("RuntimeInvocation must commit on Holds");
        let verdict_iri = verdict_iri.expect("Verdict must commit on Holds");

        // Verdict IRI is derived from invocation IRI per D31 §6.3.
        assert_eq!(
            verdict_iri.as_str(),
            format!("{}:verdict", invocation_iri.as_str())
        );

        // Verdict points back at the invocation via `runtime_invocation`.
        let verdict = prov_layer.get_resource(&verdict_iri).expect("verdict");
        let inv_back = verdict
            .get(&Iri::parse("urn:eigenius:institution:runtime_invocation").unwrap())
            .and_then(|v| match v {
                Value::ResourceRef(i) => Some(i.clone()),
                Value::String(s) => Iri::parse(s).ok(),
                _ => None,
            });
        assert_eq!(inv_back.as_ref(), Some(&invocation_iri));

        // RuntimeInvocation carries the kernel-stamped fields
        // (script ← signature_iri, inputs ← gated resource, output
        // ← Verdict) per D31 §6.3.
        let invocation = prov_layer
            .get_resource(&invocation_iri)
            .expect("invocation");
        let script = invocation
            .get(&Iri::parse("urn:eigenius:runtime:script").unwrap())
            .expect("script");
        match script {
            Value::ResourceRef(i) => assert_eq!(i.as_str(), "urn:eigenius:test:cwv:proc:check"),
            other => panic!("script: expected ResourceRef, got {other:?}"),
        }
        let output_ref = invocation
            .get(&Iri::parse("urn:eigenius:runtime:output").unwrap())
            .expect("output");
        match output_ref {
            Value::ResourceRef(i) => assert_eq!(i, &verdict_iri),
            other => panic!("output: expected ResourceRef, got {other:?}"),
        }
    }

    #[test]
    fn commit_with_validation_rejects_when_auto_on_load_returns_fails() {
        let (chain, idx, runtime, storage) = build_cwv_setup();
        let prior_head_id = chain.id().to_string();
        let mut ctx = ExecutionContext::new(chain, "test", ExecutionMode::ReadWrite, storage);

        ctx.add_resource(make_resource(
            "urn:eigenius:test:cwv:bad",
            vec![(
                wk::IS_A,
                Value::Array(vec![Value::String("urn:eigenius:test:cwv:Subject".into())]),
            )],
        ))
        .unwrap();

        let err = ctx
            .commit_with_validation("loaded", &idx, &runtime)
            .expect_err("AlwaysFails should abort the gated commit");
        let provenance_layer_id = match err {
            ContextError::ValidationFailed {
                errors,
                provenance_layer,
            } => {
                assert!(errors.iter().any(|e| e.message.contains("returned Fails")));
                // Per D31 §6.3: gated resource is rejected, but the
                // Verdict audit anchor commits on a separate layer.
                let layer = provenance_layer
                    .expect("Fails dispatch must commit a Verdict provenance layer per D31 §6.3");
                let verdict_committed = layer
                    .iter_resources()
                    .any(|(iri, _)| iri.as_str().contains(":verdict"));
                assert!(
                    verdict_committed,
                    "provenance layer must contain at least one Verdict resource"
                );
                layer.id().to_string()
            }
            other => panic!("expected ValidationFailed, got {other}"),
        };
        // The gated `urn:eigenius:test:cwv:bad` resource never landed —
        // the layer holding it was dropped. Head now points at the
        // provenance-only layer, which sits on top of the prior head.
        assert_ne!(ctx.head().id().to_string(), prior_head_id);
        assert_eq!(ctx.head().id().to_string(), provenance_layer_id);
        assert!(
            ctx.head()
                .get_resource(&iri("urn:eigenius:test:cwv:bad"))
                .is_none(),
            "gated resource must not appear on the chain after a Fails"
        );
    }
}
