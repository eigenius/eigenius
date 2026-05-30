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

//! Garbage-collection RPC handlers (D34 §G.4 / §9.4): `EstimateGc`,
//! `RunGc`. Both run against the live root set assembled by
//! [`EigeniusService::gather_gc_roots`].

use super::proto::*;
use super::EigeniusService;
use crate::layer::LayerStorage;
use crate::observability::{operation, RpcGuard};
use std::sync::Arc;
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_estimate_gc(
        &self,
        _req: EstimateGcRequest,
    ) -> Result<Response<EstimateGcResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_ESTIMATE_GC);
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| Status::failed_precondition("GC requires a persistent backend"))?;

        let (roots, branch_pins, tag_pins, task_pins_count) =
            self.gather_gc_roots(backend.as_ref()).await?;

        let config = crate::gc::GcConfig::default();
        let stats = crate::gc::estimate(roots, &config, backend.as_ref())
            .map_err(|e| Status::internal(format!("gc estimate failed: {e}")))?;

        let eligible_layers = stats
            .layers_unreachable
            .saturating_sub(stats.layers_protected_by_age);
        Ok(Response::new(EstimateGcResponse {
            success: true,
            error: String::new(),
            eligible_layers,
            protected_by_age: stats.layers_protected_by_age,
            branch_pins,
            tag_pins,
            task_pins: task_pins_count,
            // Sum of `LayerHandle.byte_size` over the eligible set —
            // see `gc::SweepStats.bytes_reclaimable` for the
            // approximation contract (encoded resource bytes only;
            // bloom + topo + index overhead excluded). Layers
            // persisted by pre-byte-size kernels have `byte_size = 0`
            // via `#[serde(default)]`; the estimate under-counts for
            // them until they churn through GC + recommit.
            reclaimable_bytes: stats.bytes_reclaimable,
        }))
    }

    pub(super) async fn handle_run_gc(
        &self,
        _req: RunGcRequest,
    ) -> Result<Response<RunGcResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_RUN_GC);
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| Status::failed_precondition("GC requires a persistent backend"))?;

        let (roots, _branch_pins, _tag_pins, _task_pins) =
            self.gather_gc_roots(backend.as_ref()).await?;

        // The sweep needs cache handles so it can `evict_layer` on
        // each deletion. We construct a transient LayerStorage here
        // only to access the caches — production reads against this
        // chain use their own per-request storage.
        let storage = LayerStorage::with_persistent(Arc::clone(backend));
        let config = crate::gc::GcConfig::default();
        let stats = crate::gc::collect(
            roots,
            &config,
            storage.cache.as_ref(),
            storage.bloom_cache.as_ref(),
            backend.as_ref(),
        )
        .map_err(|e| Status::internal(format!("gc run failed: {e}")))?;

        Ok(Response::new(RunGcResponse {
            success: true,
            error: String::new(),
            layers_marked: stats.layers_marked,
            layers_unreachable: stats.layers_unreachable,
            layers_swept: stats.layers_swept,
            layers_protected_by_age: stats.layers_protected_by_age,
        }))
    }

    /// Build the GC root set from the live state: branch heads, tag
    /// targets, and non-terminal task pins. Returns the root set plus
    /// the three counts the RPC surfaces as protection accounting.
    pub(super) async fn gather_gc_roots(
        &self,
        backend: &dyn crate::storage::PersistentBackend,
    ) -> Result<(crate::gc::GcRoots, u64, u64, u64), Status> {
        let mut roots = crate::gc::GcRoots::from_branches(backend)
            .map_err(|e| Status::internal(format!("list_branches/tags failed: {e}")))?;
        let branch_pins = roots.branch_heads.len() as u64;
        let tag_pins = roots.tag_targets.len() as u64;

        // Task pins: non-terminal task records hold their `layer_head`
        // alive. Mirrors the DeleteBranch handler's pin-gather logic.
        let task_pins: Vec<crate::layer::LayerId> = if let Some(store) = self.task_store.as_ref() {
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
        let task_pins_count = task_pins.len() as u64;
        roots.task_pins = task_pins;
        Ok((roots, branch_pins, tag_pins, task_pins_count))
    }
}
