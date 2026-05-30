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

//! Rule 6: Range checking — numeric values must respect their
//! `min_value` / `max_value` declarations.

use super::super::{iri, ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 6: Range checking (min_value/max_value).
    pub(in crate::validation) fn check_range(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let num_val = match value {
            Value::Integer(n) => *n as f64,
            Value::Float(f) => *f,
            _ => return vec![],
        };

        let mut errors = Vec::new();

        if let Some(Value::Float(min)) = prop_def.get(&iri(wk::MIN_VALUE)) {
            if num_val < *min {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::RangeViolation,
                    message: format!("value {num_val} is less than minimum {min}"),
                });
            }
        }
        if let Some(Value::Integer(min)) = prop_def.get(&iri(wk::MIN_VALUE)) {
            if num_val < *min as f64 {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::RangeViolation,
                    message: format!("value {num_val} is less than minimum {min}"),
                });
            }
        }

        if let Some(Value::Float(max)) = prop_def.get(&iri(wk::MAX_VALUE)) {
            if num_val > *max {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::RangeViolation,
                    message: format!("value {num_val} is greater than maximum {max}"),
                });
            }
        }
        if let Some(Value::Integer(max)) = prop_def.get(&iri(wk::MAX_VALUE)) {
            if num_val > *max as f64 {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::RangeViolation,
                    message: format!("value {num_val} is greater than maximum {max}"),
                });
            }
        }

        errors
    }
}
