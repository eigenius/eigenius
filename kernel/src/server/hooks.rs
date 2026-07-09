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
use crate::observability::{field, operation};
use crate::ontology::Resource;
use crate::program::component::ComponentRegistry;
use std::sync::Arc;

impl EigeniusService {
    /// Rebuild the [`crate::institution::registry::InstitutionIndex`]
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
        // Index-driven discovery (D23): find the chain's institution declarations via
        // the triple index, not by materialising the whole chain. On a chain carrying
        // a large domain lexicon this is the difference between an O(handful) rebuild
        // and an O(hundreds-of-thousands) full scan on every commit.
        let (idx, errors) =
            crate::institution::registry::InstitutionIndex::from_layer_indexed(layer);
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
            crate::capability::registration::build_wasm_institution_runtime_indexed(layer);
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
    /// registration is **no longer** performed here — institutions
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

        // No institution-published resources — declarations
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
    /// via institution dispatch (chain scan + InstitutionRuntime) — no per-layer
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
// no longer does — institutions declare their classes directly
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

    /// D43 §5.5 — post-Load vector-index sweep. The coordinator is
    /// optional: no embedders registered → no coordinator → hook is
    /// a no-op. When present, we spawn the sweep onto the current
    /// tokio runtime via [`SweepCoordinator::trigger_async`] so the
    /// commit pipeline doesn't block on Embedder IO (per D43 §5.5's
    /// "async and non-gating" stance). The handle is intentionally
    /// detached — the sweep's terminal state is observable via the
    /// `SweepRegistry`, not by awaiting here.
    fn trigger_vector_sweep_for_layer(
        &self,
        layer: &Arc<crate::layer::Layer>,
    ) -> Result<(), Vec<crate::validation::ValidationError>> {
        let Some(coord) = self.sweep_coordinator.clone() else {
            return Ok(());
        };
        // Cheap pre-check: skip the spawn entirely when the layer
        // has no active VectorIndex Resources — the coordinator
        // would short-circuit anyway, but the empty case is the
        // common one (any non-vector Load) and we don't want a
        // detached task per commit on those.
        let active = crate::layer::resolve_active_vector_indexes(layer);
        if active.is_empty() {
            return Ok(());
        }
        let layer_arc = Arc::clone(layer);
        let layer_id_disp = format!("{}", layer.id());
        let n_indexes = active.len();
        tracing::info!(
            { crate::observability::field::OPERATION } =
                crate::observability::operation::COMMIT_DID_PERSIST,
            { crate::observability::field::LAYER_ID } = %layer_id_disp,
            n_indexes = n_indexes,
            "scheduling post-Load vector sweep"
        );
        tokio::spawn(async move {
            match coord.trigger_async(layer_arc).await {
                Ok(None) => {
                    tracing::debug!(
                        { crate::observability::field::OPERATION } =
                            crate::observability::operation::COMMIT_DID_PERSIST,
                        { crate::observability::field::LAYER_ID } = %layer_id_disp,
                        "vector sweep finished: no active indexes (race after detection)"
                    );
                }
                Ok(Some((_handle, report))) => {
                    tracing::info!(
                        { crate::observability::field::OPERATION } =
                            crate::observability::operation::COMMIT_DID_PERSIST,
                        { crate::observability::field::LAYER_ID } = %layer_id_disp,
                        total_subjects = report.total_subjects,
                        skipped = report.skipped,
                        "vector sweep completed"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        { crate::observability::field::OPERATION } =
                            crate::observability::operation::COMMIT_DID_PERSIST,
                        { crate::observability::field::ERROR_KIND } = "vector_sweep_failed",
                        { crate::observability::field::LAYER_ID } = %layer_id_disp,
                        { crate::observability::field::ERROR_MESSAGE } = %e,
                        "post-Load vector sweep failed"
                    );
                }
            }
        });
        Ok(())
    }
}
