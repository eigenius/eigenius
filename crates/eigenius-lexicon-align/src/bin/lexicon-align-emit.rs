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
use eigenius_kernel::ontology::{Iri, Value};
use eigenius_kernel::storage::PersistentBackend;
use eigenius_lexicon_align::emit::{load_merges, render, Rewrite, HEADER};
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

/// The `num` argument of a `cat_n(umlscui:<CUI>, num)` category, or `None` if the category is not
/// that shape — which is how a **named individual** (`cat_np(umlssty:<TUI>, sg)`) is excluded: it is
/// an instance, not a class, and pointing it at a WordNet class would be a type error.
fn cat_n_num(cat: &Value, cui: &str) -> Option<String> {
    let Value::Json(j) = cat else { return None };
    let s = j.to_string();
    if !s.contains("\"cat_n\"") {
        return None; // cat_np (named individual) or anything else — skip.
    }
    if !s.contains(&format!("urn:eigenius:umlscui:{cui}")) {
        return None; // the category does not index THIS concept — do not touch it.
    }
    for n in ["num_any", "mass", "sg", "pl"] {
        if s.contains(&format!("\"{n}\"")) {
            return Some(n.to_string());
        }
    }
    None
}

fn as_str(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// `urn:eigenius:reflection:epistemic:declared` → `epistemic:declared`
fn qname(iri: &str) -> String {
    for (ns, pfx) in [
        ("urn:eigenius:reflection:epistemic:", "epistemic:"),
        ("urn:eigenius:lexicon:", "lexicon:"),
    ] {
        if let Some(local) = iri.strip_prefix(ns) {
            return format!("{pfx}{local}");
        }
    }
    iri.to_string()
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

    for (cui, wanted) in &by_cui {
        let mut hit = 0usize;
        let mut miss_run = 0usize;
        for i in 0..args.max_form_index {
            if miss_run > 30 && hit >= wanted.len() {
                break; // this concept's merged surfaces are all accounted for
            }
            let mut found_any = false;
            for suffix in ["", "_mass"] {
                let iri_s = format!("urn:eigenius:umlscui:e_{cui}_{i}{suffix}");
                let Ok(iri) = Iri::parse(&iri_s) else {
                    continue;
                };
                let Some(r) = head.resolve(&iri) else {
                    continue;
                };
                found_any = true;
                let Some(form) = as_str(r.get(&Iri::parse("urn:eigenius:lexicon:form").unwrap()))
                else {
                    continue;
                };
                let key = form.to_lowercase();
                let Some((_, off)) = wanted.iter().find(|(s, _)| *s == key) else {
                    continue; // this surface of the concept was NOT merged — leave it alone
                };
                let cat = r.get(&Iri::parse("urn:eigenius:lexicon:cat").unwrap());
                let Some(num) = cat.and_then(|c| cat_n_num(c, cui)) else {
                    skipped_named += 1; // named individual (cat_np) — cannot denote a class
                    continue;
                };
                body.push_str(&render(&Rewrite {
                    entry_iri: iri_s.clone(),
                    num,
                    wn_offset: (*off).to_string(),
                    form,
                    sense: as_str(r.get(&Iri::parse("urn:eigenius:lexicon:sense").unwrap()))
                        .unwrap_or_default(),
                    grade: qname(
                        &as_str(r.get(&Iri::parse("urn:eigenius:lexicon:grade").unwrap()))
                            .unwrap_or_default(),
                    ),
                    in_lexicon: qname(
                        &as_str(r.get(&Iri::parse("urn:eigenius:lexicon:in_lexicon").unwrap()))
                            .unwrap_or_default(),
                    ),
                    sem_type: "Set".to_string(),
                }));
                written += 1;
                hit += 1;
            }
            if found_any {
                miss_run = 0;
            } else {
                miss_run += 1;
            }
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
