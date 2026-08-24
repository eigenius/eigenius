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

//! Rule 21 — the single commit-time validator for `eigentt:TypeExpr` values.
//!
//! Any property whose declared range is `eigentt:TypeExpr` carries a
//! D47-encoded EigenTT tree (a proposition, a type, or a term). This rule is
//! the *one* place that validates such values, end to end:
//!
//! 1. **decode** the tree via the D47 codec (`decode_type`) — catches malformed
//!    trees, unresolved `ConstRef`s, and `CtorApp`s to unknown ctors →
//!    [`ValidationRule::TypeExprMalformed`];
//! 2. **type-check** the decoded `Exp` against the chain (`nbe::check_infer`) —
//!    the Semantic Felicity Condition: a predicate applied to the wrong
//!    argument type, an application of a non-function, etc. →
//!    [`ValidationRule::TypeExprIllTyped`];
//! 3. **require `Prop`** of the slots that assert something —
//!    [`wk::PROPOSITION_SLOTS`] — by keeping the type step 2 infers and
//!    demanding `Sort(0)` → [`ValidationRule::TypeExprNotAProposition`].
//!
//! Step 3 is what separates a claim from an arbitrary term. `check_infer`
//! already computes the type; discarding it let an integer literal commit as
//! a resource's `reflection:canonical_proposition` — the slot the witness
//! index projects into `IsDeclaredAs`/`IsDerivedAs` and the slot
//! `JustifiedBy` certificates are checked against (eigenius#175).
//!
//! Steps 1–2 key off the declared **range** (`class_types ∋
//! eigentt:TypeExpr`), not a property name. Step 3 cannot: that range covers
//! propositions, types, and terms alike, so the propositionhood obligation is
//! carried per-property by [`wk::PROPOSITION_SLOTS`]. This rule
//! **consolidates** what were
//! three overlapping checks: the canonical-proposition decode check (old Rule
//! 20), `check_inductive_value`'s bespoke `ConstRef`/`CtorApp` resolution walk
//! for `eigentt:TypeExpr` (now skipped — see `inductive.rs`), and the
//! type-check itself. One validator, one set of diagnostics, no duplicates.
//!
//! The layer is in hand, so cross-layer `ConstRef`s (axioms, classes,
//! inductives) re-resolve at decode time and applications are fully typed.

use super::super::{ValidationError, ValidationRule, Validator};
use crate::nbe::readback::readback_val;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::program::eigentt_type_mirror::decode_type;

/// The `urn:` of the `eigentt:TypeExpr` inductive — the range marker that
/// designates a property value as a D47-encoded EigenTT tree.
const TYPE_EXPR_IRI: &str = "urn:eigenius:eigentt:TypeExpr";

impl Validator {
    /// Rule 21 — decode + type-check every `eigentt:TypeExpr`-ranged value.
    /// See the module docs. No-op for properties whose range is not
    /// `eigentt:TypeExpr`.
    pub(in crate::validation) fn check_type_expr_well_typed(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let type_expr_iri = match Iri::parse(TYPE_EXPR_IRI) {
            Ok(i) => i,
            Err(_) => return vec![],
        };
        let is_type_expr = prop_def
            .get(&wk::iri(wk::CLASS_TYPES))
            .map(|v| v.as_iri_array().contains(&type_expr_iri))
            .unwrap_or(false);
        if !is_type_expr {
            return vec![];
        }

        // `eigentt:definition_body` is exempt, and deliberately so (D66 slice 2).
        //
        // This rule ends in `check_infer` — INFERENCE. A definition's body is a lambda chain, and a
        // bare `Exp::Lam` has no inferable type: a lambda is *checked against* an expected type, not
        // inferred from itself. Applying inference here rejects every well-formed definition with
        // "cannot infer type of: Lam(...)".
        //
        // The body is not going unchecked — Rule 24 checks it in the correct mode, against the
        // declared `definition_type`, which is strictly stronger than anything inference could
        // establish. `definition_type` itself is NOT exempt and still comes through here.
        if prop_iri.as_str() == "urn:eigenius:eigentt:definition_body" {
            return vec![];
        }

        // `core:param_kind` and `core:type_name` are fragments of a declaration TELESCOPE, not
        // closed terms, and this rule has no telescope. `core:Option`'s `some(value : A)` names the
        // declaration's own parameter `A`, so checking it here reports `unbound variable in type
        // context: A` for a value that is perfectly well-formed where it lives.
        //
        // They are not going unchecked, and the coverage is exact rather than approximate: Rule 23
        // (`rules::inductive_decl.rs`) routes the whole `core:InductiveType` resource through
        // `check_type`'s `Exp::Inductive` arm, which checks the type former `Π params. Π indices.
        // sort` and each constructor's full `Π params. Π args. Self(params)` chain. Every value of
        // these two properties is a domain in one of those Π chains, so each is checked by the Π
        // typing rule — in the scope of the binders before it, which is the scope it was written
        // in. That is strictly stronger than what this rule could establish, and it is the same
        // relationship `eigentt:definition_body` has with Rule 24 above.
        //
        // The coverage holds because `core:InductiveParam` and `core:InductiveArgType` occur ONLY
        // as embedded resources under a `core:InductiveType` — they carry no `@id` and nothing
        // references them. An instance of either reachable any other way would be unchecked; no
        // chain has one, and Rule 23 skips a declaration it cannot decode, which is the shape that
        // would produce one.
        if matches!(
            prop_iri.as_str(),
            "urn:eigenius:core:param_kind" | "urn:eigenius:core:type_name"
        ) {
            return vec![];
        }

        // (1) Decode the D47-encoded tree. Malformed trees, unresolved
        // ConstRefs, and unknown CtorApps surface here.
        let exp = match decode_type(value, &self.layer) {
            Ok(e) => e,
            Err(e) => {
                return vec![ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::TypeExprMalformed,
                    message: format!(
                        "eigentt:TypeExpr value failed to decode through the D47 codec: {e}"
                    ),
                }];
            }
        };

        // (2) Type-check the decoded term against the chain — the felicity
        // check. An ill-typed proposition (e.g. a predicate applied to the
        // wrong argument type) is rejected here, not silently committed.
        let mut ctx = crate::nbe::check::CheckCtx::with_layer(
            crate::nbe::env::Rho::Nil,
            Vec::new(),
            std::sync::Arc::clone(&self.layer),
        );
        let inferred = match crate::nbe::check::check_infer(&mut ctx, &exp) {
            Ok(ty) => ty,
            Err(reason) => {
                return vec![ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::TypeExprIllTyped,
                    message: format!(
                        "eigentt:TypeExpr value decodes but does not type-check against the \
                         chain: {reason}"
                    ),
                }];
            }
        };

        // (3) Propositionhood. Step 2 establishes only that the term HAS a
        // type. A slot that asserts something must hold a term whose type is
        // `Prop` — `Sort(0)`. `Sort(1)` is a type, not a claim; a literal's
        // type is not a universe at all.
        if wk::PROPOSITION_SLOTS.contains(&prop_iri.as_str())
            && !matches!(&inferred, Val::Sort(l) if l.is_nat(0))
        {
            return vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::TypeExprNotAProposition,
                message: format!(
                    "{prop_iri} must hold a proposition — a term inhabiting Prop = Sort(0) — but \
                     this value inhabits {}. The slot is read as a proposition by the witness \
                     index and by JustifiedBy certificate checking.",
                    describe_inhabited(&inferred)
                ),
            }];
        }
        vec![]
    }
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

    /// Bootstrap chain (has `eigentt:TypeExpr` + the core type-formers) plus a
    /// property `test:tx : core:resource` ranged at `eigentt:TypeExpr`.
    fn chain_with_eigentt_prop() -> Arc<Layer> {
        let head = Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let mut top = LayerBuilder::new("eigentt_value_test", Some(head));
        let mut prop = Resource::new(iri("urn:eigenius:test:tx"));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::SHORT_NAME), Value::String("tx".into()));
        prop.set(
            iri(wk::DATA_TYPE_PROP),
            Value::ResourceRef(iri(wk::RESOURCE)),
        );
        prop.set(
            iri(wk::CLASS_TYPES),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:eigentt:TypeExpr",
            ))]),
        );
        top.add_resource(prop).unwrap();
        Arc::new(top.build(LayerStorage::in_memory()))
    }

    fn holder_with_tx(id: &str, value: Value) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::DECLARED_RESOURCE.to_string())]),
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
                    ValidationRule::TypeExprMalformed
                        | ValidationRule::TypeExprIllTyped
                        | ValidationRule::TypeExprNotAProposition
                )
            })
            .collect()
    }

    /// A `reflection:DeclaredResource` carrying `value` in the real
    /// `reflection:canonical_proposition` slot — a member of
    /// `wk::PROPOSITION_SLOTS`, so step 3 applies.
    fn claim_with_proposition(id: &str, value: Value) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::DECLARED_RESOURCE.to_string())]),
        );
        r.set(
            iri("urn:eigenius:reflection:declared_by"),
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
        assert!(matches!(
            errs[0].rule,
            ValidationRule::TypeExprNotAProposition
        ));
        assert!(
            errs[0].message.contains("Prop = Sort(0)"),
            "diagnostic should name the obligation: {}",
            errs[0].message
        );
    }

    #[test]
    fn a_type_in_a_proposition_slot_rejected() {
        // `Prop` itself is a perfectly good `eigentt:TypeExpr` — it passes in
        // the unconstrained `test:tx` slot above — but it inhabits `Set`, so
        // it asserts nothing.
        let encoded = encode_type(&Exp::sort(0)).unwrap();
        let errs = errors_for_claim(encoded);
        assert_eq!(errs.len(), 1, "`Prop` asserts nothing; got {errs:?}");
        assert!(matches!(
            errs[0].rule,
            ValidationRule::TypeExprNotAProposition
        ));
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
        assert!(matches!(
            errs[0].rule,
            ValidationRule::TypeExprNotAProposition
        ));
    }

    #[test]
    fn non_proposition_slots_still_admit_non_props() {
        // The obligation is per-slot, not per-range: `test:tx` is
        // `eigentt:TypeExpr`-ranged but not a proposition slot, so a type and
        // a literal both belong there. Guards against step 3 being widened to
        // the whole range, which would reject every `eigentt:axiom_statement`
        // and `lexicon:cat` on the chain.
        for exp in [Exp::sort(1), Exp::LitInt(7)] {
            let chain = chain_with_eigentt_prop();
            let mut top = LayerBuilder::new("tx", Some(chain));
            let encoded = encode_type(&exp).unwrap();
            top.add_resource(holder_with_tx("urn:eigenius:test:tx_holder", encoded))
                .unwrap();
            let errs = eigentt_errors(Arc::new(top.build(LayerStorage::in_memory())));
            assert!(
                errs.is_empty(),
                "{exp:?} must pass in test:tx; got {errs:?}"
            );
        }
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
        assert!(matches!(errs[0].rule, ValidationRule::TypeExprMalformed));
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
        assert!(matches!(errs[0].rule, ValidationRule::TypeExprMalformed));
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
                .any(|e| matches!(e.rule, ValidationRule::TypeExprIllTyped)),
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
            Value::Array(vec![Value::String(wk::DECLARED_RESOURCE.to_string())]),
        );
        top.add_resource(r).unwrap();
        let layer = Arc::new(top.build(LayerStorage::in_memory()));
        assert!(eigentt_errors(layer).is_empty());
    }
}
