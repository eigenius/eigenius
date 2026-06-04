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

//! Eigon-JSON parser and serializer.
//!
//! Parses and serializes resources in the Eigon-JSON format as specified
//! in design doc D1. Supports single-resource and multi-resource documents.

use crate::ontology::iri::{Iri, IriError};
use crate::ontology::resource::{Resource, Value};

/// Errors that can occur during Eigon-JSON parsing.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// Invalid JSON syntax.
    JsonSyntax(String),
    /// Document root must be an object or array of objects.
    InvalidDocumentRoot,
    /// IRI validation failed.
    InvalidIri { key: String, source: IriError },
    /// Explicit null values are not allowed.
    NullNotAllowed { property: String },
    /// Empty objects are not allowed.
    EmptyObject { property: String },
    /// Embedded resource must not have an @id.
    EmbeddedWithId,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::JsonSyntax(msg) => write!(f, "JSON syntax error: {msg}"),
            ParseError::InvalidDocumentRoot => {
                write!(f, "document root must be an object or array of objects")
            }
            ParseError::InvalidIri { key, source } => {
                write!(f, "invalid IRI for key '{key}': {source}")
            }
            ParseError::NullNotAllowed { property } => {
                write!(f, "null value not allowed for property '{property}'")
            }
            ParseError::EmptyObject { property } => {
                write!(f, "empty object not allowed for property '{property}'")
            }
            ParseError::EmbeddedWithId => {
                write!(f, "embedded resource must not have an @id")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse an Eigon-JSON document into a list of top-level resources.
///
/// Accepts either a single resource (JSON object with `@id`) or
/// an array of resources (JSON array of objects with `@id`).
pub fn parse_document(json: &str) -> Result<Vec<Resource>, ParseError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ParseError::JsonSyntax(e.to_string()))?;

    match &value {
        serde_json::Value::Object(_) => {
            let resource = parse_top_level_resource(&value)?;
            Ok(vec![resource])
        }
        serde_json::Value::Array(arr) => {
            let mut resources = Vec::with_capacity(arr.len());
            for item in arr {
                resources.push(parse_top_level_resource(item)?);
            }
            Ok(resources)
        }
        _ => Err(ParseError::InvalidDocumentRoot),
    }
}

/// Parse a JSON string as an embedded resource (no `@id` required).
///
/// Useful for parsing component outputs and query results that are
/// not top-level resources.
pub fn parse_embedded(json: &str) -> Result<Resource, ParseError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ParseError::JsonSyntax(e.to_string()))?;
    parse_embedded_resource(&value, "<root>")
}

/// Parse a JSON object as a top-level resource (must have `@id`).
fn parse_top_level_resource(value: &serde_json::Value) -> Result<Resource, ParseError> {
    let obj = match value.as_object() {
        Some(obj) => obj,
        None => return Err(ParseError::InvalidDocumentRoot),
    };

    let id_value = obj.get("@id").ok_or(ParseError::InvalidDocumentRoot)?;
    let id_str = id_value.as_str().ok_or(ParseError::InvalidDocumentRoot)?;
    let id = Iri::parse(id_str).map_err(|e| ParseError::InvalidIri {
        key: "@id".to_string(),
        source: e,
    })?;

    let mut resource = Resource::new(id);
    parse_properties(obj, &mut resource)?;
    Ok(resource)
}

/// Parse a JSON object as an embedded resource (must NOT have `@id`).
fn parse_embedded_resource(
    value: &serde_json::Value,
    parent_property: &str,
) -> Result<Resource, ParseError> {
    let obj = match value.as_object() {
        Some(obj) => obj,
        None => {
            return Err(ParseError::EmptyObject {
                property: parent_property.to_string(),
            })
        }
    };

    if obj.contains_key("@id") {
        return Err(ParseError::EmbeddedWithId);
    }

    if obj.is_empty() {
        return Err(ParseError::EmptyObject {
            property: parent_property.to_string(),
        });
    }

    let mut resource = Resource::new_embedded();
    parse_properties(obj, &mut resource)?;
    Ok(resource)
}

/// Parse all non-@id properties from a JSON object into a Resource.
fn parse_properties(
    obj: &serde_json::Map<String, serde_json::Value>,
    resource: &mut Resource,
) -> Result<(), ParseError> {
    for (key, value) in obj {
        if key == "@id" {
            continue;
        }

        let property_iri = Iri::parse(key).map_err(|e| ParseError::InvalidIri {
            key: key.clone(),
            source: e,
        })?;

        let parsed_value = parse_value(value, key)?;
        resource.set(property_iri, parsed_value);
    }
    Ok(())
}

/// Parse a JSON value into an Eigon Value.
///
/// The parser does not know the expected data type at this point —
/// that's the validator's job. Instead, it infers the Value variant
/// from the JSON structure:
///
/// - JSON string → try as IRI (ResourceRef) if it looks like one, otherwise String
///   (but we can't distinguish reliably without the property definition,
///   so all strings become Value::String; the validator will interpret them
///   based on the property's data_type)
/// - JSON number → Integer if no decimal, Float otherwise
/// - JSON boolean → Boolean
/// - JSON object → Embedded resource
/// - JSON array → Array of values
/// - JSON null → error
fn parse_value(value: &serde_json::Value, property: &str) -> Result<Value, ParseError> {
    match value {
        serde_json::Value::Null => Err(ParseError::NullNotAllowed {
            property: property.to_string(),
        }),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                // Shouldn't happen with serde_json, but handle gracefully
                Ok(Value::Float(0.0))
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Array(arr) => {
            // Empty arrays are valid JSON and valid Eigon — they
            // represent "no items." Per-property "must be non-empty"
            // semantics (e.g., `is_a`, `requires`) are enforced by the
            // validator, not the parser.
            let mut values = Vec::with_capacity(arr.len());
            for item in arr {
                values.push(parse_value(item, property)?);
            }
            Ok(Value::Array(values))
        }
        serde_json::Value::Object(obj) => {
            if obj.is_empty() {
                return Err(ParseError::EmptyObject {
                    property: property.to_string(),
                });
            }
            // Discriminate Resource-shape vs. opaque JSON: a Resource
            // has IRI-shaped keys (or `@id`) so its property keys
            // resolve in the chain. An object whose keys are all
            // bare strings (`ctor`, `args`, …) is opaque JSON — used
            // for `data_type: core:json` and `core:inductive`
            // property values where the wire shape is a tagged dict
            // tree, not a typed Resource. D32 §3.7.
            let any_iri_key = obj.keys().any(|k| k == "@id" || Iri::parse(k).is_ok());
            if any_iri_key {
                let resource = parse_embedded_resource(value, property)?;
                Ok(Value::Embedded(Box::new(resource)))
            } else {
                Ok(Value::Json(value.clone()))
            }
        }
    }
}

/// Serialize a resource to an Eigon-JSON value.
pub fn serialize_resource(resource: &Resource) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    if let Some(id) = resource.id() {
        map.insert(
            "@id".to_string(),
            serde_json::Value::String(id.as_str().to_string()),
        );
    }

    for (prop_iri, value) in resource.properties() {
        map.insert(prop_iri.as_str().to_string(), serialize_value(value));
    }

    serde_json::Value::Object(map)
}

/// Serialize a Value to a JSON value.
///
/// `Value::Vector` is treated as a programming-error invariant: the
/// D43 design (§4.1, §5) makes vectors transient compute values that
/// flow from `EMBED` into `VECTOR_NEAR` / `VECTOR_SIM` within a single
/// query and are persisted as `vec_seg:<I>:<L>` blobs (§2.4), never as
/// inline property values. Reaching this arm means a Vector ended up
/// on a Resource that's being canonicalised or wire-serialised — that
/// is structurally wrong and should be caught before this point.
fn serialize_value(value: &Value) -> serde_json::Value {
    match value {
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Integer(n) => serde_json::json!(*n),
        Value::Float(f) => serde_json::json!(*f),
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::ResourceRef(iri) => serde_json::Value::String(iri.as_str().to_string()),
        Value::Embedded(resource) => serialize_resource(resource),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(serialize_value).collect()),
        Value::Json(v) => v.clone(),
        Value::Vector { model_iri, data } => panic!(
            "Value::Vector is transient and must not reach JSON serialisation; \
             model={}, dim={}",
            model_iri.as_str(),
            data.len()
        ),
    }
}

/// Serialize a list of resources to an Eigon-JSON document.
pub fn serialize_document(resources: &[Resource]) -> serde_json::Value {
    if resources.len() == 1 {
        serialize_resource(&resources[0])
    } else {
        serde_json::Value::Array(resources.iter().map(serialize_resource).collect())
    }
}

/// Produce the RFC 8785 canonical form of a resource for content-addressed hashing.
///
/// This produces a deterministic byte sequence:
/// - Keys sorted lexicographically (BTreeMap already ensures this)
/// - No insignificant whitespace
/// - Deterministic number representation
pub fn canonicalize(resource: &Resource) -> Vec<u8> {
    let json = serialize_resource(resource);
    // serde_json::to_vec produces minified output.
    // BTreeMap iteration is already sorted, so keys come out in order.
    // serde_json handles number representation deterministically.
    serde_json::to_vec(&json).expect("Resource serialization should not fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_resource() {
        let json = r#"{
            "@id": "urn:eigenius:example:alice",
            "urn:eigenius:core:is_a": ["urn:eigenius:example:Person"],
            "urn:eigenius:example:name": "Alice"
        }"#;

        let resources = parse_document(json).unwrap();
        assert_eq!(resources.len(), 1);
        let r = &resources[0];
        assert_eq!(r.id().unwrap().as_str(), "urn:eigenius:example:alice");
    }

    #[test]
    fn parse_multi_resource_array() {
        let json = r#"[
            {"@id": "urn:eigenius:example:a", "urn:eigenius:example:x": 1},
            {"@id": "urn:eigenius:example:b", "urn:eigenius:example:x": 2}
        ]"#;

        let resources = parse_document(json).unwrap();
        assert_eq!(resources.len(), 2);
    }

    #[test]
    fn parse_embedded_resource() {
        let json = r#"{
            "@id": "urn:eigenius:example:alice",
            "urn:eigenius:example:address": {
                "urn:eigenius:example:city": "Berlin"
            }
        }"#;

        let resources = parse_document(json).unwrap();
        let r = &resources[0];
        let addr_prop = Iri::parse("urn:eigenius:example:address").unwrap();
        let addr = r.get(&addr_prop).unwrap().as_embedded().unwrap();
        assert!(addr.id().is_none());

        let city_prop = Iri::parse("urn:eigenius:example:city").unwrap();
        assert_eq!(addr.get(&city_prop).unwrap().as_str(), Some("Berlin"));
    }

    #[test]
    fn reject_null_value() {
        let json = r#"{"@id": "urn:eigenius:example:a", "urn:eigenius:example:x": null}"#;
        assert!(matches!(
            parse_document(json),
            Err(ParseError::NullNotAllowed { .. })
        ));
    }

    #[test]
    fn accept_empty_array() {
        // Empty arrays are valid Eigon — they represent "no items."
        // Per-property "must be non-empty" enforcement (e.g., `is_a`)
        // lives in the validator, not the parser.
        let json = r#"{"@id": "urn:eigenius:example:a", "urn:eigenius:example:x": []}"#;
        let doc = parse_document(json).expect("parse empty array");
        let r = &doc[0];
        let x = r
            .get(&Iri::parse("urn:eigenius:example:x").unwrap())
            .expect("property x present");
        assert!(matches!(x, Value::Array(arr) if arr.is_empty()));
    }

    #[test]
    fn reject_empty_object() {
        let json = r#"{"@id": "urn:eigenius:example:a", "urn:eigenius:example:x": {}}"#;
        assert!(matches!(
            parse_document(json),
            Err(ParseError::EmptyObject { .. })
        ));
    }

    #[test]
    fn reject_embedded_with_id() {
        let json = r#"{
            "@id": "urn:eigenius:example:a",
            "urn:eigenius:example:x": {"@id": "urn:eigenius:example:b"}
        }"#;
        assert!(matches!(
            parse_document(json),
            Err(ParseError::EmbeddedWithId)
        ));
    }

    #[test]
    fn parse_value_types() {
        let json = r#"{
            "@id": "urn:eigenius:example:test",
            "urn:eigenius:example:s": "hello",
            "urn:eigenius:example:i": 42,
            "urn:eigenius:example:f": 2.72,
            "urn:eigenius:example:b": true,
            "urn:eigenius:example:arr": [1, 2, 3]
        }"#;

        let resources = parse_document(json).unwrap();
        let r = &resources[0];

        let s = r
            .get(&Iri::parse("urn:eigenius:example:s").unwrap())
            .unwrap();
        assert_eq!(s.as_str(), Some("hello"));

        let i = r
            .get(&Iri::parse("urn:eigenius:example:i").unwrap())
            .unwrap();
        assert_eq!(i.as_integer(), Some(42));

        let f = r
            .get(&Iri::parse("urn:eigenius:example:f").unwrap())
            .unwrap();
        assert_eq!(f.as_float(), Some(2.72));

        let b = r
            .get(&Iri::parse("urn:eigenius:example:b").unwrap())
            .unwrap();
        assert_eq!(b.as_boolean(), Some(true));

        let arr = r
            .get(&Iri::parse("urn:eigenius:example:arr").unwrap())
            .unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 3);
    }

    #[test]
    fn round_trip() {
        let json = r#"{"@id":"urn:eigenius:example:test","urn:eigenius:example:name":"Alice","urn:eigenius:example:age":30}"#;
        let resources = parse_document(json).unwrap();
        let serialized = serialize_resource(&resources[0]);
        let reparsed = parse_document(&serialized.to_string()).unwrap();
        assert_eq!(resources[0].id(), reparsed[0].id());
    }

    #[test]
    fn parse_core_ontology() {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let resources = parse_document(core_json).unwrap();
        // Should have classes, properties, data types, formats, encodings
        assert!(resources.len() > 30);
        // All should be top-level (have @id)
        assert!(resources.iter().all(|r| r.is_top_level()));
    }

    #[test]
    fn canonicalize_deterministic() {
        let json = r#"{"@id":"urn:eigenius:example:test","urn:eigenius:example:z":"last","urn:eigenius:example:a":"first"}"#;
        let resources = parse_document(json).unwrap();
        let canon1 = canonicalize(&resources[0]);
        let canon2 = canonicalize(&resources[0]);
        assert_eq!(canon1, canon2);
        // Keys should be sorted: @id, then urn:...a, then urn:...z
        let as_str = String::from_utf8(canon1).unwrap();
        let a_pos = as_str.find("urn:eigenius:example:a").unwrap();
        let z_pos = as_str.find("urn:eigenius:example:z").unwrap();
        assert!(a_pos < z_pos);
    }
}
