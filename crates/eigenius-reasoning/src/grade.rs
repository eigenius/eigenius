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

//! **Grading** — the parser → reasoning-layer bridge (D63 kind-predication reshape §4, §6 Phase C).
//!
//! The DCG pipeline ([`eigenius_kernel::dcg::DocumentPipeline`]) ends at a closed proposition: a
//! `SentenceOutcome::Encoded(item)` carries `item.sem() : Prop`, a typed tree. That is well-typed
//! *syntax*, not yet a claim the graph holds. This module turns it into one — a graded, witnessed,
//! chain-resident claim — which is a **different institution** (D39 Justification Logic) with its own
//! commit gate ([`crate::validate`]). The reshape's whole thesis is that justification is a *grade*,
//! not a parser hole; grading is where that grade is attached, downstream of the parse.
//!
//! ## A graded claim is a 3-resource cluster, not one resource
//!
//! For the D39 [`crate::validate`] gate to admit a `ReasoningSentence`, its `JustifiedBy.declared`
//! certificate must type-check against an *admitted chain witness*. That witness is emitted by a
//! `reflection:DeclarationTrace` over a `reflection:DeclaredResource` that carries the proposition as
//! its `canonical_proposition`. So one Declared claim is three resources committed together:
//!
//! 1. the **declaring** `reflection:DeclaredResource` — carries `canonical_proposition = P`;
//! 2. its **`reflection:DeclarationTrace`** — emits `IsDeclaredAs(declaring, P)` into the witness index;
//! 3. the **`reasoning:ReasoningSentence`** — `proposition = P`, `justification = DeclaredEvidence(declaring)`,
//!    `certificate = JustifiedBy.declared(declaring, P, _)` (the kernel synthesises the witness slot).
//!
//! [`ClaimGrader::grade`] builds that cluster; committing it runs the gate → `Verdict::Holds`.

use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::encode_type;
use serde_json::json;

use crate::institution::iris;

/// `urn:eigenius:reasoning:ReasoningSentence` — the sentence class the D39 AutoOnLoad gate fires on.
const REASONING_SENTENCE_CLASS: &str = "urn:eigenius:reasoning:ReasoningSentence";
/// `urn:eigenius:reasoning:JustifiedBy` — the indexed inductive whose `declared` ctor the certificate uses.
const JUSTIFIED_BY: &str = "urn:eigenius:reasoning:JustifiedBy";
/// `urn:eigenius:reasoning:JustificationTerm` — the justification algebra the certificate indexes over.
const JUSTIFICATION_TERM: &str = "urn:eigenius:reasoning:JustificationTerm";
const REFLECTION_DECLARED_BY: &str = "urn:eigenius:reflection:declared_by";
/// The bootstrap agent meaning "no agent was recorded" (D72 §3.1) — a real, resolvable
/// resource, unlike the literals that used to sit in this slot.
pub const UNATTRIBUTED_AGENT: &str = "urn:eigenius:reflection:agent:unattributed";
const REFLECTION_TIMESTAMP: &str = "urn:eigenius:reflection:timestamp";
/// `urn:eigenius:encoding:EncodedClaim` — the Derived cluster's claim class (D67 §1).
const ENCODED_CLAIM_CLASS: &str = "urn:eigenius:encoding:EncodedClaim";
/// `urn:eigenius:reflection:DeclarationTrace` — the trace that mints `IsDeclaredAs`. Parsed claims
/// land through this since eigenius#201 / D73 §6; it was a `ProgramTrace` minting `IsDerivedAs`
/// until `2026-08-21`.
const DECLARATION_TRACE_CLASS: &str = wk::DECLARATION_TRACE;

/// The epistemic grade of a claim. A **structural projection** of the `JustificationTerm` constructor
/// (D39) — not a stored field. `Declared` is the honest floor a parsed proposition enters at; it climbs
/// only on a real witness (observation / derivation / proof).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    Declared,
    Observed,
    Derived,
    Verified,
}

/// What warrants a claim's assertion — the axis along which the grade climbs.
///
/// `#[non_exhaustive]` marks the growth axis: the literature-warrant climb (reshape §4 row 2 — a
/// `reference:Citation`, itself a `DeclaredResource`, keeps the grade at Declared-but-attested)
/// and the `Observed`/`Verified` climbs are the next increments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warrant {
    /// The honest floor (reshape §4 row 1): the source document asserts the proposition.
    Declared,
    /// The PARSER produced the claim from a source span. Lands **Declared**, not Derived
    /// (D73 §6 / eigenius#201, superseding the `2026-08-10` Derived-landing decision): the parse
    /// establishes that the text parses to this well-typed term, not that the term is faithful to
    /// what the author wrote (D61, unbuilt) nor that what the author wrote is true. The agent named
    /// in `declared_by` is who takes responsibility — the source document's authors when encoding a
    /// paper, the operating agent when an agent formulates its own claim.
    Parsed,
}

impl Warrant {
    /// The grade this warrant projects to. Public because a caller that builds a cluster through
    /// [`ParsedClaimGrader::cluster`] directly (to control the IRIs) still has to state the
    /// grade, and it must be THIS projection, not a second hand-written mapping.
    pub fn grade(self) -> Grade {
        match self {
            Warrant::Declared => Grade::Declared,
            // Not `Grade::Derived`: the parser is a formulation instrument, not a warrant
            // (eigenius#201).
            Warrant::Parsed => Grade::Declared,
        }
    }
}

/// The provenance of a claim: where its IRIs are rooted and what warrants it.
pub struct ClaimSource<'a> {
    /// A deterministic IRI stem for the claim's cluster (e.g. `urn:eigenius:doc:<id>:s<n>`), so the
    /// declaring resource / trace / sentence get stable, dedup-friendly IRIs derived from it.
    pub stem: &'a str,
    /// What warrants the assertion.
    pub warrant: Warrant,
    /// `reflection:declared_by` — REQUIRED by `reflection:DeclaredResource`, and
    /// `reflection:timestamp` is REQUIRED by `reflection:DeclarationTrace`. Omitting either builds
    /// a cluster that cannot actually commit (`MissingRequired`) — and in-process tests will not
    /// catch it, because `LayerBuilder` does not run the validator; only a real `eigenius load`
    /// does (found 2026-08-03).
    ///
    /// Must be the **IRI of a `reflection:Agent`** since D72 §3.2 retyped the property: it is
    /// written as a `ResourceRef`, so Rule 8 and Rule 22 require it to resolve same-or-lower.
    /// A program's name is not an answer to *who* — that belongs in `provenance`. Required by BOTH
    /// clusters since eigenius#201 made the parsed cluster Declared. `UNATTRIBUTED_AGENT` is the
    /// honest value when no agent is known.
    pub declared_by: &'a str,
    pub timestamp: &'a str,
    /// The program-provenance line — *which program produced the claim from which bytes*.
    ///
    /// **Recorded once per RUN, on the `enc:ReasoningStructure`'s `ProgramTrace`, not here**
    /// (eigenius#201 second pass): one parse run is one program execution, so repeating the engine
    /// line on every claim's trace stated the same fact N times. Retained on `ClaimSource` for the
    /// curated grader and for callers assembling their own cluster; the parsed grader no longer
    /// writes it.
    pub provenance: &'a str,
    /// The discourse-KIND classes the claim carries beside its record class (D68 §2 — the
    /// two-axis claim: `is_a = [enc:EncodedClaim, <kinds…>]`, what makes it referable by a
    /// demonstrative's restrictor). Consumed by the parsed grader; ignored by the curated one.
    pub kind_classes: &'a [Iri],
}

/// A graded claim, ready to commit: the cluster's resources, the claim's chain identity, the
/// gate-validatable sentence when one exists, and the grade it commits at.
pub struct GradedClaim {
    /// The cluster — commit all of them together.
    pub resources: Vec<Resource>,
    /// The resource carrying the proposition — the claim's chain identity, what downstream
    /// certificates and discourse candidates cite: the DECLARING resource (Declared cluster),
    /// the `enc:EncodedClaim` (Derived cluster), the `ReasoningSentence` (inference clusters).
    pub claim_iri: Iri,
    /// The `ReasoningSentence` the D39 gate validates at commit, when the cluster carries one.
    /// `None` for the parsed cluster — its trust story is the agent named in `declared_by`, with
    /// no certificate to check.
    pub gate_sentence: Option<Iri>,
    /// The grade the claim commits at (projected from the [`Warrant`]).
    pub grade: Grade,
}

/// Failure to build a claim cluster — the proposition didn't encode, or a derived IRI was malformed.
#[derive(Debug)]
pub enum GradeError {
    /// The proposition `Exp` failed to encode through the D47 codec.
    Encode(String),
    /// A cluster IRI derived from the source stem was not a valid IRI.
    Iri(String),
    /// The sentence offered as a rule did not parse to an implication.
    NotAConditional(String),
    /// The conditional's antecedent is not the premise sentence's proposition. `app` needs the same
    /// `A` on both sides, so the premise must be the `if`-clause verbatim.
    AntecedentMismatch,
}

impl std::fmt::Display for GradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GradeError::Encode(m) => write!(f, "proposition failed to encode: {m}"),
            GradeError::Iri(m) => write!(f, "malformed cluster IRI: {m}"),
            GradeError::NotAConditional(e) => write!(
                f,
                "the rule sentence did not parse to an implication (`S1 if S2`); got {e}"
            ),
            GradeError::AntecedentMismatch => write!(
                f,
                "the conditional's antecedent is not the premise sentence's proposition — `app` \
                 requires the SAME term on both sides, so the premise must be the `if`-clause verbatim"
            ),
        }
    }
}

impl std::error::Error for GradeError {}

/// Turn a closed proposition — a parser's `SentenceOutcome::Encoded(item).sem()` — into a graded,
/// kernel-checkable claim. Pure construction; the D39 [`crate::validate`] gate validates the result at
/// commit. Downstream of the DCG pipeline, in the reasoning institution.
pub trait ClaimGrader {
    /// Build the claim cluster asserting `proposition` at the grade its `source` warrants.
    fn grade(&self, proposition: &Exp, source: &ClaimSource) -> Result<GradedClaim, GradeError>;
}

/// The initial grader — the **Declared floor** (reshape §4 row 1): the source document self-asserts the
/// proposition. Builds the 3-resource cluster with a `DeclaredEvidence(declaring)` justification and a
/// `JustifiedBy.declared` certificate whose witness slot the kernel synthesises from the admitted trace.
pub struct DeclaredClaimGrader;

impl ClaimGrader for DeclaredClaimGrader {
    fn grade(&self, proposition: &Exp, source: &ClaimSource) -> Result<GradedClaim, GradeError> {
        // Encode the proposition ONCE and reuse it for both the declaring resource's
        // canonical_proposition and the certificate's embedded proposition subtree — so the witness
        // the trace emits and the proposition the certificate type-checks against hash-equal by
        // construction (the gh #75 invariant: same bytes on both sides).
        let prop_value =
            encode_type(proposition).map_err(|e| GradeError::Encode(format!("{e:?}")))?;
        let Value::Json(prop_subtree) = prop_value.clone() else {
            return Err(GradeError::Encode(
                "encode_type did not return Value::Json".to_string(),
            ));
        };

        let iri = |s: &str| Iri::parse(s).map_err(|e| GradeError::Iri(format!("{s}: {e:?}")));

        // (1) The declaring DeclaredResource — carries the proposition as a declared fact.
        let declaring_iri = iri(&format!("{}:assertion", source.stem))?;
        let mut declaring = Resource::new(declaring_iri.clone());
        declaring.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(wk::DECLARED_RESOURCE)?)]),
        );
        declaring.set(iri(wk::CANONICAL_PROPOSITION)?, prop_value.clone());
        declaring.set(
            iri(REFLECTION_DECLARED_BY)?,
            Value::ResourceRef(iri(source.declared_by)?),
        );

        // (2) The DeclarationTrace — emits IsDeclaredAs(declaring, P) into the chain witness index.
        // `_trace`, not `-trace`: an IRI's local name becomes an ESL identifier when the
        // resource is written as source, and a hyphen is not one. Minting an IRI here that
        // `eigenius decompile` cannot express would put chain content beyond the reach of
        // the source language.
        let trace_iri = iri(&format!("{}:assertion_trace", source.stem))?;
        let mut trace = Resource::new(trace_iri);
        trace.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(wk::DECLARATION_TRACE)?)]),
        );
        trace.set(
            iri(wk::REFLECTION_RESOURCE)?,
            Value::ResourceRef(declaring_iri.clone()),
        );
        trace.set(
            iri(REFLECTION_DECLARED_BY)?,
            Value::ResourceRef(iri(source.declared_by)?),
        );
        trace.set(
            iri(REFLECTION_TIMESTAMP)?,
            Value::String(source.timestamp.to_string()),
        );

        // (3) The ReasoningSentence — proposition + DeclaredEvidence justification + declared certificate.
        let sentence_iri = iri(&format!("{}:sentence", source.stem))?;
        let mut sentence = Resource::new(sentence_iri.clone());
        sentence.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(REASONING_SENTENCE_CLASS)?)]),
        );
        sentence.set(iri(iris::PROP_PROPOSITION)?, prop_value);
        sentence.set(
            iri(iris::PROP_JUSTIFICATION)?,
            Value::Json(json!({ "ctor": "DeclaredEvidence", "args": [declaring_iri.as_str()] })),
        );
        sentence.set(
            iri(iris::PROP_CERTIFICATE)?,
            justified_by_declared_certificate(declaring_iri.as_str(), prop_subtree),
        );

        Ok(GradedClaim {
            resources: vec![declaring, trace, sentence],
            claim_iri: declaring_iri,
            gate_sentence: Some(sentence_iri),
            grade: source.warrant.grade(),
        })
    }
}

/// The **parsed-claim** grader (D73 §6 — the landing shape for parsed sentences): the 2-resource
/// cluster
///
/// 1. the **`enc:EncodedClaim`** — carries `reflection:canonical_proposition = P` and
///    `reflection:declared_by`, the agent taking responsibility for `P`;
/// 2. its **`reflection:DeclarationTrace`** — `reflection:resource → claim`, the same
///    `declared_by` and a timestamp — which mints `IsDeclaredAs(claim_iri, P)` into the witness
///    index at commit.
///
/// The RUN that produced the form is recorded once, elsewhere: the `enc:ReasoningStructure` is a
/// `reflection:DerivedResource` whose single `ProgramTrace` names the engine and the input bytes.
/// Two objects, two categories — the process is Derived, the propositions are Declared.
///
/// Downstream certificates cite `declared(claim_iri, P, _)`.
///
/// **This landed Derived until `2026-08-21`** (eigenius#201), as a `ProgramTrace` minting
/// `IsDerivedAs`. The `2026-08-10` settlement behind that split the world on *parsed vs curated*;
/// D73 §6 replaced the axis with *who asserts*. The parser is a formulation instrument: it
/// establishes that the text parses to this well-typed term, and cannot establish that the term is
/// faithful to what the author wrote (D61, unbuilt) or that what the author wrote is true. Three
/// propositions that must not collapse into one witness — and `IsDerivedAs(claim, P)` collapsed
/// them, because a certificate citing it read as "a program established P".
///
/// There is still no `ReasoningSentence` and no certificate, so [`GradedClaim::gate_sentence`] is
/// `None` and the D39 gate has nothing to check at commit. A certificate citing the claim still
/// breaks the moment the prose changes and the parser produces a different `P` — the witness key
/// hashes the proposition — which is the point and is unchanged by the grade.
pub struct ParsedClaimGrader;

impl ParsedClaimGrader {
    /// The Derived cluster construction — the ONE source of its shape, shared by the trait path
    /// (stem-derived IRIs) and the artifact emitter (`eigenius-encoding`), which keeps its
    /// historical `{ns}:claim_{n}` / `{ns}:trace_{n}` naming for byte-stable regeneration of
    /// committed artifacts. The emitter adds its document-structural fields (`enc:from_unit`,
    /// `core:description`) to the returned claim; the epistemics live here.
    pub fn cluster(
        claim_iri: &str,
        trace_iri: &str,
        proposition: &Exp,
        declared_by: &str,
        timestamp: &str,
        kind_classes: &[Iri],
    ) -> Result<(Resource, Resource), GradeError> {
        let prop_value =
            encode_type(proposition).map_err(|e| GradeError::Encode(format!("{e:?}")))?;
        let iri = |s: &str| Iri::parse(s).map_err(|e| GradeError::Iri(format!("{s}: {e:?}")));

        let claim_id = iri(claim_iri)?;
        let mut claim = Resource::new(claim_id.clone());
        let mut classes = vec![Value::ResourceRef(iri(ENCODED_CLAIM_CLASS)?)];
        classes.extend(kind_classes.iter().map(|k| Value::ResourceRef(k.clone())));
        claim.set(iri(wk::IS_A)?, Value::Array(classes));
        claim.set(iri(wk::CANONICAL_PROPOSITION)?, prop_value);
        // REQUIRED now that `enc:EncodedClaim` is a `reflection:DeclaredResource` (eigenius#201).
        claim.set(
            iri(REFLECTION_DECLARED_BY)?,
            Value::ResourceRef(iri(declared_by)?),
        );

        let mut trace = Resource::new(iri(trace_iri)?);
        trace.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(DECLARATION_TRACE_CLASS)?)]),
        );
        trace.set(iri(wk::REFLECTION_RESOURCE)?, Value::ResourceRef(claim_id));
        trace.set(
            iri(REFLECTION_DECLARED_BY)?,
            Value::ResourceRef(iri(declared_by)?),
        );
        trace.set(
            iri(REFLECTION_TIMESTAMP)?,
            Value::String(timestamp.to_string()),
        );
        Ok((claim, trace))
    }
}

impl ClaimGrader for ParsedClaimGrader {
    fn grade(&self, proposition: &Exp, source: &ClaimSource) -> Result<GradedClaim, GradeError> {
        let claim_iri = format!("{}:claim", source.stem);
        let trace_iri = format!("{}:trace", source.stem);
        let (claim, trace) = Self::cluster(
            &claim_iri,
            &trace_iri,
            proposition,
            source.declared_by,
            source.timestamp,
            source.kind_classes,
        )?;
        let claim_iri = claim.id().expect("cluster sets the claim id").clone();
        Ok(GradedClaim {
            resources: vec![claim, trace],
            claim_iri,
            gate_sentence: None,
            grade: source.warrant.grade(),
        })
    }
}

/// Build the `JustifiedBy.declared(iri, P, witness)` D47 certificate. The witness slot is `UnitVal` —
/// the kernel ignores the user's value and synthesises the real witness from the chain witness index
/// at type-check time (D39 §9). `prop_subtree` is the D47 encoding of `P`, embedded verbatim so it
/// matches the declaring resource's `canonical_proposition`.
fn justified_by_declared_certificate(iri: &str, prop_subtree: serde_json::Value) -> Value {
    Value::Json(grounding("declared", iri, prop_subtree))
}

/// A `JustifiedBy` grounding constructor — `declared` / `observed` / `derived` / `verified` — applied
/// to the cited IRI and the proposition. The trailing witness slot is `UnitVal`: the kernel discards
/// whatever is there and synthesises the real witness from the chain witness index at type-check time
/// (D39 §9), which is what makes the lookup — not the author — decide whether the certificate stands.
fn grounding(ctor: &str, iri: &str, prop_subtree: serde_json::Value) -> serde_json::Value {
    app_spine(
        json!({ "ctor": "CtorApp", "args": [JUSTIFIED_BY, ctor] }),
        vec![
            json!({ "ctor": "LitString", "args": [iri] }),
            prop_subtree,
            json!({ "ctor": "UnitVal", "args": [] }),
        ],
    )
}

fn app_spine(head: serde_json::Value, args: Vec<serde_json::Value>) -> serde_json::Value {
    args.into_iter()
        .fold(head, |acc, a| json!({ "ctor": "App", "args": [acc, a] }))
}

/// A `JustificationTerm` ctor inside a D47 certificate (`CtorApp` + `App`).
fn jterm_ctor(ctor: &str, iri: &str) -> serde_json::Value {
    app_spine(
        jterm_ctor_head(ctor),
        vec![json!({ "ctor": "LitString", "args": [iri] })],
    )
}

fn jterm_ctor_head(ctor: &str) -> serde_json::Value {
    json!({ "ctor": "CtorApp", "args": [JUSTIFICATION_TERM, ctor] })
}

// ═══════════════════════════════════════════════════════════════════════════════════════════════
// Prose modus ponens — the IMPLICATION itself comes from a parsed sentence
// ═══════════════════════════════════════════════════════════════════════════════════════════════

/// Modus ponens over two **parsed** sentences: a conditional and its antecedent.
///
/// The grammar renders `if` as native implication — `"S₁ if S₂" ⇒ ⟦S₂⟧ → ⟦S₁⟧`, with
/// `sem : λs₂. λs₁. (s₂ → s₁)` (`ontologies/lexicon/closed-class.esl`, whose note says encoding it
/// opaquely "would forfeit modus ponens in the checker"). So a conditional sentence parses to a real
/// `A → B` `Prop`, and its witness is the parser's `IsDerivedAs` like any other claim.
///
/// That makes the inference **entirely Derived**: both `app` premises are parser outputs, and no
/// human declares anything. Contrast [`ChainRuleApplication`], where the implication is a pinned
/// rule a person asserted and the conclusion is therefore no better than Declared.
///
/// The conclusion is not supplied — it is READ OFF the conditional's consequent, so it cannot
/// disagree with what the sentence says. And the antecedent must be **term-identical** to the
/// premise's proposition ([`GradeError::AntecedentMismatch`]), which in practice means the premise
/// sentence has to be the conditional's `if`-clause verbatim. That is a real constraint on how the
/// prose must be written, not something the encoder can paper over: `app` requires the same `A` on
/// both sides.
pub struct ProseModusPonens<'a> {
    /// IRI of the `enc:EncodedClaim` for the CONDITIONAL sentence (its proposition is `A → B`).
    pub rule_claim_iri: &'a str,
    /// IRI of the `enc:EncodedClaim` for the ANTECEDENT sentence (its proposition is `A`).
    pub premise_claim_iri: &'a str,
    /// The antecedent sentence's parsed proposition.
    pub premise: &'a Exp,
}

impl ProseModusPonens<'_> {
    /// Build the concluding sentence. `conditional` is the parsed `A → B`; the conclusion `B` is its
    /// consequent.
    pub fn conclude(
        &self,
        conditional: &Exp,
        source: &ClaimSource,
    ) -> Result<GradedClaim, GradeError> {
        let iri = |s: &str| Iri::parse(s).map_err(|e| GradeError::Iri(format!("{s}: {e:?}")));
        let (ante, conseq) = match conditional {
            Exp::Arrow(a, b) => (a.as_ref().clone(), b.as_ref().clone()),
            // An `Arrow` may already have been normalised to a non-dependent `Pi`.
            Exp::Pi(_, a, b) => (a.as_ref().clone(), b.as_ref().clone()),
            other => {
                return Err(GradeError::NotAConditional(format!("{other:?}")));
            }
        };
        // Compare through the codec: that is the same encoding the witness key hashes, so agreeing
        // here is exactly what makes `derived(premise_claim, A, _)` resolve below.
        let enc = |e: &Exp| encode_type(e).map_err(|x| GradeError::Encode(format!("{x:?}")));
        let (Value::Json(ante_j), Value::Json(prem_j), Value::Json(conseq_j)) =
            (enc(&ante)?, enc(self.premise)?, enc(&conseq)?)
        else {
            return Err(GradeError::Encode("not Value::Json".to_string()));
        };
        if ante_j != prem_j {
            return Err(GradeError::AntecedentMismatch);
        }
        let implication = json!({ "ctor": "Pi", "args": ["", ante_j.clone(), conseq_j.clone()] });

        let certificate = app_spine(
            json!({ "ctor": "CtorApp", "args": [JUSTIFIED_BY, "app"] }),
            vec![
                ante_j.clone(),
                conseq_j.clone(),
                jterm_ctor("DerivedEvidence", self.rule_claim_iri),
                jterm_ctor("DerivedEvidence", self.premise_claim_iri),
                grounding("derived", self.rule_claim_iri, implication),
                grounding("derived", self.premise_claim_iri, ante_j),
            ],
        );

        let sentence_iri = iri(&format!("{}:sentence", source.stem))?;
        let mut sentence = Resource::new(sentence_iri.clone());
        sentence.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(REASONING_SENTENCE_CLASS)?)]),
        );
        sentence.set(iri(iris::PROP_PROPOSITION)?, Value::Json(conseq_j));
        sentence.set(
            iri(iris::PROP_JUSTIFICATION)?,
            Value::Json(json!({
                "ctor": "App",
                "args": [
                    { "ctor": "DerivedEvidence", "args": [self.rule_claim_iri] },
                    { "ctor": "DerivedEvidence", "args": [self.premise_claim_iri] },
                ],
            })),
        );
        sentence.set(iri(iris::PROP_CERTIFICATE)?, Value::Json(certificate));
        Ok(GradedClaim {
            resources: vec![sentence],
            claim_iri: sentence_iri.clone(),
            gate_sentence: Some(sentence_iri),
            // BOTH premises are parser outputs, so unlike a bridged claim nothing here is Declared.
            grade: Grade::Derived,
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════════════════════════
// Applying a PINNED LITERATURE RULE to an already-justified claim
// ═══════════════════════════════════════════════════════════════════════════════════════════════

/// Apply a rule that already sits on the chain — a literature warrant, pinned and cited — to a
/// claim some earlier `ReasoningSentence` established, concluding the rule's consequent.
///
/// This is how a sentence gets justified by INFERENCE rather than by having been written. The
/// activity sentence in a document asserts its own content; the same content also *follows* from a
/// measured antecedent plus a published rule, and that second justification is independent of
/// whether the document says it at all.
///
/// Why the rule can be hand-authored here when a parse-shaped bridge cannot: the rule lives in
/// **domain vocabulary**, so its antecedent is `HighConcentration(thymidine)` — plain `ConstRef`s an
/// ESL author can write. A rule whose antecedent had to be a parse would be inexpressible, since the
/// ESL surface has no syntax for the Σ-binders and projections a DCG term contains.
///
/// The prior sentence is cited with `verified` — a committed `ReasoningSentence` mints
/// `IsVerifiedAs(sentence_iri, P)` on its own IRI (D54), which is exactly the lemma-citation path.
pub struct ChainRuleApplication<'a> {
    /// The pinned rule: a `DeclaredResource` whose `canonical_proposition` is `A → B`.
    pub rule_iri: &'a str,
    /// A committed `ReasoningSentence` that established `A`.
    pub antecedent_sentence_iri: &'a str,
    /// `A`, D47-encoded — byte-identical to that sentence's `proposition`.
    pub antecedent: &'a serde_json::Value,
    /// `B`, D47-encoded — byte-identical to the rule's consequent.
    pub consequent: &'a serde_json::Value,
}

impl ChainRuleApplication<'_> {
    /// Build the concluding `ReasoningSentence`.
    pub fn conclude(&self, source: &ClaimSource) -> Result<GradedClaim, GradeError> {
        let iri = |s: &str| Iri::parse(s).map_err(|e| GradeError::Iri(format!("{s}: {e:?}")));
        let implication = json!({
            "ctor": "Pi",
            "args": ["", self.antecedent.clone(), self.consequent.clone()]
        });
        let certificate = app_spine(
            json!({ "ctor": "CtorApp", "args": [JUSTIFIED_BY, "app"] }),
            vec![
                self.antecedent.clone(),
                self.consequent.clone(),
                jterm_ctor("DeclaredEvidence", self.rule_iri),
                jterm_ctor("VerifiedEvidence", self.antecedent_sentence_iri),
                grounding("declared", self.rule_iri, implication),
                grounding(
                    "verified",
                    self.antecedent_sentence_iri,
                    self.antecedent.clone(),
                ),
            ],
        );
        let sentence_iri = iri(&format!("{}:sentence", source.stem))?;
        let mut s = Resource::new(sentence_iri.clone());
        s.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(REASONING_SENTENCE_CLASS)?)]),
        );
        s.set(
            iri(iris::PROP_PROPOSITION)?,
            Value::Json(self.consequent.clone()),
        );
        s.set(
            iri(iris::PROP_JUSTIFICATION)?,
            Value::Json(json!({
                "ctor": "App",
                "args": [
                    { "ctor": "DeclaredEvidence", "args": [self.rule_iri] },
                    { "ctor": "VerifiedEvidence", "args": [self.antecedent_sentence_iri] },
                ],
            })),
        );
        s.set(iri(iris::PROP_CERTIFICATE)?, Value::Json(certificate));
        Ok(GradedClaim {
            resources: vec![s],
            claim_iri: sentence_iri.clone(),
            gate_sentence: Some(sentence_iri),
            // The rule is Declared (literature), so the conclusion is no stronger.
            grade: Grade::Declared,
        })
    }
}
