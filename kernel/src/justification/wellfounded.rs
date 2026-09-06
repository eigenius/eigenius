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

//! Well-foundedness: a premise's support may not transitively include the premise.
//!
//! A justification violating this is rejected at commit. The paper calls it a
//! well-formedness condition on justification terms, structurally the same as the
//! positivity check on an inductive declaration.
//!
//! ## Why this reads TERMS and not the `core:mentions` index
//!
//! The paper proposes cycle detection over `core:mentions`, and the index really does
//! carry a term's premise citations. It is nonetheless the wrong instrument, for two
//! independent reasons — and only the first is a filtering problem.
//!
//! A resource's proposition and its justification both land as `core:inductive`, so
//! both contribute edges from the same subject and the index records no predicate to
//! tell them apart. A cycle walk over raw edges would report cycles that are not
//! justification cycles. That much a filter could fix.
//!
//! **`Sum` cannot be fixed by filtering.** The condition is stated over a term's
//! SUPPORT — its disjunctive normal form — and support reads `Sum` disjunctively:
//! `Sum(a, b)` is carried by either branch alone, so a cycle through `a` while `b` is
//! acyclic leaves the conclusion well-founded. `core:mentions` records both branches'
//! edges undifferentiated, so an edge-set walk rejects that commit. That is a FALSE
//! REJECTION, which destroys data, where a false admit is caught by the next check.
//! Reference edges cannot distinguish a conjunctive `App` from a disjunctive `Sum`,
//! and no predicate filter recovers the distinction, because it is not in the edges.
//!
//! So the condition is evaluated over decoded terms, through [`support`].
//!
//! ## The carve-out is required, not convenient
//!
//! The condition is **vacuous on a premise with no support to inspect** — a
//! `justification:Claim` under a `prov:DeclarationTrace` has none: its bridge rests on
//! institutional trust rather than a derived proposition. This is not leniency.
//! Artemov's constant specifications permit self-referential axioms `c : A(c)`, and
//! that self-referentiality is strictly necessary for realizing certain S4 theorems in
//! LP. Postulated self-reference is sound; DERIVED circularity is not, and only derived
//! circularity has a support graph to inspect.
//!
//! ## Why it is transitive
//!
//! A later declaration can retroactively upgrade a bare observation into an
//! application, which creates a backward edge in the derivation graph. A single-step
//! structural check cannot see a cycle formed by two such upgrades in different layers,
//! so the check evaluates the transitive closure.

use std::collections::{BTreeMap, BTreeSet};

use crate::justification::{support, ProjectError};
use crate::layer::Layer;
use crate::ontology::iri::Iri;

/// `justification:Conclusion` — the class whose instances carry a term to inspect.
const CONCLUSION: &str = "urn:eigenius:justification:Conclusion";
/// `justification:judgement` — `holds(kernel, c, Certificate(j, P))`.
const JUDGEMENT: &str = "urn:eigenius:justification:judgement";

/// Why a conclusion is not well-founded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WellFoundedError {
    /// Every alternative in the conclusion's support passes through the conclusion
    /// itself. `path` is one witnessing cycle, in citation order.
    Circular { conclusion: Iri, path: Vec<Iri> },
    /// The conclusion's judgement or its certificate term could not be read. Reported
    /// rather than silently treated as well-founded: a term this pass cannot decode is
    /// a term it cannot vouch for.
    Unreadable { conclusion: Iri, reason: String },
}

impl std::fmt::Display for WellFoundedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Circular { conclusion, path } => write!(
                f,
                "`{conclusion}` is not well-founded: every alternative in its support \
                 passes back through it — {}. A premise's support may not transitively \
                 include the premise. If the cited premise is meant to be an assumption \
                 rather than a derivation, declare it: a Declared premise has no support \
                 to inspect and the condition is vacuous on it.",
                path.iter()
                    .map(|i| i.as_str())
                    .collect::<Vec<_>>()
                    .join(" → ")
            ),
            Self::Unreadable { conclusion, reason } => {
                write!(
                    f,
                    "`{conclusion}`'s justification could not be read: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for WellFoundedError {}

/// Is `iri`'s justification well-founded?
///
/// Vacuously true for anything that is not a conclusion carrying a judgement — see the
/// carve-out in the module docs.
pub fn check(layer: &Layer, iri: &Iri) -> Result<(), WellFoundedError> {
    let mut memo: BTreeMap<Iri, bool> = BTreeMap::new();
    let mut path: Vec<Iri> = Vec::new();
    let mut touched_grey = false;
    if well_founded(layer, iri, &mut path, &mut memo, &mut touched_grey)? {
        return Ok(());
    }
    Err(WellFoundedError::Circular {
        conclusion: iri.clone(),
        path: cycle_path(layer, iri),
    })
}

/// The term a conclusion's judgement certifies, or `None` when this resource is not a
/// conclusion carrying one — in which case the condition is vacuous on it.
fn term_of(layer: &Layer, iri: &Iri) -> Option<Result<crate::nbe::term::Exp, String>> {
    let r = layer.resolve(iri)?;
    if !r.is_a().iter().any(|c| c.as_str() == CONCLUSION) {
        return None;
    }
    let stored = r.get(&Iri::parse(JUDGEMENT).ok()?)?;
    let judgement = match crate::program::eigentt_type_mirror::decode_judgement(stored, layer) {
        Ok(j) => j,
        Err(e) => return Some(Err(format!("judgement does not decode: {e}"))),
    };
    // The certificate itself, not an index of its type. It used to come from
    // `certificate_indices(&judgement.typ).0` — the term the type carried alongside the
    // proposition — and with that index merged into the certificate (D88 §2) the derivation is
    // the judgement's own `term`: `holds(kernel, c, Certificate(P))`.
    match crate::program::eigentt_type_mirror::certificate_indices(&judgement.typ) {
        Some(_) => Some(Ok(judgement.term.clone())),
        None => Some(Err(
            "judgement's type is not a `justification:Certificate(P)`".to_string(),
        )),
    }
}

/// Least fixed point of `wf(C) = ∃ alternative. ∀ leaf. wf(leaf)`.
///
/// `true` is memoized unconditionally: a conclusion with a finite derivation has one
/// regardless of how it was reached. `false` is memoized ONLY when it was established
/// without consulting a node on the current path — a `false` that depended on a grey
/// node is provisional, and caching it would make a node's verdict depend on which
/// branch happened to be explored first. `touched_grey` carries that up.
fn well_founded(
    layer: &Layer,
    iri: &Iri,
    path: &mut Vec<Iri>,
    memo: &mut BTreeMap<Iri, bool>,
    touched_grey: &mut bool,
) -> Result<bool, WellFoundedError> {
    if path.iter().any(|p| p == iri) {
        *touched_grey = true;
        return Ok(false);
    }
    if let Some(&cached) = memo.get(iri) {
        return Ok(cached);
    }

    let term = match term_of(layer, iri) {
        // Not a conclusion, or carries no judgement: no support to inspect.
        None => return Ok(true),
        Some(Err(reason)) => {
            return Err(WellFoundedError::Unreadable {
                conclusion: iri.clone(),
                reason,
            })
        }
        Some(Ok(t)) => t,
    };

    let alternatives = support(&term).map_err(|e: ProjectError| WellFoundedError::Unreadable {
        conclusion: iri.clone(),
        reason: e.to_string(),
    })?;

    path.push(iri.clone());
    let mut any_alternative_holds = false;
    let mut grey_below = false;
    'alts: for alt in &alternatives {
        for leaf in alt {
            let Ok(leaf_iri) = Iri::parse(&leaf.iri) else {
                // A leaf whose argument is not an IRI cites nothing this pass can
                // follow; `support` already refused a malformed leaf by shape.
                continue;
            };
            let mut leaf_grey = false;
            if !well_founded(layer, &leaf_iri, path, memo, &mut leaf_grey)? {
                grey_below |= leaf_grey;
                continue 'alts;
            }
            grey_below |= leaf_grey;
        }
        any_alternative_holds = true;
        break;
    }
    path.pop();

    if any_alternative_holds {
        memo.insert(iri.clone(), true);
    } else if !grey_below {
        memo.insert(iri.clone(), false);
    }
    *touched_grey |= grey_below;
    Ok(any_alternative_holds)
}

/// One witnessing cycle, for the diagnostic. Walks the first leaf of the first
/// alternative at each step, which is the path a reader will check first.
fn cycle_path(layer: &Layer, start: &Iri) -> Vec<Iri> {
    let mut seen: BTreeSet<Iri> = BTreeSet::new();
    let mut out = vec![start.clone()];
    let mut cur = start.clone();
    while seen.insert(cur.clone()) {
        let Some(Ok(term)) = term_of(layer, &cur) else {
            break;
        };
        let Ok(alts) = support(&term) else { break };
        let Some(next) = alts
            .iter()
            .flatten()
            .filter_map(|l| Iri::parse(&l.iri).ok())
            .find(|i| term_of(layer, i).is_some())
        else {
            break;
        };
        out.push(next.clone());
        if &next == start {
            break;
        }
        cur = next;
    }
    out
}
