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
const REFLECTION_TIMESTAMP: &str = "urn:eigenius:reflection:timestamp";

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
/// The initial [`DeclaredClaimGrader`] supports only the floor. `#[non_exhaustive]` marks the growth
/// axis: the literature-warrant climb (reshape §4 row 2 — a `reference:Citation`, itself a
/// `DeclaredResource`, keeps the grade at Declared-but-attested) and the `Observed`/`Derived`/`Verified`
/// climbs are the next increments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warrant {
    /// The honest floor (reshape §4 row 1): the source document asserts the proposition.
    Declared,
}

impl Warrant {
    /// The grade this warrant projects to.
    fn grade(self) -> Grade {
        match self {
            Warrant::Declared => Grade::Declared,
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
    pub declared_by: &'a str,
    pub timestamp: &'a str,
}

/// A graded claim, ready to commit: the 3-resource cluster (see the module doc), the IRI of the
/// `ReasoningSentence` within it, and the grade it commits at.
pub struct GradedClaim {
    /// The declaring resource, its declaration trace, and the reasoning sentence — commit all three.
    pub resources: Vec<Resource>,
    /// The IRI of the `ReasoningSentence` in [`Self::resources`] (the one the D39 gate validates).
    pub sentence_iri: Iri,
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
            Value::String(source.declared_by.to_string()),
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
            Value::String(source.declared_by.to_string()),
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
            sentence_iri,
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
            sentence_iri,
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
            sentence_iri,
            // The rule is Declared (literature), so the conclusion is no stronger.
            grade: Grade::Declared,
        })
    }
}
