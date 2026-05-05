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

//! D31 §6 — `Institution` implementation for `runtime: external`
//! institutions.
//!
//! An `ExternalInstitution` holds the per-institution metadata
//! resolved from the chain at registration time (env IRI + image
//! digest, plus a `query_handler → (method_name, signature_iri)`
//! lookup) and dispatches `query` calls into the orchestrator's
//! `DispatchExternal` gRPC method. The orchestrator routes the call
//! into the substrate (Phase 19a's Docker-spawner + Julia worker for
//! the v1 backend), returning a CBOR-encoded output Resource that
//! flows back through the `Institution::query` boundary.
//!
//! Boundary methods (`extract_typed`, `reify`) currently return
//! `NotImplemented`. They land in 19a.6 alongside `IntervalArithmetic`
//! end-to-end, where extract/reify on an external runtime first
//! hits the kernel.

use crate::context::ExecutionContext;
use crate::institution::error::InstitutionError;
use crate::institution::runtime::{Institution, QueryOutcome};
use crate::nbe::val::Val;
use crate::ontology::eigon_cbor;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::server::proto::component_executor_client::ComponentExecutorClient;
use crate::server::proto::DispatchExternalRequest;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;

/// Per-`query_handler` dispatch metadata captured at registration
/// time. The orchestrator side of `DispatchExternal` reads
/// `signature_iri` to record provenance and `method_name` to resolve
/// the worker entry point.
#[derive(Debug, Clone)]
pub struct ExternalQueryHandler {
    /// Mirror-struct method symbol the worker resolves in `Main`.
    pub method_name: String,
    /// IRI of the `RuntimeMethodSignature` the dispatch satisfies.
    pub signature_iri: Iri,
}

/// `Institution` implementation that dispatches every `query` call
/// over gRPC to the orchestrator's `DispatchExternal` RPC.
pub struct ExternalInstitution {
    institution_iri: Iri,
    env_iri: Iri,
    image_digest: String,
    /// Language identifier (`"julia"`, `"python"`, …) read from the
    /// `RuntimeEnvironment.language` property at registration time.
    /// Forwarded on the wire so the orchestrator's substrate
    /// dispatcher routes to the matching `LanguageRuntime` without
    /// having to re-resolve the chain.
    language: String,
    /// Maps a `QueryClass.query_handler` IRI to the worker dispatch
    /// metadata. Populated at registration time when the index is
    /// rebuilt.
    handlers: BTreeMap<Iri, ExternalQueryHandler>,
    client: Arc<Mutex<ComponentExecutorClient<Channel>>>,
}

impl ExternalInstitution {
    pub fn new(
        institution_iri: Iri,
        env_iri: Iri,
        image_digest: String,
        language: String,
        handlers: BTreeMap<Iri, ExternalQueryHandler>,
        client: Arc<Mutex<ComponentExecutorClient<Channel>>>,
    ) -> Self {
        Self {
            institution_iri,
            env_iri,
            image_digest,
            language,
            handlers,
            client,
        }
    }
}

impl Institution for ExternalInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.institution_iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        _resource: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        // 19a.6 wires Comorphism dispatch into external runtimes; until
        // then the kernel orchestrates extract/reify directly only for
        // WASM and in-process institutions.
        Err(InstitutionError::NotImplemented(format!(
            "ExternalInstitution `{}` does not yet implement `extract_typed` for `{procedure_iri}`",
            self.institution_iri
        )))
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        _value: &Val,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "ExternalInstitution `{}` does not yet implement `reify` for `{procedure_iri}`",
            self.institution_iri
        )))
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        let handler = self.handlers.get(procedure_iri).ok_or_else(|| {
            InstitutionError::UnknownType(format!(
                "external institution `{}` has no registered handler for procedure `{procedure_iri}`",
                self.institution_iri
            ))
        })?;

        let invocation_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
        let request = DispatchExternalRequest {
            invocation_id,
            institution_iri: self.institution_iri.as_str().to_string(),
            env_iri: self.env_iri.as_str().to_string(),
            image_digest: self.image_digest.clone(),
            method_name: handler.method_name.clone(),
            signature_iri: handler.signature_iri.as_str().to_string(),
            input_resource_cbors: vec![eigon_cbor::serialize_resource(input)],
            language: self.language.clone(),
        };

        // Bridge sync trait method to the async gRPC client. Same
        // pattern used by `RemoteComponent::execute` for WASM IO
        // components — `program::remote::RemoteComponent`.
        let client = self.client.clone();
        let response = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut c = client.lock().await;
                c.dispatch_external(tonic::Request::new(request))
                    .await
                    .map_err(|e| {
                        InstitutionError::ComputationFailed(format!(
                            "DispatchExternal gRPC call failed: {e}"
                        ))
                    })
            })
        })?;

        let resp = response.into_inner();
        let output =
            eigon_cbor::parse_resource_lenient(&resp.output_resource_cbor).map_err(|e| {
                InstitutionError::ComputationFailed(format!(
                    "external dispatch returned non-Eigon output for `{procedure_iri}`: {e}"
                ))
            })?;

        // Substrate-captured partial RuntimeInvocation (D26 §5.5 /
        // D31 §6.2) — language, image_digest, started/completed
        // timestamps, numerical_metadata, optional dispatched_to. The
        // kernel commit pipeline folds this into a full
        // `RuntimeInvocation` resource by stamping the IRIs only it
        // knows (script ← signature_iri, environment ← env_iri,
        // inputs ← gated resource IRI, output ← Verdict IRI) per
        // [D31 §6.3](../../docs/design/d31-external-institution-lifecycle.md#63-verdict-commit-semantics).
        // Empty bytes from a non-conforming orchestrator surface as
        // `partial_invocation: None` rather than a parse error so the
        // gating itself still completes.
        let partial_invocation = if resp.runtime_invocation_partial_cbor.is_empty() {
            None
        } else {
            match eigon_cbor::parse_resource_lenient(&resp.runtime_invocation_partial_cbor) {
                Ok(r) => Some(r),
                Err(e) => {
                    return Err(InstitutionError::ComputationFailed(format!(
                        "external dispatch returned non-Eigon partial invocation for \
                         `{procedure_iri}`: {e}"
                    )));
                }
            }
        };

        Ok(QueryOutcome {
            output,
            partial_invocation,
        })
    }
}
