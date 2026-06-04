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

//! D43 §2.3 / M3.5 — text-index population during layer commit.
//!
//! [`populate_text_indexes`] runs at the end of
//! `LayerBuilder::build` (after the existing bloom and triple-index
//! pre-population) and walks the newly-built Layer's
//! `defined_iris()`, extracting string-typed property values whose
//! Property is targeted by an active `core:TextIndex` Resource. For
//! each indexed value, the active analyzer tokenises it; the
//! resulting `(subject, tokens)` pairs flow into
//! `TextIndex::extend_layer` in a single grouped call per Index.
//!
//! **Best-effort semantics.** The function ignores per-Index
//! failures the same way `LayerBuilder::build` ignores triple-index
//! failures — text retrieval is a best-effort accelerator over the
//! authoritative resource bytes, never a gating part of commit.
//! Specific failure modes (unknown analyzer, non-string property
//! value) are silently skipped here; M3.6 surfaces them at parse
//! time when the user actually queries against the affected Index.
//!
//! **Pre-populate symmetry with the triple index.** The Phase 14h
//! dual-path design (the in-memory backend gets pre-populated at
//! build time; the persistent backend writes the same entries in
//! `store_layer`'s atomic batch) carries over here: the text-index
//! pre-population at build time matches what
//! `RocksTextIndex::extend_layer` writes inside `store_layer`'s
//! `WriteBatch` (M2.7), so reads against a freshly-built but
//! not-yet-persisted layer work identically to reads after
//! restart.

use crate::layer::{resolve_active_text_indexes, ActiveTextIndex, Layer, TextDoc};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::query::text::analyzer::{registry, Analyzer};
use std::collections::BTreeMap;
use std::sync::Arc;

/// One owned (subject, tokens) pair pre-tokenisation. The borrowed
/// [`TextDoc`] form that the trait takes points into these
/// allocations.
struct OwnedTextDoc {
    subject: Iri,
    tokens: Vec<String>,
}

/// Per-Index batched contributions before the trait call.
struct IndexBatch {
    analyzer_id: String,
    docs: Vec<OwnedTextDoc>,
}

/// Walk `layer`'s defined Resources, tokenise every indexed
/// property value via the active TextIndex's analyzer, and populate
/// the text index per `(TextIndex, layer)` pair.
///
/// Called from `LayerBuilder::build` after the bloom + triple-index
/// pre-population. Returns nothing — failures are silently ignored
/// per the best-effort contract.
pub fn populate_text_indexes(layer: &Layer) {
    // 1. Discover active TextIndex Resources visible at this layer.
    let active = resolve_active_text_indexes(layer);
    if active.is_empty() {
        return;
    }

    // 2. Resolve analyzer implementations from the registry.
    // Unknown analyzers are silently skipped (a TextIndex Resource
    // declares an analyzer the kernel doesn't ship) — the indexing
    // pipeline degrades gracefully rather than blocking commit.
    let with_analyzers: Vec<(ActiveTextIndex, Arc<dyn Analyzer>)> = active
        .into_iter()
        .filter_map(|a| {
            let analyzer = registry::analyzer_for(&a.analyzer)?;
            Some((a, analyzer))
        })
        .collect();

    if with_analyzers.is_empty() {
        return;
    }

    // 3. Walk the layer's defined Resources, extracting indexable
    // property values and tokenising into a per-(TextIndex) batch.
    let mut batches: BTreeMap<Iri, IndexBatch> = BTreeMap::new();

    for subject_iri in layer.defined_iris().iter() {
        let resource = match layer.get_resource(subject_iri) {
            Some(r) => r,
            None => continue,
        };

        for (active, analyzer) in &with_analyzers {
            // Read the target property's value as a string. Non-
            // string values (number, boolean, etc.) silently
            // contribute nothing — v1 only embeds string content.
            let value = match resource.get(&active.target_property) {
                Some(v) => v,
                None => continue,
            };
            let text = match value {
                Value::String(s) => s.as_str(),
                _ => continue,
            };

            let tokens = analyzer.tokenize(text);
            if tokens.is_empty() {
                continue;
            }

            batches
                .entry(active.iri.clone())
                .or_insert_with(|| IndexBatch {
                    analyzer_id: active.analyzer.clone(),
                    docs: Vec::new(),
                })
                .docs
                .push(OwnedTextDoc {
                    subject: subject_iri.clone(),
                    tokens,
                });
        }
    }

    // 4. Issue one `extend_layer` call per Index whose contribution
    // is non-empty.
    for (index_iri, batch) in &batches {
        let docs: Vec<TextDoc<'_>> = batch
            .docs
            .iter()
            .map(|d| TextDoc {
                subject: &d.subject,
                tokens: &d.tokens,
            })
            .collect();

        // Errors are non-fatal — see best-effort note above.
        let _ = layer.storage().text_index.extend_layer(
            index_iri,
            layer.id(),
            &batch.analyzer_id,
            &docs,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::Resource;
    use crate::ontology::well_known as wk;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a Resource of the given class with the given properties.
    fn make_resource(id: &str, class_iri: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(iri(class_iri))]),
        );
        for (k, v) in props {
            r.set(iri(k), v);
        }
        r
    }

    /// Bootstrap a fresh chain so `is_a` resolves as a Property at
    /// the discovery layer's index scan time.
    fn bootstrap_chain() -> (Arc<Layer>, crate::layer::LayerStorage) {
        let ctx = bootstrap().unwrap();
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        (head, storage)
    }

    /// Define a TextIndex + indexable Resource in the same layer;
    /// the populate hook indexes the Resource's property value with
    /// the configured analyzer.
    #[test]
    fn populates_for_resources_in_same_layer_as_text_index() {
        let (head, storage) = bootstrap_chain();
        let mut b = LayerBuilder::new("test", Some(head));

        let target_prop_iri = "urn:eigenius:test:description";
        b.add_resource(make_resource(
            "urn:eigenius:test:ti",
            wk::TEXT_INDEX_CLASS,
            vec![
                (
                    wk::TARGET_PROPERTY,
                    Value::ResourceRef(iri(target_prop_iri)),
                ),
                (wk::TEXT_ANALYZER, Value::String("en-stem-v1".into())),
            ],
        ))
        .unwrap();

        b.add_resource(make_resource(
            "urn:eigenius:test:r1",
            "urn:eigenius:test:Thing",
            vec![(target_prop_iri, Value::String("running runs ran".into()))],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:eigenius:test:r2",
            "urn:eigenius:test:Thing",
            vec![(target_prop_iri, Value::String("alpha beta".into()))],
        ))
        .unwrap();

        let layer = Arc::new(b.build(storage));
        let text_index = Arc::clone(&layer.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");

        // r1 contributes "run run ran" after Porter stemming; r2
        // contributes "alpha beta".
        // Per-layer stats show 2 docs.
        let stats = text_index
            .get_layer_stats(&index_iri, layer.id())
            .unwrap()
            .unwrap();
        assert_eq!(stats.doc_count, 2);

        // "run" appears in r1 only.
        let hits: Vec<_> = text_index
            .scan_term(&index_iri, "run")
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].df, 1);

        // "alpha" appears in r2 only.
        let hits: Vec<_> = text_index
            .scan_term(&index_iri, "alpha")
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits[0].df, 1);

        // Analyzer ID was recorded.
        let analyzer = text_index
            .get_layer_analyzer(&index_iri, layer.id())
            .unwrap()
            .unwrap();
        assert_eq!(analyzer, "en-stem-v1");
    }

    /// TextIndex Resource defined in an ancestor; new layer
    /// contributes indexable Resources that the hook picks up.
    #[test]
    fn populates_from_inherited_text_index() {
        let (head, storage) = bootstrap_chain();
        let target_prop_iri = "urn:eigenius:test:title";

        // Parent layer declares the TextIndex.
        let mut parent_b = LayerBuilder::new("parent", Some(head));
        parent_b
            .add_resource(make_resource(
                "urn:eigenius:test:ti",
                wk::TEXT_INDEX_CLASS,
                vec![
                    (
                        wk::TARGET_PROPERTY,
                        Value::ResourceRef(iri(target_prop_iri)),
                    ),
                    (wk::TEXT_ANALYZER, Value::String("en-no-stem".into())),
                ],
            ))
            .unwrap();
        let parent = Arc::new(parent_b.build(storage.clone()));

        // Child layer adds an indexable Resource.
        let mut child_b = LayerBuilder::new("child", Some(Arc::clone(&parent)));
        child_b
            .add_resource(make_resource(
                "urn:eigenius:test:r",
                "urn:eigenius:test:Thing",
                vec![(target_prop_iri, Value::String("foo bar".into()))],
            ))
            .unwrap();
        let child = Arc::new(child_b.build(storage));

        let text_index = Arc::clone(&child.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");

        // Index contributions live at the child layer (where the
        // resource was defined), not the parent.
        let stats_child = text_index
            .get_layer_stats(&index_iri, child.id())
            .unwrap()
            .unwrap();
        assert_eq!(stats_child.doc_count, 1);

        // Parent layer has no docs (TextIndex was declared there,
        // but no indexable Resources were).
        assert!(text_index
            .get_layer_stats(&index_iri, parent.id())
            .unwrap()
            .is_none());
    }

    /// Resource without the target property contributes nothing.
    #[test]
    fn resources_without_target_property_dont_contribute() {
        let (head, storage) = bootstrap_chain();
        let target_prop_iri = "urn:eigenius:test:description";

        let mut b = LayerBuilder::new("test", Some(head));
        b.add_resource(make_resource(
            "urn:eigenius:test:ti",
            wk::TEXT_INDEX_CLASS,
            vec![
                (
                    wk::TARGET_PROPERTY,
                    Value::ResourceRef(iri(target_prop_iri)),
                ),
                (wk::TEXT_ANALYZER, Value::String("en-no-stem".into())),
            ],
        ))
        .unwrap();
        // Resource without `description`.
        b.add_resource(make_resource(
            "urn:eigenius:test:r",
            "urn:eigenius:test:Thing",
            vec![],
        ))
        .unwrap();

        let layer = Arc::new(b.build(storage));
        let text_index = Arc::clone(&layer.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");

        // No docs contributed (the resource had no property to index).
        assert!(text_index
            .get_layer_stats(&index_iri, layer.id())
            .unwrap()
            .is_none());
    }

    /// Non-string property value contributes nothing — silently
    /// skipped per the M3 v1 contract (v1 only embeds text).
    #[test]
    fn non_string_property_value_skipped() {
        let (head, storage) = bootstrap_chain();
        let target_prop_iri = "urn:eigenius:test:count";

        let mut b = LayerBuilder::new("test", Some(head));
        b.add_resource(make_resource(
            "urn:eigenius:test:ti",
            wk::TEXT_INDEX_CLASS,
            vec![
                (
                    wk::TARGET_PROPERTY,
                    Value::ResourceRef(iri(target_prop_iri)),
                ),
                (wk::TEXT_ANALYZER, Value::String("en-no-stem".into())),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:eigenius:test:r",
            "urn:eigenius:test:Thing",
            vec![(target_prop_iri, Value::Integer(42))],
        ))
        .unwrap();

        let layer = Arc::new(b.build(storage));
        let text_index = Arc::clone(&layer.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");

        assert!(text_index
            .get_layer_stats(&index_iri, layer.id())
            .unwrap()
            .is_none());
    }

    /// Unknown analyzer ID is silently skipped — TextIndex Resource
    /// declares an analyzer the kernel doesn't ship; pipeline
    /// degrades gracefully.
    #[test]
    fn unknown_analyzer_skipped_gracefully() {
        let (head, storage) = bootstrap_chain();
        let target_prop_iri = "urn:eigenius:test:description";

        let mut b = LayerBuilder::new("test", Some(head));
        b.add_resource(make_resource(
            "urn:eigenius:test:ti",
            wk::TEXT_INDEX_CLASS,
            vec![
                (
                    wk::TARGET_PROPERTY,
                    Value::ResourceRef(iri(target_prop_iri)),
                ),
                (
                    wk::TEXT_ANALYZER,
                    Value::String("nonexistent-analyzer".into()),
                ),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:eigenius:test:r",
            "urn:eigenius:test:Thing",
            vec![(target_prop_iri, Value::String("foo".into()))],
        ))
        .unwrap();

        // Build doesn't panic.
        let layer = Arc::new(b.build(storage));
        let text_index = Arc::clone(&layer.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");

        // The unknown analyzer caused the contribution to be
        // skipped; no docs indexed.
        assert!(text_index
            .get_layer_stats(&index_iri, layer.id())
            .unwrap()
            .is_none());
    }

    /// Two TextIndexes targeting different Properties each produce
    /// their own per-Index contribution at the same layer.
    #[test]
    fn multiple_text_indexes_each_populate_independently() {
        let (head, storage) = bootstrap_chain();
        let prop_a = "urn:eigenius:test:title";
        let prop_b = "urn:eigenius:test:body";

        let mut b = LayerBuilder::new("test", Some(head));
        b.add_resource(make_resource(
            "urn:eigenius:test:ti_a",
            wk::TEXT_INDEX_CLASS,
            vec![
                (wk::TARGET_PROPERTY, Value::ResourceRef(iri(prop_a))),
                (wk::TEXT_ANALYZER, Value::String("en-no-stem".into())),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:eigenius:test:ti_b",
            wk::TEXT_INDEX_CLASS,
            vec![
                (wk::TARGET_PROPERTY, Value::ResourceRef(iri(prop_b))),
                (wk::TEXT_ANALYZER, Value::String("en-stem-v1".into())),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:eigenius:test:r",
            "urn:eigenius:test:Thing",
            vec![
                (prop_a, Value::String("Title Words".into())),
                (prop_b, Value::String("running quickly".into())),
            ],
        ))
        .unwrap();

        let layer = Arc::new(b.build(storage));
        let text_index = Arc::clone(&layer.storage().text_index);

        let ti_a = iri("urn:eigenius:test:ti_a");
        let ti_b = iri("urn:eigenius:test:ti_b");

        // Each TextIndex has its own 1-doc contribution.
        assert_eq!(
            text_index
                .get_layer_stats(&ti_a, layer.id())
                .unwrap()
                .unwrap()
                .doc_count,
            1
        );
        assert_eq!(
            text_index
                .get_layer_stats(&ti_b, layer.id())
                .unwrap()
                .unwrap()
                .doc_count,
            1
        );

        // Analyzers recorded distinctly.
        assert_eq!(
            text_index
                .get_layer_analyzer(&ti_a, layer.id())
                .unwrap()
                .as_deref(),
            Some("en-no-stem")
        );
        assert_eq!(
            text_index
                .get_layer_analyzer(&ti_b, layer.id())
                .unwrap()
                .as_deref(),
            Some("en-stem-v1")
        );
    }

    /// End-to-end: indexing pipeline populates the index, and
    /// `run_text_search` retrieves the expected hits — proves the
    /// LayerBuilder hook composes with M3.3's orchestrator.
    #[test]
    fn end_to_end_index_and_search() {
        use crate::query::text::analyzer::EnStemV1;
        use crate::query::text::search::run_text_search;

        let (head, storage) = bootstrap_chain();
        let target_prop_iri = "urn:eigenius:test:description";

        let mut b = LayerBuilder::new("test", Some(head));
        b.add_resource(make_resource(
            "urn:eigenius:test:ti",
            wk::TEXT_INDEX_CLASS,
            vec![
                (
                    wk::TARGET_PROPERTY,
                    Value::ResourceRef(iri(target_prop_iri)),
                ),
                (wk::TEXT_ANALYZER, Value::String("en-stem-v1".into())),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:eigenius:test:r1",
            "urn:eigenius:test:Thing",
            vec![(
                target_prop_iri,
                Value::String("WAL truncation under concurrent commit".into()),
            )],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:eigenius:test:r2",
            "urn:eigenius:test:Thing",
            vec![(
                target_prop_iri,
                Value::String("rolling back a partial commit".into()),
            )],
        ))
        .unwrap();

        let layer = Arc::new(b.build(storage));
        let text_index = Arc::clone(&layer.storage().text_index);
        let index_iri = iri("urn:eigenius:test:ti");
        let analyzer = EnStemV1::new();

        let hits = run_text_search(
            &layer,
            text_index.as_ref(),
            &index_iri,
            &analyzer,
            "wal truncation",
        )
        .unwrap();

        // Only r1 contains both "wal" and "truncation".
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject.as_str(), "urn:eigenius:test:r1");
        assert!(hits[0].score > 0.0);
    }
}
