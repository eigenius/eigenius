//! Layer system for stratified ontology composition.
//!
//! Layers hold resources and form a chain via parent pointers.
//! Each layer sees everything below it as a read-only view.
//! The root layer (no parent) is the core ontology layer.
//!
//! A `Layer` is immutable once built. Use `LayerBuilder` to accumulate
//! resources and produce an immutable `Layer` via `build()`.

use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Content-addressed layer identifier (SHA-256 hash).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub [u8; 32]);

impl fmt::Debug for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LayerId({})", hex::encode(self.0))
    }
}

impl fmt::Display for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

/// An immutable layer in the chain.
///
/// Each layer holds its own resources and an optional parent pointer.
/// Resolution walks the parent chain: check self, then parent, then
/// grandparent, down to the root. The root layer (`parent.is_none()`)
/// holds the core ontology.
#[derive(Debug, Clone)]
pub struct Layer {
    id: LayerId,
    name: String,
    resources: BTreeMap<Iri, Resource>,
    parent: Option<Arc<Layer>>,
}

impl Layer {
    /// Returns the content-addressed identifier of this layer.
    pub fn id(&self) -> &LayerId {
        &self.id
    }

    /// Returns the human-readable name of this layer.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the parent layer, if any.
    pub fn parent(&self) -> Option<&Arc<Layer>> {
        self.parent.as_ref()
    }

    /// Returns true if this is the root layer (no parent).
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }

    /// Look up a resource in this layer only (does not walk parents).
    pub fn get_resource(&self, iri: &Iri) -> Option<&Resource> {
        self.resources.get(iri)
    }

    /// Returns all resources in this layer (not including parents).
    pub fn resources(&self) -> &BTreeMap<Iri, Resource> {
        &self.resources
    }

    /// Resolve a resource by IRI, walking the parent chain.
    ///
    /// Checks this layer first, then parent, then grandparent, etc.
    /// Returns the first match (topmost layer wins).
    pub fn resolve(&self, iri: &Iri) -> Option<&Resource> {
        if let Some(r) = self.resources.get(iri) {
            return Some(r);
        }
        if let Some(parent) = &self.parent {
            return parent.resolve(iri);
        }
        None
    }

    /// Collect all resources at this IRI across the entire chain (top to bottom).
    pub fn resolve_all(&self, iri: &Iri) -> Vec<&Resource> {
        let mut results = Vec::new();
        if let Some(r) = self.resources.get(iri) {
            results.push(r);
        }
        if let Some(parent) = &self.parent {
            results.extend(parent.resolve_all(iri));
        }
        results
    }

    /// Merged view of all resources across the entire chain.
    /// Top layer wins for duplicate IRIs.
    pub fn all_resources(&self) -> BTreeMap<&Iri, &Resource> {
        let mut merged = BTreeMap::new();
        // Start from root so that top layers overwrite
        self.collect_resources_bottom_up(&mut merged);
        merged
    }

    fn collect_resources_bottom_up<'a>(&'a self, merged: &mut BTreeMap<&'a Iri, &'a Resource>) {
        if let Some(parent) = &self.parent {
            parent.collect_resources_bottom_up(merged);
        }
        for (iri, resource) in &self.resources {
            merged.insert(iri, resource);
        }
    }
}

/// Error that can occur when building a layer.
#[derive(Debug, Clone)]
pub enum LayerError {
    /// Cannot add a core namespace resource to a non-root layer.
    CoreNamespaceViolation { iri: Iri },
    /// Resource has no @id (only top-level resources can be added to layers).
    MissingId,
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayerError::CoreNamespaceViolation { iri } => {
                write!(
                    f,
                    "cannot add core namespace resource '{iri}' to non-root layer"
                )
            }
            LayerError::MissingId => write!(f, "resource must have an @id to be added to a layer"),
        }
    }
}

impl std::error::Error for LayerError {}

/// Builder for constructing an immutable `Layer`.
///
/// Accumulates resources, then `build()` computes the content-addressed
/// `LayerId` and produces an immutable `Layer`.
pub struct LayerBuilder {
    name: String,
    resources: BTreeMap<Iri, Resource>,
    parent: Option<Arc<Layer>>,
}

impl LayerBuilder {
    /// Create a new builder.
    ///
    /// If `parent` is `None`, this builds a root layer (core ontology).
    pub fn new(name: &str, parent: Option<Arc<Layer>>) -> Self {
        Self {
            name: name.to_string(),
            resources: BTreeMap::new(),
            parent,
        }
    }

    /// Add a resource to the layer being built.
    ///
    /// Fails if:
    /// - The resource has no `@id`
    /// - The resource's IRI is in the core namespace but this is not a root layer
    pub fn add_resource(&mut self, resource: Resource) -> Result<(), LayerError> {
        let iri = resource.id().ok_or(LayerError::MissingId)?.clone();

        if iri.is_core() && self.parent.is_some() {
            return Err(LayerError::CoreNamespaceViolation { iri });
        }

        self.resources.insert(iri, resource);
        Ok(())
    }

    /// Returns true if the builder has a resource with the given IRI.
    pub fn has_resource(&self, iri: &Iri) -> bool {
        self.resources.contains_key(iri)
    }

    /// Get a resource from the builder by IRI.
    pub fn get_resource(&self, iri: &Iri) -> Option<&Resource> {
        self.resources.get(iri)
    }

    /// Returns the resources accumulated so far.
    pub fn resources(&self) -> &BTreeMap<Iri, Resource> {
        &self.resources
    }

    /// Build the immutable `Layer`.
    ///
    /// Computes the `LayerId` as the SHA-256 hash of the canonical form
    /// of all resources (sorted by IRI, minified JSON).
    pub fn build(self) -> Layer {
        let id = self.compute_layer_id();
        Layer {
            id,
            name: self.name,
            resources: self.resources,
            parent: self.parent,
        }
    }

    fn compute_layer_id(&self) -> LayerId {
        let mut hasher = Sha256::new();

        // Hash each resource's canonical form in IRI order (BTreeMap guarantees this)
        for (iri, resource) in &self.resources {
            hasher.update(iri.as_str().as_bytes());
            hasher.update(crate::ontology::eigon_json::canonicalize(resource));
        }

        let hash = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&hash);
        LayerId(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::resource::Value;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_resource(id: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        for (k, v) in props {
            r.set(iri(k), v);
        }
        r
    }

    #[test]
    fn build_root_layer() {
        let mut builder = LayerBuilder::new("core", None);
        builder
            .add_resource(make_resource("urn:eigenius:core:Class", vec![]))
            .unwrap();
        let layer = builder.build();
        assert!(layer.is_root());
        assert!(layer
            .get_resource(&iri("urn:eigenius:core:Class"))
            .is_some());
    }

    #[test]
    fn core_namespace_protection() {
        let root = Arc::new(LayerBuilder::new("core", None).build());
        let mut builder = LayerBuilder::new("domain", Some(root));
        let result = builder.add_resource(make_resource("urn:eigenius:core:Foo", vec![]));
        assert!(matches!(
            result,
            Err(LayerError::CoreNamespaceViolation { .. })
        ));
    }

    #[test]
    fn core_namespace_allowed_on_root() {
        let mut builder = LayerBuilder::new("core", None);
        let result = builder.add_resource(make_resource("urn:eigenius:core:Foo", vec![]));
        assert!(result.is_ok());
    }

    #[test]
    fn embedded_resource_rejected() {
        let mut builder = LayerBuilder::new("test", None);
        let embedded = Resource::new_embedded();
        assert!(matches!(
            builder.add_resource(embedded),
            Err(LayerError::MissingId)
        ));
    }

    #[test]
    fn resolve_walks_parent_chain() {
        // Build root with resource A
        let mut root_builder = LayerBuilder::new("root", None);
        root_builder
            .add_resource(make_resource(
                "urn:eigenius:core:A",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("from root".into()),
                )],
            ))
            .unwrap();
        let root = Arc::new(root_builder.build());

        // Build child with resource B
        let mut child_builder = LayerBuilder::new("child", Some(root));
        child_builder
            .add_resource(make_resource(
                "urn:eigenius:example:B",
                vec![(
                    "urn:eigenius:core:description",
                    Value::String("from child".into()),
                )],
            ))
            .unwrap();
        let child = child_builder.build();

        // Child can resolve both A (from root) and B (from self)
        assert!(child.resolve(&iri("urn:eigenius:core:A")).is_some());
        assert!(child.resolve(&iri("urn:eigenius:example:B")).is_some());
        // Non-existent returns None
        assert!(child.resolve(&iri("urn:eigenius:example:C")).is_none());
    }

    #[test]
    fn top_layer_shadows_parent() {
        let mut root_builder = LayerBuilder::new("root", None);
        root_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![("urn:eigenius:core:description", Value::String("v1".into()))],
            ))
            .unwrap();
        let root = Arc::new(root_builder.build());

        let mut child_builder = LayerBuilder::new("child", Some(root));
        child_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![("urn:eigenius:core:description", Value::String("v2".into()))],
            ))
            .unwrap();
        let child = child_builder.build();

        let resolved = child.resolve(&iri("urn:eigenius:example:X")).unwrap();
        let desc = resolved.get(&iri("urn:eigenius:core:description")).unwrap();
        assert_eq!(desc.as_str(), Some("v2")); // child wins
    }

    #[test]
    fn deterministic_layer_id() {
        let build = || {
            let mut builder = LayerBuilder::new("test", None);
            builder
                .add_resource(make_resource(
                    "urn:eigenius:core:A",
                    vec![(
                        "urn:eigenius:core:description",
                        Value::String("hello".into()),
                    )],
                ))
                .unwrap();
            builder.build()
        };

        let layer1 = build();
        let layer2 = build();
        assert_eq!(layer1.id(), layer2.id());
    }

    #[test]
    fn different_content_different_id() {
        let mut b1 = LayerBuilder::new("test", None);
        b1.add_resource(make_resource("urn:eigenius:core:A", vec![]))
            .unwrap();
        let l1 = b1.build();

        let mut b2 = LayerBuilder::new("test", None);
        b2.add_resource(make_resource("urn:eigenius:core:B", vec![]))
            .unwrap();
        let l2 = b2.build();

        assert_ne!(l1.id(), l2.id());
    }

    #[test]
    fn all_resources_merged() {
        let mut root_builder = LayerBuilder::new("root", None);
        root_builder
            .add_resource(make_resource("urn:eigenius:core:A", vec![]))
            .unwrap();
        root_builder
            .add_resource(make_resource("urn:eigenius:core:B", vec![]))
            .unwrap();
        let root = Arc::new(root_builder.build());

        let mut child_builder = LayerBuilder::new("child", Some(root));
        child_builder
            .add_resource(make_resource("urn:eigenius:example:C", vec![]))
            .unwrap();
        let child = child_builder.build();

        let all = child.all_resources();
        assert_eq!(all.len(), 3); // A, B from root + C from child
    }

    #[test]
    fn resolve_all_returns_both_layers() {
        let mut root_builder = LayerBuilder::new("root", None);
        root_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![("urn:eigenius:core:description", Value::String("v1".into()))],
            ))
            .unwrap();
        let root = Arc::new(root_builder.build());

        let mut child_builder = LayerBuilder::new("child", Some(root));
        child_builder
            .add_resource(make_resource(
                "urn:eigenius:example:X",
                vec![("urn:eigenius:core:description", Value::String("v2".into()))],
            ))
            .unwrap();
        let child = child_builder.build();

        let all = child.resolve_all(&iri("urn:eigenius:example:X"));
        assert_eq!(all.len(), 2);
    }
}
