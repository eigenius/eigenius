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

//! `grade` — the parser → reasoning-layer bridge (D63 reshape §6 Phase C).
//!
//! Proves the load-bearing claim: [`DeclaredClaimGrader`] turns a closed proposition into a 3-resource
//! cluster that the D39 [`do_validate_justification`] gate admits (`Verdict::Holds`) — i.e. a parsed
//! `Prop` becomes a committed, kernel-checked Declared claim. Plus a fail-closed witness that the
//! declaration trace is necessary (drop it → the certificate has no admitted witness → `Verdict::Fails`).

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::term::{Exp, InductiveDecl};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::{
    ClaimGrader, ClaimSource, DeclaredClaimGrader, Grade, ReasoningInstitution, Warrant,
};

/// Stand up the layer chain the validate handler dispatches over: core → reflection (+ eigentt +
/// institution fragments) → reasoning. Mirrors the `validate_handler.rs::build_full_chain` helper.
fn build_full_chain() -> Arc<Layer> {
    let core_json = include_str!("../../../ontologies/core/core-ontology.json");
    let core_resources = eigon_json::parse_document(core_json).unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let reflection_json = include_str!("../../../ontologies/reflection/reflection-ontology.json");
    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for r in eigon_json::parse_document(reflection_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let eigentt_json = include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json");
    for r in eigon_json::parse_document(eigentt_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let institution_json =
        include_str!("../../../ontologies/institution/institution-ontology.json");
    for r in eigon_json::parse_document(institution_json).unwrap() {
        reflection_builder.add_resource(r).unwrap();
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    let reasoning_source = include_str!("../../../ontologies/reasoning/reasoning.esl");
    let reasoning_resources = esl::compile(reasoning_source).expect("reasoning.esl compiles");
    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(reflection));
    for r in reasoning_resources {
        reasoning_builder.add_resource(r).unwrap();
    }
    Arc::new(reasoning_builder.build(LayerStorage::in_memory()))
}

/// A demo content proposition `Asserts(content_iri)` as an `Exp` — stands in for a parser's
/// `item.sem()`. `content_iri` is deliberately distinct from the declaring resource's IRI, so the test
/// exercises a real content claim, not the self-referential `Asserts(self)` default.
fn asserts_prop(content_iri: &str) -> Exp {
    let asserts_iri = Iri::parse("urn:eigenius:core:Asserts").expect("static Asserts IRI");
    let decl = Arc::new(InductiveDecl {
        iri: asserts_iri.clone(),
        name: asserts_iri.local_name().to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::Sort(0),
        ctors: Vec::new(),
    });
    Exp::InductiveType(decl, vec![Exp::LitString(content_iri.to_string())])
}

/// Commit `resources` onto the reasoning chain and return a read-only context over the result, with the
/// chain witness index built (as a real commit would leave it).
fn commit_over(base: &Arc<Layer>, resources: Vec<Resource>) -> ExecutionContext {
    let mut builder = LayerBuilder::new("doc-claims", Some(Arc::clone(base)));
    for r in resources {
        builder.add_resource(r).unwrap();
    }
    let layer = Arc::new(builder.build(LayerStorage::in_memory()));
    ExecutionContext::new(
        layer,
        "grade-test",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

fn verdict_ctor(r: &Resource) -> String {
    r.get(&Iri::parse(wk::CTOR_NAME).unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned)
        .expect("verdict resource has ctor_name")
}

fn verdict_diagnostic(r: &Resource) -> Option<String> {
    r.get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[test]
fn declared_grader_produces_a_commit_passing_claim() {
    // A parsed proposition → the Declared claim cluster → the D39 gate returns Holds.
    let base = build_full_chain();
    let prop = asserts_prop("urn:eigenius:demo:msi-contributes-to-cancer");

    let claim = DeclaredClaimGrader
        .grade(
            &prop,
            &ClaimSource {
                stem: "urn:eigenius:doc:demo:s0",
                warrant: Warrant::Declared,
                declared_by: "encoding-pipeline",
                timestamp: "2026-08-03T00:00:00Z",
            },
        )
        .expect("grade builds the cluster");

    // The cluster shape: declaring resource + declaration trace + reasoning sentence.
    assert_eq!(claim.grade, Grade::Declared);
    assert_eq!(claim.resources.len(), 3, "declaring + trace + sentence");
    let sentence = claim
        .resources
        .iter()
        .find(|r| r.id() == Some(&claim.sentence_iri))
        .expect("the sentence is in the cluster")
        .clone();

    // Commit the declaring resource + trace (they seed the witness the certificate needs), then
    // validate the sentence against that chain.
    let declaring_and_trace: Vec<Resource> = claim
        .resources
        .iter()
        .filter(|r| r.id() != Some(&claim.sentence_iri))
        .cloned()
        .collect();
    let ctx = commit_over(&base, declaring_and_trace);

    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns an outcome");
    assert_eq!(
        verdict_ctor(&outcome.output),
        wk::VERDICT_HOLDS,
        "the Declared claim cluster should validate Holds; diagnostic: {:?}",
        verdict_diagnostic(&outcome.output)
    );
}

#[test]
fn declared_claim_needs_its_declaration_trace() {
    // Fail-closed: without the DeclarationTrace committed, no `IsDeclaredAs` witness is admitted, so the
    // certificate cannot type-check — the gate must Fail, not silently pass.
    let base = build_full_chain();
    let prop = asserts_prop("urn:eigenius:demo:msi-contributes-to-cancer");

    let claim = DeclaredClaimGrader
        .grade(
            &prop,
            &ClaimSource {
                stem: "urn:eigenius:doc:demo:s0",
                warrant: Warrant::Declared,
                declared_by: "encoding-pipeline",
                timestamp: "2026-08-03T00:00:00Z",
            },
        )
        .expect("grade builds the cluster");
    let sentence = claim
        .resources
        .iter()
        .find(|r| r.id() == Some(&claim.sentence_iri))
        .expect("the sentence is in the cluster")
        .clone();

    // Commit ONLY the declaring resource — omit the trace.
    let declaring_only: Vec<Resource> = claim
        .resources
        .iter()
        .filter(|r| {
            r.id() != Some(&claim.sentence_iri)
                && !r
                    .id()
                    .is_some_and(|i| i.as_str().ends_with(":assertion_trace"))
        })
        .cloned()
        .collect();
    assert_eq!(declaring_only.len(), 1, "only the declaring resource");
    let ctx = commit_over(&base, declaring_only);

    let outcome = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("handler returns an outcome");
    assert_eq!(
        verdict_ctor(&outcome.output),
        wk::VERDICT_FAILS,
        "a claim whose declaration trace is missing must Fail, not pass"
    );
}
