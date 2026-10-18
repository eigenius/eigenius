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

//! Rule 5: Pattern checking — string values must satisfy the `pattern`
//! regex declared on their property. Pattern is matched as a full-string
//! anchor.

use super::super::{iri, ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 5: Pattern checking.
    pub(in crate::validation) fn check_pattern(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let pattern_str = match prop_def.get(&iri(wk::PATTERN)) {
            Some(Value::String(s)) => s.as_str(),
            _ => return vec![],
        };

        let string_val = match value.as_str() {
            Some(s) => s,
            None => return vec![],
        };

        // Full match: wrap in ^...$
        let full_pattern = format!("^(?:{pattern_str})$");
        match regex::Regex::new(&full_pattern) {
            Ok(re) => {
                if re.is_match(string_val) {
                    vec![]
                } else {
                    vec![ValidationError {
                        resource_id: res_id.clone(),
                        property: Some(prop_iri.clone()),
                        rule: ValidationRule::PatternViolation,
                        message: format!(
                            "value '{string_val}' does not match pattern '{pattern_str}'"
                        ),
                    }]
                }
            }
            Err(_) => vec![], // Invalid regex in property def — skip (should be caught by format validation on the property itself)
        }
    }
}
