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

//! D62 §8 — the drafted `lexicon` layer is **Expressible**: it compiles
//! against core→reflection(+eigentt) and the kernel `Validator` reports 0
//! errors. This witnesses the D62 thesis at the smallest scale — the four
//! categorial archetypes (common noun → `EigonClass`, named entity →
//! `ResourceRef`, transitive verb / adjective → `EigonAxiom`) each map onto
//! an existing kernel constructor, so the lexicon's semantic content is
//! native EigenTT, kernel-checked, with no new term language.
//!
//! A compile/validate failure here is the *Expressible* gate failing — a
//! fail-closed finding that the kernel cannot carry the lexicon content.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::validation::Validator;

fn json_layer(name: &str, parent: Option<Arc<Layer>>, sources: &[&str]) -> Arc<Layer> {
    let mut b = LayerBuilder::new(name, parent);
    for src in sources {
        for r in eigon_json::parse_document(src).expect("ontology parses") {
            b.add_resource(r).expect("ontology resource adds");
        }
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

#[test]
fn lexicon_layer_is_expressible() {
    let core = json_layer(
        "core",
        None,
        &[include_str!("../../ontologies/core/core-ontology.json")],
    );
    let reflection = json_layer(
        "reflection",
        Some(core),
        &[
            include_str!("../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../ontologies/institution/institution-ontology.json"),
            include_str!("../../ontologies/ingest/ingest-ontology.json"),
        ],
    );

    let lexicon_src = include_str!("../../experiments/lexicon/lexicon.esl");
    let resources = esl::compile_against_layer(lexicon_src, &reflection).unwrap_or_else(|errs| {
        panic!(
            "lexicon.esl failed to compile (not Expressible):\n{}",
            errs.into_iter()
                .map(|e| format!("  - {e:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut b = LayerBuilder::new("lexicon", Some(reflection));
    for r in &resources {
        b.add_resource(r.clone())
            .unwrap_or_else(|e| panic!("lexicon: add_resource failed: {e:?}"));
    }
    let lexicon = Arc::new(b.build(LayerStorage::in_memory()));

    let errors = Validator::new(lexicon).validate();
    assert!(
        errors.is_empty(),
        "the drafted lexicon layer must validate cleanly (Expressible). \
         {} error(s):\n{}",
        errors.len(),
        errors
            .iter()
            .take(25)
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
