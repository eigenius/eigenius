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

//! D43 §5.5 / M5.8 — post-Load vector-index sweep task driver.
//!
//! Layer commit is structurally complete the moment the deterministic
//! index entries land atomically with the topology record. Vector
//! segments — `vec_seg:<I>:<L>` blobs whose derivation needs an IO
//! call to an Embedder Component — commit later, asynchronously,
//! through a sweep that this module owns.
//!
//! The driver wraps the work-doer in [`crate::query::vector::indexing`]
//! ([`crate::query::vector::indexing::sweep_layer_vectors_with_options`])
//! with the per-D43 §5.5 sweep contract:
//!
//! - **Observability.** Each sweep run owns a [`TaskRecord`] whose
//!   `status` reflects `Running` → `Completed` / `Failed` /
//!   `Cancelled`. v1 keeps the record in-process; persisting it
//!   through `TaskStore` so `GetTaskStatus` can observe it is the
//!   wiring step performed when the post-Load commit hook gains a
//!   `task_store` handle (the M5 follow-up that lights up §5.5's
//!   coverage query).
//! - **Cancellation.** A cooperative-cancellation [`AtomicBool`]
//!   handle is exposed via [`VectorSweepDriver::cancel_handle`]; the
//!   sweep checks it between Resources and Indexes and returns
//!   [`crate::query::vector::indexing::SweepError::Cancelled`] when
//!   raised. `delete_layer(L)` will flip it for any sweep targeting
//!   `L` once the commit hook is in place.
//! - **Retry.** Transient [`crate::program::embedder::EmbedderError::Io`]
//!   failures retry up to `max_retries` times with exponential
//!   backoff per [`SweepOptions::retry_backoff_base_ms`].
//! - **In-flight cap.** Per D43 §5.5 the production sweep limits
//!   concurrent embedder calls to ~64. v1 ships the cap as a knob
//!   ([`VectorSweepDriver::in_flight_limit`]) recorded on the task
//!   record without yet driving concurrent embedder calls — the
//!   sweep is sync today; the M5 follow-up that makes it async
//!   reads this field to size the bounded semaphore.

use crate::layer::Layer;
use crate::program::embedder::EmbedderRegistry;
use crate::program::embedding_cache::EmbeddingCache;
use crate::query::vector::indexing::{
    sweep_layer_vectors_with_options, SweepError, SweepOptions, SweepReport,
};
use crate::task::{TaskRecord, TaskStatus};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Default in-flight embedder-call cap, per D43 §5.5.
pub const DEFAULT_IN_FLIGHT_LIMIT: u32 = 64;
/// Default retry attempts per subject.
pub const DEFAULT_MAX_RETRIES: u32 = 3;
/// Default exponential-backoff base in milliseconds.
pub const DEFAULT_RETRY_BACKOFF_BASE_MS: u64 = 100;

/// Runtime handle to a single `(layer, [VectorIndex])`
/// materialisation unit. Holds the cooperative-cancellation flag
/// (clonable so external code — `delete_layer`, the cancel RPC —
/// can flip it) and the per-sweep policy knobs.
pub struct VectorSweepDriver {
    cancel: Arc<AtomicBool>,
    pub max_retries: u32,
    pub retry_backoff_base_ms: u64,
    pub in_flight_limit: u32,
    /// Cache-miss text chunk size for the per-Index batched embed
    /// path ([`crate::query::vector::indexing::SweepOptions::batch_size`]).
    /// Defaults to [`crate::query::vector::indexing::DEFAULT_BATCH_SIZE`].
    /// Set to `1` to reproduce pre-batched legacy per-text dispatch.
    pub batch_size: usize,
    record: Option<TaskRecord>,
}

impl VectorSweepDriver {
    /// Construct a sweep driver with D43 §5.5 defaults
    /// (3 retries, 100 ms backoff base, 64 in-flight cap,
    /// batch ≈ 32).
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            max_retries: DEFAULT_MAX_RETRIES,
            retry_backoff_base_ms: DEFAULT_RETRY_BACKOFF_BASE_MS,
            in_flight_limit: DEFAULT_IN_FLIGHT_LIMIT,
            batch_size: crate::query::vector::indexing::DEFAULT_BATCH_SIZE,
            record: None,
        }
    }

    /// Attach an existing [`TaskRecord`] — the post-Load commit
    /// hook constructs one at commit time so its `layer_head` is
    /// pinned to the committing layer. v1 stores it inline; the
    /// follow-up that wires `TaskStore` reads from / writes to
    /// this field on each state transition.
    pub fn with_record(mut self, record: TaskRecord) -> Self {
        self.record = Some(record);
        self
    }

    /// Override the `batch_size` used by this driver's sweep. `0`
    /// is clamped to `1` (per-text dispatch). The default at
    /// construction is
    /// [`crate::query::vector::indexing::DEFAULT_BATCH_SIZE`]; the
    /// service-side [`crate::task::sweep_registry::SweepCoordinator`]
    /// threads its configured default through this method on every
    /// new driver it spawns.
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size.max(1);
        self
    }

    /// Clonable handle on the cooperative-cancellation flag.
    /// `delete_layer(L)` flips this when it sees an in-flight
    /// sweep targeting `L`.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Snapshot of the driver's task record, if attached. Returns
    /// `None` for the test-only no-record path.
    pub fn record(&self) -> Option<&TaskRecord> {
        self.record.as_ref()
    }

    /// Drive the sweep synchronously. Updates the attached
    /// [`TaskRecord::status`] on completion / cancellation /
    /// failure so observers see the right terminal state. Returns
    /// the [`SweepReport`] on success.
    pub fn run(
        &mut self,
        layer: &Layer,
        embedders: &EmbedderRegistry,
        cache: Option<&EmbeddingCache>,
    ) -> Result<SweepReport, SweepError> {
        let options = SweepOptions {
            cancellation: Some(self.cancel.as_ref()),
            max_retries: self.max_retries,
            retry_backoff_base_ms: self.retry_backoff_base_ms,
            batch_size: self.batch_size,
        };
        let outcome = sweep_layer_vectors_with_options(layer, embedders, cache, &options);
        if let Some(record) = self.record.as_mut() {
            record.status = match &outcome {
                Ok(_) => TaskStatus::Completed,
                Err(SweepError::Cancelled) => TaskStatus::Cancelled,
                Err(_) => TaskStatus::Failed,
            };
        }
        outcome
    }
}

impl Default for VectorSweepDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::layer::LayerBuilder;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;
    use crate::program::embedder::{DummyEmbedder, Embedder, EmbedderError, EmbedderRegistry};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn build_corpus(n_docs: usize) -> (Arc<crate::layer::Layer>, EmbedderRegistry) {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("driver-corpus", Some(parent));

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

        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model_iri, 8)));
        (layer, reg)
    }

    #[test]
    fn driver_defaults_match_design_v1() {
        let d = VectorSweepDriver::new();
        assert_eq!(d.max_retries, DEFAULT_MAX_RETRIES);
        assert_eq!(d.retry_backoff_base_ms, DEFAULT_RETRY_BACKOFF_BASE_MS);
        assert_eq!(d.in_flight_limit, DEFAULT_IN_FLIGHT_LIMIT);
        assert!(d.record().is_none());
    }

    #[test]
    fn run_completes_and_returns_report() {
        let (layer, reg) = build_corpus(3);
        let mut driver = VectorSweepDriver::new();
        let report = driver.run(&layer, &reg, None).expect("sweep");
        assert_eq!(report.total_subjects, 3);
    }

    #[test]
    fn run_updates_attached_record_to_completed() {
        let (layer, reg) = build_corpus(2);
        let record = TaskRecord::new_running(
            uuid::Uuid::nil(),
            uuid::Uuid::new_v4(),
            "urn:eigenius:program:sweep".into(),
            "urn:eigenius:input:none".into(),
            layer.id().clone(),
            0,
        );
        let mut driver = VectorSweepDriver::new().with_record(record);
        assert_eq!(driver.record().unwrap().status, TaskStatus::Running);
        let _ = driver.run(&layer, &reg, None).expect("sweep");
        assert_eq!(driver.record().unwrap().status, TaskStatus::Completed);
    }

    #[test]
    fn cancellation_flips_status_to_cancelled() {
        let (layer, reg) = build_corpus(5);
        let record = TaskRecord::new_running(
            uuid::Uuid::nil(),
            uuid::Uuid::new_v4(),
            "urn:eigenius:program:sweep".into(),
            "urn:eigenius:input:none".into(),
            layer.id().clone(),
            0,
        );
        let mut driver = VectorSweepDriver::new().with_record(record);
        // Pre-cancel so the sweep returns Cancelled at the first check.
        driver.cancel_handle().store(true, Ordering::SeqCst);
        let err = driver.run(&layer, &reg, None).unwrap_err();
        assert!(matches!(err, SweepError::Cancelled));
        assert_eq!(driver.record().unwrap().status, TaskStatus::Cancelled);
    }

    /// Embedder that fails N times with `EmbedderError::Io`, then
    /// succeeds. Used to verify retry-with-backoff works.
    struct FlakyEmbedder {
        iri: Iri,
        dim: u32,
        remaining_failures: std::sync::atomic::AtomicI32,
    }

    impl Embedder for FlakyEmbedder {
        fn model_iri(&self) -> &Iri {
            &self.iri
        }
        fn dim(&self) -> u32 {
            self.dim
        }
        fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedderError> {
            let prev = self.remaining_failures.fetch_sub(1, Ordering::SeqCst);
            if prev > 0 {
                return Err(EmbedderError::Io("transient".into()));
            }
            Ok(vec![0.0; self.dim as usize])
        }
    }

    #[test]
    fn retry_on_transient_io_failure_eventually_succeeds() {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("retry-corpus", Some(parent));

        let body_iri = "urn:eigenius:test:body";
        let model_iri = "urn:eigenius:embed:flaky:v1";

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
        vi.set(iri(wk::VEC_DIM), Value::Integer(4));
        b.add_resource(vi).unwrap();

        let mut d = Resource::new(iri("urn:eigenius:test:doc"));
        d.set(iri(body_iri), Value::String("text".into()));
        b.add_resource(d).unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(FlakyEmbedder {
            iri: iri(model_iri),
            dim: 4,
            remaining_failures: std::sync::atomic::AtomicI32::new(2),
        }));

        let mut driver = VectorSweepDriver::new();
        driver.max_retries = 3; // 2 failures, then success on 3rd
        driver.retry_backoff_base_ms = 1; // keep the test fast
        let report = driver.run(&layer, &reg, None).expect("retry succeeds");
        assert_eq!(report.total_subjects, 1);
    }

    #[test]
    fn retry_gives_up_after_max_attempts() {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("max-retry", Some(parent));

        let body_iri = "urn:eigenius:test:body";
        let model_iri = "urn:eigenius:embed:flaky:v1";

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
        vi.set(iri(wk::VEC_DIM), Value::Integer(4));
        b.add_resource(vi).unwrap();

        let mut d = Resource::new(iri("urn:eigenius:test:doc"));
        d.set(iri(body_iri), Value::String("text".into()));
        b.add_resource(d).unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(FlakyEmbedder {
            iri: iri(model_iri),
            dim: 4,
            // Always fails — more than max_retries can absorb.
            remaining_failures: std::sync::atomic::AtomicI32::new(100),
        }));

        let mut driver = VectorSweepDriver::new();
        driver.max_retries = 2;
        driver.retry_backoff_base_ms = 1;
        let err = driver.run(&layer, &reg, None).unwrap_err();
        assert!(
            matches!(
                err,
                SweepError::EmbedderDispatch {
                    source: EmbedderError::Io(_),
                    ..
                }
            ),
            "expected EmbedderDispatch(Io); got {err:?}"
        );
    }
}
