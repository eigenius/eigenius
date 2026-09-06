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

//! PROBE (eigenius#199): can `justification:Certificate(P)` be written as a type in ESL?

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;

fn chain() -> Arc<eigenius_kernel::layer::Layer> {
    let mut core = LayerBuilder::new("core", None);
    for r in eigon_json::parse_document(include_str!("../../ontologies/core/core-ontology.json"))
        .unwrap()
    {
        core.add_resource(r).unwrap();
    }
    let core = Arc::new(core.build(LayerStorage::in_memory()));

    let mut refl = LayerBuilder::new("reflection", Some(core));
    for src in [
        include_str!("../../ontologies/reflection/reflection-ontology.json"),
        include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
        include_str!("../../ontologies/institution/institution-ontology.json"),
    ] {
        for r in eigon_json::parse_document(src).unwrap() {
            refl.add_resource(r).unwrap();
        }
    }
    let refl_layer = Arc::new(refl.build(LayerStorage::in_memory()));

    // `prov` — the provenance axis, above reflection.
    let mut prov = LayerBuilder::new("prov", Some(Arc::clone(&refl_layer)));
    for r in esl::compile(include_str!("../../ontologies/prov/prov.esl"), &refl_layer).unwrap() {
        prov.add_resource(r).unwrap();
    }
    let refl = Arc::new(prov.build(LayerStorage::in_memory()));

    let mut rsn = LayerBuilder::new("reasoning", Some(Arc::clone(&refl)));
    for r in esl::compile(
        include_str!("../../ontologies/justification/justification.esl"),
        &refl,
    )
    .unwrap()
    {
        rsn.add_resource(r).unwrap();
    }
    Arc::new(rsn.build(LayerStorage::in_memory()))
}

#[test]
fn justifiedby_can_be_written_as_a_type() {
    let base = chain();
    // The literal definition of done for eigenius#199: write
    // `justification:Certificate(P)` as a TYPE at the ESL surface. An `axiom`
    // statement is the right slot — it holds a type, not a proposition,
    // so this exercises the index telescope without the `Prop`
    // obligation Rule 21 puts on `canonical_proposition`.
    //
    // Index #0's declared kind is `justification:Term`. Before
    // the fix it decoded to `EigonClass(justification:Term)` while the
    // supplied argument `Declared(...)` infers to
    // `InductiveType(justification:Term, [])`, so this failed with
    // `InductiveType(…) ≠ EigonClass(…)`.
    let src = r#"
        namespace core       = "urn:eigenius:core";
        namespace justification = "urn:eigenius:justification";
        namespace probe      = "urn:eigenius:probe";

        data probe:P : Prop { }

        axiom probe:cert : justification:Certificate(probe:P)
    "#;
    let mut b = LayerBuilder::new("probe", Some(Arc::clone(&base)));
    for r in esl::compile(src, &base).expect("probe ESL compiles") {
        b.add_resource(r).unwrap();
    }
    let probe = Arc::new(b.build(LayerStorage::in_memory()));
    let errs = eigenius_kernel::validation::Validator::new(probe).validate();
    assert!(
        errs.is_empty(),
        "justification:Certificate type rejected: {errs:#?}"
    );
}

#[test]
fn justifiedby_index_rejects_a_non_proposition() {
    // The other half of eigenius#199: the index must be ENFORCED, not merely swapped for a
    // permissive form. It was index #0, which had to be a `justification:Term` and refused a
    // `core:string`; the D88 §2 merge left one index, the proposition, and a string is refused
    // there for the same reason — the kind is checked rather than assumed.
    let base = chain();
    let src = r#"
        namespace core      = "urn:eigenius:core";
        namespace justification = "urn:eigenius:justification";
        namespace probe     = "urn:eigenius:probe";

        data probe:P : Prop { }

        axiom probe:bad : justification:Certificate("not-a-proposition")
    "#;
    let mut b = LayerBuilder::new("probe", Some(Arc::clone(&base)));
    for r in esl::compile(src, &base).expect("probe ESL compiles") {
        b.add_resource(r).unwrap();
    }
    let probe = Arc::new(b.build(LayerStorage::in_memory()));
    let errs = eigenius_kernel::validation::Validator::new(probe).validate();
    assert!(
        !errs.is_empty(),
        "a string in justification:Certificate's justification:Term index was accepted"
    );
}
