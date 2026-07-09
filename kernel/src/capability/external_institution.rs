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
//! digest, plus a per-procedure `(method_name, signature_iri)`
//! lookup populated from every `QueryClass.query_handler`,
//! `ExportFormat.procedure`, and `ImportFormat.procedure` anchored on
//! this institution) and dispatches all three institution boundary calls —
//! `extract_typed`, `reify`, and `query` — through the orchestrator's
//! `DispatchExternal` gRPC method. The orchestrator routes the call
//! into the substrate (Phase 19a's Docker-spawner + Julia worker for
//! the v1 backend), returning a CBOR-encoded output Resource that
//! flows back through the matching kernel-side boundary.
//!
//! The wire protocol does not distinguish among the three boundary
//! kinds — they all serialise the input Resource as CBOR, dispatch a
//! `(method_name, signature_iri)` pair, and decode a Resource on the
//! way back. Differentiation lives at the kernel boundary: `query`
//! returns a [`QueryOutcome`] with the substrate-captured partial
//! `RuntimeInvocation`; `extract_typed` wraps the response Resource
//! as `Val::ResourceVal`; `reify` requires `ResourceVal` input and
//! returns the response Resource verbatim.

use crate::context::ExecutionContext;
use crate::institution::error::InstitutionError;
use crate::institution::runtime::{Institution, QueryOutcome};
use crate::nbe::val::Val;
use crate::ontology::eigon_cbor;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::program::remote::OrchestratorTransport;
use crate::server::proto::component_executor_client::ComponentExecutorClient;
use crate::server::proto::DispatchExternalRequest;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    client: Arc<Mutex<ComponentExecutorClient<OrchestratorTransport>>>,
}

impl ExternalInstitution {
    pub fn new(
        institution_iri: Iri,
        env_iri: Iri,
        image_digest: String,
        language: String,
        handlers: BTreeMap<Iri, ExternalQueryHandler>,
        client: Arc<Mutex<ComponentExecutorClient<OrchestratorTransport>>>,
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
        resource: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        // The source-side D14 §9.3 step 2: an ExportFormat-anchored
        // procedure on this institution's runtime extracts a typed
        // payload from the chain-resident source resource. Wire-level
        // identical to `query` — both serialize the input Resource as
        // CBOR, route through `DispatchExternal`, and decode a
        // Resource on the way back. Wrapped here as `Val::ResourceVal`
        // because the kernel's typed middle (the comorphism's
        // transformation Component) operates on `Val`s.
        let resp = self.dispatch_substrate(procedure_iri, resource)?;
        Ok(Val::ResourceVal(Box::new(resp)))
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        value: &Val,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        // The target-side D14 §9.3 step 4: an ImportFormat-anchored
        // procedure on this institution's runtime constructs a
        // chain-typed resource from the typed payload. The substrate
        // wire protocol is Resource-shaped, so the only payloads we
        // can ship across are `Val::ResourceVal`. Anything else
        // surfaces as a typed kernel-side error so the comorphism
        // implementation bug is unmistakable.
        let input = match value {
            Val::ResourceVal(r) => r.as_ref().clone(),
            other => {
                return Err(InstitutionError::ComputationFailed(format!(
                    "ExternalInstitution `{}`: reify via `{procedure_iri}` requires a \
                     ResourceVal payload — the substrate dispatch wire only carries Eigon-CBOR \
                     resources. Got {other:?}",
                    self.institution_iri
                )));
            }
        };
        self.dispatch_substrate(procedure_iri, &input)
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        let (output, derivations, partial_invocation) =
            self.dispatch_substrate_with_invocation(procedure_iri, input)?;
        Ok(QueryOutcome {
            output,
            derivations,
            partial_invocation,
        })
    }
}

impl ExternalInstitution {
    /// Boundary-call dispatch shared by `query`, `extract_typed`, and
    /// `reify`. Marshals the input as Eigon-CBOR, routes through
    /// `DispatchExternal`, and decodes the response Resource. The
    /// substrate wire protocol does not distinguish between the three
    /// institution boundary kinds — it's the caller's job to wrap the
    /// returned Resource in the appropriate kernel-side type.
    fn dispatch_substrate(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
    ) -> Result<Resource, InstitutionError> {
        self.dispatch_substrate_with_invocation(procedure_iri, input)
            .map(|(output, _, _)| output)
    }

    /// Same as [`Self::dispatch_substrate`] but also returns the
    /// substrate-captured partial RuntimeInvocation (D26 §5.5 / D31
    /// §6.2). Only `query` (gated AutoOnLoad / OnDemand FIBER) needs
    /// the partial invocation today; extract_typed and reify discard
    /// it because the comorphism's audit trail rides on the
    /// enclosing program's trace, not on a per-step RuntimeInvocation.
    fn dispatch_substrate_with_invocation(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
    ) -> Result<(Resource, Vec<Resource>, Option<Resource>), InstitutionError> {
        let handler = self.handlers.get(procedure_iri).ok_or_else(|| {
            InstitutionError::UnknownType(format!(
                "external institution `{}` has no registered handler for procedure \
                 `{procedure_iri}` — every institution declaration anchored on this institution \
                 (QueryClass.query_handler / ExportFormat.procedure / ImportFormat.procedure) \
                 must reference a chain-resident RuntimeMethodSignature carrying a \
                 `runtime:method_name`",
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
        // pattern used by `RemoteComponent::execute` for remote IO
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

        // Decode each derivation CBOR into a chain-shaped Resource.
        // The kernel commit pipeline stamps the
        // `reflection:InstitutionEmittedDerivation` marker and the
        // linkage properties before committing — institutions are
        // responsible only for the domain-specific shape +
        // `canonical_proposition`. Empty list when the institution
        // emitted no derivations, which is the common case for
        // pass/fail-only gates (D52 §6).
        let mut derivations = Vec::with_capacity(resp.derivations_cbor.len());
        for (i, cbor) in resp.derivations_cbor.iter().enumerate() {
            let r = eigon_cbor::parse_resource_lenient(cbor).map_err(|e| {
                InstitutionError::ComputationFailed(format!(
                    "external dispatch returned non-Eigon derivation #{i} for \
                     `{procedure_iri}`: {e}"
                ))
            })?;
            derivations.push(r);
        }

        Ok((output, derivations, partial_invocation))
    }
}
