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

//! Chain-walking helpers shared by every merge variant.
//!
//! These primitives traverse the layer parent chain to locate IRIs,
//! enumerate the ancestor's contributed IRIs, and walk arbitrary
//! `Value` trees collecting IRI references. They're factored out of
//! the per-variant modules so the classifier, witness application,
//! rename, schema-quotient, restructure, and cascade walkers share
//! one implementation each.

use super::conflict::MergeSpan;
use super::MergeError;
use crate::layer::handle::LayerTopology;
use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::storage::{PersistentBackend, StorageError};
use std::collections::{BTreeSet, VecDeque};

/// Walk the parent chain rooted at `head` looking for the topmost
/// layer that defines `iri`. Returns the layer id + the resource it
/// found, or `None` if no layer in the chain defines the IRI.
///
/// BFS over `LayerHandle.parents` so the shallowest (topmost-in-the-
/// chain) layer wins on multi-parent merges. Visited set prevents
/// re-entry on diamonds. Storage errors abort the walk and
/// propagate up.
pub(crate) fn find_iri_in_chain(
    head: &LayerId,
    iri: &Iri,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Option<(LayerId, Resource)>, StorageError> {
    let mut visited: BTreeSet<LayerId> = BTreeSet::new();
    let mut queue: VecDeque<LayerId> = VecDeque::new();
    queue.push_back(head.clone());
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if let Some(resource) = backend.try_load_resource(&id, iri)? {
            return Ok(Some((id, resource)));
        }
        if let Some(handle) = topology.get_layer(&id) {
            for parent in &handle.parents {
                if !visited.contains(parent) {
                    queue.push_back(parent.clone());
                }
            }
        }
    }
    Ok(None)
}

/// Walk the merge span looking for an IRI's definition. Searches
/// each branch's contributions first (those are the most-recent
/// commits and most-likely places for a freshly-committed witness)
/// before falling back to the ancestor's parent chain.
///
/// Returns `Some((layer_id, resource))` for the topmost layer that
/// defines `iri`; `None` if the IRI isn't reachable from any of the
/// span's heads. Storage errors propagate.
pub(crate) fn find_in_span_chain(
    iri: &Iri,
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Option<(LayerId, Resource)>, StorageError> {
    if let Some(layer) = span.sources_a.get(iri) {
        if let Some(resource) = backend.try_load_resource(layer, iri)? {
            return Ok(Some((layer.clone(), resource)));
        }
    }
    if let Some(layer) = span.sources_b.get(iri) {
        if let Some(resource) = backend.try_load_resource(layer, iri)? {
            return Ok(Some((layer.clone(), resource)));
        }
    }
    find_iri_in_chain(&span.ancestor, iri, topology, backend)
}

/// Collect every (IRI, layer_id) reachable from the ancestor head,
/// walking the parent chain.
pub(crate) fn ancestor_chain_iris(
    ancestor: &LayerId,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Vec<(Iri, LayerId)>, MergeError> {
    use crate::storage::ResourceBackend;
    let mut out: Vec<(Iri, LayerId)> = Vec::new();
    let mut visited: BTreeSet<LayerId> = BTreeSet::new();
    let mut queue: VecDeque<LayerId> = VecDeque::new();
    queue.push_back(ancestor.clone());
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        let iris = ResourceBackend::list_layer_iris(backend, &id).map_err(MergeError::Storage)?;
        for iri in iris {
            out.push((iri, id.clone()));
        }
        if let Some(handle) = topology.get_layer(&id) {
            for parent in &handle.parents {
                if !visited.contains(parent) {
                    queue.push_back(parent.clone());
                }
            }
        }
    }
    Ok(out)
}

/// Yield every `Iri` referenced by a `Value`, recursing through
/// every nested container shape Eigon admits:
///
/// - `ResourceRef(iri)` — yield the IRI directly.
/// - `Array(items)` — recurse into each item (handles arrays of
///   refs, arrays of arrays, arrays of embeddeds).
/// - `Embedded(resource)` — recurse into each of the embedded
///   resource's property values; the embedded resource itself has
///   no `@id`, but its property values can mention any number of
///   IRIs (including its `is_a` class refs).
/// - Scalars (string / integer / float / boolean) and `Json` —
///   yield nothing.
///
/// Generic helper, not bound to any particular property. Callers
/// that want a specific subset (e.g., only `subclass_of` parents)
/// pass the property value directly; well-formed Eigon values for
/// most property shapes are flat arrays of refs, so recursion is a
/// no-op there and a safety net for the malformed-value cases the
/// classifier shouldn't silently ignore.
pub(crate) fn iter_iri_values(value: &crate::ontology::resource::Value) -> Vec<Iri> {
    let mut out = Vec::new();
    collect_iri_refs_into(value, &mut out);
    out
}

pub(crate) fn collect_iri_refs_into(value: &crate::ontology::resource::Value, out: &mut Vec<Iri>) {
    use crate::ontology::resource::Value;
    match value {
        Value::ResourceRef(iri) => out.push(iri.clone()),
        Value::Array(items) => {
            for v in items {
                collect_iri_refs_into(v, out);
            }
        }
        Value::Embedded(resource) => {
            for v in resource.properties().values() {
                collect_iri_refs_into(v, out);
            }
        }
        Value::String(_)
        | Value::Integer(_)
        | Value::Float(_)
        | Value::Boolean(_)
        | Value::Json(_)
        | Value::Vector { .. } => {}
    }
}
