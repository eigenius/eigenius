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
use crate::ontology::resource::Resource;
use crate::program::component::{BuiltinComponent, ComponentResult};
use crate::program::trace::ComponentMetrics;
use crate::server::proto::component_executor_client::ComponentExecutorClient;
use crate::server::proto::ComponentRequest;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;

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
    client: Arc<Mutex<ComponentExecutorClient<Channel>>>,
}

impl RemoteComponent {
    pub fn new(
        component_iri: String,
        client: Arc<Mutex<ComponentExecutorClient<Channel>>>,
    ) -> Self {
        Self {
            component_iri,
            client,
        }
    }
}

impl BuiltinComponent for RemoteComponent {
    fn is_io(&self) -> bool {
        true
    }

    fn execute(
        &self,
        input: &Resource,
        argument: Option<&Resource>,
        _layer: &Layer,
    ) -> Result<ComponentResult, String> {
        // Serialize input and argument to Eigon-CBOR (D26 §8.1 / Phase 18e).
        let input_cbor = eigon_cbor::serialize_resource(input);
        let argument_cbor = argument
            .map(eigon_cbor::serialize_resource)
            .unwrap_or_default();

        let request = ComponentRequest {
            component_iri: self.component_iri.clone(),
            input: input_cbor,
            argument: argument_cbor,
            content_type: EIGON_CBOR_CONTENT_TYPE.to_string(),
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
pub type SharedOrchestratorClient = Arc<Mutex<ComponentExecutorClient<Channel>>>;

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
    // Use `connect_lazy()` so the kernel can start up without requiring the
    // orchestrator to be ready. The connection is established on the first
    // RPC call. This matches how production deployments work (services come
    // up in parallel) and makes local dev less fragile.
    let channel = Channel::from_shared(endpoint.to_string())
        .map_err(|e| format!("invalid endpoint: {e}"))?
        .connect_lazy();

    let client: SharedOrchestratorClient = Arc::new(Mutex::new(
        ComponentExecutorClient::new(channel)
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
