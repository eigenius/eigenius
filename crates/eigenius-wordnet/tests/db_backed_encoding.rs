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

//! D62 (d) — the **DB-backed encoding measurement**: the encoding prototype, but parsing over the
//! *full* committed lexicon (WordNet + UMLS) in a snapshot of the served RocksDB store, rather than
//! a seeded in-memory WordNet slice. This is the rerun that answers "is vocabulary the encode-gate"
//! against the *real* domain lexicon, not a page-seeded slice.
//!
//! It opens a **copy** of the docker-volume store (never the live volume — RocksDB takes an
//! exclusive lock) via the kernel's persistent backend, resumes the `main` branch head (the loaded
//! chain), and builds the **lazy** `Parser` (on-demand `lexicon:form` value-index probes —
//! the only tractable path at 7.6M resources; the eager full-chain scan OOMs). The sense cap
//! (adaptive supertagging) keeps the chart tractable on long sentences; with `--features use-llm`
//! and `ANTHROPIC_API_KEY`, the contextual reranker reorders which senses the cap keeps.
//!
//! NOTE — bootstrap alignment: the snapshot's persisted chain is rooted at the bootstrap it was
//! seeded with (Option B, this session). The code's `logic` + `closed-class` ontologies must match
//! that seeded version (checked out at commit `ff7f6cc`) or the resume fails closed with
//! `ManifestDrift`. The reranker / sense-cap live in the kernel binary, not the bootstrap, so they
//! apply regardless of which closed-class version is resumed.
//!
//! Point it at a snapshot with `EIGENIUS_DB_SNAPSHOT=/path/to/store`; absent (or the WordNet dict
//! is absent), the tests skip. Run:
//!
//!     cargo test -p eigenius-wordnet --test db_backed_encoding -- --ignored --nocapture
//!     cargo test -p eigenius-wordnet --features use-llm --test db_backed_encoding -- --ignored --nocapture

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::dcg::item::Item;
use eigenius_kernel::dcg::{
    abbreviation_resources, extract_abbreviations, glossary_resources, ground_abbreviation,
    is_nonprose, pretty_term, segment_sentences, tokenize, AbbreviationBinding, Identity,
    Lemmatizer, LexicalIndex, LexicalLookup, LexiconAugmentation, Parser, Pos,
};
use eigenius_kernel::layer::{resolve_active_value_indexes, Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::check::{check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::{Exp, Patt};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use eigenius_wordnet::lemmatizer::MorphyLemmatizer;

/// Default snapshot location — the out-of-tree `db-snapshot/` sibling of the repo (where
/// `scripts/reseed-lexicon-db.sh` / the native reseed write, `SNAPSHOT_ROOT = <repo>/../db-snapshot`),
/// resolved from `CARGO_MANIFEST_DIR` (portable, CWD-independent) rather than a hardcoded home path —
/// same convention as `DICT` below. Override with `EIGENIUS_DB_SNAPSHOT`.
const DEFAULT_SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../db-snapshot/wordnet-umls-aligned-v3-2026-07-16-quant"
);

/// WordNet dict (for the Morphy lemmatizer — surface→lemma at lookup time).
const DICT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../references/WordNet-3.0/dict"
);

/// A cleaned page of real WRN-paper prose (user-provided; OCR noise removed).
const WRN_PAGE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../references/publications/WRN-Helicase-Nature-OCR/first-page-cleaned.txt"
);

/// Adaptive-supertagging sense cap (Lever A, GH #97): keep the top-N senses per lemma so
/// WordNet+UMLS polysemy doesn't blow up the chart at the leaf.
const SENSE_CAP: usize = 2;

/// Per-cell beam (Lever B, GH #97): cap each non-top CKY cell to this many lowest-`Cost` items, so
/// a fully-known structurally-complex sentence's composed cells don't OOM the chart over the dense
/// full lexicon (where Lever A alone wasn't enough — the prior run OOM'd on a 17-token known unit).
/// UNVALIDATED at full-lexicon scale in the session that added it (the snapshot couldn't be resumed
/// — bootstrap drift); tune on the next fresh-DB run if OOM recurs.
const CELL_BEAM: usize = 64;

/// Parse budget: a fully-known unit longer than this is recorded as `ScaleBound` rather than
/// parsed — a backstop ABOVE the beam (the beam is the real OOM defense now). OOV diagnosis is
/// cheap at any length, so this bounds *only* the expensive CKY parse; the OOV/encode picture is
/// still measured for every unit. Raised from 12 (the pre-beam emergency value) to let the beam be
/// exercised on the page's long sentences; lower it if the beam proves insufficient on the rerun.
const PARSE_BUDGET: usize = 60;

/// The snapshot store path, or `None` (→ skip) when neither the env override nor the default
/// exists (a valid RocksDB store has a `CURRENT` file).
fn snapshot_path() -> Option<PathBuf> {
    let p = std::env::var("EIGENIUS_DB_SNAPSHOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_SNAPSHOT));
    if p.join("CURRENT").exists() {
        Some(p)
    } else {
        eprintln!(
            "SKIP db_backed_encoding: no RocksDB store at {} (set EIGENIUS_DB_SNAPSHOT)",
            p.display()
        );
        None
    }
}

/// Open the snapshot store and resume the `main` head (the loaded WordNet+UMLS chain). Returns
/// `None` (→ skip) on a `ManifestDrift`: the persisted chain is rooted at the bootstrap it was
/// seeded with, so the code's `logic`/`closed-class` ontologies must match that seeded version
/// (this session: checked out at `ff7f6cc`) or the resume fails closed. Rather than panic, skip —
/// so this committed test stays green whatever bootstrap the working tree currently compiles.
/// A **working copy** of the snapshot, removed when the run ends.
///
/// RocksDB has no read-only open in this build: `RocksStore::open` takes the DB read-write and
/// mutates it on the spot — WAL, `MANIFEST`, `OPTIONS`, `CURRENT`, compaction. So a measurement
/// pointed at a snapshot **rewrites that snapshot**, and a run that dies mid-way (a stack overflow,
/// a kill) can leave the baseline it was measured against in an unknown state. On 2026-07-11 the
/// reference snapshot's `CURRENT`/`OPTIONS`/`.log` all carried the day's mtimes for exactly this
/// reason, and hours were spent asking whether it had been corrupted.
///
/// The measurement must therefore treat its input as **immutable**: copy first, open the copy. The
/// copy costs ~2 s on a 2.7 GB store (page cache) against a ~6 min run — nothing. It matters more
/// still once a layer is *added* on top (the WordNet/UMLS alignment), because that genuinely writes.
struct SnapshotWorkdir(PathBuf);

impl Drop for SnapshotWorkdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

thread_local! {
    /// Held for the life of the test so the working copy outlives the store that opens it.
    static SNAPSHOT_WORK: std::cell::RefCell<Option<SnapshotWorkdir>> =
        const { std::cell::RefCell::new(None) };
}

/// Copy `src` to a scratch working directory and return the copy's path.
///
/// `EIGENIUS_DB_WORKDIR` places the copy (default: the system temp dir) — point it at a fast disk,
/// or at one with room for the store. `EIGENIUS_DB_INPLACE=1` opts OUT and opens `src` directly:
/// faster, and **it will modify the snapshot**. Only for a store you intend to write to.
fn working_copy(src: &std::path::Path) -> PathBuf {
    if std::env::var("EIGENIUS_DB_INPLACE").is_ok() {
        eprintln!(
            "snapshot: IN-PLACE (EIGENIUS_DB_INPLACE) — this run WILL MODIFY {}",
            src.display()
        );
        return src.to_path_buf();
    }
    let root = std::env::var("EIGENIUS_DB_WORKDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    // Sweep any copy left by a run that was KILLED — `Drop` does not run on SIGKILL, and each copy
    // is the size of the store (GBs). Only reap a directory whose owning process is gone.
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let name = e.file_name();
            let Some(pid) = name
                .to_str()
                .and_then(|n| n.strip_prefix("eigenius-snapshot-work-"))
            else {
                continue;
            };
            let alive = std::path::Path::new(&format!("/proc/{pid}")).exists();
            if !alive {
                eprintln!(
                    "snapshot: reaping stale working copy {}",
                    e.path().display()
                );
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let dst = root.join(format!("eigenius-snapshot-work-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    let t = std::time::Instant::now();
    // `--reflink=auto`: instant on a CoW filesystem, a plain copy elsewhere.
    let ok = std::process::Command::new("cp")
        .args(["-r", "--reflink=auto"])
        .arg(src)
        .arg(&dst)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(
        ok,
        "failed to copy snapshot {} → {}",
        src.display(),
        dst.display()
    );
    eprintln!(
        "snapshot: working copy → {} ({:.1}s; the source is left untouched)",
        dst.display(),
        t.elapsed().as_secs_f32()
    );
    SNAPSHOT_WORK.with(|slot| *slot.borrow_mut() = Some(SnapshotWorkdir(dst.clone())));
    dst
}

fn open_head(path: &std::path::Path) -> Option<Arc<Layer>> {
    // Never open the caller's snapshot directly — see [`working_copy`]. RocksDB would rewrite it.
    let work = working_copy(path);
    let store = Arc::new(RocksStore::open(&work).expect("open RocksStore snapshot"));
    let backend: Arc<dyn PersistentBackend> = store;
    match bootstrap_persistent(Arc::clone(&backend)) {
        Ok(ctx) => Some(Arc::clone(ctx.head())),
        Err(e) => {
            eprintln!(
                "SKIP db_backed_encoding: cannot resume the snapshot — {e:?}.\n  The store's \
                 bootstrap must match the compiled one; check out the seeding commit's \
                 ontologies/logic + ontologies/lexicon/closed-class, or reseed."
            );
            None
        }
    }
}

/// A `SenseRanker` that shares ownership, so the harness can still flush the recording after the
/// `Parser` has taken its `Box<dyn SenseRanker>`. Only the recording path constructs it.
#[cfg(feature = "use-llm")]
struct ArcRanker(std::sync::Arc<dyn eigenius_kernel::dcg::SenseRanker + Send + Sync>);
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

// The recorder wraps the LIVE ranker, which only exists under `use-llm`. Without the feature there
// is no LLM to record, so recording is a no-op (a replay still works — it needs no ranker at all).
#[cfg(feature = "use-llm")]
type Recorder = std::sync::Arc<
    eigenius_kernel::dcg::RecordingSenseRanker<eigenius_kernel::dcg::AnthropicSenseRanker>,
>;

thread_local! {
    /// The active REPLAY ranker, so a run can assert `misses() == 0` afterwards. A miss falls back to
    /// seed order, which makes `eff = min(cap, ranked)` a no-op — sense ELIMINATION silently OFF for
    /// that sentence. `misses()` existed but was never checked; see [`assert_replay_faithful`].
    static REPLAY_RANKER: std::cell::RefCell<
        Option<std::sync::Arc<eigenius_kernel::dcg::ReplaySenseRanker>>,
    > = const { std::cell::RefCell::new(None) };
}

#[cfg(feature = "use-llm")]
thread_local! {
    /// The live recording (if `EIGENIUS_SENSE_RANKS` named a file that did not yet exist) and where
    /// to write it. Flushed by [`flush_sense_ranks`] at the end of a measurement.
    static RANK_RECORDER: std::cell::RefCell<Option<(Recorder, PathBuf)>> =
        const { std::cell::RefCell::new(None) };
}

/// Write the recorded sense rankings, if this run was recording. Called at the END of a measurement:
/// the artifact is what makes the run replayable, and what lets a later parser change be A/B'd
/// against FIXED rankings — isolating the code from the model.
fn flush_sense_ranks() {
    #[cfg(feature = "use-llm")]
    RANK_RECORDER.with(|slot| {
        if let Some((rec, path)) = slot.borrow().as_ref() {
            match rec.write(path) {
                Ok(n) => eprintln!("sense-ranks: recorded {n} rankings → {}", path.display()),
                Err(e) => eprintln!("sense-ranks: FAILED to write {}: {e}", path.display()),
            }
        }
    });
}

/// Build the lazy `Parser` over the head with the sense cap, plus the live contextual
/// reranker when built with `--features use-llm` and `ANTHROPIC_API_KEY` is set.
fn build_index(head: &Arc<Layer>) -> Parser {
    build_index_over(head, None)
}

/// Same, but with a document's OOV groundings overlaid on the LEXICON first. The overlay is a fact
/// about words, so it is applied to the `LexicalIndex`; the `Parser` is then built over that lexicon.
/// (Before the lexicon/parser split this was `.with_document_augmentation(…)` chained onto the index,
/// back when the index *was* the parser.)
fn build_index_over(head: &Arc<Layer>, aug: Option<&LexiconAugmentation>) -> Parser {
    // Combinatory-core spike: `EIGENIUS_COMBINATORY_CORE=1` enables the extra CCG combinators for the
    // A/B port measurement (default off = the established rule-by-rule path).
    let core = std::env::var("EIGENIUS_COMBINATORY_CORE")
        .map(|v| v == "1")
        .unwrap_or(false);
    if core {
        eprintln!("combinatory-core: ON");
    }
    // Cross-POS prune experiment (GH#97): EIGENIUS_POS_PRUNE=1 drops function words' open-class
    // nominal readings at seed time (can→container, for→noun, is→beryllium).
    let pos_prune = std::env::var("EIGENIUS_POS_PRUNE").is_ok();
    if pos_prune {
        eprintln!("cross-POS prune: ON");
    }
    let mut lex = LexicalIndex::build(Arc::clone(head));
    if let Some(aug) = aug {
        lex = lex.with_document_augmentation(aug);
    }
    let index = Parser::over(Arc::new(lex), Arc::clone(head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(CELL_BEAM)
        .with_combinatory_core(core)
        .with_pos_prune(pos_prune);
    // ── Reproducibility: RECORD or REPLAY the reranker's decisions ───────────────────────────
    // The contextual reranker is an LLM — the one component that can answer differently for the
    // same code against the same store, which makes the measurement irreproducible and makes it
    // impossible to A/B a parser change (the LLM moves underneath you). `EIGENIUS_SENSE_RANKS`
    // turns it from an *uncontrolled* input into a *recorded* one:
    //   file EXISTS  → REPLAY it (deterministic, no network, no API cost)
    //   file ABSENT  → RECORD the live ranker into it (written at the end of the run)
    let ranks_path = std::env::var("EIGENIUS_SENSE_RANKS")
        .ok()
        .map(PathBuf::from);
    if let Some(p) = &ranks_path {
        if p.exists() {
            match eigenius_kernel::dcg::ReplaySenseRanker::load(p) {
                Ok(r) => {
                    eprintln!(
                        "contextual reranker: REPLAY from {} (deterministic, no LLM)",
                        p.display()
                    );
                    let r = std::sync::Arc::new(r);
                    REPLAY_RANKER.with(|s| *s.borrow_mut() = Some(std::sync::Arc::clone(&r)));
                    return index.with_sense_ranker(Box::new(ArcReplay(r)));
                }
                Err(e) => panic!(
                    "EIGENIUS_SENSE_RANKS={} exists but could not be read: {e}",
                    p.display()
                ),
            }
        }
    }
    #[cfg(feature = "use-llm")]
    {
        if let Some(ranker) = eigenius_kernel::dcg::AnthropicSenseRanker::from_env() {
            if let Some(p) = ranks_path {
                eprintln!(
                    "contextual reranker: AnthropicSenseRanker (live) — RECORDING to {}",
                    p.display()
                );
                let rec =
                    std::sync::Arc::new(eigenius_kernel::dcg::RecordingSenseRanker::new(ranker));
                RANK_RECORDER
                    .with(|slot| *slot.borrow_mut() = Some((std::sync::Arc::clone(&rec), p)));
                return index.with_sense_ranker(Box::new(ArcRanker(rec)));
            }
            eprintln!("contextual reranker: AnthropicSenseRanker (live)");
            return index.with_sense_ranker(Box::new(ranker));
        }
        eprintln!("contextual reranker: none (ANTHROPIC_API_KEY unset) — cap-only");
    }
    #[cfg(not(feature = "use-llm"))]
    {
        // **Fail loudly, never silently unranked.** `EIGENIUS_SENSE_RANKS` pointing at a file that
        // does not exist is legitimate ONLY in RECORD mode (a live ranker writes it). Without
        // `use-llm` there is no live ranker, so the run would degrade to cap-only — where the
        // reranker's ELIMINATION is disabled by construction, so eliminated senses re-seed and EVERY
        // per-unit conclusion drawn from the trace is wrong. That happened twice on 2026-07-21, both
        // times from a RELATIVE path: the test binary's CWD is the crate dir, not the repo root, so
        // the file silently "did not exist". A one-line log is not enough — this is now fatal.
        if let Some(p) = &ranks_path {
            panic!(
                "EIGENIUS_SENSE_RANKS={} does not exist, and this binary has no live ranker \
                 (built without --features use-llm) — so the run would silently degrade to CAP-ONLY, \
                 in which sense ELIMINATION is off and any per-unit conclusion is invalid.\n\
                 The path must be ABSOLUTE: the test binary's CWD is the crate dir, not the repo root.\n\
                 To replay: point at an existing ranks.json. To record: rebuild with --features use-llm.",
                p.display()
            );
        }
        eprintln!("contextual reranker: none (built without --features use-llm) — cap-only");
    }
    index
}

fn morphy() -> MorphyLemmatizer {
    MorphyLemmatizer::load(std::path::Path::new(DICT)).expect("load Morphy from dict")
}

/// Does this sem kernel-gate to a `Prop`? (the felicity confirmation)
fn gates_to_prop(layer: &Arc<Layer>, sem: &Exp) -> bool {
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(layer));
    matches!(check_infer(&mut ctx, sem), Ok(ty) if readback_val(0, &ty) == Exp::Sort(0))
}

/// The four-way outcome taxonomy the pipeline routes on (D62 §4). Mirrors `encoding_prototype.rs`
/// (duplicated — these are prototype drivers, not library code).
#[derive(Debug)]
enum Outcome {
    Encoded {
        is_prop: bool,
        /// The distinct STRUCTURAL skeletons among the closed readings (senses erased) — one entry for
        /// an encoded unit by construction. Held as the SET (not a count) so the faithfulness gate can
        /// ask "does this unit still CONTAIN its expected reading" (see `expected-readings.jsonl`).
        skeletons: Vec<String>,
    },
    Ambiguous {
        count: usize,
        is_prop: bool,
        /// The distinct STRUCTURAL skeletons among the `count` closed readings — the sense-independent
        /// (drift-free) bracketing set. `count / skeletons.len()` is the sense× multiplicity.
        skeletons: Vec<String>,
    },
    MissingLexeme {
        unknown: Vec<String>,
    },
    GrammarGap,
    /// All tokens known; no CLOSED parse but a felicitous OPEN parse (referent holes — `we`/`its`/
    /// pronouns, D64). NOT a grammar gap — it parses, awaiting reference resolution. Since an open sem
    /// is now a self-contained `Π`-abstraction (a *parametric* proposition — a well-typed EigenTT term,
    /// not a free-var fragment), it HAS distinct structural skeletons like a closed reading, so the
    /// faithfulness gate can certify an open unit's parametric reading (resolution is a separate fact).
    Open {
        holes: usize,
        /// Distinct STRUCTURAL skeletons among the open (parametric) readings, senses erased.
        skeletons: Vec<String>,
    },
    /// All tokens known, but the unit exceeds [`PARSE_BUDGET`] — parse skipped (would OOM the
    /// beam-less chart over the full lexicon). A *parsing-scale* gap, distinct from a vocab gap.
    ScaleBound {
        ntok: usize,
    },
}

struct UnitReport {
    text: String,
    outcome: Outcome,
}

/// The reading-count of a classified unit — its number of CLOSED full-span parses (the multiplicity
/// the `total_readings` metric sums). Encoded = 1, Ambiguous = its count; Open/GrammarGap/
/// MissingLexeme/ScaleBound produce no closed reading, so 0.
fn unit_readings(o: &Outcome) -> usize {
    match o {
        Outcome::Encoded { .. } => 1,
        Outcome::Ambiguous { count, .. } => *count,
        _ => 0,
    }
}

// `erase_senses` / `normalize_holes` now live in the KERNEL (`dcg::skeleton`) — the parser spends its
// felicity budget per skeleton, so the gate's notion of "same structure" and the parser's must be the
// SAME function. If they drifted, the parser could drop a bracketing this gate then reports as a lost
// reading. See kernel/src/dcg/skeleton.rs.
use eigenius_kernel::dcg::skeleton::erase_senses;

/// Pins the eraser semantics that `total-skeletons` depends on (README §7b). If this ever fails,
/// the tracked structural lever has started counting SENSE differences as structure again — which
/// silently inflated it by 26% before 2026-07-21.
#[test]
fn erase_senses_collapses_cross_lexicon_sense_pairs() {
    // One bracketing, a WordNet sense vs a UMLS sense in the same slot ⇒ ONE skeleton.
    let wn = erase_senses("compound_kind(G#0, n07342049)");
    let umls = erase_senses("compound_kind(G#0, C0205341)");
    assert_eq!(
        wn, umls,
        "cross-lexicon sense pair must not read as a structural difference"
    );
    // The STRUCTURE around the sense is preserved — this is not blanket erasure.
    assert_ne!(wn, erase_senses("prep_of(G#0, n07342049)"));
    // Short numbers are NOT sense ids and must survive (`G#0`, arity markers).
    assert!(
        erase_senses("G#0").contains('0'),
        "short digit runs are not sense ids"
    );
    // Verb/adjective sense atoms collapse the same way.
    assert_eq!(erase_senses("v02203362_t"), erase_senses("v00120796_t"));
}

/// Pins hole-binder α-normalisation (2026-07-24): a D64 open reading's hole `$name$i_j` is
/// position-keyed, so the SAME reading freshened at a different derivation site must still be ONE
/// skeleton. Regression guard for the `elided_than`-shift breakage of unit 4 (`$anaphor$6_60` →
/// `$anaphor$0_90`, structurally identical).
#[test]
fn erase_senses_normalises_hole_binder_names() {
    // Same structure, hole freshened at a different span ⇒ ONE skeleton.
    assert_eq!(
        erase_senses("λ$anaphor$6_60. gt(deg(x), $anaphor$6_60)"),
        erase_senses("λ$anaphor$0_90. gt(deg(x), $anaphor$0_90)"),
    );
    // Co-reference is preserved: the binder and its body use survive as the SAME canonical name.
    assert_eq!(
        erase_senses("λ$anaphor$2_30. f($anaphor$2_30)"),
        "λ$anaphor$0. f($anaphor$0)",
    );
    // Two DISTINCT holes stay distinct (structure preserved), each with its own canonical ordinal.
    let two = erase_senses("And(p($anaphor$1_10), q($quant$3_40, $anaphor$5_60))");
    assert!(
        two.contains("$anaphor$0") && two.contains("$anaphor$1") && two.contains("$quant$0"),
        "distinct holes must get distinct canonical names, per name prefix; got {two}"
    );
}

/// D63 Defect 2b — the acronyms MSI/MSS are DEFINED (parenthetically) only on the ORIGINAL page, not
/// the CNL. Schwartz-Hearst over the source text must bind both, so `MSS` grounds to microsatellite-
/// stable, not the `C0024814` "Marinesco-Sjogren syndrome" acronym collision the lexicon otherwise
/// returns. Snapshot-free: deterministic extraction only (the sweep additionally layers the LLM
/// proposer under `use-llm`).
#[test]
fn schwartz_hearst_binds_msi_mss_from_the_original_page() {
    // `references/` is gitignored (licensed source material), so the page is absent in CI and on a
    // fresh clone — skip rather than fail, as every other `WRN_PAGE` consumer here does.
    let page_path = std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let source = match std::fs::read_to_string(&page_path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("SKIP: {page_path} not found");
            return;
        }
    };
    let defs = extract_abbreviations(&source);
    let has = |sf: &str, lf: &str| {
        defs.iter()
            .any(|d| d.short_form == sf && d.long_form.to_lowercase().contains(lf))
    };
    assert!(
        has("MSS", "microsatellite stable"),
        "Schwartz-Hearst must bind MSS -> microsatellite stable from the source; got {:?}",
        defs.iter()
            .map(|d| (d.short_form.clone(), d.long_form.clone()))
            .collect::<Vec<_>>(),
    );
    assert!(
        has("MSI", "microsatellite instability"),
        "and MSI -> microsatellite instability; got {:?}",
        defs.iter()
            .map(|d| (d.short_form.clone(), d.long_form.clone()))
            .collect::<Vec<_>>(),
    );
}

/// Share one [`ReplaySenseRanker`] between the parser and the miss-check.
struct ArcReplay(std::sync::Arc<eigenius_kernel::dcg::ReplaySenseRanker>);

impl eigenius_kernel::dcg::SenseRanker for ArcReplay {
    fn rank(
        &self,
        sentence: &str,
        context: &str,
        words: &[eigenius_kernel::dcg::WordSenses],
    ) -> Vec<Vec<usize>> {
        self.0.rank(sentence, context, words)
    }
}

/// **A replay with misses is not a replay.** A key miss falls back to seed order, so for that
/// sentence every sense counts as ranked, `eff = min(cap, ranked)` stops cutting, and the reranker's
/// ELIMINATION is silently OFF — the run then measures something between reranked and cap-only while
/// still printing "REPLAY". `misses()` was counted but never asserted; this makes it fatal.
fn assert_replay_faithful() {
    REPLAY_RANKER.with(|slot| {
        if let Some(r) = slot.borrow().as_ref() {
            let (hits, misses) = (r.hits(), r.misses());
            eprintln!("  replay: {hits} hits, {misses} misses");
            assert_eq!(
                misses,
                0,
                "REPLAY had {misses} key MISSES (of {} lookups) — each falls back to seed order, \
                 disabling sense elimination for that sentence, so this is NOT a faithful replay and \
                 its per-unit numbers are not comparable. The recorded ranks.json does not answer \
                 this run's question (lexicon, page, or rank-key/prompt changed).",
                hits + misses
            );
        }
    });
}

/// The distinct STRUCTURAL skeletons among a unit's closed readings — the sense-independent (hence
/// reranker-drift-free) bracketings, senses erased. `total_readings` sense-multiplies these; the
/// skeleton COUNT does not, so it is the clean multiplicity signal (D63 baseline gates.multiplicity).
/// Returned as the SET (sorted, unique) so the faithfulness gate can test membership of an expected
/// reading, not just the cardinality.
fn skeleton_set(closed: &[Item]) -> Vec<String> {
    closed
        .iter()
        .map(|it| erase_senses(&pretty_term(it.sem())))
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════════════════════════
// Verbalizer — render a reading's `sem` back to approximate English, for HUMAN VERIFICATION of the
// expected-reading corpus (a skeleton is hard to check by eye; "every nucleotide-repeat region is a
// microsatellite" is easy). FAIL-HONEST: any construct it does not understand is emitted as `⟦raw⟧`,
// never smoothed into fluent-but-wrong English — a partial gloss must LOOK partial.
//
// Sense NAMING uses the LOADED lexicon, not the WordNet data files: each entry's `sense` key is
// `wn:{lemma}.{tag}.{offset}`, so the unit's own tokens yield `{tag}{offset} → lemma` from the seeded
// data (and the actual surface lemma, not just a synonym); UMLS names come from the layer description.
// Two limits remain: a sem atom not contributed by any single token falls back to the layer/local, and
// generalized-quantifier (Π-CPS) sems are bracketed, not verbalized. Enable `EIGENIUS_GLOSS_READINGS=1`.
// ═══════════════════════════════════════════════════════════════════════════════════════════════

/// `{tag}{offset} → lemma` (WordNet) and `C… → preferred name` (UMLS) for every sense reachable from a
/// unit's tokens — read off the LOADED lexicon's entry `sense` keys, so it is the seeded data.
fn unit_sense_names(
    text: &str,
    index: &Parser,
    lem: &dyn Lemmatizer,
    layer: &Arc<Layer>,
) -> std::collections::BTreeMap<String, String> {
    let mut m = std::collections::BTreeMap::new();
    for tok in tokenize(text) {
        let tok = tok.trim_matches(|c: char| !c.is_alphanumeric()); // shed attached commas/periods
        for (_closed, _cat, sense) in index.debug_form_entries(tok, lem) {
            // `wn:{lemma}.{tag}.{offset}` — split from the RIGHT: offset, tag, then the lemma (which
            // may itself contain '.').
            if let Some(rest) = sense.strip_prefix("wn:") {
                let parts: Vec<&str> = rest.rsplitn(3, '.').collect(); // [offset, tag, lemma]
                if let [offset, tag, lemma] = parts.as_slice() {
                    m.entry(format!("{tag}{offset}"))
                        .or_insert_with(|| lemma.replace('_', " "));
                }
            } else if let Some(cui) = sense.strip_prefix("umls:") {
                if let Some(name) = umls_name(cui, layer) {
                    m.entry(cui.to_string()).or_insert(name);
                }
            }
        }
    }
    m
}

/// The UMLS preferred name = the concept description up to its first ` - ` / `. ` (the loaded resource).
fn umls_name(cui: &str, layer: &Arc<Layer>) -> Option<String> {
    let iri = Iri::parse(&format!("urn:eigenius:umlscui:{cui}")).ok()?;
    let res = layer.resolve(&iri)?;
    let d = res.get(&Iri::parse("urn:eigenius:core:description").ok()?)?;
    let eigenius_kernel::ontology::resource::Value::String(d) = d else {
        return None;
    };
    // Descriptions are "Preferred Name — Definition [SOURCE] UMLS CUI C…". Split at the em-dash to
    // keep just the name. (The dash renders as the mojibake `â…` — a separate importer encoding bug —
    // so split on either form.)
    let name = d.split('—').next().unwrap_or(d);
    let name = name.split('â').next().unwrap_or(name);
    let name = name.split(" - ").next().unwrap_or(name);
    // A concept with NO definition has no em-dash at all — its description is just
    // `"Depletion. UMLS CUI C0333668."` — so the splits above leave the provenance suffix attached
    // and every reading mentioning it verbalises as "a Depletion. UMLS CUI C0333668. of WRN gene".
    // Cut at the suffix directly, then drop the sentence period the label is left with.
    let name = name.split(" UMLS CUI ").next().unwrap_or(name);
    Some(name.trim().trim_end_matches('.').trim().to_string())
}

/// Naming + layer context threaded through the walk.
struct Vb<'a> {
    names: &'a std::collections::BTreeMap<String, String>,
    layer: &'a Arc<Layer>,
}

fn app_spine(e: &Exp) -> (&Exp, Vec<&Exp>) {
    let mut args = Vec::new();
    let mut cur = e;
    while let Exp::App(f, x) = cur {
        args.push(x.as_ref());
        cur = f;
    }
    args.reverse();
    (cur, args)
}

/// The local name of a sense ATOM — an axiom, a class, or a named INDIVIDUAL.
///
/// The individual arm was missing until 2026-07-27, and it mattered: the UMLS importer declares a
/// concept as a `resource` (not a `class`) when it is an individual — `C0879389` "MLH1 gene",
/// `C1337007` "WRN gene" — and those reach the sem as [`Exp::EigonResource`]. Without this arm
/// `axiom_local` returned `None`, so [`name_atom`] was never consulted and every
/// `compound(x, <individual>)` reading verbalised as the raw `⟦C0879389⟧` bracket.
///
/// That blinded the verbaliser on exactly the readings under review when adjudicating the
/// `compound` / `compound_kind` split, since `compound` (`Entity -> Entity`) is the INDIVIDUAL
/// relation and `compound_kind` (`Entity -> Set`) the kind one — so the individual side of every
/// such pair was unreadable. `umls_name` resolves these fine (the resource carries a
/// `core:description`); only the extractor was refusing to hand it the key.
fn axiom_local(e: &Exp) -> Option<&str> {
    match e {
        Exp::EigonAxiom(i) | Exp::EigonClass(i) => {
            Some(i.as_str().rsplit(':').next().unwrap_or(""))
        }
        Exp::EigonResource(r) => r.id().map(|i| i.as_str().rsplit(':').next().unwrap_or("")),
        _ => None,
    }
}

/// `logic:False` — the negation codomain. It is built as `Exp::InductiveType(logic:False, [])`
/// (`constructions::negate_prop`), NOT as an axiom or class, so `axiom_local` never matched it and
/// the verbaliser's negation arms were dead: every negated proposition reached the ⟦…⟧ bracket.
fn is_false(e: &Exp) -> bool {
    match e {
        Exp::InductiveType(d, args) => args.is_empty() && d.iri.as_str().ends_with("logic:False"),
        _ => axiom_local(e) == Some("False"),
    }
}

/// The word for a sense atom: the unit's own lemma map first, then the UMLS layer name, else the local.
fn name_atom(local: &str, vb: &Vb) -> String {
    // Normalise: strip the `deg_`/`std_` adjective wrappers and any verb frame suffix (`_t`/`_i`/…).
    let core = local
        .strip_prefix("deg_")
        .or_else(|| local.strip_prefix("std_"))
        .unwrap_or(local);
    let key = core.split('_').next().unwrap_or(core);
    if let Some(w) = vb.names.get(key) {
        return w.clone();
    }
    if key.starts_with('C') && key[1..].chars().all(|c| c.is_ascii_digit()) {
        if let Some(w) = umls_name(key, vb.layer) {
            return w;
        }
    }
    key.to_string()
}

fn verbalize(sem: &Exp, vb: &Vb) -> String {
    match sem {
        Exp::Ann(inner, _) | Exp::Fst(inner) | Exp::Snd(inner) => return verbalize(inner, vb),
        Exp::Lam(_, body) => return verbalize(body, vb),
        Exp::Var(_) => return String::new(), // a bound restrictor variable — carries no surface
        _ => {}
    }
    if let Exp::InductiveType(decl, args) = sem {
        let d = decl.iri.as_str();
        if args.len() == 2 && (d.ends_with("logic:And") || d.ends_with("logic:Or")) {
            // Verb + shared-subject PP is ONE clause, not a conjunction: `And(V(subj), prep(subj, o))`
            // → "subj V prep o" (e.g. "MSI arises from Lynch syndrome"), the dominant sentence shape.
            if d.ends_with("And") {
                if let Some(merged) = verb_pp(&args[0], &args[1], vb) {
                    return merged;
                }
            }
            let op = if d.ends_with("And") { "and" } else { "or" };
            return format!(
                "{} {op} {}",
                verbalize(&args[0], vb),
                verbalize(&args[1], vb)
            );
        }
    }
    // Negation `A → False`. The Pi branch below catches the `Pi(_, A, False)` readback, but a
    // non-dependent arrow can also read back as `Exp::Arrow`, which that branch never sees — so a
    // negated coordination (`And(respond(x), prep_to(x, …)) → False`) stayed bracketed.
    if let Some((a, f)) = as_arrow(sem) {
        if is_false(f) {
            return format!("not ({})", verbalize(a, vb));
        }
    }
    if let Exp::Pi(binder, dom, cod) = sem {
        if is_false(cod) {
            return format!("not ({})", verbalize(dom, vb));
        }
        // Existential GQ (`exists_sem`/`obj_exists_sem`, closed-class.esl): `∀C:Prop. (∀x:A. body(x) →
        // C) → C`, readback `Pi(C, Prop, (Pi(x, A=Σ, body → C)) → C)`. Non-dependent `→` reads back as
        // `Pi(Patt::Unit, …)`, so match via `as_arrow`. → "some {A} {body}".
        if let Some((Exp::Pi(xb, a, arr), _c)) = as_arrow(cod) {
            // The restrictor may be a Σ-REFINED noun ("some MSI cell lines") or a PLAIN class
            // ("many cancers", "some cancers") — the quantifier encoding is identical either way,
            // and `quant_clause` goes through `bare_np`, which handles both. Requiring `Σ` here left
            // every unrefined-subject GQ bracketed: 46 of the residual ⟦…⟧ on the audited units.
            if matches!(a.as_ref(), Exp::Sig(..) | Exp::EigonClass(..)) {
                let parts = cps_body_parts(arr);
                if !parts.is_empty() {
                    let preds: Vec<String> = parts
                        .iter()
                        .map(|p| quant_clause_pred(xb, p, vb))
                        .filter(|x| !x.is_empty())
                        .collect();
                    let np = bare_np(a, vb);
                    return if preds.is_empty() {
                        format!("some {np}")
                    } else {
                        format!("some {np}, {}", preds.join(" and "))
                    };
                }
            }
        }
        // Universal / negative GQ over a Σ noun (`forall_sem`: `∀x:A. body`; `no_sem`: `∀x:A. body →
        // False`). Object variants (`obj_*`) fill the subject in, so the readback shape matches.
        if matches!(dom.as_ref(), Exp::Sig(..) | Exp::EigonClass(..)) {
            if let Some((body, f)) = as_arrow(cod) {
                if is_false(f) {
                    return format!("no {}", quant_clause(dom, binder, body, vb));
                }
            }
            return format!("every {}", quant_clause(dom, binder, cod, vb));
        }
        return format!("⟦{}⟧", pretty_term(sem)); // other Π — not verbalizable yet
    }
    if let Exp::Sig(_, base, restr) = sem {
        let np = noun_phrase(base, restr, vb);
        return format!("{} {np}", article(&np));
    }
    let (head, args) = app_spine(sem);
    // An application headed by a BOUND VARIABLE. The predicate slot of a clausal complement
    // ("These findings show that WRN is …", "We found that WRN was …") holds the abstracted
    // variable, so the embedded clause reads back as `G#0(C1337007)`. A bare `Var` already
    // verbalises to the empty string — it carries no surface — and its application should too;
    // render just the arguments. Without this the whole embedded clause fell to the ⟦…⟧ bracket,
    // which made every `that`-complement unit unauditable.
    if matches!(head, Exp::Var(_)) && !args.is_empty() {
        let parts: Vec<String> = args
            .iter()
            .map(|a| verbalize(a, vb))
            .filter(|s| !s.is_empty())
            .collect();
        return parts.join(" ");
    }
    if let Some(local) = axiom_local(head) {
        match (local, args.len()) {
            ("subclass_of", 2) => {
                return format!(
                    "every {} is {}",
                    bare_np(args[0], vb),
                    indefinite(args[1], vb)
                );
            }
            ("is_a", 2) => {
                return format!("{} is {}", verbalize(args[0], vb), indefinite(args[1], vb));
            }
            // Top-level gradable-adjective predication: `gt(deg_X(subj), std_X)` → "subj is X".
            // Two shapes share `gt` and only the ADJECTIVE one renders. A plain gradable
            // predication compares against the STANDARD — `gt(deg_X(subj), std_X)` -> "subj is X".
            // A COMPARATIVE compares against a real target, `gt(deg_X_rel(subj), <target>)`, and its
            // `than`-clause is currently DROPPED: "MSI cell lines showed greater dependence on WRN
            // than their MSS counterparts." renders as "WRN protein, human is a00725772".
            //
            // TRACED 2026-07-29, and the fix is NOT this arm alone. The discriminator is `args[1]`
            // (standard vs target), not a `deg_` prefix on `args[0]` — that prefix is present in
            // BOTH shapes (`deg_a00725772_rel`). But rendering the target requires an arm for
            // `deg_X_rel(a, b)` as well, which has none: adding the "more … than …" branch WITHOUT
            // it took bracketed glosses from 31 to 1833 of 2871, because that shape is pervasive.
            // Measured and reverted. The comparative stays mis-rendered until `deg_*_rel` renders.
            // Two shapes share `gt`.
            //
            //   PLAIN GRADABLE     gt(deg_X(subj), std_X)                     -> "subj is X"
            //   RELATIONAL COMPARATIVE
            //                      gt(deg_X_rel(g, s0), deg_X_rel(g, s1))     -> "s0 is more X on g than s1"
            //
            // `deg_{loc}_rel : Entity(ground) -> Entity(subject) -> float` (`convert.rs`), so a
            // comparative is TWO relational degrees over the SAME ground with different subjects.
            // "MSI cell lines … showed greater dependence on WRN than their MSS counterparts."
            //
            // TWO EARLIER ATTEMPTS FAILED HERE, both measured:
            //  - discriminating on a `deg_` prefix in `args[0]` did nothing, because BOTH shapes
            //    carry it (`deg_a00725772_rel`);
            //  - discriminating on `args[1]` and then verbalising that argument took bracketed
            //    glosses from 31 to 1833 of 2871, because a bare `deg_X_rel(a, b)` has no arm of its
            //    own and the shape is pervasive.
            // Destructuring BOTH arguments here avoids that: `verbalize` is never called on a
            // relational degree, only on its operands.
            ("gt" | "lt", 2) => {
                let (h0, a0) = app_spine(args[0]);
                let (h1, a1) = app_spine(args[1]);
                let l0 = axiom_local(h0);
                if let (Some(d0), Some(d1)) = (l0, axiom_local(h1)) {
                    if d0.ends_with("_rel") && d0 == d1 && a0.len() == 2 && a1.len() == 2 {
                        let word = if local == "gt" { "more" } else { "less" };
                        return format!(
                            "{} is {word} {} on {} than {}",
                            verbalize(a0[1], vb),
                            name_atom(d0, vb),
                            verbalize(a0[0], vb),
                            verbalize(a1[1], vb)
                        );
                    }
                }
                if let (Some(dl), Some(subj)) = (l0, a0.first()) {
                    return format!("{} is {}", verbalize(subj, vb), name_atom(dl, vb));
                }
            }
            ("kind_of", 1) => return verbalize(args[0], vb),
            ("the", 1) => return format!("the {}", bare_np(args[0], vb)),
            // Referential predication (D63 Defect 3): `the(subject-class, restrictor, x)` = "x is the
            // {subject-class} that is {restrictor}" — the copula's referential distribution over a
            // coordinated predicate nominal ("These groups are MSI lines, microsatellite-stable lines
            // and indeterminate lines"). Each And-conjunct is one of these; without this case the
            // 3-arg `the` fell through to the ⟦…⟧ bracket. `x` is usually a bound restrictor var (so
            // `verbalize` returns ""), giving "the {class} that is {restrictor}".
            ("the", 3) => {
                let subj = verbalize(args[2], vb);
                let cls = bare_np(args[0], vb);
                let restr = verbalize(args[1], vb);
                return if subj.is_empty() {
                    format!("the {cls} that is {restr}")
                } else {
                    format!("{subj} is the {cls} that is {restr}")
                };
            }
            // `poss_of` is POLYMORPHIC — `forall (A:Set) => A -> Entity -> Prop` — so it reads back
            // with the Set as a leading argument and the pair (possessed, possessor) after it.
            // Accept both arities; without this "their MSS counterparts" bracketed.
            ("poss_of", 2 | 3) => {
                let (owned, owner) = if args.len() == 3 {
                    (args[1], args[2])
                } else {
                    (args[0], args[1])
                };
                let o = verbalize(owner, vb);
                let n = verbalize(owned, vb);
                return if o.is_empty() {
                    format!("its {n}")
                } else {
                    format!("{o}'s {n}")
                };
            }
            ("Possible" | "modal", 1) => return format!("possibly, {}", verbalize(args[0], vb)),
            ("speaker", _) => return "we".to_string(),
            ("anaphor", _) => return "it".to_string(),
            _ => {}
        }
        // A PP predication standing ALONE — `prep_in(subj, obj)`. `verb_pp` merges the common
        // `And(V(subj), prep(subj, obj))` shape into a single clause, but a PP conjunct it cannot
        // merge — a distributed coordination, or a clausal complement — reached the ⟦…⟧ bracket.
        // The subject is usually a bound restrictor variable (verbalising to ""), giving "in X".
        if let Some(p) = local.strip_prefix("prep_") {
            if args.len() == 2 {
                let subj = verbalize(args[0], vb);
                let obj = verbalize(args[1], vb);
                return if subj.is_empty() {
                    format!("{p} {obj}")
                } else {
                    format!("{subj} {p} {obj}")
                };
            }
        }
        // Verb: `v{offset}_{frame}(obj, subj)` transitive / `(subj)` intransitive (category
        // `(S\NP)/NP` — object first; convert.rs:225).
        if local.starts_with('v') && local.contains('_') {
            let verb = name_atom(local, vb);
            // The frame tag is the suffix after the last `_` (`convert.rs`: `_i` intransitive,
            // `_t` transitive, `_p` PP-oblique, `_as` ESSIVE, `_d` ditransitive). A 3-argument
            // frame had no arm at all, so every essive clause — "identified WRN AS the top
            // dependency", "evaluated MSI AS a biomarker" — bracketed in full.
            let tag = local.rsplit('_').next().unwrap_or("");
            return match args.as_slice() {
                [subj] => format!("{} {verb}", verbalize(subj, vb)),
                [obj, subj] => format!("{} {verb} {}", verbalize(subj, vb), verbalize(obj, vb)),
                [obj, comp, subj] if tag == "as" => format!(
                    "{} {verb} {} as {}",
                    verbalize(subj, vb),
                    verbalize(obj, vb),
                    verbalize(comp, vb)
                ),
                [a, b, subj] => format!(
                    "{} {verb} {} {}",
                    verbalize(subj, vb),
                    verbalize(a, vb),
                    verbalize(b, vb)
                ),
                _ => format!("⟦{}⟧", pretty_term(sem)),
            };
        }
    }
    if let Some(local) = axiom_local(sem) {
        return name_atom(local, vb);
    }
    format!("⟦{}⟧", pretty_term(sem))
}

/// `And(V(subj), prep_X(subj, obj))` → "subj V prep obj" when the two share a subject; else `None`.
fn verb_pp(left: &Exp, right: &Exp, vb: &Vb) -> Option<String> {
    let (lh, la) = app_spine(left);
    let (rh, ra) = app_spine(right);
    let ll = axiom_local(lh)?;
    let rl = axiom_local(rh)?;
    if !(ll.starts_with('v') && ll.contains('_') && rl.starts_with("prep_") && ra.len() == 2) {
        return None;
    }
    // Intransitive/PP verb: its sole arg is the subject; it must match the PP's first arg.
    let subj = match la.as_slice() {
        [s] => s,
        _ => return None,
    };
    if pretty_term(subj) != pretty_term(ra[0]) {
        return None;
    }
    Some(format!(
        "{} {} {} {}",
        verbalize(subj, vb),
        name_atom(ll, vb),
        &rl[5..],
        verbalize(ra[1], vb)
    ))
}

/// A quantifier's body over the bound entity: "{NP}, {predicate}" with the bound variable (already
/// named by the NP) rendered as "it", so the coreference is legible — "some group of cell lines, we
/// identified it". Fail-honest: an empty predicate degrades to just the NP.
/// One conjunct of a quantifier body, with the bound variable replaced by the anaphor placeholder —
/// the per-part half of [`quant_clause`], so a CPS body with SEVERAL conjuncts can render each.
fn quant_clause_pred(xbinder: &Patt, body: &Exp, vb: &Vb) -> String {
    let body = match xbinder {
        Patt::Var(x) => subst_var(body, x, &anaphor_atom()),
        _ => body.clone(),
    };
    verbalize(&body, vb).trim().to_string()
}

fn quant_clause(np_sig: &Exp, xbinder: &Patt, body: &Exp, vb: &Vb) -> String {
    let np = bare_np(np_sig, vb);
    let body = match xbinder {
        Patt::Var(x) => subst_var(body, x, &anaphor_atom()),
        _ => body.clone(),
    };
    let pred = verbalize(&body, vb);
    if pred.trim().is_empty() {
        np
    } else {
        format!("{np}, {pred}")
    }
}

/// The BODY of a CPS-encoded quantifier: peel the whole arrow chain `A → B → … → C → C` and drop the
/// trailing continuation variables, leaving `[A, B, …]` — the conjuncts the quantifier asserts.
///
/// Taking only the FIRST antecedent silently DROPS the rest, and on this corpus that lost an entire
/// comparative: "MSI cell lines … showed greater dependence on WRN than their MSS counterparts."
/// reads back as `poss_of(…) → gt(…) → G#0 → G#0`, and rendering just `poss_of` gave the stub
/// "some SIL1 gene counterpart, its it" — the `gt` comparison, which is the whole claim, vanished.
fn cps_body_parts(e: &Exp) -> Vec<&Exp> {
    let mut parts = Vec::new();
    let mut cur = e;
    while let Some((a, b)) = as_arrow(cur) {
        parts.push(a);
        cur = b;
    }
    while matches!(parts.last(), Some(Exp::Var(_))) {
        parts.pop();
    }
    parts
}

/// A function type `A → B`, however it reads back — the explicit `Exp::Arrow` or the non-dependent
/// `Pi(Patt::Unit, A, B)` (readback uses the latter for `→`).
fn as_arrow(e: &Exp) -> Option<(&Exp, &Exp)> {
    match e {
        Exp::Arrow(a, b) => Some((a.as_ref(), b.as_ref())),
        Exp::Pi(Patt::Unit, a, b) => Some((a.as_ref(), b.as_ref())),
        _ => None,
    }
}

/// The `lexicon:anaphor` placeholder — verbalizes as "it" (the entity the NP already names).
fn anaphor_atom() -> Exp {
    Exp::EigonAxiom(Iri::parse("urn:eigenius:lexicon:anaphor").expect("anaphor iri"))
}

/// Replace the free variable `name` with `to` throughout `e` (glossing the bound quantifier entity).
fn subst_var(e: &Exp, name: &str, to: &Exp) -> Exp {
    let go = |x: &Exp| subst_var(x, name, to);
    match e {
        Exp::Var(v) if v == name => to.clone(),
        Exp::App(f, x) => Exp::App(Box::new(go(f)), Box::new(go(x))),
        Exp::Lam(p, b) => Exp::Lam(p.clone(), Box::new(go(b))),
        Exp::Pi(p, a, b) => Exp::Pi(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Sig(p, a, b) => Exp::Sig(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Arrow(a, b) => Exp::Arrow(Box::new(go(a)), Box::new(go(b))),
        Exp::Times(a, b) => Exp::Times(Box::new(go(a)), Box::new(go(b))),
        Exp::Fst(x) => Exp::Fst(Box::new(go(x))),
        Exp::Snd(x) => Exp::Snd(Box::new(go(x))),
        Exp::Pair(a, b) => Exp::Pair(Box::new(go(a)), Box::new(go(b))),
        Exp::Ann(x, t) => Exp::Ann(Box::new(go(x)), Box::new(go(t))),
        Exp::InductiveType(d, args) => Exp::InductiveType(d.clone(), args.iter().map(go).collect()),
        Exp::InductiveCtor(d, n, args) => {
            Exp::InductiveCtor(d.clone(), n.clone(), args.iter().map(go).collect())
        }
        other => other.clone(),
    }
}

/// "a" / "an" for the following word (vowel-initial → "an").
fn article(word: &str) -> &'static str {
    match word.chars().next() {
        Some(c) if "aeiou".contains(c.to_ascii_lowercase()) => "an",
        _ => "a",
    }
}

/// "a NP" / "an NP" for a bare kind / class argument (a Σ already supplies its own article).
fn indefinite(e: &Exp, vb: &Vb) -> String {
    match e {
        Exp::Sig(..) => verbalize(e, vb),
        _ => {
            let w = verbalize(e, vb);
            format!("{} {w}", article(&w))
        }
    }
}

/// The NP text without a leading article (for `the …`).
fn bare_np(e: &Exp, vb: &Vb) -> String {
    if let Exp::Sig(_, base, restr) = e {
        return noun_phrase(base, restr, vb);
    }
    verbalize(e, vb)
}

/// "adjs compound-mods HEAD pps" from a Σ's base type and restrictor conjuncts.
fn noun_phrase(base: &Exp, restr: &Exp, vb: &Vb) -> String {
    let head = verbalize(base, vb);
    let (mut pre, mut post): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let mut conj = Vec::new();
    flatten_and_exp(restr, &mut conj);
    for c in conj {
        let (h, a) = app_spine(c);
        match axiom_local(h) {
            // A compound modifier is a bare noun ("nucleotide-repeat"), not "a nucleotide-repeat".
            Some("compound_kind" | "compound") if a.len() == 2 => pre.push(bare_np(a[1], vb)),
            Some("gt" | "lt") => {
                if let Some(first) = a.first() {
                    let (dh, _) = app_spine(first);
                    if let Some(dl) = axiom_local(dh) {
                        pre.push(name_atom(dl, vb));
                    }
                }
            }
            Some(p) if p.starts_with("prep_") => {
                if let Some(x) = a.get(1) {
                    post.push(format!("{} {}", &p[5..], verbalize(x, vb)));
                }
            }
            Some("is_a") if a.len() == 2 => post.push(format!("that is {}", indefinite(a[1], vb))),
            Some("named") if a.len() == 2 => post.push(format!("named {}", verbalize(a[1], vb))),
            // A possessive restrictor — `Σx:N. poss_of(N, x, owner)`, "their MSS counterparts".
            Some("poss_of") if a.len() == 2 || a.len() == 3 => {
                let owner = verbalize(a[a.len() - 1], vb);
                pre.push(if owner.is_empty() {
                    "its".to_string()
                } else {
                    format!("{owner}'s")
                });
            }
            // A restrictor headed by the Σ's OWN BOUND VARIABLE — `G#0(C1337007)`. This is the
            // clausal complement's predicate slot ("the finding that … WRN"): the abstracted
            // predicate applied to its argument. A bare `Var` carries no surface, so render the
            // ARGUMENTS. `about` is a gloss for the predication, in the same spirit as the `that
            // is` / `named` arms above — it names the participant without claiming the relation.
            // Without this every `that`-complement unit bracketed its entire embedded clause.
            None if matches!(h, Exp::Var(_)) && !a.is_empty() => {
                let inner: Vec<String> = a
                    .iter()
                    .map(|x| verbalize(x, vb))
                    .filter(|x| !x.is_empty())
                    .collect();
                post.push(format!("about {}", inner.join(" ")));
            }
            // Anything else: hand it to `verbalize` rather than bracketing it here. An embedded GQ
            // restrictor (`Π… prep_of …`, "of a DNA repair pathway") is perfectly renderable by the
            // quantifier arms — bracketing it at this level threw that away. `verbalize` still
            // brackets what IT cannot render, so the "never silently dropped" property is kept.
            _ => post.push(verbalize(c, vb)),
        }
    }
    let mut s = String::new();
    for m in pre.iter().filter(|m| !m.is_empty()) {
        s.push_str(m);
        s.push(' ');
    }
    s.push_str(&head);
    for m in post.iter().filter(|m| !m.is_empty()) {
        s.push(' ');
        s.push_str(m);
    }
    s.trim().to_string()
}

fn flatten_and_exp<'a>(e: &'a Exp, out: &mut Vec<&'a Exp>) {
    if let Exp::InductiveType(decl, args) = e {
        if decl.iri.as_str().ends_with("logic:And") && args.len() == 2 {
            flatten_and_exp(&args[0], out);
            flatten_and_exp(&args[1], out);
            return;
        }
    }
    out.push(e);
}

/// A classified unit's distinct-skeleton set — the closed readings for Encoded/Ambiguous, the
/// parametric (open) readings for Open (each a self-contained `Π`-abstraction, so certifiable), empty
/// for the truly reading-less outcomes (GrammarGap/MissingLexeme/ScaleBound).
fn unit_skel_set(o: &Outcome) -> &[String] {
    match o {
        Outcome::Encoded { skeletons, .. }
        | Outcome::Ambiguous { skeletons, .. }
        | Outcome::Open { skeletons, .. } => skeletons,
        _ => &[],
    }
}

/// The committed expected-reading corpus: for a curated subset of units, the sense-erased skeleton of
/// the reading a human has verified is CORRECT. The faithfulness gate asserts each such unit still
/// CONTAINS that skeleton among its readings — robust to added ambiguity (a unit going ENCODED→AMBIG
/// while keeping the right reading is NOT a regression), unlike encoded-count. Path is repo-relative;
/// missing file ⇒ empty (gate inactive). See `experiments/parsing/README.md` §7c.
/// TAB-separated: `sentence <TAB> skeleton <TAB> note`. TSV (not JSON) to avoid a serde_json dep for
/// one small file; skeletons and sentences contain no tabs. Blank lines and `#` comments are skipped.
const EXPECTED_READINGS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../experiments/parsing/expected-readings.tsv"
);

/// One curated expectation: the sentence, the correct reading's skeleton, and why it is correct.
struct Expected {
    sentence: String,
    skeleton: String,
    note: String,
}

fn load_expected_readings() -> Vec<Expected> {
    let Ok(text) = std::fs::read_to_string(EXPECTED_READINGS) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut f = line.split('\t');
        let (Some(sentence), Some(skeleton)) = (f.next(), f.next()) else {
            panic!("expected-readings.tsv: line needs sentence<TAB>skeleton: {line:?}");
        };
        out.push(Expected {
            sentence: sentence.trim().to_string(),
            skeleton: skeleton.trim().to_string(),
            note: f.next().unwrap_or("").trim().to_string(),
        });
    }
    out
}

/// The distinct-skeleton COUNT of a classified unit — Encoded/Ambiguous (closed) and Open (parametric)
/// carry structural skeletons; GrammarGap/MissingLexeme/ScaleBound produce none (0).
fn unit_skeletons(o: &Outcome) -> usize {
    match o {
        Outcome::Encoded { skeletons, .. }
        | Outcome::Ambiguous { skeletons, .. }
        | Outcome::Open { skeletons, .. } => skeletons.len(),
        _ => 0,
    }
}

/// **PINNED** reading-count histogram buckets `(label, lo, hi)` inclusive — the single source of
/// truth for the multiplicity distribution, so the buckets do NOT drift between runs. Change here
/// only, deliberately (a re-baseline event), never per-run.
const READING_BUCKETS: &[(&str, usize, usize)] = &[
    ("0 (open/gap)", 0, 0),
    ("1 (encoded)", 1, 1),
    ("2-3", 2, 3),
    ("4-10", 4, 10),
    ("11-30", 11, 30),
    ("31-100", 31, 100),
    (">100", 101, usize::MAX),
];

/// Classify one unit. **OOV-first ordering** (vs. the slice prototype's parse-first): a closed
/// full-span parse requires every (prose) token to seed a leaf, so a unit with any unknown token
/// cannot encode — diagnose it as MissingLexeme from the cheap `has_token` probes *without* running
/// CKY. Only a fully-known unit is parsed (the parse is needed only to tell Encoded / Ambiguous /
/// GrammarGap apart). This is both correct and what keeps the FULL-lexicon run tractable: an
/// OOV-heavy long unit would otherwise OOM the chart on the dense WordNet+UMLS seed set, and the
/// parse there is guaranteed-empty wasted work. (Edge: a unit whose only unknown single-tokens are
/// all subsumed by *multiword* entries that do seed, and which fully parses, would be bucketed
/// MISSING rather than ENCODED — measure-zero for this corpus, and the OOV signal is still right.)
fn encode_unit(text: &str, index: &Parser, lem: &dyn Lemmatizer, layer: &Arc<Layer>) -> Outcome {
    let toks = tokenize(text);
    let unknown: Vec<String> = toks
        .iter()
        .filter(|t| !is_nonprose(t) && !index.has_token(t, lem))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Outcome::MissingLexeme { unknown };
    }
    // Fully known. Bound the (beam-less) parse so a long known unit doesn't OOM the chart.
    if toks.len() > PARSE_BUDGET {
        return Outcome::ScaleBound { ntok: toks.len() };
    }
    // Parse to distinguish the fully-known outcomes. Use the open-parse carrier so a unit that only
    // yields an OPEN parse (referent holes from `we`/`its`/pronouns, D64) is NOT misfiled as a
    // grammar gap — it parses, awaiting reference resolution.
    let (closed, open) = index.parse_open(text, lem);
    match closed.len() {
        0 => {
            if open.is_empty() {
                Outcome::GrammarGap
            } else {
                let open_items: Vec<Item> = open.iter().map(|o| o.item.clone()).collect();
                Outcome::Open {
                    holes: open.iter().map(|o| o.holes.len()).max().unwrap_or(0),
                    skeletons: skeleton_set(&open_items),
                }
            }
        }
        1 => Outcome::Encoded {
            is_prop: gates_to_prop(layer, closed[0].sem()),
            skeletons: skeleton_set(&closed),
        },
        n => Outcome::Ambiguous {
            count: n,
            is_prop: gates_to_prop(layer, closed[0].sem()),
            skeletons: skeleton_set(&closed),
        },
    }
}

/// VERIFY the sense lever (D62/GH#97): A/B the PAGE-beam (64) parse outcome for the 5 sentences
/// with the static cap (`baseline`) vs the contextual LLM reranker (`+llm`, only with
/// `--features use-llm` + ANTHROPIC_API_KEY). Measures whether contextual sense ranking frees enough
/// beam to parse at the operational beam. (The deterministic "closed-class-wins" filter was tried
/// and REVERTED — harmful; it can't distinguish `be`-verb from beryllium — see the d63 note.)
///   cargo test -p eigenius-wordnet --features use-llm --test db_backed_encoding \
///       verify_sense_lever_at_page_beam -- --ignored --nocapture
///
/// Beam-sensitivity (Lever 2, GH#97, measured 2026-06-30): at a fixed cell beam the 5
/// grammar-complete sentences cross to parsing at — S2 b64, S3 b128, S1/S5 b256, S4 not even at
/// b1024 (needs structural reduction). That measurement motivated **beam widen-on-failure**
/// (`CELL_BEAM_WIDEN_MAX`): `parse_scoped_open` now escalates the beam (with the sense cap) for a
/// known sentence that gaps, so the base beam stays the long-sentence OOM defense while
/// beam-limited short sentences are recovered. (So a fixed-beam sweep is no longer meaningful here —
/// `parse_scoped_open` auto-widens.)
#[test]
#[ignore = "diagnostic: A/B the sense lever at the page beam; run with --ignored --nocapture"]
fn verify_sense_lever_at_page_beam() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let sentences = [
        "Synthetic lethality is an interaction between two genetic events.",
        "The co-occurrence of these two events leads to cell death.",
        "Each event alone does not lead to cell death.",
        "Scientists can exploit synthetic lethality for cancer therapeutics.",
        "DNA repair processes are attractive synthetic lethal targets.",
    ];
    let outcome = |idx: &Parser, s: &str| -> String {
        let (c, o) = idx.parse_open(s, &lem);
        if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        }
    };
    let mk = || {
        Parser::build(Arc::clone(&head))
            .with_sense_cap(SENSE_CAP)
            .with_cell_beam(CELL_BEAM)
    };

    // The variants to compare. The LLM variant only exists with `--features use-llm` +
    // ANTHROPIC_API_KEY (one reranker call per sentence).
    #[allow(unused_mut)]
    let mut variants: Vec<(String, Parser)> = vec![("baseline".into(), mk())];
    #[cfg(feature = "use-llm")]
    {
        if let Some(r) = eigenius_kernel::dcg::AnthropicSenseRanker::from_env() {
            variants.push(("+llm".into(), mk().with_sense_ranker(Box::new(r))));
        }
    }

    eprintln!("\n=== sense-lever A/B at PAGE beam ({CELL_BEAM}) ===");
    eprintln!(
        "variants: {:?}",
        variants.iter().map(|(l, _)| l).collect::<Vec<_>>()
    );
    for s in sentences {
        let cells: Vec<String> = variants
            .iter()
            .map(|(l, idx)| format!("{l}={}", outcome(idx, s)))
            .collect();
        eprintln!("  {}  {s:?}", cells.join("  "));
    }
}

/// FALSE-IDENTITY probe (ledger: 14 `invalid` rows, the predicative kind-raise `ff1690f`). The bad
/// readings assert `is_a(X, K)` where X cannot be K — e.g. «We ascertained MSI status with sequencing»
/// yields `is_a(speaker, Σ…)` with the verb absorbed into the nominal and NO `ascertain` relation.
///
/// Three slash-mode assignments were measured against this family and ALL failed to touch it
/// (`m_app` on verb+adj governed slashes cost 3 pins; `m_harm` and adj-only `m_app` were inert), so
/// the reading is not built by composing a governed PP away. This prints each root reading's
/// top-level COMBINATOR next to its sem, to name the rule that actually builds it.
///
///   EIGENIUS_DB_SNAPSHOT=/path cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       probe_false_identity_provenance -- --ignored --nocapture
#[test]
#[ignore = "probe: which combinator builds the false-identity is_a; --ignored --nocapture"]
fn probe_false_identity_provenance() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        // no copula, yet a predicative `is_a` reaches the root — the sharpest case
        "We ascertained MSI status with sequencing.",
        // gloss-governed adjective, PP stranded
        "These classifications were highly concordant with PCR-based MSI phenotyping.",
        // predicate nominal with a PP complement, `target` NOT in adjective-frames.tsv
        "These findings show that WRN is a promising drug target for MSI cancers.",
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        eprintln!(
            "\n=== {s:?} — {} closed, {} open ===",
            closed.len(),
            open.len()
        );
        use eigenius_kernel::dcg::category::pretty_cat_dbg;
        let mut rows: Vec<(String, String)> = closed
            .iter()
            .map(|it| {
                (
                    // The ROOT CATEGORY matters as much as the combinator: a predicate-nominal
                    // `is_a` needs a copula, and `is_finite_clause` admits a root only at
                    // `fin`/`fin_any` — so a copula-less `is_a` root means some entry is handing
                    // back a FINITE clause where an `adj`-featured one was expected.
                    format!("{:?} :: {}", it.prov(), pretty_cat_dbg(it.cat())),
                    pretty_term(it.sem()),
                )
            })
            .collect();
        rows.sort();
        rows.dedup();
        // Count how many roots assert a top-level identity, and by which combinator.
        let mut by_prov: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
        for (prov, sem) in &rows {
            let e = by_prov.entry(prov.clone()).or_default();
            e.0 += 1;
            if sem.contains("is_a(") {
                e.1 += 1;
            }
        }
        for (prov, (total, with_is_a)) in &by_prov {
            eprintln!("  {prov:<28} readings {total:>4}   containing is_a {with_is_a:>4}");
        }
        for (prov, sem) in rows.iter().filter(|(_, s)| s.contains("is_a(")).take(3) {
            eprintln!("    [{prov}] {}", &sem[..sem.len().min(200)]);
        }
    }
}

/// D63 compound-morphology §2a diagnostic: show *exactly* how `based on X` parses TODAY (before the
/// Step 2b object+PP extension) — the adjective(`based`, data.adj) + `on`-adjunct reading, NOT the
/// verb-argument `base(x, X)`. Dumps every distinct closed sem. Run with:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       show_based_on_x_reading -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: show the today `based on X` adjective+adjunct reading; --ignored --nocapture"]
fn show_based_on_x_reading() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        "Cells are based on genes.",
        "The method is based on sequencing.",
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        eprintln!(
            "\n=== {s:?} — {} closed, {} open ===",
            closed.len(),
            open.len()
        );
        let mut sems: Vec<String> = closed.iter().map(|it| pretty_term(it.sem())).collect();
        sems.sort();
        sems.dedup();
        for (i, sem) in sems.iter().enumerate() {
            eprintln!("  [{i}] {sem}");
        }
    }
}

/// D63 §8 C4 milestone: verify #8 (degree comparatives) parses AT SCALE — i.e. against the
/// WordNet-derived `dependence`/`dependent`/`sensitive` entries emitted by the importer (C1 bare
/// cat_measure, C2 nominalization projection, C3 relational/governed-prep reading), NOT the
/// hand-authored demo. The closed-class operators (`greater`/`more`/`less`, `than`) come from the
/// seeded `closed-class.esl`. Expected: `greater dependence on Y` and `more dependent on Y than Z`
/// produce `gt(deg_dependent(_,_), deg_dependent(_,_))`; `more sensitive than Z` the same over
/// `deg_sensitive`. #9 cardinality (`fewer genes`) is re-probed as a regression.
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       verify_degree_comparative_at_scale -- --ignored --nocapture
/// RC-8 (d63-parse-gap-closure §Phase-2 backlog) — the sentence-2 shape `… is not simply a result of
/// …` over the real WordNet lexicon. Every grammar piece closes in the demo (copula + predicate
/// nominal + of-PP + negation + clausal complement), so isolate whether the residual is the ADVERB
/// `simply` (modifying a predicate nominal) or lexical/scale, with and without it.
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_rc8_at_scale -- --ignored --nocapture
#[test]
#[ignore = "probe: RC-8 `is not simply a result of` at scale; --ignored --nocapture"]
fn probe_rc8_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        "genes are a result of mutations",     // predicate nominal + of-PP
        "genes are not a result of mutations", // + negation
        "genes are not simply a result of mutations", // + adverb `simply` (the s2 embedded clause)
        "cells suggest that genes are a result of mutations", // clausal + predicate nominal
        "cells suggest that genes are not simply a result of mutations", // full s2 shape
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("open×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  {tag:<10} {s:?}");
    }
}

/// FAITHFUL s20 isolation — the corpus sentence `WRN dependency may require specific lineages or a
/// stronger mutation phenotype` STILL gaps in the fresh-store measure despite the attributive-comparative
/// and coordination fixes (verified only on the SIMPLER demo proxy `HeLa may affect a gene or a larger
/// cell line`). Isolate which of the FULL structure — compound subject / adj+bare-plural coordinand /
/// compound-noun-in-comparative — actually gaps, over the real lexicon (WordNet words; WRN→gene proxy).
///
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_s20_isolation_at_scale -- --ignored --nocapture
#[test]
#[ignore = "probe: faithful s20 full-structure isolation at scale; --ignored --nocapture"]
fn probe_s20_isolation_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let outcome = |idx: &Parser, s: &str| -> String {
        let (c, o) = idx.parse_open(s, &lem);
        if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        }
    };
    // #2 verification on the --umls-all reseed (UMLS process/function-TUI mass fix): methylation
    // (C0025723, T044) / hypermethylation are now in-vocab AND mass, so bare `from methylation` should
    // CLOSE (was GAP). The full corpus sentence either closes (7→6) or reveals a residual search limit.
    let idx = build_index(&head);
    for w in ["methylation", "hypermethylation", "methylate"] {
        eprintln!("  has_token({w:?}) = {}", idx.has_token(w, &lem));
    }
    for (tag, s) in [
        ("#2 min-methyl", "inactivation arises from methylation"), //     was GAP → expect CLOSED
        ("#2 min-hyper", "inactivation arises from hypermethylation"), // CLOSED if its TUI is process/function
        (
            "#2 corpus-methyl",
            "Somatic MMR inactivation typically arises from methylation of the MLH1 promoter",
        ),
        (
            "#2 corpus-hyper",
            "Somatic MMR inactivation typically arises from hypermethylation of the MLH1 promoter",
        ), // the actual corpus #2
    ] {
        eprintln!("  {tag:<16} {:<10} {s:?}", outcome(&idx, s));
    }
    // Grammar vs search for the corpus #2 sentence: cap8/beam512, static rank.
    let hi = Parser::build(Arc::clone(&head))
        .with_sense_cap(8)
        .with_cell_beam(512);
    let c = "Somatic MMR inactivation typically arises from hypermethylation of the MLH1 promoter";
    eprintln!("  #2 corpus@cap8   {:<10} {c:?}", outcome(&hi, c));
}

#[test]
#[ignore = "diagnostic: #8 degree comparatives against the WordNet lexicon; --ignored --nocapture"]
fn verify_degree_comparative_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // #8 all-WordNet frames (bare-plural NPs sidestep domain-entity grounding), plus the demo frame
    // at scale, plus the #9 regression. `sensitive` is WordNet-only (absent from the demo lexicon).
    let sentences = [
        "cells show greater dependence on genes than mutations", // #8 nominalization + governed prep
        "cells are more dependent on genes than mutations",      // #8 predicative adjective
        "cells are more sensitive than mutations", // #8 predicative degree (WN-only adj)
        "HeLa affects greater dependence on BRCA1 than MSH2", // #8 demo frame, domain entities
        "HeLa affects fewer genes than MSH2",      // #9 cardinality regression
    ];
    for s in sentences {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("open×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("\n=== {tag}  {s:?} ===");
        let mut sems: Vec<(String, bool)> = closed
            .iter()
            .map(|it| (pretty_term(it.sem()), gates_to_prop(&head, it.sem())))
            .collect();
        sems.sort();
        sems.dedup();
        for (i, (sem, is_prop)) in sems.iter().enumerate() {
            eprintln!("  [{i}]{} {sem}", if *is_prop { " ⊨Prop" } else { "" });
        }
    }
}

/// D63 §5.3 C3-precision — the AT-SCALE witness: on the real WordNet lexicon, `dependent`'s gloss
/// governs `on`, so the importer emits `cat_measure / cat_pp_arg(prep_on)`; the WRONG preposition is
/// rejected at the feature-meet. The two sentences differ ONLY in the preposition. Unlike the unit
/// test (hand-authored demo entry), this proves the govern-prep detection + prep-tagged emission
/// survive the full-lexicon importer path. ASSERTS (skips cleanly when no snapshot / ManifestDrift).
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       verify_governed_preposition_at_scale -- --ignored --nocapture
#[test]
#[ignore = "diagnostic+witness: C3-precision rejects *dependent to at scale; --ignored --nocapture"]
fn verify_governed_preposition_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // The witness is on the RELATIONAL reading, not full-sentence closure: at scale `dependent` also
    // has a bare `cat_measure` (C1) + a count-noun reading, which close the sentence regardless of the
    // preposition. C3-precision's claim is narrower — the ground-taking `deg_..._rel(ground, subject)`
    // term (built only through `cat_measure/cat_pp_arg(prep)`) must appear with the GOVERNED prep and
    // be ABSENT with the wrong one.
    let rel_terms = |s: &str| -> Vec<String> {
        let (c, _) = index.parse_open(s, &lem);
        let mut rels: Vec<String> = c
            .iter()
            .map(|it| pretty_term(it.sem()))
            .filter(|t| t.contains("_rel("))
            .collect();
        rels.sort();
        rels.dedup();
        eprintln!(
            "\n=== {s:?} — {} closed, {} relational ===",
            c.len(),
            rels.len()
        );
        for (i, t) in rels.iter().enumerate() {
            eprintln!("  rel[{i}] {t}");
        }
        rels
    };
    let on_rel = rel_terms("cells are more dependent on genes than mutations");
    let to_rel = rel_terms("cells are more dependent to genes than mutations");

    assert!(
        !on_rel.is_empty(),
        "`more dependent ON genes` must yield the relational deg_rel reading (prep_on marker meets the \
         importer-emitted governed prep_on)"
    );
    assert!(
        to_rel.is_empty(),
        "C3-precision: `*more dependent TO genes` must yield NO relational deg_rel reading — `dependent` \
         governs `on`, so cat_pp_arg(prep_to) fails the feature-meet. (Bare-measure / noun readings may \
         still close the sentence; the gate is on the relational term.) got: {to_rel:?}"
    );
}

/// D63 §8.5 / d63-comparative-phrasal §8 — AT-SCALE witness: an attributive comparative (`a stronger
/// gene`, s20's `a stronger mutation phenotype`) parses OPEN with a comparison-standard hole on the real
/// WordNet lexicon (the importer's `cmp_attrib_sem` bare `S[adj]\NP` reading). Was a grammar-GAP before.
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       verify_attributive_comparative_at_scale -- --ignored --nocapture
#[test]
#[ignore = "diagnostic+witness: attributive comparative opens with a standard hole at scale; --ignored --nocapture"]
fn verify_attributive_comparative_at_scale() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        "cells affect a stronger gene",
        "cells require a stronger phenotype",
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        // The attributive-comparative reading is an OPEN parse (a comparison-standard hole) whose sem
        // compares a degree: `gt(deg_X(x), deg_X($anaphor$))`.
        let attrib = open.iter().find(|o| {
            !o.holes.is_empty() && {
                let t = pretty_term(o.item.sem());
                t.contains("gt(") && t.contains("deg_")
            }
        });
        eprintln!(
            "\n=== {s:?} — {} closed, {} open ===",
            closed.len(),
            open.len()
        );
        if let Some(o) = attrib {
            eprintln!(
                "  attributive-comparative OPEN (holes={}): {}",
                o.holes.len(),
                pretty_term(o.item.sem())
            );
        }
        assert!(
            attrib.is_some(),
            "`{s}` must have an OPEN attributive-comparative reading (gt(deg(x),deg(anaphor)) + hole) at scale"
        );
    }
}

/// D63 lexicon-augmentation diagnostic: are the UMLS `RecQ` atoms (C0084304 "RecQ Helicases") seeded as
/// `lexicon:form` entries in the snapshot? If so, a `TextIndex` over `lexicon:form` (BM25/token) would
/// ground the OOV surface `recq` → those atoms → the concept — without an HGNC import. The exact
/// `ValueIndex` misses them (`recq` ≠ `recq helicases`), which is why `recq` is OOV today. Run with:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_recq_atoms_in_snapshot -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: are RecQ atoms seeded (form-text-index grounding path)? --ignored --nocapture"]
fn probe_recq_atoms_in_snapshot() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for form in [
        "recq",
        "recq helicase",
        "recq helicases",
        "helicase, recq",
        "recq protein",
        "recq family of dna helicases",
        "recq helicase-like",
    ] {
        let known = index.has_token(form, &lem);
        let entries = index.debug_form_entries(form, &lem);
        eprintln!(
            "\n=== {form:?} — has_token={known}, {} entries ===",
            entries.len()
        );
        for (closed, cat, sense) in entries.iter().take(10) {
            eprintln!("  closed={closed}  sense={sense}  cat={cat}");
        }
    }
}

/// D2 (nominal-modification NF, d63-nominal-modification-normal-form.md §4/§8): does the snapshot carry
/// the corpus's genuine collocations as LEXICAL UNITS? A form with a `cat_n`/`cat_np` entry + a sense
/// (a `wn:`/`umlscui:` id) seeds as a multi-token span, so its compound reading is a leaf — not a
/// bracketing the compound rule reconstructs. Absent = the NF forces the all-adjective tree on it (the
/// coverage-policy decision D2). Run:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       d2_collocation_coverage -- --ignored --nocapture
#[test]
#[ignore = "D2: collocation-as-lexical-unit coverage over the snapshot; --ignored --nocapture"]
fn d2_collocation_coverage() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // Corpus collocations (space-joined lowercase, as `by_form` keys). The adjective-position
    // `synthetic lethal` is THE one the NF's interleaving hinges on; the rest are the first-5 CNL
    // compounds. `cell`/`lethality` are sanity controls (known heads).
    for form in [
        "synthetic lethality",
        "synthetic lethal",
        "synthetic lethal target",
        "synthetic lethal targets",
        "cell death",
        "dna repair",
        "repair process",
        "repair processes",
        "dna repair process",
        "dna repair processes",
        "cancer therapeutics",
        "genetic event",
        "genetic events",
        "co-occurrence",
        // controls:
        "cell",
        "lethality",
    ] {
        let known = index.has_token(form, &lem);
        let entries = index.debug_form_entries(form, &lem);
        // A collocation counts as a UNIT iff some entry is a nominal category carrying a sense id.
        let unit = entries.iter().any(|(_c, cat, sense)| {
            !sense.is_empty() && (cat.contains("cat_n") || cat.contains("cat_np"))
        });
        eprintln!(
            "\n=== {form:?} — has_token={known}  UNIT={unit}  {} entries ===",
            entries.len()
        );
        for (closed, cat, sense) in entries.iter().take(8) {
            eprintln!("  closed={closed}  sense={sense}  cat={cat}");
        }
    }
}

/// STEP 0 of the compound-pile plan (d63-compound-pile-collapse-plan.md): localize WHICH domain compound
/// tips each residual sentence over, and get its ROUTING (packed vs unpacked) — the fork that decides
/// Lever 1 (extend packing) vs Lever 2 (collapse structure). Bounded frames (one domain compound swapped
/// into a parseable generic base at a time; NO full sentence / double-swaps → avoids the OOM). Cap-only.
/// Run: cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       diagnose_compound_pile -- --ignored --nocapture
#[test]
#[ignore = "STEP 0: localize the exploding domain compound + its routing; --ignored --nocapture"]
fn diagnose_compound_pile() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // ROUTING-ONLY (fast: routes_packed does NOT parse; parsing the domain frames explodes/OOMs). The
    // fork — packed vs unpacked — is the Step-0 answer that picks Lever 1 (extend packing) vs Lever 2.
    let row = |idx: &Parser, s: &str| {
        let toks = tokenize(s);
        let unk: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !idx.has_token(t, &lem))
            .cloned()
            .collect();
        let routed = if idx.routes_packed(s, &lem) {
            "PACKED"
        } else {
            "UNPACK"
        };
        let oov = if unk.is_empty() {
            String::new()
        } else {
            format!("  OOV {unk:?}")
        };
        eprintln!("   [{routed}]{oov} {s:?}");
    };
    // (label, base generic frame, [one-domain-compound-swap frames])
    let groups: &[(&str, &str, &[&str])] = &[
        (
            "#7 — swap one domain compound into the generic base (×162)",
            "cells from lineages showed greater dependence on genes than counterparts",
            &[
                "MSI cell lines from lineages showed greater dependence on genes than counterparts", // subj compound
                "cells from these four lineages showed greater dependence on genes than counterparts", // from-PP
                "cells from lineages showed greater dependence on WRN than counterparts",  // obj (named indiv)
                "cells from lineages showed greater dependence on genes than their MSS counterparts", // than-obj
            ],
        ),
        (
            "#4 — swap one domain compound into the generic base (×121)",
            "we identified genes as a dependency in cells compared to lines",
            &[
                "we identified WRN as a dependency in cells compared to lines", // obj (named indiv)
                "we identified genes as the top preferential dependency in cells compared to lines", // as-complement
                "we identified genes as a dependency in MSI cell lines compared to lines", // in-PP compound
                "we identified genes as a dependency in cells compared to MSS cell lines", // compared-to compound
            ],
        ),
        (
            "#3 — swap one domain compound into the generic base (×6)",
            "some lines and some lines were represented by data sets",
            &[
                "some MSI lines and some MSS lines were represented by data sets", // coord subj compounds
                "some lines and some lines were represented by these screening data sets", // agent compound
            ],
        ),
    ];
    for (label, base, swaps) in groups {
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("{label}");
        row(&index, base);
        for s in *swaps {
            row(&index, s);
        }
    }
    // TRIGGER LOCALIZATION: which construct forces unpacked? (baselines that SHOULD pack + one construct)
    eprintln!("\n════════════════════════════════════════════════════════════════");
    eprintln!("TRIGGER (expect PACKED baselines; UNPACK isolates the culprit construct)");
    for s in [
        "genes affect cells",                                // SVO baseline
        "genes are large",                                   // copula baseline
        "genes are attractive targets",                      // adj + compound baseline
        "cells showed dependence on genes", // relational noun + governed-prep PP (no comparative)
        "cells showed greater dependence than counterparts", // #7 comparative
        "cells are larger than genes",      // bare comparative-than
        "lines were represented by sets",   // #3 passive
        "we identified genes as a dependency", // #4 V-as-Y
        "genes affect cells compared to lines", // 'compared to' adjunct
    ] {
        row(&index, s);
    }
}

/// RE-ASSESS the 3 residual reranked gaps (#3 passive, #4 V-as-Y+compared-to, #7 comparative+PP): for
/// each, walk a fragment ladder (isolate the construction with generic fillers) at the DEFAULT beam,
/// then parse the full sentence at DEFAULT vs WIDE (cell_beam=1024). The verdict per sentence:
///
///   - construction parses in a fragment but full sentence GAPs at default, parses at WIDE ⇒ SEARCH-limited
///     (beam pressure), and the fragment where it first breaks localizes the driver;
///   - gaps even at WIDE ⇒ a real composition gap (grammar / missing rule), NOT beam pressure.
///
/// Cap-only. Run:
///
///   cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       diagnose_residual_gaps -- --ignored --nocapture
#[test]
#[ignore = "re-assess the 3 residual gaps (search vs grammar, per sentence); --ignored --nocapture"]
fn diagnose_residual_gaps() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    let outcome = |c: usize, o: usize| -> String {
        if c > 0 {
            format!("CLOSED×{c}")
        } else if o > 0 {
            format!("open×{o}")
        } else {
            "GRAMMAR-GAP".into()
        }
    };
    let probe = |idx: &Parser, s: &str| -> String {
        let toks = tokenize(s);
        let unk: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !idx.has_token(t, &lem))
            .cloned()
            .collect();
        if !unk.is_empty() {
            return format!("OOV {unk:?}");
        }
        let (c, o) = idx.parse_open(s, &lem);
        outcome(c.len(), o.len())
    };

    // (label, ladder fragments [default beam], full sentence [default + WIDE])
    let groups: &[(&str, &[&str], &str)] = &[
        (
            "#7 COMPARATIVE + PP (greater … on … than …)",
            &[
                "cells showed greater dependence than counterparts", // comparative alone
                "cells showed greater dependence on genes than counterparts", // + on-PP (governed)
                "cells from lineages showed greater dependence on genes than counterparts", // + subj from-PP
            ],
            "MSI cell lines from these four lineages showed greater dependence on WRN than their MSS counterparts.",
        ),
        (
            "#4 V-as-Y + in-PP + compared-to",
            &[
                "we identified genes as a dependency", // V-as-Y alone
                "we identified genes as a dependency in cells", // + in-PP
                "we identified genes as a dependency compared to cells", // + compared-to
                "we identified genes as a dependency in cells compared to lines", // both PPs
            ],
            "Project Achilles and project DRIVE identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines.",
        ),
        (
            "#3 PASSIVE + coordinated subject + complex agent",
            &[
                "lines were represented by sets",          // passive, minimal
                "lines were represented by data sets",     // + compound agent
                "some lines were represented by data sets", // + some-det
                "some lines and some lines were represented by data sets", // + coordinated subject
            ],
            "Some MSI lines and some MSS lines were represented by these screening data sets.",
        ),
    ];

    for (label, ladder, full) in groups {
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("{label}");
        for f in *ladder {
            eprintln!("   [default] {:<12} {f:?}", probe(&index, f));
        }
        eprintln!("   ── full sentence ──");
        eprintln!("   [default] {:<12} {full:?}", probe(&index, full));
    }
}

#[test]
#[ignore = "TEMP dump of as/a/the/identified categories; --ignored --nocapture"]
fn dump_as_cats() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        "Synthetic lethality is an interaction between two genetic events.",
        "The co-occurrence of these two events leads to cell death.",
        "Each event alone does not lead to cell death.",
        "Scientists can exploit synthetic lethality for cancer therapeutics.",
        "DNA repair processes are attractive synthetic-lethal targets.",
        "Many cancers exhibit an impairment of a DNA repair pathway.",
        "This impairment can lead to dependence on specific repair proteins.",
    ] {
        let (c, o) = index.parse_open(s, &lem);
        eprintln!("\n{s:?}: closed={} open={}", c.len(), o.len());
        for it in c.iter().take(1) {
            eprintln!("   {}", pretty_term(it.sem()));
        }
    }
}

/// ISOLATE the #4 "Project Achilles …" residual: start from the generic base that closes
/// (`we identified genes as a dependency in cells compared to lines`, CLOSED×112) and swap ONE domain
/// feature back in at a time — coordinated named subject, named object, superlative as-complement, and
/// each domain-compound PP — then a cumulative build-up, to localize what tips it into a GAP. A
/// WIDE-beam pass on the tipping cases separates SEARCH pressure (closes at WIDE) from a real grammar
/// gap (gaps even at WIDE). Cap-only. Run:
///   cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       diagnose_project_achilles -- --ignored --nocapture
#[test]
#[ignore = "isolate the #4 Project Achilles gap (which swap tips it); --ignored --nocapture"]
fn diagnose_project_achilles() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let wide = build_index(&head).with_cell_beam(1024);
    let lem = morphy();
    let outcome = |c: usize, o: usize| -> String {
        if c > 0 {
            format!("CLOSED×{c}")
        } else if o > 0 {
            format!("open×{o}")
        } else {
            "GRAMMAR-GAP".into()
        }
    };
    let probe = |idx: &Parser, s: &str| -> String {
        let toks = tokenize(s);
        let unk: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !idx.has_token(t, &lem))
            .cloned()
            .collect();
        if !unk.is_empty() {
            return format!("OOV {unk:?}");
        }
        let (c, o) = idx.parse_open(s, &lem);
        outcome(c.len(), o.len())
    };

    // Drill into the tipping phrase "the top preferential dependency" as an as-complement: vary the
    // determiner, each modifier alone, and the stacking, to localize the composition gap. Also probe
    // the NP in plain object position to see if the as-complement is implicated or the NP itself.
    let isolated: &[(&str, &str)] = &[
        (
            "BASE: as a dependency",
            "we identified genes as a dependency",
        ),
        ("as the dependency", "we identified genes as the dependency"),
        (
            "as a preferential dep.",
            "we identified genes as a preferential dependency",
        ),
        (
            "as a top dependency",
            "we identified genes as a top dependency",
        ),
        (
            "as the top dependency",
            "we identified genes as the top dependency",
        ),
        (
            "as a top pref. dep.",
            "we identified genes as a top preferential dependency",
        ),
        (
            "as the top pref. dep.",
            "we identified genes as the top preferential dependency",
        ),
        (
            "OBJ: affect the top pref dep",
            "genes affect the top preferential dependency",
        ),
        (
            "OBJ: affect a top pref dep",
            "genes affect a top preferential dependency",
        ),
        (
            "OBJ: affect a preferential dep",
            "genes affect a preferential dependency",
        ),
        ("OBJ: affect a top dep", "genes affect a top dependency"),
    ];
    // Cumulative: add the domain features together (generic subject first, then the real subject).
    let cumulative: &[(&str, &str)] = &[
        ("+obj+asY", "we identified WRN as the top preferential dependency in cells compared to lines"),
        ("+obj+asY+inPP", "we identified WRN as the top preferential dependency in MSI cell lines compared to lines"),
        ("+obj+asY+bothPP (generic subj)", "we identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines"),
        ("FULL (real subj)", "Project Achilles and project DRIVE identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines."),
    ];

    eprintln!("\n═══ ISOLATED single-feature swaps (default beam) ═══");
    for (label, s) in isolated {
        eprintln!("   {label:<28} {:<12} {s:?}", probe(&index, s));
    }
    eprintln!("\n═══ CUMULATIVE build-up (default | WIDE cell_beam=1024) ═══");
    for (label, s) in cumulative {
        eprintln!(
            "   {label:<32} default={:<12} wide={:<12} {s:?}",
            probe(&index, s),
            probe(&wide, s)
        );
    }
}

/// D1 diagnostic (nominal-modification NF §8): run the `modifier_class` discriminator over the v3
/// corpus's REAL adjective lexicon entries (per WordNet sense), confirming its verdict on actual data
/// — `attractive` must screen as `Gradable`, classificatory adjectives (`genetic`/`somatic`/`immune`)
/// must be `Intersective` (the only collapse-eligible class). Cap-only (no parsing/rerank needed —
/// this seeds adjective leaves and classifies their sems). Run:
///   cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       d1_modifier_class_over_corpus -- --ignored --nocapture
#[test]
#[ignore = "D1 diagnostic: modifier_class over the corpus's real adjectives; --ignored --nocapture"]
fn d1_modifier_class_over_corpus() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // The v3 corpus's attributive modifiers, grouped by the verdict expected of a correct D1:
    let modifiers = [
        // the §5 hazard + the hyphenated domain term (S5):
        "attractive",
        "synthetic-lethal",
        // scalar / evaluative → expect Gradable (screened, not collapsed):
        "greater",
        "stronger",
        "strong",
        "rare",
        "frequent",
        "novel",
        "promising",
        "essential",
        // classificatory → expect Intersective (collapse-eligible):
        "genetic",
        "somatic",
        "germline",
        "immune",
        "homologous",
        "colorectal",
        "endometrial",
        // hyphen state-compounds:
        "double-stranded",
        "microsatellite-stable",
        // mixed / to observe:
        "specific",
        "deficient",
        "hypermutable",
        "independent",
        "predictive",
        "preferential",
    ];
    let mut tally: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for m in modifiers {
        let rows = index.debug_modifier_classes(m, &lem);
        if rows.is_empty() {
            eprintln!("\n{m:?} — (no adjective entry seeded)");
            continue;
        }
        eprintln!("\n{m:?} — {} adjective entries:", rows.len());
        for (cat, sense, class) in &rows {
            eprintln!("   {class:<12} sense={sense:<26} cat={cat}");
            *tally.entry(class.clone()).or_default() += 1;
        }
    }
    eprintln!("\n=== ModifierClass tally over all adjective entries ===");
    for (class, n) in &tally {
        eprintln!("  {class:<12} {n}");
    }
}

/// D63 lexicon-augmentation §6a — VERIFY both grounding indexes over the RESEEDED snapshot:
/// **(a)** the form `core:TextIndex` grounds the OOV surface `recq` → its UMLS concept C0084304
/// (`augment_lexicon_backed`, the RecQ finding over the real atoms), and **(c)** the concept
/// `core:description` `core:TextIndex` is populated over verb/adjective **axiom** glosses — the
/// converter fix (axioms now carry `core:description`; nouns/instances already did). Run:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       verify_grounding_indexes_over_snapshot -- --ignored --nocapture
#[test]
#[ignore = "verifies form+description grounding over a reseeded snapshot; --ignored --nocapture"]
fn verify_grounding_indexes_over_snapshot() {
    use eigenius_kernel::dcg::{
        augment_lexicon_backed, NoAbbreviationProposer, NominalCategoryProposer,
    };
    use eigenius_kernel::layer::resolve_active_text_indexes;
    use eigenius_kernel::ontology::resource::Value;
    use eigenius_kernel::query::text::analyzer::registry::analyzer_for;
    use eigenius_kernel::query::text::search::run_text_search;

    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };

    // Both indexes must be active over the reseeded head (declared in the lexicon schema layer).
    let active = resolve_active_text_indexes(&head);
    eprintln!(
        "=== active text indexes over snapshot head: {} ===",
        active.len()
    );
    for a in &active {
        eprintln!(
            "  idx={} target={} analyzer={}",
            a.iri.as_str(),
            a.target_property.as_str(),
            a.analyzer
        );
    }
    let form_prop = Iri::parse("urn:eigenius:lexicon:form").unwrap();
    let desc_prop = Iri::parse("urn:eigenius:core:description").unwrap();
    assert!(
        active.iter().any(|a| a.target_property == form_prop),
        "form_text_index active over the snapshot"
    );
    let desc_idx = active
        .iter()
        .find(|a| a.target_property == desc_prop)
        .expect("description_text_index active over the snapshot");

    // (a) FORM path — bare `recq` (OOV under the exact ValueIndex) grounds to C0084304 via the form
    // text index (BM25 over the seeded atoms), summed per concept.
    let lem = morphy();
    let aug = augment_lexicon_backed(
        &head,
        "recq affects HeLa.",
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &lem,
    );
    let recq = aug
        .added
        .iter()
        .find(|b| b.provenance.surface.to_lowercase() == "recq");
    match &recq {
        Some(b) => eprintln!(
            "\n(a) recq grounded_to={:?} confidence={:?}",
            b.provenance.grounded_to.as_ref().map(|i| i.as_str()),
            b.provenance.confidence
        ),
        None => eprintln!(
            "\n(a) recq NOT grounded; missing_oov={:?}",
            aug.missing_oov
                .iter()
                .map(|g| g.surface.as_str())
                .collect::<Vec<_>>()
        ),
    }
    let recq = recq.expect("recq grounds via the form text index");
    assert!(
        recq.provenance
            .grounded_to
            .as_ref()
            .map(|i| i.as_str().contains("C0084304"))
            .unwrap_or(false),
        "recq grounds to the RecQ family concept C0084304 (got {:?})",
        recq.provenance.grounded_to.as_ref().map(|i| i.as_str())
    );

    // (c) DESCRIPTION path — a verb axiom carries its synset gloss on `core:description`, and the
    // description index retrieves it by a distinctive gloss token (proves the converter fix +
    // index population over verb/adjective axioms, not just noun classes).
    let axiom_iri = Iri::parse("urn:eigenius:wn:v00860482_t").unwrap();
    let axiom = head
        .resolve(&axiom_iri)
        .expect("verb axiom wn:v00860482_t resolves in the snapshot");
    let gloss = match axiom.get(&desc_prop) {
        Some(Value::String(s)) => s.clone(),
        other => panic!("verb axiom carries no core:description gloss (got {other:?})"),
    };
    eprintln!(
        "\n(c) axiom {} core:description = {gloss:?}",
        axiom_iri.as_str()
    );
    assert!(
        gloss.contains("bravo"),
        "the axiom's description is the synset gloss"
    );
    let analyzer = analyzer_for(&desc_idx.analyzer).expect("analyzer for the description index");
    let hits = run_text_search(
        &head,
        head.storage().text_index.as_ref(),
        &desc_idx.iri,
        analyzer.as_ref(),
        "applaud bravo",
    )
    .expect("description search ok");
    eprintln!(
        "\n(c) description search 'applaud bravo' → {} hits (top 10):",
        hits.len()
    );
    for h in hits.iter().take(10) {
        eprintln!("  subj={} score={}", h.subject.as_str(), h.score);
    }
    assert!(
        hits.iter().any(|h| h.subject == axiom_iri),
        "the verb axiom is retrievable via its gloss token in the description index"
    );
}

/// End-to-end **OOV closure** over the WRN first page against the full lexicon — the DETERMINISTIC
/// (no-LLM) grounding pipeline. Measures the token-level OOV the augmentation leaves: baseline
/// (`augment_document_only`, deterministic Schwartz-Hearst abbreviations) vs after form+description
/// grounding (`augment_lexicon_backed`, nominal). The residual gaps are the fail-closed findings — what
/// the (B) LLM POS proposer (verb/adjective OOVs) and Phase-3 synthesis (genuinely novel terms) would
/// target next. Run:
///   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-wordnet --test db_backed_encoding \
///       wrn_page_oov_closure_deterministic -- --ignored --nocapture
#[test]
#[ignore = "OOV closure over the WRN page (deterministic, nominal); --ignored --nocapture"]
fn wrn_page_oov_closure_deterministic() {
    use eigenius_kernel::dcg::{
        augment_document_only, augment_lexicon_backed, NoAbbreviationProposer,
        NominalCategoryProposer,
    };
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let page_path = std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let doc = std::fs::read_to_string(&page_path).expect("read WRN page");
    let lem = morphy();

    let base = augment_document_only(&head, &doc, &NoAbbreviationProposer, &lem);
    let full = augment_lexicon_backed(
        &head,
        &doc,
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &lem,
    );

    eprintln!("=== WRN page OOV closure (deterministic, nominal) ===");
    eprintln!("baseline OOV (document-only): {}", base.missing_oov.len());
    eprintln!("added (abbrev + grounded):    {}", full.added.len());
    eprintln!("residual OOV:                 {}", full.missing_oov.len());
    eprintln!("\n-- grounded / added --");
    for b in &full.added {
        eprintln!(
            "  {:?} → {:?}  [{:?}]",
            b.provenance.surface,
            b.provenance.grounded_to.as_ref().map(|i| i.as_str()),
            b.provenance.method
        );
    }
    eprintln!("\n-- residual OOV (fail-closed findings) --");
    let mut res: Vec<&str> = full
        .missing_oov
        .iter()
        .map(|g| g.surface.as_str())
        .collect();
    res.sort();
    res.dedup();
    for s in &res {
        eprintln!("  {s:?}");
    }
}

/// Verify the `--umls-all` coverage win directly: `wilcoxon` (C0871608, T170 — outside the WRN-subset
/// TUIs) grounds over the full corpus, and `pcr-based` closes via the SHIPPED `X-based` compound rule
/// once its base `pcr` (C0032520, T063) is loaded (`docs/notes/d63-compound-morphology.md` §2a). Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-… cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_wilcoxon_pcr_grounding -- --ignored --nocapture
#[test]
#[ignore = "verify wilcoxon/pcr grounding over the --umls-all snapshot; --ignored --nocapture"]
fn probe_wilcoxon_pcr_grounding() {
    use eigenius_kernel::dcg::{
        augment_lexicon_backed, NoAbbreviationProposer, NominalCategoryProposer,
    };
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    for t in ["pcr", "pcr-based", "wilcoxon", "cas9-mediated"] {
        eprintln!("has_token({t:?}) = {}", index.has_token(t, &lem));
    }
    let aug = augment_lexicon_backed(
        &head,
        "The wilcoxon test compared MSI and MSS cell lines. A pcr-based assay confirmed the result.",
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &lem,
    );
    eprintln!("-- grounded --");
    for b in &aug.added {
        eprintln!(
            "  {:?} → {:?}",
            b.provenance.surface,
            b.provenance.grounded_to.as_ref().map(|i| i.as_str())
        );
    }
    eprintln!(
        "residual OOV: {:?}",
        aug.missing_oov
            .iter()
            .map(|g| g.surface.as_str())
            .collect::<Vec<_>>()
    );
}

/// Grammar-gap ROOT-CAUSE battery (`2026-07-05`): short isolation probes for each construction in the
/// `--umls-all` run's 20 grammar-gaps, over the augmented index (so subjects like MSI/WRN are grounded +
/// overlaid — the run's config). Each prints CLOSED×n / OPEN×n / GAP so the blocker is localized to the
/// construction, not the subject. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-… cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_grammar_gap_root_causes -- --ignored --nocapture
#[test]
#[ignore = "grammar-gap root-cause battery over the --umls-all snapshot; --ignored --nocapture"]
fn probe_grammar_gap_root_causes() {
    use eigenius_kernel::dcg::{
        augment_lexicon_backed, NoAbbreviationProposer, NominalCategoryProposer,
    };
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let probes = [
        // — argument-PP verb (Step-2 fix target): the note's contrast + the actual object —
        "instability contributes to cells",
        "MSI contributes to cells",
        "MSI contributes to several cancers",
        "MSI results from deficiency",
        "cells respond to therapy",
        "MSI is associated with responses",
        // — adjunct-PP verb (should VP-adjoin per Step-1) —
        "MSI occurs in cancers",
        "MSI arises from deficiency",
        // — comparative `than` —
        "cells showed greater dependence than counterparts",
        "cells contained fewer mutations than lineages",
        // — `V X as Y` predicative —
        "we evaluated MSI as a biomarker",
        // — copula compound kind —
        "regions are microsatellites",
        "nucleotide repeat regions are microsatellites",
        // — object coordination (mismatched NPs) —
        "WRN requires lineages or a phenotype",
        // — adjective + PP complement —
        "classifications were concordant with phenotyping",
        // — linking verb + adjective —
        "findings remained true",
        // — named entity —
        "MSI arises from Lynch syndrome",
    ];
    // Augment the whole battery as one document so OOV subjects (MSI/WRN/…) are grounded + overlaid.
    let doc = probes.join(". ");
    let aug = augment_lexicon_backed(
        &head,
        &doc,
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &lem,
    );
    eprintln!(
        "augmentation: {} grounded, {} residual",
        aug.added.len(),
        aug.missing_oov.len()
    );
    let index = build_index_over(&head, Some(&aug));
    for t in [
        "msi",
        "wrn",
        "lynch syndrome",
        "microsatellites",
        "concordant",
        "remained",
        "biomarker",
    ] {
        eprintln!("has_token({t:?}) = {}", index.has_token(t, &lem));
    }
    eprintln!("-- probes --");
    for s in probes {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

/// STEP 4 (RC-1) — witness the bare-UMLS-noun-subject mechanism (d63-parse-gap-closure §3/§4).
/// Part 1: the actual `lexicon:cat` of the abbreviation forms in the snapshot (count `num_any` vs `mass`
/// vs `cat_np`). Part 2: a determiner/number/mass battery isolating whether a determiner or a mass/plural
/// reading turns the bare `MSI` subject into a parse — confirming the count-vs-mass diagnosis. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-… cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_step4_bare_umls_subject -- --ignored --nocapture
#[test]
#[ignore = "Step 4 (RC-1): bare-UMLS-subject mechanism; --ignored --nocapture"]
fn probe_step4_bare_umls_subject() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    // Part 1 — the emitted cats (count `num_any` vs `mass` vs `cat_np`) for the abbreviation forms and the
    // WordNet mass baseline (`instability`) + a count baseline (`gene`/`genes`).
    for form in ["msi", "mmr", "mss", "instability", "gene", "genes"] {
        let entries = index.debug_form_entries(form, &lem);
        eprintln!("=== {form:?} — {} entries ===", entries.len());
        for (closed, cat, sense) in entries.iter().take(8) {
            eprintln!("  closed={closed} sense={sense:<16} cat={cat}");
        }
    }
    // Part 2 — subject battery: does a determiner / mass / plural fix the bare subject?
    eprintln!("-- subject battery (all forms known; no augmentation) --");
    for s in [
        "MSI contributes to cells",         // bare count (num_any) — GAP expected
        "the MSI contributes to cells",     // + determiner
        "MSI contribute to cells",          // bare, plural agreement
        "instability contributes to cells", // bare MASS (WordNet) — CLOSED expected
        "the instability contributes to cells", // mass + determiner
        "genes contribute to cells",        // bare PLURAL count — kind
        "gene contributes to cells",        // bare SINGULAR count — GAP expected (English)
        "a gene contributes to cells",      // singular count + determiner
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

/// STEP 5 (RC-6) — localize the coordination gaps (d63-parse-gap-closure §4 Step 5). Isolation probes
/// for each coordination sub-case (a plain baseline, comma-list, quantified `some X and some Y`,
/// proper-noun, mismatched-NP `X or a Y`, apposition `the N genes …`) over the current snapshot, so the
/// fix scope is per-construction, not "coordination" as a monolith. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-… cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_step5_coordination -- --ignored --nocapture
#[test]
#[ignore = "Step 5 (RC-6): coordination sub-case localization; --ignored --nocapture"]
fn probe_step5_coordination() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    let probes = [
        // baseline — plain 2-item NP / adjective coordination (the plan says this already parses)
        "cells and genes affect HeLa",
        "colon and gastric cancers affect HeLa",
        // (a) comma-LIST coordination (3+ items) modifying a noun
        "colon, gastric and ovarian cancers affect HeLa",
        // (c) quantified NP coordination `some X and some Y`
        "some cells and some genes affect HeLa",
        // (d) proper-noun coordination as subject
        "HeLa and BRCA1 affect cells",
        // (e) MISMATCHED-NP object coordination — bare-plural `or` singular-indefinite (different cats)
        "WRN affects genes or a phenotype",
        "WRN affects genes or cells", // matched control (both bare plural) — should coordinate
        // (b) noun-name APPOSITION + name-list
        "the genes BRCA1 and MSH2 affect cells",
        // the actual RC-6 sentences (post-mass-shim status)
        "some MSI lines and some MSS lines were represented by data sets",
        "WRN dependency may require specific lineages or a stronger mutation phenotype",
    ];
    for s in probes {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

/// STEP 5 (RC-6) — VERIFY the close-apposition rule (`appose_group`, category.rs): a definite/bare
/// common-noun head + a coreferential name-group passes the group through (gated on the members being
/// of the head's base kind), so it rides the distributive-subject / -object machinery. Isolates each
/// syntactic POSITION (subject / bare / object / prep-object) + the felicity reject, so a residual GAP
/// localizes to the position, not the apposition rule. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-2026-07-06 cargo test -p eigenius-wordnet \
///       --test db_backed_encoding probe_step5_apposition -- --ignored --nocapture
/// RC-2 comparatives — category dump + gap localization. The gap sentences use ATTRIBUTIVE comparatives
/// (`greater dependence`, `fewer mutations`, `a stronger phenotype`), unlike the existing PREDICATIVE
/// machinery (`X is larger than Y` — `(S[adj]\NP)/cat_pp_than`). This dumps what category the comparative
/// forms actually get on the real lexicon (positive? predicative comparative? lemmatized to base?) and
/// which of the sub-shapes gap. Run:
///   EIGENIUS_DB_SNAPSHOT=/…/wordnet-umls-all-2026-07-06 cargo test -p eigenius-wordnet \
///       --test db_backed_encoding probe_rc2_comparatives -- --ignored --nocapture
#[test]
#[ignore = "RC-2 comparatives: category dump + gap localization; --ignored --nocapture"]
fn probe_rc2_comparatives() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    for form in [
        "great",
        "greater",
        "few",
        "fewer",
        "strong",
        "stronger",
        "larger",
        "dependence",
        "than",
    ] {
        eprintln!("  TYPES {form}:");
        for (aug, cat, sense) in index.debug_form_entries(form, &lem) {
            let a = if aug { "+" } else { " " };
            eprintln!("     {a} {cat}   [{sense}]");
        }
    }
    for s in [
        "a stronger phenotype affects cells", // #12 attributive comparative, NO than
        "greater dependence affects cells",   // attributive comparative + noun, isolated
        "WRN showed greater dependence than genes", // the than-clause with a comparative
        "cells contained fewer mutations than genes", // #9 shape (simplified)
    ] {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

#[test]
#[ignore = "Step 5 (RC-6): apposition-rule verification; --ignored --nocapture"]
fn probe_step5_apposition() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    let probes = [
        // Apposition (Step 5) regression witnesses:
        "the genes BRCA1 and MSH2 affect cells", //             subject apposition
        "mutations in the genes BRCA1 and MSH2 cause cancer", // prep-object apposition
        // Comma-list connective inheritance (Step 5b):
        "MSH2, MSH6, PMS2 or MLH1 affect cells", //             bare comma-OR name list (was GAP)
        "the MMR genes MSH2, MSH6, PMS2 or MLH1 affect cells", // full corpus-shape apposition (was GAP)
        "mutations in the MMR genes MSH2, MSH6, PMS2 or MLH1 cause cancer", // corpus prep-obj shape (GAP)
        // Localize the prep-obj GAP: compound head vs comma-or list, in prep-object position.
        "mutations in the MMR genes BRCA1 and MSH2 cause cancer", // compound head + simple `and`
        "mutations in the genes MSH2, MSH6, PMS2 or MLH1 cause cancer", // plain head + comma-`or`
        "WRN affects the MMR genes MSH2, MSH6, PMS2 or MLH1", // same apposition in OBJECT position
        "colon, gastric and ovarian cancers affect HeLa", //    adjective comma-AND list (no regression)
        // FELICITY reject — genes are not cells; the apposition must NOT license "the cells BRCA1 …".
        "the cells BRCA1 and MSH2 affect HeLa",
    ];
    for s in probes {
        let (closed, open) = index.parse_open(s, &lem);
        let tag = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("OPEN×{}", open.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{tag:>9}] {s}");
    }
}

/// S3 over-prune localization (GH#97): `Each event alone does not lead to cell death` gaps WITH the
/// cross-POS prune but parses without. This dumps what the prune drops for each of S3's function words
/// (closed / open-nominal=dropped / open-other=kept) and A/B-parses S3 sub-variants, to find which
/// dropped nominal reading S3 needs. Run with and without `EIGENIUS_POS_PRUNE=1`:
///   [EIGENIUS_POS_PRUNE=1] cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_s3_localization -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: localize the S3 over-prune; run with --ignored --nocapture"]
fn probe_s3_localization() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head); // honors EIGENIUS_POS_PRUNE
    let lem = morphy();

    eprintln!("=== S3 function-word entries (closed / open-NOMINAL=pruned / open-other=kept) ===");
    for w in [
        "each", "alone", "does", "not", "to", "lead", "cell", "death",
    ] {
        let es = index.debug_form_entries(w, &lem);
        let closed = es.iter().filter(|e| e.0).count();
        let open_nominal = es
            .iter()
            .filter(|e| {
                !e.0 && (e.2.starts_with("cat_n(")
                    || e.2.starts_with("cat_np(")
                    || e.1.contains("cat_n("))
            })
            .count();
        // crude: an entry is nominal if its cat string contains cat_n( or cat_np(
        let nominal = es
            .iter()
            .filter(|e| !e.0 && (e.1.contains("cat_n(") || e.1.contains("cat_np(")))
            .count();
        let open_other = es.iter().filter(|e| !e.0).count() - nominal;
        eprintln!("  {w:<7} closed={closed} open-nominal(pruned)={nominal} open-other(kept)={open_other}  [{open_nominal}]");
    }

    eprintln!("\n=== S3 sub-variants (outcome under current build_index config) ===");
    let variants = [
        "WRN leads to cell death",         // control: lead + to-PP, name subject
        "each event leads to cell death",  // + each
        "events alone lead to cell death", // + alone
        "WRN does not lead to cell death", // + do-support negation
        "WRN does not affect cells",       // do-support TRANSITIVE, no to-PP
        "WRN does not affect a gene",      // do-support transitive, GQ object
        "WRN affects cells",               // control: finite transitive, no do-support
        "each event alone leads to cell death", // each + alone, no do-support
        "each event does not lead to cell death", // each + do-support, no alone
        "Each event alone does not lead to cell death.", // full S3
    ];
    for s in variants {
        let (c, o) = index.parse_open(s, &lem);
        let tag = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        };
        // Print the first parse's sem so we can tell a REAL reading from noun-pile junk.
        let sem = c
            .first()
            .map(|it| it.sem())
            .or_else(|| o.first().map(|op| op.item.sem()));
        // Raw pretty-print (no eval — open parses carry unbound `$quant$` holes that can't be
        // evaluated), enough to tell a real verb/prep reading from noun-pile / mis-typed junk.
        let sem_s = sem
            .map(|e| {
                eigenius_kernel::dcg::pretty_term(e)
                    .chars()
                    .take(160)
                    .collect::<String>()
            })
            .unwrap_or_default();
        eprintln!("  {tag:<11} {s:?}\n      → {sem_s}");
    }
}

/// Function-word-noise enumeration (D62/GH#97): for each function word in the 5 sentences, list its
/// CLOSED-class (grammatical) vs OPEN-class (wordnet/umls noun/verb/adj) entries. The open-class
/// senses on function words are what let the compound rule chain across copulas/determiners into the
/// spurious refined-noun piles that saturate the beam. `#[ignore]`d; run:
///   cargo test -p eigenius-wordnet --test db_backed_encoding enumerate_function_word_noise \
///       -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: enumerate function-word open-class noise; run with --ignored --nocapture"]
fn enumerate_function_word_noise() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    // The function/closed-class words occurring across the 5 CNL sentences.
    let words = [
        "is", "an", "a", "the", "are", "between", "two", "these", "each", "of", "for", "to", "can",
        "does", "not", "alone", "this", "and", "or",
    ];
    for w in words {
        let entries = index.debug_form_entries(w, &lem);
        let closed: Vec<&(bool, String, String)> = entries.iter().filter(|e| e.0).collect();
        let open: Vec<&(bool, String, String)> = entries.iter().filter(|e| !e.0).collect();
        eprintln!(
            "\n{w:?}: {} closed-class, {} OPEN-class (noise candidates)",
            closed.len(),
            open.len()
        );
        for (_, cat, sense) in &open {
            eprintln!("    OPEN  {sense:<20} {cat}");
        }
    }
}

/// Pretty-print the EigenTT sem (`Prop`) of the best parse of each of the first 5 CNL v2 sentences.
/// The parses are OPEN (referent/quant holes), so this shows the reduced normal form of the
/// lowest-cost parse. Honors `EIGENIUS_POS_PRUNE`. Run:
///   EIGENIUS_POS_PRUNE=1 cargo test -p eigenius-wordnet --test db_backed_encoding \
///       pretty_print_first_five_sems -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: pretty-print the first-5 sems; run with --ignored --nocapture"]
fn pretty_print_first_five_sems() {
    let Some(head) = snapshot_path().and_then(|p| open_head(&p)) else {
        return;
    };
    let index = build_index(&head);
    let lem = morphy();
    let sentences = [
        "Synthetic lethality is an interaction between two genetic events.",
        "The co-occurrence of these two events leads to cell death.",
        "Each event alone does not lead to cell death.",
        "Scientists can exploit synthetic lethality for cancer therapeutics.",
        "DNA repair processes are attractive synthetic lethal targets.",
        "Many cancers exhibit an impairment of a DNA repair pathway.",
        "This impairment can lead to dependence on specific repair proteins.",
    ];
    for (i, s) in sentences.iter().enumerate() {
        let (c, o) = index.parse_open(s, &lem);
        let (n, sem) = if !c.is_empty() {
            (c.len(), Some(c[0].sem()))
        } else if !o.is_empty() {
            (o.len(), Some(o[0].item.sem()))
        } else {
            (0, None)
        };
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("S{}  {s}", i + 1);
        eprintln!("     ({n} parse(s); best shown)");
        match sem {
            Some(e) => {
                eprintln!("  ⟦·⟧ = {}", eigenius_kernel::dcg::pretty_term(e));
                let mut iris: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                collect_iris(e, &mut iris);
                eprintln!("  where:");
                for iri_s in &iris {
                    let local = iri_s.rsplit(':').next().unwrap_or(iri_s);
                    // Only the opaque synset/CUI/axiom codes need glossing.
                    if !(local.starts_with('n')
                        || local.starts_with('C')
                        || local.starts_with('v')
                        || local.starts_with("deg_")
                        || local.starts_with('a'))
                    {
                        continue;
                    }
                    let gloss = Iri::parse(iri_s)
                        .ok()
                        .and_then(|i| head.resolve(&i))
                        .and_then(|r| {
                            match r.get(&Iri::parse("urn:eigenius:core:description").unwrap()) {
                                Some(eigenius_kernel::ontology::resource::Value::String(s)) => {
                                    Some(s.clone())
                                }
                                _ => None,
                            }
                        })
                        .map(|d| d.chars().take(60).collect::<String>());
                    if let Some(g) = gloss {
                        eprintln!("     {local:<14} = {g}");
                    }
                }
            }
            None => eprintln!("  (no parse)"),
        }
    }
    eprintln!();
}

/// Collect the opaque IRIs (synset classes, verb/adjective axioms, resources) a sem references.
fn collect_iris(e: &Exp, out: &mut std::collections::BTreeSet<String>) {
    use eigenius_kernel::nbe::term::Exp as E;
    match e {
        E::EigonClass(iri) | E::EigonAxiom(iri) => {
            out.insert(iri.as_str().to_string());
        }
        E::EigonResource(r) => {
            if let Some(id) = r.id() {
                out.insert(id.as_str().to_string());
            }
        }
        E::App(f, a) | E::Arrow(f, a) | E::Times(f, a) | E::Pair(f, a) => {
            collect_iris(f, out);
            collect_iris(a, out);
        }
        E::Lam(_, b) | E::Con(_, b) | E::Fst(b) | E::Snd(b) | E::Ann(b, _) => collect_iris(b, out),
        E::Pi(_, t, b) | E::Sig(_, t, b) => {
            collect_iris(t, out);
            collect_iris(b, out);
        }
        E::InductiveCtor(_, _, args) | E::InductiveType(_, args) => {
            for a in args {
                collect_iris(a, out);
            }
        }
        _ => {}
    }
}

/// The 7 worst noun-pile sentences (CNL v2, GH#97) — outcome + parse TIME, to measure the
/// compound-depth cost penalty (were 36–565s + GRAMMAR-GAP). Honors `EIGENIUS_POS_PRUNE`. Run:
///   EIGENIUS_POS_PRUNE=1 cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_noun_pile_sentences -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: noun-pile sentences after the compound penalty; run with --ignored --nocapture"]
fn probe_noun_pile_sentences() {
    let Some(head) = snapshot_path().and_then(|p| open_head(&p)) else {
        return;
    };
    let index = build_index(&head);
    let lem = morphy();
    for s in [
        "Some cancers do not respond to immune checkpoint blockade.",
        "Project Achilles screened cell lines with a CRISPR library.",
        "These observations suggest that WRN dependency is not simply a result of MMR deficiency.",
        "WRN dependency may require specific lineages or a stronger mutation phenotype.",
        "These cell lines contained fewer deletion mutations in microsatellite regions than typical lineages.",
        "We analysed these data sets for genes that are selectively essential in cancer cells with MSI.",
        "Project Achilles and project DRIVE identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines.",
    ] {
        let t = std::time::Instant::now();
        let (c, o) = index.parse_open(s, &lem);
        let tag = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  {tag:<11} [{:>6.1}s] {s:?}", t.elapsed().as_secs_f64());
    }
}

/// WIN PROBE for the packed forest (D63 Option A, blueprint §11 3f.4): parse a *packable* pile
/// sentence (no relatives/commas/coordination → the router engages packing) over the full lexicon,
/// with packing OFF vs ON, reporting outcome + wall-clock. With `EIGENIUS_PARSE_DEBUG=1` the packed
/// run also prints `forest nodes=N` — the pile's sense-product collapsed to O(nodes) vs the ~30k flat
/// items of the unpacked cell. Same (closed, open) ⇒ the win is a speed/space gain, not a parse
/// change. Honors `EIGENIUS_POS_PRUNE`. Run:
///   EIGENIUS_PARSE_DEBUG=1 EIGENIUS_POS_PRUNE=1 cargo test -p eigenius-wordnet \
///       --test db_backed_encoding packed_win_probe -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: packed vs unpacked win probe; run with --ignored --nocapture"]
fn packed_win_probe() {
    let Some(head) = snapshot_path().and_then(|p| open_head(&p)) else {
        return;
    };
    let lem = morphy();
    // Packable pile sentences (no `which`/comma/coordination; index-independent verbs). `that` (both
    // restrictive-relative and complementizer) now packs (§11 3g.3).
    let sentences = [
        "DNA repair processes are attractive synthetic lethal targets.",
        "Synthetic lethality is an interaction between two genetic events.",
        // that-RELATIVE pile sentence — one of the worst unpacked (~199s in the noun-pile probe):
        "We analysed these data sets for genes that are selectively essential in cancer cells with MSI.",
    ];
    let unpacked = build_index(&head).with_packing(false);
    let packed = build_index(&head).with_packing(true);
    for s in sentences {
        eprintln!("\n{s:?}");
        for (name, idx) in [("unpacked", &unpacked), ("packed", &packed)] {
            let t = std::time::Instant::now();
            let (c, o) = idx.parse_open(s, &lem);
            eprintln!(
                "  {name:<9} closed×{} open×{} [{:>6.1}s]",
                c.len(),
                o.len(),
                t.elapsed().as_secs_f64()
            );
        }
    }
}

/// A/B witness for GH#97 Fix #2 (construction-time compound-depth CAP): parse the witnessed
/// pure-pile sentence (unit 32 — full-span cell recorded at 34,472 items pre-cap) at a WIDE beam,
/// with `EIGENIUS_PARSE_DEBUG=1`, and report the MAX per-cell `produced` (items BUILT before
/// beaming — the construction cost). Run once with the cap live and once with `MAX_COMPOUND_MODS`
/// bumped high to see the delta. `#[ignore]`d; run:
///   EIGENIUS_PARSE_DEBUG=1 EIGENIUS_POS_PRUNE=1 cargo test -p eigenius-wordnet \
///       --test db_backed_encoding measure_pile_cell_population -- --ignored --nocapture 2>&1 \
///     | grep -oE 'produced=[0-9]+' | sort -t= -k2 -n | tail -1
#[test]
#[ignore = "diagnostic: max cell population of the pure-pile sentence; run with EIGENIUS_PARSE_DEBUG=1 --ignored --nocapture"]
fn measure_pile_cell_population() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = Parser::build(Arc::clone(&head))
        .with_sense_cap(2)
        .with_cell_beam(1024)
        .with_pos_prune(std::env::var("EIGENIUS_POS_PRUNE").is_ok());
    // Attach the live contextual reranker when built with --features use-llm (mirrors build_index),
    // so this probe measures the reranked serving path, not cap-only.
    #[cfg(feature = "use-llm")]
    let index = match eigenius_kernel::dcg::AnthropicSenseRanker::from_env() {
        Some(r) => {
            eprintln!("contextual reranker: AnthropicSenseRanker (live)");
            index.with_sense_ranker(Box::new(r))
        }
        None => {
            eprintln!("contextual reranker: none (ANTHROPIC_API_KEY unset)");
            index
        }
    };
    #[cfg(not(feature = "use-llm"))]
    eprintln!("contextual reranker: none (cap-only)");
    let lem = morphy();
    let s = "Some cancers do not respond to immune checkpoint blockade.";
    eprintln!("MEASURE (pile cell population): {s:?}");
    let (closed, open) = index.parse_open(s, &lem);
    eprintln!("  → closed×{} open×{}", closed.len(), open.len());
}

/// Chart-cell population analysis for the 5 CNL v2 sentences (user request 2026-06-30): parse each
/// at a WIDE beam (1024 ≈ uncapped at sense_cap=2) with `EIGENIUS_PARSE_DEBUG=1`, so the per-cell
/// shape histograms (`cat_shape`, type-indices erased) show WHERE the chart population concentrates
/// and WHETHER it is lexical/sense variation (one shape, many indices ⇒ a GH#93 type-narrowing
/// candidate) or structural ambiguity (many shapes ⇒ narrowing won't help). `#[ignore]`d; run:
///   EIGENIUS_PARSE_DEBUG=1 cargo test -p eigenius-wordnet --test db_backed_encoding \
///       analyze_chart_cells_first_five -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: chart-cell population analysis; run with EIGENIUS_PARSE_DEBUG=1 --ignored --nocapture"]
fn analyze_chart_cells_first_five() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    // Wide beam so the dumped cells show the true population, not the page-beam-capped view.
    // Honors EIGENIUS_POS_PRUNE so the pile shown is the residual AFTER the cross-POS prune.
    let index = Parser::build(Arc::clone(&head))
        .with_sense_cap(2)
        .with_cell_beam(1024)
        .with_pos_prune(std::env::var("EIGENIUS_POS_PRUNE").is_ok());
    let lem = morphy();
    let sentences = [
        "Synthetic lethality is an interaction between two genetic events.",
        "The co-occurrence of these two events leads to cell death.",
        "Each event alone does not lead to cell death.",
        "Scientists can exploit synthetic lethality for cancer therapeutics.",
        // v3: `synthetic-lethal` hyphenated (lexicalized compound modifier, style-guide fix) so it is
        // ONE compound adjective, not a `synthetic` ∧ `lethal` adjective stack (d63-nominal-mod NF §4).
        "DNA repair processes are attractive synthetic-lethal targets.",
    ];
    for s in sentences {
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("ANALYZE: {s:?}");
        let (closed, open) = index.parse_open(s, &lem);
        eprintln!("  → closed×{} open×{}", closed.len(), open.len());
    }
}

/// Per-sentence blocker diagnosis for the FIRST 5 CNL v2 sentences (user request 2026-06-30):
/// for each sentence, print token-level OOV, the full-sentence parse outcome, and a fragment
/// ladder that localizes the exact construction that stalls. `#[ignore]`d; run manually:
///   cargo test -p eigenius-wordnet --test db_backed_encoding diagnose_first_five_cnl \
///       -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: localize per-sentence blockers of CNL v2's first 5; run with --ignored --nocapture"]
fn diagnose_first_five_cnl() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    // SENTENCE-SHAPED minimal pairs (parse_open only returns full-span S parses, so a bare NP
    // fragment is always GRAMMAR-GAP and tells us nothing). Each group varies ONE construction at a
    // time, anchored on the known-good `genes are attractive targets` / `genes affect cells`, using
    // small-lexicon generic slot fillers (genes/cells) so the GRAMMAR is isolated from the specific
    // domain word's vocabulary/countability — the domain word is then swapped in as the LAST probe.
    let sentences: &[(&str, &[&str])] = &[
        (
            "THE 5 ACTUAL CNL v2 SENTENCES (end-to-end verdict)",
            &[
                "Synthetic lethality is an interaction between two genetic events.",
                "The co-occurrence of these two events leads to cell death.",
                "Each event alone does not lead to cell death.",
                "Scientists can exploit synthetic lethality for cancer therapeutics.",
                "DNA repair processes are attractive synthetic lethal targets.",
            ],
        ),
        (
            "ANCHORS (known-good)",
            &[
                "genes are attractive targets", // copula pred-nom, bare-pl subj + adj+noun pred
                "genes affect cells",           // bare-plural SVO control
            ],
        ),
        (
            "COPULA: number / bare predicate / stacked adjectives (S5, S1)",
            &[
                "genes are targets",                             // bare-plural predicate nominal
                "genes are attractive synthetic lethal targets", // 3 stacked attributive adjs
                "genes are interactions", // plural=plural pred-nom (S1 skeleton)
                "a gene is an interaction", // sg=sg pred-nom (S1 determiners)
            ],
        ),
        (
            "COMPOUND SUBJECT (S5 'DNA repair processes', S2 'co-occurrence')",
            &[
                "processes are attractive targets", // single common-noun plural subject
                "repair processes are attractive targets", // 2-noun compound subject
                "DNA repair processes are attractive targets", // 3-noun compound subject
            ],
        ),
        (
            "BARE-MASS OBJECT / SUBJECT (S4 'synthetic lethality', S1)",
            &[
                "genes exploit cells",    // verb 'exploit' + plain plural object (control)
                "genes affect lethality", // bare common-noun object (countability probe)
                "genes affect synthetic lethality", // adj + bare common-noun object
                "lethality affects cells", // bare common-noun SUBJECT
            ],
        ),
        (
            "DETERMINERS: each / these+numeral (S2, S3)",
            &[
                "each gene affects cells",      // 'each' determiner subject
                "these genes affect cells",     // plural 'these' subject
                "these two genes affect cells", // 'these' + numeral subject
                "the two genes affect cells",   // 'the' + numeral subject
            ],
        ),
        (
            "MODAL / DO-SUPPORT / NEGATION (S3, S4)",
            &[
                "genes can affect cells",       // modal + bare-plural subject
                "genes do not affect cells",    // do-support negation, bare plural
                "a gene does not affect cells", // do-support negation, singular
            ],
        ),
        (
            "PP ADJUNCTS: for / between (S4, S1)",
            &[
                "genes affect cells for therapies", // 'for' VP-adjunct, bare-plural object
                "a gene affects cells for a therapy", // 'for' VP-adjunct, singular objects
                "a gene is an interaction between cells", // 'between' noun-mod PP
            ],
        ),
    ];

    // Confirmatory probes for the two non-`to`-prep blockers + remaining constructions.
    let extras: &[&str] = &[
        "the impairment of a gene affects cells", // the + N + of-PP SUBJECT (S2 skeleton, no to-PP)
        "each gene alone affects cells",          // 'alone' floating adverb (S3)
        "a gene affects cell death",              // bare-compound singular OBJECT 'cell death' (S2)
        "genes are cell death", // 'death' bare-mass as predicate (probe countability)
    ];
    eprintln!("\n════════════════════════════════════════════════════════════════");
    eprintln!("EXTRAS (of-PP subj / 'alone' / bare-compound object)");
    for f in extras {
        let ft = tokenize(f);
        let unk: Vec<String> = ft
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unk.is_empty() {
            eprintln!(
                "    [{:>2}t] OOV         {f:?} (unknown: {unk:?})",
                ft.len()
            );
            continue;
        }
        let (c, o) = index.parse_open(f, &lem);
        let s = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GRAMMAR-GAP".into()
        };
        eprintln!("    [{:>2}t] {s:<12} {f:?}", ft.len());
    }
    // WIDE-BEAM test: does the 3-noun compound subject parse at cell_beam=1024? CLOSED/open ⇒ the
    // page-beam GAP is BEAM PRESSURE (GH #97 Lever B), not a missing compound rule.
    let wide = Parser::build(Arc::clone(&head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(1024);
    for f in [
        "DNA repair processes are attractive targets",
        "DNA repair processes are targets",
        // The 5 actual sentences at a wide beam — beam pressure (GH #97) vs a real composition gap.
        "Synthetic lethality is an interaction between two genetic events.",
        "The co-occurrence of these two events leads to cell death.",
        "Each event alone does not lead to cell death.",
        "Scientists can exploit synthetic lethality for cancer therapeutics.",
        "DNA repair processes are attractive synthetic lethal targets.",
        // S4 localization (gaps even at wide beam) — peel off modal / for-PP / each object.
        "scientists exploit synthetic lethality",
        "scientists exploit cells for therapies",
        "scientists exploit synthetic lethality for therapies",
        "scientists exploit synthetic lethality for cancer therapeutics",
        "scientists can exploit cells",
        "genes affect cancer therapeutics",
    ] {
        let (c, o) = wide.parse_open(f, &lem);
        let s = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GRAMMAR-GAP".into()
        };
        eprintln!("    [wide beam 1024] {s:<12} {f:?}");
    }

    for (sentence, ladder) in sentences {
        eprintln!("\n════════════════════════════════════════════════════════════════");
        eprintln!("SENTENCE: {sentence:?}");
        // token-level OOV
        let toks = tokenize(sentence);
        let oov: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        eprintln!("  tokens: {} | OOV: {oov:?}", toks.len());
        eprintln!("  --- fragment ladder (small→large) ---");
        for f in *ladder {
            let ftoks = tokenize(f);
            let unknown: Vec<String> = ftoks
                .iter()
                .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
                .cloned()
                .collect();
            if !unknown.is_empty() {
                eprintln!(
                    "    [{:>2}t] OOV         {f:?}  (unknown: {unknown:?})",
                    ftoks.len()
                );
                continue;
            }
            let t = std::time::Instant::now();
            let (closed, open) = index.parse_open(f, &lem);
            let status = if !closed.is_empty() {
                format!("CLOSED×{}", closed.len())
            } else if !open.is_empty() {
                format!("open×{}", open.len())
            } else {
                "GRAMMAR-GAP".to_string()
            };
            eprintln!(
                "    [{:>2}t] {status:<12} [{:.1}s] {f:?}",
                ftoks.len(),
                t.elapsed().as_secs_f64()
            );
        }
    }
}

/// Fragment bisection (D62 grammar-gap diagnosis): parse curated sub-spans of the nearest
/// grammar-gap units against the full lexicon and report which compose (closed / open / —), to
/// localize the actual stall points instead of inferring them. `#[ignore]`d; run manually:
///   cargo test -p eigenius-wordnet --test db_backed_encoding diagnose_grammar_gap_fragments \
///       -- --ignored --nocapture
#[test]
#[ignore = "diagnostic: localize grammar-gap stalls; run with --ignored --nocapture"]
fn diagnose_grammar_gap_fragments() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    // CONTROL probes — isolate the fundamental blocker (determiner vs bare noun; proper-noun subj;
    // copula) using common full-lexicon words.
    let controls = [
        "a gene affects a cell", // determiners + known noun/verb — basic SVO control
        "genes affect cells",    // bare plurals — same clause without determiners
        "a cell is a gene",      // copula + predicate-nominal with determiners
        "a gene is large",       // copula + predicative adjective
    ];
    eprintln!("\n=== control probes (determiner vs bare; copula) ===");
    for f in controls {
        eprintln!("  probing {f:?} …"); // printed BEFORE the parse, so a hang/OOM names the culprit
        let t = std::time::Instant::now();
        let (closed, open) = index.parse_open(f, &lem);
        let s = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("open×{}", open.len())
        } else {
            "—".into()
        };
        eprintln!("  {s:<10} [{:.1}s] {f:?}", t.elapsed().as_secs_f64());
    }

    // Fragments ordered small→large for unit 4 / unit 5 / unit 8 (the shortest grammar-gaps).
    let fragments = [
        // unit 4: "MSI cancer models required the helicase activity of WRN, but not its …"
        "MSI cancer models",
        "the helicase activity",
        "the helicase activity of WRN",
        "MSI cancer models required HeLa",
        "MSI cancer models required the helicase activity of WRN",
        // unit 5: "WRN is a synthetic lethal vulnerability and promising drug target for MSI cancers"
        "WRN is a vulnerability",
        "WRN is a synthetic lethal vulnerability",
        "WRN is a vulnerability and a target",
        "WRN is a vulnerability for MSI cancers",
        // unit 8: "Thus, novel therapies are needed for tumours with MSI"
        "novel therapies",
        "therapies are needed",
        "novel therapies are needed for tumours",
        "thus novel therapies are needed",
        // PREP-OBJECT isolation probes (D62 §2 GQ-as-prep-object): name vs GQ object, and the
        // cat_pp (noun-mod) family vs the VP-adjunct family, to locate the residual gap.
        // D62 §2 GQ-as-prep-object coverage anchors: a quantified/bare-plural NP scopes into a
        // preposition's object slot (was: only a bare NAME could). Both prep families — the
        // post-nominal `cat_pp` noun-mod ("vulnerability for …") and the VP-adjunct ("needed
        // for …") — and all three object kinds (name / singular ∃-GQ / bare-plural deferred-Q).
        "therapies are needed for a gene", // VP-adjunct prep, singular GQ object  ⇒ CLOSED
        "WRN is a vulnerability for a gene", // cat_pp noun-mod, singular GQ object ⇒ CLOSED
        "HeLa affects a gene within cells", // bare-plural prep object (one deferred hole) ⇒ open
    ];
    eprintln!("\n=== fragment bisection (closed / open / — ; OOV split out) ===");
    for f in fragments {
        let toks = tokenize(f);
        let ntok = toks.len();
        // OOV-FIRST: a `—` from an unknown lexeme is a VOCABULARY gap, not a grammar gap. Report the
        // missed tokens so the genuine grammar gaps (fully-known, still no parse) are not conflated
        // with OOV (e.g. `WRN` is a gene-symbol OOV — its `—` is NOT a predicate-nominal gap, which
        // the small-lexicon `HeLa is a cell line` parse proves the grammar already covers).
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!(
                "  [{ntok:>2} tok] OOV{:<7} {f:?}  (unknown: {unknown:?})",
                ""
            );
            continue;
        }
        let (closed, open) = index.parse_open(f, &lem);
        let status = if !closed.is_empty() {
            format!("CLOSED×{}", closed.len())
        } else if !open.is_empty() {
            format!("open×{}", open.len())
        } else {
            "GRAMMAR-GAP".to_string()
        };
        eprintln!("  [{ntok:>2} tok] {status:<11} {f:?}");
    }

    // BEAM-PRESSURE probe (records the §2 prep-object residual's cause): "novel therapies are
    // needed for a/an … " is GRAMMAR-GAP at the page beam (64) yet OPENS at a wide beam — so the
    // residual is ambiguity explosion (attributive-adj `novel` over a bare-plural subject + a PP),
    // a Lever-B scale issue (GH #97), NOT a missing prep-object rule (the singular/bare-plural prep
    // objects above already parse). Witnessed: at cell_beam=1024 it yields open×216.
    let wide = Parser::build(Arc::clone(&head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(1024);
    let (wclosed, wopen) = wide.parse_open("novel therapies are needed for a gene", &lem);
    eprintln!(
        "\n=== beam-pressure probe (cell_beam=1024) ===\n  closed×{} open×{}  \"novel therapies are needed for a gene\"",
        wclosed.len(),
        wopen.len()
    );
}

/// Controlled experiment (does contextual SENSE reranking rescue a STRUCTURAL-ambiguity residual?):
/// parse "novel therapies are needed for a gene" at the PAGE beam (64) — the exact config where it is
/// GRAMMAR-GAP cap-only — using whatever reranker `build_index` wires. Built without `--features
/// use-llm` ⇒ cap-only (baseline GRAMMAR-GAP). Built `--features use-llm` with `ANTHROPIC_API_KEY` ⇒ the
/// live `AnthropicSenseRanker` reorders the over-cap words' senses in sentence context. Hypothesis
/// (Declared): no rescue, because the explosion is derivational (Σ-refine × bare-plural shift × PP
/// attachment) over already-≤2 senses, and the cell beam ranks DERIVATIONS, which the sense ranker
/// never touches. Run live:
///     cargo test -p eigenius-wordnet --features use-llm --test db_backed_encoding \
///         llm_reranker_on_structural_residual -- --ignored --nocapture
#[test]
#[ignore = "live-LLM experiment; needs a snapshot and (for the on-arm) --features use-llm + ANTHROPIC_API_KEY"]
fn llm_reranker_on_structural_residual() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head); // wires the live LLM reranker iff --features use-llm + key
    let lem = morphy();
    let sentence = "novel therapies are needed for a gene";
    let t = std::time::Instant::now();
    let (closed, open) = index.parse_open(sentence, &lem);
    let status = if !closed.is_empty() {
        format!("CLOSED×{}", closed.len())
    } else if !open.is_empty() {
        format!("open×{}", open.len())
    } else {
        "GRAMMAR-GAP".to_string()
    };
    eprintln!(
        "\n=== LLM-reranker @ page beam (64): {status} [{:.1}s] {sentence:?} ===",
        t.elapsed().as_secs_f64()
    );
}

/// De-risk gate: the store opens, the chain resumes, and the `lexicon:form` value-index is ACTIVE
/// (→ lazy Parser path; the eager full-chain scan would OOM on 7.6M resources). Cheap — runs
/// by default (not `#[ignore]`d) so the harness wiring stays green even without the heavy run.
#[test]
fn snapshot_opens_with_lazy_form_index() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };

    // Chain depth (walk parent pointers) — a sanity signal the full chain resumed.
    let mut depth = 0usize;
    let mut cur = Some(head.clone());
    while let Some(layer) = cur {
        depth += 1;
        cur = layer.parent().cloned();
    }
    eprintln!("snapshot chain depth (layers): {depth}");

    let form = Iri::parse("urn:eigenius:lexicon:form").unwrap();
    let actives = resolve_active_value_indexes(&head);
    let active_props: Vec<&str> = actives.iter().map(|a| a.target_property.as_str()).collect();
    eprintln!("active value indexes: {active_props:?}");
    assert!(
        actives.iter().any(|a| a.target_property == form),
        "lexicon:form value-index must be active for the lazy path; active = {active_props:?}"
    );

    let index = Parser::build(Arc::clone(&head));
    assert!(
        index.has_token("gene", &Identity),
        "the full WordNet lexicon must know 'gene'"
    );
}

/// (d) — the measurement: feed the cleaned WRN first page through the parser over the FULL
/// WordNet+UMLS store, and report the outcome distribution + OOV fix-buckets. Heavy (full lexicon,
/// long sentences); `#[ignore]`d, run manually:
///
///     cargo test -p eigenius-wordnet --test db_backed_encoding \
///         wrn_first_page_over_full_lexicon -- --ignored --nocapture
#[test]
#[ignore = "heavy DB-backed (d) measurement; run with --ignored --nocapture"]
fn wrn_first_page_over_full_lexicon() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    // The page path is overridable (`EIGENIUS_WRN_PAGE`) so the same measurement can run against a
    // controlled-language rewrite (D62 CNL experiment, `first-page-cnl.txt`) for a coverage A/B.
    let page_path = std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let page = match std::fs::read_to_string(&page_path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("SKIP: {page_path} not found");
            return;
        }
    };
    eprintln!("measuring page: {page_path}");

    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    // Stage A — the document augmentation (D63 lexicon-augmentation §6a, this session): ground OOV atoms
    // against the form/description text indexes and OVERLAY the groundings onto the index, so the parser
    // SEES them (uncommitted, doc-scoped — the §7-2 in-memory-overlay path) instead of gapping on them.
    // Deterministic proposers here (reproducible A/B); the live LLM abbreviation/POS proposers are
    // drop-in behind the traits (exercised by the `--features use-llm` smoke tests).
    let mut aug = {
        use eigenius_kernel::dcg::{
            augment_lexicon_backed, NoAbbreviationProposer, NominalCategoryProposer,
        };
        augment_lexicon_backed(
            &head,
            &page,
            &NoAbbreviationProposer,
            &NominalCategoryProposer,
            &lem,
        )
    };
    // Named-entity source (D63 `d63-named-entity-glossary-source.md`): recognize `<common-noun-head>
    // <Name>` appositions ("Project Achilles", "project DRIVE") and OVERLAY them as doc-local named
    // individuals (`cat_np`) via the SAME in-memory augmentation — closes the "project"(N/V)-crowding
    // grammar gap without a persistent doc layer.
    let ne_aug = eigenius_kernel::dcg::named_entity_augmentation(&head, &page);
    let n_names = ne_aug.added.len();
    eprintln!(
        "named entities: {:?}",
        ne_aug
            .added
            .iter()
            .map(|b| b.provenance.surface.clone())
            .collect::<Vec<_>>()
    );
    aug.added.extend(ne_aug.added);
    aug.supporting.extend(ne_aug.supporting);
    // Abbreviation glossary (D63 Defect 2b): the CNL uses acronyms (MSI/MSS) whose DEFINITIONS live in
    // the ORIGINAL page ("microsatellite instability (MSI)", "microsatellite stable (MSS)") — not the
    // parsed CNL. Run the abbreviation extraction on the SOURCE document (Schwartz-Hearst, plus the live
    // LLM proposer under `use-llm` for non-parenthetical introductions) so `MSS` grounds to
    // microsatellite-stable rather than the `C0024814` Marinesco-Sjogren acronym collision. Merged into
    // the same doc-scoped overlay; source = WRN_PAGE (the cleaned original) by default, `page` is the
    // parsed CNL, so the definitions come from the source and bind the CNL's acronym surfaces.
    let source_path = std::env::var("EIGENIUS_WRN_SOURCE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let source_text = std::fs::read_to_string(&source_path).unwrap_or_else(|_| page.clone());
    let abbr_aug = {
        #[cfg(feature = "use-llm")]
        {
            match eigenius_kernel::dcg::AnthropicAbbreviationProposer::from_env() {
                Some(p) => {
                    eprintln!(
                        "abbreviation proposer: AnthropicAbbreviationProposer (live) on source"
                    );
                    eigenius_kernel::dcg::augment_document_only(&head, &source_text, &p, &lem)
                }
                None => eigenius_kernel::dcg::augment_document_only(
                    &head,
                    &source_text,
                    &eigenius_kernel::dcg::NoAbbreviationProposer,
                    &lem,
                ),
            }
        }
        #[cfg(not(feature = "use-llm"))]
        {
            eigenius_kernel::dcg::augment_document_only(
                &head,
                &source_text,
                &eigenius_kernel::dcg::NoAbbreviationProposer,
                &lem,
            )
        }
    };
    eprintln!(
        "abbreviations: {:?}",
        abbr_aug
            .added
            .iter()
            .map(|b| b.provenance.surface.clone())
            .collect::<Vec<_>>()
    );
    aug.added.extend(abbr_aug.added);
    aug.supporting.extend(abbr_aug.supporting);
    eprintln!(
        "augmentation: {} OOV grounded + injected, {n_names} named-entity individual(s), {} residual OOV",
        aug.added.len() - n_names,
        aug.missing_oov.len()
    );
    // Reranker CONTEXT WINDOW — OFF by default, opt-in via `EIGENIUS_CONTEXT_SENTENCES` (the
    // `--context-window` arm). When on, the ranker sees `window` sentences on each side, so a sense
    // plausible in isolation but wrong in context (UMLS "Geographic Locations" for "regions" in a
    // genomics page) can be eliminated. It CHANGES the ranker's question, so a ranks.json recorded
    // under a different window MISSES — `assert_replay_faithful` makes that fatal, not silent.
    let ctx_window: usize = std::env::var("EIGENIUS_CONTEXT_SENTENCES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let index =
        build_index_over(&head, Some(&aug)).with_document(segment_sentences(&page), ctx_window);
    eprintln!(
        "context window: {ctx_window} sentence(s) each side ({})",
        if ctx_window == 0 {
            "off — isolated-sentence ranking"
        } else {
            "on"
        }
    );

    // Characterize a few interesting buckets directly (closed-class vs -ly adverb vs domain).
    for probe in [
        "the",
        "we",
        "their",
        "would",
        "commonly",
        "typically",
        "recq",
        "wilcoxon",
    ] {
        eprintln!("  has_token({probe:?}) = {}", index.has_token(probe, &lem));
    }

    // Page-level ambiguity roll-up (set `EIGENIUS_ATTRIBUTION_ROLLUP`): aggregate every unit's
    // per-span sense/structure branch sites into ranked levers — which surface form drives the most
    // sense multiplicity, which named construction the most structural branching, across the page.
    let rollup = std::env::var("EIGENIUS_ATTRIBUTION_ROLLUP").is_ok();
    if rollup {
        eigenius_kernel::dcg::attribution::begin();
    }

    let mut report: Vec<UnitReport> = Vec::new();
    for (i, text) in segment_sentences(&page).into_iter().enumerate() {
        let ntok = tokenize(&text).len();
        let t = std::time::Instant::now();
        let outcome = encode_unit(&text, &index, &lem, &head);
        eprintln!(
            "[unit {i:>2}, {ntok:>3} tok, {:>5.1}s] {}",
            t.elapsed().as_secs_f64(),
            tag(&outcome)
        );
        // Page-wide English gloss (`EIGENIUS_GLOSS_READINGS=1`) — verbalize each reading for authoring
        // / verifying the expected-reading corpus without reading raw λ-terms.
        //
        // Grouped BY SKELETON, one representative gloss each. Adjudicating a pin means choosing
        // between *bracketings*, and the first-N-readings view could not do that: a unit's leading
        // readings routinely all sit in ONE skeleton (sense variation moves faster than structure),
        // so the competing bracketing never appeared. Printing per skeleton makes the choice the
        // gate actually pins the choice the author sees. `[S0]`/`[S1]` line up with the
        // `EIGENIUS_DUMP_SKELETONS` block, which prints the same set in the same (sorted) order.
        if std::env::var("EIGENIUS_GLOSS_READINGS").is_ok() {
            let vnames = unit_sense_names(&text, &index, &lem, &head);
            let vb = Vb {
                names: &vnames,
                layer: &head,
            };
            eprintln!("  «{}»", text.trim());
            let mut by_skel: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            for it in index.parse(&text, &lem).iter() {
                by_skel
                    .entry(erase_senses(&pretty_term(it.sem())))
                    .or_insert_with(|| verbalize(it.sem(), &vb));
            }
            // `EIGENIUS_GLOSS_MAX` raises the per-unit cap — 8 is enough to adjudicate a 2-5
            // skeleton unit but useless on an outlier (the page's worst carries 204).
            let gmax: usize = std::env::var("EIGENIUS_GLOSS_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8);
            for (i, (skel, gloss)) in by_skel.iter().enumerate().take(gmax) {
                eprintln!("      [S{i}] {skel}");
                eprintln!("       ≈ \"{gloss}\"");
            }
        }
        // RAW readings — the sense-visible λ-term, which is NEITHER of the two artifacts above and
        // the only one that answers a structural question about predicates.
        //
        // A skeleton is sense-ERASED (`v00776059_t` -> `§`) and a gloss is verbalized ENGLISH (the
        // verb renders as the word "cause", a coordination as "or"). So grepping either for a
        // predicate name silently matches nothing, and reads as "0 occurrences" rather than as the
        // category error it is. That has produced three wrong analyses in this corpus -- counting
        // PpOblique relations, counting verb predications, and counting `Or(` nesting -- each time
        // by searching an artifact from which the thing being counted had already been removed.
        // `trace_one_sentence` does print raw readings but runs cap-only WITHOUT the document
        // overlay, so it is not like-for-like with the sweep; this is.
        //
        // `EIGENIUS_DUMP_READINGS=1` (cap: `EIGENIUS_READINGS_MAX`, default 40).
        if std::env::var("EIGENIUS_DUMP_READINGS").is_ok() {
            let rmax: usize = std::env::var("EIGENIUS_READINGS_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(40);
            eprintln!("  RAW «{}»", text.trim());
            // Verbalise alongside the raw term. Adjudicating a reading means deciding whether it is
            // ADMISSIBLE, which needs it in English; and it must be done HERE rather than through
            // `trace_one_sentence`, which runs cap-only without the document overlay and so reports
            // a different reading set — "These data sets are project Achilles and project DRIVE."
            // has 0 readings there and 1 here. The adjudication ledger is keyed on THIS sweep.
            let vnames = unit_sense_names(&text, &index, &lem, &head);
            let vb = Vb {
                names: &vnames,
                layer: &head,
            };
            // Use `parse_open`, not `parse`: the latter returns CLOSED readings only, so the two
            // units with an unresolved referent hole ("MSI cell lines from these four lineages …",
            // "The lines from rare lineages …") dumped nothing at all — 48 and 4 skeletons with no
            // gloss between them, which is most of the largest unit on the page. An OPEN parse is
            // still a reading to adjudicate; it just has a hole the D64 resolver has not filled.
            let (closed_r, open_r) = index.parse_open(&text, &lem);
            if !open_r.is_empty() {
                eprintln!(
                    "      ({} closed, {} OPEN — holes awaiting resolution)",
                    closed_r.len(),
                    open_r.len()
                );
            }
            let all: Vec<&Item> = closed_r
                .iter()
                .chain(open_r.iter().map(|o| &o.item))
                .collect();
            for (i, it) in all.into_iter().enumerate().take(rmax) {
                // Print the SKELETON each reading erases to. The adjudication ledger is keyed on
                // skeletons, but a verdict is formed by reading the ENGLISH — so without this line
                // the two artifacts cannot be joined and the gloss has to be matched by eye.
                eprintln!("      [R{i}] {}", pretty_term(it.sem()));
                // `EIGENIUS_DEBUG_SEM=1` — the CATEGORY and the raw `Exp` beside the pretty form.
                // Worth its keep: the pretty printer renders `logic:And(P, Q)` and an ordinary
                // application identically, so a term that looked like an ill-typed `And(λ…, …)` could
                // not be diagnosed from the printed form alone. The `{:?}` showed it was
                // `Exp::InductiveType(AndDecl{params:[(P,Sort(0)),(Q,Sort(0))]}, [Lam(…), …])`, which
                // located the real defect in `check_type` — an applied inductive type was admitted
                // without checking its parameter arguments. Reach for this before theorising about a
                // skeleton string.
                if std::env::var("EIGENIUS_DEBUG_SEM").is_ok() {
                    eprintln!("           CAT={}", pretty_term(it.cat()));
                    eprintln!("           DBG={:?}", it.sem());
                }
                eprintln!("           sk={}", erase_senses(&pretty_term(it.sem())));
                eprintln!("           ≈ {}", verbalize(it.sem(), &vb));
            }
        }
        report.push(UnitReport { text, outcome });
        // Progress snapshot: the page sweep runs for many minutes, so emit the partial roll-up
        // periodically — an interrupted run still leaves usable attribution in the log.
        if rollup && (i + 1) % 10 == 0 {
            if let Some(s) = eigenius_kernel::dcg::attribution::snapshot() {
                eprint!("[roll-up after {} units]\n{s}", i + 1);
            }
        }
    }

    summarize(&report);
    assert_replay_faithful();

    if rollup {
        if let Some(s) = eigenius_kernel::dcg::attribution::take() {
            eprint!("{s}");
        }
    }
}

fn tag(o: &Outcome) -> &'static str {
    match o {
        Outcome::Encoded { .. } => "ENCODED",
        Outcome::Ambiguous { .. } => "AMBIG",
        Outcome::MissingLexeme { .. } => "MISSING",
        Outcome::GrammarGap => "GRAMMAR-GAP",
        Outcome::Open { .. } => "OPEN",
        Outcome::ScaleBound { .. } => "SCALE-BOUND",
    }
}

fn summarize(report: &[UnitReport]) {
    let (mut enc, mut amb, mut miss, mut gap, mut scale, mut open) = (0, 0, 0, 0, 0, 0);
    let mut oov: BTreeSet<String> = BTreeSet::new();
    for u in report {
        match &u.outcome {
            Outcome::Encoded { .. } => enc += 1,
            Outcome::Ambiguous { .. } => amb += 1,
            Outcome::MissingLexeme { unknown } => {
                miss += 1;
                oov.extend(unknown.iter().cloned());
            }
            Outcome::Open { holes, .. } => {
                open += 1;
                eprintln!(
                    "  open (referent holes={holes}, awaiting resolution): {:?}",
                    u.text
                );
            }
            Outcome::GrammarGap => {
                gap += 1;
                eprintln!("  grammar-gap (all known, no parse): {:?}", u.text);
            }
            Outcome::ScaleBound { ntok } => {
                scale += 1;
                eprintln!("  scale-bound (known, {ntok} tok): {:?}", u.text);
            }
        }
    }
    // Reading-count multiplicity: the total over all units, and the pinned-bucket distribution.
    let readings: Vec<usize> = report.iter().map(|u| unit_readings(&u.outcome)).collect();
    let total_readings: usize = readings.iter().sum();
    // Sense-independent structural multiplicity: distinct bracketings, senses erased. Drift-free (the
    // reranker's sense choices collapse to `§`), so it isolates STRUCTURE from the sense multiplicity
    // that `total_readings` conflates (D63 baseline gates.multiplicity — the tracked lever).
    let total_skeletons: usize = report.iter().map(|u| unit_skeletons(&u.outcome)).sum();
    let max_readings = readings.iter().copied().max().unwrap_or(0);

    // ── Faithfulness: does each curated unit still CONTAIN its expected (correct) reading? ──────────
    // Authoring aid: `EIGENIUS_DUMP_SKELETONS=1` prints every unit's skeleton set, so a correct
    // reading can be picked and pinned into expected-readings.jsonl.
    if std::env::var("EIGENIUS_DUMP_SKELETONS").is_ok() {
        eprintln!("\n===== PER-UNIT SKELETONS (author expected-readings.jsonl from these) =====");
        for u in report {
            let sk = unit_skel_set(&u.outcome);
            eprintln!("«{}»  [{} skeleton(s)]", u.text.trim(), sk.len());
            for s in sk {
                eprintln!("    {s}");
            }
        }
        eprintln!("===== END SKELETONS =====\n");
    }
    let expected = load_expected_readings();
    let (mut exp_hits, mut exp_miss): (usize, Vec<&Expected>) = (0, Vec::new());
    let mut exp_stale: Vec<&Expected> = Vec::new();
    for e in &expected {
        match report.iter().find(|u| u.text.trim() == e.sentence.trim()) {
            None => exp_stale.push(e),
            Some(u) => {
                if unit_skel_set(&u.outcome).iter().any(|s| s == &e.skeleton) {
                    exp_hits += 1;
                } else {
                    exp_miss.push(e);
                }
            }
        }
    }
    let exp_total = expected.len() - exp_stale.len();
    for e in &exp_miss {
        eprintln!(
            "  FAITHFULNESS MISS: «{}» no longer contains its expected reading\n    want: {}\n    ({})",
            e.sentence, e.skeleton, e.note
        );
    }
    for e in &exp_stale {
        eprintln!(
            "  expected-readings: curated sentence not on this page (stale?): «{}»",
            e.sentence
        );
    }

    // Persist the reranker's decisions (if recording) BEFORE the summary, so a run that produced a
    // number always leaves behind the artifact that makes it replayable.
    flush_sense_ranks();
    eprintln!(
        "\n=== WRN first page over FULL lexicon: {} units → encoded {enc}, ambiguous {amb}, \
         open {open}, missing-lexeme {miss}, grammar-gap {gap}, \
         scale-bound (known, >{PARSE_BUDGET} tok) {scale}, total-readings {total_readings}, \
         total-skeletons {total_skeletons} (sense× {:.2}), \
         expected-hits {exp_hits}, expected-curated {exp_total} ===",
        report.len(),
        total_readings as f32 / total_skeletons.max(1) as f32
    );
    // Reading-count histogram (PINNED buckets, [`READING_BUCKETS`]) — the multiplicity distribution.
    // `eval-parse-rate.sh` parses these `histogram:` lines; keep the format stable.
    eprintln!(
        "reading-count histogram ({} units, max {max_readings}):",
        report.len()
    );
    for &(label, lo, hi) in READING_BUCKETS {
        let n = readings.iter().filter(|&&c| c >= lo && c <= hi).count();
        eprintln!("  histogram: {label:<12} {n}");
    }
    eprintln!("distinct OOV tokens ({}): {oov:?}", oov.len());

    let per_unit: Vec<usize> = report
        .iter()
        .filter_map(|u| match &u.outcome {
            Outcome::MissingLexeme { unknown } => Some(unknown.len()),
            _ => None,
        })
        .collect();
    if !per_unit.is_empty() {
        let sum: usize = per_unit.iter().sum();
        let n1 = per_unit.iter().filter(|&&c| c == 1).count();
        eprintln!(
            "OOV-per-unit: min {}, max {}, mean {:.1}; units blocked by exactly 1 OOV: {n1}",
            per_unit.iter().min().unwrap(),
            per_unit.iter().max().unwrap(),
            sum as f64 / per_unit.len() as f64
        );
    }

    // Bucket the distinct OOV by the fix that recovers it.
    let connectives: BTreeSet<&str> = [
        "after", "also", "although", "as", "because", "between", "both", "however", "most",
        "several", "such", "these", "those", "to", "within", "yet", "alone",
    ]
    .into_iter()
    .collect();
    let (mut adverb_ly, mut stat_leak, mut connective, mut domain) = (0, 0, 0, 0);
    for t in &oov {
        if t.chars().count() <= 1 {
            stat_leak += 1;
        } else if t.ends_with("ly") {
            adverb_ly += 1;
        } else if connectives.contains(t.as_str()) {
            connective += 1;
        } else {
            domain += 1;
        }
    }
    eprintln!(
        "OOV by fix-bucket: domain-lexicon {domain}, connectives/function-words {connective}, \
         -ly adverbs {adverb_ly}, stat-symbol leaks {stat_leak}"
    );

    eprintln!("\n--- encoded / ambiguous units (the wins) ---");
    for u in report {
        let t: String = u.text.chars().take(100).collect();
        match &u.outcome {
            Outcome::Encoded { is_prop, .. } => eprintln!("  [ENCODED prop={is_prop}] {t}…"),
            Outcome::Ambiguous { count, is_prop, .. } => {
                eprintln!("  [AMBIG×{count} prop={is_prop}] {t}…")
            }
            _ => {}
        }
    }
}

/// PROBE (D63 next-lever diagnosis): is the prep-verb grammar-gap on CNL-v2 caused by the WordNet
/// importer DROPPING the PP-complement (a documented stage-1 loss — `convert.rs::classify` maps the
/// oblique frames 4/13/22 to Intransitive/Transitive with the preposition discarded), or by the
/// preposition simply not attaching? Minimal pairs over common WordNet verbs/nouns disentangle it:
/// prep-verb (V + obligatory PP, expect GAP if the complement is unmodelled); the SAME verb bare (no
/// PP, should parse — the intransitive frame IS emitted); the same verb + a DIFFERENT prep as a
/// VP-adjunct (isolates whether ANY PP attaches); a transitive control (NPs/lexemes known-good).
///
/// Cap-only (the LLM reranker is irrelevant to a grammar/lexicon probe). `#[ignore]`d; run:
///   EIGENIUS_DB_SNAPSHOT=<snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_prep_verb_gap -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; run with --ignored --nocapture"]
fn probe_prep_verb_gap() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    eprintln!("── token knownness (function words + probe verbs) ──");
    for t in [
        "from",
        "to",
        "in",
        "of",
        "arise",
        "result",
        "respond",
        "contribute",
        "occur",
        "cause",
    ] {
        eprintln!("  has_token({t:?}) = {}", index.has_token(t, &lem));
    }

    let probe = |label: &str, s: &str| {
        let toks = tokenize(s);
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!("  [{label:<20}] OOV {unknown:?} :: {s:?}");
            return;
        }
        let (c, o) = index.parse_open(s, &lem);
        let verdict = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{label:<20}] {verdict:<9} :: {s:?}");
    };

    eprintln!("\n── prep-verb complement (V + obligatory PP) ──");
    probe("prep result-from", "diseases result from mutations");
    probe("prep arise-from", "cancers arise from mutations");
    probe("prep respond-to", "cells respond to genes");
    probe("prep contribute-to", "genes contribute to cancers");
    eprintln!("── bare intransitive (same verb, no PP) ──");
    probe("bare result", "diseases result");
    probe("bare arise", "cancers arise");
    probe("bare respond", "cells respond");
    probe("bare contribute", "genes contribute");
    eprintln!("── intransitive + a DIFFERENT prep as VP-adjunct ──");
    probe("adj arise-in", "cancers arise in cells");
    probe("adj occur-in", "cancers occur in cells");
    eprintln!("── transitive control (lexemes/NPs known-good) ──");
    probe("tv cause", "mutations cause cancers");

    // The prep-verb mechanism PARSES (above), so the real blocker is elsewhere. Run the ACTUAL
    // CNL-v2 grammar-gap sentences (which gapped on FULL-UMLS) here on this snapshot: if they PARSE,
    // the FULL-UMLS gap was a lexicon-crowding beam artifact, not a grammar gap; if they GAP here
    // too, bisect one element at a time (subject / compound object / modal / negation / determiner).
    eprintln!("\n── knownness for the actual-gap tokens ──");
    for t in [
        "msi",
        "lynch",
        "syndrome",
        "several",
        "can",
        "do",
        "not",
        "deficient",
        "mismatch",
        "repair",
        "immune",
        "checkpoint",
        "blockade",
        "regions",
        "microsatellites",
    ] {
        eprintln!("  has_token({t:?}) = {}", index.has_token(t, &lem));
    }
    eprintln!("\n── actual CNL-v2 gap sentences (gapped on FULL-UMLS) ──");
    probe(
        "gap MSI-result",
        "MSI results from deficient DNA mismatch repair",
    );
    probe("gap MSI-contrib", "MSI contributes to several cancers");
    probe("gap MSI-can-arise", "MSI can arise from Lynch syndrome");
    probe("gap respond-neg", "some cancers do not respond to genes");
    probe("gap copula-plural", "regions are microsatellites");
    eprintln!("── bisect: MSI subject vs plural, simple vs compound object ──");
    probe("bis MSI+simple", "MSI results from mutations");
    probe(
        "bis plural+compound",
        "cancers result from deficient DNA mismatch repair",
    );
    probe("bis MSI+medium", "MSI results from repair");
    eprintln!("── bisect: modal / negation / determiner in isolation ──");
    probe("bis modal", "cancers can arise from mutations");
    probe("bis negation", "cancers do not respond to genes");
    probe("bis determiner", "genes contribute to several cancers");
    eprintln!("── bisect: is `MSI` a usable subject NP at all? ──");
    probe("bis MSI-bare-tv", "MSI causes cancers");
    probe("bis MSI-copula", "MSI is a disease");
    eprintln!(
        "── confirm mechanism: does a DETERMINER rescue the abbreviation? (→ cat_n, not a name) ──"
    );
    probe("det the-MSI-tv", "the MSI causes cancers");
    probe("det the-MSI-cop", "the MSI is a disease");
    probe("wrn-bare-cop", "WRN is a gene");
    probe("wrn-det-cop", "the WRN is a gene");
    eprintln!("── contrast: a DEMO named individual (HeLa) as bare subject, if present ──");
    probe("hela-bare", "HeLa is a gene");
}

/// PROBE (D63 next-lever #2): are the comparative grammar-gaps a genuine construction gap? The
/// CNL-v2 gaps `greater/fewer/stronger … than`, `compared favourably to` all involve comparatives.
/// Isolate the construction over clean bare-plural subjects / known nouns (so a gap is the comparative
/// itself, not the MSI-subject or compound-object confounds already diagnosed). Cap-only; `#[ignore]`d:
///   EIGENIUS_DB_SNAPSHOT=<snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_comparatives -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; run with --ignored --nocapture"]
fn probe_comparatives() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    eprintln!("── knownness (comparative function words + -er forms) ──");
    for t in [
        "than",
        "more",
        "less",
        "greater",
        "fewer",
        "stronger",
        "larger",
        "large",
        "strong",
        "essential",
        "common",
        "compared",
        "favourably",
        "dependence",
        "phenotype",
        "mutations",
    ] {
        eprintln!("  has_token({t:?}) = {}", index.has_token(t, &lem));
    }

    let probe = |label: &str, s: &str| {
        let toks = tokenize(s);
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!("  [{label:<22}] OOV {unknown:?} :: {s:?}");
            return;
        }
        let (c, o) = index.parse_open(s, &lem);
        let verdict = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{label:<22}] {verdict:<9} :: {s:?}");
    };

    eprintln!("\n── baseline: bare predicative adjective (control, should parse) ──");
    probe("base large", "genes are large");
    probe("base essential", "genes are essential");
    eprintln!("── predicative comparative (X is [more] ADJ than Y) ──");
    probe("pred -er than", "genes are larger than cells");
    probe("pred more-adj than", "genes are more essential than cells");
    probe("pred strong-er than", "cells are stronger than genes");
    eprintln!("── attributive comparative adjective (a STRONGER N, no `than`) ──");
    probe("attr stronger-N", "cells require a stronger phenotype");
    probe("attr greater-mass", "cells show greater dependence");
    eprintln!("── comparative quantifier over NPs (fewer/greater N than N) ──");
    probe(
        "quant fewer-than",
        "cells contain fewer mutations than genes",
    );
    probe(
        "quant greater-than",
        "cells show greater dependence than genes",
    );
    eprintln!("── comparative verb (compared [ADV] to) ──");
    probe("vb compared-fav-to", "cancers compared favourably to genes");
    probe("vb compared-to", "genes compared to cells");
}

/// PROBE (D63): Derive the CAUSE of the remaining CNL-v2 grammar-gaps (sentences not already pinned to
/// the MSI-subject / `than NP` levers). Minimal pairs over clean known vocab isolate each hypothesized
/// construction; the load-bearing one is (G) — whether a domain abbreviation as an attributive
/// MODIFIER (`MSI cells`, `WRN dependency`) also fails, which would widen the abbreviation lever.
/// Cap-only; `#[ignore]`d:
///   EIGENIUS_DB_SNAPSHOT=<snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_gap_tail -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; run with --ignored --nocapture"]
fn probe_gap_tail() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    eprintln!("── knownness ──");
    for t in [
        "msi",
        "wrn",
        "mmr",
        "dependency",
        "inactivation",
        "somatic",
        "independent",
        "target",
        "targets",
        "region",
        "regions",
        "process",
        "state",
        "lineages",
        "checkpoint",
        "blockade",
        "evaluated",
        "identified",
        "analysed",
        "queried",
        "arises",
        "as",
    ] {
        eprintln!("  has_token({t:?}) = {}", index.has_token(t, &lem));
    }

    let probe = |label: &str, s: &str| {
        let toks = tokenize(s);
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !index.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!("  [{label:<24}] OOV {unknown:?} :: {s:?}");
            return;
        }
        let (c, o) = index.parse_open(s, &lem);
        let verdict = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  [{label:<24}] {verdict:<9} :: {s:?}");
    };

    eprintln!("\n── G. abbreviation as attributive MODIFIER (sents 3/14/16/17/19) ──");
    probe("G MSI-mod-plural", "MSI cells contain genes");
    probe("G WRN-mod-subject", "WRN genes cause cancers");
    probe("G MMR-mod-subject", "MMR mutations cause cancers");
    probe("G control N-N", "cancer cells contain genes");
    eprintln!("── A. `as`-predicative (X V Y as Z) (sents 14/15) ──");
    probe("A evaluated-as", "cells evaluated genes as targets");
    probe("A identified-as", "cells identified genes as targets");
    eprintln!("── B. plural copula predicate-nominal (sent 4) ──");
    probe("B plural-predn", "regions are genes");
    probe("B control sg-predn", "a region is a gene");
    eprintln!("── C. PP-stack in object (X V Y in Z with W) (sents 1/13) ──");
    probe("C pp-stack", "cells query genes in cancers with mutations");
    probe("C control 1pp", "cells query genes in cancers");
    eprintln!("── D. numeral + adjective + N-N compound (sent 12) ──");
    probe("D bare", "cells analysed targets");
    probe("D N-N compound", "cells analysed cancer dependency targets");
    probe("D numeral+adj", "cells analysed two independent targets");
    eprintln!("── E. compound-noun prep object (sent 11) ──");
    probe(
        "E compound-obj",
        "cancers respond to immune checkpoint blockade",
    );
    eprintln!("── F. modal + or-coordination of objects (sent 19) ──");
    probe("F modal-or", "genes may require cells or mutations");
    eprintln!("── H. adjective-modified subject + prep-verb (sents 9/3) ──");
    probe(
        "H adj-subj-prepverb",
        "somatic inactivation arises from mutations",
    );
    probe(
        "H that-essential-in",
        "cells found that genes were essential in cells",
    );
}

/// PROBE (D63): are the residual CNL-v2 grammar-gaps (sentences whose constituent constructions all
/// PARSE in isolation) genuine grammar gaps, or full-UMLS beam/lexicon-crowding artifacts? Run the
/// actual sentences VERBATIM on the SUBSET (fewer senses) at the default beam (64, widen→512) and at a
/// wide fixed beam (2048, above the widen ceiling), and compare to their known FULL-UMLS GAP:
/// parses on subset@64 → the full-UMLS gap was LEXICON-CROWDING (extra senses), not grammar; gaps@64
/// but parses@2048 → BEAM-CEILING (the 512 widen cap is too low); gaps at both → a GENUINE grammar gap.
///
/// Cap-only; `#[ignore]`d:
///   EIGENIUS_DB_SNAPSHOT=<subset-snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       probe_beam_crowding -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; run with --ignored --nocapture"]
fn probe_beam_crowding() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let def = build_index(&head); // CELL_BEAM=64, widen→512
    let wide = Parser::build(Arc::clone(&head))
        .with_sense_cap(SENSE_CAP)
        .with_cell_beam(2048); // above CELL_BEAM_WIDEN_MAX → a fixed wide beam

    let verdict = |idx: &Parser, s: &str| {
        let (c, o) = idx.parse_open(s, &lem);
        if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        }
    };

    for (label, s) in [
        (
            "sent3 found-that",
            "We found that WRN was selectively essential in MSI models",
        ),
        (
            "sent12 two-indep",
            "We analysed two independent cancer dependency data sets",
        ),
        (
            "sent19 may-require",
            "WRN dependency may require specific lineages or a stronger mutation phenotype",
        ),
    ] {
        let toks = tokenize(s);
        let unknown: Vec<String> = toks
            .iter()
            .filter(|t| !is_nonprose(t) && !def.has_token(t, &lem))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            eprintln!("  [{label:<20}] OOV {unknown:?} (can't test on subset) :: {s:?}");
            continue;
        }
        eprintln!(
            "  [{label:<20}] subset@64→512={:<10} subset@2048={:<10} (full-UMLS: GAP)",
            verdict(&def, s),
            verdict(&wide, s),
        );
    }
}

/// PHASE 1 MEASUREMENT (D63 `d63-document-preprocessing-scope.md`): run the deterministic Stage-A
/// pipeline against the served snapshot and measure the recovery. Extract `Long Form (SHORT)`
/// definitions from the ORIGINAL page (which carries `microsatellite instability (MSI)` — the CNL-v2
/// rewrite dropped it), ground each long form to its concept, emit the doc-glossary resources, PERSIST
/// them as a chained layer on the SAME backend (so the value index populates and the index resolves
/// lazily — an in-memory overlay OOMs via the eager full-chain scan, §7-2), then compare base vs
/// glossary on the MSI-subject sentences that gapped in the diagnosis. Run:
///   EIGENIUS_DB_SNAPSHOT=<snap> cargo test -p eigenius-wordnet --test db_backed_encoding \
///       measure_abbreviation_glossary -- --ignored --nocapture
#[test]
#[ignore = "DB-backed Phase-1 measurement; run with --ignored --nocapture"]
fn measure_abbreviation_glossary() {
    let Some(path) = snapshot_path() else { return };
    if !std::path::Path::new(DICT).join("data.noun").exists() {
        eprintln!("SKIP: WordNet dict not found under {DICT}");
        return;
    }
    // Open the store keeping the BACKEND (to persist the doc-glossary layer onto it).
    let store = Arc::new(RocksStore::open(&path).expect("open RocksStore snapshot"));
    let backend: Arc<dyn PersistentBackend> = store;
    let ctx = match bootstrap_persistent(Arc::clone(&backend)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP: cannot resume the snapshot — {e:?}");
            return;
        }
    };
    let head = Arc::clone(ctx.head());
    let lem = morphy();

    // The ORIGINAL page carries the `Long Form (ABBR)` definitions. `EIGENIUS_WRN_PAGE` overrides.
    let page_path = std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let page = std::fs::read_to_string(&page_path).unwrap_or_default();

    // Stage A: extract → ground (ranked cross-check + fuller candidate) → emit (fresh class on miss).
    let defs = extract_abbreviations(&page);
    eprintln!("extracted {} abbreviation definition(s):", defs.len());
    for d in &defs {
        match ground_abbreviation(&head, &d.short_form, &d.long_form, &d.context) {
            Some(c) => eprintln!(
                "  {:<8} ← {:<32?} → {}",
                d.short_form,
                d.long_form,
                c.as_str()
            ),
            None => eprintln!(
                "  {:<8} ← {:<32?} → (miss → fresh doc-local class)",
                d.short_form, d.long_form
            ),
        }
    }
    let resources = glossary_resources(&head, &defs);

    // Build + persist the doc-glossary layer on the SAME backend.
    let mut b = LayerBuilder::new("doc-glossary", Some(Arc::clone(&head)));
    for r in resources {
        b.add_resource(r).expect("add glossary resource");
    }
    let doc_layer = Arc::new(b.build(LayerStorage::with_persistent(Arc::clone(&backend))));
    backend
        .store_layer(&doc_layer)
        .expect("persist doc-glossary layer");
    eprintln!(
        "\ndoc-glossary layer persisted ({} definition(s))\n",
        defs.len()
    );

    let base = build_index(&head);
    let glossary = build_index(&doc_layer);
    let verdict = |idx: &Parser, s: &str| {
        let (c, o) = idx.parse_open(s, &lem);
        if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("OPEN×{}", o.len())
        } else {
            "GAP".to_string()
        }
    };

    let sentences = [
        // MSI — "microsatellite instability", head noun `instability` is mass.
        "MSI is a disease",
        "MSI causes cancers",
        "MSI contributes to several cancers",
        "MSI can arise from Lynch syndrome",
        // MMR — "DNA mismatch repair", head noun `repair` is mass.
        "MMR is deficient in cancers",
        "MMR contributes to cancers",
    ];
    // Post-reshape: a mass-phenomenon abbreviation grounds to a CLASS → the alias emits `cat_n(C, mass)`,
    // and a bare subject shifts to the CLOSED kind-predication `kind_of(C)` (no named individual, no
    // deferred hole). So recovery should be GAP → CLOSED, not GAP → OPEN.
    let (mut recovered, mut closed) = (0usize, 0usize);
    eprintln!(
        "── base (bare MSI/MMR = raw UMLS cat_n count noun → no bare-subject shift) vs glossary \
         (mass alias → kind_of, closes via the kind shift) ──"
    );
    for s in sentences {
        let (bv, gv) = (verdict(&base, s), verdict(&glossary, s));
        let flag = if bv == "GAP" && gv.starts_with("CLOSED") {
            recovered += 1;
            closed += 1;
            "  ← RECOVERED (closed)"
        } else if bv == "GAP" && gv != "GAP" {
            recovered += 1;
            "  ← RECOVERED (open)"
        } else {
            ""
        };
        eprintln!("  base={bv:<10} glossary={gv:<10} :: {s:?}{flag}");
    }
    // Witness a recovered sem — a CLOSED kind-predication `kind_of(<CUI>)`, not a reified individual.
    if let Some(p) = glossary
        .parse("MSI contributes to several cancers", &lem)
        .first()
    {
        eprintln!(
            "\n  sem(\"MSI contributes to several cancers\") = {}",
            pretty_term(p.sem())
        );
    }
    eprintln!(
        "\nrecovered {recovered}/{} abbreviation sentences ({closed} as CLOSED kind-predications) via \
         the glossary",
        sentences.len()
    );
}

/// **What is actually driving the ambiguity?** Factor each unit's readings into
/// `structural skeletons × sense combinations`, and check whether any residual cross-lexicon
/// duplication survives (two readings differing ONLY in a `umls:` vs `wn:` sense of one word).
///
///   EIGENIUS_DB_SNAPSHOT=<store> cargo test --release -p eigenius-wordnet --features use-llm \
///     --test db_backed_encoding factor_ambiguity -- --ignored --nocapture
#[test]
#[ignore = "DB-backed; --ignored --nocapture"]
fn factor_ambiguity() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    let page_path = std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let page = std::fs::read_to_string(&page_path).expect("page");

    // Erase every sense IRI's local segment, leaving the STRUCTURE. `wn:n00024720` / `umls:C1442792`
    // both become `§`, so two readings that differ only in which lexicon's copy of one concept they
    // chose collapse to the same skeleton.
    fn erase(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            if c == ':' {
                // consume an IRI-ish local segment
                let rest = &s[i + 1..];
                let n = rest
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.')
                    .unwrap_or(rest.len());
                if n >= 4 {
                    out.push('§');
                    for _ in 0..n {
                        it.next();
                    }
                    continue;
                }
            }
            out.push(c);
        }
        out
    }

    println!(
        "\n{:<52} {:>6} {:>9} {:>7}",
        "unit", "reads", "skeletons", "sense×"
    );
    println!("{}", "-".repeat(78));
    let (mut tr, mut tk) = (0usize, 0usize);
    let mut rows: Vec<(usize, usize, String)> = Vec::new();
    for text in segment_sentences(&page) {
        let f = index.parse(&text, &lem);
        if f.len() < 2 {
            continue;
        }
        let sems: Vec<String> = f.iter().map(|it| pretty_term(it.sem())).collect();
        let skels: std::collections::BTreeSet<String> = sems.iter().map(|s| erase(s)).collect();
        tr += f.len();
        tk += skels.len();
        rows.push((f.len(), skels.len(), text.chars().take(50).collect()));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    for (r, k, t) in rows.iter().take(12) {
        println!("{:<52} {:>6} {:>9} {:>7.1}", t, r, k, *r as f32 / *k as f32);
    }
    println!("{}", "-".repeat(78));
    println!(
        "TOTAL readings {tr}   distinct skeletons {tk}   ⇒ sense× = {:.2}",
        tr as f32 / tk as f32
    );
    println!(
        "\nIf the cross-lexicon duplicates are gone, sense× ≈ 1 and the readings are STRUCTURAL.\n\
         If sense× is still large, senses are still multiplying."
    );
}

/// **Deep dive on the NEAR-ENCODED units** — every unit with ≤ 16 readings. These are the ones
/// closest to a single reading, so what separates them from ENCODED is the most informative thing
/// the corpus can tell us.
///
/// For each: the readings, the distinct STRUCTURAL skeletons (senses erased), and — when the
/// skeleton count is 1 — the actual pairwise difference between the readings, which is then a pure
/// SENSE choice and can be read off directly.
///
///   EIGENIUS_DB_SNAPSHOT=<store> EIGENIUS_SENSE_RANKS=<ranks.json> \
///     cargo test --release -p eigenius-wordnet --features use-llm --test db_backed_encoding \
///     dive_near_encoded -- --ignored --nocapture
#[test]
#[ignore = "DB-backed; --ignored --nocapture"]
fn dive_near_encoded() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    let page_path = std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| WRN_PAGE.to_string());
    let page = std::fs::read_to_string(&page_path).expect("page");

    /// Erase every SENSE identifier, leaving the combinatory STRUCTURE only, so that two readings
    /// differing only in *which sense* fills a slot collapse to one skeleton.
    ///
    /// Pass 1 handles `X:sense` suffixes (`ΣG#0:n00024720 → ΣG#0§`). Pass 2 handles the ones pass 1
    /// misses: a sense that appears as a **bare function argument** (`kind_of(C0920269)`) or inside a
    /// **predicate name** (`v02624263_i`, `deg_a00494409`) — any run of ≥4 digits (CUIs, WordNet
    /// offsets, synset numbers) → `§`, keeping the categorial part of the name (`v§_i`, `deg_a§`).
    /// `G#N` structural variables have <4 digits and are untouched. Without pass 2 a bare-argument
    /// sense pair (a cross-lexicon duplicate as `kind_of(C…)` vs `kind_of(n…)`, or a verb-sense split
    /// `v00339738_i` vs `v02624263_i`) is miscounted as a distinct *structural* skeleton.
    fn erase(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            if c == ':' {
                let rest = &s[i + 1..];
                let n = rest
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.')
                    .unwrap_or(rest.len());
                if n >= 4 {
                    out.push('§');
                    for _ in 0..n {
                        it.next();
                    }
                    continue;
                }
            }
            out.push(c);
        }
        // Pass 2: collapse bare sense-id digit runs of length >= 4.
        let mut result = String::new();
        let mut run = String::new();
        for c in out.chars() {
            if c.is_ascii_digit() {
                run.push(c);
                continue;
            }
            if !run.is_empty() {
                if run.len() >= 4 {
                    result.push('§');
                } else {
                    result.push_str(&run);
                }
                run.clear();
            }
            result.push(c);
        }
        if run.len() >= 4 {
            result.push('§');
        } else {
            result.push_str(&run);
        }
        result
    }

    /// The tokens that differ between two readings — the actual locus of the ambiguity.
    fn diff(a: &str, b: &str) -> Vec<(String, String)> {
        let (ta, tb): (Vec<&str>, Vec<&str>) = (
            a.split(['(', ')', ' ', ','])
                .filter(|t| !t.is_empty())
                .collect(),
            b.split(['(', ')', ' ', ','])
                .filter(|t| !t.is_empty())
                .collect(),
        );
        if ta.len() != tb.len() {
            return vec![("<different shape>".into(), String::new())];
        }
        ta.iter()
            .zip(tb.iter())
            .filter(|(x, y)| x != y)
            .map(|(x, y)| ((*x).to_string(), (*y).to_string()))
            .collect()
    }

    // The reading-count window to dive into: `2..=EIGENIUS_DIVE_MAX` (default 16). Raise it to
    // inspect the high-multiplicity units (`EIGENIUS_DIVE_MAX=100`). `EIGENIUS_DIVE_ONLY=<substring>`
    // restricts the dive to units whose text contains the substring (so a single unit can be dumped).
    let dive_max: usize = std::env::var("EIGENIUS_DIVE_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    let dive_only = std::env::var("EIGENIUS_DIVE_ONLY").ok();
    let mut n_dive = 0;
    for text in segment_sentences(&page) {
        if let Some(sub) = &dive_only {
            if !text.contains(sub.as_str()) {
                continue;
            }
        }
        let f = index.parse(&text, &lem);
        if f.len() < 2 || f.len() > dive_max {
            continue;
        }
        n_dive += 1;
        let sems: Vec<String> = f.iter().map(|it| pretty_term(it.sem())).collect();
        let skels: std::collections::BTreeSet<String> = sems.iter().map(|s| erase(s)).collect();
        println!("\n════════════════════════════════════════════════════════════════════");
        println!("{text}");
        println!(
            "  {} readings  |  {} structural skeleton(s)  |  sense× = {:.1}",
            f.len(),
            skels.len(),
            f.len() as f32 / skels.len() as f32
        );
        // EIGENIUS_DIVE_SKELETONS=1 dumps the full distinct skeletons (senses erased to `§`) — the
        // actual competing bracketings, for when the `<different shape>` diff is uninformative.
        // EIGENIUS_DIVE_RAW=1 additionally dumps the raw sems (with sense IRIs) for CUI/TUI tracing.
        if std::env::var("EIGENIUS_DIVE_SKELETONS").is_ok() {
            for (i, sk) in skels.iter().enumerate() {
                println!("   skel[{i}]: {sk}");
            }
        }
        if std::env::var("EIGENIUS_DIVE_RAW").is_ok() {
            for (i, s) in sems.iter().enumerate() {
                println!("   raw[{i}]: {s}");
            }
        }
        // What actually differs between reading 0 and each other reading?
        for (i, s) in sems.iter().enumerate().skip(1).take(6) {
            let d = diff(&sems[0], s);
            let same_shape = erase(&sems[0]) == erase(s);
            let tag = if same_shape { "SENSE" } else { "STRUCT" };
            let shown: Vec<String> = d
                .iter()
                .take(3)
                .map(|(x, y)| {
                    if y.is_empty() {
                        x.clone()
                    } else {
                        format!("{x} ⇄ {y}")
                    }
                })
                .collect();
            println!("   [{tag}] #{i}: {}", shown.join("   "));
        }
        if sems.len() > 7 {
            println!("   … {} more", sems.len() - 7);
        }
    }
    println!("\n════════════════════════════════════════════════════════════════════");
    println!("units with 2–16 readings: {n_dive}");
}

/// Diagnostic: `is_common_noun` for candidate apposition heads — to see which verbs/adjectives leak
/// through the head filter (NER precision).
#[test]
#[ignore = "DB-backed diagnostic; run --ignored --nocapture"]
fn debug_ne_head_precision() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    for w in [
        "project",
        "gene",
        "genes",
        "identified",
        "evaluated",
        "other",
        "somatic",
        "deficient",
        "achilles",
        "drive",
        "dna",
        "wrn",
        "msi",
    ] {
        eprintln!(
            "is_common_noun({w:?}) = {}",
            eigenius_kernel::dcg::is_common_noun(&head, w)
        );
    }
}

/// **SPIKE — named-entity glossary (D63 §2a, the third extraction source).** Unit 4 ("Project
/// Achilles and project DRIVE identified WRN as the top preferential dependency in MSI cell lines
/// compared to MSS cell lines.") is the last grammar gap, but the grammar DERIVES its structure — with
/// a non-verb compound head it parses (probe: "Gene Achilles and gene DRIVE identified WRN as … → 12
/// readings"). The gap is caused solely by "project" being noun+verb: the verb entries crowd the
/// coordinated-subject beam and the gold nominal reading is pruned. This spike registers the two
/// research-project NAMES as doc-local **named individuals** (`cat_np(Entity, sg)`), reusing the
/// abbreviation ALIAS machinery ([`abbreviation_resources`]), so the multiword name SHADOWS the
/// "project"-as-verb reading at those positions. If unit 4 parses, the named-entity source is the
/// right structural fix (a targeted extension of the existing glossary, not a new subsystem) — to be
/// generalized from the two hardcoded names to a capitalized-multiword recognizer.
///
///     EIGENIUS_DB_SNAPSHOT=<snap> cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///       spike_named_entity_closes_unit4 -- --ignored --nocapture
#[test]
#[ignore = "DB-backed spike; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn spike_named_entity_closes_unit4() {
    use eigenius_kernel::ontology::resource::{Resource, Value};
    use eigenius_kernel::ontology::well_known as wk;

    let Some(path) = snapshot_path() else { return };
    // Open the backend directly (unlike `open_head`, which drops it): the chained doc layers below are
    // built `with_persistent(backend)` so their value indexes populate LAZILY. Building a `LexicalIndex`
    // over an IN-MEMORY layer chained on the 7.6M-resource persistent head materializes the whole parent
    // (OOM) — the served path resolves lazily, so the doc layers must share the same backend.
    let work = working_copy(&path);
    let store = Arc::new(RocksStore::open(&work).expect("open RocksStore snapshot"));
    let backend: Arc<dyn PersistentBackend> = store;
    let Ok(ctx) = bootstrap_persistent(Arc::clone(&backend)) else {
        eprintln!("SKIP: cannot resume the snapshot");
        return;
    };
    let head = Arc::clone(ctx.head());
    let lem = morphy();
    let p = |s: &str| Iri::parse(s).expect("valid iri");

    // The two named research projects, as they surface in the paper. A general extractor would
    // recognize capitalized multiword "Project X" names; the spike hardcodes the two to validate the
    // mechanism before designing the recognizer.
    let names = [
        ("Project Achilles", "project_achilles"),
        ("project DRIVE", "project_drive"),
    ];

    // 1. Mint a doc-local NAMED INDIVIDUAL for each (an instance of lexicon:Entity, NOT a class) and
    //    chain them onto head so `abbreviation_resources` resolves each as an individual (→ cat_np).
    let mut b0 = LayerBuilder::new("doc-names-ni", Some(Arc::clone(&head)));
    for (surface, key) in names {
        let mut ni = Resource::new(p(&format!("urn:eigenius:doc:ni_{key}")));
        ni.set(
            p(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(p("urn:eigenius:lexicon:Entity"))]),
        );
        ni.set(
            p(wk::DESCRIPTION),
            Value::String(format!(
                "Doc-local named individual: research project {surface:?}"
            )),
        );
        b0.add_resource(ni).expect("add named individual");
    }
    let l1 = Arc::new(b0.build(LayerStorage::with_persistent(Arc::clone(&backend))));
    backend
        .store_layer(&l1)
        .expect("persist named-individual layer");

    // 2. Emit the `cat_np(Entity, sg)` proper-noun alias for each name (the individual arm of
    //    `abbreviation_resources`), then chain entries + individuals into the parse layer.
    let mut b1 = LayerBuilder::new("doc-names", Some(Arc::clone(&l1)));
    for (surface, key) in names {
        let ci = format!("urn:eigenius:doc:ni_{key}");
        let binding = AbbreviationBinding {
            abbr: surface,
            long_form: surface,
            concept_iri: &ci,
            doc_ns: "urn:eigenius:doc",
        };
        let rs = abbreviation_resources(&l1, &binding)
            .unwrap_or_else(|| panic!("emit named-entity entry for {surface:?}"));
        // The emitted alias MUST be the proper-noun (`cat_np`) arm — a `cat_n` here means the minted
        // individual was misclassified as a class (the `instance_type_classes` ResourceRef/String bug).
        let is_cat_np = rs.iter().any(|r| {
            r.get(&p("urn:eigenius:lexicon:cat"))
                .and_then(|v| {
                    eigenius_kernel::program::eigentt_type_mirror::decode_type(v, &l1).ok()
                })
                .is_some_and(|c| matches!(&c, Exp::InductiveCtor(_, n, _) if n == "cat_np"))
        });
        assert!(
            is_cat_np,
            "named individual {surface:?} must emit a cat_np proper-noun alias"
        );
        for r in rs {
            b1.add_resource(r).expect("add entry");
        }
    }
    let l2 = Arc::new(b1.build(LayerStorage::with_persistent(Arc::clone(&backend))));
    backend
        .store_layer(&l2)
        .expect("persist named-entity lexical-entry layer");

    // 3. Probes SHORTEST-FIRST (each flushes before the next, so a heavy long parse can't hide the
    //    cheap results). Each of these GAPPED at 0 before the names were registered — the bare-name and
    //    non-verb-compound analogues parse ("Achilles and DRIVE are essential" → 4; "Gene Achilles and
    //    gene DRIVE identified WRN as … compared to …" → 12), proving the grammar derives the structure
    //    and the only obstacle is "project"'s noun/verb ambiguity crowding the coordinated-subject beam.
    //    Unit 4 is the full 20-token target; run it only when asked (`EIGENIUS_SPIKE_FULL=1`).
    let index = build_index(&l2);
    let full = std::env::var("EIGENIUS_SPIKE_FULL")
        .map(|v| v == "1")
        .unwrap_or(false);
    let mut probes: Vec<(&str, &str)> = vec![
        (
            "P6 are-essential",
            "Project Achilles and project DRIVE are essential.",
        ),
        (
            "single subj+as",
            "Project Achilles identified WRN as the top dependency.",
        ),
        (
            "coord subj+as",
            "Project Achilles and project DRIVE identified WRN as the top dependency.",
        ),
    ];
    if full {
        probes.push(("unit 4 (full)", "Project Achilles and project DRIVE identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines."));
    }
    let mut readings = std::collections::BTreeMap::<&str, usize>::new();
    for (label, s) in probes {
        let f = index.parse(s, &lem);
        eprintln!("[{label:16}] → {} reading(s)  «{s}»", f.len());
        for it in f.iter().take(2) {
            eprintln!("    reading: {}", pretty_term(it.sem()));
        }
        use std::io::Write;
        let _ = std::io::stderr().flush();
        readings.insert(label, f.len());
    }
    // The minimal coordinated case (6 tokens, no beam pressure) is the load-bearing assertion.
    assert!(
        readings["P6 are-essential"] > 0,
        "SPIKE FAILED: the minimal coordinated case still gaps after registering the project names as named individuals"
    );
    assert!(
        readings["coord subj+as"] > 0,
        "SPIKE FAILED: coordinated named individuals + `identify … as` still gaps"
    );
    if full {
        assert!(
            readings["unit 4 (full)"] > 0,
            "SPIKE FAILED: full unit 4 still gaps with the project names as named individuals"
        );
    }
    eprintln!(
        "\nSPIKE PASSED: registering the project names as cat_np named individuals closes the coordinated-subject gap \
         (P6 {}, coord+as {}{}) — the named-entity source is the fix; grammar-gap → 0. Set EIGENIUS_SPIKE_FULL=1 for the full unit 4.",
        readings["P6 are-essential"],
        readings["coord subj+as"],
        if full {
            format!(", unit 4 {}", readings["unit 4 (full)"])
        } else {
            String::new()
        },
    );
}

/// **End-to-end named-entity source** — the real production path (recognizer → `named_entity_augmentation`
/// → in-memory augment overlay), NOT the spike's hand-minted persistent layers. A small document mentions
/// each project name twice (the recognizer requires recurrence ≥2); the augmentation must recognize
/// EXACTLY the two names, and unit 4 must then parse — its reading a coordination of the two doc-local
/// named individuals. This is the witness that the closed grammar gap is a CORRECT reading, not a
/// coverage-only artifact.
#[test]
#[ignore = "DB-backed; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn named_entity_source_closes_unit4_via_overlay() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();

    // Each name occurs twice → satisfies the recurrence requirement; "identified WRN" occurs once (and
    // has a verb head) so it must NOT be recognized.
    let doc =
        "Project Achilles and project DRIVE identified WRN as the top preferential dependency \
               in MSI cell lines compared to MSS cell lines. Project Achilles screened cell lines. \
               Project DRIVE analysed cell lines.";

    let aug = eigenius_kernel::dcg::named_entity_augmentation(&head, doc);
    let mut got: Vec<String> = aug
        .added
        .iter()
        .map(|b| b.provenance.surface.to_lowercase())
        .collect();
    got.sort();
    eprintln!("recognized named entities: {got:?}");
    assert_eq!(
        got,
        vec!["project achilles".to_string(), "project drive".to_string()],
        "the recognizer must find EXACTLY the two recurring project names (no verb/adjective-head false positives)"
    );

    let unit4 = "Project Achilles and project DRIVE identified WRN as the top preferential \
                 dependency in MSI cell lines compared to MSS cell lines.";
    let index = build_index_over(&head, Some(&aug));
    let f = index.parse(unit4, &lem);
    eprintln!("unit 4 → {} reading(s)", f.len());
    for it in f.iter().take(3) {
        eprintln!("    {}", pretty_term(it.sem()));
    }
    assert!(
        !f.is_empty(),
        "unit 4 must parse once the two names are overlaid as named individuals"
    );
    // The reading is a COORDINATION referencing BOTH minted individuals — the correct structure, not a
    // spurious verb-object named entity.
    let any_correct = f.iter().any(|it| {
        let s = pretty_term(it.sem());
        s.contains("ni_project_achilles") && s.contains("ni_project_drive")
    });
    assert!(
        any_correct,
        "no reading references both doc-local named individuals — unit 4 may be parsing via a wrong reading"
    );
    eprintln!("\nPASS: named-entity source closes unit 4 with a coordination of the two named individuals.");
}

/// **Single-sentence forest tracer** — parse one arbitrary sentence (`EIGENIUS_TRACE_SENTENCE`,
/// default the DNA-repair-pathway probe) through the packed path with the DB-backed lexicon, so the
/// `EIGENIUS_TRACE_FOREST` instrument (`chart::trace`) fires and prints the derivation forest to
/// stderr. The trace fires once PER CAP ATTEMPT, so a sentence that gaps at base cap (integrity on)
/// and only parses widened (integrity off) prints TWO blocks — diff them to see which edge multiword
/// span-integrity removed.
///
///     EIGENIUS_TRACE_FOREST=top \
///     EIGENIUS_TRACE_SENTENCE="A DNA repair pathway is essential." \
///     cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///     trace_one_sentence -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_TRACE_FOREST + run --ignored --nocapture"]
fn trace_one_sentence() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();
    let text = std::env::var("EIGENIUS_TRACE_SENTENCE")
        .unwrap_or_else(|_| "A DNA repair pathway is essential.".to_string());
    let f = index.parse(&text, &lem);

    // Skeletonise via the SHARED `erase_senses` (whole-token normalisation) so the skeletons printed
    // here are byte-identical to the ones the faithfulness gate computes — otherwise a pin copied from
    // this trace would silently fail to match. See `erase_senses` for the normalisation rule.
    let sems: Vec<String> = f.iter().map(|it| pretty_term(it.sem())).collect();
    let skels: std::collections::BTreeSet<String> = sems.iter().map(|s| erase_senses(s)).collect();
    println!(
        "\n=== {text} → {} reading(s) | {} structural skeleton(s) | sense× = {:.1} ===",
        f.len(),
        skels.len(),
        f.len() as f32 / skels.len().max(1) as f32,
    );
    // EIGENIUS_TRACE_SKELETONS=1 dumps the distinct STRUCTURAL skeletons (the competing bracketings,
    // senses erased) instead of the raw readings — the direct view of structural over-generation.
    if std::env::var("EIGENIUS_TRACE_SKELETONS").is_ok() {
        for (i, sk) in skels.iter().enumerate() {
            println!("  skel[{i}]: {sk}");
        }
    } else {
        let vnames = unit_sense_names(&text, &index, &lem, &head);
        let vb = Vb {
            names: &vnames,
            layer: &head,
        };
        for (i, it) in f.iter().enumerate().take(20) {
            println!("  reading[{i}]: {}", pretty_term(it.sem()));
            println!("      ≈ \"{}\"", verbalize(it.sem(), &vb));
        }
    }
}

/// Snapshot-gated behavioural guard for the definite-referential fix
/// (`experiments/parsing/near-encoded-bucket-analysis.md`, `2026-07-16`). A DEFINITE object is
/// referential (`ontology:the`, the ι operator), hence **scopeless** — it does not scope under
/// negation, so a definite object under `not` has ONE structural reading. A genuine EXISTENTIAL
/// (`an`) keeps the real `¬∃` / `∃¬` scope split — TWO. Reverting the definite sems to the
/// existential CPS (`obj_exists_sem`) reintroduces the WRN-paper "did not require the exonuclease
/// activity of WRN" over-generation; this test fails if that happens. The CI-runnable wiring guard
/// (no snapshot) is `dcg::lexicon::referential_definite_tests` in the kernel.
///
///     cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///     definite_negation_collapses_referential -- --ignored --nocapture
#[test]
#[ignore = "DB-backed; --ignored --nocapture"]
fn definite_negation_collapses_referential() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    // Distinct STRUCTURAL skeletons via the single token-normalised `erase_senses` above, so sense
    // multiplicity cannot mask the structural scope difference we are asserting on.
    let skeletons = |text: &str| -> std::collections::BTreeSet<String> {
        index
            .parse(text, &lem)
            .iter()
            .map(|it| erase_senses(&pretty_term(it.sem())))
            .collect()
    };
    // The `¬∃`/`∃¬` scope split shows up structurally as a reading whose negation `→ logic:False` is
    // NOT the outermost connective — `False` followed by more continuation (`… → False → G#0 → …`),
    // as opposed to the single scopeless `… → False` a referential definite produces.
    let split_readings = |sk: &std::collections::BTreeSet<String>| -> usize {
        sk.iter().filter(|s| s.contains("False →")).count()
    };

    // Same noun ("activity"), same negation — only `the` (definite) vs `an` (existential) differ.
    // Both carry the same lexical multiplicity of "activity" (senses / a cross-lexicon WordNet↔UMLS
    // dup), so that factor cancels; what remains is the scope split, which the existential keeps and
    // the referential definite drops.
    let def = skeletons("HeLa did not require the activity.");
    let exi = skeletons("HeLa did not require an activity.");

    // The referential definite is SCOPELESS: no reading has negation scoping under the object.
    assert_eq!(
        split_readings(&def),
        0,
        "definite object under negation must be scopeless (`the(A)` referential) — a reading with \
         `False` non-outermost means the `¬∃`/`∃¬` over-generation is back; got {:#?}",
        def
    );
    // The genuine existential KEEPS the real scope split.
    assert!(
        split_readings(&exi) >= 1,
        "existential object under negation must KEEP the ¬∃ / ∃¬ scope split — a scopeless result \
         means the referential fix wrongly leaked onto `a`/`an`; got {:#?}",
        exi
    );
    // And the count signature: strictly fewer structural readings for the definite than its matched
    // existential (robust to how many senses "activity" carries — they scale both sides equally).
    assert!(
        def.len() < exi.len(),
        "definite ({} skeletons) must have strictly fewer structural readings than the matched \
         existential ({} skeletons).\n  definite: {:#?}\n  existential: {:#?}",
        def.len(),
        exi.len(),
        def,
        exi
    );
}

/// PROBE — the named-entity-vs-kind competition on the page's worst unit (204 skeletons, 34% of the
/// page's total). Its dominant axis is the `kind_of` count (7 distinct values, 2..9): "project
/// Achilles" resolves EITHER to the glossary's named individual `ni_project_achilles` OR to a bare
/// common-noun kind `kind_of(…)`, and the subject recurs in ~3 argument positions per conjunct
/// (subject slot, `prep_in` first arg, `prep_to` first arg) × 2 coordinated projects — each position
/// choosing independently.
///
/// This dumps the competing entries for the span and its parts, to establish WHETHER a common-noun
/// analysis of the span coexists with the named-individual entry (the inferred cause) before any fix
/// is designed.
///
/// **READ THE RESULT CAREFULLY — it is on the BASE index only.** `build_index(&head)` has no
/// per-document overlay, and the named-individual alias is minted INTO an overlay
/// (`dcg::glossary`, the `cat_np` alias whose `lexicon:form` is the NE surface). So an EMPTY
/// result for "project Achilles" means "overlay-only", NOT "missing" — which is exactly the wrong
/// conclusion to draw, and was drawn once. What the dump does establish is the COMPOSITIONAL
/// competitor: `project` carries 3 `cat_n` senses (+ ~20 verb entries) and `Achilles` carries both
/// `cat_np(n09484664, sg)` (proper name) and `cat_n(C0001074)` (UMLS), so the span is analysable
/// compositionally from the base lexicon alone, alongside whatever the overlay contributes.
///
/// Run:
///   EIGENIUS_DB_SNAPSHOT=… cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///     probe_named_entity_vs_kind -- --ignored --nocapture
#[test]
#[ignore = "named-entity vs kind competition: entry dump; --ignored --nocapture"]
fn probe_named_entity_vs_kind() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    for form in [
        "project",
        "Achilles",
        "DRIVE",
        "project Achilles",
        "project DRIVE",
        "cell line",
        "MSH2",
        "MLH1",
        "genes",
        "gene",
        "compared",
        "compare",
        "comparison",
        "predicted",
    ] {
        eprintln!("  ENTRIES {form:?}:");
        for (aug, cat, sense) in index.debug_form_entries(form, &lem) {
            let a = if aug { "+" } else { " " };
            eprintln!("     {a} {cat}   [{sense}]");
        }
    }
}

/// PROBE — is the named-entity alias span REACHABLE for both conjuncts of the page's worst unit?
///
/// 172 of that unit's 204 skeletons are internally incoherent: one project resolves to its named
/// individual while the other decomposes into content senses (notably `Achilles` → UMLS C0001074,
/// the anatomical structure). The suspicion is CASE: the alias surface is minted from the document's
/// first occurrence, and the unit's first conjunct is sentence-initial ("Project Achilles") while the
/// second is lower-case ("project DRIVE"), so a case-sensitive span lookup would let exactly one
/// conjunct resolve — the asymmetry observed.
///
/// Dumps, over the AUGMENTED index (the overlay is where the alias lives — see
/// `probe_named_entity_vs_kind` for why the base index shows nothing): the minted surfaces, and the
/// entries reachable for each casing of each span.
#[test]
#[ignore = "DB-backed; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_ne_alias_case_reachability() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let page = std::fs::read_to_string(
        std::env::var("EIGENIUS_WRN_PAGE").unwrap_or_else(|_| String::new()),
    )
    .unwrap_or_else(|_| String::new());
    let doc = if page.is_empty() {
        "Project Achilles and project DRIVE identified WRN as the top preferential dependency \
         in MSI cell lines compared to MSS cell lines. Project Achilles screened cell lines. \
         Project DRIVE analysed cell lines."
            .to_string()
    } else {
        page
    };
    let mut aug = eigenius_kernel::dcg::named_entity_augmentation(&head, &doc);
    // Match the SWEEP's augmentation set exactly, or the trace explains a different parse than the one
    // whose skeletons are being adjudicated. The sweep merges TWO overlays: the named-entity glossary
    // over the parsed CNL (above) AND the abbreviation glossary over the SOURCE page (MSI/MSS
    // definitions live in the original, not the CNL). Verified by a one-number oracle: the sweep
    // reports AMBIG×236 for this unit, so anything else means the configuration does NOT match.
    {
        let source_path =
            std::env::var("EIGENIUS_WRN_SOURCE").unwrap_or_else(|_| WRN_PAGE.to_string());
        let source_text = std::fs::read_to_string(&source_path).unwrap_or_else(|_| doc.clone());
        let abbr = eigenius_kernel::dcg::augment_document_only(
            &head,
            &source_text,
            &eigenius_kernel::dcg::NoAbbreviationProposer,
            &lem,
        );
        eprintln!("ABBREVIATIONS merged: {}", abbr.added.len());
        aug.added.extend(abbr.added);
        aug.supporting.extend(abbr.supporting);
    }
    eprintln!("MINTED SURFACES (exactly as stored):");
    for b in &aug.added {
        eprintln!("   {:?}", b.provenance.surface);
    }
    let index = build_index_over(&head, Some(&aug));
    for form in [
        "Project Achilles",
        "project Achilles",
        "project achilles",
        "Project DRIVE",
        "project DRIVE",
    ] {
        let e = index.debug_form_entries(form, &lem);
        eprintln!("  {form:?} → {} entry(ies)", e.len());
        for (_a, cat, sense) in e.iter().take(4) {
            eprintln!("       {cat}   [{sense}]");
        }
    }
    // Parse the worst unit under PAGE glossary conditions (set EIGENIUS_WRN_PAGE), so the cap ladder
    // and `prefer_multiword` match the sweep rather than a toy document's.
    let unit = "Project Achilles and project DRIVE identified WRN as the top preferential \
                dependency in MSI cell lines compared to MSS cell lines.";
    let f = index.parse(unit, &lem);
    eprintln!(
        "PARSED under page conditions: {} closed reading(s)",
        f.len()
    );
    let sems: Vec<String> = f.iter().map(|it| pretty_term(it.sem())).collect();
    eprintln!(
        "  readings mentioning C0001074 (Achilles-the-noun) : {}",
        sems.iter().filter(|s| s.contains("C0001074")).count()
    );
    eprintln!(
        "  readings mentioning ni_project_*                 : {}",
        sems.iter().filter(|s| s.contains("ni_project")).count()
    );
    eprintln!(
        "  distinct skeletons                               : {}",
        sems.iter()
            .map(|s| erase_senses(s))
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
    // The decisive datum the forest trace does NOT print: the SEMS of the two cat_np leaves at the
    // "Project Achilles" span. If they differ (one the named individual, one a bare kind) that alone
    // explains an 86-reading family with a bare-kind subject despite the span never being torn.
    for probe in [
        "Project Achilles screened cell lines.",
        "Project DRIVE analysed cell lines.",
    ] {
        let r = index.parse(probe, &lem);
        eprintln!("  {probe:?} -> {} reading(s)", r.len());
        let mut seen = std::collections::BTreeSet::new();
        for it in r.iter() {
            if seen.insert(pretty_term(it.sem())) {
                eprintln!("       {}", pretty_term(it.sem()));
            }
        }
    }
}

/// COVERAGE SWEEP — every preposition in each syntactic role, by 0-reading probe.
///
/// The gate's `grammar-gap 0` is measured on the reference PAGE, so a construction the page happens
/// not to contain is invisible to it. That blind spot is why several real holes were found only by
/// hand and then mis-ranked as "off target": `each` had no object entry ("WRN affects each gene." → 0
/// readings) and six prepositions could not post-modify a noun. A 0-reading probe cannot be
/// misclassified the way an inventory scan can, so this sweeps the roles directly.
///
/// A row that reads 0 in BOTH roles means the preposition is effectively unusable; 0 in one role is a
/// missing entry of that shape. Run:
///   EIGENIUS_DB_SNAPSHOT=… cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///     probe_preposition_role_coverage -- --ignored --nocapture
#[test]
#[ignore = "DB-backed coverage sweep; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_preposition_role_coverage() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    const PREPS: &[&str] = &[
        "for", "from", "in", "on", "of", "at", "with", "to", "into", "by", "between", "within",
        "upon", "against", "about", "through", "during", "after", "before", "under", "over",
        "across", "among", "beyond", "without",
    ];
    eprintln!(
        "{:<10} {:>8} {:>8}   (0 = no parse)",
        "prep", "nmod", "adjunct"
    );
    let mut holes: Vec<String> = Vec::new();
    for p in PREPS {
        // nmod: the PP must post-modify a noun.   adjunct: the PP must modify a VP.
        let nmod = index
            .parse(&format!("The change {p} cells was clear."), &lem)
            .len();
        let adj = index.parse(&format!("WRN acts {p} cells."), &lem).len();
        eprintln!("{p:<10} {nmod:>8} {adj:>8}");
        if nmod == 0 {
            holes.push(format!("{p} (nmod)"));
        }
        if adj == 0 {
            holes.push(format!("{p} (adjunct)"));
        }
    }
    eprintln!("\nHOLES ({}): {}", holes.len(), holes.join(", "));
    // Natural-sentence confirmation: the templates above can read 0 for the wrong reason ("WRN acts
    // OF cells" is not English), so each candidate needs a sentence a scientist would actually write.
    eprintln!("\n-- natural-sentence confirmation --");
    for s in [
        "Cells survive without oxygen.",
        "MSI is common among cancers.",
        "The effect extends beyond cells.",
        "Mutations occur at the locus.",
        "The mutation at the locus was clear.",
        "Cells were killed by the inhibitor.",
        "The paper by the authors was clear.",
    ] {
        eprintln!("  {:>3}  {s}", index.parse(s, &lem).len());
    }
}

/// PROBE — localize the close-apposition gap: HEAD SHAPE × CONNECTIVE × POSITION.
///
/// `probe_step5_apposition` has two rows that disagree with what it was written to assert: the
/// PLAIN-head witness `the genes BRCA1 and MSH2 affect cells` reads GAP, while the COMPOUND-head
/// `the MMR genes MSH2, MSH6, PMS2 or MLH1 affect cells` parses ×96 — and the felicity REJECT
/// `the cells BRCA1 and MSH2 affect HeLa` parses ×4 instead of gapping. Those two rows differ in
/// three variables at once (head shape, connective, clause position), so neither can be attributed
/// yet. This crosses all three so the gap lands in one cell.
///
/// [`appose_group`] gates on `sigma_base(head) ⊑ member_ty` in EITHER direction, and `sigma_base`
/// peels the compound's `Σx:Gene. compound_kind(x, MMR)` down to the same `Gene` the plain head
/// carries — so on the documented behaviour, plain and compound heads must score IDENTICALLY. A
/// difference falsifies that, and the row that differs says where.
///
///   EIGENIUS_DB_SNAPSHOT=… cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///     probe_apposition_head_grid -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_apposition_head_grid() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    // HEADS: no classifier at all, a bare plural, a plain definite, a compound definite, a definite
    // whose kind CLASHES with the members (the intended felicity reject), and a singular.
    const HEADS: &[(&str, &str)] = &[
        ("(none)", ""),
        ("bare-pl", "genes "),
        ("plain-def", "the genes "),
        ("compound-def", "the MMR genes "),
        ("clash-def", "the cells "),
    ];
    // CONNECTIVES: all lists are ≥2 members, so subject–verb agreement is constant across the grid
    // and cannot be confounded with the head shape.
    const LISTS: &[(&str, &str)] = &[
        ("and2", "BRCA1 and MSH2"),
        ("or2", "MSH2 or MLH1"),
        ("comma-and4", "MSH2, MSH6, PMS2 and MLH1"),
        ("comma-or4", "MSH2, MSH6, PMS2 or MLH1"),
    ];
    // POSITIONS: the appositive as clause subject, as a preposition's object, as a verb's object.
    // Each frame is `(label, prefix, suffix)` around the appositive NP.
    const FRAMES: &[(&str, &str, &str)] = &[
        ("subject", "", " affect cells."),
        ("prep-obj", "Mutations in ", " cause cancer."),
        ("verb-obj", "WRN affects ", "."),
    ];
    for (fname, pre, post) in FRAMES {
        let frame = |np: &str| format!("{pre}{np}{post}");
        eprintln!("\n=== {fname} ===");
        eprint!("{:<14}", "head");
        for (ln, _) in LISTS {
            eprint!("{ln:>12}");
        }
        eprintln!();
        for (hn, h) in HEADS {
            eprint!("{hn:<14}");
            for (_, l) in LISTS {
                let n = index.parse(&frame(&format!("{h}{l}")), &lem).len();
                eprint!("{n:>12}");
            }
            eprintln!();
        }
    }
    // The apposition is a CLASSIFIER + designator construction; a singleton designator is the same
    // construction with a one-member list, and has no coordination in it at all. If singletons parse
    // where lists gap, the defect is in the coordination leg, not the apposition leg.
    eprintln!("\n=== singleton apposition (no coordination) ===");
    for s in [
        "The gene MSH2 affects cells.",
        "The genes MSH2 affect cells.",
        "The MMR gene MSH2 affects cells.",
        "The cell MSH2 affects cells.",
        "Mutations in the gene MSH2 cause cancer.",
        "WRN affects the gene MSH2.",
    ] {
        eprintln!("  {:>5}  {s}", index.parse(s, &lem).len());
    }
    // What the leaked felicity reject actually BUILDS. If `the cells BRCA1 and MSH2` parses, the sem
    // says whether apposition licensed it or some other route spanned the string.
    eprintln!("\n=== readings of the felicity REJECT ===");
    for s in [
        "the cells BRCA1 and MSH2 affect HeLa",
        "The cells BRCA1 and MSH2 affect HeLa.",
    ] {
        let r = index.parse(s, &lem);
        eprintln!("  {s:?} -> {}", r.len());
        let mut seen = std::collections::BTreeSet::new();
        for it in r.iter() {
            if seen.insert(pretty_term(it.sem())) {
                eprintln!("       {}", pretty_term(it.sem()));
            }
        }
    }
    // The appositive NP is `[det] [classifier] [designator-list]`. Strip it back one element at a
    // time: if the DETERMINED HEAD ALONE already gaps, the apposition rule is innocent and the hole
    // is in the determiner's noun-number agreement.
    eprintln!("\n=== strip the appositive NP back to its parts ===");
    for s in [
        "The gene affects cells.",      //         sg definite head, no designator
        "The genes affect cells.",      //         pl definite head, no designator   <-- key row
        "The MMR genes affect cells.",  //       pl definite COMPOUND head, no designator
        "The cells affect HeLa.",       //         pl definite head, different noun
        "Genes affect cells.",          //         bare pl head
        "The genes MSH2 affect cells.", //      + singleton designator
        "The genes BRCA1 and MSH2 affect cells.", // + 2-member designator list
        "These genes affect cells.",    //         a different pl determiner
        "All genes affect cells.",
        "Some genes affect cells.",
        "The two genes affect cells.",
    ] {
        eprintln!("  {:>5}  {s}", index.parse(s, &lem).len());
    }
    eprintln!("\n=== determiner entries ===");
    for form in ["the", "these", "some", "all", "genes", "gene"] {
        eprintln!("  ENTRIES {form:?}:");
        for (aug, cat, sense) in index.debug_form_entries(form, &lem) {
            let a = if aug { "+" } else { " " };
            eprintln!("     {a} {cat}   [{sense}]");
        }
    }
    // THE DISCRIMINATING EXPERIMENT. `appose_group`'s felicity gate is `type_subsumes` in either
    // direction, and `type_subsumes` (category.rs) compares two `EigonClass` via
    // `layer.is_subclass_of` — a UMLS-hierarchy walk. `genes` carries the ALIGNED index `n05436752`
    // (a WordNet synset); `cells` carries a raw CUI. If the gate is really testing KIND
    // COMPATIBILITY, the head's importer is irrelevant and `the cells …` (a kind clash) must fail
    // while `the genes …` (a kind match) must pass. If it is accidentally testing IMPORTER
    // PROVENANCE, the outcome flips: every CUI-indexed head passes and every WN-synset-indexed head
    // gaps, kind notwithstanding. The two hypotheses predict OPPOSITE columns, so one run decides.
    eprintln!("\n=== head type index vs. apposition licensing ===");
    eprintln!("{:<14} {:>7}  lexical type index(es)", "head", "parses");
    for h in [
        "genes",
        "cells",
        "proteins",
        "enzymes",
        "mutations",
        "tumours",
        "kinases",
        "lines",
        "syndromes",
        "models",
    ] {
        let n = index
            .parse(&format!("The {h} BRCA1 and MSH2 affect HeLa."), &lem)
            .len();
        let tys: std::collections::BTreeSet<String> = index
            .debug_form_entries(h, &lem)
            .into_iter()
            .filter_map(|(_, cat, _)| {
                let c = cat.strip_prefix("cat_n(")?;
                Some(c.split(',').next()?.to_string())
            })
            .collect();
        eprintln!(
            "{h:<14} {n:>7}  {}",
            tys.into_iter().collect::<Vec<_>>().join(" ")
        );
    }
    eprintln!("\n=== the designators' own types (the gate's other operand) ===");
    for form in ["BRCA1", "MSH2", "MLH1", "HeLa"] {
        eprintln!("  ENTRIES {form:?}:");
        for (aug, cat, sense) in index.debug_form_entries(form, &lem) {
            let a = if aug { "+" } else { " " };
            eprintln!("     {a} {cat}   [{sense}]");
        }
    }
}

/// PROBE — forest-trace the apposition gap. `probe_apposition_head_grid` localized it to ONE cell
/// (determined plain head + designator, e.g. `The genes MSH2 affect cells.` → 0, against `The genes
/// affect cells.` → 4), and falsified two explanations along the way: the licensing does NOT track
/// importer provenance (`syndromes`, WordNet-only, parses ×4 while `lines`, CUI-indexed, gaps) and the
/// rows that DO parse carry a compound-noun sem (`kind_of(Σ…compound_kind(·, cell))`), not an
/// apposition one. So the question is no longer "why does the felicity gate reject" but "what, if
/// anything, is built over the appositive span at all".
///
/// Dumps every node over the appositive span for a gapping sentence and a parsing one, so the missing
/// constituent is named rather than inferred. Set `EIGENIUS_TRACE_FOREST` yourself for the full tree:
///   EIGENIUS_TRACE_FOREST=cell:0..4 EIGENIUS_DB_SNAPSHOT=… cargo test --release \
///     -p eigenius-wordnet --test db_backed_encoding probe_appos_trace -- --ignored --nocapture
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_appos_trace() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    // Each row: the sentence, and the sub-strings whose constituency decides the derivation. A
    // sub-string is probed by putting it in a frame that forces exactly one category, so "is there a
    // constituent here" is answered by a reading count rather than by reading a chart.
    for s in [
        "The genes BRCA1 and MSH2 affect cells.", // 0 — the gap
        "The cells BRCA1 and MSH2 affect HeLa.",  // 4 — parses (compound route)
    ] {
        eprintln!("\n===== {s} -> {} =====", index.parse(s, &lem).len());
    }
    // Is the CLASSIFIER+DESIGNATOR constituent (`name` rule, bare `cat_n` left) available for each
    // head? Frame: bare classifier + designator as a clause subject.
    eprintln!("\n=== `name` rule: bare classifier + designator (cat_n + cat_np) ===");
    for s in [
        "Gene MSH2 affects cells.",
        "Genes MSH2 affect cells.",
        "Cell MSH2 affects cells.",
        "Cells MSH2 affect cells.",
        "Syndrome MSH2 affects cells.",
        "Line MSH2 affects cells.",
    ] {
        eprintln!("  {:>5}  {s}", index.parse(s, &lem).len());
    }
    // Is the COMPOUND-NOUN constituent available (`[head] [designator]` as a `cat_n`, which `the` can
    // then determine)? Frame: force a determiner onto it, singular, no coordination.
    eprintln!("\n=== compound route: determiner over [classifier designator] ===");
    for s in [
        "The gene MSH2 affects cells.",
        "The genes MSH2 affect cells.",
        "The cell MSH2 affects cells.",
        "The cells MSH2 affect cells.",
        "The syndrome MSH2 affects cells.",
        "The syndromes MSH2 affect cells.",
        "The line MSH2 affects cells.",
        "The lines MSH2 affect cells.",
    ] {
        eprintln!("  {:>5}  {s}", index.parse(s, &lem).len());
    }
    // NUMBER is the remaining uncontrolled variable: every gapping row above is PLURAL head + the
    // designators, every parsing row in the grid's `singleton` block was SINGULAR. Cross head-number
    // against designator-count directly.
    eprintln!("\n=== head number × designator count ===");
    eprintln!(
        "{:<10} {:>10} {:>10} {:>10}",
        "head", "1 name", "2 names", "4 names"
    );
    for h in ["the gene", "the genes", "the cell", "the cells"] {
        let one = index
            .parse(&format!("Mutations in {h} MSH2 cause cancer."), &lem)
            .len();
        let two = index
            .parse(
                &format!("Mutations in {h} BRCA1 and MSH2 cause cancer."),
                &lem,
            )
            .len();
        let four = index
            .parse(
                &format!("Mutations in {h} MSH2, MSH6, PMS2 or MLH1 cause cancer."),
                &lem,
            )
            .len();
        eprintln!("{h:<10} {one:>10} {two:>10} {four:>10}");
    }
}

/// PROBE — the classifier+designator NP's NUMBER, by agreement.
///
/// `probe_appos_trace` established a monotone split: a PLURAL classifier + designator gaps in
/// subject position (`The genes MSH2 affect cells.` → 0) while the SINGULAR one parses (`The gene
/// MSH2 affects cells.` → 2), and in prep-object position — where nothing agrees — plural and
/// singular are identical (6 / 24 / 144 for 1 / 2 / 4 designators). Agreement is therefore the only
/// live variable.
///
/// `build_name` (combinators.rs) builds `cat_np(sortal, num)` taking `num` from `rargs[1]`, the
/// DESIGNATOR's number; `MSH2` is `cat_np(T028, sg)`. So the construction should be `sg` whatever the
/// classifier's number is. That predicts the UNGRAMMATICAL singular verb parses and the grammatical
/// plural one does not — a prediction no other explanation of the gap makes, since every competing
/// story (felicity gate, importer provenance, compound licensing) is indifferent to the verb's
/// number. The `*` rows are the ones English rejects.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_appositive_number_agreement() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    for (ok, s) in [
        (true, "The gene MSH2 affects cells."),
        (false, "The gene MSH2 affect cells."),
        (true, "The genes MSH2 and MLH1 affect cells."),
        (false, "The genes MSH2 affect cells."), //   grammatical-ish; 0 today
        (false, "The genes MSH2 affects cells."), //  * plural head, singular verb
        (true, "Genes MSH2 and MLH1 affect cells."),
        (false, "Genes MSH2 affect cells."),
        (false, "Genes MSH2 affects cells."), //      * bare plural head, singular verb
        (true, "The cell line HeLa affects genes."),
        (false, "The cell lines HeLa affects genes."), // * plural head, singular verb
        (true, "The cell lines HeLa and MSH2 affect genes."),
    ] {
        let n = index.parse(s, &lem).len();
        let mark = if ok { " " } else { "*" };
        eprintln!("  {mark} {n:>5}  {s}");
    }
    // Same strings with the appositive moved OFF the agreement path (prep-object), as the control:
    // every row here should parse, plural or singular, because no verb agrees with this NP.
    eprintln!("\n=== control: appositive as a prep object (nothing agrees) ===");
    for s in [
        "Mutations in the gene MSH2 cause cancer.",
        "Mutations in the genes MSH2 cause cancer.",
        "Mutations in the cell lines HeLa cause cancer.",
    ] {
        eprintln!("  {:>5}  {s}", index.parse(s, &lem).len());
    }
}

/// PROBE — WHICH ANALYSIS does a determined classifier + designator get?
///
/// Sourcing the number from the classifier (`build_name`, 2026-07-26) turned the BARE classifier row
/// green (`Genes MSH2 affect cells.` 0 → 4) and left every DETERMINED row unmoved (`The genes MSH2
/// affect cells.` still 0; the ungrammatical `The genes MSH2 affects cells.` still 2). That is
/// expected once stated: `build_name`'s left operand must be a bare `cat_n`, and under a determiner
/// the left is a subject GQ — so the rule cannot fire, and `appose_group`, which DOES take a
/// determined head, requires a `cat_group` and so has no singleton case.
///
/// The suspicion was therefore that a determined classifier + one designator falls through to the
/// COMPOUND-NOUN analysis, which puts the head on the NAME ("an MSH2 of the gene kind") — exactly the
/// head placement `build_name`'s doc comment identifies as wrong. A reading count cannot tell the two
/// analyses apart; only the sem can. `the(Σ…named…).1` at the classifier's class is apposition;
/// `kind_of(Σ…compound_kind(·, classifier))` is the compound.
///
/// **CONFIRMED, 2026-07-26.** What this probe printed:
///
/// | string | readings | analysis |
/// | --- | --- | --- |
/// | `Gene MSH2 affects cells.` | 2 | `the(Σ G:n05436752. named(G, C0879290)).1` — classifier is head ✓ |
/// | `The gene MSH2 affects cells.` | 2 | `the(Σ G:C1333234. compound_kind(G, n05436752)).1` — name is head ✗ |
/// | `The MMR genes MSH2, MSH6, PMS2 or MLH1 …` | 96 | ~half apposition ✓, ~half `compound_kind` ✗ |
///
/// Three findings, none yet fixed — recorded here rather than dropped:
///
/// 1. **A determined classifier + ONE designator has no correct analysis.** Its only readings are
///    wrong-headed compounds. The construction's two rules leave that cell empty: `build_name`'s left
///    operand must be a bare `cat_n` (so it cannot fire under a determiner) and `appose_group` takes a
///    determined head but requires a `cat_group`, so it has no singleton case.
/// 2. **Where apposition IS available, the compound analysis competes** and supplies about half the
///    readings — including on the reference page's worst unit, whose construction is exactly
///    `the MMR genes MSH2, MSH6, PMS2 or MLH1`. That, not `Or`-distribution scope, is the dominant
///    ambiguity axis on that unit.
/// 3. **`build_name` accepts an already-apposed right operand**, so a designator can itself be a
///    definite description: `The cell line HeLa affects genes.` yields
///    `the(Σ G:C0007634. named(G, the(Σ G#1:n08430568. named(G#1, n09580829)).1)).1` — "the cell named
///    (the line named HeLa)". A designator must be a NAME, not a description.
///
/// The fix `apply`'s shape forces: it returns a single `Option<Item>` and takes the first matching
/// rule, so a second rule cannot share the `name` trigger. `build_name` therefore has to return the
/// REFINED COMMON NOUN `cat_n(Σx:sortal. named(x, d), num)` — the Σ-refinement `relativize` and the
/// compound rule already build — which lets `the` apply through the existing determiner-over-refined-
/// noun `Fst` machinery; the bare use then needs a unary definite shift back to `cat_np(sortal, num)`
/// with `the(Σ…).1`, principled because the naming is what makes the description unique. Two parts,
/// neither sufficient alone.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_determined_classifier_analysis() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    for s in [
        "The gene MSH2 affects cells.",
        "Genes MSH2 affect cells.",
        "Gene MSH2 affects cells.",
        "The genes MSH2 affects cells.",
        "The cell line HeLa affects genes.",
        "The MMR genes MSH2, MSH6, PMS2 or MLH1 affect cells.",
        // The bare SINGULAR classifier lost its apposition reading when the rule moved to `cat_n`
        // (`Gene MSH2 affects cells.` went from 2, both apposition, to 8, all compound) while the
        // bare PLURAL kept it. Is the apposition derivable at all for a bare singular classifier, or
        // was it displaced by the compound readings the same cell now also holds? These vary the
        // designator and the object to change how much competition sits in the cell.
        "Project Achilles affects cells.",
        "Gene MSH2 affects HeLa.",
        "Chromosome 7 affects cells.",
        // Object position needs only the definite shift, no type-raise. If these parse while the
        // subject rows gap, the shift works and the loss is in the raise; if they gap too, the shift
        // itself never fires.
        "WRN affects project Achilles.",
        "WRN affects gene MSH2.",
        "WRN affects the gene MSH2.",
    ] {
        let r = index.parse(s, &lem);
        eprintln!("\n  {s}  -> {}", r.len());
        let mut seen = std::collections::BTreeSet::new();
        for it in r.iter() {
            let t = pretty_term(it.sem());
            if seen.insert(t.clone()) {
                // Label the analysis by the shape that distinguishes them.
                let kind = if t.contains("compound_kind") {
                    "COMPOUND"
                } else if t.contains("named(") {
                    "apposition"
                } else {
                    "?"
                };
                eprintln!("       [{kind:>10}] {t}");
            }
        }
    }
}

/// PROBE — is a VACUOUS-COMPOUND normal form even available?
///
/// The classifier+designator fix (2026-07-26) gave the determined case its correct apposition
/// analysis, but the wrong-headed COMPOUND analysis stayed, so the germline unit's readings ROSE
/// 128 → 192 instead of being replaced: `gene MSH2` is read both as "the gene named MSH2" (right) and
/// as "the MSH2 that is gene-kind-modified" (wrong).
///
/// The structural kill would be a vacuity normal form: a compound whose MODIFIER SUBSUMES its HEAD
/// adds nothing, because `Σx:C. compound_kind(x, M)` with `C ≤ M` is just `C`. Every MSH2 is a gene,
/// so `Σx:MSH2gene. compound_kind(x, gene)` is vacuous — while a legitimate compound's modifier is
/// NOT a superclass of its head (`cell` and `line` are siblings; `MMR` is a process, not a gene
/// hypernym), so the constraint should leave those untouched.
///
/// That entire plan rests on a subsumption fact across two importers: the classifier `genes` carries
/// the ALIGNED WordNet index `n05436752` while the designator's noun sense is the UMLS CUI
/// `C1333234`, so the rule can only fire if the layer actually relates them. It is not enough that
/// the relation is true of the world. This prints the walk both directions for the pairs the rule
/// would depend on, plus control pairs it must NOT fire on.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_compound_modifier_subsumes_head() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    // (label, candidate SUB, candidate SUPER) — the rule needs `sub ≤ super` for the pairs marked
    // KILL and NOT for the pairs marked KEEP.
    let pairs: &[(&str, &str, &str)] = &[
        // KILL: the designator's concept under the classifier's aligned class.
        ("KILL msh2gene ≤ gene(wn)", "C1333234", "n05436752"),
        ("KILL brca1gene ≤ gene(wn)", "C1528558", "n05436752"),
        ("KILL mlh1gene ≤ gene(wn)", "C0252642", "n05436752"),
        // The same question against the UMLS gene CONCEPT rather than the WordNet synset.
        ("KILL msh2gene ≤ gene(umls)", "C1333234", "C0017337"),
        ("KILL msh2gene ≤ T028", "C1333234", "T028"),
        // KEEP: a legitimate compound's modifier must NOT subsume its head.
        ("KEEP line ≰ cell", "n08430568", "C0007634"),
        ("KEEP hela ≰ cell-line", "C0018873", "C0007600"),
    ];
    for (label, sub, sup) in pairs {
        let (Ok(s), Ok(p)) = (
            eigenius_kernel::ontology::iri::Iri::parse(&format!("urn:eigenius:umlscui:{sub}")),
            eigenius_kernel::ontology::iri::Iri::parse(&format!("urn:eigenius:umlscui:{sup}")),
        ) else {
            continue;
        };
        // Try both namespaces for each side — a WordNet synset lives under a different prefix, and
        // guessing wrong would read as "no relation" and silently kill the plan.
        let mut found = Vec::new();
        for sns in ["umlscui", "umlssty", "wn"] {
            for pns in ["umlscui", "umlssty", "wn"] {
                let (Ok(a), Ok(b)) = (
                    eigenius_kernel::ontology::iri::Iri::parse(&format!(
                        "urn:eigenius:{sns}:{sub}"
                    )),
                    eigenius_kernel::ontology::iri::Iri::parse(&format!(
                        "urn:eigenius:{pns}:{sup}"
                    )),
                ) else {
                    continue;
                };
                if head.is_subclass_of(&a, &b) {
                    found.push(format!("{sns}≤{pns}"));
                }
            }
        }
        let _ = (s, p);
        eprintln!(
            "  {label:<28} {}",
            if found.is_empty() {
                "NO relation in any namespace pairing".to_string()
            } else {
                format!("YES via {}", found.join(", "))
            }
        );
    }
    // What IRI scheme do these classes actually use? Print the real ones off the lexical entries, so
    // the namespace guessing above is checkable rather than assumed.
    eprintln!("\n=== the real class IRIs on the lexical entries ===");
    let lem = morphy();
    let index = build_index(&head);
    for form in ["genes", "MSH2", "cell", "line", "HeLa"] {
        eprintln!("  {form}:");
        for (_, cat, sense) in index.debug_form_entries(form, &lem) {
            eprintln!("     {cat}   [{sense}]");
        }
    }
}

/// PROBE — are the grammar's TYPE-COMPATIBILITY GATES inert?
///
/// `probe_compound_modifier_subsumes_head` found no subclass edge from `C1333234` ("MSH2 gene") to
/// `n05436752` / `C0017337` ("gene") in any namespace pairing. That killed a planned vacuity normal
/// form, but it raises a much larger question, because `type_subsumes` (category.rs) is the ONLY
/// mechanism behind every type gate in the grammar — `appose_group`'s felicity check, `common_super`
/// in coordination, the selectional slots. If UMLS concept-to-concept and concept-to-semantic-type
/// edges are largely absent from the aligned lexicon, those gates cannot discriminate and quietly pass
/// or fail on identity alone.
///
/// There is independent reason to suspect exactly that. `appose_group`'s doc comment reasons from
/// "`C0017337` "gene", emitted `: umlssty:T028`, i.e. `C0017337 ≤ T028`" — and the felicity reject it
/// was written to enforce (`the cells BRCA1 and MSH2 affect HeLa`) does not reject. This censuses the
/// edges those claims need. A row reading NO is a missing edge in the KNOWLEDGE, not a parser bug —
/// and the fix for it is in the importer, not the grammar.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_subclass_edge_census() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let rel = |sub: &str, sup: &str| -> bool {
        let (Ok(a), Ok(b)) = (
            eigenius_kernel::ontology::iri::Iri::parse(sub),
            eigenius_kernel::ontology::iri::Iri::parse(sup),
        ) else {
            return false;
        };
        head.is_subclass_of(&a, &b)
    };
    // The edges the grammar's own doc comments assume. `sub` and `sup` are given as FULL IRIs so a
    // namespace mistake cannot be confused with a missing edge.
    let cases: &[(&str, &str, &str)] = &[
        (
            "gene concept ≤ its semantic type (the claim in appose_group's doc)",
            "urn:eigenius:umlscui:C0017337",
            "urn:eigenius:umlssty:T028",
        ),
        (
            "MSH2 gene ≤ gene concept",
            "urn:eigenius:umlscui:C1333234",
            "urn:eigenius:umlscui:C0017337",
        ),
        (
            "cell concept ≤ its semantic type",
            "urn:eigenius:umlscui:C0007634",
            "urn:eigenius:umlssty:T025",
        ),
        (
            "HeLa ≤ cell line",
            "urn:eigenius:umlscui:C0018873",
            "urn:eigenius:umlscui:C0007600",
        ),
        (
            "any class ≤ lexicon:Entity (the top the slots rely on)",
            "urn:eigenius:umlscui:C0017337",
            "urn:eigenius:lexicon:Entity",
        ),
        (
            "reflexive control (must be YES)",
            "urn:eigenius:umlscui:C0017337",
            "urn:eigenius:umlscui:C0017337",
        ),
    ];
    for (label, sub, sup) in cases {
        eprintln!(
            "  {:<58} {}",
            label,
            if rel(sub, sup) { "YES" } else { "NO" }
        );
    }
    // How many subclass edges does the layer hold at all? If the answer is ~0 for the UMLS namespace,
    // every gate above is decided by the identity case in `type_subsumes` and nothing else.
    eprintln!("\n=== does the layer hold parent_classes at all? ===");
    for iri in [
        "urn:eigenius:umlscui:C0017337",
        "urn:eigenius:umlscui:C1333234",
        "urn:eigenius:umlscui:C0007634",
        "urn:eigenius:wn:n05436752",
    ] {
        let Ok(i) = eigenius_kernel::ontology::iri::Iri::parse(iri) else {
            continue;
        };
        let has = head.get_resource(&i).is_some();
        eprintln!("  {iri:<40} resource present: {has}");
    }
}

/// PROBE — is the `DefiniteDesignation` shift lost to node PACKING?
///
/// The bare half of the classifier+designator fix does not fire: `WRN affects the gene MSH2.` gains
/// the apposition reading (that route is `the` + the refined noun, no shift involved) while every BARE
/// row is compound-only and `Project Achilles affects cells.` gaps outright. The shift's unit test
/// passes on the real rule output, so the function is right and the wiring is wrong.
///
/// The suspected cause is the packing soundness condition the code already documents. `node_sig` keys
/// nodes on `cat_shape`, which ERASES the type index, and the packed builder runs each shift on a
/// node's REPRESENTATIVE item only. `definite_designation` decides on the Σ's restrictor — an index
/// property — so `cat_n(Σ…named…, sg)` and `cat_n(Σ…compound_kind…, sg)` land in one node and the
/// shift fires only when the naming item happens to be the representative. That is the same trap
/// `sem_is_coordination` had to join `Sig` to escape.
///
/// A/B on the same index: `with_packing(false)` decides on every item individually, so if the
/// apposition readings appear there and not under packing, the diagnosis holds and the fix is to carry
/// the property in `Sig` (not to reshape the rule).
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_definite_designation_packing_ab() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let packed = build_index(&head).with_packing(true);
    let unpacked = build_index(&head).with_packing(false);
    eprintln!(
        "{:<38} {:>8} {:>8}   {:>9} {:>9}",
        "sentence", "packed", "unpackd", "appos-p", "appos-u"
    );
    for s in [
        "Project Achilles affects cells.",
        "Gene MSH2 affects cells.",
        "Genes MSH2 affect cells.",
        "WRN affects gene MSH2.",
        "WRN affects project Achilles.",
        "The gene MSH2 affects cells.",
    ] {
        let (p, u) = (packed.parse(s, &lem), unpacked.parse(s, &lem));
        // Count readings carrying the apposition sem shape, the thing the shift is supposed to supply.
        let appos = |v: &Vec<eigenius_kernel::dcg::Item>| {
            v.iter()
                .filter(|it| pretty_term(it.sem()).contains("named("))
                .count()
        };
        eprintln!(
            "{s:<38} {:>8} {:>8}   {:>9} {:>9}",
            p.len(),
            u.len(),
            appos(&p),
            appos(&u)
        );
    }
}

/// PROBE — does NP coordination join **unlike kinds** through the `lexicon:Entity` top?
///
/// The germline unit's dominant family reads "a Germ-Line Mutation in the Mismatch Repair gene named
/// MSH2 cause HNPCC **or MSH6 protein cause HNPCC** or PMS2 … or MLH1 …" (glossed 2026-07-26): the
/// coordination is at CLAUSE-SUBJECT level over a group whose first member is the whole mutation NP
/// and whose remaining three are bare gene names. Only a group typed at the `Entity` top can hold
/// both, and `coordinate_np` builds its group at `common_super` of the conjuncts — which returns the
/// top whenever the conjuncts share nothing else. The same top then satisfies `appose_group`'s
/// bidirectional felicity gate vacuously (`type_subsumes(Entity, mutation)` holds), which is how a
/// MUTATION ends up classifying a gene group in the `prep_in`-per-disjunct family.
///
/// Two things to establish before touching either rule:
///   1. does a mutation actually coordinate with a gene name (the frames), and
///   2. is `Entity` really the join, or do these classes share something narrower (the census) —
///      because refusing the top would ALSO refuse the four-gene group if that group joins at the top.
///
/// **Both CONFIRMED, 2026-07-26 (`wordnet-umls-aligned-2026-07-26-preps-reseed`).**
///
/// | frame | readings |
/// | --- | --- |
/// | `MSH2 and MSH6 cause Lynch syndrome.` (same-kind control) | 4 |
/// | `Mutations and MSH2 cause Lynch syndrome.` | 4 |
/// | `Mutations, MSH6, PMS2 or MLH1 cause Lynch syndrome.` | 24 |
/// | `Cells and MSH2 affect HeLa.` | 8 |
/// | `Syndromes and MSH2 affect cells.` | 16 |
///
/// Cross-kind NP coordination is fully licensed, and `common_super` returns `lexicon:Entity` for
/// EVERY pair censused — including `C1333234 ⊔ C0017337` ("MSH2 gene" ⊔ "gene"), which should have a
/// narrower join. `C1333234 ⊔ C0879290` ("MSH2 gene" ⊔ "MSH6 protein") returns NONE: no join at all.
///
/// So the germline unit's dominant family is NOT removable by a grammar rule. Refusing the `Entity`
/// join in `coordinate_np` would refuse essentially all UMLS coordination on the page (every pair
/// joins there), and tightening `appose_group`'s felicity gate against the top would refuse the
/// CORRECT four-gene apposition along with the mutation-classifies-genes one. The discriminator needs
/// concept-level subclass structure the aligned lexicon does not carry — the same importer-level gap
/// `probe_subclass_edge_census` recorded for CUI→CUI hypernym edges, and this is a second, independent
/// witness to it. What IS fixable at the grammar is the classifier's own shape, which is why the fix
/// landed as `is_pp_refined` (adjacency) rather than as a type gate.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_cross_kind_np_coordination() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    // Does an NP of one kind coordinate with an NP of another? Each row is a minimal pair against a
    // same-kind control, so a 0 is the gate working and a non-zero is the `Entity` join.
    eprintln!("=== cross-kind NP coordination (0 = gate rejects) ===");
    for s in [
        "MSH2 and MSH6 cause Lynch syndrome.", // control: same kind
        "Mutations and MSH2 cause Lynch syndrome.", // mutation ⊕ gene
        "Mutations, MSH6, PMS2 or MLH1 cause Lynch syndrome.", // the shape found in the forest
        "Germline mutations and MSH2 cause Lynch syndrome.",
        "Cells and MSH2 affect HeLa.", // cell ⊕ gene — the felicity reject appose_group documents
        "Syndromes and MSH2 affect cells.",
        "MSI and MSH2 affect cells.",
    ] {
        eprintln!("  {:>6}  {s}", index.parse(s, &lem).len());
    }
    // What IS the join? `common_super` over the classes the germline unit's conjuncts carry. Printed as
    // the resolved IRI so "the top" is visible as such rather than inferred.
    eprintln!("\n=== common_super of the conjunct classes ===");
    let cls = |iri: &str| Exp::EigonClass(Iri::parse(iri).expect("probe iri"));
    let pairs: &[(&str, &str, &str)] = &[
        (
            "MSH2 gene ⊔ gene concept",
            "urn:eigenius:umlscui:C1333234",
            "urn:eigenius:umlscui:C0017337",
        ),
        (
            "MSH2 gene ⊔ MSH6 protein",
            "urn:eigenius:umlscui:C1333234",
            "urn:eigenius:umlscui:C0879290",
        ),
        (
            "gene concept ⊔ mutation",
            "urn:eigenius:umlscui:C0017337",
            "urn:eigenius:umlscui:C0026882",
        ),
        (
            "gene sem-type ⊔ mutation",
            "urn:eigenius:umlssty:T028",
            "urn:eigenius:umlscui:C0026882",
        ),
        (
            "gene sem-type ⊔ protein sem-type",
            "urn:eigenius:umlssty:T028",
            "urn:eigenius:umlssty:T116",
        ),
        // MSI (C0920269, T049 Cell or Molecular Dysfunction) and MMR deficiency (C0265325, T047
        // Disease or Syndrome). Reports `lexicon:Entity` — every semantic type is declared
        // `class umlssty:Tnnn : lexicon:Entity` (`subclass_of` count 0 across all 127), so the walk
        // reaches the top before anything else.
        //
        // It NEED NOT: the UMLS Semantic Network (now provisioned at references/umls/<rel>/NET/)
        // makes both children of T046 Pathologic Function — SRDEF tree numbers `B2.2.1.2.1` and
        // `B2.2.1.2.2`, and SRSTR states both `isa` rows. Adding just that edge was measured to make
        // this pair report `umlssty:T046` while leaving every other row here unchanged, and to be
        // parse-neutral on the page (234/1078 both ways).
        //
        // It is NOT imported, because the change it was meant to enable — refusing an `Entity`-top
        // coordination — was superseded by conjunct-parallelism in `coordinate_np`, which removes
        // strictly more invalid readings, needs no semantic-type knowledge, and does not break the
        // ten `closed_class_determiners` coordination tests that the `Entity` rule did. Kept in the
        // census as the standing record of what a `⊤` join does and does not tell you.
        (
            "MSI ⊔ MMR deficiency  [the refusal's precondition]",
            "urn:eigenius:umlscui:C0920269",
            "urn:eigenius:umlscui:C0265325",
        ),
        (
            "T049 ⊔ T047 (their sem-types)",
            "urn:eigenius:umlssty:T049",
            "urn:eigenius:umlssty:T047",
        ),
        (
            "reflexive control",
            "urn:eigenius:umlscui:C0017337",
            "urn:eigenius:umlscui:C0017337",
        ),
    ];
    for (label, a, b) in pairs {
        let got = eigenius_kernel::dcg::common_super(&cls(a), &cls(b), &head);
        eprintln!(
            "  {:<34} {}",
            label,
            match got {
                Some(Exp::EigonClass(i)) => i.as_str().to_string(),
                Some(other) => format!("{other:?}"),
                None => "NONE (no join)".to_string(),
            }
        );
    }
}

/// PROBE — does a PLURAL surface carry a spurious SINGULAR reading?
///
/// `dcg::parse::seed` refines a common noun's number per candidate lemma: `num = if *c == s_lc {sg}
/// else {pl}` — "a surface equal to the lemma is singular". Sound for a lemma-keyed lexicon; UMLS
/// `MRCONSO.STR` holds SURFACE strings, so a concept ships the inflected "genes" as a form and the
/// identity lemma then yields `sg` for a plural surface. Traced 2026-07-26: `cat_n(n05436752, sg)`
/// reaching the `name` rule for "genes", which is what let a plural classifier take a single designator
/// (classifier capture) after `Guard::NotPlural` was supposed to have stopped it.
///
/// Measured by AGREEMENT, which needs no lexicon introspection: `The X affect cells.` must parse for a
/// plural surface and `The X affects cells.` must NOT. A nonzero singular column is the spurious
/// reading; `sibling?` says whether the singular form is also a surface of the same concept, i.e.
/// whether `convert::is_inflection_of_sibling` will drop the plural entry at import.
///
/// **BASELINE, snapshot `wordnet-umls-aligned-2026-07-26-preps-reseed`: 19 of 19 surfaces carry a
/// spurious singular reading.** Not a "genes" quirk — universal on the page's vocabulary. Seven rows
/// have EQUAL `pl`/`sg` counts (cells 4/4, regions 16/16, lineages 8/8, vulnerabilities 8/8, projects
/// 8/8, counterparts 8/8, syndromes 4/4), the signature of one entry set duplicated under both numbers,
/// which is the identity-lemma route. `genes` 4/2 and `mutations` 8/2 have extra plural-only routes on
/// top. The alignment compounds it: `merges.json` was rebuilt to 38 397 merges specifically to add "the
/// plural surfaces", so those entries are aligned and live.
///
/// **The mass confound, resolved against the list the importer itself uses**
/// (`references/wiktionary/uncountable-nouns.txt`, 32 123 entries). A `mass` reading also takes singular
/// agreement (`feat_meets(mass, sg)`), so a nonzero `sg` column could in principle be legitimate. It is
/// not, in either group:
///
/// - **11 of 19 heads are NOT uncountable** — gene, cell, syndrome, region, microsatellite, biomarker,
///   response, tumour, defect, project, counterpart. No mass reading exists, so the singular is purely
///   the inflected duplicate.
/// - **8 are** (mutation, line, cancer, therapy, set, lineage, protein, vulnerability), and that does not
///   excuse them either: [`convert::push_entries`] emits the additive `cat_n(C, mass)` PER FORM, so the
///   *plural* form carries a mass entry as well — and a plural surface is not mass. Same root cause;
///   [`convert::is_inflection_of_sibling`] takes both, because the skip precedes `emit_entry`.
///
/// So after the reseed every `sg` column should go to 0. A row that does NOT is a plural-only concept
/// with no singular sibling to fall back on — the residual the importer gate cannot reach by
/// construction, and what decides whether anything further is needed.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_plural_surface_singular_reading() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    // The page's plural nouns, plus the two that drove the germline defect.
    const PLURALS: &[&str] = &[
        "genes",
        "mutations",
        "cells",
        "cell lines",
        "cancers",
        "syndromes",
        "regions",
        "microsatellites",
        "biomarkers",
        "therapies",
        "responses",
        "data sets",
        "lineages",
        "proteins",
        "tumours",
        "defects",
        "vulnerabilities",
        "projects",
        "counterparts",
    ];
    // Both columns are READING COUNTS for the two frames, which differ only in verb inflection, so the
    // verb forces subject-number agreement. Only zero-vs-nonzero is meaningful: the magnitudes are
    // inflated by unrelated ambiguity in the frame (senses of `cells`, the determiner, the verb), so a
    // count moving between snapshots says nothing on its own.
    eprintln!("{:<18} {:>8} {:>8}   verdict", "surface", "pl", "sg");
    let (mut spurious, mut gapped, mut clean) = (0usize, 0usize, 0usize);
    for p in PLURALS {
        let pl = index.parse(&format!("The {p} affect cells."), &lem).len();
        let sg = index.parse(&format!("The {p} affects cells."), &lem).len();
        // `pl == 0` is a COVERAGE FAILURE, not a pass: the plural frame must parse. Reporting 0/0 as
        // "ok" is how a regression gets read as a fix — it labelled `microsatellites`/`biomarkers`
        // clean on 2026-07-26 when in fact they had stopped parsing entirely.
        let verdict = match (pl, sg) {
            (0, _) => {
                gapped += 1;
                "GAP — the plural frame does not parse at all"
            }
            (_, 0) => {
                clean += 1;
                "ok — plural only, as it should be"
            }
            _ => {
                spurious += 1;
                "SPURIOUS singular reading"
            }
        };
        eprintln!("{p:<18} {pl:>8} {sg:>8}   {verdict}");
    }
    eprintln!("\n{spurious} spurious singular, {gapped} GAPPED (coverage loss), {clean} correct");
    assert_eq!(
        gapped, 0,
        "a plural surface that no longer parses is a coverage regression, not a result"
    );
}

/// PROBE — WHERE does a plural surface's singular reading come from?
///
/// [`probe_plural_surface_singular_reading`] establishes the symptom: 19 of 19 plural surfaces admit a
/// singular subject. The obvious mechanism — a UMLS entry keyed at the inflected form, which
/// `lookup_span` then stamps `sg` because the candidate lemma equals the surface — was TESTED AND
/// FALSIFIED on 2026-07-26: pruning exactly those entries in the importer left 17 of 19 spurious (and
/// gapped two surfaces). So the model was wrong and this replaces it with the entry set itself.
///
/// For each surface it prints, per candidate lemma, every entry [`LexicalIndex::entries_for`] returns
/// with its category (the `num` is the second argument of `cat_n`/`cat_np`) and its owning lexicon.
/// `entries_for` keys on LOWERCASED forms, so a case mismatch cannot explain a miss.
///
/// Read it as: an entry listed under a candidate EQUAL to the surface is stamped `sg`; one under a
/// shorter candidate is stamped `pl`. A surface with entries under the plural candidate is the symptom's
/// source; one with none, yet still admitting `sg`, means the singular arrives some other way entirely.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_where_the_singular_reading_comes_from() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = LexicalIndex::build(Arc::clone(&head));
    for surface in ["cancers", "biomarkers", "genes", "cells", "microsatellites"] {
        eprintln!("\n===== {surface} =====");
        // The candidate set `lookup_span` builds: the raw surface, every validated lemma, and the
        // crude plural stem. Reproduced here because the seeder's own helper is private.
        let mut cands: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::from([surface.to_string()]);
        for pos in [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv] {
            for l in lem.lemmas(surface, pos) {
                cands.insert(l);
            }
        }
        if let Some(stem) = lem.regular_plural_stem(surface) {
            cands.insert(stem);
        }
        for c in &cands {
            let entries = index.entries_for(c);
            let stamped = if c == surface { "sg" } else { "pl" };
            eprintln!(
                "  candidate {c:<20} stamped {stamped}   {} entr(ies)",
                entries.len()
            );
            for e in entries.iter().take(6) {
                eprintln!(
                    "      {:<52} {}",
                    pretty_term(e.item.cat()),
                    e.in_lexicon
                        .as_ref()
                        .map(|i| i.as_str().to_string())
                        .unwrap_or_else(|| "<untagged>".into())
                );
            }
            if entries.len() > 6 {
                eprintln!("      … {} more", entries.len() - 6);
            }
        }
    }
}

/// CONTROL — is subject–verb NUMBER agreement enforced at all in these frames?
///
/// [`probe_plural_surface_singular_reading`] assumes `The X affects …` vs `The X affect …` discriminates
/// a singular from a plural subject. That assumption was never tested, and the entry-set introspection
/// contradicts the readings it produced (a surface with ZERO entries at the plural candidate still
/// admitted the singular frame). If a SINGULAR subject also parses with the PLURAL verb, the frames are
/// not measuring agreement, and every number conclusion drawn from them is void.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_agreement_frames_actually_discriminate_number() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let lem = morphy();
    let index = build_index(&head);
    eprintln!(
        "{:<12} {:>10} {:>10}   (want: sg-subj parses only `affects`)",
        "subject", "affect", "affects"
    );
    for (label, subj) in [
        ("singular", "gene"),
        ("singular", "cell"),
        ("plural", "genes"),
        ("plural", "cells"),
    ] {
        let pl_v = index
            .parse(&format!("The {subj} affect cells."), &lem)
            .len();
        let sg_v = index
            .parse(&format!("The {subj} affects cells."), &lem)
            .len();
        eprintln!("{subj:<12} {pl_v:>10} {sg_v:>10}   {label}");
    }
    // The decisive cell: a singular subject with a PLURAL verb must not parse.
    let bad = index.parse("The gene affect cells.", &lem).len();
    eprintln!("\n`The gene affect cells.` (sg subject, pl verb) → {bad} reading(s)");
    assert_eq!(
        bad, 0,
        "the frames do not discriminate number — `probe_plural_surface_singular_reading` measures verb \
         ambiguity, not agreement, and its results are void"
    );
}

/// PROBE — what would relating `MSI` and `MMR deficiency` actually require?
///
/// Refusing an `Entity`-top join in `coordinate_np` fixes the germline unit (9 -> 1) at the cost of
/// exactly one gap: "We hypothesized that MSI and MMR deficiency may create vulnerabilities."
/// (`constructions::coordinate_np`). This prints what those two conjuncts carry, so "add one edge" can
/// be checked rather than assumed — in particular whether the missing link is CUI-level or a TUI->TUI
/// edge, the latter being the UMLS Semantic Network structure whose wholesale import broke parses on
/// 2026-07-11 and was reverted.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_msi_mmr_deficiency_join() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = LexicalIndex::build(Arc::clone(&head));
    let mut classes: Vec<(String, Exp)> = Vec::new();
    for surface in [
        "msi",
        "mmr deficiency",
        "deficiency",
        "microsatellite instability",
    ] {
        let entries = index.entries_for(surface);
        eprintln!("\n=== {surface} — {} entr(ies) ===", entries.len());
        for e in entries.iter().take(8) {
            eprintln!("    {}", pretty_term(e.item.cat()));
            if let Some([ty, _]) = eigenius_kernel::dcg::is_ctor(e.item.cat(), "cat_n") {
                classes.push((surface.to_string(), ty.clone()));
            }
        }
    }
    // Every cross pair's join, and each class's own ancestors — the two questions that decide whether
    // a CUI-level edge suffices.
    eprintln!("\n=== pairwise common_super ===");
    for (na, a) in &classes {
        for (nb, b) in &classes {
            if na >= nb {
                continue;
            }
            let j = eigenius_kernel::dcg::common_super(a, b, &head);
            eprintln!(
                "  {:<24} ⊔ {:<24} = {}",
                pretty_term(a),
                pretty_term(b),
                j.map(|x| pretty_term(&x)).unwrap_or_else(|| "NONE".into())
            );
        }
    }
}

/// PROBE — which modifier surfaces carry BOTH an entity and a kind reading?
///
/// The two compound axioms are typed apart on purpose: `compound(x, m)` takes an ENTITY modifier
/// (`refine_named_compound`, `[cat_np][cat_n]`) and `compound_kind(x, K)` a KIND
/// (`refine_kind_compound`, `[cat_n][cat_n]`). So a surface holding both a `cat_np` and a `cat_n`
/// entry fires BOTH rules and yields two readings of one N-N pair.
///
/// Measured on "MSI cell lines from these four lineages showed greater dependence on WRN than their
/// MSS counterparts." (2026-07-27), the page's worst unit at 48 skeletons: unifying the two axioms
/// collapses it to 20 distinct shapes, so 28 of the 48 differ ONLY in which axiom each site chose.
/// This prints, per content word, whether the lexicon actually licenses both — i.e. whether that
/// choice is a real lexical ambiguity or duplicated encodings of one concept.
#[test]
#[ignore = "DB-backed diagnostic; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn probe_compound_modifier_categories() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = LexicalIndex::build(Arc::clone(&head));
    eprintln!("{:<16} {:>7} {:>7}   classes", "surface", "cat_np", "cat_n");
    for s in [
        "msi",
        "mss",
        "cell",
        "line",
        "cell line",
        "lineage",
        "wrn",
        "counterpart",
        "dependence",
    ] {
        let entries = index.entries_for(s);
        let mut np: Vec<String> = Vec::new();
        let mut n: Vec<String> = Vec::new();
        for e in &entries {
            if let Some([ty, _]) = eigenius_kernel::dcg::is_ctor(e.item.cat(), "cat_np") {
                np.push(pretty_term(ty));
            } else if let Some([ty, _]) = eigenius_kernel::dcg::is_ctor(e.item.cat(), "cat_n") {
                n.push(pretty_term(ty));
            }
        }
        np.sort();
        np.dedup();
        n.sort();
        n.dedup();
        let both: Vec<&String> = np.iter().filter(|c| n.contains(c)).collect();
        eprintln!(
            "{:<16} {:>7} {:>7}   {}",
            s,
            np.len(),
            n.len(),
            if both.is_empty() {
                "—".to_string()
            } else {
                format!("SAME CLASS both ways: {both:?}")
            }
        );
    }
}

/// **Reduced-relative trigger guard** — `dcg::rules::constructions::reduced_relative` must fire on the
/// PASSIVE participle (`pass`) and NOT on the active/perfect one (`pss`), with the oblique participial
/// reaching `cat_pp` through `combinators::oblique_participial_lifts` instead.
///
/// Nothing held this before 2026-07-27: "The deficiency predicted by the model was clear." appeared
/// only in a doc comment recording that it had once parsed to 0 readings, and the whole three-way
/// distinction was invisible to the lib suite — the page measurement was the only thing that could see
/// it. Each arm below is a route that a plausible narrowing of the trigger silently removes:
///
///   - `pass` too narrow  -> the by-agent participial gaps (this was the historical 0-reading bug);
///   - `pss`  too wide    -> the ungrammatical reduced SUBJECT relative comes back;
///   - lift missing       -> the oblique participial loses its only route (measured: page 269 -> 367,
///     byte-identical to disabling the shift outright).
///
///     cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///     reduced_relative_fires_on_passive_not_perfect -- --ignored --nocapture
#[test]
#[ignore = "DB-backed; --ignored --nocapture"]
fn reduced_relative_fires_on_passive_not_perfect() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    let skeletons = |text: &str| -> std::collections::BTreeSet<String> {
        index
            .parse(text, &lem)
            .iter()
            .map(|it| erase_senses(&pretty_term(it.sem())))
            .collect()
    };

    // (1) BY-AGENT PASSIVE participial — the `pass` route. Must parse; a gap here is the historical
    // "parsed to 0 readings" regression.
    let by_agent = skeletons("The deficiency predicted by the model was clear.");
    assert!(
        !by_agent.is_empty(),
        "by-agent passive participial must parse (the `pass` reduced-relative route) — 0 readings \
         means the trigger was narrowed past `gq_prep_passive_agent`"
    );

    // (2) OBLIQUE participial — the seed-time lift route. Must parse. This one does NOT go through
    // `reduced_relative` at all any more; it reaches `cat_pp` as `cat_pp/cat_pp_arg` applied to its PP.
    let oblique = skeletons("The cell lines compared to MSS lines were resistant.");
    assert!(
        !oblique.is_empty(),
        "oblique participial must parse via `oblique_participial_lifts` — 0 readings means the \
         seed-time lift stopped firing and only the (now-refused) `pss` shift route remained"
    );

    // (3) The ACTIVE/PERFECT participle must NOT become a noun post-modifier. The signature of the bad
    // reading is structural, not a count: "WRN [that] induced DNA" makes `WRN` a refined noun and
    // leaves `breaks` as the finite main verb — so `induced` ends up INSIDE a Σ restrictor. The correct
    // readings all have `induced` as the matrix verb, with `DNA breaks` its object.
    let perfect = skeletons("Depletion of WRN induced double-stranded DNA breaks.");
    assert!(
        !perfect.is_empty(),
        "the sentence must still parse — this guard is about WHICH readings exist, not coverage"
    );
    // `breaks` as a finite intransitive verb is only reachable once the participial has been consumed
    // as a post-modifier, so the count is the signature: A/B-measured 2 skeletons with the trigger on
    // `pass`, 8 with it on `pss`. The bound sits between them rather than at 2, so a later unrelated
    // gain in ambiguity here does not read as this defect returning.
    assert!(
        perfect.len() <= 4,
        "active/perfect participle must not license a reduced SUBJECT relative (English has no \
         \"*the man ate the food\" for \"the man that ate\"). Measured 2 skeletons with the trigger \
         on `pass`, 8 with it on `pss`; got {} — {:#?}",
        perfect.len(),
        perfect
    );
}

/// **A VP-adjunct must not attach ABOVE sentential negation** — `lexicon:scope_bearing` (2026-07-27).
///
/// `not` is a VP→VP functor with its own lexical entry, so `not respond` forms as an ordinary
/// `ForwardApp` and the VP-adjunct preposition family — `((S[fin_any]\NP)\(S[fin_any]\NP))/NP`, whose
/// `fin_any` unifies with the VP either inside or outside the operator — could attach above it:
///
/// ```text
///   And(respond(s) → False, prep_to(s, X))     "they don't respond, AND they are to X"   WRONG
///   And(respond(s), prep_to(s, X)) → False     "not (they respond and are to X)"         ok
///   respond_p(X, s) → False                    `respond to` as a 2-place verb            ok
/// ```
///
/// The do-support route was already protected (`do not respond` is tagged `Combinator::Modal`, and
/// `ProvGuard::LeftNotModal` refuses it as a VP-adjunct's argument); the standalone `not` route was
/// not. Declaring `lexicon:scope_bearing` tags its leaf `Combinator::ScopeOperator`, the combinator
/// tags the output `Modal`, and the existing guard covers both.
///
/// The criterion is core-en's, and it is semantic rather than categorial: its Negation family
/// (`auxv.xsl`, `pos="V" closed="true"`) is `(s.1.from-6.E\np)/(s.6.E2\np)` — a NEW situation index
/// derived from the argument's — while its Adverb family (`adv.xsl`, `pos="Adv"`) is `s.1.E\s.1.E`
/// with LF `HasProp(E, P)`, the SAME index decorated. An adjunct above an index-preserving modifier
/// lands on the same event and is the same claim; above an index-SHIFTING operator it escapes.
///
/// Measured: this unit 9 → 6 skeletons on the page (24 → 12 in isolation), page 237 → 234, with a
/// set diff of 3 lost / 0 added — the three lost being exactly the escaped family.
///
///     cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///     negation_scope_blocks_adjunct_escape -- --ignored --nocapture
#[test]
#[ignore = "DB-backed; --ignored --nocapture"]
fn negation_scope_blocks_adjunct_escape() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    let skeletons: std::collections::BTreeSet<String> = index
        .parse(
            "Some cancers do not respond to immune checkpoint blockade.",
            &lem,
        )
        .iter()
        .map(|it| erase_senses(&pretty_term(it.sem())))
        .collect();
    assert!(!skeletons.is_empty(), "the sentence must still parse");

    // The escaped reading: the negation sits INSIDE an `And` whose sibling is the PP adjunct, so
    // only the verb is negated and the adjunct is asserted outright.
    let escaped: Vec<&String> = skeletons
        .iter()
        .filter(|s| s.contains("→ False, prep_"))
        .collect();
    assert!(
        escaped.is_empty(),
        "a VP-adjunct attached ABOVE the negation and escaped its scope — `not` lost its \
         lexicon:scope_bearing declaration, or ProvGuard::LeftNotModal stopped covering it. \
         Offending skeleton(s): {escaped:#?}"
    );

    // And the correct wide-scope reading is still there — this must be a REFUSAL, not a coverage
    // loss. `And(…, prep_to(…)) → False` is the negation taking the whole conjunction.
    assert!(
        skeletons.iter().any(|s| s.contains(") → False")),
        "the wide-scope negation reading must survive; got {skeletons:#?}"
    );
}

/// **Conjunct parallelism: a coordination may not strand a modifier on one disjunct** (2026-07-28).
///
/// `coordinate_np` refuses to pair a Σ-refined member with a bare one. Without it the germline unit
/// coordinates the WHOLE "germline mutations in the MMR genes MSH2" NP with three bare gene names;
/// the predicate then distributes, "germline mutations in" is stranded on the first disjunct, and
/// the reading asserts that MSH6/PMS2/MLH1 THEMSELVES cause Lynch syndrome. That is false under every
/// sense assignment — a gene does not cause the syndrome, mutations in it do — so it is an invalid
/// reading, not a dispreferred one.
///
/// Measured: this unit 9 -> 1 skeletons, and `Thus, MSI tumours need novel therapies.` 2 -> 1 (the
/// loss there is `MSI tumours` read as an implicit juxtaposition LIST, `And(need(…, bare), need(…,
/// MSI-tumours))`, with no coordinator present). Page 234 -> 225, encoded 10 -> 12, grammar-gap 0.
///
/// **Two earlier attempts at this unit are recorded so they are not retried.** Refusing a
/// `common_super` join at `lexicon:Entity` removed the same 8 skeletons but (a) gapped "MSI and MMR
/// deficiency create vulnerabilities" unless a targeted UMLS `T047`/`T049 -> T046` edge was imported,
/// and (b) broke ten `closed_class_determiners` coordination tests, because a demo domain with no
/// semantic-type layer has `⊤` as the GENUINE join for `CellLine` and `Gene` — "HeLa and BRCA1
/// affect HeLa" must parse. A type-level selectional restriction from the UMLS Semantic Network was
/// also measured and rejected: `SRSTRE2` licenses both the gene and the protein reading of
/// "MLH1 promoter" (`produces` vs `interacts_with`).
///
///     cargo test --release -p eigenius-wordnet --test db_backed_encoding \
///     coordination_may_not_strand_a_modifier -- --ignored --nocapture
#[test]
#[ignore = "DB-backed; --ignored --nocapture"]
fn coordination_may_not_strand_a_modifier() {
    let Some(path) = snapshot_path() else { return };
    let Some(head) = open_head(&path) else { return };
    let index = build_index(&head);
    let lem = morphy();

    let skeletons = |text: &str| -> std::collections::BTreeSet<String> {
        index
            .parse(text, &lem)
            .iter()
            .map(|it| erase_senses(&pretty_term(it.sem())))
            .collect()
    };

    let g = skeletons(
        "Germline mutations in the MMR genes MSH2, MSH6, PMS2 or MLH1 cause Lynch syndrome.",
    );
    assert!(!g.is_empty(), "the germline unit must still parse");
    // The invalid family is clause-level: `Or` OUTERMOST over four propositions, three of them bare.
    // The correct reading keeps the `Or` INSIDE a Σ restrictor, over four `named` appositions.
    let stranded: Vec<&String> = g.iter().filter(|s| s.starts_with("Or(")).collect();
    assert!(
        stranded.is_empty(),
        "a coordination stranded the `in the MMR genes …` modifier on one disjunct, asserting that \
         the bare genes cause Lynch syndrome: {stranded:#?}"
    );
    assert!(
        g.iter().any(|s| s.contains("named(") && s.contains("prep_in(")),
        "the correct distributed reading — `Or` inside the Σ, over four `named` appositions — must \
         survive; this must be a REFUSAL of the invalid family, not a coverage loss. Got {g:#?}"
    );

    // Cross-sort coordination of two BARE conjuncts stays licensed: parallelism is about stranding a
    // modifier, not about the conjuncts' kinds. This is the case the `Entity`-top rule wrongly killed.
    assert!(
        !skeletons("MSI and MMR deficiency create vulnerabilities.").is_empty(),
        "coordinating two unrefined conjuncts of unlike kind must still parse"
    );
}
