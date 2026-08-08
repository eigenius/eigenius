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

//! `BridgedClaimGrader` — a parsed proposition lifted to a DOMAIN proposition, and the edit that
//! breaks it.
//!
//! The positive proves the composition: a claim already on chain under a `ProgramTrace` (a parser
//! output, `IsDerivedAs claim P`) plus a Declared bridge `P → C` yields a `ReasoningSentence` whose
//! Artemov-application certificate the D39 gate admits — `Verdict::Holds`.
//!
//! The negative is `demo/prose-to-chain`'s whole thesis, in miniature and without a lexicon
//! snapshot: **hold the recorded argument fixed and re-derive the claim from edited prose.** The
//! parser produces `P′ ≠ P`, so the chain carries `IsDerivedAs claim P′` while the certificate still
//! cites `derived(claim, P, _)`. The witness lookup misses, the certificate does not type-check, and
//! the gate returns `Fails`. Nothing compares the two propositions for similarity, and nothing
//! compares the two texts at all — the argument is rejected because it no longer follows.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::term::{Exp, InductiveDecl, Patt};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::encode_type;
use eigenius_reasoning::grade::BridgedClaimGrader;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::{ClaimGrader, ClaimSource, ReasoningInstitution, Warrant};

const CLAIM: &str = "urn:eigenius:doc:demo:claim_1";
const PREDICATE: &str = "urn:eigenius:demo:onco:RequiresActivity";

/// core → reflection (+ eigentt + institution) → reasoning → a two-place domain predicate.
/// Mirrors `grade.rs::build_full_chain`, with the domain layer the bridge's consequent needs.
fn build_chain() -> Arc<Layer> {
    let mut core_builder = LayerBuilder::new("core", None);
    for r in eigon_json::parse_document(include_str!("../../../ontologies/core/core-ontology.json"))
        .unwrap()
    {
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

    // The domain vocabulary — the same shape as `onco:RequiresActivity` in the WRN case study.
    let domain_src = r#"
        namespace core = "urn:eigenius:core";
        namespace onco = "urn:eigenius:demo:onco";
        data onco:RequiresActivity : core:string -> core:string -> Prop { }
    "#;
    let mut domain_builder = LayerBuilder::new("domain", Some(reasoning));
    for r in esl::compile(domain_src).expect("domain ESL compiles") {
        domain_builder.add_resource(r).unwrap();
    }
    Arc::new(domain_builder.build(LayerStorage::in_memory()))
}

/// A stand-in for a parser's `item.sem()`. The two variants differ the way an edited sentence's
/// reading differs from the original's: same shape, different content.
fn parsed_prop(content_iri: &str) -> Exp {
    let asserts = Iri::parse("urn:eigenius:core:Asserts").unwrap();
    let decl = Arc::new(InductiveDecl {
        iri: asserts.clone(),
        name: asserts.local_name().to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::Sort(0),
        ctors: Vec::new(),
    });
    Exp::InductiveType(decl, vec![Exp::LitString(content_iri.to_string())])
}

/// The parser's output as the chain carries it: a `DerivedResource` holding the proposition, under a
/// `ProgramTrace` — which is what mints `IsDerivedAs CLAIM prop`.
fn parser_output(prop: &Exp) -> Vec<Resource> {
    let iri = |s: &str| Iri::parse(s).unwrap();
    let mut claim = Resource::new(iri(CLAIM));
    claim.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::DERIVED_RESOURCE))]),
    );
    claim.set(
        iri(wk::CANONICAL_PROPOSITION),
        encode_type(prop).expect("prop encodes"),
    );

    let mut trace = Resource::new(iri("urn:eigenius:doc:demo:trace_1"));
    trace.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::PROGRAM_TRACE))]),
    );
    trace.set(iri(wk::REFLECTION_RESOURCE), Value::ResourceRef(iri(CLAIM)));
    trace.set(
        iri("urn:eigenius:reflection:source"),
        Value::String("DCG parse (D63) of the source span".to_string()),
    );
    trace.set(
        iri("urn:eigenius:reflection:timestamp"),
        Value::String("2026-08-03T00:00:00Z".to_string()),
    );
    vec![claim, trace]
}

fn grader<'a>(args: &'a [String]) -> BridgedClaimGrader<'a> {
    BridgedClaimGrader {
        claim_iri: CLAIM,
        predicate: PREDICATE,
        args,
        declared_by: "chan-et-al-2019:results-p1",
        rationale: "The sentence, so read, warrants the domain claim.",
        timestamp: "2026-08-03T00:00:00Z",
    }
}

fn commit_and_validate(
    base: &Arc<Layer>,
    resources: Vec<Resource>,
    sentence_iri: &Iri,
) -> Resource {
    let sentence = resources
        .iter()
        .find(|r| r.id() == Some(sentence_iri))
        .expect("the sentence is among the committed resources")
        .clone();
    let mut builder = LayerBuilder::new("doc-claims", Some(Arc::clone(base)));
    for r in resources {
        builder.add_resource(r).unwrap();
    }
    let layer = Arc::new(builder.build(LayerStorage::in_memory()));
    let _ = layer.chain_witness_index();
    let ctx = ExecutionContext::new(
        layer,
        "bridged-grade-test",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );
    do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("the gate runs")
        .output
}

fn verdict(r: &Resource) -> (String, Option<String>) {
    let ctor = r
        .get(&Iri::parse(wk::CTOR_NAME).unwrap())
        .and_then(Value::as_str)
        .expect("verdict has ctor_name")
        .to_string();
    let diag = r
        .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned);
    (ctor, diag)
}

#[test]
fn bridged_claim_over_a_derived_witness_holds() {
    let base = build_chain();
    let prop = parsed_prop("urn:eigenius:demo:msi-requires-helicase");
    let args = ["WRN".to_string(), "helicase".to_string()];

    let claim = grader(&args)
        .grade(
            &prop,
            &ClaimSource {
                stem: "urn:eigenius:doc:demo:s1",
                warrant: Warrant::Declared,
                declared_by: "encoding-pipeline",
                timestamp: "2026-08-03T00:00:00Z",
            },
        )
        .expect("the bridged cluster builds");
    assert_eq!(claim.resources.len(), 3, "bridge + trace + sentence");

    let mut resources = parser_output(&prop);
    resources.extend(claim.resources);
    let (ctor, diag) = verdict(&commit_and_validate(&base, resources, &claim.sentence_iri));
    assert_eq!(ctor, "Holds", "diagnostic: {diag:?}");
}

/// The demo's thesis. The recorded argument is byte-identical to the one that Held above; the only
/// change is that the claim on chain was re-derived from edited prose.
#[test]
fn the_same_argument_fails_once_the_claim_is_re_derived_from_edited_prose() {
    let base = build_chain();
    let original = parsed_prop("urn:eigenius:demo:msi-requires-helicase");
    let edited = parsed_prop("urn:eigenius:demo:msi-does-not-require-helicase");
    let args = ["WRN".to_string(), "helicase".to_string()];

    // Recorded when the prose still read the original way.
    let recorded = grader(&args)
        .grade(
            &original,
            &ClaimSource {
                stem: "urn:eigenius:doc:demo:s1",
                warrant: Warrant::Declared,
                declared_by: "encoding-pipeline",
                timestamp: "2026-08-03T00:00:00Z",
            },
        )
        .expect("the bridged cluster builds");

    // The chain now carries the EDITED parse — and only that one, as the demo's two branches ensure.
    let mut resources = parser_output(&edited);
    resources.extend(recorded.resources);
    let (ctor, diag) = verdict(&commit_and_validate(
        &base,
        resources,
        &recorded.sentence_iri,
    ));
    assert_eq!(
        ctor, "Fails",
        "an argument whose premise was edited out from under it must NOT commit"
    );
    assert!(
        diag.is_some(),
        "a Fails verdict must say which subterm failed, or the demo teaches nothing"
    );
}

/// Fail-closed the other way: without the parser's `ProgramTrace` there is no `IsDerivedAs` witness
/// at all, so the same certificate cannot stand on an unwitnessed claim.
#[test]
fn bridged_claim_needs_the_parser_program_trace() {
    let base = build_chain();
    let prop = parsed_prop("urn:eigenius:demo:msi-requires-helicase");
    let args = ["WRN".to_string(), "helicase".to_string()];

    let claim = grader(&args)
        .grade(
            &prop,
            &ClaimSource {
                stem: "urn:eigenius:doc:demo:s1",
                warrant: Warrant::Declared,
                declared_by: "encoding-pipeline",
                timestamp: "2026-08-03T00:00:00Z",
            },
        )
        .expect("the bridged cluster builds");

    // Keep the claim resource, drop its trace — the witness index stays empty for it.
    let mut resources: Vec<Resource> = parser_output(&prop).into_iter().take(1).collect();
    resources.extend(claim.resources);
    let (ctor, _) = verdict(&commit_and_validate(&base, resources, &claim.sentence_iri));
    assert_eq!(
        ctor, "Fails",
        "an unwitnessed claim must not carry a bridge"
    );
}

/// Guards the encoding invariant the certificate rests on: the bridge's antecedent and the
/// `derived(...)` grounding must embed the SAME D47 subtree, or the witness key cannot match even
/// when the propositions are equal. Cheap to break by refactoring, silent when broken.
#[test]
fn bridge_antecedent_and_derived_grounding_embed_the_same_subtree() {
    let prop = parsed_prop("urn:eigenius:demo:p");
    let args = ["a".to_string(), "b".to_string()];
    let claim = grader(&args)
        .grade(
            &prop,
            &ClaimSource {
                stem: "urn:eigenius:doc:demo:s1",
                warrant: Warrant::Declared,
                declared_by: "encoding-pipeline",
                timestamp: "2026-08-03T00:00:00Z",
            },
        )
        .unwrap();

    let bridge = &claim.resources[0];
    let Some(Value::Json(implication)) =
        bridge.get(&Iri::parse(wk::CANONICAL_PROPOSITION).unwrap())
    else {
        panic!("the bridge carries a JSON canonical_proposition")
    };
    let antecedent = &implication["args"][1];
    let Value::Json(encoded) = encode_type(&prop).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        antecedent, &encoded,
        "the bridge's antecedent must be the claim's own encoding, verbatim"
    );
    // And `Arrow` must encode as a Pi with an EMPTY binder name — the D47 convention the decoder
    // relies on to read the implication back.
    assert_eq!(implication["args"][0], serde_json::json!(""));
    let _ = Patt::Unit;
}
