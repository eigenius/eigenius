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

//! The κ–τ scoring semantics, as arithmetic over plain data.
//!
//! Separated from the chain entirely: nothing here resolves an IRI or reads a
//! `Resource`, so the semantics can be checked against the paper's own worked
//! examples rather than against a fixture. The institution reads the chain and calls
//! into here.
//!
//! **What this computes, and what that means for the warrant.** Every function below
//! is a deterministic function of its arguments, so an application forms over them.
//! The WEIGHTS and INTERACTIONS they take are estimates recorded under a declared
//! protocol, and no application forms over those. That split is the paper's own
//! provenance boundary and is why a commitment entails nothing about the next
//! estimate.
//!
//! **The kernel does not decide the comparison.** `kt:Commits` is an axiom, as
//! `stats:lt` is, so a threshold comparison here is COMPUTED and asserted rather than
//! proved. That is the same standing the statistics institution's alpha comparison
//! has, and the reason κ–τ emits a proposition rather than a judgement.

use std::collections::BTreeMap;

/// The governed parameters, read off a `kt:CommitmentPolicy`.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// τ — the commitment threshold.
    pub threshold: f64,
    /// ε — the activation floor. The paper requires ε < τ, so the suspension interval
    /// `[ε, τ)` is non-empty; the caller checks that before building this.
    pub activation: f64,
}

/// κ*(φ, ψ) for two ATOMIC hypotheses, which is κ itself.
///
/// **Only the atomic case, deliberately.** The paper lifts κ to compound hypothesis
/// terms by averaging over their atom sets, subject to a structural-equivalence
/// convention: κ* is 0 between terms equivalent under the smallest congruence generated
/// by commutativity of ⊗ — and explicitly NOT under associativity, because differently
/// bracketed terms over the same atoms are genuinely distinct explanatory structures
/// that may legitimately interact.
///
/// That convention is not a function of atom sets, so it cannot be decided from them.
/// Two orderings of the same atoms are equivalent and must give 0; two bracketings of
/// the same atoms are not equivalent and must give the average. A general version needs
/// the TERM, and the chain vocabulary declares no ⊗ constructor to build one from.
///
/// An earlier version took atom slices and compared them for equality, which got both
/// directions wrong while looking general: it returned the average for a commuted pair
/// and 0 for two bracketings. There is no compound case to be general about until the
/// vocabulary has one, and when it does, the lifting and the synthesis operator arrive
/// together with the term structure they both need — along with λ, the
/// interaction-scaling parameter, which is why no policy carries one now.
pub fn lifted_interaction(subject: &str, rival: &str, kappa: &KappaTable) -> f64 {
    kappa.get(subject, rival)
}

/// Pairwise interaction values, symmetric by construction.
///
/// The paper elicits κ on ordered pairs and its diagonal convention is κ(h,h) = 0.
/// Reading it symmetrically matches how rivalry is elicited in practice — "these two
/// readings of the assay are incompatible" is not a directed statement — and means a
/// pilot need not record each pair twice.
#[derive(Debug, Clone, Default)]
pub struct KappaTable {
    values: BTreeMap<(String, String), f64>,
}

impl KappaTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record κ(a, b), returning what it displaced so the caller can refuse a
    /// disagreement rather than resolve it by insertion order.
    ///
    /// A self-pair is ignored and displaces nothing: the diagonal is 0 by convention,
    /// and letting a chain override it would let a hypothesis be its own rival.
    pub fn insert(&mut self, a: &str, b: &str, value: f64) -> Option<f64> {
        if a == b {
            return None;
        }
        let key = if a <= b {
            (a.to_string(), b.to_string())
        } else {
            (b.to_string(), a.to_string())
        };
        self.values.insert(key, value)
    }

    /// κ(a, b), or `0.0` where the pair was not recorded.
    pub fn get(&self, a: &str, b: &str) -> f64 {
        if a == b {
            return 0.0;
        }
        let key = if a <= b {
            (a.to_string(), b.to_string())
        } else {
            (b.to_string(), a.to_string())
        };
        self.values.get(&key).copied().unwrap_or(0.0)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// One rival's contribution to the commitment decision.
#[derive(Debug, Clone, PartialEq)]
pub struct RivalMargin {
    /// The rival hypothesis IRI.
    pub against: String,
    /// sc_S(ψ).
    pub rival_score: f64,
    /// κ*(t_φ, t_ψ), which is negative for every rival that reaches this struct.
    pub lifted_interaction: f64,
    /// δ(τ, −κ*).
    pub required_margin: f64,
    /// sc_S(φ) − sc_S(ψ).
    pub achieved_separation: f64,
    /// Whether the separation met the margin.
    pub cleared: bool,
}

/// What one assessment decided.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// sc_S(φ).
    pub subject_score: f64,
    /// Whether the subject cleared τ at all. A conclusion that fails here is
    /// suspended for want of evidence rather than for rivalry, and its rival list is
    /// still computed so the audit record says which.
    pub above_threshold: bool,
    /// One entry per ACTIVE rival, in IRI order.
    pub rival_margins: Vec<RivalMargin>,
    /// The active rival whose margin failed by the widest gap, where one did.
    pub binding_rival: Option<String>,
}

impl Decision {
    /// Whether the conclusion commits: above τ, and every active rival cleared.
    ///
    /// The two conditions are the paper's Definition of rival-sensitive commitment,
    /// in order. Rival-sensitive commitment strengthens the threshold condition and
    /// never weakens it, so a commitment here implies commit-worthiness there.
    pub fn commits(&self) -> bool {
        self.above_threshold && self.rival_margins.iter().all(|m| m.cleared)
    }
}

/// A margin function δ(τ, −κ*), supplied by the caller.
///
/// The institution evaluates the policy's declared `program:Lambda` through the
/// kernel's own evaluator and hands the result in here, so this module stays free of
/// the chain. The paper's constraints are `(0,1] × (0,1] → [0,1)`, non-decreasing in
/// each argument, with δ(τ, x) → 0 as x → 0.
pub type MarginFn<'a> = &'a dyn Fn(f64, f64) -> Result<f64, String>;

/// Run the commitment condition for one subject against its rivals.
///
/// `scores` must contain the subject and every rival: a rival with no weight is the
/// caller's error to report, not a zero to assume, because scoring an unexamined
/// rival at zero would let it clear every margin silently.
///
/// A rival is ACTIVE when its score reaches ε and its interaction with the subject is
/// strictly negative. Compatible or mildly interacting alternatives impose no margin
/// — coexisting constructive explanations are candidates for synthesis rather than
/// rivals to be outrun — so they produce no record at all.
pub fn decide(
    subject: &str,
    rivals: &[String],
    scores: &BTreeMap<String, f64>,
    kappa: &KappaTable,
    policy: &Policy,
    margin: MarginFn<'_>,
) -> Result<Decision, String> {
    let subject_score = *scores
        .get(subject)
        .ok_or_else(|| format!("no plausibility estimate for the subject `{subject}`"))?;

    let mut margins = Vec::new();
    for rival in rivals {
        if rival == subject {
            continue;
        }
        // **κ first, then the weight.** A non-negatively interacting alternative is not
        // a rival at all — it is a candidate for synthesis — so it can never impose a
        // margin however plausible it is, and demanding a weight for it would refuse a
        // whole assessment over a hypothesis that cannot affect the outcome. Ordering it
        // the other way also made the diagnostic assert what the input denies, calling a
        // compatible alternative an "active rival".
        let lifted = lifted_interaction(subject, rival, kappa);
        if lifted >= 0.0 {
            continue;
        }
        let rival_score = *scores
            .get(rival)
            .ok_or_else(|| format!("no plausibility estimate for the rival `{rival}`"))?;
        if rival_score < policy.activation {
            continue;
        }
        let required = margin(policy.threshold, -lifted)?;
        let achieved = subject_score - rival_score;
        margins.push(RivalMargin {
            against: rival.clone(),
            rival_score,
            lifted_interaction: lifted,
            required_margin: required,
            achieved_separation: achieved,
            cleared: achieved >= required,
        });
    }
    margins.sort_by(|a, b| a.against.cmp(&b.against));

    // The binding rival is the one that failed by the most — the one to resolve
    // first, either by the leader pulling away or by the rival dropping below
    // activation. Ties break on IRI so a re-run names the same rival.
    let binding = margins
        .iter()
        .filter(|m| !m.cleared)
        .max_by(|a, b| {
            let shortfall = |m: &RivalMargin| m.required_margin - m.achieved_separation;
            shortfall(a)
                .partial_cmp(&shortfall(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.against.cmp(&a.against))
        })
        .map(|m| m.against.clone());

    Ok(Decision {
        subject_score,
        above_threshold: subject_score >= policy.threshold,
        rival_margins: margins,
        binding_rival: binding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(threshold: f64, activation: f64) -> Policy {
        Policy {
            threshold,
            activation,
        }
    }

    /// δ ≡ 0 — the degenerate policy, which recovers first-past-τ.
    fn zero_margin(_tau: f64, _x: f64) -> Result<f64, String> {
        Ok(0.0)
    }

    /// A margin function satisfying the paper's constraints: non-decreasing in each
    /// argument, and → 0 as the interaction strength → 0.
    fn linear_margin(coefficient: f64) -> impl Fn(f64, f64) -> Result<f64, String> {
        move |tau: f64, x: f64| Ok(coefficient * tau * x)
    }

    /// The paper's two worked examples for κ*, on the atomic pairs the lifting is
    /// defined over. The compound cases they are stated with need a ⊗ the vocabulary
    /// does not declare; what is checkable here is that each pairwise value the average
    /// is taken over reads back exactly, in either order.
    #[test]
    fn the_papers_worked_interaction_values_read_back_exactly() {
        let mut k = KappaTable::new();
        for (a, b, v) in [
            ("h1", "h3", 0.8),
            ("h1", "h4", 0.6),
            ("h2", "h3", 0.4),
            ("h2", "h4", -0.2),
        ] {
            k.insert(a, b, v);
        }
        // Positive example: despite one negative interaction, the aggregate the paper
        // computes is +0.4 = (0.8 + 0.6 + 0.4 − 0.2) / 4.
        let sum: f64 = [("h1", "h3"), ("h1", "h4"), ("h2", "h3"), ("h2", "h4")]
            .iter()
            .map(|(a, b)| lifted_interaction(a, b, &k))
            .sum();
        assert!((sum / 4.0 - 0.4).abs() < 1e-12);

        let mut k2 = KappaTable::new();
        for (a, b, v) in [
            ("h1", "h3", 0.2),
            ("h1", "h4", -0.7),
            ("h2", "h3", 0.1),
            ("h2", "h4", -0.6),
        ] {
            k2.insert(a, b, v);
        }
        // Negative example: limited positive compatibilities outweighed by systematic
        // tension, −0.25.
        let sum2: f64 = [("h1", "h3"), ("h1", "h4"), ("h2", "h3"), ("h2", "h4")]
            .iter()
            .map(|(a, b)| lifted_interaction(a, b, &k2))
            .sum();
        assert!((sum2 / 4.0 + 0.25).abs() < 1e-12);
    }

    /// A hypothesis does not interact with itself: the paper's diagonal convention.
    #[test]
    fn a_hypothesis_carries_no_interaction_with_itself() {
        let mut k = KappaTable::new();
        k.insert("h1", "h2", -0.9);
        assert_eq!(lifted_interaction("h1", "h1", &k), 0.0);
    }

    #[test]
    fn the_diagonal_is_zero_and_cannot_be_overridden() {
        let mut k = KappaTable::new();
        assert_eq!(
            k.insert("h1", "h1", -0.9),
            None,
            "a self-pair displaces nothing"
        );
        assert_eq!(k.get("h1", "h1"), 0.0);
        assert!(k.is_empty(), "a self-pair is not recorded at all");
    }

    #[test]
    fn interaction_is_read_symmetrically() {
        let mut k = KappaTable::new();
        k.insert("h2", "h1", -0.4);
        assert_eq!(k.get("h1", "h2"), -0.4);
    }

    /// **The reproduction gate, in miniature.** Under δ ≡ 0, a conclusion above τ
    /// commits no matter how close a hostile rival stands — which is first-past-τ,
    /// and is exactly what the pilot must reproduce before any margin is applied.
    #[test]
    fn the_degenerate_policy_commits_despite_a_rival_one_thousandth_behind() {
        let mut k = KappaTable::new();
        k.insert("phi", "psi", -0.9);
        let scores = BTreeMap::from([("phi".to_string(), 0.81), ("psi".to_string(), 0.809)]);
        let d = decide(
            "phi",
            &["psi".to_string()],
            &scores,
            &k,
            &policy(0.8, 0.2),
            &zero_margin,
        )
        .unwrap();
        assert!(d.commits());
        assert_eq!(d.rival_margins.len(), 1, "the rival is still ACTIVE");
        assert!(d.rival_margins[0].cleared);
        assert_eq!(d.binding_rival, None);
    }

    /// The same state under a non-degenerate margin: suspended, and the audit record
    /// says by how much and against whom.
    #[test]
    fn a_non_degenerate_margin_suspends_the_same_conclusion() {
        let mut k = KappaTable::new();
        k.insert("phi", "psi", -0.9);
        let scores = BTreeMap::from([("phi".to_string(), 0.81), ("psi".to_string(), 0.809)]);
        let m = linear_margin(0.5);
        let d = decide(
            "phi",
            &["psi".to_string()],
            &scores,
            &k,
            &policy(0.8, 0.2),
            &m,
        )
        .unwrap();
        assert!(d.above_threshold, "it is still commit-WORTHY");
        assert!(!d.commits(), "but it does not commit");
        assert_eq!(d.binding_rival.as_deref(), Some("psi"));
        let margin = &d.rival_margins[0];
        assert!((margin.required_margin - 0.5 * 0.8 * 0.9).abs() < 1e-12);
        assert!((margin.achieved_separation - 0.001).abs() < 1e-12);
    }

    /// A rival below the activation floor is not in play, so it imposes no margin
    /// however hostile it is.
    #[test]
    fn a_rival_below_activation_imposes_no_margin() {
        let mut k = KappaTable::new();
        k.insert("phi", "psi", -1.0);
        let scores = BTreeMap::from([("phi".to_string(), 0.81), ("psi".to_string(), 0.1)]);
        let d = decide(
            "phi",
            &["psi".to_string()],
            &scores,
            &k,
            &policy(0.8, 0.2),
            &linear_margin(0.5),
        )
        .unwrap();
        assert!(d.rival_margins.is_empty());
        assert!(d.commits());
    }

    /// A compatible alternative is not a rival: it is a candidate for synthesis.
    #[test]
    fn a_non_negatively_interacting_alternative_imposes_no_margin() {
        let mut k = KappaTable::new();
        k.insert("phi", "psi", 0.6);
        let scores = BTreeMap::from([("phi".to_string(), 0.81), ("psi".to_string(), 0.80)]);
        let d = decide(
            "phi",
            &["psi".to_string()],
            &scores,
            &k,
            &policy(0.8, 0.2),
            &linear_margin(0.9),
        )
        .unwrap();
        assert!(d.rival_margins.is_empty());
        assert!(d.commits());
    }

    /// Below τ there is no commitment to adjudicate, and the record still says which
    /// rivals were live so the suspension can be read.
    #[test]
    fn below_threshold_suspends_for_want_of_evidence_not_rivalry() {
        let mut k = KappaTable::new();
        k.insert("phi", "psi", -0.5);
        let scores = BTreeMap::from([("phi".to_string(), 0.4), ("psi".to_string(), 0.3)]);
        let d = decide(
            "phi",
            &["psi".to_string()],
            &scores,
            &k,
            &policy(0.8, 0.2),
            &zero_margin,
        )
        .unwrap();
        assert!(!d.above_threshold);
        assert!(!d.commits());
        assert_eq!(d.rival_margins.len(), 1);
        assert!(d.rival_margins[0].cleared, "the margin itself was met");
        assert_eq!(d.binding_rival, None, "rivalry is not what refused it");
    }

    /// An unexamined rival is an error, not a zero. Scoring it at zero would let it
    /// clear every margin and silently license the commitment.
    #[test]
    fn a_rival_with_no_weight_is_refused_rather_than_scored_zero() {
        let mut k = KappaTable::new();
        k.insert("phi", "psi", -0.5);
        let scores = BTreeMap::from([("phi".to_string(), 0.9)]);
        let err = decide(
            "phi",
            &["psi".to_string()],
            &scores,
            &k,
            &policy(0.8, 0.2),
            &zero_margin,
        )
        .unwrap_err();
        assert!(err.contains("psi"), "the message names the rival: {err}");
    }

    /// The binding rival is the widest shortfall, not the first or the strongest
    /// interaction.
    #[test]
    fn the_binding_rival_is_the_widest_shortfall() {
        let mut k = KappaTable::new();
        k.insert("phi", "near", -0.2);
        k.insert("phi", "far", -1.0);
        let scores = BTreeMap::from([
            ("phi".to_string(), 0.9),
            ("near".to_string(), 0.88),
            ("far".to_string(), 0.85),
        ]);
        let d = decide(
            "phi",
            &["near".to_string(), "far".to_string()],
            &scores,
            &k,
            &policy(0.8, 0.2),
            &linear_margin(1.0),
        )
        .unwrap();
        // near: required 1.0*0.8*0.2 = 0.16, achieved 0.02 → shortfall 0.14
        // far:  required 1.0*0.8*1.0 = 0.80, achieved 0.05 → shortfall 0.75
        assert_eq!(d.binding_rival.as_deref(), Some("far"));
    }
}

#[cfg(test)]
mod margin_tests {
    use crate::institution::Margin;
    use crate::iris;

    /// δ ≡ 0 whatever the coefficient — the reproduction gate names the FORM, so a
    /// policy announces that it is first-past-τ rather than leaving a reader to notice
    /// a zero.
    #[test]
    fn the_zero_form_demands_nothing() {
        let m = Margin::read(iris::MARGIN_FORM_ZERO, 0.9).unwrap();
        assert_eq!(m.at(1.0, 1.0), 0.0);
    }

    /// δ = c·x — grows with the rivalry and ignores the stakes. The family the fixture
    /// does not use, and so the one nothing else exercises.
    #[test]
    fn the_by_rivalry_form_ignores_the_threshold() {
        let m = Margin::read(iris::MARGIN_FORM_BY_RIVALRY, 0.5).unwrap();
        assert!((m.at(0.2, 0.8) - 0.4).abs() < 1e-12);
        assert!((m.at(0.9, 0.8) - 0.4).abs() < 1e-12, "τ does not enter");
    }

    /// δ = c·τ·x — grows with both.
    #[test]
    fn the_by_stakes_and_rivalry_form_uses_both() {
        let m = Margin::read(iris::MARGIN_FORM_BY_STAKES_AND_RIVALRY, 0.5).unwrap();
        assert!((m.at(0.8, 0.8) - 0.32).abs() < 1e-12);
        assert!(m.at(0.9, 0.8) > m.at(0.8, 0.8), "non-decreasing in τ");
        assert!(
            m.at(0.8, 0.9) > m.at(0.8, 0.8),
            "non-decreasing in the rivalry"
        );
    }

    /// **Not clamped.** With the coefficient declared `[0,1]` and both arguments in
    /// `[0,1]`, δ ≤ 1 by construction. Clamping instead would record a margin the policy
    /// never named, and what was demanded is half of what auditable suspension is for.
    #[test]
    fn delta_lands_in_the_codomain_by_construction() {
        let m = Margin::read(iris::MARGIN_FORM_BY_STAKES_AND_RIVALRY, 1.0).unwrap();
        assert_eq!(m.at(1.0, 1.0), 1.0, "the extreme point, not a clamp");
        assert!(m.at(0.99, 0.99) < 1.0);
    }

    /// A coefficient outside `[0,1]` is refused rather than corrected. Negative would
    /// make δ DECREASE with rivalry, inverting the condition; above one would demand a
    /// separation no pair of in-range scores can deliver.
    #[test]
    fn a_coefficient_outside_the_unit_interval_is_refused() {
        for bad in [-0.1, 1.5] {
            let err = Margin::read(iris::MARGIN_FORM_BY_RIVALRY, bad)
                .expect_err("{bad} is outside [0,1]");
            assert!(err.contains("outside [0,1]"), "got: {err}");
        }
    }

    #[test]
    fn an_unimplemented_margin_form_is_refused() {
        let err = Margin::read("urn:eigenius:kappatau:margin_forms:invented", 0.5)
            .expect_err("an undeclared form is refused");
        assert!(err.contains("not a margin form"), "got: {err}");
    }
}
