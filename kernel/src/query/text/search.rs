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

//! D43 §2.3 / M3.3 — text-search chain-walk orchestrator.
//!
//! Ties together everything M3.1 (analyzer) and M3.2 (BM25 scorer)
//! provide with the layer chain walk + posting-set intersection +
//! shadow check + cross-layer top-k merge that the D43 §2.3 query
//! path step-by-step describes. The implementation matches that
//! recipe exactly:
//!
//! 1. Resolve the active TextIndex (caller's responsibility — they
//!    pass it in as a parameter; M3.6 typechecker discovers it).
//! 2. Tokenise the query string using the active TextIndex's
//!    analyzer.
//! 3. Collect the head's ancestor set (`collect_ancestors`).
//! 4. Compute chain-aware BM25 stats (`compute_chain_stats`).
//! 5. For each layer with hits for all terms (AND constraint):
//!    intersect postings via `TextIndex::intersect_layer`; resolve
//!    surviving doc-ids to `(subject, doc_length)` via
//!    `get_layer_docs`; score each doc with BM25; apply the
//!    bloom-walk shadow check; emit a hit.
//! 6. Merge into a sorted result list (highest score first).
//!    Caller truncates to a `TOP K`.
//!
//! M3.4 introduces caches for the deserialised intermediate state
//! (`TermCache`, `DocsCache`); M3.7 wires this orchestrator into
//! the EigenQL evaluator.

use crate::layer::{collect_ancestors, is_shadowed, Layer, LayerId, TextIndex};
use crate::ontology::iri::Iri;
use crate::query::text::analyzer::Analyzer;
use crate::query::text::bm25::{compute_chain_stats, Bm25Params, Bm25Scorer};
use crate::storage::StorageError;

/// One scored hit emitted by [`run_text_search`]. The triple matches
/// the shape D43 §2.3 query path step 7 specifies: subject IRI,
/// BM25 score, and the layer that defined the surviving body.
#[derive(Debug, Clone)]
pub struct TextScoredHit {
    pub subject: Iri,
    pub score: f32,
    pub defining_layer: LayerId,
}

/// Errors specific to the orchestrator. Wraps storage and analyzer
/// failures so the caller (the M3.7 evaluator) doesn't have to
/// unify error types from three different sources.
#[derive(Debug, thiserror::Error)]
pub enum TextSearchError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("analyzer-id mismatch: index '{index}' was built with '{indexed}' but query analyzer is '{query}'")]
    AnalyzerMismatch {
        index: String,
        indexed: String,
        query: String,
    },
    #[error("text_docs missing for layer {layer} under index {index}")]
    MissingDocs { index: String, layer: String },
    #[error("analyzer id '{analyzer_id}' is not registered in the kernel analyzer registry")]
    UnknownAnalyzer { analyzer_id: String },
}

/// Run a text search across the layer chain rooted at `head`,
/// against an active TextIndex Resource identified by `index_iri`.
///
/// The caller supplies the `analyzer` matching that TextIndex's
/// declared `text_analyzer` slot — typically resolved via
/// [`crate::layer::resolve_active_text_indexes`] (M2.6) and
/// [`crate::query::text::analyzer::registry::analyzer_for`] (M3.1).
///
/// Returns hits sorted by descending score (ties broken by
/// `defining_layer.0` lexicographically for determinism). Caller
/// truncates to a `TOP K` if needed.
///
/// The orchestrator verifies — once per visited layer — that the
/// stored per-`(index, layer)` analyzer ID matches the `analyzer`
/// passed in. A mismatch is the defence-in-depth check that
/// catches an Index Resource being re-defined with a different
/// analyzer without re-indexing (which shouldn't happen under the
/// §5.7 atomic-reindex policy, but the verification is cheap).
pub fn run_text_search(
    head: &Layer,
    text_index: &dyn TextIndex,
    index_iri: &Iri,
    analyzer: &dyn Analyzer,
    query: &str,
) -> Result<Vec<TextScoredHit>, TextSearchError> {
    let tokens = analyzer.tokenize(query);
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let chain = collect_ancestors(head);
    let stats = compute_chain_stats(head, text_index, index_iri, &tokens);

    // Dedup query terms before issuing intersect calls — repeated
    // terms in the query string (e.g. `"alpha alpha"`) collapse to
    // a single posting probe per term per layer.
    let mut unique_terms: Vec<String> = tokens.clone();
    unique_terms.sort();
    unique_terms.dedup();

    // Borrow query terms as `&str` for `Bm25Scorer::score_doc`.
    let term_refs: Vec<&str> = unique_terms.iter().map(|s| s.as_str()).collect();

    let scorer = Bm25Scorer::new(Bm25Params::default(), stats.clone());

    let mut hits: Vec<TextScoredHit> = Vec::new();

    for layer in &chain {
        // Verify analyzer-id consistency once per visited layer.
        // Layers without per-`(index, layer)` stats simply contributed
        // nothing and don't need to verify.
        match text_index.get_layer_analyzer(index_iri, layer)? {
            Some(stored) if stored != analyzer.id() => {
                return Err(TextSearchError::AnalyzerMismatch {
                    index: index_iri.as_str().to_string(),
                    indexed: stored,
                    query: analyzer.id().to_string(),
                });
            }
            // Some(match) → ok. None → no contribution under this Index here.
            _ => {}
        }

        let doc_ids = text_index.intersect_layer(index_iri, layer, &unique_terms)?;
        if doc_ids.is_empty() {
            continue;
        }

        let docs = match text_index.get_layer_docs(index_iri, layer)? {
            Some(docs) => docs,
            None => {
                return Err(TextSearchError::MissingDocs {
                    index: index_iri.as_str().to_string(),
                    layer: hex::encode(layer.0),
                });
            }
        };

        for doc_id in doc_ids {
            let idx = doc_id as usize;
            if idx >= docs.subjects.len() {
                // Bitmap referenced an out-of-range doc — index
                // corruption; skip rather than fail the whole query.
                continue;
            }
            let subject = docs.subjects[idx].clone();
            let doc_length = docs.doc_lengths[idx];

            // Shadow check: if the subject has been redefined in an
            // intermediate ancestor between `layer` and `head`, drop
            // this hit.
            if is_shadowed(head, layer, &subject) {
                continue;
            }

            let score = scorer.score_doc(&term_refs, doc_length);
            hits.push(TextScoredHit {
                subject,
                score,
                defining_layer: layer.clone(),
            });
        }
    }

    // Highest score first; deterministic tie-break by
    // defining_layer bytes (a stable byte comparison) and then
    // subject IRI string.
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.defining_layer.0.cmp(&b.defining_layer.0))
            .then_with(|| a.subject.as_str().cmp(b.subject.as_str()))
    });

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::layer::{LayerBuilder, TextDoc};
    use crate::query::text::analyzer::{EnNoStem, EnStemV1};
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a minimal layer chain over a bootstrapped head, return
    /// the head Layer and the shared MemoryTextIndex handle.
    fn bootstrap_chain() -> (Arc<Layer>, Arc<crate::layer::LayerStorage>) {
        let ctx = bootstrap().unwrap();
        let head = Arc::clone(ctx.head());
        let storage = Arc::new(head.storage().clone());
        (head, storage)
    }

    /// Single-term query: matching docs surface with positive scores.
    #[test]
    fn single_term_query_matches() {
        let (head, storage) = bootstrap_chain();
        let mut b = LayerBuilder::new("l1", Some(head));
        let dummy = crate::ontology::resource::Resource::new(iri("urn:eigenius:test:r"));
        b.add_resource(dummy).unwrap();
        let l1 = Arc::new(b.build((*storage).clone()));

        let text_index = Arc::clone(&l1.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");
        let analyzer = EnStemV1::new();

        let s_a = iri("urn:eigenius:test:sa");
        let s_b = iri("urn:eigenius:test:sb");
        let toks_a = analyzer.tokenize("wal truncation under concurrent commit");
        let toks_b = analyzer.tokenize("rolling back a partial commit");
        text_index
            .extend_layer(
                &index_iri,
                l1.id(),
                "en-stem-v1",
                &[
                    TextDoc {
                        subject: &s_a,
                        tokens: &toks_a,
                    },
                    TextDoc {
                        subject: &s_b,
                        tokens: &toks_b,
                    },
                ],
            )
            .unwrap();

        let hits = run_text_search(
            &l1,
            text_index.as_ref(),
            &index_iri,
            &analyzer,
            "wal truncation",
        )
        .unwrap();

        // Only s_a matches both terms; s_b matches neither.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject.as_str(), "urn:eigenius:test:sa");
        assert!(hits[0].score > 0.0);
        assert_eq!(&hits[0].defining_layer, l1.id());
    }

    /// Multi-term AND: only docs containing ALL terms surface.
    #[test]
    fn multi_term_and_filters_correctly() {
        let (head, storage) = bootstrap_chain();
        let mut b = LayerBuilder::new("l1", Some(head));
        let dummy = crate::ontology::resource::Resource::new(iri("urn:eigenius:test:r"));
        b.add_resource(dummy).unwrap();
        let l1 = Arc::new(b.build((*storage).clone()));

        let text_index = Arc::clone(&l1.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");
        let analyzer = EnNoStem;

        let s_full = iri("urn:eigenius:test:full");
        let s_partial = iri("urn:eigenius:test:partial");
        let toks_full = analyzer.tokenize("alpha beta gamma");
        let toks_partial = analyzer.tokenize("alpha gamma");
        text_index
            .extend_layer(
                &index_iri,
                l1.id(),
                "en-no-stem",
                &[
                    TextDoc {
                        subject: &s_full,
                        tokens: &toks_full,
                    },
                    TextDoc {
                        subject: &s_partial,
                        tokens: &toks_partial,
                    },
                ],
            )
            .unwrap();

        let hits = run_text_search(
            &l1,
            text_index.as_ref(),
            &index_iri,
            &analyzer,
            "alpha beta",
        )
        .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject.as_str(), "urn:eigenius:test:full");
    }

    /// Shadow check: a subject redefined in a descendant layer
    /// drops the older hit from the search results.
    #[test]
    fn shadow_check_drops_older_layer_hit() {
        let (head, storage) = bootstrap_chain();
        let analyzer = EnNoStem;

        // L1: define s with content containing "alpha".
        let s = iri("urn:eigenius:test:s");
        let mut l1_b = LayerBuilder::new("l1", Some(head));
        let mut r = crate::ontology::resource::Resource::new(s.clone());
        // Resource has a no-op description; the indexing pipeline
        // (M3.5) tokenises property values, but for M3.3 we drive
        // the TextIndex directly with synthetic docs that pretend
        // to be tokenised property values.
        r.set(
            iri("urn:eigenius:core:description"),
            crate::ontology::resource::Value::String("alpha".into()),
        );
        l1_b.add_resource(r).unwrap();
        let l1 = Arc::new(l1_b.build((*storage).clone()));

        // L2: redefine s with different content.
        let mut l2_b = LayerBuilder::new("l2", Some(Arc::clone(&l1)));
        let mut r2 = crate::ontology::resource::Resource::new(s.clone());
        r2.set(
            iri("urn:eigenius:core:description"),
            crate::ontology::resource::Value::String("beta".into()),
        );
        l2_b.add_resource(r2).unwrap();
        let l2 = Arc::new(l2_b.build((*storage).clone()));

        let text_index = Arc::clone(&l1.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");

        // Index L1's version under the TextIndex with "alpha".
        let toks_v1 = analyzer.tokenize("alpha");
        text_index
            .extend_layer(
                &index_iri,
                l1.id(),
                "en-no-stem",
                &[TextDoc {
                    subject: &s,
                    tokens: &toks_v1,
                }],
            )
            .unwrap();
        // Index L2's version with "beta".
        let toks_v2 = analyzer.tokenize("beta");
        text_index
            .extend_layer(
                &index_iri,
                l2.id(),
                "en-no-stem",
                &[TextDoc {
                    subject: &s,
                    tokens: &toks_v2,
                }],
            )
            .unwrap();

        // Querying at L2 for "alpha" — L1's hit is shadowed by
        // L2's redefinition of s, even though L2 doesn't contain
        // "alpha".
        let hits =
            run_text_search(&l2, text_index.as_ref(), &index_iri, &analyzer, "alpha").unwrap();
        assert!(
            hits.is_empty(),
            "L1 hit on shadowed subject should be dropped, got {hits:?}"
        );

        // But querying for "beta" surfaces L2's hit.
        let hits =
            run_text_search(&l2, text_index.as_ref(), &index_iri, &analyzer, "beta").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(&hits[0].defining_layer, l2.id());
    }

    /// Empty query → empty results, no errors.
    #[test]
    fn empty_query_returns_empty() {
        let (head, _storage) = bootstrap_chain();
        let text_index = Arc::clone(&head.storage().text_index);
        let analyzer = EnStemV1::new();
        let index_iri = iri("urn:eigenius:test:ti");
        let hits = run_text_search(&head, text_index.as_ref(), &index_iri, &analyzer, "").unwrap();
        assert!(hits.is_empty());
    }

    /// Query against an Index with no contributions → empty.
    #[test]
    fn empty_index_returns_empty() {
        let (head, _storage) = bootstrap_chain();
        let text_index = Arc::clone(&head.storage().text_index);
        let analyzer = EnStemV1::new();
        let index_iri = iri("urn:eigenius:test:nonexistent");
        let hits =
            run_text_search(&head, text_index.as_ref(), &index_iri, &analyzer, "alpha").unwrap();
        assert!(hits.is_empty());
    }

    /// Analyzer mismatch surfaces as a typed error — the query is
    /// passed an analyzer with a different id than what was used
    /// to populate the index at this `(index, layer)`.
    #[test]
    fn analyzer_mismatch_surfaces_error() {
        let (head, storage) = bootstrap_chain();
        let mut b = LayerBuilder::new("l1", Some(head));
        let dummy = crate::ontology::resource::Resource::new(iri("urn:eigenius:test:r"));
        b.add_resource(dummy).unwrap();
        let l1 = Arc::new(b.build((*storage).clone()));

        let text_index = Arc::clone(&l1.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");
        let s = iri("urn:eigenius:test:s");
        let toks = vec!["alpha".to_string(), "beta".to_string()];
        text_index
            .extend_layer(
                &index_iri,
                l1.id(),
                "en-stem-v1",
                &[TextDoc {
                    subject: &s,
                    tokens: &toks,
                }],
            )
            .unwrap();

        // Try to query with the wrong analyzer.
        let wrong_analyzer = EnNoStem;
        let err = run_text_search(
            &l1,
            text_index.as_ref(),
            &index_iri,
            &wrong_analyzer,
            "alpha",
        )
        .expect_err("analyzer mismatch should fail");
        match err {
            TextSearchError::AnalyzerMismatch { indexed, query, .. } => {
                assert_eq!(indexed, "en-stem-v1");
                assert_eq!(query, "en-no-stem");
            }
            other => panic!("expected analyzer mismatch, got {other:?}"),
        }
    }

    /// Hits across multiple layers all surface; the result is
    /// sorted by descending score; chain-aware IDF gives a single
    /// baseline across the whole chain.
    #[test]
    fn multi_layer_chain_emits_all_unshadowed_hits() {
        let (head, storage) = bootstrap_chain();
        let analyzer = EnNoStem;
        let s_a = iri("urn:eigenius:test:sa");
        let s_b = iri("urn:eigenius:test:sb");

        // L1 defines s_a with "alpha alpha alpha" (high score
        // candidate — single matching term repeated several times
        // doesn't compound under binary-TF BM25, but the doc is
        // short which boosts the BM25 score).
        let mut l1_b = LayerBuilder::new("l1", Some(head));
        let r_a = crate::ontology::resource::Resource::new(s_a.clone());
        l1_b.add_resource(r_a).unwrap();
        let l1 = Arc::new(l1_b.build((*storage).clone()));

        // L2 defines s_b with longer text containing "alpha".
        let mut l2_b = LayerBuilder::new("l2", Some(Arc::clone(&l1)));
        let r_b = crate::ontology::resource::Resource::new(s_b.clone());
        l2_b.add_resource(r_b).unwrap();
        let l2 = Arc::new(l2_b.build((*storage).clone()));

        let text_index = Arc::clone(&l2.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");

        let toks_short = vec!["alpha".to_string(), "beta".to_string()];
        let toks_long = vec![
            "alpha".to_string(),
            "the".to_string(),
            "quick".to_string(),
            "brown".to_string(),
            "fox".to_string(),
        ];
        text_index
            .extend_layer(
                &index_iri,
                l1.id(),
                "en-no-stem",
                &[TextDoc {
                    subject: &s_a,
                    tokens: &toks_short,
                }],
            )
            .unwrap();
        text_index
            .extend_layer(
                &index_iri,
                l2.id(),
                "en-no-stem",
                &[TextDoc {
                    subject: &s_b,
                    tokens: &toks_long,
                }],
            )
            .unwrap();

        let hits =
            run_text_search(&l2, text_index.as_ref(), &index_iri, &analyzer, "alpha").unwrap();
        assert_eq!(hits.len(), 2);

        // The shorter doc (`s_a`) scores higher under length
        // normalisation.
        assert_eq!(hits[0].subject.as_str(), "urn:eigenius:test:sa");
        assert_eq!(hits[1].subject.as_str(), "urn:eigenius:test:sb");
        assert!(hits[0].score > hits[1].score);
    }
}
