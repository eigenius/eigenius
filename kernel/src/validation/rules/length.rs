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

//! Rule 7: Length checking — string lengths (chars, not bytes) and
//! array lengths must respect `min_length` / `max_length`.

use super::super::{iri, ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 7: Length checking (min_length/max_length).
    pub(in crate::validation) fn check_length(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let len = match value {
            Value::String(s) => s.chars().count(),
            Value::Array(arr) => arr.len(),
            _ => return vec![],
        };

        let mut errors = Vec::new();

        if let Some(Value::Integer(min)) = prop_def.get(&iri(wk::MIN_LENGTH)) {
            if (len as i64) < *min {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::LengthViolation,
                    message: format!("length {len} is less than minimum {min}"),
                });
            }
        }

        if let Some(Value::Integer(max)) = prop_def.get(&iri(wk::MAX_LENGTH)) {
            if (len as i64) > *max {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::LengthViolation,
                    message: format!("length {len} is greater than maximum {max}"),
                });
            }
        }

        errors
    }
}
