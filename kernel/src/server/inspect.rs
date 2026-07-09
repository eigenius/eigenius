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

//! Inspection-shaped RPCs: `Inspect`, `GetSchema`, `ListInstitutions`,
//! `Health`. All read-only, all small enough to share a file.

use super::helpers::*;
use super::proto::*;
use super::EigeniusService;
use crate::observability::{field, operation, RpcGuard};
use crate::ontology::Iri;
use std::sync::Arc;
use tonic::{Response, Status};

impl EigeniusService {
    pub(super) async fn handle_inspect(
        &self,
        req: InspectRequest,
    ) -> Result<Response<InspectResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_INSPECT);
        tracing::debug!(
            { field::OPERATION } = operation::RPC_INSPECT,
            { field::RESOURCE_IRI } = %req.iri,
            "inspect target"
        );
        let iri = Iri::parse(&req.iri)
            .map_err(|e| Status::invalid_argument(format!("invalid IRI: {e}")))?;

        let layer = self.resolve_read_layer(&req.at_layer, &req.branch).await?;
        match layer.resolve(&iri) {
            Some(resource) => Ok(Response::new(InspectResponse {
                found: true,
                resource: Self::serialize_resource(&resource),
            })),
            None => Ok(Response::new(InspectResponse {
                found: false,
                resource: Vec::new(),
            })),
        }
    }

    pub(super) async fn handle_get_schema(
        &self,
        req: GetSchemaRequest,
    ) -> Result<Response<GetSchemaResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_GET_SCHEMA);
        tracing::debug!(
            { field::OPERATION } = operation::RPC_GET_SCHEMA,
            { field::CLASS_IRI } = %req.class_iri,
            "get_schema target"
        );
        let class_iri = Iri::parse(&req.class_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid IRI: {e}")))?;

        let layer = self.resolve_read_layer(&req.at_layer, "").await?;
        match crate::program::schema::schema_for_class(&class_iri, &layer) {
            Ok((schema, _table)) => Ok(Response::new(GetSchemaResponse {
                success: true,
                json_schema: serde_json::to_string_pretty(&schema).unwrap_or_default(),
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(GetSchemaResponse {
                success: false,
                json_schema: String::new(),
                error: format!("{e}"),
            })),
        }
    }

    pub(super) async fn handle_list_institutions(
        &self,
        req: ListInstitutionsRequest,
    ) -> Result<Response<ListInstitutionsResponse>, Status> {
        let _guard = RpcGuard::start(operation::RPC_LIST_INSTITUTIONS);
        if !req.at_layer.is_empty() {
            let _ = self.resolve_read_layer(&req.at_layer, "").await?;
        }
        // institution list-from-index, enriched per D34 §G.8 / §9.2 so the
        // notebook's Institutions inspector renders the list-view +
        // detail panel from a single call. Each `InstitutionInfo`
        // carries: legacy `query_types` (kept for non-notebook
        // clients), runtime classification, the full QueryClass
        // declarations, and the Comorphism triads bound to this
        // institution (source / target classes resolved through the
        // referenced ExportFormat / ImportFormat).
        let index = Arc::clone(&*self.institution_index.read().await);
        let mut infos: Vec<InstitutionInfo> = index
            .institutions()
            .map(|inst| {
                // QueryClasses declared by this institution. Sorted by
                // IRI so the response is deterministic.
                let mut qcs: Vec<&crate::institution::registry::QueryClassEntry> = index
                    .query_classes()
                    .filter(|qc| qc.institution_ref == inst.iri)
                    .collect();
                qcs.sort_by(|a, b| a.iri.cmp(&b.iri));
                let query_types: Vec<String> = qcs
                    .iter()
                    .map(|qc| qc.query_class.as_str().to_string())
                    .collect();
                let query_classes: Vec<QueryClassDecl> = qcs
                    .iter()
                    .map(|qc| QueryClassDecl {
                        iri: qc.iri.as_str().to_string(),
                        query_class: qc.query_class.as_str().to_string(),
                        result_class: qc.result_class.as_str().to_string(),
                        query_handler: qc.query_handler.as_str().to_string(),
                        dispatch_roles: qc
                            .dispatch_roles
                            .iter()
                            .map(|r| dispatch_role_to_proto(*r) as i32)
                            .collect(),
                    })
                    .collect();

                // Comorphisms whose source OR target institution is
                // this one. The Comorphism resource itself doesn't
                // name an institution directly — we follow the
                // export_format.institution_ref / import_format.
                // institution_ref to attribute the triad. A single
                // comorphism that crosses institutions shows up under
                // both ends.
                let mut comorphisms: Vec<ComorphismDecl> = index
                    .comorphisms()
                    .filter(|c| {
                        let exp_inst = index
                            .export_format(&c.export_format)
                            .map(|f| f.institution_ref.clone());
                        let imp_inst = index
                            .import_format(&c.import_format)
                            .map(|f| f.institution_ref.clone());
                        exp_inst.as_ref() == Some(&inst.iri) || imp_inst.as_ref() == Some(&inst.iri)
                    })
                    .map(|c| {
                        let from_class = index
                            .export_format(&c.export_format)
                            .map(|f| f.from_class.as_str().to_string())
                            .unwrap_or_default();
                        let to_class = index
                            .import_format(&c.import_format)
                            .map(|f| f.to_class.as_str().to_string())
                            .unwrap_or_default();
                        ComorphismDecl {
                            iri: c.iri.as_str().to_string(),
                            from_class,
                            to_class,
                            transformation: c.transformation.as_str().to_string(),
                            exact: c.exact,
                        }
                    })
                    .collect();
                comorphisms.sort_by(|a, b| a.iri.cmp(&b.iri));

                InstitutionInfo {
                    iri: inst.iri.as_str().to_string(),
                    name: inst.name.clone(),
                    query_types,
                    runtime_kind: runtime_kind_to_proto(inst.runtime) as i32,
                    requires_environment: inst
                        .requires_environment
                        .as_ref()
                        .map(|i| i.as_str().to_string())
                        .unwrap_or_default(),
                    query_classes,
                    comorphisms,
                }
            })
            .collect();
        infos.sort_by(|a, b| a.iri.cmp(&b.iri));

        Ok(Response::new(ListInstitutionsResponse {
            institutions: infos,
        }))
    }

    pub(super) async fn handle_health(
        &self,
        _req: HealthRequest,
    ) -> Result<Response<HealthResponse>, Status> {
        // Guard fires at debug level — invisible at the default
        // `info` filter, so frequent probes don't add log noise but
        // remain inspectable when debugging readiness/liveness.
        let _guard = RpcGuard::start(operation::RPC_HEALTH);
        let ctx_arc = self.get_branch_context(DEFAULT_BRANCH).await?;
        let ctx = ctx_arc.read().await;
        let resource_count = ctx.head().iter_all_resources().count() as u64;

        // Layer count covers every layer the backend currently
        // tracks (including layers reachable only through other
        // branches and orphans pending GC). This is the "health of
        // the storage backend" view; `resource_count` above stays
        // head-rooted because it's a quick sanity check against the
        // active branch's working set. When no persistent backend is
        // attached (in-memory mode), the topology is empty — fall
        // back to `0`.
        let layer_count = match self.backend.as_ref() {
            Some(backend) => backend
                .load_topology()
                .map(|t| t.layer_count() as u64)
                .unwrap_or(0),
            None => 0,
        };

        // D21 §6 resume observability — populated by the resume
        // sweep when it's active.
        use std::sync::atomic::Ordering;
        let resume_in_progress = self.resume_state.in_progress.load(Ordering::SeqCst);
        let tasks_resuming = self.resume_state.remaining.load(Ordering::SeqCst);

        Ok(Response::new(HealthResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            layer_count,
            resource_count,
            resume_in_progress,
            tasks_resuming,
        }))
    }
}
