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

//! `ingest` — the document → graded-claims path end to end (D63/D67).
//!
//! The "layer up": [`InProcessIngestion`] composes the DCG pipeline with the
//! [`ParsedClaimGrader`] (D73 §6 — parsed sentences land Declared) and proves the full algorithm
//! in one call — prose → parse → grade → committed claim whose `ProgramTrace` mints the
//! `IsDerivedAs` witness. This is the first-class form of what was an inline test-code harness.

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::dcg::{
    pretty_term, Identity, NoAbbreviationProposer, Proposal, ProposeCtx, Proposer, SentenceOutcome,
};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{layer_admits_witness, Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::witness::{WitnessCategory, WitnessKey};
use eigenius_reasoning::{DocumentIngestion, Grade, InProcessIngestion, ParsedClaimGrader};

/// A no-op anaphora proposer — the demo document has no pronouns, so the resolver never consults it.
struct NoProposer;
impl Proposer for NoProposer {
    fn propose(&self, _ctx: &ProposeCtx) -> Proposal {
        Proposal::default()
    }
}

/// Bootstrap (core → reflection → reasoning → closed-class) + the demo domain lexicon (Gene/CellLine,
/// `affects`, HeLa, the `Instability` mass noun). One base carries BOTH the lexicon (to parse) and the
/// reasoning ontology (to commit + validate claims).
fn demo_base() -> Arc<Layer> {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = include_str!("../../../experiments/lexicon/lexicon.esl");
    let resources =
        esl::compile_against_layer(demo, ctx.head()).expect("demo compiles on bootstrap");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add demo resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn outcome_kind(o: &SentenceOutcome) -> &'static str {
    match o {
        SentenceOutcome::Encoded(_) => "Encoded",
        SentenceOutcome::Ambiguous(_) => "Ambiguous",
        SentenceOutcome::Open(_) => "Open",
        SentenceOutcome::Gap => "Gap",
    }
}

#[test]
fn ingest_produces_a_declared_witnessed_claim() {
    // Prose → parse → grade → committed DERIVED cluster (D67 §1): the trust story is the
    // ProgramTrace minting `IsDerivedAs(claim, P)` on the chain — no ReasoningSentence, no gate
    // verdict. This replaces the pre-D67 Declared landing for parsed sentences
    // (`DeclaredClaimGrader` remains for curator-pinned rules; its cluster is covered by
    // `tests/grade.rs`).
    let base = demo_base();
    let grader = ParsedClaimGrader;
    let ingestion = InProcessIngestion::new(
        base,
        &Identity,
        &NoAbbreviationProposer,
        &NoProposer,
        &grader,
    );
    let doc = ingestion.ingest("demo", "instability affects HeLa.");

    let Some(s) = doc
        .sentences
        .iter()
        .find(|s| matches!(s.outcome, SentenceOutcome::Encoded(_)))
    else {
        let trace: Vec<String> = doc
            .sentences
            .iter()
            .map(|s| format!("{}={:?}", outcome_kind(&s.outcome), s.verdict))
            .collect();
        panic!("no sentence closed; per-sentence: {trace:?}");
    };
    let claim = s.claim.as_ref().expect("an Encoded sentence grades");
    assert!(
        matches!(claim.grade, Grade::Declared),
        "parsed sentences land DECLARED (D73 §6 / eigenius#201). The parser establishes that the \
         text parses to this well-typed term — not that the term is faithful to what the author \
         wrote (D61, unbuilt), nor that what the author wrote is true."
    );
    assert_eq!(
        claim.resources.len(),
        2,
        "the cluster is EncodedClaim + DeclarationTrace"
    );
    assert!(
        claim.gate_sentence.is_none(),
        "no ReasoningSentence — the declaring agent is the warrant, nothing for the D39 gate"
    );
    assert!(s.verdict.is_none(), "a parsed cluster gets no gate verdict");

    let SentenceOutcome::Encoded(item) = &s.outcome else {
        unreachable!()
    };
    // The graded claim carries the *real parsed proposition* — the closed kind-predication.
    let pretty = pretty_term(item.sem());
    assert!(
        pretty.contains("kind_of"),
        "the graded proposition is the parsed kind-predication sem: {pretty}"
    );
    // The trace MINTS the witness: `IsDeclaredAs(claim_iri, P)` is admitted on the committed
    // chain — what downstream `declared(claim_iri, P, _)` certificates resolve against.
    let key_for = |category| {
        WitnessKey::from_exp(category, claim.claim_iri.clone(), item.sem())
            .expect("the proposition hashes")
    };
    assert!(
        layer_admits_witness(&doc.layer, &key_for(WitnessCategory::Declared)),
        "the DeclarationTrace mints IsDeclaredAs(claim, P) into the chain witness index"
    );
    // The half that makes eigenius#201 a real change rather than a relabelling: a `derived(...)`
    // citation must no longer resolve. It read as "a program established P", which is the collapse
    // of D73 §6's three propositions into one witness.
    assert!(
        !layer_admits_witness(&doc.layer, &key_for(WitnessCategory::Derived)),
        "no IsDerivedAs — the parse is a formulation instrument, not a warrant"
    );
}
