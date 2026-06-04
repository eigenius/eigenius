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

//! `Query` RPC handler.
//!
//! Evaluates an EigenQL document against a per-branch
//! `ExecutionContext`, optionally committing FIBER INTO outputs
//! through the `CommitOrchestrator` under `WithRetroactive`.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::commit::persister::PersistedLayerInfo;
use crate::observability::{field, operation, RpcGuard};
use crate::ontology::eigon_cbor;
use crate::query;
use std::sync::Arc;
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_query(
        &self,
        req: QueryRequest,
    ) -> Result<Response<QueryResponse>, Status> {
        let mut guard = RpcGuard::start(operation::RPC_QUERY);
        tracing::debug!(
            { field::OPERATION } = operation::RPC_QUERY,
            { field::SIZE_BYTES } = req.eigenql.len(),
            "query payload"
        );
        let layer = self.resolve_read_layer(&req.at_layer, &req.branch).await?;
        let branch_name = resolve_branch_name(&req.branch).to_string();
        let ctx_arc = self.get_branch_context(&branch_name).await?;
        let index = Arc::clone(&*self.institution_index.read().await);
        let inst_runtime = Arc::clone(&*self.institution_runtime.read().await);
        let components = Arc::clone(&*self.components.read().await);

        let outcome = {
            let ctx = ctx_arc.read().await;
            let embedders_ref = self.embedders.as_ref();
            let runtime = query::evaluate::FiberRuntime {
                index: Some(&index),
                runtime: Some(&inst_runtime),
                components: Some(&components),
                overlay: None,
                ctx: Some(&ctx),
                similarity: None,
                embedders: Some(embedders_ref),
                embedding_cache: None,
                vector_segment_cache: None,
            };

            match query::execute_with_into(&req.eigenql, &layer, runtime) {
                Ok(o) => o,
                Err(errors) => {
                    let msgs: Vec<String> = errors.iter().map(|e| format!("{e}")).collect();
                    guard.fail("query_failed");
                    tracing::warn!(
                        { field::OPERATION } = operation::QUERY_EVALUATE,
                        { field::COUNT } = errors.len(),
                        { field::ERROR_MESSAGE } = %msgs.join("; "),
                        "query failed"
                    );
                    // Query parse/eval errored before any FIBER INTO
                    // commit could run — `merge` stays None (no CAS
                    // happened).
                    return Ok(Response::new(QueryResponse {
                        success: false,
                        document: Vec::new(),
                        content_type: String::new(),
                        error: format!("query error: {}", msgs.join("; ")),
                        output_resource_iris: Vec::new(),
                        branch_advanced: false,
                        merge: None,
                    }));
                }
            }
        };

        // FIBER ... INTO produced chain-bound resources — commit them
        // through the commit orchestrator (D14 §9.3 step 5
        // chain-reinsertion via EigenQL).
        //
        // `commit_attempted` distinguishes "this query just read"
        // (`merge` should be None) from "this query attempted a
        // FIBER INTO" (`merge` reports the CAS outcome). Without it,
        // every transient read would render as a misleading `cached`
        // badge in the notebook.
        //
        // Drop the unused per-query AutoOnLoad snapshots — per D41
        // §10 FIBER INTO commits through `WithRetroactive`, not
        // `WithInstitutions`, so the gated commit no longer needs the
        // institution index / runtime here. The snapshots are still
        // computed above for the `FiberRuntime` (read-side dispatch);
        // the commit path deliberately bypasses AutoOnLoad until INTO
        // opts back in. Revisit if a future INTO surface needs
        // institutional gating (D41 §10 table footnote).
        let _ = (&index, &inst_runtime);
        let mut branch_advanced = false;
        let mut user_persist_info: Option<PersistedLayerInfo> = None;
        let mut commit_attempted = false;
        let output_resource_iris: Vec<String> = if outcome.into_resources.is_empty() {
            Vec::new()
        } else {
            let iris: Vec<String> = outcome
                .into_resources
                .iter()
                .filter_map(|r| r.id().map(|i| i.as_str().to_string()))
                .collect();
            let mut ctx = ctx_arc.write().await;
            for r in &outcome.into_resources {
                if let Err(e) = ctx.add_resource(r.clone()) {
                    tracing::warn!(
                        { field::OPERATION } = operation::RPC_QUERY,
                        { field::ERROR_KIND } = "fiber_into_add_failed",
                        { field::ERROR_MESSAGE } = %e,
                        resource_iri = ?r.id(),
                        "failed to add FIBER INTO resource to chain layer"
                    );
                }
            }

            let working = match ctx.take_working("eigenql-into") {
                Ok(b) => b,
                Err(e) => {
                    guard.fail("eigenql_into_commit_failed");
                    return Ok(Response::new(QueryResponse {
                        success: false,
                        document: Vec::new(),
                        content_type: String::new(),
                        error: format!("FIBER INTO commit failed: {e}"),
                        output_resource_iris: Vec::new(),
                        branch_advanced: false,
                        merge: None,
                    }));
                }
            };

            let root = crate::commit::LayerEmission::from_builder(
                crate::commit::LayerRole::User,
                "eigenql-into",
                crate::commit::PipelineKind::WithRetroactive,
                crate::commit::EmissionKind::Child,
                working,
            );

            let commit_outcome = {
                let orchestrator = crate::commit::CommitOrchestrator {
                    ctx: &mut ctx,
                    pool: &self.commit_ws_pool,
                    persister: &*self.persister,
                    host: self as &dyn crate::commit::CommitHookHost,
                    branch: &branch_name,
                    policy: crate::lattice::CommitPolicy::default(),
                    institutions: None,
                    did_drain: crate::commit::CommitOrchestrator::default_did_drain(),
                };
                orchestrator.run(root)
            };
            drop(ctx);

            if let Some(commit_err) = commit_outcome.error.as_ref() {
                guard.fail("eigenql_into_commit_failed");
                let msg = format!("{commit_err}");
                tracing::warn!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::ERROR_KIND } = "eigenql_into_commit_failed",
                    { field::ERROR_MESSAGE } = %msg,
                    "FIBER INTO commit failed; surfacing error to caller"
                );
                return Ok(Response::new(QueryResponse {
                    success: false,
                    document: Vec::new(),
                    content_type: String::new(),
                    error: format!("FIBER INTO commit failed: {msg}"),
                    output_resource_iris: Vec::new(),
                    branch_advanced: false,
                    merge: None,
                }));
            }

            // Surface didPersist hook errors as warnings — the commit
            // stands either way (D41 §3.6).
            for layer_outcome in &commit_outcome.layers {
                for ve in &layer_outcome.hook_errors {
                    tracing::warn!(
                        { field::OPERATION } = operation::LAYER_COMMIT,
                        { field::ERROR_KIND } = "eigenql_into_hook_error",
                        { field::ERROR_MESSAGE } = %ve.message,
                        "FIBER INTO didPersist hook error"
                    );
                }
            }
            for ve in &commit_outcome.drain_hook_errors {
                tracing::warn!(
                    { field::OPERATION } = operation::LAYER_COMMIT,
                    { field::ERROR_KIND } = "eigenql_into_drain_hook_error",
                    { field::ERROR_MESSAGE } = %ve.message,
                    "FIBER INTO drain hook error"
                );
            }

            // Translate per-layer outcomes. INTO under `WithRetroactive`
            // emits no follow-ups, so `layers[0]` is the user layer.
            if let Some(user) = commit_outcome.layers.first() {
                commit_attempted = true;
                branch_advanced |= user.persist.branch_advanced;
                user_persist_info = Some(user.persist.clone());
            }
            iris
        };

        Ok(Response::new(QueryResponse {
            success: true,
            document: eigon_cbor::serialize_document(&outcome.document),
            content_type: "application/cbor".to_string(),
            error: String::new(),
            output_resource_iris,
            branch_advanced,
            merge: if commit_attempted {
                Some(merge_info_from_persist_info(user_persist_info.as_ref()))
            } else {
                None
            },
        }))
    }
}
