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

//! `umls-import` — render the UMLS Metathesaurus (RRF) into the Eigenius mirror +
//! derived lexicon ESL (D65 §5); deterministic, no LLM.
//!
//! ```text
//!   # WRN-relevant subset → ESL, self-validated
//!   umls-import --meta-dir references/umls/2026AA/META \
//!               --semantic-type T047 --semantic-type T049 --semantic-type T028 \
//!               --out umls.esl --validate
//! ```
//!
//! The big files (MRCONSO/MRSTY/MRDEF) are streamed line-by-line through the
//! [`ConceptBuilder`] (bounded memory for the input scan); the small files
//! (MRSAB/MRRANK) are read whole. `--validate` compiles the output against the
//! bootstrap chain, runs structural validation, and felicity-gates every emitted
//! `lexicon:LexicalEntry` — fail-closed.
//!
//! **SRL-0 only.** Atoms/definitions are kept solely from sources whose `MRSAB.SRL`
//! is 0 (Level 0); restricted sources (e.g. SNOMED CT) are dropped even when present.
//! The output carries the UMLS license notice and the redistribution constraint.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use eigenius_kernel::dcg::gate_entry;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::validation::Validator;
use eigenius_kernel::{bootstrap, esl};
use eigenius_umls::convert::{header, render_base, render_concept_block, render_document};
use eigenius_umls::rrf::{
    parse_mrconso_line, parse_mrdef_line, parse_mrrank, parse_mrsab, parse_mrsty_line,
    srl0_allowlist, ConceptBuilder, Subset,
};

#[derive(Parser, Debug)]
#[command(
    about = "Import the UMLS Metathesaurus (RRF) into Eigenius mirror + lexicon ESL (D65 §5)"
)]
struct Args {
    /// Directory containing the RRF files (MRCONSO.RRF, MRSTY.RRF, MRSAB.RRF,
    /// MRRANK.RRF, MRDEF.RRF) — typically `<release>/META`.
    #[arg(long)]
    meta_dir: PathBuf,
    /// Keep only concepts with at least one of these semantic types (TUI, repeatable);
    /// none ⇒ all semantic types (a full import).
    #[arg(long = "semantic-type")]
    semantic_type: Vec<String>,
    /// MRCONSO language to keep (LAT).
    #[arg(long, default_value = "ENG")]
    language: String,
    /// The Metathesaurus release label, for the license notice + descriptor.
    #[arg(long, default_value = "2026AA")]
    version: String,
    /// Cap to the first N concepts (sorted by CUI) — a bounded import.
    #[arg(long)]
    limit: Option<usize>,
    /// Write the ESL as a SINGLE file here.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Write the ESL as a PARTITIONED chain into this directory: `umls-000-base.esl`
    /// (semantic-type classes + the lexicon descriptor) then `umls-NNN.esl` concept
    /// batches, each under `--split-bytes`. Load them in filename order as a layer
    /// chain (each concept chunk resolves against the base). Use this for large imports
    /// (full Level-0) that exceed the gRPC message-size limit as one document.
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Max bytes per partition file (default 100 MiB — safely under the kernel's
    /// 128 MiB gRPC Load limit). Only used with `--out-dir`.
    #[arg(long, default_value_t = 100 * 1024 * 1024)]
    split_bytes: usize,
    /// Compile + validate + felicity-gate the output (self-check; fail-closed). Single
    /// `--out`/in-memory mode only — in `--out-dir` mode the kernel validates each
    /// layer at load time (the chain is the validation context).
    #[arg(long)]
    validate: bool,
}

/// Stream a `|`-delimited RRF file line-by-line, calling `f` on each non-empty line.
fn stream_lines(path: &Path, mut f: impl FnMut(&str)) -> std::io::Result<()> {
    let reader = BufReader::new(File::open(path)?);
    for line in reader.lines() {
        let line = line?;
        if !line.is_empty() {
            f(&line);
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = Args::parse();
    let meta = &args.meta_dir;

    // Small files read whole: SRL-0 allowlist + the name-ranking precedence.
    let mrsab = match fs::read_to_string(meta.join("MRSAB.RRF")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: reading MRSAB.RRF: {e}");
            return ExitCode::from(2);
        }
    };
    let mrrank = match fs::read_to_string(meta.join("MRRANK.RRF")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: reading MRRANK.RRF: {e}");
            return ExitCode::from(2);
        }
    };
    let srl0 = srl0_allowlist(&parse_mrsab(&mrsab));
    eprintln!("SRL-0 sources (allowed): {}", srl0.len());

    let allow_tui: Option<BTreeSet<String>> = if args.semantic_type.is_empty() {
        None
    } else {
        Some(args.semantic_type.iter().cloned().collect())
    };

    let mut builder = ConceptBuilder::new(srl0, parse_mrrank(&mrrank), allow_tui, &args.language);

    // Big files streamed. Order matters: STY first (decides selection + full TUI sets),
    // then atoms and definitions (gated on the selected set).
    eprintln!(">> scanning MRSTY (concept selection)…");
    if let Err(e) = stream_lines(&meta.join("MRSTY.RRF"), |l| {
        if let Some(s) = parse_mrsty_line(l) {
            builder.add_sty(&s);
        }
    }) {
        eprintln!("error: reading MRSTY.RRF: {e}");
        return ExitCode::from(2);
    }
    eprintln!(">> scanning MRCONSO (atoms)…");
    if let Err(e) = stream_lines(&meta.join("MRCONSO.RRF"), |l| {
        if let Some(a) = parse_mrconso_line(l) {
            builder.add_atom(&a);
        }
    }) {
        eprintln!("error: reading MRCONSO.RRF: {e}");
        return ExitCode::from(2);
    }
    eprintln!(">> scanning MRDEF (definitions)…");
    if let Err(e) = stream_lines(&meta.join("MRDEF.RRF"), |l| {
        if let Some(d) = parse_mrdef_line(l) {
            builder.add_def(&d);
        }
    }) {
        eprintln!("error: reading MRDEF.RRF: {e}");
        return ExitCode::from(2);
    }

    let subset = builder.finish(args.limit);
    if subset.concepts.is_empty() {
        eprintln!(
            "error: no concepts selected (semantic types: {:?})",
            args.semantic_type
        );
        return ExitCode::from(2);
    }

    // Partitioned emit: a base layer (semantic types + descriptor) + concept-batch
    // chunks, each under the size cap. The single-document path stays for small imports.
    if let Some(dir) = &args.out_dir {
        return emit_partitioned(&subset, &args.version, dir, args.split_bytes);
    }

    let (doc, rep) = render_document(&subset, &args.version);
    eprintln!(
        "umls import ({}): {} semantic-type classes, {} concept classes → {} lexical entries",
        args.version, rep.semantic_types, rep.concepts, rep.entries,
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

/// Emit the subset as a layer chain into `dir`: `umls-000-base.esl` (semantic-type
/// classes + the `lexicon:umls` descriptor) then `umls-NNN.esl` concept-batch chunks,
/// each ≤ `split_bytes`. Every file carries the full header (license notice +
/// namespaces). Load them in filename order; each concept chunk resolves its
/// `subclass_of umlssty:*` against the base layer below it.
fn emit_partitioned(subset: &Subset, version: &str, dir: &Path, split_bytes: usize) -> ExitCode {
    if let Err(e) = fs::create_dir_all(dir) {
        eprintln!("error: creating {}: {e}", dir.display());
        return ExitCode::from(1);
    }

    let hdr = header(version);
    let (base, sty) = render_base(subset, version);

    let mut files: Vec<PathBuf> = Vec::new();
    let base_path = dir.join("umls-000-base.esl");
    if let Err(e) = fs::write(&base_path, &base) {
        eprintln!("error: writing {}: {e}", base_path.display());
        return ExitCode::from(1);
    }
    files.push(base_path);

    let mut idx = 1usize;
    let mut cur = hdr.clone();
    let mut total_entries = 0usize;
    let mut chunk_concepts = 0usize;

    let flush = |idx: usize, cur: &str| -> std::io::Result<PathBuf> {
        let path = dir.join(format!("umls-{idx:03}.esl"));
        fs::write(&path, cur)?;
        Ok(path)
    };

    for c in &subset.concepts {
        let (block, entries) = render_concept_block(c);
        // Roll over before exceeding the cap (but never write an empty chunk).
        if chunk_concepts > 0 && cur.len() + block.len() > split_bytes {
            match flush(idx, &cur) {
                Ok(p) => files.push(p),
                Err(e) => {
                    eprintln!("error: writing chunk {idx}: {e}");
                    return ExitCode::from(1);
                }
            }
            idx += 1;
            cur = hdr.clone();
            chunk_concepts = 0;
        }
        cur.push_str(&block);
        total_entries += entries;
        chunk_concepts += 1;
    }
    if chunk_concepts > 0 {
        match flush(idx, &cur) {
            Ok(p) => files.push(p),
            Err(e) => {
                eprintln!("error: writing final chunk: {e}");
                return ExitCode::from(1);
            }
        }
    }

    eprintln!(
        "umls import ({version}): {sty} semantic-type classes, {} concept classes → {total_entries} lexical entries",
        subset.concepts.len(),
    );
    eprintln!(
        "wrote {} files → {} (base + {} concept chunks; load in filename order as a chain)",
        files.len(),
        dir.display(),
        files.len() - 1,
    );
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
        "umls",
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
