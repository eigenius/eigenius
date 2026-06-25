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

//! D65 Slice 0 — `core:ValueIndex` build-time population + exact lookup.
//!
//! Mirrors the text-index population tests: declare a `core:ValueIndex`
//! Resource targeting a string property, add Resources carrying that property,
//! and assert `LayerBuilder::build` populated the per-layer exact value index so
//! `value_index.lookup(index, key)` returns the right subjects + defining
//! layers — including the normalizer applied at index time, and inheritance of
//! the index declaration from an ancestor layer.

use eigenius_kernel::bootstrap::bootstrap;
use eigenius_kernel::layer::{Layer, LayerBuilder};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use std::sync::Arc;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

fn make_resource(id: &str, class_iri: &str, props: Vec<(&str, Value)>) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(class_iri))]),
    );
    for (k, v) in props {
        r.set(iri(k), v);
    }
    r
}

/// Collect `(subject, layer)` hits for an exact `(index, key)` lookup.
fn lookup(layer: &Layer, index: &Iri, key: &str) -> Vec<(Iri, eigenius_kernel::layer::LayerId)> {
    let mut hits: Vec<_> = layer
        .storage()
        .value_index
        .lookup(index, key)
        .map(Result::unwrap)
        .collect();
    hits.sort();
    hits
}

/// A ValueIndex with a `lowercase` normalizer indexes the resources in the same
/// layer; lookup is exact on the normalized key.
#[test]
fn populates_for_resources_in_same_layer() {
    let ctx = bootstrap().unwrap();
    let head = Arc::clone(ctx.head());
    let storage = head.storage().clone();
    let mut b = LayerBuilder::new("test", Some(head));

    let form = "urn:eigenius:test:form";
    let index_iri = iri("urn:eigenius:test:form_index");

    b.add_resource(make_resource(
        index_iri.as_str(),
        wk::VALUE_INDEX_CLASS,
        vec![
            (wk::TARGET_PROPERTY, Value::ResourceRef(iri(form))),
            (
                wk::VALUE_NORMALIZER,
                Value::ResourceRef(iri("urn:eigenius:core:normalizers:lowercase")),
            ),
        ],
    ))
    .unwrap();

    // Two resources share the form "Cell Line" (case-insensitively); one differs.
    b.add_resource(make_resource(
        "urn:eigenius:test:e1",
        "urn:eigenius:test:Thing",
        vec![(form, Value::String("Cell Line".into()))],
    ))
    .unwrap();
    b.add_resource(make_resource(
        "urn:eigenius:test:e2",
        "urn:eigenius:test:Thing",
        vec![(form, Value::String("cell line".into()))],
    ))
    .unwrap();
    b.add_resource(make_resource(
        "urn:eigenius:test:e3",
        "urn:eigenius:test:Thing",
        vec![(form, Value::String("gene".into()))],
    ))
    .unwrap();

    let layer = Arc::new(b.build(storage));

    // The lowercase normalizer folds "Cell Line" and "cell line" to one key.
    let hits = lookup(&layer, &index_iri, "cell line");
    let mut expected = vec![
        (iri("urn:eigenius:test:e1"), layer.id().clone()),
        (iri("urn:eigenius:test:e2"), layer.id().clone()),
    ];
    expected.sort();
    assert_eq!(hits, expected);

    // The pre-normalized capitalized form is NOT a key (the caller must
    // normalize its lookup key the same way; this raw index lookup is exact).
    assert_eq!(lookup(&layer, &index_iri, "Cell Line").len(), 0);

    // A distinct key is independent.
    assert_eq!(
        lookup(&layer, &index_iri, "gene"),
        vec![(iri("urn:eigenius:test:e3"), layer.id().clone())]
    );

    // A miss is empty.
    assert_eq!(lookup(&layer, &index_iri, "absent").len(), 0);
}

/// String-array property values each contribute one entry under the same index.
#[test]
fn indexes_each_element_of_a_string_array() {
    let ctx = bootstrap().unwrap();
    let head = Arc::clone(ctx.head());
    let storage = head.storage().clone();
    let mut b = LayerBuilder::new("test", Some(head));

    let form = "urn:eigenius:test:alias";
    let index_iri = iri("urn:eigenius:test:alias_index");

    b.add_resource(make_resource(
        index_iri.as_str(),
        wk::VALUE_INDEX_CLASS,
        // No normalizer slot → identity (verbatim).
        vec![(wk::TARGET_PROPERTY, Value::ResourceRef(iri(form)))],
    ))
    .unwrap();

    b.add_resource(make_resource(
        "urn:eigenius:test:e1",
        "urn:eigenius:test:Thing",
        vec![(
            form,
            Value::Array(vec![
                Value::String("p53".into()),
                Value::String("TP53".into()),
            ]),
        )],
    ))
    .unwrap();

    let layer = Arc::new(b.build(storage));

    // Identity normalizer: both aliases are exact, case-sensitive keys.
    assert_eq!(
        lookup(&layer, &index_iri, "p53"),
        vec![(iri("urn:eigenius:test:e1"), layer.id().clone())]
    );
    assert_eq!(
        lookup(&layer, &index_iri, "TP53"),
        vec![(iri("urn:eigenius:test:e1"), layer.id().clone())]
    );
    assert_eq!(
        lookup(&layer, &index_iri, "tp53").len(),
        0,
        "identity is exact"
    );
}

/// The ValueIndex declared in a parent layer governs Resources committed in a
/// child layer; the child's lookup sees its own contributions keyed by the
/// inherited index IRI.
#[test]
fn populates_from_inherited_value_index() {
    let ctx = bootstrap().unwrap();
    let head = Arc::clone(ctx.head());
    let storage = head.storage().clone();

    let form = "urn:eigenius:test:form";
    let index_iri = iri("urn:eigenius:test:form_index");

    let mut parent_b = LayerBuilder::new("parent", Some(head));
    parent_b
        .add_resource(make_resource(
            index_iri.as_str(),
            wk::VALUE_INDEX_CLASS,
            vec![(wk::TARGET_PROPERTY, Value::ResourceRef(iri(form)))],
        ))
        .unwrap();
    let parent = Arc::new(parent_b.build(storage.clone()));

    let mut child_b = LayerBuilder::new("child", Some(Arc::clone(&parent)));
    child_b
        .add_resource(make_resource(
            "urn:eigenius:test:e1",
            "urn:eigenius:test:Thing",
            vec![(form, Value::String("gene".into()))],
        ))
        .unwrap();
    let child = Arc::new(child_b.build(storage));

    // The child's resource was indexed under the inherited index IRI, keyed in
    // the child layer.
    assert_eq!(
        lookup(&child, &index_iri, "gene"),
        vec![(iri("urn:eigenius:test:e1"), child.id().clone())]
    );
}
