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

//! Rule 3: Type checking — a property's value must match its declared
//! `data_type`. Splits primitive types (string/integer/float/boolean)
//! from the carrier shapes for resource references, value arrays and
//! inductive trees. The deeper structural type-checks (class membership,
//! inductive ctor matching) live in their own rule files; this one is
//! the wire-level gate.

use super::super::{ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 3: Type checking — value must match property's data_type.
    pub(in crate::validation) fn check_type(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let dt = match self.get_data_type_str(prop_def) {
            Some(dt) => dt,
            None => return vec![], // No data_type defined, skip
        };

        let ok = match dt.as_str() {
            wk::STRING => matches!(value, Value::String(_)),
            wk::INTEGER => matches!(value, Value::Integer(_)),
            wk::FLOAT => matches!(value, Value::Float(_) | Value::Integer(_)),
            wk::BOOLEAN => matches!(value, Value::Boolean(_)),
            wk::RESOURCE => {
                // A resource reference is an IRI-valued text. Rule 3 is the wire-level
                // *shape* gate and must be invariant under persist/reload, so it accepts
                // `String` (a reference) and `Embedded` (an inlined Resource). Whether the
                // IRI actually *resolves* is reference integrity's job (Rule 22), not this
                // rule's — but whether it IS an IRI is this rule's, and was nobody's.
                //
                // The string has to PARSE. Accepting any string here left prose in a
                // reference slot detectable only by accident: Rule 22 collects targets with
                // `value.as_iri()`, so a value that does not parse yields no targets and the
                // resolve loop never runs. `prov:was_generated_by` — `core:resource` at
                // `class_types: [prov:Activity]` — held a description in 14 places across the
                // WRN inputs, and exactly ONE was ever reported: the one whose text contains
                // `Firefly:Renilla`, which parses as an IRI and therefore reached the check.
                // Thirteen equally wrong values were green because their prose had no colon.
                //
                // So the slot's declared type decides the shape, and the downstream rule stops
                // inferring intent from punctuation.
                //
                // A `Value::ResourceRef` was accepted here too, until it was retired on
                // `2026-08-31` (D85 §6.2). It was exactly the non-durable distinction this
                // comment warned about: `LayerBuilder::build` produced it in memory, the CBOR
                // codec serialised it as `Text`, and a reloaded layer carried `String`.
                //
                // A slot whose `class_types` names an `InductiveType` needs no separate arm:
                // an inductive value is a resource (D85 §6.1), so it arrives `Embedded` like
                // any other, and Rule 6 checks its constructor class against `class_types`.
                match value {
                    Value::Embedded(_) => true,
                    Value::String(_) => value.as_iri().is_some(),
                    _ => false,
                }
            }
            wk::RESOURCE_ARRAY => match value {
                Value::Array(arr) => arr.iter().all(|v| match v {
                    Value::Embedded(_) => true,
                    Value::String(_) => v.as_iri().is_some(),
                    _ => false,
                }),
                _ => false,
            },
            wk::VALUE_ARRAY => matches!(value, Value::Array(_)),
            wk::JSON => true, // Any value is valid for JSON
            // An inductive value is a resource whose `is_a` names the constructor's class
            // (D85 §6.1), so it lands `Embedded` and nothing else. Which constructor, and
            // whether its arguments type-check, is Rule 6's and Rule 23's to say.
            wk::INDUCTIVE => matches!(value, Value::Embedded(_)),
            _ => true, // Unknown data type, skip
        };

        if ok {
            vec![]
        } else {
            vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::TypeMismatch,
                message: format!("expected data_type '{dt}', got incompatible value"),
            }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::tests::{build_core_layer, make_resource};
    use super::super::super::{ValidationRule, Validator};
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::Value;
    use crate::ontology::well_known as wk;
    use std::sync::Arc;

    #[test]
    fn type_mismatch() {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test", Some(core));
        // description should be a string, give it an integer
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_type",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::Integer(42)), // Wrong type!
                    (wk::SHORT_NAME, Value::String("bad".into())),
                ],
            ))
            .unwrap();
        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.rule == ValidationRule::TypeMismatch),
            "expected TypeMismatch error; got: {errors:?}"
        );
    }
}
