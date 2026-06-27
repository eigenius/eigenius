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
                // A resource reference is canonically an IRI-valued text.
                // `LayerBuilder::build` -> `canonicalise_resource_refs`
                // upgrades a wire `Value::String` IRI to `Value::ResourceRef`
                // in memory, but that distinction is deliberately NOT durable:
                // the CBOR codec serialises both as `Text` and the content
                // hash treats them as identical (`value_to_cbor`), so a
                // committed layer reloaded from the backend carries
                // `Value::String` for its resource-typed properties. Rule 3
                // is the wire-level *shape* gate and must therefore be
                // invariant under persist/reload: it accepts `String` (the
                // canonical persisted/wire ref form), `ResourceRef` (the
                // in-memory canonical form), and `Embedded` (an inlined
                // Resource). Whether the IRI actually *resolves* is reference
                // integrity's job (Rule 22), not this rule's.
                //
                // When `class_types` declares an `InductiveType`, also
                // accept `Value::Json` — the tagged-dict carrier for
                // inductive values. The deeper structural check
                // (ctor / arg_types) runs in `check_class_types`,
                // mirroring the `core:inductive` split.
                if self.class_types_inductive_target(prop_def).is_some() {
                    matches!(
                        value,
                        Value::String(_)
                            | Value::ResourceRef(_)
                            | Value::Embedded(_)
                            | Value::Json(_)
                    )
                } else {
                    matches!(
                        value,
                        Value::String(_) | Value::ResourceRef(_) | Value::Embedded(_)
                    )
                }
            }
            wk::RESOURCE_ARRAY => match value {
                Value::Array(arr) => {
                    if self.class_types_inductive_target(prop_def).is_some() {
                        arr.iter().all(|v| {
                            matches!(
                                v,
                                Value::String(_)
                                    | Value::ResourceRef(_)
                                    | Value::Embedded(_)
                                    | Value::Json(_)
                            )
                        })
                    } else {
                        arr.iter().all(|v| {
                            matches!(
                                v,
                                Value::String(_) | Value::ResourceRef(_) | Value::Embedded(_)
                            )
                        })
                    }
                }
                _ => false,
            },
            wk::VALUE_ARRAY => matches!(value, Value::Array(_)),
            wk::JSON => true, // Any value is valid for JSON
            wk::INDUCTIVE => {
                // Wire-level shape check: an inductive value lands as
                // either a `Value::Json` carrying the tagged-dict tree
                // or a `Value::Embedded` resource. The deeper
                // structural type-check (ctor exists on declared
                // InductiveType, arg shapes match `arg_types`) lives in
                // `check_inductive_value` (rule 16) — same split as
                // `check_class_types` for `core:resource`.
                matches!(value, Value::Json(_) | Value::Embedded(_))
            }
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
