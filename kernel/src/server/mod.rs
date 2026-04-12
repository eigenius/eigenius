//! gRPC server for the Eigenius kernel.
//!
//! Wraps the kernel's existing functionality as a tonic gRPC service.
//! See design doc D5 for the full API specification.

use crate::bootstrap;
use crate::context::ExecutionContext;
use crate::ontology::{eigon_cbor, eigon_json, Iri, Resource};
use crate::program::execute::{self, ComponentRegistry};
use crate::program::expr;
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
}

impl EigeniusService {
    /// Create a new service by bootstrapping the kernel.
    pub fn new() -> Result<Self, String> {
        let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap failed: {e}"))?;
        Ok(Self {
            context: Arc::new(RwLock::new(ctx)),
            components: Arc::new(ComponentRegistry::default()),
        })
    }

    /// Create a tonic server from this service.
    pub fn into_server(self) -> EigeniusKernelServer<Self> {
        EigeniusKernelServer::new(self)
    }

    /// Parse resources from either CBOR or JSON based on content_type.
    #[allow(clippy::result_large_err)]
    fn parse_resources(data: &[u8], content_type: &str) -> Result<Vec<Resource>, Status> {
        if content_type.contains("cbor") {
            eigon_cbor::parse_document(data)
                .map_err(|e| Status::invalid_argument(format!("CBOR parse error: {e}")))
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

        let ctx = self.context.read().await;

        match execute::execute_program(&program, &input, ctx.head(), &self.components) {
            Ok(output) => Ok(Response::new(RunProgramResponse {
                success: true,
                output: Self::serialize_resource(&output),
                errors: Vec::new(),
            })),
            Err(e) => Ok(Response::new(RunProgramResponse {
                success: false,
                output: Vec::new(),
                errors: vec![ValidationError {
                    resource_iri: String::new(),
                    property_iri: String::new(),
                    rule: "execution".to_string(),
                    message: format!("{e}"),
                    severity: "error".to_string(),
                }],
            })),
        }
    }

    async fn reflect(
        &self,
        _request: Request<ReflectRequest>,
    ) -> Result<Response<ReflectResponse>, Status> {
        // Placeholder — reasoning traces come in Phase 4
        Ok(Response::new(ReflectResponse {
            success: false,
            trace_iri: String::new(),
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
}

/// Start the gRPC server on the given port.
pub async fn start_server(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{port}").parse()?;
    let service = EigeniusService::new()?;

    println!("Eigenius gRPC server listening on {addr}");

    tonic::transport::Server::builder()
        .add_service(service.into_server())
        .serve(addr)
        .await?;

    Ok(())
}
