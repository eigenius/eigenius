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

//! Resource and Value types for Eigenius.
//!
//! Everything in Eigenius is a Resource — classes, properties, data types,
//! formats, and instance data are all represented uniformly. A Resource
//! has an optional IRI identity and a set of property values.

use crate::ontology::iri::Iri;
use std::collections::BTreeMap;

/// A property value in the Eigon data model.
///
/// Values are typed according to the property definition's `data_type`.
/// The JSON-level representation determines which variant is used.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// UTF-8 string value.
    String(String),
    /// Signed integer in the 53-bit safe range.
    Integer(i64),
    /// 64-bit IEEE 754 floating-point number.
    Float(f64),
    /// Boolean true/false.
    Boolean(bool),
    /// Reference to another resource by IRI.
    ResourceRef(Iri),
    /// Embedded resource (no `@id`).
    Embedded(Box<Resource>),
    /// Ordered array of values (resource_array or value_array).
    Array(Vec<Value>),
    /// Opaque JSON value, not validated by the ontology.
    Json(serde_json::Value),
    /// D43 §4.1 — typed embedding vector produced by the `EMBED`
    /// primitive (M4). Carries the Embedder Component IRI it was
    /// produced by; dimensionality is `data.len()`. Vector values are
    /// **transient compute values**, not chain resources: they flow
    /// from `EMBED` into `VECTOR_NEAR` / `VECTOR_SIM` within a single
    /// query and do not survive to canonical CBOR or persisted Eigon-
    /// JSON (serialising one fails with a clear diagnostic — the
    /// chain stores vectors as `vec_seg:<I>:<L>` blobs per §2.4, not
    /// as inline property values).
    Vector {
        /// IRI of the Embedder Component that produced this vector.
        /// Used by `VECTOR_NEAR` / `VECTOR_SIM` typecheck (D43 §4.5)
        /// to verify model agreement against the queried property's
        /// active VectorIndex.
        model_iri: Iri,
        /// Packed `f32` vector data; `len()` is the dimensionality.
        data: Vec<f32>,
    },
}

impl Value {
    /// Returns the value as a string, if it is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the value as an integer, if it is one.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// Returns the value as a float, if it is one.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(n) => Some(*n),
            _ => None,
        }
    }

    /// Returns the value as a boolean, if it is one.
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Returns the value as a resource reference IRI, if it is one.
    pub fn as_resource_ref(&self) -> Option<&Iri> {
        match self {
            Value::ResourceRef(iri) => Some(iri),
            _ => None,
        }
    }

    /// Read the IRI text from any value that could represent an IRI:
    /// `Value::ResourceRef(iri)` (the canonical post-
    /// `canonicalise_resource_refs` shape) or `Value::String(s)` (the
    /// freshly-parsed pre-canonicalisation shape). Returns `None` for
    /// anything else.
    ///
    /// Use this — not `as_str` — from any reader that walks
    /// resource-typed property values (`is_a`, `subclass_of`,
    /// `requires`, `class_types`, etc.). `as_str` silently drops
    /// `ResourceRef` values because they aren't strings, which
    /// produced an entirely-empty topology graph for canonicalised
    /// chains until this method was added.
    pub fn as_iri_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            Value::ResourceRef(iri) => Some(iri.as_str()),
            _ => None,
        }
    }

    /// Read the value as an IRI reference, accepting both
    /// `ResourceRef` (the canonical post-`canonicalise_resource_refs`
    /// shape) and `String` (the parse-time shape from
    /// schema-agnostic Eigon-JSON parsing). Returns `None` when the
    /// value is neither, or when a `String` value can't be parsed
    /// as a valid IRI.
    ///
    /// Use this from any reader that needs an IRI off a
    /// resource-typed property and may run against either
    /// freshly-parsed or chain-canonicalised resources (RPC
    /// payloads, in-flight intermediates, FIBER-synthesised
    /// resources). Returns `Cow`-style — owned `Iri` for `String`
    /// (it had to be parsed), borrowed slice for `ResourceRef`.
    pub fn as_iri(&self) -> Option<Iri> {
        match self {
            Value::ResourceRef(iri) => Some(iri.clone()),
            Value::String(s) => Iri::parse(s).ok(),
            _ => None,
        }
    }

    /// Returns the value as an embedded resource, if it is one.
    pub fn as_embedded(&self) -> Option<&Resource> {
        match self {
            Value::Embedded(r) => Some(r),
            _ => None,
        }
    }

    /// Returns the value as an array, if it is one.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Returns the value's vector data + Embedder model IRI, if it
    /// is a `Vector`. The slice length is the dimensionality.
    pub fn as_vector(&self) -> Option<(&Iri, &[f32])> {
        match self {
            Value::Vector { model_iri, data } => Some((model_iri, data)),
            _ => None,
        }
    }

    /// Extracts resource reference IRIs from an array value.
    /// Handles both `ResourceRef` and `String` variants (since the JSON
    /// parser stores all strings as `Value::String` — the distinction
    /// between string literals and resource references is made by the
    /// property's data_type, not at parse time).
    /// Non-reference/non-string elements are silently skipped.
    pub fn as_iri_array(&self) -> Vec<Iri> {
        match self {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::ResourceRef(iri) => Some(iri.clone()),
                    Value::String(s) => Iri::parse(s).ok(),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        }
    }
}

/// A resource in the Eigon data model.
///
/// Resources are the universal data unit. Everything — classes, properties,
/// data types, formats, and instance data — is a Resource. Top-level resources
/// have an `@id` (IRI identity). Embedded resources have no `@id` and exist
/// only as property values of their parent.
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    /// IRI identity. `None` for embedded resources.
    id: Option<Iri>,
    /// Property values indexed by property IRI.
    /// BTreeMap for deterministic ordering (required for canonical hashing)
    /// and cache-friendly sequential access.
    properties: BTreeMap<Iri, Value>,
}

impl Resource {
    /// Create a new top-level resource with the given IRI.
    pub fn new(id: Iri) -> Self {
        Self {
            id: Some(id),
            properties: BTreeMap::new(),
        }
    }

    /// Create a new embedded resource (no `@id`).
    pub fn new_embedded() -> Self {
        Self {
            id: None,
            properties: BTreeMap::new(),
        }
    }

    /// Returns the resource's IRI identity, or `None` for embedded resources.
    pub fn id(&self) -> Option<&Iri> {
        self.id.as_ref()
    }

    /// Promote an embedded resource to a top-level resource by
    /// assigning an `@id`, or rebrand an existing top-level resource.
    /// Pass `None` to demote a top-level resource to embedded.
    pub fn set_id(&mut self, id: Option<Iri>) {
        self.id = id;
    }

    /// Returns true if this is a top-level resource (has an `@id`).
    pub fn is_top_level(&self) -> bool {
        self.id.is_some()
    }

    /// Get a property value by property IRI.
    pub fn get(&self, property: &Iri) -> Option<&Value> {
        self.properties.get(property)
    }

    /// Set a property value.
    pub fn set(&mut self, property: Iri, value: Value) {
        self.properties.insert(property, value);
    }

    /// Remove a property value, returning it if present.
    pub fn remove(&mut self, property: &Iri) -> Option<Value> {
        self.properties.remove(property)
    }

    /// Returns true if the resource has the given property.
    pub fn has(&self, property: &Iri) -> bool {
        self.properties.contains_key(property)
    }

    /// Returns all properties as a reference to the underlying BTreeMap.
    pub fn properties(&self) -> &BTreeMap<Iri, Value> {
        &self.properties
    }

    /// Returns the `is_a` class IRIs for this resource.
    ///
    /// Reads the `urn:eigenius:core:is_a` property and extracts
    /// all resource reference IRIs from the array value.
    pub fn is_a(&self) -> Vec<Iri> {
        let is_a_iri = match Iri::parse(crate::ontology::well_known::IS_A) {
            Ok(iri) => iri,
            Err(_) => return vec![],
        };
        match self.properties.get(&is_a_iri) {
            Some(value) => value.as_iri_array(),
            None => vec![],
        }
    }

    /// Returns true if this resource is an instance of the given class.
    pub fn is_instance_of(&self, class_iri: &Iri) -> bool {
        self.is_a().iter().any(|c| c == class_iri)
    }

    /// Returns an iterator over all property IRIs on this resource.
    pub fn property_iris(&self) -> impl Iterator<Item = &Iri> {
        self.properties.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    #[test]
    fn top_level_resource() {
        let r = Resource::new(iri("urn:eigenius:example:alice"));
        assert!(r.is_top_level());
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:alice");
    }

    #[test]
    fn embedded_resource() {
        let r = Resource::new_embedded();
        assert!(!r.is_top_level());
        assert!(r.id().is_none());
    }

    #[test]
    fn set_and_get_property() {
        let mut r = Resource::new(iri("urn:eigenius:example:alice"));
        let prop = iri("urn:eigenius:example:name");
        r.set(prop.clone(), Value::String("Alice".to_string()));
        assert_eq!(r.get(&prop).unwrap().as_str(), Some("Alice"));
    }

    #[test]
    fn is_a_returns_class_iris() {
        let mut r = Resource::new(iri("urn:eigenius:example:rex"));
        let is_a = iri("urn:eigenius:core:is_a");
        r.set(
            is_a,
            Value::Array(vec![
                Value::String("urn:eigenius:example:Dog".to_string()),
                Value::String("urn:eigenius:example:Pet".to_string()),
            ]),
        );
        let classes = r.is_a();
        assert_eq!(classes.len(), 2);
        assert_eq!(classes[0].as_str(), "urn:eigenius:example:Dog");
        assert_eq!(classes[1].as_str(), "urn:eigenius:example:Pet");
    }

    #[test]
    fn is_instance_of() {
        let mut r = Resource::new(iri("urn:eigenius:example:rex"));
        let is_a = iri("urn:eigenius:core:is_a");
        r.set(
            is_a,
            Value::Array(vec![Value::String("urn:eigenius:example:Dog".to_string())]),
        );
        assert!(r.is_instance_of(&iri("urn:eigenius:example:Dog")));
        assert!(!r.is_instance_of(&iri("urn:eigenius:example:Cat")));
    }

    #[test]
    fn value_accessors() {
        assert_eq!(Value::String("hi".into()).as_str(), Some("hi"));
        assert_eq!(Value::Integer(42).as_integer(), Some(42));
        assert_eq!(Value::Float(2.72).as_float(), Some(2.72));
        assert_eq!(Value::Boolean(true).as_boolean(), Some(true));
        assert!(Value::ResourceRef(iri("urn:a:b"))
            .as_resource_ref()
            .is_some());
        assert!(Value::String("hi".into()).as_integer().is_none());
    }

    #[test]
    fn vector_variant_construction_and_accessors() {
        let model = iri("urn:eigenius:embed:dummy:v1");
        let v = Value::Vector {
            model_iri: model.clone(),
            data: vec![0.1f32, 0.2, 0.3],
        };
        let (got_model, got_data) = v.as_vector().expect("should be a vector");
        assert_eq!(got_model.as_str(), model.as_str());
        assert_eq!(got_data.len(), 3);
        assert_eq!(got_data, &[0.1f32, 0.2, 0.3]);
        // Other accessors return None.
        assert!(v.as_str().is_none());
        assert!(v.as_integer().is_none());
        assert!(v.as_float().is_none());
        assert!(v.as_boolean().is_none());
        assert!(v.as_resource_ref().is_none());
        assert!(v.as_embedded().is_none());
        assert!(v.as_array().is_none());
    }

    #[test]
    fn vector_equality_requires_same_model_and_data() {
        let m1 = iri("urn:eigenius:embed:m1");
        let m2 = iri("urn:eigenius:embed:m2");
        let a = Value::Vector {
            model_iri: m1.clone(),
            data: vec![1.0, 2.0],
        };
        let a2 = Value::Vector {
            model_iri: m1.clone(),
            data: vec![1.0, 2.0],
        };
        let different_model = Value::Vector {
            model_iri: m2,
            data: vec![1.0, 2.0],
        };
        let different_data = Value::Vector {
            model_iri: m1,
            data: vec![1.0, 2.5],
        };
        assert_eq!(a, a2);
        assert_ne!(a, different_model);
        assert_ne!(a, different_data);
    }

    #[test]
    fn properties_are_ordered() {
        let mut r = Resource::new(iri("urn:eigenius:example:test"));
        r.set(iri("urn:z:prop"), Value::String("z".into()));
        r.set(iri("urn:a:prop"), Value::String("a".into()));
        r.set(iri("urn:m:prop"), Value::String("m".into()));

        let keys: Vec<&str> = r.property_iris().map(|i| i.as_str()).collect();
        assert_eq!(keys, vec!["urn:a:prop", "urn:m:prop", "urn:z:prop"]);
    }
}
