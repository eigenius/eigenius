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

//! Regression: a layer with MORE resources than the bounded resource-cache budget
//! must still persist + resolve every resource. If the build→cache→`store_layer` path
//! relied on the cache *retaining* every resource, a bounded cache (D23 §5.3) would
//! silently drop the overflow — losing resources for any layer larger than the budget
//! (e.g. a domain-lexicon chunk). The cache is a read-through hint, not the staging
//! area; this pins that.

use std::sync::Arc;

use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;

#[test]
fn layer_larger_than_cache_budget_persists_every_resource() {
    let tmp = TempDir::new().unwrap();
    let store: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(tmp.path()).unwrap());

    // Tiny cache budget (100) but a much larger layer (500 resources): the cache
    // cannot hold them all, so eviction is forced during build/commit.
    const BUDGET: u64 = 100;
    const N: usize = 500;
    let storage = LayerStorage::with_persistent_bounded(Arc::clone(&store), BUDGET);

    let mut b = LayerBuilder::new("big", None);
    for i in 0..N {
        let mut r = Resource::new(Iri::parse(&format!("urn:eigenius:demo:r{i}")).unwrap());
        r.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse("urn:eigenius:core:Class").unwrap(),
            )]),
        );
        r.set(
            Iri::parse("urn:eigenius:core:description").unwrap(),
            Value::String(format!("resource {i}")),
        );
        b.add_resource(r).unwrap();
    }
    let layer = Arc::new(b.build(storage));
    store.store_layer(&layer).unwrap();

    // Every resource must resolve — including the earliest (most likely evicted) ones.
    // A miss means `store_layer` lost the resource because the cache evicted it before
    // it reached the backend.
    for i in [0usize, 1, N / 2, N - 1] {
        let iri = Iri::parse(&format!("urn:eigenius:demo:r{i}")).unwrap();
        assert!(
            layer.resolve(&iri).is_some(),
            "resource r{i} must persist + resolve even though the layer ({N}) exceeds the \
             cache budget ({BUDGET}); a miss means the bounded cache dropped it before \
             store_layer persisted it"
        );
    }
}
