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
    Run {
        /// Path to program Eigon-JSON file
        #[arg(value_name = "PROGRAM_FILE")]
        program_file: String,

        /// Path to input Eigon-JSON file
        #[arg(value_name = "INPUT_FILE")]
        input_file: String,

        /// Optional ontology file to load first
        #[arg(long, value_name = "FILE")]
        ontology: Option<String>,
    },

    /// Print a resource by IRI
    Inspect {
        /// IRI of the resource to inspect
        #[arg(value_name = "IRI")]
        iri: String,
    },

    /// Show version and build info
    Version,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Load { file } => cmd_load(&file, cli.json),
        Commands::Validate { file } => cmd_validate(&file, cli.json),
        Commands::Query { query, file } => cmd_query(&query, file.as_deref(), cli.json),
        Commands::ProgramValidate {
            program_file,
            ontology,
        } => cmd_program_validate(&program_file, ontology.as_deref(), cli.json),
        Commands::Run {
            program_file,
            input_file,
            ontology,
        } => cmd_run(&program_file, &input_file, ontology.as_deref(), cli.json),
        Commands::Inspect { iri } => cmd_inspect(&iri, cli.json),
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

    // Read and parse file
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read '{file}': {e}");
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

fn cmd_run(program_file: &str, input_file: &str, ontology: Option<&str>, json_output: bool) {
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

    // Read program
    let program_content = match std::fs::read_to_string(program_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read program '{program_file}': {e}");
            std::process::exit(1);
        }
    };
    let program = match eigon_json::parse_document(&program_content) {
        Ok(mut r) => r.remove(0),
        Err(e) => {
            eprintln!("program parse error: {e}");
            std::process::exit(1);
        }
    };

    // Read input
    let input_content = match std::fs::read_to_string(input_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read input '{input_file}': {e}");
            std::process::exit(1);
        }
    };
    let input = match eigon_json::parse_document(&input_content) {
        Ok(mut r) => r.remove(0),
        Err(e) => {
            eprintln!("Input parse error: {e}");
            std::process::exit(1);
        }
    };

    // Execute
    let registry = eigenius_kernel::program::execute::ComponentRegistry::default();
    match eigenius_kernel::program::execute::execute_program(
        &program,
        &input,
        ctx.head(),
        &registry,
    ) {
        Ok(output) => {
            let json = eigon_json::serialize_resource(&output);
            if json_output {
                println!("{}", serde_json::to_string(&json).unwrap());
            } else {
                println!("program execution succeeded.");
                println!("{}", serde_json::to_string_pretty(&json).unwrap());
            }
        }
        Err(e) => {
            eprintln!("program execution failed: {e}");
            std::process::exit(1);
        }
    }
}

fn load_file_into_context(ctx: &mut eigenius_kernel::context::ExecutionContext, file: &str) {
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read '{file}': {e}");
            std::process::exit(1);
        }
    };
    let resources = match eigon_json::parse_document(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Parse error in '{file}': {e}");
            std::process::exit(1);
        }
    };
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

    // Read and parse file
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read '{file}': {e}");
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
