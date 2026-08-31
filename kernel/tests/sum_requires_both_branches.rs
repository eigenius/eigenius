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

/// Stack core + reflection + eigentt + institution + justification, then the
/// named fixture.
fn build_chain(fixture_source: &str, label: &str) -> ExecutionContext {
    let core_resources =
        eigon_json::parse_document(include_str!("../../ontologies/core/core-ontology.json"))
            .unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for source in [
        include_str!("../../ontologies/reflection/reflection-ontology.json"),
        include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
        include_str!("../../ontologies/institution/institution-ontology.json"),
    ] {
        for r in eigon_json::parse_document(source).unwrap() {
            reflection_builder.add_resource(r).unwrap();
        }
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    // `prov` (P5). The fixtures carry `prov:was_attributed_to`, `prov:had_primary_source`,
    // `prov:rationale` and `prov:DeclarationTrace`; without this layer none of them resolve and
    // the chain reports a dozen `UnresolvedClassReference`s. That went unnoticed because this
    // test never asserted the layer validates — the same gap that let `00-wrn-vocabulary.esl`
    // stop loading for two months. It asserts it now, below.
    //
    // Sits above `reflection` and below `justification`, matching `BOOTSTRAP_CHAIN`.
    let prov_resources =
        esl::compile_against_layer(include_str!("../../ontologies/prov/prov.esl"), &reflection)
            .expect("prov.esl compiles");
    let mut prov_builder = LayerBuilder::new("prov", Some(reflection));
    for r in prov_resources {
        prov_builder.add_resource(r).unwrap();
    }
    let prov = Arc::new(prov_builder.build(LayerStorage::in_memory()));

    let reasoning_source = include_str!("../../ontologies/justification/justification.esl");
    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(prov));
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

/// Every validation error the chain reports for `sentence_iri`'s judgement.
///
/// This called `do_validate_justification` and read a `Verdict` off the outcome. The check
/// it was reading is no longer that handler's: P2 moved it to commit, and `validate.rs` says
/// so — *"the pairing this handler used to check by hand is checked at commit by the uniform
/// check-mode rule … the check itself is no longer this handler's to own"*. What the handler
/// still produced was the institutional verdict, which is provenance, and P7 dissolves the
/// institution that produced it.
///
/// So the assertion moves to the thing that does the checking: Rule 21 decodes the judgement,
/// checks its `type` is a type, and checks its `term` against it. That is the same check the
/// handler reconstructed by hand — and stricter, because the type comes from the stored
/// judgement rather than being rebuilt from the declaration.
fn judgement_errors(ctx: &ExecutionContext, sentence_iri: &str) -> Vec<String> {
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

/// The whole chain validates. Asserted separately from the judgement so a fixture that
/// references vocabulary the chain lacks cannot masquerade as a certificate failure — which
/// is what it did here: the fixtures carry `prov:` properties that did not resolve, and
/// nothing noticed because this test never ran the validator.
fn assert_chain_is_clean(ctx: &ExecutionContext) {
    let errors = eigenius_kernel::validation::Validator::new(ctx.head().clone()).validate();
    assert!(
        errors.is_empty(),
        "the fixture chain must validate cleanly; {} error(s): {:#?}",
        errors.len(),
        errors.iter().take(5).collect::<Vec<_>>()
    );
}

#[test]
fn a_sum_over_two_grounded_branches_holds() {
    // The capability is intact: `Sum` still works, it just costs a derivation
    // per branch. Two independent reviews, either of which carries the claim.
    let ctx = build_chain(
        include_str!("fixtures/sum_requires_both_branches.esl"),
        "sum-two-real-sources",
    );
    assert_chain_is_clean(&ctx);
    let errors = judgement_errors(&ctx, "urn:eigenius:demo:screen:concl_two_real_sources");
    assert!(
        errors.is_empty(),
        "a Sum whose branches both ground must commit; got {errors:#?}"
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
    let errors = judgement_errors(&ctx, "urn:eigenius:demo:screen:concl_phantom_fallback");
    assert!(
        !errors.is_empty(),
        "a Sum whose fallback branch cites an ungroundable IRI must not commit"
    );
    let diagnostic = errors.join("\n");
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
