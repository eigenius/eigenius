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

//! `wordnet-import` — WordNet `data.<pos>` → Eigon lexicon ESL (D62 §8.7 / D63 §8.7 Slice 7).
//!
//!     # render + self-validate
//!     wordnet-import --all --out wn.esl --validate
//!
//! The importer's job is to **emit the lexicon** (ESL). PERSISTENCE is the platform's
//! generic layer-load path, not WordNet-specific: stand up `eigenius serve --db <path>`
//! and `eigenius --endpoint <addr> load wn.esl` (the server commits + persists the layer,
//! advancing the branch — the same as loading any layer).
//!
//! Deterministic, no LLM. Noun selection is always **closed under hypernymy** (and
//! `entity.n.01` added) so the emitted `subclass_of` lattice is rooted; verbs/adjectives
//! type at the noun root and compose by subsumption. `--validate` compiles +
//! `Validator`-checks + felicity-gates the output via kernel library calls (no subprocess),
//! fail-closed.

use std::fs;
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
use eigenius_wordnet::convert::{render_document, render_sections, MassNouns, ESL_HEADER};
use eigenius_wordnet::import::{read_sense_ranks, select_synsets, SeedSpec};
use eigenius_wordnet::wndb::Pos;

#[derive(Parser, Debug)]
#[command(about = "Import WordNet into Eigon lexicon ESL (D62 §8.7); deterministic, no LLM")]
struct Args {
    /// WordNet dict directory (contains data.noun / data.verb / data.adj).
    #[arg(long, default_value = "references/WordNet-3.0/dict")]
    dict: PathBuf,
    /// Seed lemma(s): import their synsets + the noun hypernym closure. Repeatable.
    #[arg(long)]
    seed: Vec<String>,
    /// Import ALL synsets of the requested POS (heavy — the full lexicon).
    #[arg(long)]
    all: bool,
    /// Cap the per-POS seed set to the first N synsets (then closed). Bounded import.
    #[arg(long)]
    limit: Option<usize>,
    /// POS to import.
    #[arg(long, value_delimiter = ',', default_value = "noun,verb,adj")]
    pos: Vec<String>,
    /// Write the ESL as a SINGLE file here.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Write the ESL as a PARTITIONED chain into this directory: `wordnet-000-base.esl`
    /// (the `lexicon:wordnet` descriptor + every synset class/axiom) then
    /// `wordnet-NNN.esl` LexicalEntry batches, each under `--split-bytes`. Load them in
    /// filename order as a layer chain (each entry chunk resolves against the base).
    /// Use this for the full lexicon (`--all`): the single document is ~165 MB, over the
    /// kernel's 128 MiB gRPC Load limit, so it cannot be loaded whole.
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Max bytes per partition file (default 100 MiB — safely under the kernel's
    /// 128 MiB gRPC Load limit). Only used with `--out-dir`.
    #[arg(long, default_value_t = 100 * 1024 * 1024)]
    split_bytes: usize,
    /// Compile + validate + felicity-gate the output (self-check; fail-closed). Single
    /// `--out`/in-memory mode only — in `--out-dir` mode the kernel validates each layer
    /// at load time (the chain is the validation context).
    #[arg(long)]
    validate: bool,
    /// **Countability lexicon** (D62 bare-mass arguments): a newline-delimited list of noun
    /// lemmas with an uncountable sense (one per line; `#` comments + blank lines ignored).
    /// Each such lemma gets an additive `cat_n(C, mass)` entry so a bare singular shifts to an
    /// NP argument. Built by `scripts/provision-countability.sh` (Wiktionary ∩ WordNet). Absent
    /// ⇒ no mass marking (count-only, the prior behaviour).
    #[arg(long, default_value = "references/wiktionary/uncountable-nouns.txt")]
    countability: PathBuf,
}

/// Load the countability lexicon (one lemma per line; `#`/blank ignored), lowercased. A missing
/// file is non-fatal — returns an empty set (no mass marking), like an absent sense-rank index.
fn load_countability(path: &Path) -> MassNouns {
    match fs::read_to_string(path) {
        Ok(s) => {
            let set: MassNouns = s
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.to_lowercase())
                .collect();
            eprintln!(
                "countability: {} uncountable lemmas from {}",
                set.len(),
                path.display()
            );
            set
        }
        Err(_) => {
            eprintln!(
                "countability: {} not found — no mass marking (count-only nouns)",
                path.display()
            );
            MassNouns::new()
        }
    }
}

fn pos_of(s: &str) -> Option<Pos> {
    match s {
        "noun" | "n" => Some(Pos::Noun),
        "verb" | "v" => Some(Pos::Verb),
        "adj" | "a" => Some(Pos::Adj),
        "adv" | "r" => Some(Pos::Adv),
        _ => None,
    }
}

fn main() -> ExitCode {
    let args = Args::parse();

    if !args.all && args.limit.is_none() && args.seed.is_empty() {
        eprintln!("error: select a bound — one of --seed <lemma>, --limit <N>, or --all");
        return ExitCode::from(2);
    }
    let pos: Vec<Pos> = match args.pos.iter().map(|p| pos_of(p)).collect::<Option<_>>() {
        Some(v) => v,
        None => {
            eprintln!("error: --pos must be noun/verb/adj/adv");
            return ExitCode::from(2);
        }
    };

    let spec = SeedSpec {
        all: args.all,
        limit: args.limit,
        seeds: args.seed.clone(),
        pos,
    };
    let chosen = match select_synsets(&args.dict, &spec) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: reading dict {}: {e}", args.dict.display());
            return ExitCode::from(2);
        }
    };
    // Sense-frequency ranks → `lexicon:sense_rank` (D63 §8.7 Stage B); a missing index
    // is non-fatal (ranks default 0).
    let ranks = read_sense_ranks(&args.dict, &spec.pos).unwrap_or_default();
    let mass = load_countability(&args.countability);

    // Partitioned emit: a base layer (descriptor + all synset classes/axioms) + entry
    // batches, each under the size cap. The single-document path stays for small imports.
    if let Some(dir) = &args.out_dir {
        return emit_partitioned(&chosen, &ranks, &mass, dir, args.split_bytes);
    }

    let (doc, rep) = render_document(&chosen, &ranks, &mass);
    eprintln!(
        "wordnet import: {} synsets selected → {} noun classes, {} instances, {} verb axioms, \
         {} adj axioms, {} entries ({} of them ger/pss participle forms) \
         ({} verb synsets deferred: only predicative/clausal/control frames)",
        chosen.len(),
        rep.noun_classes,
        rep.instances,
        rep.verb_axioms,
        rep.adj_axioms,
        rep.entries,
        rep.participle_entries,
        rep.verbs_deferred,
    );
    eprintln!(
        "  ({} additive mass-noun entries from the countability lexicon)",
        rep.mass_entries
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
                eprintln!("validate: {admitted}/{admitted} entries admitted (felicity-gated)");
            }
            Ok((admitted, rejected)) => {
                eprintln!(
                    "validate: {admitted} admitted, {} REJECTED:",
                    rejected.len()
                );
                for r in rejected.iter().take(20) {
                    eprintln!("  REJECT {r}");
                }
                return ExitCode::from(1);
            }
            Err(e) => {
                eprintln!("validate: error: {e}");
                return ExitCode::from(1);
            }
        }
    }

    ExitCode::SUCCESS
}

/// Emit the import as a layer chain into `dir`: `wordnet-000-base.esl` (the
/// `lexicon:wordnet` descriptor + every synset class/axiom) then `wordnet-NNN.esl`
/// LexicalEntry batches, each ≤ `split_bytes`. Every entry chunk carries the full
/// header (license + namespaces) and references its synset class + the descriptor by
/// IRI, so it resolves against the base layer below it — no cross-chunk dependency.
/// Load in filename order. Mirrors `umls-import`'s partitioned emit.
fn emit_partitioned(
    synsets: &[eigenius_wordnet::wndb::Synset],
    ranks: &eigenius_wordnet::convert::SenseRanks,
    mass: &MassNouns,
    dir: &Path,
    split_bytes: usize,
) -> ExitCode {
    if let Err(e) = fs::create_dir_all(dir) {
        eprintln!("error: creating {}: {e}", dir.display());
        return ExitCode::from(1);
    }

    let (base, entries, rep) = render_sections(synsets, ranks, mass);

    let mut files: Vec<PathBuf> = Vec::new();
    let base_path = dir.join("wordnet-000-base.esl");
    if let Err(e) = fs::write(&base_path, &base) {
        eprintln!("error: writing {}: {e}", base_path.display());
        return ExitCode::from(1);
    }
    files.push(base_path);

    let mut idx = 1usize;
    let mut cur = format!("{ESL_HEADER}\n");
    let mut chunk_entries = 0usize;

    let flush = |idx: usize, cur: &str| -> std::io::Result<PathBuf> {
        let path = dir.join(format!("wordnet-{idx:03}.esl"));
        fs::write(&path, cur)?;
        Ok(path)
    };

    for block in &entries {
        // Roll over before exceeding the cap (but never write an empty chunk).
        if chunk_entries > 0 && cur.len() + block.len() > split_bytes {
            match flush(idx, &cur) {
                Ok(p) => files.push(p),
                Err(e) => {
                    eprintln!("error: writing chunk {idx}: {e}");
                    return ExitCode::from(1);
                }
            }
            idx += 1;
            cur = format!("{ESL_HEADER}\n");
            chunk_entries = 0;
        }
        cur.push_str(block);
        chunk_entries += 1;
    }
    if chunk_entries > 0 {
        match flush(idx, &cur) {
            Ok(p) => files.push(p),
            Err(e) => {
                eprintln!("error: writing final chunk: {e}");
                return ExitCode::from(1);
            }
        }
    }

    eprintln!(
        "wordnet import: {} synsets → {} noun classes, {} instances, {} verb axioms, \
         {} adj axioms, {} entries ({} ger/pss participle forms)",
        synsets.len(),
        rep.noun_classes,
        rep.instances,
        rep.verb_axioms,
        rep.adj_axioms,
        rep.entries,
        rep.participle_entries,
    );
    eprintln!(
        "  ({} additive mass-noun entries from the countability lexicon)",
        rep.mass_entries
    );
    eprintln!(
        "wrote {} files → {} (base + {} entry chunks; load in filename order as a chain)",
        files.len(),
        dir.display(),
        files.len() - 1,
    );
    ExitCode::SUCCESS
}

/// Compile + structurally validate + felicity-gate the emitted ESL (all via kernel
/// library calls). Returns (admitted, rejected reasons).
fn validate(doc: &str) -> Result<(usize, Vec<String>), String> {
    let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap: {e}"))?;
    let wn_layer = build_layer(
        "wn",
        Arc::clone(ctx.head()),
        esl::compile_against_layer(doc, ctx.head()).map_err(|e| format!("wn compile: {e:?}"))?,
        LayerStorage::in_memory(),
    )?;

    let errors = Validator::new(Arc::clone(&wn_layer)).validate();
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
    for (id, r) in wn_layer.iter_resources() {
        if !r.is_instance_of(&entry_class) {
            continue;
        }
        match gate_entry(&wn_layer, &r) {
            Ok(_) => admitted += 1,
            Err(reason) => rejected.push(format!("{id}: {reason}")),
        }
    }
    Ok((admitted, rejected))
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
            .map_err(|e| format!("{name} add: {e:?}"))?;
    }
    Ok(Arc::new(b.build(storage)))
}
