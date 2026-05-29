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

//! Rules 1 / 2 / 11 support: effective `requires` / `recommends` plus
//! `conditional_requires` evaluation. The required-property check
//! itself lives inline in `validate_resource` (it's a trivial
//! intersection over the union of these two sets); this module owns
//! the chain walk that produces those sets.

use std::collections::BTreeSet;

use super::super::{iri, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

impl Validator {
    /// Collect effective `requires` and `recommends` from all classes and ancestors.
    pub(in crate::validation) fn collect_effective_properties(
        &self,
        class_iris: &[&Iri],
    ) -> (BTreeSet<Iri>, BTreeSet<Iri>) {
        let mut required = BTreeSet::new();
        let mut recommended = BTreeSet::new();

        for class_iri in class_iris {
            self.collect_from_class(
                class_iri,
                &mut required,
                &mut recommended,
                &mut BTreeSet::new(),
            );
        }

        (required, recommended)
    }

    /// Recursively collect requires/recommends from a class and its ancestors.
    fn collect_from_class(
        &self,
        class_iri: &Iri,
        required: &mut BTreeSet<Iri>,
        recommended: &mut BTreeSet<Iri>,
        visited: &mut BTreeSet<Iri>,
    ) {
        if !visited.insert(class_iri.clone()) {
            return; // Already visited (handles cycles)
        }

        if let Some(class_def) = self.layer.resolve(class_iri) {
            // Collect requires
            if let Some(requires_val) = class_def.get(&iri(wk::REQUIRES)) {
                for prop_iri in requires_val.as_iri_array() {
                    required.insert(prop_iri);
                }
            }

            // Collect recommends
            if let Some(recommends_val) = class_def.get(&iri(wk::RECOMMENDS)) {
                for prop_iri in recommends_val.as_iri_array() {
                    recommended.insert(prop_iri);
                }
            }

            // Walk parent classes
            if let Some(parents_val) = class_def.get(&iri(wk::PARENT_CLASSES)) {
                for parent_iri in parents_val.as_iri_array() {
                    self.collect_from_class(&parent_iri, required, recommended, visited);
                }
            }
        }
    }

    /// Evaluate conditional_requires for all classes.
    pub(in crate::validation) fn evaluate_conditional_requires(
        &self,
        class_iris: &[&Iri],
        resource: &Resource,
    ) -> (BTreeSet<Iri>, BTreeSet<Iri>) {
        let mut required = BTreeSet::new();
        let mut recommended = BTreeSet::new();

        for class_iri in class_iris {
            if let Some(class_def) = self.layer.resolve(class_iri) {
                if let Some(conds) = class_def.get(&iri(wk::CONDITIONAL_REQUIRES)) {
                    if let Some(cond_array) = conds.as_array() {
                        for cond in cond_array {
                            if let Value::Embedded(cond_res) = cond {
                                self.evaluate_condition(
                                    cond_res,
                                    resource,
                                    &mut required,
                                    &mut recommended,
                                );
                            }
                        }
                    }
                }
            }
        }

        (required, recommended)
    }

    /// Evaluate a single ConditionalRequirement against a resource.
    fn evaluate_condition(
        &self,
        condition: &Resource,
        resource: &Resource,
        required: &mut BTreeSet<Iri>,
        recommended: &mut BTreeSet<Iri>,
    ) {
        // Get when_property — `data_type: resource`, so the canonical
        // shape is `ResourceRef`; `as_iri` also tolerates the
        // pre-canonical `String` shape.
        let when_prop = match condition
            .get(&iri(wk::WHEN_PROPERTY))
            .and_then(|v| v.as_iri())
        {
            Some(i) => i,
            None => return,
        };

        // Get has_value
        let has_values = match condition.get(&iri(wk::HAS_VALUE)) {
            Some(val) => val.as_iri_array(),
            None => return,
        };

        // Check if the resource's property value matches any has_value.
        // The value being matched is itself an IRI in either shape.
        let resource_value = match resource.get(&when_prop) {
            Some(v) => v,
            None => return,
        };
        let matches = resource_value
            .as_iri()
            .map(|val_iri| has_values.contains(&val_iri))
            .unwrap_or(false);

        if matches {
            // Apply then_requires
            if let Some(then_req) = condition.get(&iri(wk::THEN_REQUIRES)) {
                for prop_iri in then_req.as_iri_array() {
                    required.insert(prop_iri);
                }
            }
            // Apply then_recommends
            if let Some(then_rec) = condition.get(&iri(wk::THEN_RECOMMENDS)) {
                for prop_iri in then_rec.as_iri_array() {
                    recommended.insert(prop_iri);
                }
            }
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
    fn inheritance_requires() {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test", Some(core));

        // Define Animal class requiring 'name'
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:Animal",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("An animal".into())),
                    (wk::SHORT_NAME, Value::String("Animal".into())),
                    (
                        wk::REQUIRES,
                        Value::Array(vec![Value::String("urn:eigenius:test:name".to_string())]),
                    ),
                ],
            ))
            .unwrap();

        // Define Dog class extending Animal, requiring 'breed'
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:Dog",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("A dog".into())),
                    (wk::SHORT_NAME, Value::String("Dog".into())),
                    (
                        wk::PARENT_CLASSES,
                        Value::Array(vec![Value::String("urn:eigenius:test:Animal".to_string())]),
                    ),
                    (
                        wk::REQUIRES,
                        Value::Array(vec![Value::String("urn:eigenius:test:breed".to_string())]),
                    ),
                ],
            ))
            .unwrap();

        // Define the properties
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:name",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("Name".into())),
                    (wk::SHORT_NAME, Value::String("name".into())),
                    (wk::DATA_TYPE_PROP, Value::String(wk::STRING.to_string())),
                ],
            ))
            .unwrap();

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:breed",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("Breed".into())),
                    (wk::SHORT_NAME, Value::String("breed".into())),
                    (wk::DATA_TYPE_PROP, Value::String(wk::STRING.to_string())),
                ],
            ))
            .unwrap();

        // Create a Dog instance missing 'name' (inherited from Animal)
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:rex",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String("urn:eigenius:test:Dog".to_string())]),
                    ),
                    (
                        "urn:eigenius:test:breed",
                        Value::String("German Shepherd".into()),
                    ),
                    // Missing 'name'!
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        // Should have a MissingRequired for 'name' on rex
        let rex_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:rex")
                    && e.rule == ValidationRule::MissingRequired
            })
            .collect();
        assert!(
            !rex_errors.is_empty(),
            "Dog instance missing inherited 'name' should fail validation"
        );
    }
}
