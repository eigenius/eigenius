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

//! Thin accessors over schema.org's JSON-LD `@graph` nodes.
//!
//! schema.org's distribution is deliberately irregular — a field may be a
//! single `{ "@id": ... }` object, a list of them, or a bare string, and
//! `@type` is a string or a list. Rather than force rigid serde structs, we
//! parse to `serde_json::Value` and read fields through these helpers.

use serde_json::Value;

/// Parse a schema.org JSON-LD document and return its `@graph` nodes.
pub fn parse_graph(input: &str) -> Result<Vec<Value>, String> {
    let doc: Value = serde_json::from_str(input).map_err(|e| format!("invalid JSON: {e}"))?;
    match doc.get("@graph") {
        Some(Value::Array(nodes)) => Ok(nodes.clone()),
        _ => Err("document has no `@graph` array".to_string()),
    }
}

/// The node's `@id` (a CURIE such as `schema:Dataset`).
pub fn node_id(n: &Value) -> Option<&str> {
    n.get("@id").and_then(Value::as_str)
}

/// The node's `@type`(s), normalising the string-or-list shape.
pub fn node_types(n: &Value) -> Vec<&str> {
    match n.get("@type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// A plain string field (e.g. `rdfs:label`, `rdfs:comment`).
pub fn node_str<'a>(n: &'a Value, key: &str) -> Option<&'a str> {
    n.get(key).and_then(Value::as_str)
}

/// Resolve a field that holds IRI reference(s) — a `{ "@id": ... }` object, a
/// list of them, or a bare string — into the list of referenced ids (CURIEs).
pub fn iri_refs(n: &Value, key: &str) -> Vec<String> {
    fn collect(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(Value::String(s)) = m.get("@id") {
                    out.push(s.clone());
                }
            }
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|x| collect(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(v) = n.get(key) {
        collect(v, &mut out);
    }
    out
}
