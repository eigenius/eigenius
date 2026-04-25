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

//! Result-document construction.
//!
//! Wraps evaluator-produced row resources into an Eigon document shaped
//! per D2 Appendix A: synthesized Property resources, a row Class, and a
//! ResultSet that references them.
//!
//! The synthesized IRIs live under `urn:eigenius:query:gen:<hash>:*`,
//! where `<hash>` is a stable hash of the query text (truncated SHA-256).
//! Re-running the same query produces identical IRIs, but nothing in the
//! kernel persists them — this is wire-shape only.

use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::query::ast::{AggregateOp, Expression, Name, Query};

use sha2::{Digest, Sha256};

// Well-known IRIs for the query-result ontology. Not persisted anywhere
// in the kernel layer stack — this is ephemeral metadata attached to
// query responses.
pub const RESULT_SET_CLASS: &str = "urn:eigenius:query:ResultSet";
pub const RESULT_CLASS_PROP: &str = "urn:eigenius:query:result_class";
pub const ROWS_PROP: &str = "urn:eigenius:query:rows";
pub const ROW_COUNT_PROP: &str = "urn:eigenius:query:row_count";
pub const MATCHED_PROP: &str = "urn:eigenius:query:matched";

/// A stable fingerprint for synthesized result-document IRIs.
pub struct QueryFingerprint {
    hash: String,
}

impl QueryFingerprint {
    pub fn of(query_text: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(query_text.as_bytes());
        let digest = hasher.finalize();
        // First 8 bytes (16 hex) is plenty for collision avoidance within
        // a single request and keeps the IRIs readable.
        let hash = hex::encode(&digest[..8]);
        Self { hash }
    }

    fn base(&self) -> String {
        format!("urn:eigenius:query:gen:{}", self.hash)
    }

    pub fn result_set_iri(&self) -> Iri {
        Iri::parse(&format!("{}:result", self.base())).unwrap()
    }

    pub fn row_class_iri(&self) -> Iri {
        Iri::parse(&format!("{}:row_class", self.base())).unwrap()
    }

    pub fn row_property_iri(&self, short_name: &str) -> Iri {
        Iri::parse(&format!("{}:row:{}", self.base(), short_name)).unwrap()
    }

    /// IRI for the `n`th FIBER response at the clause at `clause_idx`
    /// within the query. Deterministic per (query, clause, binding).
    pub fn fiber_response_iri(&self, clause_idx: usize, binding_idx: usize) -> Iri {
        Iri::parse(&format!(
            "{}:fiber:{}:{}",
            self.base(),
            clause_idx,
            binding_idx
        ))
        .unwrap()
    }
}

/// Wrap evaluator-produced row resources into a full result document.
///
/// The input `rows` have properties keyed by the synthesized row Property
/// IRIs (via `evaluate::shape_result`); this function adds the Property,
/// Class, and ResultSet metadata resources so the document is
/// self-describing per Appendix A.
pub fn wrap(query: &Query, query_text: &str, mut rows: Vec<Resource>) -> Vec<Resource> {
    let fp = QueryFingerprint::of(query_text);

    // Match-only queries (no RETURN) produce a minimal ResultSet with a
    // boolean `matched` property and no row-class metadata.
    if query.result.is_empty() {
        let mut result_set = Resource::new(fp.result_set_iri());
        result_set.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(RESULT_SET_CLASS.to_string())]),
        );
        result_set.set(
            Iri::parse(MATCHED_PROP).unwrap(),
            Value::Boolean(!rows.is_empty()),
        );
        result_set.set(
            Iri::parse(ROW_COUNT_PROP).unwrap(),
            Value::Integer(rows.len() as i64),
        );
        return vec![result_set];
    }

    let mut document: Vec<Resource> = Vec::new();
    let mut property_iris: Vec<String> = Vec::with_capacity(query.result.len());

    // Synthesize a Property resource for each RETURN item.
    for item in &query.result {
        let short_name = short_name_for(&item.name);
        let prop_iri = fp.row_property_iri(&short_name);
        property_iris.push(prop_iri.as_str().to_string());

        let mut prop = Resource::new(prop_iri.clone());
        prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
        );
        prop.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String(short_name),
        );
        prop.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::String(datatype_iri(&item.expression, &rows, &prop_iri)),
        );
        document.push(prop);
    }

    // Synthesize the row Class resource.
    let row_class_iri = fp.row_class_iri();
    let mut row_class = Resource::new(row_class_iri.clone());
    row_class.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
    );
    row_class.set(
        Iri::parse(wk::SHORT_NAME).unwrap(),
        Value::String(row_class_short_name(&query.result_classes)),
    );
    if !query.result_classes.is_empty() {
        // Parent classes: whatever the user declared in RETURN.
        let parents: Vec<Value> = query
            .result_classes
            .iter()
            .map(|n| Value::String(class_name_to_iri(n)))
            .collect();
        row_class.set(
            Iri::parse(wk::PARENT_CLASSES).unwrap(),
            Value::Array(parents),
        );
    }
    row_class.set(
        Iri::parse("urn:eigenius:core:properties").unwrap(),
        Value::Array(property_iris.into_iter().map(Value::String).collect()),
    );
    document.push(row_class);

    // Tag each row with `is_a: <row class IRI>` (add to any existing classes).
    let is_a_iri = Iri::parse(wk::IS_A).unwrap();
    for row in rows.iter_mut() {
        let row_class_val = Value::String(row_class_iri.as_str().to_string());
        match row.get(&is_a_iri).cloned() {
            Some(Value::Array(mut existing)) => {
                existing.push(row_class_val);
                row.set(is_a_iri.clone(), Value::Array(existing));
            }
            _ => {
                row.set(is_a_iri.clone(), Value::Array(vec![row_class_val]));
            }
        }
    }

    // Embed rows inline in the ResultSet — v1 is ephemeral per Appendix A §A.5,
    // so rows don't need stable IRIs the caller could reference.
    let row_count = rows.len() as i64;
    let embedded_rows: Vec<Value> = rows
        .into_iter()
        .map(|r| Value::Embedded(Box::new(r)))
        .collect();

    // Synthesize the ResultSet wrapper.
    let mut result_set = Resource::new(fp.result_set_iri());
    result_set.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(RESULT_SET_CLASS.to_string())]),
    );
    result_set.set(
        Iri::parse(RESULT_CLASS_PROP).unwrap(),
        Value::String(row_class_iri.as_str().to_string()),
    );
    result_set.set(Iri::parse(ROWS_PROP).unwrap(), Value::Array(embedded_rows));
    result_set.set(
        Iri::parse(ROW_COUNT_PROP).unwrap(),
        Value::Integer(row_count),
    );

    // Final order: Properties first, then Class, then ResultSet.
    document.push(result_set);
    document
}

fn short_name_for(name: &Name) -> String {
    match name {
        Name::ShortName(s) => s.clone(),
        Name::FullIri(iri) => iri
            .as_str()
            .rsplit(':')
            .next()
            .unwrap_or(iri.as_str())
            .to_string(),
    }
}

fn class_name_to_iri(name: &Name) -> String {
    match name {
        Name::ShortName(s) => s.clone(),
        Name::FullIri(iri) => iri.as_str().to_string(),
    }
}

fn row_class_short_name(classes: &[Name]) -> String {
    if classes.is_empty() {
        "QueryRow".to_string()
    } else {
        classes
            .iter()
            .map(short_name_for)
            .collect::<Vec<_>>()
            .join("_")
    }
}

/// Infer a datatype IRI for a RETURN expression. Aggregates have fixed
/// datatypes per D2 §A.3. For other expressions we peek at the first
/// row's value (if any) — v1 heuristic, acceptable while proper type
/// inference is future work.
fn datatype_iri(expr: &Expression, rows: &[Resource], prop_iri: &Iri) -> String {
    match expr {
        Expression::Aggregate { op, .. } => match op {
            AggregateOp::Count => wk::INTEGER.to_string(),
            AggregateOp::Sum => {
                // If every row's value is Integer, Sum is Integer; else Float.
                let all_int = rows
                    .iter()
                    .all(|r| matches!(r.get(prop_iri), Some(Value::Integer(_)) | None));
                if all_int {
                    wk::INTEGER.to_string()
                } else {
                    wk::FLOAT.to_string()
                }
            }
            AggregateOp::Avg => wk::FLOAT.to_string(),
            AggregateOp::Min | AggregateOp::Max => value_datatype_from_rows(rows, prop_iri),
        },
        _ => value_datatype_from_rows(rows, prop_iri),
    }
}

fn value_datatype_from_rows(rows: &[Resource], prop_iri: &Iri) -> String {
    for row in rows {
        if let Some(v) = row.get(prop_iri) {
            return match v {
                Value::String(_) => wk::STRING.to_string(),
                Value::Integer(_) => wk::INTEGER.to_string(),
                Value::Float(_) => wk::FLOAT.to_string(),
                Value::Boolean(_) => wk::BOOLEAN.to_string(),
                Value::ResourceRef(_) => wk::STRING.to_string(),
                // Fallback for complex shapes — Any/untyped.
                _ => "urn:eigenius:core:value".to_string(),
            };
        }
    }
    // No rows or property absent — default to untyped value.
    "urn:eigenius:core:value".to_string()
}
