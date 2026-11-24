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

//! Compile and validate an ordered stack of ESL layers on top of the immutable bootstrap chain.
//!
//! Each `.esl` path passed on the command line is compiled against the cumulative parent chain
//! and validated with `eigenius_kernel::validation::Validator::validate()`. Exits non-zero if
//! any layer fails to compile or reports any validation error.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::validation::Validator;

const COMPRESSION_RECORD_CLASS: &str = "urn:eigenius:align:CompressionRecord";
const REQUIRED_QUALIFIERS_PROP: &str = "urn:eigenius:align:required_qualifiers";
const DISCHARGED_QUALIFIERS_PROP: &str = "urn:eigenius:align:discharged_qualifiers";

#[derive(Parser)]
#[command(
    about = "Compile and validate an ordered stack of ESL layers over bootstrap or a lexicon snapshot"
)]
struct Args {
    /// Optional RocksDB lexicon snapshot to stack on top of (instead of bare bootstrap).
    #[arg(long)]
    snapshot: Option<PathBuf>,
    /// Enforce the D103 §7.2 non-droppable-qualifier structural gate on align:CompressionRecord resources.
    #[arg(long)]
    check_compression_gate: bool,
    /// Ordered ESL layer files to compile and validate.
    #[arg(required = true)]
    layers: Vec<PathBuf>,
    /// Maximum validation errors to display per layer before truncating.
    #[arg(long, default_value_t = 25)]
    max_errors: usize,
}

fn check_compression_gate(layer: &Layer) -> Vec<String> {
    let cls_iri = Iri::parse(COMPRESSION_RECORD_CLASS).expect("valid class IRI");
    let req_iri = Iri::parse(REQUIRED_QUALIFIERS_PROP).expect("valid required_qualifiers IRI");
    let dis_iri = Iri::parse(DISCHARGED_QUALIFIERS_PROP).expect("valid discharged_qualifiers IRI");
    let mut violations = Vec::new();
    for (id, res) in layer.iter_resources() {
        if !res.is_instance_of(&cls_iri) {
            continue;
        }
        let required: Vec<Iri> = res
            .properties()
            .get(&req_iri)
            .map(|v| v.as_iri_array())
            .unwrap_or_default();
        let discharged: BTreeSet<Iri> = res
            .properties()
            .get(&dis_iri)
            .map(|v| v.as_iri_array().into_iter().collect())
            .unwrap_or_default();
        let missing: Vec<String> = required
            .into_iter()
            .filter(|q| !discharged.contains(q))
            .map(|q| q.as_str().to_string())
            .collect();
        if !missing.is_empty() {
            violations.push(format!(
                "{}: dropped required qualifier(s): [{}]",
                id.as_str(),
                missing.join(", ")
            ));
        }
    }
    violations
}

fn main() -> ExitCode {
    let args = Args::parse();
    let mut head: Arc<Layer> = match &args.snapshot {
        Some(snap) => match eigenius_encoding::snapshot::open_head(snap) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("snapshot open failed: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => match eigenius_kernel::bootstrap::bootstrap() {
            Ok(b) => Arc::clone(b.head()),
            Err(e) => {
                eprintln!("bootstrap failed: {e:?}");
                return ExitCode::FAILURE;
            }
        },
    };
    let mut total_resources: usize = 0;

    for path in &args.layers {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("read {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_string();
        let compiled = match esl::compile(&src, &head) {
            Ok(rs) => rs,
            Err(errs) => {
                eprintln!(
                    "COMPILE ERROR in {} ({} error(s)):",
                    path.display(),
                    errs.len()
                );
                for err in errs.iter().take(args.max_errors) {
                    eprintln!("  - {err}");
                }
                return ExitCode::FAILURE;
            }
        };
        let count = compiled.len();
        let mut builder = LayerBuilder::new(&name, Some(Arc::clone(&head)));
        for r in compiled {
            if let Err(e) = builder.add_resource(r) {
                eprintln!("LayerBuilder error in {}: {e:?}", path.display());
                return ExitCode::FAILURE;
            }
        }
        let layer = Arc::new(builder.build(LayerStorage::in_memory()));
        let errors = Validator::new(Arc::clone(&layer)).validate();
        if !errors.is_empty() {
            eprintln!(
                "VALIDATION ERROR in {} ({} error(s) across {count} resource(s)):",
                path.display(),
                errors.len()
            );
            for err in errors.iter().take(args.max_errors) {
                eprintln!("  - {err:?}");
            }
            return ExitCode::FAILURE;
        }
        if args.check_compression_gate {
            let gate_errors = check_compression_gate(&layer);
            if !gate_errors.is_empty() {
                eprintln!(
                    "COMPRESSION GATE ERROR in {} ({} violation(s)):",
                    path.display(),
                    gate_errors.len()
                );
                for err in gate_errors.iter().take(args.max_errors) {
                    eprintln!("  - {err}");
                }
                return ExitCode::FAILURE;
            }
        }
        println!("OK  {:5} resource(s)  {}", count, path.display());
        total_resources += count;
        head = layer;
    }

    println!(
        "\nALL {} layer(s) valid: {} total resource(s), 0 errors.",
        args.layers.len(),
        total_resources
    );
    ExitCode::SUCCESS
}
