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

//! `wordnet-import` — WordNet `data.<pos>` → Eigon lexicon ESL (D62 §8.7).
//!
//!     wordnet-import --dict /path/to/dict --seed gene --seed depend \
//!         --out wn.esl --validate
//!
//! Deterministic, no LLM. Noun selection is always **closed under hypernymy**
//! (and `entity.n.01` added) so the emitted `subclass_of` lattice is rooted and
//! self-consistent; verbs/adjectives type at the noun root and compose by
//! subsumption. `--validate` compiles + `Validator`-checks + felicity-gates the
//! output via kernel library calls (no subprocess), fail-closed.

use std::collections::{BTreeMap, BTreeSet};
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
use eigenius_wordnet::convert::render_document;
use eigenius_wordnet::wndb::{read_data_file, Offset, Pos, Synset};

const ENTITY_ROOT_OFFSET: &str = "00001740"; // entity.n.01

#[derive(Parser, Debug)]
#[command(about = "Import WordNet into Eigon lexicon ESL (D62 §8.7); deterministic, no LLM")]
struct Args {
    /// WordNet dict directory (contains data.noun / data.verb / data.adj).
    /// Defaults to the in-repo WordNet 3.0 distribution.
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
    /// Write the ESL here.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Compile + validate + felicity-gate the output (self-check; fail-closed).
    #[arg(long)]
    validate: bool,
    /// Lexicon schema ESL (provides `lexicon:Cat` / `lexicon:LexicalEntry`), for --validate.
    #[arg(long, default_value = "ontologies/lexicon/lexicon-ontology.esl")]
    schema: PathBuf,
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

/// Reflexive-transitive closure of `seeds` over the noun index along **both**
/// `@` hypernyms and `@i` instance-hypernyms — so an individual drags in the
/// class(es) it instantiates, and every class climbs to the `entity.n.01` root.
fn close_nouns(seeds: &BTreeSet<Offset>, noun: &BTreeMap<Offset, Synset>) -> BTreeSet<Offset> {
    let mut set = BTreeSet::new();
    let mut stack: Vec<Offset> = seeds.iter().cloned().collect();
    while let Some(o) = stack.pop() {
        if !set.insert(o.clone()) {
            continue;
        }
        if let Some(s) = noun.get(&o) {
            stack.extend(s.hypernyms.iter().cloned());
            stack.extend(s.instance_of.iter().cloned());
        }
    }
    set
}

/// Select seed offsets for one POS index per the flags.
fn select_seeds(index: &BTreeMap<Offset, Synset>, args: &Args) -> BTreeSet<Offset> {
    let mut seeds = BTreeSet::new();
    if args.all {
        seeds.extend(index.keys().cloned());
    }
    if let Some(n) = args.limit {
        seeds.extend(index.keys().take(n).cloned());
    }
    if !args.seed.is_empty() {
        let want: BTreeSet<&str> = args.seed.iter().map(String::as_str).collect();
        for (off, syn) in index {
            if syn.words.iter().any(|w| want.contains(w.as_str())) {
                seeds.insert(off.clone());
            }
        }
    }
    seeds
}

fn main() -> ExitCode {
    let args = Args::parse();

    if !args.all && args.limit.is_none() && args.seed.is_empty() {
        eprintln!("error: select a bound — one of --seed <lemma>, --limit <N>, or --all");
        return ExitCode::from(2);
    }

    let pos_set: Vec<Pos> = match args.pos.iter().map(|p| pos_of(p)).collect::<Option<_>>() {
        Some(v) => v,
        None => {
            eprintln!("error: --pos must be noun/verb/adj/adv");
            return ExitCode::from(2);
        }
    };

    // The noun index is always needed (closure + entity root).
    let noun = match read_data_file(&args.dict.join(Pos::Noun.data_file())) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: reading {}: {e}", Pos::Noun.data_file());
            return ExitCode::from(2);
        }
    };
    let verb = load_pos(&args.dict, Pos::Verb, &pos_set);
    let adj = load_pos(&args.dict, Pos::Adj, &pos_set);

    // Gather selected synsets.
    let mut chosen: Vec<Synset> = Vec::new();
    if pos_set.contains(&Pos::Noun) {
        let seeds = select_seeds(&noun, &args);
        let mut closed = close_nouns(&seeds, &noun);
        closed.insert(ENTITY_ROOT_OFFSET.to_string()); // root, for verb/adj typing
        chosen.extend(closed.iter().filter_map(|o| noun.get(o).cloned()));
    } else if !verb.is_empty() || !adj.is_empty() {
        // verbs/adjs type at the noun root → it must be present even if nouns
        // weren't requested.
        if let Some(root) = noun.get(ENTITY_ROOT_OFFSET) {
            chosen.push(root.clone());
        }
    }
    for (index, p) in [(&verb, Pos::Verb), (&adj, Pos::Adj)] {
        if pos_set.contains(&p) {
            let seeds = select_seeds(index, &args);
            chosen.extend(seeds.iter().filter_map(|o| index.get(o).cloned()));
        }
    }

    let (doc, rep) = render_document(&chosen);
    eprintln!(
        "wordnet import: {} synsets selected → {} noun classes, {} instances, {} verb axioms, \
         {} adj axioms, {} entries ({} verb synsets deferred: only predicative/clausal/control frames)",
        chosen.len(),
        rep.noun_classes,
        rep.instances,
        rep.verb_axioms,
        rep.adj_axioms,
        rep.entries,
        rep.verbs_deferred,
    );

    if let Some(path) = &args.out {
        if let Err(e) = fs::write(path, &doc) {
            eprintln!("error: writing {}: {e}", path.display());
            return ExitCode::from(1);
        }
        eprintln!("wrote ESL → {}", path.display());
    }

    if args.validate {
        match validate(&doc, &args.schema) {
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

fn load_pos(dict: &Path, p: Pos, requested: &[Pos]) -> BTreeMap<Offset, Synset> {
    if !requested.contains(&p) {
        return BTreeMap::new();
    }
    read_data_file(&dict.join(p.data_file())).unwrap_or_else(|e| {
        eprintln!("warning: reading {}: {e}", p.data_file());
        BTreeMap::new()
    })
}

/// Compile + structurally validate + felicity-gate the emitted ESL, all via
/// kernel library calls. Returns (admitted, rejected reasons).
fn validate(doc: &str, schema: &Path) -> Result<(usize, Vec<String>), String> {
    let ctx = bootstrap::bootstrap().map_err(|e| format!("bootstrap: {e}"))?;

    // Schema layer (lexicon:Cat / LexicalEntry) over the bootstrap head.
    let schema_src = fs::read_to_string(schema).map_err(|e| format!("schema read: {e}"))?;
    let schema_layer = build_layer(
        "wn-schema",
        Arc::clone(ctx.head()),
        esl::compile_against_layer(&schema_src, ctx.head())
            .map_err(|e| format!("schema compile: {e:?}"))?,
    )?;

    // WordNet import layer over the schema (so its lexicon:Cat ctors resolve).
    let wn_layer = build_layer(
        "wn",
        Arc::clone(&schema_layer),
        esl::compile_against_layer(doc, &schema_layer).map_err(|e| format!("wn compile: {e:?}"))?,
    )?;

    // Structural validation (requires/class_types/Rule 21/…).
    let errors = Validator::new(Arc::clone(&wn_layer)).validate();
    if !errors.is_empty() {
        return Err(format!(
            "{} structural error(s), e.g.: {}",
            errors.len(),
            errors[0]
        ));
    }

    // Felicity gate per entry (the cross-field check Rule 21 doesn't do).
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
) -> Result<Arc<Layer>, String> {
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in resources {
        b.add_resource(r)
            .map_err(|e| format!("{name} add: {e:?}"))?;
    }
    Ok(Arc::new(b.build(LayerStorage::in_memory())))
}
