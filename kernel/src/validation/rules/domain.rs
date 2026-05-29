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

//! Rule 10: Domain checking. When a property declares a `domain` set of
//! classes, the resource carrying the property must be an instance of
//! one of them (or of a subclass).

use super::super::{iri, ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 10: Domain checking.
    pub(in crate::validation) fn check_domain(
        &self,
        prop_def: &Resource,
        resource: &Resource,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let domain_classes = match prop_def.get(&iri(wk::DOMAIN)) {
            Some(val) => val.as_iri_array(),
            None => return vec![], // No domain constraint
        };

        if domain_classes.is_empty() {
            return vec![];
        }

        let domain_refs: Vec<&Iri> = domain_classes.iter().collect();
        if self.is_instance_of_any(resource, &domain_refs) {
            vec![]
        } else {
            vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::DomainViolation,
                message: format!(
                    "property '{prop_iri}' is not allowed on this resource type (domain restriction)"
                ),
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
    fn domain_violation() {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test", Some(core));
        // 'requires' has domain [Class], but we put it on a non-Class resource
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:not_a_class",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("I'm a property".into())),
                    (wk::SHORT_NAME, Value::String("not_a_class".into())),
                    (wk::DATA_TYPE_PROP, Value::String(wk::STRING.to_string())),
                    // 'requires' is Class-only, but this is a Property
                    (
                        wk::REQUIRES,
                        Value::Array(vec![Value::String("urn:eigenius:test:foo".to_string())]),
                    ),
                ],
            ))
            .unwrap();
        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        assert!(errors
            .iter()
            .any(|e| e.rule == ValidationRule::DomainViolation));
    }
}
