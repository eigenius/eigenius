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

/// Deduplicate resources (DISTINCT).
pub(super) fn deduplicate(resources: Vec<Resource>) -> Vec<Resource> {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let mut result = Vec::new();
    for resource in resources {
        let canonical = crate::ontology::eigon_json::canonicalize(&resource);
        if !seen.contains(&canonical) {
            seen.push(canonical);
            result.push(resource);
        }
    }
    result
}

/// Sort results by ORDER BY expressions.
pub(super) fn sort_results(
    resources: &mut [Resource],
    order_by: &[OrderItem],
    fp: &QueryFingerprint,
) {
    resources.sort_by(|a, b| {
        for item in order_by {
            let val_a = extract_sort_value(a, &item.expression, fp);
            let val_b = extract_sort_value(b, &item.expression, fp);

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

fn extract_sort_value(
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
