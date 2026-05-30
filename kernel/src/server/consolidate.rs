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

//! Chain-consolidation RPC handlers: `ConsolidateChain`,
//! `EstimateConsolidation`.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::layer::LayerStorage;
use crate::observability::{operation, RpcGuard};
use std::sync::Arc;
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_consolidate_chain(
        &self,
        req: ConsolidateChainRequest,
    ) -> Result<Response<ConsolidateChainResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_CONSOLIDATE_CHAIN);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("consolidation requires a persistent backend")
        })?;

        let branch = if req.branch.is_empty() {
            "main"
        } else {
            req.branch.as_str()
        };
        let from = parse_layer_id(&req.from_layer, "from_layer")?;
        let to = parse_layer_id(&req.to_layer, "to_layer")?;

        let storage = LayerStorage::with_persistent(Arc::clone(backend));
        let opts =
            build_consolidate_opts(&req.max_walk_entries, req.preserve_history, self).await?;

        match crate::layer::consolidate_chain(branch, from, to, opts, storage, backend.as_ref()) {
            Ok(outcome) => {
                // The inner call updated the backend's branch ref
                // (at-head) and/or the redirect map (below-head) but
                // never went through `ExecutionContext`. Drop the
                // cached ctx so the next read for this branch
                // rebuilds against the post-consolidation chain —
                // otherwise reads see the stale in-memory `Layer`
                // graph and a re-run candidate's parent walk goes
                // through orphaned ancestors.
                self.branch_contexts.invalidate(branch).await;
                Ok(Response::new(ConsolidateChainResponse {
                    success: true,
                    consolidated_layer: hex::encode(outcome.consolidated_layer.0),
                    collapsed_layer_count: outcome.collapsed_layer_count,
                    head_advanced: outcome.head_advanced,
                    error_kind: ConsolidateErrorKind::Unspecified as i32,
                    error: String::new(),
                    error_layer: String::new(),
                    error_count: 0,
                }))
            }
            Err(err) => Ok(Response::new(consolidate_error_to_response(err))),
        }
    }

    pub(super) async fn handle_estimate_consolidation(
        &self,
        req: EstimateConsolidationRequest,
    ) -> Result<Response<EstimateConsolidationResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_ESTIMATE_CONSOLIDATION);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("consolidation requires a persistent backend")
        })?;

        let branch = if req.branch.is_empty() {
            "main"
        } else {
            req.branch.as_str()
        };
        let from = parse_layer_id(&req.from_layer, "from_layer")?;
        let to = parse_layer_id(&req.to_layer, "to_layer")?;

        let storage = LayerStorage::with_persistent(Arc::clone(backend));
        let opts =
            build_consolidate_opts(&req.max_walk_entries, req.preserve_history, self).await?;

        match crate::layer::estimate_consolidation(
            branch,
            from,
            to,
            opts,
            storage,
            backend.as_ref(),
        ) {
            Ok(estimate) => Ok(Response::new(EstimateConsolidationResponse {
                success: true,
                predicted_consolidated_layer: hex::encode(estimate.predicted_consolidated_layer.0),
                collapsed_layer_count: estimate.collapsed_layer_count,
                predicted_walk_entries: estimate.predicted_walk_entries,
                actual_walk_entries: estimate.actual_walk_entries,
                error_kind: ConsolidateErrorKind::Unspecified as i32,
                error: String::new(),
                error_layer: String::new(),
                error_count: 0,
            })),
            Err(err) => Ok(Response::new(estimate_error_to_response(err))),
        }
    }
}
