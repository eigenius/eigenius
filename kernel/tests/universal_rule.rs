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

//! End-to-end D39 v2 demo: a universally-quantified literature rule
//! applied to a specific compound via `justification:Certificate.spec_poly`
//! constructor.
//!
//! Closes the conceptual gap in the original `drug_screening.esl`
//! fixture: the rule is now universal (`forall c, HasLowIC50(c) ->
//! StrongInhibitor(c)`) rather than pre-specialised to EIG_0291.
//! The certificate uses the `spec_poly`
//! justification:Certificate ctor to apply the rule at "urn:EIG_0291"; the kernel's
//! NbE beta-reduces `(forall c, P c)("urn:EIG_0291")` to
//! `P("urn:EIG_0291")` so the result type matches the App composition.
//!
//! What this exercises:
//! - The `lower_type_expr_to_exp` bound-variable-with-args fix:
//!   `screen:HasLowIC50(c)` inside a forall body lowers cleanly.
//! - The `justification:Certificate.spec_poly` constructor in `justification.esl`.
//!   It leaves the justification TERM unchanged — narrowing a universal to an
//!   instance changes the proposition and introduces no ground — so there is no
//!   matching term constructor. `SpecStr(j, tag)` was that constructor, and its
//!   tag was the only unchecked argument in the algebra.
//! - Kernel beta-reduction at the spec_poly result type during
//!   certificate type-checking.
//! - End-to-end `Verdict::Holds` from a chain author using the
//!   universal rule shape that real literature rules actually have.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;

fn build_universal_rule_chain() -> ExecutionContext {
    let core_json = include_str!("../../ontologies/core/core-ontology.json");
    let core_resources = eigon_json::parse_document(core_json).unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let reflection_json = include_str!("../../ontologies/reflection/reflection-ontology.json");
    let reflection_resources = eigon_json::parse_document(reflection_json).unwrap();
    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for r in reflection_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let eigentt_json = include_str!("../../ontologies/eigentt/eigentt-type-fragment.json");
    let eigentt_resources = eigon_json::parse_document(eigentt_json).unwrap();
    for r in eigentt_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let institution_json = include_str!("../../ontologies/institution/institution-ontology.json");
    let institution_resources = eigon_json::parse_document(institution_json).unwrap();
    for r in institution_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    let reasoning_source = include_str!("../../ontologies/justification/justification.esl");
    let reasoning_resources =
        esl::compile(reasoning_source, &eigenius_kernel::layer::Layer::empty())
            .expect("reasoning.esl compiles");
    // `prov` (P5). These fixtures carry `prov:` properties and trace classes; without this
    // layer none of them resolve, and the chain reports a dozen `UnresolvedClassReference`s
    // that no assertion here was looking at. Sits above `reflection` and below
    // `justification`, matching `BOOTSTRAP_CHAIN`.
    let prov_resources = esl::compile(include_str!("../../ontologies/prov/prov.esl"), &reflection)
        .expect("prov.esl compiles");
    let mut prov_builder = LayerBuilder::new("prov", Some(reflection));
    for r in prov_resources {
        prov_builder.add_resource(r).unwrap();
    }
    let prov = Arc::new(prov_builder.build(LayerStorage::in_memory()));

    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(prov));
    for r in reasoning_resources {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    let fixture_source = include_str!("fixtures/universal_rule.esl");
    let fixture_resources = esl::compile(fixture_source, &reasoning).unwrap_or_else(|errs| {
        panic!(
            "universal_rule.esl failed to compile: {}",
            errs.into_iter()
                .map(|e| format!("{e:?}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    let mut fixture_builder = LayerBuilder::new("universal-rule-demo", Some(reasoning));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        "universal-rule-demo",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn universal_rule_with_spec_poly_validates_to_holds() {
    let ctx = build_universal_rule_chain();

    let sentence_iri =
        Iri::parse("urn:eigenius:demo:screen:concl_eig0291_strong").expect("sentence IRI");
    ctx.resolve(&sentence_iri)
        .unwrap_or_else(|| panic!("sentence `{sentence_iri}` should be on the chain"));

    // The check is Rule 21's since P2 moved it to commit: it decodes the judgement, checks
    // its `type` is a type, and checks its `term` against it. Read the errors it reports for
    // this conclusion; an empty set is what the handler used to report as `Holds`.
    let diagnostic = eigenius_kernel::validation::Validator::new(ctx.head().clone())
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref().is_some_and(|i| *i == sentence_iri))
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join(
            "
",
        );

    assert!(
        diagnostic.is_empty(),
        "expected Holds for the universal rule + spec_poly certificate; \
         got: {diagnostic}"
    );
}
