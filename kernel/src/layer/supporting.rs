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

//! Supporting-layer computation (D33 §4.3).
//!
//! For a layer `L`, the *supporting layer* is the deepest ancestor
//! whose ancestor-closure provides every IRI `L` references —
//! equivalently, the topmost-in-head→root per-reference definer, i.e.
//! the youngest ancestor that `L` explicitly depends on.
//!
//! See [D33 §4.3](../../../docs/design/d33-partial-order-chains.md)
//! for the formal definition. Properties relied on:
//!
//! - **Position freedom.** `L` can be placed at any chain position
//!   that has `supporting(L)` in its ancestor closure without
//!   breaking references. This is the structural width of `L`'s
//!   freedom in the partial order.
//! - **One-pass.** Computing `supporting(L)` is a single walk of
//!   `L`'s parent chain, stopping at the first ancestor that defines
//!   any external reference of `L`.
//!
//! This module is pure: it has no storage side effects. The
//! `LayerBuilder::build` hook caches the computed value on the
//! `Layer` itself; D25 §11.0's dedicated supporting-layer index
//! lands in PR 0 step 4.

use crate::layer::{Layer, LayerId};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Compute the supporting layer of a not-yet-committed resource set.
///
/// `resources` and `defined_iris` describe the layer being built (the
/// builder's owned state inside [`LayerBuilder::build`]); `parent` is
/// the layer's first-parent chain pointer (i.e. what
/// [`Layer::parent`] would return after `build`).
///
/// Returns `None` in three cases, all equivalent to "no supporting
/// ancestor needed":
/// - `parent` is `None` (the root layer has no ancestors).
/// - The layer has no external references (every IRI it mentions, it
///   also defines).
/// - No ancestor defines any of the layer's external references. This
///   means the layer is *invalid* (a reference doesn't resolve in the
///   chain). Chain validation surfaces that elsewhere; this function
///   returns `None` rather than panicking.
///
/// Complexity: O(|resources|) for reference extraction +
/// O(|external_refs| · |walked_ancestors|) for the chain walk, which
/// short-circuits at the first ancestor that contributes.
pub fn compute_supporting_layer(
    resources: &BTreeMap<Iri, Resource>,
    defined_iris: &BTreeSet<Iri>,
    parent: Option<&Arc<Layer>>,
) -> Option<LayerId> {
    let parent = parent?;
    let references = collect_external_references(resources, defined_iris);
    if references.is_empty() {
        return None;
    }
    // Walk head→root through the first-parent chain. The supporting
    // layer is the first ancestor that defines any external reference
    // — its ancestor closure (itself + everything below) then covers
    // every remaining reference, because chain validation already
    // ensures every reference resolves somewhere in the chain.
    let mut current: Option<&Layer> = Some(parent.as_ref());
    while let Some(layer) = current {
        for iri in &references {
            if layer.defined_iris().contains(iri) {
                return Some(layer.id().clone());
            }
        }
        current = layer.parent().map(|p| p.as_ref());
    }
    None
}

/// Collect every IRI the layer references in its content, minus the
/// IRIs the layer itself defines.
///
/// References are:
/// - Every property IRI used as a key on any resource (the property
///   definition lives elsewhere in the chain).
/// - Every IRI carried as a `Value::ResourceRef`, recursively through
///   `Value::Array` and `Value::Embedded`. This covers `is_a` class
///   refs, `subclass_of` / `requires` / `recommends` / `domain` /
///   `class_types` / `data_type` / `format_constraints` /
///   inductive-ctor `arg_types` / comorphism `export_format` etc.
///   (D33 §4.2's full reference list).
///
/// `Value::String` is *not* treated as a reference even when it looks
/// like an IRI: the `canonicalise_resource_refs` pass (run before
/// this computation in `LayerBuilder::build`) has already upgraded
/// every `String` IRI that lives under a known `resource` /
/// `resource_array` property to `Value::ResourceRef`. Strings that
/// remain are properties with unknown `data_type` (custom extensions
/// the validator hasn't typed) — we don't second-guess their
/// semantics.
fn collect_external_references(
    resources: &BTreeMap<Iri, Resource>,
    defined_iris: &BTreeSet<Iri>,
) -> BTreeSet<Iri> {
    let mut refs = BTreeSet::new();
    for resource in resources.values() {
        collect_refs_from_resource(resource, &mut refs);
    }
    // Subtract the layer's own definitions in one pass — saves
    // allocating a separate "external" set just to remove a handful
    // of entries.
    for iri in defined_iris {
        refs.remove(iri);
    }
    refs
}

fn collect_refs_from_resource(resource: &Resource, out: &mut BTreeSet<Iri>) {
    for (prop, value) in resource.properties() {
        // The property IRI itself is a reference: its definition lives
        // somewhere in the chain (typically the core ontology or a
        // domain layer above it).
        out.insert(prop.clone());
        collect_refs_from_value(value, out);
    }
}

fn collect_refs_from_value(value: &Value, out: &mut BTreeSet<Iri>) {
    match value {
        Value::ResourceRef(iri) => {
            out.insert(iri.clone());
        }
        Value::Array(items) => {
            for item in items {
                collect_refs_from_value(item, out);
            }
        }
        Value::Embedded(inner) => {
            collect_refs_from_resource(inner.as_ref(), out);
        }
        // String / Integer / Float / Boolean / Json never carry
        // typed-reference semantics here (see module docs).
        Value::String(_)
        | Value::Integer(_)
        | Value::Float(_)
        | Value::Boolean(_)
        | Value::Json(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::layer::LayerStorage;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Helper: build a layer with the given resources at the given
    /// parent. Resources are `(id, properties)` pairs. Properties are
    /// `(prop_iri, value)` pairs.
    fn build_layer(
        name: &str,
        parent: Option<Arc<Layer>>,
        resources: Vec<(&str, Vec<(&str, Value)>)>,
    ) -> Layer {
        let mut b = LayerBuilder::new(name, parent);
        for (id, props) in resources {
            let mut r = Resource::new(iri(id));
            for (p, v) in props {
                r.set(iri(p), v);
            }
            b.add_resource(r).unwrap();
        }
        b.build(LayerStorage::in_memory())
    }

    /// A pure-root layer with only self-references has no supporting
    /// layer: `parent` is `None`.
    #[test]
    fn root_layer_has_no_supporting_layer() {
        let mut builder = LayerBuilder::new("root", None);
        let mut r = Resource::new(iri("urn:eigenius:core:Foo"));
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String("foo".into()),
        );
        builder.add_resource(r).unwrap();
        let defined: BTreeSet<Iri> = builder.resources().keys().cloned().collect();
        let resources = builder.resources().clone();
        assert!(compute_supporting_layer(&resources, &defined, None).is_none());
    }

    /// A child layer that only references IRIs from a deep root (and
    /// not from its immediate parent) gets the root as its supporting
    /// layer — that's the position-freedom payoff: this child can
    /// float to any chain position descended from root.
    #[test]
    fn supporting_layer_is_root_when_only_root_refs() {
        let root = Arc::new(build_layer(
            "root",
            None,
            vec![(
                "urn:eigenius:core:ClassA",
                vec![("urn:eigenius:core:description", Value::String("A".into()))],
            )],
        ));
        let middle = Arc::new(build_layer(
            "middle",
            Some(Arc::clone(&root)),
            vec![(
                "urn:eigenius:demo:Unrelated",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("middle".into()),
                )],
            )],
        ));
        // Child layer references core:ClassA (defined in root).
        let mut b = LayerBuilder::new("child", Some(Arc::clone(&middle)));
        let mut child_resource = Resource::new(iri("urn:eigenius:demo:X"));
        child_resource.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:core:ClassA"))]),
        );
        b.add_resource(child_resource).unwrap();
        let resources = b.resources().clone();
        let defined: BTreeSet<Iri> = resources.keys().cloned().collect();
        let supporting = compute_supporting_layer(&resources, &defined, Some(&middle));
        assert_eq!(
            supporting.as_ref(),
            Some(root.id()),
            "child references only root content → supporting layer is root, \
             not the immediate parent — child has chain-wide position freedom"
        );
    }

    /// A child layer that references content from its immediate
    /// parent gets the parent as its supporting layer (the youngest
    /// ancestor it depends on).
    #[test]
    fn supporting_layer_is_immediate_parent_when_parent_refs() {
        let root = Arc::new(build_layer(
            "root",
            None,
            vec![(
                "urn:eigenius:core:ClassA",
                vec![("urn:eigenius:core:description", Value::String("A".into()))],
            )],
        ));
        let middle = Arc::new(build_layer(
            "middle",
            Some(Arc::clone(&root)),
            vec![(
                "urn:eigenius:demo:ClassB",
                vec![("urn:eigenius:core:description", Value::String("B".into()))],
            )],
        ));
        // Child references demo:ClassB (defined in middle) AND
        // core:ClassA (defined in root). Supporting layer should be
        // middle — the youngest dependency.
        let mut b = LayerBuilder::new("child", Some(Arc::clone(&middle)));
        let mut child_resource = Resource::new(iri("urn:eigenius:demo:Y"));
        child_resource.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![
                Value::ResourceRef(iri("urn:eigenius:core:ClassA")),
                Value::ResourceRef(iri("urn:eigenius:demo:ClassB")),
            ]),
        );
        b.add_resource(child_resource).unwrap();
        let resources = b.resources().clone();
        let defined: BTreeSet<Iri> = resources.keys().cloned().collect();
        let supporting = compute_supporting_layer(&resources, &defined, Some(&middle));
        assert_eq!(supporting.as_ref(), Some(middle.id()));
    }

    /// A layer whose only references are to its own definitions has
    /// no supporting layer beyond the chain — `references - defined =
    /// ∅`.
    #[test]
    fn pure_self_referential_layer_has_no_supporting_layer() {
        let root = Arc::new(build_layer(
            "root",
            None,
            vec![("urn:eigenius:core:Bootstrap", vec![])],
        ));
        // Layer defines two classes; one references the other; both
        // are local.
        let mut b = LayerBuilder::new("self_ref", Some(Arc::clone(&root)));
        let mut a = Resource::new(iri("urn:eigenius:demo:A"));
        let mut bb = Resource::new(iri("urn:eigenius:demo:B"));
        bb.set(
            iri("urn:eigenius:demo:related_to"),
            Value::ResourceRef(iri("urn:eigenius:demo:A")),
        );
        // No external refs: the only ResourceRef is to demo:A (also
        // defined here); the only property IRI used is
        // demo:related_to. But demo:related_to itself isn't in
        // defined_iris of this layer — so it IS an external ref. To
        // make the test "no supporting layer," we need the property
        // IRI to be self-defined too. Drop the related_to property.
        a.set(
            iri("urn:eigenius:demo:A"),
            Value::ResourceRef(iri("urn:eigenius:demo:A")),
        );
        // Self-referential: A's properties are {demo:A → demo:A}.
        // Both are defined here. So no external refs.
        b.add_resource(a).unwrap();
        // Don't add `bb` — its related_to property would be external.
        let _ = bb;
        let resources = b.resources().clone();
        let defined: BTreeSet<Iri> = resources.keys().cloned().collect();
        let supporting = compute_supporting_layer(&resources, &defined, Some(&root));
        assert!(
            supporting.is_none(),
            "fully self-referential layer has no supporting ancestor"
        );
    }

    /// References to property IRIs (the map keys) count as external
    /// even when no ResourceRef appears in the values. Verifies the
    /// "property IRIs are references too" branch.
    #[test]
    fn property_iri_counts_as_reference() {
        let root = Arc::new(build_layer(
            "root",
            None,
            vec![("urn:eigenius:core:description", vec![])],
        ));
        let mut b = LayerBuilder::new("child", Some(Arc::clone(&root)));
        let mut r = Resource::new(iri("urn:eigenius:demo:Note"));
        // The property is core:description — defined in root, not
        // here. Only String value, no ResourceRef. The property IRI
        // itself is the external reference.
        r.set(
            iri("urn:eigenius:core:description"),
            Value::String("hello".into()),
        );
        b.add_resource(r).unwrap();
        let resources = b.resources().clone();
        let defined: BTreeSet<Iri> = resources.keys().cloned().collect();
        let supporting = compute_supporting_layer(&resources, &defined, Some(&root));
        assert_eq!(
            supporting.as_ref(),
            Some(root.id()),
            "property IRI is a reference; supporting layer must be where it's defined"
        );
    }
}
