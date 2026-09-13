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

//! The κ–τ institution end to end, over a real chain (D91).
//!
//! `scoring.rs`'s unit tests check the semantics against the paper's worked examples.
//! This checks that the chain vocabulary reaches them: one conclusion, three
//! reconstructed rivals and three policies, dispatched through the kernel's AutoOnLoad
//! path so the assessment resource, the verdict and the emitted decision are the same
//! objects a pilot would commit.
//!
//! **The reproduction gate is the first test.** Under δ ≡ 0 the conclusion commits,
//! which is what makes any later suspension a finding about the margin rather than
//! about this implementation.

use std::sync::Arc;

use eigenius_kappatau::iris;
use eigenius_kappatau::KappaTauInstitution;
use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::dispatch::dispatch_auto_on_load_for_resource;
use eigenius_kernel::institution::registry::InstitutionIndex;
use eigenius_kernel::institution::runtime::InstitutionRuntime;
use eigenius_kernel::layer::LayerBuilder;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// The chain: bootstrap, plus the κ–τ ontology, plus the pilot fixture.
fn pilot_chain() -> (
    ExecutionContext,
    Arc<InstitutionIndex>,
    Arc<InstitutionRuntime>,
) {
    pilot_chain_from(include_str!("fixtures/pilot_shape.esl"))
}

/// The fixture with one substring replaced, asserting the replacement HAPPENED.
///
/// A `str::replace` that matches nothing returns the original, and a test built on it
/// would then exercise the unmodified fixture and pass for the wrong reason. That is the
/// same failure mode as a test that passes against broken code, and it is invisible.
fn fixture_with(from: &str, to: &str) -> String {
    let original = include_str!("fixtures/pilot_shape.esl");
    let edited = original.replace(from, to);
    assert_ne!(
        edited, original,
        "the fixture edit matched nothing, so the variant is the unmodified fixture: {from}"
    );
    edited
}

/// The same chain over a VARIANT of the fixture — one edit, so a test can show what a
/// single authoring slip does.
fn pilot_chain_from(
    fixture: &str,
) -> (
    ExecutionContext,
    Arc<InstitutionIndex>,
    Arc<InstitutionRuntime>,
) {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let storage = boot.storage().clone();
    let head = Arc::clone(boot.head());

    let kt_source = include_str!("../../../ontologies/kappatau/kappatau.esl");
    let kt_resources = eigenius_kernel::esl::compile(kt_source, &head).expect("kappatau.esl");
    let mut kt_builder = LayerBuilder::new("kappatau", Some(head));
    for r in kt_resources {
        kt_builder.add_resource(r).expect("kappatau resource");
    }
    let kt_layer = Arc::new(kt_builder.build(storage.clone()));

    let fixture_resources =
        eigenius_kernel::esl::compile(fixture, &kt_layer).expect("pilot_shape.esl");
    let mut b = LayerBuilder::new("ktpilot", Some(kt_layer));
    for r in fixture_resources {
        b.add_resource(r).expect("fixture resource");
    }
    let layer = Arc::new(b.build(storage.clone()));

    // The fixture must be a VALID chain, or a decision over it says nothing.
    let errors = eigenius_kernel::validation::Validator::new(Arc::clone(&layer)).validate();
    assert!(
        errors.is_empty(),
        "the pilot fixture does not validate:\n{}",
        errors
            .iter()
            .map(|e| format!("  [{:?}] {}", e.rule, e.message))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let (index, index_errors) = InstitutionIndex::from_layer(&layer);
    assert!(index_errors.is_empty(), "{index_errors:?}");
    let mut runtime = InstitutionRuntime::new();
    runtime
        .register(Box::new(KappaTauInstitution::new()))
        .unwrap();

    let ctx = ExecutionContext::new(layer, "ktpilot", ExecutionMode::ReadOnly, storage);
    (ctx, Arc::new(index), Arc::new(runtime))
}

/// Dispatch one assessment and return the verdict constructor plus the decision.
fn run(assessment: &str) -> (String, Resource) {
    let (ctx, index, runtime) = pilot_chain();
    let subject = ctx
        .head()
        .resolve(&iri(assessment))
        .unwrap_or_else(|| panic!("`{assessment}` is on the chain"));
    let outcome = dispatch_auto_on_load_for_resource(&subject, &index, &runtime, &ctx);
    assert!(
        outcome.errors.is_empty(),
        "dispatch errors: {:?}",
        outcome.errors
    );
    assert_eq!(outcome.dispatches.len(), 1, "one QueryClass fires");
    let d = &outcome.dispatches[0];
    assert_eq!(d.derivations.len(), 1, "one decision per assessment");
    (d.verdict.ctor_name().to_string(), d.derivations[0].clone())
}

fn float(r: &Resource, prop: &str) -> f64 {
    match r.get(&iri(prop)) {
        Some(Value::Float(f)) => *f,
        other => panic!("`{prop}` is not a float: {other:?}"),
    }
}

/// The margin records, in the order the institution emitted them.
fn margins(decision: &Resource) -> Vec<Resource> {
    match decision.get(&iri(iris::PROP_RIVAL_MARGINS)) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::Embedded(r) => (**r).clone(),
                other => panic!("a margin record is not embedded: {other:?}"),
            })
            .collect(),
        None => Vec::new(),
        other => panic!("rival_margins is not an array: {other:?}"),
    }
}

/// **The reproduction gate.** Under δ ≡ 0 the conclusion commits, with the hostile
/// rival five hundredths behind it and the audit record saying so.
#[test]
fn the_degenerate_policy_commits() {
    let (verdict, decision) = run("urn:eigenius:test:ktpilot:assess_degenerate");
    assert_eq!(verdict, wk::VERDICT_HOLDS);
    assert!((float(&decision, iris::PROP_COMPUTED_SCORE) - 0.86).abs() < 1e-12);
    assert!(
        decision.get(&iri(iris::PROP_BINDING_RIVAL)).is_none(),
        "nothing bound"
    );
    let m = margins(&decision);
    assert_eq!(m.len(), 2, "two of the three rivals are ACTIVE and hostile");
    for record in &m {
        assert_eq!(
            record.get(&iri(iris::PROP_CLEARED)),
            Some(&Value::Boolean(true))
        );
        assert_eq!(float(record, iris::PROP_REQUIRED_MARGIN), 0.0);
    }
}

/// The compatible alternative scores 0.79, well above the 0.20 activation floor, and
/// still imposes no margin: its interaction is positive, so it is a candidate for
/// synthesis rather than a rival to outrun.
#[test]
fn a_compatible_alternative_never_becomes_a_rival() {
    let (_verdict, decision) = run("urn:eigenius:test:ktpilot:assess_governed");
    let named: Vec<String> = margins(&decision)
        .iter()
        .map(|m| match m.get(&iri(iris::PROP_AGAINST)) {
            Some(Value::String(s)) => s.clone(),
            other => panic!("`against` is not an IRI string: {other:?}"),
        })
        .collect();
    assert!(
        !named.iter().any(|n| n.ends_with("h_second_lineage")),
        "the compatible alternative appears as a rival: {named:?}"
    );
}

/// A non-degenerate margin the conclusion still clears. Same evidence, different
/// governance, still a commitment — the contrast that makes the suspension below a
/// statement about the margin.
#[test]
fn a_light_margin_still_commits() {
    let (verdict, decision) = run("urn:eigenius:test:ktpilot:assess_light");
    assert_eq!(verdict, wk::VERDICT_HOLDS);
    let m = margins(&decision);
    let pseudo = m
        .iter()
        .find(|r| match r.get(&iri(iris::PROP_AGAINST)) {
            Some(Value::String(s)) => s.ends_with("h_pseudoreplication"),
            _ => false,
        })
        .expect("the pseudoreplication rival has a record");
    // δ = 0.05 · τ(0.80) · −κ*(0.80) = 0.032, against an achieved 0.05.
    assert!((float(pseudo, iris::PROP_REQUIRED_MARGIN) - 0.032).abs() < 1e-12);
    assert!((float(pseudo, iris::PROP_ACHIEVED_SEPARATION) - 0.05).abs() < 1e-12);
    assert_eq!(
        pseudo.get(&iri(iris::PROP_CLEARED)),
        Some(&Value::Boolean(true))
    );
}

/// **The finding the pilot is for.** The same conclusion, on the same evidence, is
/// SUSPENDED under a governed margin — and the chain records which rival bound it and
/// by how much, which is the auditable suspension the framework's author asked for.
#[test]
fn a_governed_margin_suspends_and_says_why() {
    let (verdict, decision) = run("urn:eigenius:test:ktpilot:assess_governed");
    assert_eq!(
        verdict,
        wk::VERDICT_UNDECIDABLE,
        "suspension is Undecidable, never Fails: below the margin means do not commit, \
         not that the chain is invalid"
    );
    assert_eq!(
        decision.get(&iri(iris::PROP_BINDING_RIVAL)),
        Some(&Value::String(
            "urn:eigenius:test:ktpilot:h_pseudoreplication".to_string()
        ))
    );
    let m = margins(&decision);
    let pseudo = m
        .iter()
        .find(|r| match r.get(&iri(iris::PROP_AGAINST)) {
            Some(Value::String(s)) => s.ends_with("h_pseudoreplication"),
            _ => false,
        })
        .expect("the pseudoreplication rival has a record");
    // δ = 0.50 · 0.80 · 0.80 = 0.32, against an achieved 0.05.
    assert!((float(pseudo, iris::PROP_REQUIRED_MARGIN) - 0.32).abs() < 1e-12);
    assert_eq!(
        pseudo.get(&iri(iris::PROP_CLEARED)),
        Some(&Value::Boolean(false))
    );
}

/// **It asserts a proposition ABOUT φ, never φ.** The emitted term's head is
/// `kt:Commits` on a commitment and `kt:SuspendedAt` on a suspension, and the
/// conclusion's own proposition sits inside it as an argument. Nothing turns one into
/// the other: crossing that gap takes a declared bridge, which is not this
/// institution's to write.
#[test]
fn the_emitted_proposition_is_about_phi_and_is_not_phi() {
    let (ctx, _index, _runtime) = pilot_chain();
    for (assessment, expected_head, expected_policy) in [
        (
            "urn:eigenius:test:ktpilot:assess_degenerate",
            iris::KT_COMMITS,
            "urn:eigenius:test:ktpilot:policy_degenerate",
        ),
        (
            "urn:eigenius:test:ktpilot:assess_governed",
            iris::KT_SUSPENDED_AT,
            "urn:eigenius:test:ktpilot:policy_governed",
        ),
    ] {
        let (_verdict, decision) = run(assessment);
        let value = decision
            .get(&iri(iris::PROP_PROPOSITION))
            .expect("the decision carries a proposition");
        let exp = eigenius_kernel::program::eigentt_type_mirror::decode_type(value, ctx.head())
            .expect("the emitted proposition decodes");
        let rendered = format!("{exp:?}");
        assert!(
            rendered.contains(expected_head),
            "`{assessment}` should head with `{expected_head}`: {rendered}"
        );
        assert!(
            rendered.contains("urn:eigenius:test:ktpilot:SelectivelyEssential"),
            "φ should appear as an ARGUMENT, not be replaced: {rendered}"
        );
        // The FULL policy IRI, not the shared prefix: on a prefix, an encoder that
        // always named the same policy would pass both iterations.
        assert!(
            rendered.contains(expected_policy),
            "`{assessment}` should name `{expected_policy}` in the claim: {rendered}"
        );
    }
}

/// D90's boundary check runs over this institution's output like any other. A green
/// dispatch above is already evidence — a contract violation would have landed in
/// `outcome.errors` and failed every test in this file — but asserting it directly
/// says that the two pieces were built to fit.
#[test]
fn the_output_satisfies_the_declared_result_contract() {
    let (ctx, index, runtime) = pilot_chain();
    let subject = ctx
        .head()
        .resolve(&iri("urn:eigenius:test:ktpilot:assess_governed"))
        .expect("on chain");
    let outcome = dispatch_auto_on_load_for_resource(&subject, &index, &runtime, &ctx);
    assert!(
        !outcome
            .errors
            .iter()
            .any(|e| e.message.contains("result contract")),
        "contract violations: {:?}",
        outcome.errors
    );
}

/// **A structural failure is not a suspension.** An active rival nobody weighed makes
/// the assessment unadjudicable, and the institution declines to RUN rather than
/// returning `Undecidable` — which a reader could not tell apart from a real
/// suspension. The refusal lands as a validation error naming the QueryClass, and no
/// verdict and no decision are produced at all.
#[test]
fn an_unadjudicable_assessment_refuses_to_run_rather_than_suspending() {
    let (ctx, index, runtime) = pilot_chain();
    let subject = ctx
        .head()
        .resolve(&iri("urn:eigenius:test:ktpilot:assess_unweighted"))
        .expect("on chain");
    let outcome = dispatch_auto_on_load_for_resource(&subject, &index, &runtime, &ctx);
    assert!(
        outcome.dispatches.is_empty(),
        "no verdict should be published: {:?}",
        outcome.dispatches
    );
    assert_eq!(outcome.errors.len(), 1, "one refusal: {:?}", outcome.errors);
    let message = &outcome.errors[0].message;
    assert!(
        message.contains("h_pseudoreplication"),
        "the refusal names the rival it could not score: {message}"
    );
}

/// **A rivalry cannot be deleted by an authoring slip.** An interaction estimate whose
/// pair is not exactly two hypotheses used to be dropped silently, with no error and no
/// record. It fell back to κ = 0, the rival stopped being active, and a suspension
/// became a commitment — the premature convergence the framework exists to prevent,
/// from one missing entry.
///
/// `kt:between` is now declared `min_length 2; max_length 2`, so the chain refuses it at
/// commit, and the handler refuses it too rather than dropping it.
#[test]
fn an_interaction_estimate_that_is_not_a_pair_does_not_commit() {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let storage = boot.storage().clone();
    let head = Arc::clone(boot.head());
    let kt_source = include_str!("../../../ontologies/kappatau/kappatau.esl");
    let kt_resources = eigenius_kernel::esl::compile(kt_source, &head).expect("kappatau.esl");
    let mut kt_builder = LayerBuilder::new("kappatau", Some(head));
    for r in kt_resources {
        kt_builder.add_resource(r).expect("kappatau resource");
    }
    let kt_layer = Arc::new(kt_builder.build(storage.clone()));

    // The fixture with one interaction estimate shortened to a single hypothesis.
    let broken = fixture_with(
        "kt:between = [ pilot:concl_selective, pilot:h_pseudoreplication ];",
        "kt:between = [ pilot:h_pseudoreplication ];",
    );
    let resources = eigenius_kernel::esl::compile(&broken, &kt_layer).expect("compiles");
    let mut b = LayerBuilder::new("ktpilot-broken", Some(kt_layer));
    for r in resources {
        b.add_resource(r).expect("adds");
    }
    let layer = Arc::new(b.build(storage));
    let errors = eigenius_kernel::validation::Validator::new(layer).validate();
    assert!(
        errors.iter().any(|e| {
            e.message.contains("length")
                && e.resource_id
                    .as_ref()
                    .is_some_and(|i| i.as_str().ends_with("k_pseudoreplication"))
        }),
        "a one-entry interaction pair must not validate; got:\n{}",
        errors
            .iter()
            .map(|e| format!("  [{:?}] {}", e.rule, e.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// **Two estimates that disagree are refused, not resolved by array position.** Two
/// independently elicited, independently attributed estimates disagreeing is exactly
/// what "each estimate is its own resource with its own trace" produces. Picking one by
/// authoring order decided a governance question silently, and flipped a suspension into
/// a commitment when the second weight dropped a rival below activation.
#[test]
fn two_weights_for_one_hypothesis_refuse_the_assessment() {
    let (ctx, index, runtime) = pilot_chain_from(&fixture_with(
        "kt:plausibility_estimates = [ pilot:w_subject, pilot:w_pseudoreplication,\n                                  pilot:w_offtarget, pilot:w_second_lineage ];\n    kt:interaction_estimates = [ pilot:k_pseudoreplication, pilot:k_offtarget,\n                                 pilot:k_second_lineage ];\n}\n\nresource pilot:assess_light",
        "kt:plausibility_estimates = [ pilot:w_subject, pilot:w_pseudoreplication,\n                                  pilot:w_offtarget, pilot:w_second_lineage,\n                                  pilot:w_pseudoreplication_low ];\n    kt:interaction_estimates = [ pilot:k_pseudoreplication, pilot:k_offtarget,\n                                 pilot:k_second_lineage ];\n}\n\nresource pilot:w_pseudoreplication_low : kt:PlausibilityEstimate {\n    kt:estimate_of = pilot:h_pseudoreplication;\n    kt:weight = 0.10;\n    kt:estimation_protocol = pilot:protocol;\n}\n\nresource pilot:assess_light",
    ));
    let subject = ctx
        .head()
        .resolve(&iri("urn:eigenius:test:ktpilot:assess_degenerate"))
        .expect("on chain");
    let outcome = dispatch_auto_on_load_for_resource(&subject, &index, &runtime, &ctx);
    assert!(
        outcome.dispatches.is_empty(),
        "no verdict should be published: {:?}",
        outcome.dispatches
    );
    assert_eq!(outcome.errors.len(), 1, "one refusal: {:?}", outcome.errors);
    assert!(
        outcome.errors[0].message.contains("disagree"),
        "the refusal says the estimates disagree: {}",
        outcome.errors[0].message
    );
}

/// **A compatible alternative with no weight does not refuse the assessment.** The
/// weight lookup used to run before the interaction test, so an alternative that can
/// never impose a margin — κ is positive, so it is a candidate for synthesis rather than
/// a rival — made the whole assessment unadjudicable, with a diagnostic calling it an
/// "active rival". The interaction is tested first now.
#[test]
fn an_unweighed_compatible_alternative_does_not_refuse_the_assessment() {
    let (ctx, index, runtime) = pilot_chain_from(&fixture_with(
        "                                  pilot:w_offtarget, pilot:w_second_lineage ];\n    kt:interaction_estimates = [ pilot:k_pseudoreplication, pilot:k_offtarget,\n                                 pilot:k_second_lineage ];\n}\n\nresource pilot:assess_light",
        "                                  pilot:w_offtarget ];\n    kt:interaction_estimates = [ pilot:k_pseudoreplication, pilot:k_offtarget,\n                                 pilot:k_second_lineage ];\n}\n\nresource pilot:assess_light",
    ));
    let subject = ctx
        .head()
        .resolve(&iri("urn:eigenius:test:ktpilot:assess_degenerate"))
        .expect("on chain");
    let outcome = dispatch_auto_on_load_for_resource(&subject, &index, &runtime, &ctx);
    assert!(
        outcome.errors.is_empty(),
        "a compatible alternative imposes no margin, so its weight is not needed: {:?}",
        outcome.errors
    );
    assert_eq!(outcome.dispatches.len(), 1);
    assert_eq!(outcome.dispatches[0].verdict.ctor_name(), wk::VERDICT_HOLDS);
}

/// **A policy whose suspension interval is empty is refused.** The paper requires ε < τ.
/// At ε ≥ τ no rival is ever active, the decision records NO margins at all, and that is
/// indistinguishable from the genuine finding that a conclusion faced no live
/// opposition — so it must not be allowed to look like one.
#[test]
fn a_policy_with_an_empty_suspension_interval_is_refused() {
    let (ctx, index, runtime) = pilot_chain_from(&fixture_with(
        "    kt:activation = 0.20;\n    kt:margin_form = forms:zero;",
        "    kt:activation = 0.95;\n    kt:margin_form = forms:zero;",
    ));
    let subject = ctx
        .head()
        .resolve(&iri("urn:eigenius:test:ktpilot:assess_degenerate"))
        .expect("on chain");
    let outcome = dispatch_auto_on_load_for_resource(&subject, &index, &runtime, &ctx);
    assert!(
        outcome.dispatches.is_empty(),
        "no verdict should be published"
    );
    assert_eq!(outcome.errors.len(), 1, "one refusal: {:?}", outcome.errors);
    assert!(
        outcome.errors[0].message.contains("suspension interval"),
        "the refusal names the empty interval: {}",
        outcome.errors[0].message
    );
}
