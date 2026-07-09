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
//! plus the async/sync delegate plumbing for the post-commit
//! institution-index rebuild and vector sweep.

use super::EigeniusService;
use crate::observability::{field, operation};
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
    /// by layering the chain's external institutions (dispatched over gRPC
    /// to the orchestrator substrate) then its in-process institutions.
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

        // Build an empty runtime, then layer in any external-runtime
        // institutions (D31 §5), then any in-process institutions
        // (D28 Phase 20a.1).
        let mut runtime = crate::institution::runtime::InstitutionRuntime::new();
        let mut report = crate::capability::registration::RegistrationReport::default();
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
}

// `CommitHookHost` impl for `EigeniusService`. The trait surface is
// sync (hook fn pointers can't be async), so the async inherent
// `rebuild_institution_index` is wrapped with
// `block_in_place(|| Handle::current().block_on(...))` — the hooks run
// on the tokio thread driving the orchestrator, so a runtime is always
// available and `block_in_place` blocks without starving the scheduler.
impl crate::commit::CommitHookHost for EigeniusService {
    fn rebuild_institution_index(
        &self,
        top_layer: &Arc<crate::layer::Layer>,
    ) -> Result<(), Vec<crate::validation::ValidationError>> {
        // The inherent async method has no error path — it logs
        // failures at warn level and updates the index best-effort.
        // The hook's `Ok` return mirrors that. A future widening
        // could surface lock-poisoning / index-rebuild failures
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
