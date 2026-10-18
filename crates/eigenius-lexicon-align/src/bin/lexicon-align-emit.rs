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

//! Emit the WordNet↔UMLS alignment layer (D63).
//!
//!   lexicon-align-emit --snapshot <store> --merges merges.json --out alignment.esl
//!
//! Reads the committed UMLS entries **from the chain** (never reconstructs them), rewrites only
//! `cat` and `sem` to denote the WordNet class, and passes every other property through unchanged.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_lexicon_align::emit::{load_merges, render, Rewrite, HEADER};
use eigenius_lexicon_align::read::{as_str, cat_n_num, entries_of, qname};
use eigenius_storage_rocksdb::RocksStore;

#[derive(Parser, Debug)]
#[command(about = "Emit the WordNet↔UMLS alignment layer from the committed chain")]
struct Args {
    /// A snapshot of the store. **A COPY** — the reader takes RocksDB read-write and would
    /// otherwise mutate the snapshot it reads (fixed for the parse harness on 2026-07-11).
    #[arg(long)]
    snapshot: PathBuf,
    #[arg(long, default_value = "experiments/lexicon-align/merges.json")]
    merges: PathBuf,
    #[arg(long, default_value = "experiments/lexicon-align/alignment.esl")]
    out: PathBuf,
    /// Highest form-index to probe per concept (entry IRIs are `e_<CUI>_<i>`).
    #[arg(long, default_value_t = 400)]
    max_form_index: usize,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let merges = match load_merges(&args.merges) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {} — {e}", args.merges.display());
            return ExitCode::from(1);
        }
    };
    eprintln!("merges: {} (cui, surface) → WordNet class", merges.len());

    let store = Arc::new(RocksStore::open(&args.snapshot).expect("open snapshot"));
    let backend: Arc<dyn PersistentBackend> = store;
    let ctx = bootstrap_persistent(backend).expect("resume chain");
    let head = ctx.head();

    // The CUIs we need, and for each the surfaces that were merged.
    let mut by_cui: std::collections::BTreeMap<&str, Vec<(&str, &str)>> = Default::default();
    for ((cui, surf), off) in &merges {
        by_cui
            .entry(cui.as_str())
            .or_default()
            .push((surf.as_str(), off.as_str()));
    }

    let mut body = String::new();
    let (mut written, mut skipped_named, mut not_found) = (0usize, 0usize, 0usize);

    let form_iri = Iri::parse("urn:eigenius:lexicon:form").unwrap();
    let cat_iri = Iri::parse("urn:eigenius:lexicon:cat").unwrap();
    let sense_iri = Iri::parse("urn:eigenius:lexicon:sense").unwrap();
    let in_lexicon_iri = Iri::parse("urn:eigenius:lexicon:in_lexicon").unwrap();
    for (cui, wanted) in &by_cui {
        // Every entry the importer minted for the concept, under every suffix it mints
        // (`read::entries_of`). This loop kept its own `["", "_mass"]`, and so never aligned D70's
        // `_name` entries.
        let mut hit = 0usize;
        for (iri_s, r) in entries_of(head, cui, args.max_form_index, 30) {
            let Some(form) = as_str(r.get(&form_iri)) else {
                continue;
            };
            let key = form.to_lowercase();
            let Some((_, off)) = wanted.iter().find(|(s, _)| *s == key) else {
                continue; // this surface of the concept was NOT merged — leave it alone
            };
            let Some(num) = r.get(&cat_iri).and_then(|c| cat_n_num(c, cui, head)) else {
                skipped_named += 1; // named individual (cat_np) — cannot denote a class
                continue;
            };
            body.push_str(&render(&Rewrite {
                entry_iri: iri_s,
                num,
                class: format!("wn:n{off}"),
                form,
                sense: as_str(r.get(&sense_iri)).unwrap_or_default(),
                in_lexicon: qname(&as_str(r.get(&in_lexicon_iri)).unwrap_or_default()),
                sem_type: "Set".to_string(),
            }));
            written += 1;
            hit += 1;
        }
        if hit == 0 {
            not_found += 1;
        }
    }

    if let Some(p) = args.out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let doc = format!("{HEADER}\n{body}");
    if std::fs::write(&args.out, &doc).is_err() {
        eprintln!("error: cannot write {}", args.out.display());
        return ExitCode::from(1);
    }

    eprintln!("\n=== ALIGNMENT LAYER ===");
    eprintln!("  entries redefined      : {written}");
    eprintln!(
        "  skipped (named indiv.) : {skipped_named}   (cat_np — an instance cannot denote a class)"
    );
    eprintln!("  concepts with no entry : {not_found}");
    eprintln!(
        "  → {} ({:.1} MB)",
        args.out.display(),
        doc.len() as f32 / 1e6
    );
    ExitCode::SUCCESS
}
