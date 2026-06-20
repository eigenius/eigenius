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

//! schema.org JSON-LD → Eigon-JSON converter (D57 m3).
//!
//! Input: `schemaorg-current-https.jsonld` (pin V30.0; content-hash for
//! reproducibility). Output: the `urn:schema_org:` ontology as Eigon-JSON, plus
//! an optional coverage report (the D57 m4 cut accounting).
//!
//!     schemaorg-import --input schemaorg-current-https.jsonld \
//!         --output schema-org.eigon.json --report coverage.json --pretty

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use eigenius_kernel::ontology::eigon_json;
use eigenius_schemaorg::report::{self, RESULT_IRI};
use eigenius_schemaorg::{convert, parse_graph};
use sha2::{Digest, Sha256};

#[derive(Parser, Debug)]
#[command(about = "Import schema.org JSON-LD into Eigon-JSON under urn:schema_org:")]
struct Args {
    /// Input schema.org JSON-LD (e.g. schemaorg-current-https.jsonld).
    #[arg(long)]
    input: PathBuf,
    /// Output Eigon-JSON file. Omit to skip writing the ontology.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Coverage / cut-accounting report (JSON). Omit to skip.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Conversion-report `DerivedResource` as Eigon-CBOR (D60 §4.1 — the `oci`
    /// tool-runtime result wire format). Omit to skip. `-` writes to stdout.
    #[arg(long)]
    report_cbor: Option<PathBuf>,
    /// Pretty-print the Eigon-JSON output.
    #[arg(long)]
    pretty: bool,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn main() -> ExitCode {
    let args = Args::parse();

    let input_bytes = match fs::read(&args.input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {e}", args.input.display());
            return ExitCode::from(2);
        }
    };
    let input_sha256 = sha256_hex(&input_bytes);
    let input = match String::from_utf8(input_bytes) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: input is not valid UTF-8: {e}");
            return ExitCode::from(2);
        }
    };
    let nodes = match parse_graph(&input) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let report = convert(&nodes);
    let c = &report.coverage;
    eprintln!(
        "schema.org import: {} resources \
         ({} classes, {} enum classes, {} enum members, {} properties)",
        report.resources.len(),
        c.classes,
        c.enumeration_classes,
        c.enumeration_members,
        c.properties,
    );
    eprintln!("  property tiers: {:?}", c.property_tiers);
    eprintln!(
        "  datatypes folded → scalars: {}; excluded (pending/meta): {}",
        c.datatypes_folded.len(),
        c.excluded_layer
    );
    eprintln!("  Tier-3 residual (not mapped): {:?}", c.residual_relations);

    if let Some(path) = &args.output {
        let doc = eigon_json::serialize_document(&report.resources);
        let s = if args.pretty {
            serde_json::to_string_pretty(&doc)
        } else {
            serde_json::to_string(&doc)
        };
        match s.and_then(|s| fs::write(path, s).map_err(serde_err)) {
            Ok(()) => eprintln!("wrote ontology → {}", path.display()),
            Err(e) => {
                eprintln!("error: writing output: {e}");
                return ExitCode::from(1);
            }
        }
    }
    if let Some(path) = &args.report {
        match serde_json::to_string_pretty(&report.coverage)
            .and_then(|s| fs::write(path, s).map_err(serde_err))
        {
            Ok(()) => eprintln!("wrote coverage → {}", path.display()),
            Err(e) => {
                eprintln!("error: writing report: {e}");
                return ExitCode::from(1);
            }
        }
    }
    if let Some(path) = &args.report_cbor {
        // Hash the canonical (compact) ontology serialization — the artifact the
        // chain pins as `gen_output` — to record the input→output provenance.
        let doc = eigon_json::serialize_document(&report.resources);
        let output_sha256 = match serde_json::to_string(&doc) {
            Ok(s) => sha256_hex(s.as_bytes()),
            Err(e) => {
                eprintln!("error: serializing ontology for hash: {e}");
                return ExitCode::from(1);
            }
        };
        let result =
            report::build_report(RESULT_IRI, &input_sha256, &output_sha256, &report.coverage);
        let cbor = report::report_to_cbor(&result);
        let write = if path.as_os_str() == "-" {
            use std::io::Write;
            std::io::stdout().write_all(&cbor).map_err(serde_err)
        } else {
            fs::write(path, &cbor).map_err(serde_err)
        };
        match write {
            Ok(()) => eprintln!(
                "wrote conversion-report (Eigon-CBOR, {} bytes) → {}",
                cbor.len(),
                path.display()
            ),
            Err(e) => {
                eprintln!("error: writing report-cbor: {e}");
                return ExitCode::from(1);
            }
        }
    }

    ExitCode::SUCCESS
}

/// Adapt `io::Error` into `serde_json::Error`'s slot so the `and_then` chains
/// above stay single-typed.
fn serde_err(e: std::io::Error) -> serde_json::Error {
    serde::de::Error::custom(e.to_string())
}
