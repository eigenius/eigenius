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

//! Expression evaluation: arithmetic, comparison, function calls,
//! Decidable QueryClass dispatch, Verdict postfix predicates,
//! aggregates and GROUP BY.

use crate::institution::registry::DispatchRole;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::error::QueryError;
use crate::query::functions::{self, like_match, to_f64, values_compare, values_equal};
use std::collections::BTreeMap;

use super::pattern::{find_property_by_shortname, literal_to_value, Binding};
use super::FiberRuntime;

/// Evaluate an expression against a binding.
pub(super) fn eval_expression(
    expr: &Expression,
    binding: &Binding,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    match expr {
        Expression::Literal(lit) => Ok(literal_to_value(lit)),
        Expression::Variable(var) => binding
            .get(&var.name)
            .cloned()
            .ok_or_else(|| QueryError::evaluation(format!("unbound variable: ?{}", var.name))),
        Expression::Binary { op, left, right } => {
            let l = eval_expression(left, binding, layer, runtime)?;
            let r = eval_expression(right, binding, layer, runtime)?;
            eval_binary(*op, &l, &r)
        }
        Expression::Unary { op, operand } => {
            let v = eval_expression(operand, binding, layer, runtime)?;
            eval_unary(*op, &v)
        }
        Expression::VerdictPredicate { kind, operand } => {
            let v = eval_expression(operand, binding, layer, runtime)?;
            eval_verdict_predicate(*kind, &v, layer, runtime)
        }
        Expression::NotExists(var) => Ok(Value::Boolean(!binding.contains_key(&var.name))),
        Expression::FunctionCall { name, args } => {
            let arg_vals: Result<Vec<Value>, QueryError> = args
                .iter()
                .map(|a| eval_expression(a, binding, layer, runtime))
                .collect();
            let arg_vals = arg_vals?;
            // D2 §3.8: qualified-name function calls dispatch as a
            // Decidable QueryClass invocation. The result is a
            // Verdict-typed resource (Value::Embedded). Comorphism
            // dispatch in expression position is not supported under
            // comorphisms surface only as FIBER param coercion
            // (D2 §3.5).
            if name.contains(':') {
                if let Ok(iri_parsed) = Iri::parse(name) {
                    if let Some(verdict) = try_dispatch_decidable(&iri_parsed, &arg_vals, runtime)?
                    {
                        return Ok(verdict);
                    }
                }
            }
            functions::call_function(name, &arg_vals)
        }
        Expression::Aggregate { .. } => {
            // Aggregates are handled during GROUP BY, not per-binding
            Err(QueryError::evaluation(
                "aggregate function outside GROUP BY context",
            ))
        }
        Expression::DotPath { root, segments } => {
            // Resolve the root variable to a resource IRI
            let root_val = binding.get(&root.name).ok_or_else(|| {
                QueryError::evaluation(format!("unbound variable: ?{}", root.name))
            })?;
            let mut current_iri = match root_val {
                Value::String(s) => Iri::parse(s).map_err(|_| {
                    QueryError::evaluation(format!("variable ?{} is not a resource IRI", root.name))
                })?,
                _ => {
                    return Err(QueryError::evaluation(format!(
                        "variable ?{} is not a resource IRI",
                        root.name
                    )))
                }
            };

            // Walk each segment except the last — resolve intermediate
            // resources via the overlay (FIBER responses) ∪ layer
            // chain, in that order. Without the overlay check, a
            // dot-path on a `FIBER … AS ?bound` variable would fail
            // to find ?bound because the synthesized response IRI
            // lives in the transient overlay, not the chain.
            for (i, segment) in segments.iter().enumerate() {
                let resource = resolve_iri_string(current_iri.as_str(), layer, runtime)
                    .ok_or_else(|| {
                        QueryError::evaluation(format!(
                            "resource '{}' not found in layer chain or FIBER overlay",
                            current_iri
                        ))
                    })?;
                let prop_iri = find_property_by_shortname(segment, resource.properties())
                    .ok_or_else(|| {
                        QueryError::evaluation(format!(
                            "property '{}' not found on resource '{}'",
                            segment, current_iri
                        ))
                    })?;
                let value = resource.get(&prop_iri).ok_or_else(|| {
                    QueryError::evaluation(format!(
                        "property '{}' has no value on resource '{}'",
                        segment, current_iri
                    ))
                })?;

                if i < segments.len() - 1 {
                    // Intermediate segment: must be a resource reference
                    current_iri = match value {
                        Value::String(s) => Iri::parse(s).map_err(|_| {
                            QueryError::evaluation(format!(
                                "property '{}' on '{}' is not a resource reference",
                                segment, current_iri
                            ))
                        })?,
                        _ => {
                            return Err(QueryError::evaluation(format!(
                                "property '{}' on '{}' is not a resource reference",
                                segment, current_iri
                            )))
                        }
                    };
                } else {
                    // Final segment: return the value
                    return Ok(value.clone());
                }
            }
            Err(QueryError::evaluation("empty dot-path segments"))
        }
        Expression::Array(elements) => {
            let vals: Result<Vec<Value>, QueryError> = elements
                .iter()
                .map(|e| eval_expression(e, binding, layer, runtime))
                .collect();
            Ok(Value::Array(vals?))
        }
        Expression::Object(_) => Err(QueryError::evaluation(
            "object literals in expressions not yet implemented",
        )),
        Expression::Similarity { .. } => eval_similarity(expr, binding, runtime),
    }
}

/// D43 §6 — per-row evaluation of `~`. Resolves the AST node to
/// its precomputed [`SimilarityProbe`] via pointer identity, looks
/// up the row's source-subject IRI, and returns `Boolean(true)` iff
/// the subject appears in the fused score map.
///
/// The score itself is held by the probe but not exposed through
/// the AST in v1 (see D43 §4.3 — the operator's value type is
/// Boolean; the score feeds ranking but isn't a user-bindable
/// Float). Future revisions can expose it via an `EXPLAIN`-shaped
/// surface (§3.7).
fn eval_similarity(
    expr: &Expression,
    binding: &Binding,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    let ctx = runtime.similarity.ok_or_else(|| {
        QueryError::evaluation(
            "similarity operator `~` invoked outside an evaluator pre-pass context",
        )
    })?;
    let probe = ctx.probe_for(expr).ok_or_else(|| {
        QueryError::evaluation("similarity operator `~` not registered in the pre-pass context")
    })?;
    let subject_value = binding.get(&probe.subject_var).ok_or_else(|| {
        QueryError::evaluation(format!(
            "similarity row-subject variable '?{}' is unbound at this position",
            probe.subject_var
        ))
    })?;
    let subject_iri = match subject_value {
        Value::ResourceRef(iri) => iri.clone(),
        Value::String(s) => match Iri::parse(s) {
            Ok(iri) => iri,
            Err(_) => return Ok(Value::Boolean(false)),
        },
        _ => return Ok(Value::Boolean(false)),
    };
    Ok(Value::Boolean(probe.scores.contains_key(&subject_iri)))
}

fn eval_binary(op: BinaryOp, left: &Value, right: &Value) -> Result<Value, QueryError> {
    match op {
        BinaryOp::Eq => Ok(Value::Boolean(values_equal(left, right))),
        BinaryOp::Neq => Ok(Value::Boolean(!values_equal(left, right))),
        BinaryOp::Lt => Ok(Value::Boolean(
            values_compare(left, right) == Some(std::cmp::Ordering::Less),
        )),
        BinaryOp::Lte => Ok(Value::Boolean(matches!(
            values_compare(left, right),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ))),
        BinaryOp::Gt => Ok(Value::Boolean(
            values_compare(left, right) == Some(std::cmp::Ordering::Greater),
        )),
        BinaryOp::Gte => Ok(Value::Boolean(matches!(
            values_compare(left, right),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ))),
        BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::Mod
        | BinaryOp::Pow => {
            let a = to_f64(left)
                .ok_or_else(|| QueryError::evaluation("arithmetic requires numeric operands"))?;
            let b = to_f64(right)
                .ok_or_else(|| QueryError::evaluation("arithmetic requires numeric operands"))?;
            let result = match op {
                BinaryOp::Add => a + b,
                BinaryOp::Sub => a - b,
                BinaryOp::Mul => a * b,
                BinaryOp::Div => {
                    if b == 0.0 {
                        return Err(QueryError::evaluation("division by zero"));
                    }
                    a / b
                }
                BinaryOp::Mod => {
                    if b == 0.0 {
                        return Err(QueryError::evaluation("modulo by zero"));
                    }
                    a % b
                }
                BinaryOp::Pow => a.powf(b),
                _ => unreachable!(),
            };
            // Preserve integer type if both operands are integers and result is integral
            if matches!(left, Value::Integer(_))
                && matches!(right, Value::Integer(_))
                && result.fract() == 0.0
                && !matches!(op, BinaryOp::Pow)
            {
                Ok(Value::Integer(result as i64))
            } else {
                Ok(Value::Float(result))
            }
        }
        BinaryOp::StringConcat => {
            let a = left
                .as_str()
                .ok_or_else(|| QueryError::evaluation("|| requires string operands"))?;
            let b = right
                .as_str()
                .ok_or_else(|| QueryError::evaluation("|| requires string operands"))?;
            Ok(Value::String(format!("{a}{b}")))
        }
        BinaryOp::And => {
            let a = left
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("AND requires boolean operands"))?;
            let b = right
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("AND requires boolean operands"))?;
            Ok(Value::Boolean(a && b))
        }
        BinaryOp::Or => {
            let a = left
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("OR requires boolean operands"))?;
            let b = right
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("OR requires boolean operands"))?;
            Ok(Value::Boolean(a || b))
        }
        BinaryOp::In => {
            if let Value::Array(arr) = right {
                Ok(Value::Boolean(arr.iter().any(|v| values_equal(left, v))))
            } else {
                Err(QueryError::evaluation("IN requires array on right side"))
            }
        }
        BinaryOp::NotIn => {
            if let Value::Array(arr) = right {
                Ok(Value::Boolean(!arr.iter().any(|v| values_equal(left, v))))
            } else {
                Err(QueryError::evaluation(
                    "NOT IN requires array on right side",
                ))
            }
        }
        BinaryOp::Like => {
            let val = left
                .as_str()
                .ok_or_else(|| QueryError::evaluation("LIKE requires string operands"))?;
            let pat = right
                .as_str()
                .ok_or_else(|| QueryError::evaluation("LIKE requires string operands"))?;
            Ok(Value::Boolean(like_match(val, pat)))
        }
        BinaryOp::NotLike => {
            let val = left
                .as_str()
                .ok_or_else(|| QueryError::evaluation("NOT LIKE requires string operands"))?;
            let pat = right
                .as_str()
                .ok_or_else(|| QueryError::evaluation("NOT LIKE requires string operands"))?;
            Ok(Value::Boolean(!like_match(val, pat)))
        }
    }
}

fn eval_unary(op: UnaryOp, val: &Value) -> Result<Value, QueryError> {
    match op {
        UnaryOp::Not => {
            let b = val
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("NOT requires boolean"))?;
            Ok(Value::Boolean(!b))
        }
        UnaryOp::Pos => {
            let n = to_f64(val).ok_or_else(|| QueryError::evaluation("+ requires numeric"))?;
            Ok(Value::Float(n))
        }
        UnaryOp::Neg => match val {
            Value::Integer(n) => Ok(Value::Integer(-n)),
            Value::Float(f) => Ok(Value::Float(-f)),
            _ => Err(QueryError::evaluation("- requires numeric")),
        },
    }
}

/// Project a `Verdict`-typed value to `Boolean` per a postfix predicate
/// (D2 v2 §3.7 / §3.8). The operand is one of:
///
/// - `Value::Embedded(verdict)` — the Verdict resource directly.
/// - `Value::String(iri)` / `Value::ResourceRef(iri)` — a synthesized
///   IRI (typically from a FIBER `AS ?var` binding) that resolves to
///   the response resource through the runtime's transient overlay or
///   the layer chain.
fn eval_verdict_predicate(
    kind: crate::query::ast::VerdictPredicate,
    val: &Value,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    let resolved: Resource;
    let resource: &Resource = match val {
        Value::Embedded(r) => r.as_ref(),
        Value::String(s) => {
            resolved = resolve_iri_string(s, layer, runtime).ok_or_else(|| {
                QueryError::evaluation(format!(
                    "{kw} operand IRI `{s}` does not resolve to a resource (FIBER overlay or layer chain)",
                    kw = kind.ctor_name(),
                ))
            })?;
            &resolved
        }
        Value::ResourceRef(iri) => {
            resolved = resolve_iri_string(iri.as_str(), layer, runtime).ok_or_else(|| {
                QueryError::evaluation(format!(
                    "{kw} operand IRI `{iri}` does not resolve to a resource",
                    kw = kind.ctor_name(),
                ))
            })?;
            &resolved
        }
        other => {
            return Err(QueryError::evaluation(format!(
                "{kw} expects a Verdict-typed operand; got {other:?}",
                kw = kind.ctor_name(),
            )));
        }
    };
    let ctor_iri = Iri::parse(wk::CTOR_NAME).expect("well-known IRI");
    let ctor = resource
        .get(&ctor_iri)
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| {
            QueryError::evaluation(
                "Verdict postfix predicate operand carries no `ctor_name` property",
            )
        })?;
    Ok(Value::Boolean(ctor == kind.ctor_name()))
}

/// Resolve a String IRI to a Resource — checks the FiberOverlay first
/// (so FIBER-bound responses are visible) then walks the layer chain.
pub(super) fn resolve_iri_string(
    s: &str,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Option<Resource> {
    let iri = Iri::parse(s).ok()?;
    if let Some(overlay) = runtime.overlay {
        for (entry_iri, entry_resource) in overlay {
            if entry_iri == &iri {
                return Some(entry_resource.clone());
            }
        }
    }
    layer.resolve(&iri).map(|r| (*r).clone())
}

/// Check if any return item uses an aggregate function.
pub(super) fn has_aggregates(result: &[ReturnItem]) -> bool {
    result
        .iter()
        .any(|item| expr_has_aggregate(&item.expression))
}

fn expr_has_aggregate(expr: &Expression) -> bool {
    match expr {
        Expression::Aggregate { .. } => true,
        Expression::Binary { left, right, .. } => {
            expr_has_aggregate(left) || expr_has_aggregate(right)
        }
        Expression::Unary { operand, .. } => expr_has_aggregate(operand),
        Expression::VerdictPredicate { operand, .. } => expr_has_aggregate(operand),
        Expression::FunctionCall { args, .. } => args.iter().any(expr_has_aggregate),
        _ => false,
    }
}

/// Try to dispatch a qualified-name call as a Decidable QueryClass
/// invocation (D2 §3.8 / §6.13). Returns:
///
/// - `Ok(Some(verdict))` if the IRI resolved to a Decidable QueryClass
///   and dispatch ran end-to-end. The Verdict is returned as a
///   `Value::Embedded` resource carrying `is_a = [Verdict]` and
///   `ctor_name`.
/// - `Ok(None)` if the index/runtime aren't attached, the IRI doesn't
///   resolve to a QueryClass, or the resolved QueryClass has no
///   Decidable role. The caller falls through to builtin function
///   dispatch (which raises `unknown function`).
/// - `Err(_)` if the index *did* find a Decidable QueryClass but a
///   downstream step failed (missing institution registration,
///   handler failure, etc.). A configured-but-broken QueryClass is a
///   structural error, not a reason to silently fall through.
///
/// Comorphism dispatch is not available in expression position under
/// comorphisms surface only as FIBER param coercion (D2 §3.5).
fn try_dispatch_decidable(
    iri: &Iri,
    args: &[Value],
    runtime: FiberRuntime<'_>,
) -> Result<Option<Value>, QueryError> {
    let (Some(index), Some(inst_runtime), Some(ctx)) =
        (runtime.index, runtime.runtime, runtime.ctx)
    else {
        return Ok(None);
    };
    let Some(qc_entry) = index.query_class(iri) else {
        return Ok(None);
    };
    if !qc_entry.dispatch_roles.contains(&DispatchRole::Decidable) {
        return Ok(None);
    }
    let institution = inst_runtime.get(&qc_entry.institution_ref).ok_or_else(|| {
        QueryError::evaluation(format!(
            "Decidable QueryClass `{iri}` declares institution `{}` not registered in runtime",
            qc_entry.institution_ref
        ))
    })?;

    // Marshal positional args onto a synthetic input resource of the
    // QueryClass's input class via the shared
    // `institution::marshal::marshal_decidable_input` helper. Same
    // logic as the kernel-side `nbe::eval::try_institution_decide` (D14 §9.2)
    // — typed required properties populated in `requires` order,
    // IRI-shaped args targeting `core:resource` properties
    // dereferenced to embedded resources.
    let input = crate::institution::marshal::marshal_decidable_input(
        &qc_entry.query_class,
        args,
        ctx.head(),
    )
    .map_err(|e| QueryError::evaluation(format!("Decidable QueryClass `{iri}`: {e}")))?;

    let outcome = institution
        .query(&qc_entry.query_handler, &input, ctx)
        .map_err(|e| {
            QueryError::evaluation(format!(
                "Decidable QueryClass `{iri}` handler `{}` failed: {e}",
                qc_entry.query_handler
            ))
        })?;

    // Decidable evaluation produces no chain-side RuntimeInvocation
    // commit — it's type-check-time reduction, not a Load. The
    // partial provenance (if any) is dropped here on purpose.
    Ok(Some(Value::Embedded(Box::new(outcome.output))))
}

/// Apply GROUP BY and aggregation.
pub(super) fn apply_group_by(
    group_by: &[Expression],
    result: &[ReturnItem],
    bindings: &[Binding],
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Vec<Binding>, QueryError> {
    // Group bindings by their group key values
    let mut groups: BTreeMap<Vec<String>, Vec<&Binding>> = BTreeMap::new();

    for binding in bindings {
        let key: Vec<String> = group_by
            .iter()
            .map(|expr| {
                eval_expression(expr, binding, layer, runtime)
                    .map(|v| format!("{v:?}"))
                    .unwrap_or_default()
            })
            .collect();
        groups.entry(key).or_default().push(binding);
    }

    let mut result_bindings = Vec::new();
    for group in groups.values() {
        let mut binding = group[0].clone(); // Start with first binding for non-aggregate values

        // Compute aggregates
        for item in result {
            if let Some((agg_name, agg_val)) =
                eval_aggregate(&item.expression, group, layer, runtime)?
            {
                binding.insert(agg_name, agg_val);
            }
        }

        result_bindings.push(binding);
    }

    Ok(result_bindings)
}

/// Evaluate an aggregate expression over a group of bindings.
fn eval_aggregate(
    expr: &Expression,
    group: &[&Binding],
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Option<(String, Value)>, QueryError> {
    if let Expression::Aggregate { op, arg } = expr {
        let values: Vec<Value> = group
            .iter()
            .filter_map(|b| eval_expression(arg, b, layer, runtime).ok())
            .collect();

        let result = match op {
            AggregateOp::Count => Value::Integer(values.len() as i64),
            AggregateOp::Sum => {
                let sum: f64 = values.iter().filter_map(to_f64).sum();
                if values.iter().all(|v| matches!(v, Value::Integer(_))) {
                    Value::Integer(sum as i64)
                } else {
                    Value::Float(sum)
                }
            }
            AggregateOp::Avg => {
                let vals: Vec<f64> = values.iter().filter_map(to_f64).collect();
                if vals.is_empty() {
                    Value::Float(0.0)
                } else {
                    Value::Float(vals.iter().sum::<f64>() / vals.len() as f64)
                }
            }
            AggregateOp::Min => values
                .iter()
                .min_by(|a, b| values_compare(a, b).unwrap_or(std::cmp::Ordering::Equal))
                .cloned()
                .unwrap_or(Value::Integer(0)),
            AggregateOp::Max => values
                .iter()
                .max_by(|a, b| values_compare(a, b).unwrap_or(std::cmp::Ordering::Equal))
                .cloned()
                .unwrap_or(Value::Integer(0)),
        };

        // Use a synthetic name for the aggregate in the binding
        let name = format!("__agg_{op:?}");
        Ok(Some((name, result)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ExecutionContext, ExecutionMode};
    use crate::institution::error::InstitutionError;
    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::layer::LayerBuilder;
    use crate::nbe::val::Val;
    use crate::ontology::eigon_json;
    use crate::query::lexer::tokenize;
    use crate::query::parser;
    use std::sync::Arc;

    const Q_INST_IRI: &str = "urn:eigenius:test:q_inst";
    const Q_POSITIVE_IRI: &str = "urn:eigenius:test:q_positive";
    const Q_INPUT_CLASS_IRI: &str = "urn:eigenius:test:QPositiveInput";
    const Q_HANDLER_IRI: &str = "urn:eigenius:test:proc:q_positive";

    /// Test institution implementing one Decidable QueryClass.
    /// `q_positive` returns Holds for a positive Integer on the
    /// input class's typed `arg_0` property, Fails otherwise. Phase
    /// 19d.7 dropped the `decide_args` array — args ride on typed
    /// required properties.
    struct QueryCapInst;

    const Q_ARG_0_PROP: &str = "urn:eigenius:test:QPositiveInput:arg_0";

    impl Institution for QueryCapInst {
        fn institution_iri(&self) -> &Iri {
            static INST: std::sync::OnceLock<Iri> = std::sync::OnceLock::new();
            INST.get_or_init(|| Iri::parse(Q_INST_IRI).unwrap())
        }
        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<Val, InstitutionError> {
            unreachable!("test fixture only implements query")
        }
        fn reify(
            &self,
            _: &Iri,
            _: &Val,
            _: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            unreachable!("test fixture only implements query")
        }
        fn query(
            &self,
            _procedure_iri: &Iri,
            input: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<crate::institution::runtime::QueryOutcome, InstitutionError> {
            // Read the typed `arg_0` property the kernel populates
            // from the first positional ESL arg.
            let arg_0_iri = Iri::parse(Q_ARG_0_PROP).unwrap();
            let ok = match input.get(&arg_0_iri) {
                Some(v) => v.as_integer().is_some_and(|n| n > 0),
                _ => false,
            };
            // Build a Verdict response carrying ctor_name.
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
            );
            r.set(
                Iri::parse(wk::CTOR_NAME).unwrap(),
                Value::String(if ok { "Holds" } else { "Fails" }.into()),
            );
            Ok(crate::institution::runtime::QueryOutcome::from_output(r))
        }
    }

    /// Build a layer carrying the core ontology + the q_test fixtures
    /// (Institution, QueryClass, typed input class) and an
    /// `InstitutionIndex` over it. Phase 19d.7 typed-marshaling needs
    /// the input class to resolve on the layer the dispatch sees, so
    /// the test layer must include both the q_test resources and the
    /// core ontology — the previous split-layer setup (q_test parent
    /// = None, separately-built core layer for the ExecutionContext)
    /// no longer works.
    fn q_index() -> (
        Arc<crate::layer::Layer>,
        crate::layer::LayerStorage,
        Arc<InstitutionIndex>,
    ) {
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_b = LayerBuilder::new("core", None);
        for r in core_resources {
            core_b.add_resource(r).unwrap();
        }
        let core_layer = Arc::new(core_b.build(storage.clone()));

        let mut b = LayerBuilder::new("q_test", Some(Arc::clone(&core_layer)));

        let mut inst = Resource::new(Iri::parse(Q_INST_IRI).unwrap());
        inst.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:institution:Institution".to_string(),
            )]),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            Value::String(Q_INST_IRI.to_string()),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            Value::String("QueryCapInst".to_string()),
        );
        b.add_resource(inst).unwrap();

        // Declare arg_0 property + the typed input class with
        // requires=[arg_0]. Phase 19d.7 typed-marshaling needs the
        // input class to resolve on the layer.
        let mut arg_prop = Resource::new(Iri::parse(Q_ARG_0_PROP).unwrap());
        arg_prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::PROPERTY.into())]),
        );
        b.add_resource(arg_prop).unwrap();
        let mut input_class = Resource::new(Iri::parse(Q_INPUT_CLASS_IRI).unwrap());
        input_class.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.into())]),
        );
        input_class.set(
            Iri::parse(wk::REQUIRES).unwrap(),
            Value::Array(vec![Value::String(Q_ARG_0_PROP.into())]),
        );
        b.add_resource(input_class).unwrap();

        let mut qc = Resource::new(Iri::parse(Q_POSITIVE_IRI).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.to_string())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            Value::String(Q_INPUT_CLASS_IRI.to_string()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            Value::String(wk::VERDICT.to_string()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            Value::Array(vec![Value::String(wk::DISPATCH_DECIDABLE.to_string())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            Value::String(Q_HANDLER_IRI.to_string()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            Value::String(Q_INST_IRI.to_string()),
        );
        b.add_resource(qc).unwrap();

        let layer = Arc::new(b.build(storage.clone()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "fixture index errors: {errors:?}");
        (layer, storage, Arc::new(idx))
    }

    fn q_runtime() -> Arc<InstitutionRuntime> {
        let mut runtime = InstitutionRuntime::new();
        runtime.register(Box::new(QueryCapInst)).unwrap();
        Arc::new(runtime)
    }

    fn q_exec_ctx(
        layer: Arc<crate::layer::Layer>,
        storage: crate::layer::LayerStorage,
    ) -> ExecutionContext {
        ExecutionContext::new(layer, "q_test", ExecutionMode::ReadOnly, storage)
    }

    #[test]
    fn parser_accepts_qualified_function_calls() {
        // Parse-only: the parser must accept `ns:local(args)`
        // without requiring institution registration.
        let source = r#"
            MATCH ?x {}
            WHERE cap:q_positive(42)
            RETURN [] { ok: ?x }
        "#;
        let tokens = tokenize(source).unwrap();
        let _program = parser::parse(tokens).expect("parse qualified call");
    }

    #[test]
    fn where_clause_decide_dispatch_returns_verdict() {
        // a Decidable QueryClass call returns a Verdict
        // resource (not a Boolean). The postfix predicate (D2 §3.8)
        // is what projects to Boolean — a separate parser concern.
        let (layer, storage, index) = q_index();
        let inst_runtime = q_runtime();
        let exec_ctx = q_exec_ctx(Arc::clone(&layer), storage);

        let runtime = FiberRuntime {
            index: Some(&index),
            runtime: Some(&inst_runtime),
            components: None,
            overlay: None,
            ctx: Some(&exec_ctx),
            similarity: None,
            embedders: None,
            embedding_cache: None,
            vector_segment_cache: None,
        };

        // Use FunctionCall directly at eval_expression level for a
        // focused test — the full-query integration would need more
        // pattern-matching infrastructure. This verifies the core
        // dispatch path.
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: Q_POSITIVE_IRI.to_string(),
            args: vec![Expression::Literal(Literal::Integer(42))],
        };
        let v = eval_expression(&expr, &binding, &layer, runtime).expect("eval");
        let verdict = match v {
            Value::Embedded(r) => r,
            other => panic!("expected embedded Verdict, got {other:?}"),
        };
        let ctor = verdict
            .get(&Iri::parse(wk::CTOR_NAME).unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(ctor.as_deref(), Some("Holds"));

        // Negative arg → Fails.
        let expr_neg = Expression::FunctionCall {
            name: Q_POSITIVE_IRI.to_string(),
            args: vec![Expression::Literal(Literal::Integer(-5))],
        };
        let v = eval_expression(&expr_neg, &binding, &layer, runtime).expect("eval");
        let verdict = match v {
            Value::Embedded(r) => r,
            other => panic!("expected embedded Verdict, got {other:?}"),
        };
        let ctor = verdict
            .get(&Iri::parse(wk::CTOR_NAME).unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(ctor.as_deref(), Some("Fails"));
    }

    #[test]
    fn unknown_iri_falls_through_to_builtin_error() {
        // An IRI that doesn't resolve to a Decidable QueryClass falls
        // through to `functions::call_function`, which errors with
        // "no such function."
        let (layer, storage, index) = q_index();
        let inst_runtime = q_runtime();
        let exec_ctx = q_exec_ctx(Arc::clone(&layer), storage);

        let runtime = FiberRuntime {
            index: Some(&index),
            runtime: Some(&inst_runtime),
            components: None,
            overlay: None,
            ctx: Some(&exec_ctx),
            similarity: None,
            embedders: None,
            embedding_cache: None,
            vector_segment_cache: None,
        };

        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "urn:eigenius:test:unknown_fn".to_string(),
            args: vec![],
        };
        let err = eval_expression(&expr, &binding, &layer, runtime).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown") || msg.contains("function"));
    }

    /// Phase 19d.7 follow-on: when an EigenQL Decidable predicate
    /// receives an IRI-shaped arg targeting a typed `core:resource`
    /// property, the kernel dereferences the IRI to the embedded
    /// chain resource before serialising for the institution. This
    /// is the same plumbing fix that landed for FIBER param values
    /// in `embed_typed_resource_param` — both surfaces now share
    /// `institution::marshal::embed_typed_resource_arg`.
    #[test]
    fn decide_dereferences_iri_args_for_typed_resource_props() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static OBSERVED_EMBEDDED: AtomicBool = AtomicBool::new(false);

        const DEREF_INST_IRI: &str = "urn:eigenius:test:deref_inst";
        const DEREF_QC_IRI: &str = "urn:eigenius:test:deref_qc";
        const DEREF_INPUT_CLASS_IRI: &str = "urn:eigenius:test:DerefInput";
        const DEREF_TARGET_PROP_IRI: &str = "urn:eigenius:test:DerefInput:target";
        const DEREF_TARGET_INSTANCE_IRI: &str = "urn:eigenius:test:deref_target";

        struct DerefInst;
        impl Institution for DerefInst {
            fn institution_iri(&self) -> &Iri {
                static INST: std::sync::OnceLock<Iri> = std::sync::OnceLock::new();
                INST.get_or_init(|| Iri::parse(DEREF_INST_IRI).unwrap())
            }
            fn extract_typed(
                &self,
                _: &Iri,
                _: &Resource,
                _: &ExecutionContext,
            ) -> Result<Val, InstitutionError> {
                unreachable!()
            }
            fn reify(
                &self,
                _: &Iri,
                _: &Val,
                _: &ExecutionContext,
            ) -> Result<Resource, InstitutionError> {
                unreachable!()
            }
            fn query(
                &self,
                _: &Iri,
                input: &Resource,
                _: &ExecutionContext,
            ) -> Result<crate::institution::runtime::QueryOutcome, InstitutionError> {
                // The target property must be Embedded (the kernel
                // dereferenced the IRI), NOT String — that's the
                // entire point.
                let target = input.get(&Iri::parse(DEREF_TARGET_PROP_IRI).unwrap());
                if matches!(target, Some(Value::Embedded(_))) {
                    OBSERVED_EMBEDDED.store(true, Ordering::SeqCst);
                }
                let mut r = Resource::new_embedded();
                r.set(
                    Iri::parse(wk::IS_A).unwrap(),
                    Value::Array(vec![Value::String(wk::VERDICT.into())]),
                );
                r.set(
                    Iri::parse(wk::CTOR_NAME).unwrap(),
                    Value::String("Holds".into()),
                );
                Ok(crate::institution::runtime::QueryOutcome::from_output(r))
            }
        }

        // Layer carries:
        //   - a target instance the IRI arg references
        //   - the typed `target: core:resource` property
        //   - the input class with `requires: [target]`
        //   - the Decidable QueryClass + Institution
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_b = LayerBuilder::new("core", None);
        for r in core_resources {
            core_b.add_resource(r).unwrap();
        }
        let core_layer = Arc::new(core_b.build(storage.clone()));

        let mut b = LayerBuilder::new("deref_test", Some(Arc::clone(&core_layer)));

        // Target instance (some chain-committed resource).
        let mut target = Resource::new(Iri::parse(DEREF_TARGET_INSTANCE_IRI).unwrap());
        target.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("the_target".into()),
        );
        b.add_resource(target).unwrap();

        // Property declaration with `data_type: core:resource`.
        let mut prop = Resource::new(Iri::parse(DEREF_TARGET_PROP_IRI).unwrap());
        prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::PROPERTY.into())]),
        );
        prop.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::String(wk::RESOURCE.into()),
        );
        b.add_resource(prop).unwrap();

        // Input class.
        let mut input_class = Resource::new(Iri::parse(DEREF_INPUT_CLASS_IRI).unwrap());
        input_class.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.into())]),
        );
        input_class.set(
            Iri::parse(wk::REQUIRES).unwrap(),
            Value::Array(vec![Value::String(DEREF_TARGET_PROP_IRI.into())]),
        );
        b.add_resource(input_class).unwrap();

        // Institution.
        let mut inst = Resource::new(Iri::parse(DEREF_INST_IRI).unwrap());
        inst.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:institution:Institution".into(),
            )]),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            Value::String(DEREF_INST_IRI.into()),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            Value::String("DerefInst".into()),
        );
        b.add_resource(inst).unwrap();

        // QueryClass.
        let mut qc = Resource::new(Iri::parse(DEREF_QC_IRI).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            Value::String(DEREF_INPUT_CLASS_IRI.into()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            Value::String(wk::VERDICT.into()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            Value::Array(vec![Value::String(wk::DISPATCH_DECIDABLE.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            Value::String("urn:eigenius:test:deref:handler".into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            Value::String(DEREF_INST_IRI.into()),
        );
        b.add_resource(qc).unwrap();

        let layer = Arc::new(b.build(storage.clone()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");
        let mut rt = InstitutionRuntime::new();
        rt.register(Box::new(DerefInst)).unwrap();

        let exec_ctx = q_exec_ctx(Arc::clone(&layer), storage);
        let inst_runtime = Arc::new(rt);
        let runtime = FiberRuntime {
            index: Some(&idx),
            runtime: Some(&inst_runtime),
            components: None,
            overlay: None,
            ctx: Some(&exec_ctx),
            similarity: None,
            embedders: None,
            embedding_cache: None,
            vector_segment_cache: None,
        };

        // Pass the IRI as a String literal — same shape MATCH
        // bindings produce when binding `?var` to a chain resource
        // subject. The kernel must dereference it before the
        // institution sees the input.
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: DEREF_QC_IRI.to_string(),
            args: vec![Expression::Literal(Literal::String(
                DEREF_TARGET_INSTANCE_IRI.into(),
            ))],
        };
        let _ = eval_expression(&expr, &binding, &layer, runtime).expect("eval");
        assert!(
            OBSERVED_EMBEDDED.load(Ordering::SeqCst),
            "institution must have observed the typed property as Embedded — \
             the kernel's IRI-dereference pass should have unwrapped the IRI"
        );
    }

    // ─── D2 v2 §3.7 / §3.8 — postfix Verdict predicate ─────────────────

    #[test]
    fn parser_accepts_postfix_verdict_predicates() {
        // The grammar verdict_term ::= primary_expr (verdict_predicate)?
        // sits between unary and primary. All three postfix tokens must
        // parse, AND combinations across postfix-projected operands must
        // still parse.
        let source = r#"
            MATCH ?x {}
            WHERE cap:q_positive(42) HOLDS
              AND cap:other(?x) FAILS
              AND cap:third(?x) UNDECIDABLE
            RETURN [] { ok: ?x }
        "#;
        let tokens = tokenize(source).unwrap();
        let _program = parser::parse(tokens).expect("parse postfix predicates");
    }

    #[test]
    fn parser_postfix_binds_tighter_than_not() {
        // `NOT qc:check(?x) HOLDS` should parse as `NOT (qc:check(?x) HOLDS)`,
        // not `(NOT qc:check(?x)) HOLDS`. Verify by inspecting the AST shape.
        use crate::query::ast::{Expression, UnaryOp, VerdictPredicate};
        let source = r#"
            MATCH ?x {}
            WHERE NOT cap:q_positive(?x) HOLDS
            RETURN [] { ok: ?x }
        "#;
        let tokens = tokenize(source).unwrap();
        let program = parser::parse(tokens).expect("parse NOT-postfix");
        let cond = program
            .query
            .body
            .conditions
            .first()
            .expect("WHERE condition");
        match cond {
            Expression::Unary { op, operand } => {
                assert_eq!(*op, UnaryOp::Not);
                match operand.as_ref() {
                    Expression::VerdictPredicate { kind, .. } => {
                        assert_eq!(*kind, VerdictPredicate::Holds);
                    }
                    other => panic!("expected `NOT (qc HOLDS)`, got NOT followed by {other:?}"),
                }
            }
            other => panic!("expected `NOT …`, got {other:?}"),
        }
    }

    #[test]
    fn postfix_holds_projects_verdict_to_boolean() {
        // Build a Verdict resource with ctor_name = "Holds" and project
        // it through each of the three postfix predicates.
        use crate::query::ast::{Expression, VerdictPredicate};
        let mut verdict = Resource::new_embedded();
        verdict.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
        );
        verdict.set(
            Iri::parse(wk::CTOR_NAME).unwrap(),
            Value::String("Holds".into()),
        );
        let layer = Arc::new(
            LayerBuilder::new("postfix-test", None).build(crate::layer::LayerStorage::in_memory()),
        );
        let runtime = FiberRuntime::default();
        let mut binding: BTreeMap<String, Value> = BTreeMap::new();
        binding.insert("v".into(), Value::Embedded(Box::new(verdict)));

        let var_v = Expression::Variable(crate::query::ast::Variable { name: "v".into() });
        let project = |kind: VerdictPredicate| -> Value {
            eval_expression(
                &Expression::VerdictPredicate {
                    kind,
                    operand: Box::new(var_v.clone()),
                },
                &binding,
                &layer,
                runtime,
            )
            .expect("eval verdict predicate")
        };
        assert_eq!(project(VerdictPredicate::Holds), Value::Boolean(true));
        assert_eq!(project(VerdictPredicate::Fails), Value::Boolean(false));
        assert_eq!(
            project(VerdictPredicate::Undecidable),
            Value::Boolean(false)
        );
    }

    #[test]
    fn postfix_predicate_rejects_non_verdict_operand() {
        // A non-Verdict operand (e.g. an Integer) should error with a
        // type-mismatch evaluation error rather than silently returning
        // false. Type-checker enforcement of this rule lands as part of
        // §5.9 rule coverage; the runtime guard is the floor.
        use crate::query::ast::{Expression, Literal, VerdictPredicate};
        let layer = Arc::new(
            LayerBuilder::new("postfix-test", None).build(crate::layer::LayerStorage::in_memory()),
        );
        let runtime = FiberRuntime::default();
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::VerdictPredicate {
            kind: VerdictPredicate::Holds,
            operand: Box::new(Expression::Literal(Literal::Integer(42))),
        };
        let err = eval_expression(&expr, &binding, &layer, runtime).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Verdict-typed operand"),
            "unexpected message: {msg}"
        );
    }
}
