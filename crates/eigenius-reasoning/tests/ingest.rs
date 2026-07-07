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

//! `ingest` — the document → graded-claims path end to end (D63).
//!
//! The "layer up": [`InProcessIngestion`] composes the DCG pipeline with the [`DeclaredClaimGrader`] and
//! proves the full algorithm in one call — prose → parse → grade → committed, `Holds`-validated claim.
//! This is the first-class form of what was an inline test-code harness.

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::dcg::{
    pretty_term, Identity, NoAbbreviationProposer, ProposeCtx, Proposer, SentenceOutcome,
};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::Iri;
use eigenius_reasoning::{
    ClaimVerdict, DeclaredClaimGrader, DocumentIngestion, Grade, InProcessIngestion,
    IngestedSentence,
};

/// A no-op anaphora proposer — the demo document has no pronouns, so the resolver never consults it.
struct NoProposer;
impl Proposer for NoProposer {
    fn propose(&self, _ctx: &ProposeCtx) -> Vec<Iri> {
        Vec::new()
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
fn ingest_produces_a_validated_graded_claim() {
    // Prose → parse → grade → committed, Holds-validated claim, in one `ingest()`.
    let base = demo_base();
    let grader = DeclaredClaimGrader;
    let ingestion = InProcessIngestion::new(
        base,
        &Identity,
        &NoAbbreviationProposer,
        &NoProposer,
        &grader,
    );
    let doc = ingestion.ingest("demo", "instability affects HeLa.");

    let holds: Vec<&IngestedSentence> = doc.encoded_holds().collect();
    if holds.is_empty() {
        let trace: Vec<String> = doc
            .sentences
            .iter()
            .map(|s| format!("{}={:?}", outcome_kind(&s.outcome), s.verdict))
            .collect();
        panic!("no sentence closed and validated Holds; per-sentence: {trace:?}");
    }

    let s = holds[0];
    // The claim commits at the honest floor — Declared.
    assert!(
        matches!(s.claim.as_ref().map(|c| c.grade), Some(Grade::Declared)),
        "the graded claim commits at Declared"
    );
    // …and it is a 3-resource cluster (declaring + trace + sentence).
    assert_eq!(
        s.claim.as_ref().expect("holds ⇒ claim").resources.len(),
        3,
        "the cluster is declaring + trace + sentence"
    );
    // Witness: the graded claim carries the *real parsed proposition* — the closed kind-predication,
    // not an empty or placeholder term.
    if let SentenceOutcome::Encoded(item) = &s.outcome {
        let pretty = pretty_term(item.sem());
        assert!(
            pretty.contains("kind_of"),
            "the graded proposition is the parsed kind-predication sem: {pretty}"
        );
    }
    // Verdict is specifically Holds (redundant with encoded_holds, but explicit).
    assert!(matches!(s.verdict, Some(ClaimVerdict::Holds)));
}
