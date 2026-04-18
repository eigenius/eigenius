//! Remote component dispatch via gRPC.
//!
//! When the kernel evaluates a program that references an IO component
//! not in the local registry, it dispatches the call to the orchestrator
//! via the ComponentExecutor gRPC service.

use crate::layer::Layer;
use crate::ontology::eigon_json;
use crate::ontology::resource::Resource;
use crate::program::component::{BuiltinComponent, ComponentResult};
use crate::program::trace::ComponentMetrics;
use crate::server::proto::component_executor_client::ComponentExecutorClient;
use crate::server::proto::ComponentRequest;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;

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
        // Serialize input and argument to Eigon-JSON
        let input_json = eigon_json::serialize_resource(input).to_string();
        let argument_json = argument
            .map(|a| eigon_json::serialize_resource(a).to_string())
            .unwrap_or_default();

        let request = ComponentRequest {
            component_iri: self.component_iri.clone(),
            input: input_json.into_bytes(),
            argument: argument_json.into_bytes(),
            content_type: "application/eigon+json".to_string(),
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

        // Deserialize output from the orchestrator.
        // Try Eigon-JSON first (full IRIs). If that fails, the response likely
        // uses short-name keys (e.g. CompleteJson LLM output) — store as raw JSON
        // so dispatch_component can convert it via ShortNameTable.
        let output_json =
            String::from_utf8(resp.output).map_err(|e| format!("invalid UTF-8 output: {e}"))?;
        let output = match eigon_json::parse_document(&output_json) {
            Ok(mut resources) => resources.pop().unwrap_or_else(Resource::new_embedded),
            Err(_) => match eigon_json::parse_embedded(&output_json) {
                Ok(r) => r,
                Err(_) => {
                    // Short-name keys from LLM — store as raw JSON on a resource
                    let json_val: serde_json::Value = serde_json::from_str(&output_json)
                        .map_err(|e| format!("invalid JSON output: {e}"))?;
                    let mut r = Resource::new_embedded();
                    r.set(
                        crate::ontology::iri::Iri::parse("urn:eigenius:core:raw_json").unwrap(),
                        crate::ontology::resource::Value::Json(json_val),
                    );
                    r
                }
            },
        };

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

/// Connect to the orchestrator and register all remote components.
///
/// Returns the component registry with remote components registered.
pub async fn connect_orchestrator(
    endpoint: &str,
    component_iris: &[&str],
) -> Result<Vec<(String, Box<dyn BuiltinComponent>)>, String> {
    let channel = Channel::from_shared(endpoint.to_string())
        .map_err(|e| format!("invalid endpoint: {e}"))?
        .connect()
        .await
        .map_err(|e| format!("failed to connect to orchestrator at {endpoint}: {e}"))?;

    let client = Arc::new(Mutex::new(ComponentExecutorClient::new(channel)));

    let mut components: Vec<(String, Box<dyn BuiltinComponent>)> = Vec::new();
    for iri in component_iris {
        components.push((
            iri.to_string(),
            Box::new(RemoteComponent::new(iri.to_string(), client.clone())),
        ));
    }

    Ok(components)
}
