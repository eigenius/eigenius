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

//! RETURN projection: row construction, result-class wrapping,
//! DISTINCT deduplication, ORDER BY.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::document::QueryFingerprint;
use crate::query::error::QueryError;
use crate::query::functions::values_compare;

use super::expression::eval_expression;
use super::pattern::Binding;
use super::FiberRuntime;

/// Shape a binding into a result resource.
///
/// Property IRIs for short-name RETURN items are synthesized from `fp`,
/// so the downstream document wrapper produces matching Property metadata
/// resources. Full-IRI RETURN items use the user-supplied IRI unchanged.
pub(super) fn shape_result(
    binding: &Binding,
    classes: &[Name],
    items: &[ReturnItem],
    layer: &Layer,
    fp: &QueryFingerprint,
    runtime: FiberRuntime<'_>,
) -> Result<Resource, QueryError> {
    let mut resource = Resource::new_embedded(); // Result resources don't get @id

    // Set is_a from result classes
    if !classes.is_empty() {
        let is_a_iri = Iri::parse(wk::IS_A).unwrap();
        let class_values: Vec<Value> = classes
            .iter()
            .map(|n| match n {
                Name::FullIri(iri) => Value::String(iri.as_str().to_string()),
                Name::ShortName(s) => Value::String(s.clone()),
            })
            .collect();
        if !class_values.is_empty() {
            resource.set(is_a_iri, Value::Array(class_values));
        }
    }

    for item in items {
        let prop_iri = match &item.name {
            Name::FullIri(iri) => iri.clone(),
            Name::ShortName(s) => fp.row_property_iri(s),
        };

        // Handle aggregate expressions specially
        let value = match &item.expression {
            Expression::Aggregate { op, .. } => {
                let agg_key = format!("__agg_{op:?}");
                binding.get(&agg_key).cloned().unwrap_or(Value::Integer(0))
            }
            _ => eval_expression(&item.expression, binding, layer, runtime)
                .map_err(|e| QueryError::evaluation(format!("in RETURN: {e}")))?,
        };

        resource.set(prop_iri, value);
    }

    Ok(resource)
}

/// Convert a binding to a simple resource (for match queries without RETURN).
pub(super) fn binding_to_resource(binding: &Binding, _classes: &[Name]) -> Resource {
    let mut resource = Resource::new_embedded();
    for (key, value) in binding {
        if let Ok(iri) = Iri::parse(&format!("urn:query:var:{key}")) {
            resource.set(iri, value.clone());
        }
    }
    resource
}

/// One element of the post-shape result pipeline: the original
/// binding index (for RRF context lookup during sort), the
/// underlying binding (for full expression evaluation), and the
/// shaped result resource.
///
/// D43 §6.4 / M7.4 — sort and DISTINCT operate on this triple so
/// `TOP K BY RRF(...)` can evaluate the score expression against
/// the binding (with the rrf context) instead of being restricted
/// to row-property short-name lookups.
pub(super) type ResultRow = (usize, Binding, Resource);

/// Deduplicate result rows by canonical-form equality on the
/// shaped resource (DISTINCT). The original binding stays paired
/// with its representative row so the sort path can still evaluate
/// against the right context.
pub(super) fn deduplicate(rows: Vec<ResultRow>) -> Vec<ResultRow> {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let mut result = Vec::new();
    for (idx, binding, resource) in rows {
        let canonical = crate::ontology::eigon_json::canonicalize(&resource);
        if !seen.contains(&canonical) {
            seen.push(canonical);
            result.push((idx, binding, resource));
        }
    }
    result
}

/// Sort result rows by ORDER BY / TOP K BY expressions. D43 §6.4 /
/// M7.4 — the key extractor first tries `eval_expression` against
/// the underlying binding (with `current_binding_idx` set so RRF
/// can look up its materialised ranks); if that errors, it falls
/// back to the legacy row-property short-name lookup so existing
/// surfaces (notably ORDER BY of a RETURN-renamed aggregate) keep
/// working.
pub(super) fn sort_results(
    rows: &mut [ResultRow],
    order_by: &[OrderItem],
    fp: &QueryFingerprint,
    layer: &Layer,
    runtime: super::FiberRuntime<'_>,
) {
    rows.sort_by(|a, b| {
        for item in order_by {
            let val_a = sort_key(a, &item.expression, fp, layer, runtime);
            let val_b = sort_key(b, &item.expression, fp, layer, runtime);

            if let (Some(va), Some(vb)) = (&val_a, &val_b) {
                if let Some(ord) = values_compare(va, vb) {
                    let ord = match item.direction {
                        SortDirection::Asc => ord,
                        SortDirection::Desc => ord.reverse(),
                    };
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                }
            }
        }
        std::cmp::Ordering::Equal
    });
}

/// Compute the sort key for one row under `expr`. Tries
/// binding-eval first (handles RRF, BIND-introduced variables,
/// arithmetic combinations, and MATCH-bound variables); on error
/// (e.g. an aggregate AST node that's only legal in GROUP BY
/// context), falls back to the row-property short-name lookup that
/// the pre-M7.4 sort used exclusively. The fallback preserves the
/// existing `ORDER BY ?aggregate_renamed_in_RETURN` shape.
fn sort_key(
    row: &ResultRow,
    expr: &Expression,
    fp: &QueryFingerprint,
    layer: &Layer,
    runtime: super::FiberRuntime<'_>,
) -> Option<Value> {
    let (binding_idx, binding, resource) = row;
    let runtime_for_sort = super::FiberRuntime {
        current_binding_idx: Some(*binding_idx),
        ..runtime
    };
    match eval_expression(expr, binding, layer, runtime_for_sort) {
        Ok(v) => Some(v),
        Err(_) => row_property_fallback(resource, expr, fp),
    }
}

fn row_property_fallback(
    resource: &Resource,
    expr: &Expression,
    fp: &QueryFingerprint,
) -> Option<Value> {
    match expr {
        Expression::Variable(var) => {
            let iri = fp.row_property_iri(&var.name);
            resource.get(&iri).cloned()
        }
        _ => None,
    }
}
