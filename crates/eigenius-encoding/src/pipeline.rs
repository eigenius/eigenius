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
//! prose-to-esl --snapshot ../db-snapshot/wordnet-umls-aligned-2026-08-15-d70b \
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
use std::path::{Path, PathBuf};

use crate::emit::{
    emit_document, CutReason, CutSentence, DocumentMeta, ParsedSentence, SentenceSelection,
};
use crate::select::load_pins;
use crate::snapshot::{build_sense_ranker, open_head_and_backend, CELL_BEAM, SENSE_CAP};
use clap::Parser as ClapParser;
use eigenius_kernel::dcg::skeleton::skeleton_of;
use eigenius_kernel::dcg::{
    InProcessPipeline, NoAbbreviationProposer, Parser, PinReadingRanker, Proposal, ProposeCtx,
    Proposer, ReadingRanker, ReplayProposer, ReplayReadingRanker, SentenceOutcome,
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
    /// Exactly one of `--pins` / `--selections` is required.
    #[arg(long)]
    pins: Option<PathBuf>,
    /// A reading-selection draw (`selections.json`) — the COMPUTED selection arm
    /// (d63-reading-selection.md). **Exists → REPLAY** (deterministic, no LLM); **absent →
    /// RECORD** a live draw into it (needs `use-llm` + a key) while generating. The record arm
    /// lives HERE because selection keys hash the presented pool, and this pipeline's pool
    /// (its own Stage A) is not the measurement harness's — a draw recorded there cannot
    /// answer this driver's questions (found 2026-08-12). Exactly one of `--pins` /
    /// `--selections` is required.
    #[arg(long)]
    selections: Option<PathBuf>,
    /// A recorded `ranks.json` to replay — deterministic, no LLM. Omit for cap-only (a DIFFERENT
    /// experiment: sense elimination is off, so the pins may not match).
    #[arg(long)]
    ranks: Option<PathBuf>,
    /// An anaphora-proposal draw (`proposals.json`) — D64/D67 §3, resolves referent holes
    /// through the discourse loop. **Exists → REPLAY** (deterministic, no LLM); **absent →
    /// RECORD** a live draw into it (needs `use-llm` + a key). Omit ⇒ no anaphora resolves (open
    /// parses stay Open — and abort the emission, which is fail-closed).
    #[arg(long)]
    proposals: Option<PathBuf>,
    /// A recorded discourse-KIND draw (`kinds.json`) — D68. Installing it turns on the CLAIM
    /// LANDER: each closed sentence's claim is graded and landed INSIDE the discourse loop, so a
    /// later demonstrative («these findings») can bind to it. **Exists → REPLAY** (deterministic);
    /// **absent → RECORD** a live draw into it (needs `use-llm` + a key). Omit ⇒ no lander: claims
    /// are emitted at the end as before and no claim is available as an antecedent.
    #[arg(long)]
    kinds: Option<PathBuf>,
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
    /// The `reflection:timestamp` on each ProgramTrace. Fixed by the caller so the emission is
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
}

/// Write an emitted Eigon-JSON document in the requested format.
///
/// In [`OutputFormat::Esl`] the JSON is printed back as source. A document the printer cannot
/// express is an ERROR, not a fallback to JSON: silently writing a different format under a
/// `.esl` path would be worse than failing.
fn write_doc(path: &Path, json: &str, format: OutputFormat) -> Result<(), String> {
    let body = match format {
        OutputFormat::Json => json.to_string(),
        OutputFormat::Esl => {
            let doc: serde_json::Value = serde_json::from_str(json)
                .map_err(|e| format!("emitted document is not valid JSON: {e}"))?;
            // Pretty: this binary exists to produce source a person reads. A parsed sentence's
            // proposition is a deep application spine, and on one line its structure is invisible.
            eigenius_kernel::esl::print::print_document_with(
                &doc,
                eigenius_kernel::esl::print::Layout::Pretty,
            )
            .map_err(|e| format!("cannot print {} as ESL: {e}", path.display()))?
        }
    };
    std::fs::write(path, body).map_err(|e| format!("write {}: {e}", path.display()))
}

/// A proposer that never proposes — no anaphora resolves; open parses stay `Open`.
struct NoProposer;
impl Proposer for NoProposer {
    fn propose(&self, _ctx: &ProposeCtx) -> Proposal {
        Proposal::default()
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
        match (&args.pins, &args.selections) {
            (Some(p), None) => {
                let pins = load_pins(p).map_err(|e| format!("read {}: {e}", p.display()))?;
                eprintln!("pins:   {} entries", pins.len());
                Some(pins)
            }
            (None, Some(_)) => None,
            (None, None) if args.partial => {
                // NO authority — only sole-survivor readings encode; the rest are cuts.
                eprintln!("selection: NONE (--partial) — sole-survivor readings only");
                None
            }
            _ => return Err(
                "exactly one of --pins (declared arm) / --selections (computed arm) is required \
                 (or --partial with neither: sole-survivor readings only)"
                    .to_string(),
            ),
        };
    #[cfg(feature = "use-llm")]
    let mut selection_recording: Option<(
        std::sync::Arc<
            eigenius_kernel::dcg::RecordingReadingRanker<
                eigenius_kernel::dcg::AnthropicReadingRanker,
            >,
        >,
        PathBuf,
    )> = None;
    let ranker: Option<Box<dyn ReadingRanker>> = match (&pins, &args.selections) {
        (Some(pins), _) => Some(Box::new(PinReadingRanker::new(
            pins.iter()
                .map(|(s, p)| (s.clone(), p.skeleton.clone()))
                .collect(),
        ))),
        (None, Some(s)) if s.exists() => {
            let r =
                ReplayReadingRanker::load(s).map_err(|e| format!("read {}: {e}", s.display()))?;
            eprintln!("selections: REPLAY {} (deterministic, no LLM)", s.display());
            Some(Box::new(r))
        }
        (None, None) => None,
        (None, Some(s)) => {
            // RECORD mode — the live ranker answers and the draw is written after generation.
            #[cfg(feature = "use-llm")]
            {
                let Some(live) = eigenius_kernel::dcg::AnthropicReadingRanker::from_env() else {
                    return Err(format!(
                        "--selections {} does not exist (RECORD mode) but ANTHROPIC_API_KEY \
                         is unset",
                        s.display()
                    ));
                };
                eprintln!(
                    "selections: AnthropicReadingRanker (live) — RECORDING to {}",
                    s.display()
                );
                let rec =
                    std::sync::Arc::new(eigenius_kernel::dcg::RecordingReadingRanker::new(live));
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
    };

    // Anaphora arm (D64/D67 §3), same three-arm discipline as every other recorded stage.
    // Without it no referent hole resolves.
    // RECORD mode is decided by the same condition the arm below matches on (a named draw that
    // does not exist yet), so it needs no mutation — and stays warning-clean in the build without
    // a live proposer, where that arm returns an error instead.
    let proposal_recording = matches!(&args.proposals, Some(p) if !p.exists());
    let inner_proposer: Box<dyn Proposer> = match &args.proposals {
        Some(p) if p.exists() => {
            let r = ReplayProposer::load(p).map_err(|e| format!("read {}: {e}", p.display()))?;
            eprintln!("proposals:  REPLAY {} (deterministic, no LLM)", p.display());
            Box::new(r)
        }
        Some(p) => {
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
        None => Box::new(NoProposer),
    };
    // The recorder wraps whichever arm (memoizing), so a run always leaves the draw that
    // reproduces it; a replayed draw re-recorded is a no-op copy.
    let proposer = eigenius_kernel::dcg::RecordingProposer::new(inner_proposer);
    let binding_authority = args.proposals.as_ref().map(|_| {
        if proposal_recording {
            "binding_proposer"
        } else {
            "binding_replay"
        }
    });

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
        let resources = eigenius_kernel::esl::compile_against_layer(&src, &head).map_err(|e| {
            format!(
                "{} does not compile against the chain: {e:?}",
                path.display()
            )
        })?;
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
    let (sense_ranker, recording) = build_sense_ranker(&args.ranks)?;
    let ranker_slot = RefCell::new(sense_ranker);
    let setup = move |p: Parser| {
        let p = p.with_sense_cap(SENSE_CAP).with_cell_beam(CELL_BEAM);
        match ranker_slot.borrow_mut().take() {
            Some(r) => p.with_sense_ranker(r),
            None => p,
        }
    };

    // The CLAIM LANDER (D68): with a kind draw, each closed sentence's claim is graded and landed
    // INSIDE the discourse loop, which is what makes it available as an antecedent to a later
    // demonstrative. The kinds arm mirrors every other recorded stage (exists → replay, absent +
    // live → record); the recorder wraps whichever arm, so a run always leaves its draw.
    let kind_classifier: Option<Box<dyn eigenius_reasoning::KindClassifier>> = match &args.kinds {
        Some(p) if p.exists() => {
            let r = eigenius_reasoning::ReplayKindClassifier::load(p)
                .map_err(|e| format!("read {}: {e}", p.display()))?;
            eprintln!("kinds:      REPLAY {} (deterministic, no LLM)", p.display());
            Some(Box::new(r))
        }
        Some(p) => {
            #[cfg(feature = "use-llm")]
            {
                let Some(c) = eigenius_reasoning::AnthropicKindClassifier::from_env(&doc) else {
                    return Err(format!(
                        "--kinds {} does not exist (RECORD mode) but ANTHROPIC_API_KEY is unset",
                        p.display()
                    ));
                };
                eprintln!(
                    "kinds:      AnthropicKindClassifier (live) — RECORDING to {}",
                    p.display()
                );
                Some(Box::new(c) as Box<dyn eigenius_reasoning::KindClassifier>)
            }
            #[cfg(not(feature = "use-llm"))]
            return Err(format!(
                "--kinds {} does not exist, and this binary has no live classifier (built \
                 without --features use-llm)",
                p.display()
            ));
        }
        None => None,
    };
    let kind_recorder = kind_classifier.map(eigenius_reasoning::RecordingKindClassifier::new);
    // ONE claim identity: the lander names claims exactly as the emitter will, so the
    // `enc:AnaphorBinding` this run records points at resources this run's artifact contains.
    let lander = kind_recorder.as_ref().map(|k| {
        eigenius_reasoning::DerivedClaimLander::new(&doc_id, k).with_emission_namespace(&args.ns)
    });

    let mut pipeline = InProcessPipeline::new(head, &lem, &NoAbbreviationProposer, &proposer)
        .with_parser_setup(&setup)
        .with_storage(backend, &doc_id);
    if let Some(r) = &ranker {
        pipeline = pipeline.with_reading_ranker(r);
    }
    if let Some(l) = &lander {
        pipeline = pipeline.with_claim_lander(l);
    }
    let (encoding, _doc_layer) = pipeline
        .encode_with_layer(&doc)
        .map_err(|e| format!("{e}"))?;
    // Flush the proposal draw before any fail-closed abort below.
    if proposal_recording {
        if let Some(p) = &args.proposals {
            let n = proposer
                .write(p)
                .map_err(|e| format!("write {}: {e}", p.display()))?;
            eprintln!("proposals:  recorded {n} proposal(s) → {}", p.display());
        }
    }
    // Flush the kind draw before any fail-closed abort below.
    if let (Some(rec), Some(p)) = (&kind_recorder, &args.kinds) {
        if !p.exists() {
            let n = rec
                .write(p)
                .map_err(|e| format!("write {}: {e}", p.display()))?;
            eprintln!("kinds:      recorded {n} verdict(s) → {}", p.display());
        }
    }
    // The lander's clusters, keyed by claim IRI — the artifact emits THESE (they carry the
    // discourse kind an anaphor's restrictor was checked against).
    let landed: std::collections::BTreeMap<
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
        let n = rec
            .write(path)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        eprintln!("selections: recorded {n} decision(s) → {}", path.display());
    }

    // Map each sentence's outcome to the emission record. Default: fail-closed on anything that
    // did not encode under the chosen authority. Under `--partial`: the non-encoding lands as a
    // `CutSentence` (DiscourseUnit + CutItem) and the run continues — the artifact states what
    // did not encode (D67 §5).
    let mut parsed: Vec<ParsedSentence> = Vec::new();
    let mut cuts: Vec<CutSentence> = Vec::new();
    let cut = |cuts: &mut Vec<CutSentence>, n: usize, se_text: &str, reason: CutReason| {
        let label = match &reason {
            CutReason::Ambiguous { readings } => format!("ambiguous ({readings} readings)"),
            CutReason::Unresolved { holes } => format!("unresolved ({holes} hole(s))"),
            CutReason::NoParse { oov } if !oov.is_empty() => format!("no parse (OOV: {oov:?})"),
            CutReason::NoParse { .. } => "no parse (grammar)".to_string(),
        };
        eprintln!("  [{n}] CUT — {label} — {}", se_text.trim());
        let start = doc.find(se_text).unwrap_or(0);
        cuts.push(CutSentence {
            ordinal: n,
            text: se_text.to_string(),
            span: (start, start + se_text.len()),
            reason,
        });
    };
    for (i, se) in encoding.sentences.iter().enumerate() {
        let n = i + 1;
        let text = se.text.trim();
        let item = match &se.outcome {
            SentenceOutcome::Encoded(item) => item,
            SentenceOutcome::Ambiguous(pool) => {
                if args.partial {
                    cut(
                        &mut cuts,
                        n,
                        &se.text,
                        CutReason::Ambiguous {
                            readings: pool.len(),
                        },
                    );
                    continue;
                }
                let skels: Vec<String> = pool.iter().map(|it| skeleton_of(it.sem())).collect();
                return Err(match &pins {
                    Some(pins) => match pins.get(text) {
                        None => format!("sentence {n} «{text}»: no pin, {} readings", pool.len()),
                        Some(pin) => {
                            let hits = skels.iter().filter(|s| **s == pin.skeleton).count();
                            if hits == 0 {
                                format!(
                                    "sentence {n} «{text}»: the pinned skeleton matches none of \
                                     the {} readings\n  pinned: {}\n  forest:\n    {}",
                                    pool.len(),
                                    pin.skeleton,
                                    skels.join("\n    ")
                                )
                            } else {
                                format!(
                                    "sentence {n} «{text}»: the pinned skeleton matches {hits} \
                                     readings — a sense-level tie a skeleton pin cannot break \
                                     (fail-closed)",
                                )
                            }
                        }
                    },
                    None => format!(
                        "sentence {n} «{text}»: the selection replay abstained or missed \
                         ({} readings) — the recording does not answer this question",
                        pool.len()
                    ),
                });
            }
            SentenceOutcome::Open(o) => {
                if args.partial {
                    cut(
                        &mut cuts,
                        n,
                        &se.text,
                        CutReason::Unresolved {
                            holes: o.holes.len(),
                        },
                    );
                    continue;
                }
                return Err(format!(
                    "sentence {n} «{text}»: {} unresolved referent hole(s) — provide --proposals \
                     with a recorded draw that resolves them, or pick prose without anaphora",
                    o.holes.len()
                ));
            }
            SentenceOutcome::Gap => {
                if args.partial {
                    // Classify: residual Stage-A OOV surfaces occurring in this sentence make it
                    // a vocabulary cut; none makes it a grammar cut.
                    let oov: Vec<String> = encoding
                        .augmentation
                        .missing_oov
                        .iter()
                        .map(|g| g.surface.clone())
                        .filter(|s| contains_word(&se.text, s))
                        .collect();
                    cut(&mut cuts, n, &se.text, CutReason::NoParse { oov });
                    continue;
                }
                return Err(format!(
                    "sentence {n} «{text}»: no parse — a grammar gap or out-of-vocabulary tokens"
                ));
            }
        };
        // The emission's selection record. Under pins, verify the encoded reading IS the pinned
        // one even when it was the sole survivor (the ranker only fires on pools > 1). A pin
        // CONTRADICTION stays fatal even under --partial (pin drift, not a coverage gap); a
        // missing pin for a sole survivor is tolerated under --partial (no choice existed).
        let selection = match &pins {
            Some(pins) => match pins.get(text) {
                None if args.partial => SentenceSelection::Sole,
                None => return Err(format!("sentence {n} «{text}»: no pin")),
                Some(pin) => {
                    let sk = skeleton_of(item.sem());
                    if sk != pin.skeleton {
                        return Err(format!(
                            "sentence {n} «{text}»: the encoded reading is not the pinned one\n  \
                             pinned: {}\n  got:    {sk}",
                            pin.skeleton
                        ));
                    }
                    SentenceSelection::Pinned(pin)
                }
            },
            None => match &se.selection {
                Some(sel) => SentenceSelection::Ranked(sel),
                None => SentenceSelection::Sole,
            },
        };
        let candidates = se.selection.as_ref().map(|s| s.candidates).unwrap_or(1);
        eprintln!(
            "  [{n}] encoded (of {candidates} reading(s)){} — {text}",
            if se.resolution.is_some() {
                " [anaphora resolved]"
            } else {
                ""
            }
        );
        let start = doc.find(se.text.as_str()).unwrap_or(0);
        parsed.push(ParsedSentence {
            ordinal: n,
            text: se.text.clone(),
            span: (start, start + se.text.len()),
            item,
            candidates,
            selection,
            bindings: se
                .resolution
                .as_ref()
                .map(|r| r.bindings.clone())
                .unwrap_or_default(),
            binding_authority,
            cluster: landed.get(&format!("{}:claim_{n}", args.ns)).cloned(),
        });
    }

    // Stage-A glossary resources go into the artifact — the entries that grounded the parse
    // (a claim's proposition may reference a doc-glossary-only concept; without them the
    // artifact does not load on a chain that lacks the doc branch).
    let glossary = encoding.augmentation.resources();
    if !glossary.is_empty() {
        eprintln!("glossary: {} Stage-A resource(s) emitted", glossary.len());
    }
    let json = emit_document(
        &DocumentMeta {
            ns: &args.ns,
            source_path: &args.source.display().to_string(),
            source_sha256: &sha,
            timestamp: &args.timestamp,
            source_ref: args.source_ref.as_deref(),
        },
        &glossary,
        &parsed,
        &cuts,
    )
    .map_err(|e| e.to_string())?;
    write_doc(&args.out, &json, format)?;
    eprintln!(
        "\nwrote {} ({} encoded, {} cut, {} glossary)",
        args.out.display(),
        parsed.len(),
        cuts.len(),
        glossary.len()
    );

    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Does `sentence` contain `word` as a whole token (case-insensitive)? Attributing a residual
/// OOV surface to a sentence by substring would credit «then» to «strengthen» — the artifact's
/// cut reason has to name surfaces the sentence actually contains. Alphanumerics bound a token;
/// a hyphen does not (`Cas9-mediated` is one Stage-A surface).
fn contains_word(sentence: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let hay: Vec<char> = sentence.to_lowercase().chars().collect();
    let needle: Vec<char> = word.to_lowercase().chars().collect();
    let bounded = |c: Option<&char>| c.is_none_or(|c| !c.is_alphanumeric());
    hay.windows(needle.len()).enumerate().any(|(i, w)| {
        w == needle.as_slice()
            && bounded(i.checked_sub(1).and_then(|p| hay.get(p)))
            && bounded(hay.get(i + needle.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::contains_word;

    #[test]
    fn oov_attribution_is_token_bounded() {
        assert!(contains_word("We then evaluated MSI.", "then"));
        assert!(!contains_word("Chromatin can strengthen it.", "then"));
        assert!(contains_word(
            "CRISPR–Cas9-mediated knockout",
            "Cas9-mediated"
        ));
        assert!(contains_word("essential in vitro and in vivo", "VITRO"));
        assert!(!contains_word("nitrovitrogen", "vitro"));
        assert!(!contains_word("anything", ""));
    }
}
