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

//! `Reflect` RPC handler — commit a trace (ProgramTrace,
//! DeclarationTrace, …) into the chain via the `CommitOrchestrator`
//! under `WithRetroactive`.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::observability::{operation, RpcGuard};
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_reflect(
        &self,
        req: ReflectRequest,
    ) -> Result<Response<ReflectResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_REFLECT);
        let branch = resolve_branch_name(&req.branch).to_string();
        let resources = self
            .parse_resources(&req.trace, &req.content_type, Some(&branch))
            .await?;

        if resources.is_empty() {
            // No resources to commit — `merge` stays None (no CAS).
            return Ok(Response::new(ReflectResponse {
                success: false,
                trace_iri: String::new(),
                branch_advanced: false,
                merge: None,
            }));
        }

        // The first resource should be a trace (ProgramTrace, DeclarationTrace, etc.)
        let trace_resource = &resources[0];
        let trace_iri = trace_resource
            .id()
            .map(|i| i.as_str().to_string())
            .unwrap_or_default();

        // D41 Phase F migration. Same shape as Load (D41 §10): build
        // the working layer, hand it to the orchestrator as a
        // `WithRetroactive` root emission, translate the
        // `MultiLayerOutcome` back into `ReflectResponse` fields. The
        // orchestrator centralises head revert + persist-not-advanced
        // bookkeeping. Reflect content is kernel-emitted trace data —
        // not user-authored — but it is committed through
        // `WithRetroactive` per D41 §10 so cascade tombstoning still
        // applies if a future trace shape touches lower-layer
        // institutional declarations.
        let ctx_arc = self.get_branch_context(&branch).await?;
        let mut ctx = ctx_arc.write().await;
        for resource in resources {
            ctx.add_resource(resource)
                .map_err(|e| Status::failed_precondition(format!("reflect error: {e}")))?;
        }

        let working = ctx
            .take_working("reflect")
            .map_err(|e| Status::failed_precondition(format!("reflect error: {e}")))?;

        let root = crate::commit::LayerEmission::from_builder(
            crate::commit::LayerRole::User,
            "reflect",
            crate::commit::PipelineKind::WithRetroactive,
            crate::commit::EmissionKind::Child,
            working,
        );

        let outcome = {
            let orchestrator = crate::commit::CommitOrchestrator {
                ctx: &mut ctx,
                pool: &self.commit_ws_pool,
                persister: &*self.persister,
                host: self as &dyn crate::commit::CommitHookHost,
                branch: &branch,
                policy: crate::lattice::CommitPolicy::default(),
                // Reflect does not run AutoOnLoad — only Load does.
                institutions: None,
                did_drain: crate::commit::CommitOrchestrator::default_did_drain(),
            };
            orchestrator.run(root)
        };
        drop(ctx);

        // Translate `MultiLayerOutcome` → `ReflectResponse`. Reflect
        // produces no follow-up layers (no AutoOnLoad), so
        // `outcome.layers` carries at most one entry — the trace layer.
        let user_persist_info = outcome.layers.first().map(|l| l.persist.clone());
        let branch_advanced = user_persist_info
            .as_ref()
            .map(|p| p.branch_advanced)
            .unwrap_or(false);
        let merge = Some(merge_info_from_persist_info(user_persist_info.as_ref()));

        if let Some(commit_err) = outcome.error.as_ref() {
            // Match the pre-D41 contract: reflect commit failures
            // surface as `Status::internal`. The pre-D41 path had two
            // exit shapes (`commit` failure → internal, `persist`
            // failure → internal); the new orchestrator collapses both
            // into `MultiLayerOutcome.error`, but the response shape
            // stays the same for backwards compatibility with notebook
            // clients that key on the gRPC status code.
            return Err(Status::internal(format!(
                "reflect commit failed: {commit_err}"
            )));
        }

        Ok(Response::new(ReflectResponse {
            success: true,
            trace_iri,
            branch_advanced,
            merge,
        }))
    }
}
