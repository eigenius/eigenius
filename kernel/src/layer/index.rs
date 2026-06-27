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

//! Per-layer triple index for EigenQL read acceleration (Phase 14h / D23 §5.9).
//!
//! The index stores `(predicate, object, subject, layer)` tuples for every
//! IRI-valued property in every layer. EigenQL patterns of the shape
//! `MATCH ?x : Class { is_a = Class }` (where the predicate's `data_type`
//! is `resource` or `resource_array`) become a single prefix scan against
//! the index, deduplicated against the head's chain via per-layer blooms.
//!
//! Two physical orderings persist:
//! - `idx_pos:<p>:<o>:<s>:<layer>` — read path (one prefix scan per query)
//! - `idx_layer:<layer>:<p>:<o>:<s>` — GC path (one prefix scan per layer drop)
//!
//! Both use length-prefixed keys (4-byte big-endian `u32` length followed
//! by raw bytes per IRI segment; layer is a fixed 32 bytes). The encoder
//! lives in [`index_keys`].
//!
//! See `docs/design/phase-14h-indexed-reads.md` for the full design.

use crate::layer::{Layer, LayerId};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::storage::StorageError;
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, RwLock};

/// Resolve every chain-resident resource whose `is_a` *directly* contains one of
/// `metaclasses`, without materialising the whole chain (`iter_all_resources`). Each
/// matching subject is resolved to its merged top view — overrides win, tombstoned
/// IRIs drop out — and deduplicated across metaclasses.
///
/// This is the index-driven replacement for the "scan the whole chain to find the
/// handful of resources of type X" anti-pattern. Cost scales with the number of
/// matching resources, not chain size — essential now that large sources (domain
/// lexica, UMLS, and further knowledge-graph content) live on the interactive chain.
///
/// Each chain layer's content is discoverable via **its own storage** in one of two
/// states:
/// 1. **Stored** (committed / loaded from disk) → its `is_a` triples are in that
///    storage's triple index (populated at `store_layer`). Found via
///    `scan_predicate_object`.
/// 2. **In-flight** (built this session, not yet stored) → not in the index, but its
///    resources are staged in that storage's `pending`. Found by reading the staged
///    entry for the layer.
///
/// So we walk the chain and consult each layer's own storage — deduping triple indexes
/// by `Arc` identity, so the common case (one shared storage for the whole chain) scans
/// once, while still covering chains whose layers were built on separate storages (some
/// tests). Discovered subjects are then resolved through the head, which yields the
/// merged top view AND filters out any subject not reachable from this chain (the
/// triple index is DAG-wide, so it may surface subjects from other branches). No
/// per-layer persisted artifact, no whole-chain materialisation.
///
/// `is_a` must be an indexable predicate (`data_type = resource_array`) for step 1 —
/// true on any core-rooted chain; step 2 covers in-flight content regardless.
pub fn resolve_typed_resources(layer: &Layer, metaclasses: &[&str]) -> Vec<Arc<Resource>> {
    let candidates = typed_resource_iris(layer, metaclasses);
    // Resolve each candidate through the head: merged top view + filters to this chain.
    let mut out: Vec<Arc<Resource>> = Vec::with_capacity(candidates.len());
    for subject in candidates {
        if let Some(resource) = layer.resolve(&subject) {
            out.push(resource);
        }
    }
    out
}

/// Discover the IRIs of every chain-resident resource whose `is_a` *directly* contains
/// one of `metaclasses`, **without resolving any bodies** — the cheap first half of
/// [`resolve_typed_resources`]. Returns subjects as found in each layer's own storage
/// (triple index for stored layers, `pending` for in-flight ones), deduped, but NOT yet
/// filtered to this chain (the caller resolves through the head to do that).
///
/// Exposed so callers that only need a *subset* of the matches (e.g. short-name
/// resolution scoped to an imported namespace prefix) can filter the IRIs first and
/// resolve only the survivors — keeping body materialisation O(survivors) rather than
/// O(all matches). See the indexability/staging contract on [`resolve_typed_resources`].
pub fn typed_resource_iris(layer: &Layer, metaclasses: &[&str]) -> BTreeSet<Iri> {
    let mut candidates: BTreeSet<Iri> = BTreeSet::new();
    let Ok(is_a) = Iri::parse(wk::IS_A) else {
        return candidates;
    };
    let metaclass_iris: Vec<Iri> = metaclasses
        .iter()
        .filter_map(|m| Iri::parse(m).ok())
        .collect();

    // Triple indexes already scanned, by `Arc` identity — a shared-storage chain scans
    // once rather than once per layer.
    let mut scanned: Vec<Arc<dyn TripleIndex>> = Vec::new();
    let mut current: Option<&Layer> = Some(layer);
    let mut visited: BTreeSet<LayerId> = BTreeSet::new();
    while let Some(l) = current {
        if !visited.insert(l.id().clone()) {
            break;
        }
        let storage = l.storage();

        // Stored content — that storage's triple index (once per distinct index).
        let triple_index = storage.triple_index.clone();
        if !scanned.iter().any(|s| Arc::ptr_eq(s, &triple_index)) {
            scanned.push(triple_index.clone());
            for object in &metaclass_iris {
                for (subject, _defining_layer) in
                    triple_index.scan_predicate_object(&is_a, object).flatten()
                {
                    candidates.insert(subject);
                }
            }
        }

        // In-flight content — that storage's staged entry for this layer.
        {
            let pending = storage.pending.read().expect("pending stage poisoned");
            if let Some(staged) = pending.get(l.id()) {
                for (subject, resource) in staged {
                    if metaclass_iris.iter().any(|mc| resource.is_instance_of(mc)) {
                        candidates.insert(subject.clone());
                    }
                }
            }
        }

        current = l.parent().map(|p| p.as_ref());
    }

    candidates
}

/// A single subject-predicate-object triple, borrowed from a `Resource`'s
/// property values at indexing time. All three positions are IRIs in v1
/// (literal-valued properties are skipped — see the indexability rule in
/// the design doc).
#[derive(Debug, Clone, Copy)]
pub struct Triple<'a> {
    pub subject: &'a Iri,
    pub predicate: &'a Iri,
    pub object: &'a Iri,
}

/// Counters reported by [`TripleIndex::stats`]. Implementations may report
/// zero for fields they don't track.
#[derive(Debug, Default, Clone, Copy)]
pub struct IndexStats {
    /// Live triples (sum of `idx_pos:` entries).
    pub triples: u64,
    /// Distinct layers contributing entries.
    pub layers: u64,
    /// Total `scan_predicate_object` calls served (cumulative).
    pub scans: u64,
    /// Cumulative entries returned from `scan_predicate_object`.
    pub entries_returned: u64,
}

/// Per-layer triple index — the storage trait Phase 14h's read path
/// consults.
///
/// **Storage shape (per-layer, globally scannable).** Index entries embed
/// the defining `LayerId` as the trailing key segment of the forward
/// (`idx_pos`) ordering and as the leading segment of the reverse
/// (`idx_layer`) ordering. A query at head `H` does one global prefix scan
/// on `(predicate, object)`, filters results to layers in `H`'s chain,
/// and shadow-checks each surviving subject against the per-layer blooms
/// — same dedup mechanic `Layer::resolve` already uses.
///
/// **Atomic with `store_layer`.** RocksDB-backed implementations write
/// index entries inside the same `WriteBatch` that persists the layer's
/// resources, blooms, and topology — partial drift is impossible.
/// In-memory implementations write under their existing lock.
///
/// **GC integration.** When a layer is swept (Phase 14f), `drop_layer`
/// removes both orderings' entries for that layer in one atomic operation.
pub trait TripleIndex: Send + Sync {
    /// Insert all triples that the given layer defines. Called by the
    /// commit path after the layer's content is materialised. Idempotent
    /// by `(layer, p, o, s)` — re-inserting a triple is a no-op.
    fn extend_layer(&self, layer: &LayerId, triples: &[Triple<'_>]) -> Result<(), StorageError>;

    /// Drop every entry contributed by `layer` from both orderings.
    /// Called by GC's `delete_layer`. No-op if the layer has no entries.
    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError>;

    /// Iterate `(subject, defining_layer)` pairs matching `(p, o)`,
    /// across the entire DAG. Caller filters by chain membership and
    /// shadow-checks via the per-layer bloom cache.
    ///
    /// Yields `Result` per item so streaming backends can surface
    /// transient errors mid-iteration. The in-memory implementation
    /// always yields `Ok`.
    fn scan_predicate_object<'a>(
        &'a self,
        p: &Iri,
        o: &Iri,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + 'a>;

    /// Snapshot of operational counters.
    fn stats(&self) -> IndexStats;
}

/// Indexability rule (D23 §5.9 + Phase 14h plan, Q1).
///
/// A `(subject, predicate, object)` triple is indexable iff `predicate`'s
/// `Property.data_type` resolves to `urn:eigenius:core:resource` or
/// `urn:eigenius:core:resource_array` at the layer being inspected. Both
/// the write path (`extract_indexable_triples` at commit time) and the
/// read path (the query planner deciding whether to use the index for a
/// given pattern) call this so the two sides agree by construction.
///
/// Returns `false` when the predicate's def can't be resolved or has no
/// `data_type` field — same posture as the validator: undefined props
/// silently bypass the index without erroring.
pub fn is_indexable_predicate(layer: &Layer, predicate: &Iri) -> bool {
    let data_type_prop = match Iri::parse(wk::DATA_TYPE_PROP) {
        Ok(iri) => iri,
        Err(_) => return false,
    };
    let prop_def = match layer.resolve(predicate) {
        Some(def) => def,
        None => return false,
    };
    // `as_iri_str` accepts both `Value::String` (pre-canonicalisation
    // shape) and `Value::ResourceRef` (post-canonicalisation shape).
    // Using `as_str` here was a pre-existing bug that broke the
    // index for every chain that round-tripped through
    // `canonicalise_resource_refs` — i.e., every production chain.
    let data_type = match prop_def.get(&data_type_prop).and_then(|v| v.as_iri_str()) {
        Some(t) => t,
        None => return false,
    };
    data_type == wk::RESOURCE || data_type == wk::RESOURCE_ARRAY
}

/// Owned form of [`Triple`] — emitted by [`extract_indexable_triples`] so
/// callers can hold the triple set without lifetime entanglement to a
/// `Layer` they no longer borrow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedTriple {
    pub subject: Iri,
    pub predicate: Iri,
    pub object: Iri,
}

impl OwnedTriple {
    /// Borrow as the `Triple<'_>` that [`TripleIndex::extend_layer`] takes.
    pub fn as_borrowed(&self) -> Triple<'_> {
        Triple {
            subject: &self.subject,
            predicate: &self.predicate,
            object: &self.object,
        }
    }
}

/// Walk every resource defined in `layer` and yield the indexable
/// triples it contributes (D23 §5.9 / Phase 14h).
///
/// Indexability is gated by [`is_indexable_predicate`]: only properties
/// whose `data_type` is `resource` or `resource_array` produce entries.
/// `resource_array` values unpack to one triple per element. Object
/// values that aren't valid IRI strings (or that can't be parsed) are
/// silently skipped — the index is best-effort, never blocks a commit.
///
/// Called from the storage layer's `store_layer` path so the resulting
/// triples join the same atomic batch as the layer's resource bytes,
/// blooms, and topology entries (D23 §6.3). The function takes a
/// `&Layer` (not just a list of resources) because the indexability
/// rule consults the predicate's `Property.data_type` definition,
/// which may live in a parent layer.
pub fn extract_indexable_triples(layer: &Layer) -> Vec<OwnedTriple> {
    let data_type_prop = match Iri::parse(wk::DATA_TYPE_PROP) {
        Ok(iri) => iri,
        Err(_) => return Vec::new(),
    };

    let mut triples = Vec::new();
    for (subject_iri, resource) in layer.iter_resources() {
        for (predicate_iri, value) in resource.properties() {
            // Inline the indexability check rather than calling
            // `is_indexable_predicate` so we resolve each predicate def
            // exactly once per resource per layer, not twice.
            let prop_def = match layer.resolve(predicate_iri) {
                Some(def) => def,
                None => continue,
            };
            // `as_iri_str` covers both `Value::String` and
            // `Value::ResourceRef` shapes — see the matching comment in
            // `is_indexable_predicate`.
            let data_type = match prop_def.get(&data_type_prop).and_then(|v| v.as_iri_str()) {
                Some(t) => t,
                None => continue,
            };
            let push_iri_value = |triples: &mut Vec<OwnedTriple>, raw: &str| {
                if let Ok(object) = Iri::parse(raw) {
                    triples.push(OwnedTriple {
                        subject: subject_iri.clone(),
                        predicate: predicate_iri.clone(),
                        object,
                    });
                }
            };
            match data_type {
                wk::RESOURCE => match value {
                    Value::String(s) => push_iri_value(&mut triples, s),
                    Value::ResourceRef(iri) => triples.push(OwnedTriple {
                        subject: subject_iri.clone(),
                        predicate: predicate_iri.clone(),
                        object: iri.clone(),
                    }),
                    _ => {}
                },
                wk::RESOURCE_ARRAY => {
                    if let Value::Array(items) = value {
                        for item in items {
                            match item {
                                Value::String(s) => push_iri_value(&mut triples, s),
                                Value::ResourceRef(iri) => triples.push(OwnedTriple {
                                    subject: subject_iri.clone(),
                                    predicate: predicate_iri.clone(),
                                    object: iri.clone(),
                                }),
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    triples
}

/// In-memory `TripleIndex` for tests, the in-memory bootstrap path, and
/// the `MemoryPersistentBackend` fixture. Production deployments use
/// the RocksDB-backed implementation that lands in commit 2.
///
/// Stores both orderings as sorted `BTreeSet<Vec<u8>>` of length-prefixed
/// keys. `scan_predicate_object` materialises matching entries into a
/// `Vec` because the inner `RwLock` can't be held across the iterator's
/// lifetime; for in-memory workloads the materialisation cost is
/// negligible.
pub struct MemoryTripleIndex {
    inner: RwLock<MemoryTripleIndexState>,
}

struct MemoryTripleIndexState {
    /// Forward keys: `pos_key(p, o, s, layer)`.
    pos: BTreeSet<Vec<u8>>,
    /// Reverse keys: `layer_key(layer, p, o, s)`.
    layer: BTreeSet<Vec<u8>>,
    /// Distinct layers represented in the index.
    layers: BTreeSet<LayerId>,
    /// Cumulative scan + return counters.
    scans: u64,
    entries_returned: u64,
}

impl MemoryTripleIndex {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(MemoryTripleIndexState {
                pos: BTreeSet::new(),
                layer: BTreeSet::new(),
                layers: BTreeSet::new(),
                scans: 0,
                entries_returned: 0,
            }),
        }
    }
}

impl Default for MemoryTripleIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl TripleIndex for MemoryTripleIndex {
    fn extend_layer(&self, layer: &LayerId, triples: &[Triple<'_>]) -> Result<(), StorageError> {
        if triples.is_empty() {
            return Ok(());
        }
        let mut state = self.inner.write().expect("MemoryTripleIndex poisoned");
        state.layers.insert(layer.clone());
        for t in triples {
            let pos = index_keys::pos_key(t.predicate, t.object, t.subject, layer);
            let lay = index_keys::layer_key(layer, t.predicate, t.object, t.subject);
            state.pos.insert(pos);
            state.layer.insert(lay);
        }
        Ok(())
    }

    fn drop_layer(&self, layer: &LayerId) -> Result<(), StorageError> {
        let mut state = self.inner.write().expect("MemoryTripleIndex poisoned");

        // Walk the reverse index for this layer to find every (p, o, s)
        // it contributed; remove the matching forward entries; then
        // remove the reverse entries themselves.
        let prefix = index_keys::layer_prefix(layer);
        let to_remove: Vec<Vec<u8>> = state
            .layer
            .range(prefix.clone()..)
            .take_while(|k| k.starts_with(&prefix))
            .cloned()
            .collect();

        for lay_key in &to_remove {
            let (p, o, s) = index_keys::decode_layer_key(lay_key)
                .expect("MemoryTripleIndex stored a malformed reverse key");
            let pos_key = index_keys::pos_key(&p, &o, &s, layer);
            state.pos.remove(&pos_key);
            state.layer.remove(lay_key);
        }
        state.layers.remove(layer);
        Ok(())
    }

    fn scan_predicate_object<'a>(
        &'a self,
        p: &Iri,
        o: &Iri,
    ) -> Box<dyn Iterator<Item = Result<(Iri, LayerId), StorageError>> + 'a> {
        let prefix = index_keys::pos_prefix(p, o);
        let mut results = Vec::new();
        {
            let mut state = self.inner.write().expect("MemoryTripleIndex poisoned");
            state.scans += 1;
            for key in state
                .pos
                .range(prefix.clone()..)
                .take_while(|k| k.starts_with(&prefix))
            {
                match index_keys::decode_pos_key(key) {
                    Ok((_, _, s, layer)) => results.push(Ok((s, layer))),
                    Err(e) => results.push(Err(StorageError::Internal(format!(
                        "MemoryTripleIndex decode error: {e}"
                    )))),
                }
            }
            state.entries_returned += results.len() as u64;
        }
        Box::new(results.into_iter())
    }

    fn stats(&self) -> IndexStats {
        let state = self.inner.read().expect("MemoryTripleIndex poisoned");
        IndexStats {
            triples: state.pos.len() as u64,
            layers: state.layers.len() as u64,
            scans: state.scans,
            entries_returned: state.entries_returned,
        }
    }
}

/// Length-prefixed key encoders for the two physical orderings.
///
/// Each variable-length segment (an IRI's UTF-8 bytes) is preceded by a
/// 4-byte big-endian length. The fixed-length 32-byte `LayerId` carries
/// no prefix — its position in the key is unambiguous.
///
/// Centralised here so the in-memory and RocksDB implementations agree
/// on byte-for-byte layout without duplicating logic.
pub mod index_keys {
    use crate::layer::LayerId;
    use crate::ontology::iri::Iri;

    fn write_segment(out: &mut Vec<u8>, segment: &[u8]) {
        let len: u32 = segment
            .len()
            .try_into()
            .expect("IRI segment exceeds u32::MAX bytes");
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(segment);
    }

    fn read_segment(buf: &[u8], pos: usize) -> Result<(&[u8], usize), String> {
        if pos + 4 > buf.len() {
            return Err(format!(
                "truncated length prefix at pos {pos} (buf len {})",
                buf.len()
            ));
        }
        let len = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        let start = pos + 4;
        let end = start + len;
        if end > buf.len() {
            return Err(format!(
                "truncated segment of length {len} at pos {start} (buf len {})",
                buf.len()
            ));
        }
        Ok((&buf[start..end], end))
    }

    /// `idx_pos:<p>:<o>:<s>:<layer>` — read-path key.
    pub fn pos_key(p: &Iri, o: &Iri, s: &Iri, layer: &LayerId) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(32 + p.as_str().len() + o.as_str().len() + s.as_str().len() + 16);
        write_segment(&mut out, p.as_str().as_bytes());
        write_segment(&mut out, o.as_str().as_bytes());
        write_segment(&mut out, s.as_str().as_bytes());
        out.extend_from_slice(&layer.0);
        out
    }

    /// Prefix matching every entry for a given `(p, o)` across all
    /// subjects and layers.
    pub fn pos_prefix(p: &Iri, o: &Iri) -> Vec<u8> {
        let mut out = Vec::with_capacity(p.as_str().len() + o.as_str().len() + 8);
        write_segment(&mut out, p.as_str().as_bytes());
        write_segment(&mut out, o.as_str().as_bytes());
        out
    }

    /// `idx_layer:<layer>:<p>:<o>:<s>` — GC-path reverse key.
    pub fn layer_key(layer: &LayerId, p: &Iri, o: &Iri, s: &Iri) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(32 + p.as_str().len() + o.as_str().len() + s.as_str().len() + 12);
        out.extend_from_slice(&layer.0);
        write_segment(&mut out, p.as_str().as_bytes());
        write_segment(&mut out, o.as_str().as_bytes());
        write_segment(&mut out, s.as_str().as_bytes());
        out
    }

    /// Prefix matching every entry contributed by a given layer.
    pub fn layer_prefix(layer: &LayerId) -> Vec<u8> {
        layer.0.to_vec()
    }

    /// Decode a forward `(p, o, s, layer)` key.
    pub fn decode_pos_key(key: &[u8]) -> Result<(Iri, Iri, Iri, LayerId), String> {
        let (p_bytes, pos) = read_segment(key, 0)?;
        let (o_bytes, pos) = read_segment(key, pos)?;
        let (s_bytes, pos) = read_segment(key, pos)?;
        if pos + 32 != key.len() {
            return Err(format!(
                "expected 32-byte LayerId trailer; got {} bytes at pos {pos}",
                key.len() - pos
            ));
        }
        let mut layer_bytes = [0u8; 32];
        layer_bytes.copy_from_slice(&key[pos..pos + 32]);
        let p = Iri::parse(std::str::from_utf8(p_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("predicate IRI: {e}"))?;
        let o = Iri::parse(std::str::from_utf8(o_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("object IRI: {e}"))?;
        let s = Iri::parse(std::str::from_utf8(s_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("subject IRI: {e}"))?;
        Ok((p, o, s, LayerId(layer_bytes)))
    }

    /// Decode a reverse `(layer, p, o, s)` key (returns just `(p, o, s)`
    /// — caller already knows the layer).
    pub fn decode_layer_key(key: &[u8]) -> Result<(Iri, Iri, Iri), String> {
        if key.len() < 32 {
            return Err(format!(
                "reverse key shorter than 32-byte LayerId prefix: {} bytes",
                key.len()
            ));
        }
        let pos = 32;
        let (p_bytes, pos) = read_segment(key, pos)?;
        let (o_bytes, pos) = read_segment(key, pos)?;
        let (s_bytes, pos) = read_segment(key, pos)?;
        if pos != key.len() {
            return Err(format!(
                "trailing {} bytes after reverse key segments",
                key.len() - pos
            ));
        }
        let p = Iri::parse(std::str::from_utf8(p_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("predicate IRI: {e}"))?;
        let o = Iri::parse(std::str::from_utf8(o_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("object IRI: {e}"))?;
        let s = Iri::parse(std::str::from_utf8(s_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("subject IRI: {e}"))?;
        Ok((p, o, s))
    }

    // ---- D65 exact value index ----
    //
    // Keyed by the `core:ValueIndex` Resource IRI + the normalized string key
    // (an arbitrary value, not an IRI), mapping to `(subject, layer)`. Same
    // length-prefixed segment encoding as the triple index; the RocksDB backend
    // adds its own table prefixes (`vidx_pos:` / `vidx_layer:`) so these never
    // collide with the triple index's `idx_*` keyspace.

    /// `<index>:<key>:<subject>:<layer>` — value-index read-path key.
    pub fn value_pos_key(index: &Iri, key: &str, subject: &Iri, layer: &LayerId) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(32 + index.as_str().len() + key.len() + subject.as_str().len() + 16);
        write_segment(&mut out, index.as_str().as_bytes());
        write_segment(&mut out, key.as_bytes());
        write_segment(&mut out, subject.as_str().as_bytes());
        out.extend_from_slice(&layer.0);
        out
    }

    /// Prefix matching every value-index entry for a given `(index, key)`.
    pub fn value_pos_prefix(index: &Iri, key: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(index.as_str().len() + key.len() + 8);
        write_segment(&mut out, index.as_str().as_bytes());
        write_segment(&mut out, key.as_bytes());
        out
    }

    /// `<layer>:<index>:<key>:<subject>` — value-index GC-path reverse key.
    pub fn value_layer_key(layer: &LayerId, index: &Iri, key: &str, subject: &Iri) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(32 + index.as_str().len() + key.len() + subject.as_str().len() + 12);
        out.extend_from_slice(&layer.0);
        write_segment(&mut out, index.as_str().as_bytes());
        write_segment(&mut out, key.as_bytes());
        write_segment(&mut out, subject.as_str().as_bytes());
        out
    }

    /// Decode a forward value-index `(index, key, subject, layer)` key.
    pub fn decode_value_pos_key(key: &[u8]) -> Result<(Iri, String, Iri, LayerId), String> {
        let (index_bytes, pos) = read_segment(key, 0)?;
        let (key_bytes, pos) = read_segment(key, pos)?;
        let (s_bytes, pos) = read_segment(key, pos)?;
        if pos + 32 != key.len() {
            return Err(format!(
                "expected 32-byte LayerId trailer; got {} bytes at pos {pos}",
                key.len() - pos
            ));
        }
        let mut layer_bytes = [0u8; 32];
        layer_bytes.copy_from_slice(&key[pos..pos + 32]);
        let index = Iri::parse(std::str::from_utf8(index_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("index IRI: {e}"))?;
        let key_str = std::str::from_utf8(key_bytes)
            .map_err(|e| e.to_string())?
            .to_string();
        let subject = Iri::parse(std::str::from_utf8(s_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("subject IRI: {e}"))?;
        Ok((index, key_str, subject, LayerId(layer_bytes)))
    }

    /// Decode a reverse value-index `(layer, index, key, subject)` key
    /// (returns `(index, key, subject)` — caller already knows the layer).
    pub fn decode_value_layer_key(key: &[u8]) -> Result<(Iri, String, Iri), String> {
        if key.len() < 32 {
            return Err(format!(
                "reverse value key shorter than 32-byte LayerId prefix: {} bytes",
                key.len()
            ));
        }
        let pos = 32;
        let (index_bytes, pos) = read_segment(key, pos)?;
        let (key_bytes, pos) = read_segment(key, pos)?;
        let (s_bytes, pos) = read_segment(key, pos)?;
        if pos != key.len() {
            return Err(format!(
                "trailing {} bytes after reverse value key segments",
                key.len() - pos
            ));
        }
        let index = Iri::parse(std::str::from_utf8(index_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("index IRI: {e}"))?;
        let key_str = std::str::from_utf8(key_bytes)
            .map_err(|e| e.to_string())?
            .to_string();
        let subject = Iri::parse(std::str::from_utf8(s_bytes).map_err(|e| e.to_string())?)
            .map_err(|e| format!("subject IRI: {e}"))?;
        Ok((index, key_str, subject))
    }
}

/// Collect every ancestor `LayerId` reachable from `head` via parent
/// pointers, including `head` itself.
///
/// Used by [`scan_chain`] to filter index candidates to layers in the
/// head's chain. The cost is one BFS over the in-memory `Arc<Layer>`
/// topology — no backend reads. For typical workloads (10–50 layer
/// chains, occasional Phase 14e merges with a few extra parents) this
/// is microseconds.
pub fn collect_ancestors(head: &Layer) -> BTreeSet<LayerId> {
    let mut visited = BTreeSet::<LayerId>::new();
    visited.insert(head.id().clone());
    let mut queue: VecDeque<Arc<Layer>> = VecDeque::new();
    for parent in head.parents() {
        queue.push_back(Arc::clone(parent));
    }
    while let Some(layer) = queue.pop_front() {
        if !visited.insert(layer.id().clone()) {
            continue;
        }
        for parent in layer.parents() {
            queue.push_back(Arc::clone(parent));
        }
    }
    visited
}

/// Bloom-walk shadow check: is `subject`'s visibility modified by any
/// ancestor of `head` other than `defining_layer` itself?
///
/// Walks `head` and its ancestors via parent pointers; for each visited
/// layer (skipping `defining_layer`) consults the per-layer shadowing
/// bloom and falls through to a tombstone-or-definition check on a
/// positive bloom. The first confirmed hit returns `true`.
///
/// **Shadowing events.** Two kinds of higher-up changes shadow the
/// candidate `(subject, defining_layer)`:
/// - **Redefinition** — a higher layer defines `subject` with a body.
///   `Layer::resolve(subject)` from `head` would return that body
///   instead of `defining_layer`'s.
/// - **Tombstone** (D20 §6.2 / §6.3, 15g step 3) — a higher layer
///   tombstones `subject`. `Layer::resolve(subject)` from `head`
///   short-circuits at the tombstone and returns `None`.
///
/// Either case means the indexed entry at `defining_layer` is not
/// observable from `head`, so the EigenQL `scan_chain` caller drops it.
/// The per-layer shadowing bloom (`BloomFilter::for_layer`) covers
/// both kinds of changes — built from `defined ∪ tombstoned` —
/// so the bloom-skip fast path is safe to use here too.
///
/// **Multi-parent care.** With Phase 14e merges, the walk may visit
/// layers parallel to `defining_layer` (e.g., the other branch of a
/// merge). In valid trivial merges the parallel branch's contributions
/// are unioned into the merge layer's `defined_iris`; the shadow
/// check correctly drops the older entry as "shadowed by the merge"
/// even though the underlying resource value is identical. The
/// dedup outcome (one subject in the result set) is correct in either
/// reading.
pub fn is_shadowed(head: &Layer, defining_layer: &LayerId, subject: &Iri) -> bool {
    if head.id() == defining_layer {
        // Candidate is at head itself — nothing above to shadow it.
        return false;
    }
    // Probe head, then BFS its parents via Arc clones.
    if layer_changes_visibility(head, subject) {
        return true;
    }
    let mut visited = BTreeSet::<LayerId>::new();
    visited.insert(head.id().clone());
    let mut queue: VecDeque<Arc<Layer>> = VecDeque::new();
    for parent in head.parents() {
        queue.push_back(Arc::clone(parent));
    }
    while let Some(layer) = queue.pop_front() {
        if !visited.insert(layer.id().clone()) {
            continue;
        }
        if layer.id() == defining_layer {
            // Don't probe the defining layer itself, and don't enqueue
            // its parents — anything below `defining_layer` can't
            // shadow a candidate at `defining_layer`.
            continue;
        }
        if layer_changes_visibility(&layer, subject) {
            return true;
        }
        for parent in layer.parents() {
            queue.push_back(Arc::clone(parent));
        }
    }
    false
}

/// True if `layer` changes the visibility of `subject` — either by
/// defining a body or by tombstoning a parent's body. The per-layer
/// shadowing bloom is consulted first as a fast-skip; on a positive
/// bloom the explicit checks decide.
fn layer_changes_visibility(layer: &Layer, subject: &Iri) -> bool {
    let maybe_present = match layer.bloom_cache().get_or_load(layer.id()) {
        Ok(Some(bloom)) => bloom.might_contain(subject),
        _ => true, // Defensive: missing bloom → treat as maybe-present.
    };
    if !maybe_present {
        return false;
    }
    layer.tombstoned_iris().contains(subject) || layer.get_resource(subject).is_some()
}

/// Indexed scan over the layer chain rooted at `head`. Returns every
/// distinct subject `s` such that `(s, predicate, object)` is defined
/// in some ancestor of `head` and not shadowed by a redefinition above
/// the defining layer.
///
/// Algorithm (D23 §5.9, Phase 14h plan):
/// 1. Build `head`'s ancestor set via [`collect_ancestors`].
/// 2. Iterate the global POS index for `(predicate, object)`.
/// 3. Drop entries whose defining layer isn't in the chain.
/// 4. Drop entries shadowed by a redefinition closer to `head`
///    (per [`is_shadowed`]).
/// 5. Return the remaining subjects, deduplicated.
///
/// Returns an empty `Vec` on storage errors mid-scan — the index is a
/// best-effort accelerator and a transient failure should not propagate
/// up as a query error. Callers that want strict failure semantics
/// should bypass this helper and walk the iterator directly.
pub fn scan_chain(head: &Layer, predicate: &Iri, object: &Iri) -> Vec<Iri> {
    let chain = collect_ancestors(head);
    let mut subjects: BTreeSet<Iri> = BTreeSet::new();
    for entry in head
        .storage()
        .triple_index
        .scan_predicate_object(predicate, object)
    {
        let (subject, defining) = match entry {
            Ok(pair) => pair,
            Err(_) => continue,
        };
        if !chain.contains(&defining) {
            continue;
        }
        if is_shadowed(head, &defining, &subject) {
            continue;
        }
        subjects.insert(subject);
    }
    subjects.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::iri::Iri;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn lid(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }

    #[test]
    fn pos_key_roundtrip() {
        let p = iri("urn:eigenius:core:is_a");
        let o = iri("urn:eigenius:test:Dog");
        let s = iri("urn:eigenius:test:rex");
        let layer = lid(0xab);

        let key = index_keys::pos_key(&p, &o, &s, &layer);
        let (p2, o2, s2, layer2) = index_keys::decode_pos_key(&key).unwrap();
        assert_eq!(p, p2);
        assert_eq!(o, o2);
        assert_eq!(s, s2);
        assert_eq!(layer, layer2);
    }

    #[test]
    fn pos_prefix_matches_full_key() {
        let p = iri("urn:eigenius:core:is_a");
        let o = iri("urn:eigenius:test:Dog");
        let s = iri("urn:eigenius:test:rex");
        let layer = lid(0xab);

        let prefix = index_keys::pos_prefix(&p, &o);
        let key = index_keys::pos_key(&p, &o, &s, &layer);
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn layer_key_roundtrip() {
        let layer = lid(0x01);
        let p = iri("urn:eigenius:core:is_a");
        let o = iri("urn:eigenius:test:Dog");
        let s = iri("urn:eigenius:test:rex");

        let key = index_keys::layer_key(&layer, &p, &o, &s);
        let (p2, o2, s2) = index_keys::decode_layer_key(&key).unwrap();
        assert_eq!(p, p2);
        assert_eq!(o, o2);
        assert_eq!(s, s2);
        assert!(key.starts_with(&index_keys::layer_prefix(&layer)));
    }

    #[test]
    fn extend_and_scan() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        let p = iri("urn:eigenius:core:is_a");
        let dog = iri("urn:eigenius:test:Dog");
        let rex = iri("urn:eigenius:test:rex");
        let buddy = iri("urn:eigenius:test:buddy");

        index
            .extend_layer(
                &layer,
                &[
                    Triple {
                        subject: &rex,
                        predicate: &p,
                        object: &dog,
                    },
                    Triple {
                        subject: &buddy,
                        predicate: &p,
                        object: &dog,
                    },
                ],
            )
            .unwrap();

        let hits: Vec<(Iri, LayerId)> = index
            .scan_predicate_object(&p, &dog)
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits.len(), 2);
        // Ordered by subject IRI thanks to BTreeSet/key ordering.
        assert!(hits.iter().any(|(s, l)| s == &rex && l == &layer));
        assert!(hits.iter().any(|(s, l)| s == &buddy && l == &layer));

        let stats = index.stats();
        assert_eq!(stats.triples, 2);
        assert_eq!(stats.layers, 1);
        assert!(stats.scans >= 1);
        assert!(stats.entries_returned >= 2);
    }

    #[test]
    fn scan_filters_by_predicate_object() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        let is_a = iri("urn:eigenius:core:is_a");
        let dog = iri("urn:eigenius:test:Dog");
        let cat = iri("urn:eigenius:test:Cat");
        let rex = iri("urn:eigenius:test:rex");
        let mittens = iri("urn:eigenius:test:mittens");

        index
            .extend_layer(
                &layer,
                &[
                    Triple {
                        subject: &rex,
                        predicate: &is_a,
                        object: &dog,
                    },
                    Triple {
                        subject: &mittens,
                        predicate: &is_a,
                        object: &cat,
                    },
                ],
            )
            .unwrap();

        let dogs: Vec<Iri> = index
            .scan_predicate_object(&is_a, &dog)
            .map(|r| r.unwrap().0)
            .collect();
        assert_eq!(dogs, vec![rex.clone()]);

        let cats: Vec<Iri> = index
            .scan_predicate_object(&is_a, &cat)
            .map(|r| r.unwrap().0)
            .collect();
        assert_eq!(cats, vec![mittens.clone()]);
    }

    #[test]
    fn drop_layer_removes_all_entries() {
        let index = MemoryTripleIndex::new();
        let layer_a = lid(0x01);
        let layer_b = lid(0x02);
        let is_a = iri("urn:eigenius:core:is_a");
        let dog = iri("urn:eigenius:test:Dog");
        let rex = iri("urn:eigenius:test:rex");
        let buddy = iri("urn:eigenius:test:buddy");

        index
            .extend_layer(
                &layer_a,
                &[Triple {
                    subject: &rex,
                    predicate: &is_a,
                    object: &dog,
                }],
            )
            .unwrap();
        index
            .extend_layer(
                &layer_b,
                &[Triple {
                    subject: &buddy,
                    predicate: &is_a,
                    object: &dog,
                }],
            )
            .unwrap();

        index.drop_layer(&layer_a).unwrap();

        let hits: Vec<(Iri, LayerId)> = index
            .scan_predicate_object(&is_a, &dog)
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, buddy);
        assert_eq!(hits[0].1, layer_b);

        let stats = index.stats();
        assert_eq!(stats.triples, 1);
        assert_eq!(stats.layers, 1);
    }

    #[test]
    fn drop_layer_idempotent() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        index.drop_layer(&layer).unwrap();
        index.drop_layer(&layer).unwrap();
        assert_eq!(index.stats().triples, 0);
    }

    #[test]
    fn extend_idempotent_on_duplicate_triple() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        let is_a = iri("urn:eigenius:core:is_a");
        let dog = iri("urn:eigenius:test:Dog");
        let rex = iri("urn:eigenius:test:rex");

        let triple = Triple {
            subject: &rex,
            predicate: &is_a,
            object: &dog,
        };
        index.extend_layer(&layer, &[triple]).unwrap();
        index.extend_layer(&layer, &[triple]).unwrap();

        assert_eq!(index.stats().triples, 1);
    }

    #[test]
    fn extend_empty_is_noop() {
        let index = MemoryTripleIndex::new();
        let layer = lid(0x01);
        index.extend_layer(&layer, &[]).unwrap();
        assert_eq!(index.stats().triples, 0);
        assert_eq!(index.stats().layers, 0);
    }

    // --- Tests for is_indexable_predicate + extract_indexable_triples ---

    use crate::layer::{LayerBuilder, LayerStorage};
    use crate::ontology::resource::{Resource, Value};
    use std::sync::Arc;

    fn property_def(prop_iri: &str, data_type: &str) -> Resource {
        let mut r = Resource::new(iri(prop_iri));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Property".into())]),
        );
        r.set(iri(wk::DATA_TYPE_PROP), Value::String(data_type.into()));
        r
    }

    /// Build a parent layer that defines `properties` (IRI → data_type).
    /// Used to give the child layer a chain in which to resolve predicates.
    fn parent_with_properties(props: &[(&str, &str)]) -> Arc<crate::layer::Layer> {
        let storage = LayerStorage::in_memory();
        let mut builder = LayerBuilder::new("test_parent", None);
        for (iri_str, data_type) in props {
            builder
                .add_resource(property_def(iri_str, data_type))
                .unwrap();
        }
        Arc::new(builder.build(storage))
    }

    #[test]
    fn is_indexable_predicate_true_for_resource_typed() {
        let parent = parent_with_properties(&[("urn:eigenius:test:owner", wk::RESOURCE)]);
        assert!(is_indexable_predicate(
            &parent,
            &iri("urn:eigenius:test:owner")
        ));
    }

    #[test]
    fn is_indexable_predicate_true_for_resource_array_typed() {
        let parent = parent_with_properties(&[("urn:eigenius:core:is_a", wk::RESOURCE_ARRAY)]);
        assert!(is_indexable_predicate(
            &parent,
            &iri("urn:eigenius:core:is_a")
        ));
    }

    #[test]
    fn is_indexable_predicate_false_for_string_typed() {
        let parent =
            parent_with_properties(&[("urn:eigenius:core:short_name", "urn:eigenius:core:string")]);
        assert!(!is_indexable_predicate(
            &parent,
            &iri("urn:eigenius:core:short_name")
        ));
    }

    #[test]
    fn is_indexable_predicate_false_for_undefined_property() {
        let parent = parent_with_properties(&[]);
        assert!(!is_indexable_predicate(
            &parent,
            &iri("urn:eigenius:test:never_defined")
        ));
    }

    #[test]
    fn extract_unpacks_resource_array_one_per_element() {
        let parent = parent_with_properties(&[("urn:eigenius:core:is_a", wk::RESOURCE_ARRAY)]);
        let storage = parent.storage().clone();
        let mut builder = LayerBuilder::new("test_child", Some(parent));
        let mut rex = Resource::new(iri("urn:eigenius:test:rex"));
        rex.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![
                Value::String("urn:eigenius:test:Dog".into()),
                Value::String("urn:eigenius:test:Pet".into()),
            ]),
        );
        builder.add_resource(rex).unwrap();
        let layer = builder.build(storage);

        let triples = extract_indexable_triples(&layer);
        assert_eq!(triples.len(), 2);
        let objects: BTreeSet<Iri> = triples.iter().map(|t| t.object.clone()).collect();
        assert!(objects.contains(&iri("urn:eigenius:test:Dog")));
        assert!(objects.contains(&iri("urn:eigenius:test:Pet")));
        for t in &triples {
            assert_eq!(t.subject, iri("urn:eigenius:test:rex"));
            assert_eq!(t.predicate, iri("urn:eigenius:core:is_a"));
        }
    }

    #[test]
    fn extract_skips_non_indexable_predicates() {
        let parent = parent_with_properties(&[
            ("urn:eigenius:core:is_a", wk::RESOURCE_ARRAY),
            ("urn:eigenius:core:short_name", "urn:eigenius:core:string"),
        ]);
        let storage = parent.storage().clone();
        let mut builder = LayerBuilder::new("test_child", Some(parent));
        let mut dog = Resource::new(iri("urn:eigenius:test:Dog"));
        dog.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
        );
        dog.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("Dog".into()),
        );
        builder.add_resource(dog).unwrap();
        let layer = builder.build(storage);

        let triples = extract_indexable_triples(&layer);
        // Only the is_a triple — short_name is a string, not indexed.
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].predicate, iri("urn:eigenius:core:is_a"));
        assert_eq!(triples[0].object, iri("urn:eigenius:core:Class"));
    }

    #[test]
    fn extract_skips_unparseable_iri_object() {
        let parent = parent_with_properties(&[("urn:eigenius:test:owner", wk::RESOURCE)]);
        let storage = parent.storage().clone();
        let mut builder = LayerBuilder::new("test_child", Some(parent));
        let mut r = Resource::new(iri("urn:eigenius:test:thing"));
        r.set(
            iri("urn:eigenius:test:owner"),
            Value::String("not a valid IRI string".into()),
        );
        builder.add_resource(r).unwrap();
        let layer = builder.build(storage);

        let triples = extract_indexable_triples(&layer);
        assert!(triples.is_empty());
    }

    // --- Tests for scan_chain / is_shadowed (Phase 14h commit 3 prep) ---

    /// Build a chain `core_props_layer → instances_layer` where:
    /// - Core layer defines `is_a` Property with data_type=resource_array.
    /// - Instance layer defines `rex` and `buddy` as instances of `Dog`,
    ///   and `mittens` as an instance of `Cat`.
    ///
    /// Returns `(head_arc, layer_id_of_instances)`.
    fn build_simple_chain() -> (Arc<Layer>, LayerId) {
        let storage = LayerStorage::in_memory();
        let mut core_builder = LayerBuilder::new("core", None);
        core_builder
            .add_resource(property_def(wk::IS_A, wk::RESOURCE_ARRAY))
            .unwrap();
        let core = Arc::new(core_builder.build(storage.clone()));

        // Use the helper to populate the index — in-memory backend
        // doesn't go through `store_layer` here, so we simulate
        // commit-time index population by calling the trait directly.
        let owned = extract_indexable_triples(&core);
        let borrowed: Vec<Triple> = owned.iter().map(|t| t.as_borrowed()).collect();
        storage
            .triple_index
            .extend_layer(core.id(), &borrowed)
            .unwrap();

        let mut inst_builder = LayerBuilder::new("instances", Some(Arc::clone(&core)));
        let mut rex = Resource::new(iri("urn:eigenius:test:rex"));
        rex.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:test:Dog".into())]),
        );
        inst_builder.add_resource(rex).unwrap();
        let mut buddy = Resource::new(iri("urn:eigenius:test:buddy"));
        buddy.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:test:Dog".into())]),
        );
        inst_builder.add_resource(buddy).unwrap();
        let mut mittens = Resource::new(iri("urn:eigenius:test:mittens"));
        mittens.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:test:Cat".into())]),
        );
        inst_builder.add_resource(mittens).unwrap();
        let instances = Arc::new(inst_builder.build(storage.clone()));
        let inst_id = instances.id().clone();

        let owned = extract_indexable_triples(&instances);
        let borrowed: Vec<Triple> = owned.iter().map(|t| t.as_borrowed()).collect();
        storage
            .triple_index
            .extend_layer(instances.id(), &borrowed)
            .unwrap();

        (instances, inst_id)
    }

    #[test]
    fn collect_ancestors_includes_head_and_walks_parents() {
        let (head, inst_id) = build_simple_chain();
        let ancestors = collect_ancestors(&head);
        assert!(ancestors.contains(&inst_id));
        // Two layers in the chain (core + instances).
        assert_eq!(ancestors.len(), 2);
    }

    #[test]
    fn scan_chain_returns_subjects_at_head() {
        let (head, _) = build_simple_chain();
        let dogs = scan_chain(&head, &iri(wk::IS_A), &iri("urn:eigenius:test:Dog"));
        let dogs_set: BTreeSet<Iri> = dogs.into_iter().collect();
        assert_eq!(dogs_set.len(), 2);
        assert!(dogs_set.contains(&iri("urn:eigenius:test:rex")));
        assert!(dogs_set.contains(&iri("urn:eigenius:test:buddy")));
    }

    #[test]
    fn scan_chain_returns_empty_for_unknown_class() {
        let (head, _) = build_simple_chain();
        let nothing = scan_chain(&head, &iri(wk::IS_A), &iri("urn:eigenius:test:Unicorn"));
        assert!(nothing.is_empty());
    }

    #[test]
    fn shadow_check_drops_redefined_subjects() {
        // Build core + main + feature where `feature` redefines `rex`.
        let storage = LayerStorage::in_memory();
        let mut core_builder = LayerBuilder::new("core", None);
        core_builder
            .add_resource(property_def(wk::IS_A, wk::RESOURCE_ARRAY))
            .unwrap();
        let core = Arc::new(core_builder.build(storage.clone()));
        let owned = extract_indexable_triples(&core);
        storage
            .triple_index
            .extend_layer(
                core.id(),
                &owned.iter().map(|t| t.as_borrowed()).collect::<Vec<_>>(),
            )
            .unwrap();

        let mut main_builder = LayerBuilder::new("main", Some(Arc::clone(&core)));
        let mut rex_dog = Resource::new(iri("urn:eigenius:test:rex"));
        rex_dog.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:test:Dog".into())]),
        );
        main_builder.add_resource(rex_dog).unwrap();
        let main = Arc::new(main_builder.build(storage.clone()));
        let owned = extract_indexable_triples(&main);
        storage
            .triple_index
            .extend_layer(
                main.id(),
                &owned.iter().map(|t| t.as_borrowed()).collect::<Vec<_>>(),
            )
            .unwrap();

        let mut feature_builder = LayerBuilder::new("feature", Some(Arc::clone(&main)));
        let mut rex_cat = Resource::new(iri("urn:eigenius:test:rex"));
        rex_cat.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:test:Cat".into())]),
        );
        feature_builder.add_resource(rex_cat).unwrap();
        let feature = Arc::new(feature_builder.build(storage.clone()));
        let owned = extract_indexable_triples(&feature);
        storage
            .triple_index
            .extend_layer(
                feature.id(),
                &owned.iter().map(|t| t.as_borrowed()).collect::<Vec<_>>(),
            )
            .unwrap();

        // At head=feature, rex is_a Cat (Dog is shadowed by feature's redef).
        let dogs = scan_chain(&feature, &iri(wk::IS_A), &iri("urn:eigenius:test:Dog"));
        assert!(
            dogs.is_empty(),
            "rex_is_a_Dog should be shadowed at feature head, got {:?}",
            dogs
        );
        let cats = scan_chain(&feature, &iri(wk::IS_A), &iri("urn:eigenius:test:Cat"));
        assert_eq!(cats, vec![iri("urn:eigenius:test:rex")]);

        // At head=main, rex is_a Dog (no shadow).
        let dogs_at_main = scan_chain(&main, &iri(wk::IS_A), &iri("urn:eigenius:test:Dog"));
        assert_eq!(dogs_at_main, vec![iri("urn:eigenius:test:rex")]);
    }

    /// A higher-up tombstone shadows an indexed subject just as a
    /// redefinition does — `Layer::resolve(subject)` from `head`
    /// short-circuits at the tombstone, so the indexed entry at the
    /// defining layer is not observable. EigenQL's `scan_chain` must
    /// drop the candidate. Without tombstone-awareness in
    /// `layer_changes_visibility` (formerly `layer_might_define`),
    /// `is_shadowed` would falsely return `false` here because the
    /// tombstoning layer's `defined_iris` is empty for the subject.
    #[test]
    fn tombstone_shadows_indexed_subject() {
        let storage = LayerStorage::in_memory();
        let mut core_builder = LayerBuilder::new("core", None);
        core_builder
            .add_resource(property_def(wk::IS_A, wk::RESOURCE_ARRAY))
            .unwrap();
        let core = Arc::new(core_builder.build(storage.clone()));
        let owned = extract_indexable_triples(&core);
        storage
            .triple_index
            .extend_layer(
                core.id(),
                &owned.iter().map(|t| t.as_borrowed()).collect::<Vec<_>>(),
            )
            .unwrap();

        // base: defines rex is_a Dog
        let mut base_builder = LayerBuilder::new("base", Some(Arc::clone(&core)));
        let mut rex_dog = Resource::new(iri("urn:eigenius:test:rex"));
        rex_dog.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:test:Dog".into())]),
        );
        base_builder.add_resource(rex_dog).unwrap();
        let base = Arc::new(base_builder.build(storage.clone()));
        let owned = extract_indexable_triples(&base);
        storage
            .triple_index
            .extend_layer(
                base.id(),
                &owned.iter().map(|t| t.as_borrowed()).collect::<Vec<_>>(),
            )
            .unwrap();

        // head: tombstones rex (removes it from view)
        let mut head_builder = LayerBuilder::new("head", Some(Arc::clone(&base)));
        head_builder
            .tombstone(iri("urn:eigenius:test:rex"))
            .unwrap();
        let head = Arc::new(head_builder.build(storage.clone()));

        // Sanity: resolve at head returns None.
        assert!(head.resolve(&iri("urn:eigenius:test:rex")).is_none());
        // EigenQL's scan_chain must agree: rex is shadowed by the
        // tombstone at head, so the indexed entry at base drops out.
        let dogs = scan_chain(&head, &iri(wk::IS_A), &iri("urn:eigenius:test:Dog"));
        assert!(
            dogs.is_empty(),
            "tombstone at head must shadow base's indexed entry; got {:?}",
            dogs
        );

        // And `is_shadowed` reports true for the candidate.
        assert!(is_shadowed(&head, base.id(), &iri("urn:eigenius:test:rex")));
    }

    #[test]
    fn is_shadowed_at_head_is_false() {
        let (head, head_id) = build_simple_chain();
        // A candidate at head itself can never be shadowed.
        assert!(!is_shadowed(&head, &head_id, &iri("urn:eigenius:test:rex")));
    }

    #[test]
    fn extract_emits_owned_triples_usable_for_extend_layer() {
        let parent = parent_with_properties(&[("urn:eigenius:core:is_a", wk::RESOURCE_ARRAY)]);
        let storage = parent.storage().clone();
        let mut builder = LayerBuilder::new("test_child", Some(parent));
        let mut rex = Resource::new(iri("urn:eigenius:test:rex"));
        rex.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::String("urn:eigenius:test:Dog".into())]),
        );
        builder.add_resource(rex).unwrap();
        let layer = builder.build(storage);

        let triples = extract_indexable_triples(&layer);
        let borrowed: Vec<Triple> = triples.iter().map(|t| t.as_borrowed()).collect();

        let index = MemoryTripleIndex::new();
        index.extend_layer(layer.id(), &borrowed).unwrap();

        let hits: Vec<(Iri, LayerId)> = index
            .scan_predicate_object(
                &iri("urn:eigenius:core:is_a"),
                &iri("urn:eigenius:test:Dog"),
            )
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, iri("urn:eigenius:test:rex"));
        assert_eq!(hits[0].1, *layer.id());
    }
}
