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

//! **Rule 25 — an inductive stays closed** (D85 §6.1).
//!
//! An inductive type is CLOSED: its constructors are exactly the entries in `core:ctors`. That
//! is what makes exhaustiveness checking sound (`non-exhaustive match: missing case for …`,
//! [`program::expr`]), the eliminator total, and "no user-constructible inhabitant" true of the
//! zero-constructor witness types. A `core:Class` with `subclass_of` is OPEN: any later layer
//! may add one.
//!
//! D85 §6.1 gives a constructor a class, which borrows the open mechanism for a closed concept.
//! Before it, closedness was STRUCTURAL — `core:ctors` holds embedded resources with no `@id`,
//! so there was nowhere to add a constructor. Deriving the classes
//! ([`crate::layer`]'s `ctor_classes`) keeps it structural on the normal path, because nothing
//! authors one. This rule answers the case that remains: a class someone writes by hand.
//!
//! Two-sided, and both sides are needed:
//!
//! 1. **Same layer.** The class must be declared in the layer that declares the inductive. A
//!    lower layer cannot reference a higher one, so same-layer is the only locality that admits
//!    anything at all; the content of the rule is the refusal of every layer ABOVE, which is
//!    exactly where a later author would add one.
//! 2. **Named by `core:ctors`.** Its IRI must be `<inductive>-<ctor_name>` for a constructor the
//!    inductive declares. This is the load-bearing half: `core:ctors` stays the authority, and it
//!    is what exhaustiveness already reads, so a class cannot introduce a constructor the
//!    eliminator does not know about even inside the declaring layer.
//!
//! Without both, §6.1 converts a closed type into an open one silently — a value carrying
//! `is_a: [eigentt:Term-Bogus]` would satisfy every slot declaring `class_types eigentt:Term`,
//! because subsumption walks `subclass_of`, while no match arm covers it and no eliminator
//! handles it.

use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::ontology::well_known::iri;

use super::super::{ValidationError, ValidationRule, Validator};

impl Validator {
    /// Rule 25 — see the module docs.
    pub(in crate::validation) fn check_inductive_closure(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let Some(parents) = resource.get(&iri(wk::PARENT_CLASSES)) else {
            return vec![];
        };
        let inductive_type = iri(wk::INDUCTIVE_TYPE);
        let mut errors = Vec::new();

        for parent_iri in parents.as_iri_array() {
            let Some(parent) = self.layer.resolve(&parent_iri) else {
                continue; // reference integrity is Rule 22's job, not this one
            };
            if !parent.is_a().contains(&inductive_type) {
                continue; // an ordinary class parent — open, and none of this rule's business
            }

            // (1) The inductive must be declared in THIS layer, not merely resolvable through
            // the chain. `get_resource` is layer-local where `resolve` walks the parents.
            if self.layer.get_resource(&parent_iri).is_none() {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(iri(wk::PARENT_CLASSES)),
                    rule: ValidationRule::InductiveNotClosed,
                    message: format!(
                        "subclass_of: '{parent_iri}' is an inductive type declared in a LOWER \
                         layer. A constructor class may only be declared in the layer that \
                         declares its inductive — an inductive is closed, and admitting one from \
                         a later layer would add a constructor no eliminator handles"
                    ),
                });
                continue;
            }

            // (2) It must name a constructor the inductive declares.
            let expected_prefix = format!("{parent_iri}-");
            let named = res_id
                .as_ref()
                .and_then(|i| i.as_str().strip_prefix(&expected_prefix))
                .map(str::to_string);
            let declared: Vec<String> = match parent.get(&iri(wk::CTORS)) {
                Some(Value::Array(cs)) => cs
                    .iter()
                    .filter_map(|c| match c {
                        Value::Embedded(r) => r
                            .get(&iri(wk::CTOR_NAME))
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            match named {
                Some(n) if declared.contains(&n) => {}
                _ => errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(iri(wk::PARENT_CLASSES)),
                    rule: ValidationRule::InductiveNotClosed,
                    message: format!(
                        "subclass_of: '{parent_iri}' is an inductive type, so this class must be \
                         one of its constructors — named '{parent_iri}-<ctor>' for a ctor in its \
                         core:ctors, which are [{}]. An inductive's constructors are exactly its \
                         core:ctors; a class outside that set would be a constructor no \
                         eliminator handles",
                        declared.join(", ")
                    ),
                }),
            }
        }
        errors
    }
}
