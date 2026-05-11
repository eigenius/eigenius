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
//! when empty) up to `max_depth` parent hops, emitting per-layer
//! summary nodes plus optional per-resource nodes (Class / Property /
//! Institution always; ordinary instance Resources only when
//! `include_resources` is true). Edges record the structural
//! relationships the notebook renderers care about: parent layer,
//! `is_a`, `subclass_of`, `requires`, `recommends`, property
//! cross-references (via `class_types`), and institution declarations.
//!
//! Read-only: no IO, no mutation. Suitable for the kernel's existing
//! `Read` capability mode.

use crate::layer::Layer;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::ontology::Iri;
use crate::server::proto;
use std::collections::BTreeSet;

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

    walk_layer(
        layer,
        max_depth,
        0,
        include_resources,
        &mut nodes,
        &mut edges,
        &mut seen_node_ids,
    );

    proto::LayerTopologyResponse { nodes, edges }
}

fn walk_layer(
    layer: &Layer,
    max_depth: u32,
    depth: u32,
    include_resources: bool,
    nodes: &mut Vec<proto::TopologyNode>,
    edges: &mut Vec<proto::TopologyEdge>,
    seen_node_ids: &mut BTreeSet<String>,
) {
    let layer_id = layer.id().to_string();

    // Per-layer counts (recomputed even when include_resources=true,
    // since the LayerStackView always wants them in attrs).
    let counts = layer_counts(layer);

    // Emit the layer node.
    if seen_node_ids.insert(layer_id.clone()) {
        let mut attrs = std::collections::BTreeMap::new();
        attrs.insert("name".to_string(), layer.name().to_string());
        attrs.insert("class_count".to_string(), counts.classes.to_string());
        attrs.insert("property_count".to_string(), counts.properties.to_string());
        attrs.insert("resource_count".to_string(), counts.resources.to_string());
        attrs.insert(
            "institution_count".to_string(),
            counts.institutions.to_string(),
        );
        nodes.push(proto::TopologyNode {
            id: layer_id.clone(),
            kind: proto::NodeKind::Layer as i32,
            label: layer.name().to_string(),
            attrs: attrs.into_iter().collect(),
        });
    }

    // Walk this layer's resources.
    let class_iri = Iri::parse(wk::CLASS).expect("CLASS IRI");
    let property_iri = Iri::parse(wk::PROPERTY).expect("PROPERTY IRI");
    let institution_iri =
        Iri::parse("urn:eigenius:institution:Institution").expect("Institution IRI");

    for (iri, arc_resource) in layer.iter_resources() {
        let resource: &Resource = &arc_resource;
        let is_class = resource.is_instance_of(&class_iri);
        let is_property = resource.is_instance_of(&property_iri);
        let is_institution = resource.is_instance_of(&institution_iri);
        let is_taxonomy = is_class || is_property || is_institution;

        if !include_resources && !is_taxonomy {
            // Skip ordinary instance resources unless the caller asked
            // for them — they're aggregated into the layer's counts.
            continue;
        }

        let kind = if is_institution {
            proto::NodeKind::Institution
        } else if is_class {
            proto::NodeKind::Class
        } else if is_property {
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
                nodes,
                edges,
                seen_node_ids,
            );
        }
    }
}

#[derive(Default)]
struct LayerCounts {
    classes: usize,
    properties: usize,
    resources: usize,
    institutions: usize,
}

fn layer_counts(layer: &Layer) -> LayerCounts {
    let class_iri = Iri::parse(wk::CLASS).expect("CLASS IRI");
    let property_iri = Iri::parse(wk::PROPERTY).expect("PROPERTY IRI");
    let institution_iri =
        Iri::parse("urn:eigenius:institution:Institution").expect("Institution IRI");

    let mut c = LayerCounts::default();
    for arc_resource in layer.iter_resources().map(|(_, r)| r) {
        let resource: &Resource = &arc_resource;
        if resource.is_instance_of(&class_iri) {
            c.classes += 1;
        } else if resource.is_instance_of(&property_iri) {
            c.properties += 1;
        } else if resource.is_instance_of(&institution_iri) {
            c.institutions += 1;
        } else {
            c.resources += 1;
        }
    }
    c
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

    #[test]
    fn walks_two_layer_chain_skipping_instances_by_default() {
        let layer = build_chain();
        let topo = walk(&layer, 0, /* include_resources */ false);

        // 2 layer nodes + Class Animal + Property name + Class Dog = 5 nodes.
        // Instance `rex` is excluded (include_resources=false).
        assert_eq!(topo.nodes.len(), 5, "nodes: {:?}", topo.nodes);
        let kinds: std::collections::BTreeMap<String, i32> =
            topo.nodes.iter().map(|n| (n.id.clone(), n.kind)).collect();
        assert_eq!(
            kinds.get("urn:eigenius:example:Animal"),
            Some(&(proto::NodeKind::Class as i32))
        );
        assert_eq!(
            kinds.get("urn:eigenius:example:name"),
            Some(&(proto::NodeKind::Property as i32))
        );
        assert_eq!(
            kinds.get("urn:eigenius:example:Dog"),
            Some(&(proto::NodeKind::Class as i32))
        );
        assert!(!kinds.contains_key("urn:eigenius:example:rex"));

        // Layer counts in attrs.
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
            "the rex instance should be counted even when not emitted as a node"
        );

        // Edges: parent_layer + Dog→Animal subclass_of + Dog→name requires.
        let edge_kinds: Vec<i32> = topo.edges.iter().map(|e| e.kind).collect();
        assert!(
            edge_kinds.contains(&(proto::EdgeKind::ParentLayer as i32)),
            "parent_layer edge missing"
        );
        assert!(
            topo.edges
                .iter()
                .any(|e| e.kind == proto::EdgeKind::SubclassOf as i32
                    && e.source == "urn:eigenius:example:Dog"
                    && e.target == "urn:eigenius:example:Animal"),
            "subclass_of edge missing"
        );
        assert!(
            topo.edges
                .iter()
                .any(|e| e.kind == proto::EdgeKind::Requires as i32
                    && e.source == "urn:eigenius:example:Dog"
                    && e.target == "urn:eigenius:example:name"),
            "requires edge missing"
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

        let topo = walk(&layer, 0, false);
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

        let topo = walk(&layer, 0, false);
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

        let topo = walk(&layer, 0, false);

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
