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

//! Phase 6 smoke tests for `ReasoningInstitution::query(validate_justification)`.
//!
//! Locks in the validate path's plumbing: property reading, decoder
//! dispatch, type-check invocation, Verdict resource shape. The full
//! "Holds" path (a well-formed certificate whose witnesses are
//! synthesized at type-check time) requires Phase 9's
//! `synthesize_chain_witness` integration in `nbe::check`; until that
//! lands, well-formed certificates surface as `Verdict::Fails` with a
//! missing-witness diagnostic. The Phase 10 end-to-end test will flip
//! the expected verdict to `Holds` once Phase 9 closes the loop.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::Institution;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_reasoning::institution::iris;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::ReasoningInstitution;
use serde_json::json;

/// Stand up the layer chain needed to dispatch the validate handler:
/// core → reflection (+ eigentt fragment) → reasoning. Mirrors the
/// kernel `esl::compile::tests::reasoning_ontology_resolves_through_codec`
/// helper.
fn build_full_chain() -> ExecutionContext {
    let core_json = include_str!("../../../ontologies/core/core-ontology.json");
    let core_resources = eigon_json::parse_document(core_json).unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let reflection_json = include_str!("../../../ontologies/reflection/reflection-ontology.json");
    let reflection_resources = eigon_json::parse_document(reflection_json).unwrap();
    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for r in reflection_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let eigentt_json = include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json");
    let eigentt_resources = eigon_json::parse_document(eigentt_json).unwrap();
    for r in eigentt_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let institution_json =
        include_str!("../../../ontologies/institution/institution-ontology.json");
    let institution_resources = eigon_json::parse_document(institution_json).unwrap();
    for r in institution_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    let reasoning_source = include_str!("../../../ontologies/reasoning/reasoning.esl");
    let reasoning_resources = esl::compile(reasoning_source).expect("reasoning.esl compiles");
    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(reflection));
    for r in reasoning_resources {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        reasoning,
        "validate_test",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

/// Build a synthetic ReasoningSentence with the supplied property
/// values. Tests pass the three required fields directly; the helper
/// stamps the resource shape so the validate handler sees exactly
/// what a committed sentence would carry.
fn synthetic_sentence(
    proposition: Option<Value>,
    justification: Option<Value>,
    certificate: Option<Value>,
) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse("urn:eigenius:reasoning:ReasoningSentence").unwrap(),
        )]),
    );
    if let Some(v) = proposition {
        r.set(Iri::parse(iris::PROP_PROPOSITION).unwrap(), v);
    }
    if let Some(v) = justification {
        r.set(Iri::parse(iris::PROP_JUSTIFICATION).unwrap(), v);
    }
    if let Some(v) = certificate {
        r.set(Iri::parse(iris::PROP_CERTIFICATE).unwrap(), v);
    }
    r
}

/// Read the `ctor_name` field off a verdict resource — discriminates
/// `Holds` from `Fails`.
fn verdict_ctor(r: &Resource) -> String {
    r.get(&Iri::parse(wk::CTOR_NAME).unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned)
        .expect("verdict resource has ctor_name")
}

/// Read the `diagnostic` field off a verdict resource (typically
/// present on `Fails` verdicts only).
fn verdict_diagnostic(r: &Resource) -> Option<String> {
    r.get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[test]
fn missing_proposition_surfaces_computation_failed() {
    let ctx = build_full_chain();
    let sentence = synthetic_sentence(
        None,
        Some(Value::Json(
            json!({"ctor": "DeclaredEvidence", "args": ["urn:a"]}),
        )),
        Some(Value::Json(json!({"ctor": "Sort", "args": [0]}))),
    );
    let err = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx).unwrap_err();
    match err {
        InstitutionError::ComputationFailed(msg) => {
            assert!(
                msg.contains("proposition"),
                "expected proposition error, got: {msg}"
            );
        }
        other => panic!("expected ComputationFailed, got {other:?}"),
    }
}

#[test]
fn missing_certificate_surfaces_computation_failed() {
    let ctx = build_full_chain();
    let sentence = synthetic_sentence(
        Some(Value::Json(json!({"ctor": "Sort", "args": [0]}))),
        Some(Value::Json(
            json!({"ctor": "DeclaredEvidence", "args": ["urn:a"]}),
        )),
        None,
    );
    let err = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx).unwrap_err();
    match err {
        InstitutionError::ComputationFailed(msg) => {
            assert!(
                msg.contains("certificate"),
                "expected certificate error, got: {msg}"
            );
        }
        other => panic!("expected ComputationFailed, got {other:?}"),
    }
}

#[test]
fn malformed_proposition_surfaces_verdict_fails() {
    let ctx = build_full_chain();
    let sentence = synthetic_sentence(
        // Wrong shape — D47 codec rejects an unknown ctor.
        Some(Value::Json(json!({"ctor": "NotARealCtor", "args": []}))),
        Some(Value::Json(
            json!({"ctor": "DeclaredEvidence", "args": ["urn:a"]}),
        )),
        Some(Value::Json(json!({"ctor": "Sort", "args": [0]}))),
    );
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_FAILS);
    let diag = verdict_diagnostic(&outcome.output).expect("Fails carries diagnostic");
    assert!(
        diag.contains("proposition"),
        "diagnostic should mention proposition, got: {diag}"
    );
}

#[test]
fn malformed_justification_surfaces_verdict_fails() {
    let ctx = build_full_chain();
    let sentence = synthetic_sentence(
        // Valid Prop term.
        Some(Value::Json(json!({"ctor": "Sort", "args": [0]}))),
        // Unknown JustificationTerm ctor — chain inductive decoder
        // catches it.
        Some(Value::Json(json!({"ctor": "NotAJTctor", "args": []}))),
        Some(Value::Json(json!({"ctor": "Sort", "args": [0]}))),
    );
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_FAILS);
    let diag = verdict_diagnostic(&outcome.output).expect("Fails carries diagnostic");
    assert!(
        diag.contains("justification"),
        "diagnostic should mention justification, got: {diag}"
    );
}

#[test]
fn institution_dispatch_routes_to_validate_handler() {
    // Confirms ReasoningInstitution::query(PROC_VALIDATE_JUSTIFICATION,
    // …) routes to the same logic the direct
    // do_validate_justification entry point exercises. This is the
    // path the kernel's AutoOnLoad dispatch will take when the chain
    // sees a ReasoningSentence commit.
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let sentence = synthetic_sentence(
        // Wrong shape — same as the malformed_proposition test —
        // surfaces the Fails outcome via institution dispatch.
        Some(Value::Json(json!({"ctor": "NotARealCtor", "args": []}))),
        Some(Value::Json(
            json!({"ctor": "DeclaredEvidence", "args": ["urn:a"]}),
        )),
        Some(Value::Json(json!({"ctor": "Sort", "args": [0]}))),
    );
    let proc_iri = Iri::parse(iris::PROC_VALIDATE_JUSTIFICATION).unwrap();
    let outcome = inst
        .query(&proc_iri, &sentence, &ctx)
        .expect("dispatch succeeds");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_FAILS);
}

#[test]
fn institution_dispatch_rejects_unknown_procedure() {
    // Confirms unknown procedure IRIs surface NotImplemented rather
    // than panicking or misdispatching. Locks the dispatch table's
    // closed-set discipline.
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let sentence = synthetic_sentence(None, None, None);
    let bogus = Iri::parse("urn:eigenius:reasoning:proc:does_not_exist").unwrap();
    let err = inst.query(&bogus, &sentence, &ctx).unwrap_err();
    assert!(matches!(err, InstitutionError::NotImplemented(_)));
}

#[test]
fn entailment_and_consistency_return_not_implemented_in_phase_6() {
    // Phase 7 will implement these. The institution's dispatch table
    // is in place now so the IRIs route, but the handlers return
    // NotImplemented. Locks the dispatch surface in advance of Phase 7
    // so the handlers can be plugged in without touching dispatch.
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let sentence = synthetic_sentence(None, None, None);
    for proc_iri_str in &[iris::PROC_ENTAILMENT_QUERY, iris::PROC_CONSISTENCY_CHECK] {
        let proc_iri = Iri::parse(proc_iri_str).unwrap();
        let err = inst.query(&proc_iri, &sentence, &ctx).unwrap_err();
        assert!(
            matches!(err, InstitutionError::NotImplemented(_)),
            "{proc_iri_str} should return NotImplemented, got {err:?}"
        );
    }
}

#[test]
fn well_formed_inputs_but_no_witnesses_yet_surface_verdict_fails_pending_phase_9() {
    // The interesting case the smoke test locks in: when every input
    // decodes cleanly but the certificate's type-check needs ChainWitness
    // inhabitants the kernel doesn't yet synthesise (Phase 9's
    // `synthesize_chain_witness` integration), the handler surfaces
    // Verdict::Fails with the kernel's type-error diagnostic. Phase 10
    // will flip the expected verdict to `Holds` once Phase 9 lands.
    let ctx = build_full_chain();

    // Proposition: `Asserts("urn:foo")` — the canonical atomic Prop.
    // D47 encoding: App(ConstRef("urn:eigenius:core:Asserts"), LitString("urn:foo")).
    let proposition = Value::Json(json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
            {"ctor": "LitString", "args": ["urn:foo"]},
        ],
    }));
    // Justification: `DeclaredEvidence("urn:foo")` in chain inductive shape.
    let justification = Value::Json(json!({
        "ctor": "DeclaredEvidence",
        "args": ["urn:foo"],
    }));
    // Certificate: `JustifiedBy.declared(<witness>)` — we put a
    // placeholder Sort literal in the witness slot so the certificate
    // decodes cleanly; type-check rejects it because the placeholder
    // doesn't inhabit `ChainWitness.IsDeclaredAs("urn:foo", Asserts("urn:foo"))`.
    // The exact rejection shape is what Phase 9 changes — for now we
    // just require Fails.
    let certificate = Value::Json(json!({
        "ctor": "App",
        "args": [
            {"ctor": "CtorApp", "args": [
                "urn:eigenius:reasoning:JustifiedBy",
                "declared",
            ]},
            {"ctor": "Sort", "args": [0]},
        ],
    }));

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    // We only assert Fails here — Phase 10 flips this to Holds.
    assert_eq!(
        verdict_ctor(&outcome.output),
        wk::VERDICT_FAILS,
        "expected Fails verdict (Phase 9 not yet wired); got verdict={}",
        verdict_ctor(&outcome.output)
    );
    // The diagnostic should mention certificate-side type-check failure.
    let diag = verdict_diagnostic(&outcome.output).expect("Fails carries diagnostic");
    assert!(
        diag.contains("certificate") || diag.contains("type-check") || diag.contains("JustifiedBy"),
        "diagnostic should explain certificate type-check failure, got: {diag}"
    );
}
