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

//! Eigenius WASM SDK.
//!
//! Provides `Resource` and `Value` types for building WASM components and
//! institutions that run in the Eigenius kernel or orchestrator. The SDK
//! mirrors the kernel's Eigon data model but has no kernel dependency —
//! it compiles cleanly for `wasm32-unknown-unknown`.
//!
//! # Example
//!
//! ```ignore
//! use eigenius_wasm_sdk::{Resource, Value};
//!
//! fn execute(input: Vec<u8>, _argument: Vec<u8>) -> Result<Vec<u8>, String> {
//!     let input = Resource::from_cbor(&input)?;
//!     let name = input.get_string("urn:example:name").ok_or("missing name")?;
//!
//!     let mut output = Resource::new();
//!     output.set("urn:example:greeting", Value::String(format!("Hello, {name}")));
//!     Ok(output.to_cbor())
//! }
//! ```

pub mod institution;

use std::collections::BTreeMap;

/// A property value in the Eigon data model. Mirrors the kernel's
/// `ontology::resource::Value` but owns its own data (no shared IRI type).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// UTF-8 string value.
    String(String),
    /// Signed 64-bit integer.
    Integer(i64),
    /// 64-bit IEEE 754 floating-point number.
    Float(f64),
    /// Boolean true/false.
    Boolean(bool),
    /// Ordered array of values.
    Array(Vec<Value>),
    /// Embedded resource (no `@id`).
    Embedded(Box<Resource>),
}

impl Value {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_embedded(&self) -> Option<&Resource> {
        match self {
            Value::Embedded(r) => Some(r),
            _ => None,
        }
    }
}

/// An Eigon resource. Either has an `@id` (top-level) or is embedded (no id).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Resource {
    id: Option<String>,
    properties: BTreeMap<String, Value>,
}

impl Resource {
    /// Create a new embedded resource (no `@id`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new top-level resource with an `@id`.
    pub fn with_id(iri: impl Into<String>) -> Self {
        Self {
            id: Some(iri.into()),
            properties: BTreeMap::new(),
        }
    }

    /// The resource's `@id`, if any.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Set the `@id`.
    pub fn set_id(&mut self, iri: impl Into<String>) {
        self.id = Some(iri.into());
    }

    /// Get a property value by IRI.
    pub fn get(&self, property_iri: &str) -> Option<&Value> {
        self.properties.get(property_iri)
    }

    /// Get a string property value.
    pub fn get_string(&self, property_iri: &str) -> Option<&str> {
        self.properties.get(property_iri)?.as_string()
    }

    /// Get an integer property value.
    pub fn get_integer(&self, property_iri: &str) -> Option<i64> {
        self.properties.get(property_iri)?.as_integer()
    }

    /// Get a boolean property value.
    pub fn get_boolean(&self, property_iri: &str) -> Option<bool> {
        self.properties.get(property_iri)?.as_boolean()
    }

    /// Set a property value.
    pub fn set(&mut self, property_iri: impl Into<String>, value: Value) {
        self.properties.insert(property_iri.into(), value);
    }

    /// Iterate over all properties in IRI order.
    pub fn properties(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.properties.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Get the `is_a` class IRIs for this resource.
    pub fn is_a(&self) -> Vec<&str> {
        match self.get("urn:eigenius:core:is_a") {
            Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_string()).collect(),
            _ => Vec::new(),
        }
    }

    /// Set the `is_a` class IRIs for this resource.
    pub fn set_is_a(&mut self, class_iris: impl IntoIterator<Item = impl Into<String>>) {
        let items: Vec<Value> = class_iris
            .into_iter()
            .map(|s| Value::String(s.into()))
            .collect();
        self.set("urn:eigenius:core:is_a", Value::Array(items));
    }

    /// Serialize to Eigon-CBOR bytes, matching the kernel's format exactly.
    pub fn to_cbor(&self) -> Vec<u8> {
        let cbor_value = self.to_cbor_value();
        let mut buf = Vec::new();
        ciborium::into_writer(&cbor_value, &mut buf).expect("CBOR serialization should not fail");
        buf
    }

    /// Parse from Eigon-CBOR bytes (accepts embedded resources without `@id`).
    pub fn from_cbor(cbor: &[u8]) -> Result<Self, String> {
        let value: ciborium::Value =
            ciborium::from_reader(cbor).map_err(|e| format!("CBOR decode error: {e}"))?;
        cbor_to_resource(&value)
    }

    fn to_cbor_value(&self) -> ciborium::Value {
        let mut entries: Vec<(ciborium::Value, ciborium::Value)> = Vec::new();
        if let Some(id) = &self.id {
            entries.push((
                ciborium::Value::Text("@id".to_string()),
                ciborium::Value::Text(id.clone()),
            ));
        }
        for (prop, value) in &self.properties {
            entries.push((ciborium::Value::Text(prop.clone()), value_to_cbor(value)));
        }
        ciborium::Value::Map(entries)
    }
}

fn value_to_cbor(value: &Value) -> ciborium::Value {
    match value {
        Value::String(s) => ciborium::Value::Text(s.clone()),
        Value::Integer(n) => ciborium::Value::Integer((*n).into()),
        Value::Float(f) => ciborium::Value::Float(*f),
        Value::Boolean(b) => ciborium::Value::Bool(*b),
        Value::Array(arr) => ciborium::Value::Array(arr.iter().map(value_to_cbor).collect()),
        Value::Embedded(r) => r.to_cbor_value(),
    }
}

fn cbor_to_resource(value: &ciborium::Value) -> Result<Resource, String> {
    let map = match value {
        ciborium::Value::Map(m) => m,
        _ => return Err("expected CBOR map for resource".to_string()),
    };

    let mut resource = Resource::new();
    for (k, v) in map {
        let key = match k {
            ciborium::Value::Text(s) => s,
            _ => return Err("expected text key in resource map".to_string()),
        };

        if key == "@id" {
            resource.set_id(match v {
                ciborium::Value::Text(s) => s.clone(),
                _ => return Err("@id must be a text value".to_string()),
            });
        } else {
            resource.set(key.clone(), cbor_to_value(v)?);
        }
    }
    Ok(resource)
}

fn cbor_to_value(value: &ciborium::Value) -> Result<Value, String> {
    match value {
        ciborium::Value::Text(s) => Ok(Value::String(s.clone())),
        ciborium::Value::Integer(i) => {
            let n: i128 = (*i).into();
            Ok(Value::Integer(n as i64))
        }
        ciborium::Value::Float(f) => Ok(Value::Float(*f)),
        ciborium::Value::Bool(b) => Ok(Value::Boolean(*b)),
        ciborium::Value::Array(arr) => Ok(Value::Array(
            arr.iter()
                .map(cbor_to_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        ciborium::Value::Map(_) => {
            let r = cbor_to_resource(value)?;
            Ok(Value::Embedded(Box::new(r)))
        }
        ciborium::Value::Null => Err("null values not allowed".to_string()),
        other => Err(format!("unsupported CBOR value type: {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_roundtrip_with_id() {
        let mut r = Resource::with_id("urn:example:alice");
        r.set("urn:example:name", Value::String("Alice".into()));
        r.set("urn:example:age", Value::Integer(30));

        let cbor = r.to_cbor();
        let parsed = Resource::from_cbor(&cbor).unwrap();

        assert_eq!(parsed.id(), Some("urn:example:alice"));
        assert_eq!(parsed.get_string("urn:example:name"), Some("Alice"));
        assert_eq!(parsed.get_integer("urn:example:age"), Some(30));
    }

    #[test]
    fn resource_roundtrip_embedded() {
        let mut r = Resource::new();
        r.set("urn:example:flag", Value::Boolean(true));

        let cbor = r.to_cbor();
        let parsed = Resource::from_cbor(&cbor).unwrap();

        assert_eq!(parsed.id(), None);
        assert_eq!(parsed.get_boolean("urn:example:flag"), Some(true));
    }

    #[test]
    fn resource_with_array() {
        let mut r = Resource::new();
        r.set(
            "urn:example:tags",
            Value::Array(vec![
                Value::String("alpha".into()),
                Value::String("beta".into()),
            ]),
        );

        let cbor = r.to_cbor();
        let parsed = Resource::from_cbor(&cbor).unwrap();
        let tags = parsed.get("urn:example:tags").unwrap().as_array().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].as_string(), Some("alpha"));
        assert_eq!(tags[1].as_string(), Some("beta"));
    }

    #[test]
    fn resource_with_nested() {
        let mut inner = Resource::new();
        inner.set("urn:example:street", Value::String("Main St".into()));

        let mut outer = Resource::new();
        outer.set("urn:example:address", Value::Embedded(Box::new(inner)));

        let cbor = outer.to_cbor();
        let parsed = Resource::from_cbor(&cbor).unwrap();
        let addr = parsed
            .get("urn:example:address")
            .unwrap()
            .as_embedded()
            .unwrap();
        assert_eq!(addr.get_string("urn:example:street"), Some("Main St"));
    }

    #[test]
    fn is_a_extracts_class_iris() {
        let mut r = Resource::new();
        r.set(
            "urn:eigenius:core:is_a",
            Value::Array(vec![Value::String("urn:example:Dog".into())]),
        );
        let classes = r.is_a();
        assert_eq!(classes, vec!["urn:example:Dog"]);
    }
}
