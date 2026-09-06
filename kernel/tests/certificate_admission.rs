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

//! **A certificate is type-checked at commit** — the witness machinery end to end.
//!
//! A conclusion's judgement carries `holds(logic, c, Certificate(j, P))`. Rule 21 decodes it,
//! checks `Certificate(j, P)` is a type, and checks `c` against it. Checking `c` drives the
//! kernel's `synthesize_chain_witness`, which admits a ctor argument only if the witness index
//! holds a matching key — so these tests exercise chain → witness index → synthesis →
//! admission, and the soundness boundary where admission must FAIL.
//!
//! Rehomed from `crates/eigenius-reasoning/tests/validate_handler.rs` at P7, which dispatched
//! `ValidateJustification` through the Reasoning institution. That handler no longer owns the
//! check — P2 moved it to commit — and P7 dissolves the institution, so the tests now commit
//! the sentence into a layer and read what validation reports. That is a stronger test than
//! the handler call it replaces: it exercises the path a real commit takes, rather than a
//! detached call on a resource that was never committed.
//!
//! What did NOT come with them: the institution-dispatch and `InstitutionError` shape tests
//! (they assert machinery P7 deletes), the EntailmentQuery and ConsistencyCheck tests (P7
//! deletes both QueryClasses), and the `VerificationTrace` tests — the only minter in the tree
//! went with the reasoning crate, and the Lean institution becomes the producer of `Verified`
//! witnesses under eigenius#160, which is where that property gets re-pinned.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::testing::term_value;
use serde_json::json;

/// Commit `sentence` onto the chain and return every validation error naming it.
///
/// The handler these tests used to call took a DETACHED `Resource`. Validation works on a
/// layer, so the sentence is committed the way a real one is. Errors are filtered to the
/// sentence itself; the surrounding chain is asserted clean separately by
/// [`assert_chain_is_clean`], so a fixture gap cannot be mistaken for a certificate failure.
fn commit_and_validate(ctx: &ExecutionContext, sentence: Resource) -> Vec<String> {
    let iri = sentence
        .id()
        .cloned()
        .expect("a committed sentence carries an @id");
    let mut b = LayerBuilder::new("probe", Some(ctx.head().clone()));
    b.add_resource(sentence).expect("sentence adds");
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    eigenius_kernel::validation::Validator::new(layer)
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref().is_some_and(|i| *i == iri))
        .map(|e| e.message)
        .collect()
}

/// Every validation error the base chain reports, whatever resource it names.
fn chain_errors(ctx: &ExecutionContext) -> Vec<String> {
    eigenius_kernel::validation::Validator::new(ctx.head().clone())
        .validate()
        .into_iter()
        .map(|e| format!("[{:?}] {}", e.resource_id, e.message))
        .collect()
}

/// Stand up the layer chain a conclusion needs to validate:
/// core → reflection (+ eigentt fragment) → prov → justification. Mirrors the
/// kernel `esl::compile::tests::reasoning_ontology_resolves_through_codec`
/// helper.
fn build_full_chain() -> ExecutionContext {
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

    // Compiled against `prov`, the layer it is about to sit on — not against an empty one.
    // Its values name their constructors' arguments (D85 §6.1), and those names live in
    // `eigentt:Term`'s declaration down the chain.
    let reasoning_source = include_str!("../../ontologies/justification/justification.esl");
    let reasoning_resources =
        esl::compile(reasoning_source, &prov).expect("reasoning.esl compiles");

    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(prov));
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
/// values. Tests pass the three parts directly; the helper folds them into the one judgement
/// slot a conclusion carries and stamps the `is_a` a committed sentence would have.
/// The IRI every probe sentence is committed under.
///
/// It was `Resource::new_embedded()` while the handler took a detached resource. A resource
/// that gets committed has an identity, so it gets one here.
const PROBE_SENTENCE: &str = "urn:test:probe:sentence";

fn synthetic_sentence(
    proposition: Option<Value>,
    justification: Option<serde_json::Value>,
    certificate: Option<Value>,
) -> Resource {
    let mut r = Resource::new(Iri::parse(PROBE_SENTENCE).unwrap());
    r.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(
            Iri::parse("urn:eigenius:justification:Conclusion")
                .unwrap()
                .as_str()
                .to_string(),
        )]),
    );
    // The three parts are now one slot, so "a part is missing" becomes "the
    // judgement is missing": with the parts collapsed into the certificate
    // type there is no way to supply two of three. The error paths these
    // callers exercise are unchanged in kind — validation still reports a
    // conclusion it cannot read — but there is now one way to be unreadable
    // instead of three.
    if let (Some(p), Some(j), Some(c)) = (proposition, justification, certificate) {
        r.set(
            Iri::parse("urn:eigenius:justification:judgement").unwrap(),
            judgement(p, j, c),
        );
    }
    r
}
/// Assemble the one judgement a conclusion carries: `holds(kernel, cert, Certificate(P))`.
///
/// It took a separate justification term until the D88 §2 merge. The certificate IS the term now,
/// so `cert` is the only derivation here and the type carries the proposition alone. The
/// `justification` argument is retained so the callers below keep reading as the shapes they are
/// about; it is no longer part of what gets encoded.
fn judgement(proposition: Value, _justification: serde_json::Value, cert: Value) -> Value {
    use eigenius_kernel::program::eigentt_type_mirror::{certificate_type, encode_judgement};
    let typ = certificate_type(&proposition, codec()).expect("certificate type encodes");
    encode_judgement("urn:eigenius:eigentt:logic_kernel", &cert, &typ, codec())
        .expect("judgement encodes")
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
        Value::Array(vec![Value::String(
            Iri::parse(wk_local::CLASS).unwrap().as_str().to_string(),
        )]),
    );
    // `core:Class` REQUIRES both. The handler these tests used to call took a
    // detached resource, so the fixture could omit them and nothing noticed;
    // committing the sentence validates the layer it lands in, which does.
    target.set(
        Iri::parse(wk::SHORT_NAME).unwrap(),
        Value::String("probe_axiom".to_string()),
    );
    target.set(
        Iri::parse(wk::DESCRIPTION).unwrap(),
        Value::String("A declared axiom standing in for a real class under test.".to_string()),
    );

    // The DeclarationTrace pointing at the target. Its presence is
    // what makes `build_witness_index` emit the Declared witness key.
    let trace_iri_str = format!("{target_iri_str}-decl-trace");
    let mut trace = Resource::new(Iri::parse(&trace_iri_str).unwrap());
    trace.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(
            Iri::parse(wk_local::DECLARATION_TRACE)
                .unwrap()
                .as_str()
                .to_string(),
        )]),
    );
    trace.set(
        Iri::parse(wk_local::REFLECTION_RESOURCE).unwrap(),
        Value::iri(&target_iri.clone()),
    );
    // `prov:DeclarationTrace` requires both alongside `prov:resource`.
    trace.set(
        Iri::parse("urn:eigenius:prov:timestamp").unwrap(),
        Value::String("2026-01-01T00:00:00Z".to_string()),
    );
    trace.set(
        Iri::parse("urn:eigenius:prov:was_attributed_to").unwrap(),
        Value::iri(&Iri::parse("urn:eigenius:prov:agent:unattributed").unwrap()),
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
    term_value(&json!({
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
    let prop_value = encode_type(&prop_exp, codec()).expect("encode Asserts(iri)");

    let target_iri = Iri::parse(target_iri_str).unwrap();
    let mut target = Resource::new(target_iri.clone());
    target.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(
            Iri::parse(wk_local::CLASS).unwrap().as_str().to_string(),
        )]),
    );
    // `core:Class` REQUIRES both. The handler these tests used to call took a
    // detached resource, so the fixture could omit them and nothing noticed;
    // committing the sentence validates the layer it lands in, which does.
    target.set(
        Iri::parse(wk::SHORT_NAME).unwrap(),
        Value::String("probe_axiom".to_string()),
    );
    target.set(
        Iri::parse(wk::DESCRIPTION).unwrap(),
        Value::String("A declared axiom standing in for a real class under test.".to_string()),
    );
    target.set(
        Iri::parse(wk_local::CANONICAL_PROPOSITION).unwrap(),
        prop_value,
    );

    let trace_iri_str = format!("{target_iri_str}-decl-trace");
    let mut trace = Resource::new(Iri::parse(&trace_iri_str).unwrap());
    trace.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(
            Iri::parse(wk_local::DECLARATION_TRACE)
                .unwrap()
                .as_str()
                .to_string(),
        )]),
    );
    trace.set(
        Iri::parse(wk_local::REFLECTION_RESOURCE).unwrap(),
        Value::iri(&target_iri.clone()),
    );
    // `prov:DeclarationTrace` requires both alongside `prov:resource`.
    trace.set(
        Iri::parse("urn:eigenius:prov:timestamp").unwrap(),
        Value::String("2026-01-01T00:00:00Z".to_string()),
    );
    trace.set(
        Iri::parse("urn:eigenius:prov:was_attributed_to").unwrap(),
        Value::iri(&Iri::parse("urn:eigenius:prov:agent:unattributed").unwrap()),
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

/// The fixture chain itself validates clean.
///
/// Every test below reads errors filtered to its own synthetic sentence. That filter is only
/// honest if the chain underneath carries no errors of its own — otherwise a "clean" result
/// The D47 codec's constructor argument names, from the bootstrap chain, built once.
///
/// Encoding a term names its constructor's arguments (D85 §6.1), and the names live in
/// `eigentt:Term` and `core:Level`'s declarations — so an encode needs a chain.
fn codec() -> &'static eigenius_kernel::program::eigentt_type_mirror::CodecNames {
    static NAMES: std::sync::OnceLock<eigenius_kernel::program::eigentt_type_mirror::CodecNames> =
        std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        eigenius_kernel::program::eigentt_type_mirror::CodecNames::from_layer(
            eigenius_kernel::bootstrap::bootstrap()
                .expect("bootstrap")
                .head(),
        )
    })
}

/// could be a filter hiding a broken fixture. This asserts the premise directly.
#[test]
fn the_fixture_chain_validates_clean() {
    for (name, ctx) in [
        ("full", build_full_chain()),
        (
            "declared-axiom",
            build_chain_with_declared_axiom("urn:test:probe:axiom"),
        ),
        (
            "explicit-canonical",
            build_chain_with_explicit_canonical_proposition("urn:test:probe:axiom"),
        ),
    ] {
        let errors = chain_errors(&ctx);
        assert!(
            errors.is_empty(),
            "the {name} chain must validate clean before any sentence is committed onto it; got:\n{}",
            errors.join("\n")
        );
    }
}

#[test]
fn an_explicit_canonical_proposition_admits_the_same_witness_as_the_default() {
    // gh #75 regression check: a target resource with an *explicit*
    // canonical_proposition (encoded via the D47 codec) must produce
    // a witness whose prop_hash matches what the synthesis hook
    // computes by eval+readback+encode on the certificate's
    // proposition. Pre-fix the encoder used `decl.name` for the
    // ConstRef slot — chain-author tools using IRI-shaped names and
    // resolver-built decls using short names produced different
    // bytes. Post-fix both read `decl.iri`, the bytes agree, the
    // witness is admitted, and the certificate type-checks.
    let target = "urn:test:phase10:explicit-axiom";
    let ctx = build_chain_with_explicit_canonical_proposition(target);

    let asserts_subtree = json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts", []]},
            {"ctor": "LitString", "args": [target]},
        ],
    });
    let proposition = term_value(&asserts_subtree);
    let justification = json!({
        "ctor": "Declared",
        "args": [target],
    });
    let certificate = justified_by_declared_certificate(target, asserts_subtree);

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let errors = commit_and_validate(&ctx, sentence);
    assert!(
        errors.is_empty(),
        "explicit canonical_proposition should validate clean (gh #75); got:\n{}",
        errors.join("\n")
    );
}

#[test]
fn a_certificate_matching_an_admitted_witness_type_checks() {
    // The headline test: a complete justified-reasoning commit
    // validates clean. Chain has a DeclarationTrace
    // emitting an admitted `IsDeclaredAs(target, Asserts(target))`
    // witness; the certificate's `justification:Certificate.declared` ctor's third
    // arg slot is filled in by the kernel's Phase 9 synthesis hook;
    // the type-check succeeds.
    let target = "urn:test:phase10:axiom";
    let ctx = build_chain_with_declared_axiom(target);

    let asserts_subtree = json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts", []]},
            {"ctor": "LitString", "args": [target]},
        ],
    });

    let proposition = term_value(&asserts_subtree);
    let justification = json!({
        "ctor": "Declared",
        "args": [target],
    });
    let certificate = justified_by_declared_certificate(target, asserts_subtree);

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let errors = commit_and_validate(&ctx, sentence);
    assert!(
        errors.is_empty(),
        "a certificate matching an admitted witness must validate clean; got:\n{}",
        errors.join("\n")
    );
}

#[test]
fn a_certificate_citing_the_wrong_proposition_is_rejected() {
    // Contrast: the chain admits a witness for `Asserts(target)`, but
    // the certificate claims `Asserts(different_iri)` as the
    // proposition. The witness lookup misses (the prop_hash differs),
    // so the synthesis hook surfaces a "no admitted witness" error
    // and the commit is rejected. Locks the
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
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts", []]},
            {"ctor": "LitString", "args": [mismatched]},
        ],
    });

    let proposition = term_value(&mismatched_subtree);
    // The justification still cites `target` (a valid Declared
    // grounding), but the proposition the certificate claims doesn't
    // match what the chain admits for that resource.
    let justification = json!({
        "ctor": "Declared",
        "args": [target],
    });
    let certificate = justified_by_declared_certificate(target, mismatched_subtree);

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let errors = commit_and_validate(&ctx, sentence);
    assert!(
        !errors.is_empty(),
        "a proposition the chain never admitted must not type-check"
    );
    let joined = errors.join("\n");
    assert!(
        joined.contains("no admitted IsDeclaredAs witness for IRI urn:test:phase10:axiom"),
        "the miss must be reported as an unadmitted witness for the target IRI, got: {joined}"
    );
}

#[test]
fn a_certificate_citing_an_untraced_iri_is_rejected() {
    // Contrast: target IRI is named in the certificate but no
    // DeclarationTrace was committed for it. The witness index has
    // no key matching the certificate's claim, so synthesis fails.
    // Demonstrates the soundness boundary against forged citations.
    let ctx = build_full_chain(); // no axiom chain layer added

    let target = "urn:test:phase10:not_committed";
    let asserts_subtree = json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts", []]},
            {"ctor": "LitString", "args": [target]},
        ],
    });

    let proposition = term_value(&asserts_subtree);
    let justification = json!({
        "ctor": "Declared",
        "args": [target],
    });
    let certificate = justified_by_declared_certificate(target, asserts_subtree);

    let sentence = synthetic_sentence(Some(proposition), Some(justification), Some(certificate));
    let errors = commit_and_validate(&ctx, sentence);
    assert!(
        errors.iter().any(|e| e
            .contains("no admitted IsDeclaredAs witness for IRI urn:test:phase10:not_committed")),
        "an IRI with no DeclarationTrace admits no witness; got:\n{}",
        errors.join("\n")
    );
}

#[test]
fn arity_mismatch_in_certificate_is_rejected() {
    // Regression check on the arity-mismatch path: a certificate
    // whose justification:Certificate.declared application is missing the witness
    // arg slot (1 App-arg instead of 3) fails the kernel's
    // `check_inductive_ctor_args` arity assertion. It is rejected for
    // a different reason than missing-witness — confirming the
    // upstream check still catches structurally-broken certificates
    // before the witness-synthesis hook runs.
    let ctx = build_full_chain();

    let proposition = term_value(&json!({
        "ctor": "App",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts", []]},
            {"ctor": "LitString", "args": ["urn:foo"]},
        ],
    }));
    let justification = json!({
        "ctor": "Declared",
        "args": ["urn:foo"],
    });
    // Certificate with only ONE App-arg — `justification:Certificate.declared`
    // expects three (iri, P, witness).
    let certificate = term_value(&json!({
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
    let errors = commit_and_validate(&ctx, sentence);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("type mismatch: Sort(Succ(Zero)) \u{2260} EigonPrimitive(String)")),
        "an arity mismatch must be reported as the type mismatch it is, got:\n{}",
        errors.join("\n")
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
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Asserts", []]},
            {"ctor": "LitString", "args": [target]},
        ],
    });

    // The artifact whose values were transcribed from a run the kernel never invoked.
    let mut artifact = Resource::new(target_iri.clone());
    artifact.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(
            Iri::parse(wk::CLASS).unwrap().as_str().to_string(),
        )]),
    );
    artifact.set(
        Iri::parse(wk::DECLARED_BY).unwrap(),
        Value::String("urn:eigenius:prov:agent:unattributed".into()),
    );
    artifact.set(
        Iri::parse(wk::CANONICAL_PROPOSITION).unwrap(),
        term_value(&prop),
    );

    let mut trace = Resource::new(Iri::parse("urn:test:v205:transcribed-trace").unwrap());
    trace.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(
            Iri::parse(wk::EXTERNAL_EXECUTION_TRACE)
                .unwrap()
                .as_str()
                .to_string(),
        )]),
    );
    trace.set(
        Iri::parse(wk::REFLECTION_RESOURCE).unwrap(),
        Value::iri(&target_iri.clone()),
    );
    trace.set(
        Iri::parse(wk::DECLARED_BY).unwrap(),
        Value::String("urn:eigenius:prov:agent:unattributed".into()),
    );
    trace.set(
        Iri::parse("urn:eigenius:prov:was_generated_by").unwrap(),
        Value::String("R 4.3.3 recompute run outside the kernel (linked-external)".into()),
    );

    let mut b = LayerBuilder::new("v205", Some(ctx.head().clone()));
    b.add_resource(artifact).unwrap();
    b.add_resource(trace).unwrap();
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let exp =
        eigenius_kernel::program::eigentt_type_mirror::decode_type(&term_value(&prop), &layer)
            .expect("proposition decodes");
    let key = |c| WitnessKey::from_exp(c, target_iri.clone(), &exp, codec()).expect("key builds");

    assert!(
        layer_admits_witness(&layer, &key(WitnessCategory::Declared)),
        "an ExternalExecutionTrace must admit IsDeclaredAs — someone asserts the run happened"
    );
    assert!(
        !layer_admits_witness(&layer, &key(WitnessCategory::Observed)),
        "and must NOT admit IsObservedAs — nobody measured anything; the assertion is that a \
         program ran elsewhere"
    );
}

// ── An ill-formed judgement is rejected at commit ────────────────────
//
// Rehomed from the four `*_surfaces_computation_failed` / `*_surfaces_verdict_fails` tests.
// They asserted the shape of the handler's return — `InstitutionError::ComputationFailed` for
// a missing part, `Verdict::Fails` with a diagnostic for a malformed one. Neither shape
// survives P7, but the property under both does, and it is the kernel's: P2 moved the
// certificate check to commit, so a conclusion the kernel cannot read is rejected there.

#[test]
fn a_conclusion_with_no_judgement_is_rejected() {
    // `justification:Conclusion` requires `justification:judgement`
    // (ontologies/justification/justification.esl:303). It used to require three separate
    // slots checked by three paths, with nothing requiring them to be about the same claim.
    let ctx = build_full_chain();
    let errors = commit_and_validate(&ctx, synthetic_sentence(None, None, None));
    assert!(
        errors
            .iter()
            .any(|e| e.contains("urn:eigenius:justification:judgement") && e.contains("missing")),
        "a Conclusion carrying no judgement must be rejected, got:\n{}",
        errors.join("\n")
    );
}

#[test]
fn a_judgement_the_codec_cannot_read_is_rejected() {
    // `UnitVal` is a perfectly good `eigentt:Term`; it is not a judgement. The slot is
    // `eigentt:Judgement`-ranged, so Rule 21 reports it twice — once decoding the
    // judgement, once reading the conclusion's justification — and both name the slot.
    //
    // The fixture used to be `NotARealCtor`, a constructor no inductive declares. That is
    // no longer expressible: a value states its constructor's CLASS (D85 §6.1), so a name
    // the chain does not have has no class to name, and the fixture builder refuses it
    // rather than the codec. The remaining unreadable case is the one here — a value the
    // codec reads fine, in a slot that wanted something else.
    let ctx = build_full_chain();
    let mut sentence = synthetic_sentence(None, None, None);
    sentence.set(
        Iri::parse("urn:eigenius:justification:judgement").unwrap(),
        term_value(&json!({"ctor": "UnitVal", "args": []})),
    );
    let errors = commit_and_validate(&ctx, sentence);
    let joined = errors.join("\n");
    assert!(
        joined.contains("does not decode as an eigentt:Judgement"),
        "the error must name the slot, got: {joined}"
    );
}

#[test]
fn a_proposition_that_is_a_sort_rather_than_a_term_is_rejected() {
    // `Sort(Zero)` is `Prop` ITSELF, not a term inhabiting it. It sat in the old fixtures
    // commented "Valid Prop term", unnoticed because the proposition was a slot of its own that
    // nothing checked for propositionhood. Inside a judgement it is the second index of
    // `Certificate(j, P)`, where `P : Prop` is checked.
    let ctx = build_full_chain();
    let sentence = synthetic_sentence(
        Some(term_value(
            &json!({"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]}),
        )),
        Some(json!({"ctor": "Declared", "args": ["urn:a"]})),
        Some(term_value(&json!({"ctor": "UnitVal", "args": []}))),
    );
    let errors = commit_and_validate(&ctx, sentence);
    let joined = errors.join("\n");
    assert!(
        joined.contains("`type` field is not a type")
            && joined.contains("universe stratification: Sort(0) does not inhabit Sort(0)"),
        "supplying Prop where a proposition belongs must fail `check_type` on the certificate \
         type, got: {joined}"
    );
}
