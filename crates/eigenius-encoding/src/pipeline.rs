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
use std::path::{Path, PathBuf};

use crate::emit::DocumentMeta;
use crate::select::load_pins;
use crate::snapshot::{build_sense_ranker, open_head_and_backend, CELL_BEAM, SENSE_CAP};
use clap::Parser as ClapParser;
use eigenius_kernel::dcg::{
    InProcessPipeline, NoAbbreviationProposer, Parser, PinReadingRanker, Proposal, ProposeCtx,
    Proposer, ReadingRanker, ReplayProposer, ReplayReadingRanker,
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
}

/// Write an emitted Eigon-JSON document in the requested format.
///
/// In [`OutputFormat::Esl`] the JSON is printed back as source. A document the printer cannot
/// express is an ERROR, not a fallback to JSON: silently writing a different format under a
/// `.esl` path would be worse than failing.
fn write_doc(path: &Path, body: &[u8]) -> Result<(), String> {
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
        let resources = eigenius_kernel::esl::compile(&src, &head).map_err(|e| {
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
    let kind_classifier: Option<Box<dyn crate::KindClassifier>> = match &args.kinds {
        Some(p) if p.exists() => {
            let r = crate::ReplayKindClassifier::load(p)
                .map_err(|e| format!("read {}: {e}", p.display()))?;
            eprintln!("kinds:      REPLAY {} (deterministic, no LLM)", p.display());
            Some(Box::new(r))
        }
        Some(p) => {
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
        None => None,
    };
    let kind_recorder = kind_classifier.map(crate::RecordingKindClassifier::new);
    // ONE claim identity: the lander names claims exactly as the emitter will, so the
    // `enc:AnaphorBinding` this run records points at resources this run's artifact contains.
    let lander = kind_recorder.as_ref().map(|k| {
        crate::DerivedClaimLander::new(&doc_id, k)
            .with_emission_namespace(&args.ns)
            .with_source(&format!("{} (sha256 {sha})", args.source.display()))
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

    // Emission is the SHARED half (D71 §7 / slice 5a): outcome -> record under the run's selection
    // authority, fail-closed or partial, then the artifact. Identical work for every surface, so it
    // lives in `formalize` and this driver only supplies the inputs.
    let artifact = crate::formalize::emit_from_encoding(&crate::formalize::EmissionInputs {
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
            // rather than hiding it behind the program that parsed it (eigenius#201 / D72).
            // Supplying a real `prov:Agent` is D71's `land` story: the moment a
            // formulation becomes an assertion is the moment someone takes responsibility.
            declared_by: crate::UNATTRIBUTED_AGENT,
            source_ref: args.source_ref.as_deref(),
        },
    })?;
    // The CLI's `--out` decides the encoding; the kernel owns the mapping, so this is the one
    // place the driver expresses a format at all.
    use eigenius_kernel::dcg::formalizer::{render_artifact, ArtifactFormat};
    let rendered = render_artifact(
        &artifact.resources,
        match format {
            OutputFormat::Json => ArtifactFormat::EigonJson,
            OutputFormat::Esl => ArtifactFormat::Esl,
        },
    )?;
    write_doc(&args.out, &rendered)?;
    eprintln!(
        "\nwrote {} ({} encoded, {} cut, {} glossary)",
        args.out.display(),
        artifact.encoded,
        artifact.cut,
        artifact.glossary
    );

    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
