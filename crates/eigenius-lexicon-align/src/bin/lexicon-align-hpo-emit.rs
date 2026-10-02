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

//! Emit the HPO↔UMLS alignment layer ([`eigenius_lexicon_align::hpo`]).
//!
//!   lexicon-align-hpo-emit --snapshot <store> --mrconso MRCONSO.RRF --merges merges.json --out hpo-alignment.esl
//!
//! The chain must carry HPO (the obograph import) and the WordNet↔UMLS layer. Reads the committed
//! UMLS entries **from the chain**, rewrites only `cat` and `sem` of the entries whose surface is one
//! of the HP code's own HPO names, and passes every other property through unchanged.

use std::collections::BTreeSet;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::ontology::{Iri, Value};
use eigenius_kernel::storage::PersistentBackend;
use eigenius_lexicon_align::emit::{load_merges, render, Rewrite, HPO_HEADER};
use eigenius_lexicon_align::hpo::{hp_qname, hpo_atoms, is_hpo_name, plan};
use eigenius_lexicon_align::read::{as_str, cat_n_num, entries_of, qname};
use eigenius_storage_rocksdb::RocksStore;

#[derive(Parser, Debug)]
#[command(about = "Emit the HPO↔UMLS alignment layer from the committed chain")]
struct Args {
    /// A snapshot of the store. **A COPY** — the reader takes RocksDB read-write and would
    /// otherwise mutate the snapshot it reads.
    #[arg(long)]
    snapshot: PathBuf,
    /// UMLS `MRCONSO.RRF` — its HPO rows carry NLM's HP code ↔ CUI mapping.
    #[arg(long, default_value = "references/umls/2026AA/META/MRCONSO.RRF")]
    mrconso: PathBuf,
    /// The WordNet↔UMLS merge set: its concepts stay WordNet's.
    #[arg(long, default_value = "experiments/lexicon-align/merges.json")]
    merges: PathBuf,
    #[arg(long, default_value = "experiments/lexicon-align/hpo-alignment.esl")]
    out: PathBuf,
    /// Highest form-index to probe per concept (entry IRIs are `e_<CUI>_<i>`).
    #[arg(long, default_value_t = 400)]
    max_form_index: usize,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let wordnet_cuis: BTreeSet<String> = match load_merges(&args.merges) {
        Ok(m) => m.keys().map(|(cui, _)| cui.clone()).collect(),
        Err(e) => {
            eprintln!("error: {} — {e}", args.merges.display());
            return ExitCode::from(1);
        }
    };
    let atoms = match std::fs::File::open(&args.mrconso) {
        Ok(f) => {
            let lines: Vec<String> = std::io::BufReader::new(f)
                .lines()
                .map_while(Result::ok)
                .filter(|l| l.contains("|HPO|"))
                .collect();
            hpo_atoms(lines.iter().map(String::as_str))
        }
        Err(e) => {
            eprintln!("error: {} — {e}", args.mrconso.display());
            return ExitCode::from(1);
        }
    };

    let store = Arc::new(RocksStore::open(&args.snapshot).expect("open snapshot"));
    let backend: Arc<dyn PersistentBackend> = store;
    let ctx = bootstrap_persistent(backend).expect("resume chain");
    let head = ctx.head();

    // Live = the HP class resolves on this chain and is not deprecated.
    let deprecated = Iri::parse("urn:eigenius:core:deprecated").unwrap();
    let is_live = |code: &str| {
        let Ok(iri) = Iri::parse(&format!("urn:obo:{code}")) else {
            return false;
        };
        head.resolve(&iri)
            .is_some_and(|r| !matches!(r.get(&deprecated), Some(Value::Boolean(true))))
    };
    let p = plan(&atoms, &wordnet_cuis, is_live);

    let form = Iri::parse("urn:eigenius:lexicon:form").unwrap();
    let cat = Iri::parse("urn:eigenius:lexicon:cat").unwrap();
    let sense = Iri::parse("urn:eigenius:lexicon:sense").unwrap();
    let in_lexicon = Iri::parse("urn:eigenius:lexicon:in_lexicon").unwrap();

    let mut body = String::new();
    let (mut written, mut skipped_named, mut no_entry, mut other_names) =
        (0usize, 0usize, 0usize, 0usize);
    let mut classes: BTreeSet<&str> = BTreeSet::new();
    for (cui, code) in &p.merges {
        let entries = entries_of(head, cui, args.max_form_index, 30);
        if entries.is_empty() {
            no_entry += 1;
            continue;
        }
        for (iri_s, r) in entries {
            let Some(f) = as_str(r.get(&form)) else {
                continue;
            };
            if !is_hpo_name(&atoms, cui, code, &f) {
                other_names += 1; // another source's synonym in the concept — stays UMLS's
                continue;
            }
            let Some(num) = r.get(&cat).and_then(|c| cat_n_num(c, cui, head)) else {
                skipped_named += 1; // named individual (cat_np) — cannot denote a class
                continue;
            };
            body.push_str(&render(&Rewrite {
                entry_iri: iri_s,
                num,
                class: hp_qname(code),
                form: f,
                sense: as_str(r.get(&sense)).unwrap_or_default(),
                in_lexicon: qname(&as_str(r.get(&in_lexicon)).unwrap_or_default()),
                sem_type: "Set".to_string(),
            }));
            written += 1;
            classes.insert(code);
        }
    }

    if let Some(dir) = args.out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let doc = format!("{HPO_HEADER}\n{body}");
    if std::fs::write(&args.out, &doc).is_err() {
        eprintln!("error: cannot write {}", args.out.display());
        return ExitCode::from(1);
    }

    eprintln!("\n=== HPO↔UMLS ALIGNMENT LAYER ===");
    eprintln!("  UMLS concepts with an HP code : {}", atoms.len());
    eprintln!("  merged concepts               : {}", p.merges.len());
    eprintln!("  left to WordNet               : {}", p.skipped_wordnet);
    eprintln!("  several live HP codes         : {}", p.skipped_ambiguous);
    eprintln!("  no live HP code on the chain  : {}", p.skipped_not_live);
    eprintln!("  merged concepts with no entry : {no_entry}");
    eprintln!(
        "  entries redefined             : {written}  ({} HP classes)",
        classes.len()
    );
    eprintln!("  kept (another source's name)  : {other_names}");
    eprintln!("  skipped (named individual)    : {skipped_named}");
    eprintln!(
        "  → {} ({:.1} MB)",
        args.out.display(),
        doc.len() as f32 / 1e6
    );
    ExitCode::SUCCESS
}
