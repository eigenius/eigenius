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
const REFLECTION_RATIONALE: &str = "urn:eigenius:reflection:rationale";
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
    /// `reflection:timestamp` is REQUIRED by `reflection:DeclarationTrace`. Both were previously
    /// omitted, so the cluster this grader builds could not actually be committed
    /// (`MissingRequired`). The in-process tests missed it because they build layers with
    /// `LayerBuilder` directly, which does not run the validator — only a real `eigenius load`
    /// does (found 2026-08-03 building `demo/prose-to-chain`).
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
    /// A bridge named a class its sentence never mentions. Fail closed: a lift may NARROW what a
    /// sentence means, but it may not introduce a subject out of nowhere — and the certificate
    /// alone would not catch it, since only the antecedent is pinned to the witness.
    ArgumentNotInProposition { argument: String, predicate: String },
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
            GradeError::ArgumentNotInProposition {
                argument,
                predicate,
            } => write!(
                f,
                "bridge to `{predicate}` names `{argument}`, which does not occur in the parsed \
                 proposition — the sentence never mentions it"
            ),
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

// ═══════════════════════════════════════════════════════════════════════════════════════════════
// The bridged climb — a parsed proposition lifted to a DOMAIN proposition
// ═══════════════════════════════════════════════════════════════════════════════════════════════

/// Lift a parsed proposition to a **domain** proposition through a Declared bridge.
///
/// [`DeclaredClaimGrader`] commits what the sentence *says*, in the lexicon's own vocabulary
/// (`wn:v01234…`, `umlscui:C…`). A domain conclusion is in the domain's (`onco:RequiresActivity`).
/// Nothing in the parse licenses the step between them: that a sentence meaning `P` warrants the
/// domain claim `C` is a human judgement, so it commits as a `reflection:DeclaredResource` whose
/// canonical proposition is the ground implication `P → C` — the same shape the WRN chain uses for
/// its statistical→domain lifts (`wrn:bridge_msi_selective`), with a parsed antecedent instead of a
/// statistical one.
///
/// The sentence then composes bridge and claim by Artemov application:
///
/// ```text
///   app(P, C, DeclaredEvidence(bridge), DerivedEvidence(claim),
///             declared(bridge, P → C, _),
///             derived(claim, P, _))
/// ```
///
/// **`derived(claim, P, _)` is the load-bearing edge.** It type-checks only against an
/// `IsDerivedAs claim P` witness already in the chain — the one a parser's `reflection:ProgramTrace`
/// minted over the encoded claim. Re-derive the claim from *edited* prose and the parser produces a
/// different `P`; the witness no longer matches; the certificate fails; [`crate::validate`] returns
/// `Fails` and the commit is rejected. Nothing compares the two texts.
///
/// Both sides of the bridge are **closed** propositions, so no term translation is needed — and none
/// is reusable either: a new sentence needs a new bridge. That is the standing cost of this climb.
pub struct BridgedClaimGrader<'a> {
    /// IRI of the already-committed [`enc:EncodedClaim`] whose `IsDerivedAs` witness carries `P`.
    pub claim_iri: &'a str,
    /// The domain predicate, e.g. `urn:eigenius:benchmark:onco:RequiresActivity`.
    pub predicate: &'a str,
    /// The predicate's arguments, in order. An argument beginning `urn:` is emitted as a
    /// **`ConstRef`** — a reference to the very class the parse contains; anything else becomes a
    /// `LitString`.
    ///
    /// That distinction is the whole difference between a bridge that relates two *formulas* and one
    /// that relates a formula to string literals. With `core:string` predicates the consequent says
    /// `RequiresActivity("WRN", "helicase")` while the antecedent contains
    /// `umlscui:C0920283` — and **nothing in the system relates the string to the class**, so the
    /// step is unverifiable by construction. With `Set`-typed predicates the same class appears on
    /// both sides and the correspondence is structural.
    ///
    /// Every `ConstRef` argument is checked to actually OCCUR in the parsed proposition
    /// ([`GradeError::ArgumentNotInProposition`]) — a bridge may narrow what a sentence means, but
    /// it may not introduce a subject the sentence never mentioned.
    pub args: &'a [String],
    /// Who declares the bridge — the authority the lift rests on.
    pub declared_by: &'a str,
    /// Why the lift is warranted. Recorded on chain; this is the human's part of the encoding.
    pub rationale: &'a str,
    /// `reflection:timestamp` for the `DeclarationTrace`. **Required** by the class, so omitting it
    /// produces a cluster that cannot commit (`MissingRequired`) — caught only when the layer goes
    /// through the validator, which the in-process gate tests do not do.
    pub timestamp: &'a str,
}

impl ClaimGrader for BridgedClaimGrader<'_> {
    /// `proposition` is the **parsed** `P` — byte-identical to what the claim's witness carries, or
    /// the certificate will not type-check.
    fn grade(&self, proposition: &Exp, source: &ClaimSource) -> Result<GradedClaim, GradeError> {
        let iri = |s: &str| Iri::parse(s).map_err(|e| GradeError::Iri(format!("{s}: {e:?}")));

        let Value::Json(p) =
            encode_type(proposition).map_err(|e| GradeError::Encode(format!("{e:?}")))?
        else {
            return Err(GradeError::Encode(
                "encode_type did not return Value::Json".to_string(),
            ));
        };
        // A class-referencing argument must be one the sentence actually mentions. Without this the
        // bridge could name any class at all and the kernel would never notice: the certificate only
        // requires the ANTECEDENT to match the witness, so the consequent's arguments are otherwise
        // unconstrained.
        let mentioned = referenced_iris(proposition);
        for a in self.args.iter().filter(|a| a.starts_with("urn:")) {
            if !mentioned.contains(a.as_str()) {
                return Err(GradeError::ArgumentNotInProposition {
                    argument: a.clone(),
                    predicate: self.predicate.to_string(),
                });
            }
        }
        let c = app_spine(
            json!({ "ctor": "ConstRef", "args": [self.predicate] }),
            self.args
                .iter()
                .map(|a| {
                    if a.starts_with("urn:") {
                        // A CLASS the parse itself contains — the consequent shares a subterm with
                        // the antecedent instead of naming it in a string.
                        json!({ "ctor": "ConstRef", "args": [a] })
                    } else {
                        json!({ "ctor": "LitString", "args": [a] })
                    }
                })
                .collect(),
        );
        // `P → C`: an Arrow encodes as a `Pi` with an empty binder name (D47 §3).
        let implication = json!({ "ctor": "Pi", "args": ["", p, c] });
        // Re-borrow out of the literal so both halves stay byte-identical to what went in.
        let (p, c) = (&implication["args"][1], &implication["args"][2]);

        let bridge_iri = iri(&format!("{}:bridge", source.stem))?;
        let mut bridge = Resource::new(bridge_iri.clone());
        bridge.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(wk::DECLARED_RESOURCE)?)]),
        );
        bridge.set(
            iri(wk::CANONICAL_PROPOSITION)?,
            Value::Json(implication.clone()),
        );
        bridge.set(
            iri(REFLECTION_DECLARED_BY)?,
            Value::String(self.declared_by.to_string()),
        );
        bridge.set(
            iri(REFLECTION_RATIONALE)?,
            Value::String(self.rationale.to_string()),
        );

        // `_trace`, not `-trace`: an IRI's local name becomes an ESL identifier when the
        // resource is written as source, and a hyphen is not one. Minting an IRI here that
        // `eigenius decompile` cannot express would put chain content beyond the reach of
        // the source language.
        let trace_iri = iri(&format!("{}:bridge_trace", source.stem))?;
        let mut trace = Resource::new(trace_iri);
        trace.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(wk::DECLARATION_TRACE)?)]),
        );
        trace.set(
            iri(wk::REFLECTION_RESOURCE)?,
            Value::ResourceRef(bridge_iri.clone()),
        );
        trace.set(
            iri(REFLECTION_DECLARED_BY)?,
            Value::String(self.declared_by.to_string()),
        );
        trace.set(
            iri(REFLECTION_TIMESTAMP)?,
            Value::String(self.timestamp.to_string()),
        );

        let certificate = app_spine(
            json!({ "ctor": "CtorApp", "args": [JUSTIFIED_BY, "app"] }),
            vec![
                p.clone(),
                c.clone(),
                justification_term("DeclaredEvidence", bridge_iri.as_str()),
                justification_term("DerivedEvidence", self.claim_iri),
                grounding("declared", bridge_iri.as_str(), implication.clone()),
                grounding("derived", self.claim_iri, p.clone()),
            ],
        );

        let sentence_iri = iri(&format!("{}:sentence", source.stem))?;
        let mut sentence = Resource::new(sentence_iri.clone());
        sentence.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(REASONING_SENTENCE_CLASS)?)]),
        );
        sentence.set(iri(iris::PROP_PROPOSITION)?, Value::Json(c.clone()));
        sentence.set(
            iri(iris::PROP_JUSTIFICATION)?,
            Value::Json(json!({
                "ctor": "App",
                "args": [
                    { "ctor": "DeclaredEvidence", "args": [bridge_iri.as_str()] },
                    { "ctor": "DerivedEvidence", "args": [self.claim_iri] },
                ],
            })),
        );
        sentence.set(iri(iris::PROP_CERTIFICATE)?, Value::Json(certificate));

        Ok(GradedClaim {
            resources: vec![bridge, trace, sentence],
            sentence_iri,
            // The bridge is Declared however strong the claim's own witness is: the conclusion is no
            // better than the weakest link, and the lift is the weak one.
            grade: Grade::Declared,
        })
    }
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

/// A `JustificationTerm` constructor as it appears INSIDE a D47 certificate: `CtorApp` + `App`, the
/// generic rendering of any inductive-ctor application. (On the `reasoning:justification` property it
/// instead takes the bare D32 §3.7 tagged-dict form — two different encodings of the same term.)
fn justification_term(ctor: &str, iri: &str) -> serde_json::Value {
    app_spine(
        json!({ "ctor": "CtorApp", "args": [JUSTIFICATION_TERM, ctor] }),
        vec![json!({ "ctor": "LitString", "args": [iri] })],
    )
}

/// Every class / axiom / individual IRI the term references, as a set.
fn referenced_iris(e: &Exp) -> std::collections::BTreeSet<String> {
    fn walk(e: &Exp, out: &mut std::collections::BTreeSet<String>) {
        match e {
            Exp::EigonClass(i) | Exp::EigonAxiom(i) => {
                out.insert(i.as_str().to_string());
            }
            Exp::EigonResource(r) => {
                if let Some(i) = r.id() {
                    out.insert(i.as_str().to_string());
                }
            }
            Exp::App(a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) | Exp::Pair(a, b) => {
                walk(a, out);
                walk(b, out);
            }
            Exp::Pi(_, a, b) | Exp::Sig(_, a, b) | Exp::Ann(a, b) => {
                walk(a, out);
                walk(b, out);
            }
            Exp::Lam(_, b) | Exp::Fst(b) | Exp::Snd(b) => walk(b, out),
            Exp::InductiveType(d, args) | Exp::InductiveCtor(d, _, args) => {
                out.insert(d.iri.as_str().to_string());
                for a in args {
                    walk(a, out);
                }
            }
            _ => {}
        }
    }
    let mut out = std::collections::BTreeSet::new();
    walk(e, &mut out);
    out
}

fn app_spine(head: serde_json::Value, args: Vec<serde_json::Value>) -> serde_json::Value {
    args.into_iter()
        .fold(head, |acc, a| json!({ "ctor": "App", "args": [acc, a] }))
}

// ═══════════════════════════════════════════════════════════════════════════════════════════════
// Shape rules — ONE declared rule serving every sentence of the same parse shape
// ═══════════════════════════════════════════════════════════════════════════════════════════════

/// A **shape rule**: the parsed proposition with its argument classes abstracted into `Set`-bound
/// variables, implying the domain predicate at those same variables.
///
/// ```text
/// ∀ (v0 : Set) (v1 : Set). <parse shape>(v0, v1) → Pred(v0, v1)
/// ```
///
/// [`BridgedClaimGrader`] declares one ground implication *per sentence*; this declares one per
/// parse **shape**. The shape is obtained by ABSTRACTING a real parse — never hand-written — for
/// two reasons: the ESL surface has no syntax for the Σ-binders and projections a DCG term contains,
/// and instantiating the abstraction must reproduce the original term exactly, or the
/// `derived(claim, ·)` grounding stops matching the parser's sha256 witness key.
#[derive(Debug)]
pub struct ShapeRule {
    /// `∀ v… : Set. shape(v…) → predicate(v…)`, D47-encoded — the rule's `canonical_proposition`.
    pub proposition: Value,
    /// The bound variable names, in order.
    pub binders: Vec<String>,
}

/// Build the rule from one sentence's parse plus the classes that fill its argument slots.
///
/// Every occurrence of `classes[i]` in `proposition` becomes `Var(v{i})`. An argument class that
/// does not occur is an error — the same fail-closed rule [`BridgedClaimGrader`] applies, and here
/// it additionally guarantees the abstraction is non-vacuous.
pub fn build_shape_rule(
    proposition: &Exp,
    predicate: &str,
    classes: &[String],
) -> Result<ShapeRule, GradeError> {
    let mentioned = referenced_iris(proposition);
    for c in classes {
        if !mentioned.contains(c.as_str()) {
            return Err(GradeError::ArgumentNotInProposition {
                argument: c.clone(),
                predicate: predicate.to_string(),
            });
        }
    }
    let binders: Vec<String> = (0..classes.len()).map(|i| format!("v{i}")).collect();
    let mut abstracted = proposition.clone();
    for (i, c) in classes.iter().enumerate() {
        abstracted = abstract_class(&abstracted, c, &binders[i]);
    }
    let Value::Json(shape) =
        encode_type(&abstracted).map_err(|e| GradeError::Encode(format!("{e:?}")))?
    else {
        return Err(GradeError::Encode("not Value::Json".to_string()));
    };
    let consequent = app_spine(
        json!({ "ctor": "ConstRef", "args": [predicate] }),
        binders
            .iter()
            .map(|b| json!({ "ctor": "Var", "args": [b] }))
            .collect(),
    );
    // Innermost: the implication. Then wrap in `∀ (v_i : Set)` outermost-first.
    let mut body = json!({ "ctor": "Pi", "args": ["", shape, consequent] });
    for b in binders.iter().rev() {
        body = json!({ "ctor": "Pi", "args": [b, { "ctor": "Sort", "args": [1] }, body] });
    }
    Ok(ShapeRule {
        proposition: Value::Json(body),
        binders,
    })
}

/// Replace every occurrence of the class `iri` with the free variable `var`.
fn abstract_class(e: &Exp, iri: &str, var: &str) -> Exp {
    let go = |x: &Exp| abstract_class(x, iri, var);
    match e {
        Exp::EigonClass(i) if i.as_str() == iri => Exp::Var(var.to_string()),
        Exp::EigonAxiom(i) if i.as_str() == iri => Exp::Var(var.to_string()),
        Exp::App(a, b) => Exp::App(Box::new(go(a)), Box::new(go(b))),
        Exp::Arrow(a, b) => Exp::Arrow(Box::new(go(a)), Box::new(go(b))),
        Exp::Times(a, b) => Exp::Times(Box::new(go(a)), Box::new(go(b))),
        Exp::Pair(a, b) => Exp::Pair(Box::new(go(a)), Box::new(go(b))),
        Exp::Pi(p, a, b) => Exp::Pi(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Sig(p, a, b) => Exp::Sig(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Ann(a, b) => Exp::Ann(Box::new(go(a)), Box::new(go(b))),
        Exp::Lam(p, b) => Exp::Lam(p.clone(), Box::new(go(b))),
        Exp::Fst(b) => Exp::Fst(Box::new(go(b))),
        Exp::Snd(b) => Exp::Snd(Box::new(go(b))),
        Exp::InductiveType(d, args) => {
            Exp::InductiveType(d.clone(), args.iter().map(&go).collect())
        }
        Exp::InductiveCtor(d, n, args) => {
            Exp::InductiveCtor(d.clone(), n.clone(), args.iter().map(&go).collect())
        }
        other => other.clone(),
    }
}

/// Cite a [`ShapeRule`] for one sentence: instantiate it at that sentence's classes, then apply it
/// to the parser's `IsDerivedAs` witness.
///
/// Produces only the `ReasoningSentence` — the rule's `DeclaredResource` + `DeclarationTrace` are
/// committed **once**, by [`shape_rule_resources`], and shared by every sentence of the shape.
/// That sharing is the whole point: authoring cost becomes one rule per parse shape rather than one
/// bridge per sentence.
pub struct ShapeRuleCitation<'a> {
    /// IRI of the committed shape rule (the `DeclaredResource` carrying the ∀-implication).
    pub rule_iri: &'a str,
    /// IRI of the `enc:EncodedClaim` whose `IsDerivedAs` witness carries this sentence's `P`.
    pub claim_iri: &'a str,
    /// The rule's binder names, in order (from [`ShapeRule::binders`]).
    pub binders: &'a [String],
    /// This sentence's classes, in the same order — what the binders instantiate to.
    pub classes: &'a [String],
    /// The domain predicate the rule concludes.
    pub predicate: &'a str,
}

impl ClaimGrader for ShapeRuleCitation<'_> {
    fn grade(&self, proposition: &Exp, source: &ClaimSource) -> Result<GradedClaim, GradeError> {
        let iri = |s: &str| Iri::parse(s).map_err(|e| GradeError::Iri(format!("{s}: {e:?}")));
        let Value::Json(p) =
            encode_type(proposition).map_err(|e| GradeError::Encode(format!("{e:?}")))?
        else {
            return Err(GradeError::Encode("not Value::Json".to_string()));
        };
        // The abstraction, rebuilt so each partially-applied motive can be formed below.
        let mut abstracted = proposition.clone();
        for (i, c) in self.classes.iter().enumerate() {
            abstracted = abstract_class(&abstracted, c, &self.binders[i]);
        }
        let Value::Json(shape) =
            encode_type(&abstracted).map_err(|e| GradeError::Encode(format!("{e:?}")))?
        else {
            return Err(GradeError::Encode("not Value::Json".to_string()));
        };
        let cons_at = |args: Vec<serde_json::Value>| {
            app_spine(
                json!({ "ctor": "ConstRef", "args": [self.predicate] }),
                args,
            )
        };
        let var = |b: &str| json!({ "ctor": "Var", "args": [b] });
        let cls = |c: &str| json!({ "ctor": "ConstRef", "args": [c] });

        let n = self.binders.len();
        // The consequent at a mix of already-instantiated classes and still-bound variables.
        let mixed = |upto: usize| -> Vec<serde_json::Value> {
            (0..n)
                .map(|k| {
                    if k < upto {
                        cls(&self.classes[k])
                    } else {
                        var(&self.binders[k])
                    }
                })
                .collect()
        };
        // The rule body with binders `[0, subst)` replaced by their classes and binders
        // `[bind, n)` still universally quantified. The two uses differ by exactly one binder:
        //
        //   rule proposition   body(0, 0)      — every binder quantified
        //   motive at k        body(k, k+1)    — v_k left FREE for the enclosing `Lam` to bind
        //   result at k        body(k+1, k+1)  — v_k now substituted
        //
        // Conflating them re-binds v_k inside its own motive, and the certificate then claims a
        // proposition with one binder too many — which the gate reports as an index mismatch
        // against the rule's witness.
        let body = |subst: usize, bind: usize| -> serde_json::Value {
            let mut sh = shape.clone();
            for k in 0..subst {
                sh = substitute_var(&sh, &self.binders[k], &cls(&self.classes[k]));
            }
            let mut b = json!({ "ctor": "Pi", "args": ["", sh, cons_at(mixed(subst))] });
            for k in (bind..n).rev() {
                b = json!({ "ctor": "Pi", "args": [self.binders[k], { "ctor": "Sort", "args": [1] }, b] });
            }
            b
        };

        // Start from the rule's own witness, then eliminate one ∀ per binder.
        let mut jterm = jterm_ctor("DeclaredEvidence", self.rule_iri);
        let mut proof = grounding("declared", self.rule_iri, body(0, 0));
        for k in 0..n {
            // Motive `λ v_k : Set. <body with binders 0..k substituted, v_k still free>`.
            let motive = json!({
                "ctor": "Lam",
                "args": [self.binders[k], { "ctor": "Sort", "args": [1] }, body(k, k + 1)]
            });
            let tag = format!("{}#{}", self.rule_iri, self.classes[k]);
            proof = app_spine(
                json!({ "ctor": "CtorApp", "args": [JUSTIFIED_BY, "spec_poly"] }),
                vec![
                    json!({ "ctor": "Sort", "args": [1] }), // T := Set
                    motive,                                 // P
                    jterm.clone(),                          // j
                    cls(&self.classes[k]),                  // x := this sentence's class
                    json!({ "ctor": "LitString", "args": [tag] }),
                    proof,
                ],
            );
            jterm = app_spine(
                jterm_ctor_head("SpecStr"),
                vec![jterm, json!({ "ctor": "LitString", "args": [tag] })],
            );
        }

        let c = cons_at(self.classes.iter().map(|c| cls(c)).collect());
        let implication = json!({ "ctor": "Pi", "args": ["", p.clone(), c.clone()] });
        let certificate = app_spine(
            json!({ "ctor": "CtorApp", "args": [JUSTIFIED_BY, "app"] }),
            vec![
                p.clone(),
                c.clone(),
                jterm.clone(),
                jterm_ctor("DerivedEvidence", self.claim_iri),
                proof,
                grounding("derived", self.claim_iri, p),
            ],
        );
        let _ = implication;

        let sentence_iri = iri(&format!("{}:sentence", source.stem))?;
        let mut sentence = Resource::new(sentence_iri.clone());
        sentence.set(
            iri(wk::IS_A)?,
            Value::Array(vec![Value::ResourceRef(iri(REASONING_SENTENCE_CLASS)?)]),
        );
        sentence.set(iri(iris::PROP_PROPOSITION)?, Value::Json(c));
        sentence.set(
            iri(iris::PROP_JUSTIFICATION)?,
            Value::Json(jterm_value(&jterm_spec_chain(
                self.rule_iri,
                self.classes,
                self.claim_iri,
            ))),
        );
        sentence.set(iri(iris::PROP_CERTIFICATE)?, Value::Json(certificate));
        Ok(GradedClaim {
            resources: vec![sentence],
            sentence_iri,
            grade: Grade::Declared,
        })
    }
}

/// The rule's own 2-resource cluster — declared ONCE, cited by every sentence of the shape.
pub fn shape_rule_resources(
    rule_iri: &str,
    rule: &ShapeRule,
    declared_by: &str,
    rationale: &str,
    timestamp: &str,
) -> Result<Vec<Resource>, GradeError> {
    let iri = |s: &str| Iri::parse(s).map_err(|e| GradeError::Iri(format!("{s}: {e:?}")));
    let mut r = Resource::new(iri(rule_iri)?);
    r.set(
        iri(wk::IS_A)?,
        Value::Array(vec![Value::ResourceRef(iri(wk::DECLARED_RESOURCE)?)]),
    );
    r.set(iri(wk::CANONICAL_PROPOSITION)?, rule.proposition.clone());
    r.set(
        iri(REFLECTION_DECLARED_BY)?,
        Value::String(declared_by.into()),
    );
    r.set(iri(REFLECTION_RATIONALE)?, Value::String(rationale.into()));

    // `_trace`, not `-trace` — see the note in `justified_by_declared_certificate`.
    let mut t = Resource::new(iri(&format!("{rule_iri}_trace"))?);
    t.set(
        iri(wk::IS_A)?,
        Value::Array(vec![Value::ResourceRef(iri(wk::DECLARATION_TRACE)?)]),
    );
    t.set(
        iri(wk::REFLECTION_RESOURCE)?,
        Value::ResourceRef(iri(rule_iri)?),
    );
    t.set(
        iri(REFLECTION_DECLARED_BY)?,
        Value::String(declared_by.into()),
    );
    t.set(iri(REFLECTION_TIMESTAMP)?, Value::String(timestamp.into()));
    Ok(vec![r, t])
}

/// `SpecStr(…SpecStr(DeclaredEvidence(rule), t0)…, tn)` applied to `DerivedEvidence(claim)`, in the
/// D32 §3.7 tagged-dict form the `reasoning:justification` property takes.
fn jterm_spec_chain(rule_iri: &str, classes: &[String], claim_iri: &str) -> serde_json::Value {
    let mut j = json!({ "ctor": "DeclaredEvidence", "args": [rule_iri] });
    for c in classes {
        j = json!({ "ctor": "SpecStr", "args": [j, format!("{rule_iri}#{c}")] });
    }
    json!({ "ctor": "App", "args": [j, { "ctor": "DerivedEvidence", "args": [claim_iri] }] })
}

fn jterm_value(v: &serde_json::Value) -> serde_json::Value {
    v.clone()
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

/// Substitute a free variable in an already-encoded D47 tree.
fn substitute_var(
    tree: &serde_json::Value,
    var: &str,
    to: &serde_json::Value,
) -> serde_json::Value {
    match tree {
        serde_json::Value::Object(o) => {
            if o.get("ctor").and_then(|c| c.as_str()) == Some("Var")
                && o.get("args")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    == Some(&json!(var))
            {
                return to.clone();
            }
            let args: Vec<_> = o
                .get("args")
                .and_then(|a| a.as_array())
                .map(|a| a.iter().map(|x| substitute_var(x, var, to)).collect())
                .unwrap_or_default();
            json!({ "ctor": o.get("ctor").cloned().unwrap_or(json!(null)), "args": args })
        }
        other => other.clone(),
    }
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
/// human declares anything. Contrast [`BridgedClaimGrader`] and [`ShapeRule`], where the implication
/// is asserted by a person because it crosses from lexicon vocabulary into a domain ontology.
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
