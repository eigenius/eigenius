//! gRPC server for the Eigenius kernel.
//!
//! Wraps the kernel's existing functionality as a tonic gRPC service.
//! See design doc D5 for the full API specification.

use crate::bootstrap;
use crate::context::ExecutionContext;
use crate::ontology::{eigon_cbor, eigon_json, Iri, Resource};
use crate::program::component::ComponentRegistry;
use crate::program::expr;
use crate::program::trace::{InMemoryTraceStore, TraceStore};
use crate::query;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("eigenius.v1");
}

use proto::eigenius_kernel_server::{EigeniusKernel, EigeniusKernelServer};
use proto::*;

/// The Eigenius gRPC service implementation.
pub struct EigeniusService {
    context: Arc<RwLock<ExecutionContext>>,
    components: Arc<ComponentRegistry>,
    trace_store: Arc<dyn TraceStore>,
    institutions: Arc<crate::institution::InstitutionRegistry>,
}

impl EigeniusService {
    /// Create a new service by bootstrapping the kernel.
    pub fn new() -> Result<Self, String> {
        Self::with_components(ComponentRegistry::default())
    }

    /// Create a new service with a custom component registry.
    pub fn with_components(components: ComponentRegistry) -> Result<Self, String> {
        let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap failed: {e}"))?;
        Ok(Self {
            context: Arc::new(RwLock::new(ctx)),
            components: Arc::new(components),
            trace_store: Arc::new(InMemoryTraceStore::new()),
            institutions: Arc::new(crate::institution::InstitutionRegistry::new()),
        })
    }

    /// Create a new service with a custom component registry and trace store.
    pub fn with_trace_store(
        components: ComponentRegistry,
        trace_store: Arc<dyn TraceStore>,
    ) -> Result<Self, String> {
        let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap failed: {e}"))?;
        Ok(Self {
            context: Arc::new(RwLock::new(ctx)),
            components: Arc::new(components),
            trace_store,
            institutions: Arc::new(crate::institution::InstitutionRegistry::new()),
        })
    }

    /// Create a tonic server from this service.
    pub fn into_server(self) -> EigeniusKernelServer<Self> {
        EigeniusKernelServer::new(self)
    }

    /// Parse resources from CBOR, JSON, or ESL based on content_type.
    #[allow(clippy::result_large_err)]
    fn parse_resources(data: &[u8], content_type: &str) -> Result<Vec<Resource>, Status> {
        if content_type.contains("cbor") {
            eigon_cbor::parse_document(data)
                .map_err(|e| Status::invalid_argument(format!("CBOR parse error: {e}")))
        } else if content_type.contains("esl") {
            let source = std::str::from_utf8(data)
                .map_err(|e| Status::invalid_argument(format!("invalid UTF-8: {e}")))?;
            crate::esl::compile(source).map_err(|errors| {
                let msgs: Vec<String> = errors.iter().map(|e| format!("{e}")).collect();
                Status::invalid_argument(format!("ESL compile error: {}", msgs.join("; ")))
            })
        } else {
            let json_str = std::str::from_utf8(data)
                .map_err(|e| Status::invalid_argument(format!("invalid UTF-8: {e}")))?;
            eigon_json::parse_document(json_str)
                .map_err(|e| Status::invalid_argument(format!("JSON parse error: {e}")))
        }
    }

    /// Serialize a resource to CBOR bytes.
    fn serialize_resource(resource: &Resource) -> Vec<u8> {
        eigon_cbor::serialize_resource(resource)
    }
}

#[allow(clippy::result_large_err)]
#[tonic::async_trait]
impl EigeniusKernel for EigeniusService {
    async fn load(&self, request: Request<LoadRequest>) -> Result<Response<LoadResponse>, Status> {
        let req = request.into_inner();
        let resources = Self::parse_resources(&req.resources, &req.content_type)?;
        let count = resources.len() as u32;

        let mut ctx = self.context.write().await;
        for resource in resources {
            ctx.add_resource(resource)
                .map_err(|e| Status::failed_precondition(format!("load error: {e}")))?;
        }

        let mut layer_id = String::new();
        let mut errors = Vec::new();

        if req.auto_commit {
            match ctx.commit("loaded") {
                Ok(layer) => {
                    layer_id = layer.id().to_string();
                }
                Err(e) => {
                    errors.push(ValidationError {
                        resource_iri: String::new(),
                        property_iri: String::new(),
                        rule: "commit".to_string(),
                        message: format!("{e}"),
                        severity: "error".to_string(),
                    });
                }
            }
        }

        Ok(Response::new(LoadResponse {
            success: errors.is_empty(),
            errors,
            layer_id,
            resource_count: count,
        }))
    }

    async fn inspect(
        &self,
        request: Request<InspectRequest>,
    ) -> Result<Response<InspectResponse>, Status> {
        let req = request.into_inner();
        let iri = Iri::parse(&req.iri)
            .map_err(|e| Status::invalid_argument(format!("invalid IRI: {e}")))?;

        let ctx = self.context.read().await;
        match ctx.resolve(&iri) {
            Some(resource) => Ok(Response::new(InspectResponse {
                found: true,
                resource: Self::serialize_resource(resource),
            })),
            None => Ok(Response::new(InspectResponse {
                found: false,
                resource: Vec::new(),
            })),
        }
    }

    type QueryStream = tokio_stream::wrappers::ReceiverStream<Result<QueryResult, Status>>;

    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<Self::QueryStream>, Status> {
        let req = request.into_inner();
        let ctx = self.context.read().await;

        let result = query::execute(&req.eigenql, ctx.head()).map_err(|errors| {
            let msgs: Vec<String> = errors.iter().map(|e| format!("{e}")).collect();
            Status::failed_precondition(format!("query error: {}", msgs.join("; ")))
        })?;

        let (tx, rx) = tokio::sync::mpsc::channel(128);

        // Send results
        tokio::spawn(async move {
            for (index, resource) in result.resources.iter().enumerate() {
                let msg = QueryResult {
                    resource: eigon_cbor::serialize_resource(resource),
                    index: index as u64,
                };
                if tx.send(Ok(msg)).await.is_err() {
                    break; // Client disconnected
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    async fn validate_program(
        &self,
        request: Request<ValidateProgramRequest>,
    ) -> Result<Response<ValidateProgramResponse>, Status> {
        let req = request.into_inner();
        let resources = Self::parse_resources(&req.program, &req.content_type)?;
        let program = resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no program resource"))?;

        let ctx = self.context.read().await;

        match expr::parse_program(&program, ctx.head()) {
            Ok((_term, typ)) => Ok(Response::new(ValidateProgramResponse {
                valid: true,
                errors: Vec::new(),
                program_type: format!("{typ:?}"),
            })),
            Err(e) => Ok(Response::new(ValidateProgramResponse {
                valid: false,
                errors: vec![ValidationError {
                    resource_iri: String::new(),
                    property_iri: String::new(),
                    rule: "type_check".to_string(),
                    message: e,
                    severity: "error".to_string(),
                }],
                program_type: String::new(),
            })),
        }
    }

    async fn run_program(
        &self,
        request: Request<RunProgramRequest>,
    ) -> Result<Response<RunProgramResponse>, Status> {
        let req = request.into_inner();
        let program_resources = Self::parse_resources(&req.program, &req.content_type)?;
        let program = program_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no program resource"))?;

        let input_resources = Self::parse_resources(&req.input, &req.content_type)?;
        let input = input_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no input resource"))?;

        // Execute via NbE in IO mode
        let exec_result = {
            let ctx = self.context.read().await;
            match crate::program::eval_io::execute_program_nbe(
                &program,
                &input,
                Arc::clone(ctx.head()),
                Arc::clone(&self.components),
                Some(Arc::clone(&self.trace_store)),
            ) {
                Ok(result) => result,
                Err(e) => {
                    return Ok(Response::new(RunProgramResponse {
                        success: false,
                        output: Vec::new(),
                        errors: vec![ValidationError {
                            resource_iri: String::new(),
                            property_iri: String::new(),
                            rule: "execution".to_string(),
                            message: format!("{e}"),
                            severity: "error".to_string(),
                        }],
                        trace_iri: String::new(),
                    }));
                }
            }
        };

        let output = exec_result.output;
        let dispatched_traces = exec_result.dispatched_traces;

        // Compute metrics from dispatched ComponentTraces
        let total_tokens: i64 = dispatched_traces
            .iter()
            .filter_map(|ct| ct.metrics.as_ref())
            .map(|m| m.prompt_tokens + m.completion_tokens)
            .sum();
        let executed_steps = dispatched_traces.len() as i64;

        // Build ProgramTrace
        let trace_iri_str = format!("urn:eigenius:trace:exec-{}", uuid::Uuid::new_v4());

        let mut trace_resource = Resource::new(Iri::parse(&trace_iri_str).unwrap());
        trace_resource.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    "urn:eigenius:reflection:ProgramTrace".to_string(),
                ),
            ]),
        );
        if let Some(prog_id) = program.id() {
            trace_resource.set(
                Iri::parse("urn:eigenius:reflection:program").unwrap(),
                crate::ontology::resource::Value::String(prog_id.as_str().to_string()),
            );
        }
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:total_tokens").unwrap(),
            crate::ontology::resource::Value::Integer(total_tokens),
        );
        trace_resource.set(
            Iri::parse("urn:eigenius:reflection:executed_steps").unwrap(),
            crate::ontology::resource::Value::Integer(executed_steps),
        );

        // Auto-commit trace layer: ProgramTrace + all IO ComponentTraces
        {
            let mut ctx = self.context.write().await;
            // Add ProgramTrace
            let _ = ctx.add_resource(trace_resource);
            // Add each IO ComponentTrace as a resource
            for ct in &dispatched_traces {
                let ct_resource = crate::program::trace::trace_to_resource(
                    &crate::program::trace::Trace::Component(ct.clone()),
                );
                let _ = ctx.add_resource(ct_resource);
            }
            let _ = ctx.commit("trace");
        }

        Ok(Response::new(RunProgramResponse {
            success: true,
            output: Self::serialize_resource(&output),
            errors: Vec::new(),
            trace_iri: trace_iri_str,
        }))
    }

    async fn reflect(
        &self,
        request: Request<ReflectRequest>,
    ) -> Result<Response<ReflectResponse>, Status> {
        let req = request.into_inner();
        let resources = Self::parse_resources(&req.trace, &req.content_type)?;

        if resources.is_empty() {
            return Ok(Response::new(ReflectResponse {
                success: false,
                trace_iri: String::new(),
            }));
        }

        // The first resource should be a trace (ProgramTrace, DeclarationTrace, etc.)
        let trace_resource = &resources[0];
        let trace_iri = trace_resource
            .id()
            .map(|i| i.as_str().to_string())
            .unwrap_or_default();

        // Commit all trace resources to a new layer
        let mut ctx = self.context.write().await;
        for resource in resources {
            ctx.add_resource(resource)
                .map_err(|e| Status::failed_precondition(format!("reflect error: {e}")))?;
        }
        ctx.commit("reflect")
            .map_err(|e| Status::internal(format!("reflect commit failed: {e}")))?;

        Ok(Response::new(ReflectResponse {
            success: true,
            trace_iri,
        }))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let ctx = self.context.read().await;
        let all = ctx.head().all_resources();

        Ok(Response::new(HealthResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            layer_count: 2, // core + program ontology
            resource_count: all.len() as u64,
        }))
    }

    async fn fiber_query(
        &self,
        request: Request<FiberQueryRequest>,
    ) -> Result<Response<FiberQueryResponse>, Status> {
        let req = request.into_inner();
        let inst_iri = Iri::parse(&req.institution_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid institution IRI: {e}")))?;

        let reasoner = self
            .institutions
            .get(&inst_iri)
            .ok_or_else(|| Status::not_found(format!("institution not found: {inst_iri}")))?;

        let query_resources = Self::parse_resources(&req.query, &req.content_type)?;
        let query = query_resources
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("no query resource"))?;

        let ctx = self.context.read().await;
        match reasoner.query(&query, &ctx) {
            Ok(result) => Ok(Response::new(FiberQueryResponse {
                success: true,
                result: Self::serialize_resource(&result),
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(FiberQueryResponse {
                success: false,
                result: Vec::new(),
                error: format!("{e}"),
            })),
        }
    }

    async fn discover_morphisms(
        &self,
        request: Request<DiscoverMorphismsRequest>,
    ) -> Result<Response<DiscoverMorphismsResponse>, Status> {
        let req = request.into_inner();
        let inst_iri = Iri::parse(&req.institution_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid institution IRI: {e}")))?;

        let reasoner = self
            .institutions
            .get(&inst_iri)
            .ok_or_else(|| Status::not_found(format!("institution not found: {inst_iri}")))?;

        let mut resources = Vec::new();
        for data in &req.resources {
            let parsed = Self::parse_resources(data, &req.content_type)?;
            resources.extend(parsed);
        }

        let ctx = self.context.read().await;
        match reasoner.discover_morphisms(&resources, &ctx) {
            Ok(morphisms) => {
                let serialized: Vec<Vec<u8>> =
                    morphisms.iter().map(Self::serialize_resource).collect();
                Ok(Response::new(DiscoverMorphismsResponse {
                    success: true,
                    morphisms: serialized,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(DiscoverMorphismsResponse {
                success: false,
                morphisms: Vec::new(),
                error: format!("{e}"),
            })),
        }
    }

    async fn list_institutions(
        &self,
        _request: Request<ListInstitutionsRequest>,
    ) -> Result<Response<ListInstitutionsResponse>, Status> {
        let infos: Vec<proto::InstitutionInfo> = self
            .institutions
            .list()
            .iter()
            .map(|info| proto::InstitutionInfo {
                iri: info.iri.as_str().to_string(),
                name: info.name.clone(),
                morphism_types: info
                    .morphism_type_iris
                    .iter()
                    .map(|i| i.as_str().to_string())
                    .collect(),
                query_types: info
                    .query_type_iris
                    .iter()
                    .map(|i| i.as_str().to_string())
                    .collect(),
            })
            .collect();

        Ok(Response::new(ListInstitutionsResponse {
            institutions: infos,
        }))
    }
}

/// Known remote component IRIs that should be dispatched to the orchestrator.
const REMOTE_COMPONENTS: &[&str] = &[
    "urn:eigenius:program:components:CompleteText",
    "urn:eigenius:program:components:CompleteJson",
    "urn:eigenius:program:components:HttpRequest",
];

/// Start the gRPC server on the given port.
///
/// If `orchestrator_endpoint` is provided, remote components are registered
/// that dispatch IO calls to the orchestrator via ComponentExecutor gRPC.
pub async fn start_server(
    port: u16,
    orchestrator_endpoint: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{port}").parse()?;

    let mut registry = ComponentRegistry::default();

    if let Some(endpoint) = orchestrator_endpoint {
        println!("Connecting to orchestrator at {endpoint}...");
        match crate::program::remote::connect_orchestrator(endpoint, REMOTE_COMPONENTS).await {
            Ok(components) => {
                for (iri, component) in components {
                    println!("  Registered remote component: {iri}");
                    registry.register(iri, component);
                }
            }
            Err(e) => {
                eprintln!("Warning: failed to connect to orchestrator: {e}");
                eprintln!("  IO components will not be available");
            }
        }
    }

    let service = EigeniusService::with_components(registry)?;

    println!("Eigenius gRPC server listening on {addr}");

    tonic::transport::Server::builder()
        .add_service(service.into_server())
        .serve(addr)
        .await?;

    Ok(())
}
