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

//! `KappaTauInstitution` — the chain-facing half.
//!
//! Reads a `kt:CommitmentAssessment`, calls [`crate::scoring`], and emits the gate
//! verdict plus one `kt:CommitmentDecision`. Stateless: every dispatch re-reads its
//! inputs, so a re-run over the same assessment produces the same decision at the
//! same IRI and the chain's append-only discipline collapses it to a no-op.
//!
//! **What it returns, and why only two constructors.** `Holds` where the conclusion
//! commits, `Undecidable` where it is suspended, and never `Fails`. The framework
//! holds veto power and must not exercise it: below threshold means *do not commit to
//! φ*, not *this chain is invalid*, and `Fails` would reject the commit. The
//! QueryClass declares that restriction and D90's boundary check enforces it, so a
//! future edit that starts returning `Fails` is refused rather than silently
//! rejecting chains.

use std::collections::BTreeMap;
use std::sync::Arc;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::{Institution, QueryOutcome};
use eigenius_kernel::layer::Layer;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::{
    certificate_indices, decode_judgement, encode_type, CodecNames,
};

use crate::iris;
use crate::scoring::{decide, Decision, KappaTable, Policy};

/// In-process κ–τ institution.
pub struct KappaTauInstitution {
    iri: Iri,
}

impl KappaTauInstitution {
    pub fn new() -> Self {
        Self {
            iri: Iri::parse(iris::INSTITUTION).expect("static institution IRI"),
        }
    }

    pub fn arc() -> Arc<dyn Institution> {
        Arc::new(Self::new())
    }
}

impl Default for KappaTauInstitution {
    fn default() -> Self {
        Self::new()
    }
}

impl Institution for KappaTauInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        _resource: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "KappaTauInstitution has no extract_typed handler for `{procedure_iri}` \
             (it declares no ExportFormat)"
        )))
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        _value: &Val,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "KappaTauInstitution has no reify handler for `{procedure_iri}` \
             (it declares no ImportFormat)"
        )))
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        match procedure_iri.as_str() {
            iris::PROC_ASSESS_COMMITMENT => assess_commitment(input, ctx),
            _ => Err(InstitutionError::NotImplemented(format!(
                "KappaTauInstitution has no query handler for procedure `{procedure_iri}`"
            ))),
        }
    }
}

/// Everything one assessment names, read off the chain.
struct Inputs {
    subject_iri: String,
    /// φ — the subject conclusion's proposition, projected out of its grounds
    /// judgement rather than restated on the assessment, so the two cannot drift.
    subject_proposition: Exp,
    policy_iri: String,
    policy: Policy,
    margin: Margin,
    rivals: Vec<String>,
    scores: BTreeMap<String, f64>,
    kappa: KappaTable,
}

/// Run one assessment.
///
/// **A structural failure is not a suspension.** A missing reference, a margin form
/// this institution does not implement, an active rival nobody weighed — none of these
/// is a governance state, and returning `Undecidable` for them would publish a verdict
/// a reader could not tell apart from `SuspendedAt`. They are the institution refusing
/// to RUN, so they return an error, which the dispatch records as a validation error
/// naming the QueryClass and which rejects the commit. The other kind of "no" — the
/// institution running and declining to commit — is the `Undecidable` verdict with a
/// decision resource beside it saying which rival bound it.
///
/// Rule 1 already requires every property on the assessment, so what reaches here
/// malformed is a reference that resolves to the wrong thing or a number nobody
/// supplied, not a missing slot.
fn assess_commitment(
    input: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    let layer = ctx.head();
    let refused = |stage: &str, e: String| {
        InstitutionError::InvalidInput(format!(
            "the assessment could not be adjudicated ({stage}): {e}"
        ))
    };

    let inputs = read_inputs(input, layer).map_err(|e| refused("reading the assessment", e))?;

    let margin = |tau: f64, x: f64| Ok(inputs.margin.at(tau, x));
    let decision = decide(
        &inputs.subject_iri,
        &inputs.rivals,
        &inputs.scores,
        &inputs.kappa,
        &inputs.policy,
        &margin,
    )
    .map_err(|e| refused("scoring", e))?;

    let names = CodecNames::from_layer(layer);
    let committed = decision.commits();
    let proposition =
        encode_claim(&inputs, committed, &names).map_err(|e| refused("encoding the claim", e))?;

    let derivation = input
        .id()
        .map(|assessment_iri| decision_resource(assessment_iri, &decision, proposition, committed));

    Ok(QueryOutcome {
        output: gate_verdict(if committed {
            wk::VERDICT_HOLDS
        } else {
            wk::VERDICT_UNDECIDABLE
        }),
        derivations: derivation.into_iter().collect(),
        partial_invocation: None,
    })
}

// ── Reading the chain ────────────────────────────────────────────────

fn read_inputs(input: &Resource, layer: &Arc<Layer>) -> Result<Inputs, String> {
    let subject = ref_of(input, iris::PROP_SUBJECT, layer)
        .ok_or("assessment names no resolvable kt:subject")?;
    let subject_iri = subject
        .id()
        .ok_or("the subject has no @id, so nothing can cite the decision")?
        .as_str()
        .to_string();
    let subject_proposition = proposition_of(&subject, layer)?;

    let policy_res = ref_of(input, iris::PROP_POLICY, layer)
        .ok_or("assessment names no resolvable kt:policy")?;
    let policy_iri = policy_res
        .id()
        .ok_or("the policy has no @id, so the claim could not name what governed it")?
        .as_str()
        .to_string();
    let policy = Policy {
        threshold: float_of(&policy_res, iris::PROP_THRESHOLD).ok_or("policy has no threshold")?,
        activation: float_of(&policy_res, iris::PROP_ACTIVATION)
            .ok_or("policy has no activation")?,
    };
    // The paper requires ε < τ, so the suspension interval `[ε, τ)` is non-empty. A
    // relation between two slots, which `min_value` cannot express, so it is checked
    // here. At ε ≥ τ no rival is ever active and the decision records NO margins — which
    // reads exactly like the genuine finding that a conclusion faced no live opposition.
    if policy.activation >= policy.threshold {
        return Err(format!(
            "the policy sets ε = {} and τ = {}: the suspension interval [ε, τ) is empty, \
             so no rival could ever be active and every margin record would be missing \
             for a reason the record cannot show",
            policy.activation, policy.threshold
        ));
    }
    let form_iri =
        iri_of(&policy_res, iris::PROP_MARGIN_FORM).ok_or("policy names no kt:margin_form")?;
    let coefficient = float_of(&policy_res, iris::PROP_MARGIN_COEFFICIENT)
        .ok_or("policy has no margin_coefficient")?;
    let margin = Margin::read(&form_iri, coefficient)?;

    let rival_set = ref_of(input, iris::PROP_RIVAL_SET, layer)
        .ok_or("assessment names no resolvable kt:rival_set")?;
    if let Some(declared_subject) = ref_of(&rival_set, iris::PROP_RIVAL_OF, layer) {
        if let Some(id) = declared_subject.id() {
            if id.as_str() != subject_iri {
                // A rival set reconstructed for a different conclusion says nothing
                // about this one, and scoring against it would produce a decision
                // nobody reconstructed.
                return Err(format!(
                    "the rival set was reconstructed for `{}`, not for the subject `{subject_iri}`",
                    id.as_str()
                ));
            }
        }
    }
    let rivals = ref_array_iris(&rival_set, iris::PROP_RIVALS);

    // **A second estimate for the same subject is refused, not overwritten.** Two
    // independently elicited, independently attributed estimates disagreeing is exactly
    // what "each estimate is its own resource with its own trace" produces, and picking
    // one by array position decides a governance question by authoring order. The same
    // rule the design already states for a MISSING weight — the caller's error to
    // report, not a value to assume — applies to a conflicting one.
    let mut scores: BTreeMap<String, f64> = BTreeMap::new();
    for estimate in ref_array(input, iris::PROP_PLAUSIBILITY_ESTIMATES, layer) {
        let of = ref_of(&estimate, iris::PROP_ESTIMATE_OF, layer)
            .ok_or("a plausibility estimate names no resolvable kt:estimate_of")?;
        let id = of
            .id()
            .ok_or("a plausibility estimate scores something with no @id")?
            .as_str()
            .to_string();
        let w = float_of(&estimate, iris::PROP_WEIGHT)
            .ok_or_else(|| format!("the plausibility estimate for `{id}` carries no weight"))?;
        if let Some(existing) = scores.insert(id.clone(), w) {
            if existing != w {
                return Err(format!(
                    "two plausibility estimates for `{id}` disagree ({existing} and {w}); \
                     which one governs is not this institution's to decide"
                ));
            }
        }
    }

    let mut kappa = KappaTable::new();
    for estimate in ref_array(input, iris::PROP_INTERACTION_ESTIMATES, layer) {
        let pair = ref_array_iris(&estimate, iris::PROP_BETWEEN);
        let k = float_of(&estimate, iris::PROP_KAPPA)
            .ok_or("an interaction estimate carries no kappa")?;
        // `kt:between` is declared `min_length 2; max_length 2`, so a malformed pair does
        // not commit. Refusing here as well is not belt-and-braces: dropping the estimate
        // instead — which is what this did — deleted a rivalry silently and flipped a
        // suspension into a commitment, which is the premature convergence the framework
        // exists to prevent.
        let [a, b] = pair.as_slice() else {
            return Err(format!(
                "an interaction estimate holds {} hypotheses, not two; an interaction is \
                 between a pair",
                pair.len()
            ));
        };
        if let Some(existing) = kappa.insert(a, b, k) {
            if existing != k {
                return Err(format!(
                    "two interaction estimates for `{a}` and `{b}` disagree ({existing} \
                     and {k}); which one governs is not this institution's to decide"
                ));
            }
        }
    }

    Ok(Inputs {
        subject_iri,
        subject_proposition,
        policy_iri,
        policy,
        margin,
        rivals,
        scores,
        kappa,
    })
}

/// φ, out of the subject conclusion's `grounds_judgement`.
///
/// The judgement is `holds(kernel, c, Grounds(P))`, so P sits inside its TYPE. Reading
/// it from there rather than from a second slot on the assessment is what keeps the
/// proposition κ–τ scores identical to the one the conclusion concluded.
fn proposition_of(subject: &Resource, layer: &Arc<Layer>) -> Result<Exp, String> {
    let judgement_value = subject
        .get(&Iri::parse(iris::PROP_GROUNDS_JUDGEMENT).expect("static IRI"))
        .ok_or("the subject carries no justification:grounds_judgement")?;
    let judgement = decode_judgement(judgement_value, layer)
        .map_err(|e| format!("the subject's grounds judgement does not decode: {e:?}"))?;
    certificate_indices(&judgement.typ)
        .cloned()
        .ok_or_else(|| "the judgement's type is not a `justification:Grounds(P)`".to_string())
}

/// Resolve one `core:resource`-valued property, whether it arrived embedded (the
/// dispatch path inlines references before the handler runs) or as an IRI string.
fn ref_of(owner: &Resource, prop: &str, layer: &Arc<Layer>) -> Option<Resource> {
    let value = owner.get(&Iri::parse(prop).ok()?)?;
    value_to_resource(value, layer)
}

fn value_to_resource(value: &Value, layer: &Arc<Layer>) -> Option<Resource> {
    match value {
        Value::Embedded(r) => Some((**r).clone()),
        Value::String(s) => {
            let iri = Iri::parse(s).ok()?;
            layer.resolve(&iri).map(|r| (*r).clone())
        }
        _ => None,
    }
}

fn ref_array(owner: &Resource, prop: &str, layer: &Arc<Layer>) -> Vec<Resource> {
    let Some(Value::Array(items)) = owner.get(&match Iri::parse(prop) {
        Ok(i) => i,
        Err(_) => return Vec::new(),
    }) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| value_to_resource(v, layer))
        .collect()
}

/// The IRIs a `core:resource_array` names, whether its entries arrived embedded or as
/// strings. Identity is the IRI, so an entry without one is dropped rather than
/// silently treated as a distinct hypothesis on every run.
fn ref_array_iris(owner: &Resource, prop: &str) -> Vec<String> {
    let Some(Value::Array(items)) = owner.get(&match Iri::parse(prop) {
        Ok(i) => i,
        Err(_) => return Vec::new(),
    }) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Embedded(r) => r.id().map(|i| i.as_str().to_string()),
            _ => None,
        })
        .collect()
}

fn float_of(owner: &Resource, prop: &str) -> Option<f64> {
    match owner.get(&Iri::parse(prop).ok()?)? {
        Value::Float(f) => Some(*f),
        // An author writing `threshold = 1` means 1.0; refusing it would be a lexical
        // complaint rather than a semantic one.
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

// ── The margin function ──────────────────────────────────────────────

/// δ, as the declared family plus its coefficient.
///
/// A named form rather than a term, because the program AST has no arithmetic: it has
/// Apply, Lambda, Var, Literal, Case, Map, Reduce and Component, and every number that
/// gets multiplied is multiplied inside a Component dispatched to a runtime. So a
/// chain-resident `λ τ x. c·τ·x` is not expressible today. Computing δ here is the
/// same decision the rest of the scoring semantics already makes rather than a
/// concession to it.
///
/// Each variant satisfies the paper's constraints for `c ≥ 0`: `(0,1] × (0,1] →
/// [0,1)`, non-decreasing in each argument, and → 0 as the interaction strength → 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Margin {
    /// δ ≡ 0 — first-past-τ, the degenerate policy and the reproduction gate.
    Zero,
    /// δ(τ, x) = c·x.
    ByRivalry(f64),
    /// δ(τ, x) = c·τ·x.
    ByStakesAndRivalry(f64),
}

impl Margin {
    /// Read the declared form.
    ///
    /// The coefficient's range is declared on `kt:margin_coefficient` and enforced at
    /// commit, so a policy outside `[0,1]` does not reach here. The check survives
    /// because this is also callable directly, and because a governance parameter
    /// silently corrected is worse than one refused.
    pub fn read(form_iri: &str, coefficient: f64) -> Result<Self, String> {
        if !(0.0..=1.0).contains(&coefficient) {
            return Err(format!(
                "margin_coefficient is {coefficient}, outside [0,1]: δ must be \
                 non-decreasing in the strength of the rivalry and land in the paper's \
                 codomain, and a negative coefficient reverses the first"
            ));
        }
        match form_iri {
            iris::MARGIN_FORM_ZERO => Ok(Self::Zero),
            iris::MARGIN_FORM_BY_RIVALRY => Ok(Self::ByRivalry(coefficient)),
            iris::MARGIN_FORM_BY_STAKES_AND_RIVALRY => Ok(Self::ByStakesAndRivalry(coefficient)),
            other => Err(format!(
                "`{other}` is not a margin form this institution implements"
            )),
        }
    }

    /// δ(τ, x).
    ///
    /// **Not clamped.** `kt:margin_coefficient` is declared `[0,1]` and τ and the
    /// interaction strength are both in `[0,1]`, so δ ≤ 1 by construction and the
    /// codomain is a property of the declaration rather than of this function. Clamping
    /// instead would record a margin the policy never named — a c of 5 would publish a
    /// demanded margin of 1.0 where the policy asked for 4.0 — and the record of what
    /// was demanded is half of what auditable suspension is for. It would also not
    /// prevent the impossible demand: a clamped 1.0 is unsatisfiable anyway, since a
    /// rival scoring 0 is below any ε > 0 and so is never active.
    pub fn at(&self, tau: f64, interaction_strength: f64) -> f64 {
        match self {
            Self::Zero => 0.0,
            Self::ByRivalry(c) => c * interaction_strength,
            Self::ByStakesAndRivalry(c) => c * tau * interaction_strength,
        }
    }
}

/// One `core:resource`-valued property, as the IRI it names — for a slot whose value
/// is a marker instance rather than a resource to read properties off.
fn iri_of(owner: &Resource, prop: &str) -> Option<String> {
    match owner.get(&Iri::parse(prop).ok()?)? {
        Value::String(s) => Some(s.clone()),
        Value::Embedded(r) => r.id().map(|i| i.as_str().to_string()),
        _ => None,
    }
}

// ── Emitting ─────────────────────────────────────────────────────────

/// `Commits(policy, φ)` or `SuspendedAt(policy, φ)`, encoded as a term value.
///
/// The policy IRI, not a bare τ: the audit trail then runs to an attributed resource
/// carrying τ, ε, δ and λ together. φ is the subject's own proposition, unchanged —
/// this institution never asserts φ, only a proposition ABOUT it, and the gap between
/// them is crossed by a declared bridge or not at all.
fn encode_claim(inputs: &Inputs, committed: bool, names: &CodecNames) -> Result<Value, String> {
    let head = Iri::parse(if committed {
        iris::KT_COMMITS
    } else {
        iris::KT_SUSPENDED_AT
    })
    .map_err(|e| format!("{e:?}"))?;
    let claim = Exp::App(
        Box::new(Exp::App(
            Box::new(Exp::Const(head, Vec::new())),
            Box::new(Exp::LitString(inputs.policy_iri.clone())),
        )),
        Box::new(inputs.subject_proposition.clone()),
    );
    encode_type(&claim, names).map_err(|e| format!("{e:?}"))
}

fn gate_verdict(ctor: &str) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
    );
    r.set(
        Iri::parse(wk::CTOR_NAME).expect("well-known IRI"),
        Value::String(ctor.to_string()),
    );
    r
}

/// The `kt:CommitmentDecision` derivation, at `{assessment_iri}:decision`.
///
/// Deterministic from the assessment, so a re-run lands on the same IRI. The kernel
/// stamps `institution:from_subject` and the class markers; this sets the domain
/// payload.
fn decision_resource(
    assessment_iri: &Iri,
    decision: &Decision,
    proposition: Value,
    _committed: bool,
) -> Resource {
    let iri = Iri::parse(&format!("{}:decision", assessment_iri.as_str()))
        .expect("a decision IRI parses");
    let mut r = Resource::new(iri);
    r.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::String(iris::COMMITMENT_DECISION.to_string())]),
    );
    r.set(
        Iri::parse(iris::PROP_FROM_ASSESSMENT).expect("static IRI"),
        Value::iri(&assessment_iri.clone()),
    );
    r.set(
        Iri::parse(iris::PROP_PROPOSITION).expect("static IRI"),
        proposition,
    );
    r.set(
        Iri::parse(iris::PROP_COMPUTED_SCORE).expect("static IRI"),
        Value::Float(decision.subject_score),
    );
    if !decision.rival_margins.is_empty() {
        r.set(
            Iri::parse(iris::PROP_RIVAL_MARGINS).expect("static IRI"),
            Value::Array(
                decision
                    .rival_margins
                    .iter()
                    .map(|m| {
                        let mut e = Resource::new_embedded();
                        e.set(
                            Iri::parse(wk::IS_A).expect("well-known IRI"),
                            Value::Array(vec![Value::String(iris::RIVAL_MARGIN.to_string())]),
                        );
                        e.set(
                            Iri::parse(iris::PROP_AGAINST).expect("static IRI"),
                            Value::String(m.against.clone()),
                        );
                        e.set(
                            Iri::parse(iris::PROP_RIVAL_SCORE).expect("static IRI"),
                            Value::Float(m.rival_score),
                        );
                        e.set(
                            Iri::parse(iris::PROP_LIFTED_INTERACTION).expect("static IRI"),
                            Value::Float(m.lifted_interaction),
                        );
                        e.set(
                            Iri::parse(iris::PROP_REQUIRED_MARGIN).expect("static IRI"),
                            Value::Float(m.required_margin),
                        );
                        e.set(
                            Iri::parse(iris::PROP_ACHIEVED_SEPARATION).expect("static IRI"),
                            Value::Float(m.achieved_separation),
                        );
                        e.set(
                            Iri::parse(iris::PROP_CLEARED).expect("static IRI"),
                            Value::Boolean(m.cleared),
                        );
                        Value::Embedded(Box::new(e))
                    })
                    .collect(),
            ),
        );
    }
    if let Some(binding) = &decision.binding_rival {
        r.set(
            Iri::parse(iris::PROP_BINDING_RIVAL).expect("static IRI"),
            Value::String(binding.clone()),
        );
    }
    r
}
