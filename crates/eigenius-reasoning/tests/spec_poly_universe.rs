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

//! `justification:Certificate.spec_poly` at a `Set`-quantified rule (eigenius#136).
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

    // `prov` (P5). These fixtures carry `prov:` properties and trace classes; without this
    // layer none of them resolve, and the chain reports a dozen `UnresolvedClassReference`s
    // that no assertion here was looking at. Sits above `reflection` and below
    // `justification`, matching `BOOTSTRAP_CHAIN`.
    let prov_resources = esl::compile_against_layer(
        include_str!("../../../ontologies/prov/prov.esl"),
        &reflection,
    )
    .expect("prov.esl compiles");
    let mut prov_builder = LayerBuilder::new("prov", Some(reflection));
    for r in prov_resources {
        prov_builder.add_resource(r).unwrap();
    }
    let prov = Arc::new(prov_builder.build(LayerStorage::in_memory()));

    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(prov));
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

/// The validation errors for the fixture's one `justification:Conclusion`, joined.
/// Empty is what the handler used to report as `Holds`.
fn judgement_diagnostic(reasoning_source: &str) -> String {
    let ctx = build_chain(
        reasoning_source,
        include_str!("fixtures/spec_poly_set_domain.esl"),
    );
    let sentence_iri = Iri::parse("urn:eigenius:demo:poly:concl").expect("sentence IRI");
    ctx.resolve(&sentence_iri)
        .expect("sentence is on the chain");

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
    diagnostic
}

/// The shipped ontology binds `T : Type 1`, so instantiating `T := Set` is
/// `Set : Type 1` — legal by stratification — and the demo's certificate holds.
#[test]
fn spec_poly_holds_as_shipped() {
    let diagnostic = judgement_diagnostic(include_str!(
        "../../../ontologies/justification/justification.esl"
    ));
    assert!(
        diagnostic.is_empty(),
        "spec_poly at T := Set must hold against the shipped `T : Type 1` binder; \
         got: {diagnostic}"
    );
}

/// The converse, and the reason the ontology moved: with the binder back at `T : Set`,
/// instantiating `T := Set` IS `Set : Set` and the checker rejects it.
///
/// Rewriting the shipped source rather than keeping a stale copy — a second copy would
/// drift, and the assertion below fails loudly if the binder ever moves again.
#[test]
fn spec_poly_at_a_set_domain_is_rejected() {
    let source = include_str!("../../../ontologies/justification/justification.esl")
        .replace("forall (T : Type 1,", "forall (T : Set,");
    assert!(
        source.contains("forall (T : Set,"),
        "the `spec_poly` domain binder moved — this rewrite no longer applies"
    );
    let diagnostic = judgement_diagnostic(&source);
    assert!(
        !diagnostic.is_empty(),
        "a `T : Set` binder instantiated at `Set` is `Set : Set` and must be rejected"
    );
    assert!(
        diagnostic.contains("universe stratification"),
        "expected a universe-stratification diagnostic, got: {diagnostic}"
    );
}
