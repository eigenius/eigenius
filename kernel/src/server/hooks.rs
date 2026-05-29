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

//! D41 §3.6 — [`crate::commit::CommitHookHost`] impl for [`EigeniusService`],
//! plus the async/sync delegate plumbing for WASM-component registration
//! and the post-commit institution-index rebuild.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::capability::registration::PendingIoComponent;
use crate::commit::persister::PersistedLayerInfo;
use crate::layer::LayerStorage;
use crate::observability::{field, operation};
use crate::ontology::Resource;
use crate::program::component::ComponentRegistry;
use std::sync::Arc;

impl EigeniusService {
    /// Rebuild the D14 [`crate::institution::registry::InstitutionIndex`]
    /// from the given layer (which is the new head of the chain). Called
    /// after every successful commit + after Phase 9a rehydration.
    ///
    /// Walks the entire chain from the supplied layer downward; any
    /// per-resource parse errors are logged at warn-level and skipped
    /// (the well-formed entries still index — same shape as the
    /// existing capability-scan flow).
    ///
    /// Also rebuilds the [`crate::institution::runtime::InstitutionRuntime`]
    /// by scanning the chain for Institution declarations whose `runtime`
    /// is `urn:eigenius:institution:runtimes:wasm` and constructing a
    /// [`crate::capability::wasm_institution::WasmInstitution`] for each.
    /// In-process / external runtime declarations are skipped — those
    /// callers register programmatically via the runtime API. This
    /// closes the "ontology-first" loop for WASM institutions: declaring
    /// an Institution + `wasm_binary` in the chain auto-installs its
    /// dispatcher on commit.
    pub(super) async fn rebuild_institution_index(&self, layer: &crate::layer::Layer) {
        let (idx, errors) = crate::institution::registry::InstitutionIndex::from_layer(layer);
        for err in &errors {
            tracing::warn!(
                { field::OPERATION } = operation::INSTITUTION_REGISTER,
                kind = err.kind,
                resource_iri = err
                    .resource_iri
                    .as_ref()
                    .map(|i| i.as_str())
                    .unwrap_or(""),
                { field::ERROR_MESSAGE } = %err.reason,
                "institution-index parse error"
            );
        }
        let idx_arc = Arc::new(idx);
        *self.institution_index.write().await = Arc::clone(&idx_arc);

        // Rebuild the runtime from chain-declared WASM institutions,
        // then layer in any external-runtime institutions (D31 §5),
        // then layer in any in-process institutions (D28 Phase 20a.1).
        let (mut runtime, mut report) =
            crate::capability::registration::build_wasm_institution_runtime(layer);
        if let Some(client) = self.orchestrator_client.as_ref() {
            crate::capability::registration::register_external_institutions(
                layer,
                idx_arc.as_ref(),
                &mut runtime,
                Arc::clone(client),
                &mut report,
            );
        } else {
            // No orchestrator wired — external institutions cannot
            // dispatch. Surface this once per rebuild rather than per
            // institution so the operator sees it.
            let has_external = idx_arc.institutions().any(|e| {
                matches!(
                    e.runtime,
                    Some(crate::institution::registry::RuntimeKind::External)
                )
            });
            if has_external {
                tracing::warn!(
                    { field::OPERATION } = operation::INSTITUTION_REGISTER,
                    "chain declares `runtime: external` institutions but the kernel was started \
                     without --orchestrator; their dispatch will fail"
                );
            }
        }
        crate::capability::registration::register_in_process_institutions(
            idx_arc.as_ref(),
            &mut runtime,
            self.in_process_registry.as_ref(),
            &mut report,
        );
        for err in &report.errors {
            tracing::warn!(
                { field::OPERATION } = operation::INSTITUTION_REGISTER,
                resource_iri = %err.resource_iri,
                { field::ERROR_MESSAGE } = %err.message,
                "institution registration error"
            );
        }
        for inst_iri in &report.institutions_registered {
            tracing::info!(
                { field::OPERATION } = operation::INSTITUTION_REGISTER,
                { field::INSTITUTION_IRI } = %inst_iri,
                host = "kernel",
                "registered institution"
            );
        }
        *self.institution_runtime.write().await = Arc::new(runtime);
    }

    /// Walk a newly committed layer and register every WASM component
    /// (kernel-hosted or IO-class) declared therein. WASM-institution
    /// registration is **no longer** performed here — D14 institutions
    /// register through the chain via the
    /// [`crate::institution::registry::InstitutionIndex`] +
    /// [`crate::institution::runtime::InstitutionRuntime`] populated by
    /// [`Self::rebuild_institution_index`].
    pub(super) async fn register_wasm_from_layer(
        &self,
        layer: &crate::layer::Layer,
        errors: &mut Vec<ValidationError>,
    ) -> Vec<Resource> {
        // Build a new ComponentRegistry layered on top of the current one.
        let mut new_registry = {
            let current = self.components.read().await;
            ComponentRegistry::new_with_parent(Arc::clone(&current))
        };

        let scan_result =
            crate::capability::registration::scan_and_register(layer, &mut new_registry);

        for e in &scan_result.report.errors {
            errors.push(ValidationError {
                resource_iri: e.resource_iri.clone(),
                property_iri: String::new(),
                rule: "wasm_registration".to_string(),
                message: e.message.clone(),
                severity: "error".to_string(),
            });
        }
        for w in &scan_result.report.warnings {
            tracing::warn!(
                { field::OPERATION } = operation::CAPABILITY_INSTALL,
                "wasm scan warning: {}",
                w
            );
        }

        // Forward IO WASM components to the orchestrator and register a
        // RemoteComponent locally so the kernel can dispatch to them.
        let mut any_kernel_component_added = !scan_result.report.components_registered.is_empty()
            && scan_result.pending_io_components.is_empty();
        for pending in scan_result.pending_io_components {
            match self.register_io_wasm(&pending).await {
                Ok(remote) => {
                    tracing::info!(
                        { field::OPERATION } = operation::CAPABILITY_INSTALL,
                        { field::COMPONENT_IRI } = %pending.resource_iri,
                        host = "orchestrator",
                        "registered IO WASM component"
                    );
                    new_registry.register(pending.resource_iri.clone(), remote);
                    any_kernel_component_added = true;
                }
                Err(e) => {
                    errors.push(ValidationError {
                        resource_iri: pending.resource_iri,
                        property_iri: String::new(),
                        rule: "wasm_registration".to_string(),
                        message: e,
                        severity: "error".to_string(),
                    });
                }
            }
        }

        for iri in &scan_result.report.components_registered {
            tracing::info!(
                { field::OPERATION } = operation::CAPABILITY_INSTALL,
                { field::COMPONENT_IRI } = %iri,
                host = "kernel",
                "registered WASM component"
            );
        }

        if any_kernel_component_added {
            let mut guard = self.components.write().await;
            *guard = Arc::new(new_registry);
        }

        // No institution-published resources under D14 — declarations
        // ride into the chain as ordinary Eigon resources. Returns an
        // empty Vec for source-compatibility with the Load handler's
        // follow-up-commit logic (which is now a no-op).
        Vec::new()
    }

    /// RESUME counterpart of [`Self::register_wasm_from_layer`]. Walks a
    /// rehydrated layer and re-registers every WASM component it
    /// finds. IO components are forwarded to the orchestrator again
    /// (same semantics as fresh install; the orchestrator may reject
    /// if it already has the component). WASM institutions register
    /// via D14 (chain scan + InstitutionRuntime) — no per-layer
    /// rehydration call here.
    async fn rehydrate_wasm_from_layer(
        &self,
        layer: &crate::layer::Layer,
        errors: &mut Vec<ValidationError>,
    ) {
        let mut new_registry = {
            let current = self.components.read().await;
            ComponentRegistry::new_with_parent(Arc::clone(&current))
        };

        let scan_result =
            crate::capability::registration::scan_and_register(layer, &mut new_registry);

        for e in &scan_result.report.errors {
            errors.push(ValidationError {
                resource_iri: e.resource_iri.clone(),
                property_iri: String::new(),
                rule: "wasm_rehydrate".to_string(),
                message: e.message.clone(),
                severity: "error".to_string(),
            });
        }

        let mut any_kernel_component_added = !scan_result.report.components_registered.is_empty()
            && scan_result.pending_io_components.is_empty();
        for pending in scan_result.pending_io_components {
            match self.register_io_wasm(&pending).await {
                Ok(remote) => {
                    tracing::info!(
                        { field::OPERATION } = operation::CAPABILITY_INSTALL,
                        { field::COMPONENT_IRI } = %pending.resource_iri,
                        host = "orchestrator",
                        rehydrated = true,
                        "rehydrated IO WASM component"
                    );
                    new_registry.register(pending.resource_iri.clone(), remote);
                    any_kernel_component_added = true;
                }
                Err(e) => {
                    errors.push(ValidationError {
                        resource_iri: pending.resource_iri,
                        property_iri: String::new(),
                        rule: "wasm_rehydrate".to_string(),
                        message: e,
                        severity: "error".to_string(),
                    });
                }
            }
        }

        if any_kernel_component_added {
            let mut guard = self.components.write().await;
            *guard = Arc::new(new_registry);
        }
    }

    /// Walk the persisted chain from root to head and rehydrate every
    /// WASM capability resource found in each layer. Called once by the
    /// server at startup when a persistent backend is attached.
    pub async fn rehydrate_wasm_from_chain(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let head = match self.get_branch_context(DEFAULT_BRANCH).await {
            Ok(ctx_arc) => {
                let ctx = ctx_arc.read().await;
                Arc::clone(ctx.head())
            }
            Err(status) => {
                errors.push(ValidationError {
                    resource_iri: String::new(),
                    property_iri: String::new(),
                    rule: "rehydrate".to_string(),
                    message: format!("get main context: {status}"),
                    severity: "error".to_string(),
                });
                return errors;
            }
        };
        // Collect root-to-head order so earlier layers register first.
        let mut chain: Vec<Arc<crate::layer::Layer>> = Vec::new();
        let mut cursor = Some(head);
        while let Some(layer) = cursor {
            let parent = layer.parent().cloned();
            chain.push(layer);
            cursor = parent;
        }
        chain.reverse();

        for layer in &chain {
            self.rehydrate_wasm_from_layer(layer, &mut errors).await;
        }
        errors
    }

    /// Forward an IO WASM component to the orchestrator and produce a
    /// local `RemoteComponent` wrapper that dispatches `Execute` calls
    /// back to the orchestrator.
    async fn register_io_wasm(
        &self,
        pending: &PendingIoComponent,
    ) -> Result<Box<dyn crate::program::component::BuiltinComponent>, String> {
        let client = self.orchestrator_client.as_ref().ok_or_else(|| {
            "IO WASM components require an orchestrator to be configured \
                 (pass --orchestrator to `serve`)"
                .to_string()
        })?;

        let request = RegisterWasmComponentRequest {
            component_iri: pending.resource_iri.clone(),
            wasm_binary: pending.wasm_binary.clone(),
            fuel_limit: pending.fuel_limit,
            memory_limit_pages: pending.memory_limit_pages as u64,
        };

        let response = {
            let mut c = client.lock().await;
            c.register_wasm_component(tonic::Request::new(request))
                .await
                .map_err(|e| format!("RegisterWasmComponent gRPC call failed: {e}"))?
        };
        let resp = response.into_inner();
        if !resp.success {
            return Err(format!(
                "orchestrator rejected WASM registration: {}",
                resp.error
            ));
        }

        // Build a local RemoteComponent that forwards Execute calls.
        Ok(Box::new(crate::program::remote::RemoteComponent::new(
            pending.resource_iri.clone(),
            Arc::clone(client),
        )))
    }
}

// D41 §3.6 — `CommitHookHost` impl for `EigeniusService`. Lives next to
// the inherent `register_wasm_from_layer` / `rebuild_institution_index_async`
// methods that back the two hook points so the async-to-sync bridge
// shape stays visible in one place.
//
// **Async-to-sync bridge.** Both delegate methods on `EigeniusService`
// are `async fn`, but the `CommitHookHost` trait surface is sync
// (hook function pointers can't be async). Each method wraps with
// `tokio::task::block_in_place(|| Handle::current().block_on(...))`
// — the same dual-context pattern Phase B noted for
// `BackendStorePersister`-style sync-over-async work. The hooks run
// on the tokio thread driving the orchestrator (which itself runs
// inside a tonic handler), so a current-thread runtime is always
// available; `block_in_place` permits blocking without starving the
// scheduler.
//
// **`register_wasm_for_layer` shape.** The existing
// `register_wasm_from_layer` does more than register WASM components
// — it also forwards IO components to the orchestrator, updates the
// kernel-hosted component registry, and (by historical convention)
// returns a `Vec<Resource>` that today is always empty (the
// institution-classes follow-up content used to come from here but
// no longer does — D14 institutions declare their classes directly
// in the chain). The hook surface still requires the return because
// future institution shapes may re-introduce auto-published classes.
// The returned `Vec` flows into the `institution_classes` Child
// emission (no-op today; non-empty future).
impl crate::commit::CommitHookHost for EigeniusService {
    fn register_wasm_for_layer(
        &self,
        layer: &Arc<crate::layer::Layer>,
    ) -> Result<Vec<Resource>, Vec<crate::validation::ValidationError>> {
        let mut proto_errors: Vec<ValidationError> = Vec::new();
        let resources = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.register_wasm_from_layer(layer.as_ref(), &mut proto_errors))
        });
        if proto_errors.is_empty() {
            Ok(resources)
        } else {
            // D41 §3.6: errors flow into `state.hook_errors` and the
            // commit stands. The host returns Err so the hook can
            // surface them; the returned `resources` (if any) are
            // discarded because the institution_classes follow-up
            // can't proceed if registration failed for some
            // declarations. This mirrors today's handler which also
            // surfaces these as errors but commits the user layer
            // anyway.
            Err(proto_errors
                .iter()
                .map(convert_proto_validation_error)
                .collect())
        }
    }

    fn rebuild_institution_index(
        &self,
        top_layer: &Arc<crate::layer::Layer>,
    ) -> Result<(), Vec<crate::validation::ValidationError>> {
        // The inherent async method has no error path — it logs
        // failures at warn level and updates the index best-effort.
        // The hook's `Ok` return mirrors that. A future widening
        // could surface lock-poisoning / wasm-runtime build failures
        // here; for today the hook is always Ok.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.rebuild_institution_index(top_layer.as_ref()))
        });
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// LayerPersister impl + inherent persist helpers.
//
// The orchestrator drives the per-layer persist step through the
// `crate::commit::LayerPersister` trait so the kernel-side commit
// module stays free of any direct dependency on `EigeniusService`.
// `EigeniusService` is the canonical implementer; commit-shaped RPC
// handlers pass `self as &dyn crate::commit::LayerPersister` to the
// orchestrator (D41 §7 / Phase F).
//
// The three inherent helpers below (`probe_anchored_commit`,
// `put_anchored_commit_for_layer`, `advance_branch_for_layer`) carve
// the anchored-commit cache probe + CAS dispatch out of the persist
// body so the cache-miss and same-position cache-hit paths share
// logic.
// ---------------------------------------------------------------------------

impl EigeniusService {
    /// Compute the anchored-commit cache key for `layer` and probe the
    /// backend. Returns `None` when the layer has no supporting layer
    /// (root / self-referential) or when no cache entry exists.
    /// Verifies the cached layer is still in storage before returning
    /// — a stale entry (cached layer was reclaimed by GC) is treated
    /// as a cache miss.
    fn probe_anchored_commit(
        &self,
        backend: &dyn crate::storage::PersistentBackend,
        layer: &crate::layer::Layer,
    ) -> Option<crate::layer::LayerId> {
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
    fn put_anchored_commit_for_layer(
        &self,
        backend: &dyn crate::storage::PersistentBackend,
        layer: &crate::layer::Layer,
    ) {
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
    /// Carved out of the persist body so both the cache-miss path and
    /// the same-position cache-hit path can share the logic.
    ///
    /// Returns the lattice's [`UpdateOutcome`](crate::lattice::UpdateOutcome)
    /// verbatim so the caller can:
    ///
    /// - distinguish `FastForward` (clean CAS) from `TrivialMerge`
    ///   (concurrent disjoint-IRI contributions; kernel produced a
    ///   merge layer) from `NeedsWitnessedMerge` (concurrent
    ///   conflicting contributions; branch unchanged);
    /// - correctly compute `branch_advanced` — in particular,
    ///   `NeedsWitnessedMerge` means the branch did **not** advance
    ///   (the layer is stored but unreachable from any branch ref).
    ///
    /// Pre-D34 §G.1 this method swallowed all `Ok` variants as
    /// `Ok(())`, masking the `NeedsWitnessedMerge` failure as success.
    ///
    /// D41 Phase F: returns kernel-internal
    /// [`crate::validation::ValidationError`] now that the only caller
    /// is the [`crate::commit::LayerPersister`] trait impl on
    /// [`EigeniusService`] (the inherent `persist_layer_if_backend`
    /// was inlined and deleted in Phase F).
    fn advance_branch_for_layer(
        &self,
        branch: &str,
        layer: &crate::layer::Layer,
        backend: &dyn crate::storage::PersistentBackend,
    ) -> Result<crate::lattice::UpdateOutcome, crate::validation::ValidationError> {
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
                Err(crate::validation::ValidationError {
                    resource_id: None,
                    property: None,
                    rule: crate::validation::ValidationRule::InstitutionValidation,
                    message: format!("advance_branch failed: {e}"),
                })
            }
        }
    }
}

impl crate::commit::LayerPersister for EigeniusService {
    fn persist(
        &self,
        branch: &str,
        layer: &Arc<crate::layer::Layer>,
    ) -> Result<PersistedLayerInfo, crate::validation::ValidationError> {
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
            // Different-position cache hit — the canonical layer is
            // elsewhere. Skip both `store_layer` and `update_branch`;
            // the branch stays where it is (D33 §6 supporting-
            // equivalent context). No CAS attempted, so merge_outcome
            // is None.
            tracing::debug!(
                { field::OPERATION } = operation::LAYER_COMMIT,
                { field::LAYER_ID } = %layer.id(),
                cached_layer = %cached_id,
                branch = branch,
                cache = "hit_different_position",
                "anchored-commit cache hit (different position) — branch unchanged"
            );
            return Ok(PersistedLayerInfo {
                layer_id: cached_id,
                branch_advanced: false,
                merge_outcome: None,
                cache_hit_different_position: true,
            });
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
            return Err(crate::validation::ValidationError {
                resource_id: None,
                property: None,
                rule: crate::validation::ValidationRule::InstitutionValidation,
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
