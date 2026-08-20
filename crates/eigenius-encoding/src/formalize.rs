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

//! **Encoding → artifact**, independent of how the run was driven (D71 §7, slice 5a).
//!
//! Everything downstream of `DocumentPipeline::encode` is the same work whether a CLI, an RPC, a
//! notebook cell or an MCP tool asked for it: map each sentence's [`SentenceOutcome`] to an
//! emission record under the run's selection authority, fail closed (or record a `CutItem` under
//! `partial`), and emit the artifact. Only the INPUTS differ — file paths on one side, request
//! fields on the other.
//!
//! This module is that shared half. It was lifted out of the `prose-to-esl` driver rather than
//! reimplemented for the served path, because the alternative is two emitters that agree until they
//! do not: the CLI's artifacts are the demo's committed, byte-compared fixtures, and a served run
//! that emitted a slightly different shape would be discovered by nothing.

use std::collections::BTreeMap;

use eigenius_kernel::dcg::pipeline::DocumentEncoding;
use eigenius_kernel::dcg::skeleton::skeleton_of;
use eigenius_kernel::dcg::SentenceOutcome;
use eigenius_kernel::ontology::resource::Resource;

use crate::emit::{
    emit_resources, CutReason, CutSentence, DocumentMeta, ParsedSentence, SentenceSelection,
};
use crate::select::Pin;

/// What a run produced: the artifact's RESOURCES and the counts a driver reports.
///
/// Deliberately not a rendered string. The artifact has three encodings (CBOR, Eigon-JSON, ESL) and
/// which one a run wants is the caller's business; baking one in here put the choice in every
/// driver, which is how a served run ends up emitting a shape the committed fixtures never
/// compared against. Render with `eigenius_kernel::dcg::formalizer::render_artifact`.
pub struct Artifact {
    pub resources: Vec<eigenius_kernel::ontology::resource::Resource>,
    pub encoded: usize,
    pub cut: usize,
    pub glossary: usize,
}

/// Everything the emission half needs, as VALUES — no paths, no clap, no snapshot.
pub struct EmissionInputs<'a> {
    /// The source text the units were segmented from — spans are `find`ed in it.
    pub doc: &'a str,
    pub encoding: &'a DocumentEncoding,
    /// The claim clusters the in-loop lander built, keyed by claim IRI. Emitting THESE rather than
    /// rebuilding is not an optimization: a landed claim carries its discourse kind as a second
    /// `is_a`, and a rebuilt cluster has none, so an anaphor resolved to it stops type-checking.
    pub landed: &'a BTreeMap<String, (Resource, Resource)>,
    /// The declared selection arm. `None` means the computed arm chose (or abstained).
    pub pins: Option<&'a BTreeMap<String, Pin>>,
    pub binding_authority: Option<&'a str>,
    /// Record non-encoding units as `CutItem`s instead of aborting (D67 §5).
    pub partial: bool,
    pub meta: DocumentMeta<'a>,
}

/// Map a document's encoding to its artifact.
pub fn emit_from_encoding(inputs: &EmissionInputs<'_>) -> Result<Artifact, String> {
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
        let start = inputs.doc.find(se_text).unwrap_or(0);
        cuts.push(CutSentence {
            ordinal: n,
            text: se_text.to_string(),
            span: (start, start + se_text.len()),
            reason,
        });
    };
    for (i, se) in inputs.encoding.sentences.iter().enumerate() {
        let n = i + 1;
        let text = se.text.trim();
        let item = match &se.outcome {
            SentenceOutcome::Encoded(item) => item,
            SentenceOutcome::Ambiguous(pool) => {
                if inputs.partial {
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
                return Err(match &inputs.pins {
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
                if inputs.partial {
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
                if inputs.partial {
                    // Classify: residual Stage-A OOV surfaces occurring in this sentence make it
                    // a vocabulary cut; none makes it a grammar cut.
                    let oov: Vec<String> = inputs
                        .encoding
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
        let selection = match &inputs.pins {
            Some(pins) => match pins.get(text) {
                None if inputs.partial => SentenceSelection::Sole,
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
        let start = inputs.doc.find(se.text.as_str()).unwrap_or(0);
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
            binding_authority: inputs.binding_authority,
            cluster: inputs
                .landed
                .get(&format!("{}:claim_{n}", inputs.meta.ns))
                .cloned(),
        });
    }

    // Stage-A glossary resources go into the artifact — the entries that grounded the parse
    // (a claim's proposition may reference a doc-glossary-only concept; without them the
    // artifact does not load on a chain that lacks the doc branch).
    let glossary = inputs.encoding.augmentation.resources();
    if !glossary.is_empty() {
        eprintln!("glossary: {} Stage-A resource(s) emitted", glossary.len());
    }
    let resources =
        emit_resources(&inputs.meta, &glossary, &parsed, &cuts).map_err(|e| e.to_string())?;
    Ok(Artifact {
        resources,
        encoded: parsed.len(),
        cut: cuts.len(),
        glossary: glossary.len(),
    })
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

// ════════════════════════════════════════════════════════════════════════════════════════════
// The served formalizer (D71 §7.1, slice 5b)
// ════════════════════════════════════════════════════════════════════════════════════════════

use std::sync::Arc;

use eigenius_kernel::dcg::draw::{commit_draws, draw_resources, draws_from_layer, DrawSeam};
use eigenius_kernel::dcg::formalizer::{
    DocumentFormalizer, DrawSource, FormalizeOutput, FormalizeRequest,
};
use eigenius_kernel::dcg::{
    InProcessPipeline, Lemmatizer, NoAbbreviationProposer, Parser, Proposal, ProposeCtx, Proposer,
    ReadingRanker, ReplayProposer, ReplayReadingRanker, ReplaySenseRanker,
};
use eigenius_kernel::layer::Layer;
use eigenius_kernel::storage::PersistentBackend;

/// The `DocumentFormalizer` the top-level binary injects into the kernel server.
///
/// Holds the lemmatizer, which is the one piece of per-process state a run needs and the kernel
/// cannot construct (`eigenius-wordnet` depends on it). Everything else comes per request.
pub struct EncodingFormalizer {
    lemmatizer: Arc<dyn Lemmatizer + Send + Sync>,
}

impl EncodingFormalizer {
    pub fn new(lemmatizer: Arc<dyn Lemmatizer + Send + Sync>) -> Self {
        Self { lemmatizer }
    }
}

/// A proposer that never proposes — no referent hole resolves.
struct NoProposer;
impl Proposer for NoProposer {
    fn propose(&self, _ctx: &ProposeCtx) -> Proposal {
        Proposal::default()
    }
}

/// The `doc-<id>` branch head, if a previous run left one. `None` on a first run — which is not an
/// error: there are simply no draws to replay yet.
fn doc_branch_head(
    backend: &Arc<dyn PersistentBackend>,
    doc_id: &str,
) -> Result<Option<Arc<Layer>>, String> {
    let Some(id) = backend
        .get_branch(&format!("doc-{doc_id}"))
        .map_err(|e| format!("read branch doc-{doc_id}: {e}"))?
    else {
        return Ok(None);
    };
    let Some(info) = backend
        .load_chain_from(&id)
        .map_err(|e| format!("load doc-{doc_id}: {e}"))?
    else {
        // The ref names a layer the store does not have. A stale branch is not a reason to fail a
        // fresh run — there are simply no draws to replay.
        return Ok(None);
    };
    let storage = eigenius_kernel::layer::LayerStorage::with_persistent(Arc::clone(backend));
    Ok(Some(eigenius_kernel::layer::build_chain(info, storage)))
}

/// One seam's recorded JSON for this run: supplied inline, read off the working branch, or absent.
fn seam_json(
    req: &FormalizeRequest,
    branch: Option<&Arc<Layer>>,
    seam: DrawSeam,
) -> Result<Option<String>, String> {
    match &req.draws {
        DrawSource::Live => Ok(None),
        DrawSource::Inline(map) => Ok(map.get(&seam).cloned()),
        DrawSource::Branch => match branch {
            // An empty draw set reads back as `[]`, which loads as a recording that answers
            // nothing. Treat it as absent so the arm is "no recording" rather than "a recording
            // that misses every question" — the two behave differently at the fail-closed check.
            Some(l) => {
                let json = draws_from_layer(l, seam)?;
                Ok(if json == "[]" { None } else { Some(json) })
            }
            None => Ok(None),
        },
    }
}

impl DocumentFormalizer for EncodingFormalizer {
    fn formalize(
        &self,
        base: Arc<Layer>,
        backend: Arc<dyn PersistentBackend>,
        req: &FormalizeRequest,
    ) -> Result<FormalizeOutput, String> {
        use sha2::{Digest, Sha256};
        let sha = hex(&Sha256::digest(req.source_text.as_bytes()));

        // Draws are read from the branch a PREVIOUS run left (or supplied inline). On a first run
        // there are none and every seam asks live — which needs a key, and fails closed without one.
        let branch = doc_branch_head(&backend, &req.doc_id)?;

        // ── the four arms ───────────────────────────────────────────────────────────────────
        let rank_json = seam_json(req, branch.as_ref(), DrawSeam::SenseRank)?;
        let sel_json = seam_json(req, branch.as_ref(), DrawSeam::ReadingSelection)?;
        let prop_json = seam_json(req, branch.as_ref(), DrawSeam::Anaphora)?;
        let kind_json = seam_json(req, branch.as_ref(), DrawSeam::DiscourseKind)?;

        let mut arms = Arms::build(
            rank_json.as_deref(),
            sel_json.as_deref(),
            prop_json.as_deref(),
            kind_json.as_deref(),
            req,
        )?;
        arms.source_label = format!("{} (sha256 {sha})", req.source_path);
        // Order matters: `lander` borrows `arms.kinds` for the whole run, so everything that is
        // MOVED out of `arms` has to come out before that borrow starts.
        let proposer = arms.proposer.take().unwrap_or_else(|| Box::new(NoProposer));
        let sense_arm = arms.sense.take();
        let lander = arms.lander(req);

        // ── the run ─────────────────────────────────────────────────────────────────────────
        let ranker_slot = std::cell::RefCell::new(sense_arm);
        let sense_cap = req.sense_cap;
        let cell_beam = req.cell_beam;
        let setup = move |p: Parser| {
            let mut p = p;
            if let Some(n) = sense_cap {
                p = p.with_sense_cap(n);
            }
            if let Some(m) = cell_beam {
                p = p.with_cell_beam(m);
            }
            match ranker_slot.borrow_mut().take() {
                Some(r) => p.with_sense_ranker(r),
                None => p,
            }
        };

        let mut pipeline =
            InProcessPipeline::new(base, &*self.lemmatizer, &NoAbbreviationProposer, &*proposer)
                .with_parser_setup(&setup)
                .with_storage(Arc::clone(&backend), &req.doc_id);
        if let Some(s) = &req.scope {
            pipeline = pipeline.with_scope(s.clone());
        }
        if let Some(r) = &arms.selection {
            pipeline = pipeline.with_reading_ranker(&**r);
        }
        if let Some(l) = &lander {
            pipeline = pipeline.with_claim_lander(l);
        }
        let (encoding, doc_layer) = pipeline
            .encode_with_layer(&req.source_text)
            .map_err(|e| e.to_string())?;

        let landed = match &lander {
            Some(l) => l
                .take_landed()
                .into_iter()
                .filter_map(|c| {
                    let mut it = c.resources.into_iter();
                    let claim = it.next()?;
                    let trace = it.next()?;
                    Some((claim.id()?.as_str().to_string(), (claim, trace)))
                })
                .collect(),
            None => BTreeMap::new(),
        };

        // ── the artifact — the SHARED emitter (slice 5a) ────────────────────────────────────
        let artifact = emit_from_encoding(&EmissionInputs {
            doc: &req.source_text,
            encoding: &encoding,
            landed: &landed,
            // The served path has no declared (pin) arm: a pin is a human's skeleton, and the
            // service's selection authority is the ranker or a sole survivor.
            pins: None,
            binding_authority: Some("binding_replay"),
            partial: !req.strict,
            meta: DocumentMeta {
                ns: &req.ns,
                source_path: &req.source_path,
                source_sha256: &sha,
                timestamp: &req.timestamp,
                source_ref: req.source_ref.as_deref(),
            },
        })?;

        // ── the transcript, onto the working branch (D71 §9.1) ──────────────────────────────
        let mut draw_resources_all = Vec::new();
        for (seam, keyed) in arms.keyed_draws()? {
            if keyed.is_empty() {
                continue;
            }
            draw_resources_all.extend(draw_resources(
                &req.ns,
                seam,
                &keyed,
                Some(&req.model.model),
                &req.timestamp,
            )?);
        }
        let draws_committed = commit_draws(&backend, &req.doc_id, doc_layer, draw_resources_all)?;

        Ok(FormalizeOutput {
            artifact: eigenius_kernel::dcg::formalizer::render_artifact(
                &artifact.resources,
                req.format,
            )?,
            content_type: req.format,
            structure_iri: format!("{}:structure", req.ns),
            encoded: artifact.encoded,
            cut: artifact.cut,
            draws_committed,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ── the four arms, as VALUES ────────────────────────────────────────────────────────────────
//
// Mirrors the CLI's discipline, with one thing the served path makes explicit: a recorded draw
// REPLAYS BARE, with no recorder wrapped around it. The CLI does the same (`Recording::None` on the
// replay path), and here it also means a replayed run commits nothing back to its branch — which is
// right, since draw IRIs are content-addressed and re-recording an identical answer is a no-op that
// would only churn the branch.
//
// The live arms are `use-llm`-only. Without the feature a missing recording is an ERROR, never a
// silent degradation: a run with no sense ranker is cap-only, which is a different experiment, and
// a run with no anaphora proposer resolves no referent hole at all.
struct Arms {
    sense: Option<Box<dyn eigenius_kernel::dcg::SenseRanker + Send + Sync>>,
    selection: Option<Box<dyn ReadingRanker>>,
    proposer: Option<Box<dyn Proposer>>,
    kinds: Option<Box<dyn eigenius_reasoning::KindClassifier>>,
    /// Live recorders, held so their draws can be harvested after the run.
    #[cfg(feature = "use-llm")]
    rec: LiveRecorders,
    /// What each trace names as the derivation's source — path + sha, never the branch id.
    source_label: String,
}

#[cfg(feature = "use-llm")]
#[derive(Default)]
struct LiveRecorders {
    sense: Option<Arc<RecordingSenseRanker<eigenius_kernel::dcg::AnthropicSenseRanker>>>,
    selection: Option<Arc<RecordingReadingRanker<eigenius_kernel::dcg::AnthropicReadingRanker>>>,
    proposer: Option<Arc<RecordingProposer<eigenius_kernel::dcg::resolver_llm::AnthropicProposer>>>,
    kinds: Option<
        Arc<
            eigenius_reasoning::RecordingKindClassifier<
                eigenius_reasoning::AnthropicKindClassifier,
            >,
        >,
    >,
}

impl Arms {
    fn build(
        rank: Option<&str>,
        selection: Option<&str>,
        anaphora: Option<&str>,
        kinds: Option<&str>,
        req: &FormalizeRequest,
    ) -> Result<Self, String> {
        let _ = (&req.model, &req.source_text);
        #[cfg(feature = "use-llm")]
        let mut rec = LiveRecorders::default();

        let sense: Option<Box<dyn eigenius_kernel::dcg::SenseRanker + Send + Sync>> = match rank {
            Some(j) => Some(Box::new(
                ReplaySenseRanker::from_json(j).map_err(|e| format!("sense-rank draw: {e}"))?,
            )),
            None => {
                #[cfg(feature = "use-llm")]
                {
                    let live = eigenius_kernel::dcg::AnthropicSenseRanker::from_env_with(
                        req.model.clone(),
                    )
                    .ok_or("no sense-rank recording and ANTHROPIC_API_KEY is unset")?;
                    let a = Arc::new(RecordingSenseRanker::new(live));
                    rec.sense = Some(Arc::clone(&a));
                    Some(Box::new(ArcSense(a)))
                }
                #[cfg(not(feature = "use-llm"))]
                return Err(
                    "no sense-rank recording, and this binary has no live ranker (built \
                            without --features use-llm) — a cap-only run is a DIFFERENT experiment"
                        .to_string(),
                );
            }
        };

        let selection: Option<Box<dyn ReadingRanker>> = match selection {
            Some(j) => Some(Box::new(
                ReplayReadingRanker::from_json(j).map_err(|e| format!("selection draw: {e}"))?,
            )),
            None => {
                #[cfg(feature = "use-llm")]
                {
                    let live = eigenius_kernel::dcg::AnthropicReadingRanker::from_env_with(
                        req.model.clone(),
                    )
                    .ok_or("no selection recording and ANTHROPIC_API_KEY is unset")?;
                    let a = Arc::new(RecordingReadingRanker::new(live));
                    rec.selection = Some(Arc::clone(&a));
                    Some(Box::new(ArcSelection(a)))
                }
                #[cfg(not(feature = "use-llm"))]
                None // no ranker: ambiguous units stay ambiguous and are recorded as cuts
            }
        };

        let proposer: Option<Box<dyn Proposer>> = match anaphora {
            Some(j) => Some(Box::new(
                ReplayProposer::from_json(j).map_err(|e| format!("anaphora draw: {e}"))?,
            )),
            None => {
                #[cfg(feature = "use-llm")]
                {
                    let live =
                        eigenius_kernel::dcg::resolver_llm::AnthropicProposer::from_env_with(
                            req.model.clone(),
                        )
                        .ok_or("no anaphora recording and ANTHROPIC_API_KEY is unset")?;
                    let a = Arc::new(RecordingProposer::new(live));
                    rec.proposer = Some(Arc::clone(&a));
                    Some(Box::new(ArcProposer(a)))
                }
                #[cfg(not(feature = "use-llm"))]
                None // no proposer: open parses stay open and are recorded as cuts
            }
        };

        let kinds: Option<Box<dyn eigenius_reasoning::KindClassifier>> = match kinds {
            Some(j) => Some(Box::new(
                eigenius_reasoning::ReplayKindClassifier::from_json(j)
                    .map_err(|e| format!("kind draw: {e}"))?,
            )),
            None => {
                #[cfg(feature = "use-llm")]
                {
                    match eigenius_reasoning::AnthropicKindClassifier::from_env_with(
                        &req.source_text,
                        req.model.clone(),
                    ) {
                        Some(live) => {
                            let a =
                                Arc::new(eigenius_reasoning::RecordingKindClassifier::new(live));
                            rec.kinds = Some(Arc::clone(&a));
                            Some(Box::new(ArcKinds(a))
                                as Box<dyn eigenius_reasoning::KindClassifier>)
                        }
                        None => None,
                    }
                }
                #[cfg(not(feature = "use-llm"))]
                None // no classifier: no claim lands in-loop, so no claim is an antecedent
            }
        };

        Ok(Self {
            sense,
            selection,
            proposer,
            kinds,
            #[cfg(feature = "use-llm")]
            rec,
            source_label: String::new(),
        })
    }

    /// The in-loop claim lander, when a kind arm exists (D68): a landed claim carries its discourse
    /// kind, which is what lets a later demonstrative bind to it.
    fn lander(&self, req: &FormalizeRequest) -> Option<eigenius_reasoning::DerivedClaimLander<'_>> {
        self.kinds.as_ref().map(|k| {
            eigenius_reasoning::DerivedClaimLander::new(&req.doc_id, &**k)
                .with_emission_namespace(&req.ns)
                .with_source(&self.source_label)
        })
    }

    /// Every live arm's recorded exchanges, by seam. Replayed arms contribute nothing.
    fn keyed_draws(
        &self,
    ) -> Result<Vec<(DrawSeam, Vec<eigenius_kernel::dcg::draw::KeyedDraw>)>, String> {
        #[cfg(not(feature = "use-llm"))]
        {
            Ok(Vec::new())
        }
        #[cfg(feature = "use-llm")]
        {
            let mut out = Vec::new();
            if let Some(a) = &self.rec.sense {
                out.push((
                    DrawSeam::SenseRank,
                    a.keyed_draws().map_err(|e| e.to_string())?,
                ));
            }
            if let Some(a) = &self.rec.selection {
                out.push((
                    DrawSeam::ReadingSelection,
                    a.keyed_draws().map_err(|e| e.to_string())?,
                ));
            }
            if let Some(a) = &self.rec.proposer {
                out.push((
                    DrawSeam::Anaphora,
                    a.keyed_draws().map_err(|e| e.to_string())?,
                ));
            }
            if let Some(a) = &self.rec.kinds {
                out.push((
                    DrawSeam::DiscourseKind,
                    a.keyed_draws().map_err(|e| e.to_string())?,
                ));
            }
            Ok(out)
        }
    }
}

// ── shared-handle newtypes ──────────────────────────────────────────────────────────────────
//
// A live arm is installed on the pipeline AND harvested for its draws after the run, so the
// recorder needs two owners. The traits take `Box<dyn _>`, and none of them has a blanket impl over
// `Arc`, so each seam gets a one-line newtype that forwards. (`snapshot.rs` already does this for
// the sense ranker under the CLI's file-based arm; same shape, four seams.)
#[cfg(feature = "use-llm")]
use eigenius_kernel::dcg::{RecordingProposer, RecordingReadingRanker, RecordingSenseRanker};

#[cfg(feature = "use-llm")]
mod arc_handles {
    use super::*;

    pub(super) struct ArcSense(
        pub Arc<RecordingSenseRanker<eigenius_kernel::dcg::AnthropicSenseRanker>>,
    );
    impl eigenius_kernel::dcg::SenseRanker for ArcSense {
        fn rank(
            &self,
            sentence: &str,
            context: &str,
            words: &[eigenius_kernel::dcg::WordSenses],
        ) -> Option<Vec<Vec<usize>>> {
            self.0.rank(sentence, context, words)
        }
    }

    pub(super) struct ArcSelection(
        pub Arc<RecordingReadingRanker<eigenius_kernel::dcg::AnthropicReadingRanker>>,
    );
    impl ReadingRanker for ArcSelection {
        fn select(
            &self,
            ctx: &eigenius_kernel::dcg::DocumentContext,
            cands: &[eigenius_kernel::dcg::ReadingCandidate],
        ) -> Option<eigenius_kernel::dcg::ReadingSelection> {
            self.0.select(ctx, cands)
        }
    }

    pub(super) struct ArcProposer(
        pub Arc<RecordingProposer<eigenius_kernel::dcg::resolver_llm::AnthropicProposer>>,
    );
    impl Proposer for ArcProposer {
        fn propose(&self, ctx: &ProposeCtx) -> Proposal {
            self.0.propose(ctx)
        }
    }

    pub(super) struct ArcKinds(
        pub  Arc<
            eigenius_reasoning::RecordingKindClassifier<
                eigenius_reasoning::AnthropicKindClassifier,
            >,
        >,
    );
    impl eigenius_reasoning::KindClassifier for ArcKinds {
        fn classify(
            &self,
            ordinal: usize,
            sentence: &str,
            gloss: &str,
        ) -> eigenius_reasoning::KindVerdict {
            self.0.classify(ordinal, sentence, gloss)
        }
    }
}
#[cfg(feature = "use-llm")]
use arc_handles::{ArcKinds, ArcProposer, ArcSelection, ArcSense};

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
