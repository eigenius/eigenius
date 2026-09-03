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
//! **The gate already existed and was unreachable from here.** Admission is
//! [`check_inductive_declaration`](crate::nbe::check::check_inductive_declaration)
//! — until D76 Phase B an arm of `check_type` reached through `Exp::Inductive`,
//! which was constructed nowhere in
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
        match crate::nbe::check::check_inductive_declaration(&mut ctx, &decl) {
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

// ── D79 §2.3 — the seal ───────────────────────────────────────────────

impl Validator {
    /// **D79 §2.3 — a `core:InductiveType` declaration may not be redefined.**
    ///
    /// Same trigger as Rule 23 above (`is_a core:InductiveType`) and a different
    /// question: not *is this declaration admissible* but *may this layer declare
    /// it at all*.
    ///
    /// **Why inductives and not classes.** D76 made the chain the typing
    /// environment: it binds names to declarations, and `Env::lookup` returns one
    /// `InductiveDecl`. A class admits a monotone edit — redeclaring it with a
    /// parent added is the idiom the wordnet↔umls alignment and
    /// `claim-kind-alignment.esl` are built on. An inductive admits none:
    /// constructors have no chain-resolvable identity (`nbe::term::InductiveCtorDecl`
    /// carries a name and a type, no IRI), so redefining the type is the only way
    /// to change them and it replaces the whole constructor set at once. Every
    /// committed term mentioning the type then means something else, with nothing
    /// to detect it.
    ///
    /// **Measured before it rejected anything**, per the protocol eigenius#136
    /// earned and Rule 23 above followed: a scan of every `InductiveType`
    /// declaration across `ontologies/`, `experiments/` and `demo/` found **zero**
    /// declared in more than one file. The rule costs nothing today, which is why
    /// it lands before anything depends on it.
    ///
    /// **Byte-identical shadowing is not a redefinition, and that exemption is
    /// load-bearing rather than an optimisation.** A reseed re-loads the bootstrap
    /// chain, shadowing every `InductiveType` it declares. Without
    /// [`redefines_ancestor`]'s canonical-CBOR comparison this rule would refuse
    /// every reseed — including the one D79 P2 needs.
    pub(in crate::validation) fn check_inductive_not_redefined(
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
        let Some(iri) = res_id else {
            // Embedded — no IRI, so nothing below it to shadow.
            return vec![];
        };
        if !crate::validation::retroactive::redefines_ancestor(&self.layer, iri) {
            return vec![];
        }
        vec![ValidationError {
            resource_id: res_id.clone(),
            property: None,
            rule: ValidationRule::InductiveRedefinition,
            message: format!(
                "`{iri}` is a core:InductiveType already declared lower in the chain, and this \
                 layer redeclares it with a different body. An inductive cannot be redefined: its \
                 constructors have no identity apart from it, so this replaces the whole \
                 constructor set and silently changes what every committed term mentioning \
                 `{iri}` means. Declare a new inductive under its own IRI instead. (Classes and \
                 properties stay redefinable — this restriction is inductives only. D79 §2.3.)"
            ),
        }]
    }
}

#[cfg(test)]
mod seal_tests {
    use super::super::super::tests::{build_core_layer, make_resource};
    use super::super::super::{ValidationRule, Validator};
    use crate::layer::{Layer, LayerBuilder, LayerStorage};
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).expect("static iri")
    }

    /// A minimal `core:InductiveType` with one nullary ctor whose name is `ctor`.
    /// Varying `ctor` is what makes two versions differ.
    fn colour(ctor: &str) -> Resource {
        let c = make_resource(
            "urn:eigenius:test:Colour:c",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri(wk::INDUCTIVE_CTOR).as_str().to_string(),
                    )]),
                ),
                (wk::CTOR_NAME, Value::String(ctor.into())),
                (wk::ARG_TYPES, Value::Array(vec![])),
            ],
        );
        make_resource(
            "urn:eigenius:test:Colour",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri(wk::INDUCTIVE_TYPE).as_str().to_string(),
                    )]),
                ),
                (wk::SHORT_NAME, Value::String("Colour".into())),
                (wk::TYPE_PARAMS, Value::Array(vec![])),
                (wk::CTORS, Value::Array(vec![Value::Embedded(Box::new(c))])),
            ],
        )
    }

    fn chain(base_ctor: &str, child_ctor: Option<&str>) -> Arc<Layer> {
        let mut b = LayerBuilder::new("base", Some(build_core_layer()));
        b.add_resource(colour(base_ctor)).unwrap();
        let base = Arc::new(b.build(LayerStorage::in_memory()));
        match child_ctor {
            None => base,
            Some(c) => {
                let mut b2 = LayerBuilder::new("child", Some(base));
                b2.add_resource(colour(c)).unwrap();
                Arc::new(b2.build(LayerStorage::in_memory()))
            }
        }
    }

    fn seal_errors(layer: Arc<Layer>) -> Vec<crate::validation::ValidationError> {
        Validator::new(layer)
            .validate()
            .into_iter()
            .filter(|e| e.rule == ValidationRule::InductiveRedefinition)
            .collect()
    }

    #[test]
    fn redefining_an_inductive_with_a_different_body_is_refused() {
        let errs = seal_errors(chain("red", Some("blue")));
        assert_eq!(
            errs.len(),
            1,
            "a child layer redeclaring test:Colour with a different ctor set must be refused; \
             got {errs:?}"
        );
        assert!(
            errs[0].message.contains("cannot be redefined"),
            "the diagnostic must say why, got {:?}",
            errs[0].message
        );
    }

    /// **The exemption a reseed depends on.** Re-loading the bootstrap chain shadows
    /// every `InductiveType` it declares with a byte-identical body. If this fired,
    /// D79 P2's reseed — and every reseed after it — would be refused.
    #[test]
    fn byte_identical_shadowing_is_not_a_redefinition() {
        let errs = seal_errors(chain("red", Some("red")));
        assert!(
            errs.is_empty(),
            "shadowing an inductive with an identical body is a re-load, not a redefinition; \
             got {errs:?}"
        );
    }

    #[test]
    fn a_first_declaration_is_not_a_redefinition() {
        let errs = seal_errors(chain("red", None));
        assert!(
            errs.is_empty(),
            "declaring an inductive once is fine; got {errs:?}"
        );
    }

    /// Classes stay redefinable **on purpose** (D79 §5): redeclaring one with a
    /// parent added is the wordnet↔umls alignment idiom, and checking that hazard is
    /// D77's subject, not this rule's.
    #[test]
    fn redefining_a_class_is_still_allowed() {
        let mk = |desc: &str| {
            make_resource(
                "urn:eigenius:test:Animal",
                vec![
                    (wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::CLASS))])),
                    (wk::DESCRIPTION, Value::String(desc.into())),
                ],
            )
        };
        let mut b = LayerBuilder::new("base", Some(build_core_layer()));
        b.add_resource(mk("v1")).unwrap();
        let base = Arc::new(b.build(LayerStorage::in_memory()));
        let mut b2 = LayerBuilder::new("child", Some(base));
        b2.add_resource(mk("v2")).unwrap();
        let head = Arc::new(b2.build(LayerStorage::in_memory()));
        assert!(
            seal_errors(head).is_empty(),
            "the seal is inductives only; a redefined class must pass"
        );
    }
}

#[cfg(test)]
mod ctor_type_tests {
    //! **D79 P2's gate.** `core:ctor_type` was declared `core:json` — "an opaque
    //! JSON value. Not validated by the ontology." — so `check_type_expr_well_typed`
    //! (Rule 21) skipped it and `walk_inductive_value` (Rule 16) had no
    //! `class_types` to walk against. A constructor's arrow type could be arbitrary
    //! garbage and still commit, which is why `layer::declaration_order` had to
    //! descend into `Value::Json` by hand to find inductive-to-inductive edges.
    //!
    //! P2 declares it `core:inductive` + `class_types eigentt:Term`. These tests
    //! pin the difference: a `ctor_type` naming an IRI that does not resolve is now
    //! refused, and a well-formed one still passes.
    //!
    //! The declaration is on an **embedded** `core:InductiveCtor`, which the
    //! validator reaches through its `is_a`-gated embedded-resource recursion
    //! (`validation/mod.rs:559`). That gate keys on `is_a`, not on `@id`, which is
    //! also why D79 P4 can drop the constructor's vestigial `@id` without
    //! un-validating it.
    use super::super::super::Validator;
    use crate::layer::{LayerBuilder, LayerStorage};
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).expect("static iri")
    }

    /// An inductive `test:Box` with one ctor whose `ctor_type` is `ctor_type`.
    fn errors_for_ctor_type(ctor_type: serde_json::Value) -> Vec<String> {
        let head = Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let mut top = LayerBuilder::new("ctor_type_test", Some(head));

        let mut c = Resource::new_embedded();
        c.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(
                iri(wk::INDUCTIVE_CTOR).as_str().to_string(),
            )]),
        );
        c.set(iri(wk::CTOR_NAME), Value::String("mk".into()));
        c.set(iri(wk::ARG_TYPES), Value::Array(vec![]));
        c.set(iri(wk::CTOR_TYPE), crate::testing::term_value(&ctor_type));

        let mut b = Resource::new(iri("urn:eigenius:test:Box"));
        b.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(
                iri(wk::INDUCTIVE_TYPE).as_str().to_string(),
            )]),
        );
        b.set(iri(wk::SHORT_NAME), Value::String("Box".into()));
        // Required since `core:InductiveType subclass_of core:Class` (D85 §6.1).
        b.set(
            iri(wk::DESCRIPTION),
            Value::String("test fixture for ctor_type well-formedness".into()),
        );
        b.set(iri(wk::TYPE_PARAMS), Value::Array(vec![]));
        b.set(
            iri(wk::CTORS),
            Value::Array(vec![Value::Embedded(Box::new(c))]),
        );

        top.add_resource(b).unwrap();
        let layer = Arc::new(top.build(LayerStorage::in_memory()));
        Validator::new(layer)
            .validate()
            .into_iter()
            .map(|e| e.message)
            .collect()
    }

    /// The case that previously committed silently.
    #[test]
    fn a_ctor_type_naming_an_unresolvable_iri_is_refused() {
        let errs = errors_for_ctor_type(serde_json::json!({
            "ctor": "ConstRef", "args": ["urn:eigenius:test:no-such-type", []],
        }));
        assert!(
            !errs.is_empty(),
            "a ctor_type referencing an IRI that resolves to nothing must be refused now that \
             the property is core:inductive; under core:json this committed silently"
        );
        assert!(
            errs.iter().any(|m| m.contains("no-such-type")),
            "the diagnostic must name the unresolvable IRI, got {errs:?}"
        );
    }

    /// And the well-formed case still passes — the rule must not reject every
    /// `ctor_type`, which a `class_types` typo would produce and which would look
    /// identical from the "P2 changed something" side.
    #[test]
    fn a_well_formed_ctor_type_still_passes() {
        let errs = errors_for_ctor_type(serde_json::json!({
            "ctor": "ConstRef", "args": ["urn:eigenius:test:Box", []],
        }));
        assert!(
            errs.is_empty(),
            "a ctor_type naming its own inductive is well-formed; got {errs:?}"
        );
    }
}
