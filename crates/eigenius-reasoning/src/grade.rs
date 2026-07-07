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
}

impl std::fmt::Display for GradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GradeError::Encode(m) => write!(f, "proposition failed to encode: {m}"),
            GradeError::Iri(m) => write!(f, "malformed cluster IRI: {m}"),
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

        // (2) The DeclarationTrace — emits IsDeclaredAs(declaring, P) into the chain witness index.
        let trace_iri = iri(&format!("{}:assertion-trace", source.stem))?;
        let mut trace = Resource::new(trace_iri);
        trace.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(wk::DECLARATION_TRACE)?)]),
        );
        trace.set(
            iri(wk::REFLECTION_RESOURCE)?,
            Value::ResourceRef(declaring_iri.clone()),
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
    Value::Json(json!({
        "ctor": "App",
        "args": [
            { "ctor": "App", "args": [
                { "ctor": "App", "args": [
                    { "ctor": "CtorApp", "args": [JUSTIFIED_BY, "declared"] },
                    { "ctor": "LitString", "args": [iri] },
                ]},
                prop_subtree,
            ]},
            { "ctor": "UnitVal", "args": [] },
        ],
    }))
}
