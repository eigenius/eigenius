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

//! `specialist-senses` — the sense judge for adjectives (D97 decision 7; eigenius#263) and verbs
//! (decision 1, revised 2026-10-01; slice 2).
//!
//! ```text
//! # 1. What the evidence leaves open, and the gold it decides.
//! cargo run --release -p eigenius-lexicon-align --bin specialist-senses -- --pos verb items
//! # 2. Score the judge on the gold, the evidence hidden. Below 95% recall ⇒ STOP.
//! cargo run --release -p eigenius-lexicon-align --features use-llm --bin specialist-senses -- --pos verb validate-gold
//! # 3. Judge every open item. Resumable; a failed batch records nothing.
//! cargo run --release -p eigenius-lexicon-align --features use-llm --bin specialist-senses -- --pos verb adjudicate
//! # 4. A fixed sample of the placed senses, for review.
//! cargo run --release -p eigenius-lexicon-align --bin specialist-senses -- --pos verb precision-probe
//! # 5. The placements the WordNet importer reads.
//! cargo run --release -p eigenius-lexicon-align --bin specialist-senses -- --pos verb resolve
//! ```
//!
//! Committed, per part: the verdicts (`{pos}-senses.jsonl`), which are not reproducible, the
//! precision probe (`{pos}-precision-probe.tsv`) and the placements (`{pos}-senses.tsv`), which the
//! importer needs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use eigenius_lexicon_align::senses::{
    kind_of, load, read_verdicts, render_placements, resolve, score_gold, union, Item, Part,
    Placed, Verdict, Wording, ACCEPT, MIN_RECALL,
};

#[derive(Parser, Debug)]
#[command(about = "The sense judge for adjectives and verbs (D97 decisions 1 and 7)")]
struct Cli {
    /// WordNet dict directory.
    #[arg(long, global = true, default_value = "references/WordNet-3.0/dict")]
    dict: PathBuf,
    /// The SPECIALIST Lexicon (`scripts/provision-specialist.sh`).
    #[arg(long, global = true, default_value = "references/specialist/LEXICON")]
    specialist: PathBuf,
    /// Which judge.
    #[arg(long, global = true, value_enum, default_value_t = Pos::Adjective)]
    pos: Pos,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Pos {
    Adjective,
    Verb,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum WordingArg {
    Shared,
    PerKind,
}

impl From<WordingArg> for Wording {
    fn from(w: WordingArg) -> Wording {
        match w {
            WordingArg::Shared => Wording::Shared,
            WordingArg::PerKind => Wording::PerKind,
        }
    }
}

/// The acceptance threshold per kind of complement.
#[derive(clap::Args, Debug, Clone, Copy)]
struct Thresholds {
    #[arg(long, default_value_t = ACCEPT)]
    clause_threshold: f32,
    #[arg(long, default_value_t = ACCEPT)]
    object_threshold: f32,
    #[arg(long, default_value_t = ACCEPT)]
    preposition_threshold: f32,
}

impl Thresholds {
    fn of(&self, kind: &str) -> f32 {
        match kind {
            "clause" => self.clause_threshold,
            "object" => self.object_threshold,
            _ => self.preposition_threshold,
        }
    }
}

impl From<Pos> for Part {
    fn from(p: Pos) -> Part {
        match p {
            Pos::Adjective => Part::Adjective,
            Pos::Verb => Part::Verb,
        }
    }
}

/// `experiments/lexicon-specialist/{pos}-{suffix}`, unless `given`.
fn path(given: Option<PathBuf>, part: Part, suffix: &str) -> PathBuf {
    given.unwrap_or_else(|| {
        PathBuf::from(format!(
            "experiments/lexicon-specialist/{}-{suffix}",
            part.name()
        ))
    })
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Count the open items and the gold; `--list` prints them.
    Items {
        #[arg(long)]
        list: bool,
    },
    /// Print the prompt the judge is given for the named lemmas' items, one batch per kind, the gold
    /// among them looking like any other item.
    Prompt {
        lemmas: Vec<String>,
        #[arg(long, value_enum, default_value_t = WordingArg::Shared)]
        wording: WordingArg,
    },
    /// Score recorded verdicts on the gold sample, per kind, without calling the model. Several
    /// files are united: a sense is accepted where any of them accepts it.
    Score {
        #[arg(long, required = true)]
        verdicts: Vec<PathBuf>,
        #[arg(long, default_value_t = 150)]
        per_kind: usize,
        #[arg(long, default_value_t = 0)]
        skip: usize,
        #[command(flatten)]
        thresholds: Thresholds,
        /// Also print recall at thresholds 0.50 to 0.95.
        #[arg(long)]
        sweep: bool,
        /// Print the misses.
        #[arg(long)]
        misses: bool,
    },
    /// Judge the gold items first — the items whose senses WordNet's own evidence places — and
    /// score recall on those senses, per kind of complement. Run this BEFORE trusting the judge; its
    /// verdicts go where `adjudicate` records, which then judges the rest.
    ValidateGold {
        #[command(flatten)]
        run: Run,
        /// Default `{pos}-senses.jsonl`.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Gold items judged per kind of complement, a fixed sample; 0 judges them all.
        #[arg(long, default_value_t = 0)]
        per_kind: usize,
        /// Gold items per kind skipped before the sample, in the sample's order: `--skip 150
        /// --per-kind 150` is a held-out sample beside the first 150.
        #[arg(long, default_value_t = 0)]
        skip: usize,
        #[command(flatten)]
        thresholds: Thresholds,
    },
    /// Print a fixed sample of the senses the judge placed, and write it for review, so its
    /// precision can be read: a sense without WordNet's evidence is not a known negative.
    PrecisionProbe {
        /// Default `{pos}-senses.jsonl`.
        #[arg(long)]
        verdicts: Option<PathBuf>,
        #[arg(long, default_value_t = 40)]
        sample: usize,
        /// Default `{pos}-precision-probe.tsv`.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Judge every open item. Resumable: items already in `--out` are skipped. Fails closed per
    /// batch: retries, then records nothing for it.
    Adjudicate {
        #[command(flatten)]
        run: Run,
        /// Default `{pos}-senses.jsonl`.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Resolve the verdicts into the placements the importer reads. Deterministic; refuses while an
    /// open item has no verdict.
    Resolve {
        /// Default `{pos}-senses.jsonl`.
        #[arg(long)]
        verdicts: Option<PathBuf>,
        /// Default `{pos}-senses.tsv`.
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = ACCEPT)]
        threshold: f32,
    },
}

#[derive(clap::Args, Debug)]
struct Run {
    /// The adjective verdicts were drawn on `claude-sonnet-4-6`, as were the alignment's. The
    /// kernel's structured client reaches the Claude 5 models through `output_config.format`, since
    /// they refuse a forced tool choice (eigenius#264).
    #[arg(long, default_value = "claude-sonnet-4-6")]
    model: String,
    /// Items per call.
    #[arg(long, default_value_t = 8)]
    batch: usize,
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
    #[arg(long, default_value_t = 3)]
    retries: usize,
    /// The verb judge's wording.
    #[arg(long, value_enum, default_value_t = WordingArg::Shared)]
    wording: WordingArg,
}

/// A content-derived order, so a sample is the same on every run.
fn content_order<T: std::hash::Hash>(key: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

/// Up to `n` items of each kind of complement after the first `skip`, a fixed sample; all of them
/// when `n` is 0.
fn per_kind_sample(items: &[Item], n: usize, skip: usize) -> Vec<Item> {
    if n == 0 {
        return items.to_vec();
    }
    let mut by_kind: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    for i in items {
        by_kind.entry(kind_of(&i.complement)).or_default().push(i);
    }
    by_kind
        .into_values()
        .flat_map(|mut v| {
            v.sort_by_key(|i| content_order(&(&i.lemma, &i.complement)));
            v.into_iter().skip(skip).take(n).collect::<Vec<_>>()
        })
        .cloned()
        .collect()
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let part = Part::from(cli.pos);
    let sources = match load(&cli.dict, &cli.specialist, part) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let (open, gold) = (&sources.open, &sources.gold);
    match cli.cmd {
        Cmd::Items { list } => {
            for line in &sources.summary {
                eprintln!("{line}");
            }
            let senses: usize = open.iter().map(|i| i.senses.len()).sum();
            let mut gold_by_kind: BTreeMap<&str, usize> = BTreeMap::new();
            for i in gold {
                *gold_by_kind.entry(kind_of(&i.complement)).or_default() += 1;
            }
            eprintln!(
                "open: {} items over {} lemmas, {senses} sense judgements; gold: {} items {:?}",
                open.len(),
                open.iter().map(|i| &i.lemma).collect::<BTreeSet<_>>().len(),
                gold.len(),
                gold_by_kind
            );
            if list {
                for i in open {
                    println!("open\t{}\t{}\t{}", i.lemma, i.complement, i.senses.len());
                }
                for i in gold {
                    println!(
                        "gold\t{}\t{}\t{}/{}",
                        i.lemma,
                        i.complement,
                        i.gold.len(),
                        i.senses.len()
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Cmd::Prompt { lemmas, wording } => {
            let mut seen = BTreeSet::new();
            let items: Vec<Item> = open
                .iter()
                .chain(gold)
                .filter(|i| lemmas.contains(&i.lemma))
                .filter(|i| seen.insert((&i.lemma, &i.complement)))
                .cloned()
                .collect();
            for batch in eigenius_lexicon_align::senses::batches(&items, usize::MAX) {
                let refs: Vec<&Item> = batch.iter().collect();
                println!(
                    "{}\n",
                    eigenius_lexicon_align::senses::prompt(part, wording.into(), &refs)
                );
            }
            ExitCode::SUCCESS
        }
        Cmd::Score {
            verdicts,
            per_kind,
            skip,
            thresholds,
            sweep,
            misses,
        } => {
            let mut runs = Vec::new();
            for path in &verdicts {
                match read_verdicts(path) {
                    Ok(v) => runs.push(v),
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::from(1);
                    }
                }
            }
            let (verdicts, _) = sources.require_frame(&union(&runs));
            let gold = per_kind_sample(gold, per_kind, skip);
            if sweep {
                for kind in ["clause", "object", "preposition"] {
                    let items: Vec<Item> = gold
                        .iter()
                        .filter(|i| kind_of(&i.complement) == kind)
                        .cloned()
                        .collect();
                    let line: Vec<String> = [0.5, 0.6, 0.7, 0.75, 0.8, 0.85, 0.9, 0.95]
                        .iter()
                        .map(|&t| {
                            let s = score_gold(&items, &verdicts, t);
                            format!("{t:.2}: {:.3}/{}", s.recall(), s.false_pos)
                        })
                        .collect();
                    eprintln!(
                        "sweep {kind} (recall/yes beyond WordNet): {}",
                        line.join("  ")
                    );
                }
            }
            if report(&gold, &verdicts, &thresholds, misses) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Cmd::ValidateGold {
            run,
            out,
            per_kind,
            skip,
            thresholds,
        } => {
            let out = path(out, part, "senses.jsonl");
            let gold = per_kind_sample(gold, per_kind, skip);
            // Scored even where batches failed: their items count as unjudged, which refuses.
            let judged = judge_all(part, &gold, &run, &out);
            let verdicts = match read_verdicts(&out) {
                Ok(v) => sources.require_frame(&v).0,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };
            if !report(&gold, &verdicts, &thresholds, true) {
                return ExitCode::from(2);
            }
            judged
        }
        Cmd::Adjudicate { run, out } => {
            judge_all(part, open, &run, &path(out, part, "senses.jsonl"))
        }
        Cmd::PrecisionProbe {
            verdicts,
            sample,
            out,
        } => match read_verdicts(&path(verdicts, part, "senses.jsonl")) {
            Ok(verdicts) => precision_probe(
                part,
                open,
                &sources.require_frame(&verdicts).0,
                sample,
                &path(out, part, "precision-probe.tsv"),
            ),
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        },
        Cmd::Resolve {
            verdicts,
            out,
            threshold,
        } => {
            let out = path(out, part, "senses.tsv");
            let verdicts = match read_verdicts(&path(verdicts, part, "senses.jsonl")) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };
            // Only the open items' verdicts are placed; the gold items' are the judge's record.
            let open_keys: BTreeSet<(&str, &str)> = open
                .iter()
                .map(|i| (i.lemma.as_str(), i.complement.as_str()))
                .collect();
            let verdicts: Vec<Verdict> = verdicts
                .into_iter()
                .filter(|v| open_keys.contains(&(v.lemma.as_str(), v.complement.as_str())))
                .collect();
            let (verdicts, withdrawn) = sources.require_frame(&verdicts);
            if withdrawn > 0 {
                eprintln!(
                    "{withdrawn} yes votes withdrawn: the judge's sentence does not show the \
                     preposition right after the verb"
                );
            }
            match resolve(open, &verdicts, threshold) {
                Ok(rows) => {
                    if let Err(e) = std::fs::write(&out, render_placements(part, &rows, threshold))
                    {
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

/// Score `verdicts` on `gold` per kind of complement and print it; `true` if every kind meets
/// [`MIN_RECALL`] with no item unjudged.
fn report(gold: &[Item], verdicts: &[Verdict], thresholds: &Thresholds, misses: bool) -> bool {
    let mut by_kind: BTreeMap<&str, Vec<Item>> = BTreeMap::new();
    for i in gold {
        by_kind
            .entry(kind_of(&i.complement))
            .or_default()
            .push(i.clone());
    }
    let mut trusted = true;
    for (kind, items) in &by_kind {
        let threshold = thresholds.of(kind);
        let score = score_gold(items, verdicts, threshold);
        if misses {
            for d in score
                .disagreements
                .iter()
                .filter(|d| d.contains("evidence yes"))
            {
                eprintln!("  {d}");
            }
        }
        eprintln!(
            "=== GOLD ({kind}, accepted at {threshold}) === recall {:.3} ({} of {} senses WordNet's \
             frames place); yes beyond WordNet {} of {}; {} items unjudged",
            score.recall(),
            score.true_pos,
            score.true_pos + score.false_neg,
            score.false_pos,
            score.false_pos + score.true_neg,
            score.missing
        );
        trusted &= score.missing == 0 && score.recall() >= MIN_RECALL;
    }
    if !trusted {
        eprintln!(
            "STOP: the judge is not trusted below {MIN_RECALL} recall on the gold, or with gold \
             items unjudged."
        );
    }
    trusted
}

/// A fixed sample of the senses the judge placed on open items, printed and written for review.
fn precision_probe(
    part: Part,
    open: &[Item],
    verdicts: &[Verdict],
    sample: usize,
    out: &Path,
) -> ExitCode {
    let open_keys: BTreeSet<(&str, &str)> = open
        .iter()
        .map(|i| (i.lemma.as_str(), i.complement.as_str()))
        .collect();
    let gloss: BTreeMap<&str, &str> = open
        .iter()
        .flat_map(|i| i.senses.iter())
        .map(|s| (s.offset.as_str(), s.gloss.as_str()))
        .collect();
    let mut placed: Vec<(String, String, String, f32)> = verdicts
        .iter()
        .filter(|v| open_keys.contains(&(v.lemma.as_str(), v.complement.as_str())))
        .flat_map(|v| {
            v.senses
                .iter()
                .filter(|s| s.fits && s.confidence >= ACCEPT)
                .map(|s| {
                    (
                        v.lemma.clone(),
                        v.complement.clone(),
                        s.offset.clone(),
                        s.confidence,
                    )
                })
        })
        .collect();
    placed.sort_by_key(|(l, p, o, _)| content_order(&(l, p, o)));
    placed.truncate(sample);
    let column = match part {
        Part::Adjective => "preposition",
        Part::Verb => "complement",
    };
    let after = match part {
        Part::Adjective => ".",
        Part::Verb => {
            ", after the\n# adjacency test withdrew the yes votes whose sentence does not show the \
             preposition right after the verb."
        }
    };
    let mut tsv = format!(
        "# The {} sense judge's precision probe (D97): a fixed sample of the senses it placed{after}\n\
         # Review each: correct | wrong | unclear, with a note.\n\
         # lemma <TAB> {column} <TAB> offset <TAB> confidence <TAB> review <TAB> note <TAB> gloss\n",
        part.name()
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
fn judge_all(part: Part, items: &[Item], run: &Run, out: &Path) -> ExitCode {
    let done: BTreeSet<(String, String)> = match read_verdicts(out) {
        Ok(v) => v.into_iter().map(|v| (v.lemma, v.complement)).collect(),
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let todo: Vec<Item> = items
        .iter()
        .filter(|i| !done.contains(&(i.lemma.clone(), i.complement.clone())))
        .cloned()
        .collect();
    eprintln!(
        ">> judging {} items, model {} ({} already recorded in {})",
        todo.len(),
        run.model,
        done.len(),
        out.display()
    );
    let wording: Wording = run.wording.into();
    run_batches(
        eigenius_lexicon_align::senses::batches(&todo, run.batch),
        run,
        out,
        |i: &Item| format!("{} {}", i.lemma, i.complement),
        move |key, model, batch: Vec<Item>| async move {
            let refs: Vec<&Item> = batch.iter().collect();
            eigenius_lexicon_align::senses::judge_batch(&key, &model, part, wording, &refs).await
        },
    )
}

/// Run `batches` through `call`, `run.concurrency` at a time, appending each reply's records to
/// `out` as it lands. A failure is retried, except a refusal or a reply cut at its cap, which no
/// retry cures: such a batch is run one job at a time, so only a job that fails alone goes
/// unrecorded. A failed job records nothing.
#[cfg(feature = "use-llm")]
fn run_batches<T, R, F, Fut>(
    batches: Vec<Vec<T>>,
    run: &Run,
    out: &Path,
    label: fn(&T) -> String,
    call: F,
) -> ExitCode
where
    T: Clone + Send + Sync + 'static,
    R: serde::Serialize + Send + 'static,
    F: Fn(String, String, Vec<T>) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Vec<R>, String>> + Send + 'static,
{
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    if batches.is_empty() {
        eprintln!("nothing to do");
        return ExitCode::SUCCESS;
    }
    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("error: ANTHROPIC_API_KEY unset");
        return ExitCode::from(1);
    };
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
    let total = batches.len();
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
            let (ok, failed, call) = (Arc::clone(&ok), Arc::clone(&failed), call.clone());
            tasks.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore");
                let n = batch.len();
                let groups: Vec<Vec<T>> =
                    match attempt(&call, &key, &model, batch.clone(), retries).await {
                        Ok(records) => {
                            record(&sink, &records);
                            ok.fetch_add(n, Ordering::Relaxed);
                            return;
                        }
                        Err(e) if stops_for_good(&e) && n > 1 => {
                            eprintln!(
                                "   batch refused or cut at the cap; running its {n} jobs singly"
                            );
                            batch.into_iter().map(|t| vec![t]).collect()
                        }
                        Err(e) => {
                            let e: String = e.chars().take(200).collect();
                            eprintln!("   batch FAILED: {e}");
                            failed.fetch_add(n, Ordering::Relaxed);
                            return;
                        }
                    };
                for one in groups {
                    let name = label(&one[0]);
                    match attempt(&call, &key, &model, one, retries).await {
                        Ok(records) => {
                            record(&sink, &records);
                            ok.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            let e: String = e.chars().take(160).collect();
                            eprintln!("   {name} FAILED: {e}");
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }));
        }
        for t in tasks {
            let _ = t.await;
        }
    });
    let (ok, failed) = (ok.load(Ordering::Relaxed), failed.load(Ordering::Relaxed));
    eprintln!(
        "=== {ok} jobs recorded in {total} batches, {failed} failed → {}",
        out.display()
    );
    if failed > 0 {
        eprintln!("{failed} jobs were NOT recorded; re-run to retry them — it resumes.");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}

/// A failure retrying does not cure: a refusal, or a reply cut at its cap.
#[cfg(feature = "use-llm")]
fn stops_for_good(e: &str) -> bool {
    e.contains("stopped on refusal") || e.contains("stopped on max_tokens")
}

/// One call, retried on failure — except one [`stops_for_good`], returned at once.
#[cfg(feature = "use-llm")]
async fn attempt<T, R, F, Fut>(
    call: &F,
    key: &str,
    model: &str,
    batch: Vec<T>,
    retries: usize,
) -> Result<Vec<R>, String>
where
    T: Clone,
    F: Fn(String, String, Vec<T>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<R>, String>>,
{
    let mut last = String::new();
    for n in 0..=retries {
        match call(key.to_string(), model.to_string(), batch.clone()).await {
            Ok(r) => return Ok(r),
            Err(e) if stops_for_good(&e) => return Err(e),
            Err(e) => {
                last = e;
                tokio::time::sleep(std::time::Duration::from_millis(500 << n.min(4))).await;
            }
        }
    }
    Err(format!("after {retries} retries: {last}"))
}

/// Append records to the file, flushed: a crash loses nothing.
#[cfg(feature = "use-llm")]
fn record<R: serde::Serialize>(
    sink: &std::sync::Mutex<std::io::BufWriter<std::fs::File>>,
    records: &[R],
) {
    use std::io::Write;
    let mut w = sink.lock().expect("sink");
    for r in records {
        let _ = writeln!(w, "{}", serde_json::to_string(r).expect("record"));
    }
    let _ = w.flush();
}

#[cfg(not(feature = "use-llm"))]
fn judge_all(_part: Part, _items: &[Item], _run: &Run, _out: &Path) -> ExitCode {
    eprintln!("error: the judge needs `--features use-llm` (and ANTHROPIC_API_KEY)");
    ExitCode::from(1)
}
