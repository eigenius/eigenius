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
//! Parse a text file over a lexicon snapshot and write the D62 pipeline record — as Eigon-JSON
//! ready for `eigenius load`, or as the ESL source that compiles to it. The two differ only in
//! [`OutputFormat`]; everything upstream of the write is identical, so the two commands cannot
//! drift into encoding different things.
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
//! **Fail closed everywhere.** A sentence that does not parse, whose pin is missing, or whose pinned
//! reading is absent from the forest, aborts the whole emission with a diagnostic — a partial
//! encoding is not a result.

use std::path::{Path, PathBuf};

use crate::emit::{emit_document, ParsedSentence, SentenceSelection};
use crate::select::{load_pins, select_pinned, select_ranked};
use crate::snapshot::{build_parser, open_head, ParserConfig};
use clap::Parser as ClapParser;
use eigenius_kernel::dcg::{
    segment_sentences, tokenize, PriorSelection, ReplayReadingRanker, SelectionOutcome,
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

pub fn run(args: &Args, format: OutputFormat) -> Result<(), String> {
    let doc = std::fs::read_to_string(&args.source)
        .map_err(|e| format!("read {}: {e}", args.source.display()))?;
    let sha = hex(&Sha256::digest(doc.as_bytes()));
    eprintln!("source: {} (sha256 {sha})", args.source.display());

    // Exactly one selection authority per run — an emission with a mixed or defaulted authority
    // would be unauditable.
    enum Authority {
        Pins(std::collections::BTreeMap<String, crate::select::Pin>),
        Replay(ReplayReadingRanker),
    }
    let authority =
        match (&args.pins, &args.selections) {
            (Some(p), None) => {
                let pins = load_pins(p).map_err(|e| format!("read {}: {e}", p.display()))?;
                eprintln!("pins:   {} entries", pins.len());
                Authority::Pins(pins)
            }
            (None, Some(s)) => {
                let ranker = ReplayReadingRanker::load(s)
                    .map_err(|e| format!("read {}: {e}", s.display()))?;
                eprintln!("selections: REPLAY {} (deterministic, no LLM)", s.display());
                Authority::Replay(ranker)
            }
            _ => return Err(
                "exactly one of --pins (declared arm) / --selections (computed arm) is required"
                    .to_string(),
            ),
        };

    let head = open_head(&args.snapshot)?;
    let (parser, recording) = build_parser(
        &head,
        &ParserConfig {
            ranks: args.ranks.clone(),
        },
    )?;
    let lem = MorphyLemmatizer::load(&args.dict)
        .map_err(|e| format!("load Morphy from {}: {e}", args.dict.display()))?;

    // Parse every sentence first: `ParsedSentence` borrows its `Item` out of the forest, so the
    // forests have to outlive the emission.
    let sentences = segment_sentences(&doc);
    let mut forests = Vec::new();
    for (i, text) in sentences.iter().enumerate() {
        let n = i + 1;
        let unknown: Vec<String> = tokenize(text)
            .into_iter()
            .filter(|t| !parser.has_token(t, &lem))
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "sentence {n} «{text}»: out-of-vocabulary tokens {unknown:?} — the demo does not \
                 ground OOV (that is D62 S5a); either extend the lexicon or pick prose the \
                 committed lexicon covers"
            ));
        }
        let (closed, open) = parser.parse_open(text, &lem);
        if closed.is_empty() {
            return Err(format!(
                "sentence {n} «{text}»: no closed parse ({} open, hole-bearing) — a grammar gap or \
                 an unresolved referent, neither of which this demo encodes",
                open.len()
            ));
        }
        eprintln!("  [{n}] {} closed reading(s) — {text}", closed.len());
        forests.push((n, text.clone(), closed));
    }
    // Before selection: a RECORD run must leave its artifact even if a pin then fails to match,
    // because the recording is exactly what a re-run needs to diagnose the mismatch deterministically.
    recording.flush()?;

    // Selection. The ranked arm's per-sentence records are hoisted so `ParsedSentence` can
    // borrow them past the match; the pin arm borrows the pins out of `authority` directly.
    let mut ranked: Vec<(usize, Option<SelectionOutcome>)> = Vec::new();
    let mut parsed = Vec::new();
    match &authority {
        Authority::Pins(pins) => {
            for (n, text, closed) in &forests {
                let (item, pin) = select_pinned(text, closed, pins).map_err(|e| e.to_string())?;
                let start = doc.find(text.as_str()).unwrap_or(0);
                parsed.push(ParsedSentence {
                    ordinal: *n,
                    text: text.clone(),
                    span: (start, start + text.len()),
                    item,
                    candidates: closed.len(),
                    selection: SentenceSelection::Pinned(pin),
                });
            }
        }
        Authority::Replay(ranker) => {
            // Thread the discourse exactly as the recording harness did: prior selections
            // accumulate in SEGMENT order with 0-based ordinals — they are part of the replay
            // KEY, so a divergent gloss or ordinal is a counted miss (→ Abstained, fail-closed),
            // never a silently different question.
            let mut prior: Vec<PriorSelection> = Vec::new();
            for (n, text, closed) in &forests {
                let ordinal = *n - 1;
                if closed.len() == 1 {
                    prior.push(PriorSelection {
                        ordinal,
                        gloss: parser.reading_gloss(text, &lem, &closed[0]),
                    });
                    eprintln!("  [{n}] sole reading — no selection to make");
                    ranked.push((0, None));
                } else {
                    let (idx, sel) =
                        select_ranked(&parser, ranker, &doc, text, &lem, &prior, closed)
                            .map_err(|e| e.to_string())?;
                    eprintln!(
                        "  [{n}] ranker selected 1 of {}: {}",
                        closed.len(),
                        sel.chosen_skeleton
                    );
                    prior.push(PriorSelection {
                        ordinal,
                        gloss: sel.chosen_gloss.clone(),
                    });
                    ranked.push((idx, Some(sel)));
                }
            }
            for ((n, text, closed), (idx, outcome)) in forests.iter().zip(&ranked) {
                let start = doc.find(text.as_str()).unwrap_or(0);
                parsed.push(ParsedSentence {
                    ordinal: *n,
                    text: text.clone(),
                    span: (start, start + text.len()),
                    item: &closed[*idx],
                    candidates: closed.len(),
                    selection: match outcome {
                        Some(sel) => SentenceSelection::Ranked(sel),
                        None => SentenceSelection::Sole,
                    },
                });
            }
        }
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
        "\nwrote {} ({} sentences × 5 resources)",
        args.out.display(),
        parsed.len()
    );

    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
