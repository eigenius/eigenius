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

//! Opening a lexicon snapshot and building the parser over it.
//!
//! The knobs (`SENSE_CAP`, `CELL_BEAM`) are **deliberately the measured ones** — the same constants
//! the parse-rate harness (`crates/eigenius-wordnet/tests/db_backed_encoding.rs`) uses. A demo that
//! parsed under different knobs would not be parsing the grammar the baseline measures, and the
//! pinned skeletons in `experiments/parsing/expected-readings.tsv` would not be the right pins.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::dcg::{LexicalIndex, Parser, ReplaySenseRanker};
use eigenius_kernel::layer::Layer;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;

/// Adaptive-supertagging sense cap — `db_backed_encoding.rs::SENSE_CAP`.
pub const SENSE_CAP: usize = 2;

/// Per-cell beam — `db_backed_encoding.rs::CELL_BEAM`.
pub const CELL_BEAM: usize = 64;

/// How to build the parser over a snapshot.
pub struct ParserConfig {
    /// The reranker recording. **Exists → REPLAY it** (deterministic, no LLM, no network, no key —
    /// what the demo ships with). **Absent → RECORD** the live ranker into it, which needs
    /// `--features use-llm` and `ANTHROPIC_API_KEY`.
    pub ranks: Option<PathBuf>,
}

/// Copy the store to a scratch directory and open the `main` head there.
///
/// **Never opens the caller's snapshot directly**: RocksDB takes an exclusive lock and rewrites the
/// store it opens, so pointing this at a shared snapshot would mutate it. `EIGENIUS_DB_WORKDIR`
/// places the copy (default: the system temp dir).
pub fn open_head(snapshot: &Path) -> Result<Arc<Layer>, String> {
    if !snapshot.join("CURRENT").exists() {
        return Err(format!(
            "no RocksDB store at {} (a valid store has a CURRENT file)",
            snapshot.display()
        ));
    }
    let work = working_copy(snapshot)?;
    let store =
        Arc::new(RocksStore::open(&work).map_err(|e| format!("open {}: {e:?}", work.display()))?);
    let backend: Arc<dyn PersistentBackend> = store;
    let ctx = bootstrap_persistent(Arc::clone(&backend)).map_err(|e| {
        format!(
            "cannot resume the snapshot — {e:?}.\n  The store's bootstrap must match the compiled \
             one: check out the seeding commit's ontologies/logic + ontologies/lexicon/closed-class, \
             or reseed."
        )
    })?;
    Ok(Arc::clone(ctx.head()))
}

/// The lazy parser over a resumed head: on-demand `lexicon:form` index probes (the only tractable
/// path at 7.6M resources — an eager full-chain scan OOMs).
///
/// In RECORD mode the returned [`Recording`] must be [`Recording::flush`]ed after parsing, or the
/// run produces no replayable artifact.
pub fn build_parser(head: &Arc<Layer>, cfg: &ParserConfig) -> Result<(Parser, Recording), String> {
    let lex = LexicalIndex::build(Arc::clone(head));
    let parser = Parser::over(Arc::new(lex), Arc::clone(head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(CELL_BEAM);
    let Some(path) = &cfg.ranks else {
        // Cap-only. Legitimate, but it is a DIFFERENT experiment: with no ranker there is no sense
        // ELIMINATION at all, so the reading set is not the one the pins were verified against.
        eprintln!("contextual reranker: none — cap-only (pins may not match)");
        return Ok((parser, Recording::None));
    };
    if path.exists() {
        let replay = ReplaySenseRanker::load(path)
            .map_err(|e| format!("--ranks {} could not be read: {e}", path.display()))?;
        eprintln!(
            "contextual reranker: REPLAY from {} (deterministic, no LLM)",
            path.display()
        );
        return Ok((parser.with_sense_ranker(Box::new(replay)), Recording::None));
    }
    record(parser, path.clone())
}

/// A live recording in flight, or nothing. Returned by [`build_parser`] so the caller can write the
/// artifact after the parse.
pub enum Recording {
    None,
    #[cfg(feature = "use-llm")]
    Live(
        std::sync::Arc<
            eigenius_kernel::dcg::RecordingSenseRanker<eigenius_kernel::dcg::AnthropicSenseRanker>,
        >,
        PathBuf,
    ),
}

impl Recording {
    /// Write the recording, if this run was recording. Call after the last parse: the artifact is
    /// what makes the run replayable without an LLM.
    pub fn flush(&self) -> Result<(), String> {
        match self {
            Self::None => Ok(()),
            #[cfg(feature = "use-llm")]
            Self::Live(rec, path) => {
                let n = rec
                    .write(path)
                    .map_err(|e| format!("write {}: {e}", path.display()))?;
                eprintln!("sense-ranks: recorded {n} rankings → {}", path.display());
                Ok(())
            }
        }
    }
}

#[cfg(feature = "use-llm")]
fn record(parser: Parser, path: PathBuf) -> Result<(Parser, Recording), String> {
    let Some(live) = eigenius_kernel::dcg::AnthropicSenseRanker::from_env() else {
        return Err(format!(
            "--ranks {} does not exist (RECORD mode) but ANTHROPIC_API_KEY is unset",
            path.display()
        ));
    };
    eprintln!(
        "contextual reranker: AnthropicSenseRanker (live) — RECORDING to {}",
        path.display()
    );
    let rec = std::sync::Arc::new(eigenius_kernel::dcg::RecordingSenseRanker::new(live));
    let parser = parser.with_sense_ranker(Box::new(ArcRanker(std::sync::Arc::clone(&rec))));
    Ok((parser, Recording::Live(rec, path)))
}

/// **Fail loudly, never silently unranked.** Without `use-llm` there is no live ranker, so a
/// non-existent `--ranks` would degrade the run to cap-only — where sense ELIMINATION is off and the
/// reading set is not the one the pins were verified against.
#[cfg(not(feature = "use-llm"))]
fn record(_parser: Parser, path: PathBuf) -> Result<(Parser, Recording), String> {
    Err(format!(
        "--ranks {} does not exist, and this binary has no live ranker (built without \
         --features use-llm), so the run would silently degrade to CAP-ONLY.\n  To replay: point at \
         an existing ranks.json. To record: rebuild with --features use-llm.",
        path.display()
    ))
}

/// Shares ownership of the recorder, so the caller can still flush after the `Parser` has taken its
/// `Box<dyn SenseRanker>`.
#[cfg(feature = "use-llm")]
struct ArcRanker(
    std::sync::Arc<
        eigenius_kernel::dcg::RecordingSenseRanker<eigenius_kernel::dcg::AnthropicSenseRanker>,
    >,
);

#[cfg(feature = "use-llm")]
impl eigenius_kernel::dcg::SenseRanker for ArcRanker {
    fn rank(
        &self,
        sentence: &str,
        context: &str,
        words: &[eigenius_kernel::dcg::WordSenses],
    ) -> Vec<Vec<usize>> {
        self.0.rank(sentence, context, words)
    }
}

/// Deletes the working copy when the process ends. Held in a thread-local for the life of the run.
///
/// Without this each invocation leaks a full copy of the snapshot — ~1 GB — and on a tmpfs `/tmp`
/// that is RAM. A morning of emitter runs filled a 16 GB tmpfs (2026-08-03).
struct SnapshotWorkdir(PathBuf);

impl Drop for SnapshotWorkdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

thread_local! {
    static SNAPSHOT_WORK: std::cell::RefCell<Option<SnapshotWorkdir>> =
        const { std::cell::RefCell::new(None) };
}

/// `cp -r --reflink=auto` the store into a scratch dir. Instant on a CoW filesystem.
fn working_copy(src: &Path) -> Result<PathBuf, String> {
    let root = std::env::var("EIGENIUS_DB_WORKDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dst = root.join(format!("eigenius-encoding-work-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    let ok = std::process::Command::new("cp")
        .args(["-r", "--reflink=auto"])
        .arg(src)
        .arg(&dst)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err(format!(
            "failed to copy snapshot {} → {}",
            src.display(),
            dst.display()
        ));
    }
    // Reap copies left by a run that was KILLED — `Drop` does not run on SIGKILL. Only reap a
    // directory whose owning process is gone.
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let name = e.file_name();
            let Some(pid) = name
                .to_str()
                .and_then(|n| n.strip_prefix("eigenius-encoding-work-"))
            else {
                continue;
            };
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    eprintln!(
        "snapshot: working copy → {} (the source is left untouched; removed on exit)",
        dst.display()
    );
    SNAPSHOT_WORK.with(|slot| *slot.borrow_mut() = Some(SnapshotWorkdir(dst.clone())));
    Ok(dst)
}
