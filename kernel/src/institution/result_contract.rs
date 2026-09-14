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

//! D90 — the institution result contract, checked at the boundary.
//!
//! An institution's declared contract was an INPUT class. `marshal.rs` checks arity
//! and property shape on the way in; nothing checked the way out. `result_class` was
//! declared on every QueryClass and read by nothing, and `dispatch.rs` carried a
//! comment claiming a check that did not exist. What covered the live statistics path
//! was incidental: the Verdict is committed, so layer validation runs Rule 21 over it,
//! and Rule 21 fires where a property's `class_types` names `eigentt:Term` or
//! `eigentt:Judgement`. A property declared with any other range carried a term-shaped
//! value past every type-level check (eigenius#226).
//!
//! Three checks run here, before the dispatch is recorded.
//!
//! 1. **The output contract is closed.** Every property the institution set on its gate
//!    output must be declared by the QueryClass's `result_class` (transitively over
//!    `subclass_of`) or listed in its `institution:result_properties`. This is what
//!    turns the declaration from a label into a contract: an institution-invented
//!    property is absent from it, not merely unranged.
//!
//!    **Why `result_properties` and not a `Verdict` subclass per institution.** Rule 25
//!    closes an inductive: a class may name one in `subclass_of` only from that
//!    inductive's own layer, and `institution:Verdict` is an inductive. So no later
//!    layer can subclass it, and the subclass reading of a closed `result_class` is not
//!    available. Two QueryClasses of one institution also legitimately return different
//!    property sets, which a class per institution would not express either.
//!
//! 2. **A term-bearing slot is declared in one of the two forms.** A value whose
//!    `is_a` names a constructor class of `eigentt:Term` or `eigentt:Judgement` may
//!    only sit on a property declared `class_types [eigentt:Term]` WITH an
//!    `eigentt:expected_type` (the proposition form), or `class_types
//!    [eigentt:Judgement]` (the judgement form). Both are then checked by Rule 21 at
//!    commit; neither is checked without the declaration.
//!
//! 3. **The verdict constructor is permitted.** A QueryClass may declare
//!    `institution:permitted_verdicts`. Where it does, a verdict outside that set is
//!    a contract violation — the case that motivated it is a governance institution
//!    that must return `Holds` or `Undecidable` and never `Fails`, because below
//!    threshold means *do not commit*, not *this chain is invalid*.
//!
//! **Where each check applies.** The closed-class check is about the GATE output,
//! because `result_class` is what the QueryClass declares it returns. The term-slot
//! check runs over the gate output AND over every emitted derivation, because a
//! derivation is output too and the hole is the same there.
//!
//! **`core:json` is deliberately not covered.** A JSON blob is data; nothing reads one
//! as a term unless a declaration says to. Check 1 is what stops an institution calling
//! such a blob its epistemic output.

use std::sync::Arc;

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

/// The `urn:` of the `eigentt:Term` inductive — the range marker designating a
/// property value as a D47-encoded EigenTT tree. Same constant Rule 21 selects on.
const TERM_IRI: &str = "urn:eigenius:eigentt:Term";
/// The `urn:` of the `eigentt:Judgement` inductive — the other form.
const JUDGEMENT_IRI: &str = "urn:eigenius:eigentt:Judgement";

/// `institution:permitted_verdicts` — the verdict constructors a QueryClass may
/// return. Absent means all three, which is the pre-D90 behaviour.
pub const PERMITTED_VERDICTS_PROP: &str = "urn:eigenius:institution:permitted_verdicts";

/// `institution:result_properties` — what this QueryClass's verdict may carry beyond
/// what its `result_class` declares.
pub const RESULT_PROPERTIES_PROP: &str = "urn:eigenius:institution:result_properties";

/// Whether a `permitted_verdicts` entry names this constructor.
///
/// The entries are constructor CLASSES (`institution:Verdict-Holds`), because that is
/// what D85 materialises and what `allows_only` can range over; a verdict names its
/// constructor. Compared FORWARDS, by building the class IRI the constructor would
/// have, rather than by stripping the suffix off the entry: `ctor_classes::class_iri`
/// is the one authority for the `-` scheme, and string-stripping is a second,
/// looser implementation of its inverse that would also match
/// `urn:eigenius:anything-Holds`.
fn names_ctor(class_iri: &Iri, ctor: &str) -> bool {
    class_iri.as_str() == crate::layer::ctor_classes::class_iri(wk::VERDICT, ctor)
}

/// One way an institution's output failed its declared contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractViolation {
    /// The QueryClass's `result_class` does not resolve. Rule 22 §b reports the dangling
    /// reference where the QueryClass was committed — NOT Rule 14, which returns early
    /// unless the resource is a `core:Class` or a `core:Property` and so never looks at a
    /// QueryClass instance. This says the dispatch could not be checked against it.
    ResultClassUnresolved { result_class: Iri },
    /// An AutoOnLoad QueryClass declared a `result_class` other than
    /// `institution:Verdict`. D14 §4.4 has always said it must be one; nothing checked,
    /// while `build_verdict_resource` hard-stamps `is_a: [Verdict]` regardless. So the
    /// closed-property check would have run against a class the committed resource is
    /// not an instance of, and an institution could widen its own contract by naming a
    /// wider class.
    ResultClassNotVerdict { result_class: Iri },
    /// The institution set a property the result class does not declare.
    UndeclaredProperty { property: Iri },
    /// A term-shaped value sits on a slot declared as neither form.
    TermOnUndeclaredSlot {
        property: Iri,
        /// `"eigentt:Term"` or `"eigentt:Judgement"` — what the value turned out to be.
        found: &'static str,
    },
    /// A `class_types [eigentt:Term]` slot with no `eigentt:expected_type`. Rule 21
    /// would infer rather than check, so the slot states no contract.
    TermSlotWithoutExpectedType { property: Iri },
    /// The verdict constructor is outside the QueryClass's `permitted_verdicts`.
    VerdictNotPermitted {
        ctor: String,
        permitted: Vec<String>,
    },
}

impl std::fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ResultClassUnresolved { result_class } => {
                write!(f, "declared result_class `{result_class}` does not resolve")
            }
            Self::ResultClassNotVerdict { result_class } => write!(
                f,
                "declared result_class `{result_class}`, but an AutoOnLoad QueryClass \
                 returns a Verdict and the kernel commits one regardless"
            ),
            Self::UndeclaredProperty { property } => write!(
                f,
                "returned property `{property}`, which the declared result class does not declare"
            ),
            Self::TermOnUndeclaredSlot { property, found } => write!(
                f,
                "returned a `{found}` value on `{property}`, which declares neither \
                 `class_types [eigentt:Term]` nor `class_types [eigentt:Judgement]`"
            ),
            Self::TermSlotWithoutExpectedType { property } => write!(
                f,
                "`{property}` declares `class_types [eigentt:Term]` with no \
                 `eigentt:expected_type`, so its value is inferred rather than checked"
            ),
            Self::VerdictNotPermitted { ctor, permitted } => write!(
                f,
                "returned verdict `{ctor}`, outside the declared permitted set [{}]",
                permitted.join(", ")
            ),
        }
    }
}

/// Check a verdict constructor against the QueryClass's `permitted_verdicts`.
///
/// Split out because it is the ONE check that also applies on the Decidable path.
/// There, the institution's output never reaches the chain — `decide_institution`
/// reads the constructor to reduce a `NativeDecide` and discards everything else — so
/// the closed-property and term-slot checks have nothing to protect. The permitted set
/// still has to hold, or the same declaration would mean one thing on commit and
/// another during type-check reduction.
pub fn check_permitted_verdict(permitted: &[Iri], verdict_ctor: &str) -> Option<ContractViolation> {
    if permitted.is_empty() || permitted.iter().any(|c| names_ctor(c, verdict_ctor)) {
        return None;
    }
    Some(ContractViolation::VerdictNotPermitted {
        ctor: verdict_ctor.to_string(),
        permitted: permitted.iter().map(|i| i.as_str().to_string()).collect(),
    })
}

/// Check one dispatch's output against the QueryClass's declared contract.
///
/// `result_class`, `result_properties` and `permitted` are the QueryClass's
/// declarations. `permitted` is empty where it declares no restriction, which permits
/// all three constructors and is the pre-D90 behaviour. `verdict_ctor` is the
/// constructor name already parsed off the output.
pub fn check_output(
    output: &Resource,
    derivations: &[Resource],
    result_class: &Iri,
    result_properties: &[Iri],
    permitted: &[Iri],
    verdict_ctor: &str,
    layer: &Arc<Layer>,
) -> Vec<ContractViolation> {
    let mut out = Vec::new();

    if layer.resolve(result_class).is_none() {
        out.push(ContractViolation::ResultClassUnresolved {
            result_class: result_class.clone(),
        });
        return out;
    }

    // D14 §4.4 — an AutoOnLoad QueryClass returns a Verdict, and this is the only
    // caller. `build_verdict_resource` stamps `is_a: [Verdict]` whatever was declared,
    // so without this the closed check below would run against a class the committed
    // resource is not an instance of.
    if result_class.as_str() != wk::VERDICT {
        out.push(ContractViolation::ResultClassNotVerdict {
            result_class: result_class.clone(),
        });
        return out;
    }

    out.extend(check_permitted_verdict(permitted, verdict_ctor));

    let mut declared = layer.declared_properties(result_class);
    declared.extend(result_properties.iter().cloned());
    for (prop_iri, value) in output.properties() {
        if gate_output_is_kernel_stamped(prop_iri.as_str()) {
            continue;
        }
        if !declared.contains(prop_iri) {
            out.push(ContractViolation::UndeclaredProperty {
                property: prop_iri.clone(),
            });
            // An undeclared property has no declaration to check the value against,
            // so the term check below would report a second violation for one cause.
            continue;
        }
        out.extend(check_term_slot(prop_iri, value, layer));
    }

    for derivation in derivations {
        for (prop_iri, value) in derivation.properties() {
            if derivation_is_kernel_stamped(prop_iri.as_str()) {
                continue;
            }
            out.extend(check_term_slot(prop_iri, value, layer));
        }
    }

    out
}

/// Properties the kernel sets on the GATE OUTPUT after this check, so the contract does
/// not have to declare them.
///
/// Delegates to the merge's own drop set rather than keeping a copy. A hand-kept copy
/// drifts WIDER, and a wider exemption is a hole: every entry here escapes both the
/// closed check and the term check, and `institution:from_subject` —
/// `core:resource` with no `class_types`, the one shape that admits a term at the layer
/// boundary — was exempted here while the merge copies it through verbatim.
fn gate_output_is_kernel_stamped(prop: &str) -> bool {
    crate::institution::dispatch::protected_verdict_properties().contains(prop)
}

/// Properties `finalize_emitted_resource` stamps on a DERIVATION unconditionally.
///
/// `prov:was_generated_by` and `institution:runtime_invocation` are deliberately absent:
/// that function writes them only when the dispatch carries an activity or an
/// invocation, and an in-process dispatch carries neither, so exempting them would let
/// an institution set them itself and escape the check.
fn derivation_is_kernel_stamped(prop: &str) -> bool {
    matches!(prop, wk::IS_A | wk::FROM_SUBJECT)
}

/// Check one property's value, and everything nested under it, against the declared
/// forms.
///
/// **It descends.** The check used to look at the top-level value only, which left a
/// term one level down inside a declared, `class_types`-ranged embedded value reaching
/// the chain unchecked — the closed contract listed the OUTER key, and nothing listed
/// the inner one. Descending closes that, and the recursion terminates naturally at the
/// right place: once a value IS a term, the walk stops rather than descending into its
/// constructor arguments, which are nested embedded resources that Rule 21 deliberately
/// exempts (`is_constructor_argument`) because they are open terms in a binder scope.
fn check_term_slot(prop_iri: &Iri, value: &Value, layer: &Arc<Layer>) -> Vec<ContractViolation> {
    match value {
        // A term: this is the slot that has to declare it. Do not descend — the
        // constructor arguments below belong to the term, not to the resource.
        v if term_shape_of(v, layer).is_some() => check_declared_form(
            prop_iri,
            term_shape_of(v, layer).expect("just matched"),
            layer,
        ),
        Value::Array(items) => items
            .iter()
            .flat_map(|v| check_term_slot(prop_iri, v, layer))
            .collect(),
        // An ordinary embedded resource. Its properties are slots in their own right,
        // each checked against its own declaration.
        Value::Embedded(r) => r
            .properties()
            .iter()
            .flat_map(|(inner_iri, inner)| check_term_slot(inner_iri, inner, layer))
            .collect(),
        _ => Vec::new(),
    }
}

/// Is this property declared in one of the forms that CHECKS a term rather than
/// inferring one?
///
/// Three, matching Rule 21's own cases rather than a subset of them:
///
/// - `class_types [eigentt:Judgement]` — decode both halves, check the type is a type,
///   check the term against it.
/// - `class_types [eigentt:Term]` + `eigentt:expected_type` — Rule 21 case 1, check the
///   value AGAINST that type.
/// - `class_types [eigentt:Term]` + `eigentt:is_a_type` — Rule 21 case 2, the value must
///   itself be a type (`check_type`). Strictly stronger than inference, and refusing it
///   would have refused `eigentt:axiom_statement`, `eigentt:definition_type`,
///   `core:ctor_type` and `lexicon:sem_type` — every slot whose inhabited sorts vary
///   within the slot, which is why case 2 exists.
///
/// What stays refused is Rule 21's case 3: a `class_types [eigentt:Term]` slot with
/// NEITHER, where the value is self-describing and inference alone is what runs. An
/// institution's epistemic output should not rest on inference agreeing with intent.
fn check_declared_form(
    prop_iri: &Iri,
    found: &'static str,
    layer: &Arc<Layer>,
) -> Vec<ContractViolation> {
    let Some(prop_def) = layer.resolve(prop_iri) else {
        // Rule 22 §c reports an undeclared property key where the resource commits.
        return vec![ContractViolation::TermOnUndeclaredSlot {
            property: prop_iri.clone(),
            found,
        }];
    };
    let ranges = prop_def
        .get(&wk::iri(wk::CLASS_TYPES))
        .map(|v| v.as_iri_array())
        .unwrap_or_default();
    let ranged_on = |s: &str| Iri::parse(s).is_ok_and(|t| ranges.contains(&t));

    if ranged_on(JUDGEMENT_IRI) {
        return Vec::new();
    }
    if ranged_on(TERM_IRI) {
        let checks = prop_def.get(&wk::iri(wk::EXPECTED_TYPE)).is_some()
            || matches!(
                prop_def.get(&wk::iri(wk::IS_A_TYPE)),
                Some(Value::Boolean(true))
            );
        if !checks {
            return vec![ContractViolation::TermSlotWithoutExpectedType {
                property: prop_iri.clone(),
            }];
        }
        return Vec::new();
    }
    vec![ContractViolation::TermOnUndeclaredSlot {
        property: prop_iri.clone(),
        found,
    }]
}

/// Whether a value is a D85 §6.1 inductive value of `eigentt:Term` or
/// `eigentt:Judgement` — an embedded resource whose `is_a` names a constructor class
/// whose `subclass_of` is that inductive. Descends arrays, because an array slot
/// carries the same risk one element at a time.
///
/// A `Value::Json` blob is NOT a term however term-shaped it looks: nothing reads a
/// blob as a term unless a declaration says to, and no declaration does.
fn term_shape_of(value: &Value, layer: &Arc<Layer>) -> Option<&'static str> {
    match value {
        Value::Embedded(r) => r.is_a().into_iter().find_map(|c| inductive_of(&c, layer)),
        Value::Array(items) => items.iter().find_map(|v| term_shape_of(v, layer)),
        _ => None,
    }
}

/// The term inductive a constructor class belongs to, if it is one of the two.
fn inductive_of(class_iri: &Iri, layer: &Arc<Layer>) -> Option<&'static str> {
    let def = layer.resolve(class_iri)?;
    let parents = def.get(&wk::iri(wk::PARENT_CLASSES))?.as_iri_array();
    parents.iter().find_map(|p| match p.as_str() {
        TERM_IRI => Some("eigentt:Term"),
        JUDGEMENT_IRI => Some("eigentt:Judgement"),
        _ => None,
    })
}
