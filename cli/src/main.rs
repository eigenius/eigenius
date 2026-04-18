//! Eigenius CLI — primary developer interface for the Eigenius platform.

use clap::{Parser, Subcommand};
use eigenius_kernel::bootstrap;
use eigenius_kernel::layer::LayerBuilder;
use eigenius_kernel::ontology::{eigon_json, Iri};
use eigenius_kernel::validation::Validator;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "eigenius")]
#[command(about = "Eigenius — Typed Knowledge Graph Platform", long_about = None)]
#[command(version)]
struct Cli {
    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    /// Connect to a remote gRPC endpoint instead of using the local kernel
    #[arg(long, global = true)]
    endpoint: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Load an Eigon-JSON file as a new layer, validate against the stack
    Load {
        /// Path to Eigon-JSON file
        #[arg(value_name = "FILE")]
        file: String,
    },

    /// Validate an Eigon-JSON file without loading
    Validate {
        /// Path to Eigon-JSON file
        #[arg(value_name = "FILE")]
        file: String,
    },

    /// Execute an EigenQL query
    Query {
        /// EigenQL query string
        #[arg(value_name = "QUERY")]
        query: String,

        /// Optional Eigon-JSON file to load before querying
        #[arg(long, value_name = "FILE")]
        file: Option<String>,
    },

    /// Type-check a program
    ProgramValidate {
        /// Path to program Eigon-JSON file
        #[arg(value_name = "PROGRAM_FILE")]
        program_file: String,

        /// Optional ontology file to load first
        #[arg(long, value_name = "FILE")]
        ontology: Option<String>,
    },

    /// Execute a program
    /// Execute a program (requires --endpoint)
    Run {
        /// Path to program file (Eigon-JSON or ESL)
        #[arg(value_name = "PROGRAM_FILE")]
        program_file: String,

        /// Path to input file (Eigon-JSON or ESL)
        #[arg(value_name = "INPUT_FILE")]
        input_file: String,
    },

    /// Print a resource by IRI
    Inspect {
        /// IRI of the resource to inspect
        #[arg(value_name = "IRI")]
        iri: String,
    },

    /// Start the gRPC server
    Serve {
        /// gRPC port
        #[arg(long, default_value = "50051")]
        port: u16,

        /// Orchestrator endpoint for IO component dispatch
        #[arg(long, env = "EIGENIUS_ORCHESTRATOR_ENDPOINT")]
        orchestrator: Option<String>,
    },

    /// Database administration
    Db {
        #[command(subcommand)]
        command: DbCommands,
    },

    /// Compile an ESL file to Eigon-JSON
    Compile {
        /// Path to ESL (.esl) file
        #[arg(value_name = "FILE")]
        file: String,
    },

    /// Record a reasoning trace
    Reflect {
        /// Path to trace file (Eigon-JSON or ESL)
        #[arg(value_name = "FILE")]
        file: String,
    },

    /// List registered institutions (requires --endpoint)
    ListInstitutions,

    /// Generate JSON Schema for an ontology class (requires --endpoint)
    GetSchema {
        /// IRI of the class
        #[arg(value_name = "CLASS_IRI")]
        class_iri: String,
    },

    /// Show version and build info
    Version,
}

#[derive(Subcommand)]
enum DbCommands {
    /// Print storage statistics
    Stats {
        /// RocksDB path
        #[arg(value_name = "PATH")]
        path: String,
    },
    /// Trigger manual compaction
    Compact {
        /// RocksDB path
        #[arg(value_name = "PATH")]
        path: String,
    },
    /// Export all resources as Eigon-JSON
    Export {
        /// RocksDB path
        #[arg(value_name = "DB_PATH")]
        db_path: String,
        /// Output directory
        #[arg(value_name = "OUTPUT_PATH")]
        output_path: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Remote mode: delegate to gRPC client
    if let Some(ref endpoint) = cli.endpoint {
        match cli.command {
            Commands::Inspect { iri } => remote_inspect(endpoint, &iri, cli.json).await,
            Commands::Query { query, file: _ } => remote_query(endpoint, &query, cli.json).await,
            Commands::Run {
                program_file,
                input_file,
                ..
            } => remote_run(endpoint, &program_file, &input_file, cli.json).await,
            Commands::Load { file } => remote_load(endpoint, &file, cli.json).await,
            Commands::Reflect { file } => remote_reflect(endpoint, &file, cli.json).await,
            Commands::ListInstitutions => remote_list_institutions(endpoint, cli.json).await,
            Commands::GetSchema { class_iri } => {
                remote_get_schema(endpoint, &class_iri, cli.json).await
            }
            Commands::Serve { .. } => {
                eprintln!("Cannot use --endpoint with serve");
                std::process::exit(1);
            }
            _ => {
                eprintln!("Remote mode not yet supported for this command");
                std::process::exit(1);
            }
        }
        return;
    }

    // Local mode: embedded kernel
    match cli.command {
        Commands::Load { file } => cmd_load(&file, cli.json),
        Commands::Validate { file } => cmd_validate(&file, cli.json),
        Commands::Query { query, file } => cmd_query(&query, file.as_deref(), cli.json),
        Commands::ProgramValidate {
            program_file,
            ontology,
        } => cmd_program_validate(&program_file, ontology.as_deref(), cli.json),
        Commands::Run { .. } => {
            eprintln!("'run' requires --endpoint (connect to a running kernel+orchestrator)");
            eprintln!("  eigenius --endpoint http://localhost:50051 run program.json input.json");
            std::process::exit(1);
        }
        Commands::Inspect { iri } => cmd_inspect(&iri, cli.json),
        Commands::Serve { port, orchestrator } => cmd_serve(port, orchestrator.as_deref()).await,
        Commands::Compile { file } => cmd_compile(&file, cli.json),
        Commands::Reflect { file } => cmd_reflect(&file, cli.json),
        Commands::ListInstitutions => {
            eprintln!("'list-institutions' requires --endpoint");
            std::process::exit(1);
        }
        Commands::GetSchema { .. } => {
            eprintln!("'get-schema' requires --endpoint");
            std::process::exit(1);
        }
        Commands::Db { command } => cmd_db(command),
        Commands::Version => {
            println!("eigenius {}", env!("CARGO_PKG_VERSION"));
        }
    }
}

fn cmd_load(file: &str, json_output: bool) {
    // Bootstrap
    let mut ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    // Read and parse file (auto-detects ESL vs JSON)
    let resources = load_resources_from_file(file);
    let count = resources.len();

    // Add resources to context
    for resource in resources {
        if let Err(e) = ctx.add_resource(resource) {
            eprintln!("Error adding resource: {e}");
            std::process::exit(1);
        }
    }

    // Commit (validates and builds layer)
    match ctx.commit("loaded") {
        Ok(layer) => {
            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"resources\":{count},\"layer_id\":\"{}\"}}",
                    layer.id()
                );
            } else {
                println!("Loaded {count} resource(s) into layer {}", layer.id());
                println!("Validation passed.");
            }
        }
        Err(e) => {
            if json_output {
                eprintln!("{{\"status\":\"error\",\"message\":\"{e}\"}}");
            } else {
                eprintln!("Load failed: {e}");
            }
            std::process::exit(1);
        }
    }
}

fn cmd_query(query_str: &str, file: Option<&str>, json_output: bool) {
    // Bootstrap
    let mut ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    // Optionally load a file first
    if let Some(file_path) = file {
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to read '{file_path}': {e}");
                std::process::exit(1);
            }
        };
        let resources = match eigon_json::parse_document(&content) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Parse error: {e}");
                std::process::exit(1);
            }
        };
        for resource in resources {
            if let Err(e) = ctx.add_resource(resource) {
                eprintln!("Error adding resource: {e}");
                std::process::exit(1);
            }
        }
        if let Err(e) = ctx.commit("loaded") {
            eprintln!("Load failed: {e}");
            std::process::exit(1);
        }
    }

    // Execute query
    match eigenius_kernel::query::execute(query_str, ctx.head()) {
        Ok(result) => {
            if json_output {
                let json_results: Vec<serde_json::Value> = result
                    .resources
                    .iter()
                    .map(eigon_json::serialize_resource)
                    .collect();
                println!("{}", serde_json::to_string(&json_results).unwrap());
            } else {
                println!("{} result(s):", result.resources.len());
                for resource in &result.resources {
                    let json = eigon_json::serialize_resource(resource);
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
            }
        }
        Err(errors) => {
            if json_output {
                eprintln!("{{\"status\":\"error\",\"error_count\":{}}}", errors.len());
            } else {
                eprintln!("Query failed with {} error(s):", errors.len());
                for e in &errors {
                    eprintln!("  {e}");
                }
            }
            std::process::exit(1);
        }
    }
}

fn cmd_program_validate(program_file: &str, ontology: Option<&str>, json_output: bool) {
    let mut ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    // Load ontology if provided
    if let Some(ont_file) = ontology {
        load_file_into_context(&mut ctx, ont_file);
    }

    // Read and parse program
    let content = match std::fs::read_to_string(program_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read '{program_file}': {e}");
            std::process::exit(1);
        }
    };

    let resources = match eigon_json::parse_document(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Parse error: {e}");
            std::process::exit(1);
        }
    };

    let program = match resources.into_iter().next() {
        Some(r) => r,
        None => {
            eprintln!("No resources in program file");
            std::process::exit(1);
        }
    };

    // Parse and type-check
    match eigenius_kernel::program::expr::parse_program(&program, ctx.head()) {
        Ok((_term, typ)) => {
            // Validate output schemas (bijectivity check, D8 §4)
            let schema_errors =
                eigenius_kernel::program::schema::validate_output_schemas(&program, ctx.head());
            if !schema_errors.is_empty() {
                eprintln!("Schema validation failed:");
                for e in &schema_errors {
                    eprintln!("  {e}");
                }
                std::process::exit(1);
            }

            if json_output {
                println!("{{\"status\":\"ok\",\"type\":\"{typ:?}\"}}");
            } else {
                println!("Program type-checks successfully.");
                println!("Type: {typ:?}");
            }
        }
        Err(e) => {
            eprintln!("Program validation failed: {e}");
            std::process::exit(1);
        }
    }
}

fn load_file_into_context(ctx: &mut eigenius_kernel::context::ExecutionContext, file: &str) {
    let resources = load_resources_from_file(file);
    for resource in resources {
        if let Err(e) = ctx.add_resource(resource) {
            eprintln!("Error loading '{file}': {e}");
            std::process::exit(1);
        }
    }
    if let Err(e) = ctx.commit("loaded") {
        eprintln!("Commit failed for '{file}': {e}");
        std::process::exit(1);
    }
}

fn cmd_validate(file: &str, json_output: bool) {
    // Bootstrap
    let ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    // Read and parse file (auto-detects ESL vs JSON)
    let resources = load_resources_from_file(file);
    let count = resources.len();

    // Build a temporary layer for validation
    let mut builder = LayerBuilder::new("validate", Some(Arc::clone(ctx.head())));
    for resource in resources {
        if let Err(e) = builder.add_resource(resource) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
    let layer = builder.build();

    // Validate
    let validator = Validator::new(&layer);
    let errors = validator.validate();

    if errors.is_empty() {
        if json_output {
            println!("{{\"status\":\"ok\",\"resources\":{count}}}");
        } else {
            println!("Validated {count} resource(s). No errors.");
        }
    } else {
        if json_output {
            eprintln!("{{\"status\":\"error\",\"error_count\":{}}}", errors.len());
        } else {
            eprintln!("Validation found {} error(s):", errors.len());
            for e in &errors {
                eprintln!("  {e}");
            }
        }
        std::process::exit(1);
    }
}

fn cmd_inspect(iri_str: &str, json_output: bool) {
    // Bootstrap
    let ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    let iri = match Iri::parse(iri_str) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Invalid IRI '{iri_str}': {e}");
            std::process::exit(1);
        }
    };

    match ctx.resolve(&iri) {
        Some(resource) => {
            let json = eigon_json::serialize_resource(resource);
            if json_output {
                println!("{}", serde_json::to_string(&json).unwrap());
            } else {
                println!("{}", serde_json::to_string_pretty(&json).unwrap());
            }
        }
        None => {
            eprintln!("Resource not found: {iri_str}");
            std::process::exit(1);
        }
    }
}

fn cmd_compile(file: &str, json_output: bool) {
    let source = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("Failed to read file: {e}");
        std::process::exit(1);
    });

    let resources = eigenius_kernel::esl::compile(&source).unwrap_or_else(|errors| {
        for e in &errors {
            eprintln!("{file}: {e}");
        }
        std::process::exit(1);
    });

    // Output as Eigon-JSON array
    let json_values: Vec<serde_json::Value> = resources
        .iter()
        .map(eigon_json::serialize_resource)
        .collect();
    let output = serde_json::Value::Array(json_values);

    if json_output {
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    }
}

/// Load resources from a file, auto-detecting ESL (.esl) vs Eigon-JSON.
fn load_resources_from_file(file: &str) -> Vec<eigenius_kernel::ontology::resource::Resource> {
    let data = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("Failed to read file: {e}");
        std::process::exit(1);
    });

    if file.ends_with(".esl") {
        eigenius_kernel::esl::compile(&data).unwrap_or_else(|errors| {
            for e in &errors {
                eprintln!("{file}: {e}");
            }
            std::process::exit(1);
        })
    } else {
        eigon_json::parse_document(&data).unwrap_or_else(|e| {
            eprintln!("Failed to parse {file}: {e}");
            std::process::exit(1);
        })
    }
}

fn cmd_reflect(file: &str, json_output: bool) {
    let mut ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    let resources = load_resources_from_file(file);
    let count = resources.len();

    if resources.is_empty() {
        eprintln!("No trace resources found in file");
        std::process::exit(1);
    }

    let trace_iri = resources[0]
        .id()
        .map(|i| i.as_str().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    for resource in resources {
        if let Err(e) = ctx.add_resource(resource) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }

    if let Err(e) = ctx.commit("reflect") {
        eprintln!("Commit failed: {e}");
        std::process::exit(1);
    }

    if json_output {
        println!("{{\"success\":true,\"trace_iri\":\"{trace_iri}\",\"resource_count\":{count}}}");
    } else {
        println!("Recorded {count} trace resource(s). Trace IRI: {trace_iri}");
    }
}

fn cmd_db(command: DbCommands) {
    use eigenius_kernel::storage::{LayerStore, ResourceStore};

    match command {
        DbCommands::Stats { path } => {
            let store = eigenius_storage_rocksdb::RocksStore::open(std::path::Path::new(&path))
                .unwrap_or_else(|e| {
                    eprintln!("Failed to open database: {e}");
                    std::process::exit(1);
                });

            let rt = tokio::runtime::Runtime::new().unwrap();
            let layers = rt.block_on(store.list_layers()).unwrap_or_default();

            println!("Database: {path}");
            println!("Layers: {}", layers.len());

            let mut total_resources = 0;
            for layer_id in &layers {
                let resources = rt
                    .block_on(store.list_resources(layer_id))
                    .unwrap_or_default();
                total_resources += resources.len();
                println!("  Layer {}: {} resources", layer_id, resources.len());
            }
            println!("Total resources: {total_resources}");

            match store.get_head() {
                Ok(Some(head)) => println!("Head: {head}"),
                Ok(None) => println!("Head: (none)"),
                Err(e) => println!("Head: error ({e})"),
            }
        }
        DbCommands::Compact { path } => {
            let store = eigenius_storage_rocksdb::RocksStore::open(std::path::Path::new(&path))
                .unwrap_or_else(|e| {
                    eprintln!("Failed to open database: {e}");
                    std::process::exit(1);
                });
            store.compact();
            println!("Compaction complete.");
        }
        DbCommands::Export {
            db_path,
            output_path,
        } => {
            let store = eigenius_storage_rocksdb::RocksStore::open(std::path::Path::new(&db_path))
                .unwrap_or_else(|e| {
                    eprintln!("Failed to open database: {e}");
                    std::process::exit(1);
                });

            let rt = tokio::runtime::Runtime::new().unwrap();
            let layers = rt.block_on(store.list_layers()).unwrap_or_default();

            std::fs::create_dir_all(&output_path).unwrap_or_else(|e| {
                eprintln!("Failed to create output directory: {e}");
                std::process::exit(1);
            });

            for layer_id in &layers {
                let layer = rt.block_on(store.load_layer(layer_id)).unwrap();
                let resources: Vec<serde_json::Value> = layer
                    .resources()
                    .values()
                    .map(eigon_json::serialize_resource)
                    .collect();

                let json = serde_json::to_string_pretty(&resources).unwrap();
                let file_path =
                    std::path::Path::new(&output_path).join(format!("{}.json", layer_id));
                std::fs::write(&file_path, json).unwrap_or_else(|e| {
                    eprintln!("Failed to write {}: {e}", file_path.display());
                    std::process::exit(1);
                });
                println!(
                    "Exported layer {} ({} resources) → {}",
                    layer_id,
                    layer.resources().len(),
                    file_path.display()
                );
            }
        }
    }
}

async fn cmd_serve(port: u16, orchestrator: Option<&str>) {
    if let Err(e) = eigenius_kernel::server::start_server(port, orchestrator).await {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }
}

// --- Remote mode (gRPC client) ---

/// Read a file, compiling ESL to Eigon-JSON if needed. Returns JSON bytes.
fn read_as_json(file: &str) -> Vec<u8> {
    if file.ends_with(".esl") {
        let source = std::fs::read_to_string(file).unwrap_or_else(|e| {
            eprintln!("Failed to read {file}: {e}");
            std::process::exit(1);
        });
        let resources = eigenius_kernel::esl::compile(&source).unwrap_or_else(|errors| {
            for e in &errors {
                eprintln!("{file}: {e}");
            }
            std::process::exit(1);
        });
        let json_values: Vec<serde_json::Value> = resources
            .iter()
            .map(eigon_json::serialize_resource)
            .collect();
        serde_json::to_vec(&json_values).unwrap()
    } else {
        std::fs::read(file).unwrap_or_else(|e| {
            eprintln!("Failed to read {file}: {e}");
            std::process::exit(1);
        })
    }
}

fn content_type_for_file(file: &str) -> String {
    if file.ends_with(".esl") {
        "application/esl".to_string()
    } else if file.ends_with(".cbor") {
        "application/cbor".to_string()
    } else {
        "application/eigon+json".to_string()
    }
}

use eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient;

async fn connect_client(endpoint: &str) -> EigeniusKernelClient<tonic::transport::Channel> {
    EigeniusKernelClient::connect(endpoint.to_string())
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to connect to {endpoint}: {e}");
            std::process::exit(1);
        })
}

async fn remote_inspect(endpoint: &str, iri_str: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    let request = eigenius_kernel::server::proto::InspectRequest {
        iri: iri_str.to_string(),
    };

    match client.inspect(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.found {
                let resource =
                    eigenius_kernel::ontology::eigon_cbor::parse_resource(&resp.resource)
                        .unwrap_or_else(|e| {
                            eprintln!("Failed to parse response: {e}");
                            std::process::exit(1);
                        });
                let json = eigon_json::serialize_resource(&resource);
                if json_output {
                    println!("{}", serde_json::to_string(&json).unwrap());
                } else {
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
            } else {
                eprintln!("Resource not found: {iri_str}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_query(endpoint: &str, eigenql: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    let request = eigenius_kernel::server::proto::QueryRequest {
        eigenql: eigenql.to_string(),
    };

    match client.query(request).await {
        Ok(response) => {
            let mut stream = response.into_inner();
            let mut count = 0u64;
            while let Ok(Some(result)) = stream.message().await {
                let resource =
                    eigenius_kernel::ontology::eigon_cbor::parse_resource_lenient(&result.resource)
                        .unwrap_or_else(|e| {
                            eprintln!("Failed to parse result: {e}");
                            std::process::exit(1);
                        });
                let json = eigon_json::serialize_resource(&resource);
                if json_output {
                    println!("{}", serde_json::to_string(&json).unwrap());
                } else {
                    if count == 0 {
                        println!("Results:");
                    }
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
                count += 1;
            }
            if !json_output {
                println!("{count} result(s).");
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_run(endpoint: &str, program_file: &str, input_file: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    // Compile ESL files client-side since program and input may have different formats
    let program_data = read_as_json(program_file);
    let input_data = read_as_json(input_file);

    let request = eigenius_kernel::server::proto::RunProgramRequest {
        program: program_data,
        input: input_data,
        content_type: "application/eigon+json".to_string(),
    };

    match client.run_program(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                let resource =
                    eigenius_kernel::ontology::eigon_cbor::parse_resource_lenient(&resp.output)
                        .unwrap_or_else(|e| {
                            eprintln!("Failed to parse output: {e}");
                            std::process::exit(1);
                        });
                let json = eigon_json::serialize_resource(&resource);
                if json_output {
                    println!("{}", serde_json::to_string(&json).unwrap());
                } else {
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
            } else {
                eprintln!("Program execution failed:");
                for err in &resp.errors {
                    eprintln!("  {}: {}", err.rule, err.message);
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_load(endpoint: &str, file: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    let data = std::fs::read(file).unwrap_or_else(|e| {
        eprintln!("Failed to read file: {e}");
        std::process::exit(1);
    });

    let content_type = content_type_for_file(file);
    let request = eigenius_kernel::server::proto::LoadRequest {
        resources: data,
        content_type,
        auto_commit: true,
    };

    match client.load(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                if json_output {
                    println!(
                        "{{\"success\":true,\"resource_count\":{},\"layer_id\":\"{}\"}}",
                        resp.resource_count, resp.layer_id
                    );
                } else {
                    println!(
                        "Loaded {} resource(s). Layer: {}",
                        resp.resource_count, resp.layer_id
                    );
                }
            } else {
                eprintln!("Load failed:");
                for err in &resp.errors {
                    eprintln!("  {}: {}", err.rule, err.message);
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_reflect(endpoint: &str, file: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    let data = read_as_json(file);

    let request = eigenius_kernel::server::proto::ReflectRequest {
        trace: data,
        content_type: "application/eigon+json".to_string(),
    };

    match client.reflect(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                if json_output {
                    println!("{{\"success\":true,\"trace_iri\":\"{}\"}}", resp.trace_iri);
                } else {
                    println!("Recorded trace: {}", resp.trace_iri);
                }
            } else {
                eprintln!("Reflect failed");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_list_institutions(endpoint: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    match client
        .list_institutions(eigenius_kernel::server::proto::ListInstitutionsRequest {})
        .await
    {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let json: Vec<serde_json::Value> = resp
                    .institutions
                    .iter()
                    .map(|i| {
                        serde_json::json!({
                            "iri": i.iri,
                            "name": i.name,
                            "morphism_types": i.morphism_types,
                            "query_types": i.query_types,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string(&json).unwrap());
            } else if resp.institutions.is_empty() {
                println!("No institutions registered.");
            } else {
                println!("Registered institutions:");
                for inst in &resp.institutions {
                    println!("  {} ({})", inst.name, inst.iri);
                    for mt in &inst.morphism_types {
                        println!("    morphism: {mt}");
                    }
                    for qt in &inst.query_types {
                        println!("    query:    {qt}");
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_get_schema(endpoint: &str, class_iri: &str, _json_output: bool) {
    let mut client = connect_client(endpoint).await;

    match client
        .get_schema(eigenius_kernel::server::proto::GetSchemaRequest {
            class_iri: class_iri.to_string(),
        })
        .await
    {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                println!("{}", resp.json_schema);
            } else {
                eprintln!("Schema generation failed: {}", resp.error);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}
