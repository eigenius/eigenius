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

//! The prose → chain-record pipeline, shared by the `prose-to-eigon` and `prose-to-esl` binaries.
//!
//! Since D67 §3 this is a **thin driver over the kernel's [`DocumentPipeline`]** — the same
//! Stage A → parse → select → resolve loop the measurement harness and ingestion run; the CLI
//! only maps its deterministic replay arms onto the pipeline's seams and emits the record:
//!
//! - `--pins`       → [`PinReadingRanker`] (the declared gate arm)
//! - `--selections` → [`ReplayReadingRanker`] (the computed arm — replay-only here)
//! - `--ranks`      → the sense-rank replay/record arm (via the parser hook)
//! - `--proposals`  → [`ReplayProposer`] (anaphora — replay-only here; record a draw with the
//!   close-out harness's `EIGENIUS_PROPOSALS` arm). Without it, no anaphora resolves — open
//!   parses stay `Open`, which keeps the pin-arm artifacts byte-stable.
//!
//! The `--selections` arm, as the maintained demo drives it (`demo/prose-to-formulas-v2/run.sh`):
//!
//! ```bash
//! prose-to-esl --snapshot ../db-snapshot/wordnet-umls-aligned-2026-08-20c \
//!              --source     demo/prose-to-formulas-v2/paragraph.txt \
//!              --ranks      demo/prose-to-formulas-v2/ranks.json \
//!              --selections demo/prose-to-formulas-v2/selections.json \
//!              --proposals  demo/prose-to-formulas-v2/proposals.json \
//!              --kinds      demo/prose-to-formulas-v2/kinds.json \
//!              --ns         urn:eigenius:demo:v2 \
//!              --out        demo/prose-to-formulas-v2/claims-intact.esl
//! ```
//!
//! The `--pins` arm takes the same shape with `--pins <file>` in place of `--selections`. There is no
//! committed example of a pins file: `demo/prose-to-formulas/pins.tsv` was the only one and it was
//! retired 2026-08-17 (a sense-erased pin cannot break a sense-only tie, so it was inventory-dependent
//! — see that demo's README). The arm itself is exercised on every deterministic parse-rate sweep,
//! where [`PinReadingRanker`] selects from `experiments/parsing/expected-readings.tsv`.
//!
//! **Fail closed everywhere.** A sentence that does not encode — a gap, an unresolved referent
//! hole, a pin that matches zero or several pooled readings, a selection-replay abstention —
//! aborts the whole emission with a diagnostic: a partial encoding is not a result. Under
//! `--partial` the non-encoding is instead RECORDED — the unit lands as its `DiscourseUnit` plus
//! an `enc:CutItem` naming the reason (D67 §5: the artifact states what did not encode; it never
//! silently drops a unit) — and a pin contradiction still aborts. Stage-A glossary resources are
//! emitted into the artifact in both modes.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::emit::DocumentMeta;
use crate::select::load_pins;
use crate::snapshot::{
    build_sense_ranker_with_model, open_head_and_backend, CELL_BEAM, SENSE_CAP,
};
use clap::Parser as ClapParser;
use eigenius_kernel::dcg::{
    AugmentOptions, CostFirstReadingRanker, InProcessPipeline, NoAbbreviationProposer, Parser,
    PinReadingRanker, Proposal, ProposeCtx, Proposer, ReadingRanker, RecordingReadingRanker,
    ReplayProposer, ReplayReadingRanker, SentenceOutcome,
};
use eigenius_wordnet::lemmatizer::MorphyLemmatizer;
use sha2::{Digest, Sha256};

/// What the `--out` family of paths receives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputFormat {
    /// Eigon-JSON, the wire format `eigenius load` consumes.
    Json,
    /// ESL source. Compiling it yields the same resources — the printer is the inverse of the
    /// loader, and `eigenius decompile --verify` is the same check applied to a file.
    Esl,
}

#[derive(ClapParser)]
#[command(about = "Parse prose over a lexicon snapshot; emit the D62 encoding record")]
pub struct Args {
    /// RocksDB lexicon snapshot (WordNet + UMLS, aligned). Copied before opening; never mutated.
    #[arg(long)]
    snapshot: PathBuf,
    /// The prose to encode.
    #[arg(long)]
    source: PathBuf,
    /// Pinned readings: `sentence <TAB> skeleton <TAB> note` — the DECLARED selection arm.
    /// Exactly one of `--pins` / `--selections` / `--cost-first` is required (unless `--partial`).
    #[arg(long)]
    pins: Option<PathBuf>,
    /// A reading-selection draw (`selections.json`) — the COMPUTED selection arm
    /// (d63-reading-selection.md). **Exists → REPLAY** (deterministic, no LLM); **absent →
    /// RECORD** a live draw into it (needs `use-llm` + a key) while generating. The record arm
    /// lives HERE because selection keys hash the presented pool, and this pipeline's pool
    /// (its own Stage A) is not the measurement harness's — a draw recorded there cannot
    /// answer this driver's questions (found 2026-08-12). Exactly one of `--pins` /
    /// `--selections` / `--cost-first` is required (unless `--partial`).
    #[arg(long)]
    selections: Option<PathBuf>,
    /// Deterministic offline reading selector (`--no-llm`): selects the lowest-cost surviving
    /// reading (`forest_rank == 0`) while still enforcing D69 §3 presentation injectivity.
    #[arg(long)]
    cost_first: bool,
    /// Where to write the recorded `selections.json` draw produced by `--cost-first` (replayable
    /// via `--selections`).
    #[arg(long)]
    selections_out: Option<PathBuf>,
    /// A recorded `ranks.json` to replay — deterministic, no LLM. Omit for cap-only (a DIFFERENT
    /// experiment: sense elimination is off, so the pins may not match).
    #[arg(long)]
    ranks: Option<PathBuf>,
    /// Optional model id override for live `--ranks` RECORD mode (e.g. `jev-latest` or
    /// `claude-sonnet-4-6`); defaults to `EIGENIUS_SENSE_MODEL` else `DEFAULT_MODEL`.
    #[arg(long)]
    sense_model: Option<String>,
    /// Resume an interrupted live recording: preload existing `--ranks` and `--selections`
    /// checkpoint files, replaying hits and invoking the live model only on remaining sentences.
    #[arg(long)]
    resume: bool,
    /// An anaphora-proposal draw (`proposals.json`) — D64/D67 §3, resolves referent holes
    /// through the discourse loop. **Exists → REPLAY** (deterministic, no LLM); **absent →
    /// RECORD** a live draw into it (needs `use-llm` + a key). Omit ⇒ no anaphora resolves (open
    /// parses stay Open — and abort the emission, which is fail-closed) unless `--recency` is set.
    #[arg(long)]
    proposals: Option<PathBuf>,
    /// Deterministic recency anaphora proposer (`enc:binding_recency`, `--no-llm`): ranks candidate
    /// antecedents in discourse-recency order (`0..candidates.len()`), vetted by the kernel's
    /// restrictor + felicity checks.
    #[arg(long)]
    recency: bool,
    /// Where to write the recorded `proposals.json` draw produced by `--recency` (replayable via
    /// `--proposals`).
    #[arg(long)]
    proposals_out: Option<PathBuf>,
    /// A recorded discourse-KIND draw (`kinds.json`) — D68. Installing it turns on the CLAIM
    /// LANDER: each closed sentence's claim is graded and landed INSIDE the discourse loop, so a
    /// later demonstrative («these findings») can bind to it. **Exists → REPLAY** (deterministic);
    /// **absent → RECORD** a live draw into it (needs `use-llm` + a key). Omit ⇒ no lander unless
    /// `--land-in-loop` is set.
    #[arg(long)]
    kinds: Option<PathBuf>,
    /// Install the D68/D71 [`DerivedClaimLander`] inside the discourse loop without an LLM kind
    /// draw (uses [`NoKindClassifier`]: deterministic `frame_kind` + `enc:Assertion` default).
    #[arg(long)]
    land_in_loop: bool,
    /// Where to write the recorded `kinds.json` draw produced by `--land-in-loop` without `--kinds`.
    #[arg(long)]
    kinds_out: Option<PathBuf>,
    /// Run the full 10-source deterministic Stage-A glossary (`augment_full_glossary`) instead of
    /// `DocumentOnly`.
    #[arg(long)]
    full_glossary: bool,
    /// Optional un-rewritten source document for Stage-A abbreviation extraction under
    /// `--full-glossary` (e.g. `docmodel-page.txt` when `--source` is pre-rewritten CNL).
    #[arg(long)]
    glossary_source: Option<PathBuf>,
    /// Treat each non-empty line of `--source` as one pre-segmented discourse unit.
    #[arg(long)]
    units_per_line: bool,
    /// ESL file(s) to chain-load onto the working copy's `main` before parsing, in order —
    /// vocabulary the parse needs that the LEXICON snapshot does not carry. Repeatable. The
    /// claim-kind machinery needs exactly this: a demonstrative's restrictor («these findings»)
    /// is vetoed against the claim's kind class, and that class plus its lexicon alignment
    /// (`ontologies/encoding/encoding.esl`, `ontologies/encoding/claim-kind-alignment.esl`) are
    /// not in the snapshot. Classes only — a lexical entry here would change the forest.
    #[arg(long = "chain-load")]
    chain_load: Vec<PathBuf>,
    /// IRI prefix for the emitted resources.
    #[arg(long, default_value = "urn:eigenius:demo:prose")]
    ns: String,
    /// An existing `reference:Reference` IRI for the source work — every emitted `DiscourseUnit`
    /// cites it, and the artifact emits no Reference of its own. It must already resolve on the
    /// chain the artifact loads onto, or Rule 22 rejects the load. Omit to mint a document-local
    /// `<ns>:source` Reference into the artifact (the right default for a plain text file).
    #[arg(long)]
    source_ref: Option<String>,
    /// Where to write the claims layer (units, encoded claims, ProgramTraces, decision points).
    /// Regenerated on every run — this is the layer the prose determines.
    #[arg(long)]
    out: PathBuf,
    /// Also write the emitted artifact as Eigon-JSON to this path in the same run (avoids a second
    /// parse pass when both `.esl` and `.eigon.json` are wanted).
    #[arg(long)]
    also_json: Option<PathBuf>,
    /// Write a D63 `reading-adjudications.tsv` ledger (`sentence <TAB> sem <TAB> uncertain <TAB>
    /// evidence`) for every encoded reading in this run, deduplicated by `(sentence, sem)`.
    #[arg(long)]
    adjudications_out: Option<PathBuf>,
    /// The `prov:timestamp` on each ProgramTrace. Fixed by the caller so the emission is
    /// byte-reproducible.
    #[arg(long, default_value = "2026-08-03T00:00:00Z")]
    timestamp: String,
    /// WordNet dict for the Morphy lemmatizer.
    #[arg(long, default_value = "references/WordNet-3.0/dict")]
    dict: PathBuf,
    /// Emit a PARTIAL artifact: a sentence that does not encode lands as its `DiscourseUnit` +
    /// an `enc:CutItem` naming the reason (ambiguous / unresolved referent / no parse) instead
    /// of aborting the run (D67 §5 — the artifact states what did not encode). Selection
    /// authority becomes optional: with neither `--pins` nor `--selections`, only sole-survivor
    /// readings encode and every multi-reading unit is cut. A pin that CONTRADICTS the encoded
    /// reading still aborts — that is pin drift, not a coverage gap.
    #[arg(long)]
    partial: bool,
    /// After writing the artifact, compile the emitted ESL back against the chain, build a
    /// persistent layer on the disposable working copy, and run `Validator::validate()` over every
    /// emitted resource (Rule 3, Rule 14, Rule 22 closed-world references, and EigenTT proposition
    /// well-typedness).
    #[arg(long)]
    verify_load: bool,
}

/// Write an emitted Eigon-JSON document in the requested format.
///
/// In [`OutputFormat::Esl`] the JSON is printed back as source. A document the printer cannot
/// express is an ERROR, not a fallback to JSON: silently writing a different format under a
/// `.esl` path would be worse than failing.
fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    Ok(())
}

fn write_doc(path: &Path, body: &[u8]) -> Result<(), String> {
    ensure_parent_dir(path)?;
    std::fs::write(path, body).map_err(|e| format!("write {}: {e}", path.display()))
}

/// A proposer that never proposes — no anaphora resolves; open parses stay `Open`.
struct NoProposer;
impl Proposer for NoProposer {
    fn propose(&self, _ctx: &ProposeCtx) -> Proposal {
        Proposal::default()
    }
}

/// Deterministic recency proposer (`enc:binding_recency`, `--no-llm`): ranks candidate antecedents
/// in discourse-recency order (`0..candidates.len()`), vetted by the kernel's restrictor + felicity
/// checks.
struct RecencyProposer;
impl Proposer for RecencyProposer {
    fn propose(&self, ctx: &ProposeCtx) -> Proposal {
        Proposal::ranked((0..ctx.candidates.len()).collect())
    }
}

pub fn run(args: &Args, format: OutputFormat) -> Result<(), String> {
    let doc = std::fs::read_to_string(&args.source)
        .map_err(|e| format!("read {}: {e}", args.source.display()))?;
    let sha = hex(&Sha256::digest(doc.as_bytes()));
    eprintln!("source: {} (sha256 {sha})", args.source.display());

    // Exactly one selection authority per run — an emission with a mixed or defaulted authority
    // would be unauditable. The pins double as the emission's byte-stable `Pinned` records.
    let pins =
        match (&args.pins, &args.selections, args.cost_first) {
            (Some(p), None, false) => {
                let pins = load_pins(p).map_err(|e| format!("read {}: {e}", p.display()))?;
                eprintln!("pins:   {} entries", pins.len());
                Some(pins)
            }
            (None, Some(_), false) | (None, None, true) => None,
            (None, None, false) if args.partial => {
                // NO authority — only sole-survivor readings encode; the rest are cuts.
                eprintln!("selection: NONE (--partial) — sole-survivor readings only");
                None
            }
            _ => return Err(
                "exactly one of --pins (declared arm) / --selections (computed arm) / --cost-first \
                 (deterministic offline arm) is required (or --partial with none: sole-survivor \
                 readings only)"
                    .to_string(),
            ),
        };
    #[cfg(feature = "use-llm")]
    let mut selection_recording: Option<(
        std::sync::Arc<RecordingReadingRanker<eigenius_kernel::dcg::LiveReadingRanker>>,
        PathBuf,
    )> = None;
    let mut cost_first_recording: Option<(
        std::sync::Arc<RecordingReadingRanker<CostFirstReadingRanker>>,
        PathBuf,
    )> = None;
    let ranker: Option<Box<dyn ReadingRanker>> = match (&pins, &args.selections, args.cost_first) {
        (Some(pins), _, _) => Some(Box::new(PinReadingRanker::new(
            pins.iter()
                .map(|(s, p)| (s.clone(), p.skeleton.clone()))
                .collect(),
        ))),
        (None, None, true) => {
            eprintln!(
                "selections: CostFirstReadingRanker (deterministic, no LLM){}",
                args.selections_out
                    .as_ref()
                    .map(|p| format!(" — RECORDING to {}", p.display()))
                    .unwrap_or_default()
            );
            let rec = std::sync::Arc::new(RecordingReadingRanker::new(CostFirstReadingRanker));
            if let Some(out_path) = &args.selections_out {
                cost_first_recording = Some((std::sync::Arc::clone(&rec), out_path.clone()));
            }
            Some(Box::new(rec))
        }
        (None, Some(s), false) if s.exists() && !args.resume => {
            let r =
                ReplayReadingRanker::load(s).map_err(|e| format!("read {}: {e}", s.display()))?;
            eprintln!("selections: REPLAY {} (deterministic, no LLM)", s.display());
            Some(Box::new(r))
        }
        (None, None, false) => None,
        (None, Some(s), false) => {
            // RECORD (or RESUME) mode — the live ranker answers and the draw is written after generation.
            #[cfg(feature = "use-llm")]
            {
                let Some(live) = eigenius_kernel::dcg::live_reading_ranker_from_env() else {
                    return Err(format!(
                        "--selections {} does not exist (RECORD mode) but the model's API key \
                         is unset",
                        s.display()
                    ));
                };
                let model_name = live.model().unwrap_or_default();
                let rec = std::sync::Arc::new(
                    RecordingReadingRanker::new(live).with_checkpoint(s.clone()),
                );
                let seeded = if args.resume && s.exists() {
                    rec.load_checkpoint(s)
                        .map_err(|e| format!("resume --selections {}: {e}", s.display()))?
                } else {
                    0
                };
                eprintln!(
                    "selections: live reading ranker, {model_name} — {} to {}{}",
                    if seeded > 0 { "RESUMING" } else { "RECORDING" },
                    s.display(),
                    if seeded > 0 {
                        format!(" ({seeded} checkpointed selections preloaded)")
                    } else {
                        String::new()
                    }
                );
                selection_recording = Some((std::sync::Arc::clone(&rec), s.clone()));
                Some(Box::new(rec))
            }
            #[cfg(not(feature = "use-llm"))]
            return Err(format!(
                "--selections {} does not exist, and this binary has no live reading ranker \
                 (built without --features use-llm) — to replay, point at an existing \
                 selections.json; to record, rebuild with the feature",
                s.display()
            ));
        }
        _ => unreachable!("guarded by selection-authority check above"),
    };

    // Anaphora arm (D64/D67 §3), same three-arm discipline as every other recorded stage.
    // Without it no referent hole resolves.
    // RECORD mode is decided by the same condition the arm below matches on (a named draw that
    // does not exist yet), so it needs no mutation — and stays warning-clean in the build without
    // a live proposer, where that arm returns an error instead.
    if args.proposals.is_some() && args.recency {
        return Err("--proposals and --recency are mutually exclusive".to_string());
    }
    let proposal_recording = matches!(&args.proposals, Some(p) if !p.exists());
    let inner_proposer: Box<dyn Proposer> = match (&args.proposals, args.recency) {
        (_, true) => {
            eprintln!(
                "proposals:  RecencyProposer (deterministic, no LLM){}",
                args.proposals_out
                    .as_ref()
                    .map(|p| format!(" — RECORDING to {}", p.display()))
                    .unwrap_or_default()
            );
            Box::new(RecencyProposer)
        }
        (Some(p), false) if p.exists() => {
            let r = ReplayProposer::load(p).map_err(|e| format!("read {}: {e}", p.display()))?;
            eprintln!("proposals:  REPLAY {} (deterministic, no LLM)", p.display());
            Box::new(r)
        }
        (Some(p), false) => {
            #[cfg(feature = "use-llm")]
            {
                let Some(live) = eigenius_kernel::dcg::resolver_llm::AnthropicProposer::from_env()
                else {
                    return Err(format!(
                        "--proposals {} does not exist (RECORD mode) but ANTHROPIC_API_KEY is                          unset",
                        p.display()
                    ));
                };
                eprintln!(
                    "proposals:  AnthropicProposer (live) — RECORDING to {}",
                    p.display()
                );
                Box::new(live) as Box<dyn Proposer>
            }
            #[cfg(not(feature = "use-llm"))]
            return Err(format!(
                "--proposals {} does not exist, and this binary has no live proposer (built                  without --features use-llm)",
                p.display()
            ));
        }
        (None, false) => Box::new(NoProposer),
    };
    // The recorder wraps whichever arm (memoizing), so a run always leaves the draw that
    // reproduces it; a replayed draw re-recorded is a no-op copy.
    let proposer = eigenius_kernel::dcg::RecordingProposer::new(inner_proposer);
    let binding_authority = if args.recency {
        Some("binding_recency")
    } else {
        args.proposals.as_ref().map(|_| {
            if proposal_recording {
                "binding_proposer"
            } else {
                "binding_replay"
            }
        })
    };

    let (head, backend) = open_head_and_backend(&args.snapshot)?;
    // Chain-load the extra vocabulary onto the working copy's main (§7-2: build the layer ON the
    // storage it is persisted to). The working copy is disposable, so this never touches the
    // source snapshot.
    let mut head = head;
    for path in &args.chain_load {
        let src =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("chain-load");
        let resources = eigenius_kernel::esl::compile(&src, &head).map_err(|e| {
            format!(
                "{} does not compile against the chain: {e:?}",
                path.display()
            )
        })?;
        // Guard against re-loading an ontology already present on `head` (redefining a
        // chain-resident resource triggers Rule 22 retroactive validation across the 7.6M-resource
        // store).
        if !resources.is_empty()
            && resources
                .iter()
                .all(|r| r.id().is_some_and(|id| head.resolve(id).is_some()))
        {
            eprintln!("chain-load: {} already on chain (skipping)", path.display());
            continue;
        }
        let mut b =
            eigenius_kernel::layer::LayerBuilder::new(name, Some(std::sync::Arc::clone(&head)));
        for r in resources {
            b.add_resource(r)
                .map_err(|e| format!("{}: {e:?}", path.display()))?;
        }
        let layer = std::sync::Arc::new(b.build(
            eigenius_kernel::layer::LayerStorage::with_persistent(std::sync::Arc::clone(&backend)),
        ));
        use eigenius_kernel::commit::LayerPersister;
        let info =
            eigenius_kernel::commit::BackendPersister::new(Some(std::sync::Arc::clone(&backend)))
                .persist("main", &layer)
                .map_err(|e| format!("persist {}: {e:?}", path.display()))?;
        if !info.branch_advanced {
            return Err(format!(
                "chain-load {} did not advance main",
                path.display()
            ));
        }
        eprintln!("chain-load: {} → {}", path.display(), layer.id());
        head = layer;
    }
    let lem = MorphyLemmatizer::load(&args.dict)
        .map_err(|e| format!("load Morphy from {}: {e}", args.dict.display()))?;
    // The doc layer goes through the PERSISTENT path (D67 §2), committed to a `doc-<id>` branch
    // of the working copy (disposable — removed on exit): an in-memory doc layer over the
    // DB-backed base OOMs (§7-2, build-time index population walks the full chain).
    let doc_id: String = args
        .source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("prose")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    // The sense-rank arm, installed through the pipeline's parser hook (the pipeline builds its
    // own parser over the doc layer). Built HERE so its errors surface before any parse and the
    // RECORD arm's artifact can be flushed after.
    let (sense_ranker, recording) =
        build_sense_ranker_with_model(&args.ranks, args.sense_model.as_deref(), args.resume)?;
    let ranker_slot = RefCell::new(sense_ranker);
    let setup = move |p: Parser| {
        let p = p.with_sense_cap(SENSE_CAP).with_cell_beam(CELL_BEAM);
        match ranker_slot.borrow_mut().take() {
            Some(r) => p.with_sense_ranker(r),
            None => p,
        }
    };

    // The CLAIM LANDER (D68): with a kind draw (or `--land-in-loop`), each closed sentence's claim
    // is graded and landed INSIDE the discourse loop, which is what makes it available as an
    // antecedent to a later demonstrative. The kinds arm mirrors every other recorded stage
    // (exists → replay, absent + live → record); `--land-in-loop` without `--kinds` installs the
    // deterministic floor (`NoKindClassifier`: `frame_kind` + `enc:Assertion`).
    let kind_classifier: Option<Box<dyn crate::KindClassifier>> =
        match (&args.kinds, args.land_in_loop) {
            (Some(p), _) if p.exists() => {
                let r = crate::ReplayKindClassifier::load(p)
                    .map_err(|e| format!("read {}: {e}", p.display()))?;
                eprintln!("kinds:      REPLAY {} (deterministic, no LLM)", p.display());
                Some(Box::new(r))
            }
            (Some(p), _) => {
                #[cfg(feature = "use-llm")]
                {
                    let Some(c) = crate::AnthropicKindClassifier::from_env(&doc) else {
                        return Err(format!(
                            "--kinds {} does not exist (RECORD mode) but ANTHROPIC_API_KEY is unset",
                            p.display()
                        ));
                    };
                    eprintln!(
                        "kinds:      AnthropicKindClassifier (live) — RECORDING to {}",
                        p.display()
                    );
                    Some(Box::new(c) as Box<dyn crate::KindClassifier>)
                }
                #[cfg(not(feature = "use-llm"))]
                return Err(format!(
                    "--kinds {} does not exist, and this binary has no live classifier (built \
                     without --features use-llm)",
                    p.display()
                ));
            }
            (None, true) => {
                eprintln!(
                    "kinds:      NoKindClassifier (deterministic frame_kind + Assertion){}",
                    args.kinds_out
                        .as_ref()
                        .map(|p| format!(" — RECORDING to {}", p.display()))
                        .unwrap_or_default()
                );
                Some(Box::new(crate::NoKindClassifier))
            }
            (None, false) => None,
        };
    let kind_recorder = kind_classifier.map(crate::RecordingKindClassifier::new);
    // The codec's constructor argument names, from the chain this run emits against.
    let codec = eigenius_kernel::program::eigentt_type_mirror::CodecNames::from_layer(&head);
    let print_chain = std::sync::Arc::clone(&head);
    // ONE claim identity: the lander names claims exactly as the emitter will, so the
    // `enc:AnaphorBinding` this run records points at resources this run's artifact contains.
    let lander = kind_recorder.as_ref().map(|k| {
        crate::DerivedClaimLander::new(&doc_id, k, codec.clone())
            .with_emission_namespace(&args.ns)
            .with_source(&format!("{} (sha256 {sha})", args.source.display()))
            .with_timestamp(&args.timestamp)
    });

    let glossary_source_text = match &args.glossary_source {
        Some(p) => Some(
            std::fs::read_to_string(p)
                .map_err(|e| format!("read glossary source {}: {e}", p.display()))?,
        ),
        None => None,
    };
    let verify_backend = std::sync::Arc::clone(&backend);
    let mut pipeline = InProcessPipeline::new(head, &lem, &NoAbbreviationProposer, &proposer)
        .with_parser_setup(&setup)
        .with_storage(backend, &doc_id)
        .with_units_per_line(args.units_per_line);
    if args.full_glossary {
        pipeline = pipeline.with_augment_options(AugmentOptions::FullGlossary {
            source_document: glossary_source_text,
        });
    }
    if let Some(r) = &ranker {
        pipeline = pipeline.with_reading_ranker(r);
    }
    if let Some(l) = &lander {
        pipeline = pipeline.with_claim_lander(l);
    }
    let (encoding, doc_layer) = pipeline
        .encode_with_layer(&doc)
        .map_err(|e| format!("{e}"))?;
    // Flush the proposal draw before any fail-closed abort below.
    if proposal_recording {
        if let Some(p) = &args.proposals {
            ensure_parent_dir(p)?;
            let n = proposer
                .write(p)
                .map_err(|e| format!("write {}: {e}", p.display()))?;
            eprintln!("proposals:  recorded {n} proposal(s) → {}", p.display());
        }
    }
    if let Some(p) = &args.proposals_out {
        ensure_parent_dir(p)?;
        let n = proposer
            .write(p)
            .map_err(|e| format!("write {}: {e}", p.display()))?;
        eprintln!("proposals:  recorded {n} proposal(s) → {}", p.display());
    }
    // Flush the kind draw before any fail-closed abort below.
    if let (Some(rec), Some(p)) = (&kind_recorder, &args.kinds) {
        if !p.exists() {
            ensure_parent_dir(p)?;
            let n = rec
                .write(p)
                .map_err(|e| format!("write {}: {e}", p.display()))?;
            eprintln!("kinds:      recorded {n} verdict(s) → {}", p.display());
        }
    }
    if let (Some(rec), Some(p)) = (&kind_recorder, &args.kinds_out) {
        ensure_parent_dir(p)?;
        let n = rec
            .write(p)
            .map_err(|e| format!("write {}: {e}", p.display()))?;
        eprintln!("kinds:      recorded {n} verdict(s) → {}", p.display());
    }
    // The lander's clusters, keyed by claim IRI — the artifact emits THESE (they carry the
    // discourse kind an anaphor's restrictor was checked against).
    let landed: BTreeMap<
        String,
        (
            eigenius_kernel::ontology::resource::Resource,
            eigenius_kernel::ontology::resource::Resource,
        ),
    > = match &lander {
        Some(l) => {
            let clusters = l.take_landed();
            eprintln!("claims:     {} landed in-loop", clusters.len());
            clusters
                .into_iter()
                .filter_map(|c| {
                    let mut it = c.resources.into_iter();
                    let claim = it.next()?;
                    let trace = it.next()?;
                    Some((claim.id()?.as_str().to_string(), (claim, trace)))
                })
                .collect()
        }
        None => Default::default(),
    };
    // A RECORD run leaves its ranks artifact even if a pin then fails to match below — the
    // recording is exactly what a re-run needs to diagnose the mismatch deterministically.
    recording.flush()?;
    #[cfg(feature = "use-llm")]
    if let Some((rec, path)) = &selection_recording {
        ensure_parent_dir(path)?;
        let n = rec
            .write(path)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        eprintln!("selections: recorded {n} decision(s) → {}", path.display());
    }
    if let Some((rec, path)) = &cost_first_recording {
        ensure_parent_dir(path)?;
        let n = rec
            .write(path)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        eprintln!("selections: recorded {n} decision(s) → {}", path.display());
    }
    if let Some(adj_path) = &args.adjudications_out {
        let n = write_adjudications_tsv(adj_path, &encoding)?;
        eprintln!(
            "adjudications: wrote {n} unique (sentence, sem) row(s) → {}",
            adj_path.display()
        );
    }

    // Emission is the SHARED half (D71 §7 / slice 5a): outcome -> record under the run's selection
    // authority, fail-closed or partial, then the artifact. Identical work for every surface, so it
    // lives in `formalize` and this driver only supplies the inputs.
    let typed_codec = codec.with_typing_layer(std::sync::Arc::clone(&doc_layer));
    let artifact = crate::formalize::emit_from_encoding(&crate::formalize::EmissionInputs {
        codec: &typed_codec,
        doc: &doc,
        encoding: &encoding,
        landed: &landed,
        pins: pins.as_ref(),
        binding_authority,
        partial: args.partial,
        meta: DocumentMeta {
            ns: &args.ns,
            source_path: &args.source.display().to_string(),
            source_sha256: &sha,
            timestamp: &args.timestamp,
            // No agent is threaded through this surface yet, so the claim names the absence
            // rather than hiding it behind the program that parsed it (D72).
            // Supplying a real `prov:Agent` is D71's `land` story: the moment a
            // formulation becomes an assertion is the moment someone takes responsibility.
            declared_by: crate::UNATTRIBUTED_AGENT,
            source_ref: args.source_ref.as_deref(),
        },
    })?;
    // The CLI's `--out` decides the encoding; the kernel owns the mapping, so this is the one
    // place the driver expresses a format at all. When `--also-json` is supplied alongside ESL
    // output, write the Eigon-JSON first so a single parse run emits both formats.
    use eigenius_kernel::dcg::formalizer::{render_artifact, ArtifactFormat};
    if let Some(json_path) = &args.also_json {
        let json_rendered =
            render_artifact(&artifact.resources, ArtifactFormat::EigonJson, &print_chain)?;
        write_doc(json_path, &json_rendered)?;
        eprintln!(
            "wrote {} ({} encoded, {} cut, {} glossary)",
            json_path.display(),
            artifact.encoded,
            artifact.cut,
            artifact.glossary
        );
    }
    let rendered = render_artifact(
        &artifact.resources,
        match format {
            OutputFormat::Json => ArtifactFormat::EigonJson,
            OutputFormat::Esl => ArtifactFormat::Esl,
        },
        &print_chain,
    )?;
    write_doc(&args.out, &rendered)?;
    eprintln!(
        "\nwrote {} ({} encoded, {} cut, {} glossary)",
        args.out.display(),
        artifact.encoded,
        artifact.cut,
        artifact.glossary
    );

    if args.verify_load {
        let esl_bytes = match format {
            OutputFormat::Esl => rendered,
            OutputFormat::Json => {
                render_artifact(&artifact.resources, ArtifactFormat::Esl, &print_chain)?
            }
        };
        let esl_src = std::str::from_utf8(&esl_bytes)
            .map_err(|e| format!("rendered ESL is not valid UTF-8: {e}"))?;
        let compiled = eigenius_kernel::esl::compile(esl_src, &print_chain)
            .map_err(|e| format!("verify-load: emitted ESL failed to compile: {e:?}"))?;
        let mut b = eigenius_kernel::layer::LayerBuilder::new(
            "verify-artifact",
            Some(std::sync::Arc::clone(&print_chain)),
        );
        let resource_count = compiled.len();
        for r in compiled {
            b.add_resource(r)
                .map_err(|e| format!("verify-load: LayerBuilder::add_resource failed: {e:?}"))?;
        }
        let verify_layer = std::sync::Arc::new(b.build(
            eigenius_kernel::layer::LayerStorage::with_persistent(verify_backend),
        ));
        let errors = eigenius_kernel::validation::Validator::new(std::sync::Arc::clone(
            &verify_layer,
        ))
        .validate();
        if !errors.is_empty() {
            let preview: Vec<String> = errors
                .iter()
                .take(15)
                .map(|e| format!("  - {:?}: {}", e.resource_id, e.message))
                .collect();
            return Err(format!(
                "verify-load: {} validation error(s) across {resource_count} compiled resource(s):\n{}",
                errors.len(),
                preview.join("\n")
            ));
        }
        eprintln!(
            "verify-load: OK — {resource_count} resource(s) compiled from ESL and passed kernel Validator (0 errors)"
        );
    }

    Ok(())
}

/// Emit a D63 reading-adjudication ledger (`sentence <TAB> sem <TAB> uncertain <TAB> evidence`)
/// deduplicated by `(sentence, sem)` so repeated sentences in multi-section reports satisfy the
/// ledger's unique-key invariant (`ledger_contradictions`).
fn write_adjudications_tsv(
    path: &Path,
    encoding: &eigenius_kernel::dcg::DocumentEncoding,
) -> Result<usize, String> {
    let mut rows: BTreeMap<(String, String), (Vec<usize>, String)> = BTreeMap::new();
    for (idx, se) in encoding.sentences.iter().enumerate() {
        let SentenceOutcome::Encoded(item) = &se.outcome else {
            continue;
        };
        let unit = idx + 1;
        let sentence = se.text.trim().replace(['\t', '\r', '\n'], " ");
        let sem = se
            .selection
            .as_ref()
            .map(|s| s.chosen_sem.clone())
            .unwrap_or_else(|| eigenius_kernel::dcg::pretty_term(item.sem()))
            .replace(['\t', '\r', '\n'], " ");
        let note = match &se.selection {
            Some(sel) => format!(
                "ranked (1 of {}): {} | gloss: {}",
                sel.candidates,
                sel.rationale.replace(['\t', '\r', '\n'], " "),
                sel.chosen_gloss.replace(['\t', '\r', '\n'], " ")
            ),
            None => "sole surviving reading".to_string(),
        };
        let entry = rows
            .entry((sentence, sem))
            .or_insert_with(|| (Vec::new(), note));
        entry.0.push(unit);
    }

    let mut out = String::from(
        "# Reading-level adjudication ledger (D63 par.5): sentence <TAB> sem <TAB> verdict <TAB> \
         evidence [<TAB> basis]\n\
         # Machine-seeded as `uncertain` (empty basis) from deterministic reading selection; \
         awaiting human review.\n",
    );
    for ((sentence, sem), (units, note)) in &rows {
        let unit_label = if units.len() == 1 {
            format!("unit {}", units[0])
        } else {
            let list: Vec<String> = units.iter().map(usize::to_string).collect();
            format!("units {}", list.join(","))
        };
        out.push_str(&format!("{sentence}\t{sem}\tuncertain\t{unit_label} | {note}\n"));
    }
    write_doc(path, out.as_bytes())?;
    Ok(rows.len())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
