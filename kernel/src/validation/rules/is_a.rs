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

//! Rule 0: every resource must declare at least one `is_a` class.
//! Rule 14: class-definition reference integrity (eigenius#26) — every
//! IRI declared in `requires` / `recommends` / `subclass_of` /
//! `class_types` / `data_type` on a Class or Property resource must
//! resolve to a chain resident of the expected kind. Without this,
//! typos commit cleanly and only blow up much later at instance
//! validation or program execution time.

use super::super::{iri, value_as_iri, ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 0: every resource declares at least one `is_a` class.
    /// Previously this was enforced incidentally by the parser
    /// rejecting empty arrays; that check was removed because empty
    /// arrays are a valid Eigon value for other properties (e.g.
    /// `urn:eigenius:query:rows` for a zero-row query result). The
    /// semantic of "an Eigon resource has at least one is_a class"
    /// is a validator rule, not a wire-format rule.
    pub(in crate::validation) fn check_missing_is_a(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        if resource.is_a().is_empty() {
            vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(iri(wk::IS_A)),
                rule: ValidationRule::MissingRequired,
                message: "resource has no `is_a` classes".to_string(),
            }]
        } else {
            vec![]
        }
    }

    /// Rule 14: class-definition reference integrity (eigenius#26).
    ///
    /// For Class resources, every IRI in `requires` / `recommends` must
    /// resolve to a `core:Property` and every IRI in `subclass_of` must
    /// resolve to a `core:Class`. For Property resources, every IRI in
    /// `class_types` must resolve to a `core:Class` and `data_type` (if
    /// present) must resolve to a `core:DataType`.
    ///
    /// Without this, a typo in `requires patent:innovation_category`
    /// (vs `invention_category`) commits cleanly and only fails much
    /// later at instance validation or program execution time, far
    /// from the offending declaration.
    pub(in crate::validation) fn check_class_definition_references(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let is_class = resource.is_instance_of(&iri(wk::CLASS));
        let is_property = resource.is_instance_of(&iri(wk::PROPERTY));
        if !is_class && !is_property {
            return errors;
        }

        if is_class {
            self.check_array_refs(
                resource,
                ReferenceCheck::REQUIRES_PROPERTY,
                res_id,
                &mut errors,
            );
            self.check_array_refs(
                resource,
                ReferenceCheck::RECOMMENDS_PROPERTY,
                res_id,
                &mut errors,
            );
            self.check_array_refs(
                resource,
                ReferenceCheck::SUBCLASS_OF_CLASS,
                res_id,
                &mut errors,
            );
        }

        if is_property {
            // class_types accepts BOTH Class and InductiveType IRIs
            // (D32 §3.5): `data_type: core:resource(_array)` properties
            // reference Classes; `data_type: core:inductive` properties
            // reference InductiveTypes. Walk the array and accept
            // either kind.
            if let Some(value) = resource.get(&iri(wk::CLASS_TYPES)) {
                for target in value.as_iri_array() {
                    match self.layer.resolve(&target) {
                        Some(t)
                            if t.is_instance_of(&iri(wk::CLASS))
                                || t.is_instance_of(&iri(wk::INDUCTIVE_TYPE)) => {}
                        Some(_) => errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: Some(iri(wk::CLASS_TYPES)),
                            rule: ValidationRule::UnresolvedClassReference,
                            message: format!(
                                "class_types: '{target}' resolves to a resource that is not an instance of core:Class or core:InductiveType"
                            ),
                        }),
                        None => errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: Some(iri(wk::CLASS_TYPES)),
                            rule: ValidationRule::UnresolvedClassReference,
                            message: format!(
                                "class_types: '{target}' does not resolve to any resource in the layer chain"
                            ),
                        }),
                    }
                }
            }
            // `data_type` is a single resource ref (not an array).
            if let Some(value) = resource.get(&iri(wk::DATA_TYPE_PROP)) {
                if let Some(target) = value_as_iri(value) {
                    self.check_resolves_to(&target, ReferenceCheck::DATA_TYPE, res_id, &mut errors);
                }
            }
        }
        errors
    }

    /// Walk an array-valued reference field on `resource` and verify
    /// every element resolves to a resource of the expected class.
    fn check_array_refs(
        &self,
        resource: &Resource,
        check: ReferenceCheck<'static>,
        res_id: &Option<Iri>,
        errors: &mut Vec<ValidationError>,
    ) {
        let Some(value) = resource.get(&iri(check.field_iri)) else {
            return;
        };
        for target in value.as_iri_array() {
            self.check_resolves_to(&target, check, res_id, errors);
        }
    }

    /// Verify a single referenced IRI resolves to a resource of the
    /// expected class. Reports unresolved or wrong-kind references
    /// against `ValidationRule::UnresolvedClassReference`.
    fn check_resolves_to(
        &self,
        target: &Iri,
        check: ReferenceCheck<'_>,
        res_id: &Option<Iri>,
        errors: &mut Vec<ValidationError>,
    ) {
        let expected_class = iri(check.expected_class_iri);
        match self.layer.resolve(target) {
            Some(target_resource) if target_resource.is_instance_of(&expected_class) => {}
            Some(_) => errors.push(ValidationError {
                resource_id: res_id.clone(),
                property: Some(iri(check.field_iri)),
                rule: ValidationRule::UnresolvedClassReference,
                message: format!(
                    "{}: '{target}' resolves to a resource that is not an instance of {}",
                    check.field_label, check.expected_class_label,
                ),
            }),
            None => errors.push(ValidationError {
                resource_id: res_id.clone(),
                property: Some(iri(check.field_iri)),
                rule: ValidationRule::UnresolvedClassReference,
                message: format!(
                    "{}: '{target}' does not resolve to any resource in the layer chain",
                    check.field_label,
                ),
            }),
        }
    }
}

/// Bundle of "what we're checking" for the class-definition reference
/// validation pass (rule 14, eigenius#26). One value per field/expected-
/// class pair; the constants below cover the five sites the validator
/// inspects.
#[derive(Copy, Clone)]
struct ReferenceCheck<'a> {
    /// Field IRI on the source resource (e.g. `core:requires`).
    field_iri: &'a str,
    /// Human label for the field used in error messages (e.g. `requires`).
    field_label: &'a str,
    /// Class IRI the referenced resource must be an instance of
    /// (e.g. `core:Property`).
    expected_class_iri: &'a str,
    /// Human label for the expected class (e.g. `core:Property`).
    expected_class_label: &'a str,
}

impl ReferenceCheck<'static> {
    const REQUIRES_PROPERTY: Self = Self {
        field_iri: wk::REQUIRES,
        field_label: "requires",
        expected_class_iri: wk::PROPERTY,
        expected_class_label: "core:Property",
    };
    const RECOMMENDS_PROPERTY: Self = Self {
        field_iri: wk::RECOMMENDS,
        field_label: "recommends",
        expected_class_iri: wk::PROPERTY,
        expected_class_label: "core:Property",
    };
    const SUBCLASS_OF_CLASS: Self = Self {
        field_iri: wk::PARENT_CLASSES,
        field_label: "subclass_of",
        expected_class_iri: wk::CLASS,
        expected_class_label: "core:Class",
    };
    // class_types accepts both Class and InductiveType references
    // (D32 §3.5); the check is open-coded in
    // `check_class_definition_references` because `ReferenceCheck` is
    // single-class-only.
    const DATA_TYPE: Self = Self {
        field_iri: wk::DATA_TYPE_PROP,
        field_label: "data_type",
        expected_class_iri: wk::DATA_TYPE,
        expected_class_label: "core:DataType",
    };
}

#[cfg(test)]
mod tests {
    use super::super::super::tests::{build_core_layer, make_resource};
    use super::super::super::{ValidationRule, Validator};
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::Value;
    use crate::ontology::well_known as wk;
    use std::sync::Arc;

    // --- eigenius#26: class-definition reference integrity ---

    /// A class that `requires` an IRI with no matching Property
    /// declaration anywhere in the chain must fail validation rather
    /// than commit cleanly.
    #[test]
    fn class_requires_unresolved_property_is_rejected() {
        let core = build_core_layer();
        let mut top = LayerBuilder::new("test", Some(core));

        // Class declaring a requires reference to a property that
        // doesn't exist (typo / forgotten-declaration scenario).
        let bad_class = make_resource(
            "urn:eigenius:test:Foo",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::CLASS.into())]),
                ),
                (wk::SHORT_NAME, Value::String("Foo".into())),
                (wk::DESCRIPTION, Value::String("Test class.".into())),
                (
                    wk::REQUIRES,
                    Value::Array(vec![Value::String(
                        "urn:eigenius:test:totally_made_up_property".into(),
                    )]),
                ),
            ],
        );
        top.add_resource(bad_class).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::UnresolvedClassReference
                    && e.message.contains("totally_made_up_property")
            })
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected exactly one UnresolvedClassReference for the missing property; got {errors:?}"
        );
    }

    /// Same class, but the referenced property is declared in the
    /// same load batch — must validate cleanly.
    #[test]
    fn class_requires_same_batch_property_is_accepted() {
        let core = build_core_layer();
        let mut top = LayerBuilder::new("test", Some(core));

        let prop = make_resource(
            "urn:eigenius:test:my_prop",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::PROPERTY.into())]),
                ),
                (wk::SHORT_NAME, Value::String("my_prop".into())),
                (wk::DESCRIPTION, Value::String("A test property.".into())),
                (
                    wk::DATA_TYPE_PROP,
                    Value::String("urn:eigenius:core:string".into()),
                ),
            ],
        );
        let class = make_resource(
            "urn:eigenius:test:Foo",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::CLASS.into())]),
                ),
                (wk::SHORT_NAME, Value::String("Foo".into())),
                (wk::DESCRIPTION, Value::String("Test class.".into())),
                (
                    wk::REQUIRES,
                    Value::Array(vec![Value::String("urn:eigenius:test:my_prop".into())]),
                ),
            ],
        );
        top.add_resource(prop).unwrap();
        top.add_resource(class).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::UnresolvedClassReference)
            .collect();
        assert!(
            dangling.is_empty(),
            "valid forward-reference should not surface UnresolvedClassReference; got {dangling:?}"
        );
    }

    /// A property whose `data_type` doesn't resolve must fail.
    #[test]
    fn property_data_type_unresolved_is_rejected() {
        let core = build_core_layer();
        let mut top = LayerBuilder::new("test", Some(core));

        let bad_prop = make_resource(
            "urn:eigenius:test:my_prop",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::PROPERTY.into())]),
                ),
                (wk::SHORT_NAME, Value::String("my_prop".into())),
                (wk::DESCRIPTION, Value::String("Bad prop.".into())),
                (
                    wk::DATA_TYPE_PROP,
                    Value::String("urn:eigenius:test:not_a_real_type".into()),
                ),
            ],
        );
        top.add_resource(bad_prop).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::UnresolvedClassReference
                    && e.message.contains("not_a_real_type")
            })
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected exactly one UnresolvedClassReference for the missing data_type; got {errors:?}"
        );
    }

    #[test]
    fn property_class_types_pointing_at_non_class_is_rejected() {
        let core = build_core_layer();
        let mut top = LayerBuilder::new("test", Some(core));

        // A non-Class resource (just an instance of `core:Class`'s
        // base — actually use core:DataType, which is a Class but its
        // *instances* aren't classes themselves).
        let instance = make_resource(
            "urn:eigenius:test:not_a_class",
            vec![
                // is_a a DataType, NOT a Class.
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::DATA_TYPE.into())]),
                ),
                (wk::SHORT_NAME, Value::String("not_a_class".into())),
                (wk::DESCRIPTION, Value::String("placeholder".into())),
            ],
        );
        let bad_prop = make_resource(
            "urn:eigenius:test:my_prop",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::PROPERTY.into())]),
                ),
                (wk::SHORT_NAME, Value::String("my_prop".into())),
                (wk::DESCRIPTION, Value::String("Bad prop.".into())),
                (
                    wk::DATA_TYPE_PROP,
                    Value::String("urn:eigenius:core:resource".into()),
                ),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::String("urn:eigenius:test:not_a_class".into())]),
                ),
            ],
        );
        top.add_resource(instance).unwrap();
        top.add_resource(bad_prop).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::UnresolvedClassReference
                    && e.message.contains("not_a_class")
            })
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected one UnresolvedClassReference for class_types pointing at a non-Class; got {errors:?}"
        );
    }
}
