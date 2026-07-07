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

//! Active-Index discovery helpers (D43 M2.6).
//!
//! Walks the visible chain rooted at a given head to find all
//! `core:TextIndex` and `core:VectorIndex` Resources that are
//! currently active (defined and non-shadowed). Used by five
//! call-sites in the kernel:
//!
//! 1. `LayerBuilder::build` (M3 / M5) — to know which property
//!    values to tokenise + embed.
//! 2. Query path (M3 / M5) — to find the active Index for a
//!    queried property.
//! 3. Post-Load embedding sweep (M5) — to know which properties
//!    need embedding at each layer.
//! 4. `delete_layer` (M2.7) — the GC sweep is per-layer, not
//!    per-Index, so this helper is less central there but the
//!    discovered Index list is useful for accounting.
//! 5. D25 chain consolidation (M8) — to re-extract / re-build
//!    index entries for the consolidated layer.
//!
//! Centralising the discovery here is what guarantees the five
//! call-sites agree on the same "what's active at this head" view
//! — a critical correctness invariant (the implementation-plan
//! audit flagged drift as a high-impact risk).

use crate::layer::{collect_ancestors, scan_chain, Layer};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;

/// Description of one active `core:TextIndex` Resource.
///
/// Carries enough to drive both index-side work (`extend_layer`)
/// and query-side work (`scan_term` plus analyzer-consistency
/// check): the Index's own IRI, the property it targets, and the
/// analyzer ID it was configured with.
#[derive(Debug, Clone)]
pub struct ActiveTextIndex {
    /// The TextIndex Resource's own IRI — the key under which all
    /// of its segments are stored (D43 §2.3 per-Index keying).
    pub iri: Iri,
    /// The Property the TextIndex targets.
    pub target_property: Iri,
    /// The analyzer ID configured on the Resource (e.g. `"en-stem-v1"`).
    pub analyzer: String,
}

/// Description of one active `core:VectorIndex` Resource.
///
/// Carries everything M5 (vector retrieval) and the post-Load
/// embedding sweep need to dispatch the right Embedder Component
/// and to write a self-describing segment.
#[derive(Debug, Clone)]
pub struct ActiveVectorIndex {
    /// The VectorIndex Resource's own IRI — the key under which
    /// all of its segments are stored (D43 §2.4 per-Index keying).
    pub iri: Iri,
    /// The Property the VectorIndex targets.
    pub target_property: Iri,
    /// IRI of the Embedder Component this VectorIndex commits to.
    pub model: Iri,
    /// Output dimensionality declared on the Resource.
    pub dim: u32,
    /// Distance-metric Resource IRI (one of
    /// `urn:eigenius:core:distances:cosine|l2|dot`).
    pub distance: Iri,
    /// Strategy Resource IRI (one of
    /// `urn:eigenius:core:strategies:flat|hnsw|auto`). Default
    /// `core:strategies:auto` when omitted.
    pub strategy: Iri,
    /// HNSW M parameter (default 16 when omitted).
    pub hnsw_m: u32,
    /// HNSW build-time exploration depth (default 200 when omitted).
    pub hnsw_ef_construction: u32,
    /// Embedding-policy Resource IRI (one of
    /// `urn:eigenius:core:embedding_policies:eager_on_load|lazy_on_query|manual`).
    /// Default `core:embedding_policies:eager_on_load` when omitted.
    pub embedding_policy: Iri,
}

/// Description of one active `core:ValueIndex` Resource (D65).
///
/// Carries what both index-side population (`extend_layer`) and query-side
/// lookup need: the Index's own IRI (the key its entries are stored under),
/// the property it targets, and the normalizer Resource IRI it applies.
#[derive(Debug, Clone)]
pub struct ActiveValueIndex {
    /// The ValueIndex Resource's own IRI — the key under which its entries are stored.
    pub iri: Iri,
    /// The Property the ValueIndex targets.
    pub target_property: Iri,
    /// The normalizer Resource IRI (one of
    /// `urn:eigenius:core:normalizers:{identity,lowercase,lowercase_trim}`); default
    /// `core:normalizers:identity` when omitted.
    pub normalizer: Iri,
}

/// Default normalizer applied when a ValueIndex omits `value_normalizer`.
const VALUE_NORMALIZER_DEFAULT: &str = "urn:eigenius:core:normalizers:identity";

/// Default values applied when a VectorIndex omits a recommended slot.
mod vec_defaults {
    pub const STRATEGY: &str = "urn:eigenius:core:strategies:auto";
    pub const EMBEDDING_POLICY: &str = "urn:eigenius:core:embedding_policies:eager_on_load";
    pub const HNSW_M: u32 = 16;
    pub const HNSW_EF_CONSTRUCTION: u32 = 200;
}

// ---------------- Property-value extraction helpers ----------------

/// Read an IRI-valued slot from a Resource. Returns `Some(iri)` iff
/// the Resource has the property and the value resolves to an IRI
/// (canonical `ResourceRef` form or the pre-canonical string form).
///
/// Uses `Layer::resolve` (not `get_resource`) so that Resources
/// defined in ancestor layers are visible — the Index Resource
/// being read may have been declared upstream.
fn read_iri(layer: &Layer, resource_iri: &Iri, property_iri: &str) -> Option<Iri> {
    let resource = layer.resolve(resource_iri)?;
    let prop = Iri::parse(property_iri).ok()?;
    let value = resource.get(&prop)?;
    let iri_str = value.as_iri_str()?;
    Iri::parse(iri_str).ok()
}

/// Read a string-valued slot from a Resource. See [`read_iri`] for
/// the chain-walk rationale.
fn read_string(layer: &Layer, resource_iri: &Iri, property_iri: &str) -> Option<String> {
    let resource = layer.resolve(resource_iri)?;
    let prop = Iri::parse(property_iri).ok()?;
    match resource.get(&prop)? {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Read an integer-valued slot from a Resource. See [`read_iri`]
/// for the chain-walk rationale.
fn read_u32(layer: &Layer, resource_iri: &Iri, property_iri: &str) -> Option<u32> {
    let resource = layer.resolve(resource_iri)?;
    let prop = Iri::parse(property_iri).ok()?;
    match resource.get(&prop)? {
        Value::Integer(i) => u32::try_from(*i).ok(),
        _ => None,
    }
}

// ---------------- Discovery API ----------------

/// Enumerate every `core:TextIndex` Resource active at `head`.
///
/// Uses [`scan_chain`] to find Resources whose `is_a` contains
/// `core:TextIndex`, then walks each to extract its
/// `target_property` and `text_analyzer` slots.
///
/// Resources that fail to resolve (resource backend error, missing
/// required slot) are silently skipped — the discovery is
/// best-effort; the v1 multiplicity constraint (one active
/// TextIndex per target Property per head) is enforced separately
/// at Load time, not here.
pub fn resolve_active_text_indexes(head: &Layer) -> Vec<ActiveTextIndex> {
    let class_iri = match Iri::parse(wk::TEXT_INDEX_CLASS) {
        Ok(iri) => iri,
        Err(_) => return Vec::new(),
    };
    let is_a_iri = match Iri::parse(wk::IS_A) {
        Ok(iri) => iri,
        Err(_) => return Vec::new(),
    };
    let candidates = scan_chain(head, &is_a_iri, &class_iri);
    let mut out = Vec::with_capacity(candidates.len());
    for subject in candidates {
        let target_property = match read_iri(head, &subject, wk::TARGET_PROPERTY) {
            Some(iri) => iri,
            None => continue,
        };
        let analyzer = read_string(head, &subject, wk::TEXT_ANALYZER)
            .unwrap_or_else(|| "en-stem-v1".to_string());
        out.push(ActiveTextIndex {
            iri: subject,
            target_property,
            analyzer,
        });
    }
    out
}

/// Enumerate every `core:VectorIndex` Resource active at `head`.
///
/// Same shape as [`resolve_active_text_indexes`]. Applies default
/// values from [`vec_defaults`] for omitted recommended slots so
/// downstream callers always see a fully-populated
/// [`ActiveVectorIndex`].
pub fn resolve_active_vector_indexes(head: &Layer) -> Vec<ActiveVectorIndex> {
    let class_iri = match Iri::parse(wk::VECTOR_INDEX_CLASS) {
        Ok(iri) => iri,
        Err(_) => return Vec::new(),
    };
    let is_a_iri = match Iri::parse(wk::IS_A) {
        Ok(iri) => iri,
        Err(_) => return Vec::new(),
    };
    let candidates = scan_chain(head, &is_a_iri, &class_iri);
    let mut out = Vec::with_capacity(candidates.len());
    for subject in candidates {
        let target_property = match read_iri(head, &subject, wk::TARGET_PROPERTY) {
            Some(iri) => iri,
            None => continue,
        };
        let model = match read_iri(head, &subject, wk::VEC_MODEL) {
            Some(iri) => iri,
            None => continue,
        };
        let dim = match read_u32(head, &subject, wk::VEC_DIM) {
            Some(d) => d,
            None => continue,
        };
        let distance = read_iri(head, &subject, wk::VEC_DISTANCE)
            .unwrap_or_else(|| Iri::parse("urn:eigenius:core:distances:cosine").unwrap());
        let strategy = read_iri(head, &subject, wk::VEC_STRATEGY)
            .unwrap_or_else(|| Iri::parse(vec_defaults::STRATEGY).unwrap());
        let hnsw_m = read_u32(head, &subject, wk::VEC_HNSW_M).unwrap_or(vec_defaults::HNSW_M);
        let hnsw_ef_construction = read_u32(head, &subject, wk::VEC_HNSW_EF_CONSTRUCTION)
            .unwrap_or(vec_defaults::HNSW_EF_CONSTRUCTION);
        let embedding_policy = read_iri(head, &subject, wk::VEC_EMBEDDING_POLICY)
            .unwrap_or_else(|| Iri::parse(vec_defaults::EMBEDDING_POLICY).unwrap());
        out.push(ActiveVectorIndex {
            iri: subject,
            target_property,
            model,
            dim,
            distance,
            strategy,
            hnsw_m,
            hnsw_ef_construction,
            embedding_policy,
        });
    }
    out
}

/// Enforce the v1 multiplicity constraint for TextIndex resources
/// (D43 §3.1): at most one active TextIndex per target Property.
/// Returns the first conflict found, if any.
///
/// Returns `Ok(())` when all target Properties have at most one
/// active TextIndex; returns `Err` with a description of the first
/// conflict when two TextIndexes target the same Property at the
/// same head.
pub fn verify_text_index_multiplicity(indexes: &[ActiveTextIndex]) -> Result<(), String> {
    let mut seen: std::collections::BTreeMap<&Iri, &Iri> = std::collections::BTreeMap::new();
    for active in indexes {
        if let Some(prior) = seen.get(&active.target_property) {
            return Err(format!(
                "two active TextIndex Resources target property {} at this head: {} and {}",
                active.target_property.as_str(),
                prior.as_str(),
                active.iri.as_str()
            ));
        }
        seen.insert(&active.target_property, &active.iri);
    }
    Ok(())
}

/// Enumerate every `core:ValueIndex` Resource active at `head` (D65).
///
/// Same shape as [`resolve_active_text_indexes`]: find Resources whose `is_a`
/// contains `core:ValueIndex`, then read each one's `target_property` and
/// `value_normalizer` slots (normalizer defaulting to `core:normalizers:identity`).
pub fn resolve_active_value_indexes(head: &Layer) -> Vec<ActiveValueIndex> {
    let class_iri = match Iri::parse(wk::VALUE_INDEX_CLASS) {
        Ok(iri) => iri,
        Err(_) => return Vec::new(),
    };
    let is_a_iri = match Iri::parse(wk::IS_A) {
        Ok(iri) => iri,
        Err(_) => return Vec::new(),
    };
    let candidates = scan_chain(head, &is_a_iri, &class_iri);
    let mut out = Vec::with_capacity(candidates.len());
    for subject in candidates {
        let target_property = match read_iri(head, &subject, wk::TARGET_PROPERTY) {
            Some(iri) => iri,
            None => continue,
        };
        let normalizer = read_iri(head, &subject, wk::VALUE_NORMALIZER)
            .unwrap_or_else(|| Iri::parse(VALUE_NORMALIZER_DEFAULT).expect("valid default iri"));
        out.push(ActiveValueIndex {
            iri: subject,
            target_property,
            normalizer,
        });
    }
    out
}

/// Extract the value-index entries a layer DEFINES (D65) — for each active
/// `core:ValueIndex` and each resource carrying a `String` (or `String`-array)
/// value on the index's target property, an [`OwnedValueEntry`] keyed by the
/// index IRI + the normalized value. Mirrors `extract_indexable_triples`;
/// consulted only the resources defined in this layer (`iter_resources`), so the
/// build-time pre-population is self-contained.
pub fn extract_value_entries(layer: &Layer) -> Vec<super::value_index::OwnedValueEntry> {
    use super::value_index::{normalize_value, OwnedValueEntry};
    let actives = resolve_active_value_indexes(layer);
    if actives.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (subject, resource) in layer.iter_resources() {
        for active in &actives {
            let Some(value) = resource.get(&active.target_property) else {
                continue;
            };
            let mut push = |s: &str| {
                out.push(OwnedValueEntry {
                    index: active.iri.clone(),
                    key: normalize_value(&active.normalizer, s),
                    subject: subject.clone(),
                });
            };
            match value {
                Value::String(s) => push(s),
                Value::Array(items) => {
                    for item in items {
                        if let Value::String(s) = item {
                            push(s);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Enforce the v1 multiplicity constraint for ValueIndex resources (D65): at
/// most one active ValueIndex per target Property.
pub fn verify_value_index_multiplicity(indexes: &[ActiveValueIndex]) -> Result<(), String> {
    let mut seen: std::collections::BTreeMap<&Iri, &Iri> = std::collections::BTreeMap::new();
    for active in indexes {
        if let Some(prior) = seen.get(&active.target_property) {
            return Err(format!(
                "two active ValueIndex Resources target property {} at this head: {} and {}",
                active.target_property.as_str(),
                prior.as_str(),
                active.iri.as_str()
            ));
        }
        seen.insert(&active.target_property, &active.iri);
    }
    Ok(())
}

/// Enforce the v1 multiplicity constraint for VectorIndex resources
/// (D43 §3.1): at most one active VectorIndex per target Property.
pub fn verify_vector_index_multiplicity(indexes: &[ActiveVectorIndex]) -> Result<(), String> {
    let mut seen: std::collections::BTreeMap<&Iri, &Iri> = std::collections::BTreeMap::new();
    for active in indexes {
        if let Some(prior) = seen.get(&active.target_property) {
            return Err(format!(
                "two active VectorIndex Resources target property {} at this head: {} and {}",
                active.target_property.as_str(),
                prior.as_str(),
                active.iri.as_str()
            ));
        }
        seen.insert(&active.target_property, &active.iri);
    }
    Ok(())
}

/// Suppress an unused-warning on the helper. `collect_ancestors`
/// isn't called from this module, but the discovery functions
/// transitively rely on its semantics via `scan_chain`. Re-export
/// to keep the documentation pointer honest.
#[allow(dead_code)]
fn _collect_ancestors_reachable(head: &Layer) -> std::collections::BTreeSet<crate::layer::LayerId> {
    collect_ancestors(head)
}

/// D43 §5.7 / M8.4 — one active VectorIndex Resource whose declared
/// `vec_model` no longer matches the model that produced its
/// existing segments. Triggers a chain-wide reindex against the new
/// model.
///
/// Emitted by [`detect_reindex_targets`] when the schema owner
/// commits an upgraded model under the same Index IRI (the natural
/// upgrade path: same stable IRI, fresh `vec_model` slot) — the
/// existing per-layer segments still carry the old model's vectors,
/// which the segment-model verification in the query path
/// (`top_k_subjects`) would reject as `ModelMismatch`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReindexTarget {
    /// IRI of the VectorIndex Resource that needs its segments
    /// rewritten.
    pub index_iri: Iri,
    /// The new model declared on the VectorIndex Resource at head.
    pub declared_model: Iri,
    /// The model recorded on the existing segments. v1 returns the
    /// model from the first segment found (segments under one Index
    /// are model-consistent by §5.7 invariant — the reindex itself
    /// is what introduces an intermediate inconsistency, but the
    /// invariant holds at the pre-reindex steady state).
    pub segment_model: Iri,
}

/// D43 §5.7 / M8.4 — find every active VectorIndex Resource at
/// `head` whose declared `vec_model` no longer matches the model
/// recorded on its existing segments. Each returned
/// [`ReindexTarget`] is a chain-wide reindex unit; the caller
/// (typically the commit hook) constructs a
/// [`crate::task::reindex::ReindexDriver`] per target and either
/// runs it inline or schedules it on the task executor.
///
/// Detection logic, per Index Resource:
///
/// 1. Resolve every active `core:VectorIndex` Resource at `head`
///    via [`resolve_active_vector_indexes`].
/// 2. For each, ask the VectorIndex backend for the layers under
///    that Index (`scan_index`) and probe the first chain-visible
///    segment's `model_iri`. Segments contributed by layers no
///    longer reachable from `head` are skipped — a model mismatch
///    against an orphan segment isn't actionable because the
///    segment isn't queried anyway.
/// 3. If the segment's model differs from the Index Resource's
///    declared model, emit a [`ReindexTarget`].
///
/// **Fresh VectorIndex Resources** (no segments yet at any visible
/// layer) are *not* targets — the regular post-Load sweep
/// ([`crate::task::sweep::VectorSweepDriver`]) populates those.
/// Only model upgrades against pre-existing segments need the
/// chain-wide rewrite.
///
/// Errors from the storage backend during `scan_index` are
/// propagated as `Err`; one target's failure aborts the whole
/// detection pass rather than silently dropping it — partial
/// detection would mislead the commit hook into thinking the
/// upgrade was fully observed.
pub fn detect_reindex_targets(
    head: &Layer,
) -> Result<Vec<ReindexTarget>, crate::storage::StorageError> {
    use std::collections::BTreeSet;
    let active = resolve_active_vector_indexes(head);
    if active.is_empty() {
        return Ok(Vec::new());
    }
    let reachable: BTreeSet<crate::layer::LayerId> = collect_ancestors(head);
    let backend = head.storage().vector_index.clone();

    let mut targets = Vec::new();
    for index in &active {
        // First chain-visible segment determines the recorded model.
        // Segments from non-reachable layers are skipped; if no
        // visible segment exists at all, this is a fresh Index — the
        // sweep handles it, no reindex needed.
        let mut visible_model: Option<Iri> = None;
        for layer_id in backend.scan_index(&index.iri) {
            let layer_id = layer_id?;
            if !reachable.contains(&layer_id) {
                continue;
            }
            if let Some(seg) = backend.get_segment(&index.iri, &layer_id)? {
                visible_model = Some(seg.model_iri.clone());
                break;
            }
        }
        let segment_model = match visible_model {
            Some(m) => m,
            None => continue,
        };
        if segment_model != index.model {
            targets.push(ReindexTarget {
                index_iri: index.iri.clone(),
                declared_model: index.model.clone(),
                segment_model,
            });
        }
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::Resource;
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

    /// Bootstrap a fresh chain with the core ontology loaded — required
    /// so `is_a` resolves as a Property with `data_type:
    /// resource_array` and the triple index picks it up at commit time.
    /// Returns the head Layer and a clonable LayerStorage so child
    /// builders can commit on top.
    fn bootstrap_head() -> (Arc<crate::layer::Layer>, crate::layer::LayerStorage) {
        let ctx = bootstrap().expect("bootstrap should succeed");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        (head, storage)
    }

    /// Retain only the indexes a test declared (`urn:ex:*`), dropping the
    /// bootstrap-shipped `core:description_text_index` that every real chain
    /// now carries — it targets `core:description`, a property distinct from
    /// any `urn:ex:*` these discovery/multiplicity tests exercise.
    fn ex_only(mut v: Vec<ActiveTextIndex>) -> Vec<ActiveTextIndex> {
        v.retain(|a| a.iri.as_str().starts_with("urn:ex:"));
        v
    }

    /// Two TextIndex Resources targeting different Properties at the
    /// same head — both should surface, neither shadowed.
    #[test]
    fn discovers_text_indexes_in_chain() {
        let (head, storage) = bootstrap_head();
        let mut b = LayerBuilder::new("indexes", Some(head));
        b.add_resource(make_resource(
            "urn:ex:ti_a",
            "urn:eigenius:core:TextIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::ResourceRef(iri("urn:ex:prop_a")),
                ),
                (
                    "urn:eigenius:core:text_analyzer",
                    Value::String("en-stem-v1".into()),
                ),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:ti_b",
            "urn:eigenius:core:TextIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::ResourceRef(iri("urn:ex:prop_b")),
                ),
                (
                    "urn:eigenius:core:text_analyzer",
                    Value::String("en-no-stem".into()),
                ),
            ],
        ))
        .unwrap();
        let layer = Arc::new(b.build(storage));

        let mut active = ex_only(resolve_active_text_indexes(&layer));
        active.sort_by(|a, b| a.iri.as_str().cmp(b.iri.as_str()));
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].iri.as_str(), "urn:ex:ti_a");
        assert_eq!(active[0].target_property.as_str(), "urn:ex:prop_a");
        assert_eq!(active[0].analyzer, "en-stem-v1");
        assert_eq!(active[1].iri.as_str(), "urn:ex:ti_b");
        assert_eq!(active[1].analyzer, "en-no-stem");

        // Multiplicity passes (different target properties).
        assert!(verify_text_index_multiplicity(&active).is_ok());
    }

    /// Missing `text_analyzer` defaults to `"en-stem-v1"` so callers
    /// always get a usable analyzer ID without conditional logic.
    #[test]
    fn missing_analyzer_defaults_to_en_stem_v1() {
        let (head, storage) = bootstrap_head();
        let mut b = LayerBuilder::new("indexes", Some(head));
        b.add_resource(make_resource(
            "urn:ex:ti",
            "urn:eigenius:core:TextIndex",
            vec![(
                "urn:eigenius:core:target_property",
                Value::ResourceRef(iri("urn:ex:prop")),
            )],
        ))
        .unwrap();
        let layer = Arc::new(b.build(storage));
        let active = ex_only(resolve_active_text_indexes(&layer));
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].analyzer, "en-stem-v1");
    }

    /// VectorIndex discovery populates required fields and applies
    /// defaults for omitted recommended slots.
    #[test]
    fn discovers_vector_indexes_with_defaults() {
        let (head, storage) = bootstrap_head();
        let mut b = LayerBuilder::new("indexes", Some(head));
        b.add_resource(make_resource(
            "urn:ex:vi",
            "urn:eigenius:core:VectorIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::ResourceRef(iri("urn:ex:prop")),
                ),
                (
                    "urn:eigenius:core:vec_model",
                    Value::ResourceRef(iri("urn:ex:model")),
                ),
                ("urn:eigenius:core:vec_dim", Value::Integer(256)),
            ],
        ))
        .unwrap();
        let layer = Arc::new(b.build(storage));

        let active = resolve_active_vector_indexes(&layer);
        assert_eq!(active.len(), 1);
        let v = &active[0];
        assert_eq!(v.iri.as_str(), "urn:ex:vi");
        assert_eq!(v.target_property.as_str(), "urn:ex:prop");
        assert_eq!(v.model.as_str(), "urn:ex:model");
        assert_eq!(v.dim, 256);
        // Defaults applied for omitted slots.
        assert_eq!(v.distance.as_str(), "urn:eigenius:core:distances:cosine");
        assert_eq!(v.strategy.as_str(), "urn:eigenius:core:strategies:auto");
        assert_eq!(v.hnsw_m, 16);
        assert_eq!(v.hnsw_ef_construction, 200);
        assert_eq!(
            v.embedding_policy.as_str(),
            "urn:eigenius:core:embedding_policies:eager_on_load"
        );
    }

    /// VectorIndex without required fields (missing `vec_model` or
    /// `vec_dim`) is silently skipped — defensive against partial
    /// commits during construction.
    #[test]
    fn vector_index_missing_required_fields_skipped() {
        let (head, storage) = bootstrap_head();
        let mut b = LayerBuilder::new("indexes", Some(head));
        b.add_resource(make_resource(
            "urn:ex:vi_missing_model",
            "urn:eigenius:core:VectorIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::ResourceRef(iri("urn:ex:prop")),
                ),
                ("urn:eigenius:core:vec_dim", Value::Integer(256)),
            ],
        ))
        .unwrap();
        let layer = Arc::new(b.build(storage));
        let active = resolve_active_vector_indexes(&layer);
        assert!(active.is_empty());
    }

    /// Multiplicity check rejects two TextIndexes targeting the same
    /// Property at the same head (D43 §3.1 v1 constraint).
    #[test]
    fn multiplicity_check_rejects_duplicate_target_property() {
        let (head, storage) = bootstrap_head();
        let mut b = LayerBuilder::new("indexes", Some(head));
        for name in ["urn:ex:ti_1", "urn:ex:ti_2"] {
            b.add_resource(make_resource(
                name,
                "urn:eigenius:core:TextIndex",
                vec![
                    (
                        "urn:eigenius:core:target_property",
                        Value::ResourceRef(iri("urn:ex:shared_prop")),
                    ),
                    (
                        "urn:eigenius:core:text_analyzer",
                        Value::String("en-stem-v1".into()),
                    ),
                ],
            ))
            .unwrap();
        }
        let layer = Arc::new(b.build(storage));
        let active = ex_only(resolve_active_text_indexes(&layer));
        assert_eq!(active.len(), 2);
        let err = verify_text_index_multiplicity(&active).unwrap_err();
        assert!(err.contains("urn:ex:shared_prop"), "error: {err}");
    }

    /// A shadowed TextIndex Resource (redefined in a descendant
    /// layer) is dropped from the active set — the discovery
    /// helper inherits scan_chain's shadow-check semantics.
    #[test]
    fn shadowed_text_index_is_dropped() {
        let (head, storage) = bootstrap_head();

        let mut root_b = LayerBuilder::new("v1", Some(head));
        root_b
            .add_resource(make_resource(
                "urn:ex:ti",
                "urn:eigenius:core:TextIndex",
                vec![
                    (
                        "urn:eigenius:core:target_property",
                        Value::ResourceRef(iri("urn:ex:prop_v1")),
                    ),
                    (
                        "urn:eigenius:core:text_analyzer",
                        Value::String("en-stem-v1".into()),
                    ),
                ],
            ))
            .unwrap();
        let root = Arc::new(root_b.build(storage.clone()));

        let mut child_b = LayerBuilder::new("v2", Some(Arc::clone(&root)));
        child_b
            .add_resource(make_resource(
                "urn:ex:ti",
                "urn:eigenius:core:TextIndex",
                vec![
                    (
                        "urn:eigenius:core:target_property",
                        Value::ResourceRef(iri("urn:ex:prop_v2")),
                    ),
                    (
                        "urn:eigenius:core:text_analyzer",
                        Value::String("en-no-stem".into()),
                    ),
                ],
            ))
            .unwrap();
        let child = Arc::new(child_b.build(storage));

        // At the child head, only the redefined ti is active. The root
        // version is shadowed.
        let active = ex_only(resolve_active_text_indexes(&child));
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].iri.as_str(), "urn:ex:ti");
        // The shadowed version was the v1 definition; the active one
        // is the v2 redefinition. Verify by checking the analyzer.
        assert_eq!(active[0].analyzer, "en-no-stem");
        assert_eq!(active[0].target_property.as_str(), "urn:ex:prop_v2");
    }

    // ─── D43 §5.7 / M8.4 reindex-target detection tests ────────────────

    /// Build a Resource with the given class and properties under the
    /// supplied LayerBuilder. Mirror of the local `make_resource`
    /// helper above; restated here so the reindex tests don't need to
    /// be threaded into the existing `discovers_text_indexes_in_chain`
    /// flow.
    fn add_vi(builder: &mut LayerBuilder, iri_str: &str, target_property: &str, model: &str) {
        let mut vi = Resource::new(iri(iri_str));
        vi.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(
            iri(wk::TARGET_PROPERTY),
            Value::ResourceRef(iri(target_property)),
        );
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        builder.add_resource(vi).unwrap();
    }

    fn add_doc(builder: &mut LayerBuilder, iri_str: &str, prop: &str, value: &str) {
        let mut r = Resource::new(iri(iri_str));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(iri("urn:ex:Document"))]),
        );
        r.set(iri(prop), Value::String(value.into()));
        builder.add_resource(r).unwrap();
    }

    /// Build a chain head (L0 bootstrap → L1 VI declared with model_a
    /// → L2 documents swept under model_a → L3 VI re-declared at the
    /// SAME IRI with model_b). At the L3 head the active VectorIndex
    /// declares model_b but its segments at L2 still carry model_a —
    /// the reindex-target detection must fire.
    #[test]
    fn detect_reindex_targets_fires_on_model_upgrade_at_same_iri() {
        use crate::program::embedder::{DummyEmbedder, EmbedderRegistry};
        use crate::query::vector::indexing::sweep_layer_vectors;

        let (head, storage) = bootstrap_head();
        let model_a = "urn:eigenius:embed:dummy:v1";
        let model_b = "urn:eigenius:embed:dummy:v2";
        let target_prop = "urn:ex:body";

        // L1: VI declared with model_a under a stable IRI.
        let mut l1 = LayerBuilder::new("l1", Some(head));
        add_vi(&mut l1, "urn:ex:vi", target_prop, model_a);
        let l1 = Arc::new(l1.build(storage.clone()));

        // L2: documents whose body property gets swept under model_a.
        let mut l2 = LayerBuilder::new("l2", Some(Arc::clone(&l1)));
        add_doc(&mut l2, "urn:ex:d1", target_prop, "alpha beta");
        let l2 = Arc::new(l2.build(storage.clone()));
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model_a, 8)));
        reg.register(Arc::new(DummyEmbedder::new(model_b, 8)));
        sweep_layer_vectors(&l2, &reg, None).expect("sweep under model_a");

        // L3: VI re-declared at the SAME IRI with model_b. The new
        // Resource shadows the L1 one; the segment under L2 still
        // carries model_a's vectors.
        let mut l3 = LayerBuilder::new("l3", Some(Arc::clone(&l2)));
        add_vi(&mut l3, "urn:ex:vi", target_prop, model_b);
        let l3 = Arc::new(l3.build(storage.clone()));

        let targets = detect_reindex_targets(&l3).expect("detect");
        assert_eq!(targets.len(), 1, "expected one shadow, got {targets:?}");
        assert_eq!(targets[0].index_iri.as_str(), "urn:ex:vi");
        assert_eq!(targets[0].declared_model.as_str(), model_b);
        assert_eq!(targets[0].segment_model.as_str(), model_a);
    }

    /// A freshly-declared VectorIndex with no segments yet is *not* a
    /// reindex target — the post-Load sweep populates it from the
    /// regular path. Detection must distinguish "first ever" from
    /// "model upgrade against pre-existing segments."
    #[test]
    fn detect_reindex_targets_skips_fresh_vector_index() {
        let (head, storage) = bootstrap_head();
        let mut b = LayerBuilder::new("fresh-vi", Some(head));
        add_vi(
            &mut b,
            "urn:ex:vi",
            "urn:ex:body",
            "urn:eigenius:embed:dummy:v1",
        );
        let layer = Arc::new(b.build(storage));
        let targets = detect_reindex_targets(&layer).expect("detect");
        assert!(
            targets.is_empty(),
            "fresh VectorIndex must not be a reindex target; got {targets:?}"
        );
    }

    /// Same model on both the Resource declaration and the existing
    /// segments — no upgrade in flight, no reindex needed. Catches the
    /// case where someone re-issues the same declaration (idempotent
    /// commit) and the detector would otherwise spam reindexes.
    #[test]
    fn detect_reindex_targets_skips_when_models_match() {
        use crate::program::embedder::{DummyEmbedder, EmbedderRegistry};
        use crate::query::vector::indexing::sweep_layer_vectors;

        let (head, storage) = bootstrap_head();
        let model = "urn:eigenius:embed:dummy:v1";
        let target_prop = "urn:ex:body";

        let mut l1 = LayerBuilder::new("l1", Some(head));
        add_vi(&mut l1, "urn:ex:vi", target_prop, model);
        let l1 = Arc::new(l1.build(storage.clone()));

        let mut l2 = LayerBuilder::new("l2", Some(Arc::clone(&l1)));
        add_doc(&mut l2, "urn:ex:d1", target_prop, "alpha beta");
        let l2 = Arc::new(l2.build(storage.clone()));
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(DummyEmbedder::new(model, 8)));
        sweep_layer_vectors(&l2, &reg, None).expect("sweep");

        // L3: redeclare with the SAME model. Idempotent commit.
        let mut l3 = LayerBuilder::new("l3", Some(Arc::clone(&l2)));
        add_vi(&mut l3, "urn:ex:vi", target_prop, model);
        let l3 = Arc::new(l3.build(storage.clone()));

        let targets = detect_reindex_targets(&l3).expect("detect");
        assert!(
            targets.is_empty(),
            "matching-model redeclaration must not trigger reindex; got {targets:?}"
        );
    }
}
