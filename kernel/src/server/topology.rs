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

//! Layer-chain topology walker for the notebook UI (D22 §4.2).
//!
//! Walks a layer chain starting from `root_layer` (or the active top
//! when empty) up to `max_depth` parent hops.
//!
//! Two modes, keyed on `include_resources`:
//! - **`false` (the layer-stack view):** emit only per-layer summary nodes
//!   carrying per-kind *counts* (classes / properties / institutions /
//!   instances). No resource bodies are materialised — the counts come from the
//!   triple index — and no per-resource nodes are shipped. This keeps the stack
//!   view O(layers), independent of how large any layer is (a domain-lexicon
//!   layer may hold tens of thousands of concept classes / millions of
//!   instances). To inspect one layer's contents, fetch that layer specifically
//!   (`root_layer = <id>`, `max_depth = 1`, `include_resources = true`).
//! - **`true` (the contents/graph view):** additionally emit a node per resource
//!   (Class / Property / Institution / instance), with the structural edges the
//!   notebook renderers care about: parent layer, `is_a`, `subclass_of`,
//!   `requires`, `recommends`, property cross-references (via `class_types`), and
//!   institution declarations.
//!
//! Read-only: no IO, no mutation. Suitable for the kernel's existing
//! `Read` capability mode.

use crate::layer::{Layer, LayerId};
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::ontology::Iri;
use crate::server::proto;
use std::collections::{BTreeSet, HashMap};

/// Walk `layer` (and parents up to `max_depth` hops) and emit a topology.
///
/// `max_depth = 0` means unlimited. The output is deterministic: layers
/// are emitted top-to-bottom, resources within a layer in BTreeMap
/// (sorted-IRI) order, edges in the order they're produced as resources
/// are walked.
pub fn walk(
    layer: &Layer,
    max_depth: u32,
    include_resources: bool,
) -> proto::LayerTopologyResponse {
    let mut nodes: Vec<proto::TopologyNode> = Vec::new();
    let mut edges: Vec<proto::TopologyEdge> = Vec::new();
    // Deduplicate nodes by id across layers (a class defined in core
    // and referenced from a child layer should appear once).
    let mut seen_node_ids: BTreeSet<String> = BTreeSet::new();

    // Per-layer taxonomy counts from the triple index — computed once for the whole
    // DAG up front (a few streaming scans over the is_a index), so no layer's
    // resource bodies are materialised just to count them. The per-layer "resource"
    // (instance) count is derived in `walk_layer` as `defined_iris.len() - taxonomy`.
    let taxonomy = taxonomy_counts_by_layer(layer);

    walk_layer(
        layer,
        max_depth,
        0,
        include_resources,
        &taxonomy,
        &mut nodes,
        &mut edges,
        &mut seen_node_ids,
    );

    proto::LayerTopologyResponse { nodes, edges }
}

/// Per-layer `(classes, properties, institutions)` counts, computed from the triple
/// index instead of by materialising resources. Scans the `is_a` index for each
/// meta-class object and buckets matching subjects by the layer that defines them
/// (D23 §5.9). Cost scales with the taxonomy size (classes + properties +
/// institutions), never the instance population — the whole point: a chain carrying
/// a domain lexicon has tens of thousands of concept *classes* but millions of
/// *instances*, and counting must not page the instances in.
///
/// Matches the previous `is_instance_of`-based counting exactly: both test direct
/// `is_a` membership of the meta-class ([`Resource::is_instance_of`] is direct, and
/// the index keys triples by their literal `(s, is_a, o)`). Layers not in the walked
/// chain may appear in the map; `walk_layer` only reads the entries it needs.
fn taxonomy_counts_by_layer(layer: &Layer) -> HashMap<LayerId, (u64, u64, u64)> {
    let triple_index = &layer.storage().triple_index;
    let is_a = match Iri::parse(wk::IS_A) {
        Ok(i) => i,
        Err(_) => return HashMap::new(),
    };
    let class_iri = Iri::parse(wk::CLASS).expect("CLASS IRI");
    let property_iri = Iri::parse(wk::PROPERTY).expect("PROPERTY IRI");
    let institution_iri =
        Iri::parse("urn:eigenius:institution:Institution").expect("Institution IRI");

    let mut counts: HashMap<LayerId, (u64, u64, u64)> = HashMap::new();
    for (object, slot) in [(&class_iri, 0u8), (&property_iri, 1), (&institution_iri, 2)] {
        // `.flatten()` drops any transient scan error (the topology view is
        // advisory, never a correctness gate).
        for (_subject, defining_layer) in
            triple_index.scan_predicate_object(&is_a, object).flatten()
        {
            let entry = counts.entry(defining_layer).or_insert((0, 0, 0));
            match slot {
                0 => entry.0 += 1,
                1 => entry.1 += 1,
                _ => entry.2 += 1,
            }
        }
    }
    counts
}

// Internal recursive walker; the accumulators + read-only context naturally make
// for a wide signature.
#[allow(clippy::too_many_arguments)]
fn walk_layer(
    layer: &Layer,
    max_depth: u32,
    depth: u32,
    include_resources: bool,
    taxonomy: &HashMap<LayerId, (u64, u64, u64)>,
    nodes: &mut Vec<proto::TopologyNode>,
    edges: &mut Vec<proto::TopologyEdge>,
    seen_node_ids: &mut BTreeSet<String>,
) {
    let layer_id = layer.id().to_string();

    // Per-layer counts, from the precomputed index buckets — no resource bodies are
    // loaded. Instances = everything the layer defines minus the taxonomy.
    let (classes, properties, institutions) =
        taxonomy.get(layer.id()).copied().unwrap_or((0, 0, 0));
    let taxonomy_total = classes + properties + institutions;
    let resources = (layer.defined_iris().len() as u64).saturating_sub(taxonomy_total);

    // Emit the layer node.
    if seen_node_ids.insert(layer_id.clone()) {
        let mut attrs = std::collections::BTreeMap::new();
        attrs.insert("name".to_string(), layer.name().to_string());
        attrs.insert("class_count".to_string(), classes.to_string());
        attrs.insert("property_count".to_string(), properties.to_string());
        attrs.insert("resource_count".to_string(), resources.to_string());
        attrs.insert("institution_count".to_string(), institutions.to_string());
        // Commit timestamp (D34 §5.2). Millis since Unix epoch.
        // Consumers render this as the layer's "Last commit" timestamp;
        // the notebook's History panel keys its row ordering on it.
        attrs.insert("created_at_ms".to_string(), layer.created_at().to_string());
        nodes.push(proto::TopologyNode {
            id: layer_id.clone(),
            kind: proto::NodeKind::Layer as i32,
            label: layer.name().to_string(),
            attrs: attrs.into_iter().collect(),
        });
    }

    // Per-resource nodes are emitted ONLY when the caller asks for them
    // (`include_resources`). The layer-stack view fetches with it off and gets just
    // layer nodes + the counts above — so it never pages in (or ships to the client)
    // a domain layer's tens of thousands of concept classes / millions of instances.
    // A client wanting one layer's contents fetches that layer specifically
    // (`root_layer = <id>`, `max_depth = 1`, `include_resources = true`).
    if include_resources {
        let class_iri = Iri::parse(wk::CLASS).expect("CLASS IRI");
        let property_iri = Iri::parse(wk::PROPERTY).expect("PROPERTY IRI");
        let institution_iri =
            Iri::parse("urn:eigenius:institution:Institution").expect("Institution IRI");

        for (iri, arc_resource) in layer.iter_resources() {
            let resource: &Resource = &arc_resource;
            let kind = if resource.is_instance_of(&institution_iri) {
                proto::NodeKind::Institution
            } else if resource.is_instance_of(&class_iri) {
                proto::NodeKind::Class
            } else if resource.is_instance_of(&property_iri) {
                proto::NodeKind::Property
            } else {
                proto::NodeKind::Resource
            };

            let id = iri.as_str().to_string();
            if seen_node_ids.insert(id.clone()) {
                let label = node_label(resource, &iri);
                let mut attrs = resource_attrs(resource);
                // Attribute the node to the layer that introduced it so
                // clients can filter "what's in this layer" without
                // re-querying. Walker visits head-down with a seen-set,
                // so each resource is attributed to whichever layer first
                // declared it in the chain.
                attrs.insert("layer_id".to_string(), layer_id.clone());
                nodes.push(proto::TopologyNode {
                    id: id.clone(),
                    kind: kind as i32,
                    label,
                    attrs,
                });
                // Emit resource edges only on first sighting — gating
                // alongside the node dedup. Without this, when the same
                // class/property resource appears in N layers (e.g. the
                // user re-ran an ESL cell N times, stacking N near-
                // identical layers), the walker would emit each edge N
                // times. Head-down traversal means the edges come from
                // the topmost (most-specific) version of the resource,
                // matching what the validator/resolver sees.
                emit_resource_edges(resource, &iri, kind, edges);
            }
        }
    }

    // Walk parent. The parent_layer edge is only emitted when we
    // actually walk the parent — otherwise the edge would point at a
    // node not present in the response, which renderers can't lay out.
    if let Some(parent) = layer.parent() {
        if max_depth == 0 || depth + 1 < max_depth {
            edges.push(proto::TopologyEdge {
                source: layer_id.clone(),
                target: parent.id().to_string(),
                kind: proto::EdgeKind::ParentLayer as i32,
                attrs: std::collections::HashMap::new(),
            });
            walk_layer(
                parent,
                max_depth,
                depth + 1,
                include_resources,
                taxonomy,
                nodes,
                edges,
                seen_node_ids,
            );
        }
    }
}

fn node_label(resource: &crate::ontology::resource::Resource, iri: &Iri) -> String {
    let short_name_iri = Iri::parse(wk::SHORT_NAME).expect("SHORT_NAME IRI");
    if let Some(v) = resource.get(&short_name_iri) {
        if let Some(s) = v.as_str() {
            return s.to_string();
        }
    }
    // Fall back to the local IRI tail.
    let s = iri.as_str();
    s.rsplit(':').next().unwrap_or(s).to_string()
}

fn resource_attrs(
    resource: &crate::ontology::resource::Resource,
) -> std::collections::HashMap<String, String> {
    let mut attrs = std::collections::HashMap::new();
    let description_iri = Iri::parse(wk::DESCRIPTION).expect("DESCRIPTION IRI");
    if let Some(v) = resource.get(&description_iri) {
        if let Some(s) = v.as_str() {
            attrs.insert("description".to_string(), s.to_string());
        }
    }
    let data_type_iri = Iri::parse(wk::DATA_TYPE_PROP).expect("DATA_TYPE_PROP IRI");
    if let Some(v) = resource.get(&data_type_iri) {
        // `data_type` is a resource-typed property — its value is
        // `Value::ResourceRef` after `canonicalise_resource_refs` runs,
        // not `Value::String`. Use `as_iri_str` to cover both shapes.
        if let Some(s) = v.as_iri_str() {
            attrs.insert("data_type".to_string(), s.to_string());
        }
    }
    attrs
}

fn emit_resource_edges(
    resource: &crate::ontology::resource::Resource,
    iri: &Iri,
    kind: proto::NodeKind,
    edges: &mut Vec<proto::TopologyEdge>,
) {
    let source = iri.as_str().to_string();

    // is_a edges (resource → class). Skip for layer nodes (no is_a) and
    // for the meta-class self-references that would just clutter the
    // graph — we skip is_a edges from Class resources back to Class.
    let is_a_iri = Iri::parse(wk::IS_A).expect("IS_A IRI");
    if let Some(Value::Array(values)) = resource.get(&is_a_iri) {
        for v in values {
            if let Some(target_iri) = v.as_iri_str() {
                // Skip self-typing for taxonomy meta-resources.
                if kind == proto::NodeKind::Class && target_iri == wk::CLASS {
                    continue;
                }
                if kind == proto::NodeKind::Property && target_iri == wk::PROPERTY {
                    continue;
                }
                edges.push(proto::TopologyEdge {
                    source: source.clone(),
                    target: target_iri.to_string(),
                    kind: proto::EdgeKind::IsA as i32,
                    attrs: std::collections::HashMap::new(),
                });
            }
        }
    }

    // subclass_of edges (class → parent class).
    let subclass_iri = Iri::parse(wk::PARENT_CLASSES).expect("PARENT_CLASSES IRI");
    if let Some(Value::Array(values)) = resource.get(&subclass_iri) {
        for v in values {
            if let Some(target_iri) = v.as_iri_str() {
                edges.push(proto::TopologyEdge {
                    source: source.clone(),
                    target: target_iri.to_string(),
                    kind: proto::EdgeKind::SubclassOf as i32,
                    attrs: std::collections::HashMap::new(),
                });
            }
        }
    }

    // requires edges (class → required property).
    let requires_iri = Iri::parse(wk::REQUIRES).expect("REQUIRES IRI");
    if let Some(Value::Array(values)) = resource.get(&requires_iri) {
        for v in values {
            if let Some(target_iri) = v.as_iri_str() {
                edges.push(proto::TopologyEdge {
                    source: source.clone(),
                    target: target_iri.to_string(),
                    kind: proto::EdgeKind::Requires as i32,
                    attrs: std::collections::HashMap::new(),
                });
            }
        }
    }

    // recommends edges (class → recommended property).
    let recommends_iri = Iri::parse(wk::RECOMMENDS).expect("RECOMMENDS IRI");
    if let Some(Value::Array(values)) = resource.get(&recommends_iri) {
        for v in values {
            if let Some(target_iri) = v.as_iri_str() {
                edges.push(proto::TopologyEdge {
                    source: source.clone(),
                    target: target_iri.to_string(),
                    kind: proto::EdgeKind::Recommends as i32,
                    attrs: std::collections::HashMap::new(),
                });
            }
        }
    }

    // class_types edges (property → referenced class).
    if kind == proto::NodeKind::Property {
        let class_types_iri = Iri::parse(wk::CLASS_TYPES).expect("CLASS_TYPES IRI");
        if let Some(Value::Array(values)) = resource.get(&class_types_iri) {
            for v in values {
                if let Some(target_iri) = v.as_iri_str() {
                    edges.push(proto::TopologyEdge {
                        source: source.clone(),
                        target: target_iri.to_string(),
                        kind: proto::EdgeKind::PropertyRef as i32,
                        attrs: std::collections::HashMap::new(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::Resource;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_class_resource(id: &str, short_name: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::CLASS.to_string())]),
        );
        r.set(iri(wk::SHORT_NAME), Value::String(short_name.to_string()));
        r
    }

    fn make_property_resource(id: &str, short_name: &str, data_type: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
        );
        r.set(iri(wk::SHORT_NAME), Value::String(short_name.to_string()));
        r.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String(data_type.to_string()),
        );
        r
    }

    fn make_instance(id: &str, class_iri: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(class_iri.to_string())]),
        );
        r
    }

    /// Build a small two-layer chain:
    ///   root: Class `Animal`, Property `name` (string)
    ///   top:  Class `Dog` (subclass_of Animal, requires `name`),
    ///         instance `rex` (is_a Dog)
    fn build_chain() -> Arc<crate::layer::Layer> {
        let mut root = LayerBuilder::new("root", None);
        root.add_resource(make_class_resource("urn:eigenius:example:Animal", "Animal"))
            .unwrap();
        root.add_resource(make_property_resource(
            "urn:eigenius:example:name",
            "name",
            "urn:eigenius:core:string",
        ))
        .unwrap();
        let root_layer = Arc::new(root.build(crate::layer::LayerStorage::in_memory()));

        let mut top = LayerBuilder::new("top", Some(root_layer.clone()));
        let mut dog = make_class_resource("urn:eigenius:example:Dog", "Dog");
        dog.set(
            iri(wk::PARENT_CLASSES),
            Value::Array(vec![Value::String(
                "urn:eigenius:example:Animal".to_string(),
            )]),
        );
        dog.set(
            iri(wk::REQUIRES),
            Value::Array(vec![Value::String("urn:eigenius:example:name".to_string())]),
        );
        top.add_resource(dog).unwrap();
        top.add_resource(make_instance(
            "urn:eigenius:example:rex",
            "urn:eigenius:example:Dog",
        ))
        .unwrap();
        Arc::new(top.build(crate::layer::LayerStorage::in_memory()))
    }

    /// Like [`build_chain`] but rooted on the real core ontology, all layers sharing
    /// one storage. The index-based counts need `core:is_a` to be a known indexable
    /// predicate (`data_type = resource_array`) and all layers in one triple index —
    /// both true in production (bootstrap + shared backend), neither true for the
    /// minimal `build_chain` fixtures. Used by the count-asserting test.
    fn build_core_rooted_chain() -> Arc<crate::layer::Layer> {
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let mut core = LayerBuilder::new("core", None);
        for r in crate::ontology::eigon_json::parse_document(core_json).unwrap() {
            core.add_resource(r).unwrap();
        }
        let core_layer = Arc::new(core.build(storage.clone()));

        let mut root = LayerBuilder::new("root", Some(core_layer));
        root.add_resource(make_class_resource("urn:eigenius:example:Animal", "Animal"))
            .unwrap();
        root.add_resource(make_property_resource(
            "urn:eigenius:example:name",
            "name",
            "urn:eigenius:core:string",
        ))
        .unwrap();
        let root_layer = Arc::new(root.build(storage.clone()));

        let mut top = LayerBuilder::new("top", Some(root_layer));
        let mut dog = make_class_resource("urn:eigenius:example:Dog", "Dog");
        dog.set(
            iri(wk::PARENT_CLASSES),
            Value::Array(vec![Value::String(
                "urn:eigenius:example:Animal".to_string(),
            )]),
        );
        top.add_resource(dog).unwrap();
        top.add_resource(make_instance(
            "urn:eigenius:example:rex",
            "urn:eigenius:example:Dog",
        ))
        .unwrap();
        Arc::new(top.build(storage))
    }

    #[test]
    fn layers_only_by_default_emits_layer_nodes_with_index_counts() {
        let layer = build_core_rooted_chain();
        let topo = walk(&layer, 0, /* include_resources */ false);

        // include_resources=false is the lightweight stack-view path: ONLY the layer
        // nodes are emitted (core + root + top = 3 here), never the per-resource
        // class/property/instance nodes. This is what keeps a domain-lexicon chain
        // (tens of thousands of concept classes) from being paged in and shipped to
        // the client.
        assert_eq!(topo.nodes.len(), 3, "nodes: {:?}", topo.nodes);
        assert!(
            topo.nodes
                .iter()
                .all(|n| n.kind == proto::NodeKind::Layer as i32),
            "only layer nodes should be emitted with include_resources=false"
        );

        // Counts still come through in attrs — computed from the triple index, not
        // by materialising bodies. The top layer has Class `Dog` (1) + instance
        // `rex` (1).
        let top_layer_node = topo
            .nodes
            .iter()
            .find(|n| n.kind == proto::NodeKind::Layer as i32 && n.label == "top")
            .expect("top layer node present");
        assert_eq!(
            top_layer_node.attrs.get("class_count"),
            Some(&"1".to_string())
        );
        assert_eq!(
            top_layer_node.attrs.get("resource_count"),
            Some(&"1".to_string()),
            "the rex instance is counted via the index, not emitted as a node"
        );
        // The root layer has Class `Animal` (1) + Property `name` (1).
        let root_layer_node = topo
            .nodes
            .iter()
            .find(|n| n.kind == proto::NodeKind::Layer as i32 && n.label == "root")
            .expect("root layer node present");
        assert_eq!(
            root_layer_node.attrs.get("class_count"),
            Some(&"1".to_string())
        );
        assert_eq!(
            root_layer_node.attrs.get("property_count"),
            Some(&"1".to_string())
        );

        // The structural parent_layer edge is still emitted (it's layer-level, not
        // resource-level); resource edges are not (no resource nodes were walked).
        assert!(
            topo.edges
                .iter()
                .any(|e| e.kind == proto::EdgeKind::ParentLayer as i32),
            "parent_layer edge missing"
        );
        assert!(
            !topo
                .edges
                .iter()
                .any(|e| e.kind == proto::EdgeKind::SubclassOf as i32),
            "no resource edges should be emitted with include_resources=false"
        );
    }

    #[test]
    fn walks_two_layer_chain_with_instances_included() {
        let layer = build_chain();
        let topo = walk(&layer, 0, /* include_resources */ true);

        // Same as above + the `rex` instance node + its is_a edge.
        assert_eq!(topo.nodes.len(), 6, "nodes: {:?}", topo.nodes);
        assert!(
            topo.nodes.iter().any(|n| n.id == "urn:eigenius:example:rex"
                && n.kind == proto::NodeKind::Resource as i32),
            "rex resource node missing"
        );
        assert!(
            topo.edges
                .iter()
                .any(|e| e.kind == proto::EdgeKind::IsA as i32
                    && e.source == "urn:eigenius:example:rex"
                    && e.target == "urn:eigenius:example:Dog"),
            "is_a edge from rex to Dog missing"
        );
    }

    #[test]
    fn max_depth_limits_traversal() {
        let layer = build_chain();
        // max_depth=1 should walk only the top layer, not the parent.
        let topo = walk(&layer, 1, false);

        // Only the top layer node + its emitted Class Dog. No Animal,
        // no name, no parent_layer edge.
        let layer_nodes: Vec<_> = topo
            .nodes
            .iter()
            .filter(|n| n.kind == proto::NodeKind::Layer as i32)
            .collect();
        assert_eq!(layer_nodes.len(), 1, "only the top layer should be walked");
        assert!(
            !topo
                .nodes
                .iter()
                .any(|n| n.id == "urn:eigenius:example:Animal"),
            "parent-layer Class Animal should not appear"
        );
        assert!(
            !topo
                .edges
                .iter()
                .any(|e| e.kind == proto::EdgeKind::ParentLayer as i32),
            "parent_layer edge should not be emitted at max_depth=1"
        );
    }

    #[test]
    fn deduplicates_nodes_seen_in_multiple_layers() {
        // Build a chain where the same Class IRI appears in both layers
        // (an override pattern). Each node should appear once.
        let mut root = LayerBuilder::new("root", None);
        root.add_resource(make_class_resource("urn:example:Foo", "Foo"))
            .unwrap();
        let root_layer = Arc::new(root.build(crate::layer::LayerStorage::in_memory()));

        let mut top = LayerBuilder::new("top", Some(root_layer));
        // Same IRI as in root, with a different short_name (override).
        let mut foo_v2 = make_class_resource("urn:example:Foo", "FooV2");
        foo_v2.set(
            iri(wk::DESCRIPTION),
            Value::String("the second-version override".into()),
        );
        top.add_resource(foo_v2).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let topo = walk(&layer, 0, true);
        let foo_nodes: Vec<_> = topo
            .nodes
            .iter()
            .filter(|n| n.id == "urn:example:Foo")
            .collect();
        assert_eq!(
            foo_nodes.len(),
            1,
            "same-IRI resource should appear exactly once across layers"
        );
        // The top-layer (first-walked) version wins.
        assert_eq!(foo_nodes[0].label, "FooV2");
    }

    #[test]
    fn deduplicates_edges_when_same_resource_in_multiple_layers() {
        // Repeatedly stacking the "same" schema (e.g., user re-runs an
        // ESL cell, creating multiple near-identical layers) must not
        // multiply emitted edges. Two layers with the same Class →
        // requires Property pair should yield one edge, not two.
        let mut root = LayerBuilder::new("root", None);
        root.add_resource(make_property_resource(
            "urn:example:name",
            "name",
            "urn:eigenius:core:string",
        ))
        .unwrap();
        let mut foo = make_class_resource("urn:example:Foo", "Foo");
        foo.set(
            iri(wk::REQUIRES),
            Value::Array(vec![Value::String("urn:example:name".to_string())]),
        );
        root.add_resource(foo.clone()).unwrap();
        let root_layer = Arc::new(root.build(crate::layer::LayerStorage::in_memory()));

        // The same schema, again, in a child layer. Without edge
        // dedup the walker would emit each requires/recommends/
        // property_ref twice.
        let mut top = LayerBuilder::new("top", Some(root_layer));
        top.add_resource(foo).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let topo = walk(&layer, 0, true);
        let requires_edges: Vec<_> = topo
            .edges
            .iter()
            .filter(|e| {
                e.kind == proto::EdgeKind::Requires as i32
                    && e.source == "urn:example:Foo"
                    && e.target == "urn:example:name"
            })
            .collect();
        assert_eq!(
            requires_edges.len(),
            1,
            "expected exactly one Foo → name requires edge despite the resource appearing in two layers; got {:?}",
            requires_edges
        );
    }

    /// Production resources go through `canonicalise_resource_refs` at
    /// `LayerBuilder::build` time, which rewrites `Value::String` IRIs
    /// on resource-typed properties to `Value::ResourceRef`. The walker
    /// originally used `Value::as_str` which returns `None` for
    /// `ResourceRef`, silently dropping every type/hierarchy edge in
    /// any chain that had been built (= every production chain). This
    /// test pins the post-canonicalisation shape directly so we'd
    /// catch a regression even without a full LayerBuilder round-trip.
    #[test]
    fn walker_emits_edges_for_canonicalised_resource_refs() {
        let mut animal = Resource::new(iri("urn:eigenius:example:Animal"));
        animal.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        animal.set(iri(wk::SHORT_NAME), Value::String("Animal".to_string()));

        let mut name_prop = Resource::new(iri("urn:eigenius:example:name"));
        name_prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        name_prop.set(iri(wk::SHORT_NAME), Value::String("name".to_string()));
        name_prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));

        let mut dog = Resource::new(iri("urn:eigenius:example:Dog"));
        dog.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        dog.set(iri(wk::SHORT_NAME), Value::String("Dog".to_string()));
        dog.set(
            iri(wk::PARENT_CLASSES),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:example:Animal"))]),
        );
        dog.set(
            iri(wk::REQUIRES),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:example:name"))]),
        );

        let mut root = LayerBuilder::new("root", None);
        root.add_resource(animal).unwrap();
        root.add_resource(name_prop).unwrap();
        root.add_resource(dog).unwrap();
        let layer = Arc::new(root.build(crate::layer::LayerStorage::in_memory()));

        let topo = walk(&layer, 0, true);

        let subclass = topo.edges.iter().find(|e| {
            e.kind == proto::EdgeKind::SubclassOf as i32
                && e.source == "urn:eigenius:example:Dog"
                && e.target == "urn:eigenius:example:Animal"
        });
        assert!(
            subclass.is_some(),
            "expected SUBCLASS_OF Dog → Animal edge from ResourceRef-shaped data; edges = {:?}",
            topo.edges,
        );

        let requires = topo.edges.iter().find(|e| {
            e.kind == proto::EdgeKind::Requires as i32
                && e.source == "urn:eigenius:example:Dog"
                && e.target == "urn:eigenius:example:name"
        });
        assert!(
            requires.is_some(),
            "expected REQUIRES Dog → name edge from ResourceRef-shaped data; edges = {:?}",
            topo.edges,
        );

        // data_type attr should be readable post-canonicalisation too.
        let name_node = topo
            .nodes
            .iter()
            .find(|n| n.id == "urn:eigenius:example:name")
            .expect("name property node present");
        assert_eq!(
            name_node.attrs.get("data_type").map(String::as_str),
            Some(wk::STRING),
            "expected data_type attr read off ResourceRef value; got: {:?}",
            name_node.attrs,
        );
    }
}
