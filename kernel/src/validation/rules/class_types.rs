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

//! Rule 8: Class-type checking. Class-typed properties (`data_type:
//! resource[_array]` with `class_types` declared) restrict their values
//! to instances of the declared classes. When `class_types` instead
//! references an `InductiveType` (Option A unification), values are
//! walked as inductive trees.

use super::super::{
    format_iri_refs, format_is_a_list, iri, ValidationError, ValidationRule, Validator,
};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 8: Class type checking.
    ///
    /// `class_types` may name either a `Class` (the historical case —
    /// the value must be a ResourceRef/Embedded whose `is_a` matches)
    /// or an `InductiveType` (Option A unification — the value is a
    /// tagged-dict tree carried by `Value::Json`, and we dispatch to
    /// the inductive walker). Per the singleton constraint that
    /// already applies to `data_type: core:inductive`, an
    /// InductiveType `class_types` must be the sole entry; mixed
    /// Class/InductiveType lists are not a defined shape.
    pub(in crate::validation) fn check_class_types(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let allowed_classes = match prop_def.get(&iri(wk::CLASS_TYPES)) {
            Some(val) => val.as_iri_array(),
            None => return vec![],
        };

        if allowed_classes.is_empty() {
            return vec![];
        }

        // InductiveType branch: walk the tagged-dict tree(s).
        // Skipping non-Json elements here is intentional — wire-shape
        // (`check_type`) handles whether a Ref/Embedded is admissible
        // for the property's data_type; deep class-membership of
        // stored inductive instances is not a v1 use case.
        //
        // For `data_type: core:inductive`, `check_inductive_value`
        // (rule 16) owns the walk + the singleton precondition; we
        // defer to it to avoid duplicate diagnostics.
        let dt_is_inductive = self
            .get_data_type_str(prop_def)
            .map(|dt| dt == wk::INDUCTIVE)
            .unwrap_or(false);
        if !dt_is_inductive {
            if let Some(inductive_type) = self.class_types_inductive_target(prop_def) {
                let mut errors = Vec::new();
                match value {
                    Value::Json(_) => {
                        self.walk_inductive_value(
                            value,
                            &inductive_type,
                            prop_iri.as_str().to_string(),
                            res_id,
                            &mut errors,
                        );
                    }
                    Value::Array(arr) => {
                        for (i, v) in arr.iter().enumerate() {
                            if matches!(v, Value::Json(_)) {
                                let path = format!("{prop_iri}[{i}]");
                                self.walk_inductive_value(
                                    v,
                                    &inductive_type,
                                    path,
                                    res_id,
                                    &mut errors,
                                );
                            }
                        }
                    }
                    _ => {}
                }
                return errors;
            }
        }

        let allowed_refs: Vec<&Iri> = allowed_classes.iter().collect();

        let mut errors = Vec::new();
        let values_to_check = match value {
            Value::String(_) | Value::ResourceRef(_) | Value::Embedded(_) => vec![value],
            Value::Array(arr) => arr.iter().collect(),
            _ => return vec![],
        };

        for v in values_to_check {
            // Embedded resources are checked directly against the
            // allowed-class set; IRI references (in either canonical
            // `ResourceRef` or pre-canonical `String` shape) are
            // resolved through the chain first.
            if let Value::Embedded(embedded) = v {
                if !self.is_instance_of_any(embedded, &allowed_refs) {
                    let actual = format_is_a_list(embedded.is_a());
                    let allowed = format_iri_refs(&allowed_refs);
                    errors.push(ValidationError {
                        resource_id: res_id.clone(),
                        property: Some(prop_iri.clone()),
                        rule: ValidationRule::ClassTypeMismatch,
                        message: format!(
                            "embedded value on property '{prop_iri}' has is_a {actual} but must be an instance of one of: {allowed}"
                        ),
                    });
                }
                continue;
            }
            if let Some(ref_iri) = v.as_iri() {
                if let Some(referenced) = self.layer.resolve(&ref_iri) {
                    if !self.is_instance_of_any(&referenced, &allowed_refs) {
                        let actual = format_is_a_list(referenced.is_a());
                        let allowed = format_iri_refs(&allowed_refs);
                        errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: Some(prop_iri.clone()),
                            rule: ValidationRule::ClassTypeMismatch,
                            message: format!(
                                "referenced resource '{ref_iri}' has is_a {actual} but must be an instance of one of: {allowed}"
                            ),
                        });
                    }
                }
                // If we can't resolve, skip — might be external.
            }
        }

        errors
    }
}
