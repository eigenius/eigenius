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

//! `ncbi-gene-import` — render NCBI Gene `gene_info` into the Eigenius mirror +
//! derived lexicon ESL (D65 §5); deterministic, no LLM.
//!
//! ```text
//!   # human genes → ESL, self-validated
//!   ncbi-gene-import --gene-info references/ncbi/Homo_sapiens.gene_info \
//!                    --out ncbi-gene.esl --validate
//! ```
//!
//! `--validate` compiles the output against the bootstrap chain, runs structural
//! validation, and felicity-gates every emitted `lexicon:LexicalEntry` — fail-closed.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use eigenius_kernel::dcg::gate_entry;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::validation::Validator;
use eigenius_kernel::{bootstrap, esl};
use eigenius_ncbi_gene::convert::render_document;
use eigenius_ncbi_gene::gene_info::parse_document;

#[derive(Parser, Debug)]
#[command(about = "Import NCBI Gene (gene_info) into Eigenius mirror + lexicon ESL (D65 §5)")]
struct Args {
    /// Path to a `gene_info` file (e.g. Homo_sapiens.gene_info).
    #[arg(long)]
    gene_info: PathBuf,
    /// Keep only rows for this NCBI Taxonomy id (default: human).
    #[arg(long, default_value = "9606")]
    tax_id: String,
    /// Cap to the first N genes (after the taxon filter) — a bounded import.
    #[arg(long)]
    limit: Option<usize>,
    /// Write the ESL here.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Compile + validate + felicity-gate the output (self-check; fail-closed).
    #[arg(long)]
    validate: bool,
    /// Add the ncbi:Gene ⊑ wn:gene.n.01 grounding edge (only valid when committing on a chain with the WordNet layer).
    #[arg(long)]
    wordnet_anchor: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let text = match fs::read_to_string(&args.gene_info) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: reading {}: {e}", args.gene_info.display());
            return ExitCode::from(2);
        }
    };
    let mut genes = parse_document(&text, &args.tax_id);
    if let Some(n) = args.limit {
        genes.truncate(n);
    }
    if genes.is_empty() {
        eprintln!(
            "error: no genes parsed for tax_id {} from {}",
            args.tax_id,
            args.gene_info.display()
        );
        return ExitCode::from(2);
    }

    let (doc, rep) = render_document(&genes, &args.tax_id, args.wordnet_anchor);
    eprintln!(
        "ncbi-gene import (tax {}): {} gene witnesses → {} lexical entries (symbol + synonyms)",
        args.tax_id, rep.genes, rep.entries,
    );

    if let Some(path) = &args.out {
        if let Err(e) = fs::write(path, &doc) {
            eprintln!("error: writing {}: {e}", path.display());
            return ExitCode::from(1);
        }
        eprintln!("wrote ESL → {}", path.display());
    }

    if args.validate {
        match validate(&doc) {
            Ok((admitted, rejected)) if rejected.is_empty() => {
                eprintln!("validate: OK — {admitted} lexical entries felicity-gated clean");
            }
            Ok((admitted, rejected)) => {
                eprintln!(
                    "validate: FAILED — {admitted} admitted, {} rejected, e.g.: {}",
                    rejected.len(),
                    rejected.first().map(String::as_str).unwrap_or(""),
                );
                return ExitCode::from(1);
            }
            Err(e) => {
                eprintln!("validate: FAILED — {e}");
                return ExitCode::from(1);
            }
        }
    }

    ExitCode::SUCCESS
}

fn build_layer(
    name: &str,
    parent: Arc<Layer>,
    resources: Vec<Resource>,
    storage: LayerStorage,
) -> Result<Arc<Layer>, String> {
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in resources {
        b.add_resource(r)
            .map_err(|e| format!("add_resource: {e:?}"))?;
    }
    Ok(Arc::new(b.build(storage)))
}

fn validate(doc: &str) -> Result<(usize, Vec<String>), String> {
    let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap: {e}"))?;
    let layer = build_layer(
        "ncbi-gene",
        Arc::clone(ctx.head()),
        esl::compile_against_layer(doc, ctx.head()).map_err(|e| format!("compile: {e:?}"))?,
        LayerStorage::in_memory(),
    )?;

    let errors = Validator::new(Arc::clone(&layer)).validate();
    if !errors.is_empty() {
        return Err(format!(
            "{} structural error(s), e.g.: {}",
            errors.len(),
            errors[0]
        ));
    }

    let entry_class = Iri::parse("urn:eigenius:lexicon:LexicalEntry").unwrap();
    let mut admitted = 0usize;
    let mut rejected = Vec::new();
    for (id, r) in layer.iter_resources() {
        if !r.is_instance_of(&entry_class) {
            continue;
        }
        match gate_entry(&layer, &r) {
            Ok(_) => admitted += 1,
            Err(reason) => rejected.push(format!("{id}: {reason}")),
        }
    }
    Ok((admitted, rejected))
}
