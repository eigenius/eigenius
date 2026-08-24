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

//! **Rule 23 — a `core:InductiveType` declaration is admissible (eigenius#92, eigenius#188).**
//!
//! A `core:InductiveType` resource is a *declaration*. An inadmissible declaration is not a local
//! problem: the kernel is the commit gate's felicity oracle, so everything validated against a
//! chain that carries one inherits the defect. Strict positivity is the sharpest case — a
//! non-positive constructor admits a fixpoint that inhabits every proposition — but it is not the
//! only one, and this rule is not a positivity rule. It is the edge from the commit path to the
//! kernel's declaration gate.
//!
//! **The gate already existed and was unreachable from here.** `check_type`'s `Exp::Inductive` arm
//! is where a declaration is admitted, and `Exp::Inductive(` is constructed nowhere in
//! `kernel/src/esl/compile.rs` — a `data` declaration written in ESL becomes a resource carrying
//! `core:type_params` / `core:ctors`, so nothing in the commit path ever called it. That is why
//! eigenius#92's probe reported zero errors from `Validator::validate()` while the probe's own
//! declaration was, by the checker's then-criterion, inadmissible.
//!
//! **The rule calls `check_type`, not the individual checks it performs.** Listing the arm's
//! components here would be a second definition of "admissible declaration" for this rule and the
//! kernel to drift apart on — the failure mode N1 §3 names, and the reason
//! `nbe::positivity::recursive_arg_shape` exists. When the arm gains a check, this rule enforces it
//! with no edit.
//!
//! **Measured before it rejected anything** (`2026-08-22`, the protocol eigenius#136 earned): over
//! the bootstrap chain, 42 `core:InductiveType` resources, 42 admitted, 0 decode failures. Three
//! constructors — `lexicon:Cat`'s `cat_forall`, `cat_fin_forall` and `cat_num_forall` — are
//! higher-order positive, so under the criterion in force before eigenius#92 this rule would have
//! rejected `ontologies/lexicon/lexicon-ontology.esl` and the bootstrap would not load. Widening
//! the criterion is what made the routing possible, not a convenience alongside it.
//!
//! **A resource that does not decode is skipped, not reported.** Admissibility is a property of a
//! declaration; a resource that cannot be read as one has a different defect, and the decode
//! diagnostic belongs to whichever rule owns that shape. Reporting it here would give one
//! malformed resource two unrelated errors, the second of them misleading.

use super::super::{ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;

impl Validator {
    /// Rule 23: every `core:InductiveType` declaration is admissible to the kernel.
    pub(in crate::validation) fn check_inductive_declaration(
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
        let mut ctx = crate::nbe::check::CheckCtx::with_layer(
            crate::nbe::env::Rho::Nil,
            Vec::new(),
            std::sync::Arc::clone(&self.layer),
        );
        let decl_exp = crate::nbe::term::Exp::Inductive(decl);
        match crate::nbe::check::check_type(&mut ctx, &decl_exp) {
            Ok(()) => vec![],
            Err(e) => vec![ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: ValidationRule::InductiveDeclInadmissible,
                message: e.to_string(),
            }],
        }
    }
}
