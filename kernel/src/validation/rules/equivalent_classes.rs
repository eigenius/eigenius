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

//! Rule 26 — a `core:EquivalentClasses` lists at least two classes, and they require the same
//! properties (D99 §11).
//!
//! An equivalence makes an instance of any listed class inhabit every other one. If one required a
//! property another did not, an instance would inhabit a class whose obligations it never met. The
//! required set compared is each class's RECORD — its own and inherited `requires` — which an
//! equivalence never changes: it acts on subsumption, not on definitions.

use std::collections::BTreeSet;

use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;
use crate::ontology::well_known::iri;

use super::super::{ValidationError, ValidationRule, Validator};

impl Validator {
    /// Rule 26 — see the module docs.
    pub(in crate::validation) fn check_equivalent_classes(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let class = iri(wk::EQUIVALENT_CLASSES);
        if !resource.is_a().iter().any(|c| c == &class) {
            return vec![];
        }
        let members_prop = iri(wk::EQUIVALENT_CLASSES_MEMBERS);
        let members: Vec<Iri> = resource
            .get(&members_prop)
            .map(|v| v.as_iri_array())
            .unwrap_or_default();
        let error = |message: String| ValidationError {
            resource_id: res_id.clone(),
            property: Some(members_prop.clone()),
            rule: ValidationRule::EquivalenceUnsound,
            message,
        };
        if members.len() < 2 {
            return vec![error(format!(
                "an EquivalentClasses lists {} class(es); an equivalence needs at least two",
                members.len()
            ))];
        }
        let required = |c: &Iri| {
            let mut fields: BTreeSet<Iri> = BTreeSet::new();
            self.extend_with_record_fields(c, &mut fields);
            fields
        };
        let first = required(&members[0]);
        members[1..]
            .iter()
            .filter_map(|m| {
                let other = required(m);
                (other != first).then(|| {
                    error(format!(
                        "{} and {m} require different properties ({first:?} vs {other:?}); an \
                         instance of one would inhabit the other without its obligations",
                        members[0]
                    ))
                })
            })
            .collect()
    }
}
