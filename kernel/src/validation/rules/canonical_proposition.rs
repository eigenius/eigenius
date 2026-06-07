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

//! Rule 20 — `reflection:canonical_proposition` decoder check (D49 §6).
//!
//! When a resource carries a `reflection:canonical_proposition` property,
//! the value must decode cleanly through the D47 type-fragment codec.
//! Malformed propositions are rejected at commit so they never reach the
//! per-`Layer` witness index — without this gate, a malformed proposition
//! would silently absent the corresponding `ChainWitness` rather than
//! producing a diagnostic.
//!
//! The decoder check is structural: it confirms the value parses as a
//! well-formed `eigentt:TypeExpr` JSON tree and that all `ConstRef` /
//! `CtorApp` references resolve in the chain. The full `Prop`-typing of
//! the decoded `Exp` (D49 §6) is layered on top once D39's Reasoning
//! institution introduces a context that can invoke the kernel's NbE
//! checker against the proposition; this v1 rule catches the gross
//! malformations (wrong shape, missing references) and lets the typing
//! check land additively.

use super::super::{ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;
use crate::program::eigentt_type_mirror::decode_type;

impl Validator {
    /// Walk every property on the resource looking for
    /// `reflection:canonical_proposition`. When present, decode the
    /// value via the D47 codec; surface decode failures as a structured
    /// `CanonicalPropositionMalformed` error.
    pub(in crate::validation) fn check_canonical_proposition(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let prop_iri = match Iri::parse(wk::CANONICAL_PROPOSITION) {
            Ok(i) => i,
            Err(_) => return vec![],
        };
        let value = match resource.get(&prop_iri) {
            Some(v) => v,
            None => return vec![],
        };
        match decode_type(value, &self.layer) {
            Ok(_exp) => vec![],
            Err(e) => vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri),
                rule: ValidationRule::CanonicalPropositionMalformed,
                message: format!(
                    "reflection:canonical_proposition failed to decode through D47 codec: {e:?}. \
                     The value must be a Value::Json carrying a well-formed eigentt:TypeExpr tree \
                     (Sort / Var / ConstRef / App / Pi / Sig / Lam / One / Id / etc.) with all \
                     ConstRef / CtorApp references resolving in the chain. Mis-typed or malformed \
                     propositions are rejected at commit per D49 §6 so they never silently absent \
                     the corresponding ChainWitness."
                ),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::layer::{Layer, LayerBuilder, LayerStorage};
    use crate::nbe::term::Exp;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;
    use crate::ontology::{eigon_cbor, Iri};
    use crate::program::eigentt_type_mirror::encode_type;
    use crate::validation::{ValidationRule, Validator};
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn empty_layer() -> Arc<Layer> {
        Arc::new(LayerBuilder::new("test", None).build(LayerStorage::in_memory()))
    }

    fn resource_with_canonical_prop(target_iri: &str, value: Value) -> Resource {
        let mut r = Resource::new(iri(target_iri));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::DECLARED_RESOURCE.to_string())]),
        );
        r.set(iri(wk::CANONICAL_PROPOSITION), value);
        r
    }

    #[test]
    fn well_formed_canonical_proposition_passes_validation() {
        let layer = empty_layer();
        let validator = Validator::new(layer);
        let prop = Exp::Sort(0); // Prop sort literal
        let encoded = encode_type(&prop).unwrap();
        let resource = resource_with_canonical_prop("urn:eigenius:example:thing", encoded);
        let errs = validator.check_canonical_proposition(&resource, &resource.id().cloned());
        assert!(
            errs.is_empty(),
            "well-formed Sort(0) canonical_proposition should pass; got: {errs:?}"
        );
    }

    #[test]
    fn missing_canonical_proposition_is_silently_skipped() {
        let layer = empty_layer();
        let validator = Validator::new(layer);
        let mut r = Resource::new(iri("urn:eigenius:example:thing"));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::DECLARED_RESOURCE.to_string())]),
        );
        let errs = validator.check_canonical_proposition(&r, &r.id().cloned());
        assert!(
            errs.is_empty(),
            "no canonical_proposition → no validation error (Phase 4 default still applies)"
        );
    }

    #[test]
    fn malformed_canonical_proposition_rejected() {
        // A canonical_proposition value that's a raw string instead of
        // an eigentt:TypeExpr tree decodes to a TypeMismatch on the
        // codec side; the rule surfaces it as
        // CanonicalPropositionMalformed.
        let layer = empty_layer();
        let validator = Validator::new(layer);
        let resource = resource_with_canonical_prop(
            "urn:eigenius:example:thing",
            Value::String("not-a-typeexpr".to_string()),
        );
        let errs = validator.check_canonical_proposition(&resource, &resource.id().cloned());
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            errs[0].rule,
            ValidationRule::CanonicalPropositionMalformed
        ));
        // Sanity that the diagnostic references the codec
        assert!(
            errs[0].message.contains("D47 codec"),
            "diagnostic should name the D47 codec: {}",
            errs[0].message
        );

        // Sanity: this isn't because the value can't be serialised — it
        // can be CBOR'd just fine; the issue is purely the codec semantic.
        let _bytes = eigon_cbor::serialize_value(&Value::String("not-a-typeexpr".to_string()));
    }
}
