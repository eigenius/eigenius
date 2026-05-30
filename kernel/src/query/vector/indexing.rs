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

//! D43 §5.5 / M5.3 — vector-index population (sweep).
//!
//! Unlike text indexing — which is cheap, deterministic, and runs
//! synchronously inside `LayerBuilder::build` (`query::text::indexing`
//! / M3.5) — vector indexing requires an IO call to an Embedder
//! Component for every indexable string. D43 §5.5 commits to making
//! that work **asynchronous and non-gating**: a layer commits without
//! waiting on the embedder, and a separate post-Load sweep produces
//! the `vec_seg:<I>:<L>` segments later. The sweep is observable
//! through a D21 TaskRecord and cancellable via `delete_layer(L)`.
//!
//! For v1 the proper task infrastructure (in-flight cap, exponential
//! backoff, the TaskRecord surface) is deferred. This module ships
//! [`sweep_layer_vectors`] — the work-doer the eventual task will
//! invoke — so callers (tests today; the sweep task tomorrow) have a
//! single entry point that:
//!
//! 1. Discovers every active `core:VectorIndex` Resource at `head`.
//! 2. For each, walks `head.defined_iris()`, reads the target
//!    Property's string value off each defined Resource, dispatches
//!    the corresponding Embedder Component (cache-first), and batches
//!    the resulting `(subject, vector)` pairs.
//! 3. Verifies the Embedder's declared `dim` matches the VectorIndex
//!    Resource's `vec_dim` slot — a mismatch fails the sweep with
//!    [`SweepError::DimDeclarationMismatch`] rather than silently
//!    indexing with a model whose output shape disagrees with the
//!    Index's declared contract.
//! 4. Issues one [`VectorIndex::extend_layer`] call per Index whose
//!    contribution is non-empty.
//!
//! Returns a [`SweepReport`] summarising the work — subject count
//! per Index, cache-hit ratio, embedder-call count — for the TaskRecord
//! that will eventually consume it.

use crate::layer::{resolve_active_vector_indexes, ActiveVectorIndex, Layer, VectorDoc};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::program::embedder::{Embedder, EmbedderError, EmbedderRegistry};
use crate::program::embedding_cache::EmbeddingCache;
use crate::storage::StorageError;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Knobs controlling sweep execution. Pass via [`SweepOptions::default`]
/// for the M5 defaults (no retries, no cancellation).
#[derive(Debug, Clone)]
pub struct SweepOptions<'a> {
    /// Cooperative-cancellation flag. Checked between Resources
    /// within an Index and between Indexes. When the flag flips to
    /// `true`, the sweep returns
    /// [`SweepError::Cancelled`] after the next check; any segment
    /// fully embedded before the check is still written.
    pub cancellation: Option<&'a AtomicBool>,
    /// Maximum retry attempts on transient `EmbedderError::Io`
    /// failures per subject. `0` disables retries.
    pub max_retries: u32,
    /// Base backoff in milliseconds. The Nth retry sleeps
    /// `base * 2^N` before re-dispatching.
    pub retry_backoff_base_ms: u64,
}

impl Default for SweepOptions<'_> {
    fn default() -> Self {
        Self {
            cancellation: None,
            max_retries: 0,
            retry_backoff_base_ms: 100,
        }
    }
}

/// Per-Index summary of one sweep. Aggregated into a top-level
/// [`SweepReport`] so callers can correlate sweep outcomes with the
/// Index Resources that produced them.
#[derive(Debug, Default, Clone)]
pub struct IndexSweepStats {
    /// Number of `(subject, vector)` pairs written under this Index.
    pub subjects: usize,
    /// How many of those were served from the embedding cache.
    pub cache_hits: usize,
    /// How many invoked the Embedder Component (cache misses).
    pub embedder_calls: usize,
}

/// Top-level sweep summary, one entry per Index that participated.
#[derive(Debug, Default, Clone)]
pub struct SweepReport {
    /// Per-`VectorIndex Resource` stats.
    pub per_index: BTreeMap<Iri, IndexSweepStats>,
    /// Total subjects across all Indexes.
    pub total_subjects: usize,
    /// Number of `(VectorIndex, subject)` pairs that were silently
    /// skipped because the target property had no string-typed
    /// value on the Resource. Not an error — v1 vector indexing
    /// only covers string properties, mirroring the text-indexing
    /// `populate_text_indexes` contract.
    pub skipped: usize,
}

/// Errors that abort the sweep.
#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    /// The Embedder Component declared in the VectorIndex Resource's
    /// `vec_model` slot is not present in the registry. The sweep
    /// can't proceed without a way to produce vectors.
    #[error("VectorIndex `{index}` declares embedder model `{model}` but no Embedder is registered for it")]
    EmbedderNotRegistered { index: String, model: String },
    /// The Embedder dispatch failed (IO error, hosted-API rate
    /// limit, etc.). v1 propagates the first such error to the
    /// caller; the M5-followup sweep task will add per-doc retry
    /// + a configurable in-flight cap.
    #[error("Embedder dispatch failed for VectorIndex `{index}`, subject `{subject}`: {source}")]
    EmbedderDispatch {
        index: String,
        subject: String,
        #[source]
        source: EmbedderError,
    },
    /// The Embedder's declared `dim` doesn't match the VectorIndex
    /// Resource's `vec_dim` slot. v1 fails the whole sweep —
    /// indexing under a mismatched dim would produce segments the
    /// query path can't read.
    #[error(
        "VectorIndex `{index}` declares vec_dim={declared} but Embedder `{model}` declares dim={embedder}"
    )]
    DimDeclarationMismatch {
        index: String,
        model: String,
        declared: u32,
        embedder: u32,
    },
    /// Writing the segment to the [`crate::layer::VectorIndex`]
    /// backend failed.
    #[error("vector index storage error for index `{index}`: {source}")]
    Storage {
        index: String,
        #[source]
        source: StorageError,
    },
    /// The cooperative-cancellation flag was raised. The sweep
    /// stopped before completing; any segment fully written before
    /// the cancellation check remains in the index.
    #[error("sweep cancelled before completion")]
    Cancelled,
}

/// Walk `layer`'s defined Resources, embed every indexable property
/// value via the configured Embedder, and write the resulting
/// segments into `layer.storage().vector_index`.
///
/// See module docs for the full contract. Equivalent to
/// [`sweep_layer_vectors_with_options`] with [`SweepOptions::default`].
pub fn sweep_layer_vectors(
    layer: &Layer,
    embedders: &EmbedderRegistry,
    cache: Option<&EmbeddingCache>,
) -> Result<SweepReport, SweepError> {
    sweep_layer_vectors_with_options(layer, embedders, cache, &SweepOptions::default())
}

/// Configurable sweep entry point. M5.8 callers (the post-Load
/// sweep task — `crate::task::sweep::VectorSweepDriver`) supply
/// custom [`SweepOptions`] to enable retries on transient embedder
/// failures and to expose a cooperative-cancellation flag.
pub fn sweep_layer_vectors_with_options(
    layer: &Layer,
    embedders: &EmbedderRegistry,
    cache: Option<&EmbeddingCache>,
    options: &SweepOptions<'_>,
) -> Result<SweepReport, SweepError> {
    let active = resolve_active_vector_indexes(layer);
    if active.is_empty() {
        return Ok(SweepReport::default());
    }

    let mut report = SweepReport::default();
    for index in &active {
        if is_cancelled(options.cancellation) {
            return Err(SweepError::Cancelled);
        }
        let stats = sweep_one_index(layer, index, embedders, cache, options, &mut report.skipped)?;
        report.total_subjects += stats.subjects;
        report.per_index.insert(index.iri.clone(), stats);
    }
    Ok(report)
}

fn is_cancelled(token: Option<&AtomicBool>) -> bool {
    matches!(token, Some(b) if b.load(Ordering::SeqCst))
}

/// Dispatch the embedder with optional retry-on-`Io` backoff. Only
/// [`EmbedderError::Io`] is retried — `InvalidInput` is a permanent
/// failure (the input isn't going to suddenly become tokenisable).
/// Sleeps via `std::thread::sleep` since the sync sweep doesn't have
/// an async runtime; the M5 follow-up that makes the sweep async
/// (per D43 §5.5 "per-orchestrator in-flight Embedder-call limit
/// (default ~64)") replaces this with `tokio::time::sleep`.
fn embed_with_retry(
    embedder: &dyn Embedder,
    text: &str,
    options: &SweepOptions<'_>,
) -> Result<Vec<f32>, EmbedderError> {
    let mut attempt: u32 = 0;
    loop {
        match embedder.embed(text) {
            Ok(v) => return Ok(v),
            Err(EmbedderError::Io(msg)) if attempt < options.max_retries => {
                let backoff_ms = options
                    .retry_backoff_base_ms
                    .saturating_mul(1u64 << attempt);
                std::thread::sleep(Duration::from_millis(backoff_ms));
                attempt += 1;
                let _ = msg; // suppress unused-binding lint without
                             // committing to a particular log shape;
                             // tracing wiring is the M5.8 follow-up.
            }
            Err(e) => return Err(e),
        }
    }
}

/// Public helper exposed so the sweep task driver (`task/sweep.rs`)
/// can flip the cancellation flag without forking the sweep API.
pub fn make_cancellation_token() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// Sweep one `(layer, VectorIndex Resource)` pair. Pulled out so
/// per-index errors carry enough context for the [`SweepError`]
/// constructors.
fn sweep_one_index(
    layer: &Layer,
    index: &ActiveVectorIndex,
    embedders: &EmbedderRegistry,
    cache: Option<&EmbeddingCache>,
    options: &SweepOptions<'_>,
    skipped: &mut usize,
) -> Result<IndexSweepStats, SweepError> {
    let embedder =
        embedders
            .get(&index.model)
            .ok_or_else(|| SweepError::EmbedderNotRegistered {
                index: index.iri.as_str().to_string(),
                model: index.model.as_str().to_string(),
            })?;
    if embedder.dim() != index.dim {
        return Err(SweepError::DimDeclarationMismatch {
            index: index.iri.as_str().to_string(),
            model: index.model.as_str().to_string(),
            declared: index.dim,
            embedder: embedder.dim(),
        });
    }
    let metric_short = match index.distance.as_str() {
        "urn:eigenius:core:distances:cosine" => "cosine",
        "urn:eigenius:core:distances:l2" => "l2",
        "urn:eigenius:core:distances:dot" => "dot",
        other => other,
    }
    .to_string();

    // Buffer subject-IRI / vector pairs so we can issue one
    // `extend_layer` call at the end (matches the M2 trait's
    // batched-write contract).
    let mut owned_subjects: Vec<Iri> = Vec::new();
    let mut owned_vectors: Vec<Vec<f32>> = Vec::new();
    let mut stats = IndexSweepStats::default();

    for subject_iri in layer.defined_iris().iter() {
        if is_cancelled(options.cancellation) {
            return Err(SweepError::Cancelled);
        }
        let resource = match layer.get_resource(subject_iri) {
            Some(r) => r,
            None => continue,
        };
        let value = match resource.get(&index.target_property) {
            Some(v) => v,
            None => continue,
        };
        let text = match value {
            Value::String(s) => s.as_str(),
            _ => {
                *skipped += 1;
                continue;
            }
        };

        // Cache-first dispatch — mirrors the EMBED evaluator
        // ([`crate::query::evaluate::expression::eval_embed`]) so
        // index-side and query-side embeds share the same cache
        // entries (D43 §5.1 cross-path reuse).
        let vector = if let Some(c) = cache {
            if let Some(cached) = c.get(text, &index.model) {
                stats.cache_hits += 1;
                (*cached).clone()
            } else {
                stats.embedder_calls += 1;
                let v = embed_with_retry(embedder.as_ref(), text, options).map_err(|e| {
                    SweepError::EmbedderDispatch {
                        index: index.iri.as_str().to_string(),
                        subject: subject_iri.as_str().to_string(),
                        source: e,
                    }
                })?;
                c.insert(text, &index.model, std::sync::Arc::new(v.clone()));
                v
            }
        } else {
            stats.embedder_calls += 1;
            embed_with_retry(embedder.as_ref(), text, options).map_err(|e| {
                SweepError::EmbedderDispatch {
                    index: index.iri.as_str().to_string(),
                    subject: subject_iri.as_str().to_string(),
                    source: e,
                }
            })?
        };

        owned_subjects.push(subject_iri.clone());
        owned_vectors.push(vector);
    }

    stats.subjects = owned_subjects.len();
    if owned_subjects.is_empty() {
        return Ok(stats);
    }

    let docs: Vec<VectorDoc<'_>> = owned_subjects
        .iter()
        .zip(owned_vectors.iter())
        .map(|(s, v)| VectorDoc {
            subject: s,
            vector: v.as_slice(),
        })
        .collect();
    layer
        .storage()
        .vector_index
        .extend_layer(
            &index.iri,
            layer.id(),
            &index.model,
            index.dim,
            &metric_short,
            &docs,
        )
        .map_err(|e| SweepError::Storage {
            index: index.iri.as_str().to_string(),
            source: e,
        })?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::Resource;
    use crate::ontology::well_known as wk;
    use crate::program::embedder::DummyEmbedder;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a layer chain: bootstrap + a child layer declaring a
    /// string Property, a `core:VectorIndex` Resource targeting it,
    /// and `n_docs` Documents whose `body` is `"text {i}"`.
    fn build_corpus(
        target_prop: &str,
        model_iri: &str,
        dim: u32,
        n_docs: usize,
    ) -> Arc<crate::layer::Layer> {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("vec-corpus", Some(parent));

        // Target Property.
        let mut prop = Resource::new(iri(target_prop));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::SHORT_NAME), Value::String("body".into()));
        prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        b.add_resource(prop).unwrap();

        // VectorIndex Resource.
        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_prop)),
        );
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_iri)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(dim as i64));
        b.add_resource(vi).unwrap();

        // Document Resources.
        for i in 0..n_docs {
            let mut d = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
            d.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
            );
            d.set(iri(target_prop), Value::String(format!("text {i}")));
            b.add_resource(d).unwrap();
        }

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn sweep_writes_one_segment_per_index_with_expected_subject_count() {
        let layer = build_corpus(
            "urn:eigenius:test:body",
            "urn:eigenius:embed:dummy:v1",
            8,
            3,
        );
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(
            "urn:eigenius:embed:dummy:v1",
            8,
        )));
        let report = sweep_layer_vectors(&layer, &reg, None).expect("sweep");
        assert_eq!(report.total_subjects, 3);
        assert_eq!(report.per_index.len(), 1);

        // Segment is queryable through the layer's storage.
        let segment = layer
            .storage()
            .vector_index
            .get_segment(&iri("urn:eigenius:test:vi"), layer.id())
            .expect("storage")
            .expect("segment was written");
        assert_eq!(segment.count(), 3);
        assert_eq!(segment.dim, 8);
        assert_eq!(segment.model_iri.as_str(), "urn:eigenius:embed:dummy:v1");
        assert_eq!(segment.distance, "cosine"); // default distance
    }

    #[test]
    fn sweep_uses_cache_to_avoid_redundant_dispatch() {
        // Two Resources whose body string is identical produce one
        // embedder call when the cache is present.
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("cache-corpus", Some(parent));

        let prop_iri = "urn:eigenius:test:body";
        let mut prop = Resource::new(iri(prop_iri));
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
        vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(prop_iri)));
        vi.set(
            iri(wk::VEC_MODEL),
            Value::ResourceRef(iri("urn:eigenius:embed:dummy:v1")),
        );
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        b.add_resource(vi).unwrap();

        for i in 0..3 {
            let mut d = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
            d.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Doc"))]),
            );
            // Same content in all three docs.
            d.set(iri(prop_iri), Value::String("identical text".into()));
            b.add_resource(d).unwrap();
        }
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(
            "urn:eigenius:embed:dummy:v1",
            8,
        )));
        let cache = EmbeddingCache::new(16);

        let report = sweep_layer_vectors(&layer, &reg, Some(&cache)).expect("sweep");
        let stats = report
            .per_index
            .get(&iri("urn:eigenius:test:vi"))
            .expect("stats");
        assert_eq!(stats.subjects, 3);
        assert_eq!(stats.embedder_calls, 1);
        assert_eq!(stats.cache_hits, 2);
    }

    #[test]
    fn sweep_skips_non_string_property_values() {
        // The target property exists on the Resource but the value
        // is an Integer. The sweep skips silently, returning a
        // `skipped` counter.
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("skip-corpus", Some(parent));

        let prop_iri = "urn:eigenius:test:numeric";
        let mut prop = Resource::new(iri(prop_iri));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(
            iri(wk::DATA_TYPE_PROP),
            Value::ResourceRef(iri(wk::INTEGER)),
        );
        b.add_resource(prop).unwrap();

        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(prop_iri)));
        vi.set(
            iri(wk::VEC_MODEL),
            Value::ResourceRef(iri("urn:eigenius:embed:dummy:v1")),
        );
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        b.add_resource(vi).unwrap();

        let mut d = Resource::new(iri("urn:eigenius:test:d"));
        d.set(iri(prop_iri), Value::Integer(42));
        b.add_resource(d).unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(
            "urn:eigenius:embed:dummy:v1",
            8,
        )));
        let report = sweep_layer_vectors(&layer, &reg, None).expect("sweep");
        assert_eq!(report.total_subjects, 0);
        assert_eq!(report.skipped, 1);
    }

    #[test]
    fn sweep_fails_on_dim_declaration_mismatch() {
        let layer = build_corpus(
            "urn:eigenius:test:body",
            "urn:eigenius:embed:dummy:v1",
            // Declare vec_dim=16 but the Embedder produces 8.
            16,
            1,
        );
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(
            "urn:eigenius:embed:dummy:v1",
            8,
        )));
        let err = sweep_layer_vectors(&layer, &reg, None).unwrap_err();
        assert!(
            matches!(err, SweepError::DimDeclarationMismatch { .. }),
            "expected DimDeclarationMismatch; got {err:?}"
        );
    }

    #[test]
    fn sweep_fails_when_embedder_not_registered() {
        let layer = build_corpus(
            "urn:eigenius:test:body",
            "urn:eigenius:embed:dummy:v1",
            8,
            1,
        );
        // Empty registry.
        let reg = EmbedderRegistry::new();
        let err = sweep_layer_vectors(&layer, &reg, None).unwrap_err();
        assert!(
            matches!(err, SweepError::EmbedderNotRegistered { .. }),
            "expected EmbedderNotRegistered; got {err:?}"
        );
    }

    #[test]
    fn sweep_with_no_active_indexes_is_noop() {
        // Layer with no VectorIndex Resource — sweep does nothing.
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("empty", Some(parent));
        b.add_resource(Resource::new(iri("urn:eigenius:test:placeholder")))
            .unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let reg = EmbedderRegistry::new();
        let report = sweep_layer_vectors(&layer, &reg, None).expect("sweep");
        assert!(report.per_index.is_empty());
        assert_eq!(report.total_subjects, 0);
    }
}
