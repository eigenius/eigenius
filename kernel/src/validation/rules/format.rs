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

//! Rule 4: Format checking — string values bearing a `format` annotation
//! (date / datetime / time / iri / uuid / regex) must satisfy the
//! corresponding shape predicate.

use super::super::{iri, ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 4: Format checking.
    pub(in crate::validation) fn check_format(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let format_str = match prop_def.get(&iri(wk::FORMAT_PROP)) {
            Some(Value::String(s)) => s.as_str(),
            _ => return vec![],
        };

        let string_val = match value.as_str() {
            Some(s) => s,
            None => return vec![], // Not a string, type checking handles this
        };

        let valid = match format_str {
            wk::FMT_DATE => is_valid_date(string_val),
            wk::FMT_DATETIME => is_valid_datetime(string_val),
            wk::FMT_TIME => is_valid_time(string_val),
            wk::FMT_IRI => Iri::parse(string_val).is_ok(),
            wk::FMT_UUID => is_valid_uuid(string_val),
            wk::FMT_REGEX => regex::Regex::new(string_val).is_ok(),
            _ => true, // Unknown format, skip
        };

        if valid {
            vec![]
        } else {
            vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::FormatViolation,
                message: format!("value '{string_val}' does not match format '{format_str}'"),
            }]
        }
    }
}

// --- Format validation helpers ---

pub(in crate::validation) fn is_valid_date(s: &str) -> bool {
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    if !re.is_match(s) {
        return false;
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    let year: u32 = parts[0].parse().unwrap_or(0);
    let month: u32 = parts[1].parse().unwrap_or(0);
    let day: u32 = parts[2].parse().unwrap_or(0);
    (1..=9999).contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day)
}

pub(in crate::validation) fn is_valid_datetime(s: &str) -> bool {
    // Accept ISO 8601 with timezone: YYYY-MM-DDTHH:MM:SSZ or +HH:MM
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$")
        .unwrap();
    re.is_match(s)
}

pub(in crate::validation) fn is_valid_time(s: &str) -> bool {
    let re = regex::Regex::new(r"^\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?$").unwrap();
    re.is_match(s)
}

pub(in crate::validation) fn is_valid_uuid(s: &str) -> bool {
    let re = regex::Regex::new(
        r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
    )
    .unwrap();
    re.is_match(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_date_validation() {
        assert!(is_valid_date("2026-04-11"));
        assert!(!is_valid_date("2026-13-01"));
        assert!(!is_valid_date("not-a-date"));
    }

    #[test]
    fn format_datetime_validation() {
        assert!(is_valid_datetime("2026-04-11T14:30:00Z"));
        assert!(is_valid_datetime("2026-04-11T14:30:00+05:30"));
        assert!(!is_valid_datetime("2026-04-11"));
    }

    #[test]
    fn format_uuid_validation() {
        assert!(is_valid_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_valid_uuid("not-a-uuid"));
    }
}
