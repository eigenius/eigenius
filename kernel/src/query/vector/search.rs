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

//! D43 §2.4 / M5.2 — chain-aware brute-force vector k-NN orchestrator.
//!
//! Two query primitives need different shapes:
//!
//! 1. [`top_k_subjects`] — backs `VECTOR_NEAR(?vec, q, k: K)`. Walks
//!    the head's ancestor set, fetches each layer's segment under
//!    the given Index, computes per-segment top-K via brute-force
//!    similarity, applies the bloom-walk shadow check, and merges
//!    into a global top-K.
//! 2. [`subject_similarity`] — backs `VECTOR_SIM(?vec, q)`. Walks
//!    the ancestor set, finds the most-recent (head-most) segment
//!    that defines the given subject IRI, computes the similarity
//!    score for that single vector vs `q`. Returns `None` if the
//!    subject has no vector under any visible layer.
//!
//! Both honour the per-segment defence-in-depth check from D43
//! §2.4 step 5: the segment's recorded `model_iri` and `distance`
//! must match what the caller (typechecker / active VectorIndex
//! Resource) declares. A mismatch fails the query rather than
//! silently returning scores in the wrong metric.
//!
//! v1 is brute-force per segment — fine for typical layer sizes
//! (≪ 100k vectors). HNSW is the M6 follow-up; segments that opt
//! into it expose a graph the orchestrator traverses instead of
//! the linear scan.

use crate::layer::{collect_ancestors, is_shadowed, Layer, LayerId, VectorIndex};
use crate::ontology::iri::Iri;
use crate::query::vector::cache::SegmentCache;
use crate::query::vector::distance::{compare_similarity, Metric};
use crate::query::vector::recall::{heuristic_recall, min_recall, RecallSource, SegmentRecall};
use crate::query::vector::segment::SegmentView;
use crate::storage::StorageError;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

/// One scored hit emitted by [`top_k_subjects`]. The shape mirrors
/// `text::search::TextScoredHit` so both retrieval primitives plug
/// into the evaluator with one binding-extraction helper.
#[derive(Debug, Clone)]
pub struct VectorScoredHit {
    pub subject: Iri,
    /// Similarity score under the metric the segment was built with.
    /// Higher is better (see [`Metric::similarity`]).
    pub similarity: f32,
    pub defining_layer: LayerId,
}

/// Top-K result bundle returned by [`top_k_subjects_with_recall`].
/// Carries the hits plus a `min_recall` rolled up across every
/// segment the search touched — per D43 §3.4, the result set
/// surfaces the minimum per-segment recall so callers can reason
/// about exactness. `None` when no segment was touched.
#[derive(Debug, Clone)]
pub struct TopKResult {
    pub hits: Vec<VectorScoredHit>,
    pub min_recall: Option<f32>,
}

/// Errors specific to the vector orchestrator. Wraps storage
/// failures and the per-segment validation failures so the
/// evaluator gets one error type to forward to the user.
#[derive(Debug, thiserror::Error)]
pub enum VectorSearchError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error(
        "segment metric mismatch: index '{index}' was built with '{indexed}' but caller declares '{queried}'"
    )]
    MetricMismatch {
        index: String,
        indexed: String,
        queried: String,
    },
    #[error(
        "segment model mismatch: index '{index}' was built with '{indexed}' but caller declares '{queried}'"
    )]
    ModelMismatch {
        index: String,
        indexed: String,
        queried: String,
    },
    #[error("query vector has dim {query_dim} but segment of index '{index}' declares dim {segment_dim}")]
    DimMismatch {
        index: String,
        query_dim: usize,
        segment_dim: usize,
    },
    #[error("metric '{0}' is not a recognised core:DistanceMetric")]
    UnknownMetric(String),
}

/// VECTOR_NEAR — top-K subjects by similarity to `query_vec` under
/// `metric`, drawn from every chain-visible segment of `index_iri`
/// at `head` with shadow filtering applied.
///
/// Returns hits sorted by descending similarity (ties broken by
/// `defining_layer.0` lex order, then by subject IRI, for
/// determinism). The returned `Vec` has length `min(K, total
/// non-shadowed candidates)`.
///
/// `cache` is optional: when supplied, segment fetches go through
/// it (cache-first, populate on miss); when `None`, every fetch
/// hits the underlying `VectorIndex` backend. Test callers pass
/// `None` to drive the storage path directly; production callers
/// pass the kernel-shared cache so repeat probes are O(map-lookup).
///
/// `ef` controls the HNSW search exploration depth for segments
/// whose strategy built an HNSW graph; `None` defaults to
/// `max(k * 4, 64)` per D43 §3.4. Segments without an HNSW graph
/// ignore `ef` and use the brute-force path.
#[allow(clippy::too_many_arguments)]
pub fn top_k_subjects(
    head: &Layer,
    vector_index: &dyn VectorIndex,
    cache: Option<&SegmentCache>,
    index_iri: &Iri,
    query_vec: &[f32],
    k: usize,
    ef: Option<usize>,
    expected_model: &Iri,
    expected_metric: Metric,
) -> Result<Vec<VectorScoredHit>, VectorSearchError> {
    let result = top_k_subjects_with_recall(
        head,
        vector_index,
        cache,
        index_iri,
        query_vec,
        k,
        ef,
        expected_model,
        expected_metric,
    )?;
    Ok(result.hits)
}

/// As [`top_k_subjects`], but additionally surfaces the per-segment
/// recall rollup (D43 §3.4). The hits are unchanged from the
/// brute-only variant; the new field is `min_recall = min(per-
/// segment recall across every touched segment)`, with `Exact`
/// (flat) segments contributing `1.0` and HNSW segments
/// contributing a heuristic from `ef / k`. `None` when no segment
/// was touched.
#[allow(clippy::too_many_arguments)]
pub fn top_k_subjects_with_recall(
    head: &Layer,
    vector_index: &dyn VectorIndex,
    cache: Option<&SegmentCache>,
    index_iri: &Iri,
    query_vec: &[f32],
    k: usize,
    ef: Option<usize>,
    expected_model: &Iri,
    expected_metric: Metric,
) -> Result<TopKResult, VectorSearchError> {
    if k == 0 {
        return Ok(TopKResult {
            hits: Vec::new(),
            min_recall: None,
        });
    }
    let effective_ef = ef.unwrap_or_else(|| (k * 4).max(64));
    let chain = collect_ancestors(head);
    let mut heap: BinaryHeap<Reverse<HeapEntry>> = BinaryHeap::with_capacity(k + 1);
    // One SegmentRecall entry per *touched* segment (segment exists
    // in the index for this chain layer); accumulated across the
    // chain walk and rolled up after the heap drain.
    let mut per_segment_recalls: Vec<SegmentRecall> = Vec::new();

    for layer_id in &chain {
        let segment = match fetch_segment(vector_index, cache, index_iri, layer_id)? {
            Some(s) => s,
            None => continue,
        };
        verify_segment_shape(
            index_iri,
            &segment,
            query_vec,
            expected_model,
            expected_metric,
        )?;
        // Strategy dispatch (D43 §2.4): when the segment carries an
        // HNSW graph, traverse it with `ef` and oversample to
        // `k * 2` per segment so the global merge can reorder.
        // Otherwise brute-force scan. Per-segment recall: HNSW
        // segments report the §3.4 heuristic; flat segments are
        // exact.
        if let Some(graph) = segment.hnsw() {
            let per_segment_k = (k * 2).max(k);
            let hits = graph.search(query_vec, per_segment_k, effective_ef);
            per_segment_recalls.push(SegmentRecall::Approx {
                recall: heuristic_recall(k, effective_ef),
                source: RecallSource::Heuristic,
            });
            let subjects = segment.subjects();
            for (node_idx, sim) in hits {
                let subject = subjects[node_idx].clone();
                if is_shadowed(head, layer_id, &subject) {
                    continue;
                }
                push_heap(
                    &mut heap,
                    HeapEntry {
                        sim,
                        layer: layer_id.clone(),
                        subject,
                    },
                    k,
                );
            }
        } else {
            per_segment_recalls.push(SegmentRecall::Exact);
            let dim = segment.dim() as usize;
            let vectors = segment.vectors();
            let subjects = segment.subjects();
            for i in 0..segment.count() {
                let vec_i = &vectors[i * dim..(i + 1) * dim];
                let sim = expected_metric.similarity(query_vec, vec_i);
                let subject = subjects[i].clone();
                if is_shadowed(head, layer_id, &subject) {
                    continue;
                }
                push_heap(
                    &mut heap,
                    HeapEntry {
                        sim,
                        layer: layer_id.clone(),
                        subject,
                    },
                    k,
                );
            }
        }
    }

    // `BinaryHeap<Reverse<X>>::into_sorted_vec()` returns ascending
    // order by `Reverse<X>`, which is descending order by `X`. So the
    // resulting vec is already highest-similarity-first — no reverse
    // needed.
    let hits: Vec<VectorScoredHit> = heap
        .into_sorted_vec()
        .into_iter()
        .map(|Reverse(e)| VectorScoredHit {
            subject: e.subject,
            similarity: e.sim,
            defining_layer: e.layer,
        })
        .collect();
    Ok(TopKResult {
        hits,
        min_recall: min_recall(per_segment_recalls),
    })
}

/// VECTOR_SIM — similarity score for one specific subject under
/// `metric`, taken from the head-most chain-visible segment that
/// holds a vector for that subject. Returns `None` if no segment
/// in the chain provides a vector for `subject`.
///
/// Walks ancestors in chain order. The first segment found that
/// holds the subject wins (this matches the layer chain's "top
/// layer wins" body-resolution discipline).
#[allow(clippy::too_many_arguments)]
pub fn subject_similarity(
    head: &Layer,
    vector_index: &dyn VectorIndex,
    cache: Option<&SegmentCache>,
    index_iri: &Iri,
    subject: &Iri,
    query_vec: &[f32],
    expected_model: &Iri,
    expected_metric: Metric,
) -> Result<Option<f32>, VectorSearchError> {
    let chain = collect_ancestors(head);
    // `collect_ancestors` returns ancestors as a set; walk via the
    // resolution path (head → parents) to find the *head-most*
    // segment that holds the subject. The chain set is used as a
    // membership filter on visited layers.
    let mut current: Option<&Layer> = Some(head);
    while let Some(layer) = current {
        if !chain.contains(layer.id()) {
            current = layer.parents().first().map(|p| p.as_ref());
            continue;
        }
        if let Some(segment) = fetch_segment(vector_index, cache, index_iri, layer.id())? {
            verify_segment_shape(
                index_iri,
                &segment,
                query_vec,
                expected_model,
                expected_metric,
            )?;
            // Linear search for the subject — segments are small in
            // v1, no per-subject index needed.
            if let Some(i) = segment.subjects().iter().position(|s| s == subject) {
                let dim = segment.dim() as usize;
                let vec_i = &segment.vectors()[i * dim..(i + 1) * dim];
                return Ok(Some(expected_metric.similarity(query_vec, vec_i)));
            }
        }
        current = layer.parents().first().map(|p| p.as_ref());
    }
    Ok(None)
}

// ---------------- internals ----------------

/// Cache-aware segment fetch. Cache miss falls through to the
/// backend's `get_segment`; the fetched segment is then admitted
/// into the cache so a sibling probe (`VECTOR_SIM` after
/// `VECTOR_NEAR`, or the next row of a per-row evaluation loop)
/// hits in-memory.
fn fetch_segment(
    vector_index: &dyn VectorIndex,
    cache: Option<&SegmentCache>,
    index_iri: &Iri,
    layer_id: &LayerId,
) -> Result<Option<Arc<SegmentView>>, StorageError> {
    if let Some(c) = cache {
        if let Some(hit) = c.get(index_iri, layer_id) {
            return Ok(Some(hit));
        }
    }
    let view = match vector_index.get_segment(index_iri, layer_id)? {
        Some(s) => Arc::new(SegmentView::from_segment(s)),
        None => return Ok(None),
    };
    if let Some(c) = cache {
        c.insert(index_iri.clone(), layer_id.clone(), Arc::clone(&view));
    }
    Ok(Some(view))
}

fn verify_segment_shape(
    index_iri: &Iri,
    segment: &SegmentView,
    query_vec: &[f32],
    expected_model: &Iri,
    expected_metric: Metric,
) -> Result<(), VectorSearchError> {
    if segment.model_iri() != expected_model {
        return Err(VectorSearchError::ModelMismatch {
            index: index_iri.as_str().to_string(),
            indexed: segment.model_iri().as_str().to_string(),
            queried: expected_model.as_str().to_string(),
        });
    }
    let segment_metric = Metric::from_short_name(segment.distance())
        .ok_or_else(|| VectorSearchError::UnknownMetric(segment.distance().to_string()))?;
    if segment_metric != expected_metric {
        return Err(VectorSearchError::MetricMismatch {
            index: index_iri.as_str().to_string(),
            indexed: segment.distance().to_string(),
            queried: expected_metric.short_name().to_string(),
        });
    }
    if query_vec.len() != segment.dim() as usize {
        return Err(VectorSearchError::DimMismatch {
            index: index_iri.as_str().to_string(),
            query_dim: query_vec.len(),
            segment_dim: segment.dim() as usize,
        });
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct HeapEntry {
    sim: f32,
    layer: LayerId,
    subject: Iri,
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        compare_similarity(self.sim, other.sim).is_eq()
            && self.layer == other.layer
            && self.subject == other.subject
    }
}
impl Eq for HeapEntry {}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        compare_similarity(self.sim, other.sim)
            .then_with(|| self.layer.0.cmp(&other.layer.0))
            .then_with(|| self.subject.as_str().cmp(other.subject.as_str()))
    }
}
impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn push_heap(heap: &mut BinaryHeap<Reverse<HeapEntry>>, entry: HeapEntry, k: usize) {
    if heap.len() < k {
        heap.push(Reverse(entry));
        return;
    }
    // Peek the worst-scoring kept entry (top of min-heap = smallest
    // sim under the Reverse wrapper). If the new entry's similarity
    // is higher, evict it.
    if let Some(Reverse(worst)) = heap.peek() {
        if compare_similarity(entry.sim, worst.sim).is_gt() {
            heap.pop();
            heap.push(Reverse(entry));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerBuilder, LayerStorage, VectorDoc, VectorIndex};
    use crate::ontology::resource::Resource;
    use crate::ontology::well_known as wk;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a head layer with a single subject + an embedded vector
    /// under the named VectorIndex, returning the head + storage so
    /// the test can issue queries against it.
    ///
    /// `subjects` and `vectors` are parallel — `vectors[i]` is the
    /// embedding for `subjects[i]`. Each vector must have length
    /// `dim`. The fixture goes straight to `VectorIndex::extend_layer`
    /// (no Embedder dispatch); the indexing-side integration that
    /// runs at LayerBuilder::build is the M5.3 milestone.
    #[allow(clippy::too_many_arguments)]
    fn build_corpus_layer(
        index_iri_str: &str,
        model_iri_str: &str,
        metric: &str,
        dim: u32,
        subjects: &[&str],
        vectors: &[Vec<f32>],
    ) -> (Arc<Layer>, Arc<dyn VectorIndex>) {
        assert_eq!(subjects.len(), vectors.len());
        let storage = LayerStorage::in_memory();
        // The MemoryVectorIndex used by the layer storage is the one
        // we'll write segments to. Pull it out of the storage handle
        // so we can drive it directly.
        let vec_index = Arc::clone(&storage.vector_index);
        // A trivial Resource so `defined_iris()` is non-empty (some
        // helpers rely on it). The vector data goes through the
        // index directly, not via property values.
        let mut b = LayerBuilder::new("vec-corpus", None);
        let placeholder = Resource::new(iri("urn:eigenius:test:placeholder"));
        b.add_resource(placeholder).unwrap();
        let layer = Arc::new(b.build(storage));
        let docs: Vec<VectorDoc<'_>> = subjects
            .iter()
            .zip(vectors.iter())
            .map(|(s, v)| VectorDoc {
                subject: Box::leak(Box::new(iri(s))),
                vector: v.as_slice(),
            })
            .collect();
        // The MemoryVectorIndex is shared via Arc; write the segment
        // directly so the orchestrator sees it.
        vec_index
            .extend_layer(
                &iri(index_iri_str),
                layer.id(),
                &iri(model_iri_str),
                dim,
                metric,
                &docs,
                None,
            )
            .expect("write segment");
        (layer, vec_index)
    }

    /// Build a chain: parent + child. Both contribute segments under
    /// the same Index so multi-layer behaviour can be tested.
    #[allow(clippy::too_many_arguments)]
    fn build_chain(
        index_iri_str: &str,
        model_iri_str: &str,
        metric: &str,
        dim: u32,
        parent_subjects: &[&str],
        parent_vectors: &[Vec<f32>],
        child_subjects: &[&str],
        child_vectors: &[Vec<f32>],
    ) -> (Arc<Layer>, Arc<dyn VectorIndex>) {
        let storage = LayerStorage::in_memory();
        let vec_index = Arc::clone(&storage.vector_index);

        let mut pb = LayerBuilder::new("parent", None);
        pb.add_resource(Resource::new(iri("urn:eigenius:test:parent_marker")))
            .unwrap();
        let parent = Arc::new(pb.build(storage.clone()));

        let parent_docs: Vec<VectorDoc<'_>> = parent_subjects
            .iter()
            .zip(parent_vectors.iter())
            .map(|(s, v)| VectorDoc {
                subject: Box::leak(Box::new(iri(s))),
                vector: v.as_slice(),
            })
            .collect();
        vec_index
            .extend_layer(
                &iri(index_iri_str),
                parent.id(),
                &iri(model_iri_str),
                dim,
                metric,
                &parent_docs,
                None,
            )
            .expect("write parent segment");

        let mut cb = LayerBuilder::new("child", Some(parent.clone()));
        cb.add_resource(Resource::new(iri("urn:eigenius:test:child_marker")))
            .unwrap();
        let child = Arc::new(cb.build(storage));

        let child_docs: Vec<VectorDoc<'_>> = child_subjects
            .iter()
            .zip(child_vectors.iter())
            .map(|(s, v)| VectorDoc {
                subject: Box::leak(Box::new(iri(s))),
                vector: v.as_slice(),
            })
            .collect();
        vec_index
            .extend_layer(
                &iri(index_iri_str),
                child.id(),
                &iri(model_iri_str),
                dim,
                metric,
                &child_docs,
                None,
            )
            .expect("write child segment");

        (child, vec_index)
    }

    const IDX: &str = "urn:eigenius:test:vi";
    const MODEL: &str = "urn:eigenius:embed:test:m1";

    #[test]
    fn top_k_zero_returns_empty() {
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            2,
            &["urn:eigenius:test:a"],
            &[vec![1.0, 0.0]],
        );
        let hits = top_k_subjects(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            0,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn top_k_returns_sorted_by_descending_similarity() {
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            2,
            &[
                "urn:eigenius:test:a",
                "urn:eigenius:test:b",
                "urn:eigenius:test:c",
            ],
            &[
                vec![1.0, 0.0], // identical to query
                vec![0.0, 1.0], // orthogonal
                vec![0.7, 0.7], // 45°
            ],
        );
        let hits = top_k_subjects(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            3,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert_eq!(hits.len(), 3);
        // a (1.0) > c (0.707) > b (0.0)
        assert_eq!(hits[0].subject.as_str(), "urn:eigenius:test:a");
        assert_eq!(hits[1].subject.as_str(), "urn:eigenius:test:c");
        assert_eq!(hits[2].subject.as_str(), "urn:eigenius:test:b");
        assert!(hits[0].similarity > hits[1].similarity);
        assert!(hits[1].similarity > hits[2].similarity);
    }

    #[test]
    fn top_k_truncates_to_k() {
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            2,
            &[
                "urn:eigenius:test:a",
                "urn:eigenius:test:b",
                "urn:eigenius:test:c",
            ],
            &[vec![1.0, 0.0], vec![0.0, 1.0], vec![0.7, 0.7]],
        );
        let hits = top_k_subjects(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            2,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].subject.as_str(), "urn:eigenius:test:a");
        assert_eq!(hits[1].subject.as_str(), "urn:eigenius:test:c");
    }

    #[test]
    fn top_k_walks_chain_and_includes_parent_segments() {
        let (head, idx) = build_chain(
            IDX,
            MODEL,
            "cosine",
            2,
            // parent has 'a' close to query
            &["urn:eigenius:test:a"],
            &[vec![1.0, 0.0]],
            // child has 'b' orthogonal
            &["urn:eigenius:test:b"],
            &[vec![0.0, 1.0]],
        );
        let hits = top_k_subjects(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            5,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        let subjects: Vec<&str> = hits.iter().map(|h| h.subject.as_str()).collect();
        assert!(subjects.contains(&"urn:eigenius:test:a"));
        assert!(subjects.contains(&"urn:eigenius:test:b"));
    }

    #[test]
    fn top_k_empty_chain_returns_empty() {
        // Head layer with no vector segments.
        let storage = LayerStorage::in_memory();
        let mut b = LayerBuilder::new("empty", None);
        b.add_resource(Resource::new(iri("urn:eigenius:test:placeholder")))
            .unwrap();
        let head = Arc::new(b.build(storage.clone()));
        let hits = top_k_subjects(
            &head,
            storage.vector_index.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            5,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn top_k_rejects_query_with_wrong_dim() {
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            3,
            &["urn:eigenius:test:a"],
            &[vec![1.0, 0.0, 0.0]],
        );
        let err = top_k_subjects(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            5,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap_err();
        assert!(
            matches!(err, VectorSearchError::DimMismatch { .. }),
            "expected DimMismatch; got {err:?}"
        );
    }

    #[test]
    fn top_k_rejects_segment_with_wrong_model() {
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            2,
            &["urn:eigenius:test:a"],
            &[vec![1.0, 0.0]],
        );
        let other_model = "urn:eigenius:embed:test:other";
        let err = top_k_subjects(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            5,
            None,
            &iri(other_model),
            Metric::Cosine,
        )
        .unwrap_err();
        assert!(
            matches!(err, VectorSearchError::ModelMismatch { .. }),
            "expected ModelMismatch; got {err:?}"
        );
    }

    #[test]
    fn top_k_rejects_metric_mismatch() {
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "l2",
            2,
            &["urn:eigenius:test:a"],
            &[vec![1.0, 0.0]],
        );
        let err = top_k_subjects(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            5,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap_err();
        assert!(
            matches!(err, VectorSearchError::MetricMismatch { .. }),
            "expected MetricMismatch; got {err:?}"
        );
    }

    // ─── subject_similarity ─────────────────────────────────────

    #[test]
    fn subject_similarity_returns_score_for_known_subject() {
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            2,
            &["urn:eigenius:test:a", "urn:eigenius:test:b"],
            &[vec![1.0, 0.0], vec![0.0, 1.0]],
        );
        let sim = subject_similarity(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &iri("urn:eigenius:test:a"),
            &[1.0, 0.0],
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert!(sim.is_some());
        assert!((sim.unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn subject_similarity_returns_none_for_unknown_subject() {
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            2,
            &["urn:eigenius:test:a"],
            &[vec![1.0, 0.0]],
        );
        let sim = subject_similarity(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &iri("urn:eigenius:test:never_indexed"),
            &[1.0, 0.0],
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert!(sim.is_none());
    }

    #[test]
    fn subject_similarity_picks_head_most_segment() {
        // Parent and child both define the same subject — child's
        // vector is what `subject_similarity` returns (chain head
        // wins).
        let (head, idx) = build_chain(
            IDX,
            MODEL,
            "cosine",
            2,
            &["urn:eigenius:test:s"],
            &[vec![0.0, 1.0]], // parent: orthogonal to query (1,0)
            &["urn:eigenius:test:s"],
            &[vec![1.0, 0.0]], // child: identical to query (1,0)
        );
        let sim = subject_similarity(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &iri("urn:eigenius:test:s"),
            &[1.0, 0.0],
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        // Should match child's vector → similarity ≈ 1.0.
        assert!(
            (sim.unwrap() - 1.0).abs() < 1e-6,
            "expected child's vector to win; got {:?}",
            sim
        );
    }

    /// Suppress the `wk` import warning when wk constants aren't
    /// actually used in tests — keeps the wildcard available for
    /// future fixtures without surfacing dead-code warnings.
    #[allow(dead_code)]
    fn _touch_wk() {
        let _ = wk::PROPERTY;
    }

    // ─── SegmentCache integration ───────────────────────────────

    #[test]
    fn cache_takes_precedence_over_empty_backend() {
        // Backend has no segment, but the cache holds one with the
        // right key. `top_k_subjects` should serve it from the cache
        // without ever consulting the backend.
        let storage = LayerStorage::in_memory();
        let mut b = LayerBuilder::new("cache-test", None);
        b.add_resource(Resource::new(iri("urn:eigenius:test:placeholder")))
            .unwrap();
        let head = Arc::new(b.build(storage.clone()));

        // Empty backend.
        let backend = storage.vector_index.clone();
        assert!(backend.get_segment(&iri(IDX), head.id()).unwrap().is_none());

        // Cache pre-loaded with a single-subject segment.
        let cache = SegmentCache::new(16);
        let fake = crate::layer::VectorSegment {
            model_iri: iri(MODEL),
            dim: 2,
            distance: "cosine".into(),
            subjects: vec![iri("urn:eigenius:test:from_cache")],
            vectors: vec![1.0, 0.0],
            hnsw_graph_bytes: None,
        };
        cache.insert(
            iri(IDX),
            head.id().clone(),
            Arc::new(SegmentView::from_segment(fake)),
        );

        let hits = top_k_subjects(
            &head,
            backend.as_ref(),
            Some(&cache),
            &iri(IDX),
            &[1.0, 0.0],
            5,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject.as_str(), "urn:eigenius:test:from_cache");
    }

    #[test]
    fn cache_populates_from_backend_on_miss() {
        // Cache empty; backend has a segment. After one probe, the
        // segment should be in the cache (visible via
        // `approximate_count > 0` after `run_pending_tasks`).
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            2,
            &["urn:eigenius:test:a"],
            &[vec![1.0, 0.0]],
        );
        let cache = SegmentCache::new(16);
        assert_eq!(cache.approximate_count(), 0);
        let _ = top_k_subjects(
            &head,
            idx.as_ref(),
            Some(&cache),
            &iri(IDX),
            &[1.0, 0.0],
            1,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        cache.run_pending_tasks();
        assert!(
            cache.approximate_count() >= 1,
            "cache should hold the fetched segment"
        );
        // And the cache entry is keyed by (IDX, head.id()).
        assert!(cache.get(&iri(IDX), head.id()).is_some());
    }

    #[test]
    fn cache_is_used_by_subject_similarity_too() {
        let storage = LayerStorage::in_memory();
        let mut b = LayerBuilder::new("cache-sim", None);
        b.add_resource(Resource::new(iri("urn:eigenius:test:placeholder")))
            .unwrap();
        let head = Arc::new(b.build(storage.clone()));

        let cache = SegmentCache::new(16);
        let fake = crate::layer::VectorSegment {
            model_iri: iri(MODEL),
            dim: 2,
            distance: "cosine".into(),
            subjects: vec![iri("urn:eigenius:test:a")],
            vectors: vec![1.0, 0.0],
            hnsw_graph_bytes: None,
        };
        cache.insert(
            iri(IDX),
            head.id().clone(),
            Arc::new(SegmentView::from_segment(fake)),
        );

        let sim = subject_similarity(
            &head,
            storage.vector_index.as_ref(),
            Some(&cache),
            &iri(IDX),
            &iri("urn:eigenius:test:a"),
            &[1.0, 0.0],
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert!(sim.is_some());
        assert!((sim.unwrap() - 1.0).abs() < 1e-6);
    }

    // ─── HNSW dispatch (M6.4) ───────────────────────────────────

    /// When the cached SegmentView has an HNSW graph attached, the
    /// query path traverses it instead of the brute-force scan.
    /// We can't directly observe the path taken from outside, but
    /// we can prove the behaviour by:
    ///
    /// 1. Building a corpus where the brute-force backend would
    ///    return one result set (e.g. the segment), and
    /// 2. Pre-loading the cache with a SegmentView whose HNSW graph
    ///    indexes a *different* vector set, then
    /// 3. Verifying the query result reflects the cache's HNSW
    ///    contents, not the backend.
    ///
    /// HNSW returning the right top-k under self-queries is the
    /// adapter's own contract (covered by `query::vector::hnsw`
    /// tests); this test only verifies the dispatch decision.
    #[test]
    fn cache_view_with_hnsw_serves_top_k_via_hnsw_path() {
        use crate::query::vector::hnsw::HnswBuildConfig;
        use crate::query::vector::segment::{admit_segment, SegmentStrategy};

        let storage = LayerStorage::in_memory();
        let mut b = LayerBuilder::new("hnsw-dispatch", None);
        b.add_resource(Resource::new(iri("urn:eigenius:test:placeholder")))
            .unwrap();
        let head = Arc::new(b.build(storage.clone()));

        // Cache holds a segment with three vectors; HNSW built over
        // them. The backend is empty.
        let cache = SegmentCache::new(16);
        let fake = crate::layer::VectorSegment {
            model_iri: iri(MODEL),
            dim: 2,
            distance: "cosine".into(),
            subjects: vec![
                iri("urn:eigenius:test:doca"),
                iri("urn:eigenius:test:docb"),
                iri("urn:eigenius:test:docc"),
            ],
            vectors: vec![1.0, 0.0, 0.0, 1.0, -1.0, 0.0],
            hnsw_graph_bytes: None,
        };
        let view = admit_segment(
            fake,
            Metric::Cosine,
            SegmentStrategy::Hnsw,
            HnswBuildConfig::for_segment(3),
        );
        assert!(view.hnsw().is_some(), "test setup: HNSW should be built");
        cache.insert(iri(IDX), head.id().clone(), Arc::new(view));

        // Query along the (1, 0) axis. HNSW top-1 should be doca.
        let hits = top_k_subjects(
            &head,
            storage.vector_index.as_ref(),
            Some(&cache),
            &iri(IDX),
            &[1.0, 0.0],
            1,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject.as_str(), "urn:eigenius:test:doca");
        assert!((hits[0].similarity - 1.0).abs() < 1e-4);
    }

    #[test]
    fn top_k_with_recall_returns_exact_for_flat_only_chain() {
        // All segments are flat → per-segment recall is Exact →
        // min_recall is 1.0.
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            2,
            &["urn:eigenius:test:a", "urn:eigenius:test:b"],
            &[vec![1.0, 0.0], vec![0.0, 1.0]],
        );
        let result = top_k_subjects_with_recall(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            1,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(
            result.min_recall,
            Some(1.0),
            "flat segments should yield min_recall = 1.0"
        );
    }

    #[test]
    fn top_k_with_recall_emits_heuristic_for_hnsw_segments() {
        use crate::query::vector::hnsw::HnswBuildConfig;
        use crate::query::vector::segment::{admit_segment, SegmentStrategy};

        // Cache holds an HNSW segment; backend empty. Query goes
        // through the HNSW path and should report a sub-1.0
        // min_recall.
        let storage = LayerStorage::in_memory();
        let mut b = LayerBuilder::new("recall-hnsw", None);
        b.add_resource(Resource::new(iri("urn:eigenius:test:placeholder")))
            .unwrap();
        let head = Arc::new(b.build(storage.clone()));

        let cache = SegmentCache::new(16);
        let fake = crate::layer::VectorSegment {
            model_iri: iri(MODEL),
            dim: 2,
            distance: "cosine".into(),
            subjects: vec![iri("urn:eigenius:test:a"), iri("urn:eigenius:test:b")],
            vectors: vec![1.0, 0.0, 0.0, 1.0],
            hnsw_graph_bytes: None,
        };
        let view = admit_segment(
            fake,
            Metric::Cosine,
            SegmentStrategy::Hnsw,
            HnswBuildConfig::for_segment(2),
        );
        cache.insert(iri(IDX), head.id().clone(), Arc::new(view));

        // k=4, ef=None → default ef = max(k*4, 64) = 64, ratio=16
        // → cap at 0.99. Whatever the exact value, it must be < 1.0
        // and >= 0.85.
        let result = top_k_subjects_with_recall(
            &head,
            storage.vector_index.as_ref(),
            Some(&cache),
            &iri(IDX),
            &[1.0, 0.0],
            4,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        let recall = result.min_recall.expect("HNSW segment was touched");
        assert!(
            (0.85..1.0).contains(&recall),
            "HNSW recall should be approximate; got {recall}"
        );
    }

    #[test]
    fn top_k_with_recall_min_picks_lowest_across_segments() {
        // A mixed chain: parent has an HNSW segment, child has a
        // flat segment. min_recall is the HNSW segment's heuristic
        // (lower than the flat 1.0).
        use crate::query::vector::hnsw::HnswBuildConfig;
        use crate::query::vector::segment::{admit_segment, SegmentStrategy};

        // Use build_chain to set up the chain structure, then
        // override the parent's cache entry with an HNSW-bearing
        // SegmentView.
        let (head, idx) = build_chain(
            IDX,
            MODEL,
            "cosine",
            2,
            &["urn:eigenius:test:a"],
            &[vec![1.0, 0.0]],
            &["urn:eigenius:test:b"],
            &[vec![0.0, 1.0]],
        );
        let parent_id = head.parents()[0].id().clone();

        // Pre-load the cache: parent gets an HNSW view, child gets
        // a flat view. (Both built from the same vector data — the
        // chain's actual storage is the source of truth, but the
        // cache takes precedence per M5.6 test
        // `cache_takes_precedence_over_empty_backend`.)
        let cache = SegmentCache::new(16);
        let parent_seg = crate::layer::VectorSegment {
            model_iri: iri(MODEL),
            dim: 2,
            distance: "cosine".into(),
            subjects: vec![iri("urn:eigenius:test:a")],
            vectors: vec![1.0, 0.0],
            hnsw_graph_bytes: None,
        };
        cache.insert(
            iri(IDX),
            parent_id.clone(),
            Arc::new(admit_segment(
                parent_seg,
                Metric::Cosine,
                SegmentStrategy::Hnsw,
                HnswBuildConfig::for_segment(1),
            )),
        );
        let child_seg = crate::layer::VectorSegment {
            model_iri: iri(MODEL),
            dim: 2,
            distance: "cosine".into(),
            subjects: vec![iri("urn:eigenius:test:b")],
            vectors: vec![0.0, 1.0],
            hnsw_graph_bytes: None,
        };
        cache.insert(
            iri(IDX),
            head.id().clone(),
            Arc::new(admit_segment(
                child_seg,
                Metric::Cosine,
                SegmentStrategy::Flat,
                HnswBuildConfig::for_segment(1),
            )),
        );

        let result = top_k_subjects_with_recall(
            &head,
            idx.as_ref(),
            Some(&cache),
            &iri(IDX),
            &[1.0, 0.0],
            4,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        let recall = result.min_recall.expect("two segments touched");
        // HNSW heuristic should be < 1.0 (parent's segment).
        // Flat's 1.0 doesn't lower the min — HNSW's does.
        assert!(
            recall < 1.0,
            "min across (HNSW, flat) should be the HNSW heuristic < 1.0; got {recall}"
        );
        assert!(recall >= 0.85);
    }

    #[test]
    fn top_k_with_recall_returns_none_for_empty_chain() {
        let storage = LayerStorage::in_memory();
        let mut b = LayerBuilder::new("empty", None);
        b.add_resource(Resource::new(iri("urn:eigenius:test:placeholder")))
            .unwrap();
        let head = Arc::new(b.build(storage.clone()));
        let result = top_k_subjects_with_recall(
            &head,
            storage.vector_index.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            5,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert!(result.hits.is_empty());
        assert_eq!(result.min_recall, None);
    }

    #[test]
    fn ef_default_max_k4_or_64() {
        // top_k_subjects computes effective_ef = ef.unwrap_or(max(k*4, 64)).
        // Pin the floor at k=10: max(40, 64) = 64.
        // Pin the multiplier at k=20: max(80, 64) = 80.
        // The behaviour is observable only when HNSW is in play, but
        // we can verify the function accepts None without panicking
        // even with k > 16 (the regime where the default formula
        // matters).
        let (head, idx) = build_corpus_layer(
            IDX,
            MODEL,
            "cosine",
            2,
            &["urn:eigenius:test:a"],
            &[vec![1.0, 0.0]],
        );
        // k=20 exercises the (k*4 > 64) branch; smaller corpus is
        // intentional — we only verify the call shape.
        let hits = top_k_subjects(
            &head,
            idx.as_ref(),
            None,
            &iri(IDX),
            &[1.0, 0.0],
            20,
            None,
            &iri(MODEL),
            Metric::Cosine,
        )
        .unwrap();
        assert_eq!(hits.len(), 1, "single-vector corpus yields one hit");
    }
}
