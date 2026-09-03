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

//! **Grading** — turning a parsed proposition into a chain-resident claim.
//!
//! The DCG pipeline ([`eigenius_kernel::dcg::DocumentPipeline`]) ends at a closed proposition: a
//! `SentenceOutcome::Encoded(item)` carries `item.sem() : Prop`, a typed tree. That is well-typed
//! *syntax*, not yet a claim the graph holds. This module turns it into one.
//!
//! ## A parsed claim is a 2-resource cluster
//!
//! 1. the **`enc:EncodedClaim`** — `reflection:canonical_proposition = P` plus
//!    `prov:was_attributed_to`, the agent taking responsibility for `P`;
//! 2. its **`prov:DeclarationTrace`** — which mints `IsDeclaredAs(claim_iri, P)` into the
//!    witness index at commit.
//!
//! Parsed sentences land **Declared**, by the agent or the source document's authors (D73 §6): the
//! parser is a formulation instrument, so it establishes that the text parses to this well-typed
//! term and nothing more. The RUN that produced the form is recorded once, elsewhere — the
//! `enc:ReasoningStructure` is a `reflection:DerivedResource` under a single `ProgramTrace`.
//!
//! **This module lived in `eigenius-reasoning` until it moved here.** Its consumers are this
//! crate's [`emit`](crate::emit), [`pipeline`](crate::pipeline) and
//! [`formalize`](crate::formalize) — grading a parsed claim is the encoding pipeline's own job,
//! not the justification calculus's.

use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::encode_type;

const REFLECTION_DECLARED_BY: &str = "urn:eigenius:prov:was_attributed_to";
/// The bootstrap agent meaning "no agent was recorded" (D72 §3.1) — a real, resolvable
/// resource, unlike the literals that used to sit in this slot.
pub const UNATTRIBUTED_AGENT: &str = "urn:eigenius:prov:agent:unattributed";
const REFLECTION_TIMESTAMP: &str = "urn:eigenius:prov:timestamp";
/// `urn:eigenius:encoding:EncodedClaim` — the Derived cluster's claim class (D67 §1).
const ENCODED_CLAIM_CLASS: &str = "urn:eigenius:encoding:EncodedClaim";
/// `urn:eigenius:prov:DeclarationTrace` — the trace that mints `IsDeclaredAs`. Parsed claims
/// land through this since eigenius#201 / D73 §6; it was a `ProgramTrace` minting `IsDerivedAs`
/// until `2026-08-21`. A `ProgramTrace` now mints nothing at all, so the move anticipated by
/// several months what the three grounds made general.
const DECLARATION_TRACE_CLASS: &str = wk::DECLARATION_TRACE;

// `Grade` and `Warrant` stood here, and they meant each other's referent.
//
// `Grade {Declared, Observed, Derived, Verified}` was the paper's GROUNDS.
// `Warrant {Declared, Parsed}` was documented as "the axis along which the grade
// climbs" and projected onto a Grade. The paper uses *warrant* for the axis whose
// values are grounds, so the two words were swapped.
//
// Both are gone rather than renamed. `Grade` graded a claim on an axis that is
// now computed from a justification term and stored nowhere. `Warrant`'s own
// distinction was never warrant either: BOTH its variants projected to
// `Grade::Declared`, and what separated them is that a parse run produced one —
// which is provenance. The code already said so on the projection ("the parser is
// a formulation instrument, not a warrant"), so the insight was present and only
// the name was wrong. That distinction is now `prov:was_generated_by(parse run)`
// against its absence, with `prov:was_attributed_to(agent)` on both, and the enum
// has nothing left to carry. `Warrant::grade()` retires with them.
//
// Its `#[non_exhaustive]` growth path — "the Observed/Verified climbs are the
// next increments" — is superseded: those are grounds, not refinements of one.

/// The provenance of a claim: where its IRIs are rooted and what warrants it.
pub struct ClaimSource<'a> {
    /// A deterministic IRI stem for the claim's cluster (e.g. `urn:eigenius:doc:<id>:s<n>`), so the
    /// declaring resource / trace / sentence get stable, dedup-friendly IRIs derived from it.
    pub stem: &'a str,
    /// `prov:was_attributed_to` — REQUIRED by `reflection:DeclaredResource`, and
    /// `prov:timestamp` is REQUIRED by `prov:DeclarationTrace`. Omitting either builds
    /// a cluster that cannot actually commit (`MissingRequired`) — and in-process tests will not
    /// catch it, because `LayerBuilder` does not run the validator; only a real `eigenius load`
    /// does (found 2026-08-03).
    ///
    /// Must be the **IRI of a `prov:Agent`** since D72 §3.2 retyped the property: it is
    /// written as an IRI reference, so Rule 8 and Rule 22 require it to resolve same-or-lower.
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
    /// the `enc:EncodedClaim` (Derived cluster), the `justification:Conclusion` (inference clusters).
    pub claim_iri: Iri,
    /// The `justification:Conclusion` the D39 gate validates at commit, when the cluster carries one.
    /// `None` for the parsed cluster — its trust story is the agent named in `declared_by`, with
    /// no certificate to check.
    pub gate_sentence: Option<Iri>,
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
/// kernel-checkable claim. Pure construction; the D39 D39 gate validates the result at
/// commit. Downstream of the DCG pipeline.
pub trait ClaimGrader {
    /// Build the claim cluster asserting `proposition` at the grade its `source` warrants.
    fn grade(
        &self,
        proposition: &Exp,
        source: &ClaimSource,
        names: &eigenius_kernel::program::eigentt_type_mirror::CodecNames,
    ) -> Result<GradedClaim, GradeError>;
}

/// The **parsed-claim** grader (D73 §6 — the landing shape for parsed sentences): the 2-resource
/// cluster
///
/// 1. the **`enc:EncodedClaim`** — carries `reflection:canonical_proposition = P` and
///    `prov:was_attributed_to`, the agent taking responsibility for `P`;
/// 2. its **`prov:DeclarationTrace`** — `prov:resource → claim`, the same
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
/// There is still no `justification:Conclusion` and no certificate, so [`GradedClaim::gate_sentence`] is
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
        names: &eigenius_kernel::program::eigentt_type_mirror::CodecNames,
    ) -> Result<(Resource, Resource), GradeError> {
        let prop_value =
            encode_type(proposition, names).map_err(|e| GradeError::Encode(format!("{e:?}")))?;
        let iri = |s: &str| Iri::parse(s).map_err(|e| GradeError::Iri(format!("{s}: {e:?}")));

        let claim_id = iri(claim_iri)?;
        let mut claim = Resource::new(claim_id.clone());
        let mut classes = vec![Value::String(
            iri(ENCODED_CLAIM_CLASS)?.as_str().to_string(),
        )];
        classes.extend(kind_classes.iter().map(|k| Value::iri(&k.clone())));
        claim.set(iri(wk::IS_A)?, Value::Array(classes));
        claim.set(iri(wk::CANONICAL_PROPOSITION)?, prop_value);
        // REQUIRED now that `enc:EncodedClaim` is a `reflection:DeclaredResource` (eigenius#201).
        claim.set(
            iri(REFLECTION_DECLARED_BY)?,
            Value::String(iri(declared_by)?.as_str().to_string()),
        );

        let mut trace = Resource::new(iri(trace_iri)?);
        trace.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::String(
                iri(DECLARATION_TRACE_CLASS)?.as_str().to_string(),
            )]),
        );
        trace.set(iri(wk::REFLECTION_RESOURCE)?, Value::iri(&claim_id));
        trace.set(
            iri(REFLECTION_DECLARED_BY)?,
            Value::String(iri(declared_by)?.as_str().to_string()),
        );
        trace.set(
            iri(REFLECTION_TIMESTAMP)?,
            Value::String(timestamp.to_string()),
        );
        Ok((claim, trace))
    }
}

impl ClaimGrader for ParsedClaimGrader {
    fn grade(
        &self,
        proposition: &Exp,
        source: &ClaimSource,
        names: &eigenius_kernel::program::eigentt_type_mirror::CodecNames,
    ) -> Result<GradedClaim, GradeError> {
        let claim_iri = format!("{}:claim", source.stem);
        let trace_iri = format!("{}:trace", source.stem);
        let (claim, trace) = Self::cluster(
            &claim_iri,
            &trace_iri,
            proposition,
            source.declared_by,
            source.timestamp,
            source.kind_classes,
            names,
        )?;
        let claim_iri = claim.id().expect("cluster sets the claim id").clone();
        Ok(GradedClaim {
            resources: vec![claim, trace],
            claim_iri,
            gate_sentence: None,
        })
    }
}
