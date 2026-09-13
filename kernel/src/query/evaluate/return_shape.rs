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
                Name::FullIri(iri) => Value::iri(iri),
                Name::ShortName(s) => Value::String(s.clone()),
            })
            .collect();
        if !class_values.is_empty() {
            resource.set(is_a_iri, Value::Array(class_values));
        }
    }

    for (position, item) in items.iter().enumerate() {
        let prop_iri = match &item.name {
            Name::FullIri(iri) => iri.clone(),
            Name::ShortName(s) => fp.row_property_iri(s),
        };

        // Handle aggregate expressions specially
        let value = match &item.expression {
            Expression::Aggregate { .. } => binding
                .get(&super::expression::aggregate_key(position))
                .cloned()
                .unwrap_or(Value::Integer(0)),
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
///
/// Sorting happens over the SHAPED resources, so an `ORDER BY` expression can only be
/// read if the `RETURN` list projected it as a column. Matching the expression against
/// that list is what makes `ORDER BY COUNT(?d) DESC` work: the count lives under whatever
/// name the `RETURN` item gave it, not under anything derivable from the expression.
///
/// It previously matched only a bare `Expression::Variable` and returned `None` for
/// everything else, so both operands were `None`, the comparison was skipped, and the
/// order was left untouched — D2 §8.8's own worked example returned an unordered result
/// with no error.
pub(super) fn sort_results(
    resources: &mut [Resource],
    order_by: &[OrderItem],
    items: &[ReturnItem],
    fp: &QueryFingerprint,
) {
    resources.sort_by(|a, b| {
        for item in order_by {
            let val_a = extract_sort_value(a, &item.expression, items, fp);
            let val_b = extract_sort_value(b, &item.expression, items, fp);

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
    items: &[ReturnItem],
    fp: &QueryFingerprint,
) -> Option<Value> {
    // The column the RETURN list projected this expression as. Covers every expression
    // form, aggregates included, and it is also more correct than the name-based read
    // below when a RETURN item renames a variable.
    if let Some(item) = items.iter().find(|i| i.expression == *expr) {
        let iri = match &item.name {
            Name::FullIri(iri) => iri.clone(),
            Name::ShortName(s) => fp.row_property_iri(s),
        };
        return resource.get(&iri).cloned();
    }
    // A bare variable that the RETURN list did not name explicitly still projects under
    // its own name, so it sorts without appearing in the list above.
    match expr {
        Expression::Variable(var) => resource.get(&fp.row_property_iri(&var.name)).cloned(),
        _ => None,
    }
}
