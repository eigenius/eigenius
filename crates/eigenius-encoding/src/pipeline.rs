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

use crate::claims::load_claims;
use crate::emit::{emit_argument, emit_document, emit_inference, emit_shape_rules, ParsedSentence};
use crate::select::{load_pins, select_pinned};
use crate::snapshot::{build_parser, open_head, ParserConfig};
use clap::Parser as ClapParser;
use eigenius_kernel::dcg::{segment_sentences, tokenize};
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
    /// Pinned readings: `sentence <TAB> skeleton <TAB> note`.
    #[arg(long)]
    pins: PathBuf,
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
    /// Declared claim map: `sentence <TAB> predicate <TAB> args <TAB> subject_iri <TAB> declared_by
    /// <TAB> rationale`. Required with `--argument-out`.
    #[arg(long)]
    claims: Option<PathBuf>,
    /// Where to write the argument layer (bridges + reasoning sentences).
    ///
    /// **Generate this ONCE and commit it.** It is the recorded argument, not a function of the
    /// current prose; regenerating it on every run would re-derive the argument around any edit and
    /// nothing would ever fail to commit.
    #[arg(long, requires = "claims")]
    argument_out: Option<PathBuf>,
    /// Where to write the SHAPE-RULE layer — one Declared rule per distinct (predicate, parse
    /// shape), quantified over the argument classes. Use with `--citations-out`.
    ///
    /// A rule serves every sentence of its shape, so the rule count is the authoring cost:
    /// `--argument-out` writes one ground bridge per sentence, this writes one rule per shape.
    #[arg(long, requires = "claims")]
    rules_out: Option<PathBuf>,
    /// Where to write the per-sentence `ReasoningSentence`s that cite the shape rules.
    #[arg(long, requires = "rules_out")]
    citations_out: Option<PathBuf>,
    /// The `reflection:timestamp` on each ProgramTrace. Fixed by the caller so the emission is
    /// byte-reproducible.
    #[arg(long, default_value = "2026-08-03T00:00:00Z")]
    timestamp: String,
    /// Apply a rule already pinned on the chain: `<rule-iri>:<antecedent-ordinal>:<consequent-ordinal>`.
    /// Writes the concluding `ReasoningSentence` to `--inference-out`.
    #[arg(long, requires = "claims")]
    inference: Option<String>,
    /// Where to write the inference layer.
    #[arg(long, requires = "inference")]
    inference_out: Option<PathBuf>,
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

    let pins = load_pins(&args.pins).map_err(|e| format!("read {}: {e}", args.pins.display()))?;
    eprintln!("pins:   {} entries", pins.len());

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

    let mut parsed = Vec::new();
    for (n, text, closed) in &forests {
        let (item, pin) = select_pinned(text, closed, &pins).map_err(|e| e.to_string())?;
        let start = doc.find(text.as_str()).unwrap_or(0);
        parsed.push(ParsedSentence {
            ordinal: *n,
            text: text.clone(),
            span: (start, start + text.len()),
            item,
            candidates: closed.len(),
            pin,
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
        "\nwrote {} ({} sentences × 5 resources)",
        args.out.display(),
        parsed.len()
    );

    if let Some(argument_out) = &args.argument_out {
        let path = args.claims.as_ref().expect("clap `requires` guarantees it");
        let claims = load_claims(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let json = emit_argument(&args.ns, &args.timestamp, &parsed, &claims)
            .map_err(|e| e.to_string())?;
        write_doc(argument_out, &json, format)?;
        eprintln!(
            "wrote {} ({} bridges + {} reasoning sentences) — COMMIT THIS; do not regenerate it \
             per run",
            argument_out.display(),
            parsed.len(),
            parsed.len()
        );
    }
    if let (Some(rules_out), Some(cites_out)) = (&args.rules_out, &args.citations_out) {
        let path = args.claims.as_ref().expect("clap `requires` guarantees it");
        let claims = load_claims(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let (rules, cites) = emit_shape_rules(&args.ns, &args.timestamp, &parsed, &claims)
            .map_err(|e| e.to_string())?;
        write_doc(rules_out, &rules, format)?;
        write_doc(cites_out, &cites, format)?;
        eprintln!(
            "wrote {} and {} — COMMIT BOTH; they are the recorded argument",
            rules_out.display(),
            cites_out.display()
        );
    }
    if let (Some(spec), Some(out)) = (&args.inference, &args.inference_out) {
        let parts: Vec<&str> = spec.rsplitn(3, ':').collect();
        let (conseq, ante, rule) = match parts.as_slice() {
            [c, a, r] => (
                c.parse::<usize>()
                    .map_err(|e| format!("--inference consequent: {e}"))?,
                a.parse::<usize>()
                    .map_err(|e| format!("--inference antecedent: {e}"))?,
                *r,
            ),
            _ => return Err("--inference must be <rule-iri>:<antecedent>:<consequent>".into()),
        };
        let path = args.claims.as_ref().expect("clap `requires`");
        let claims = load_claims(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let json = emit_inference(
            &args.ns,
            &args.timestamp,
            rule,
            ante,
            conseq,
            &parsed,
            &claims,
        )
        .map_err(|e| e.to_string())?;
        write_doc(out, &json, format)?;
        eprintln!("wrote {} (the INFERRED claim)", out.display());
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
