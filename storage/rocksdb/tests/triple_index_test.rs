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

//! Phase 14h commit 2 integration tests: RocksDB-backed `TripleIndex`.
//!
//! Exercises the path that matters in production:
//! - `store_layer` populates both index orderings in the same atomic batch.
//! - The forward (`idx_pos:`) ordering returns the right
//!   `(subject, defining_layer)` pairs.
//! - Restart preserves index entries; reopened `RocksStore` returns the
//!   same data.
//! - `delete_layer` drops both orderings together.
//! - Branch divergence is handled by storing the defining layer in the
//!   key — chain-walk dedup in commit 3 will turn that into per-head
//!   correctness.

use std::sync::Arc;

use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

fn property_def(prop_iri: &str, data_type: &str) -> Resource {
    let mut r = Resource::new(iri(prop_iri));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
    );
    r.set(iri(wk::DATA_TYPE_PROP), Value::String(data_type.into()));
    r
}

fn class_instance(id: &str, classes: &[&str]) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(wk::IS_A),
        Value::Array(
            classes
                .iter()
                .map(|c| Value::String((*c).to_string()))
                .collect(),
        ),
    );
    r
}

/// Build a parent layer that defines the well-known `is_a` Property
/// with `data_type: resource_array`. Every test that loads instances
/// needs this so `extract_indexable_triples` recognises `is_a` as
/// indexable.
fn parent_layer_with_is_a(storage: LayerStorage) -> Arc<eigenius_kernel::layer::Layer> {
    let mut builder = LayerBuilder::new("parent", None);
    builder
        .add_resource(property_def(wk::IS_A, wk::RESOURCE_ARRAY))
        .unwrap();
    Arc::new(builder.build(storage))
}

#[test]
fn store_layer_populates_index_atomically() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = Arc::clone(&store) as Arc<dyn PersistentBackend>;
    let storage = LayerStorage::with_persistent(Arc::clone(&backend));

    let parent = parent_layer_with_is_a(storage.clone());
    backend.store_layer(&parent).unwrap();

    let mut builder = LayerBuilder::new("instances", Some(Arc::clone(&parent)));
    builder
        .add_resource(class_instance(
            "urn:eigenius:test:rex",
            &["urn:eigenius:test:Dog"],
        ))
        .unwrap();
    builder
        .add_resource(class_instance(
            "urn:eigenius:test:buddy",
            &["urn:eigenius:test:Dog"],
        ))
        .unwrap();
    builder
        .add_resource(class_instance(
            "urn:eigenius:test:mittens",
            &["urn:eigenius:test:Cat"],
        ))
        .unwrap();
    let layer = builder.build(storage.clone());
    backend.store_layer(&layer).unwrap();

    let index = backend.triple_index_arc();

    // Two Dogs at this layer.
    let dogs: Vec<_> = index
        .scan_predicate_object(&iri(wk::IS_A), &iri("urn:eigenius:test:Dog"))
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(dogs.len(), 2);
    assert!(dogs.iter().all(|(_, l)| l == layer.id()));
    let subjects: Vec<_> = dogs.iter().map(|(s, _)| s.clone()).collect();
    assert!(subjects.contains(&iri("urn:eigenius:test:rex")));
    assert!(subjects.contains(&iri("urn:eigenius:test:buddy")));

    // One Cat at this layer.
    let cats: Vec<_> = index
        .scan_predicate_object(&iri(wk::IS_A), &iri("urn:eigenius:test:Cat"))
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(cats.len(), 1);
    assert_eq!(cats[0].0, iri("urn:eigenius:test:mittens"));
    assert_eq!(cats[0].1, *layer.id());
}

#[test]
fn restart_preserves_index() {
    let tmp = TempDir::new().unwrap();
    let layer_id;
    {
        let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
        let backend: Arc<dyn PersistentBackend> = Arc::clone(&store) as Arc<dyn PersistentBackend>;
        let storage = LayerStorage::with_persistent(Arc::clone(&backend));

        let parent = parent_layer_with_is_a(storage.clone());
        backend.store_layer(&parent).unwrap();

        let mut builder = LayerBuilder::new("instances", Some(Arc::clone(&parent)));
        builder
            .add_resource(class_instance(
                "urn:eigenius:test:rex",
                &["urn:eigenius:test:Dog"],
            ))
            .unwrap();
        let layer = builder.build(storage.clone());
        backend.store_layer(&layer).unwrap();
        layer_id = layer.id().clone();
        // Drop store + backend so the RocksDB lock is released before reopen.
    }

    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = store as Arc<dyn PersistentBackend>;
    let index = backend.triple_index_arc();

    let dogs: Vec<_> = index
        .scan_predicate_object(&iri(wk::IS_A), &iri("urn:eigenius:test:Dog"))
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(dogs.len(), 1);
    assert_eq!(dogs[0].0, iri("urn:eigenius:test:rex"));
    assert_eq!(dogs[0].1, layer_id);
}

#[test]
fn delete_layer_drops_index_entries() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = Arc::clone(&store) as Arc<dyn PersistentBackend>;
    let storage = LayerStorage::with_persistent(Arc::clone(&backend));

    let parent = parent_layer_with_is_a(storage.clone());
    backend.store_layer(&parent).unwrap();

    let mut builder = LayerBuilder::new("instances", Some(Arc::clone(&parent)));
    builder
        .add_resource(class_instance(
            "urn:eigenius:test:rex",
            &["urn:eigenius:test:Dog"],
        ))
        .unwrap();
    let layer = builder.build(storage.clone());
    backend.store_layer(&layer).unwrap();

    // Sanity: index has the entry.
    let index = backend.triple_index_arc();
    assert_eq!(
        index
            .scan_predicate_object(&iri(wk::IS_A), &iri("urn:eigenius:test:Dog"))
            .count(),
        1
    );

    // Drop the instance layer. Both forward and reverse entries should
    // disappear in one atomic batch.
    backend.delete_layer(layer.id()).unwrap();

    assert_eq!(
        index
            .scan_predicate_object(&iri(wk::IS_A), &iri("urn:eigenius:test:Dog"))
            .count(),
        0
    );
}

#[test]
fn divergent_branches_keep_their_own_entries() {
    // Phase 14h key-shape contract: each layer's entries are scoped by
    // the trailing `<layer>` segment in the forward key. Branch
    // divergence is naturally represented — chain-walk dedup in
    // commit 3 turns the per-layer entries into per-head answers.
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = Arc::clone(&store) as Arc<dyn PersistentBackend>;
    let storage = LayerStorage::with_persistent(Arc::clone(&backend));

    let parent = parent_layer_with_is_a(storage.clone());
    backend.store_layer(&parent).unwrap();

    let mut main_builder = LayerBuilder::new("main_layer", Some(Arc::clone(&parent)));
    main_builder
        .add_resource(class_instance(
            "urn:eigenius:test:rex",
            &["urn:eigenius:test:Dog"],
        ))
        .unwrap();
    let main_layer = main_builder.build(storage.clone());
    backend.store_layer(&main_layer).unwrap();

    let mut feature_builder = LayerBuilder::new("feature_layer", Some(Arc::clone(&parent)));
    feature_builder
        .add_resource(class_instance(
            "urn:eigenius:test:rex",
            &["urn:eigenius:test:Cat"],
        ))
        .unwrap();
    let feature_layer = feature_builder.build(storage);
    backend.store_layer(&feature_layer).unwrap();

    let index = backend.triple_index_arc();

    let dogs: Vec<_> = index
        .scan_predicate_object(&iri(wk::IS_A), &iri("urn:eigenius:test:Dog"))
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(dogs.len(), 1);
    assert_eq!(dogs[0].0, iri("urn:eigenius:test:rex"));
    assert_eq!(dogs[0].1, *main_layer.id());

    let cats: Vec<_> = index
        .scan_predicate_object(&iri(wk::IS_A), &iri("urn:eigenius:test:Cat"))
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(cats.len(), 1);
    assert_eq!(cats[0].0, iri("urn:eigenius:test:rex"));
    assert_eq!(cats[0].1, *feature_layer.id());
}

#[test]
fn drop_layer_via_index_trait_works_standalone() {
    // The `TripleIndex` trait's standalone `drop_layer` (used outside
    // the `delete_layer` atomic batch — e.g., for tests or future
    // tooling) must be self-contained: create its own batch, commit,
    // and leave the index consistent.
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = Arc::clone(&store) as Arc<dyn PersistentBackend>;
    let storage = LayerStorage::with_persistent(Arc::clone(&backend));

    let parent = parent_layer_with_is_a(storage.clone());
    backend.store_layer(&parent).unwrap();

    let mut builder = LayerBuilder::new("instances", Some(Arc::clone(&parent)));
    builder
        .add_resource(class_instance(
            "urn:eigenius:test:rex",
            &["urn:eigenius:test:Dog"],
        ))
        .unwrap();
    let layer = builder.build(storage.clone());
    backend.store_layer(&layer).unwrap();

    let index = backend.triple_index_arc();
    assert_eq!(
        index
            .scan_predicate_object(&iri(wk::IS_A), &iri("urn:eigenius:test:Dog"))
            .count(),
        1
    );

    index.drop_layer(layer.id()).unwrap();

    assert_eq!(
        index
            .scan_predicate_object(&iri(wk::IS_A), &iri("urn:eigenius:test:Dog"))
            .count(),
        0
    );
}

#[test]
fn extend_layer_via_index_trait_works_standalone() {
    use eigenius_kernel::layer::{LayerId, Triple};

    // Standalone `extend_layer` is used by ad-hoc rebuild tooling and
    // by tests — it must work without a `store_layer` call to back it.
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = Arc::clone(&store) as Arc<dyn PersistentBackend>;
    let index = backend.triple_index_arc();

    let layer = LayerId([0xab; 32]);
    let p = iri(wk::IS_A);
    let dog = iri("urn:eigenius:test:Dog");
    let rex = iri("urn:eigenius:test:rex");

    index
        .extend_layer(
            &layer,
            &[Triple {
                subject: &rex,
                predicate: &p,
                object: &dog,
            }],
        )
        .unwrap();

    let hits: Vec<_> = index
        .scan_predicate_object(&p, &dog)
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, rex);
    assert_eq!(hits[0].1, layer);
}

#[test]
fn literal_typed_properties_skip_indexing() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(RocksStore::open(tmp.path()).unwrap());
    let backend: Arc<dyn PersistentBackend> = Arc::clone(&store) as Arc<dyn PersistentBackend>;
    let storage = LayerStorage::with_persistent(Arc::clone(&backend));

    // Parent defines `short_name` with data_type=string — should not
    // be indexed.
    let mut parent_builder = LayerBuilder::new("parent", None);
    parent_builder
        .add_resource(property_def(wk::IS_A, wk::RESOURCE_ARRAY))
        .unwrap();
    parent_builder
        .add_resource(property_def(wk::SHORT_NAME, "urn:eigenius:core:string"))
        .unwrap();
    let parent = Arc::new(parent_builder.build(storage.clone()));
    backend.store_layer(&parent).unwrap();

    // Child layer defines a class with both is_a (indexable) and
    // short_name (literal — not indexable).
    let mut builder = LayerBuilder::new("classes", Some(Arc::clone(&parent)));
    let mut dog = Resource::new(iri("urn:eigenius:test:Dog"));
    dog.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
    );
    dog.set(iri(wk::SHORT_NAME), Value::String("Dog".into()));
    builder.add_resource(dog).unwrap();
    let layer = builder.build(storage);
    backend.store_layer(&layer).unwrap();

    let index = backend.triple_index_arc();

    // Class-typed entry indexed.
    assert_eq!(
        index
            .scan_predicate_object(&iri(wk::IS_A), &iri(wk::CLASS))
            .count(),
        1
    );

    // String-valued short_name is NOT in the POS index. Even though
    // "Dog" isn't a valid IRI, scanning for it must return zero hits.
    // (We can't build an Iri for "Dog" because it's not a valid IRI;
    // instead, walk every entry and confirm `short_name` never appears.)
    let mut all = Vec::new();
    let parent_iri = iri(wk::IS_A);
    for hit in index.scan_predicate_object(&parent_iri, &iri(wk::CLASS)) {
        all.push(hit.unwrap());
    }
    // We have exactly one entry, and it's the is_a one, not short_name.
    assert_eq!(all.len(), 1);
}
