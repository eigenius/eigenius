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

//! Emit the D62 pipeline record for a parsed document as Eigon-JSON.
//!
//! The vocabulary is the **committed** D62 ontology (`ontologies/encoding/encoding.esl`) — nothing
//! demo-specific. Per sentence:
//!
//! ```text
//!   enc:DiscourseUnit    prose + character span in the source
//!   enc:ScopedUnit       (thin — unscoped; the whole chain is the scope)
//!   enc:EncodedClaim     reflection:canonical_proposition = the parsed Prop, D47-encoded
//!   prov:DeclarationTrace ─▶ IsDeclaredAs claim_iri P   ← the witness downstream cites
//!   enc:DecisionPoint    which reading was taken, out of how many, and on whose authority
//! ```
//!
//! and once per document:
//!
//! ```text
//!   reference:Reference       the source work every unit hangs off (minted, or cited by IRI)
//!   enc:ReasoningStructure    the artifact ROOT — the claims, the source, the bytes parsed
//!   prov:ProgramTrace                            ← the RUN, recorded once (grounds nothing)
//! ```
//!
//! The root exists so the artifact has a HANDLE: a service returns it, a notebook cell re-opens it,
//! and a later run has something to supersede (D71 §4.1). Without it the artifact is a bag of
//! resources whose only membership test is "was in the same file".
//!
//! TWO OBJECTS, TWO CATEGORIES (eigenius#201). Encoding a document produces two kinds of thing:
//!
//!   - THE PROCESS's output — `enc:ReasoningStructure`, a function of (engine, bytes) -> structure.
//!     A program run, so **Derived**, witnessed by ONE `ProgramTrace` for the run.
//!   - THE PROPOSITIONS — `enc:EncodedClaim`, **Declared** by a named agent. The parser fixes their
//!     FORM; it does not assert their content.
//!
//! The old shape had this inverted where it counted: no trace on the structure, and N
//! `ProgramTrace`s on the CLAIMS — one per sentence — each minting `IsDerivedAs(claim, P)` for a
//! `P` about the world. Wrong CARDINALITY (one run is one execution) and wrong PROPOSITION (the run
//! establishes what came out of the engine, never that a claim is true).
//!
//! A certificate citing `declared(claim_iri, P)` still breaks the moment the prose changes and the
//! parser produces a different `P` — the witness key hashes the proposition — which is the whole
//! point and is unaffected by the grade.

use crate::ParsedClaimGrader;
use eigenius_kernel::dcg::item::Item;
use eigenius_kernel::dcg::skeleton::skeleton_of;
use eigenius_kernel::dcg::{Candidate, ResolvedBinding, SelectionOutcome};
use eigenius_kernel::ontology::eigon_json::serialize_document;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::program::eigentt_type_mirror::encode_type;

use crate::select::Pin;

const CORE: &str = "urn:eigenius:core";
/// The provenance axis. Split out of `reflection`; see `ontologies/prov/prov.esl`.
const PROV: &str = "urn:eigenius:prov";
const ENC: &str = "urn:eigenius:encoding";
const REF: &str = "urn:eigenius:reference";

/// On whose authority a sentence's reading was taken — the emitted `DecisionPoint` records it.
pub enum SentenceSelection<'a> {
    /// A human-verified skeleton pin (the declared gate arm). Emits exactly the historical
    /// record shape — the committed demo artifacts regenerate byte-identically under this arm.
    Pinned(&'a Pin),
    /// The reading ranker's choice (live or replayed) — the computed-choice record
    /// (`d63-reading-selection.md` slice 5): authority individual, the ranker's rationale
    /// verbatim, and the runner-up skeletons it ranked the choice against.
    Ranked(&'a SelectionOutcome),
    /// The forest offered a single reading — no choice existed to make.
    Sole,
}

/// Document-level inputs to [`emit_document`] — what is being encoded, from which bytes, under
/// which IRI prefix.
pub struct DocumentMeta<'a> {
    /// IRI prefix the emitted resources live under (e.g. `urn:eigenius:demo:prose`).
    pub ns: &'a str,
    /// Where the parsed bytes came from — recorded once, on the root.
    pub source_path: &'a str,
    /// SHA-256 of the parsed bytes. A prose edit is then visible on the chain, not only in the
    /// propositions it changed.
    pub source_sha256: &'a str,
    /// The `prov:timestamp` on each DeclarationTrace. Caller-fixed so emission is reproducible.
    pub timestamp: &'a str,
    /// `prov:was_attributed_to` — the agent taking responsibility for every claim this document
    /// lands. REQUIRED since eigenius#201 made `enc:EncodedClaim` a `reflection:DeclaredResource`:
    /// a parse establishes form, not warrant, so a landed claim must name who asserts it (D73 §6).
    ///
    /// Must be the IRI of a resolvable `prov:Agent` (D72). For an encoded paper that is the
    /// paper's authors; for an agent formulating its own claims it is that agent.
    /// [`UNATTRIBUTED_AGENT`] is the honest value when the caller knows of no agent — it names the
    /// absence rather than hiding it behind the program that did the parsing.
    pub declared_by: &'a str,
    /// The `reference:Reference` for the source work.
    ///
    /// `None` MINTS a document-local one at `<ns>:source` and emits it into the artifact — the
    /// honest record for a plain text file with no bibliographic identity. `Some(iri)` cites an
    /// existing Reference and emits nothing, so Rule 22's closed-world check does the verifying:
    /// an IRI that names no chain-resident Reference fails the load rather than conjuring one.
    pub source_ref: Option<&'a str>,
}

impl DocumentMeta<'_> {
    /// The Reference IRI every `enc:source_document` points at — minted or cited.
    fn reference_iri(&self) -> String {
        match self.source_ref {
            Some(r) => r.to_string(),
            None => format!("{}:source", self.ns),
        }
    }
}

/// One sentence that parsed and whose reading was selected.
pub struct ParsedSentence<'a> {
    /// 1-based position in the document — the local-name key for every resource emitted for it.
    pub ordinal: usize,
    pub text: String,
    /// Character offsets of `text` in the source file.
    pub span: (usize, usize),
    /// The selected reading.
    pub item: &'a Item,
    /// How many closed readings the forest offered (`1` = the unit encoded on its own).
    pub candidates: usize,
    /// How the reading was selected — recorded on the chain via the `DecisionPoint`.
    pub selection: SentenceSelection<'a>,
    /// The accepted anaphora bindings when the reading is a RESOLVED open parse (D67 §3) —
    /// emitted as one `enc:AnaphorBinding` per hole. Empty for closed readings, so the pin-arm
    /// artifacts regenerate byte-identically.
    pub bindings: Vec<ResolvedBinding>,
    /// The `enc:BindingAuthority` local name (`binding_recency` / `binding_proposer` /
    /// `binding_replay`) behind [`Self::bindings`]. `None` omits `enc:bound_by`.
    pub binding_authority: Option<&'a str>,
    /// The claim cluster the CLAIM LANDER built for this sentence in-loop, when one is installed
    /// (`claim`, `trace`).
    ///
    /// Emitting the lander's resources rather than rebuilding them is not an optimization: the
    /// landed claim carries its DISCOURSE KIND as a second `is_a` class, and a later sentence's
    /// proposition may USE that claim as a term (an anaphor resolved to it). A rebuilt cluster
    /// has no kind, so the claim no longer inhabits the lexicon class the kind aligns to, and
    /// the artifact fails to load — `TermIllTyped: … does not inhabit lexicon:Entity`,
    /// witnessed 2026-08-12. One claim, one resource.
    pub cluster: Option<(Resource, Resource)>,
}

/// Why a sentence did not encode — mapped to the committed `enc:CutKind` individuals. The
/// artifact records every unit; a non-encoding is stated, never dropped (D67 §5).
pub enum CutReason {
    /// Multiple readings survived and the run's selection authority did not choose.
    Ambiguous { readings: usize },
    /// Every surviving reading carries a referent hole no antecedent resolved.
    Unresolved { holes: usize },
    /// No parse. `oov` lists the sentence's residual out-of-vocabulary surfaces (from the
    /// Stage-A augmentation) — non-empty classifies the cut as a vocabulary gap, empty as a
    /// grammar gap.
    NoParse { oov: Vec<String> },
}

/// One sentence that did NOT encode — emitted as its `enc:DiscourseUnit` plus an `enc:CutItem`
/// carrying the reason (fail-closed provenance; D62 §3).
pub struct CutSentence {
    /// 1-based position in the document — shares the numbering space with [`ParsedSentence`].
    pub ordinal: usize,
    pub text: String,
    /// Character offsets of `text` in the source file.
    pub span: (usize, usize),
    pub reason: CutReason,
}

#[derive(Debug)]
pub enum EmitError {
    /// The parsed `Prop` is outside the D47 chain-mirrored type fragment.
    Encode { ordinal: usize, detail: String },
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode { ordinal, detail } => write!(
                f,
                "sentence {ordinal}: the parsed proposition is not expressible in the D47 \
                 chain-mirrored type fragment — {detail}"
            ),
        }
    }
}

/// Build the Eigon-JSON document for a parsed source file.
///
/// `ns` is the IRI prefix the emitted resources live under (e.g. `urn:eigenius:demo:prose`);
/// `source_sha256` and `source_path` pin *which bytes* were parsed, so a prose edit is visible on
/// the chain and not only in the propositions. `glossary` is the Stage-A lexicon augmentation
/// (`LexiconAugmentation::resources()`) — the entries that grounded the parse belong in the
/// artifact, which is otherwise not self-contained (a claim's proposition may reference a
/// doc-glossary-only concept). `cuts` are the sentences that did not encode, each landed as its
/// `DiscourseUnit` + an `enc:CutItem` — the artifact states what did not encode; it never
/// silently drops a unit (D67 §5).
pub fn emit_document(
    meta: &DocumentMeta<'_>,
    glossary: &[Resource],
    sentences: &[ParsedSentence<'_>],
    cuts: &[CutSentence],
) -> Result<String, EmitError> {
    Ok(
        serde_json::to_string_pretty(&serialize_document(&emit_resources(
            meta, glossary, sentences, cuts,
        )?))
        .expect("serialize Eigon-JSON"),
    )
}

/// The artifact as RESOURCES, before any encoding is chosen.
///
/// [`emit_document`] is the Eigon-JSON rendering of this; the served path renders whichever format
/// the request asked for (`render_artifact`). One builder, three encodings — the alternative is a
/// format decision living in every caller, which is how a served run ends up emitting a shape the
/// committed fixtures never compared against.
pub fn emit_resources(
    meta: &DocumentMeta<'_>,
    glossary: &[Resource],
    sentences: &[ParsedSentence<'_>],
    cuts: &[CutSentence],
) -> Result<Vec<Resource>, EmitError> {
    let DocumentMeta {
        ns,
        source_path,
        source_sha256,
        timestamp,
        declared_by,
        ..
    } = *meta;
    let doc_iri = meta.reference_iri();

    let mut out: Vec<Resource> = Vec::new();
    // A minted Reference leads the artifact; a CITED one is already on the chain and must not be
    // re-emitted here — a second definition of an existing IRI is a redefinition, not a reference.
    if meta.source_ref.is_none() {
        let mut r = res(&doc_iri, &[&format!("{REF}:Reference")]);
        r.set(
            iri(&format!("{CORE}:description")),
            Value::String(format!(
                "The source this encoding was derived from: {source_path} \
                 (sha256 {source_sha256}). Minted document-locally — the file carries no DOI or \
                 PMID; pass an existing reference:Reference IRI to cite a bibliographic record \
                 instead."
            )),
        );
        out.push(r);
    }
    out.extend_from_slice(glossary);

    let mut claim_iris: Vec<Value> = Vec::new();
    for s in sentences {
        let n = s.ordinal;
        let unit_iri = format!("{ns}:unit_{n}");
        let scoped_iri = format!("{ns}:scoped_{n}");
        let claim_iri = format!("{ns}:claim_{n}");

        out.push(discourse_unit(ns, n, &s.text, s.span, &doc_iri));

        let mut scoped = res(&scoped_iri, &[&format!("{ENC}:ScopedUnit")]);
        scoped.set(
            iri(&format!("{ENC}:unit")),
            Value::String(iri(&unit_iri).as_str().to_string()),
        );
        out.push(scoped);

        // The parsed claim cluster — claim + DeclarationTrace — comes from the ONE construction
        // (`ParsedClaimGrader::cluster`, D73 §6); this emitter keeps its historical
        // `claim_{n}` / `trace_{n}` naming (committed artifacts regenerate byte-identically)
        // and adds only the document-structural fields the grader does not know about.
        //
        // No per-claim engine provenance: one parse run is one program execution, recorded once on
        // the root's `ProgramTrace` below. The per-claim SPAN it used to carry alongside the engine
        // line is already on this unit's `enc:DiscourseUnit` (`enc:span_start` / `enc:span_end`).
        let (mut claim, trace) = match &s.cluster {
            Some((claim, trace)) => (claim.clone(), trace.clone()),
            None => ParsedClaimGrader::cluster(
                &claim_iri,
                &format!("{ns}:trace_{n}"),
                s.item.sem(),
                declared_by,
                timestamp,
                &[],
            )
            .map_err(|e| EmitError::Encode {
                ordinal: n,
                detail: e.to_string(),
            })?,
        };
        claim.set(
            iri(&format!("{ENC}:from_unit")),
            Value::String(iri(&scoped_iri).as_str().to_string()),
        );
        let claim_desc = match &s.selection {
            SentenceSelection::Pinned(pin) => format!(
                "«{}» — the reading pinned as correct: {}",
                s.text, pin.skeleton
            ),
            SentenceSelection::Ranked(sel) => format!(
                "«{}» — the reading the ranker selected: {}",
                s.text, sel.chosen_skeleton
            ),
            SentenceSelection::Sole => format!(
                "«{}» — the sole surviving reading: {}",
                s.text,
                skeleton_of(s.item.sem())
            ),
        };
        claim.set(
            iri(&format!("{CORE}:description")),
            Value::String(claim_desc),
        );
        claim_iris.push(Value::String(iri(&claim_iri).as_str().to_string()));
        out.push(claim);
        out.push(trace);

        // Selection is recorded even when the unit was unambiguous, so the chain always says on
        // whose authority the reading was taken. The PIN arm emits the exact historical shape
        // (no `selected_by`; its authority is stated in the rationale text) so the committed demo
        // artifacts regenerate byte-identically; the ranker and sole arms carry the
        // `SelectionAuthority` individual, and the ranker arm additionally the runner-up
        // skeletons — the choice's audit trail (there is no kernel veto on selection).
        let mut dp = res(
            &format!("{ns}:decision_{n}"),
            &[&format!("{ENC}:DecisionPoint")],
        );
        dp.set(
            iri(&format!("{ENC}:decision_unit")),
            Value::String(iri(&scoped_iri).as_str().to_string()),
        );
        dp.set(
            iri(&format!("{ENC}:selected_claim")),
            Value::String(iri(&claim_iri).as_str().to_string()),
        );
        dp.set(
            iri(&format!("{ENC}:candidate_count")),
            Value::Integer(s.candidates as i64),
        );
        match &s.selection {
            SentenceSelection::Pinned(pin) => {
                dp.set(
                    iri(&format!("{PROV}:rationale")),
                    Value::String(format!(
                        "Reading selected by SKELETON PIN, not by the pipeline: the one reading whose \
                         sense-erased skeleton equals the human-verified pin. Structural disambiguation \
                         (D62 S4) is open work — this is declared selection, and it fails closed if the pin \
                         matches zero or several readings. Pin note: {}",
                        if pin.note.is_empty() { "(none)" } else { &pin.note }
                    )),
                );
            }
            SentenceSelection::Ranked(sel) => {
                dp.set(
                    iri(&format!("{ENC}:selected_by")),
                    Value::String(iri(&format!("{ENC}:authority_ranker")).as_str().to_string()),
                );
                if !sel.runner_up_skeletons.is_empty() {
                    dp.set(
                        iri(&format!("{ENC}:runner_up_skeletons")),
                        Value::Array(
                            sel.runner_up_skeletons
                                .iter()
                                .map(|s| Value::String(s.clone()))
                                .collect(),
                        ),
                    );
                }
                dp.set(
                    iri(&format!("{PROV}:rationale")),
                    Value::String(format!(
                        "Reading selected by the READING RANKER (d63-reading-selection): an \
                         untrusted choice in document context, recorded for audit — every \
                         candidate type-checks, so no kernel veto exists; the reading-adjudication \
                         ledger and this record are the controls. Ranker rationale: {}",
                        sel.rationale
                    )),
                );
            }
            SentenceSelection::Sole => {
                dp.set(
                    iri(&format!("{ENC}:selected_by")),
                    Value::String(iri(&format!("{ENC}:authority_sole")).as_str().to_string()),
                );
                dp.set(
                    iri(&format!("{PROV}:rationale")),
                    Value::String(
                        "Sole surviving reading — the forest offered exactly one felicitous \
                         parse; no selection existed to make."
                            .to_string(),
                    ),
                );
            }
        }
        out.push(dp);

        // Anaphora bindings (D67 §3) — one `enc:AnaphorBinding` per resolved hole: the accepted
        // antecedent, machine-readable (an IRI reference for individuals/claims, the D47 encoding
        // for kind terms), plus the proposing authority and the proposer's audit fields.
        for (k, b) in s.bindings.iter().enumerate() {
            let mut ab = res(
                &format!("{ns}:binding_{n}_{k}"),
                &[&format!("{ENC}:AnaphorBinding")],
            );
            ab.set(
                iri(&format!("{ENC}:binding_unit")),
                Value::String(iri(&scoped_iri).as_str().to_string()),
            );
            ab.set(
                iri(&format!("{ENC}:hole_var")),
                Value::String(b.hole.clone()),
            );
            ab.set(
                iri(&format!("{ENC}:antecedent_surface")),
                Value::String(b.antecedent.surface().to_string()),
            );
            match &b.antecedent {
                Candidate::Individual { iri: ante, .. } => {
                    ab.set(
                        iri(&format!("{ENC}:antecedent_resource")),
                        Value::iri(&ante.clone()),
                    );
                }
                Candidate::Kind { term, .. } => {
                    let encoded = encode_type(term).map_err(|e| EmitError::Encode {
                        ordinal: n,
                        detail: format!("kind antecedent: {e:?}"),
                    })?;
                    ab.set(iri(&format!("{ENC}:antecedent_term")), encoded);
                }
                Candidate::Claim { resource, .. } => {
                    if let Some(id) = resource.id() {
                        ab.set(
                            iri(&format!("{ENC}:antecedent_resource")),
                            Value::iri(&id.clone()),
                        );
                    }
                }
                // A SET antecedent (D68 §5): one resource ref per member, in run order.
                Candidate::ClaimSet { members, .. } => {
                    let refs: Vec<Value> = members
                        .iter()
                        .filter_map(|r| r.id())
                        .map(|id| Value::iri(&id.clone()))
                        .collect();
                    ab.set(
                        iri(&format!("{ENC}:antecedent_resources")),
                        Value::Array(refs),
                    );
                }
            }
            if let Some(a) = s.binding_authority {
                ab.set(
                    iri(&format!("{ENC}:bound_by")),
                    Value::String(iri(&format!("{ENC}:{a}")).as_str().to_string()),
                );
            }
            if let Some(r) = &b.rationale {
                ab.set(iri(&format!("{PROV}:rationale")), Value::String(r.clone()));
            }
            if let Some(c) = b.confidence {
                ab.set(iri(&format!("{ENC}:confidence")), Value::Float(c));
            }
            out.push(ab);
        }
    }

    // Non-encoded units: the DiscourseUnit (same shape as an encoded unit's) + the CutItem
    // stating why. `cut_unit` references the unit; no ScopedUnit/claim exists to reference.
    for c in cuts {
        let n = c.ordinal;
        let unit_iri = format!("{ns}:unit_{n}");
        out.push(discourse_unit(ns, n, &c.text, c.span, &doc_iri));
        let (kind, rationale) = match &c.reason {
            CutReason::Ambiguous { readings } => (
                "cut_ambiguous",
                format!(
                    "{readings} readings survived and the run's selection authority did not \
                     choose among them — a selection gap, recorded fail-closed."
                ),
            ),
            CutReason::Unresolved { holes } => (
                "cut_unresolved",
                format!(
                    "{holes} referent hole(s) no discourse antecedent resolved — an open parse, \
                     recorded fail-closed."
                ),
            ),
            CutReason::NoParse { oov } if !oov.is_empty() => (
                "cut_vocabulary",
                format!(
                    "no parse; residual out-of-vocabulary surfaces (Stage A): {}",
                    oov.join(", ")
                ),
            ),
            CutReason::NoParse { .. } => (
                "cut_grammar",
                "no parse with every token in vocabulary — the grammar could not compose the \
                 construction."
                    .to_string(),
            ),
        };
        let mut cut = res(&format!("{ns}:cut_{n}"), &[&format!("{ENC}:CutItem")]);
        cut.set(
            iri(&format!("{ENC}:cut_unit")),
            Value::String(iri(&unit_iri).as_str().to_string()),
        );
        cut.set(
            iri(&format!("{ENC}:cut_kind")),
            Value::String(iri(&format!("{ENC}:{kind}")).as_str().to_string()),
        );
        cut.set(iri(&format!("{PROV}:rationale")), Value::String(rationale));
        out.push(cut);
    }

    // The ROOT, last: it lists the claims, so every IRI it names is defined above it in the same
    // document. `enc:claims` is `requires`d, so a document that encoded NOTHING still emits the
    // root with an empty array — the artifact then states "this source yielded no claims", which
    // is a result, not an absence.
    let mut structure = res(
        &format!("{ns}:structure"),
        &[&format!("{ENC}:ReasoningStructure")],
    );
    structure.set(iri(&format!("{ENC}:claims")), Value::Array(claim_iris));
    structure.set(
        iri(&format!("{ENC}:document")),
        Value::String(iri(&doc_iri).as_str().to_string()),
    );
    structure.set(
        iri(&format!("{ENC}:source_path")),
        Value::String(source_path.to_string()),
    );
    structure.set(
        iri(&format!("{ENC}:source_sha256")),
        Value::String(source_sha256.to_string()),
    );
    structure.set(
        iri(&format!("{CORE}:description")),
        Value::String(format!(
            "The encoding of {source_path}: {} claim(s), {} unit(s) recorded as not encoding.",
            sentences.len(),
            cuts.len()
        )),
    );

    // ONE ProgramTrace for the RUN (eigenius#201, second pass). The parse is a program execution —
    // (engine, bytes) -> structure — so it gets exactly one trace, on the run's output. Not one per
    // sentence: that was the old shape's cardinality error, and each of those N traces additionally
    // claimed the program had derived a proposition about the world.
    //
    // Emitted BEFORE the structure so the IRI it references is defined above it, matching the
    // ordering discipline the root already follows for `enc:claims`.
    let run_trace_iri = format!("{ns}:run_trace");
    let mut run_trace = res(&run_trace_iri, &[&format!("{PROV}:ProgramTrace")]);
    run_trace.set(
        iri(&format!("{PROV}:resource")),
        Value::String(iri(&format!("{ns}:structure")).as_str().to_string()),
    );
    run_trace.set(
        iri(&format!("{PROV}:was_generated_by")),
        Value::String(format!(
            "eigenius-encoding prose-to-eigon: DCG parse (D63) of {source_path} \
             (source sha256 {source_sha256})"
        )),
    );
    run_trace.set(
        iri(&format!("{PROV}:timestamp")),
        Value::String(timestamp.to_string()),
    );
    out.push(run_trace);

    structure.set(
        iri(&format!("{PROV}:derivation")),
        Value::String(iri(&run_trace_iri).as_str().to_string()),
    );
    out.push(structure);

    Ok(out)
}

/// The `enc:DiscourseUnit` record — identical for encoded and cut sentences.
///
/// `doc_iri` is the `reference:Reference` for the source work. It replaces the former
/// `enc:section = "<path> (sha256 <hex>)"` string, which put run provenance in a field meant for a
/// human-readable location within the document ("Results §2.1"). The bytes are now pinned once, on
/// the root; `enc:section` is left for what it is for and is emitted only when known — which, for a
/// plain text file, is never.
fn discourse_unit(
    ns: &str,
    ordinal: usize,
    text: &str,
    span: (usize, usize),
    doc_iri: &str,
) -> Resource {
    let mut unit = res(
        &format!("{ns}:unit_{ordinal}"),
        &[&format!("{ENC}:DiscourseUnit")],
    );
    unit.set(
        iri(&format!("{ENC}:prose")),
        Value::String(text.to_string()),
    );
    unit.set(
        iri(&format!("{ENC}:unit_kind")),
        Value::String(iri(&format!("{ENC}:kind_prose")).as_str().to_string()),
    );
    unit.set(
        iri(&format!("{ENC}:span_start")),
        Value::Integer(span.0 as i64),
    );
    unit.set(
        iri(&format!("{ENC}:span_end")),
        Value::Integer(span.1 as i64),
    );
    unit.set(
        iri(&format!("{ENC}:source_document")),
        Value::iri(&iri(doc_iri)),
    );
    unit
}

/// The skeleton the parser actually produced for a reading — used by the driver's report.
pub fn reading_skeleton(item: &Item) -> String {
    skeleton_of(item.sem())
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-formed IRI")
}

fn res(id: &str, classes: &[&str]) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(&format!("{CORE}:is_a")),
        Value::Array(classes.iter().map(|c| Value::iri(&iri(c))).collect()),
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::dcg::{Combinator, Cost};
    use eigenius_kernel::nbe::term::Exp;

    /// A minimal encodable Item — an `EigonClass` sem is inside the D47 fragment, and the cat is
    /// never consulted by emission.
    fn item() -> Item {
        let cls = Exp::EigonClass(iri("urn:eigenius:demo:Thing"));
        Item::from_parts(cls.clone(), cls, Combinator::Other, Cost::ZERO)
    }

    fn sentence<'a>(item: &'a Item, selection: SentenceSelection<'a>) -> ParsedSentence<'a> {
        ParsedSentence {
            ordinal: 1,
            text: "A thing.".to_string(),
            span: (0, 8),
            item,
            candidates: 3,
            selection,
            bindings: Vec::new(),
            binding_authority: None,
            cluster: None,
        }
    }

    fn emit(s: &[ParsedSentence<'_>]) -> String {
        emit_full(&[], s, &[])
    }

    fn emit_full(glossary: &[Resource], s: &[ParsedSentence<'_>], cuts: &[CutSentence]) -> String {
        emit_document(
            &DocumentMeta {
                ns: "urn:eigenius:test:doc",
                source_path: "test.txt",
                source_sha256: "deadbeef",
                timestamp: "2026-08-11T00:00:00Z",
                declared_by: crate::UNATTRIBUTED_AGENT,
                source_ref: None,
            },
            glossary,
            s,
            cuts,
        )
        .expect("emits")
    }

    #[test]
    fn ranked_selection_emits_the_computed_choice_record() {
        let it = item();
        let sel = SelectionOutcome {
            chosen_skeleton: "skel-chosen".to_string(),
            chosen_sem: "sem-chosen".to_string(),
            chosen_gloss: "a thing".to_string(),
            rationale: "the document is about things".to_string(),
            runner_up_skeletons: vec!["skel-b".to_string(), "skel-c".to_string()],
            candidates: 3,
        };
        let json = emit(&[sentence(&it, SentenceSelection::Ranked(&sel))]);
        assert!(json.contains("urn:eigenius:encoding:authority_ranker"));
        assert!(json.contains("urn:eigenius:encoding:runner_up_skeletons"));
        assert!(json.contains("skel-b") && json.contains("skel-c"));
        assert!(json.contains("the document is about things"));
        assert!(json.contains("READING RANKER"));
        assert!(
            json.contains("the reading the ranker selected: skel-chosen"),
            "the claim description names the chosen skeleton"
        );
    }

    #[test]
    fn sole_selection_emits_the_sole_authority_and_no_runners_up() {
        let it = item();
        let json = emit(&[sentence(&it, SentenceSelection::Sole)]);
        assert!(json.contains("urn:eigenius:encoding:authority_sole"));
        assert!(!json.contains("runner_up_skeletons"));
        assert!(json.contains("Sole surviving reading"));
    }

    /// D71 §4.1 — the artifact has a ROOT. Without it a service has nothing to return, a notebook
    /// cell nothing to re-open, and a superseding run nothing to point at.
    #[test]
    fn the_artifact_is_rooted_at_a_reasoning_structure_listing_its_claims() {
        let it = item();
        let json = emit(&[sentence(&it, SentenceSelection::Sole)]);
        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let root = doc
            .as_array()
            .expect("document is an array")
            .iter()
            .find(|r| r["@id"] == "urn:eigenius:test:doc:structure")
            .expect("the root is emitted");
        assert_eq!(
            root["urn:eigenius:encoding:claims"],
            serde_json::json!(["urn:eigenius:test:doc:claim_1"]),
            "the root lists the claims it assembled"
        );
        assert_eq!(
            root["urn:eigenius:encoding:document"],
            serde_json::json!("urn:eigenius:test:doc:source")
        );
        assert_eq!(
            root["urn:eigenius:encoding:source_sha256"],
            serde_json::json!("deadbeef"),
            "the bytes are pinned once, on the root"
        );
    }

    /// A document that encoded NOTHING still emits the root: "this source yielded no claims" is a
    /// result, and a caller that got an artifact back must not have to guess whether it ran.
    #[test]
    fn a_document_that_encodes_nothing_still_emits_the_root() {
        let json = emit_full(
            &[],
            &[],
            &[CutSentence {
                ordinal: 1,
                text: "Zorblax.".to_string(),
                span: (0, 8),
                reason: CutReason::NoParse { oov: vec![] },
            }],
        );
        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let root = doc
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["@id"] == "urn:eigenius:test:doc:structure")
            .expect("the root is emitted even with zero claims");
        assert_eq!(
            root["urn:eigenius:encoding:claims"],
            serde_json::json!([]),
            "an empty claim list, not an absent root"
        );
    }

    /// Every unit cites the source WORK by reference, rather than carrying the run's path+sha in
    /// `enc:section` — a field for a human-readable location inside the document.
    #[test]
    fn units_cite_the_source_reference_and_leave_section_alone() {
        let it = item();
        let json = emit(&[sentence(&it, SentenceSelection::Sole)]);
        assert!(json.contains(
            r#""urn:eigenius:encoding:source_document": "urn:eigenius:test:doc:source""#
        ));
        assert!(
            !json.contains("urn:eigenius:encoding:section"),
            "section is emitted only when a real section label is known"
        );
    }

    /// A CITED reference is not re-emitted: a second definition of a chain-resident IRI is a
    /// redefinition. Rule 22 then does the verifying — an IRI naming no Reference fails the load.
    #[test]
    fn a_cited_reference_is_pointed_at_never_minted() {
        let it = item();
        let s = [sentence(&it, SentenceSelection::Sole)];
        let json = emit_document(
            &DocumentMeta {
                ns: "urn:eigenius:test:doc",
                source_path: "test.txt",
                source_sha256: "deadbeef",
                timestamp: "2026-08-11T00:00:00Z",
                declared_by: crate::UNATTRIBUTED_AGENT,
                source_ref: Some("urn:eigenius:reference:lit:chan_2019"),
            },
            &[],
            &s,
            &[],
        )
        .expect("emits");
        assert!(json.contains(
            r#""urn:eigenius:encoding:source_document": "urn:eigenius:reference:lit:chan_2019""#
        ));
        assert!(
            !json.contains("urn:eigenius:reference:Reference"),
            "no Reference resource is emitted when one is cited"
        );
        assert!(!json.contains("urn:eigenius:test:doc:source"));
    }

    /// A cut sentence lands as its DiscourseUnit + a CutItem naming the reason — never dropped.
    #[test]
    fn cut_sentences_land_as_units_with_cut_items() {
        let cuts = [
            CutSentence {
                ordinal: 1,
                text: "Ambiguous prose.".to_string(),
                span: (0, 16),
                reason: CutReason::Ambiguous { readings: 7 },
            },
            CutSentence {
                ordinal: 2,
                text: "These findings dangle.".to_string(),
                span: (17, 39),
                reason: CutReason::Unresolved { holes: 1 },
            },
            CutSentence {
                ordinal: 3,
                text: "Zorblax fixination.".to_string(),
                span: (40, 59),
                reason: CutReason::NoParse {
                    oov: vec!["zorblax".to_string(), "fixination".to_string()],
                },
            },
            CutSentence {
                ordinal: 4,
                text: "Known words, no parse.".to_string(),
                span: (60, 82),
                reason: CutReason::NoParse { oov: vec![] },
            },
        ];
        let json = emit_full(&[], &[], &cuts);
        for n in 1..=4 {
            assert!(json.contains(&format!("urn:eigenius:test:doc:unit_{n}")));
            assert!(json.contains(&format!("urn:eigenius:test:doc:cut_{n}")));
        }
        assert!(json.contains("urn:eigenius:encoding:cut_ambiguous"));
        assert!(json.contains("7 readings survived"));
        assert!(json.contains("urn:eigenius:encoding:cut_unresolved"));
        assert!(json.contains("1 referent hole(s)"));
        assert!(json.contains("urn:eigenius:encoding:cut_vocabulary"));
        assert!(json.contains("zorblax, fixination"));
        assert!(json.contains("urn:eigenius:encoding:cut_grammar"));
        assert!(
            !json.contains("claim_") && !json.contains("scoped_"),
            "a cut unit carries no claim cluster and no ScopedUnit"
        );
    }

    /// Stage-A glossary resources pass through into the artifact verbatim.
    #[test]
    fn glossary_resources_are_emitted() {
        let mut g = Resource::new(iri("urn:eigenius:test:doc:lex:msi"));
        g.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("MSI".to_string()),
        );
        let it = item();
        let json = emit_full(&[g], &[sentence(&it, SentenceSelection::Sole)], &[]);
        assert!(json.contains("urn:eigenius:test:doc:lex:msi"));
        assert!(json.contains("urn:eigenius:test:doc:unit_1"));
    }

    /// The PIN arm's record is BYTE-STABLE: no `selected_by`, the historical rationale text —
    /// the committed demo artifacts regenerate identically under it.
    #[test]
    fn pinned_selection_emits_the_historical_shape() {
        let it = item();
        let pin = crate::select::Pin {
            sentence: "A thing.".to_string(),
            skeleton: "skel-pinned".to_string(),
            note: "verified".to_string(),
        };
        let json = emit(&[sentence(&it, SentenceSelection::Pinned(&pin))]);
        assert!(json.contains("Reading selected by SKELETON PIN"));
        assert!(json.contains("Pin note: verified"));
        assert!(json.contains("the reading pinned as correct: skel-pinned"));
        assert!(
            !json.contains("selected_by") && !json.contains("authority_"),
            "the pin arm emits no SelectionAuthority — byte-stability of committed artifacts"
        );
    }

    /// eigenius#201, second pass: the artifact's TWO objects take opposite epistemic categories,
    /// and the run gets exactly ONE trace.
    #[test]
    fn the_run_is_derived_once_and_the_claims_are_declared() {
        let it = item();
        let three = [
            sentence(&it, SentenceSelection::Sole),
            sentence(&it, SentenceSelection::Sole),
            sentence(&it, SentenceSelection::Sole),
        ];
        let json = emit(&three);
        let doc: serde_json::Value = serde_json::from_str(&json).expect("artifact is JSON");
        let resources = doc["@graph"]
            .as_array()
            .or_else(|| doc.as_array())
            .expect("the artifact is a resource list");

        let class_is = |r: &serde_json::Value, c: &str| {
            r["urn:eigenius:core:is_a"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(c)))
        };

        // CARDINALITY is the point: one parse run is one program execution. The old shape emitted
        // one ProgramTrace per SENTENCE — here that would be three.
        let program_traces: Vec<_> = resources
            .iter()
            .filter(|r| class_is(r, "urn:eigenius:prov:ProgramTrace"))
            .collect();
        assert_eq!(
            program_traces.len(),
            1,
            "exactly one ProgramTrace for the run, over {} sentences",
            three.len()
        );

        // ...and it is on the RUN's output, not on a claim.
        let structure_iri = "urn:eigenius:test:doc:structure";
        assert_eq!(
            program_traces[0]["urn:eigenius:prov:resource"].as_str(),
            Some(structure_iri),
            "the run's trace targets the ReasoningStructure"
        );

        let structure = resources
            .iter()
            .find(|r| r["@id"].as_str() == Some(structure_iri))
            .expect("the artifact has a root");
        assert!(class_is(
            structure,
            "urn:eigenius:encoding:ReasoningStructure"
        ));
        assert!(
            structure["urn:eigenius:prov:derivation"].is_string()
                || structure["urn:eigenius:prov:derivation"].is_object(),
            "the structure points at its ProgramTrace (required by DerivedResource)"
        );

        // The propositions are Declared, one trace each, each naming an agent.
        let decl_traces: Vec<_> = resources
            .iter()
            .filter(|r| class_is(r, "urn:eigenius:prov:DeclarationTrace"))
            .collect();
        assert_eq!(
            decl_traces.len(),
            three.len(),
            "one DeclarationTrace per claim"
        );
        for t in &decl_traces {
            assert!(
                !t["urn:eigenius:prov:was_attributed_to"].is_null(),
                "a DeclarationTrace names who declared: {t}"
            );
        }
    }
}
