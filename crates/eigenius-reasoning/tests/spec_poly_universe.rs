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

//! `JustifiedBy.spec_poly` at a `Set`-quantified rule (eigenius#136).
//!
//! The fixture is the `spec_poly` site of `demo/prose-to-formulas-v2/inference.esl`
//! reduced to a chain that builds in memory: a rule quantified over `Set` (the
//! subject is a kind, so the domain of the quantifier is `Set`), eliminated at a
//! concrete class.
//!
//! `reasoning.esl` binds `spec_poly`'s domain as `T : Set`, so eliminating that
//! rule instantiates `T := Set` — `Set : Set`, which the checker admitted only
//! through the lenient arm eigenius#136 removed. The two tests below pin both
//! ends: as shipped the certificate now fails with a universe-stratification
//! diagnostic, and raising the binder one universe (`T : Type 1`) is enough to
//! make it hold again. Which reformulation the reasoning ontology takes —
//! the level-1 bump, or universe-polymorphic binders — is an open decision;
//! `spec_poly`'s signature is part of `docs/spec/ai-computed-provenance-1.0.md`.

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
fn build_chain(reasoning_source: &str, fixture_source: &str) -> ExecutionContext {
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
    for r in esl::compile(reasoning_source).expect("reasoning.esl compiles") {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    let fixture_resources =
        esl::compile_against_layer(fixture_source, &reasoning).unwrap_or_else(|errs| {
            panic!(
                "fixture failed to compile: {}",
                errs.into_iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let mut fixture_builder = LayerBuilder::new("spec-poly-demo", Some(reasoning));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        "spec-poly-demo",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

/// `(verdict ctor, diagnostic)` for the fixture's one `ReasoningSentence`.
fn verdict(reasoning_source: &str) -> (String, Option<String>) {
    let ctx = build_chain(
        reasoning_source,
        include_str!("fixtures/spec_poly_set_domain.esl"),
    );
    let sentence_iri = Iri::parse("urn:eigenius:demo:poly:concl").expect("sentence IRI");
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

/// `T : Set` as shipped: instantiating `T := Set` is `Set : Set`, rejected.
#[test]
fn spec_poly_at_a_set_domain_is_rejected_as_shipped() {
    let (ctor, diagnostic) = verdict(include_str!("../../../ontologies/reasoning/reasoning.esl"));
    assert_eq!(
        ctor,
        wk::VERDICT_FAILS,
        "spec_poly at T := Set instantiates a `T : Set` binder with `Set` itself; \
         got {ctor}, diagnostic: {diagnostic:?}"
    );
    let diagnostic = diagnostic.unwrap_or_default();
    assert!(
        diagnostic.contains("universe stratification"),
        "expected a universe-stratification diagnostic, got: {diagnostic}"
    );
}

/// Raising the binder one universe (`T : Type 1`) is the whole fix on the
/// ontology side — the certificate is unchanged and holds again.
#[test]
fn spec_poly_holds_when_the_domain_binder_is_raised_one_universe() {
    let source = include_str!("../../../ontologies/reasoning/reasoning.esl")
        .replace("forall (T : Set,", "forall (T : Type 1,");
    assert!(
        source.contains("forall (T : Type 1,"),
        "the `spec_poly` domain binder moved — this rewrite no longer applies"
    );
    let (ctor, diagnostic) = verdict(&source);
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds with `T : Type 1`; got {ctor}, diagnostic: {diagnostic:?}"
    );
}
