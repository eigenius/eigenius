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

//! `Load` RPC handler.
//!
//! Parses an Eigon document, stages it onto the per-branch
//! `ExecutionContext`'s working layer, applies caller-supplied
//! explicit tombstones, and drives a single
//! [`crate::commit::CommitOrchestrator`] run through `WithInstitutions`
//! when `auto_commit` is set. Translates the resulting
//! `MultiLayerOutcome` into `LoadResponse` fields per D41 §10.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::commit::persister::PersistedLayerInfo;
use crate::observability::{field, operation, RpcGuard};
use crate::ontology::Iri;
use std::sync::Arc;
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_load(
        &self,
        req: LoadRequest,
    ) -> Result<Response<LoadResponse>, Status> {
        let mut guard = RpcGuard::start(operation::RPC_LOAD);
        let branch = resolve_branch_name(&req.branch).to_string();
        tracing::debug!(
            { field::OPERATION } = operation::RPC_LOAD,
            { field::CONTENT_TYPE } = %req.content_type,
            { field::SIZE_BYTES } = req.resources.len(),
            branch = %branch,
            "load payload"
        );
        let resources = self
            .parse_resources(&req.resources, &req.content_type, Some(&branch))
            .await?;
        let count = resources.len() as u32;

        let ctx_arc = self.get_branch_context(&branch).await?;
        let mut ctx = ctx_arc.write().await;
        for resource in resources {
            ctx.add_resource(resource)
                .map_err(|e| Status::failed_precondition(format!("load error: {e}")))?;
        }
        // D41 §10.1: apply caller-supplied explicit tombstones to the
        // working builder before `take_working` consumes it. Validating
        // each IRI here mirrors the resource-parsing error surface — a
        // malformed IRI is a client-side bug, not silently-dropped.
        for raw in &req.explicit_tombstones {
            let iri = Iri::parse(raw).map_err(|e| {
                Status::invalid_argument(format!("invalid tombstone IRI {raw:?}: {e}"))
            })?;
            ctx.tombstone(iri)
                .map_err(|e| Status::failed_precondition(format!("tombstone error: {e}")))?;
        }

        let mut layer_id = String::new();
        let mut branch_advanced = false;
        let mut total_violations: u32 = 0;
        let mut committed_layers: Vec<CommittedLayer> = Vec::new();
        // The user-layer's persist info — the user-facing one. Any
        // follow-up persists (AutoOnLoad provenance, institution_classes)
        // log on failure but their outcomes are not surfaced in the
        // response; see proto comment on `RunProgramResponse.merge`
        // for the design rationale. Stashed as the full struct so the
        // response can disambiguate `CACHED_DIFFERENT_POSITION` from
        // `UNSPECIFIED` via `info.cache_hit_different_position`.
        let mut user_persist_info: Option<PersistedLayerInfo> = None;
        let mut errors = Vec::new();

        if req.auto_commit {
            // D41 Phase E migration. The ~250-line handler-side
            // revert state-machine collapses into one
            // `CommitOrchestrator::run` call:
            //
            // 1. Take the working builder we just populated above.
            //    `take_working` consumes it and installs a fresh
            //    builder parented at `ctx.head` so the orchestrator
            //    can re-use the context for emission-driven layers.
            // 2. Snapshot the institution index + runtime. Newly
            //    committed resources are gated by AutoOnLoad
            //    QueryClasses *already* declared in the chain;
            //    QueryClasses declared in the same Load batch take
            //    effect on subsequent loads (the
            //    `rebuild_institution_index` `didDrain` hook
            //    refreshes the cell after the drain completes).
            // 3. Construct the root `LayerEmission` from the working
            //    builder.
            // 4. Run the orchestrator. It drains the root through
            //    `WithInstitutions`, runs the `trigger_vector_sweep`
            //    `didPersist` hook, and finally fires the
            //    `rebuild_institution_index` `didDrain` hook once with
            //    the top layer.
            // 5. Translate the resulting `MultiLayerOutcome` to
            //    `LoadResponse` fields.
            //
            // The write guard on `ctx` must outlive the orchestrator
            // call because the orchestrator borrows `&mut ExecutionContext`
            // for `take_working` / `advance_head` / `revert_head` (D41 §9).
            // We extend it for the orchestrator's lifetime and drop it
            // explicitly before computing the response.
            let working = match ctx.take_working("loaded") {
                Ok(b) => b,
                Err(e) => {
                    return Err(Status::failed_precondition(format!("load error: {e}")));
                }
            };
            let index_snapshot = Arc::clone(&*self.institution_index.read().await);
            let runtime_snapshot = Arc::clone(&*self.institution_runtime.read().await);

            let root = crate::commit::LayerEmission::from_builder(
                crate::commit::LayerRole::User,
                "loaded",
                crate::commit::PipelineKind::WithInstitutions,
                crate::commit::EmissionKind::Child,
                working,
            );

            // The orchestrator borrows `&mut ctx` for the duration of
            // `run`. We scope its construction inside a block so the
            // borrow ends before we read `outcome` fields.
            let policy = commit_policy_from_proto(req.policy.as_ref());
            let outcome = {
                let orchestrator = crate::commit::CommitOrchestrator {
                    ctx: &mut ctx,
                    pool: &self.commit_ws_pool,
                    persister: &*self.persister,
                    host: self as &dyn crate::commit::CommitHookHost,
                    branch: &branch,
                    policy,
                    institutions: Some(crate::commit::InstitutionContext {
                        index: index_snapshot,
                        runtime: runtime_snapshot,
                        _marker: std::marker::PhantomData,
                    }),
                    did_drain: crate::commit::CommitOrchestrator::default_did_drain(),
                };
                orchestrator.run(root)
            };

            // Drop the write guard now — we're done with `ctx` for the
            // rest of the handler; the response shape only needs
            // values copied out of `outcome`.
            drop(ctx);

            // D41 §10: translate `MultiLayerOutcome` to the
            // `LoadResponse` fields. The user layer is the outcome
            // whose role is `LayerRole::User` — *not* `layers[0]`,
            // because on the rejected-but-audited path
            // (`autoonload_dispatch` returns Err with a
            // `verdict_provenance` Sibling rescue) the failing
            // user-layer pipeline pushes nothing to `outcome.layers`
            // and the rescued audit lands at index 0. The closed
            // [`crate::commit::LayerRole`] taxonomy (D41 §6 /
            // outcome.LayerRole) lets us pick the right entry
            // unambiguously without string compares.
            let user_layer_outcome = outcome
                .layers
                .iter()
                .find(|l| l.role == crate::commit::LayerRole::User);
            if let Some(u) = user_layer_outcome {
                // Use `persist.layer_id` rather than `layer.id()`: the
                // anchored-commit cache (D33 §6) substitutes the
                // canonical layer's id on a different-position hit, so
                // `persist.layer_id` is the id callers expect to see.
                // On a fresh commit the two are equal.
                layer_id = u.persist.layer_id.to_string();
                branch_advanced = u.persist.branch_advanced;
                user_persist_info = Some(u.persist.clone());
                tracing::info!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::LAYER_ID } = %layer_id,
                    { field::COUNT } = count,
                    branch = %branch,
                    "layer committed"
                );
            }

            // Per-layer outcomes for the response. Mapped in drain
            // order so callers can recover the chronological story of
            // what landed.
            committed_layers = outcome
                .layers
                .iter()
                .map(committed_layer_to_proto)
                .collect();

            // Surface didPersist hook errors and drain hook errors —
            // these don't unwind the commit but the caller should see
            // them.
            for layer_outcome in &outcome.layers {
                for ve in &layer_outcome.hook_errors {
                    errors.push(kernel_validation_error_to_proto(ve));
                }
            }
            for ve in &outcome.drain_hook_errors {
                errors.push(kernel_validation_error_to_proto(ve));
            }

            // Surface the pipeline error (if any). Each error path is
            // logged at warn level the same way the pre-D41 handler
            // logged ContextError::ValidationFailed entries — one
            // event per rule violation so dashboards can group on
            // `error_kind`.
            if let Some(commit_err) = outcome.error.as_ref() {
                // Capture the policy's `total_violations` cap-aware
                // count before we destructure. The proto field surfaces
                // the full violation count even when `errors` was
                // truncated by `CommitPolicy::Reject::max_violations`.
                total_violations = commit_error_total_violations(commit_err);
                match commit_err {
                    crate::commit::CommitError::Validation { errors: verrs, .. }
                    | crate::commit::CommitError::CascadeAbort { errors: verrs, .. } => {
                        for ve in verrs {
                            tracing::warn!(
                                { field::OPERATION } = operation::VALIDATE_RESOURCE,
                                { field::ERROR_KIND } = ?ve.rule,
                                { field::RESOURCE_IRI } = ve.resource_id.as_ref().map(|i| i.as_str()).unwrap_or(""),
                                { field::PROPERTY_IRI } = ve.property.as_ref().map(|i| i.as_str()).unwrap_or(""),
                                { field::ERROR_MESSAGE } = %ve.message,
                                "validation error"
                            );
                        }
                    }
                    other => {
                        tracing::warn!(
                            { field::OPERATION } = operation::LAYER_COMMIT,
                            { field::ERROR_KIND } = "commit_failed",
                            { field::ERROR_MESSAGE } = %other,
                            "layer commit failed"
                        );
                    }
                }
                for proto_err in commit_error_to_proto(commit_err) {
                    errors.push(proto_err);
                }
            }
        }

        // `merge` is populated only when a commit was actually
        // attempted. `auto_commit=false` (validate-only Load) skips the
        // entire commit block above, so no CAS happened; sending an
        // `UNSPECIFIED` MergeInfo in that case would render as a
        // misleading `cached` badge in notebook UIs.
        let response = LoadResponse {
            success: errors.is_empty(),
            errors,
            layer_id,
            resource_count: count,
            branch,
            branch_advanced,
            merge: if req.auto_commit {
                Some(merge_info_from_persist_info(user_persist_info.as_ref()))
            } else {
                None
            },
            total_violations,
            committed_layers,
        };
        if !response.success {
            guard.fail("validation_failed");
            tracing::warn!(
                { field::OPERATION } = operation::RPC_LOAD,
                { field::COUNT } = response.errors.len(),
                "load completed with errors"
            );
        }
        Ok(Response::new(response))
    }
}
