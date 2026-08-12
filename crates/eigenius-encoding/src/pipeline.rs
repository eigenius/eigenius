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
//! ```bash
//! prose-to-eigon --snapshot ../db-snapshot/wordnet-umls-aligned-2026-08-02-consolidated \
//!                --source demo/prose-to-chain/paragraph.txt \
//!                --pins   demo/prose-to-chain/pins.tsv \
//!                --ranks  demo/prose-to-chain/ranks.json \
//!                --ns     urn:eigenius:demo:prose \
//!                --out    /tmp/03-parsed.json
//! ```
//!
//! **Fail closed everywhere.** A sentence that does not encode — a gap, an unresolved referent
//! hole, a pin that matches zero or several pooled readings, a selection-replay abstention —
//! aborts the whole emission with a diagnostic: a partial encoding is not a result.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::emit::{emit_document, ParsedSentence, SentenceSelection};
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
    /// A recorded reading-selection draw (`selections.json`) to REPLAY — the COMPUTED selection
    /// arm (d63-reading-selection.md). Replay-only here, deliberately: artifact generation stays
    /// deterministic; record a draw with `scripts/measure-parse-rate.sh --selections <new-file>`.
    /// Exactly one of `--pins` / `--selections` is required.
    #[arg(long)]
    selections: Option<PathBuf>,
    /// A recorded `ranks.json` to replay — deterministic, no LLM. Omit for cap-only (a DIFFERENT
    /// experiment: sense elimination is off, so the pins may not match).
    #[arg(long)]
    ranks: Option<PathBuf>,
    /// A recorded anaphora-proposal draw (`proposals.json`) to REPLAY (D67 §3) — resolves
    /// referent holes through the discourse loop, deterministically. Replay-only here; record a
    /// draw with the close-out harness's `EIGENIUS_PROPOSALS` arm. Omit ⇒ no anaphora resolves
    /// (open parses stay Open — and abort the emission, which is fail-closed).
    #[arg(long)]
    proposals: Option<PathBuf>,
    /// IRI prefix for the emitted resources.
    #[arg(long, default_value = "urn:eigenius:demo:prose")]
    ns: String,
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
            _ => return Err(
                "exactly one of --pins (declared arm) / --selections (computed arm) is required"
                    .to_string(),
            ),
        };
    let ranker: Box<dyn ReadingRanker> = match (&pins, &args.selections) {
        (Some(pins), _) => Box::new(PinReadingRanker::new(
            pins.iter()
                .map(|(s, p)| (s.clone(), p.skeleton.clone()))
                .collect(),
        )),
        (None, Some(s)) => {
            let r =
                ReplayReadingRanker::load(s).map_err(|e| format!("read {}: {e}", s.display()))?;
            eprintln!("selections: REPLAY {} (deterministic, no LLM)", s.display());
            Box::new(r)
        }
        _ => unreachable!("validated above"),
    };

    // Anaphora arm (D67 §3): replay-only. Without it no referent hole resolves.
    let proposer: Box<dyn Proposer> = match &args.proposals {
        Some(p) => {
            let r = ReplayProposer::load(p).map_err(|e| format!("read {}: {e}", p.display()))?;
            eprintln!("proposals:  REPLAY {} (deterministic, no LLM)", p.display());
            Box::new(r)
        }
        None => Box::new(NoProposer),
    };
    let binding_authority = args.proposals.as_ref().map(|_| "binding_replay");

    let (head, backend) = open_head_and_backend(&args.snapshot)?;
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

    let pipeline = InProcessPipeline::new(head, &lem, &NoAbbreviationProposer, &proposer)
        .with_reading_ranker(&ranker)
        .with_parser_setup(&setup)
        .with_storage(backend, &doc_id);
    let (encoding, _doc_layer) = pipeline
        .encode_with_layer(&doc)
        .map_err(|e| format!("{e}"))?;
    // A RECORD run leaves its ranks artifact even if a pin then fails to match below — the
    // recording is exactly what a re-run needs to diagnose the mismatch deterministically.
    recording.flush()?;

    // Map each sentence's outcome to the emission record — fail-closed on anything that did not
    // encode under the chosen authority.
    let mut parsed: Vec<ParsedSentence> = Vec::new();
    for (i, se) in encoding.sentences.iter().enumerate() {
        let n = i + 1;
        let text = se.text.trim();
        let item = match &se.outcome {
            SentenceOutcome::Encoded(item) => item,
            SentenceOutcome::Ambiguous(pool) => {
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
                return Err(format!(
                    "sentence {n} «{text}»: {} unresolved referent hole(s) — provide --proposals \
                     with a recorded draw that resolves them, or pick prose without anaphora",
                    o.holes.len()
                ));
            }
            SentenceOutcome::Gap => {
                return Err(format!(
                    "sentence {n} «{text}»: no parse — a grammar gap or out-of-vocabulary tokens"
                ));
            }
        };
        // The emission's selection record. Under pins, verify the encoded reading IS the pinned
        // one even when it was the sole survivor (the ranker only fires on pools > 1).
        let selection = match &pins {
            Some(pins) => {
                let pin = pins
                    .get(text)
                    .ok_or_else(|| format!("sentence {n} «{text}»: no pin"))?;
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
        });
    }

    let json = emit_document(
        &args.ns,
        &args.source.display().to_string(),
        &sha,
        &args.timestamp,
        &parsed,
    )
    .map_err(|e| e.to_string())?;
    write_doc(&args.out, &json, format)?;
    eprintln!(
        "\nwrote {} ({} sentences)",
        args.out.display(),
        parsed.len()
    );

    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
