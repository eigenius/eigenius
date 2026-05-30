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

//! Branch / merge / resolution RPC handlers.
//!
//! Covers: `ListBranches`, `GetBranch`, `CreateBranch`, `DeleteBranch`,
//! `MergeBranches`, `PreviewMerge`, `PrepareMerge`, `PreviewCascade`,
//! `SubmitResolution`. The corresponding wire-encoding tests
//! (`prepare_merge_encoding_tests`) live at the bottom of this file
//! so the wire shape and the handler that produces it stay co-located.

use super::helpers::*;
use super::proto::{self, *};
use super::EigeniusService;
use crate::commit::persister::PersistedLayerInfo;
use crate::observability::{operation, RpcGuard};
use std::sync::Arc;
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_list_branches(
        &self,
        _req: ListBranchesRequest,
    ) -> Result<Response<ListBranchesResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LIST_BRANCHES);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;
        let branches = backend
            .list_branches()
            .map_err(|e| Status::internal(format!("list_branches failed: {e}")))?;
        let branches = branches
            .into_iter()
            .map(|(name, head)| {
                // One `load_handle` per branch — typical chains have
                // a small number of branches, so the fan-out cost is
                // negligible compared to forcing the client to call
                // `GetBranch` for each row to learn the commit time.
                let head_committed_at_ms = backend
                    .load_handle(&head)
                    .ok()
                    .flatten()
                    .map(|h| h.created_at)
                    .unwrap_or(0);
                BranchInfo {
                    name,
                    head_layer: hex::encode(head.0),
                    head_committed_at_ms,
                }
            })
            .collect();
        Ok(Response::new(ListBranchesResponse { branches }))
    }

    pub(super) async fn handle_get_branch(
        &self,
        req: GetBranchRequest,
    ) -> Result<Response<GetBranchResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_GET_BRANCH);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;
        match backend
            .get_branch(&req.name)
            .map_err(|e| Status::internal(format!("get_branch failed: {e}")))?
        {
            Some(head) => {
                // Look up the head's commit timestamp via its
                // `LayerHandle`. Missing handle (shouldn't happen for
                // a live branch ref, but GC corner cases exist)
                // reports `0` — the wire shape allows it.
                let head_committed_at_ms = backend
                    .load_handle(&head)
                    .ok()
                    .flatten()
                    .map(|h| h.created_at)
                    .unwrap_or(0);
                Ok(Response::new(GetBranchResponse {
                    found: true,
                    head_layer: hex::encode(head.0),
                    head_committed_at_ms,
                }))
            }
            None => Ok(Response::new(GetBranchResponse {
                found: false,
                head_layer: String::new(),
                head_committed_at_ms: 0,
            })),
        }
    }

    pub(super) async fn handle_create_branch(
        &self,
        req: CreateBranchRequest,
    ) -> Result<Response<CreateBranchResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_CREATE_BRANCH);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;
        // Validate from_layer is a known layer.
        let bytes = hex::decode(&req.from_layer)
            .map_err(|e| Status::invalid_argument(format!("from_layer not valid hex: {e}")))?;
        if bytes.len() != 32 {
            return Err(Status::invalid_argument(
                "from_layer must be a 32-byte SHA-256 (64 hex chars)",
            ));
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);
        let from_layer = crate::layer::LayerId(id);
        match backend.load_chain_from(&from_layer) {
            Ok(Some(_)) => {}
            Ok(None) | Err(crate::storage::StorageError::NotFound(_)) => {
                return Err(Status::not_found(format!(
                    "from_layer {} not in store",
                    req.from_layer
                )))
            }
            Err(e) => return Err(Status::internal(format!("load_chain_from failed: {e}"))),
        }

        let storage = crate::layer::LayerStorage::with_persistent(Arc::clone(backend));
        match crate::lattice::update_branch(
            &req.name,
            None,
            from_layer.clone(),
            crate::lattice::ConflictPolicy::StrictFastForward,
            storage,
            backend.as_ref(),
        ) {
            Ok(crate::lattice::UpdateOutcome::FastForward) => {
                Ok(Response::new(CreateBranchResponse {
                    success: true,
                    head_layer: hex::encode(from_layer.0),
                    error: String::new(),
                }))
            }
            Ok(_) => unreachable!(
                "CreateBranch passes None expected_old_head; only FastForward or error possible"
            ),
            Err(crate::lattice::BranchUpdateError::InvalidBranchName(_)) => {
                Err(Status::invalid_argument(format!(
                    "invalid branch name: {:?} (must match [A-Za-z0-9_-]+, max 256 chars)",
                    req.name
                )))
            }
            Err(crate::lattice::BranchUpdateError::StrictFastForwardViolation { .. }) => {
                // Branch already exists.
                Ok(Response::new(CreateBranchResponse {
                    success: false,
                    head_layer: String::new(),
                    error: format!("branch {:?} already exists", req.name),
                }))
            }
            Err(crate::lattice::BranchUpdateError::Storage(e)) => {
                Err(Status::internal(format!("storage error: {e}")))
            }
        }
    }

    pub(super) async fn handle_delete_branch(
        &self,
        req: DeleteBranchRequest,
    ) -> Result<Response<DeleteBranchResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_DELETE_BRANCH);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;

        // Gather active task pins for the CheckPins safety policy. With
        // force=true we skip this scan entirely.
        let pins: Vec<crate::layer::LayerId> = if req.force {
            Vec::new()
        } else if let Some(store) = self.task_store.as_ref() {
            let session_id = self.session.read().await.session_id;
            match store.list_tasks(&session_id) {
                Ok(records) => records
                    .into_iter()
                    .filter(|r| !r.status.is_terminal())
                    .map(|r| r.layer_head)
                    .collect(),
                Err(e) => return Err(Status::internal(format!("list_tasks failed: {e}"))),
            }
        } else {
            Vec::new()
        };

        let safety = if req.force {
            crate::lattice::PruneSafety::Force
        } else {
            crate::lattice::PruneSafety::CheckPins(&pins)
        };

        match crate::lattice::prune_branch(&req.name, safety, backend.as_ref()) {
            Ok(crate::lattice::PruneOutcome::Pruned { previous_head }) => {
                Ok(Response::new(DeleteBranchResponse {
                    success: true,
                    deleted: true,
                    previous_head: hex::encode(previous_head.0),
                    error: String::new(),
                }))
            }
            Ok(crate::lattice::PruneOutcome::NotFound) => Ok(Response::new(DeleteBranchResponse {
                success: true,
                deleted: false,
                previous_head: String::new(),
                error: String::new(),
            })),
            Err(crate::lattice::PruneError::InvalidBranchName(_)) => {
                Err(Status::invalid_argument(format!(
                    "invalid branch name: {:?} (must match [A-Za-z0-9_-]+, max 256 chars)",
                    req.name
                )))
            }
            Err(crate::lattice::PruneError::InUse { branch, head }) => {
                Ok(Response::new(DeleteBranchResponse {
                    success: false,
                    deleted: false,
                    previous_head: String::new(),
                    error: format!(
                        "branch {branch:?} is in use (head {head} matches an active task pin); pass force=true to delete anyway",
                    ),
                }))
            }
            Err(crate::lattice::PruneError::Storage(e)) => {
                Err(Status::internal(format!("storage error: {e}")))
            }
        }
    }

    pub(super) async fn handle_merge_branches(
        &self,
        req: MergeBranchesRequest,
    ) -> Result<Response<MergeBranchesResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_MERGE_BRANCHES);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;

        // Resolve both tips up front. The kernel returns
        // `failed_precondition` rather than `not_found` to match the
        // shape `create_branch` / `delete_branch` use — branches that
        // don't exist are a caller bug, not a missing resource.
        let source_tip = backend
            .get_branch(&req.source)
            .map_err(|e| Status::internal(format!("get_branch source failed: {e}")))?
            .ok_or_else(|| {
                Status::failed_precondition(format!("source branch {:?} not found", req.source))
            })?;
        let target_tip = backend
            .get_branch(&req.target)
            .map_err(|e| Status::internal(format!("get_branch target failed: {e}")))?
            .ok_or_else(|| {
                Status::failed_precondition(format!("target branch {:?} not found", req.target))
            })?;

        // Trivial: source and target already match — no-op, surface
        // as FastForward without invoking the CAS.
        if source_tip == target_tip {
            let info = PersistedLayerInfo {
                layer_id: target_tip.clone(),
                branch_advanced: false,
                merge_outcome: Some(crate::lattice::UpdateOutcome::FastForward),
                cache_hit_different_position: false,
            };
            return Ok(Response::new(MergeBranchesResponse {
                success: true,
                error: String::new(),
                merge: Some(merge_info_from_persist_info(Some(&info))),
                target_tip: hex::encode(target_tip.0),
            }));
        }

        let storage = crate::layer::LayerStorage::with_persistent(Arc::clone(backend));
        // `merge_branch_tips` (vs. `update_branch`) computes the
        // ancestor relationship between the two tips before deciding
        // whether to fast-forward, trivial-merge, or surface
        // NeedsWitnessedMerge — `update_branch`'s CAS-shortcut path
        // would happily overwrite the target with an unrelated source
        // tip and call it FastForward, dropping the target's history.
        let outcome = crate::lattice::merge_branch_tips(
            &req.target,
            source_tip.clone(),
            storage,
            backend.as_ref(),
        );
        match outcome {
            Ok(update) => {
                // FastForward covers two sub-cases (source-descends-
                // from-target advances the branch; target-already-
                // ahead leaves it unchanged) and the outcome itself
                // doesn't distinguish them. Re-read the branch tip
                // for the authoritative answer in all cases.
                let new_tip = match &update {
                    crate::lattice::UpdateOutcome::FastForward => backend
                        .get_branch(&req.target)
                        .map_err(|e| {
                            Status::internal(format!("get_branch after merge failed: {e}"))
                        })?
                        .ok_or_else(|| {
                            Status::internal(format!(
                                "branch {:?} disappeared after merge_branch_tips",
                                req.target
                            ))
                        })?,
                    crate::lattice::UpdateOutcome::TrivialMerge { merge_layer } => {
                        merge_layer.clone()
                    }
                    crate::lattice::UpdateOutcome::NeedsWitnessedMerge { .. } => target_tip,
                };
                let info = PersistedLayerInfo {
                    layer_id: new_tip.clone(),
                    branch_advanced: !matches!(
                        update,
                        crate::lattice::UpdateOutcome::NeedsWitnessedMerge { .. }
                    ),
                    merge_outcome: Some(update),
                    cache_hit_different_position: false,
                };
                Ok(Response::new(MergeBranchesResponse {
                    success: true,
                    error: String::new(),
                    merge: Some(merge_info_from_persist_info(Some(&info))),
                    target_tip: hex::encode(new_tip.0),
                }))
            }
            Err(e) => Ok(Response::new(MergeBranchesResponse {
                success: false,
                error: format!("{e}"),
                merge: None,
                target_tip: String::new(),
            })),
        }
    }

    pub(super) async fn handle_submit_resolution(
        &self,
        req: SubmitResolutionRequest,
    ) -> Result<Response<SubmitResolutionResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_SUBMIT_RESOLUTION);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;

        // Resolve branch tip + candidate head.
        let branch_tip = backend
            .get_branch(&req.branch)
            .map_err(|e| Status::internal(format!("get_branch failed: {e}")))?
            .ok_or_else(|| {
                Status::failed_precondition(format!("branch {:?} not found", req.branch))
            })?;
        let candidate_head = parse_layer_id(&req.candidate_head, "candidate_head")?;

        // Decode the wire resolutions / acks into kernel types.
        // Malformed shapes surface as `MALFORMED_RESOLUTION` with the
        // human-readable reason — the CLI/UI can render verbatim.
        let resolutions = match decode_resolutions(&req.resolutions) {
            Ok(r) => r,
            Err(reason) => {
                return Ok(Response::new(SubmitResolutionResponse {
                    success: false,
                    error: reason,
                    error_kind: proto::SubmitResolutionErrorKind::MalformedResolution as i32,
                    ..Default::default()
                }));
            }
        };
        let acks: Vec<crate::layer::merge::CascadeAck> = req
            .acknowledgments
            .iter()
            .map(|a| crate::layer::merge::CascadeAck {
                item_id: crate::layer::merge::CascadeItemId(a.item_id.clone()),
            })
            .collect();

        // Build the span between branch tip and candidate head.
        let topology = backend
            .load_topology()
            .map_err(|e| Status::internal(format!("load_topology failed: {e}")))?;
        let span = match crate::layer::merge::build_merge_span(
            &branch_tip,
            &candidate_head,
            &topology,
            backend.as_ref(),
        ) {
            Ok(s) => s,
            Err(crate::layer::merge::MergeError::NoCommonAncestor { .. }) => {
                return Ok(Response::new(SubmitResolutionResponse {
                    success: false,
                    error: format!(
                        "no common ancestor between branch tip and candidate head {}",
                        req.candidate_head
                    ),
                    error_kind: proto::SubmitResolutionErrorKind::NoCommonAncestor as i32,
                    ..Default::default()
                }));
            }
            Err(e) => {
                return Ok(submit_resolution_internal_error(format!(
                    "build_merge_span failed: {e}"
                )));
            }
        };

        // Apply the resolutions and commit the merge layer.
        let storage = crate::layer::LayerStorage::with_persistent(Arc::clone(backend));
        // D38 §4 — caller-supplied search-branch scope for the witness
        // resolver's fourth-tier walk. Empty list means span-only
        // (the pre-D38 default).
        let extra_branches: Vec<String> = req
            .witness_search_branches
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect();
        let merge_layer_id = match crate::layer::merge::merge_with_resolutions(
            &span,
            resolutions,
            acks,
            extra_branches,
            storage.clone(),
            backend.as_ref(),
        ) {
            Ok(crate::layer::merge::MergeOutcome::Merged { merge_layer }) => merge_layer,
            Ok(crate::layer::merge::MergeOutcome::NeedsResolution { .. }) => {
                // Shouldn't happen post-resolution-application — the
                // surface returns NeedsResolution only when
                // `resolutions` is empty. Surface as internal.
                return Ok(submit_resolution_internal_error(
                    "merge surface returned NeedsResolution despite supplied resolutions"
                        .to_string(),
                ));
            }
            Err(e) => return Ok(merge_error_to_submit_response(&e)),
        };

        // CAS-advance the branch ref. The merge layer's parents are
        // `[branch_tip, candidate_head]`, so advancing `branch` from
        // `branch_tip` to `merge_layer_id` is a fast-forward in DAG
        // terms — `update_branch` with `StrictFastForward` does
        // exactly the CAS without attempting a second merge.
        let cas = crate::lattice::update_branch(
            &req.branch,
            Some(branch_tip.clone()),
            merge_layer_id.clone(),
            crate::lattice::ConflictPolicy::StrictFastForward,
            storage,
            backend.as_ref(),
        );
        match cas {
            Ok(_) => Ok(Response::new(SubmitResolutionResponse {
                success: true,
                error: String::new(),
                error_kind: proto::SubmitResolutionErrorKind::Unspecified as i32,
                merge_layer_id: hex::encode(merge_layer_id.0),
                branch_tip: hex::encode(merge_layer_id.0),
                missing_acknowledgments: Vec::new(),
            })),
            Err(crate::lattice::BranchUpdateError::StrictFastForwardViolation { .. }) => {
                Ok(Response::new(SubmitResolutionResponse {
                    success: false,
                    error: format!(
                        "branch {:?} moved between resolution preview and commit; re-fetch and retry",
                        req.branch
                    ),
                    error_kind: proto::SubmitResolutionErrorKind::BranchCasRace as i32,
                    merge_layer_id: hex::encode(merge_layer_id.0),
                    ..Default::default()
                }))
            }
            Err(e) => Ok(submit_resolution_internal_error(format!(
                "branch CAS failed: {e}"
            ))),
        }
    }

    pub(super) async fn handle_preview_cascade(
        &self,
        req: PreviewCascadeRequest,
    ) -> Result<Response<PreviewCascadeResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_PREVIEW_CASCADE);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;

        let branch_tip = backend
            .get_branch(&req.branch)
            .map_err(|e| Status::internal(format!("get_branch failed: {e}")))?
            .ok_or_else(|| {
                Status::failed_precondition(format!("branch {:?} not found", req.branch))
            })?;
        let candidate_head = parse_layer_id(&req.candidate_head, "candidate_head")?;

        let resolutions = match decode_resolutions(&req.resolutions) {
            Ok(r) => r,
            Err(reason) => {
                return Ok(Response::new(PreviewCascadeResponse {
                    success: false,
                    error: reason,
                    error_kind: proto::PreviewCascadeErrorKind::MalformedResolution as i32,
                    items: Vec::new(),
                }));
            }
        };

        let topology = backend
            .load_topology()
            .map_err(|e| Status::internal(format!("load_topology failed: {e}")))?;
        let span = match crate::layer::merge::build_merge_span(
            &branch_tip,
            &candidate_head,
            &topology,
            backend.as_ref(),
        ) {
            Ok(s) => s,
            Err(crate::layer::merge::MergeError::NoCommonAncestor { .. }) => {
                return Ok(Response::new(PreviewCascadeResponse {
                    success: false,
                    error: format!(
                        "no common ancestor between branch tip and candidate head {}",
                        req.candidate_head
                    ),
                    error_kind: proto::PreviewCascadeErrorKind::NoCommonAncestor as i32,
                    items: Vec::new(),
                }));
            }
            Err(e) => {
                return Ok(Response::new(PreviewCascadeResponse {
                    success: false,
                    error: format!("build_merge_span failed: {e}"),
                    error_kind: proto::PreviewCascadeErrorKind::Internal as i32,
                    items: Vec::new(),
                }));
            }
        };

        let preview =
            match crate::layer::merge::preview_cascade(&span, &resolutions, backend.as_ref()) {
                Ok(p) => p,
                Err(e) => {
                    let kind = match e {
                        crate::layer::merge::MergeError::ConflictNotFound(_) => {
                            proto::PreviewCascadeErrorKind::ConflictNotFound
                        }
                        _ => proto::PreviewCascadeErrorKind::Internal,
                    };
                    return Ok(Response::new(PreviewCascadeResponse {
                        success: false,
                        error: e.to_string(),
                        error_kind: kind as i32,
                        items: Vec::new(),
                    }));
                }
            };

        Ok(Response::new(PreviewCascadeResponse {
            success: true,
            error: String::new(),
            error_kind: proto::PreviewCascadeErrorKind::Unspecified as i32,
            items: preview.items.iter().map(encode_cascade_item).collect(),
        }))
    }

    pub(super) async fn handle_prepare_merge(
        &self,
        req: PrepareMergeRequest,
    ) -> Result<Response<PrepareMergeResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_PREPARE_MERGE);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;

        let branch_tip = backend
            .get_branch(&req.branch)
            .map_err(|e| Status::internal(format!("get_branch failed: {e}")))?
            .ok_or_else(|| {
                Status::failed_precondition(format!("branch {:?} not found", req.branch))
            })?;
        let candidate_head = parse_layer_id(&req.candidate_head, "candidate_head")?;

        let topology = backend
            .load_topology()
            .map_err(|e| Status::internal(format!("load_topology failed: {e}")))?;

        let span = match crate::layer::merge::build_merge_span(
            &branch_tip,
            &candidate_head,
            &topology,
            backend.as_ref(),
        ) {
            Ok(s) => s,
            Err(crate::layer::merge::MergeError::NoCommonAncestor { .. }) => {
                return Ok(Response::new(PrepareMergeResponse {
                    success: false,
                    error: format!(
                        "no common ancestor between branch tip and candidate head {}",
                        req.candidate_head
                    ),
                    error_kind: proto::PrepareMergeErrorKind::NoCommonAncestor as i32,
                    conflicts: Vec::new(),
                    branch_tip: hex::encode(branch_tip.0),
                }));
            }
            Err(e) => {
                return Ok(Response::new(PrepareMergeResponse {
                    success: false,
                    error: format!("build_merge_span failed: {e}"),
                    error_kind: proto::PrepareMergeErrorKind::Internal as i32,
                    conflicts: Vec::new(),
                    branch_tip: hex::encode(branch_tip.0),
                }));
            }
        };

        let conflicts = match crate::layer::merge::classify_conflicts(&span, backend.as_ref()) {
            Ok(c) => c,
            Err(e) => {
                return Ok(Response::new(PrepareMergeResponse {
                    success: false,
                    error: format!("classify_conflicts failed: {e}"),
                    error_kind: proto::PrepareMergeErrorKind::Internal as i32,
                    conflicts: Vec::new(),
                    branch_tip: hex::encode(branch_tip.0),
                }));
            }
        };

        Ok(Response::new(PrepareMergeResponse {
            success: true,
            error: String::new(),
            error_kind: proto::PrepareMergeErrorKind::Unspecified as i32,
            conflicts: conflicts.iter().map(encode_typed_conflict).collect(),
            branch_tip: hex::encode(branch_tip.0),
        }))
    }

    pub(super) async fn handle_preview_merge(
        &self,
        req: PreviewMergeRequest,
    ) -> Result<Response<PreviewMergeResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_PREVIEW_MERGE);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("branch operations require a persistent backend")
        })?;

        let source_tip = backend
            .get_branch(&req.source)
            .map_err(|e| Status::internal(format!("get_branch source failed: {e}")))?
            .ok_or_else(|| {
                Status::failed_precondition(format!("source branch {:?} not found", req.source))
            })?;
        let target_tip = backend
            .get_branch(&req.target)
            .map_err(|e| Status::internal(format!("get_branch target failed: {e}")))?
            .ok_or_else(|| {
                Status::failed_precondition(format!("target branch {:?} not found", req.target))
            })?;

        // Same trivial-case short-circuit as `MergeBranches`: if the
        // two tips already match, the predicted outcome is
        // FastForward without invoking any lattice work.
        if source_tip == target_tip {
            return Ok(Response::new(PreviewMergeResponse {
                success: true,
                error: String::new(),
                merge: Some(proto::MergeInfo {
                    outcome: proto::MergeOutcome::FastForward as i32,
                    ..Default::default()
                }),
                predicted_iri_count: 0,
            }));
        }

        match crate::lattice::preview_merge_independent_heads(
            vec![target_tip.clone(), source_tip.clone()],
            backend.as_ref(),
        ) {
            Ok(crate::lattice::MergePreview::FastForward) => {
                Ok(Response::new(PreviewMergeResponse {
                    success: true,
                    error: String::new(),
                    merge: Some(proto::MergeInfo {
                        outcome: proto::MergeOutcome::FastForward as i32,
                        ..Default::default()
                    }),
                    predicted_iri_count: 0,
                }))
            }
            Ok(crate::lattice::MergePreview::Disjoint { iri_count }) => {
                Ok(Response::new(PreviewMergeResponse {
                    success: true,
                    error: String::new(),
                    merge: Some(proto::MergeInfo {
                        outcome: proto::MergeOutcome::TrivialMerge as i32,
                        ..Default::default()
                    }),
                    predicted_iri_count: iri_count.min(u32::MAX as usize) as u32,
                }))
            }
            Ok(crate::lattice::MergePreview::Conflict { conflicting_iris }) => {
                Ok(Response::new(PreviewMergeResponse {
                    success: true,
                    error: String::new(),
                    merge: Some(proto::MergeInfo {
                        outcome: proto::MergeOutcome::NeedsWitnessedMerge as i32,
                        merge_layer_id: String::new(),
                        conflicting_iris: conflicting_iris
                            .iter()
                            .map(|iri| iri.as_str().to_string())
                            .collect(),
                        current_head: hex::encode(target_tip.0),
                        // Preview doesn't have an orphan — no layer has
                        // been built. The dialog uses this preview to
                        // decide whether to attempt the merge at all;
                        // an actual `MergeBranches` is what materialises
                        // the orphan-on-conflict.
                        orphan_layer_id: String::new(),
                    }),
                    predicted_iri_count: 0,
                }))
            }
            Err(e) => Ok(Response::new(PreviewMergeResponse {
                success: false,
                error: format!("{e}"),
                merge: None,
                predicted_iri_count: 0,
            })),
        }
    }
}

#[cfg(test)]
mod prepare_merge_encoding_tests {
    //! Pin the `encode_typed_conflict` + `applicable_strategies_for`
    //! wire-encoding (D36 §3.1). The handler is a thin wrapper around
    //! `build_merge_span` + `classify_conflicts` + this encoder — the
    //! kernel-internal merge tests already pin the classifier; these
    //! tests pin the wire shape so a future enum-variant churn breaks
    //! the build at the right layer.
    use super::super::helpers::*;
    use super::super::proto;
    use crate::layer::merge::{
        ConflictId, ConflictKind, ResourceBody, ResourceKind, TypedConflict,
    };
    use crate::layer::LayerId;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};

    fn iri(s: &str) -> Iri {
        Iri::parse(s).expect("test IRI parses")
    }

    fn lid(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }

    fn make_body(id: &str, props: &[(&str, Value)]) -> Resource {
        let mut r = Resource::new(iri(id));
        for (k, v) in props {
            r.set(iri(k), v.clone());
        }
        r
    }

    #[test]
    fn encode_property_data_type_round_trips_all_fields() {
        let conflict = TypedConflict {
            id: ConflictId("pdt:urn:test:weight".to_string()),
            kind: ConflictKind::PropertyDataType {
                property: iri("urn:test:weight"),
                branch_a: iri("urn:eigenius:core:integer"),
                branch_b: iri("urn:eigenius:core:string"),
                ancestor: Some(iri("urn:eigenius:core:integer")),
            },
        };
        let wire = encode_typed_conflict(&conflict);
        assert_eq!(wire.id, "pdt:urn:test:weight");
        match wire.kind {
            Some(proto::typed_conflict_wire::Kind::PropertyDataType(p)) => {
                assert_eq!(p.property, "urn:test:weight");
                assert_eq!(p.branch_a_type, "urn:eigenius:core:integer");
                assert_eq!(p.branch_b_type, "urn:eigenius:core:string");
                assert_eq!(p.ancestor_type, "urn:eigenius:core:integer");
            }
            other => panic!("expected PropertyDataType, got {other:?}"),
        }
    }

    #[test]
    fn encode_property_data_type_handles_branch_introduced() {
        // ancestor = None when the property was introduced on the
        // branches (no pre-divergence value). The wire's
        // `ancestor_type` is the empty string.
        let conflict = TypedConflict {
            id: ConflictId("pdt:urn:test:weight".to_string()),
            kind: ConflictKind::PropertyDataType {
                property: iri("urn:test:weight"),
                branch_a: iri("urn:eigenius:core:integer"),
                branch_b: iri("urn:eigenius:core:string"),
                ancestor: None,
            },
        };
        let wire = encode_typed_conflict(&conflict);
        match wire.kind {
            Some(proto::typed_conflict_wire::Kind::PropertyDataType(p)) => {
                assert!(p.ancestor_type.is_empty());
            }
            other => panic!("expected PropertyDataType, got {other:?}"),
        }
    }

    #[test]
    fn encode_kind_mismatch_renders_resource_kinds() {
        let conflict = TypedConflict {
            id: ConflictId("kind:urn:test:X".to_string()),
            kind: ConflictKind::KindMismatch {
                iri: iri("urn:test:X"),
                branch_a_kind: ResourceKind::Class,
                branch_b_kind: ResourceKind::Property,
            },
        };
        let wire = encode_typed_conflict(&conflict);
        match wire.kind {
            Some(proto::typed_conflict_wire::Kind::KindMismatch(k)) => {
                assert_eq!(k.iri, "urn:test:X");
                assert_eq!(k.branch_a_kind, "Class");
                assert_eq!(k.branch_b_kind, "Property");
            }
            other => panic!("expected KindMismatch, got {other:?}"),
        }
    }

    #[test]
    fn encode_iri_collision_serialises_bodies_as_eigon_json() {
        let body_a = make_body("urn:test:X", &[("urn:test:weight", Value::Integer(75))]);
        let body_b = make_body("urn:test:X", &[("urn:test:weight", Value::Integer(76))]);
        let ancestor_body = make_body("urn:test:X", &[("urn:test:weight", Value::Integer(0))]);
        let conflict = TypedConflict {
            id: ConflictId("collision:urn:test:X".to_string()),
            kind: ConflictKind::IriCollision {
                iri: iri("urn:test:X"),
                branch_a_body: ResourceBody {
                    source_layer: lid(0xAA),
                    resource: body_a,
                },
                branch_b_body: ResourceBody {
                    source_layer: lid(0xBB),
                    resource: body_b,
                },
                ancestor_body: Some(ResourceBody {
                    source_layer: lid(0xCC),
                    resource: ancestor_body,
                }),
            },
        };
        let wire = encode_typed_conflict(&conflict);
        match wire.kind {
            Some(proto::typed_conflict_wire::Kind::IriCollision(c)) => {
                assert_eq!(c.iri, "urn:test:X");
                // Bodies round-trip as Eigon-JSON; the diff-renderer
                // on the notebook side parses them. We pin that the
                // serialization is non-empty and contains the value;
                // exact byte match would be brittle against
                // serde_json's whitespace defaults.
                assert!(c.branch_a_body_json.contains("75"));
                assert!(c.branch_b_body_json.contains("76"));
                assert!(c.ancestor_body_json.contains('0'));
            }
            other => panic!("expected IriCollision, got {other:?}"),
        }
    }

    #[test]
    fn encode_iri_collision_empty_ancestor_when_branch_introduced() {
        let body_a = make_body("urn:test:X", &[]);
        let body_b = make_body("urn:test:X", &[]);
        let conflict = TypedConflict {
            id: ConflictId("collision:urn:test:X".to_string()),
            kind: ConflictKind::IriCollision {
                iri: iri("urn:test:X"),
                branch_a_body: ResourceBody {
                    source_layer: lid(0xAA),
                    resource: body_a,
                },
                branch_b_body: ResourceBody {
                    source_layer: lid(0xBB),
                    resource: body_b,
                },
                ancestor_body: None,
            },
        };
        let wire = encode_typed_conflict(&conflict);
        match wire.kind {
            Some(proto::typed_conflict_wire::Kind::IriCollision(c)) => {
                assert!(c.ancestor_body_json.is_empty());
            }
            other => panic!("expected IriCollision, got {other:?}"),
        }
    }

    #[test]
    fn encode_inheritance_cycle_preserves_cycle_order() {
        let conflict = TypedConflict {
            id: ConflictId("cycle:a,b,c".to_string()),
            kind: ConflictKind::InheritanceCycle {
                cycle: vec![iri("urn:test:A"), iri("urn:test:B"), iri("urn:test:C")],
            },
        };
        let wire = encode_typed_conflict(&conflict);
        match wire.kind {
            Some(proto::typed_conflict_wire::Kind::InheritanceCycle(c)) => {
                assert_eq!(
                    c.cycle,
                    vec![
                        "urn:test:A".to_string(),
                        "urn:test:B".to_string(),
                        "urn:test:C".to_string(),
                    ]
                );
            }
            other => panic!("expected InheritanceCycle, got {other:?}"),
        }
    }

    #[test]
    fn applicable_strategies_for_classified_kinds_excludes_keep_both() {
        // Every v1-classified kind is single-valued or
        // mutually-exclusive; `KeepBoth` is structurally inapplicable
        // and must be absent from `applicable_strategies`.
        let pdt = ConflictKind::PropertyDataType {
            property: iri("urn:test:p"),
            branch_a: iri("urn:eigenius:core:integer"),
            branch_b: iri("urn:eigenius:core:string"),
            ancestor: None,
        };
        let strategies = applicable_strategies_for(&pdt);
        assert!(strategies.contains(&proto::MergeStrategyKind::Witness));
        assert!(strategies.contains(&proto::MergeStrategyKind::Rename));
        assert!(strategies.contains(&proto::MergeStrategyKind::KeepOne));
        assert!(strategies.contains(&proto::MergeStrategyKind::KeepNeither));
        assert!(strategies.contains(&proto::MergeStrategyKind::Restructure));
        assert!(
            !strategies.contains(&proto::MergeStrategyKind::KeepBoth),
            "KeepBoth must not be applicable to PropertyDataType"
        );
    }

    #[test]
    fn applicable_strategies_for_inheritance_cycle_matches_classified_shape() {
        let cycle = ConflictKind::InheritanceCycle {
            cycle: vec![iri("urn:test:A"), iri("urn:test:B")],
        };
        let strategies = applicable_strategies_for(&cycle);
        assert!(strategies.contains(&proto::MergeStrategyKind::KeepOne));
        assert!(!strategies.contains(&proto::MergeStrategyKind::KeepBoth));
    }

    #[test]
    fn decode_resolutions_restructure_with_new_parent_def_round_trips() {
        // Wire shape carrying an inline Eigon-JSON Class for a fresh
        // parent should decode into a `MergeResolution::Restructure`
        // with a `new_parent_def: Some(Resource)`.
        use crate::layer::merge::MergeResolution;

        let def_json = serde_json::json!({
            "@id": "urn:test:Animal",
            "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
            "urn:eigenius:core:short_name": "Animal",
            "urn:eigenius:core:description": "A common parent."
        });
        let wire = vec![proto::MergeResolutionWire {
            conflict_id: "subclass_conflict:urn:test:Dog".to_string(),
            strategy: Some(proto::merge_resolution_wire::Strategy::Restructure(
                proto::RestructureStrategy {
                    affected_class: "urn:test:Dog".to_string(),
                    new_parent: "urn:test:Animal".to_string(),
                    new_parent_def_json: def_json.to_string(),
                    classes_under_new: vec![
                        "urn:test:Mammal".to_string(),
                        "urn:test:Reptile".to_string(),
                    ],
                    affected_class_under_new: true,
                },
            )),
        }];
        let resolutions = decode_resolutions(&wire).expect("decode succeeds");
        assert_eq!(resolutions.len(), 1);
        match &resolutions[0] {
            MergeResolution::Restructure { conflict, spec } => {
                assert_eq!(conflict.0, "subclass_conflict:urn:test:Dog");
                assert_eq!(spec.affected_class.as_str(), "urn:test:Dog");
                assert_eq!(spec.new_parent.as_str(), "urn:test:Animal");
                assert!(spec.new_parent_def.is_some());
                let def = spec.new_parent_def.as_ref().unwrap();
                assert_eq!(def.id().map(|i| i.as_str()), Some("urn:test:Animal"));
                assert_eq!(spec.classes_under_new.len(), 2);
                assert!(spec.affected_class_under_new);
            }
            other => panic!("expected Restructure, got {other:?}"),
        }
    }

    #[test]
    fn decode_resolutions_restructure_empty_def_means_attach_existing() {
        // Empty `new_parent_def_json` decodes to `None` — the
        // resolution is attaching to a parent that already exists
        // in the chain. The kernel's apply path validates the
        // presence rule (`RestructureParentMissingDefinition` /
        // `RestructureParentRedeclaration`).
        use crate::layer::merge::MergeResolution;

        let wire = vec![proto::MergeResolutionWire {
            conflict_id: "subclass_conflict:urn:test:Dog".to_string(),
            strategy: Some(proto::merge_resolution_wire::Strategy::Restructure(
                proto::RestructureStrategy {
                    affected_class: "urn:test:Dog".to_string(),
                    new_parent: "urn:test:Mammal".to_string(),
                    new_parent_def_json: String::new(),
                    classes_under_new: vec!["urn:test:Reptile".to_string()],
                    affected_class_under_new: false,
                },
            )),
        }];
        let resolutions = decode_resolutions(&wire).expect("decode succeeds");
        match &resolutions[0] {
            MergeResolution::Restructure { spec, .. } => {
                assert!(spec.new_parent_def.is_none());
                assert!(!spec.affected_class_under_new);
            }
            other => panic!("expected Restructure, got {other:?}"),
        }
    }

    #[test]
    fn decode_resolutions_restructure_rejects_malformed_def_json() {
        // Garbage JSON in the def field surfaces as a typed decode
        // error so the handler returns `MALFORMED_RESOLUTION` to the
        // client — better than silently ignoring or panicking.
        let wire = vec![proto::MergeResolutionWire {
            conflict_id: "x".to_string(),
            strategy: Some(proto::merge_resolution_wire::Strategy::Restructure(
                proto::RestructureStrategy {
                    affected_class: "urn:test:Dog".to_string(),
                    new_parent: "urn:test:Animal".to_string(),
                    new_parent_def_json: "not-valid-json}".to_string(),
                    classes_under_new: vec![],
                    affected_class_under_new: true,
                },
            )),
        }];
        let result = decode_resolutions(&wire);
        match result {
            Err(reason) => {
                assert!(
                    reason.contains("new_parent_def_json"),
                    "diagnostic should mention the field; got {reason:?}"
                );
            }
            Ok(_) => panic!("expected decode failure on malformed JSON"),
        }
    }
}
