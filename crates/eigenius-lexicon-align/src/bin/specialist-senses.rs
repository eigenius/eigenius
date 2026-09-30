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

//! `specialist-senses` — the adjective sense judge (D97 decision 7; eigenius#263).
//!
//! ```text
//! # 1. What the evidence leaves open, and the gold it decides.
//! cargo run --release -p eigenius-lexicon-align --bin specialist-senses -- items
//! # 2. Score the judge on the gold, the evidence hidden. Below 95% recall ⇒ STOP.
//! cargo run --release -p eigenius-lexicon-align --features use-llm --bin specialist-senses -- validate-gold
//! # 3. Judge every open item. Resumable; a failed batch records nothing.
//! cargo run --release -p eigenius-lexicon-align --features use-llm --bin specialist-senses -- adjudicate
//! # 4. The placements the WordNet importer reads.
//! cargo run --release -p eigenius-lexicon-align --bin specialist-senses -- resolve
//! ```
//!
//! Committed: the verdicts (`adjective-senses.jsonl`, `adjective-gold-verdicts.jsonl`), which are not
//! reproducible, and the placements (`adjective-senses.tsv`), which the importer needs.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use eigenius_lexicon_align::senses::{
    load, read_verdicts, render_placements, resolve, score_gold, Item, Placed, ACCEPT, MIN_RECALL,
};

#[derive(Parser, Debug)]
#[command(about = "The adjective sense judge (D97 decision 7; eigenius#263)")]
struct Cli {
    /// WordNet dict directory.
    #[arg(long, global = true, default_value = "references/WordNet-3.0/dict")]
    dict: PathBuf,
    /// The SPECIALIST Lexicon (`scripts/provision-specialist.sh`).
    #[arg(long, global = true, default_value = "references/specialist/LEXICON")]
    specialist: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Count the open items and the gold; `--list` prints them.
    Items {
        #[arg(long)]
        list: bool,
    },
    /// Judge the gold items first — the open items with a sense whose own gloss names the
    /// preposition — and score recall on those senses. Run this BEFORE trusting the judge; its
    /// verdicts go where `adjudicate` records, which then judges the rest.
    ValidateGold {
        #[command(flatten)]
        run: Run,
        #[arg(
            long,
            default_value = "experiments/lexicon-specialist/adjective-senses.jsonl"
        )]
        out: PathBuf,
    },
    /// Print a fixed sample of the senses the judge placed, and write it for review, so its
    /// precision can be read: a sense without WordNet's note is not a known negative.
    PrecisionProbe {
        #[arg(
            long,
            default_value = "experiments/lexicon-specialist/adjective-senses.jsonl"
        )]
        verdicts: PathBuf,
        #[arg(long, default_value_t = 40)]
        sample: usize,
        #[arg(
            long,
            default_value = "experiments/lexicon-specialist/precision-probe.tsv"
        )]
        out: PathBuf,
    },
    /// Judge every open item. Resumable: items already in `--out` are skipped. Fails closed per
    /// batch: retries, then records nothing for it.
    Adjudicate {
        #[command(flatten)]
        run: Run,
        #[arg(
            long,
            default_value = "experiments/lexicon-specialist/adjective-senses.jsonl"
        )]
        out: PathBuf,
    },
    /// Resolve the verdicts into the placements the importer reads. Deterministic; refuses while an
    /// open item has no verdict.
    Resolve {
        #[arg(
            long,
            default_value = "experiments/lexicon-specialist/adjective-senses.jsonl"
        )]
        verdicts: PathBuf,
        #[arg(
            long,
            default_value = "experiments/lexicon-specialist/adjective-senses.tsv"
        )]
        out: PathBuf,
        #[arg(long, default_value_t = ACCEPT)]
        threshold: f32,
    },
}

#[derive(clap::Args, Debug)]
struct Run {
    /// The kernel's structured client forces a tool choice, which the Claude 5 models refuse
    /// (`tool_choice: type "tool" and "any" are not supported for this model`, 2026-09-30); the
    /// client is eigenius#264's. The alignment's verdicts were drawn on this model too.
    #[arg(long, default_value = "claude-sonnet-4-6")]
    model: String,
    /// Items per call.
    #[arg(long, default_value_t = 8)]
    batch: usize,
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
    #[arg(long, default_value_t = 3)]
    retries: usize,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let sources = match load(&cli.dict, &cli.specialist) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let (open, gold) = (sources.open_items(), sources.gold_items());
    match cli.cmd {
        Cmd::Items { list } => {
            let c = &sources.classified.counts;
            eprintln!(
                "{} adjective lemmas attested; {} items in lexicon:Prep: {} on one sense, {} for \
                 the judge; outside lexicon:Prep: {:?}",
                c.lemmas_attested, c.items, c.one_sense, c.open, c.outside
            );
            let senses: usize = open.iter().map(|i| i.senses.len()).sum();
            eprintln!(
                "open: {} items over {} lemmas, {senses} sense judgements; gold: {} items",
                open.len(),
                open.iter()
                    .map(|i| &i.lemma)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                gold.len()
            );
            if list {
                for i in &open {
                    println!("open\t{}\t{}\t{}", i.lemma, i.preposition, i.senses.len());
                }
                for i in &gold {
                    println!(
                        "gold\t{}\t{}\t{}/{}",
                        i.lemma,
                        i.preposition,
                        i.gold.len(),
                        i.senses.len()
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Cmd::ValidateGold { run, out } => {
            let code = judge_all(&gold, &run, &out);
            if code != ExitCode::SUCCESS {
                return code;
            }
            let verdicts = match read_verdicts(&out) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };
            let score = score_gold(&gold, &verdicts, ACCEPT);
            for d in score
                .disagreements
                .iter()
                .filter(|d| d.contains("evidence yes"))
            {
                eprintln!("  {d}");
            }
            eprintln!(
                "\n=== GOLD === {} senses whose own gloss names the preposition: recall {:.3} ({} \
                 of {}); {} items unjudged",
                score.true_pos + score.false_neg,
                score.recall(),
                score.true_pos,
                score.true_pos + score.false_neg,
                score.missing
            );
            if score.missing > 0 || score.recall() < MIN_RECALL {
                eprintln!("STOP: the judge is not trusted below {MIN_RECALL} recall on the gold.");
                return ExitCode::from(2);
            }
            ExitCode::SUCCESS
        }
        Cmd::Adjudicate { run, out } => judge_all(&open, &run, &out),
        Cmd::PrecisionProbe {
            verdicts,
            sample,
            out,
        } => precision_probe(&open, &verdicts, sample, &out),
        Cmd::Resolve {
            verdicts,
            out,
            threshold,
        } => {
            let verdicts = match read_verdicts(&verdicts) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };
            match resolve(&open, &verdicts, threshold) {
                Ok(rows) => {
                    if let Err(e) = std::fs::write(&out, render_placements(&rows, threshold)) {
                        eprintln!("error: {}: {e}", out.display());
                        return ExitCode::from(1);
                    }
                    let count = |p| rows.iter().filter(|r| r.placed == p).count();
                    eprintln!(
                        "placements: {} items → {} ({} accepted, {} below the threshold, {} gaps)",
                        rows.len(),
                        out.display(),
                        count(Placed::Accepted),
                        count(Placed::BelowThreshold),
                        count(Placed::Gap)
                    );
                    ExitCode::SUCCESS
                }
                Err(missing) => {
                    eprintln!(
                        "error: {} open items have no verdict — run `adjudicate`; e.g. {}",
                        missing.len(),
                        missing
                            .iter()
                            .take(5)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("; ")
                    );
                    ExitCode::from(2)
                }
            }
        }
    }
}

/// A fixed sample of the senses the judge placed, printed and written for review.
fn precision_probe(open: &[Item], verdicts: &Path, sample: usize, out: &Path) -> ExitCode {
    let verdicts = match read_verdicts(verdicts) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let gloss: std::collections::BTreeMap<&str, &str> = open
        .iter()
        .flat_map(|i| i.senses.iter())
        .map(|s| (s.offset.as_str(), s.gloss.as_str()))
        .collect();
    let mut placed: Vec<(String, String, String, f32)> = verdicts
        .iter()
        .flat_map(|v| {
            v.senses
                .iter()
                .filter(|s| s.fits && s.confidence >= ACCEPT)
                .map(|s| {
                    (
                        v.lemma.clone(),
                        v.preposition.clone(),
                        s.offset.clone(),
                        s.confidence,
                    )
                })
        })
        .collect();
    // A content-derived order, so the sample is the same on every run.
    placed.sort_by_key(|(l, p, o, _)| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (l, p, o).hash(&mut h);
        h.finish()
    });
    placed.truncate(sample);
    let mut tsv = String::from(
        "# The adjective sense judge's precision probe (D97 decision 7; eigenius#263): a fixed sample\n\
         # of the senses it placed. Review each: correct | wrong | unclear, with a note.\n\
         # lemma <TAB> preposition <TAB> offset <TAB> confidence <TAB> review <TAB> note <TAB> gloss\n",
    );
    for (l, p, o, c) in &placed {
        let g = gloss.get(o.as_str()).copied().unwrap_or("");
        eprintln!("  {l} {p} {o} ({c:.2}): {g}");
        tsv.push_str(&format!("{l}\t{p}\t{o}\t{c:.2}\t\t\t{g}\n"));
    }
    if let Err(e) = std::fs::write(out, tsv) {
        eprintln!("error: {}: {e}", out.display());
        return ExitCode::from(1);
    }
    eprintln!("{} placed senses sampled → {}", placed.len(), out.display());
    ExitCode::SUCCESS
}

/// Judge `items` into `out`, concurrently, with retries, resumably.
#[cfg(feature = "use-llm")]
fn judge_all(items: &[Item], run: &Run, out: &Path) -> ExitCode {
    use eigenius_lexicon_align::senses::judge_batch;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("error: ANTHROPIC_API_KEY unset");
        return ExitCode::from(1);
    };
    let done: std::collections::BTreeSet<(String, String)> = match read_verdicts(out) {
        Ok(v) => v.into_iter().map(|v| (v.lemma, v.preposition)).collect(),
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let todo: Vec<Item> = items
        .iter()
        .filter(|i| !done.contains(&(i.lemma.clone(), i.preposition.clone())))
        .cloned()
        .collect();
    if todo.is_empty() {
        eprintln!(
            "nothing to do — {} verdicts in {}",
            done.len(),
            out.display()
        );
        return ExitCode::SUCCESS;
    }
    let batches: Vec<Vec<Item>> = todo
        .chunks(run.batch.max(1))
        .map(<[Item]>::to_vec)
        .collect();
    let total = batches.len();
    eprintln!(
        ">> judging {} items in {total} batches of {}, {} concurrent, model {} ({} already recorded)",
        todo.len(),
        run.batch,
        run.concurrency,
        run.model,
        done.len()
    );
    if let Some(p) = out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {}: {e}", out.display());
            return ExitCode::from(1);
        }
    };
    let sink = Arc::new(std::sync::Mutex::new(std::io::BufWriter::new(file)));
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let sem = Arc::new(tokio::sync::Semaphore::new(run.concurrency.max(1)));
    let (ok, failed) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let retries = run.retries;
    rt.block_on(async {
        let mut tasks = Vec::with_capacity(total);
        for batch in batches {
            let (sem, key, model, sink) = (
                Arc::clone(&sem),
                key.clone(),
                run.model.clone(),
                Arc::clone(&sink),
            );
            let (ok, failed) = (Arc::clone(&ok), Arc::clone(&failed));
            tasks.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore");
                let refs: Vec<&Item> = batch.iter().collect();
                let mut last = String::new();
                for attempt in 0..=retries {
                    match judge_batch(&key, &model, &refs).await {
                        Ok(verdicts) => {
                            use std::io::Write;
                            let mut w = sink.lock().expect("sink");
                            for v in &verdicts {
                                let _ =
                                    writeln!(w, "{}", serde_json::to_string(v).expect("verdict"));
                            }
                            let _ = w.flush(); // durable as we go: a crash loses nothing
                            ok.fetch_add(1, Ordering::Relaxed);
                            return;
                        }
                        Err(e) => {
                            last = e;
                            tokio::time::sleep(std::time::Duration::from_millis(
                                500 << attempt.min(4),
                            ))
                            .await;
                        }
                    }
                }
                // Fail CLOSED: a failed call is not a verdict, and records nothing.
                eprintln!("   batch FAILED after {retries} retries: {last}");
                failed.fetch_add(1, Ordering::Relaxed);
            }));
        }
        for t in tasks {
            let _ = t.await;
        }
    });
    let (ok, failed) = (ok.load(Ordering::Relaxed), failed.load(Ordering::Relaxed));
    eprintln!(
        "=== {ok} of {total} batches recorded, {failed} failed → {}",
        out.display()
    );
    if failed > 0 {
        eprintln!("{failed} batches were NOT recorded; re-run to retry them — it resumes.");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}

#[cfg(not(feature = "use-llm"))]
fn judge_all(_items: &[Item], _run: &Run, _out: &Path) -> ExitCode {
    eprintln!("error: the judge needs `--features use-llm` (and ANTHROPIC_API_KEY)");
    ExitCode::from(1)
}
