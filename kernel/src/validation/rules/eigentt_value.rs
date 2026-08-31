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

//! Rule 21 — the single commit-time validator for term-valued slots.
//!
//! One rule, in check mode, for every slot carrying a D47-encoded EigenTT tree.
//! It is the kernel's own annotation rule, not a new one:
//!
//! ```text
//! let checked = match expected_type {
//!     Some(t) => Exp::Ann(value, t),   // "treated as an annotated term"
//!     None    => value,                // self-describing
//! };
//! check_infer(&mut ctx, &checked)
//! ```
//!
//! `check_infer`'s `Ann` arm already does *infer the annotation while requiring
//! a sort, check the value against it in check mode, return it*. Naming a type
//! on the property therefore introduces no typing rule — it only says where the
//! annotation comes from when the stored value does not carry one.
//!
//! Three cases, declared per property:
//!
//! 1. [`wk::EXPECTED_TYPE`] names a type → check the value against it.
//! 2. [`wk::IS_A_TYPE`] is true → the value must itself be a type
//!    (`check_type`), which is the *first step* of the same `Ann` rule applied
//!    to the value rather than to an annotation. Separate from case 1 because
//!    the inhabited sorts vary WITHIN a slot and case 1 holds one term: a
//!    `lexicon:sem_type` is `Set` (in `Sort(2)`) 2,062,659 times and a semantic
//!    -type class (in `Sort(1)`) 281,599 times.
//! 3. Neither → the value is self-describing (it carries its own `Ann`, or is a
//!    rigid reference to a declared axiom / definition / constructor) and
//!    inference alone suffices.
//!
//! **What this replaced.** The rule used to end in `check_infer` and DISCARD the
//! result for every slot except a hardcoded `PROPOSITION_SLOTS` list, where it
//! demanded `Sort(0)`. Discarding the result is the defect: nothing asserted the
//! inferred type was the intended one, so a `justification:Declared(..)` value
//! in a `lexicon:cat` slot inferred cleanly and committed.
//!
//! **The rule does not descend into declaration internals.** `core:param_kind`
//! and `core:type_name` are telescope fragments — open terms in the scope of the
//! binders before them. `core:Option`'s `some(value : A)` stores
//! `Var("A")`, which is well-formed where it lives and reports "unbound variable
//! in type context: A" anywhere else. These occur ONLY as embedded resources
//! under a `core:InductiveType`, carry no `@id`, and are checked by Rule 23
//! against the full binder chain — which is strictly stronger than anything a
//! closed-term check could establish. Skipping them is a structural condition on
//! where the rule applies, not a per-property exemption.
//!
//! The layer is in hand, so cross-layer `ConstRef`s re-resolve at decode time
//! and applications are fully typed.

use super::super::{ValidationError, ValidationRule, Validator};
use crate::nbe::readback::readback_val;
use crate::nbe::term::Exp;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::program::eigentt_type_mirror::decode_type;

/// The `urn:` of the `eigentt:Term` inductive — the range marker designating a
/// property value as a D47-encoded EigenTT tree.
const TERM_IRI: &str = "urn:eigenius:eigentt:Term";
/// The `urn:` of the `eigentt:Judgement` inductive — the other range marker this
/// rule owns. Its values are D47-encoded exactly as `eigentt:Term`'s are, so the
/// generic inductive walk cannot read them.
const JUDGEMENT_IRI: &str = "urn:eigenius:eigentt:Judgement";

impl Validator {
    /// Rule 21 — check every term-valued slot. See the module docs.
    pub(in crate::validation) fn check_type_expr_well_typed(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
        owner: &Resource,
    ) -> Vec<ValidationError> {
        let ranged_on = |iri_str: &str| {
            Iri::parse(iri_str)
                .ok()
                .and_then(|t| {
                    prop_def
                        .get(&wk::iri(wk::CLASS_TYPES))
                        .map(|v| v.as_iri_array().contains(&t))
                })
                .unwrap_or(false)
        };
        let ranged_on_term = ranged_on(TERM_IRI);
        let ranged_on_judgement = ranged_on(JUDGEMENT_IRI);
        if (!ranged_on_term && !ranged_on_judgement) || is_declaration_internal(prop_iri) {
            return vec![];
        }

        let err = |rule, message| {
            vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule,
                message,
            }]
        };

        let mut ctx = crate::nbe::check::CheckCtx::with_layer(
            crate::nbe::env::Rho::Nil,
            Vec::new(),
            std::sync::Arc::clone(&self.layer),
        );

        // A JUDGEMENT slot. `eigentt:Judgement`'s own description states the
        // contract: "A slot ranging over this type is checked in CHECK mode —
        // decode both fields, check `type` is a type, check `term` against it —
        // so no slot relies on inference and no exemption list is needed." That
        // is what happens here.
        //
        // Nothing did it until now. Rule 21 selected only on `eigentt:Term`, so a
        // Judgement slot fell through to Rule 16's generic inductive walk — which
        // reads the D32 tagged-dict form, while a Judgement is stored D47-encoded
        // as an `App` spine. Every judgement on every chain therefore reported
        // "ctor `App` not declared on InductiveType `eigentt:Judgement`", and no
        // test saw it because the chains that carry judgements are built with
        // `LayerBuilder::build`, which does not validate.
        if ranged_on_judgement {
            let j = match crate::program::eigentt_type_mirror::decode_judgement(value, &self.layer)
            {
                Ok(j) => j,
                Err(e) => {
                    return err(
                        ValidationRule::TermMalformed,
                        format!("{prop_iri} does not decode as an eigentt:Judgement: {e}"),
                    )
                }
            };
            if let Err(reason) = crate::nbe::check::check_type(&mut ctx, &j.typ) {
                return err(
                    ValidationRule::TermIllTyped,
                    format!("{prop_iri}'s `type` field is not a type: {reason}"),
                );
            }
            // `check` takes the type as a VALUE, so evaluate it first — the same
            // order the contract states: the type is checked to be a type, then
            // the term is checked against it.
            let typ_val = match ctx.eval(&j.typ, &crate::nbe::env::Rho::Nil) {
                Ok(v) => v,
                Err(e) => {
                    return err(
                        ValidationRule::TermIllTyped,
                        format!("{prop_iri}'s `type` field does not evaluate: {e}"),
                    )
                }
            };
            return match crate::nbe::check::check(&mut ctx, &j.term, &typ_val) {
                Ok(()) => vec![],
                Err(reason) => err(
                    ValidationRule::TermIllTyped,
                    format!(
                        "{prop_iri}'s `term` does not inhabit its `type` under logic {}: {reason}",
                        j.logic
                    ),
                ),
            };
        }

        let exp = match decode_type(value, &self.layer) {
            Ok(e) => e,
            Err(e) => {
                return err(
                    ValidationRule::TermMalformed,
                    format!("value failed to decode through the D47 codec: {e}"),
                )
            }
        };

        // Case 2 — the value must itself be a type.
        if matches!(
            prop_def.get(&wk::iri(wk::IS_A_TYPE)),
            Some(Value::Boolean(true))
        ) {
            return match crate::nbe::check::check_type(&mut ctx, &exp) {
                Ok(()) => vec![],
                Err(reason) => err(
                    ValidationRule::TermIllTyped,
                    format!("{prop_iri} must hold a TYPE, but this value is not one: {reason}"),
                ),
            };
        }

        // Cases 1 and 3 — form the annotation if the property names one, then
        // run the kernel's existing inference path either way.
        // The neighbouring-field case. A definition's body is a bare lambda
        // chain whose type is stored beside it, and the design's instruction is
        // to TREAT the pair as an annotated term rather than to introduce a
        // pairing construct: the type comes from the sibling, `Ann` does the
        // rest, and the stored shape does not change. Inference structurally
        // fails on a bare `Lam`, so without this the body is uncheckable here —
        // which is why it used to carry an exemption.
        if let Some((sibling, ty)) = paired_slot(prop_iri, owner) {
            let t = match decode_type(&ty, &self.layer) {
                Ok(t) => t,
                Err(e) => {
                    return err(
                        ValidationRule::TermMalformed,
                        format!("{prop_iri}'s companion {sibling} does not decode: {e}"),
                    )
                }
            };
            let ann = Exp::Ann(Box::new(exp), Box::new(t));
            return match crate::nbe::check::check_infer(&mut ctx, &ann) {
                Ok(_) => vec![],
                Err(reason) => err(
                    ValidationRule::TermIllTyped,
                    format!("{prop_iri} does not inhabit its {sibling}: {reason}"),
                ),
            };
        }

        let expected = match prop_def.get(&wk::iri(wk::EXPECTED_TYPE)) {
            Some(v) => match decode_type(v, &self.layer) {
                Ok(t) => Some(t),
                Err(e) => {
                    return err(
                        ValidationRule::TermMalformed,
                        format!("{prop_iri}'s eigentt:expected_type does not decode: {e}"),
                    )
                }
            },
            None => None,
        };
        let expects_prop = matches!(&expected, Some(Exp::Sort(l)) if l.is_nat(0));
        let checked = match &expected {
            Some(t) => Exp::Ann(Box::new(exp.clone()), Box::new(t.clone())),
            None => exp.clone(),
        };

        match crate::nbe::check::check_infer(&mut ctx, &checked) {
            Ok(_) => vec![],
            Err(reason) => {
                // Say what the value DOES inhabit where we can — "this inhabits
                // Set, not Prop" is what an author can act on. A slot expecting
                // `Prop` reports the dedicated variant: holding a type where a
                // claim belongs is a different error from an ill-typed term.
                let mut ctx2 = crate::nbe::check::CheckCtx::with_layer(
                    crate::nbe::env::Rho::Nil,
                    Vec::new(),
                    std::sync::Arc::clone(&self.layer),
                );
                let inhabited = crate::nbe::check::check_infer(&mut ctx2, &exp).ok();

                // Two different authoring errors, and they want different
                // fixes. A value that infers NOTHING is ill-typed on its own
                // terms — a predicate applied to the wrong argument type, an
                // application of a non-function. A value that infers something
                // is well-formed and merely in the wrong slot. Reporting the
                // first as "not a proposition" would name the slot when the
                // defect is in the term.
                let Some(ty) = inhabited else {
                    return err(
                        ValidationRule::TermIllTyped,
                        format!(
                            "{prop_iri} value decodes but does not type-check against the \
                             chain: {reason}"
                        ),
                    );
                };
                if expects_prop {
                    return err(
                        ValidationRule::TermNotAProposition,
                        format!(
                            "{prop_iri} must hold a proposition — a term inhabiting \
                             Prop = Sort(0) — but this value inhabits {}.",
                            describe_inhabited(&ty)
                        ),
                    );
                }
                err(
                    ValidationRule::TermIllTyped,
                    format!(
                        "{prop_iri} value does not check against its declared type — it \
                         inhabits {} instead: {reason}",
                        describe_inhabited(&ty)
                    ),
                )
            }
        }
    }
}

/// Slots whose type is stored in a neighbouring slot on the same resource.
/// Returns the companion's short name and its value.
///
/// One pair, hardcoded, rather than a general sibling-reference mechanism: a
/// declared pointer built for a single user is a vocabulary nobody else needs,
/// and `eigentt:Definition` requires both halves, so the pairing is already an
/// invariant of the class rather than a convention this rule imposes.
fn paired_slot(prop_iri: &Iri, owner: &Resource) -> Option<(&'static str, Value)> {
    if prop_iri.as_str() != "urn:eigenius:eigentt:definition_body" {
        return None;
    }
    let ty = Iri::parse("urn:eigenius:eigentt:definition_type").ok()?;
    Some(("eigentt:definition_type", owner.get(&ty)?.clone()))
}

/// Telescope fragments of a `core:InductiveType` declaration. They are open
/// terms in a binder scope this rule does not have, and Rule 23 checks them
/// against the full chain. See the module docs.
///
/// `core:ctor_type` is deliberately NOT here even though it is also an
/// embedded declaration internal: its values decode and check standalone, and
/// the check is load-bearing — it is what refuses a constructor type naming an
/// unresolvable IRI. The two telescope properties cannot even be decoded
/// reliably out of scope (`core:type_name` on a statistics sample set carries
/// `ConstRef(core:value_array)`, which does not resolve as a term), which is
/// what separates them.
fn is_declaration_internal(prop_iri: &Iri) -> bool {
    matches!(
        prop_iri.as_str(),
        "urn:eigenius:core:param_kind" | "urn:eigenius:core:type_name"
    )
}

/// Name what a decoded term turned out to inhabit, for the step-3
/// diagnostic. Universes get their D46 names; anything else is reported as a
/// non-universe, since the author's error there is that the slot holds a
/// term rather than a statement.
fn describe_inhabited(ty: &Val) -> String {
    match ty {
        Val::Sort(l) if l.is_nat(0) => "Prop = Sort(0)".to_string(),
        Val::Sort(l) if l.is_nat(1) => "Set = Sort(1)".to_string(),
        Val::Sort(n) => match n.as_nat() {
            Some(k) if k >= 1 => format!("Type({}) = Sort({n})", k - 1),
            _ => format!("Sort({n})"),
        },
        other => {
            let readback = format!("{:?}", readback_val(0, other));
            let shown: String = readback.chars().take(160).collect();
            format!("a non-universe type (so the value is a term, not a statement): {shown}")
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::layer::{Layer, LayerBuilder, LayerStorage};
    use crate::nbe::term::Exp;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;
    use crate::ontology::Iri;
    use crate::program::eigentt_type_mirror::encode_type;
    use crate::validation::{ValidationRule, Validator};
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Bootstrap chain (has `eigentt:Term` + the core type-formers) plus a
    /// property `test:tx : core:resource` ranged at `eigentt:Term`.
    fn chain_with_eigentt_prop() -> Arc<Layer> {
        let head = Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let mut top = LayerBuilder::new("eigentt_value_test", Some(head));
        let mut prop = Resource::new(iri("urn:eigenius:test:tx"));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::iri(&iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::SHORT_NAME), Value::String("tx".into()));
        prop.set(iri(wk::DATA_TYPE_PROP), Value::iri(&iri(wk::RESOURCE)));
        prop.set(
            iri(wk::CLASS_TYPES),
            Value::Array(vec![Value::String(
                iri("urn:eigenius:eigentt:Term").as_str().to_string(),
            )]),
        );
        // `test:tx` is a TYPE slot, which is what the cases below rely on:
        // `Set` passes, an ill-typed application does not, and nothing here
        // demands `Prop`.
        prop.set(iri(wk::IS_A_TYPE), Value::Boolean(true));
        top.add_resource(prop).unwrap();
        Arc::new(top.build(LayerStorage::in_memory()))
    }

    fn holder_with_tx(id: &str, value: Value) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        r.set(iri("urn:eigenius:test:tx"), value);
        r
    }

    fn eigentt_errors(layer: Arc<Layer>) -> Vec<crate::validation::ValidationError> {
        Validator::new(layer)
            .validate()
            .into_iter()
            .filter(|e| {
                matches!(
                    e.rule,
                    ValidationRule::TermMalformed
                        | ValidationRule::TermIllTyped
                        | ValidationRule::TermNotAProposition
                )
            })
            .collect()
    }

    /// A `reflection:DeclaredResource` carrying `value` in the real
    /// `reflection:canonical_proposition` slot, whose declared obligation is
    /// `inhabits(Prop)` — so the propositionhood check applies.
    fn claim_with_proposition(id: &str, value: Value) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        r.set(
            iri("urn:eigenius:prov:was_attributed_to"),
            Value::String("test:eigentt_value".into()),
        );
        r.set(iri(wk::CANONICAL_PROPOSITION), value);
        r
    }

    fn errors_for_claim(value: Value) -> Vec<crate::validation::ValidationError> {
        let chain = chain_with_eigentt_prop();
        let mut top = LayerBuilder::new("claim", Some(chain));
        top.add_resource(claim_with_proposition("urn:eigenius:test:claim", value))
            .unwrap();
        eigentt_errors(Arc::new(top.build(LayerStorage::in_memory())))
    }

    /// `measurements:lt(1.0, 2.0)` — an axiom application at `Prop`.
    fn a_real_proposition() -> Value {
        Value::Json(serde_json::json!({
            "ctor": "App",
            "args": [
                {"ctor": "App", "args": [
                    {"ctor": "ConstRef", "args": ["urn:eigenius:measurements:lt"]},
                    {"ctor": "LitFloat", "args": [1.0]}
                ]},
                {"ctor": "LitFloat", "args": [2.0]}
            ]
        }))
    }

    #[test]
    fn proposition_in_a_proposition_slot_passes() {
        let errs = errors_for_claim(a_real_proposition());
        assert!(
            errs.is_empty(),
            "an axiom application at Prop must commit; got {errs:?}"
        );
    }

    #[test]
    fn integer_literal_in_a_proposition_slot_rejected() {
        // eigenius#175's example: a literal decodes and type-checks (at
        // `core:integer`), so steps 1–2 pass. Only step 3 catches it.
        let encoded = encode_type(&Exp::LitInt(42)).unwrap();
        let errs = errors_for_claim(encoded);
        assert_eq!(
            errs.len(),
            1,
            "an integer literal is not a proposition; got {errs:?}"
        );
        assert!(matches!(errs[0].rule, ValidationRule::TermNotAProposition));
        assert!(
            errs[0].message.contains("Prop = Sort(0)"),
            "diagnostic should name the obligation: {}",
            errs[0].message
        );
    }

    #[test]
    fn a_type_in_a_proposition_slot_rejected() {
        // `Prop` itself is a perfectly good `eigentt:Term` — it passes in
        // the unconstrained `test:tx` slot above — but it inhabits `Set`, so
        // it asserts nothing.
        let encoded = encode_type(&Exp::sort(0)).unwrap();
        let errs = errors_for_claim(encoded);
        assert_eq!(errs.len(), 1, "`Prop` asserts nothing; got {errs:?}");
        assert!(matches!(errs[0].rule, ValidationRule::TermNotAProposition));
        assert!(
            errs[0].message.contains("Set = Sort(1)"),
            "diagnostic should name what the value does inhabit: {}",
            errs[0].message
        );
    }

    #[test]
    fn unapplied_predicate_in_a_proposition_slot_rejected() {
        // `measurements:lt` on its own is `float -> float -> Prop` — a
        // predicate, not the claim that some pair satisfies it.
        let value = Value::Json(serde_json::json!({
            "ctor": "ConstRef",
            "args": ["urn:eigenius:measurements:lt"]
        }));
        let errs = errors_for_claim(value);
        assert_eq!(
            errs.len(),
            1,
            "an unapplied predicate is not a proposition; got {errs:?}"
        );
        assert!(matches!(errs[0].rule, ValidationRule::TermNotAProposition));
    }

    #[test]
    fn the_obligation_is_per_slot_not_per_range() {
        // Guards against the propositionhood demand being widened to the whole
        // `eigentt:Term` range, which would reject every `eigentt:axiom_statement`
        // and `lexicon:cat` on the chain. The SAME value is admissible in a slot
        // declaring `is_a_type` and inadmissible in one declaring
        // `inhabits(Prop)` — the range is identical, only the obligation differs.
        //
        // This replaces a test asserting that a type AND an integer literal both
        // belonged in the same slot. That was true when a slot outside the
        // proposition list carried no obligation at all; a slot admitting both is
        // now unrepresentable, which is the defect this phase closes.
        let set = encode_type(&Exp::sort(1)).unwrap();

        let chain = chain_with_eigentt_prop();
        let mut top = LayerBuilder::new("tx", Some(chain));
        top.add_resource(holder_with_tx("urn:eigenius:test:tx_holder", set.clone()))
            .unwrap();
        let errs = eigentt_errors(Arc::new(top.build(LayerStorage::in_memory())));
        assert!(
            errs.is_empty(),
            "Set must pass an is_a_type slot; got {errs:?}"
        );

        let errs = errors_for_claim(set);
        assert_eq!(errs.len(), 1, "Set must be rejected by inhabits(Prop)");
        assert!(
            matches!(errs[0].rule, ValidationRule::TermNotAProposition),
            "got {:?}",
            errs[0]
        );
    }

    #[test]
    fn well_formed_eigentt_value_passes() {
        let chain = chain_with_eigentt_prop();
        let mut top = LayerBuilder::new("ok", Some(chain));
        // `Prop` (Sort(0)) is a valid type expression that type-checks.
        let encoded = encode_type(&Exp::sort(0)).unwrap();
        top.add_resource(holder_with_tx("urn:eigenius:test:ok", encoded))
            .unwrap();
        let layer = Arc::new(top.build(LayerStorage::in_memory()));
        let errs = eigentt_errors(layer);
        assert!(
            errs.is_empty(),
            "well-formed eigentt value (Prop) must pass; got {errs:?}"
        );
    }

    #[test]
    fn malformed_eigentt_value_rejected() {
        // A raw string instead of a D47 tree fails to decode.
        let chain = chain_with_eigentt_prop();
        let mut top = LayerBuilder::new("bad", Some(chain));
        top.add_resource(holder_with_tx(
            "urn:eigenius:test:bad",
            Value::String("not-a-typeexpr".into()),
        ))
        .unwrap();
        let layer = Arc::new(top.build(LayerStorage::in_memory()));
        let errs = eigentt_errors(layer);
        assert_eq!(
            errs.len(),
            1,
            "malformed eigentt value must be rejected exactly once; got {errs:?}"
        );
        assert!(matches!(errs[0].rule, ValidationRule::TermMalformed));
        assert!(
            errs[0].message.contains("D47 codec"),
            "diagnostic should name the D47 codec: {}",
            errs[0].message
        );
    }

    #[test]
    fn unresolved_constref_rejected() {
        // ConstRef to a non-existent IRI fails to decode.
        let chain = chain_with_eigentt_prop();
        let mut top = LayerBuilder::new("unresolved", Some(chain));
        let value = Value::Json(serde_json::json!({
            "ctor": "ConstRef",
            "args": ["urn:eigenius:nonexistent:Foo"]
        }));
        top.add_resource(holder_with_tx("urn:eigenius:test:unresolved", value))
            .unwrap();
        let layer = Arc::new(top.build(LayerStorage::in_memory()));
        let errs = eigentt_errors(layer);
        assert_eq!(
            errs.len(),
            1,
            "unresolved ConstRef must be rejected; got {errs:?}"
        );
        assert!(matches!(errs[0].rule, ValidationRule::TermMalformed));
        assert!(
            errs[0].message.contains("urn:eigenius:nonexistent:Foo"),
            "diagnostic should name the offending IRI: {}",
            errs[0].message
        );
    }

    #[test]
    fn ill_typed_eigentt_value_rejected() {
        // App(ConstRef(measurements:lt), ConstRef(core:Class)) — the axiom
        // `lt : float -> float -> Prop` applied to `Class` (a type, not a
        // float). This DECODES (an axiom is applicable) but fails check_infer:
        // the argument's type does not match the axiom's domain. This is the
        // felicity check a decode-only gate would miss.
        let chain = chain_with_eigentt_prop();
        let mut top = LayerBuilder::new("illtyped", Some(chain));
        let value = Value::Json(serde_json::json!({
            "ctor": "App",
            "args": [
                {"ctor": "ConstRef", "args": ["urn:eigenius:measurements:lt"]},
                {"ctor": "ConstRef", "args": ["urn:eigenius:core:Class"]}
            ]
        }));
        top.add_resource(holder_with_tx("urn:eigenius:test:illtyped", value))
            .unwrap();
        let layer = Arc::new(top.build(LayerStorage::in_memory()));
        let errs = eigentt_errors(layer);
        assert!(
            errs.iter()
                .any(|e| matches!(e.rule, ValidationRule::TermIllTyped)),
            "ill-typed eigentt value (App of a class to a class) must be rejected by \
             check_infer; got {errs:?}"
        );
    }

    #[test]
    fn missing_eigentt_value_is_skipped() {
        // A resource without the eigentt property → the rule is a no-op.
        let chain = chain_with_eigentt_prop();
        let mut top = LayerBuilder::new("missing", Some(chain));
        let mut r = Resource::new(iri("urn:eigenius:test:missing"));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        top.add_resource(r).unwrap();
        let layer = Arc::new(top.build(LayerStorage::in_memory()));
        assert!(eigentt_errors(layer).is_empty());
    }
}
