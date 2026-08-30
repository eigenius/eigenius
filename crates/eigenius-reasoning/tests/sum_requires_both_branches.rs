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

//! P4's `Sum` exit criterion: a `Sum` whose second branch cites an ungroundable
//! IRI is refused at commit.
//!
//! `sum_l` used to read
//!
//! ```text
//! sum_l : forall (P, j1, j2) => Certificate(j1, P) -> Certificate(Sum(j1, j2), P)
//! ```
//!
//! leaving `j2` bound and entirely unconstrained — faithful to Artemov, whose
//! axiom `t:F → (t+s):F` quantifies over an arbitrary `s`. It is unsound here
//! because `support` reads `Sum` disjunctively and reports the unchecked branch
//! as a real alternative: `Sum(real_evidence, Declared("urn:does-not-exist"))`
//! type-checked, and `survives_without(real_evidence_iri)` then returned TRUE.
//! The conclusion "survived" losing its only grounded evidence by way of a
//! branch nothing ever grounded — the counterfactual D73 §1.2 calls the whole
//! argument for retaining the polynomial, answered in the reassuring direction.
//!
//! The existing `a_second_source_would_make_a_recompute_droppable` in
//! `projection.rs` cannot catch this: it builds raw `Exp` with no chain behind
//! it and *asserts* "the DRIVE branch carries it alone" as a premise. This test
//! goes through commit, where the witness lookup happens.

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

/// Stack core + reflection + eigentt + institution + justification, then the
/// named fixture.
fn build_chain(fixture_source: &str, label: &str) -> ExecutionContext {
    let core_resources =
        eigon_json::parse_document(include_str!("../../../ontologies/core/core-ontology.json"))
            .unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for source in [
        include_str!("../../../ontologies/reflection/reflection-ontology.json"),
        include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
        include_str!("../../../ontologies/institution/institution-ontology.json"),
    ] {
        for r in eigon_json::parse_document(source).unwrap() {
            reflection_builder.add_resource(r).unwrap();
        }
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    let reasoning_source = include_str!("../../../ontologies/justification/justification.esl");
    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(reflection));
    for r in esl::compile(reasoning_source).expect("justification.esl compiles") {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    let fixture_resources = esl::compile_against_layer(fixture_source, &reasoning)
        .unwrap_or_else(|errs| panic!("{label} failed to compile: {errs:?}"));
    let mut fixture_builder = LayerBuilder::new(label, Some(reasoning));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        fixture_layer,
        label,
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

/// Validate a conclusion and return `(ctor_name, diagnostic)`.
fn validate(ctx: &ExecutionContext, sentence_iri: &str) -> (String, Option<String>) {
    let iri = Iri::parse(sentence_iri).expect("sentence IRI");
    let sentence_arc = ctx
        .resolve(&iri)
        .unwrap_or_else(|| panic!("conclusion `{iri}` should be on the chain"));
    let sentence = (*sentence_arc).clone();

    let inst = ReasoningInstitution::new();
    let outcome = do_validate_justification(&inst, &sentence, ctx)
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

#[test]
fn a_sum_over_two_grounded_branches_holds() {
    // The capability is intact: `Sum` still works, it just costs a derivation
    // per branch. Two independent reviews, either of which carries the claim.
    let ctx = build_chain(
        include_str!("fixtures/sum_requires_both_branches.esl"),
        "sum-two-real-sources",
    );
    let (ctor, diagnostic) = validate(&ctx, "urn:eigenius:demo:screen:concl_two_real_sources");
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "a Sum whose branches both ground must commit; got {ctor}, diagnostic: {diagnostic:?}"
    );
}

#[test]
fn a_sum_whose_fallback_cites_an_ungroundable_iri_is_refused() {
    // Identical except the second summand names a resource the chain does not
    // carry, so no `IsDeclaredAs` witness exists to build its certificate from.
    // Under the old `sum_l` this committed and `survives_without` then reported
    // the real ground as droppable.
    let ctx = build_chain(
        include_str!("fixtures/sum_phantom_fallback.esl"),
        "sum-phantom-fallback",
    );
    let (ctor, diagnostic) = validate(&ctx, "urn:eigenius:demo:screen:concl_phantom_fallback");
    assert_ne!(
        ctor,
        wk::VERDICT_HOLDS,
        "a Sum whose fallback branch cites an ungroundable IRI must not commit"
    );
    let diagnostic = diagnostic.unwrap_or_default();
    assert!(
        diagnostic.contains("review_absent"),
        "the diagnostic must name the branch that could not be grounded; got: {diagnostic}"
    );
    // Refused for the RIGHT reason: the witness lookup missed, not a parse or
    // arity error on the way in.
    assert!(
        diagnostic.contains("IsDeclaredAs"),
        "the refusal must be a missing-witness diagnostic; got: {diagnostic}"
    );
}
