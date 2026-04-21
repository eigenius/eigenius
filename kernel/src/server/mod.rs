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
    /// Outer lock allows swapping the registry (for WASM registration on load).
    /// Inner Arc allows cheap cloning for passing to the evaluator.
    components: Arc<RwLock<Arc<ComponentRegistry>>>,
    trace_store: Arc<dyn TraceStore>,
    institutions: Arc<RwLock<crate::institution::InstitutionRegistry>>,
    /// Optional persistent backend. When present, committed layers,
    /// the seed manifest, and trace state all live here; absent means
    /// the server is in-memory-only (the pre-Phase-9a behaviour).
    /// See D13.
    backend: Option<Arc<dyn crate::storage::PersistentBackend>>,
    /// Optional gRPC client for the orchestrator. Used to forward IO-capability
    /// WASM components and to dispatch remote IO components during program
    /// execution. None means no orchestrator is configured — IO WASM installs
    /// will be rejected with a clear error in that case.
    orchestrator_client: Option<
        Arc<
            tokio::sync::Mutex<
                proto::component_executor_client::ComponentExecutorClient<
                    tonic::transport::Channel,
                >,
            >,
        >,
    >,
}

impl EigeniusService {
    /// Create a new service by bootstrapping the kernel.
    pub fn new() -> Result<Self, String> {
        Self::with_components(ComponentRegistry::default())
    }

    /// Create a new service with a custom component registry.
    ///
    /// Uses the in-memory bootstrap path. See
    /// [`Self::with_persistent_backend`] for the durable variant.
    pub fn with_components(components: ComponentRegistry) -> Result<Self, String> {
        let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap failed: {e}"))?;
        Ok(Self {
            context: Arc::new(RwLock::new(ctx)),
            components: Arc::new(RwLock::new(Arc::new(components))),
            trace_store: Arc::new(InMemoryTraceStore::new()),
            institutions: Arc::new(RwLock::new(crate::institution::InstitutionRegistry::new())),
            backend: None,
            orchestrator_client: None,
        })
    }

    /// Create a new service backed by a persistent store.
    ///
    /// Implements the SEED and RESUME paths from D13 §4:
    /// - Empty backend: commit the four embedded ontologies and a
    ///   seed manifest, then treat the backend as authoritative.
    /// - Non-empty backend: reconstruct the `ExecutionContext` from
    ///   the persisted layer chain, verifying the seed manifest against
    ///   the current embedded ontologies (refuse to boot on drift).
    ///
    /// The backend also supplies the trace store, so
    /// `ComponentTrace` reads/writes flow through the same DB.
    pub fn with_persistent_backend(
        components: ComponentRegistry,
        backend: Arc<dyn crate::storage::PersistentBackend>,
    ) -> Result<Self, String> {
        let ctx = bootstrap::bootstrap_persistent(backend.as_ref())
            .map_err(|e| format!("persistent bootstrap failed: {e}"))?;

        // Wrap the backend's trace-store view into an Arc<dyn TraceStore>
        // so the service can hold it independently. We do this by keeping
        // the backend alive via `trace_store_arc_from_backend` — the
        // returned Arc shares ownership with `backend`.
        let trace_store: Arc<dyn TraceStore> =
            Arc::new(BackendTraceStore::new(Arc::clone(&backend)));

        Ok(Self {
            context: Arc::new(RwLock::new(ctx)),
            components: Arc::new(RwLock::new(Arc::new(components))),
            trace_store,
            institutions: Arc::new(RwLock::new(crate::institution::InstitutionRegistry::new())),
            backend: Some(backend),
            orchestrator_client: None,
        })
    }

    /// Attach an orchestrator client so IO-capability WASM components can be
    /// forwarded to the orchestrator and remote components dispatched back.
    pub fn with_orchestrator_client(
        mut self,
        client: Arc<
            tokio::sync::Mutex<
                proto::component_executor_client::ComponentExecutorClient<
                    tonic::transport::Channel,
                >,
            >,
        >,
    ) -> Self {
        self.orchestrator_client = Some(client);
        self
    }

    /// Create a new service with a custom component registry and trace store.
    pub fn with_trace_store(
        components: ComponentRegistry,
        trace_store: Arc<dyn TraceStore>,
    ) -> Result<Self, String> {
        let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap failed: {e}"))?;
        Ok(Self {
            context: Arc::new(RwLock::new(ctx)),
            components: Arc::new(RwLock::new(Arc::new(components))),
            trace_store,
            institutions: Arc::new(RwLock::new(crate::institution::InstitutionRegistry::new())),
            backend: None,
            orchestrator_client: None,
        })
    }

    /// Create a tonic server from this service.
    pub fn into_server(self) -> EigeniusKernelServer<Self> {
        EigeniusKernelServer::new(self)
    }

    /// Persist a freshly-committed layer through the backend, if one is
    /// attached. No-op otherwise. See D13 §5.
    ///
    /// Returns a validation-like error on storage failure so the caller
    /// can surface it to clients without crashing the server.
    fn persist_layer_if_backend(&self, layer: &crate::layer::Layer) -> Option<ValidationError> {
        let backend = self.backend.as_ref()?;
        if let Err(e) = backend.store_layer(layer) {
            return Some(ValidationError {
                resource_iri: String::new(),
                property_iri: String::new(),
                rule: "persist_layer".to_string(),
                message: format!("{e}"),
                severity: "error".to_string(),
            });
        }
        if let Err(e) = backend.set_head(layer.id()) {
            return Some(ValidationError {
                resource_iri: String::new(),
                property_iri: String::new(),
                rule: "persist_head".to_string(),
                message: format!("{e}"),
                severity: "error".to_string(),
            });
        }
        None
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

    /// Scan a layer for WASM components/institutions and register them.
    ///
    /// Errors encountered during registration are added to `errors` as
    /// validation errors (but do not fail the load — a malformed WASM
    /// resource is reported but other resources in the same layer still
    /// load successfully).
    ///
    /// Returns the set of declared class/property resources the
    /// institution registration produced. The caller is expected to
    /// commit these to a follow-up layer so they become queryable.
    /// RESUME callers should instead use
    /// [`Self::rehydrate_wasm_from_layer`] which skips publishing.
    async fn register_wasm_from_layer(
        &self,
        layer: &crate::layer::Layer,
        errors: &mut Vec<ValidationError>,
    ) -> Vec<Resource> {
        // Build a new ComponentRegistry layered on top of the current one.
        // This avoids needing `BuiltinComponent: Clone` — the parent Arc
        // is shared, new WASM entries are added to the child.
        let mut new_registry = {
            let current = self.components.read().await;
            ComponentRegistry::new_with_parent(Arc::clone(&current))
        };

        let scan_result =
            crate::capability::registration::scan_and_register(layer, &mut new_registry);

        for e in &scan_result.report.errors {
            errors.push(ValidationError {
                resource_iri: e.resource_iri.clone(),
                property_iri: String::new(),
                rule: "wasm_registration".to_string(),
                message: e.message.clone(),
                severity: "error".to_string(),
            });
        }
        for w in &scan_result.report.warnings {
            eprintln!("  {w}");
        }

        // Forward IO WASM components to the orchestrator and register a
        // RemoteComponent locally so the kernel can dispatch to them.
        let mut any_kernel_component_added = !scan_result.report.components_registered.is_empty()
            && scan_result.pending_io_components.is_empty();
        for pending in scan_result.pending_io_components {
            match self.register_io_wasm(&pending).await {
                Ok(remote) => {
                    new_registry.register(pending.resource_iri.clone(), remote);
                    eprintln!(
                        "  Registered IO WASM component: {} (orchestrator-hosted)",
                        pending.resource_iri
                    );
                    any_kernel_component_added = true;
                }
                Err(e) => {
                    errors.push(ValidationError {
                        resource_iri: pending.resource_iri,
                        property_iri: String::new(),
                        rule: "wasm_registration".to_string(),
                        message: e,
                        severity: "error".to_string(),
                    });
                }
            }
        }

        if !scan_result.report.components_registered.is_empty() {
            for iri in &scan_result.report.components_registered {
                eprintln!("  Registered WASM component: {iri}");
            }
        }

        if any_kernel_component_added {
            let mut guard = self.components.write().await;
            *guard = Arc::new(new_registry);
        }

        // Register institutions. Collect the declared class/property
        // resources each institution publishes so the caller can commit
        // them in a follow-up layer. Closes #15 when paired with the
        // Load-path commit below.
        let mut published_resources: Vec<Resource> = Vec::new();
        if !scan_result.wasm_institutions.is_empty() {
            let mut institutions = self.institutions.write().await;
            for reasoner in scan_result.wasm_institutions {
                let iri = reasoner.institution_iri().as_str().to_string();
                match institutions.register(Box::new(reasoner)) {
                    Ok(declared) => {
                        eprintln!(
                            "  Registered WASM institution: {iri} (+{} declared classes)",
                            declared.len()
                        );
                        published_resources.extend(declared);
                    }
                    Err(e) => {
                        errors.push(ValidationError {
                            resource_iri: iri,
                            property_iri: String::new(),
                            rule: "wasm_registration".to_string(),
                            message: format!("institution registration failed: {e}"),
                            severity: "error".to_string(),
                        });
                    }
                }
            }
        }
        published_resources
    }

    /// RESUME counterpart of [`Self::register_wasm_from_layer`]. Walks a
    /// rehydrated layer and re-registers every WASM component /
    /// institution it finds, **without** re-publishing institution
    /// declared classes — those are already in the persisted chain.
    /// IO components are forwarded to the orchestrator again (same
    /// semantics as fresh install; the orchestrator may reject if it
    /// already has the component).
    async fn rehydrate_wasm_from_layer(
        &self,
        layer: &crate::layer::Layer,
        errors: &mut Vec<ValidationError>,
    ) {
        let mut new_registry = {
            let current = self.components.read().await;
            ComponentRegistry::new_with_parent(Arc::clone(&current))
        };

        let scan_result =
            crate::capability::registration::scan_and_register(layer, &mut new_registry);

        for e in &scan_result.report.errors {
            errors.push(ValidationError {
                resource_iri: e.resource_iri.clone(),
                property_iri: String::new(),
                rule: "wasm_rehydrate".to_string(),
                message: e.message.clone(),
                severity: "error".to_string(),
            });
        }

        let mut any_kernel_component_added = !scan_result.report.components_registered.is_empty()
            && scan_result.pending_io_components.is_empty();
        for pending in scan_result.pending_io_components {
            match self.register_io_wasm(&pending).await {
                Ok(remote) => {
                    new_registry.register(pending.resource_iri.clone(), remote);
                    eprintln!("  Rehydrated IO WASM component: {}", pending.resource_iri);
                    any_kernel_component_added = true;
                }
                Err(e) => {
                    errors.push(ValidationError {
                        resource_iri: pending.resource_iri,
                        property_iri: String::new(),
                        rule: "wasm_rehydrate".to_string(),
                        message: e,
                        severity: "error".to_string(),
                    });
                }
            }
        }

        if any_kernel_component_added {
            let mut guard = self.components.write().await;
            *guard = Arc::new(new_registry);
        }

        if !scan_result.wasm_institutions.is_empty() {
            let mut institutions = self.institutions.write().await;
            for reasoner in scan_result.wasm_institutions {
                let iri = reasoner.institution_iri().as_str().to_string();
                if let Err(e) = institutions.register_rehydrated(Box::new(reasoner)) {
                    errors.push(ValidationError {
                        resource_iri: iri,
                        property_iri: String::new(),
                        rule: "wasm_rehydrate".to_string(),
                        message: format!("institution rehydrate failed: {e}"),
                        severity: "error".to_string(),
                    });
                } else {
                    eprintln!("  Rehydrated WASM institution: {iri}");
                }
            }
        }
    }

    /// Walk the persisted chain from root to head and rehydrate every
    /// WASM capability resource found in each layer. Called once by the
    /// server at startup when a persistent backend is attached.
    pub async fn rehydrate_wasm_from_chain(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let head = {
            let ctx = self.context.read().await;
            Arc::clone(ctx.head())
        };
        // Collect root-to-head order so earlier layers register first.
        let mut chain: Vec<Arc<crate::layer::Layer>> = Vec::new();
        let mut cursor = Some(head);
        while let Some(layer) = cursor {
            let parent = layer.parent().cloned();
            chain.push(layer);
            cursor = parent;
        }
        chain.reverse();

        for layer in &chain {
            self.rehydrate_wasm_from_layer(layer, &mut errors).await;
        }
        errors
    }

    /// Forward an IO WASM component to the orchestrator and produce a
    /// local `RemoteComponent` wrapper that dispatches `Execute` calls
    /// back to the orchestrator.
    async fn register_io_wasm(
        &self,
        pending: &crate::capability::registration::PendingIoComponent,
    ) -> Result<Box<dyn crate::program::component::BuiltinComponent>, String> {
        let client = self.orchestrator_client.as_ref().ok_or_else(|| {
            "IO WASM components require an orchestrator to be configured \
                 (pass --orchestrator to `serve`)"
                .to_string()
        })?;

        let request = proto::RegisterWasmComponentRequest {
            component_iri: pending.resource_iri.clone(),
            wasm_binary: pending.wasm_binary.clone(),
            fuel_limit: pending.fuel_limit,
            memory_limit_pages: pending.memory_limit_pages as u64,
        };

        let response = {
            let mut c = client.lock().await;
            c.register_wasm_component(tonic::Request::new(request))
                .await
                .map_err(|e| format!("RegisterWasmComponent gRPC call failed: {e}"))?
        };
        let resp = response.into_inner();
        if !resp.success {
            return Err(format!(
                "orchestrator rejected WASM registration: {}",
                resp.error
            ));
        }

        // Build a local RemoteComponent that forwards Execute calls.
        Ok(Box::new(crate::program::remote::RemoteComponent::new(
            pending.resource_iri.clone(),
            Arc::clone(client),
        )))
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
                    drop(ctx);
                    if let Some(err) = self.persist_layer_if_backend(&layer) {
                        errors.push(err);
                    }
                    // Scan the newly committed layer for WASM components/institutions.
                    // For institutions, this returns the declared morphism / query
                    // class resources that the registration produced.
                    let published = self.register_wasm_from_layer(&layer, &mut errors).await;

                    // Commit the published institution classes as a follow-up layer
                    // so they become queryable via EigenQL / inspect. Closes #15.
                    if !published.is_empty() {
                        let mut ctx = self.context.write().await;
                        for resource in published {
                            if let Err(e) = ctx.add_resource(resource) {
                                errors.push(ValidationError {
                                    resource_iri: String::new(),
                                    property_iri: String::new(),
                                    rule: "institution_publish".to_string(),
                                    message: format!(
                                        "failed to add institution-declared class: {e}"
                                    ),
                                    severity: "error".to_string(),
                                });
                            }
                        }
                        if ctx.has_changes() {
                            match ctx.commit("institution_classes") {
                                Ok(extra) => {
                                    drop(ctx);
                                    if let Some(err) = self.persist_layer_if_backend(&extra) {
                                        errors.push(err);
                                    }
                                }
                                Err(e) => {
                                    errors.push(ValidationError {
                                        resource_iri: String::new(),
                                        property_iri: String::new(),
                                        rule: "institution_publish".to_string(),
                                        message: format!("commit failed: {e}"),
                                        severity: "error".to_string(),
                                    });
                                }
                            }
                        }
                    }
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

    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let req = request.into_inner();
        let ctx = self.context.read().await;
        let institutions = self.institutions.read().await;

        let runtime = query::evaluate::FiberRuntime {
            institutions: Some(&institutions),
            ctx: Some(&ctx),
        };

        let document = match query::execute_with(&req.eigenql, ctx.head(), runtime) {
            Ok(doc) => doc,
            Err(errors) => {
                let msgs: Vec<String> = errors.iter().map(|e| format!("{e}")).collect();
                return Ok(Response::new(QueryResponse {
                    success: false,
                    document: Vec::new(),
                    content_type: String::new(),
                    error: format!("query error: {}", msgs.join("; ")),
                }));
            }
        };

        Ok(Response::new(QueryResponse {
            success: true,
            document: eigon_cbor::serialize_document(&document),
            content_type: "application/cbor".to_string(),
            error: String::new(),
        }))
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
            Ok((_term, typ)) => {
                // Validate template references against input type
                let mut template_errors = Vec::new();
                let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
                let input_type_prop = Iri::parse("urn:eigenius:program:input_type").unwrap();
                if let (
                    Some(crate::ontology::resource::Value::String(input_type_str)),
                    Some(crate::ontology::resource::Value::Embedded(body)),
                ) = (program.get(&input_type_prop), program.get(&body_prop))
                {
                    if let Ok(input_type_iri) = Iri::parse(input_type_str) {
                        let comp_arg_prop =
                            Iri::parse("urn:eigenius:program:component_argument").unwrap();
                        // Walk expression tree looking for component arguments
                        fn find_comp_args(resource: &Resource, prop: &Iri) -> Vec<Resource> {
                            let mut args = Vec::new();
                            if let Some(crate::ontology::resource::Value::Embedded(arg)) =
                                resource.get(prop)
                            {
                                args.push(arg.as_ref().clone());
                            }
                            // Recurse into embedded resources
                            for val in resource.properties().values() {
                                if let crate::ontology::resource::Value::Embedded(child) = val {
                                    args.extend(find_comp_args(child, prop));
                                }
                            }
                            args
                        }
                        for comp_arg in find_comp_args(body, &comp_arg_prop) {
                            let errs = crate::program::schema::validate_component_templates(
                                &comp_arg,
                                &input_type_iri,
                                ctx.head(),
                            );
                            for e in errs {
                                template_errors.push(ValidationError {
                                    resource_iri: String::new(),
                                    property_iri: String::new(),
                                    rule: "template".to_string(),
                                    message: format!("{e}"),
                                    severity: "error".to_string(),
                                });
                            }
                        }
                    }
                }

                // Validate output schemas (bijectivity check, D8 §4)
                for e in crate::program::schema::validate_output_schemas(&program, ctx.head()) {
                    template_errors.push(ValidationError {
                        resource_iri: String::new(),
                        property_iri: String::new(),
                        rule: "schema_bijectivity".to_string(),
                        message: format!("{e}"),
                        severity: "error".to_string(),
                    });
                }

                if template_errors.is_empty() {
                    Ok(Response::new(ValidateProgramResponse {
                        valid: true,
                        errors: Vec::new(),
                        program_type: format!("{typ:?}"),
                    }))
                } else {
                    Ok(Response::new(ValidateProgramResponse {
                        valid: false,
                        errors: template_errors,
                        program_type: format!("{typ:?}"),
                    }))
                }
            }
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
            let components = Arc::clone(&*self.components.read().await);
            match crate::program::eval_io::execute_program_nbe(
                &program,
                &input,
                Arc::clone(ctx.head()),
                components,
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
            if let Ok(layer) = ctx.commit("trace") {
                // Best-effort persist of the trace layer. A failure here
                // logs but doesn't fail the RunProgram call — the output
                // is still valid, the trace just isn't durable.
                if let Some(err) = self.persist_layer_if_backend(&layer) {
                    eprintln!("warning: failed to persist trace layer: {}", err.message);
                }
            }
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
        let layer = ctx
            .commit("reflect")
            .map_err(|e| Status::internal(format!("reflect commit failed: {e}")))?;
        if let Some(err) = self.persist_layer_if_backend(&layer) {
            return Err(Status::internal(format!(
                "reflect persist failed: {}",
                err.message
            )));
        }

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

        let institutions = self.institutions.read().await;
        let reasoner = institutions
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

        let institutions = self.institutions.read().await;
        let reasoner = institutions
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
        let institutions = self.institutions.read().await;
        let infos: Vec<proto::InstitutionInfo> = institutions
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

    async fn get_schema(
        &self,
        request: Request<GetSchemaRequest>,
    ) -> Result<Response<GetSchemaResponse>, Status> {
        let req = request.into_inner();
        let class_iri = Iri::parse(&req.class_iri)
            .map_err(|e| Status::invalid_argument(format!("invalid IRI: {e}")))?;

        let ctx = self.context.read().await;
        match crate::program::schema::schema_for_class(&class_iri, ctx.head()) {
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
///
/// If `backend` is `Some`, the server runs in durable mode: layers, traces
/// and WASM capabilities survive restart. An empty backend is seeded with
/// the embedded ontologies; a populated one is rehydrated. See D13.
pub async fn start_server(
    port: u16,
    orchestrator_endpoint: Option<&str>,
    backend: Option<Arc<dyn crate::storage::PersistentBackend>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{port}").parse()?;

    let mut registry = ComponentRegistry::default();
    let mut orchestrator_client: Option<crate::program::remote::SharedOrchestratorClient> = None;

    if let Some(endpoint) = orchestrator_endpoint {
        println!("Connecting to orchestrator at {endpoint}...");
        match crate::program::remote::connect_orchestrator(endpoint, REMOTE_COMPONENTS).await {
            Ok((client, components)) => {
                for (iri, component) in components {
                    println!("  Registered remote component: {iri}");
                    registry.register(iri, component);
                }
                orchestrator_client = Some(client);
            }
            Err(e) => {
                eprintln!("Warning: failed to connect to orchestrator: {e}");
                eprintln!("  IO components will not be available");
            }
        }
    }

    let (mut service, is_persistent) = match backend {
        Some(b) => {
            println!("Persistent backend attached; using SEED-or-RESUME bootstrap (D13).");
            (EigeniusService::with_persistent_backend(registry, b)?, true)
        }
        None => {
            println!("In-memory mode (no --db). All state lost on exit.");
            (EigeniusService::with_components(registry)?, false)
        }
    };
    if let Some(client) = orchestrator_client {
        service = service.with_orchestrator_client(client);
    }

    // On a persistent backend, walk the rehydrated chain and
    // re-register every WASM capability it finds. Institutions go
    // through `register_rehydrated` (doesn't re-publish classes).
    if is_persistent {
        let errors = service.rehydrate_wasm_from_chain().await;
        for e in errors {
            eprintln!(
                "warning: WASM rehydrate: [{}] {}",
                e.resource_iri, e.message
            );
        }
    }

    println!("Eigenius gRPC server listening on {addr}");

    // Raise gRPC message size limits to 128 MB to accommodate WASM component
    // binaries (which are base64-encoded and can be multiple MB).
    tonic::transport::Server::builder()
        .add_service(
            service
                .into_server()
                .max_decoding_message_size(128 * 1024 * 1024)
                .max_encoding_message_size(128 * 1024 * 1024),
        )
        .serve(addr)
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// BackendTraceStore — forwards TraceStore calls to a PersistentBackend.
// Lets the service hold `Arc<dyn TraceStore>` without needing to hand out
// two Arc types of the same RocksStore.
// ---------------------------------------------------------------------------

struct BackendTraceStore {
    backend: Arc<dyn crate::storage::PersistentBackend>,
}

impl BackendTraceStore {
    fn new(backend: Arc<dyn crate::storage::PersistentBackend>) -> Self {
        Self { backend }
    }
}

impl TraceStore for BackendTraceStore {
    fn get_component_trace(&self, key: &[u8; 32]) -> Option<crate::program::trace::ComponentTrace> {
        self.backend.as_trace_store().get_component_trace(key)
    }

    fn put_component_trace(&self, key: [u8; 32], trace: crate::program::trace::ComponentTrace) {
        self.backend
            .as_trace_store()
            .put_component_trace(key, trace);
    }
}
