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

//! Tag RPC handlers (D34 §G.2 / §8): `CreateTag`, `ListTags`, `DeleteTag`.

use super::helpers::parse_layer_id;
use super::proto::*;
use super::EigeniusService;
use crate::observability::{operation, RpcGuard};
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_create_tag(
        &self,
        req: CreateTagRequest,
    ) -> Result<Response<CreateTagResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_CREATE_TAG);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("tag operations require a persistent backend")
        })?;

        // Validate the name matches the same lexical rules as branches
        // — surfacing this as `invalid_argument` rather than a soft
        // failure mirrors how `CreateBranch` rejects malformed names.
        if !crate::lattice::is_valid_ref_name(&req.name) {
            return Err(Status::invalid_argument(format!(
                "invalid tag name: {:?} (must match [A-Za-z0-9_-]+, max 256 chars)",
                req.name
            )));
        }

        let layer_id = parse_layer_id(&req.layer_id, "layer_id")?;

        // Verify the target layer exists in storage; tagging an
        // unknown id would create a dangling ref.
        match backend.load_handle(&layer_id) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Ok(Response::new(CreateTagResponse {
                    success: false,
                    error: format!("layer_id {} not in store", req.layer_id),
                    already_exists: false,
                }));
            }
            Err(e) => return Err(Status::internal(format!("load_handle failed: {e}"))),
        }

        match backend.create_tag(&req.name, &layer_id) {
            Ok(true) => Ok(Response::new(CreateTagResponse {
                success: true,
                error: String::new(),
                already_exists: false,
            })),
            Ok(false) => Ok(Response::new(CreateTagResponse {
                success: false,
                error: format!("tag {:?} already exists", req.name),
                already_exists: true,
            })),
            Err(e) => Err(Status::internal(format!("create_tag failed: {e}"))),
        }
    }

    pub(super) async fn handle_list_tags(
        &self,
        _req: ListTagsRequest,
    ) -> Result<Response<ListTagsResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LIST_TAGS);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("tag operations require a persistent backend")
        })?;
        let entries = backend
            .list_tags()
            .map_err(|e| Status::internal(format!("list_tags failed: {e}")))?;
        let tags = entries
            .into_iter()
            .map(|(name, layer_id)| {
                // One `load_handle` per tag — typical chains have a
                // small number of tags so the fan-out cost is
                // negligible; matches the `ListBranches` shape.
                let tagged_at_ms = backend
                    .load_handle(&layer_id)
                    .ok()
                    .flatten()
                    .map(|h| h.created_at)
                    .unwrap_or(0);
                TagInfo {
                    name,
                    layer_id: hex::encode(layer_id.0),
                    tagged_at_ms,
                }
            })
            .collect();
        Ok(Response::new(ListTagsResponse { tags }))
    }

    pub(super) async fn handle_delete_tag(
        &self,
        req: DeleteTagRequest,
    ) -> Result<Response<DeleteTagResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_DELETE_TAG);
        let backend = self.backend.as_ref().ok_or_else(|| {
            Status::failed_precondition("tag operations require a persistent backend")
        })?;
        if !crate::lattice::is_valid_ref_name(&req.name) {
            return Err(Status::invalid_argument(format!(
                "invalid tag name: {:?} (must match [A-Za-z0-9_-]+, max 256 chars)",
                req.name
            )));
        }
        match backend.delete_tag(&req.name) {
            Ok(deleted) => Ok(Response::new(DeleteTagResponse {
                success: true,
                error: String::new(),
                deleted,
            })),
            Err(e) => Err(Status::internal(format!("delete_tag failed: {e}"))),
        }
    }
}
