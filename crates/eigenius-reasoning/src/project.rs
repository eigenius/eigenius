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

//! Projections of a retained `JustificationTerm` — D73 §1.2, eigenius#204.
//!
//! **This is what justification logic buys over modal epistemic logic.** A stored scalar grade
//! ("this claim is Derived") answers one question and forgets the reasons. The polynomial keeps
//! them, so the chain can be asked things a scalar cannot answer at all — above all the
//! counterfactual *"would this still stand if we lost instrument X?"*
//!
//! D39 §8 specified collapsing the term to a scalar at commit. It was never implemented, and D73
//! withdrew it; this module is the replacement that was also never built, which left the polynomial
//! retained and nothing able to ask anything of it.
//!
//! ## The support algebra
//!
//! Everything here is one function, [`support`], plus readings of its output. A term's **support**
//! is its disjunctive normal form: the set of ALTERNATIVE minimal leaf-sets, any one of which
//! carries the conclusion.
//!
//! | term | support |
//! |---|---|
//! | a grounding leaf `L` | `{{L}}` — one alternative, one leaf |
//! | `App(a, b)` | `{ sa ∪ sb : sa ∈ support(a), sb ∈ support(b) }` — CONJUNCTIVE, both needed |
//! | `Sum(a, b)` | `support(a) ∪ support(b)` — DISJUNCTIVE, either suffices |
//! | `SpecStr(j, tag)` | `support(j)` — specialization changes the proposition, not the grounds |
//!
//! **`Sum` being disjunctive is the thing to get right**, and it is exactly what D39 §8's
//! propagation rule got wrong. A conclusion is fully verified if SOME spanning selection is, not if
//! every leaf is: a claim resting on `Sum(VerifiedEvidence(a), DeclaredEvidence(b))` is verified,
//! because the `a` branch alone carries it. Reading `Sum` conjunctively understates every
//! conclusion that has a fallback, which is precisely the shape a careful author writes.
//!
//! ## Cost, and the cap
//!
//! `App` over `Sum` multiplies, so support is exponential in the number of nested alternatives.
//! Real terms are small — the largest on the WRN chain is four leaves — but the bound is real, so
//! [`support`] refuses past [`MAX_SUPPORT_SETS`] rather than returning a truncated answer that
//! would read as a complete one.

use std::collections::BTreeSet;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::QueryOutcome;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

use crate::institution::iris;

/// Ceiling on the number of alternative support sets. Exceeding it is an ERROR, never a silent
/// truncation: every projection here is read as exhaustive ("these are the agents we trust", "no
/// alternative survives losing X"), and a quietly truncated support set would make each of them
/// lie in the safe-looking direction.
pub const MAX_SUPPORT_SETS: usize = 4096;

/// The four grounding families, as they appear at a `JustificationTerm` leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ground {
    Declared,
    Observed,
    Derived,
    Verified,
}

impl Ground {
    fn from_ctor(name: &str) -> Option<Self> {
        match name {
            "DeclaredEvidence" => Some(Ground::Declared),
            "ObservedEvidence" => Some(Ground::Observed),
            "DerivedEvidence" => Some(Ground::Derived),
            "VerifiedEvidence" => Some(Ground::Verified),
            _ => None,
        }
    }

    /// The constructor name, for diagnostics and for rendering a projection back to the chain.
    pub fn ctor_name(self) -> &'static str {
        match self {
            Ground::Declared => "DeclaredEvidence",
            Ground::Observed => "ObservedEvidence",
            Ground::Derived => "DerivedEvidence",
            Ground::Verified => "VerifiedEvidence",
        }
    }
}

/// One grounding leaf: which family, and the IRI it cites.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Leaf {
    pub ground: Ground,
    pub iri: String,
}

/// Why a term could not be projected.
#[derive(Debug, PartialEq, Eq)]
pub enum ProjectError {
    /// A constructor outside the seven `JustificationTerm` forms.
    UnknownCtor(String),
    /// A grounding constructor whose argument is not a string literal IRI.
    MalformedLeaf(String),
    /// An operator applied to the wrong number of arguments.
    Arity { ctor: String, got: usize },
    /// The term is not a constructor application at all.
    NotATerm,
    /// Support exceeded [`MAX_SUPPORT_SETS`]. Reported rather than truncated.
    TooManyAlternatives(usize),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownCtor(c) => write!(f, "`{c}` is not a JustificationTerm constructor"),
            Self::MalformedLeaf(c) => {
                write!(f, "`{c}`'s argument is not a string literal IRI")
            }
            Self::Arity { ctor, got } => write!(f, "`{ctor}` applied to {got} argument(s)"),
            Self::NotATerm => write!(f, "not a constructor application"),
            Self::TooManyAlternatives(n) => write!(
                f,
                "support has more than {MAX_SUPPORT_SETS} alternatives ({n} and counting); \
                 refusing rather than returning a truncated set"
            ),
        }
    }
}

impl std::error::Error for ProjectError {}

/// A term's support: the alternative minimal leaf-sets, any one of which carries the conclusion.
///
/// See the module docs for the algebra. Order is deterministic (`BTreeSet` inside, and alternatives
/// in left-to-right term order) so a projection is stable across runs and diffable.
pub fn support(term: &Exp) -> Result<Vec<BTreeSet<Leaf>>, ProjectError> {
    let (ctor, args) = match term {
        Exp::InductiveCtor(_, name, args) => (name.as_str(), args.as_slice()),
        _ => return Err(ProjectError::NotATerm),
    };

    if let Some(ground) = Ground::from_ctor(ctor) {
        let iri = match args {
            [Exp::LitString(s)] => s.clone(),
            [_] => return Err(ProjectError::MalformedLeaf(ctor.to_string())),
            other => {
                return Err(ProjectError::Arity {
                    ctor: ctor.to_string(),
                    got: other.len(),
                })
            }
        };
        let mut set = BTreeSet::new();
        set.insert(Leaf { ground, iri });
        return Ok(vec![set]);
    }

    match ctor {
        // CONJUNCTIVE: applying `A -> B` to `A` needs both grounds, so every alternative on the
        // left pairs with every alternative on the right.
        "App" => {
            let (a, b) = two(ctor, args)?;
            let (sa, sb) = (support(a)?, support(b)?);
            let total = sa.len().saturating_mul(sb.len());
            if total > MAX_SUPPORT_SETS {
                return Err(ProjectError::TooManyAlternatives(total));
            }
            let mut out = Vec::with_capacity(total);
            for l in &sa {
                for r in &sb {
                    out.push(l.union(r).cloned().collect());
                }
            }
            Ok(out)
        }
        // DISJUNCTIVE: `Sum` packages two independent grounds for the SAME proposition, so either
        // alternative carries it alone. Reading this conjunctively is D39 §8's error.
        "Sum" => {
            let (a, b) = two(ctor, args)?;
            let (mut sa, sb) = (support(a)?, support(b)?);
            if sa.len() + sb.len() > MAX_SUPPORT_SETS {
                return Err(ProjectError::TooManyAlternatives(sa.len() + sb.len()));
            }
            sa.extend(sb);
            Ok(sa)
        }
        // Specialization instantiates the PROPOSITION; the grounds are the quantified term's.
        "SpecStr" => match args {
            [j, _tag] => support(j),
            other => Err(ProjectError::Arity {
                ctor: ctor.to_string(),
                got: other.len(),
            }),
        },
        other => Err(ProjectError::UnknownCtor(other.to_string())),
    }
}

fn two<'a>(ctor: &str, args: &'a [Exp]) -> Result<(&'a Exp, &'a Exp), ProjectError> {
    match args {
        [a, b] => Ok((a, b)),
        other => Err(ProjectError::Arity {
            ctor: ctor.to_string(),
            got: other.len(),
        }),
    }
}

/// Is every ground of SOME alternative `VerifiedEvidence`?
///
/// The existential is the point: `Sum` is disjunctive, so one fully-verified branch verifies the
/// conclusion even where another branch rests on a declaration.
pub fn is_fully_verified(term: &Exp) -> Result<bool, ProjectError> {
    Ok(support(term)?
        .iter()
        .any(|s| s.iter().all(|l| l.ground == Ground::Verified)))
}

/// The leaves of a given family, across every alternative.
///
/// Union rather than intersection: *"which agents are we trusting"* and *"which measurements"* are
/// questions about exposure, and a ground that appears on any branch is one the conclusion may
/// rest on.
pub fn leaves_of(term: &Exp, ground: Ground) -> Result<BTreeSet<Leaf>, ProjectError> {
    Ok(support(term)?
        .into_iter()
        .flatten()
        .filter(|l| l.ground == ground)
        .collect())
}

/// **The counterfactual — the argument for retaining the polynomial at all.**
///
/// Would the conclusion still stand having lost `iri`? True when SOME alternative cites it nowhere.
/// A stored scalar cannot answer this: the grade records what the grounds came to, not what they
/// were, so removing one leaves nothing to recompute over.
pub fn survives_without(term: &Exp, iri: &str) -> Result<bool, ProjectError> {
    Ok(support(term)?
        .iter()
        .any(|s| !s.iter().any(|l| l.iri == iri)))
}

/// Every IRI the conclusion could rest on, in any alternative — the audit surface.
pub fn cited_iris(term: &Exp) -> Result<BTreeSet<String>, ProjectError> {
    Ok(support(term)?
        .into_iter()
        .flatten()
        .map(|l| l.iri)
        .collect())
}

/// `proc:project_justification` — the OnDemand handler behind `qc_project_justification`.
///
/// Reads the request's `subject_sentence`, resolves it on the chain, extracts its
/// `JustificationTerm`, and reports every slice of the term's support at once. Computing the
/// support is the whole cost; slicing it is free, so there is no projection-kind parameter.
///
/// Returns a `reasoning:JustificationProjection`, not a `Verdict`. This REPORTS what a conclusion
/// rests on; it does not judge it, and it carries no `canonical_proposition` because it asserts
/// nothing.
pub fn do_project_justification(
    input: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    let subject = input
        .get(&Iri::parse(iris::PROP_SUBJECT_SENTENCE).expect("static IRI"))
        .and_then(|v| match v {
            Value::ResourceRef(i) => Some(i.clone()),
            Value::String(s) => Iri::parse(s).ok(),
            _ => None,
        })
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "ProjectionRequest missing required `subject_sentence`".to_string(),
            )
        })?;

    let sentence = ctx.head().resolve(&subject).ok_or_else(|| {
        InstitutionError::ComputationFailed(format!(
            "ProjectionRequest `subject_sentence` `{subject}` does not resolve on the chain"
        ))
    })?;

    // Reuse the ExportFormat path the validate handler uses, then read back the syntactic tree:
    // `support` walks constructor applications, which is what the term IS.
    let term = crate::extract::justification_exp(&sentence, ctx)?;

    let sets = support(&term).map_err(|e| {
        InstitutionError::ComputationFailed(format!("malformed justification: {e}"))
    })?;

    let counterfactual = input
        .get(&Iri::parse(iris::PROP_COUNTERFACTUAL_IRI).expect("static IRI"))
        .and_then(|v| v.as_str().map(str::to_string));

    Ok(QueryOutcome::from_output(projection_resource(
        &subject,
        &sets,
        counterfactual.as_deref(),
    )))
}

/// Build the `reasoning:JustificationProjection` result resource from a computed support.
fn projection_resource(
    subject: &Iri,
    sets: &[BTreeSet<Leaf>],
    counterfactual: Option<&str>,
) -> Resource {
    let iri = |s: &str| Iri::parse(s).expect("static IRI");
    let mut r = Resource::new_embedded();
    r.set(
        iri(eigenius_kernel::ontology::well_known::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            iris::JUSTIFICATION_PROJECTION,
        ))]),
    );
    r.set(
        iri(iris::PROP_SUBJECT_SENTENCE),
        Value::ResourceRef(subject.clone()),
    );
    r.set(
        iri(iris::PROP_SUPPORT_COUNT),
        Value::Integer(sets.len() as i64),
    );
    // Existential over alternatives — Sum is disjunctive.
    r.set(
        iri(iris::PROP_FULLY_VERIFIED),
        Value::Boolean(
            sets.iter()
                .any(|s| s.iter().all(|l| l.ground == Ground::Verified)),
        ),
    );

    // Union across alternatives: these are exposure questions.
    let grounds = |g: Ground| {
        let mut v: Vec<Value> = sets
            .iter()
            .flatten()
            .filter(|l| l.ground == g)
            .map(|l| l.iri.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(Value::String)
            .collect();
        v.shrink_to_fit();
        v
    };
    for (prop, g) in [
        (iris::PROP_DECLARED_GROUNDS, Ground::Declared),
        (iris::PROP_OBSERVED_GROUNDS, Ground::Observed),
        (iris::PROP_DERIVED_GROUNDS, Ground::Derived),
        (iris::PROP_VERIFIED_GROUNDS, Ground::Verified),
    ] {
        let vals = grounds(g);
        if !vals.is_empty() {
            r.set(iri(prop), Value::Array(vals));
        }
    }

    if let Some(x) = counterfactual {
        r.set(
            iri(iris::PROP_COUNTERFACTUAL_IRI),
            Value::String(x.to_string()),
        );
        r.set(
            iri(iris::PROP_SURVIVES_WITHOUT),
            Value::Boolean(sets.iter().any(|s| !s.iter().any(|l| l.iri == x))),
        );
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::nbe::term::InductiveDecl;
    use eigenius_kernel::ontology::iri::Iri;
    use std::sync::Arc;

    fn decl() -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            iri: Iri::parse("urn:eigenius:reasoning:JustificationTerm").unwrap(),
            name: "JustificationTerm".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(1),
            ctors: Vec::new(),
        })
    }

    fn leaf(ctor: &str, iri: &str) -> Exp {
        Exp::InductiveCtor(
            decl(),
            ctor.to_string(),
            vec![Exp::LitString(iri.to_string())],
        )
    }
    fn app(a: Exp, b: Exp) -> Exp {
        Exp::InductiveCtor(decl(), "App".to_string(), vec![a, b])
    }
    fn sum(a: Exp, b: Exp) -> Exp {
        Exp::InductiveCtor(decl(), "Sum".to_string(), vec![a, b])
    }
    fn spec(j: Exp, tag: &str) -> Exp {
        Exp::InductiveCtor(
            decl(),
            "SpecStr".to_string(),
            vec![j, Exp::LitString(tag.to_string())],
        )
    }

    #[test]
    fn a_grounding_leaf_is_its_own_support() {
        let s = support(&leaf("ObservedEvidence", "urn:m1")).expect("projects");
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].iter().next().unwrap(),
            &Leaf {
                ground: Ground::Observed,
                iri: "urn:m1".to_string()
            }
        );
    }

    #[test]
    fn app_is_conjunctive_one_alternative_carrying_both_grounds() {
        // Applying `A -> B` to `A` needs both. One alternative, two leaves.
        let t = app(
            leaf("DeclaredEvidence", "urn:rule"),
            leaf("ObservedEvidence", "urn:m1"),
        );
        let s = support(&t).expect("projects");
        assert_eq!(s.len(), 1, "App yields one alternative");
        assert_eq!(s[0].len(), 2, "and it needs both grounds");
    }

    #[test]
    fn sum_is_disjunctive_two_alternatives_each_carrying_alone() {
        // THE case D39 §8's propagation rule got wrong. `Sum` packages two independent grounds for
        // the SAME proposition; either carries it.
        let t = sum(
            leaf("VerifiedEvidence", "urn:proof"),
            leaf("DeclaredEvidence", "urn:assumed"),
        );
        let s = support(&t).expect("projects");
        assert_eq!(s.len(), 2, "Sum yields two alternatives");
        assert!(s.iter().all(|a| a.len() == 1), "each carries alone");

        // ...so the conclusion IS fully verified, even though a leaf is merely Declared. Reading
        // Sum conjunctively would understate every conclusion that has a fallback.
        assert!(
            is_fully_verified(&t).expect("projects"),
            "one fully-verified branch verifies the conclusion"
        );
    }

    #[test]
    fn a_declared_ground_under_app_blocks_full_verification() {
        // Contrast with the Sum case: under App there is no alternative to fall back to.
        let t = app(
            leaf("VerifiedEvidence", "urn:proof"),
            leaf("DeclaredEvidence", "urn:assumed"),
        );
        assert!(!is_fully_verified(&t).expect("projects"));
    }

    #[test]
    fn specialization_passes_the_grounds_through() {
        // `SpecStr` instantiates the PROPOSITION; the grounds stay the quantified term's.
        let inner = leaf("DeclaredEvidence", "urn:rule");
        assert_eq!(
            support(&spec(inner.clone(), "urn:instance")).expect("projects"),
            support(&inner).expect("projects"),
            "specialization changes what is concluded, not what it rests on"
        );
    }

    #[test]
    fn the_counterfactual_distinguishes_a_fallback_from_a_dependency() {
        // The argument for retaining the polynomial. A stored scalar cannot answer either of these.
        let m1 = || leaf("ObservedEvidence", "urn:instrument_x");

        // Under Sum, instrument X has an alternative: losing it costs nothing.
        let with_fallback = sum(m1(), leaf("ObservedEvidence", "urn:instrument_y"));
        assert!(
            survives_without(&with_fallback, "urn:instrument_x").expect("projects"),
            "the y branch carries the conclusion without x"
        );

        // Under App it does not: every alternative cites x.
        let load_bearing = app(leaf("DeclaredEvidence", "urn:rule"), m1());
        assert!(
            !survives_without(&load_bearing, "urn:instrument_x").expect("projects"),
            "no alternative avoids x"
        );
    }

    #[test]
    fn app_over_sum_distributes_into_both_alternatives() {
        // The multiplying case: `App(rule, Sum(a, b))` gives {rule,a} and {rule,b}.
        let t = app(
            leaf("DeclaredEvidence", "urn:rule"),
            sum(
                leaf("ObservedEvidence", "urn:a"),
                leaf("ObservedEvidence", "urn:b"),
            ),
        );
        let s = support(&t).expect("projects");
        assert_eq!(s.len(), 2);
        assert!(s
            .iter()
            .all(|alt| alt.len() == 2 && alt.iter().any(|l| l.iri == "urn:rule")));
        // The rule is load-bearing on both branches; either measurement is optional.
        assert!(!survives_without(&t, "urn:rule").expect("projects"));
        assert!(survives_without(&t, "urn:a").expect("projects"));
    }

    #[test]
    fn the_audit_projections_read_across_every_alternative() {
        let t = app(
            leaf("DeclaredEvidence", "urn:agent_rule"),
            sum(
                leaf("ObservedEvidence", "urn:m1"),
                leaf("DeclaredEvidence", "urn:assumed"),
            ),
        );
        let declared = leaves_of(&t, Ground::Declared).expect("projects");
        assert_eq!(
            declared.iter().map(|l| l.iri.as_str()).collect::<Vec<_>>(),
            vec!["urn:agent_rule", "urn:assumed"],
            "what it rests on that nobody proved — exposure, so a union over branches"
        );
        assert_eq!(
            leaves_of(&t, Ground::Observed)
                .expect("projects")
                .iter()
                .map(|l| l.iri.as_str())
                .collect::<Vec<_>>(),
            vec!["urn:m1"]
        );
        assert_eq!(cited_iris(&t).expect("projects").len(), 3);
    }

    #[test]
    fn a_malformed_term_is_refused_by_shape() {
        assert_eq!(
            support(&Exp::LitString("nope".into())),
            Err(ProjectError::NotATerm)
        );
        assert_eq!(
            support(&Exp::InductiveCtor(decl(), "Nope".into(), vec![])),
            Err(ProjectError::UnknownCtor("Nope".into()))
        );
        assert_eq!(
            support(&Exp::InductiveCtor(decl(), "App".into(), vec![])),
            Err(ProjectError::Arity {
                ctor: "App".into(),
                got: 0
            })
        );
    }

    #[test]
    fn support_refuses_rather_than_truncating() {
        // Nested Sums under Apps multiply. Every projection here reads as exhaustive, so a
        // truncated support set would make each of them lie in the safe-looking direction.
        let mut t = sum(
            leaf("ObservedEvidence", "urn:a0"),
            leaf("ObservedEvidence", "urn:b0"),
        );
        for i in 1..14 {
            t = app(
                t,
                sum(
                    leaf("ObservedEvidence", &format!("urn:a{i}")),
                    leaf("ObservedEvidence", &format!("urn:b{i}")),
                ),
            );
        }
        // 2^14 = 16384 > MAX_SUPPORT_SETS.
        assert!(matches!(
            support(&t),
            Err(ProjectError::TooManyAlternatives(_))
        ));
    }
}
