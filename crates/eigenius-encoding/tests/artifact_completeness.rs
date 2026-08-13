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

//! **D67 §5 (3.4) artifact completeness** — the whole document lands, not only what encoded.
//! The artifact carries the Stage-A glossary resources that grounded the parse and one
//! `enc:CutItem` per non-encoded unit, and the result LOADS through the kernel (ESL printer →
//! `compile_against_layer` → validated layer build) on a chain that has `encoding.esl` and
//! nothing document-specific.
//!
//!   EIGENIUS_DB_SNAPSHOT=/path/to/wordnet-umls-aligned-2026-08-12-d67 \
//!     cargo test --release -p eigenius-encoding --test artifact_completeness -- --ignored --nocapture
//!
//! No parse runs here: the glossary is Stage A alone (deterministic, abbreviation extraction),
//! and the cuts are the four reasons enumerated. The parsed-claim half of the artifact is
//! covered by `acceptance.rs` and by `demo/prose-to-formulas/run.sh`.

use std::path::PathBuf;
use std::sync::Arc;

use eigenius_encoding::{emit_document, CutReason, CutSentence};
use eigenius_kernel::dcg::{augment_document_only, NoAbbreviationProposer};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::storage::PersistentBackend;
use eigenius_wordnet::lemmatizer::MorphyLemmatizer;

fn repo(rel: &str) -> String {
    format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))
}

/// Compile `src` against `head` and build the layer ON the persistent store (§7-2).
fn load_layer(
    backend: &Arc<dyn PersistentBackend>,
    head: &Arc<Layer>,
    name: &str,
    src: &str,
) -> Arc<Layer> {
    let resources = esl::compile_against_layer(src, head)
        .unwrap_or_else(|e| panic!("{name} compiles against the chain: {e:?}"));
    let mut b = LayerBuilder::new(name, Some(Arc::clone(head)));
    for r in resources {
        b.add_resource(r)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    }
    Arc::new(b.build(LayerStorage::with_persistent(Arc::clone(backend))))
}

#[test]
#[ignore = "DB-backed; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn glossary_and_cut_records_load_through_the_kernel() {
    let Some(snap) = std::env::var("EIGENIUS_DB_SNAPSHOT")
        .ok()
        .map(PathBuf::from)
    else {
        eprintln!("SKIP: EIGENIUS_DB_SNAPSHOT unset");
        return;
    };
    let (base, backend) = match eigenius_encoding::snapshot::open_head_and_backend(&snap) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("SKIP: {e}");
            return;
        }
    };
    let head = load_layer(
        &backend,
        &base,
        "encoding",
        &std::fs::read_to_string(repo("ontologies/encoding/encoding.esl")).expect("encoding.esl"),
    );

    // Stage A over the corpus page: the abbreviation entries that ground «MSI», «MMR», … .
    let document = std::fs::read_to_string(repo(
        "references/publications/WRN-Helicase-Nature-OCR/first-page-cleaned.txt",
    ))
    .expect("the corpus page");
    let lem = MorphyLemmatizer::load(&PathBuf::from(repo("references/WordNet-3.0/dict")))
        .expect("Morphy");
    let augmentation = augment_document_only(&head, &document, &NoAbbreviationProposer, &lem);
    let glossary = augmentation.resources();
    assert!(
        !glossary.is_empty(),
        "the page defines abbreviations — Stage A must produce entries"
    );

    // One cut per reason, so every `enc:CutKind` the emitter can name is exercised.
    let cuts = vec![
        CutSentence {
            ordinal: 1,
            text: "Thus, novel therapies are needed for tumours with MSI.".to_string(),
            span: (0, 54),
            reason: CutReason::Ambiguous { readings: 16 },
        },
        CutSentence {
            ordinal: 2,
            text: "These findings remained true with PCR-based MSI classifications.".to_string(),
            span: (55, 119),
            reason: CutReason::Unresolved { holes: 2 },
        },
        CutSentence {
            ordinal: 3,
            text: "Projects Achilles and DRIVE identified WRN.".to_string(),
            span: (120, 163),
            reason: CutReason::NoParse {
                oov: vec!["Achilles".to_string()],
            },
        },
        CutSentence {
            ordinal: 4,
            text: "More commonly, MSI cancers arise after somatic MMR inactivation.".to_string(),
            span: (164, 228),
            reason: CutReason::NoParse { oov: vec![] },
        },
    ];

    let json = emit_document(
        "urn:eigenius:test:artifact",
        "first-page-cleaned.txt",
        "0000",
        "2026-08-12T00:00:00Z",
        &glossary,
        &[],
        &cuts,
    )
    .expect("emits");
    // The committed artifact is ESL — print it back as source, then load THAT (the printer is
    // the inverse of the loader; a resource it cannot express is a real gap, found this way).
    let doc: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let source = eigenius_kernel::esl::print::print_document_with(
        &doc,
        eigenius_kernel::esl::print::Layout::Pretty,
    )
    .expect("the artifact prints as ESL");
    let layer = load_layer(&backend, &head, "artifact", &source);

    for n in 1..=4 {
        for local in ["unit", "cut"] {
            let id = format!("urn:eigenius:test:artifact:{local}_{n}");
            assert!(
                layer
                    .resolve(&eigenius_kernel::ontology::Iri::parse(&id).unwrap())
                    .is_some(),
                "{id} is on the loaded layer"
            );
        }
    }
    for r in &glossary {
        let id = r.id().expect("glossary resources are named");
        assert!(
            layer.resolve(id).is_some(),
            "glossary resource {id} is on the loaded layer"
        );
    }
    eprintln!(
        "artifact loaded: {} glossary + {} units + {} cut items",
        glossary.len(),
        cuts.len(),
        cuts.len()
    );
}
