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

//! **Rule 23 — strict positivity of inductive declarations (eigenius#92).**
//!
//! A `core:InductiveType` resource is a *declaration*, and a declaration whose constructors are
//! not strictly positive admits a fixpoint that inhabits every proposition. The kernel is the
//! commit gate's felicity oracle, so an unsound declaration reaching a chain is not a local
//! problem: everything validated against that chain afterwards inherits it.
//!
//! **The checker already existed and was unreachable from here.**
//! [`crate::nbe::positivity::check_positivity`] runs from `check_type`'s `Exp::Inductive` arm,
//! which is the TERM form — a `data` declaration written in ESL becomes a resource carrying
//! `core:params` / `core:ctors`, never an `Exp::Inductive`, so nothing in the commit path ever
//! called it. `grep 'Exp::Inductive(' kernel/src/esl/compile.rs` returns nothing. That is why
//! eigenius#92's probe reported zero errors from `Validator::validate()` while the probe's own
//! declaration was, by the checker's then-criterion, inadmissible.
//!
//! This rule is the missing edge: resolve the resource to an `InductiveDecl` exactly as every
//! consumer does, and run the same function.
//!
//! **Measured before it rejected anything** (`2026-08-22`, the protocol eigenius#136 earned): over
//! the bootstrap chain, 42 `core:InductiveType` resources, 42 admitted, 0 decode failures. Three
//! constructors — `lexicon:Cat`'s `cat_forall`, `cat_fin_forall` and `cat_num_forall` — are
//! higher-order positive, so under the criterion in force before eigenius#92 this rule would have
//! rejected `ontologies/lexicon/lexicon-ontology.esl` and the bootstrap would not load. Widening
//! the criterion is what made the routing possible, not a convenience alongside it.
//!
//! **A resource that does not decode is skipped, not reported.** Positivity is a property of a
//! declaration; a resource that cannot be read as one has a different defect, and the decode
//! diagnostic belongs to whichever rule owns that shape. Reporting it here would give one
//! malformed resource two unrelated errors, the second of them misleading. The measurement found
//! no such resource on the bootstrap chain, so this is a guard against a shape that does not
//! currently occur.

use super::super::{ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 23: every `core:InductiveType` declaration is strictly positive.
    pub(in crate::validation) fn check_inductive_positivity(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        if !resource
            .is_a()
            .iter()
            .any(|c| c.as_str() == wk::INDUCTIVE_TYPE)
        {
            return vec![];
        }
        let Some(class_iri) = res_id else {
            // An embedded inductive declaration has no IRI to resolve against; the decode path
            // keys on one. Nothing on any chain today declares an inductive this way.
            return vec![];
        };
        let val = match crate::program::ground::resolve_inductive_type(
            class_iri,
            resource,
            &self.layer,
        ) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        let crate::nbe::val::Val::InductiveType { decl, .. } = val else {
            return vec![];
        };
        match crate::nbe::positivity::check_positivity(&decl) {
            Ok(()) => vec![],
            Err(message) => vec![ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: ValidationRule::NonPositiveInductive,
                message,
            }],
        }
    }
}
