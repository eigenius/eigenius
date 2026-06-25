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
//!     wordnet-import --seed gene --seed depend --out wn.esl --validate
//!     # stand up a PERSISTED standing layer (commit onto a RocksStore, advance `main`)
//!     wordnet-import --all --commit /var/lib/eigenius/wn.db
//!     # reload the persisted standing layer + build the parse index (fast)
//!     wordnet-import --from /var/lib/eigenius/wn.db
//!
//! Deterministic, no LLM. Noun selection is always **closed under hypernymy** (and
//! `entity.n.01` added) so the emitted `subclass_of` lattice is rooted; verbs/adjectives
//! type at the noun root and compose by subsumption. `--validate` compiles +
//! `Validator`-checks + felicity-gates the output via kernel library calls (no subprocess),
//! fail-closed.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use eigenius_kernel::dcg::{gate_entry, LexicalIndex};
use eigenius_kernel::lattice::commit_layer_default;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_kernel::validation::Validator;
use eigenius_kernel::{bootstrap, esl};
use eigenius_storage_rocksdb::RocksStore;
use eigenius_wordnet::convert::render_document;
use eigenius_wordnet::import::{select_synsets, SeedSpec};
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
    /// Write the ESL here.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Compile + validate + felicity-gate the output (self-check; fail-closed).
    #[arg(long)]
    validate: bool,
    /// Commit the import as a PERSISTED standing layer onto a RocksStore at this path
    /// (seeds/resumes the bootstrap chain, commits WordNet as its child, advances `main`).
    #[arg(long)]
    commit: Option<PathBuf>,
    /// Reload a persisted standing layer from this RocksStore path and build the parse
    /// index over it (the fast standing-layer reuse path; ignores selection flags).
    #[arg(long)]
    from: Option<PathBuf>,
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

    // Reload path: stand a persisted layer back up and build the parse index.
    if let Some(db) = &args.from {
        return match reload(db) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("from: error: {e}");
                ExitCode::from(1)
            }
        };
    }

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

    let (doc, rep) = render_document(&chosen);
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

    if let Some(db) = &args.commit {
        match commit_standing_layer(&doc, db) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("commit: error: {e}");
                return ExitCode::from(1);
            }
        }
    }

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

/// Stand the import up as a PERSISTED standing layer: open the RocksStore,
/// seed/resume the bootstrap chain, commit the WordNet ESL as its child, and
/// advance `main` so the layer is the resumable head (D63 §8.7 Slice 7,
/// "standing, parseable layer").
fn commit_standing_layer(doc: &str, db: &std::path::Path) -> Result<(), String> {
    let store: Arc<dyn PersistentBackend> =
        Arc::new(RocksStore::open(db).map_err(|e| format!("open {}: {e}", db.display()))?);
    let ctx = bootstrap::bootstrap_persistent(Arc::clone(&store))
        .map_err(|e| format!("bootstrap_persistent: {e}"))?;

    let resources =
        esl::compile_against_layer(doc, ctx.head()).map_err(|e| format!("wn compile: {e:?}"))?;
    let mut b = LayerBuilder::new("wn", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).map_err(|e| format!("wn add: {e:?}"))?;
    }
    let t0 = Instant::now();
    let layer = commit_layer_default(b, ctx.storage().clone(), store.as_ref())
        .map_err(|e| format!("commit: {e}"))?;
    store
        .put_branch("main", layer.id())
        .map_err(|e| format!("put_branch(main): {e}"))?;
    eprintln!(
        "commit: WordNet standing layer committed + `main` advanced to {} in {:.1?} → {}",
        layer.id(),
        t0.elapsed(),
        db.display(),
    );
    Ok(())
}

/// Reload a persisted standing layer and build the parse index over it — the fast
/// reuse path that turns the on-disk artifact back into a parseable index.
fn reload(db: &std::path::Path) -> Result<(), String> {
    let store: Arc<dyn PersistentBackend> =
        Arc::new(RocksStore::open(db).map_err(|e| format!("open {}: {e}", db.display()))?);
    let t0 = Instant::now();
    let ctx = bootstrap::bootstrap_persistent(Arc::clone(&store))
        .map_err(|e| format!("bootstrap_persistent: {e}"))?;
    let load = t0.elapsed();
    let t1 = Instant::now();
    let index = LexicalIndex::build(Arc::clone(ctx.head()));
    eprintln!(
        "from: standing layer reloaded in {load:.1?}, index built in {:.1?} ({} indexed forms) → {}",
        t1.elapsed(),
        index.len(),
        db.display(),
    );
    Ok(())
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
