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
//! `reasoning:JustifiedBy.declared` binds `P : Prop`, so its second argument
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
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::ReasoningInstitution;

/// core + reflection/eigentt/institution + `reasoning.esl` + the fixture.
fn build_chain() -> ExecutionContext {
    let core_json = include_str!("../../../ontologies/core/core-ontology.json");
    let mut core_builder = LayerBuilder::new("core", None);
    for r in eigon_json::parse_document(core_json).unwrap() {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for src in [
        include_str!("../../../ontologies/reflection/reflection-ontology.json"),
        include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
        include_str!("../../../ontologies/institution/institution-ontology.json"),
    ] {
        for r in eigon_json::parse_document(src).unwrap() {
            reflection_builder.add_resource(r).unwrap();
        }
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(reflection));
    for r in esl::compile(include_str!("../../../ontologies/reasoning/reasoning.esl"))
        .expect("reasoning.esl compiles")
    {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    let fixture_resources = esl::compile_against_layer(
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

/// `(verdict ctor, diagnostic)` for one of the fixture's `ReasoningSentence`s.
fn verdict(local_name: &str) -> (String, Option<String>) {
    let ctx = build_chain();
    let sentence_iri =
        Iri::parse(&format!("urn:eigenius:demo:screen:{local_name}")).expect("sentence IRI");
    let sentence = (*ctx
        .resolve(&sentence_iri)
        .expect("sentence is on the chain"))
    .clone();

    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("validate handler returns an outcome");
    let ctor = outcome
        .output
        .get(&Iri::parse(wk::CTOR_NAME).unwrap())
        .and_then(Value::as_str)
        .expect("verdict carries ctor_name")
        .to_string();
    let diagnostic = outcome
        .output
        .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned);
    (ctor, diagnostic)
}

/// A certificate naming a class in its `P : Prop` slot is rejected.
#[test]
fn a_class_does_not_discharge_a_prop_obligation() {
    let (ctor, diagnostic) = verdict("concl_class");
    assert_eq!(
        ctor,
        wk::VERDICT_FAILS,
        "`declared(iri, screen:CellLine, _)` puts a class in a `P : Prop` slot; \
         got {ctor}, diagnostic: {diagnostic:?}"
    );
    let diagnostic = diagnostic.unwrap_or_default();
    assert!(
        diagnostic.contains("universe stratification"),
        "expected a universe-stratification diagnostic, got: {diagnostic}"
    );
}

/// The same shape with a genuine `Prop`-sorted inductive still validates.
#[test]
fn a_proposition_still_discharges_a_prop_obligation() {
    let (ctor, diagnostic) = verdict("concl_prop");
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "a Prop-sorted inductive in the same slot must still hold; \
         got {ctor}, diagnostic: {diagnostic:?}"
    );
}
