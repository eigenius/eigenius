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

//! Per-rule validation modules. Each file owns one D1 §5.4 rule plus
//! its directly-related helpers, with rule tests living alongside the
//! rule they exercise. The Validator's driver loop and shared helpers
//! remain in `validation/mod.rs`; the rule modules extend `Validator`
//! with `impl Validator { ... }` blocks split across files.

pub(super) mod allows_only;
pub(super) mod class_types;
mod closed_inductive;
pub(super) mod conditional;
pub(super) mod domain;
pub(super) mod eigentt_value;
pub(super) mod format;
pub(super) mod inductive;
pub(super) mod inductive_decl;
pub(super) mod is_a;
pub(super) mod length;
pub(super) mod pattern;
pub(super) mod range;
pub(super) mod reference_integrity;
pub(super) mod type_check;

/// Rule 23 — well-foundedness of a conclusion's justification.
///
/// A premise's support may not transitively include the premise. The condition is
/// vacuous on anything with no support to inspect, which is the carve-out Artemov's
/// constant specifications require: `c : A(c)` is sound as a POSTULATE, and postulated
/// self-reference is strictly necessary for realizing certain S4 theorems in LP. Only
/// DERIVED circularity has a support graph, and only derived circularity is unsound.
impl super::Validator {
    pub(in crate::validation) fn check_well_founded(
        &self,
        resource: &crate::ontology::resource::Resource,
        res_id: &Option<crate::ontology::iri::Iri>,
    ) -> Vec<super::ValidationError> {
        let Some(iri) = res_id else {
            return vec![];
        };
        if !resource
            .is_a()
            .iter()
            .any(|c| c.as_str() == "urn:eigenius:justification:Conclusion")
        {
            return vec![];
        }
        match crate::justification::wellfounded::check(&self.layer, iri) {
            Ok(()) => vec![],
            Err(e) => vec![super::ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: super::ValidationRule::NotWellFounded,
                message: e.to_string(),
            }],
        }
    }
}
