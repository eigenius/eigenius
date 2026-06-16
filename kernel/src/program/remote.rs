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

//! Remote component dispatch via gRPC.
//!
//! When the kernel evaluates a program that references an IO component
//! not in the local registry, it dispatches the call to the orchestrator
//! via the ComponentExecutor gRPC service.

use crate::layer::Layer;
use crate::ontology::eigon_cbor;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::program::component::{BuiltinComponent, ComponentResult};
use crate::program::trace::ComponentMetrics;
use crate::server::proto::component_executor_client::ComponentExecutorClient;
use crate::server::proto::ComponentRequest;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::rt::TokioExecutor;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::body::Body as TonicBody;
use tonic_web::{GrpcWebCall, GrpcWebClientService};

/// Transport the kernel uses to reach the orchestrator's
/// `ComponentExecutor` service.
///
/// The kernel speaks **gRPC-Web**, not native gRPC, on this hop. The
/// orchestrator serves the service via connect-es on `Deno.serve`,
/// whose Fetch `Response` cannot carry HTTP/2 trailers — and native
/// gRPC delivers its terminal `grpc-status` *as* a trailer. With no
/// trailer on the wire, a tonic ≥0.14 client treats the stream as
/// truncated and fails the call with "missing grpc-status trailer"
/// (confirmed against the running stack: the orchestrator logs the LLM
/// completion, then the kernel rejects the trailer-less response).
/// gRPC-Web instead frames the trailing status as the final length-
/// prefixed frame of the response *body*, which Deno delivers
/// faithfully. (This mirrors the orchestrator→kernel direction, which
/// already uses a gRPC-Web transport for the same reason.)
///
/// tonic's own `Channel` can't sit underneath the gRPC-Web adapter — it
/// is itself a complete native-gRPC transport — so the adapter wraps a
/// bare hyper HTTP/1.1 client. HTTP/1.1 (not h2c) matches the transport
/// the orchestrator's reverse-direction client already uses.
pub type OrchestratorTransport =
    GrpcWebClientService<HyperClient<HttpConnector, GrpcWebCall<TonicBody>>>;

/// Content-type tag emitted on every outbound `ComponentRequest`. The
/// orchestrator's `component_executor.ts` branches on this to pick its
/// codec and echoes the same value on the response. D26 §8.1 / Phase
/// 18e — the kernel ↔ orchestrator boundary is now CBOR; the proto's
/// `content_type` field has carried the codec tag since day one.
pub const EIGON_CBOR_CONTENT_TYPE: &str = "application/eigon+cbor";

/// A component that dispatches execution to a remote orchestrator
/// via the ComponentExecutor gRPC service.
pub struct RemoteComponent {
    component_iri: String,
    client: Arc<Mutex<ComponentExecutorClient<OrchestratorTransport>>>,
}

impl RemoteComponent {
    pub fn new(
        component_iri: String,
        client: Arc<Mutex<ComponentExecutorClient<OrchestratorTransport>>>,
    ) -> Self {
        Self {
            component_iri,
            client,
        }
    }
}

/// Property naming a component_argument's auxiliary inputs for a multi-file
/// join (D53 §4.3): a list of resource IRIs (typically PinnedExternalFiles)
/// shipped to the worker alongside the primary input.
const RUNTIME_ADDITIONAL_INPUTS: &str = "urn:eigenius:runtime:additional_inputs";

/// Resolve `runtime:additional_inputs` (a list of resource IRIs) on a
/// component_argument into Eigon-CBOR-serialized resources, preserving declared
/// order. Fails closed if an entry is malformed or absent from the chain.
fn resolve_additional_inputs(argument: &Resource, layer: &Layer) -> Result<Vec<Vec<u8>>, String> {
    let prop = Iri::parse(RUNTIME_ADDITIONAL_INPUTS).expect("static IRI");
    let value_iri = |v: &Value| -> Option<String> {
        match v {
            Value::String(s) => Some(s.clone()),
            other => other.as_iri_str().map(str::to_string),
        }
    };
    let iris: Vec<String> = match argument.get(&prop) {
        None => return Ok(Vec::new()),
        Some(Value::Array(items)) => items.iter().filter_map(value_iri).collect(),
        Some(v) => value_iri(v).into_iter().collect(),
    };
    let mut out = Vec::with_capacity(iris.len());
    for iri_str in iris {
        let iri = Iri::parse(&iri_str).map_err(|_| {
            format!("runtime:additional_inputs entry `{iri_str}` is not a valid IRI")
        })?;
        let res = layer.resolve(&iri).ok_or_else(|| {
            format!("runtime:additional_inputs `{iri_str}` not found on the chain")
        })?;
        out.push(eigon_cbor::serialize_resource(&res));
    }
    Ok(out)
}

impl BuiltinComponent for RemoteComponent {
    fn is_io(&self) -> bool {
        true
    }

    fn execute(
        &self,
        input: &Resource,
        argument: Option<&Resource>,
        layer: &Layer,
    ) -> Result<ComponentResult, String> {
        // Serialize input and argument to Eigon-CBOR (D26 §8.1 / Phase 18e).
        let input_cbor = eigon_cbor::serialize_resource(input);
        let argument_cbor = argument
            .map(eigon_cbor::serialize_resource)
            .unwrap_or_default();

        // Multi-file join (D53 §4.3): a component_argument may name auxiliary
        // inputs via `runtime:additional_inputs` (IRIs of committed resources —
        // typically PinnedExternalFiles). Resolve each against the chain and
        // ship it alongside the primary input; the substrate materializes +
        // content-verifies each and the worker reads them as the tail of its
        // input list.
        let additional_inputs = match argument {
            Some(arg) => resolve_additional_inputs(arg, layer)?,
            None => Vec::new(),
        };

        let request = ComponentRequest {
            component_iri: self.component_iri.clone(),
            input: input_cbor,
            argument: argument_cbor,
            content_type: EIGON_CBOR_CONTENT_TYPE.to_string(),
            additional_inputs,
        };

        // Block on the async gRPC call within the tokio runtime
        let client = self.client.clone();
        let response = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut client = client.lock().await;
                client
                    .execute(tonic::Request::new(request))
                    .await
                    .map_err(|e| format!("gRPC call failed: {e}"))
            })
        })?;

        let resp = response.into_inner();

        if !resp.success {
            return Err(format!("remote component failed: {}", resp.error));
        }

        // Deserialize output from the orchestrator as an Eigon-CBOR
        // resource. Phase 18e.2: the orchestrator's CompleteJson
        // handler translates short-name LLM output to IRI-keyed shape
        // before returning, so a non-Eigon-resource response now means
        // the handler is broken — surface that as an error rather than
        // wrapping the bytes as `raw_json`.
        let output = eigon_cbor::parse_resource_lenient(&resp.output)
            .map_err(|e| format!("orchestrator returned non-Eigon output: {e}"))?;

        // Extract metrics if present
        let metrics = resp.metrics.map(|m| ComponentMetrics {
            provider: m.provider,
            model: m.model,
            prompt_tokens: m.prompt_tokens,
            completion_tokens: m.completion_tokens,
            latency_ms: m.latency_ms,
        });

        Ok(ComponentResult { output, metrics })
    }
}

/// Shared gRPC client type alias to reduce boilerplate.
pub type SharedOrchestratorClient = Arc<Mutex<ComponentExecutorClient<OrchestratorTransport>>>;

/// Connect to the orchestrator, returning the shared client and the
/// built-in remote components registered against it.
pub async fn connect_orchestrator(
    endpoint: &str,
    component_iris: &[&str],
) -> Result<
    (
        SharedOrchestratorClient,
        Vec<(String, Box<dyn BuiltinComponent>)>,
    ),
    String,
> {
    // Parse the origin up front so a malformed endpoint fails here
    // rather than on the first RPC. `with_origin` stamps the scheme +
    // authority onto every outbound request; the hyper client is lazy
    // (connects on first use, pools thereafter), preserving the old
    // `connect_lazy()` property that the kernel can start before the
    // orchestrator is ready.
    let origin: tonic::transport::Uri = endpoint
        .parse()
        .map_err(|e| format!("invalid endpoint: {e}"))?;

    // Bare HTTP/1.1 client under the gRPC-Web adapter so the terminal
    // `grpc-status` arrives in the response body rather than as an
    // HTTP/2 trailer the Deno-hosted orchestrator can't send. See
    // `OrchestratorTransport`.
    let http = HyperClient::builder(TokioExecutor::new()).build_http();
    let transport = GrpcWebClientService::new(http);

    let client: SharedOrchestratorClient = Arc::new(Mutex::new(
        ComponentExecutorClient::with_origin(transport, origin)
            .max_decoding_message_size(128 * 1024 * 1024)
            .max_encoding_message_size(128 * 1024 * 1024),
    ));

    let mut components: Vec<(String, Box<dyn BuiltinComponent>)> = Vec::new();
    for iri in component_iris {
        components.push((
            iri.to_string(),
            Box::new(RemoteComponent::new(iri.to_string(), client.clone())),
        ));
    }

    Ok((client, components))
}

#[cfg(test)]
mod tests {
    //! Codec-contract tests for `RemoteComponent`.
    //!
    //! These don't spin up a Connect server — that would require
    //! pulling in tonic-test machinery and adds little beyond what
    //! `eigon_cbor::tests` and the orchestrator's
    //! `component_executor_codec_test.ts` already cover. What's worth
    //! pinning here is the *choice* `remote.rs` makes about which
    //! codec to use on the wire (Phase 18e: Eigon-CBOR with the
    //! `application/eigon+cbor` content_type) and that the symmetric
    //! parse path correctly inverts the serialise path. If a future
    //! refactor flips `serialize_resource` to `serialize_document`,
    //! changes the content_type tag, or breaks the round-trip
    //! invariant, these tests catch it before it hits the wire.
    use super::*;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::Value;
    use serde_json::json;

    #[test]
    fn content_type_tag_is_eigon_cbor() {
        // Phase 18e contract: kernel always emits CBOR.
        assert_eq!(EIGON_CBOR_CONTENT_TYPE, "application/eigon+cbor");
    }

    #[test]
    fn serialize_then_parse_round_trips_a_simple_resource() {
        let mut r = Resource::new(Iri::parse("urn:eigenius:test:remote:input").unwrap());
        r.set(
            Iri::parse("urn:eigenius:test:s").unwrap(),
            Value::String("payload".into()),
        );
        r.set(
            Iri::parse("urn:eigenius:test:i").unwrap(),
            Value::Integer(42),
        );

        // Mirror the byte path inside `RemoteComponent::execute`.
        let encoded = eigon_cbor::serialize_resource(&r);
        let decoded = eigon_cbor::parse_resource_lenient(&encoded)
            .expect("orchestrator's response should round-trip");
        assert_eq!(decoded, r);
    }

    #[test]
    fn serialize_then_parse_round_trips_an_embedded_resource() {
        // Components like CompleteJson return a resource whose
        // properties include nested embedded resources. The kernel's
        // parse_resource_lenient must accept the embedded shape on the
        // way back from the orchestrator.
        let mut inner = Resource::new_embedded();
        inner.set(
            Iri::parse("urn:eigenius:test:nested").unwrap(),
            Value::String("inner-value".into()),
        );

        let mut r = Resource::new(Iri::parse("urn:eigenius:test:remote:wrapper").unwrap());
        r.set(
            Iri::parse("urn:eigenius:test:embed").unwrap(),
            Value::Embedded(Box::new(inner.clone())),
        );

        let encoded = eigon_cbor::serialize_resource(&r);
        let decoded = eigon_cbor::parse_resource_lenient(&encoded).expect("round-trip");
        assert_eq!(decoded, r);
    }

    #[test]
    fn serialize_then_parse_round_trips_a_value_json_property() {
        // Substrate-routed traffic carries `numerical_metadata` as
        // `Value::Json`. The codec fix from Phase 18c.5 (EIGENIUS_JSON_TAG)
        // must round-trip through the kernel↔orchestrator path; this
        // test pins that.
        let mut r = Resource::new(Iri::parse("urn:eigenius:test:remote:withjson").unwrap());
        r.set(
            Iri::parse("urn:eigenius:test:metadata").unwrap(),
            Value::Json(json!({"host_kernel": "linux-6.6", "fma_enabled": true})),
        );

        let encoded = eigon_cbor::serialize_resource(&r);
        let decoded = eigon_cbor::parse_resource_lenient(&encoded).expect("round-trip");
        assert_eq!(decoded, r);
    }

    #[test]
    fn lenient_parser_accepts_embedded_response_shape() {
        // The orchestrator may return a resource with no `@id` (the
        // ComponentResponse output is a value, not a top-level chain
        // resource). RemoteComponent::execute uses
        // `parse_resource_lenient` for exactly this — pin the
        // contract.
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:test:k").unwrap(),
            Value::String("v".into()),
        );
        let encoded = eigon_cbor::serialize_resource(&r);
        let decoded = eigon_cbor::parse_resource_lenient(&encoded)
            .expect("lenient parser must accept embedded resources");
        assert_eq!(decoded, r);
    }

    #[test]
    fn argument_serialization_is_optional() {
        // RemoteComponent::execute uses
        // `argument.map(eigon_cbor::serialize_resource).unwrap_or_default()`
        // — when no argument is supplied, an empty byte vec is sent.
        // Pin that an `argument: None` produces empty bytes (not, e.g.,
        // an empty CBOR map).
        let argument: Option<&Resource> = None;
        let bytes: Vec<u8> = argument
            .map(eigon_cbor::serialize_resource)
            .unwrap_or_default();
        assert!(
            bytes.is_empty(),
            "argument=None must serialize to empty bytes; got {} bytes",
            bytes.len()
        );
    }
}
