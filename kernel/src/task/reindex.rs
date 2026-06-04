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

//! D43 §5.7 / M8.4 — atomic-reindex task driver.
//!
//! When a new `core:VectorIndex` Resource shadows an existing one on
//! the same `target_property` — typically because the schema owner
//! committed an upgraded embedding model — every visible layer needs
//! its vector segments rewritten under the new Index's IRI before
//! queries against the post-shadow head can return well-typed hits.
//! That rewrite is what this driver runs.
//!
//! Architecturally parallel to
//! [`crate::task::sweep::VectorSweepDriver`]:
//!
//! - The work-doer is
//!   [`crate::query::vector::indexing::reindex_chain`] — walks the
//!   chain head→root and per-layer sweeps every defined Resource's
//!   property value under the target VectorIndex's model. Per-layer
//!   atomicity from the existing `extend_layer` trait contract;
//!   chain-wide is *not* atomic (per D43 §5.7's "progressive
//!   availability" stance) — queries between the first and last
//!   per-layer commit see partial coverage.
//! - Observability via an attached [`TaskRecord`] whose `status`
//!   transitions to `Completed` / `Failed` / `Cancelled` on
//!   termination. v1 keeps the record in-process; persisting it
//!   through `TaskStore` so `GetTaskStatus` can observe it is the
//!   M8.5 follow-up.
//! - Cancellation through a cooperative [`AtomicBool`] flag. The
//!   reindex checks it between layers and between Resources within a
//!   layer; cancel propagates as
//!   [`SweepError::Cancelled`].
//! - Retries on transient
//!   [`crate::program::embedder::EmbedderError::Io`] failures with
//!   exponential backoff, sharing the
//!   [`SweepOptions`] knobs the sweep driver uses.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::program::embedder::EmbedderRegistry;
use crate::program::embedding_cache::EmbeddingCache;
use crate::query::vector::indexing::{reindex_chain, SweepError, SweepOptions, SweepReport};
use crate::task::sweep::{
    DEFAULT_IN_FLIGHT_LIMIT, DEFAULT_MAX_RETRIES, DEFAULT_RETRY_BACKOFF_BASE_MS,
};
use crate::task::{TaskRecord, TaskStatus};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Runtime handle to a single chain-wide reindex against one target
/// VectorIndex Resource. Constructed by the commit hook that
/// observes the new Index shadowing the old one; run synchronously
/// or scheduled onto the task executor.
pub struct ReindexDriver {
    target_index_iri: Iri,
    cancel: Arc<AtomicBool>,
    pub max_retries: u32,
    pub retry_backoff_base_ms: u64,
    pub in_flight_limit: u32,
    pub batch_size: usize,
    record: Option<TaskRecord>,
}

impl ReindexDriver {
    /// Construct a reindex driver targeting `target_index_iri` with
    /// the D43 §5.5 defaults (3 retries, 100 ms backoff base, 64
    /// in-flight cap). The target IRI must resolve to an active
    /// VectorIndex Resource at the head passed to [`Self::run`];
    /// otherwise the chain-walk returns the standard "embedder not
    /// registered" diagnostic via [`reindex_chain`].
    pub fn new(target_index_iri: Iri) -> Self {
        Self {
            target_index_iri,
            cancel: Arc::new(AtomicBool::new(false)),
            max_retries: DEFAULT_MAX_RETRIES,
            retry_backoff_base_ms: DEFAULT_RETRY_BACKOFF_BASE_MS,
            in_flight_limit: DEFAULT_IN_FLIGHT_LIMIT,
            batch_size: crate::query::vector::indexing::DEFAULT_BATCH_SIZE,
            record: None,
        }
    }

    /// Override the `batch_size` passed to `reindex_chain`'s sweep.
    /// `0` is clamped to `1`. Defaults to
    /// [`crate::query::vector::indexing::DEFAULT_BATCH_SIZE`].
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size.max(1);
        self
    }

    /// Attach an existing [`TaskRecord`]. Per the [`VectorSweepDriver`]
    /// pattern, the commit hook constructs one at commit time so its
    /// `layer_head` is pinned to the head against which the reindex
    /// runs; this method stashes it for the status transition on
    /// [`Self::run`].
    ///
    /// [`VectorSweepDriver`]: crate::task::sweep::VectorSweepDriver
    pub fn with_record(mut self, record: TaskRecord) -> Self {
        self.record = Some(record);
        self
    }

    /// IRI of the VectorIndex Resource this driver targets.
    pub fn target_index_iri(&self) -> &Iri {
        &self.target_index_iri
    }

    /// Clonable handle on the cooperative-cancellation flag.
    /// `delete_layer(L)` on any layer the reindex touches flips
    /// this; the reindex returns `SweepError::Cancelled` at the
    /// next check.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Snapshot of the driver's task record, if attached. Returns
    /// `None` for the test-only no-record path.
    pub fn record(&self) -> Option<&TaskRecord> {
        self.record.as_ref()
    }

    /// Drive the reindex synchronously. Walks the chain head→root,
    /// per-layer sweeps every Resource with the target property
    /// against the new VectorIndex's model, and writes the result
    /// segments. Updates the attached [`TaskRecord::status`] on
    /// completion / cancellation / failure so observers see the
    /// right terminal state. Returns the [`SweepReport`] on success.
    pub fn run(
        &mut self,
        head: &Layer,
        embedders: &EmbedderRegistry,
        cache: Option<&EmbeddingCache>,
    ) -> Result<SweepReport, SweepError> {
        let options = SweepOptions {
            cancellation: Some(self.cancel.as_ref()),
            max_retries: self.max_retries,
            retry_backoff_base_ms: self.retry_backoff_base_ms,
            batch_size: self.batch_size,
        };
        let outcome = reindex_chain(head, &self.target_index_iri, embedders, cache, &options);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;
    use crate::program::embedder::DummyEmbedder;
    use std::sync::atomic::Ordering;

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

    /// Build a chain with a VectorIndex declared at the head and
    /// one Resource with a string property to be embedded. The
    /// chain hasn't been swept; the reindex itself populates the
    /// segment.
    fn build_corpus(target_index_iri: &str, model_iri: &str) -> Arc<Layer> {
        let ctx = bootstrap().expect("bootstrap");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let mut b = LayerBuilder::new("corpus", Some(head));

        b.add_resource(make_resource(
            "urn:ex:body",
            wk::PROPERTY,
            vec![
                (wk::SHORT_NAME, Value::String("body".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::STRING))),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            target_index_iri,
            wk::VECTOR_INDEX_CLASS,
            vec![
                (wk::TARGET_PROPERTY, Value::ResourceRef(iri("urn:ex:body"))),
                (wk::VEC_MODEL, Value::ResourceRef(iri(model_iri))),
                (wk::VEC_DIM, Value::Integer(8)),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:d1",
            "urn:ex:Document",
            vec![("urn:ex:body", Value::String("alpha beta gamma".into()))],
        ))
        .unwrap();
        Arc::new(b.build(storage))
    }

    #[test]
    fn reindex_completes_and_marks_record_completed() {
        let target = "urn:ex:vi_v2";
        let model = "urn:eigenius:embed:dummy:v1";
        let head = build_corpus(target, model);

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model, 8)));

        let record = TaskRecord::new_running(
            uuid::Uuid::nil(),
            uuid::Uuid::new_v4(),
            "urn:ex:reindex".to_string(),
            "urn:ex:input".to_string(),
            head.id().clone(),
            0,
        );
        let mut driver = ReindexDriver::new(iri(target)).with_record(record);

        let report = driver.run(&head, &reg, None).expect("reindex succeeds");
        assert!(report.total_subjects >= 1);
        assert_eq!(driver.record().unwrap().status, TaskStatus::Completed);

        // Segment written under the target index IRI at the head.
        let seg = head
            .storage()
            .vector_index
            .get_segment(&iri(target), head.id())
            .expect("segment lookup");
        assert!(seg.is_some(), "reindex must write a segment at the head");
    }

    #[test]
    fn reindex_unknown_target_marks_record_failed() {
        let model = "urn:eigenius:embed:dummy:v1";
        let head = build_corpus("urn:ex:vi_existing", model);

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model, 8)));

        let record = TaskRecord::new_running(
            uuid::Uuid::nil(),
            uuid::Uuid::new_v4(),
            "urn:ex:reindex".to_string(),
            "urn:ex:input".to_string(),
            head.id().clone(),
            0,
        );
        let mut driver = ReindexDriver::new(iri("urn:ex:vi_nonexistent")).with_record(record);

        let err = driver
            .run(&head, &reg, None)
            .expect_err("reindex against unknown target should fail");
        match err {
            SweepError::EmbedderNotRegistered { .. } => {}
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(driver.record().unwrap().status, TaskStatus::Failed);
    }

    /// Cancellation must propagate before the reindex starts when
    /// the flag is already set — this is the "cancel landed before
    /// scheduling" case the commit-hook integration relies on.
    #[test]
    fn pre_set_cancellation_short_circuits_reindex() {
        let target = "urn:ex:vi_v2";
        let model = "urn:eigenius:embed:dummy:v1";
        let head = build_corpus(target, model);

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model, 8)));

        let record = TaskRecord::new_running(
            uuid::Uuid::nil(),
            uuid::Uuid::new_v4(),
            "urn:ex:reindex".to_string(),
            "urn:ex:input".to_string(),
            head.id().clone(),
            0,
        );
        let mut driver = ReindexDriver::new(iri(target)).with_record(record);
        driver.cancel_handle().store(true, Ordering::SeqCst);

        let err = driver
            .run(&head, &reg, None)
            .expect_err("pre-set cancel must short-circuit");
        assert!(matches!(err, SweepError::Cancelled));
        assert_eq!(driver.record().unwrap().status, TaskStatus::Cancelled);
    }
}
