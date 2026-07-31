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

//! Cross-lexicon concept unification (D63) — the deterministic stage.
//!
//!   lexicon-align candidates --meta-dir <UMLS META> --dict <WordNet dict> --out candidates.jsonl
//!
//! Emits every (UMLS concept, WordNet noun synset) pair sharing a surface where both sides carry a
//! gloss, plus a `gold` subset (near-identical glosses) that is the **answer key the adjudicator is
//! judged against** before it is trusted on the rest.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use eigenius_lexicon_align::adjudicate::Verdict;
use eigenius_lexicon_align::drops::{resolve_drops, DROP_CONFIDENCE};
use eigenius_lexicon_align::merge::{resolve, MERGE_CONFIDENCE};
use eigenius_lexicon_align::{candidates, gold, Candidate, GOLD_JACCARD};

#[derive(Parser, Debug)]
#[command(about = "WordNet↔UMLS concept unification (D63); deterministic, no LLM")]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Score the LLM adjudicator against the GOLD set — run this BEFORE trusting it on anything.
    /// A judge that cannot recover pairs whose glosses are near-identical is not a judge.
    #[cfg(feature = "use-llm")]
    ValidateGold {
        /// The candidates file from `candidates`.
        #[arg(long, default_value = "experiments/lexicon-align/candidates.jsonl")]
        candidates: PathBuf,
        /// Model id. The judge is the cost driver of the full run — compare tiers before committing.
        #[arg(long, default_value = "claude-sonnet-4-6")]
        model: String,
        /// Pairs per LLM call.
        #[arg(long, default_value_t = 20)]
        batch: usize,
        /// Cap the gold pairs scored (0 = all).
        #[arg(long, default_value_t = 0)]
        limit: usize,
        /// Where to record the verdicts.
        #[arg(long, default_value = "experiments/lexicon-align/gold-verdicts.jsonl")]
        out: PathBuf,
    },
    /// FULL adjudication of every candidate pair → the alignment proposals.
    ///
    /// Resumable: pairs already present in `--out` are skipped, so a run that dies picks up where it
    /// stopped. Fails closed per batch (retries, then records nothing for that batch) — a failed
    /// call is never treated as evidence of "different".
    #[cfg(feature = "use-llm")]
    Adjudicate {
        #[arg(long, default_value = "experiments/lexicon-align/candidates.jsonl")]
        candidates: PathBuf,
        #[arg(long, default_value = "claude-sonnet-4-6")]
        model: String,
        #[arg(long, default_value_t = 20)]
        batch: usize,
        /// Concurrent in-flight requests.
        #[arg(long, default_value_t = 16)]
        concurrency: usize,
        /// Retries per batch before giving up on it.
        #[arg(long, default_value_t = 3)]
        retries: usize,
        #[arg(long, default_value = "experiments/lexicon-align/alignment.jsonl")]
        out: PathBuf,
    },
    /// Estimate PRECISION: adjudicate a sample of NON-gold pairs and print every `same` verdict with
    /// both glosses, for inspection. This is the dangerous direction — a wrong merge fuses two
    /// distinct senses and DESTROYS the correct reading, where a missed merge only changes nothing.
    #[cfg(feature = "use-llm")]
    PrecisionProbe {
        #[arg(long, default_value = "experiments/lexicon-align/candidates.jsonl")]
        candidates: PathBuf,
        /// Model id. The judge is the cost driver of the full run — compare tiers before committing.
        #[arg(long, default_value = "claude-sonnet-4-6")]
        model: String,
        /// How many non-gold pairs to sample.
        #[arg(long, default_value_t = 200)]
        n: usize,
        #[arg(long, default_value_t = 20)]
        batch: usize,
        #[arg(long, default_value = "experiments/lexicon-align/probe-verdicts.jsonl")]
        out: PathBuf,
    },
    /// Generate the candidate pairs + the gold subset.
    Candidates {
        /// UMLS META dir (MRCONSO / MRDEF / MRSTY).
        #[arg(long, default_value = "references/umls/2026AA/META")]
        meta_dir: PathBuf,
        /// WordNet dict dir.
        #[arg(long, default_value = "references/WordNet-3.0/dict")]
        dict: PathBuf,
        /// Where to write the candidates (JSONL, one [`Candidate`] per line).
        #[arg(long, default_value = "experiments/lexicon-align/candidates.jsonl")]
        out: PathBuf,
    },
    /// Resolve the adjudicator's verdicts into the merge set the emitter consumes. Deterministic —
    /// the rules (one verdict per CONCEPT pair licenses every surface; confidence ≥ 0.85; ties
    /// dropped) are in [`eigenius_lexicon_align::merge`].
    Merges {
        #[arg(long, default_value = "experiments/lexicon-align/candidates.jsonl")]
        candidates: PathBuf,
        #[arg(long, default_value = "experiments/lexicon-align/alignment.jsonl")]
        verdicts: PathBuf,
        #[arg(long, default_value = "experiments/lexicon-align/merges.json")]
        out: PathBuf,
    },
    /// Resolve the adjudicator's verdicts into the DROP set the importer consumes — junk atoms whose
    /// only contribution is a case-mangled collision with a common word (`gENE`→`gene`). Deterministic:
    /// a confident `same=false` verdict on an irregular-cased `SY`/`PEP` atom, never on a merged
    /// surface. Rules are in [`eigenius_lexicon_align::drops`].
    Drops {
        #[arg(long, default_value = "experiments/lexicon-align/candidates.jsonl")]
        candidates: PathBuf,
        #[arg(long, default_value = "experiments/lexicon-align/alignment.jsonl")]
        verdicts: PathBuf,
        #[arg(long, default_value = "experiments/lexicon-align/drops.json")]
        out: PathBuf,
    },
}

fn load_candidates(p: &std::path::Path) -> std::io::Result<Vec<Candidate>> {
    let text = std::fs::read_to_string(p)?;
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str::<Candidate>(l).ok())
        .collect())
}

/// Verdicts + candidates → `merges.json`. The rules live in [`eigenius_lexicon_align::merge`]; this
/// only does the IO and reports what the resolution dropped.
fn build_merges(
    cpath: &std::path::Path,
    vpath: &std::path::Path,
    out: &std::path::Path,
) -> ExitCode {
    let cands = match load_candidates(cpath) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {} — {e}", cpath.display());
            return ExitCode::FAILURE;
        }
    };
    let text = match std::fs::read_to_string(vpath) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {} — {e}", vpath.display());
            return ExitCode::FAILURE;
        }
    };
    let verdicts: Vec<Verdict> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Verdict>(l).ok())
        .collect();

    let (merges, stats) = resolve(&cands, &verdicts);

    eprintln!("candidates            {}", cands.len());
    eprintln!("verdicts              {}", verdicts.len());
    eprintln!(
        "accepted concept pairs {}   (same=true, confidence ≥ {MERGE_CONFIDENCE})",
        stats.accepted_concept_pairs
    );
    eprintln!("ties dropped          {}", stats.ties_dropped);
    // Fails CLOSED and stays visible: a candidate the adjudicator never answered for is NOT
    // silently counted as "different".
    eprintln!("UNJUDGED (no verdict) {}", stats.unjudged);
    eprintln!("MERGES                {}", merges.len());

    let json = serde_json::to_string_pretty(&merges).expect("serialize merges");
    if let Some(p) = out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    if let Err(e) = std::fs::write(out, json) {
        eprintln!("error: {} — {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {}", out.display());
    ExitCode::SUCCESS
}

/// Verdicts + candidates → `drops.json`. The rules live in [`eigenius_lexicon_align::drops`]; this
/// only does the IO and reports what qualified.
fn build_drops(
    cpath: &std::path::Path,
    vpath: &std::path::Path,
    out: &std::path::Path,
) -> ExitCode {
    let cands = match load_candidates(cpath) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {} — {e}", cpath.display());
            return ExitCode::FAILURE;
        }
    };
    let text = match std::fs::read_to_string(vpath) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {} — {e}", vpath.display());
            return ExitCode::FAILURE;
        }
    };
    let verdicts: Vec<Verdict> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Verdict>(l).ok())
        .collect();

    let (drops, stats) = resolve_drops(&cands, &verdicts);

    eprintln!("candidates            {}", cands.len());
    eprintln!("verdicts              {}", verdicts.len());
    eprintln!(
        "merged (not dropped)  {}   (the merge owns the surface)",
        stats.merged_not_dropped
    );
    eprintln!(
        "DROPS                 {}   (same=false, confidence ≥ {DROP_CONFIDENCE}; irregular-cased SY/PEP atom, or metadata-artefact concept)",
        drops.len()
    );
    for d in drops.iter().take(40) {
        eprintln!(
            "    {:<10} {:<20} (→ {}, conf {:.2})",
            d.cui, d.form, d.surface, d.confidence
        );
    }
    if drops.len() > 40 {
        eprintln!("    … and {} more", drops.len() - 40);
    }

    let json = serde_json::to_string_pretty(&drops).expect("serialize drops");
    if let Some(p) = out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    if let Err(e) = std::fs::write(out, json) {
        eprintln!("error: {} — {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {}", out.display());
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args = Args::parse();
    match args.cmd {
        #[cfg(feature = "use-llm")]
        Cmd::Adjudicate {
            candidates: cpath,
            model,
            batch,
            concurrency,
            retries,
            out,
        } => adjudicate_all(&cpath, &model, batch, concurrency, retries, &out),
        #[cfg(feature = "use-llm")]
        Cmd::PrecisionProbe {
            candidates: cpath,
            model,
            n,
            batch,
            out,
        } => precision_probe(&cpath, &model, n, batch, &out),
        #[cfg(feature = "use-llm")]
        Cmd::ValidateGold {
            candidates: cpath,
            model,
            batch,
            limit,
            out,
        } => validate_gold(&cpath, &model, batch, limit, &out),
        Cmd::Merges {
            candidates: cpath,
            verdicts: vpath,
            out,
        } => build_merges(&cpath, &vpath, &out),
        Cmd::Drops {
            candidates: cpath,
            verdicts: vpath,
            out,
        } => build_drops(&cpath, &vpath, &out),
        Cmd::Candidates {
            meta_dir,
            dict,
            out,
        } => {
            eprintln!(">> scanning UMLS (MRDEF, MRSTY, MRCONSO) + WordNet nouns…");
            let cands = match candidates(&meta_dir, &dict) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };
            let g = gold(&cands);

            if let Some(p) = out.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            let Ok(f) = std::fs::File::create(&out) else {
                eprintln!("error: cannot write {}", out.display());
                return ExitCode::from(1);
            };
            let mut w = std::io::BufWriter::new(f);
            for c in &cands {
                let _ = writeln!(w, "{}", serde_json::to_string(c).unwrap());
            }

            eprintln!(
                "candidates: {} pairs (same surface, both glossed) → {}",
                cands.len(),
                out.display()
            );
            eprintln!(
                "gold (gloss Jaccard ≥ {GOLD_JACCARD}): {} pairs — the answer key the adjudicator \
                 must recover",
                g.len()
            );
            report_shape(&cands, &g);
            ExitCode::SUCCESS
        }
    }
}

/// What the candidate set actually looks like — so a wrong number is caught here, not three stages
/// downstream.
fn report_shape(cands: &[Candidate], g: &[&Candidate]) {
    let mut buckets = [0usize; 5];
    for c in cands {
        let b = match c.gloss_jaccard {
            j if j >= 0.75 => 0,
            j if j >= 0.50 => 1,
            j if j >= 0.25 => 2,
            j if j > 0.0 => 3,
            _ => 4,
        };
        buckets[b] += 1;
    }
    eprintln!("\n  gloss-Jaccard distribution:");
    for (label, n) in [
        ("≥0.75 (gold)", buckets[0]),
        ("0.50–0.75", buckets[1]),
        ("0.25–0.50", buckets[2]),
        (">0 –0.25", buckets[3]),
        ("0 (no overlap / too short)", buckets[4]),
    ] {
        eprintln!("    {label:<28} {n:>6}");
    }
    eprintln!("\n  sample gold pairs (near-identical glosses — near-certainly one concept):");
    for c in g.iter().take(5) {
        eprintln!(
            "    '{}'  C{} ≡ n{}  (J={:.2})",
            c.surface,
            &c.cui[1..],
            c.offset,
            c.gloss_jaccard
        );
        eprintln!("        umls: {}", truncate(&c.umls_gloss, 76));
        eprintln!("        wn  : {}", truncate(&c.wn_gloss, 76));
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

/// Score the adjudicator against the gold set. **The go/no-go for the whole approach.**
#[cfg(feature = "use-llm")]
fn validate_gold(
    cpath: &std::path::Path,
    model: &str,
    batch: usize,
    limit: usize,
    out: &PathBuf,
) -> ExitCode {
    use eigenius_lexicon_align::adjudicate::{adjudicate_batch, score_against_gold, Verdict};

    let Ok(cands) = load_candidates(cpath) else {
        eprintln!(
            "error: cannot read {} — run `candidates` first",
            cpath.display()
        );
        return ExitCode::from(1);
    };
    let mut g = gold(&cands);
    if limit > 0 {
        g.truncate(limit);
    }
    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("error: ANTHROPIC_API_KEY unset");
        return ExitCode::from(1);
    };
    eprintln!(
        ">> scoring the adjudicator on {} GOLD pairs (near-identical glosses), model {model}, \
         batches of {batch}",
        g.len()
    );

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut verdicts: Vec<Verdict> = Vec::new();
    for (n, chunk) in g.chunks(batch).enumerate() {
        match rt.block_on(adjudicate_batch(&key, model, chunk)) {
            Ok(v) => {
                eprintln!(
                    "   batch {n}: {} verdicts ({} same)",
                    v.len(),
                    v.iter().filter(|x| x.same).count()
                );
                verdicts.extend(v);
            }
            // Fail CLOSED: a failed call is not evidence of "different".
            Err(e) => {
                eprintln!("   batch {n}: FAILED — {e}");
                eprintln!("error: adjudication failed; refusing to report a partial score");
                return ExitCode::from(1);
            }
        }
    }

    if let Some(p) = out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    if let Ok(f) = std::fs::File::create(out) {
        let mut w = std::io::BufWriter::new(f);
        for v in &verdicts {
            let _ = writeln!(w, "{}", serde_json::to_string(v).unwrap());
        }
    }

    let score = score_against_gold(&g, &verdicts);
    eprintln!("\n=== ADJUDICATOR vs GOLD ===");
    eprintln!("  gold pairs      : {}", score.total);
    eprintln!("  called SAME     : {}", score.recovered);
    eprintln!("  RECALL          : {:.1}%", score.recall() * 100.0);
    eprintln!("  verdicts        : {}", out.display());
    if !score.missed.is_empty() {
        eprintln!("\n  MISSED — it called these near-identical glosses DIFFERENT:");
        for (surf, why) in score.missed.iter().take(12) {
            eprintln!("    {surf:<26} {why}");
        }
    }
    // The judge must recover glosses that are near-identical. Below this it cannot be trusted on the
    // 94% of pairs whose glosses only overlap slightly — which is the entire point of having it.
    if score.recall() < 0.95 {
        eprintln!("\n  VERDICT: recall < 95% — the adjudicator is NOT trustworthy. Stop here.");
        return ExitCode::from(2);
    }
    eprintln!(
        "\n  VERDICT: recall ≥ 95% — usable. Next: sample its SAME verdicts on non-gold pairs \
               to estimate precision (a wrong merge destroys a reading)."
    );
    ExitCode::SUCCESS
}

/// Adjudicate a SAMPLE of non-gold pairs and show every `same` verdict, so its precision can be
/// judged by reading. Deterministic sample (stride, not RNG) so the probe is repeatable.
#[cfg(feature = "use-llm")]
fn precision_probe(
    cpath: &std::path::Path,
    model: &str,
    n: usize,
    batch: usize,
    out: &PathBuf,
) -> ExitCode {
    use eigenius_lexicon_align::adjudicate::{adjudicate_batch, Verdict};
    use eigenius_lexicon_align::GOLD_JACCARD;

    let Ok(cands) = load_candidates(cpath) else {
        eprintln!("error: cannot read {}", cpath.display());
        return ExitCode::from(1);
    };
    let non_gold: Vec<&Candidate> = cands
        .iter()
        .filter(|c| c.gloss_jaccard < GOLD_JACCARD)
        .collect();
    let stride = (non_gold.len() / n.max(1)).max(1);
    let sample: Vec<&Candidate> = non_gold.iter().step_by(stride).take(n).copied().collect();

    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("error: ANTHROPIC_API_KEY unset");
        return ExitCode::from(1);
    };
    eprintln!(
        ">> precision probe: {} non-gold pairs (of {}), every {stride}th",
        sample.len(),
        non_gold.len()
    );

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut verdicts: Vec<Verdict> = Vec::new();
    for chunk in sample.chunks(batch) {
        match rt.block_on(adjudicate_batch(&key, model, chunk)) {
            Ok(v) => verdicts.extend(v),
            Err(e) => {
                eprintln!("error: batch failed — {e}");
                return ExitCode::from(1);
            }
        }
    }

    if let Some(p) = out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    if let Ok(f) = std::fs::File::create(out) {
        let mut w = std::io::BufWriter::new(f);
        for v in &verdicts {
            let _ = writeln!(w, "{}", serde_json::to_string(v).unwrap());
        }
    }

    let by_key: std::collections::BTreeMap<(&str, &str), &Candidate> = sample
        .iter()
        .map(|c| ((c.cui.as_str(), c.offset.as_str()), *c))
        .collect();
    let same: Vec<&Verdict> = verdicts.iter().filter(|v| v.same).collect();
    eprintln!(
        "\n=== PRECISION PROBE ===\n  sampled : {}\n  SAME    : {} ({:.0}%)  <- these become merges; read them",
        verdicts.len(),
        same.len(),
        100.0 * same.len() as f32 / verdicts.len().max(1) as f32
    );
    eprintln!("\n  Every proposed merge (judge by reading — a wrong one destroys a reading):");
    for v in same.iter().take(25) {
        let c = by_key.get(&(v.cui.as_str(), v.offset.as_str()));
        eprintln!(
            "\n  '{}'  (conf {:.2})  {}",
            v.surface, v.confidence, v.reason
        );
        if let Some(c) = c {
            eprintln!("      J={:.2}", c.gloss_jaccard);
            eprintln!("      umls: {}", truncate(&c.umls_gloss, 90));
            eprintln!("      wn  : {}", truncate(&c.wn_gloss, 90));
        }
    }
    ExitCode::SUCCESS
}

/// The full run: adjudicate every candidate pair, concurrently, with retries, resumably.
#[cfg(feature = "use-llm")]
fn adjudicate_all(
    cpath: &std::path::Path,
    model: &str,
    batch: usize,
    concurrency: usize,
    retries: usize,
    out: &PathBuf,
) -> ExitCode {
    use eigenius_lexicon_align::adjudicate::{adjudicate_batch, Verdict};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let Ok(cands) = load_candidates(cpath) else {
        eprintln!(
            "error: cannot read {} — run `candidates` first",
            cpath.display()
        );
        return ExitCode::from(1);
    };
    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("error: ANTHROPIC_API_KEY unset");
        return ExitCode::from(1);
    };

    // RESUME: skip pairs already adjudicated in `out`.
    let mut done: std::collections::BTreeSet<(String, String)> = Default::default();
    if let Ok(prev) = std::fs::read_to_string(out) {
        for l in prev.lines() {
            if let Ok(v) = serde_json::from_str::<Verdict>(l) {
                done.insert((v.cui, v.offset));
            }
        }
    }
    let todo: Vec<&Candidate> = cands
        .iter()
        .filter(|c| !done.contains(&(c.cui.clone(), c.offset.clone())))
        .collect();
    if !done.is_empty() {
        eprintln!(
            "resuming: {} already adjudicated, {} to go",
            done.len(),
            todo.len()
        );
    }
    if todo.is_empty() {
        eprintln!(
            "nothing to do — {} verdicts already in {}",
            done.len(),
            out.display()
        );
        return ExitCode::SUCCESS;
    }

    let batches: Vec<Vec<&Candidate>> = todo.chunks(batch).map(|c| c.to_vec()).collect();
    let total = batches.len();
    eprintln!(
        ">> adjudicating {} pairs in {total} batches of {batch}, {concurrency} concurrent, model {model}",
        todo.len()
    );

    if let Some(p) = out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
        .expect("open alignment.jsonl");
    let sink = Arc::new(std::sync::Mutex::new(std::io::BufWriter::new(file)));

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let ok = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let merges = Arc::new(AtomicUsize::new(0));
    let started = std::time::Instant::now();

    rt.block_on(async {
        let mut tasks = Vec::with_capacity(total);
        for b in batches {
            let (sem, key, model, sink) = (
                Arc::clone(&sem),
                key.clone(),
                model.to_string(),
                Arc::clone(&sink),
            );
            let (ok, failed, merges) = (Arc::clone(&ok), Arc::clone(&failed), Arc::clone(&merges));
            let owned: Vec<Candidate> = b.into_iter().cloned().collect();
            tasks.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore");
                let refs: Vec<&Candidate> = owned.iter().collect();
                let mut last = String::new();
                for attempt in 0..=retries {
                    match adjudicate_batch(&key, &model, &refs).await {
                        Ok(v) => {
                            let m = v.iter().filter(|x| x.same).count();
                            {
                                use std::io::Write;
                                let mut w = sink.lock().expect("sink");
                                for x in &v {
                                    let _ = writeln!(w, "{}", serde_json::to_string(x).unwrap());
                                }
                                let _ = w.flush(); // durable as we go: a crash loses nothing
                            }
                            merges.fetch_add(m, Ordering::Relaxed);
                            let n = ok.fetch_add(1, Ordering::Relaxed) + 1;
                            if n % 50 == 0 {
                                eprintln!(
                                    "   {n}/{total} batches  ({} merges so far, {} failed)",
                                    merges.load(Ordering::Relaxed),
                                    failed.load(Ordering::Relaxed)
                                );
                            }
                            return;
                        }
                        Err(e) => {
                            last = e;
                            // Back off: transient 429/529 are the common case at this concurrency.
                            tokio::time::sleep(std::time::Duration::from_millis(
                                500 << attempt.min(4),
                            ))
                            .await;
                        }
                    }
                }
                // Fail CLOSED: record NOTHING for this batch. A failed call is not a verdict, and a
                // silently-absent pair is simply left un-merged — the safe direction.
                eprintln!("   batch FAILED after {retries} retries: {last}");
                failed.fetch_add(1, Ordering::Relaxed);
            }));
        }
        for t in tasks {
            let _ = t.await;
        }
    });

    let (ok, failed, merges) = (
        ok.load(Ordering::Relaxed),
        failed.load(Ordering::Relaxed),
        merges.load(Ordering::Relaxed),
    );
    eprintln!("\n=== ADJUDICATION COMPLETE ===");
    eprintln!("  batches      : {ok} ok, {failed} failed (of {total})");
    eprintln!("  merges (same): {merges}");
    eprintln!(
        "  elapsed      : {:.1} min",
        started.elapsed().as_secs_f32() / 60.0
    );
    eprintln!("  verdicts     : {}", out.display());
    if failed > 0 {
        eprintln!(
            "\n  {failed} batches failed and were NOT recorded. Re-run this command to retry \
             them — it resumes from what is already in the file."
        );
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
