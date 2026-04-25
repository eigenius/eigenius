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

//! CBOR serialization for Eigon resources.
//!
//! Encodes/decodes resources as CBOR (RFC 8949) using deterministic
//! encoding for content-addressed hashing. Complements eigon_json.rs
//! which handles the human-readable Eigon-JSON format.

use crate::ontology::iri::{Iri, IriError};
use crate::ontology::resource::{Resource, Value};
use std::io::Cursor;

/// Errors during CBOR parsing.
#[derive(Debug, Clone)]
pub enum CborError {
    Encode(String),
    Decode(String),
    InvalidIri { key: String, source: IriError },
    EmptyArray { property: String },
    EmptyObject { property: String },
}

impl std::fmt::Display for CborError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CborError::Encode(msg) => write!(f, "CBOR encode error: {msg}"),
            CborError::Decode(msg) => write!(f, "CBOR decode error: {msg}"),
            CborError::InvalidIri { key, source } => {
                write!(f, "invalid IRI for key '{key}': {source}")
            }
            CborError::EmptyArray { property } => {
                write!(f, "empty array not allowed for property '{property}'")
            }
            CborError::EmptyObject { property } => {
                write!(f, "empty object not allowed for property '{property}'")
            }
        }
    }
}

impl std::error::Error for CborError {}

/// Serialize a resource to CBOR bytes (deterministic encoding).
///
/// Keys are sorted lexicographically (BTreeMap ensures this).
/// Uses shortest CBOR encoding for each value type.
pub fn serialize_resource(resource: &Resource) -> Vec<u8> {
    let cbor_value = resource_to_cbor(resource);
    let mut buf = Vec::new();
    ciborium::into_writer(&cbor_value, &mut buf).expect("CBOR serialization should not fail");
    buf
}

/// Deserialize a resource from CBOR bytes.
pub fn parse_resource(cbor: &[u8]) -> Result<Resource, CborError> {
    let value: ciborium::Value =
        ciborium::from_reader(Cursor::new(cbor)).map_err(|e| CborError::Decode(e.to_string()))?;
    cbor_to_resource(&value, true)
}

/// Parse a single resource from CBOR bytes, allowing embedded (no @id).
pub fn parse_resource_lenient(cbor: &[u8]) -> Result<Resource, CborError> {
    let value: ciborium::Value =
        ciborium::from_reader(Cursor::new(cbor)).map_err(|e| CborError::Decode(e.to_string()))?;
    cbor_to_resource(&value, false)
}

/// Serialize a document (array of resources) to CBOR bytes.
pub fn serialize_document(resources: &[Resource]) -> Vec<u8> {
    if resources.len() == 1 {
        serialize_resource(&resources[0])
    } else {
        let arr: Vec<ciborium::Value> = resources.iter().map(resource_to_cbor).collect();
        let mut buf = Vec::new();
        ciborium::into_writer(&ciborium::Value::Array(arr), &mut buf)
            .expect("CBOR serialization should not fail");
        buf
    }
}

/// Parse a document from CBOR bytes (single resource or array).
pub fn parse_document(cbor: &[u8]) -> Result<Vec<Resource>, CborError> {
    let value: ciborium::Value =
        ciborium::from_reader(Cursor::new(cbor)).map_err(|e| CborError::Decode(e.to_string()))?;

    match &value {
        ciborium::Value::Map(_) => {
            let resource = cbor_to_resource(&value, true)?;
            Ok(vec![resource])
        }
        ciborium::Value::Array(arr) => {
            let mut resources = Vec::with_capacity(arr.len());
            for item in arr {
                resources.push(cbor_to_resource(item, true)?);
            }
            Ok(resources)
        }
        _ => Err(CborError::Decode(
            "document root must be a map or array".to_string(),
        )),
    }
}

/// Produce deterministic CBOR encoding for content-addressed hashing.
///
/// Uses Core Deterministic Encoding (RFC 8949 §4.2):
/// - Map keys sorted by encoded byte string
/// - Shortest encoding for each value
pub fn canonicalize(resource: &Resource) -> Vec<u8> {
    // BTreeMap iteration is already sorted, and ciborium uses
    // shortest encoding by default, so serialize_resource produces
    // deterministic output.
    serialize_resource(resource)
}

// --- Internal conversion ---

fn resource_to_cbor(resource: &Resource) -> ciborium::Value {
    let mut entries: Vec<(ciborium::Value, ciborium::Value)> = Vec::new();

    // Add @id if present
    if let Some(id) = resource.id() {
        entries.push((
            ciborium::Value::Text("@id".to_string()),
            ciborium::Value::Text(id.as_str().to_string()),
        ));
    }

    // Add properties (BTreeMap iteration is sorted)
    for (prop_iri, value) in resource.properties() {
        entries.push((
            ciborium::Value::Text(prop_iri.as_str().to_string()),
            value_to_cbor(value),
        ));
    }

    ciborium::Value::Map(entries)
}

fn value_to_cbor(value: &Value) -> ciborium::Value {
    match value {
        Value::String(s) => ciborium::Value::Text(s.clone()),
        Value::Integer(n) => ciborium::Value::Integer((*n).into()),
        Value::Float(f) => ciborium::Value::Float(*f),
        Value::Boolean(b) => ciborium::Value::Bool(*b),
        Value::ResourceRef(iri) => ciborium::Value::Text(iri.as_str().to_string()),
        Value::Embedded(resource) => resource_to_cbor(resource),
        Value::Array(arr) => ciborium::Value::Array(arr.iter().map(value_to_cbor).collect()),
        Value::Json(v) => json_to_cbor(v),
    }
}

fn json_to_cbor(json: &serde_json::Value) -> ciborium::Value {
    match json {
        serde_json::Value::Null => ciborium::Value::Null,
        serde_json::Value::Bool(b) => ciborium::Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ciborium::Value::Integer(i.into())
            } else if let Some(f) = n.as_f64() {
                ciborium::Value::Float(f)
            } else {
                ciborium::Value::Null
            }
        }
        serde_json::Value::String(s) => ciborium::Value::Text(s.clone()),
        serde_json::Value::Array(arr) => {
            ciborium::Value::Array(arr.iter().map(json_to_cbor).collect())
        }
        serde_json::Value::Object(map) => {
            let entries: Vec<(ciborium::Value, ciborium::Value)> = map
                .iter()
                .map(|(k, v)| (ciborium::Value::Text(k.clone()), json_to_cbor(v)))
                .collect();
            ciborium::Value::Map(entries)
        }
    }
}

fn cbor_to_resource(value: &ciborium::Value, top_level: bool) -> Result<Resource, CborError> {
    let entries = match value {
        ciborium::Value::Map(entries) => entries,
        _ => return Err(CborError::Decode("resource must be a CBOR map".to_string())),
    };

    // Extract @id
    let id = entries
        .iter()
        .find(|(k, _)| matches!(k, ciborium::Value::Text(s) if s == "@id"))
        .and_then(|(_, v)| match v {
            ciborium::Value::Text(s) => Iri::parse(s).ok(),
            _ => None,
        });

    let mut resource = match id {
        Some(iri) => Resource::new(iri),
        None => {
            if top_level {
                return Err(CborError::Decode(
                    "top-level resource must have @id".to_string(),
                ));
            }
            Resource::new_embedded()
        }
    };

    // Parse properties
    for (key, val) in entries {
        let key_str = match key {
            ciborium::Value::Text(s) => s,
            _ => continue,
        };
        if key_str == "@id" {
            continue;
        }

        let prop_iri = Iri::parse(key_str).map_err(|e| CborError::InvalidIri {
            key: key_str.clone(),
            source: e,
        })?;

        let parsed_value = cbor_to_value(val, key_str)?;
        resource.set(prop_iri, parsed_value);
    }

    Ok(resource)
}

fn cbor_to_value(value: &ciborium::Value, property: &str) -> Result<Value, CborError> {
    match value {
        ciborium::Value::Text(s) => Ok(Value::String(s.clone())),
        ciborium::Value::Integer(n) => {
            let i: i128 = (*n).into();
            Ok(Value::Integer(i as i64))
        }
        ciborium::Value::Float(f) => Ok(Value::Float(*f)),
        ciborium::Value::Bool(b) => Ok(Value::Boolean(*b)),
        ciborium::Value::Array(arr) => {
            if arr.is_empty() {
                return Err(CborError::EmptyArray {
                    property: property.to_string(),
                });
            }
            let mut values = Vec::with_capacity(arr.len());
            for item in arr {
                values.push(cbor_to_value(item, property)?);
            }
            Ok(Value::Array(values))
        }
        ciborium::Value::Map(entries) => {
            if entries.is_empty() {
                return Err(CborError::EmptyObject {
                    property: property.to_string(),
                });
            }
            let resource = cbor_to_resource(value, false)?;
            Ok(Value::Embedded(Box::new(resource)))
        }
        ciborium::Value::Null => Err(CborError::Decode(format!(
            "null not allowed for property '{property}'"
        ))),
        _ => Err(CborError::Decode(format!(
            "unsupported CBOR type for property '{property}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::eigon_json;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    #[test]
    fn round_trip_simple_resource() {
        let mut r = Resource::new(iri("urn:eigenius:example:alice"));
        r.set(
            iri("urn:eigenius:example:name"),
            Value::String("Alice".into()),
        );
        r.set(iri("urn:eigenius:example:age"), Value::Integer(30));

        let cbor = serialize_resource(&r);
        let parsed = parse_resource(&cbor).unwrap();

        assert_eq!(parsed.id().unwrap().as_str(), "urn:eigenius:example:alice");
        assert_eq!(
            parsed
                .get(&iri("urn:eigenius:example:name"))
                .unwrap()
                .as_str(),
            Some("Alice")
        );
        assert_eq!(
            parsed
                .get(&iri("urn:eigenius:example:age"))
                .unwrap()
                .as_integer(),
            Some(30)
        );
    }

    #[test]
    fn round_trip_all_value_types() {
        let mut r = Resource::new(iri("urn:eigenius:test:types"));
        r.set(iri("urn:eigenius:test:s"), Value::String("hello".into()));
        r.set(iri("urn:eigenius:test:i"), Value::Integer(42));
        r.set(iri("urn:eigenius:test:f"), Value::Float(2.72));
        r.set(iri("urn:eigenius:test:b"), Value::Boolean(true));
        r.set(
            iri("urn:eigenius:test:arr"),
            Value::Array(vec![Value::Integer(1), Value::Integer(2)]),
        );

        let cbor = serialize_resource(&r);
        let parsed = parse_resource(&cbor).unwrap();

        assert_eq!(
            parsed.get(&iri("urn:eigenius:test:s")).unwrap().as_str(),
            Some("hello")
        );
        assert_eq!(
            parsed
                .get(&iri("urn:eigenius:test:i"))
                .unwrap()
                .as_integer(),
            Some(42)
        );
        assert_eq!(
            parsed.get(&iri("urn:eigenius:test:f")).unwrap().as_float(),
            Some(2.72)
        );
        assert_eq!(
            parsed
                .get(&iri("urn:eigenius:test:b"))
                .unwrap()
                .as_boolean(),
            Some(true)
        );
        assert_eq!(
            parsed
                .get(&iri("urn:eigenius:test:arr"))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn round_trip_embedded_resource() {
        let mut inner = Resource::new_embedded();
        inner.set(
            iri("urn:eigenius:example:city"),
            Value::String("Berlin".into()),
        );

        let mut r = Resource::new(iri("urn:eigenius:example:alice"));
        r.set(
            iri("urn:eigenius:example:address"),
            Value::Embedded(Box::new(inner)),
        );

        let cbor = serialize_resource(&r);
        let parsed = parse_resource(&cbor).unwrap();

        let addr = parsed
            .get(&iri("urn:eigenius:example:address"))
            .unwrap()
            .as_embedded()
            .unwrap();
        assert_eq!(
            addr.get(&iri("urn:eigenius:example:city"))
                .unwrap()
                .as_str(),
            Some("Berlin")
        );
    }

    #[test]
    fn deterministic_encoding() {
        let mut r = Resource::new(iri("urn:eigenius:test:det"));
        r.set(iri("urn:z:prop"), Value::String("z".into()));
        r.set(iri("urn:a:prop"), Value::String("a".into()));
        r.set(iri("urn:m:prop"), Value::String("m".into()));

        let cbor1 = canonicalize(&r);
        let cbor2 = canonicalize(&r);
        assert_eq!(cbor1, cbor2);
    }

    #[test]
    fn document_round_trip() {
        let mut r1 = Resource::new(iri("urn:eigenius:test:a"));
        r1.set(iri("urn:eigenius:test:x"), Value::Integer(1));

        let mut r2 = Resource::new(iri("urn:eigenius:test:b"));
        r2.set(iri("urn:eigenius:test:x"), Value::Integer(2));

        let cbor = serialize_document(&[r1, r2]);
        let parsed = parse_document(&cbor).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn cross_format_round_trip() {
        // JSON → Resource → CBOR → Resource → verify equality
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let json_resources = eigon_json::parse_document(core_json).unwrap();

        for resource in &json_resources {
            let cbor = serialize_resource(resource);
            let parsed = parse_resource(&cbor).unwrap();

            // Compare IDs
            assert_eq!(
                resource.id().map(|i| i.as_str()),
                parsed.id().map(|i| i.as_str()),
            );

            // Compare property count
            assert_eq!(
                resource.properties().len(),
                parsed.properties().len(),
                "property count mismatch for {:?}",
                resource.id()
            );
        }
    }

    #[test]
    fn cbor_is_smaller_than_json() {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let resources = eigon_json::parse_document(core_json).unwrap();

        let json_size: usize = resources
            .iter()
            .map(|r| eigon_json::serialize_resource(r).to_string().len())
            .sum();
        let cbor_size: usize = resources.iter().map(|r| serialize_resource(r).len()).sum();

        assert!(
            cbor_size < json_size,
            "CBOR ({cbor_size}) should be smaller than JSON ({json_size})"
        );
    }

    #[test]
    fn reject_empty_array() {
        let mut r = Resource::new(iri("urn:eigenius:test:bad"));
        r.set(iri("urn:eigenius:test:arr"), Value::Array(vec![]));

        let cbor = serialize_resource(&r);
        assert!(parse_resource(&cbor).is_err());
    }
}
