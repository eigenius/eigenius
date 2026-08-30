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

//! Projections of a retained `justification:Term` — D73 §1.2, eigenius#204.
//!
//! **Why this is in the kernel, beside `witness/` rather than inside `nbe/`.** The two
//! modules answer different questions and stay apart: `witness/` answers *does the chain
//! admit this ground* — keys, hashes, α-canonicalisation; `justification/` answers *what
//! does this term rest on*. `nbe/` is the wrong home for either, because this is a reading
//! of one particular inductive, not type theory.
//!
//! It moved here from `crates/eigenius-reasoning/src/project.rs` because the kernel needs
//! the ALGEBRA, not the edge set. The well-foundedness check is stated over a term's
//! support, and support reads `Sum` disjunctively: `Sum(a, b)` is carried by either branch
//! alone, so a cycle through `a` while `b` is acyclic leaves the conclusion well-founded.
//! `core:mentions` records both branches' edges undifferentiated, so a cycle walk over the
//! edge set would reject that commit — a FALSE REJECTION, which loses data, where a false
//! admit is caught by the next check. No predicate filter recovers the distinction, because
//! it is not in the edges at all.
//!
//! The `do_project_justification` dispatch wrapper did NOT move: it is institution surface,
//! and stays in `crates/eigenius-reasoning` until P7 deletes it with the rest.
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

//! There is no specialization row. `spec_poly` used to build `SpecStr(j, tag)`, whose support was
//! `support(j)` — specialization changes the proposition, not the grounds. Now that the rule leaves
//! the term at `j`, that identity holds by there being nothing to project.
//!
//! **`Sum` being disjunctive is the thing to get right**, and it is exactly what D39 §8's
//! propagation rule got wrong. A conclusion is fully verified if SOME spanning selection is, not if
//! every leaf is: a claim resting on `Sum(Verified(a), Declared(b))` is verified,
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

pub mod wellfounded;

use crate::nbe::term::Exp;

/// Ceiling on the number of alternative support sets. Exceeding it is an ERROR, never a silent
/// truncation: every projection here is read as exhaustive ("these are the agents we trust", "no
/// alternative survives losing X"), and a quietly truncated support set would make each of them
/// lie in the safe-looking direction.
pub const MAX_SUPPORT_SETS: usize = 4096;

/// The three grounding families, as they appear at a `justification:Term` leaf.
///
/// A `Derived` variant read the `DerivedEvidence(iri)` constructor until the three-grounds change.
/// A computed claim now grounds as `App(Declared(plan), Observed(inputs))`, so it projects to TWO
/// leaves in different families rather than one opaque leaf naming a program output — which is the
/// point: `leaves_of(term, Ground::Observed)` returns the sample set, and `survives_without` on
/// that sample set answers false. Both answered wrongly before, in the reassuring direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ground {
    Declared,
    Observed,
    Verified,
}

impl Ground {
    fn from_ctor(name: &str) -> Option<Self> {
        match name {
            "Declared" => Some(Ground::Declared),
            "Observed" => Some(Ground::Observed),
            "Verified" => Some(Ground::Verified),
            _ => None,
        }
    }

    /// The constructor name, for diagnostics and for rendering a projection back to the chain.
    pub fn ctor_name(self) -> &'static str {
        match self {
            Ground::Declared => "Declared",
            Ground::Observed => "Observed",
            Ground::Verified => "Verified",
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
    /// A constructor outside the five `justification:Term` forms.
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
            Self::UnknownCtor(c) => write!(f, "`{c}` is not a justification:Term constructor"),
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

/// Is every ground of SOME alternative `Verified`?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::InductiveDecl;
    use crate::ontology::iri::Iri;
    use std::sync::Arc;

    fn decl() -> Arc<InductiveDecl> {
        Arc::new(InductiveDecl {
            uparams: Vec::new(),
            iri: Iri::parse("urn:eigenius:justification:Term").unwrap(),
            name: "justification:Term".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::sort(1),
            ctors: Vec::new(),
        })
    }

    fn leaf(ctor: &str, iri: &str) -> Exp {
        Exp::InductiveCtor(
            decl().iri.clone(),
            ctor.to_string(),
            vec![Exp::LitString(iri.to_string())],
        )
    }
    fn app(a: Exp, b: Exp) -> Exp {
        Exp::InductiveCtor(decl().iri.clone(), "App".to_string(), vec![a, b])
    }
    fn sum(a: Exp, b: Exp) -> Exp {
        Exp::InductiveCtor(decl().iri.clone(), "Sum".to_string(), vec![a, b])
    }

    #[test]
    fn a_grounding_leaf_is_its_own_support() {
        let s = support(&leaf("Observed", "urn:m1")).expect("projects");
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
        let t = app(leaf("Declared", "urn:rule"), leaf("Observed", "urn:m1"));
        let s = support(&t).expect("projects");
        assert_eq!(s.len(), 1, "App yields one alternative");
        assert_eq!(s[0].len(), 2, "and it needs both grounds");
    }

    #[test]
    fn sum_is_disjunctive_two_alternatives_each_carrying_alone() {
        // THE case D39 §8's propagation rule got wrong. `Sum` packages two independent grounds for
        // the SAME proposition; either carries it.
        let t = sum(
            leaf("Verified", "urn:proof"),
            leaf("Declared", "urn:assumed"),
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
            leaf("Verified", "urn:proof"),
            leaf("Declared", "urn:assumed"),
        );
        assert!(!is_fully_verified(&t).expect("projects"));
    }

    #[test]
    fn specialization_is_no_longer_a_term_at_all() {
        // `spec_poly` used to build `SpecStr(j, tag)` and `support` passed the grounds through it,
        // because instantiating a universal changes the PROPOSITION and not what it rests on. The
        // rule now leaves the term at `j`, so that pass-through is the identity on nothing —
        // and a term still carrying the old constructor is refused rather than silently projected.
        assert_eq!(
            support(&Exp::InductiveCtor(
                decl().iri.clone(),
                "SpecStr".to_string(),
                vec![leaf("Declared", "urn:rule"), Exp::LitString("urn:x".into())],
            )),
            Err(ProjectError::UnknownCtor("SpecStr".to_string()))
        );
    }

    #[test]
    fn a_computed_ground_projects_to_its_plan_and_its_input() {
        // The three-grounds shape, and P4's exit criterion. A statistics-derived claim used to be
        // one `DerivedEvidence(<program>:result)` leaf: `leaves_of(term, Observed)` returned
        // NOTHING, and `survives_without(<dataset>)` returned TRUE — the conclusion "survived"
        // losing the only data it was computed from, because the dataset appeared nowhere in the
        // term. Both answers were wrong in the reassuring direction.
        let t = app(
            leaf(
                "Declared",
                "urn:eigenius:pub:wrn:wrn_dep_plan_yields_effect",
            ),
            leaf("Observed", "urn:eigenius:pub:wrn:wrn_dep_sampleset"),
        );
        assert_eq!(
            leaves_of(&t, Ground::Observed)
                .expect("projects")
                .iter()
                .map(|l| l.iri.as_str())
                .collect::<Vec<_>>(),
            vec!["urn:eigenius:pub:wrn:wrn_dep_sampleset"],
            "the sample set is a ground, so the projection names it"
        );
        assert!(
            !survives_without(&t, "urn:eigenius:pub:wrn:wrn_dep_sampleset").expect("projects"),
            "losing the data the claim was computed from must not leave it standing"
        );
        assert!(
            !survives_without(&t, "urn:eigenius:pub:wrn:wrn_dep_plan_yields_effect")
                .expect("projects"),
            "nor does losing the declaration that the plan computes what it claims to"
        );
    }

    #[test]
    fn the_counterfactual_distinguishes_a_fallback_from_a_dependency() {
        // The argument for retaining the polynomial. A stored scalar cannot answer either of these.
        let m1 = || leaf("Observed", "urn:instrument_x");

        // Under Sum, instrument X has an alternative: losing it costs nothing.
        let with_fallback = sum(m1(), leaf("Observed", "urn:instrument_y"));
        assert!(
            survives_without(&with_fallback, "urn:instrument_x").expect("projects"),
            "the y branch carries the conclusion without x"
        );

        // Under App it does not: every alternative cites x.
        let load_bearing = app(leaf("Declared", "urn:rule"), m1());
        assert!(
            !survives_without(&load_bearing, "urn:instrument_x").expect("projects"),
            "no alternative avoids x"
        );
    }

    #[test]
    fn app_over_sum_distributes_into_both_alternatives() {
        // The multiplying case: `App(rule, Sum(a, b))` gives {rule,a} and {rule,b}.
        let t = app(
            leaf("Declared", "urn:rule"),
            sum(leaf("Observed", "urn:a"), leaf("Observed", "urn:b")),
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
            leaf("Declared", "urn:agent_rule"),
            sum(leaf("Observed", "urn:m1"), leaf("Declared", "urn:assumed")),
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
            support(&Exp::InductiveCtor(
                decl().iri.clone(),
                "Nope".into(),
                vec![]
            )),
            Err(ProjectError::UnknownCtor("Nope".into()))
        );
        assert_eq!(
            support(&Exp::InductiveCtor(
                decl().iri.clone(),
                "App".into(),
                vec![]
            )),
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
        let mut t = sum(leaf("Observed", "urn:a0"), leaf("Observed", "urn:b0"));
        for i in 1..14 {
            t = app(
                t,
                sum(
                    leaf("Observed", &format!("urn:a{i}")),
                    leaf("Observed", &format!("urn:b{i}")),
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
