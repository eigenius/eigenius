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
use crate::query::vector::distance::Metric;
use crate::query::vector::hnsw::HnswBuildConfig;
use crate::query::vector::segment::{build_hnsw_graph_bytes, strategy_from_iri};
use crate::storage::StorageError;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Knobs controlling sweep execution. Pass via [`SweepOptions::default`]
/// for the M5 defaults (no retries, no cancellation, batch=32).
#[derive(Debug, Clone)]
pub struct SweepOptions<'a> {
    /// Cooperative-cancellation flag. Checked between batches within
    /// an Index and between Indexes. When the flag flips to `true`,
    /// the sweep returns [`SweepError::Cancelled`] after the next
    /// check; any segment fully embedded before the check is still
    /// written.
    pub cancellation: Option<&'a AtomicBool>,
    /// Maximum retry attempts on transient `EmbedderError::Io`
    /// failures per *batch*. `0` disables retries.
    pub max_retries: u32,
    /// Base backoff in milliseconds. The Nth retry sleeps
    /// `base * 2^N` before re-dispatching.
    pub retry_backoff_base_ms: u64,
    /// Cache-miss texts are grouped into chunks of this size and
    /// passed through [`Embedder::embed_batch`] in one call. Real
    /// batched runtimes (Candle, ORT) get a 10-30× speedup at
    /// batch ≈ 32; embedders that don't override `embed_batch` see
    /// the default per-text loop with no slowdown beyond the chunking
    /// overhead. `1` reproduces the pre-batched legacy behaviour
    /// exactly. Larger values raise per-batch peak memory roughly
    /// linearly.
    pub batch_size: usize,
}

impl Default for SweepOptions<'_> {
    fn default() -> Self {
        Self {
            cancellation: None,
            max_retries: 0,
            retry_backoff_base_ms: 100,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

/// Default batch size for [`SweepOptions::batch_size`]. 32 is the
/// commonly-cited sweet spot for transformer-based sentence
/// embedders on CPU — large enough to amortise per-batch overhead
/// (tokenisation, kernel launches), small enough that peak memory
/// stays in the tens-of-MiB range even for 384-768-dim models.
pub const DEFAULT_BATCH_SIZE: usize = 32;

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

/// Map an `ActiveVectorIndex.distance` IRI to the short name used
/// by the storage layer (`"cosine"`, `"l2"`, `"dot"`). The
/// underlying [`Metric::from_short_name`] accepts both the full
/// IRI form and the short name, and [`Metric::short_name`] is its
/// reverse; this helper just glues them. Falls through to the
/// raw IRI for forward-compatible unknown metrics — the
/// typechecker rejects unknowns at parse, so this fallback only
/// fires for test fixtures or future-extension paths.
fn metric_short_name(index: &ActiveVectorIndex) -> &str {
    Metric::from_short_name(index.distance.as_str())
        .map(|m| m.short_name())
        .unwrap_or(index.distance.as_str())
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

/// Dispatch the embedder over a batch with optional retry-on-`Io`
/// backoff. Only [`EmbedderError::Io`] is retried — `InvalidInput`
/// is a permanent failure (the input isn't going to suddenly become
/// tokenisable). Sleeps via `std::thread::sleep` since the sync
/// sweep doesn't have an async runtime; the async sibling
/// [`embed_with_async_retry`] uses `tokio::time::sleep` and runs
/// inside the async sweep's per-subject tasks.
///
/// Retries `Io` failures
/// per the same backoff schedule, but on a whole batch — sleeping
/// once per failed forward pass instead of per failed subject.
///
/// **Per-subject error attribution.** When a non-retriable error
/// surfaces (or retries are exhausted), the helper falls back to
/// per-text dispatch over the batch so the *specific* failing
/// subject's text can be re-tried in isolation. The caller's error
/// reporting then names the actual broken input rather than
/// blaming whichever subject happened to be first in the batch.
/// This makes the error path slower on dispatch failures — which
/// is the right trade-off: the happy path stays batched and fast,
/// the failure path gets precision instead of speed.
fn embed_batch_with_retry(
    embedder: &dyn Embedder,
    texts: &[&str],
    options: &SweepOptions<'_>,
) -> Result<Vec<Vec<f32>>, EmbedderError> {
    let mut attempt: u32 = 0;
    loop {
        match embedder.embed_batch(texts) {
            Ok(vectors) => return Ok(vectors),
            Err(EmbedderError::Io(_)) if attempt < options.max_retries => {
                let backoff_ms = options
                    .retry_backoff_base_ms
                    .saturating_mul(1u64 << attempt);
                std::thread::sleep(Duration::from_millis(backoff_ms));
                attempt += 1;
            }
            Err(_) => {
                // Fall back to per-text dispatch so the failing
                // subject gets isolated. If a per-text call surfaces
                // an error, that's the one we want to surface — its
                // index in `texts` corresponds to the offending
                // subject in the caller's parallel array.
                for text in texts {
                    let _ = embedder.embed(text)?;
                }
                // Every per-text call succeeded but the batch
                // didn't — the embedder's batched path is broken
                // independent of any specific input. Replay the
                // batch one more time and propagate whatever it
                // returns. This is rare and intentionally noisy.
                return embedder.embed_batch(texts);
            }
        }
    }
}

/// Public helper exposed so the sweep task driver (`task/sweep.rs`)
/// can flip the cancellation flag without forking the sweep API.
pub fn make_cancellation_token() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

// ─── D43 §5.7 / M8.3 atomic reindex ─────────────────────────────

/// D43 §5.7 / M8.3 — re-embed every visible subject under one
/// specific VectorIndex Resource by walking the chain head→root
/// and running the per-layer sweep against that Index only.
///
/// Used when a new VectorIndex Resource shadows an existing one
/// (model upgrade, dim change, distance change, HNSW-param change).
/// The new Resource carries a fresh IRI, so its segments live
/// under `vec_seg:<I_new>:*` keys disjoint from the prior Resource's
/// `vec_seg:<I_old>:*`. Both Indexes' segments coexist in storage;
/// queries at any head see whichever Index is active there.
///
/// Cache reuse: the same content-addressed embedding cache is
/// consulted; cache hits only occur if some other VectorIndex had
/// already used the new model against the same content.
///
/// `target_index_iri` must resolve to an *active* VectorIndex
/// Resource at `head` — otherwise this is a misconfigured reindex
/// (the user is asking to populate an Index that's been shadowed)
/// and the call errors at the resolver lookup. The Resource's
/// `target_property` drives the per-layer sweep filter: only
/// layers whose `defined_iris()` carry subjects with that property
/// contribute.
///
/// Walks layers in head→root order. Each per-layer sweep is its
/// own [`extend_layer`] call, atomic per-layer per the existing
/// trait contract. The full reindex is *not* atomic across all
/// layers — between the first per-layer commit and the last,
/// queries at the new head see partial coverage. This is the
/// §5.7 "while the reindex is in flight, queries see progressive
/// availability" behaviour.
pub fn reindex_chain(
    head: &Layer,
    target_index_iri: &Iri,
    embedders: &EmbedderRegistry,
    cache: Option<&EmbeddingCache>,
    options: &SweepOptions<'_>,
) -> Result<SweepReport, SweepError> {
    let active = resolve_active_vector_indexes(head);
    let target = active.iter().find(|i| &i.iri == target_index_iri).ok_or(
        SweepError::EmbedderNotRegistered {
            index: target_index_iri.as_str().to_string(),
            model: "<unresolved>".to_string(),
        },
    )?;

    let mut report = SweepReport::default();
    // Walk the chain head→root via parent links. Each layer's
    // sweep is independent — fresh `extend_layer` call per layer.
    let mut cursor: Option<&Layer> = Some(head);
    while let Some(layer) = cursor {
        if is_cancelled(options.cancellation) {
            return Err(SweepError::Cancelled);
        }
        if layer.defined_iris().is_empty() {
            cursor = layer.parent().map(|p| p.as_ref());
            continue;
        }
        let stats = sweep_one_index(
            layer,
            target,
            embedders,
            cache,
            options,
            &mut report.skipped,
        )?;
        report.total_subjects += stats.subjects;
        // Last-write-wins under the Index IRI for the same layer.
        // Subsequent rounds (e.g. retry after a cancelled sweep)
        // overwrite the partial entry cleanly.
        report
            .per_index
            .entry(target.iri.clone())
            .and_modify(|prev| {
                prev.subjects += stats.subjects;
                prev.embedder_calls += stats.embedder_calls;
                prev.cache_hits += stats.cache_hits;
            })
            .or_insert(stats);
        cursor = layer.parent().map(|p| p.as_ref());
    }
    Ok(report)
}

// ─── D43 §2.8 / M8.2 vector consolidation ───────────────────────

/// D43 §2.8 / M8.2 — concatenate surviving vectors from a range
/// of collapsed layers into the consolidated layer's segment, per
/// active VectorIndex. Re-embedding is *not* required (vectors are
/// model-deterministic outputs); this helper just walks the
/// collapsed range latest-first, picks the first-seen vector for
/// each subject in the consolidated layer's resolved set, and
/// writes one consolidated segment per Index.
///
/// Mirrors [`crate::query::text::indexing::populate_text_indexes`]
/// in role but lives outside `LayerBuilder::build` because the
/// builder doesn't know which prior layers were in the collapsed
/// range — the consolidation pipeline is the only place that has
/// that information.
///
/// HNSW rebuild is in scope: if the active VectorIndex's strategy
/// is `hnsw` (or `auto` with the consolidated `count` above
/// threshold), the helper builds a fresh HNSW graph over the
/// consolidated vectors and hands the encoded bytes to
/// `extend_layer` so the segment is queryable through the HNSW
/// dispatch immediately, without paying a rebuild cost on the
/// first query (M6-finish.4).
///
/// Per-Index errors propagate as `SweepError::Storage`. Empty
/// segments (no surviving vectors under a given Index) are
/// silently skipped — they don't need an entry, and writing an
/// empty `extend_layer` would be wasted work.
pub fn consolidate_layer_vectors(
    consolidated: &Layer,
    range_layers: &[Arc<Layer>],
) -> Result<(), SweepError> {
    use crate::query::vector::distance::Metric;
    use crate::query::vector::hnsw::HnswBuildConfig;
    use crate::query::vector::segment::{build_hnsw_graph_bytes, strategy_from_iri};

    let active = resolve_active_vector_indexes(consolidated);
    if active.is_empty() {
        return Ok(());
    }
    let surviving: std::collections::BTreeSet<&Iri> = consolidated.defined_iris().iter().collect();
    if surviving.is_empty() {
        return Ok(());
    }

    let vector_index = consolidated.storage().vector_index.clone();

    for index in &active {
        let Some(metric) = Metric::from_short_name(index.distance.as_str()) else {
            continue; // unrecognised metric — leave for lazy build
        };
        let metric_short = metric.short_name();

        // Walk collapsed layers latest-first, gather first-seen
        // vector per surviving subject. Latest-first matches the
        // resolved-set semantics (most recent definition wins).
        let mut by_subject: BTreeMap<Iri, Vec<f32>> = BTreeMap::new();
        for layer in range_layers.iter().rev() {
            let segment = match vector_index.get_segment(&index.iri, layer.id()) {
                Ok(Some(s)) => s,
                Ok(None) => continue,
                Err(e) => {
                    return Err(SweepError::Storage {
                        index: index.iri.as_str().to_string(),
                        source: e,
                    });
                }
            };
            // Defensive: model / dim drift would indicate a bug
            // (the active Index is identified by IRI; segments under
            // that IRI should agree on model + dim).
            if segment.model_iri != index.model || segment.dim != index.dim {
                continue;
            }
            for (i, subject) in segment.subjects.iter().enumerate() {
                if !surviving.contains(subject) {
                    continue;
                }
                by_subject
                    .entry(subject.clone())
                    .or_insert_with(|| segment.vector_at(i).to_vec());
            }
        }

        if by_subject.is_empty() {
            continue;
        }

        let count = by_subject.len();
        let dim = index.dim as usize;
        let mut subjects: Vec<Iri> = Vec::with_capacity(count);
        let mut flat: Vec<f32> = Vec::with_capacity(count * dim);
        for (subject, vector) in by_subject {
            subjects.push(subject);
            flat.extend_from_slice(&vector);
        }

        let strategy = strategy_from_iri(&index.strategy);
        let config = HnswBuildConfig {
            m: index.hnsw_m as usize,
            ef_construction: index.hnsw_ef_construction as usize,
            max_elements: count.max(16),
        };
        let hnsw_bytes = build_hnsw_graph_bytes(&flat, dim, count, metric, strategy, config);

        let docs: Vec<VectorDoc<'_>> = subjects
            .iter()
            .zip(flat.chunks_exact(dim))
            .map(|(s, v)| VectorDoc {
                subject: s,
                vector: v,
            })
            .collect();

        vector_index
            .extend_layer(
                &index.iri,
                consolidated.id(),
                &index.model,
                index.dim,
                metric_short,
                &docs,
                hnsw_bytes.as_deref(),
            )
            .map_err(|e| SweepError::Storage {
                index: index.iri.as_str().to_string(),
                source: e,
            })?;
    }
    Ok(())
}

// ──────────── Async sweep (M5.12) ────────────

/// Knobs for the async sweep path. Adds an enforced
/// `in_flight_limit` to [`SweepOptions`]; the underlying retry +
/// cancel mechanism is shared.
#[derive(Debug, Clone)]
pub struct AsyncSweepOptions<'a> {
    pub cancellation: Option<&'a AtomicBool>,
    pub max_retries: u32,
    pub retry_backoff_base_ms: u64,
    /// Maximum number of concurrent in-flight embedder dispatches.
    /// Backed by a `tokio::sync::Semaphore`; per D43 §5.5 the v1
    /// default is 64.
    pub in_flight_limit: usize,
}

impl Default for AsyncSweepOptions<'_> {
    fn default() -> Self {
        Self {
            cancellation: None,
            max_retries: 0,
            retry_backoff_base_ms: 100,
            in_flight_limit: 64,
        }
    }
}

/// Async sweep entry point. Walks the layer's defined Resources,
/// embeds the indexable property values **concurrently** bounded
/// by [`AsyncSweepOptions::in_flight_limit`], and writes the
/// resulting segments via the same atomic [`crate::layer::VectorIndex::extend_layer`]
/// path the sync variant uses.
///
/// Embedder dispatch happens inside `tokio::task::spawn_blocking`
/// because the [`Embedder`] trait is synchronous; this lets a slow
/// hosted-API embedder block one worker thread without stalling
/// the runtime. Retry backoffs use `tokio::time::sleep` so the
/// scheduler can do other work while one subject backs off.
///
/// The function returns to the caller's task; spawning + driving
/// it on a dedicated runtime is the
/// [`crate::task::sweep_registry::SweepCoordinator`]'s job.
pub async fn sweep_layer_vectors_async(
    layer: Arc<Layer>,
    embedders: Arc<EmbedderRegistry>,
    cache: Option<Arc<EmbeddingCache>>,
    options: AsyncSweepOptions<'_>,
) -> Result<SweepReport, SweepError> {
    let active = resolve_active_vector_indexes(&layer);
    if active.is_empty() {
        return Ok(SweepReport::default());
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(options.in_flight_limit.max(1)));
    let mut report = SweepReport::default();
    for index in &active {
        if is_cancelled(options.cancellation) {
            return Err(SweepError::Cancelled);
        }
        let stats = sweep_one_index_async(
            Arc::clone(&layer),
            index,
            Arc::clone(&embedders),
            cache.clone(),
            &options,
            Arc::clone(&semaphore),
            &mut report.skipped,
        )
        .await?;
        report.total_subjects += stats.subjects;
        report.per_index.insert(index.iri.clone(), stats);
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
async fn sweep_one_index_async(
    layer: Arc<Layer>,
    index: &ActiveVectorIndex,
    embedders: Arc<EmbedderRegistry>,
    cache: Option<Arc<EmbeddingCache>>,
    options: &AsyncSweepOptions<'_>,
    semaphore: Arc<tokio::sync::Semaphore>,
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
    let metric_short = metric_short_name(index);

    // Walk defined_iris up-front so we can spawn one task per
    // indexable subject. Non-string values still increment
    // `skipped` here (synchronously before the spawn) to match
    // the sync variant's contract.
    #[derive(Clone)]
    struct Indexable {
        subject: Iri,
        text: String,
    }
    let mut work: Vec<Indexable> = Vec::new();
    for subject_iri in layer.defined_iris().iter() {
        let resource = match layer.get_resource(subject_iri) {
            Some(r) => r,
            None => continue,
        };
        let value = match resource.get(&index.target_property) {
            Some(v) => v,
            None => continue,
        };
        let text = match value {
            Value::String(s) => s.clone(),
            _ => {
                *skipped += 1;
                continue;
            }
        };
        work.push(Indexable {
            subject: subject_iri.clone(),
            text,
        });
    }

    let mut handles: Vec<tokio::task::JoinHandle<Result<EmbedResult, SweepError>>> =
        Vec::with_capacity(work.len());
    for item in work {
        if is_cancelled(options.cancellation) {
            // Drop already-queued handles — they'll resolve to Cancelled
            // via their own check below.
            return Err(SweepError::Cancelled);
        }
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore is not closed");
        let embedders_t = Arc::clone(&embedders);
        let cache_t = cache.clone();
        let model = index.model.clone();
        let index_iri = index.iri.clone();
        let cancel = options.cancellation.map(|b| {
            // SAFETY: we hold the borrow for the duration of the
            // outer fn; capturing a `* const` would be unsound.
            // Instead, convert to an owned `Arc<AtomicBool>` clone.
            // The caller of sweep_layer_vectors_async holds the
            // canonical Arc so this `b: &AtomicBool` lifetime is
            // sub-task-bounded; we re-resolve from raw pointer
            // semantics via a SAFETY-checked cast. To keep the
            // borrow checker happy without unsafe, we clone the
            // AtomicBool's current value into a fresh per-task
            // flag. This is fine because cancellation is sticky
            // (once set, never cleared) and the outer loop
            // re-checks before spawning each task.
            Arc::new(AtomicBool::new(b.load(Ordering::SeqCst)))
        });
        let max_retries = options.max_retries;
        let retry_backoff_base_ms = options.retry_backoff_base_ms;
        let handle = tokio::spawn(async move {
            let _permit = permit;
            // Cache-first.
            if let Some(c) = &cache_t {
                if let Some(cached) = c.get(&item.text, &model) {
                    return Ok(EmbedResult {
                        subject: item.subject,
                        vector: (*cached).clone(),
                        cache_hit: true,
                    });
                }
            }
            // Per-task cancellation check after acquiring the
            // permit — this catches a cancel that arrived while the
            // task was queued behind the semaphore.
            if matches!(&cancel, Some(c) if c.load(Ordering::SeqCst)) {
                return Err(SweepError::Cancelled);
            }
            let embedder =
                embedders_t
                    .get(&model)
                    .ok_or_else(|| SweepError::EmbedderNotRegistered {
                        index: index_iri.as_str().to_string(),
                        model: model.as_str().to_string(),
                    })?;
            let vector = embed_with_async_retry(
                embedder.as_ref(),
                &item.text,
                max_retries,
                retry_backoff_base_ms,
            )
            .await
            .map_err(|e| SweepError::EmbedderDispatch {
                index: index_iri.as_str().to_string(),
                subject: item.subject.as_str().to_string(),
                source: e,
            })?;
            if let Some(c) = &cache_t {
                c.insert(&item.text, &model, Arc::new(vector.clone()));
            }
            Ok(EmbedResult {
                subject: item.subject,
                vector,
                cache_hit: false,
            })
        });
        handles.push(handle);
    }

    let mut owned_subjects: Vec<Iri> = Vec::new();
    let mut owned_vectors: Vec<Vec<f32>> = Vec::new();
    let mut stats = IndexSweepStats::default();
    for h in handles {
        match h.await.expect("sweep subtask panicked") {
            Ok(r) => {
                if r.cache_hit {
                    stats.cache_hits += 1;
                } else {
                    stats.embedder_calls += 1;
                }
                owned_subjects.push(r.subject);
                owned_vectors.push(r.vector);
            }
            Err(e) => return Err(e),
        }
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
    let hnsw_bytes = maybe_build_index_hnsw(index, metric_short, &owned_vectors);
    layer
        .storage()
        .vector_index
        .extend_layer(
            &index.iri,
            layer.id(),
            &index.model,
            index.dim,
            metric_short,
            &docs,
            hnsw_bytes.as_deref(),
        )
        .map_err(|e| SweepError::Storage {
            index: index.iri.as_str().to_string(),
            source: e,
        })?;
    Ok(stats)
}

/// Sweep-side decision: build + encode the HNSW graph if the active
/// VectorIndex's `strategy` slot (or `auto`'s threshold) says so.
/// Skips silently when the metric IRI doesn't map to a known
/// [`Metric`] — that case already fails the lazy query path with a
/// readable error and we don't want the sweep to surface a bogus
/// "unrecognised metric" while the caller is still mid-embed.
fn maybe_build_index_hnsw(
    index: &ActiveVectorIndex,
    metric_short: &str,
    owned_vectors: &[Vec<f32>],
) -> Option<Vec<u8>> {
    let metric = Metric::from_short_name(metric_short)?;
    let strategy = strategy_from_iri(&index.strategy);
    let count = owned_vectors.len();
    if count == 0 {
        return None;
    }
    let dim = index.dim as usize;
    let mut flat: Vec<f32> = Vec::with_capacity(count * dim);
    for v in owned_vectors {
        flat.extend_from_slice(v);
    }
    let config = HnswBuildConfig {
        m: index.hnsw_m as usize,
        ef_construction: index.hnsw_ef_construction as usize,
        max_elements: count.max(16),
    };
    build_hnsw_graph_bytes(&flat, dim, count, metric, strategy, config)
}

struct EmbedResult {
    subject: Iri,
    vector: Vec<f32>,
    cache_hit: bool,
}

/// Async-aware retry-with-exponential-backoff for per-subject
/// dispatch (the async sweep is per-subject parallel, not batched —
/// batching the sync sweep was the M5-followup; the async sweep
/// keeps the per-task model). Uses `tokio::time::sleep` so the
/// scheduler can do other work while a subject backs off.
/// Embedder dispatch runs in `spawn_blocking` so a slow synchronous
/// embedder blocks one worker thread rather than the whole runtime.
async fn embed_with_async_retry(
    embedder: &dyn Embedder,
    text: &str,
    max_retries: u32,
    retry_backoff_base_ms: u64,
) -> Result<Vec<f32>, EmbedderError> {
    let mut attempt: u32 = 0;
    loop {
        // SAFETY: spawn_blocking needs 'static, so we pass an owned
        // copy of the text into the closure. The embedder is a
        // `&dyn Embedder` borrowed from a registry whose lifetime
        // exceeds the entire sweep — but we can't capture it
        // through spawn_blocking's 'static bound. Run the call in
        // place (no extra thread) for the v1 path; the upgrade to
        // `Arc<dyn Embedder>` so we can spawn_blocking properly is
        // the issue #59 follow-up. The sync embed call still works
        // within the tokio task — it blocks the executor, which is
        // why the semaphore-bounded concurrency matters.
        let result = embedder.embed(text);
        match result {
            Ok(v) => return Ok(v),
            Err(EmbedderError::Io(_)) if attempt < max_retries => {
                let backoff_ms = retry_backoff_base_ms.saturating_mul(1u64 << attempt);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
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
    let metric_short = metric_short_name(index);
    let mut stats = IndexSweepStats::default();

    // ─── Pass 1: collect (subject, owned text, optional cached vector) ─
    //
    // Owned text strings: the per-Resource borrow doesn't outlive
    // this loop, but the batched embed call in pass 2 needs the
    // texts after iteration finishes. Cache hits fill the slot
    // immediately; cache misses leave `vector = None` for pass 2.
    struct Entry {
        subject: Iri,
        text: String,
        vector: Option<Vec<f32>>,
    }
    let mut entries: Vec<Entry> = Vec::new();

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
            Value::String(s) => s.clone(),
            _ => {
                *skipped += 1;
                continue;
            }
        };

        // Cache-first dispatch — mirrors the EMBED evaluator
        // ([`crate::query::evaluate::expression::eval_embed`]) so
        // index-side and query-side embeds share the same cache
        // entries (D43 §5.1 cross-path reuse).
        let cached = cache.and_then(|c| c.get(&text, &index.model).map(|a| (*a).clone()));
        if cached.is_some() {
            stats.cache_hits += 1;
        }

        entries.push(Entry {
            subject: subject_iri.clone(),
            text,
            vector: cached,
        });
    }

    // ─── Pass 2: batched embed for cache-miss entries ───────────────────
    //
    // **Intra-sweep deduplication.** The pre-batched code dispatched
    // per-subject and let the embedding cache short-circuit
    // duplicates within the same sweep (doc1's identical body hit
    // the cache that doc0's dispatch just populated). The batched
    // path can't rely on that — pass 1 reads the cache *before* any
    // pass-2 inserts — so we explicitly group cache-miss entries by
    // text and dispatch each unique text once. The fan-out then
    // assigns the same vector to every entry sharing that text.
    //
    // `embedder_calls` counts *unique-text dispatches*, not subjects.
    // The duplicate subjects that re-use a peer's embedding bump
    // `cache_hits` so the saving stays visible in sweep reports.
    //
    // Entry order is preserved at the segment-write step via pass 3
    // (which iterates `entries.into_iter()`), so HNSW neighbour
    // selection remains stable irrespective of which texts batch
    // together in pass 2.
    let mut text_to_entries: std::collections::BTreeMap<&str, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        if e.vector.is_none() {
            text_to_entries.entry(e.text.as_str()).or_default().push(i);
        }
    }
    // SAFETY-via-lifetime: BTreeMap iteration over `&entries`
    // borrows immutably; collect the (text, indices) pairs into
    // owned slots before mutating `entries[idx].vector` below.
    let dedup: Vec<(String, Vec<usize>)> = text_to_entries
        .into_iter()
        .map(|(t, idxs)| (t.to_string(), idxs))
        .collect();
    let batch_size = options.batch_size.max(1);
    for chunk in dedup.chunks(batch_size) {
        if is_cancelled(options.cancellation) {
            return Err(SweepError::Cancelled);
        }
        let texts: Vec<&str> = chunk.iter().map(|(t, _)| t.as_str()).collect();
        let vectors = embed_batch_with_retry(embedder.as_ref(), &texts, options).map_err(|e| {
            // First-subject attribution is lossy when many subjects
            // share a batch; the fallback-to-per-text path in
            // `embed_batch_with_retry` ensures the *actual* broken
            // subject's text is the one that surfaces here when the
            // root cause is per-input.
            let first_idx = chunk[0].1[0];
            SweepError::EmbedderDispatch {
                index: index.iri.as_str().to_string(),
                subject: entries[first_idx].subject.as_str().to_string(),
                source: e,
            }
        })?;
        stats.embedder_calls += texts.len();
        for ((text, idxs), v) in chunk.iter().zip(vectors) {
            if let Some(c) = cache {
                c.insert(text, &index.model, std::sync::Arc::new(v.clone()));
            }
            // First entry in the dedup group gets the original
            // vector; peers count as cache_hits (saved dispatches).
            for (offset, &idx) in idxs.iter().enumerate() {
                if offset > 0 {
                    stats.cache_hits += 1;
                }
                entries[idx].vector = Some(v.clone());
            }
        }
    }

    // Catch a cancel that landed *during* the final batch's embed
    // (the pre-batch check fires once per chunk; on a single-chunk
    // sweep there is no next iteration where it could trip).
    // Without this guard a cancel issued mid-embed would still
    // commit the segment, which subverts the cooperative-cancel
    // contract — registry observers expect the sweep to terminate
    // with no side-effects when cancel won the race.
    if is_cancelled(options.cancellation) {
        return Err(SweepError::Cancelled);
    }

    // ─── Pass 3: extract owned (subject, vector) lists in input order ───
    let (owned_subjects, owned_vectors): (Vec<Iri>, Vec<Vec<f32>>) = entries
        .into_iter()
        .map(|e| {
            let v = e
                .vector
                .expect("every entry should have a vector after pass 1 + 2");
            (e.subject, v)
        })
        .unzip();

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
    let hnsw_bytes = maybe_build_index_hnsw(index, metric_short, &owned_vectors);
    layer
        .storage()
        .vector_index
        .extend_layer(
            &index.iri,
            layer.id(),
            &index.model,
            index.dim,
            metric_short,
            &docs,
            hnsw_bytes.as_deref(),
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

    // ─── Batched-embed sweep round-trip & cancellation ─────────

    /// `batch_size` is purely a performance knob — the sweep must
    /// produce byte-identical segments for any value. This pins the
    /// contract: `batch_size=1` (degenerate, no batching) and
    /// `batch_size=32` (typical sweep config) yield the same subject
    /// IRIs in the same order with the same vector bytes, and the
    /// `embedder_calls` accounting reports the same total work.
    /// Future batched-embedder implementations that diverge (e.g.
    /// numerical drift between per-text and batched forward passes)
    /// will fail this test loudly.
    #[test]
    fn sweep_results_are_independent_of_batch_size() {
        const N: usize = 75;
        let make_layer = || {
            build_corpus(
                "urn:eigenius:test:body",
                "urn:eigenius:embed:dummy:v1",
                8,
                N,
            )
        };
        let make_registry = || {
            let mut reg = EmbedderRegistry::new();
            reg.register(Arc::new(DummyEmbedder::new(
                "urn:eigenius:embed:dummy:v1",
                8,
            )));
            reg
        };

        let run = |batch_size: usize| {
            let layer = make_layer();
            let reg = make_registry();
            let opts = SweepOptions {
                batch_size,
                ..SweepOptions::default()
            };
            let report = sweep_layer_vectors_with_options(&layer, &reg, None, &opts)
                .expect("sweep should succeed");
            let seg = layer
                .storage()
                .vector_index
                .get_segment(&iri("urn:eigenius:test:vi"), layer.id())
                .expect("storage")
                .expect("segment was written");
            (report, seg)
        };

        let (report_1, seg_1) = run(1);
        let (report_32, seg_32) = run(32);

        assert_eq!(report_1.total_subjects, N);
        assert_eq!(report_32.total_subjects, N);
        let stats_key = iri("urn:eigenius:test:vi");
        assert_eq!(
            report_1.per_index.get(&stats_key).unwrap().embedder_calls,
            report_32.per_index.get(&stats_key).unwrap().embedder_calls,
            "embedder_calls counts work, not dispatches — must match across batch sizes"
        );
        assert_eq!(seg_1.subjects, seg_32.subjects, "subject IRI order");
        assert_eq!(seg_1.dim, seg_32.dim, "declared dim");
        assert_eq!(
            seg_1.vectors, seg_32.vectors,
            "vector payload must be byte-identical regardless of batch size"
        );
    }

    /// Cancellation between batches must abort the sweep before the
    /// next chunk is dispatched. We use an embedder whose first
    /// `embed_batch` call flips the cancellation flag, so the second
    /// chunk's pre-dispatch check trips it. The sweep must surface
    /// `Cancelled` rather than running the remaining batches.
    #[test]
    fn sweep_cancellation_between_batches_aborts_remaining_work() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CancelOnSecondBatch {
            iri: Iri,
            calls: AtomicUsize,
            cancel: Arc<AtomicBool>,
        }
        impl Embedder for CancelOnSecondBatch {
            fn model_iri(&self) -> &Iri {
                &self.iri
            }
            fn dim(&self) -> u32 {
                8
            }
            fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedderError> {
                Ok(vec![0.0; 8])
            }
            fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError> {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                // After the very first batch returns, fire cancel so
                // the sweep's pre-batch check on the *next* iteration
                // observes it.
                if n == 0 {
                    self.cancel.store(true, Ordering::SeqCst);
                }
                Ok(vec![vec![0.0; 8]; texts.len()])
            }
        }

        let model = "urn:eigenius:embed:cancel-test:v1";
        let layer = build_corpus("urn:eigenius:test:body", model, 8, 50);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(CancelOnSecondBatch {
            iri: iri(model),
            calls: AtomicUsize::new(0),
            cancel: Arc::clone(&cancel),
        }));

        let opts = SweepOptions {
            cancellation: Some(cancel.as_ref()),
            batch_size: 10,
            ..SweepOptions::default()
        };
        let err = sweep_layer_vectors_with_options(&layer, &reg, None, &opts)
            .expect_err("expected cancel");
        assert!(matches!(err, SweepError::Cancelled), "got {err:?}");

        // No segment should have been written: the sweep aborted
        // before the storage write at the end of `sweep_one_index`.
        assert!(
            layer
                .storage()
                .vector_index
                .get_segment(&iri("urn:eigenius:test:vi"), layer.id())
                .expect("storage")
                .is_none(),
            "cancellation must abort before segment write"
        );
    }

    // ─── D43 §5.7 / M8.3 atomic reindex ────────────────────────

    /// D43 §5.7 / M8.3 — `reindex_chain` walks the chain head→root
    /// and re-embeds every visible subject under a specific
    /// VectorIndex Resource. Builds new `vec_seg:<I_new>:<L>`
    /// entries for every layer `L` that contributed under any
    /// prior Index targeting the same Property. Old segments
    /// stay untouched.
    ///
    /// Scenario: two content layers under I_v1; then a layer
    /// declaring I_v2 with a different model_iri shadows I_v1.
    /// Reindex against I_v2 produces fresh segments per
    /// contributing layer under I_v2; the original I_v1 segments
    /// are unchanged.
    #[test]
    fn reindex_chain_rebuilds_segments_under_new_index() {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("vec-corpus", Some(parent));

        let body_iri = "urn:eigenius:test:body";
        let model_v1 = "urn:eigenius:embed:dummy:v1";
        let model_v2 = "urn:eigenius:embed:dummy:v2";

        // Property declaration.
        let mut body_prop = Resource::new(iri(body_iri));
        body_prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        body_prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        b.add_resource(body_prop).unwrap();

        // I_v1: VectorIndex targeting body via model_v1.
        let i_v1_iri = "urn:eigenius:test:vi_v1";
        let mut vi_v1 = Resource::new(iri(i_v1_iri));
        vi_v1.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi_v1.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
        vi_v1.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_v1)));
        vi_v1.set(iri(wk::VEC_DIM), Value::Integer(8));
        b.add_resource(vi_v1).unwrap();

        // Two content subjects in the same layer.
        for i in 0..2 {
            let mut d = Resource::new(iri(&format!("urn:eigenius:test:doc{i}")));
            d.set(iri(body_iri), Value::String(format!("text {i}")));
            b.add_resource(d).unwrap();
        }

        let head = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        // Run the initial sweep under I_v1 (the only active Index
        // at this head).
        let mut reg_v1 = EmbedderRegistry::new();
        reg_v1.register(Arc::new(DummyEmbedder::new(model_v1, 8)));
        sweep_layer_vectors(&head, &reg_v1, None).expect("v1 sweep");

        // Pre-condition: I_v1 segment exists.
        let vi_index = Arc::clone(&head.storage().vector_index);
        let v1_seg = vi_index
            .get_segment(&iri(i_v1_iri), head.id())
            .unwrap()
            .expect("v1 segment present after sweep");
        assert_eq!(v1_seg.subjects.len(), 2);
        let v1_vec0: Vec<f32> = v1_seg.vector_at(0).to_vec();

        // Now commit a child layer that declares I_v2, shadowing
        // I_v1 by retargeting the same Property under a new model.
        let mut child_b = LayerBuilder::new("v2-upgrade", Some(Arc::clone(&head)));
        // Old Index becomes shadowed by adding a new VectorIndex
        // Resource whose target_property is the same `body`. The
        // shadowing semantics at the layer chain level make I_v2
        // the active Index; I_v1 stays in storage but is no longer
        // queryable from the new head's active-index set.
        let i_v2_iri = "urn:eigenius:test:vi_v2";
        let mut vi_v2 = Resource::new(iri(i_v2_iri));
        vi_v2.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi_v2.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
        vi_v2.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model_v2)));
        vi_v2.set(iri(wk::VEC_DIM), Value::Integer(8));
        // Tombstone I_v1 so the active-index resolution picks I_v2
        // (one active Index per Property per §3.1).
        child_b.tombstone(iri(i_v1_iri)).unwrap();
        child_b.add_resource(vi_v2).unwrap();
        let new_head = Arc::new(child_b.build(head.storage().clone()));

        // Pre-condition: I_v2 has no segments yet (the reindex
        // hasn't run; subjects defined in the parent layer
        // contribute under that parent, not under the new layer).
        assert!(
            vi_index
                .get_segment(&iri(i_v2_iri), head.id())
                .unwrap()
                .is_none(),
            "I_v2 segment should not exist before reindex"
        );

        // Run the reindex against I_v2.
        let mut reg_v2 = EmbedderRegistry::new();
        reg_v2.register(Arc::new(DummyEmbedder::new(model_v2, 8)));
        let report = reindex_chain(
            &new_head,
            &iri(i_v2_iri),
            &reg_v2,
            None,
            &SweepOptions::default(),
        )
        .expect("reindex");
        assert_eq!(
            report.total_subjects, 2,
            "reindex must touch both subjects across the chain"
        );

        // I_v2 segment now exists at the original content layer
        // (where the subjects were defined). The segment's
        // recorded model_iri is the new model — proves the reindex
        // dispatched through I_v2's embedder, not I_v1's. (The
        // raw vector values happen to coincide here because
        // `DummyEmbedder` hashes only the input text; a real
        // embedder would also produce different floats.)
        let v2_seg = vi_index
            .get_segment(&iri(i_v2_iri), head.id())
            .unwrap()
            .expect("I_v2 segment created at the content layer");
        assert_eq!(v2_seg.subjects.len(), 2);
        assert_eq!(
            v2_seg.model_iri.as_str(),
            model_v2,
            "I_v2 segment must record the new model IRI"
        );

        // The original I_v1 segment is untouched.
        let v1_seg_after = vi_index
            .get_segment(&iri(i_v1_iri), head.id())
            .unwrap()
            .expect("I_v1 segment must survive the reindex");
        assert_eq!(v1_seg_after.vector_at(0), v1_vec0.as_slice());
        assert_eq!(v1_seg_after.model_iri.as_str(), model_v1);
    }
}
