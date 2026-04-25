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

        /// Evaluate the query against a specific LayerId (hex-encoded SHA-256)
        /// instead of the session's active top (D21 §3.6). Useful for
        /// reaching a forked task result layer. Remote mode only.
        #[arg(long, value_name = "LAYER_ID")]
        at_layer: Option<String>,
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

        /// Resolve against a specific LayerId (hex-encoded SHA-256)
        /// instead of the session's active top (D21 §3.6). Remote
        /// mode only.
        #[arg(long, value_name = "LAYER_ID")]
        at_layer: Option<String>,
    },

    /// Start the gRPC server
    Serve {
        /// gRPC port
        #[arg(long, default_value = "50051")]
        port: u16,

        /// Orchestrator endpoint for IO component dispatch
        #[arg(long, env = "EIGENIUS_ORCHESTRATOR_ENDPOINT")]
        orchestrator: Option<String>,

        /// Path to a RocksDB directory for persistent state. When omitted,
        /// the server runs in-memory and loses all state on exit.
        /// See D13 — Durable Kernel State.
        #[arg(long, env = "EIGENIUS_DB", value_name = "PATH")]
        db: Option<String>,
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

    /// Manage WASM capabilities (components and institutions)
    Capability {
        #[command(subcommand)]
        command: CapabilityCommands,
    },

    /// Inspect and control persisted tasks (D21). Remote mode only.
    Tasks {
        #[command(subcommand)]
        command: TaskCommands,
    },

    /// Show version and build info
    Version,
}

#[derive(Subcommand)]
enum TaskCommands {
    /// List all tasks in the session
    List,

    /// Show a task's status and metadata
    Status {
        /// Task UUID
        #[arg(value_name = "TASK_ID")]
        task_id: String,
    },

    /// Request cooperative cancellation of a task
    Cancel {
        /// Task UUID
        #[arg(value_name = "TASK_ID")]
        task_id: String,
    },
}

#[derive(Subcommand)]
enum CapabilityCommands {
    /// List registered components and institutions
    List,

    /// Inspect a registered capability by IRI
    Inspect {
        /// IRI of the component or institution
        #[arg(value_name = "IRI")]
        iri: String,
    },

    /// Install a WASM component or institution
    Install {
        /// Path to the WASM binary file (built with cargo-component)
        #[arg(value_name = "WASM_FILE")]
        binary: String,

        /// Path to an Eigon-JSON or ESL file declaring the capability.
        /// The file should contain the component/institution resource
        /// with its type declarations; the CLI fills in `wasm_binary`
        /// and `implementation: "wasm"`.
        #[arg(long, value_name = "FILE")]
        definition: Option<String>,

        /// Quick mode: IRI for the capability when no definition file is provided
        #[arg(long, value_name = "IRI", conflicts_with = "definition")]
        as_iri: Option<String>,

        /// Quick mode: kind of capability (component or institution)
        #[arg(long, value_name = "KIND", default_value = "component")]
        kind: String,

        /// Quick mode: capability level (pure, read, or io)
        #[arg(long, value_name = "LEVEL", default_value = "pure")]
        capability: String,

        /// Quick mode: input_type IRI (components only)
        #[arg(long, value_name = "IRI")]
        input_type: Option<String>,

        /// Quick mode: output_type IRI (components only)
        #[arg(long, value_name = "IRI")]
        output_type: Option<String>,
    },

    /// Invoke a registered capability with test input
    Test {
        /// IRI of the capability to test
        #[arg(value_name = "IRI")]
        iri: String,

        /// Input file (Eigon-JSON or ESL)
        #[arg(long, value_name = "FILE")]
        input: String,

        /// For institutions: dispatch as fiber query (default) or discover-morphisms
        #[arg(long, value_name = "MODE", default_value = "query")]
        mode: String,
    },
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
            Commands::Inspect { iri, at_layer } => {
                remote_inspect(endpoint, &iri, at_layer.as_deref(), cli.json).await
            }
            Commands::Query {
                query,
                file: _,
                at_layer,
            } => remote_query(endpoint, &query, at_layer.as_deref(), cli.json).await,
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
            Commands::Capability { command } => {
                remote_capability(endpoint, command, cli.json).await
            }
            Commands::Tasks { command } => remote_tasks(endpoint, command, cli.json).await,
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
        Commands::Query { query, file, .. } => cmd_query(&query, file.as_deref(), cli.json),
        Commands::ProgramValidate {
            program_file,
            ontology,
        } => cmd_program_validate(&program_file, ontology.as_deref(), cli.json),
        Commands::Run { .. } => {
            eprintln!("'run' requires --endpoint (connect to a running kernel+orchestrator)");
            eprintln!("  eigenius --endpoint http://localhost:50051 run program.json input.json");
            std::process::exit(1);
        }
        Commands::Inspect { iri, .. } => cmd_inspect(&iri, cli.json),
        Commands::Serve {
            port,
            orchestrator,
            db,
        } => cmd_serve(port, orchestrator.as_deref(), db.as_deref()).await,
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
        Commands::Capability { .. } => {
            eprintln!("'capability' commands require --endpoint");
            std::process::exit(1);
        }
        Commands::Tasks { .. } => {
            eprintln!("'tasks' commands require --endpoint");
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

    // Execute query — returns the full result document (Property resources,
    // row Class, ResultSet with embedded rows) per D2 Appendix A.
    match eigenius_kernel::query::execute(query_str, ctx.head()) {
        Ok(document) => {
            if json_output {
                let json_results: Vec<serde_json::Value> = document
                    .iter()
                    .map(eigon_json::serialize_resource)
                    .collect();
                println!("{}", serde_json::to_string(&json_results).unwrap());
            } else {
                println!("{} resource(s) in result document:", document.len());
                for resource in &document {
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

async fn cmd_serve(port: u16, orchestrator: Option<&str>, db: Option<&str>) {
    let backend: Option<std::sync::Arc<dyn eigenius_kernel::storage::PersistentBackend>> = match db
    {
        Some(path) => {
            match eigenius_storage_rocksdb::RocksStore::open(std::path::Path::new(path)) {
                Ok(store) => {
                    println!("Opened persistent backend at {path}");
                    Some(std::sync::Arc::new(store))
                }
                Err(e) => {
                    eprintln!("Failed to open --db {path}: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => None,
    };

    if let Err(e) = eigenius_kernel::server::start_server(port, orchestrator, backend).await {
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
    let channel = tonic::transport::Endpoint::from_shared(endpoint.to_string())
        .unwrap_or_else(|e| {
            eprintln!("Invalid endpoint '{endpoint}': {e}");
            std::process::exit(1);
        })
        .connect()
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to connect to {endpoint}: {e}");
            std::process::exit(1);
        });
    // Raise gRPC message size limits to 128 MB to accommodate WASM component
    // binaries (which are base64-encoded and can be multiple MB).
    EigeniusKernelClient::new(channel)
        .max_decoding_message_size(128 * 1024 * 1024)
        .max_encoding_message_size(128 * 1024 * 1024)
}

async fn remote_inspect(endpoint: &str, iri_str: &str, at_layer: Option<&str>, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    let request = eigenius_kernel::server::proto::InspectRequest {
        at_layer: at_layer.unwrap_or("").to_string(),
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

async fn remote_query(endpoint: &str, eigenql: &str, at_layer: Option<&str>, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    let request = eigenius_kernel::server::proto::QueryRequest {
        at_layer: at_layer.unwrap_or("").to_string(),
        eigenql: eigenql.to_string(),
    };

    match client.query(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if !resp.success {
                eprintln!("Query failed: {}", resp.error);
                std::process::exit(1);
            }
            let document = eigenius_kernel::ontology::eigon_cbor::parse_document(&resp.document)
                .unwrap_or_else(|e| {
                    eprintln!("Failed to parse result document: {e}");
                    std::process::exit(1);
                });
            if json_output {
                let arr: Vec<serde_json::Value> = document
                    .iter()
                    .map(eigon_json::serialize_resource)
                    .collect();
                println!("{}", serde_json::to_string(&arr).unwrap());
            } else {
                println!("{} resource(s) in result document:", document.len());
                for r in &document {
                    let json = eigon_json::serialize_resource(r);
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
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
        .list_institutions(eigenius_kernel::server::proto::ListInstitutionsRequest {
            at_layer: String::new(),
        })
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
            at_layer: String::new(),
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

// --- Capability subcommand ---

async fn remote_tasks(endpoint: &str, command: TaskCommands, json: bool) {
    match command {
        TaskCommands::List => remote_tasks_list(endpoint, json).await,
        TaskCommands::Status { task_id } => remote_task_status(endpoint, &task_id, json).await,
        TaskCommands::Cancel { task_id } => remote_task_cancel(endpoint, &task_id, json).await,
    }
}

async fn remote_tasks_list(endpoint: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::ListTasksRequest {};
    match client.list_tasks(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let items: Vec<serde_json::Value> = resp
                    .tasks
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "task_id": t.task_id,
                            "program_iri": t.program_iri,
                            "status": t.status,
                            "layer_head": t.layer_head,
                            "step_seq": t.step_seq,
                            "result_layer_head": t.result_layer_head,
                            "created_at_ms": t.created_at_ms,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&items).unwrap());
            } else if resp.tasks.is_empty() {
                println!("No tasks.");
            } else {
                println!("{:<36}  {:<12}  PROGRAM", "TASK ID", "STATUS");
                for t in &resp.tasks {
                    println!("{:<36}  {:<12}  {}", t.task_id, t.status, t.program_iri);
                }
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_task_status(endpoint: &str, task_id: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::GetTaskStatusRequest {
        task_id: task_id.to_string(),
    };
    match client.get_task_status(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if !resp.found {
                eprintln!("Task not found: {task_id}");
                std::process::exit(1);
            }
            let t = resp.task.unwrap();
            if json_output {
                let j = serde_json::json!({
                    "task_id": t.task_id,
                    "session_id": t.session_id,
                    "program_iri": t.program_iri,
                    "input_iri": t.input_iri,
                    "status": t.status,
                    "layer_head": t.layer_head,
                    "step_seq": t.step_seq,
                    "latest_trace_seq": t.latest_trace_seq,
                    "last_checkpoint_step": t.last_checkpoint_step,
                    "result_layer_head": t.result_layer_head,
                    "created_at_ms": t.created_at_ms,
                    "updated_at_ms": t.updated_at_ms,
                    "retain_forever": t.retain_forever,
                });
                println!("{}", serde_json::to_string_pretty(&j).unwrap());
            } else {
                println!("Task:         {}", t.task_id);
                println!("Status:       {}", t.status);
                println!("Program:      {}", t.program_iri);
                println!("Input:        {}", t.input_iri);
                println!("Layer head:   {}", t.layer_head);
                println!("Step seq:     {}", t.step_seq);
                println!("Last ckpt:    {}", t.last_checkpoint_step);
                if !t.result_layer_head.is_empty() {
                    println!("Result layer: {}", t.result_layer_head);
                }
                println!(
                    "Created:      {} ms (unix epoch)\nUpdated:      {} ms",
                    t.created_at_ms, t.updated_at_ms
                );
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_task_cancel(endpoint: &str, task_id: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::CancelTaskRequest {
        task_id: task_id.to_string(),
    };
    match client.cancel_task(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let j = serde_json::json!({
                    "success": resp.success,
                    "status": resp.status,
                    "error": resp.error,
                });
                println!("{}", serde_json::to_string_pretty(&j).unwrap());
            } else if resp.success {
                println!("Task {task_id}: {}", resp.status);
            } else {
                eprintln!("Cancel failed: {}", resp.error);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_capability(endpoint: &str, command: CapabilityCommands, json: bool) {
    match command {
        CapabilityCommands::List => remote_capability_list(endpoint, json).await,
        CapabilityCommands::Inspect { iri } => {
            remote_capability_inspect(endpoint, &iri, json).await
        }
        CapabilityCommands::Install {
            binary,
            definition,
            as_iri,
            kind,
            capability,
            input_type,
            output_type,
        } => {
            remote_capability_install(
                endpoint,
                &binary,
                definition.as_deref(),
                as_iri.as_deref(),
                &kind,
                &capability,
                input_type.as_deref(),
                output_type.as_deref(),
                json,
            )
            .await
        }
        CapabilityCommands::Test { iri, input, mode } => {
            remote_capability_test(endpoint, &iri, &input, &mode, json).await
        }
    }
}

async fn remote_capability_list(endpoint: &str, json: bool) {
    let mut client = connect_client(endpoint).await;

    // Find all Component resources
    let components_query = r#"
        MATCH "urn:eigenius:program:Component"(?c) {
            "urn:eigenius:core:short_name": ?name
        }
        RETURN [] { iri: ?c, name: ?name }
    "#;

    // Find all Institution resources
    let institutions_query = r#"
        MATCH "urn:eigenius:institution:Institution"(?i) {
            "urn:eigenius:institution:institution_name": ?name
        }
        RETURN [] { iri: ?i, name: ?name }
    "#;

    let components = run_query(&mut client, components_query).await;
    let institutions = run_query(&mut client, institutions_query).await;

    if json {
        println!(
            "{{\"components\":{},\"institutions\":{}}}",
            serde_json::to_string(&components).unwrap(),
            serde_json::to_string(&institutions).unwrap()
        );
    } else {
        println!("Components:");
        if components.is_empty() {
            println!("  (none registered)");
        } else {
            for r in &components {
                let iri = r.get("iri").and_then(|v| v.as_str()).unwrap_or("?");
                let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  {name} ({iri})");
            }
        }
        println!();
        println!("Institutions:");
        if institutions.is_empty() {
            println!("  (none registered)");
        } else {
            for r in &institutions {
                let iri = r.get("iri").and_then(|v| v.as_str()).unwrap_or("?");
                let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  {name} ({iri})");
            }
        }
    }
}

/// Run a remote EigenQL query and materialise its rows into plain JSON
/// objects keyed by the RETURN clause's short names.
///
/// Walks the response document per D2 Appendix A:
///   1. Parse the response bytes as an Eigon document.
///   2. Find the ResultSet (has `is_a` including `urn:eigenius:query:ResultSet`).
///   3. Find the row class (`result_class` IRI points at a Class resource
///      in the same document).
///   4. For each Property IRI listed on the class, read its `short_name`
///      and build a short_name → property IRI map.
///   5. For each embedded row in `rows`, emit a JSON object keyed by
///      short name.
///
/// Callers access values by the short name they put in the RETURN clause
/// — e.g. `row.get("iri")` when the query said `RETURN [] { iri: ?c }`.
async fn run_query(
    client: &mut eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient<
        tonic::transport::Channel,
    >,
    eigenql: &str,
) -> Vec<serde_json::Value> {
    use eigenius_kernel::ontology::eigon_cbor;
    use eigenius_kernel::ontology::iri::Iri;
    use eigenius_kernel::ontology::resource::{Resource, Value as RValue};
    use eigenius_kernel::ontology::well_known as wk;
    use eigenius_kernel::query::document as qdoc;

    let resp = match client
        .query(eigenius_kernel::server::proto::QueryRequest {
            at_layer: String::new(),
            eigenql: eigenql.to_string(),
        })
        .await
    {
        Ok(r) => r.into_inner(),
        Err(e) => {
            eprintln!("Query failed: {e}");
            return Vec::new();
        }
    };
    if !resp.success {
        eprintln!("Query failed: {}", resp.error);
        return Vec::new();
    }

    let document = match eigon_cbor::parse_document(&resp.document) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to parse result document: {e}");
            return Vec::new();
        }
    };

    // Index the document by IRI for class/property lookup.
    let by_iri: std::collections::BTreeMap<String, &Resource> = document
        .iter()
        .filter_map(|r| r.id().map(|iri| (iri.as_str().to_string(), r)))
        .collect();

    // Locate the ResultSet.
    let is_a_iri = Iri::parse(wk::IS_A).unwrap();
    let rs_class = qdoc::RESULT_SET_CLASS;
    let result_set = document.iter().find(|r| match r.get(&is_a_iri) {
        Some(RValue::Array(a)) => a.iter().any(|v| s_as_str(v) == rs_class),
        _ => false,
    });
    let result_set = match result_set {
        Some(r) => r,
        None => return Vec::new(),
    };

    // Walk to the row class.
    let row_class_iri = match result_set.get(&Iri::parse(qdoc::RESULT_CLASS_PROP).unwrap()) {
        Some(RValue::String(s)) => s.clone(),
        Some(RValue::ResourceRef(i)) => i.as_str().to_string(),
        _ => return Vec::new(),
    };
    let row_class = match by_iri.get(&row_class_iri) {
        Some(c) => *c,
        None => return Vec::new(),
    };

    // Build short_name → property IRI map from the class's property list.
    let properties_prop = Iri::parse("urn:eigenius:core:properties").unwrap();
    let short_name_prop = Iri::parse(wk::SHORT_NAME).unwrap();
    let mut short_to_iri: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    if let Some(RValue::Array(props)) = row_class.get(&properties_prop) {
        for p in props {
            let prop_iri = match p {
                RValue::String(s) => s.clone(),
                RValue::ResourceRef(i) => i.as_str().to_string(),
                _ => continue,
            };
            let Some(prop_res) = by_iri.get(&prop_iri) else {
                continue;
            };
            if let Some(RValue::String(short)) = prop_res.get(&short_name_prop) {
                short_to_iri.insert(short.clone(), prop_iri);
            }
        }
    }

    // Iterate rows (embedded inside the ResultSet) and project each into a
    // JSON object keyed by short name.
    let mut out = Vec::new();
    if let Some(RValue::Array(rows)) = result_set.get(&Iri::parse(qdoc::ROWS_PROP).unwrap()) {
        for row_val in rows {
            let row = match row_val {
                RValue::Embedded(r) => r.as_ref(),
                _ => continue,
            };
            let mut obj = serde_json::Map::new();
            for (short, iri_str) in &short_to_iri {
                let Ok(iri) = Iri::parse(iri_str) else {
                    continue;
                };
                if let Some(v) = row.get(&iri) {
                    if let Some(json) = value_to_json(v) {
                        obj.insert(short.clone(), json);
                    }
                }
            }
            out.push(serde_json::Value::Object(obj));
        }
    }
    out
}

fn s_as_str(v: &eigenius_kernel::ontology::resource::Value) -> &str {
    use eigenius_kernel::ontology::resource::Value;
    match v {
        Value::String(s) => s.as_str(),
        Value::ResourceRef(i) => i.as_str(),
        _ => "",
    }
}

fn value_to_json(v: &eigenius_kernel::ontology::resource::Value) -> Option<serde_json::Value> {
    use eigenius_kernel::ontology::resource::Value;
    Some(match v {
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::ResourceRef(i) => serde_json::Value::String(i.as_str().to_string()),
        Value::Integer(n) => serde_json::Value::Number((*n).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f).map(serde_json::Value::Number)?,
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        _ => return None,
    })
}

async fn remote_capability_inspect(endpoint: &str, iri: &str, json: bool) {
    let mut client = connect_client(endpoint).await;

    let request = eigenius_kernel::server::proto::InspectRequest {
        at_layer: String::new(),
        iri: iri.to_string(),
    };

    match client.inspect(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if !resp.found {
                eprintln!("Capability not found: {iri}");
                std::process::exit(1);
            }
            let resource =
                match eigenius_kernel::ontology::eigon_cbor::parse_resource(&resp.resource) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("Failed to parse resource: {e}");
                        std::process::exit(1);
                    }
                };
            if json {
                let v = eigenius_kernel::ontology::eigon_json::serialize_resource(&resource);
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                print_capability_human(&resource);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

fn print_capability_human(resource: &eigenius_kernel::ontology::resource::Resource) {
    use eigenius_kernel::ontology::iri::Iri;
    use eigenius_kernel::ontology::resource::Value;

    let get = |key: &str| -> Option<&Value> { resource.get(&Iri::parse(key).unwrap()) };
    let get_str =
        |key: &str| -> Option<String> { get(key).and_then(|v| v.as_str()).map(|s| s.to_string()) };

    if let Some(id) = resource.id() {
        println!("IRI:             {}", id.as_str());
    }
    if let Some(name) = get_str("urn:eigenius:core:short_name") {
        println!("Name:            {name}");
    }
    if let Some(desc) = get_str("urn:eigenius:core:description") {
        println!("Description:     {desc}");
    }

    // is_a
    let is_a_iris = resource.is_a();
    if !is_a_iris.is_empty() {
        let classes: Vec<String> = is_a_iris.iter().map(|i| i.as_str().to_string()).collect();
        println!("Classes:         {}", classes.join(", "));
    }

    // Component-specific
    if let Some(impl_) = get_str("urn:eigenius:program:component:implementation") {
        println!("Implementation:  {impl_}");
    }
    if let Some(cap) = get_str("urn:eigenius:program:component:capability_level") {
        println!("Capability:      {cap}");
    }
    if let Some(input) = get_str("urn:eigenius:program:component:input_type") {
        println!("Input type:      {input}");
    }
    if let Some(output) = get_str("urn:eigenius:program:component:output_type") {
        println!("Output type:     {output}");
    }
    if let Some(arg) = get_str("urn:eigenius:program:component:argument_type") {
        println!("Argument type:   {arg}");
    }

    // Institution-specific
    if let Some(impl_) = get_str("urn:eigenius:institution:implementation") {
        println!("Implementation:  {impl_}");
    }
    if let Some(inst_iri) = get_str("urn:eigenius:institution:institution_iri") {
        println!("Institution IRI: {inst_iri}");
    }

    // WASM metadata (size in bytes for binary)
    let wasm_bytes = get("urn:eigenius:program:component:wasm_binary")
        .or_else(|| get("urn:eigenius:institution:wasm_binary"))
        .and_then(|v| v.as_str());
    if let Some(b64) = wasm_bytes {
        // Rough decoded size: base64 is 4/3 of raw bytes
        let estimated_size = b64.len() * 3 / 4;
        println!("WASM binary:     ~{estimated_size} bytes (inline base64)");
    }

    // Fuel/memory config
    if let Some(Value::Integer(n)) = get("urn:eigenius:program:component:fuel_limit")
        .or_else(|| get("urn:eigenius:institution:fuel_limit"))
    {
        println!("Fuel limit:      {n}");
    }
    if let Some(Value::Integer(n)) = get("urn:eigenius:program:component:memory_limit_pages")
        .or_else(|| get("urn:eigenius:institution:memory_limit_pages"))
    {
        println!("Memory limit:    {n} pages ({} MB)", n * 64 / 1024);
    }
}

#[allow(clippy::too_many_arguments)]
async fn remote_capability_install(
    endpoint: &str,
    binary_file: &str,
    definition_file: Option<&str>,
    as_iri: Option<&str>,
    kind: &str,
    capability: &str,
    input_type: Option<&str>,
    output_type: Option<&str>,
    json: bool,
) {
    let wasm_bytes = std::fs::read(binary_file).unwrap_or_else(|e| {
        eprintln!("Failed to read WASM file '{binary_file}': {e}");
        std::process::exit(1);
    });
    let base64_binary = encode_base64(&wasm_bytes);

    let resource_json = if let Some(def_file) = definition_file {
        merge_definition_with_binary(def_file, &base64_binary, kind)
    } else {
        let iri = as_iri.unwrap_or_else(|| {
            eprintln!("'install' requires either --definition or --as (quick mode)");
            std::process::exit(1);
        });
        generate_quick_resource(
            iri,
            kind,
            capability,
            &base64_binary,
            input_type,
            output_type,
        )
    };

    // Send via load RPC with auto_commit
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::LoadRequest {
        resources: resource_json.into_bytes(),
        content_type: "application/eigon+json".to_string(),
        auto_commit: true,
    };

    match client.load(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                if json {
                    println!(
                        "{{\"success\":true,\"resource_count\":{},\"layer_id\":\"{}\"}}",
                        resp.resource_count, resp.layer_id
                    );
                } else {
                    println!(
                        "Installed {} resource(s). Layer: {}",
                        resp.resource_count, resp.layer_id
                    );
                    println!(
                        "(WASM binary: {} bytes from {binary_file})",
                        wasm_bytes.len()
                    );
                }
            } else {
                eprintln!("Install failed:");
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

fn merge_definition_with_binary(def_file: &str, base64_binary: &str, kind: &str) -> String {
    // Read and compile to JSON if needed
    let json_bytes = read_as_json(def_file);
    let json_str = String::from_utf8(json_bytes).unwrap_or_else(|e| {
        eprintln!("Definition file is not valid UTF-8: {e}");
        std::process::exit(1);
    });

    let mut value: serde_json::Value = serde_json::from_str(&json_str).unwrap_or_else(|e| {
        eprintln!("Failed to parse definition as JSON: {e}");
        std::process::exit(1);
    });

    // The definition may be a single object or an array — find the top-level
    // capability resource and patch in the wasm_binary + implementation.
    let (binary_prop, impl_prop) = match kind {
        "component" => (
            "urn:eigenius:program:component:wasm_binary",
            "urn:eigenius:program:component:implementation",
        ),
        "institution" => (
            "urn:eigenius:institution:wasm_binary",
            "urn:eigenius:institution:implementation",
        ),
        other => {
            eprintln!("Unknown --kind: {other} (expected 'component' or 'institution')");
            std::process::exit(1);
        }
    };

    fn patch(
        obj: &mut serde_json::Map<String, serde_json::Value>,
        binary_prop: &str,
        impl_prop: &str,
        binary: &str,
    ) {
        obj.insert(
            binary_prop.to_string(),
            serde_json::Value::String(binary.to_string()),
        );
        obj.insert(
            impl_prop.to_string(),
            serde_json::Value::String("wasm".to_string()),
        );
    }

    match &mut value {
        serde_json::Value::Object(obj) => patch(obj, binary_prop, impl_prop, base64_binary),
        serde_json::Value::Array(arr) => {
            // Patch the first top-level object with @id (that's the capability resource)
            let mut patched = false;
            for item in arr.iter_mut() {
                if let serde_json::Value::Object(obj) = item {
                    if obj.contains_key("@id") {
                        patch(obj, binary_prop, impl_prop, base64_binary);
                        patched = true;
                        break;
                    }
                }
            }
            if !patched {
                eprintln!("Definition file contains no top-level resource with @id");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Definition file root must be an object or array");
            std::process::exit(1);
        }
    }

    serde_json::to_string(&value).unwrap()
}

fn generate_quick_resource(
    iri: &str,
    kind: &str,
    capability: &str,
    base64_binary: &str,
    input_type: Option<&str>,
    output_type: Option<&str>,
) -> String {
    use serde_json::json;

    match kind {
        "component" => {
            let input = input_type.unwrap_or("urn:eigenius:core:Class");
            let output = output_type.unwrap_or("urn:eigenius:core:Class");
            let cap_iri = match capability {
                "pure" => "urn:eigenius:program:capability_levels:pure",
                "read" => "urn:eigenius:program:capability_levels:read",
                "io" => "urn:eigenius:program:capability_levels:io",
                other => {
                    eprintln!("Unknown --capability: {other} (expected 'pure', 'read', or 'io')");
                    std::process::exit(1);
                }
            };
            json!({
                "@id": iri,
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Component"],
                "urn:eigenius:core:short_name": iri.rsplit(':').next().unwrap_or(iri),
                "urn:eigenius:program:component:input_type": input,
                "urn:eigenius:program:component:output_type": output,
                "urn:eigenius:program:component:capability_level": cap_iri,
                "urn:eigenius:program:component:implementation": "wasm",
                "urn:eigenius:program:component:wasm_binary": base64_binary,
            })
            .to_string()
        }
        "institution" => json!({
            "@id": iri,
            "urn:eigenius:core:is_a": ["urn:eigenius:institution:Institution"],
            "urn:eigenius:institution:institution_iri": iri,
            "urn:eigenius:institution:institution_name": iri.rsplit(':').next().unwrap_or(iri),
            "urn:eigenius:institution:implementation": "wasm",
            "urn:eigenius:institution:wasm_binary": base64_binary,
        })
        .to_string(),
        other => {
            eprintln!("Unknown --kind: {other} (expected 'component' or 'institution')");
            std::process::exit(1);
        }
    }
}

/// Encode bytes as standard base64 (RFC 4648, with padding).
fn encode_base64(bytes: &[u8]) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        out.push(ALPHA[(b0 >> 2) as usize] as char);
        out.push(ALPHA[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHA[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHA[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

async fn remote_capability_test(
    endpoint: &str,
    iri: &str,
    input_file: &str,
    mode: &str,
    json: bool,
) {
    let mut client = connect_client(endpoint).await;

    // Detect institution-hood via ListInstitutions (the authoritative view —
    // institutions may register under a binary-declared IRI that differs from
    // the ontology resource's @id).
    let institutions = client
        .list_institutions(eigenius_kernel::server::proto::ListInstitutionsRequest {
            at_layer: String::new(),
        })
        .await
        .map(|r| r.into_inner().institutions)
        .unwrap_or_default();

    let is_institution = institutions.iter().any(|i| i.iri == iri);

    let input_json = read_as_json(input_file);

    if is_institution {
        let req = if mode == "discover" {
            eigenius_kernel::server::proto::DiscoverMorphismsRequest {
                institution_iri: iri.to_string(),
                resources: vec![input_json],
                content_type: "application/eigon+json".to_string(),
            }
        } else {
            let response = client
                .fiber_query(eigenius_kernel::server::proto::FiberQueryRequest {
                    institution_iri: iri.to_string(),
                    query: input_json,
                    content_type: "application/eigon+json".to_string(),
                })
                .await;
            match response {
                Ok(r) => {
                    let resp = r.into_inner();
                    if resp.success {
                        print_test_result(&resp.result, json);
                    } else {
                        eprintln!("Fiber query failed: {}", resp.error);
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("gRPC error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        };
        match client.discover_morphisms(req).await {
            Ok(r) => {
                let resp = r.into_inner();
                if resp.success {
                    if json {
                        let parsed: Vec<serde_json::Value> = resp
                            .morphisms
                            .iter()
                            .filter_map(|m| {
                                eigenius_kernel::ontology::eigon_cbor::parse_resource_lenient(m)
                                    .ok()
                                    .map(|r| {
                                        eigenius_kernel::ontology::eigon_json::serialize_resource(
                                            &r,
                                        )
                                    })
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&parsed).unwrap());
                    } else {
                        println!("Discovered {} morphism(s)", resp.morphisms.len());
                    }
                } else {
                    eprintln!("Discover morphisms failed: {}", resp.error);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("gRPC error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        // Component: wrap in a trivial program that applies the component to input
        let program_json = format!(
            r#"{{
                "@id": "urn:eigenius:cli:capability_test_program",
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
                "urn:eigenius:program:input_type": "urn:eigenius:core:Class",
                "urn:eigenius:program:output_type": "urn:eigenius:core:Class",
                "urn:eigenius:program:body": {{
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
                    "urn:eigenius:program:function": "{iri}",
                    "urn:eigenius:program:argument": {{
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                        "urn:eigenius:program:name": "input"
                    }}
                }}
            }}"#
        );

        match client
            .run_program(eigenius_kernel::server::proto::RunProgramRequest {
                program: program_json.into_bytes(),
                input: input_json,
                content_type: "application/eigon+json".to_string(),
            })
            .await
        {
            Ok(response) => {
                let resp = response.into_inner();
                if resp.success {
                    print_test_result(&resp.output, json);
                } else {
                    eprintln!("Component execution failed:");
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
}

fn print_test_result(cbor_bytes: &[u8], json: bool) {
    if json {
        if let Ok(resource) =
            eigenius_kernel::ontology::eigon_cbor::parse_resource_lenient(cbor_bytes)
        {
            let v = eigenius_kernel::ontology::eigon_json::serialize_resource(&resource);
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        } else {
            eprintln!("Failed to parse result CBOR");
            std::process::exit(1);
        }
    } else if let Ok(resource) =
        eigenius_kernel::ontology::eigon_cbor::parse_resource_lenient(cbor_bytes)
    {
        let v = eigenius_kernel::ontology::eigon_json::serialize_resource(&resource);
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        eprintln!("Failed to parse result");
        std::process::exit(1);
    }
}
