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

//! Rule 9: Allowed-values checking. A property declaring `allows_only`
//! restricts its values to the listed IRI set. Applies to both single
//! and array-valued properties.

use std::collections::BTreeSet;

use super::super::{iri, ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 9: Allowed values checking.
    pub(in crate::validation) fn check_allows_only(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let allowed = match prop_def.get(&iri(wk::ALLOWS_ONLY)) {
            Some(val) => val.as_iri_array(),
            None => return vec![],
        };

        if allowed.is_empty() {
            return vec![];
        }

        let allowed_set: BTreeSet<Iri> = allowed.into_iter().collect();
        let mut errors = Vec::new();

        // Collect candidate IRIs to test against the allows_only set.
        // Single-value properties hold one IRI directly; resource_array
        // properties hold a `Value::Array` of IRI elements. `as_iri`
        // accepts both canonical `ResourceRef` and pre-canonical
        // `String` shapes.
        let refs_to_check: Vec<Iri> = match value {
            Value::Array(arr) => arr.iter().filter_map(|v| v.as_iri()).collect(),
            single => single.as_iri().map(|i| vec![i]).unwrap_or_default(),
        };

        for ref_iri in refs_to_check {
            {
                if !allowed_set.contains(&ref_iri) {
                    errors.push(ValidationError {
                        resource_id: res_id.clone(),
                        property: Some(prop_iri.clone()),
                        rule: ValidationRule::AllowedValueViolation,
                        message: format!("value '{ref_iri}' is not in the allows_only set"),
                    });
                }
            }
        }

        errors
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
    fn allows_only_violation() {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test", Some(core));
        // data_type has allows_only constraint — use an invalid value
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_dt",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("bad data_type".into())),
                    (wk::SHORT_NAME, Value::String("bad_dt".into())),
                    (
                        wk::DATA_TYPE_PROP,
                        Value::String("urn:eigenius:core:nonexistent".to_string()),
                    ),
                ],
            ))
            .unwrap();
        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        assert!(errors
            .iter()
            .any(|e| e.rule == ValidationRule::AllowedValueViolation));
    }
}
