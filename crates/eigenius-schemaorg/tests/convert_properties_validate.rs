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

//! The D61 typed property set for `convert.rs` is **Expressible**: it compiles
//! against core→reflection→reasoning→reference→objective and the kernel
//! `Validator` reports 0 errors. This witnesses two things at once:
//!
//! 1. the D61 §5 decision-layer ontology (the extension to
//!    `ontologies/objective/objective-ontology.esl`) actually holds the
//!    converter's real properties as **typed content** — `requires` / `allows_only`
//!    / `class_types` all satisfied; and
//! 2. the property set itself
//!    (`experiments/objectives/d57-schema-org/convert-properties.esl`) is
//!    well-formed.
//!
//! The first concrete output of the D57-redux dogfood
//! (`docs/design/d61-llm-based-encoding-methodology.md` §8). A compile failure here
//! would be the *Expressible* gate failing — a fail-closed finding that the §5
//! vocabulary cannot carry the content (the harvest doing its job).

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

/// Compile ESL against its parent (the *Expressible* check — a compile error means
/// the vocabulary cannot express the content) and build the layer.
fn esl_layer(source: &str, parent: &Arc<Layer>, name: &str) -> Arc<Layer> {
    let resources = esl::compile_against_layer(source, parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile (not Expressible):\n{}",
            errs.into_iter()
                .map(|e| format!("  - {e:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent.clone()));
    for r in &resources {
        b.add_resource(r.clone())
            .unwrap_or_else(|e| panic!("{name}: add_resource failed: {e:?}"));
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

#[test]
fn convert_property_set_is_expressible() {
    let core = json_layer(
        "core",
        None,
        &[include_str!("../../../ontologies/core/core-ontology.json")],
    );
    let reflection = json_layer(
        "reflection",
        Some(core),
        &[
            include_str!("../../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../../ontologies/institution/institution-ontology.json"),
            include_str!("../../../ontologies/ingest/ingest-ontology.json"),
        ],
    );
    let reasoning = {
        let mut b = LayerBuilder::new("reasoning", Some(reflection));
        for r in esl::compile(include_str!("../../../ontologies/reasoning/reasoning.esl"))
            .expect("reasoning.esl compiles")
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reference = esl_layer(
        include_str!("../../../ontologies/reference/reference.esl"),
        &reasoning,
        "reference",
    );
    let objective = esl_layer(
        include_str!("../../../ontologies/objective/objective-ontology.esl"),
        &reference,
        "objective",
    );
    let properties = esl_layer(
        include_str!("../../../experiments/objectives/d57-schema-org/convert-properties.esl"),
        &objective,
        "convert-properties",
    );

    let errors = Validator::new(properties).validate();
    assert!(
        errors.is_empty(),
        "the convert.rs typed property set must validate cleanly (Expressible). \
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
