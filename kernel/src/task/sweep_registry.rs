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

//! D43 §5.5 / M5.11 — in-memory sweep registry and coordinator.
//!
//! M5.8 shipped the runtime piece — [`crate::task::sweep::VectorSweepDriver`]
//! wraps `sweep_layer_vectors_with_options` with cooperative
//! cancellation, retry, and a `TaskRecord`. What it *doesn't* do is
//! the lifecycle glue D43 §5.5 commits to:
//!
//! > Layer commit emits a sweep task targeting `(L, I)`. ... While
//! > the sweep is in flight, vector queries at any head visible
//! > through L see no contribution from L for property P. ...
//! > Layer-delete interaction: `delete_layer(L)` cancels any
//! > in-flight sweep targeting L via the D21 task-cancel surface.
//!
//! This module ships the in-process state that makes both halves
//! work without yet integrating with `TaskStore` (the cross-restart
//! persistence path remains the [#59](https://github.com/eigenius/eigenius/issues/59)
//! follow-up).
//!
//! - [`SweepRegistry`] — `BTreeMap<LayerId, Arc<SweepHandle>>`,
//!   queryable for status and cancellable per-layer.
//! - [`SweepHandle`] — `(cancel: Arc<AtomicBool>, record:
//!   Arc<RwLock<TaskRecord>>, indexes: Vec<Iri>)`. The `Arc<RwLock>`
//!   on the record lets the spawned sweep update status while
//!   observers concurrently read it.
//! - [`SweepCoordinator`] — bundles the registry + an
//!   `EmbedderRegistry` + an optional `EmbeddingCache` so callers
//!   only pass a layer to fire a sweep. The commit orchestrator's
//!   `DidPersistHook` becomes a one-line dispatch.

use crate::layer::{
    detect_reindex_targets, resolve_active_vector_indexes, ActiveVectorIndex, Layer, LayerId,
};
use crate::ontology::iri::Iri;
use crate::program::embedder::EmbedderRegistry;
use crate::program::embedding_cache::EmbeddingCache;
use crate::query::vector::cache::SegmentCache;
use crate::query::vector::distance::Metric;
use crate::query::vector::hnsw::HnswBuildConfig;
use crate::query::vector::indexing::{
    sweep_layer_vectors_async, AsyncSweepOptions, SweepError, SweepReport,
};
use crate::query::vector::segment::{admit_segment, strategy_from_iri};
use crate::task::reindex::ReindexDriver;
use crate::task::sweep::{VectorSweepDriver, DEFAULT_IN_FLIGHT_LIMIT};
use crate::task::{TaskRecord, TaskStatus};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// Live handle to one in-flight sweep. Returned by
/// [`SweepCoordinator::trigger`] and held in the registry under the
/// sweep's `(layer_id)` key.
#[derive(Debug, Clone)]
pub struct SweepHandle {
    /// Cooperative-cancellation flag the sweep loop polls.
    /// Clonable so external code (`delete_layer`, cancel RPCs) can
    /// flip it without holding the registry lock.
    cancel: Arc<AtomicBool>,
    /// Shared task record. The sweep updates `status` on terminal
    /// transitions; observers read it through the read lock.
    record: Arc<RwLock<TaskRecord>>,
    /// VectorIndex Resource IRIs the sweep was created to
    /// materialise. Recorded for observability ("which Indexes is
    /// this sweep covering?").
    pub indexes: Vec<Iri>,
}

impl SweepHandle {
    /// Flip the cancellation flag. The sweep returns
    /// [`SweepError::Cancelled`] at its next per-Resource or
    /// per-Index check.
    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Snapshot the task record (deep-cloned out of the lock).
    pub fn record_snapshot(&self) -> TaskRecord {
        self.record.read().expect("sweep record poisoned").clone()
    }

    /// Status convenience accessor.
    pub fn status(&self) -> TaskStatus {
        self.record.read().expect("sweep record poisoned").status
    }
}

/// `BTreeMap<LayerId, Arc<SweepHandle>>` with thread-safe
/// `register` / `lookup` / `cancel_by_layer` / `iter` operations.
/// Wrapped in `Arc<RwLock<…>>` internally so the
/// [`SweepCoordinator`] can be cloned cheaply across threads.
///
/// **Two maps**, one per task kind:
///
/// - `sweeps` is keyed by `LayerId` — a per-Load sweep covers every
///   active VectorIndex at the layer in one driver call, so the
///   layer's id is the natural unit.
/// - `reindexes` is keyed by the VectorIndex Resource IRI — a
///   reindex (D43 §5.7 model upgrade) walks the entire chain rather
///   than one layer, and several reindexes against different target
///   Indexes can be in flight concurrently against the same head.
#[derive(Default)]
pub struct SweepRegistry {
    sweeps: RwLock<BTreeMap<LayerId, Arc<SweepHandle>>>,
    reindexes: RwLock<BTreeMap<Iri, Arc<SweepHandle>>>,
}

impl SweepRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the handle for a layer, if any sweep is registered.
    /// Returns a fresh `Arc` clone — drop semantics on the result
    /// don't affect the registry.
    pub fn get(&self, layer_id: &LayerId) -> Option<Arc<SweepHandle>> {
        self.sweeps
            .read()
            .expect("sweep registry poisoned")
            .get(layer_id)
            .map(Arc::clone)
    }

    /// All currently-registered sweeps as `(layer_id, handle)`
    /// pairs. Used by the future GetTaskStatus RPC and by tests.
    pub fn list(&self) -> Vec<(LayerId, Arc<SweepHandle>)> {
        self.sweeps
            .read()
            .expect("sweep registry poisoned")
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    }

    /// Flip the cancellation flag for the layer's sweep, if any.
    /// Returns `true` if a sweep was found and signalled. The
    /// `delete_layer(L)` hook calls this synchronously before
    /// proceeding to its own GC.
    pub fn cancel_by_layer(&self, layer_id: &LayerId) -> bool {
        let guard = self.sweeps.read().expect("sweep registry poisoned");
        if let Some(handle) = guard.get(layer_id) {
            handle.cancel();
            true
        } else {
            false
        }
    }

    /// Forget the registry entry for `layer_id`. Called by the
    /// sweep's cleanup epilogue after it finishes (Completed,
    /// Failed, or Cancelled) so the registry doesn't accumulate
    /// terminal records. The `record_snapshot` taken before this
    /// point preserves the terminal status for observers.
    fn unregister(&self, layer_id: &LayerId) {
        self.sweeps
            .write()
            .expect("sweep registry poisoned")
            .remove(layer_id);
    }

    fn register(&self, layer_id: LayerId, handle: Arc<SweepHandle>) {
        self.sweeps
            .write()
            .expect("sweep registry poisoned")
            .insert(layer_id, handle);
    }

    /// D43 §5.7 — fetch the in-flight reindex handle for a target
    /// VectorIndex Resource, if any.
    pub fn get_reindex(&self, index_iri: &Iri) -> Option<Arc<SweepHandle>> {
        self.reindexes
            .read()
            .expect("reindex registry poisoned")
            .get(index_iri)
            .map(Arc::clone)
    }

    /// All currently-registered reindexes as
    /// `(index_iri, handle)` pairs.
    pub fn list_reindexes(&self) -> Vec<(Iri, Arc<SweepHandle>)> {
        self.reindexes
            .read()
            .expect("reindex registry poisoned")
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    }

    /// Flip the cancellation flag for a reindex targeting
    /// `index_iri`, if one is in flight. Returns `true` iff a
    /// reindex was found and signalled.
    pub fn cancel_reindex(&self, index_iri: &Iri) -> bool {
        let guard = self.reindexes.read().expect("reindex registry poisoned");
        if let Some(handle) = guard.get(index_iri) {
            handle.cancel();
            true
        } else {
            false
        }
    }

    fn register_reindex(&self, index_iri: Iri, handle: Arc<SweepHandle>) {
        self.reindexes
            .write()
            .expect("reindex registry poisoned")
            .insert(index_iri, handle);
    }

    fn unregister_reindex(&self, index_iri: &Iri) {
        self.reindexes
            .write()
            .expect("reindex registry poisoned")
            .remove(index_iri);
    }
}

/// Bundles the [`SweepRegistry`] with the dispatchable resources
/// (embedders, cache) and the entry-point methods the commit
/// orchestrator / RPC layer call into.
pub struct SweepCoordinator {
    registry: Arc<SweepRegistry>,
    embedders: Arc<EmbedderRegistry>,
    cache: Option<Arc<EmbeddingCache>>,
    /// Shared SegmentCache populated post-sweep with the strategy-
    /// dispatched [`crate::query::vector::segment::SegmentView`] —
    /// flat-only when the active VectorIndex's strategy is `flat`,
    /// or with an HNSW graph built for `hnsw` / `auto` (per the
    /// §3.1 strategy slot, M6.3). Same handle the query path's
    /// `FiberRuntime::vector_segment_cache` reads from, so the
    /// sweep's HNSW build pays the cost once.
    segment_cache: Option<Arc<SegmentCache>>,
    /// Default `batch_size` for the [`VectorSweepDriver`] this
    /// coordinator spawns. Inherits
    /// [`crate::query::vector::indexing::DEFAULT_BATCH_SIZE`] (32)
    /// when not overridden. The service-side config layer
    /// ([`eigenius_config::EmbedderConfig::batch_size`]) is the only
    /// production caller; tests rely on the default.
    default_batch_size: usize,
}

impl SweepCoordinator {
    pub fn new(embedders: Arc<EmbedderRegistry>, cache: Option<Arc<EmbeddingCache>>) -> Self {
        Self {
            registry: Arc::new(SweepRegistry::new()),
            embedders,
            cache,
            segment_cache: None,
            default_batch_size: crate::query::vector::indexing::DEFAULT_BATCH_SIZE,
        }
    }

    /// Attach a shared [`SegmentCache`] so post-sweep the
    /// coordinator can admit segments + their HNSW graphs to the
    /// same cache the query handlers read from. Without this, the
    /// HNSW build pays its cost on the first query against the
    /// segment instead of at sweep time.
    pub fn with_segment_cache(mut self, segment_cache: Arc<SegmentCache>) -> Self {
        self.segment_cache = Some(segment_cache);
        self
    }

    /// Set the default `batch_size` for sweeps this coordinator
    /// dispatches. `0` is clamped up to `1`. Affects every
    /// [`Self::trigger_blocking`] / [`Self::trigger_async`] /
    /// [`Self::trigger_reindex_blocking`] call thereafter.
    pub fn with_default_batch_size(mut self, batch_size: usize) -> Self {
        self.default_batch_size = batch_size.max(1);
        self
    }

    /// Shared `Arc<SweepRegistry>` for callers that want to query
    /// status without owning the coordinator — e.g., the RPC
    /// handler that serves `GetVectorSweepStatus`.
    pub fn registry(&self) -> Arc<SweepRegistry> {
        Arc::clone(&self.registry)
    }

    /// Synchronous sweep dispatch — runs the sweep in the calling
    /// thread, registers it for the duration, and unregisters on
    /// completion. Used by tests and by callers that want the
    /// sweep to block (e.g., synchronous CLI commit modes).
    ///
    /// Returns `Ok(None)` when no active VectorIndex Resources are
    /// visible at the layer — no sweep was needed, no handle was
    /// registered.
    pub fn trigger_blocking(&self, layer: &Arc<Layer>) -> Result<Option<SweepHandle>, SweepError> {
        let active = resolve_active_vector_indexes(layer);
        if active.is_empty() {
            return Ok(None);
        }
        let indexes: Vec<Iri> = active.iter().map(|a| a.iri.clone()).collect();
        let record = TaskRecord::new_running(
            Uuid::nil(),
            Uuid::new_v4(),
            "urn:eigenius:program:vector_sweep".into(),
            "urn:eigenius:input:none".into(),
            layer.id().clone(),
            now_millis(),
        );
        let mut driver = VectorSweepDriver::new()
            .with_batch_size(self.default_batch_size)
            .with_record(record.clone());
        let cancel = driver.cancel_handle();
        let record_arc = Arc::new(RwLock::new(record));
        let handle = Arc::new(SweepHandle {
            cancel: Arc::clone(&cancel),
            record: Arc::clone(&record_arc),
            indexes,
        });
        self.registry
            .register(layer.id().clone(), Arc::clone(&handle));

        let cache_ref = self.cache.as_deref();
        let outcome = driver.run(layer, &self.embedders, cache_ref);
        // Sync the driver's terminal status back into the shared
        // record so observers see the right state.
        let terminal_status = match &outcome {
            Ok(_) => TaskStatus::Completed,
            Err(SweepError::Cancelled) => TaskStatus::Cancelled,
            Err(_) => TaskStatus::Failed,
        };
        record_arc.write().expect("sweep record poisoned").status = terminal_status;
        // Strategy-dispatched HNSW build + SegmentCache admission
        // (M6.3). Only fires for fully-completed sweeps; partial /
        // cancelled outcomes leave the cache untouched so a later
        // retry can re-admit cleanly.
        if outcome.is_ok() {
            self.admit_swept_segments_to_cache(layer, &active);
        }
        // Eager cleanup keeps the registry to in-flight + just-
        // terminated entries only. Observers who care about the
        // final state hold their own `Arc<SweepHandle>` clone.
        self.registry.unregister(layer.id());
        outcome?;
        Ok(Some((*handle).clone()))
    }

    /// D43 §5.7 / M8.4 — synchronous reindex dispatch.
    ///
    /// Detects every reindex target visible at `head` via
    /// [`detect_reindex_targets`], spawns a
    /// [`ReindexDriver`] per target, registers each under the
    /// target's IRI for observability + cancellation, runs the
    /// driver synchronously, and returns one [`SweepHandle`] per
    /// target with its terminal [`TaskStatus`] populated.
    ///
    /// The commit hook fires this *after* its regular
    /// [`Self::trigger_blocking`] (or its async sibling): the
    /// post-Load sweep handles fresh VectorIndex Resources at the
    /// new layer; the reindex handles model upgrades at any
    /// pre-existing VectorIndex Resource. Reindex targets are
    /// processed independently — one target's failure does *not*
    /// abort the remaining targets, but the failed target's handle
    /// records its terminal `Failed` state for the caller to act on.
    ///
    /// Returns an empty `Vec` when no target needs reindex (fresh
    /// chains, idempotent re-commits with no model change, etc.) —
    /// the commit hook can call this unconditionally and pay nothing
    /// when there's no work.
    pub fn trigger_reindex_blocking(
        &self,
        head: &Arc<Layer>,
    ) -> Result<Vec<SweepHandle>, SweepError> {
        let targets = detect_reindex_targets(head).map_err(|source| SweepError::Storage {
            index: "<reindex-target-detection>".to_string(),
            source,
        })?;
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(targets.len());
        for target in targets {
            let target_iri = target.index_iri.clone();
            let record = TaskRecord::new_running(
                Uuid::nil(),
                Uuid::new_v4(),
                "urn:eigenius:program:vector_reindex".into(),
                target_iri.as_str().to_string(),
                head.id().clone(),
                now_millis(),
            );
            let mut driver = ReindexDriver::new(target_iri.clone())
                .with_batch_size(self.default_batch_size)
                .with_record(record.clone());
            let cancel = driver.cancel_handle();
            let record_arc = Arc::new(RwLock::new(record));
            let handle = Arc::new(SweepHandle {
                cancel: Arc::clone(&cancel),
                record: Arc::clone(&record_arc),
                indexes: vec![target_iri.clone()],
            });
            self.registry
                .register_reindex(target_iri.clone(), Arc::clone(&handle));

            let cache_ref = self.cache.as_deref();
            let outcome = driver.run(head, &self.embedders, cache_ref);
            // Mirror the sweep epilogue: stamp the terminal status
            // back onto the shared record so observers holding their
            // own `Arc<SweepHandle>` clone after `unregister_reindex`
            // see the right state.
            let terminal_status = match &outcome {
                Ok(_) => TaskStatus::Completed,
                Err(SweepError::Cancelled) => TaskStatus::Cancelled,
                Err(_) => TaskStatus::Failed,
            };
            record_arc.write().expect("reindex record poisoned").status = terminal_status;
            // SegmentCache admission post-reindex — the new
            // segments need the same strategy-dispatched build the
            // sweep applies (`admit_swept_segments_to_cache`). We
            // reuse it: the active VectorIndex at head is the new
            // one (the reindex's target), so passing the head's
            // active set names the just-rewritten segments.
            if outcome.is_ok() {
                let active = resolve_active_vector_indexes(head);
                self.admit_swept_segments_to_cache(head, &active);
            }
            self.registry.unregister_reindex(&target_iri);
            out.push((*handle).clone());
        }
        Ok(out)
    }

    /// Async sweep dispatch — runs the sweep on the current
    /// tokio runtime with `in_flight_limit` concurrent embedder
    /// dispatches per [`AsyncSweepOptions`]. Registers the sweep
    /// for observability + cancellation throughout.
    ///
    /// Returns `Ok(None)` when no active VectorIndex Resources
    /// are visible — same shape as [`Self::trigger_blocking`].
    pub async fn trigger_async(
        &self,
        layer: Arc<Layer>,
    ) -> Result<Option<(SweepHandle, SweepReport)>, SweepError> {
        let active = resolve_active_vector_indexes(&layer);
        if active.is_empty() {
            return Ok(None);
        }
        let indexes: Vec<Iri> = active.iter().map(|a| a.iri.clone()).collect();
        let record = TaskRecord::new_running(
            Uuid::nil(),
            Uuid::new_v4(),
            "urn:eigenius:program:vector_sweep".into(),
            "urn:eigenius:input:none".into(),
            layer.id().clone(),
            now_millis(),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let record_arc = Arc::new(RwLock::new(record));
        let handle = Arc::new(SweepHandle {
            cancel: Arc::clone(&cancel),
            record: Arc::clone(&record_arc),
            indexes,
        });
        let layer_id = layer.id().clone();
        self.registry
            .register(layer_id.clone(), Arc::clone(&handle));

        let options = AsyncSweepOptions {
            cancellation: Some(cancel.as_ref()),
            max_retries: 0,
            retry_backoff_base_ms: 100,
            in_flight_limit: DEFAULT_IN_FLIGHT_LIMIT as usize,
        };
        let outcome = sweep_layer_vectors_async(
            Arc::clone(&layer),
            Arc::clone(&self.embedders),
            self.cache.clone(),
            options,
        )
        .await;
        let terminal_status = match &outcome {
            Ok(_) => TaskStatus::Completed,
            Err(SweepError::Cancelled) => TaskStatus::Cancelled,
            Err(_) => TaskStatus::Failed,
        };
        record_arc.write().expect("sweep record poisoned").status = terminal_status;
        if outcome.is_ok() {
            self.admit_swept_segments_to_cache(&layer, &active);
        }
        self.registry.unregister(&layer_id);
        let report = outcome?;
        Ok(Some(((*handle).clone(), report)))
    }

    /// Post-sweep: for every `(active VectorIndex, just-committed
    /// segment)` pair under `layer`, build the HNSW graph per the
    /// Index's strategy and admit the resulting [`SegmentView`] to
    /// the shared SegmentCache. No-op when no segment cache is
    /// attached or when no segment was actually written (small
    /// layers with no indexable resources).
    fn admit_swept_segments_to_cache(&self, layer: &Layer, active: &[ActiveVectorIndex]) {
        let Some(segment_cache) = self.segment_cache.as_ref() else {
            return;
        };
        for index in active {
            let segment = match layer
                .storage()
                .vector_index
                .get_segment(&index.iri, layer.id())
            {
                Ok(Some(s)) => s,
                _ => continue, // no segment written (e.g. no indexable
                               // resources under this Index in this layer)
            };
            let metric = match Metric::from_short_name(
                &index.distance.as_str()[index
                    .distance
                    .as_str()
                    .rfind(':')
                    .map(|i| i + 1)
                    .unwrap_or(0)..],
            ) {
                Some(m) => m,
                None => continue, // unrecognised metric — leave it for
                                  // lazy build at query time to surface
                                  // the error.
            };
            let strategy = strategy_from_iri(&index.strategy);
            let config = HnswBuildConfig {
                m: index.hnsw_m as usize,
                ef_construction: index.hnsw_ef_construction as usize,
                max_elements: segment.subjects.len().max(16),
            };
            let view = admit_segment(segment, metric, strategy, config);
            segment_cache.insert(index.iri.clone(), layer.id().clone(), Arc::new(view));
        }
    }
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;
    use crate::program::embedder::{DummyEmbedder, Embedder, EmbedderError};

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a head layer with N indexable Documents + one VectorIndex
    /// Resource targeting their body property.
    fn build_corpus(n_docs: usize) -> Arc<Layer> {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("registry-corpus", Some(parent));

        let body_iri = "urn:eigenius:test:body";
        let model_iri = "urn:eigenius:embed:dummy:v1";

        let mut prop = Resource::new(iri(body_iri));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        b.add_resource(prop).unwrap();

        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        b.add_resource(vi).unwrap();

        for i in 0..n_docs {
            let mut d = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
            d.set(iri(body_iri), Value::String(format!("text {i}")));
            b.add_resource(d).unwrap();
        }

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    fn make_coordinator() -> SweepCoordinator {
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(
            "urn:eigenius:embed:dummy:v1",
            8,
        )));
        SweepCoordinator::new(Arc::new(reg), None)
    }

    #[test]
    fn trigger_blocking_runs_to_completion_and_unregisters() {
        let layer = build_corpus(3);
        let coord = make_coordinator();
        let handle = coord
            .trigger_blocking(&layer)
            .expect("sweep")
            .expect("handle returned");

        // Terminal status visible on the returned handle.
        assert_eq!(handle.status(), TaskStatus::Completed);
        // Registry is empty after completion.
        assert!(coord.registry.get(layer.id()).is_none());
        assert_eq!(coord.registry.list().len(), 0);
    }

    #[test]
    fn trigger_returns_none_when_no_active_vector_indexes() {
        // A layer with no VectorIndex Resource — coordinator should
        // short-circuit before registering anything.
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("empty", Some(parent));
        b.add_resource(Resource::new(iri("urn:eigenius:test:placeholder")))
            .unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let coord = make_coordinator();
        let handle = coord.trigger_blocking(&layer).expect("sweep");
        assert!(handle.is_none());
        assert_eq!(coord.registry.list().len(), 0);
    }

    /// Slow embedder that signals start, then waits for an external
    /// release before returning. Lets a separate thread observe
    /// "sweep is running" and act on the registry.
    struct GatedEmbedder {
        iri: Iri,
        dim: u32,
        started: Arc<AtomicBool>,
        release: Arc<AtomicBool>,
    }
    impl Embedder for GatedEmbedder {
        fn model_iri(&self) -> &Iri {
            &self.iri
        }
        fn dim(&self) -> u32 {
            self.dim
        }
        fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedderError> {
            self.started
                .store(true, std::sync::atomic::Ordering::SeqCst);
            // Spin until the test flips `release` (or, if it never
            // does, until we observe the cancel-flag path through
            // the sweep loop — gated embedders are test-only).
            while !self.release.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Ok(vec![0.5; self.dim as usize])
        }
    }

    #[test]
    fn cancel_by_layer_flips_status_to_cancelled() {
        // Spin the sweep on another thread with a gated embedder, then
        // cancel via the registry. The sweep returns with the
        // cancelled status. Verifies the round-trip from
        // `registry.cancel_by_layer` → sweep loop → terminal record.
        let layer = build_corpus(3);
        let model = "urn:eigenius:embed:gated:v1";

        // Re-build with a VectorIndex pointing at the gated embedder.
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("registry-cancel", Some(parent));
        let body_iri = "urn:eigenius:test:body";
        let mut prop = Resource::new(iri(body_iri));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        b.add_resource(prop).unwrap();
        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        b.add_resource(vi).unwrap();
        for i in 0..3 {
            let mut d = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
            d.set(iri(body_iri), Value::String(format!("text {i}")));
            b.add_resource(d).unwrap();
        }
        let _ = layer; // shadow the build-corpus one; this layer drives the test.
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let started = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(GatedEmbedder {
            iri: iri(model),
            dim: 8,
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }));
        let coord = Arc::new(SweepCoordinator::new(Arc::new(reg), None));
        let registry = coord.registry();
        let layer_for_sweep = Arc::clone(&layer);
        let coord_for_sweep = Arc::clone(&coord);

        let join = std::thread::spawn(move || coord_for_sweep.trigger_blocking(&layer_for_sweep));

        // Wait until the embedder is actually executing — meaning
        // the registry entry is live and the sweep is mid-flight.
        while !started.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // Sanity: the sweep is registered.
        assert!(registry.get(layer.id()).is_some());

        // Cancel via the registry, then release the gate so the
        // embedder can return.
        let cancelled = registry.cancel_by_layer(layer.id());
        assert!(cancelled, "registry should find an in-flight sweep");
        release.store(true, std::sync::atomic::Ordering::SeqCst);

        let outcome = join.join().expect("sweep thread");
        let err = outcome.unwrap_err();
        assert!(matches!(err, SweepError::Cancelled));
    }

    #[test]
    fn cancel_by_layer_returns_false_for_unregistered_layer() {
        let coord = make_coordinator();
        let bogus = LayerId([0xab; 32]);
        assert!(!coord.registry.cancel_by_layer(&bogus));
    }

    #[test]
    fn registry_list_includes_in_flight_sweeps() {
        // Drive two simultaneous sweeps on two different layers
        // (gated embedder so they don't complete before we observe).
        let model = "urn:eigenius:embed:gated:v1";
        let started_a = Arc::new(AtomicBool::new(false));
        let started_b = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));

        // Two separate registries, each with a gated embedder, so
        // we can release them together at test end.
        let make_corpus_with_model = |label: &str, started: Arc<AtomicBool>| -> Arc<Layer> {
            let ctx = bootstrap().expect("bootstrap");
            let parent = Arc::clone(ctx.head());
            let mut b = LayerBuilder::new(label, Some(parent));
            let body_iri = "urn:eigenius:test:body";
            let mut prop = Resource::new(iri(body_iri));
            prop.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
            );
            prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
            b.add_resource(prop).unwrap();
            let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
            vi.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
            );
            vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
            vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model)));
            vi.set(iri(wk::VEC_DIM), Value::Integer(8));
            b.add_resource(vi).unwrap();
            let mut d = Resource::new(iri(&format!("urn:eigenius:test:{label}_doc")));
            d.set(iri(body_iri), Value::String("text".into()));
            b.add_resource(d).unwrap();
            // The shared `started` flag is per-layer so we can wait
            // until *this* layer's embedder is actually running.
            let _ = started;
            Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
        };

        let layer_a = make_corpus_with_model("layer_a", Arc::clone(&started_a));
        let layer_b = make_corpus_with_model("layer_b", Arc::clone(&started_b));

        let mut reg_a = EmbedderRegistry::new();
        reg_a.register(Arc::new(GatedEmbedder {
            iri: iri(model),
            dim: 8,
            started: Arc::clone(&started_a),
            release: Arc::clone(&release),
        }));
        let coord_a = Arc::new(SweepCoordinator::new(Arc::new(reg_a), None));
        let registry_a = coord_a.registry();
        let layer_a_for_sweep = Arc::clone(&layer_a);
        let coord_a_for_sweep = Arc::clone(&coord_a);

        let mut reg_b = EmbedderRegistry::new();
        reg_b.register(Arc::new(GatedEmbedder {
            iri: iri(model),
            dim: 8,
            started: Arc::clone(&started_b),
            release: Arc::clone(&release),
        }));
        let coord_b = Arc::new(SweepCoordinator::new(Arc::new(reg_b), None));
        let registry_b = coord_b.registry();
        let layer_b_for_sweep = Arc::clone(&layer_b);
        let coord_b_for_sweep = Arc::clone(&coord_b);

        let join_a =
            std::thread::spawn(move || coord_a_for_sweep.trigger_blocking(&layer_a_for_sweep));
        let join_b =
            std::thread::spawn(move || coord_b_for_sweep.trigger_blocking(&layer_b_for_sweep));

        // Wait for both to start.
        while !started_a.load(std::sync::atomic::Ordering::SeqCst)
            || !started_b.load(std::sync::atomic::Ordering::SeqCst)
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // Each coordinator's own registry sees its own sweep.
        assert!(registry_a.get(layer_a.id()).is_some());
        assert!(registry_b.get(layer_b.id()).is_some());
        assert!(registry_a.get(layer_b.id()).is_none());
        assert!(registry_b.get(layer_a.id()).is_none());

        // Release both.
        release.store(true, std::sync::atomic::Ordering::SeqCst);
        join_a
            .join()
            .expect("join a")
            .expect("sweep a")
            .expect("handle a");
        join_b
            .join()
            .expect("join b")
            .expect("sweep b")
            .expect("handle b");
    }

    // ─── Async sweep / in-flight cap (M5.12) ────────────────────

    /// Embedder that sleeps for a fixed duration on each call so we
    /// can observe the speedup from concurrent dispatch.
    struct SleepingEmbedder {
        iri: Iri,
        dim: u32,
        delay_ms: u64,
    }
    impl Embedder for SleepingEmbedder {
        fn model_iri(&self) -> &Iri {
            &self.iri
        }
        fn dim(&self) -> u32 {
            self.dim
        }
        fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedderError> {
            std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
            Ok(vec![0.5; self.dim as usize])
        }
    }

    fn build_corpus_with_model(model_iri: &str, n_docs: usize) -> Arc<Layer> {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("perf-corpus", Some(parent));
        let body_iri = "urn:eigenius:test:body";
        let mut prop = Resource::new(iri(body_iri));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        b.add_resource(prop).unwrap();

        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        b.add_resource(vi).unwrap();

        for i in 0..n_docs {
            let mut d = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
            // Distinct text per doc so the embedding cache doesn't
            // collapse the calls into one.
            d.set(iri(body_iri), Value::String(format!("doc {i}")));
            b.add_resource(d).unwrap();
        }
        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn async_sweep_is_faster_than_sequential() {
        // The sleeping embedder takes 50 ms per call. 16 subjects
        // serialised would take ~800 ms; with an in-flight cap of
        // 8 and the multi-thread runtime, two batches should run
        // in ~150 ms. Assert under 400 ms for CI tolerance.
        let model = "urn:eigenius:embed:sleeping:v1";
        let layer = build_corpus_with_model(model, 16);
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(SleepingEmbedder {
            iri: iri(model),
            dim: 8,
            delay_ms: 50,
        }));
        let coord = SweepCoordinator::new(Arc::new(reg), None);

        let start = std::time::Instant::now();
        let outcome = coord
            .trigger_async(Arc::clone(&layer))
            .await
            .expect("sweep");
        let elapsed = start.elapsed();

        let (handle, report) = outcome.expect("handle + report");
        assert_eq!(handle.status(), TaskStatus::Completed);
        assert_eq!(report.total_subjects, 16);
        assert!(
            elapsed.as_millis() < 400,
            "async sweep should beat serial (~800 ms); got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn async_sweep_with_no_active_indexes_returns_none() {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("empty", Some(parent));
        b.add_resource(Resource::new(iri("urn:eigenius:test:placeholder")))
            .unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));
        let coord = make_coordinator();
        assert!(coord.trigger_async(layer).await.expect("sweep").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_sweep_writes_segments_queryable_via_chain() {
        // Sanity: async sweep completes and its segments are visible
        // through `VectorIndex::get_segment` for the right
        // `(index_iri, layer_id)` key.
        let model = "urn:eigenius:embed:dummy:v1";
        let layer = build_corpus_with_model(model, 4);
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model, 8)));
        let coord = SweepCoordinator::new(Arc::new(reg), None);
        let _ = coord
            .trigger_async(Arc::clone(&layer))
            .await
            .expect("sweep")
            .expect("handle");

        let segment = layer
            .storage()
            .vector_index
            .get_segment(&iri("urn:eigenius:test:vi"), layer.id())
            .expect("storage")
            .expect("segment present after sweep");
        assert_eq!(segment.subjects.len(), 4);
    }

    // ─── Build-time strategy dispatch (M6.3) ────────────────────

    /// Build a corpus with the given strategy IRI configured on the
    /// VectorIndex Resource. Mirrors [`build_corpus_with_model`]
    /// but exposes the strategy slot.
    fn build_corpus_with_strategy(
        model_iri: &str,
        strategy_iri: Option<&str>,
        n_docs: usize,
    ) -> Arc<Layer> {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("strategy-corpus", Some(parent));
        let body_iri = "urn:eigenius:test:body";
        let mut prop = Resource::new(iri(body_iri));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        b.add_resource(prop).unwrap();

        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        if let Some(s) = strategy_iri {
            vi.set(iri(wk::VEC_STRATEGY), Value::ResourceRef(iri(s)));
        }
        b.add_resource(vi).unwrap();

        for i in 0..n_docs {
            let mut d = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
            d.set(iri(body_iri), Value::String(format!("doc {i}")));
            b.add_resource(d).unwrap();
        }
        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    fn coordinator_with_segment_cache(model_iri: &str) -> (SweepCoordinator, Arc<SegmentCache>) {
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model_iri, 8)));
        let segment_cache = Arc::new(SegmentCache::new(16));
        let coord = SweepCoordinator::new(Arc::new(reg), None)
            .with_segment_cache(Arc::clone(&segment_cache));
        (coord, segment_cache)
    }

    #[test]
    fn strategy_hnsw_admits_view_with_hnsw_graph() {
        let model = "urn:eigenius:embed:dummy:v1";
        let layer = build_corpus_with_strategy(model, Some("urn:eigenius:core:strategies:hnsw"), 8);
        let (coord, segment_cache) = coordinator_with_segment_cache(model);
        coord.trigger_blocking(&layer).expect("sweep");

        let view = segment_cache
            .get(&iri("urn:eigenius:test:vi"), layer.id())
            .expect("cached after sweep");
        assert!(
            view.hnsw().is_some(),
            "strategy=hnsw should attach an HNSW graph"
        );
        assert_eq!(view.hnsw().unwrap().count(), 8);
    }

    #[test]
    fn strategy_flat_admits_view_without_hnsw_graph() {
        let model = "urn:eigenius:embed:dummy:v1";
        let layer = build_corpus_with_strategy(model, Some("urn:eigenius:core:strategies:flat"), 8);
        let (coord, segment_cache) = coordinator_with_segment_cache(model);
        coord.trigger_blocking(&layer).expect("sweep");

        let view = segment_cache
            .get(&iri("urn:eigenius:test:vi"), layer.id())
            .expect("cached after sweep");
        assert!(
            view.hnsw().is_none(),
            "strategy=flat should not attach an HNSW graph"
        );
    }

    #[test]
    fn strategy_auto_below_threshold_skips_hnsw_build() {
        // 8 docs is well below the 50K default threshold — auto
        // should leave HNSW unbuilt.
        let model = "urn:eigenius:embed:dummy:v1";
        let layer = build_corpus_with_strategy(model, Some("urn:eigenius:core:strategies:auto"), 8);
        let (coord, segment_cache) = coordinator_with_segment_cache(model);
        coord.trigger_blocking(&layer).expect("sweep");

        let view = segment_cache
            .get(&iri("urn:eigenius:test:vi"), layer.id())
            .expect("cached after sweep");
        assert!(
            view.hnsw().is_none(),
            "auto strategy with 8 docs (< 50K threshold) should skip HNSW"
        );
    }

    #[test]
    fn omitted_strategy_defaults_to_auto() {
        // No strategy set on the Resource → auto by default (per
        // index_discovery::vec_defaults::STRATEGY).
        let model = "urn:eigenius:embed:dummy:v1";
        let layer = build_corpus_with_strategy(model, None, 4);
        let (coord, segment_cache) = coordinator_with_segment_cache(model);
        coord.trigger_blocking(&layer).expect("sweep");

        let view = segment_cache
            .get(&iri("urn:eigenius:test:vi"), layer.id())
            .expect("cached after sweep");
        // 4 < 50K → no HNSW.
        assert!(view.hnsw().is_none());
    }

    #[test]
    fn sweep_without_segment_cache_is_noop_for_admission() {
        // The coordinator gets no SegmentCache. The sweep still
        // completes; the cache stays empty and queries pay the
        // lazy build cost (M6.4 territory). Verifies the optional-
        // segment-cache wiring doesn't break the happy path.
        let model = "urn:eigenius:embed:dummy:v1";
        let layer = build_corpus_with_strategy(model, Some("urn:eigenius:core:strategies:hnsw"), 4);
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model, 8)));
        let coord = SweepCoordinator::new(Arc::new(reg), None);
        let handle = coord
            .trigger_blocking(&layer)
            .expect("sweep")
            .expect("handle");
        assert_eq!(handle.status(), TaskStatus::Completed);
    }

    // ─── D43 §5.7 / M8.4 reindex-trigger tests ──────────────────────────

    /// Build a chain `bootstrap → L1 (VI model_a) → L2 (docs swept
    /// under model_a) → L3 (VI redeclared with model_b)`. At L3 the
    /// declared model and the segment's recorded model disagree;
    /// the coordinator's reindex trigger should detect this, run the
    /// driver, and surface a Completed handle.
    fn build_chain_with_model_upgrade(
        model_a: &str,
        model_b: &str,
    ) -> (Arc<Layer>, EmbedderRegistry) {
        let ctx = bootstrap().expect("bootstrap");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let target_prop = "urn:eigenius:test:body";

        let mut l1 = LayerBuilder::new("l1", Some(head));
        let mut prop = Resource::new(iri(target_prop));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        l1.add_resource(prop).unwrap();
        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_a)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        l1.add_resource(vi).unwrap();
        let l1 = Arc::new(l1.build(storage.clone()));

        let mut l2 = LayerBuilder::new("l2", Some(Arc::clone(&l1)));
        let mut d = Resource::new(iri("urn:eigenius:test:d1"));
        d.set(iri(target_prop), Value::String("alpha beta".into()));
        l2.add_resource(d).unwrap();
        let l2 = Arc::new(l2.build(storage.clone()));
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model_a, 8)));
        reg.register(Arc::new(DummyEmbedder::new(model_b, 8)));
        crate::query::vector::indexing::sweep_layer_vectors(&l2, &reg, None).unwrap();

        let mut l3 = LayerBuilder::new("l3", Some(Arc::clone(&l2)));
        let mut vi2 = Resource::new(iri("urn:eigenius:test:vi"));
        vi2.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi2.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        vi2.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_b)));
        vi2.set(iri(wk::VEC_DIM), Value::Integer(8));
        l3.add_resource(vi2).unwrap();
        let l3 = Arc::new(l3.build(storage));
        (l3, reg)
    }

    #[test]
    fn trigger_reindex_blocking_runs_to_completion_and_unregisters() {
        let model_a = "urn:eigenius:embed:dummy:v1";
        let model_b = "urn:eigenius:embed:dummy:v2";
        let (head, reg) = build_chain_with_model_upgrade(model_a, model_b);
        let coord = SweepCoordinator::new(Arc::new(reg), None);

        let handles = coord
            .trigger_reindex_blocking(&head)
            .expect("reindex trigger");
        assert_eq!(handles.len(), 1, "expected one reindex target");
        assert_eq!(handles[0].status(), TaskStatus::Completed);
        assert_eq!(
            handles[0].indexes[0].as_str(),
            "urn:eigenius:test:vi",
            "handle records the reindexed Index IRI"
        );
        // Registry is empty post-completion.
        assert!(coord
            .registry
            .get_reindex(&iri("urn:eigenius:test:vi"))
            .is_none());
        assert!(coord.registry.list_reindexes().is_empty());

        // Post-reindex, the segment at the head's reachable layer
        // (the original L2) carries the new model.
        let seg = head
            .storage()
            .vector_index
            .get_segment(&iri("urn:eigenius:test:vi"), head.parent().unwrap().id())
            .unwrap()
            .expect("segment exists post-reindex");
        assert_eq!(seg.model_iri.as_str(), model_b);
    }

    #[test]
    fn trigger_reindex_blocking_returns_empty_when_no_target() {
        // A fresh corpus — VectorIndex declared once, no shadow at
        // upper layer. The trigger should detect zero targets and
        // surface an empty Vec without touching the registry.
        let layer = build_corpus(2);
        let coord = make_coordinator();
        let handles = coord
            .trigger_reindex_blocking(&layer)
            .expect("reindex trigger");
        assert!(
            handles.is_empty(),
            "expected no reindex targets on fresh corpus"
        );
        assert!(coord.registry.list_reindexes().is_empty());
    }

    #[test]
    fn cancel_reindex_returns_false_for_unregistered_index() {
        let coord = make_coordinator();
        let cancelled = coord
            .registry
            .cancel_reindex(&iri("urn:eigenius:test:nonexistent"));
        assert!(!cancelled);
    }

    /// Spin a reindex on a separate thread with a gated embedder
    /// driving the per-subject embed call. Once the embedder
    /// signals `started`, cancel the reindex via the registry. The
    /// driver returns `Cancelled` and the handle records the
    /// terminal `Cancelled` status. End-to-end proof that the
    /// reindex registry's cancellation surface composes with the
    /// driver's cooperative-cancellation flag.
    #[test]
    fn cancel_reindex_in_flight_flips_status_to_cancelled() {
        let model_a = "urn:eigenius:embed:dummy:v1";
        let model_b = "urn:eigenius:embed:gated:v2";
        // Build the upgrade chain with model_a docs swept; redeclare
        // the VI at model_b so the reindex needs to call the gated
        // embedder for every subject.
        let ctx = bootstrap().expect("bootstrap");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let target_prop = "urn:eigenius:test:body";

        let mut l1 = LayerBuilder::new("l1", Some(head));
        let mut prop = Resource::new(iri(target_prop));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        l1.add_resource(prop).unwrap();
        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_a)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        l1.add_resource(vi).unwrap();
        let l1 = Arc::new(l1.build(storage.clone()));

        let mut l2 = LayerBuilder::new("l2", Some(Arc::clone(&l1)));
        let mut d = Resource::new(iri("urn:eigenius:test:d1"));
        d.set(iri(target_prop), Value::String("alpha beta".into()));
        l2.add_resource(d).unwrap();
        let l2 = Arc::new(l2.build(storage.clone()));
        // Sweep under model_a via the dummy embedder.
        let mut sweep_reg = EmbedderRegistry::new();
        sweep_reg.register(Arc::new(DummyEmbedder::new(model_a, 8)));
        crate::query::vector::indexing::sweep_layer_vectors(&l2, &sweep_reg, None).unwrap();

        let mut l3 = LayerBuilder::new("l3", Some(Arc::clone(&l2)));
        let mut vi2 = Resource::new(iri("urn:eigenius:test:vi"));
        vi2.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi2.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        vi2.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_b)));
        vi2.set(iri(wk::VEC_DIM), Value::Integer(8));
        l3.add_resource(vi2).unwrap();
        let head = Arc::new(l3.build(storage));

        // Coordinator with the GATED embedder for model_b — every
        // per-subject embed call blocks until `release` is flipped.
        let started = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(GatedEmbedder {
            iri: iri(model_b),
            dim: 8,
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }));
        let coord = Arc::new(SweepCoordinator::new(Arc::new(reg), None));
        let registry = coord.registry();
        let head_for_reindex = Arc::clone(&head);
        let coord_for_reindex = Arc::clone(&coord);

        let join = std::thread::spawn(move || {
            coord_for_reindex.trigger_reindex_blocking(&head_for_reindex)
        });

        // Wait for the gated embedder to signal it's running.
        while !started.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // The reindex is registered under the target Index IRI.
        let target_iri = iri("urn:eigenius:test:vi");
        assert!(registry.get_reindex(&target_iri).is_some());

        // Cancel via the registry, then release the gate.
        let cancelled = registry.cancel_reindex(&target_iri);
        assert!(cancelled, "registry should find the in-flight reindex");
        release.store(true, std::sync::atomic::Ordering::SeqCst);

        let outcome = join.join().expect("reindex thread");
        let handles = outcome.expect("reindex trigger returns Ok");
        // The reindex's terminal status is Cancelled — propagated
        // through the driver's record onto the registered handle.
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].status(), TaskStatus::Cancelled);
        // Registry is empty post-completion.
        assert!(registry.get_reindex(&target_iri).is_none());
    }
}
