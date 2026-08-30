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

//! Integration tests for the Reasoning institution's handlers.
//!
//! Layered by phase:
//!
//! - **Phase 6 plumbing** — property reading, decoder dispatch, type-
//!   check invocation, Verdict resource shape, dispatch routing.
//! - **Phase 7 query handlers** — EntailmentQuery (lookup-based v1)
//!   and ConsistencyCheck (Undecidable stub) input parsing + outcomes.
//! - **Phase 10 end-to-end** — full chain → witness-index →
//!   kernel-side `synthesize_chain_witness` → ctor-arg admission →
//!   `Verdict::Holds` path, plus matching soundness-boundary checks
//!   (proposition mismatch and missing-trace both Fail correctly).

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

    let reasoning_source = include_str!("../../../ontologies/justification/justification.esl");
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

/// Build a synthetic justification:Conclusion with the supplied property
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
            Iri::parse("urn:eigenius:justification:Conclusion").unwrap(),
        )]),
    );
    // The three parts are now one slot, so "a part is missing" becomes "the
    // judgement is missing": with the parts collapsed into the certificate
    // type there is no way to supply two of three. The error paths these
    // callers exercise are unchanged in kind — the handler still reports a
    // conclusion it cannot read — but there is now one way to be unreadable
    // instead of three.
    if let (Some(p), Some(j), Some(c)) = (proposition, justification, certificate) {
        r.set(
            Iri::parse(iris::PROP_JUDGEMENT).unwrap(),
            judgement(p, j, c),
        );
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
        Some(Value::Json(json!({"ctor": "Declared", "args": ["urn:a"]}))),
        Some(Value::Json(
            json!({"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]}),
        )),
    );
    let err = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx).unwrap_err();
    match err {
        InstitutionError::ComputationFailed(msg) => {
            assert!(
                msg.contains("judgement"),
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
        Some(Value::Json(
            json!({"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]}),
        )),
        Some(Value::Json(json!({"ctor": "Declared", "args": ["urn:a"]}))),
        None,
    );
    let err = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx).unwrap_err();
    match err {
        InstitutionError::ComputationFailed(msg) => {
            assert!(
                msg.contains("judgement"),
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
        Some(Value::Json(json!({"ctor": "Declared", "args": ["urn:a"]}))),
        Some(Value::Json(
            json!({"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]}),
        )),
    );
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_FAILS);
    let diag = verdict_diagnostic(&outcome.output).expect("Fails carries diagnostic");
    assert!(
        diag.contains("judgement") || diag.contains("proposition"),
        "diagnostic should name the judgement the proposition now lives in, got: {diag}"
    );
}

#[test]
fn malformed_justification_surfaces_verdict_fails() {
    let ctx = build_full_chain();
    // `Asserts("urn:a")` — a term that actually INHABITS Prop.
    //
    // This used to be `Sort(Zero)`, commented "Valid Prop term". It is not: it
    // is `Prop` itself, a type. The old handler never noticed, because the
    // proposition was a slot of its own that nothing checked for
    // propositionhood. Inside a judgement it is the second index of
    // `Certificate(j, P)`, so `P : Prop` is checked — and the fixture's own
    // defect surfaced ahead of the one the test is about.
    let sentence = synthetic_sentence(
        Some(Value::Json(json!({
            "ctor": "App",
            "args": [
                {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
                {"ctor": "LitString", "args": ["urn:a"]},
            ],
        }))),
        // Unknown justification:Term ctor.
        Some(Value::Json(json!({"ctor": "NotAJTctor", "args": []}))),
        Some(Value::Json(
            json!({"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]}),
        )),
    );
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_FAILS);
    let diag = verdict_diagnostic(&outcome.output).expect("Fails carries diagnostic");
    assert!(
        diag.contains("judgement") || diag.contains("justification"),
        "diagnostic should name the judgement the justification term now lives in, got: {diag}"
    );
}

#[test]
fn institution_dispatch_routes_to_validate_handler() {
    // Confirms ReasoningInstitution::query(PROC_VALIDATE_JUSTIFICATION,
    // …) routes to the same logic the direct
    // do_validate_justification entry point exercises. This is the
    // path the kernel's AutoOnLoad dispatch will take when the chain
    // sees a justification:Conclusion commit.
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let sentence = synthetic_sentence(
        // Wrong shape — same as the malformed_proposition test —
        // surfaces the Fails outcome via institution dispatch.
        Some(Value::Json(json!({"ctor": "NotARealCtor", "args": []}))),
        Some(Value::Json(json!({"ctor": "Declared", "args": ["urn:a"]}))),
        Some(Value::Json(
            json!({"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]}),
        )),
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

// ── Phase 7 — EntailmentQuery + ConsistencyCheck handlers ───────────

/// Build a synthetic EntailmentRequest carrying the supplied
/// candidate proposition value.
fn entailment_request(candidate: Value) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse("urn:eigenius:justification:EntailmentRequest").unwrap(),
        )]),
    );
    r.set(
        Iri::parse(iris::PROP_CANDIDATE_PROPOSITION).unwrap(),
        candidate,
    );
    r
}

/// Build a synthetic ConsistencyRequest carrying the supplied
/// sentence-set array value.
fn consistency_request(sentence_set: Value) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse("urn:eigenius:justification:ConsistencyRequest").unwrap(),
        )]),
    );
    r.set(Iri::parse(iris::PROP_SENTENCE_SET).unwrap(), sentence_set);
    r
}

#[test]
fn entailment_query_returns_undecidable_when_no_sentence_matches() {
    // No ReasoningSentences committed in the test layer chain, so the
    // candidate proposition cannot match any. v1's lookup-based
    // search returns Undecidable (not Fails — absence of evidence is
    // not proof of impossibility).
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let request = entailment_request(Value::Json(
        json!({"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]}),
    ));
    let proc_iri = Iri::parse(iris::PROC_ENTAILMENT_QUERY).unwrap();
    let outcome = inst.query(&proc_iri, &request, &ctx).expect("dispatch");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_UNDECIDABLE);
}

#[test]
fn entailment_query_undecidable_when_candidate_malformed() {
    // A malformed candidate (unknown D47 ctor) should surface
    // Undecidable with a decoder diagnostic — not a hard
    // ComputationFailed. The QueryClass surface is best-effort.
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let request = entailment_request(Value::Json(json!({"ctor": "NotARealCtor", "args": []})));
    let proc_iri = Iri::parse(iris::PROC_ENTAILMENT_QUERY).unwrap();
    let outcome = inst.query(&proc_iri, &request, &ctx).expect("dispatch");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_UNDECIDABLE);
    let diag = verdict_diagnostic(&outcome.output).expect("diagnostic");
    assert!(
        diag.contains("D47") || diag.contains("decode") || diag.contains("codec"),
        "diagnostic should mention decoder failure, got: {diag}"
    );
}

#[test]
fn entailment_query_missing_candidate_surfaces_computation_failed() {
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let mut request = Resource::new_embedded();
    request.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse("urn:eigenius:justification:EntailmentRequest").unwrap(),
        )]),
    );
    let proc_iri = Iri::parse(iris::PROC_ENTAILMENT_QUERY).unwrap();
    let err = inst.query(&proc_iri, &request, &ctx).unwrap_err();
    match err {
        InstitutionError::ComputationFailed(msg) => {
            assert!(
                msg.contains("candidate_proposition"),
                "expected missing-candidate error, got: {msg}"
            );
        }
        other => panic!("expected ComputationFailed, got {other:?}"),
    }
}

#[test]
fn consistency_check_returns_holds_on_empty_set() {
    // Vacuous: the empty set is consistent by definition. v1 catches
    // this case so callers probing dispatch don't need a non-trivial
    // input to confirm routing.
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let request = consistency_request(Value::Array(vec![]));
    let proc_iri = Iri::parse(iris::PROC_CONSISTENCY_CHECK).unwrap();
    let outcome = inst.query(&proc_iri, &request, &ctx).expect("dispatch");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_HOLDS);
}

#[test]
fn consistency_check_returns_undecidable_on_nontrivial_set() {
    // v1 of the handler returns Undecidable for any non-trivial input.
    // Phase 10's full implementation may upgrade to a real propositional
    // decision procedure.
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let request = consistency_request(Value::Array(vec![Value::ResourceRef(
        Iri::parse("urn:eigenius:notebook:demo:some_sentence").unwrap(),
    )]));
    let proc_iri = Iri::parse(iris::PROC_CONSISTENCY_CHECK).unwrap();
    let outcome = inst.query(&proc_iri, &request, &ctx).expect("dispatch");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_UNDECIDABLE);
    let diag = verdict_diagnostic(&outcome.output).expect("diagnostic");
    assert!(
        diag.contains("v1") || diag.contains("Undecidable") || diag.contains("follow-on"),
        "diagnostic should explain v1 limitation, got: {diag}"
    );
}

#[test]
fn consistency_check_missing_sentence_set_surfaces_computation_failed() {
    let ctx = build_full_chain();
    let inst = ReasoningInstitution::new();
    let mut request = Resource::new_embedded();
    request.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse("urn:eigenius:justification:ConsistencyRequest").unwrap(),
        )]),
    );
    let proc_iri = Iri::parse(iris::PROC_CONSISTENCY_CHECK).unwrap();
    let err = inst.query(&proc_iri, &request, &ctx).unwrap_err();
    match err {
        InstitutionError::ComputationFailed(msg) => {
            assert!(
                msg.contains("sentence_set"),
                "expected missing-sentence-set error, got: {msg}"
            );
        }
        other => panic!("expected ComputationFailed, got {other:?}"),
    }
}

// ── Phase 10 — end-to-end Holds path ────────────────────────────────

/// Stand up the standard reasoning chain (core → reflection → eigentt
/// → institution → reasoning) plus a user layer carrying a
/// DeclaredResource at `target_iri` (no explicit
/// `canonical_proposition` — the default `Asserts(target_iri)` from
/// D49 §6 applies) and a DeclarationTrace pointing at it. The trace
/// populates the user layer's witness index with a `Declared`
/// witness for `(target_iri, Asserts(target_iri))`.
///
/// This helper uses the *default* `Asserts(target_iri)` witness
/// emission path (`canonical_proposition` not set on the target). The
/// explicit-canonical_proposition variant is exercised by
/// [`build_chain_with_explicit_canonical_proposition`] below; before
/// gh #75 it was a soundness hazard because the chain-encoded bytes
/// (full-IRI `ConstRef` from the D47 encoder against an ESL-stub
/// decl) didn't match the synthesis-side encoding (short-name
/// `ConstRef` from the resolved decl). Now that `InductiveDecl`
/// carries `iri` as the stable identifier, both paths produce
/// byte-identical encodings.
fn build_chain_with_declared_axiom(target_iri_str: &str) -> ExecutionContext {
    use eigenius_kernel::ontology::well_known as wk_local;

    let base_ctx = build_full_chain();
    let reasoning_layer = base_ctx.head().clone();

    // The DeclaredResource — minimal shape (is_a only). The default-
    // Asserts witness emission path triggers when canonical_proposition
    // is absent.
    let target_iri = Iri::parse(target_iri_str).unwrap();
    let mut target = Resource::new(target_iri.clone());
    target.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk_local::DECLARED_RESOURCE).unwrap(),
        )]),
    );

    // The DeclarationTrace pointing at the target. Its presence is
    // what makes `build_witness_index` emit the Declared witness key.
    let trace_iri_str = format!("{target_iri_str}-decl-trace");
    let mut trace = Resource::new(Iri::parse(&trace_iri_str).unwrap());
    trace.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk_local::DECLARATION_TRACE).unwrap(),
        )]),
    );
    trace.set(
        Iri::parse(wk_local::REFLECTION_RESOURCE).unwrap(),
        Value::ResourceRef(target_iri.clone()),
    );

    let mut builder = LayerBuilder::new("phase10-axioms", Some(reasoning_layer));
    builder.add_resource(target).unwrap();
    builder.add_resource(trace).unwrap();
    let user_layer = Arc::new(builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        user_layer,
        "phase10-test",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

/// Build a `justification:Certificate.declared(iri, P, witness_placeholder)` D47
/// certificate where the witness slot is `UnitVal` — the kernel
/// ignores the user's value and synthesizes the witness. `P` is
/// supplied as a pre-encoded D47 sub-tree so callers can mismatch
/// it against the chain's canonical_proposition to test the
/// negative path.
fn justified_by_declared_certificate(
    iri_str: &str,
    proposition_subtree: serde_json::Value,
) -> Value {
    Value::Json(json!({
        "ctor": "App",
        "args": [
            {"ctor": "App", "args": [
                {"ctor": "App", "args": [
                    {"ctor": "CtorApp", "args": [
                        "urn:eigenius:justification:Certificate",
                        "declared",
                    ]},
                    {"ctor": "LitString", "args": [iri_str]},
                ]},
                proposition_subtree,
            ]},
            {"ctor": "UnitVal", "args": []},
        ],
    }))
}

/// Variant of [`build_chain_with_declared_axiom`] that stamps
/// `canonical_proposition` *explicitly* on the target resource (via
/// the D47 encoder running over a resolved-style decl). The witness
/// emitter reads the explicit value rather than computing the
/// `Asserts(iri)` default — different code path, must still hash-
/// equal the synthesis-side computation.
///
/// Before gh #75 this was the broken path: the chain bytes used one
/// `ConstRef` shape (driven by the test's stub decl), the synthesis
/// hook used another (driven by the resolved decl), the hashes
/// differed, the witness lookup missed. Post-fix, both sides read
/// `decl.iri` and produce the same bytes.
fn build_chain_with_explicit_canonical_proposition(target_iri_str: &str) -> ExecutionContext {
    use eigenius_kernel::nbe::term::{Exp, InductiveDecl};
    use eigenius_kernel::ontology::well_known as wk_local;
    use eigenius_kernel::program::eigentt_type_mirror::encode_type;

    let base_ctx = build_full_chain();
    let reasoning_layer = base_ctx.head().clone();

    // Stamp canonical_proposition = `Asserts(target_iri)` using the
    // D47 encoder. The stub mimics what a chain-author tool would
    // produce: short name + full IRI on the decl. After gh #75 the
    // encoder reads decl.iri, so both this shape and the resolver-
    // built shape produce identical bytes.
    let asserts_iri = Iri::parse("urn:eigenius:core:Asserts").expect("static Asserts IRI");
    let stub_decl = std::sync::Arc::new(InductiveDecl {
        uparams: Vec::new(),
        iri: asserts_iri.clone(),
        name: asserts_iri.local_name().to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::sort(0),
        ctors: Vec::new(),
    });
    let prop_exp = Exp::const_applied(
        stub_decl.iri.clone(),
        Vec::new(),
        vec![Exp::LitString(target_iri_str.to_string())],
    );
    let prop_value = encode_type(&prop_exp).expect("encode Asserts(iri)");

    let target_iri = Iri::parse(target_iri_str).unwrap();
    let mut target = Resource::new(target_iri.clone());
    target.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk_local::DECLARED_RESOURCE).unwrap(),
        )]),
    );
    target.set(
        Iri::parse(wk_local::CANONICAL_PROPOSITION).unwrap(),
        prop_value,
    );

    let trace_iri_str = format!("{target_iri_str}-decl-trace");
    let mut trace = Resource::new(Iri::parse(&trace_iri_str).unwrap());
    trace.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk_local::DECLARATION_TRACE).unwrap(),
        )]),
    );
    trace.set(
        Iri::parse(wk_local::REFLECTION_RESOURCE).unwrap(),
        Value::ResourceRef(target_iri.clone()),
    );

    let mut builder = LayerBuilder::new("phase10-axioms-explicit", Some(reasoning_layer));
    builder.add_resource(target).unwrap();
    builder.add_resource(trace).unwrap();
    let user_layer = Arc::new(builder.build(LayerStorage::in_memory()));

    ExecutionContext::new(
        user_layer,
        "phase10-explicit-canonical-proposition",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn end_to_end_validate_holds_with_explicit_canonical_proposition_after_iri_split() {
    // gh #75 regression check: a target resource with an *explicit*
    // canonical_proposition (encoded via the D47 codec) must produce
    // a witness whose prop_hash matches what the synthesis hook
    // computes by eval+readback+encode on the certificate's
    // proposition. Pre-fix the encoder used `decl.name` for the
    // ConstRef slot — chain-author tools using IRI-shaped names and
    // resolver-built decls using short names produced different
    // bytes. Post-fix both read `decl.iri`, the bytes agree, the
    // witness is admitted, the certificate type-checks, Verdict::Holds.
    let target = "urn:test:phase10:explicit-axiom";
    let ctx = build_chain_with_explicit_canonical_proposition(target);

    let asserts_subtree = json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
            {"ctor": "LitString", "args": [target]},
        ],
    });
    let proposition = Value::Json(asserts_subtree.clone());
    let justification = Value::Json(json!({
        "ctor": "Declared",
        "args": [target],
    }));
    let certificate = justified_by_declared_certificate(target, asserts_subtree);

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    let ctor = verdict_ctor(&outcome.output);
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "explicit canonical_proposition should now produce Holds (gh #75); \
         got {ctor}, diagnostic: {:?}",
        verdict_diagnostic(&outcome.output)
    );
}

#[test]
fn end_to_end_validate_holds_when_certificate_matches_admitted_witness() {
    // The Phase 10 headline test: a complete justified-reasoning
    // commit lands as Verdict::Holds. Chain has a DeclarationTrace
    // emitting an admitted `IsDeclaredAs(target, Asserts(target))`
    // witness; the certificate's `justification:Certificate.declared` ctor's third
    // arg slot is filled in by the kernel's Phase 9 synthesis hook;
    // the type-check succeeds.
    let target = "urn:test:phase10:axiom";
    let ctx = build_chain_with_declared_axiom(target);

    let asserts_subtree = json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
            {"ctor": "LitString", "args": [target]},
        ],
    });

    let proposition = Value::Json(asserts_subtree.clone());
    let justification = Value::Json(json!({
        "ctor": "Declared",
        "args": [target],
    }));
    let certificate = justified_by_declared_certificate(target, asserts_subtree);

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    let ctor = verdict_ctor(&outcome.output);
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds verdict; got {ctor}, diagnostic: {:?}",
        verdict_diagnostic(&outcome.output)
    );
}

#[test]
fn end_to_end_validate_fails_when_proposition_mismatches_admitted_witness() {
    // Contrast: the chain admits a witness for `Asserts(target)`, but
    // the certificate claims `Asserts(different_iri)` as the
    // proposition. The witness lookup misses (the prop_hash differs),
    // so the synthesis hook surfaces a "no admitted witness" error
    // which the validate handler lifts into Verdict::Fails. Locks the
    // soundness boundary: a sentence can't cite a witnessed resource
    // for the wrong proposition.
    let target = "urn:test:phase10:axiom";
    let ctx = build_chain_with_declared_axiom(target);

    // Proposition + certificate claim a *different* iri's assertion —
    // not what the chain's DeclarationTrace witnesses.
    let mismatched = "urn:test:phase10:unrelated";
    let mismatched_subtree = json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
            {"ctor": "LitString", "args": [mismatched]},
        ],
    });

    let proposition = Value::Json(mismatched_subtree.clone());
    // The justification still cites `target` (a valid Declared
    // grounding), but the proposition the certificate claims doesn't
    // match what the chain admits for that resource.
    let justification = Value::Json(json!({
        "ctor": "Declared",
        "args": [target],
    }));
    let certificate = justified_by_declared_certificate(target, mismatched_subtree);

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    assert_eq!(
        verdict_ctor(&outcome.output),
        wk::VERDICT_FAILS,
        "expected Fails verdict for proposition-mismatch case"
    );
    let diag = verdict_diagnostic(&outcome.output).expect("Fails carries diagnostic");
    assert!(
        diag.contains("witness") || diag.contains("certificate") || diag.contains("admit"),
        "diagnostic should mention witness failure, got: {diag}"
    );
}

#[test]
fn end_to_end_validate_fails_when_target_iri_lacks_declaration_trace() {
    // Contrast: target IRI is named in the certificate but no
    // DeclarationTrace was committed for it. The witness index has
    // no key matching the certificate's claim → synthesis fails →
    // Verdict::Fails. Demonstrates the soundness boundary against
    // forged citations.
    let ctx = build_full_chain(); // no axiom chain layer added

    let target = "urn:test:phase10:not_committed";
    let asserts_subtree = json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
            {"ctor": "LitString", "args": [target]},
        ],
    });

    let proposition = Value::Json(asserts_subtree.clone());
    let justification = Value::Json(json!({
        "ctor": "Declared",
        "args": [target],
    }));
    let certificate = justified_by_declared_certificate(target, asserts_subtree);

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    assert_eq!(
        verdict_ctor(&outcome.output),
        wk::VERDICT_FAILS,
        "expected Fails verdict for missing-trace case"
    );
}

#[test]
fn arity_mismatch_in_certificate_surfaces_verdict_fails() {
    // Regression check on the arity-mismatch path: a certificate
    // whose justification:Certificate.declared application is missing the witness
    // arg slot (1 App-arg instead of 3) fails the kernel's
    // `check_inductive_ctor_args` arity assertion. Verdict is Fails
    // for a different reason than missing-witness — confirms the
    // upstream check still catches structurally-broken certificates
    // before the Phase 9 witness-synthesis hook runs.
    let ctx = build_full_chain();

    let proposition = Value::Json(json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
            {"ctor": "LitString", "args": ["urn:foo"]},
        ],
    }));
    let justification = Value::Json(json!({
        "ctor": "Declared",
        "args": ["urn:foo"],
    }));
    // Certificate with only ONE App-arg — `justification:Certificate.declared`
    // expects three (iri, P, witness).
    let certificate = Value::Json(json!({
        "ctor": "App",
        "args": [
            {"ctor": "CtorApp", "args": [
                "urn:eigenius:justification:Certificate",
                "declared",
            ]},
            {"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]},
        ],
    }));

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_FAILS);
}

// ── eigenius#200: a passing check mints a VerificationTrace ──────────

/// The sentence IRI used by the two `VerificationTrace` tests below.
const TRACED_SENTENCE: &str = "urn:test:v200:sentence";

/// Assemble the one judgement a conclusion now carries from the three parts
/// that used to be separate slots: `holds(kernel, cert, Certificate(j, P))`.
fn judgement(proposition: Value, justification: Value, cert: Value) -> Value {
    use eigenius_kernel::program::eigentt_type_mirror::{certificate_type, encode_judgement};
    let typ =
        certificate_type(&d47(&justification), &proposition).expect("certificate type encodes");
    encode_judgement("urn:eigenius:eigentt:logic_kernel", &cert, &typ).expect("judgement encodes")
}

/// Re-encode a plain D32 §3.7 tagged-dict `justification:Term` into the D47
/// form a term embedded in a judgement must carry.
///
/// This conversion is the encoding boundary the collapse moved. A justification
/// term used to sit in a slot of its own as a plain `{"ctor", "args"}` dict; it
/// now rides inside the judgement, which is an `eigentt:Term`-ranged value, so
/// the D47 codec reads it and a foreign inductive's constructor is named by
/// `CtorApp` with arguments folded through `App`. Callers below still write the
/// plain shape because it is what an author reads.
fn d47(v: &Value) -> Value {
    const JT: &str = "urn:eigenius:justification:Term";
    let Value::Json(j) = v else { return v.clone() };
    let (Some(name), args) = (
        j.get("ctor").and_then(serde_json::Value::as_str),
        j.get("args")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default(),
    ) else {
        return v.clone();
    };
    let mut acc = json!({"ctor": "CtorApp", "args": [JT, name]});
    for a in args {
        let arg = match &a {
            serde_json::Value::String(s) => json!({"ctor": "LitString", "args": [s]}),
            serde_json::Value::Object(_) => match d47(&Value::Json(a.clone())) {
                Value::Json(x) => x,
                _ => a.clone(),
            },
            other => other.clone(),
        };
        acc = json!({"ctor": "App", "args": [acc, arg]});
    }
    Value::Json(acc)
}

/// A `justification:Conclusion` with a real IRI (unlike `synthetic_sentence`, which is embedded and so has
/// nothing to attest).
fn iri_sentence(iri_str: &str, proposition: Value, justification: Value, cert: Value) -> Resource {
    let mut r = Resource::new(Iri::parse(iri_str).unwrap());
    r.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse("urn:eigenius:justification:Conclusion").unwrap(),
        )]),
    );
    r.set(
        Iri::parse(iris::PROP_JUDGEMENT).unwrap(),
        judgement(proposition, justification, cert),
    );
    r
}

/// Assemble the passing sentence the two tests share, plus its context.
fn passing_traced_sentence() -> (ExecutionContext, Resource, serde_json::Value) {
    let target = "urn:test:v200:axiom";
    let ctx = build_chain_with_declared_axiom(target);
    let asserts_subtree = json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
            {"ctor": "LitString", "args": [target]},
        ],
    });
    let sentence = iri_sentence(
        TRACED_SENTENCE,
        Value::Json(asserts_subtree.clone()),
        Value::Json(json!({"ctor": "Declared", "args": [target]})),
        justified_by_declared_certificate(target, asserts_subtree.clone()),
    );
    (ctx, sentence, asserts_subtree)
}

#[test]
fn passing_validation_emits_a_kernel_verification_trace() {
    // D39 §5: the trace and the witness are two projections of one validator event. That held for
    // Declared, Observed and Derived and not for Verified — nothing in the kernel ever created a
    // `VerificationTrace`, so every Verified witness was traceless (eigenius#200).
    let (ctx, sentence, _) = passing_traced_sentence();
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    assert_eq!(verdict_ctor(&outcome.output), wk::VERDICT_HOLDS);

    let trace = match outcome.derivations.as_slice() {
        [t] => t,
        other => panic!(
            "expected exactly one VerificationTrace, got {}",
            other.len()
        ),
    };
    let get = |p: &str| {
        trace
            .get(&Iri::parse(p).unwrap())
            .and_then(|v| v.as_str().map(str::to_string))
    };
    assert!(trace
        .is_a()
        .iter()
        .any(|c| c.as_str() == wk::VERIFICATION_TRACE));
    assert_eq!(
        get(wk::REFLECTION_RESOURCE).as_deref(),
        Some(TRACED_SENTENCE)
    );
    // The verifier is what distinguishes this from a Lean trace — same class, different prover.
    assert_eq!(
        get(wk::PROOF_SYSTEM).as_deref(),
        Some("urn:eigenius:kernel")
    );
    // The certificate lives on the sentence, so the sentence IS the proof term's location.
    assert_eq!(get(wk::PROOF_TERM).as_deref(), Some(TRACED_SENTENCE));
    assert!(get(wk::TIMESTAMP).is_some(), "trace carries a timestamp");
    // `derivation_trace` is `recommends`, not `requires`: a justification:Conclusion has no ProgramTrace
    // to point at, and pointing the slot at itself would be a fiction.
    assert!(
        trace
            .get(&Iri::parse("urn:eigenius:reflection:derivation_trace").unwrap())
            .is_none(),
        "the kernel case must not invent a derivation_trace"
    );
}

#[test]
fn the_minted_trace_keys_the_witness_on_the_sentences_own_proposition() {
    // The subtle half. `emit_from_trace` reads the TARGET's `reflection:canonical_proposition`,
    // but a justification:Conclusion keeps its proposition under `justification:proposition`. Without the
    // justification:Conclusion arm in `target_proposition_hash` the trace falls through to the D39 §4.1
    // default and keys the witness against `Asserts(sentence_iri)` — a different hash from the one
    // the sentence emits, and one no certificate legitimately cites. A chain could then discharge
    // `justification:Certificate(Verified(s), Asserts(s))`: the sentence asserting itself.
    //
    // The trace is committed in a CHILD of the layer holding the sentence, so
    // `layer_admits_witness`'s self-attesting step (which is layer-LOCAL) cannot answer and the
    // trace path is the only one under test. Committing them together — what the gate actually
    // does — lets the sentence answer first and would hide the difference.
    use eigenius_kernel::layer::layer_admits_witness;
    use eigenius_kernel::witness::{WitnessCategory, WitnessKey};

    let (ctx, sentence, asserts_subtree) = passing_traced_sentence();
    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns outcome");
    let trace = outcome.derivations[0].clone();

    let mut b = LayerBuilder::new("v200-sentence", Some(ctx.head().clone()));
    b.add_resource(sentence).unwrap();
    let with_sentence = Arc::new(b.build(LayerStorage::in_memory()));

    let mut b2 = LayerBuilder::new("v200-trace", Some(with_sentence));
    b2.add_resource(trace).unwrap();
    let layer = Arc::new(b2.build(LayerStorage::in_memory()));

    let key_for = |subtree: serde_json::Value| {
        let exp = eigenius_kernel::program::eigentt_type_mirror::decode_type(
            &Value::Json(subtree),
            &layer,
        )
        .expect("proposition decodes");
        WitnessKey::from_exp(
            WitnessCategory::Verified,
            Iri::parse(TRACED_SENTENCE).unwrap(),
            &exp,
        )
        .expect("witness key builds")
    };

    assert!(
        layer_admits_witness(&layer, &key_for(asserts_subtree)),
        "the trace must admit the sentence's OWN proposition as its Verified key"
    );
    assert!(
        !layer_admits_witness(
            &layer,
            &key_for(json!({
                "ctor": "App",
                "args": [
                    {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
                    {"ctor": "LitString", "args": [TRACED_SENTENCE]},
                ],
            }))
        ),
        "the trace must NOT admit `Asserts(sentence_iri)` — the sentence does not assert itself"
    );
}

// ── eigenius#205: a declared-external execution admits Declared, never Derived ──

#[test]
fn an_external_execution_trace_admits_declared_not_derived() {
    // `Derived` holds a trace tied to a KERNEL-INITIATED activity — running a program, invoking an
    // institution, a query that writes back. An author writing down that a program ran elsewhere is
    // making a different claim: there is no `f : I -> O`, so no specification, so nothing entailed
    // (D73 §3.3). `ExternalExecutionTrace` carries that claim and `trace_category` maps it to
    // Declared.
    //
    // The kernel cannot tell a hand-authored `ProgramTrace` from one it minted — no "kernel-only,
    // refused from input" mechanism exists anywhere in the validator — so the distinction has to be
    // carried by the CLASS. This test is that distinction.
    use eigenius_kernel::layer::layer_admits_witness;
    use eigenius_kernel::witness::{WitnessCategory, WitnessKey};

    let ctx = build_full_chain();
    let target = "urn:test:v205:transcribed";
    let target_iri = Iri::parse(target).unwrap();

    let prop = json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts"]},
            {"ctor": "LitString", "args": [target]},
        ],
    });

    // The artifact whose values were transcribed from a run the kernel never invoked.
    let mut artifact = Resource::new(target_iri.clone());
    artifact.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk::DECLARED_RESOURCE).unwrap(),
        )]),
    );
    artifact.set(
        Iri::parse(wk::DECLARED_BY).unwrap(),
        Value::String("urn:eigenius:reflection:agent:unattributed".into()),
    );
    artifact.set(
        Iri::parse(wk::CANONICAL_PROPOSITION).unwrap(),
        Value::Json(prop.clone()),
    );

    let mut trace = Resource::new(Iri::parse("urn:test:v205:transcribed-trace").unwrap());
    trace.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk::EXTERNAL_EXECUTION_TRACE).unwrap(),
        )]),
    );
    trace.set(
        Iri::parse(wk::REFLECTION_RESOURCE).unwrap(),
        Value::ResourceRef(target_iri.clone()),
    );
    trace.set(
        Iri::parse(wk::DECLARED_BY).unwrap(),
        Value::String("urn:eigenius:reflection:agent:unattributed".into()),
    );
    trace.set(
        Iri::parse("urn:eigenius:reflection:source").unwrap(),
        Value::String("R 4.3.3 recompute run outside the kernel (linked-external)".into()),
    );

    let mut b = LayerBuilder::new("v205", Some(ctx.head().clone()));
    b.add_resource(artifact).unwrap();
    b.add_resource(trace).unwrap();
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let exp =
        eigenius_kernel::program::eigentt_type_mirror::decode_type(&Value::Json(prop), &layer)
            .expect("proposition decodes");
    let key = |c| WitnessKey::from_exp(c, target_iri.clone(), &exp).expect("key builds");

    assert!(
        layer_admits_witness(&layer, &key(WitnessCategory::Declared)),
        "an ExternalExecutionTrace must admit IsDeclaredAs — someone asserts the run happened"
    );
    assert!(
        !layer_admits_witness(&layer, &key(WitnessCategory::Derived)),
        "and must NOT admit IsDerivedAs — no kernel-initiated activity produced this"
    );
}
