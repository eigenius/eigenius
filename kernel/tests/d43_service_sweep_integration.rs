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

//! D43 — service-level wiring contract: when `with_embedders` is
//! called, the `EigeniusService`'s `CommitHookHost::trigger_vector_sweep_for_layer`
//! impl spawns a sweep through the `SweepCoordinator` and a vector
//! segment lands in the layer's storage. Without `with_embedders`,
//! the hook is a no-op.
//!
//! These tests pin the structural contract that connects the commit
//! pipeline's `didPersist` slot to the production sweep path. They do
//! not exercise the gRPC layer — the hook is the integration seam.

use eigenius_kernel::bootstrap::bootstrap;
use eigenius_kernel::commit::CommitHookHost;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::embedder::{DummyEmbedder, EmbedderRegistry};
use eigenius_kernel::server::EigeniusService;
use std::sync::Arc;
use std::time::Duration;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// Build a child layer on top of a freshly-bootstrapped chain that
/// declares a `core:VectorIndex` Resource (pointing at a string
/// Property) and three Document Resources whose `body` carries text
/// to be embedded. The test uses its own bootstrap rather than
/// reaching into the service's internal context cache so the layer
/// (and its storage) stays in scope for the post-sweep assertion.
fn build_corpus_layer(model_iri: &str) -> Arc<Layer> {
    let ctx = bootstrap().expect("bootstrap");
    let parent = Arc::clone(ctx.head());

    let body_iri = "urn:eigenius:test:body";
    let mut b = LayerBuilder::new("d43-sweep-test", Some(parent));

    // String Property `body`.
    let mut prop = Resource::new(iri(body_iri));
    prop.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
    );
    prop.set(iri(wk::SHORT_NAME), Value::String("body".into()));
    prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
    b.add_resource(prop).unwrap();

    // VectorIndex Resource pointing at it.
    let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
    vi.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
    );
    vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
    vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
    vi.set(iri(wk::VEC_DIM), Value::Integer(8));
    b.add_resource(vi).unwrap();

    for i in 0..3 {
        let mut d = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
        d.set(iri(body_iri), Value::String(format!("doc {i} body")));
        b.add_resource(d).unwrap();
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// Poll `predicate` every `interval` until it returns `Some(t)` or
/// `deadline` expires. Used in place of an arbitrary `sleep` so the
/// test fails fast when the sweep doesn't run while still being
/// tolerant of how quickly tokio schedules the spawned task.
async fn poll_until<F, T>(deadline: Duration, interval: Duration, mut predicate: F) -> Option<T>
where
    F: FnMut() -> Option<T>,
{
    let start = tokio::time::Instant::now();
    loop {
        if let Some(v) = predicate() {
            return Some(v);
        }
        if start.elapsed() >= deadline {
            return None;
        }
        tokio::time::sleep(interval).await;
    }
}

/// With no embedders registered, the `trigger_vector_sweep_for_layer`
/// hook is a no-op even when the layer declares an active
/// VectorIndex: there's no `SweepCoordinator` to dispatch to. The
/// commit stands, the layer is durable, no segment is written.
/// This pins the "service starts cleanly without embedders" guarantee
/// — operationally important for deployments that don't use vector
/// retrieval at all.
#[tokio::test]
async fn without_embedders_hook_is_a_noop() {
    let service = EigeniusService::new().unwrap();
    let model = "urn:eigenius:embed:dummy:v1";
    let layer = build_corpus_layer(model);

    // Call the hook directly (this is what the `didPersist` slot
    // calls in production).
    let result =
        <EigeniusService as CommitHookHost>::trigger_vector_sweep_for_layer(&service, &layer);
    assert!(result.is_ok(), "no-embedders hook must succeed: {result:?}");

    // Give any spuriously-spawned task time to fire. None should.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let seg = layer
        .storage()
        .vector_index
        .get_segment(&iri("urn:eigenius:test:vi"), layer.id())
        .expect("storage lookup");
    assert!(
        seg.is_none(),
        "with no embedders registered, no segment should ever be written"
    );
}

/// The full contract: register an embedder via `with_embedders`,
/// fire the post-Load hook against a layer that declares an active
/// VectorIndex, and observe a populated segment in the layer's
/// storage. The sweep runs on a spawned task so the test polls
/// (bounded by a generous deadline) for the segment to appear.
#[tokio::test]
async fn with_embedders_hook_spawns_sweep_and_writes_segment() {
    let model = "urn:eigenius:embed:dummy:v1";
    let mut registry = EmbedderRegistry::new();
    registry.register(Arc::new(DummyEmbedder::new(model, 8)));
    let service = EigeniusService::new().unwrap().with_embedders(registry, 32);

    let layer = build_corpus_layer(model);
    let result =
        <EigeniusService as CommitHookHost>::trigger_vector_sweep_for_layer(&service, &layer);
    assert!(
        result.is_ok(),
        "embedder-registered hook must succeed: {result:?}"
    );

    // Sweep runs on a tokio task. 5 s is generous — the dummy
    // embedder is microseconds per call.
    let vi_iri = iri("urn:eigenius:test:vi");
    let layer_id = layer.id().clone();
    let storage = layer.storage().clone();
    let seg = poll_until(Duration::from_secs(5), Duration::from_millis(20), || {
        storage
            .vector_index
            .get_segment(&vi_iri, &layer_id)
            .ok()
            .flatten()
    })
    .await;
    let seg = seg.expect("sweep should produce a segment within 5s");
    assert_eq!(seg.count(), 3, "three docs → three vectors");
    assert_eq!(seg.dim, 8);
    assert_eq!(seg.model_iri.as_str(), model);
}
