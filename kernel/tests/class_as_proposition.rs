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

//! A class is not a proposition (eigenius#191).
//!
//! `justification:Certificate.declared` binds `P : Prop`, so its second argument
//! is checked against `Sort(0)`. Check mode carried
//! `(Exp::EigonClass(_), Val::Sort(_)) => Ok(())`, admitting an `EigonClass`
//! against every universe including `Prop`, while `check_infer` gives
//! `Sort(1)`. A certificate could therefore name a class where its
//! proposition belongs and the sentence validated as Holds.
//!
//! Same chain and handler as `spec_poly_universe.rs`, so the judgement is
//! exercised where Rule 21 exercises it, not only at the `check` API.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;

/// core + reflection/eigentt/institution + `reasoning.esl` + the fixture.
fn build_chain() -> ExecutionContext {
    let core_json = include_str!("../../ontologies/core/core-ontology.json");
    let mut core_builder = LayerBuilder::new("core", None);
    for r in eigon_json::parse_document(core_json).unwrap() {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for src in [
        include_str!("../../ontologies/reflection/reflection-ontology.json"),
        include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
        include_str!("../../ontologies/institution/institution-ontology.json"),
    ] {
        for r in eigon_json::parse_document(src).unwrap() {
            reflection_builder.add_resource(r).unwrap();
        }
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

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

    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(Arc::clone(&prov)));
    for r in esl::compile(
        include_str!("../../ontologies/justification/justification.esl"),
        &prov,
    )
    .expect("reasoning.esl compiles")
    {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    let fixture_resources = esl::compile(
        include_str!("fixtures/class_as_proposition.esl"),
        &reasoning,
    )
    .unwrap_or_else(|errs| {
        panic!(
            "fixture failed to compile: {}",
            errs.into_iter()
                .map(|e| format!("{e:?}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    let mut fixture_builder = LayerBuilder::new("class-as-prop-demo", Some(reasoning));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        "class-as-prop-demo",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

/// Every validation error the chain reports for one of the fixture's `justification:Conclusion`s.
///
/// This read a `Verdict` off `do_validate_justification`. That handler no longer owns the
/// check — P2 moved it to commit, and `validate.rs` says so: *"the pairing this handler used
/// to check by hand is checked at commit by the uniform check-mode rule."* Rule 21 decodes the
/// judgement, checks its `type` is a type and its `term` against it, which is the same check.
fn judgement_errors(local_name: &str) -> Vec<String> {
    let ctx = build_chain();
    let sentence_iri = format!("urn:eigenius:demo:screen:{local_name}");
    eigenius_kernel::validation::Validator::new(ctx.head().clone())
        .validate()
        .into_iter()
        .filter(|e| {
            e.resource_id
                .as_ref()
                .is_some_and(|i| i.as_str() == sentence_iri)
        })
        .map(|e| e.message)
        .collect()
}

/// A certificate naming a class in its `P : Prop` slot is rejected.
#[test]
fn a_class_does_not_discharge_a_prop_obligation() {
    let errors = judgement_errors("concl_class");
    assert!(
        !errors.is_empty(),
        "`declared(iri, screen:CellLine, _)` puts a class in a `P : Prop` slot and must be refused"
    );
    let diagnostic = errors.join("\n");
    assert!(
        diagnostic.contains("universe stratification"),
        "expected a universe-stratification diagnostic, got: {diagnostic}"
    );
}

/// The same shape with a genuine `Prop`-sorted inductive still validates.
#[test]
fn a_proposition_still_discharges_a_prop_obligation() {
    let errors = judgement_errors("concl_prop");
    assert!(
        errors.is_empty(),
        "a Prop-sorted inductive in the same slot must still hold; got {errors:#?}"
    );
}
