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

//! **The served path emits the CLI's artifact, byte for byte** (D71 slice 5b).
//!
//! Two drivers, one contract. If they diverge, the divergence is silent: the demo compares the
//! CLI's artifact against a committed fixture, and nothing compares the served one against
//! anything. So this test drives `EncodingFormalizer` — the same impl the kernel server calls —
//! over the demo's paragraph and its four committed draws, and byte-compares the result with
//! `demo/prose-to-formulas-v2/claims-intact.esl`.
//!
//! It exercises the whole 5b stack below the RPC: the arm construction from `DrawSource::Inline`,
//! the scope threading, the `ModelConfig` (unused here — every seam replays), the shared emission
//! core from 5a, and the `ArtifactFormat::Esl` rendering. The gRPC layer above it is a thin
//! translation from proto fields to these same values.
//!
//! ```bash
//! EIGENIUS_DB_SNAPSHOT=/path/to/wordnet-umls-aligned-2026-08-15-d70b \
//!   cargo test --release -p eigenius-encoding --test served_formalization -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use eigenius_encoding::formalize::EncodingFormalizer;
use eigenius_encoding::snapshot::open_head_and_backend;
use eigenius_kernel::dcg::draw::DrawSeam;
use eigenius_kernel::dcg::formalizer::{
    ArtifactFormat, DocumentFormalizer, DrawSource, FormalizeRequest,
};
use eigenius_kernel::dcg::Lemmatizer;

fn repo(rel: &str) -> String {
    format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

#[test]
#[ignore = "DB-backed; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn the_served_run_emits_the_same_artifact_as_the_cli() {
    let Ok(snapshot) = std::env::var("EIGENIUS_DB_SNAPSHOT") else {
        panic!("set EIGENIUS_DB_SNAPSHOT to a lexicon snapshot");
    };
    let (head, backend) =
        open_head_and_backend(std::path::Path::new(&snapshot)).expect("open snapshot");

    // The claim-kind vocabulary is not in the lexicon snapshot, and the demo chain-loads it before
    // parsing: a demonstrative's restrictor is checked against a claim's KIND class.
    let mut head = head;
    for rel in [
        "ontologies/encoding/encoding.esl",
        "ontologies/encoding/claim-kind-alignment.esl",
    ] {
        let src = read(rel);
        let resources = eigenius_kernel::esl::compile_against_layer(&src, &head)
            .unwrap_or_else(|e| panic!("{rel} does not compile: {e:?}"));
        let mut b = eigenius_kernel::layer::LayerBuilder::new(rel, Some(Arc::clone(&head)));
        for r in resources {
            b.add_resource(r).expect("add");
        }
        let layer = Arc::new(
            b.build(eigenius_kernel::layer::LayerStorage::with_persistent(
                Arc::clone(&backend),
            )),
        );
        use eigenius_kernel::commit::LayerPersister;
        eigenius_kernel::commit::BackendPersister::new(Some(Arc::clone(&backend)))
            .persist("main", &layer)
            .unwrap_or_else(|e| panic!("persist {rel}: {e:?}"));
        head = layer;
    }

    let lemmatizer: Arc<dyn Lemmatizer + Send + Sync> = Arc::new(
        eigenius_wordnet::lemmatizer::MorphyLemmatizer::load(std::path::Path::new(&repo(
            "references/WordNet-3.0/dict",
        )))
        .expect("load Morphy"),
    );

    // The demo's four committed draws, supplied inline — the same recordings the CLI replays.
    let mut inline = BTreeMap::new();
    for (seam, rel) in [
        (DrawSeam::SenseRank, "demo/prose-to-formulas-v2/ranks.json"),
        (
            DrawSeam::ReadingSelection,
            "demo/prose-to-formulas-v2/selections.json",
        ),
        (
            DrawSeam::Anaphora,
            "demo/prose-to-formulas-v2/proposals.json",
        ),
        (
            DrawSeam::DiscourseKind,
            "demo/prose-to-formulas-v2/kinds.json",
        ),
    ] {
        inline.insert(seam, read(rel));
    }

    let req = FormalizeRequest {
        source_text: read("demo/prose-to-formulas-v2/paragraph.txt"),
        // The demo passes this path repo-relative so the artifact is machine-independent; matching
        // it is what makes the byte comparison meaningful rather than accidental.
        source_path: "demo/prose-to-formulas-v2/paragraph.txt".to_string(),
        source_ref: None,
        doc_id: "served-e2e".to_string(),
        ns: "urn:eigenius:demo:v2".to_string(),
        timestamp: "2026-08-03T00:00:00Z".to_string(),
        scope: None,
        model: Default::default(),
        // The demo's caps, which the committed draws were recorded under.
        sense_cap: Some(2),
        cell_beam: Some(64),
        strict: false,
        draws: DrawSource::Inline(inline),
        format: ArtifactFormat::Esl,
    };

    let out = EncodingFormalizer::new(lemmatizer)
        .formalize(head, backend, &req)
        .expect("the served run completes");

    assert_eq!(out.encoded, 3, "three claims, as the demo reports");
    assert_eq!(out.cut, 0);
    assert_eq!(out.structure_iri, "urn:eigenius:demo:v2:structure");
    assert_eq!(out.content_type, ArtifactFormat::Esl);
    assert_eq!(
        out.draws_committed, 0,
        "every seam REPLAYED, so there is no new transcript to commit — a replayed run that \
         re-recorded would churn the branch for nothing"
    );

    let served = String::from_utf8(out.artifact).expect("ESL is utf-8");
    let cli = read("demo/prose-to-formulas-v2/claims-intact.esl");
    assert_eq!(
        served, cli,
        "the served artifact differs from the CLI's committed one — two drivers, one contract, \
         and a divergence here is silent because nothing else compares them"
    );
}
