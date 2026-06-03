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

//! One-shot OBO-JSON → Eigon-JSON converter.
//!
//! Usage:
//!
//! ```text
//! obograph-import \
//!     --input go.json \
//!     --output go.eigon.json \
//!     [--pretty]
//! ```
//!
//! Reports the per-type node count and any soft errors to stderr;
//! the converted Eigon-JSON document goes to the output path or
//! stdout if no `--output` is given. Pretty-printing is off by
//! default — the on-disk JSON is one big object/array, which is
//! the right default for re-ingest into the kernel.

use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use eigenius_kernel::ontology::eigon_json;
use eigenius_obograph::{convert_document_with, ConvertOptions, GraphDocument};

#[derive(Parser, Debug)]
#[command(
    name = "obograph-import",
    about = "Convert an OBO Graphs JSON ontology dump into Eigon-JSON Resources."
)]
struct Args {
    /// Input OBO-JSON file. `-` reads from stdin.
    #[arg(short, long)]
    input: String,

    /// Output Eigon-JSON file. Omit to write to stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Pretty-print the output. Off by default — production ingest
    /// reads the dense form.
    #[arg(long)]
    pretty: bool,

    /// Override the `declared_by` attribution on every imported
    /// Resource. When omitted, defaults per-graph to the graph's own
    /// IRI (`graph.id`). Useful when the curating authority isn't
    /// the graph IRI — e.g. ingesting a community-curated subset.
    #[arg(long)]
    declared_by: Option<String>,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let input_str = match read_input(&args.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read input `{}`: {e}", args.input);
            return ExitCode::from(2);
        }
    };
    let doc: GraphDocument = match serde_json::from_str(&input_str) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: input is not valid OBO-JSON: {e}");
            return ExitCode::from(2);
        }
    };

    let opts = ConvertOptions {
        declared_by: args.declared_by.clone(),
    };
    let report = convert_document_with(&doc, &opts);

    eprintln!("converted {} Resources", report.resources.len());
    for (k, v) in &report.counts_by_type {
        eprintln!("  {k}: {v}");
    }
    if !report.errors.is_empty() {
        eprintln!("{} soft errors:", report.errors.len());
        for err in &report.errors {
            eprintln!("  {err}");
        }
    }

    let document = eigon_json::serialize_document(&report.resources);
    let output_str = if args.pretty {
        serde_json::to_string_pretty(&document)
    } else {
        serde_json::to_string(&document)
    };
    let output_str = match output_str {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot serialize output: {e}");
            return ExitCode::from(1);
        }
    };

    match args.output {
        Some(path) => {
            if let Err(e) = fs::write(&path, &output_str) {
                eprintln!("error: cannot write `{}`: {e}", path.display());
                return ExitCode::from(1);
            }
        }
        None => {
            if let Err(e) = io::stdout().write_all(output_str.as_bytes()) {
                eprintln!("error: stdout write failed: {e}");
                return ExitCode::from(1);
            }
        }
    }

    ExitCode::SUCCESS
}

fn read_input(spec: &str) -> io::Result<String> {
    if spec == "-" {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        fs::read_to_string(spec)
    }
}
