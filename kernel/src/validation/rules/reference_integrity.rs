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

//! Rule 22: **reference integrity**. Every IRI a resource references must resolve to a
//! resident of the layer chain — the committing layer or below. The committing layer's
//! own resources count, so intra-layer forward references (A → B, both new) are fine;
//! only a reference that resolves *nowhere* is rejected.
//!
//! This closes the open-world "skip — might be external" hole (formerly in
//! [`super::class_types`]): a `core:resource` value, or an `is_a` class, that points at
//! nothing is a *broken* reference, not a deferred one — a typo committed cleanly and
//! only blew up later. With this enforced, the layer model's invariant holds — a
//! resource can only reference same-or-lower — which in turn means a brand-new IRI has
//! no lower dependents, so retroactive validation can scope to redefinitions/tombstones
//! instead of scanning every new IRI.
//!
//! Scope: `is_a` targets (must resolve to a `core:Class` / `core:InductiveType`) and
//! every value of a `core:resource` / `core:resource_array` property (must resolve to
//! some chain resident). Definition-slot references on Class/Property resources
//! (`subclass_of` / `requires` / `recommends` / `class_types` / `data_type`) are owned
//! by Rule 14 ([`super::is_a::Validator::check_class_definition_references`]); embedded
//! (inline) values carry no IRI and are validated structurally elsewhere.
//!
//! **No exemptions.** Every reference resolves uniformly: built-in types are committed
//! as resources (so `is_a core:Macro` resolves), and cross-layer "applies-to" facts are
//! expressed as layer-local redefinitions (so `core:ctor_name`'s `domain` references
//! only same-or-lower in each layer). A failure here is a slip-through to fix at its
//! source, not a special case to wave through.

use super::super::{ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

impl Validator {
    pub(in crate::validation) fn check_reference_integrity(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let class_iri = Iri::parse(wk::CLASS).expect("core:Class iri");
        let inductive_iri = Iri::parse(wk::INDUCTIVE_TYPE).expect("core:InductiveType iri");
        let is_a_prop = Iri::parse(wk::IS_A).expect("core:is_a iri");

        // (a) `is_a`: every declared class must resolve to a Class / InductiveType.
        for target in resource.is_a() {
            match self.layer.resolve(&target) {
                Some(t) if t.is_instance_of(&class_iri) || t.is_instance_of(&inductive_iri) => {}
                Some(_) => errors.push(unresolved(
                    res_id,
                    &is_a_prop,
                    format!(
                        "is_a target '{target}' resolves to a resource that is not a core:Class"
                    ),
                )),
                None => errors.push(unresolved(
                    res_id,
                    &is_a_prop,
                    format!(
                        "is_a target '{target}' does not resolve to any resource in the layer chain"
                    ),
                )),
            }
        }

        // (b) Every value of a resource-typed property must resolve to a chain resident.
        // The definition-slot references on Class/Property resources (`subclass_of` /
        // `requires` / `recommends` / `class_types` / `data_type`) are owned by Rule 14
        // — skip them here so a dangling definition reference is reported once, not twice.
        let rule14_owned: [&str; 5] = [
            wk::PARENT_CLASSES, // subclass_of
            wk::REQUIRES,
            wk::RECOMMENDS,
            wk::CLASS_TYPES,
            wk::DATA_TYPE_PROP,
        ];
        for (prop_iri, value) in resource.properties() {
            if *prop_iri == is_a_prop {
                continue; // handled in (a)
            }
            if rule14_owned.iter().any(|p| prop_iri.as_str() == *p) {
                continue; // owned by Rule 14
            }
            let Some(prop_def) = self.layer.resolve(prop_iri) else {
                continue; // unknown property — Rule 12 open-world; not our concern
            };
            let Some(dt) = self.get_data_type_str(&prop_def) else {
                continue;
            };
            if dt != wk::RESOURCE && dt != wk::RESOURCE_ARRAY {
                continue; // not a reference-typed property
            }
            for target in iris_of(value) {
                if self.layer.resolve(&target).is_none() {
                    errors.push(unresolved(
                        res_id,
                        prop_iri,
                        format!(
                            "property '{prop_iri}' references '{target}', which does not resolve \
                             to any resource in the layer chain"
                        ),
                    ));
                }
            }
        }

        // (c) Property-key existence. Open-world (Rule 12) lets a resource carry
        // properties beyond the requires/recommends sets of its `is_a` classes — but a
        // property IRI used as a *key* must still be DEFINED in the system: it must
        // resolve to a `core:Property` same-or-lower (the committing layer counts, so a
        // property declared in the same layer being validated is fine). A key that
        // resolves to nothing is a typo / dangling reference, not an open-world extra —
        // the same broken-reference class as (a)/(b). Enforcing this makes the invariant
        // "a layer only writes property keys known at its commit" real, which lets
        // retroactive validation skip brand-new properties (a property no lower layer
        // could have written has no lower carriers to revalidate).
        let property_class = Iri::parse(wk::PROPERTY).expect("core:Property iri");
        for prop_iri in resource.properties().keys() {
            match self.layer.resolve(prop_iri) {
                Some(def) if def.is_instance_of(&property_class) => {}
                Some(_) => errors.push(unresolved(
                    res_id,
                    prop_iri,
                    format!("property key '{prop_iri}' resolves to a resource that is not a core:Property"),
                )),
                None => errors.push(unresolved(
                    res_id,
                    prop_iri,
                    format!("property key '{prop_iri}' is not defined as a core:Property in the layer chain"),
                )),
            }
        }

        errors
    }
}

/// Build an `UnresolvedClassReference` error (reused so dangling references ride the
/// same cascade-tombstone policy as Rule 14's definition-reference failures).
fn unresolved(res_id: &Option<Iri>, property: &Iri, message: String) -> ValidationError {
    ValidationError {
        resource_id: res_id.clone(),
        property: Some(property.clone()),
        rule: ValidationRule::UnresolvedClassReference,
        message,
    }
}

/// The IRI values of a resource-typed property value: a single ref, or each element of
/// an array. Embedded (inline) values carry no IRI and are skipped.
fn iris_of(value: &Value) -> Vec<Iri> {
    match value {
        Value::Array(_) => value.as_iri_array(),
        _ => value.as_iri().into_iter().collect(),
    }
}
